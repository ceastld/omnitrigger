//! 有界剪贴板格式读取，只在动作工作线程调用；文件路径始终作为数据处理。

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardData {
    Text(String),
    Files(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardDataSnapshot {
    pub data: ClipboardData,
    pub sequence_number: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardDataError {
    pub code: &'static str,
    pub message: String,
}

impl std::fmt::Display for ClipboardDataError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ClipboardDataError {}

/// 序号在读取租约中、格式取出后采样；不证明数据由哪个窗口生成。
/// 有界重试不构成延迟渲染等所有 Win32 调用的硬超时。
#[cfg(windows)]
pub fn read_clipboard_data() -> Result<ClipboardDataSnapshot, ClipboardDataError> {
    reader::read_with(&mut native::Clipboard)
}

#[cfg(any(windows, test))]
mod reader {
    use super::{ClipboardData, ClipboardDataError, ClipboardDataSnapshot};

    pub(super) const UNICODE_TEXT: u32 = 13;
    pub(super) const FILE_DROP: u32 = 15;
    pub(super) const MAX_UNITS: usize = 65_536;
    const MAX_FILES: usize = 64;
    pub(super) const MAX_TEXT_BYTES: usize = (MAX_UNITS + 1) * 2;
    pub(super) const DROP_HEADER_BYTES: usize = 20;
    const OPEN_ATTEMPTS: usize = 8;

    pub(super) fn error(code: &'static str, message: &str) -> ClipboardDataError {
        ClipboardDataError {
            code,
            message: message.into(),
        }
    }

    pub(super) fn malformed() -> ClipboardDataError {
        error(
            "invalid_clipboard_data",
            "剪贴板格式、终止标记或 Unicode 数据无效。",
        )
    }

    pub(super) fn too_large() -> ClipboardDataError {
        error("clipboard_too_large", "剪贴板数据超过读取上限。")
    }

    pub(super) fn file_byte_limit(wide: bool) -> usize {
        // 包含各项终止符与列表终止符；ANSI 输入采用每个 UTF-16 单元最多四字节上界。
        if wide {
            (MAX_UNITS + MAX_FILES + 1) * 2
        } else {
            MAX_UNITS * 4 + MAX_FILES + 1
        }
    }

    pub(super) trait Backend {
        fn open(&mut self) -> Result<(), ClipboardDataError>;
        fn close(&mut self) -> Result<(), ClipboardDataError>;
        fn available(&mut self, format: u32) -> bool;
        fn bytes(&mut self, format: u32) -> Result<Vec<u8>, ClipboardDataError>;
        fn decode_ansi(&mut self, bytes: &[u8]) -> Result<String, ClipboardDataError>;
        fn sequence_number(&mut self) -> u32;
        fn wait_retry(&mut self);
    }

    struct Lease<'a, B: Backend> {
        backend: &'a mut B,
        active: bool,
    }

    impl<'a, B: Backend> Lease<'a, B> {
        fn open(backend: &'a mut B) -> Result<Self, ClipboardDataError> {
            for attempt in 0..OPEN_ATTEMPTS {
                if backend.open().is_ok() {
                    return Ok(Self {
                        backend,
                        active: true,
                    });
                }
                if attempt + 1 < OPEN_ATTEMPTS {
                    backend.wait_retry();
                }
            }
            Err(error("clipboard_busy", "剪贴板在有限重试后仍无法打开。"))
        }

        fn close(mut self) -> Result<(), ClipboardDataError> {
            self.active = false;
            self.backend.close()
        }
    }

    impl<B: Backend> Drop for Lease<'_, B> {
        fn drop(&mut self) {
            if self.active {
                let _ = self.backend.close();
            }
        }
    }

    pub(super) fn read_with(
        backend: &mut impl Backend,
    ) -> Result<ClipboardDataSnapshot, ClipboardDataError> {
        let lease = Lease::open(backend)?;
        let data = if lease.backend.available(UNICODE_TEXT) {
            ClipboardData::Text(parse_text(&lease.backend.bytes(UNICODE_TEXT)?)?)
        } else if lease.backend.available(FILE_DROP) {
            let bytes = lease.backend.bytes(FILE_DROP)?;
            ClipboardData::Files(parse_files(&bytes, |part| lease.backend.decode_ansi(part))?)
        } else {
            return Err(error(
                "unsupported_clipboard_format",
                "剪贴板没有受支持的文本或文件列表格式。",
            ));
        };
        // 延迟渲染可能在 GetClipboardData 内改变序号，因此不使用读取前的序号。
        let sequence_number = lease.backend.sequence_number();
        lease.close()?;
        Ok(ClipboardDataSnapshot {
            data,
            sequence_number,
        })
    }

    fn wide_units(bytes: &[u8]) -> Result<Vec<u16>, ClipboardDataError> {
        if !bytes.len().is_multiple_of(2) {
            return Err(malformed());
        }
        Ok(bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect())
    }

    fn parse_text(bytes: &[u8]) -> Result<String, ClipboardDataError> {
        let count = bytes.len().min(MAX_TEXT_BYTES) / 2 * 2;
        let bounded = &bytes[..count];
        let units = wide_units(bounded)?;
        let end = units.iter().position(|unit| *unit == 0).ok_or_else(|| {
            if units.len() > MAX_UNITS {
                too_large()
            } else {
                malformed()
            }
        })?;
        String::from_utf16(&units[..end]).map_err(|_| malformed())
    }

    pub(super) fn file_layout(
        header: &[u8],
        size: usize,
    ) -> Result<(usize, bool), ClipboardDataError> {
        if header.len() < DROP_HEADER_BYTES || size < DROP_HEADER_BYTES {
            return Err(malformed());
        }
        let offset = u32::from_le_bytes(header[..4].try_into().map_err(|_| malformed())?) as usize;
        let wide = u32::from_le_bytes(header[16..20].try_into().map_err(|_| malformed())?) != 0;
        if offset < DROP_HEADER_BYTES
            || offset > size
            || offset > isize::MAX as usize
            || (wide && !offset.is_multiple_of(2))
        {
            return Err(malformed());
        }
        Ok((offset, wide))
    }

    fn parse_files(
        bytes: &[u8],
        decode_ansi: impl FnMut(&[u8]) -> Result<String, ClipboardDataError>,
    ) -> Result<Vec<String>, ClipboardDataError> {
        let (offset, wide) = file_layout(bytes, bytes.len())?;
        let count = (bytes.len() - offset).min(file_byte_limit(wide));
        let list = &bytes[offset..offset + count];
        if wide {
            // 最后一个不完整单元只能是终止后的分配填充；无终止符仍会失败。
            let units = wide_units(&list[..list.len() / 2 * 2])?;
            parse_list(&units, MAX_UNITS, |part| {
                String::from_utf16(part).map_err(|_| malformed())
            })
        } else {
            parse_list(list, MAX_UNITS * 4, decode_ansi)
        }
    }

    fn parse_list<T: Copy + PartialEq + Default>(
        list: &[T],
        max_encoded_units: usize,
        mut decode: impl FnMut(&[T]) -> Result<String, ClipboardDataError>,
    ) -> Result<Vec<String>, ClipboardDataError> {
        let zero = T::default();
        if list.first() == Some(&zero) {
            return if list.get(1) == Some(&zero) {
                Ok(Vec::new())
            } else {
                Err(malformed())
            };
        }
        let mut paths = Vec::new();
        let mut total_units = 0;
        let mut cursor = 0;
        loop {
            if paths.len() == MAX_FILES {
                return Err(too_large());
            }
            let tail = list.get(cursor..).ok_or_else(malformed)?;
            let length = tail.iter().position(|unit| *unit == zero).ok_or_else(|| {
                if tail.len() > max_encoded_units {
                    too_large()
                } else {
                    malformed()
                }
            })?;
            // 在解码/分配前还要约束单项输入；ANSI 最多按 UTF-8 四字节上界计。
            if length > max_encoded_units {
                return Err(too_large());
            }
            let path = decode(&tail[..length])?;
            if path.is_empty() || path.contains('\0') {
                return Err(malformed());
            }
            let count = path.encode_utf16().take(MAX_UNITS + 1).count();
            total_units += count;
            if total_units > MAX_UNITS {
                return Err(too_large());
            }
            paths.push(path);
            cursor += length + 1;
            match list.get(cursor) {
                Some(unit) if *unit == zero => return Ok(paths),
                Some(_) => {}
                None => return Err(malformed()),
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn wide(text: &str) -> Vec<u8> {
            text.encode_utf16().flat_map(u16::to_le_bytes).collect()
        }

        fn drop_data(wide: bool, payload: &[u8]) -> Vec<u8> {
            let mut bytes = vec![0; DROP_HEADER_BYTES];
            bytes[..4].copy_from_slice(&(DROP_HEADER_BYTES as u32).to_le_bytes());
            bytes[16..20].copy_from_slice(&u32::from(wide).to_le_bytes());
            bytes.extend_from_slice(payload);
            bytes
        }

        fn files(bytes: &[u8]) -> Result<Vec<String>, ClipboardDataError> {
            parse_files(bytes, |_| panic!("宽字符格式不能调用 ANSI 解码"))
        }

        #[test]
        fn text_preserves_empty_unicode_and_stops_at_terminator() {
            assert_eq!(parse_text(&wide("\0")), Ok(String::new()));
            assert_eq!(
                parse_text(&wide("中😀\n\0private padding")),
                Ok("中😀\n".into())
            );
            assert_eq!(
                parse_text(&wide(&format!("{}\0", "a".repeat(MAX_UNITS)))),
                Ok("a".repeat(MAX_UNITS))
            );
        }

        #[test]
        fn text_rejects_missing_terminator_odd_memory_surrogates_and_limits() {
            for bytes in [
                vec![],
                vec![0],
                wide("no terminator"),
                vec![0, 0xd8, 0, 0],
                vec![0, 0xdc, 0, 0],
            ] {
                assert_eq!(
                    parse_text(&bytes).unwrap_err().code,
                    "invalid_clipboard_data"
                );
            }
            assert_eq!(
                parse_text(&wide(&format!("{}\0", "a".repeat(MAX_UNITS + 1))))
                    .unwrap_err()
                    .code,
                "clipboard_too_large"
            );
            let mut padded = wide("short\0");
            padded.resize(2 * 1024 * 1024 + 1, 0xff);
            assert_eq!(parse_text(&padded), Ok("short".into()));
        }

        #[test]
        fn wide_file_list_preserves_data_without_path_operations() {
            let paths = files(&drop_data(
                true,
                &wide("C:\\中😀.txt\0\\\\server\\share\\file\0relative data\0\0padding"),
            ))
            .unwrap();
            assert_eq!(
                paths,
                ["C:\\中😀.txt", "\\\\server\\share\\file", "relative data"]
            );
            assert_eq!(files(&drop_data(true, &wide("\0\0"))), Ok(vec![]));
            let mut gap = drop_data(true, &[]);
            gap[..4].copy_from_slice(&22_u32.to_le_bytes());
            gap.extend_from_slice(&[0xff, 0xff]);
            gap.extend_from_slice(&wide("x\0\0"));
            assert_eq!(files(&gap), Ok(vec!["x".into()]));
        }

        #[test]
        fn file_headers_and_termination_are_checked_before_decode() {
            let valid = drop_data(true, &wide("x\0\0"));
            let mut fixtures = vec![
                vec![],
                vec![0; 19],
                drop_data(true, &[]),
                drop_data(true, &wide("\0")),
                drop_data(true, &wide("x\0")),
                drop_data(true, &wide("x")),
                drop_data(true, &[0, 0xd8, 0, 0, 0, 0]),
            ];
            for offset in [0_u32, 19, 21, u32::MAX] {
                let mut bytes = valid.clone();
                bytes[..4].copy_from_slice(&offset.to_le_bytes());
                fixtures.push(bytes);
            }
            let mut padded = valid;
            padded.resize(2 * 1024 * 1024 + 1, 0xff);
            assert_eq!(files(&padded), Ok(vec!["x".into()]));
            for bytes in fixtures {
                assert_eq!(files(&bytes).unwrap_err().code, "invalid_clipboard_data");
            }
        }

        #[test]
        fn file_count_and_aggregate_utf16_limits_include_supplementary_chars() {
            let sixty_four = format!("{}\0", "x\0".repeat(MAX_FILES));
            assert_eq!(
                files(&drop_data(true, &wide(&sixty_four))).unwrap().len(),
                MAX_FILES
            );
            let sixty_five = format!("{}\0", "x\0".repeat(MAX_FILES + 1));
            assert_eq!(
                files(&drop_data(true, &wide(&sixty_five)))
                    .unwrap_err()
                    .code,
                "clipboard_too_large"
            );
            let exact = format!("{}\0\0", "😀".repeat(MAX_UNITS / 2));
            assert_eq!(files(&drop_data(true, &wide(&exact))).unwrap().len(), 1);
            let over = format!("{}\0x\0\0", "😀".repeat(MAX_UNITS / 2));
            assert_eq!(
                files(&drop_data(true, &wide(&over))).unwrap_err().code,
                "clipboard_too_large"
            );
        }

        #[test]
        fn ansi_uses_explicit_decoder_and_requires_double_terminator() {
            let bytes = drop_data(false, b"C:\\caf\xe9.txt\0second\0\0");
            let mut inputs = Vec::new();
            let result = parse_files(&bytes, |part| {
                inputs.push(part.to_vec());
                Ok(part.iter().map(|byte| char::from(*byte)).collect())
            })
            .unwrap();
            assert_eq!(result, ["C:\\café.txt", "second"]);
            assert_eq!(inputs.len(), 2);
            assert_eq!(
                parse_files(&drop_data(false, &[0, 0]), |_| unreachable!()),
                Ok(vec![])
            );
            for bytes in [b"\0".as_slice(), b"x\0", b"x"] {
                assert_eq!(
                    parse_files(&drop_data(false, bytes), |_| Ok("x".into()))
                        .unwrap_err()
                        .code,
                    "invalid_clipboard_data"
                );
            }
            assert_eq!(
                parse_files(&bytes, |_| Err(malformed())).unwrap_err().code,
                "invalid_clipboard_data"
            );
            assert_eq!(
                parse_files(&bytes, |_| Ok("x".repeat(MAX_UNITS + 1)))
                    .unwrap_err()
                    .code,
                "clipboard_too_large"
            );
        }

        #[derive(Default)]
        struct Mock {
            open_failures: usize,
            opens: usize,
            waits: usize,
            closes: usize,
            close_fails: bool,
            read_fails: bool,
            text: Option<Vec<u8>>,
            files: Option<Vec<u8>>,
            events: Vec<&'static str>,
        }

        impl Backend for Mock {
            fn open(&mut self) -> Result<(), ClipboardDataError> {
                self.opens += 1;
                if self.opens <= self.open_failures {
                    Err(malformed())
                } else {
                    Ok(())
                }
            }
            fn close(&mut self) -> Result<(), ClipboardDataError> {
                self.closes += 1;
                self.events.push("close");
                if self.close_fails {
                    Err(error("clipboard_error", "无法关闭剪贴板。"))
                } else {
                    Ok(())
                }
            }
            fn available(&mut self, format: u32) -> bool {
                match format {
                    UNICODE_TEXT => self.text.is_some(),
                    FILE_DROP => self.files.is_some(),
                    _ => unreachable!(),
                }
            }
            fn bytes(&mut self, format: u32) -> Result<Vec<u8>, ClipboardDataError> {
                self.events.push("bytes");
                if self.read_fails {
                    return Err(error("clipboard_error", "无法取得剪贴板数据。"));
                }
                Ok(match format {
                    UNICODE_TEXT => self.text.clone().unwrap(),
                    FILE_DROP => self.files.clone().unwrap(),
                    _ => unreachable!(),
                })
            }
            fn decode_ansi(&mut self, _: &[u8]) -> Result<String, ClipboardDataError> {
                panic!("测试未提供 ANSI 数据")
            }
            fn sequence_number(&mut self) -> u32 {
                self.events.push("sequence");
                u32::MAX
            }
            fn wait_retry(&mut self) {
                self.waits += 1;
            }
        }

        #[test]
        fn reader_prefers_even_empty_text_and_samples_before_close() {
            let mut mock = Mock {
                text: Some(wide("\0")),
                files: Some(vec![]),
                ..Default::default()
            };
            assert_eq!(
                read_with(&mut mock),
                Ok(ClipboardDataSnapshot {
                    data: ClipboardData::Text(String::new()),
                    sequence_number: u32::MAX
                })
            );
            assert_eq!(mock.events, ["bytes", "sequence", "close"]);
            assert_eq!(mock.closes, 1);
            mock.text = Some(vec![1]);
            assert_eq!(
                read_with(&mut mock).unwrap_err().code,
                "invalid_clipboard_data"
            );
            assert_eq!(mock.closes, 2);
            mock.text = None;
            mock.files = Some(drop_data(true, &wide("x\0\0")));
            assert_eq!(
                read_with(&mut mock).unwrap().data,
                ClipboardData::Files(vec!["x".into()])
            );
        }

        #[test]
        fn reader_retries_are_bounded_and_failure_paths_close_only_open_leases() {
            let mut mock = Mock {
                open_failures: OPEN_ATTEMPTS,
                ..Default::default()
            };
            assert_eq!(read_with(&mut mock).unwrap_err().code, "clipboard_busy");
            assert_eq!((mock.opens, mock.waits, mock.closes), (8, 7, 0));
            let mut mock = Mock {
                open_failures: 2,
                ..Default::default()
            };
            assert_eq!(
                read_with(&mut mock).unwrap_err().code,
                "unsupported_clipboard_format"
            );
            assert_eq!((mock.opens, mock.waits, mock.closes), (3, 2, 1));
            let mut mock = Mock {
                text: Some(wide("x\0")),
                close_fails: true,
                ..Default::default()
            };
            assert_eq!(read_with(&mut mock).unwrap_err().code, "clipboard_error");
            assert_eq!(mock.closes, 1);
            let mut mock = Mock {
                text: Some(wide("x\0")),
                read_fails: true,
                ..Default::default()
            };
            assert_eq!(read_with(&mut mock).unwrap_err().code, "clipboard_error");
            assert_eq!(mock.events, ["bytes", "close"]);
        }
    }
}

#[cfg(windows)]
mod native {
    use super::{reader, ClipboardDataError};
    use windows::Win32::{
        Foundation::HGLOBAL,
        Globalization::{GetACP, MultiByteToWideChar, MB_ERR_INVALID_CHARS},
        System::{
            DataExchange::{
                CloseClipboard, GetClipboardData, GetClipboardSequenceNumber,
                IsClipboardFormatAvailable, OpenClipboard,
            },
            Memory::{GlobalLock, GlobalSize, GlobalUnlock},
        },
    };

    pub(super) struct Clipboard;

    struct Unlock(HGLOBAL);

    impl Drop for Unlock {
        fn drop(&mut self) {
            // 锁计数归零时 GlobalUnlock 也可能返回零，不能据此断言失败。
            let _ = unsafe { GlobalUnlock(self.0) };
        }
    }

    impl reader::Backend for Clipboard {
        fn open(&mut self) -> Result<(), ClipboardDataError> {
            unsafe { OpenClipboard(None) }
                .map_err(|_| reader::error("clipboard_busy", "无法打开剪贴板。"))
        }

        fn close(&mut self) -> Result<(), ClipboardDataError> {
            unsafe { CloseClipboard() }
                .map_err(|_| reader::error("clipboard_error", "无法关闭剪贴板。"))
        }

        fn available(&mut self, format: u32) -> bool {
            unsafe { IsClipboardFormatAvailable(format) }.is_ok()
        }

        fn bytes(&mut self, format: u32) -> Result<Vec<u8>, ClipboardDataError> {
            let handle = unsafe { GetClipboardData(format) }
                .map_err(|_| reader::error("clipboard_error", "无法取得剪贴板数据。"))?;
            let memory = HGLOBAL(handle.0);
            let size = unsafe { GlobalSize(memory) };
            if size == 0 {
                return Err(reader::malformed());
            }
            let pointer = unsafe { GlobalLock(memory) };
            if pointer.is_null() {
                return Err(reader::error("clipboard_error", "无法锁定剪贴板数据。"));
            }
            let _unlock = Unlock(memory);
            // 只构造有界字节切片，不把未验证 DROPFILES 交给 DragQueryFile。
            // GlobalSize 的额外填充不是实际数据长度，短内容不会因巨大尾部填充而失败。
            if format == reader::UNICODE_TEXT {
                let count = size.min(reader::MAX_TEXT_BYTES);
                return Ok(
                    unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), count) }.to_vec(),
                );
            }
            if format != reader::FILE_DROP || size < reader::DROP_HEADER_BYTES {
                return Err(reader::malformed());
            }
            let header = unsafe {
                std::slice::from_raw_parts(pointer.cast::<u8>(), reader::DROP_HEADER_BYTES)
            };
            let (offset, wide) = reader::file_layout(header, size)?;
            let count = (size - offset).min(reader::file_byte_limit(wide));
            // offset 已对实际分配范围、头部与 UTF-16 对齐验证，count 不超过剩余分配。
            let list =
                unsafe { std::slice::from_raw_parts(pointer.cast::<u8>().add(offset), count) };
            let mut bytes = Vec::with_capacity(reader::DROP_HEADER_BYTES + count);
            bytes.extend_from_slice(header);
            // 移除可能很大的头后间隙，将已验证的列表复制到规范化头部之后。
            bytes[..4].copy_from_slice(&(reader::DROP_HEADER_BYTES as u32).to_le_bytes());
            bytes.extend_from_slice(list);
            Ok(bytes)
        }

        fn decode_ansi(&mut self, bytes: &[u8]) -> Result<String, ClipboardDataError> {
            // CF_HDROP 的 fWide=0 明确使用系统 ANSI 代码页；禁止有损替换。
            decode_ansi_at(bytes, unsafe { GetACP() })
        }

        fn sequence_number(&mut self) -> u32 {
            unsafe { GetClipboardSequenceNumber() }
        }

        fn wait_retry(&mut self) {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    fn decode_ansi_at(bytes: &[u8], code_page: u32) -> Result<String, ClipboardDataError> {
        if bytes.len() > reader::MAX_UNITS * 4 {
            return Err(reader::too_large());
        }
        let count = unsafe { MultiByteToWideChar(code_page, MB_ERR_INVALID_CHARS, bytes, None) };
        if count <= 0 {
            return Err(reader::malformed());
        }
        if count as usize > reader::MAX_UNITS {
            return Err(reader::too_large());
        }
        let mut units = vec![0; count as usize];
        let written = unsafe {
            MultiByteToWideChar(code_page, MB_ERR_INVALID_CHARS, bytes, Some(&mut units))
        };
        if written != count {
            return Err(reader::malformed());
        }
        String::from_utf16(&units).map_err(|_| reader::malformed())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn conversion_is_explicit_strict_and_bounded_without_clipboard_io() {
            assert_eq!(decode_ansi_at(b"caf\xe9", 1252), Ok("café".into()));
            assert_eq!(decode_ansi_at("文😀".as_bytes(), 65001), Ok("文😀".into()));
            for bytes in [b"\xff".as_slice(), b"\xc0\xaf", b"\xed\xa0\x80"] {
                assert_eq!(
                    decode_ansi_at(bytes, 65001).unwrap_err().code,
                    "invalid_clipboard_data"
                );
            }
            assert_eq!(
                decode_ansi_at(&vec![b'x'; reader::MAX_UNITS + 1], 65001)
                    .unwrap_err()
                    .code,
                "clipboard_too_large"
            );
            assert_eq!(
                decode_ansi_at(b"", 65001).unwrap_err().code,
                "invalid_clipboard_data"
            );
        }
    }
}
