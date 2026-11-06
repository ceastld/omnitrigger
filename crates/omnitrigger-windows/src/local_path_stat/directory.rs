//! 句柄枚举直接子项；不打开子项，不递归，也不跟随子项链接。
//! 路径/身份复验可发现部分并发变更，不提供名称空间锁或原子快照。
use super::{checkpoint, fault, validate_local_path, LocalStatError};

pub const MAX_DIRECTORY_ENTRIES: usize = 1024;
pub const MAX_DIRECTORY_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalDirectoryEntry {
    pub name: String,
    pub path: String,
    pub kind: &'static str,
    pub reason: Option<&'static str>,
}

fn directory_fault(code: &'static str) -> LocalStatError {
    let message = match code {
        "path_missing" => "目标目录或祖先不存在。",
        "not_directory" => "目标不是目录。",
        "directory_too_large" => "目录子项数量或回执总大小超过上限。",
        "directory_changed" => "目录在枚举期间发生变化。",
        _ => return fault(code),
    };
    LocalStatError {
        code,
        message: message.into(),
    }
}

/// max_entries 为 1–1024；结果总 JSON 字节数有 64 KiB 上界，超限整次报错。
/// check 在每个 OS 调用前后运行，不能抢占已经进入的系统调用。
pub fn read_local_directory(
    path: &str,
    max_entries: usize,
    check: impl Fn() -> Result<(), LocalStatError>,
) -> Result<Vec<LocalDirectoryEntry>, LocalStatError> {
    checkpoint(&check)?;
    validate_local_path(path)?;
    if !(1..=MAX_DIRECTORY_ENTRIES).contains(&max_entries) {
        return Err(directory_fault("invalid_input"));
    }
    #[cfg(windows)]
    {
        native::read(path, max_entries, &check)
    }
    #[cfg(not(windows))]
    {
        Err(directory_fault("fs_unavailable"))
    }
}

#[cfg(any(windows, test))]
struct Collector<'a> {
    path: &'a str,
    max_entries: usize,
    // 32 字节外壳，加每项两个 JSON 字符串的精确转义成本和固定字段的保守上界。
    json_bound: usize,
    entries: Vec<LocalDirectoryEntry>,
    names: std::collections::BTreeSet<String>,
}

#[cfg(any(windows, test))]
fn json_string_bytes(text: &str) -> usize {
    2 + text
        .chars()
        .map(|character| match character {
            '"' | '\\' | '\u{8}' | '\u{c}' | '\n' | '\r' | '\t' => 2,
            '\0'..='\u{1f}' => 6,
            _ => character.len_utf8(),
        })
        .sum::<usize>()
}

#[cfg(any(windows, test))]
impl<'a> Collector<'a> {
    fn new(path: &'a str, max_entries: usize) -> Self {
        Self {
            path,
            max_entries,
            json_bound: 32,
            entries: Vec::new(),
            names: Default::default(),
        }
    }

    fn add(&mut self, name: String, attributes: u32) -> Result<(), LocalStatError> {
        if matches!(name.as_str(), "." | "..") {
            return Ok(());
        }
        // 不允许一个记录构造多个组件；不可表示的 Unicode 在解码器中整次拒绝。
        if name.is_empty() || name.contains(['\\', '/', '\0']) {
            return Err(directory_fault("fs_error"));
        }
        if self.entries.len() >= self.max_entries {
            return Err(directory_fault("directory_too_large"));
        }
        let path = format!(
            "{}{}{}",
            self.path,
            if self.path.ends_with(['\\', '/']) {
                ""
            } else {
                "\\"
            },
            name
        );
        if path.encode_utf16().take(super::MAX_PATH_UNITS + 1).count() > super::MAX_PATH_UNITS {
            return Err(directory_fault("directory_too_large"));
        }
        let cost = json_string_bytes(&name) + json_string_bytes(&path) + 128;
        if self.json_bound + cost > MAX_DIRECTORY_BYTES {
            return Err(directory_fault("directory_too_large"));
        }
        if !self.names.insert(name.clone()) {
            return Err(directory_fault("directory_changed"));
        }
        let reason = if attributes & 0x400 != 0 {
            Some("reparse_point")
        } else if attributes & (0x1000 | 0x40000 | 0x400000) != 0 {
            Some("cloud_placeholder")
        } else if let Err(error) = validate_local_path(&path) {
            Some(if error.code == "path_unavailable" {
                "cloud_path"
            } else {
                "unsupported_name"
            })
        } else {
            None
        };
        let kind = if reason.is_some() {
            "unavailable"
        } else if attributes & super::ATTRIBUTE_DEVICE != 0 {
            "other"
        } else if attributes & super::ATTRIBUTE_DIRECTORY != 0 {
            "directory"
        } else {
            "file"
        };
        self.json_bound += cost;
        self.entries.push(LocalDirectoryEntry {
            name,
            path,
            kind,
            reason,
        });
        Ok(())
    }

    fn finish(mut self) -> Vec<LocalDirectoryEntry> {
        self.entries.sort_by(|a, b| a.name.cmp(&b.name));
        self.entries
    }
}

// FILE_FULL_DIR_INFO 的 FileName 偏移为 68；Native 的布局断言在 Windows 测试核验。
// 切片读取避免给变长记录建立可能未对齐/越界的 Rust 引用。
#[cfg(any(windows, test))]
fn parse_buffer(
    bytes: &[u8],
    collector: &mut Collector<'_>,
    check: &impl Fn() -> Result<(), LocalStatError>,
) -> Result<(), LocalStatError> {
    let mut offset = 0usize;
    loop {
        checkpoint(check)?;
        let header = bytes
            .get(
                offset
                    ..offset
                        .checked_add(68)
                        .ok_or_else(|| directory_fault("fs_error"))?,
            )
            .ok_or_else(|| directory_fault("fs_error"))?;
        let u32_at =
            |index| u32::from_le_bytes(header[index..index + 4].try_into().expect("fixed header"));
        let next = u32_at(0) as usize;
        let attributes = u32_at(56);
        let length = u32_at(60) as usize;
        if length == 0 || !length.is_multiple_of(2) || length / 2 > super::MAX_PATH_UNITS {
            return Err(directory_fault("fs_error"));
        }
        let end = offset
            .checked_add(68)
            .and_then(|start| start.checked_add(length))
            .ok_or_else(|| directory_fault("fs_error"))?;
        let name = bytes
            .get(offset + 68..end)
            .ok_or_else(|| directory_fault("fs_error"))?;
        let units: Vec<u16> = name
            .chunks_exact(2)
            .map(|unit| u16::from_le_bytes([unit[0], unit[1]]))
            .collect();
        let name = String::from_utf16(&units).map_err(|_| directory_fault("fs_error"))?;
        collector.add(name, attributes)?;
        if next == 0 {
            return Ok(());
        }
        if !next.is_multiple_of(8) || next < 68 + length {
            return Err(directory_fault("fs_error"));
        }
        offset = offset
            .checked_add(next)
            .ok_or_else(|| directory_fault("fs_error"))?;
    }
}

#[cfg(windows)]
mod native {
    use super::super::{
        classify_attributes, error_for_os, physical_volume_device, stat_local_path, system_step,
        LocalPathKind, Metadata, NativeMetadata,
    };
    use super::*;
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::{CloseHandle, HANDLE},
            Storage::FileSystem::{
                CreateFileW, FileFullDirectoryInfo, GetFileInformationByHandle,
                GetFileInformationByHandleEx, GetFinalPathNameByHandleW,
                BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_NO_RECALL,
                FILE_FLAG_OPEN_REPARSE_POINT, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES,
                FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
                VOLUME_NAME_NT,
            },
        },
    };

    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: 本类型仅持有成功创建且唯一拥有的句柄。
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
    fn code(error: windows::core::Error) -> LocalStatError {
        error_for_os(error.code().0 as u32 & 0xffff)
    }
    fn require_directory(kind: LocalPathKind) -> Result<(), LocalStatError> {
        match kind {
            LocalPathKind::Directory => Ok(()),
            LocalPathKind::Missing => Err(directory_fault("path_missing")),
            _ => Err(directory_fault("not_directory")),
        }
    }
    fn open(path: &str) -> Result<Handle, LocalStatError> {
        let normalized = path.replace('/', "\\");
        let normalized = if normalized.len() > 3 {
            normalized.trim_end_matches('\\')
        } else {
            &normalized
        };
        let wide: Vec<u16> = "\\\\?\\"
            .encode_utf16()
            .chain(normalized.encode_utf16())
            .chain(Some(0))
            .collect();
        // SAFETY: 词法验证已完成，缓冲区 NUL 结尾；只请求枚举/属性访问。
        // OPEN_REPARSE_POINT 防止最终组件的链接被跟随；祖先和最终名称另行复验。
        unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                FILE_LIST_DIRECTORY.0 | FILE_READ_ATTRIBUTES.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS
                    | FILE_FLAG_OPEN_REPARSE_POINT
                    | FILE_FLAG_OPEN_NO_RECALL,
                None,
            )
        }
        .map(Handle)
        .map_err(code)
    }
    fn info(handle: &Handle) -> Result<BY_HANDLE_FILE_INFORMATION, LocalStatError> {
        let mut information = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: 句柄和可写结构在调用期间有效。
        unsafe { GetFileInformationByHandle(handle.0, &mut information) }.map_err(code)?;
        require_directory(classify_attributes(Ok(information.dwFileAttributes))?)?;
        Ok(information)
    }
    fn verify_name(
        handle: &Handle,
        path: &str,
        check: &impl Fn() -> Result<(), LocalStatError>,
    ) -> Result<(), LocalStatError> {
        let drive = &path[..2];
        let device =
            system_step(check, || NativeMetadata.current_device(drive))?.map_err(error_for_os)?;
        if !physical_volume_device(&device) {
            return Err(directory_fault("path_not_local"));
        }
        let mut buffer = vec![0u16; super::super::MAX_PATH_UNITS + 1];
        let name = system_step(check, || {
            // SAFETY: 句柄有效，输出切片大小固定；NT 卷名可与受限 DOS 当前映射核对。
            let length = unsafe { GetFinalPathNameByHandleW(handle.0, &mut buffer, VOLUME_NAME_NT) }
                as usize;
            if length == 0 {
                return Err(error_for_os(
                    unsafe { windows::Win32::Foundation::GetLastError() }.0,
                ));
            }
            if length >= buffer.len() {
                return Err(directory_fault("path_unavailable"));
            }
            String::from_utf16(&buffer[..length]).map_err(|_| directory_fault("path_unavailable"))
        })??;
        let expected = format!("{}\\{}", device, path[3..].replace('/', "\\"));
        // 比较副本只去单个合法尾分隔符，不改变公开路径。非 ASCII 大小写别名可保守拒绝。
        if !name
            .trim_end_matches('\\')
            .eq_ignore_ascii_case(expected.trim_end_matches('\\'))
        {
            return Err(directory_fault("path_unavailable"));
        }
        Ok(())
    }
    fn identity(value: &BY_HANDLE_FILE_INFORMATION) -> (u32, u32, u32) {
        (
            value.dwVolumeSerialNumber,
            value.nFileIndexHigh,
            value.nFileIndexLow,
        )
    }

    pub(super) fn read(
        path: &str,
        max_entries: usize,
        check: &impl Fn() -> Result<(), LocalStatError>,
    ) -> Result<Vec<LocalDirectoryEntry>, LocalStatError> {
        require_directory(stat_local_path(path, check)?)?;
        // system_step 内部创建 RAII 包装：若返回后取消，丢弃 Result 时同样关闭句柄。
        let handle = system_step(check, || open(path))??;
        let original = system_step(check, || info(&handle))??;
        verify_name(&handle, path, check)?;
        // 打开后复验祖先；句柄枚举始终针对已核验的原目录。
        require_directory(stat_local_path(path, check)?)?;
        let mut collector = Collector::new(path, max_entries);
        let mut buffer = vec![0u64; 8192]; // 64 KiB、8 字节对齐，不随目录大小增长。
        let mut batches = 0;
        loop {
            batches += 1;
            if batches > max_entries + 3 {
                return Err(directory_fault("directory_changed"));
            }
            buffer.fill(0);
            let result = system_step(check, || {
                // SAFETY: 有效目录句柄；固定、对齐、可写的 64 KiB 缓冲区。
                unsafe {
                    GetFileInformationByHandleEx(
                        handle.0,
                        FileFullDirectoryInfo,
                        buffer.as_mut_ptr().cast(),
                        MAX_DIRECTORY_BYTES as u32,
                    )
                }
            })?;
            match result {
                Ok(()) => {
                    // SAFETY: u64 的全部位模式有效；只借用已分配缓冲区的字节范围。
                    let bytes = unsafe {
                        std::slice::from_raw_parts(
                            buffer.as_ptr().cast::<u8>(),
                            MAX_DIRECTORY_BYTES,
                        )
                    };
                    parse_buffer(bytes, &mut collector, check)?;
                }
                Err(error) if error.code().0 as u32 & 0xffff == 18 => break, // ERROR_NO_MORE_FILES
                Err(error) if error.code().0 as u32 & 0xffff == 234 => {
                    return Err(directory_fault("directory_too_large"));
                }
                Err(error) => return Err(code(error)),
            }
        }
        require_directory(stat_local_path(path, check)?)?;
        let current = system_step(check, || open(path))??;
        let current_info = system_step(check, || info(&current))??;
        verify_name(&current, path, check)?;
        verify_name(&handle, path, check)?;
        if identity(&original) != identity(&current_info) {
            return Err(directory_fault("directory_changed"));
        }
        checkpoint(check)?;
        Ok(collector.finish())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record(name: &[u16], attrs: u32) -> Vec<u8> {
        let mut value = vec![0u8; 68 + name.len() * 2];
        value[56..60].copy_from_slice(&attrs.to_le_bytes());
        value[60..64].copy_from_slice(&((name.len() * 2) as u32).to_le_bytes());
        for (i, unit) in name.iter().enumerate() {
            value[68 + i * 2..70 + i * 2].copy_from_slice(&unit.to_le_bytes());
        }
        value
    }
    #[test]
    fn actual_decoder_and_collector_keep_unavailable_children_without_traversal() {
        let mut collector = Collector::new("C:/fixture/", 10);
        for (name, attrs) in [
            ("file.txt", 0),
            ("directory", 0x10),
            ("link", 0x410),
            ("offline", 0x1000),
            ("recall", 0x40000),
            ("OneDrive-work", 0x10),
            ("name.", 0),
            (".", 0x10),
            ("..", 0x10),
        ] {
            parse_buffer(
                &record(&name.encode_utf16().collect::<Vec<_>>(), attrs),
                &mut collector,
                &|| Ok(()),
            )
            .unwrap();
        }
        let entries = collector.finish();
        assert_eq!(entries.len(), 7);
        assert!(entries
            .iter()
            .all(|entry| entry.path == format!("C:/fixture/{}", entry.name)));
        assert_eq!(
            entries
                .iter()
                .find(|entry| entry.name == "link")
                .unwrap()
                .reason,
            Some("reparse_point")
        );
        assert_eq!(
            entries
                .iter()
                .find(|entry| entry.name == "OneDrive-work")
                .unwrap()
                .reason,
            Some("cloud_path")
        );
        assert_eq!(
            entries
                .iter()
                .find(|entry| entry.name == "name.")
                .unwrap()
                .reason,
            Some("unsupported_name")
        );
    }
    #[test]
    fn count_size_and_duplicate_are_explicit_errors_without_partial_success() {
        let mut collector = Collector::new("C:\\fixture", 1);
        collector.add("a".into(), 0).unwrap();
        assert_eq!(
            collector.add("b".into(), 0).unwrap_err().code,
            "directory_too_large"
        );
        let mut collector = Collector::new("C:\\fixture", 10);
        collector.add("a".into(), 0).unwrap();
        assert_eq!(
            collector.add("a".into(), 0).unwrap_err().code,
            "directory_changed"
        );
        let mut collector = Collector::new("C:\\fixture", 1024);
        let error = (0..1024)
            .find_map(|i| collector.add(format!("{i}{}", "中".repeat(100)), 0).err())
            .unwrap();
        assert_eq!(error.code, "directory_too_large");
    }
    #[test]
    fn malformed_record_and_utf16_never_become_lossy_names() {
        for bytes in [
            vec![],
            vec![0; 68],
            record(&[0xd800], 0),
            record(&[b'a' as u16, b'/' as u16, b'b' as u16], 0),
        ] {
            assert_eq!(
                parse_buffer(&bytes, &mut Collector::new("C:\\fixture", 10), &|| Ok(()))
                    .unwrap_err()
                    .code,
                "fs_error"
            );
        }
        let mut bytes = record(&[b'a' as u16], 0);
        bytes[0..4].copy_from_slice(&8u32.to_le_bytes());
        assert_eq!(
            parse_buffer(&bytes, &mut Collector::new("C:\\fixture", 10), &|| Ok(()))
                .unwrap_err()
                .code,
            "fs_error"
        );
    }
    #[test]
    fn decoder_checks_cancellation_before_each_record() {
        let error = parse_buffer(
            &record(&[b'a' as u16], 0),
            &mut Collector::new("C:\\fixture", 10),
            &|| Err(fault("cancelled")),
        )
        .unwrap_err();
        assert_eq!(error.code, "cancelled");
    }
    #[cfg(windows)]
    #[test]
    fn windows_record_layout_matches_checked_decoder() {
        use windows::Win32::Storage::FileSystem::FILE_FULL_DIR_INFO;
        assert_eq!(std::mem::offset_of!(FILE_FULL_DIR_INFO, FileName), 68);
        assert_eq!(std::mem::offset_of!(FILE_FULL_DIR_INFO, FileAttributes), 56);
        assert_eq!(std::mem::offset_of!(FILE_FULL_DIR_INFO, FileNameLength), 60);
    }
}
