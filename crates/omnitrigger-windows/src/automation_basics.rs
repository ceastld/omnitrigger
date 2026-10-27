//! 基础自动化的 Windows 适配。外壳、剪贴板和输入操作只能由动作工作线程调用。
//! 文本输入不借用剪贴板，不抢焦点；部分提交不能回滚为原应用状态。

const MAX_TEXT_UNITS: usize = 64 * 1024;
const MAX_TARGET_UNITS: usize = 32_766;

/// 打开目标只接受显式白名单协议或盘符开头的本地绝对路径，不拼接命令行。
pub fn validate_open_target(target: &str) -> Result<(), String> {
    if target.is_empty()
        || target.len() > MAX_TARGET_UNITS * 4
        || target.encode_utf16().take(MAX_TARGET_UNITS + 1).count() > MAX_TARGET_UNITS
    {
        return Err("打开目标不能为空，且不能超过 32,766 个 UTF-16 单元。".into());
    }
    if target.chars().any(char::is_control) || target.trim() != target {
        return Err("打开目标不能包含控制字符或首尾空白。".into());
    }
    let bytes = target.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
    {
        if target[2..]
            .chars()
            .any(|ch| matches!(ch, ':' | '"' | '<' | '>' | '|' | '*' | '?'))
        {
            return Err("本地路径包含不支持的 Windows 路径字符。".into());
        }
        return Ok(());
    }
    let (scheme, value) = target
        .split_once(':')
        .ok_or("请输入 http、https、mailto、about 地址或本地绝对路径。")?;
    if value.is_empty()
        || target.chars().any(char::is_whitespace)
        || target.contains(['\\', '"', '<', '>'])
    {
        return Err("地址内容不能为空，空白、引号和反斜杠等字符需要先正确编码。".into());
    }
    match scheme.to_ascii_lowercase().as_str() {
        "http" | "https" => {
            let authority = value
                .strip_prefix("//")
                .and_then(|rest| rest.split(['/', '?', '#']).next())
                .filter(|host| !host.is_empty())
                .ok_or("网页地址需要 // 和非空主机名。")?;
            if authority.starts_with(':') || authority.ends_with('@') {
                return Err("网页地址主机名无效。".into());
            }
            Ok(())
        }
        "mailto" | "about" => Ok(()),
        _ => Err("仅支持 http、https、mailto、about 和本地绝对路径；其他协议不会自动执行。".into()),
    }
}

/// 文本上限按 UTF-16 单元计算；空文本允许用于清空文本剪贴板。
pub fn validate_automation_text(text: &str) -> Result<(), String> {
    if text.len() > MAX_TEXT_UNITS * 4
        || text.encode_utf16().take(MAX_TEXT_UNITS + 1).count() > MAX_TEXT_UNITS
    {
        return Err("文本不能超过 65,536 个 UTF-16 单元。".into());
    }
    if text.contains('\0') {
        return Err("文本不能包含 NUL 字符。".into());
    }
    Ok(())
}

#[cfg(any(windows, test))]
mod text_input {
    use super::validate_automation_text;

    const BATCH_EVENTS: usize = 64;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) struct UnicodeStroke {
        pub scan: u16,
        pub up: bool,
    }

    /// 保持一个 Unicode 标量的代理对位于同批，CRLF/LF 各发送一次回车字符。
    pub(super) fn plan(text: &str) -> Result<Vec<Vec<UnicodeStroke>>, String> {
        validate_automation_text(text)?;
        let mut batches = Vec::new();
        let mut batch = Vec::with_capacity(BATCH_EVENTS);
        let mut chars = text.chars().peekable();
        while let Some(mut ch) = chars.next() {
            if ch == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            } else if ch == '\n' {
                ch = '\r';
            }
            let mut encoded = [0; 2];
            let units = ch.encode_utf16(&mut encoded);
            if batch.len() + units.len() * 2 > BATCH_EVENTS {
                batches.push(std::mem::replace(
                    &mut batch,
                    Vec::with_capacity(BATCH_EVENTS),
                ));
            }
            for &scan in units.iter() {
                batch.push(UnicodeStroke { scan, up: false });
                batch.push(UnicodeStroke { scan, up: true });
            }
        }
        if !batch.is_empty() {
            batches.push(batch);
        }
        Ok(batches)
    }

    pub(super) fn cleanup(strokes: &[UnicodeStroke], sent: usize) -> Vec<UnicodeStroke> {
        let mut held = Vec::new();
        for stroke in strokes.iter().take(sent) {
            if stroke.up {
                if let Some(index) = held.iter().rposition(|scan| *scan == stroke.scan) {
                    held.remove(index);
                }
            } else {
                held.push(stroke.scan);
            }
        }
        held.into_iter()
            .rev()
            .map(|scan| UnicodeStroke { scan, up: true })
            .collect()
    }

    pub(super) trait Input {
        fn guard(&mut self) -> Result<(), String>;
        fn send(&mut self, strokes: &[UnicodeStroke]) -> usize;
    }

    pub(super) fn send(text: &str, input: &mut impl Input) -> Result<(), String> {
        let batches = plan(text)?;
        let total: usize = batches.iter().map(Vec::len).sum();
        let mut submitted = 0;
        if batches.is_empty() {
            return input.guard();
        }
        for batch in batches {
            input
                .guard()
                .map_err(|error| format!("{error} 已提交 {submitted}/{total} 个文本事件。"))?;
            let sent = input.send(&batch).min(batch.len());
            submitted += sent;
            if sent != batch.len() {
                let releases = cleanup(&batch, sent);
                let mut released = 0;
                for _ in 0..5 {
                    if released == releases.len() {
                        break;
                    }
                    released += input
                        .send(&releases[released..])
                        .min(releases.len() - released);
                }
                let tail = if released == releases.len() {
                    "已尝试收尾本次未完成的 Unicode 按下。"
                } else {
                    "无法确认本次 Unicode 按下已释放；请停止后续输入并检查目标应用。"
                };
                return Err(format!("文本仅提交 {submitted}/{total} 个事件；{tail} 目标可能受权限隔离或输入拦截影响，已提交文本不会自动撤回。"));
            }
            input.guard().map_err(|error| {
                format!("{error} 已提交 {submitted}/{total} 个文本事件，目标应用结果需要检查。")
            })?;
        }
        Ok(())
    }
}

#[cfg(any(windows, test))]
mod clipboard {
    use super::{validate_automation_text, MAX_TEXT_UNITS};

    const OPEN_ATTEMPTS: usize = 8;

    pub(super) trait Backend {
        type Handle: Copy;
        fn allocate(&self, units: &[u16]) -> Result<Self::Handle, String>;
        fn free(&self, handle: Self::Handle);
        fn open(&self) -> Result<(), String>;
        fn close(&self) -> Result<(), String>;
        fn empty(&self) -> Result<(), String>;
        fn set_unicode(&self, handle: Self::Handle) -> Result<(), String>;
        fn read_unicode(&self) -> Result<Vec<u16>, String>;
        fn wait_retry(&self) {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    struct Lease<'a, B: Backend> {
        backend: &'a B,
        active: bool,
    }
    impl<'a, B: Backend> Lease<'a, B> {
        fn open(backend: &'a B) -> Result<Self, String> {
            let mut last_error = String::new();
            for attempt in 0..OPEN_ATTEMPTS {
                match backend.open() {
                    Ok(()) => {
                        return Ok(Self {
                            backend,
                            active: true,
                        })
                    }
                    Err(error) => last_error = error,
                }
                if attempt + 1 < OPEN_ATTEMPTS {
                    backend.wait_retry();
                }
            }
            Err(format!(
                "剪贴板可能正被其他程序占用，重试 {OPEN_ATTEMPTS} 次后仍无法打开：{last_error}"
            ))
        }
        fn close(mut self) -> Result<(), String> {
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

    struct OwnedMemory<'a, B: Backend> {
        backend: &'a B,
        handle: Option<B::Handle>,
    }
    impl<B: Backend> Drop for OwnedMemory<'_, B> {
        fn drop(&mut self) {
            if let Some(handle) = self.handle.take() {
                self.backend.free(handle);
            }
        }
    }

    pub(super) fn write(text: &str, backend: &impl Backend) -> Result<(), String> {
        validate_automation_text(text)?;
        let units: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let handle = backend.allocate(&units)?;
        let mut memory = OwnedMemory {
            backend,
            handle: Some(handle),
        };
        let lease = Lease::open(backend)?;
        backend
            .empty()
            .map_err(|error| format!("无法清空剪贴板：{error}"))?;
        backend.set_unicode(handle).map_err(|error| {
            format!("剪贴板已清空，但文本写入失败；无法保证恢复原内容：{error}")
        })?;
        // SetClipboardData 成功之后，内存由系统拥有，应用绝不再释放或修改。
        memory.handle = None;
        lease
            .close()
            .map_err(|error| format!("文本已写入，但关闭剪贴板失败：{error}"))
    }

    pub(super) fn decode(units: &[u16]) -> Result<String, String> {
        let end = units
            .iter()
            .position(|unit| *unit == 0)
            .ok_or("剪贴板 Unicode 文本缺少终止符，或超过 65,536 个 UTF-16 单元。")?;
        if end > MAX_TEXT_UNITS {
            return Err("剪贴板文本超过 65,536 个 UTF-16 单元。".into());
        }
        String::from_utf16(&units[..end]).map_err(|_| "剪贴板包含无效的 UTF-16 文本。".into())
    }

    pub(super) fn read(backend: &impl Backend) -> Result<String, String> {
        let lease = Lease::open(backend)?;
        let units = backend.read_unicode()?;
        lease
            .close()
            .map_err(|error| format!("读取文本后关闭剪贴板失败：{error}"))?;
        decode(&units)
    }
}

#[cfg(windows)]
mod native {
    use super::{clipboard, text_input, MAX_TEXT_UNITS};
    use crate::shortcut::{ensure_shortcut_target, ShortcutTarget, MODIFIERS, SHORTCUT_TAG};
    use std::mem::size_of;
    use windows::{
        core::{w, PCWSTR},
        Win32::{
            Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND},
            System::{
                DataExchange::{
                    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable,
                    OpenClipboard, SetClipboardData,
                },
                Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE},
                Ole::CF_UNICODETEXT,
            },
            UI::{
                Input::KeyboardAndMouse::{
                    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
                    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
                },
                WindowsAndMessaging::{
                    CreateWindowExW, DestroyWindow, HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE,
                },
            },
        },
    };

    pub(super) struct TextInput(pub ShortcutTarget);
    impl text_input::Input for TextInput {
        fn guard(&mut self) -> Result<(), String> {
            ensure_shortcut_target(self.0)?;
            if MODIFIERS
                .iter()
                .any(|key| unsafe { GetAsyncKeyState(i32::from(*key)) } < 0)
            {
                return Err(
                    "修饰键仍被按住，已取消发送文本；请松开 Ctrl、Shift、Alt、Win 后重试。".into(),
                );
            }
            Ok(())
        }
        fn send(&mut self, strokes: &[text_input::UnicodeStroke]) -> usize {
            let inputs: Vec<INPUT> = strokes
                .iter()
                .map(|stroke| INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wScan: stroke.scan,
                            dwFlags: if stroke.up {
                                KEYEVENTF_UNICODE | KEYEVENTF_KEYUP
                            } else {
                                KEYEVENTF_UNICODE
                            },
                            dwExtraInfo: SHORTCUT_TAG,
                            ..Default::default()
                        },
                    },
                })
                .collect();
            (unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) }) as usize
        }
    }

    pub(super) struct Clipboard {
        owner: Option<HWND>,
    }
    impl Clipboard {
        pub fn new(write: bool) -> Result<Self, String> {
            let owner = if write {
                Some(
                    unsafe {
                        CreateWindowExW(
                            WINDOW_EX_STYLE(0),
                            w!("STATIC"),
                            w!("OmniTrigger Clipboard"),
                            WINDOW_STYLE(0),
                            0,
                            0,
                            0,
                            0,
                            Some(HWND_MESSAGE),
                            None,
                            None,
                            None,
                        )
                    }
                    .map_err(|error| format!("无法创建剪贴板所有者窗口：{error}"))?,
                )
            } else {
                None
            };
            Ok(Self { owner })
        }
    }
    impl Drop for Clipboard {
        fn drop(&mut self) {
            if let Some(owner) = self.owner {
                let _ = unsafe { DestroyWindow(owner) };
            }
        }
    }

    struct GlobalLockGuard(HGLOBAL);
    impl Drop for GlobalLockGuard {
        fn drop(&mut self) {
            // GlobalUnlock 返回零也可能表示锁计数已正确归零，不能据此断言失败。
            let _ = unsafe { GlobalUnlock(self.0) };
        }
    }

    impl clipboard::Backend for Clipboard {
        type Handle = HGLOBAL;
        fn allocate(&self, units: &[u16]) -> Result<HGLOBAL, String> {
            let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, std::mem::size_of_val(units)) }
                .map_err(|error| format!("无法分配剪贴板文本内存：{error}"))?;
            let pointer = unsafe { GlobalLock(memory) };
            if pointer.is_null() {
                let error = windows::core::Error::from_win32();
                let _ = unsafe { GlobalFree(Some(memory)) };
                return Err(format!("无法锁定剪贴板文本内存：{error}"));
            }
            let _lock = GlobalLockGuard(memory);
            unsafe { std::ptr::copy_nonoverlapping(units.as_ptr(), pointer.cast(), units.len()) };
            Ok(memory)
        }
        fn free(&self, handle: HGLOBAL) {
            let _ = unsafe { GlobalFree(Some(handle)) };
        }
        fn open(&self) -> Result<(), String> {
            unsafe { OpenClipboard(self.owner) }.map_err(|error| error.to_string())
        }
        fn close(&self) -> Result<(), String> {
            unsafe { CloseClipboard() }.map_err(|error| error.to_string())
        }
        fn empty(&self) -> Result<(), String> {
            unsafe { EmptyClipboard() }.map_err(|error| error.to_string())
        }
        fn set_unicode(&self, handle: HGLOBAL) -> Result<(), String> {
            unsafe { SetClipboardData(u32::from(CF_UNICODETEXT.0), Some(HANDLE(handle.0))) }
                .map(|_| ())
                .map_err(|error| error.to_string())
        }
        fn read_unicode(&self) -> Result<Vec<u16>, String> {
            unsafe { IsClipboardFormatAvailable(u32::from(CF_UNICODETEXT.0)) }
                .map_err(|_| "剪贴板没有可读取的 Unicode 文本。".to_string())?;
            let handle = unsafe { GetClipboardData(u32::from(CF_UNICODETEXT.0)) }
                .map_err(|error| format!("无法读取剪贴板 Unicode 文本：{error}"))?;
            let memory = HGLOBAL(handle.0);
            let bytes = unsafe { GlobalSize(memory) };
            if bytes < 2 || bytes % 2 != 0 {
                return Err("剪贴板 Unicode 文本的内存大小无效。".into());
            }
            let pointer = unsafe { GlobalLock(memory) };
            if pointer.is_null() {
                return Err(format!(
                    "无法锁定剪贴板 Unicode 文本：{}",
                    windows::core::Error::from_win32()
                ));
            }
            let _lock = GlobalLockGuard(memory);
            // 只查看受限 Unicode 文本片段，不读取其他剪贴板格式或完整超大缓冲区。
            let count = (bytes / 2).min(MAX_TEXT_UNITS + 1);
            let units = unsafe { std::slice::from_raw_parts(pointer.cast::<u16>(), count) };
            Ok(units.to_vec())
        }
    }

    pub(super) fn open(target: &str) -> Result<(), String> {
        use windows::Win32::{
            Foundation::RPC_E_CHANGED_MODE,
            System::Com::{
                CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
            },
            UI::{
                Shell::{
                    ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW,
                },
                WindowsAndMessaging::SW_SHOWNORMAL,
            },
        };
        let status =
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        if status.is_err() && status != RPC_E_CHANGED_MODE {
            return Err(format!("无法初始化系统打开接口：{status}"));
        }
        struct ComGuard(bool);
        impl Drop for ComGuard {
            fn drop(&mut self) {
                if self.0 {
                    unsafe { CoUninitialize() };
                }
            }
        }
        let _com = ComGuard(status.is_ok());
        let wide: Vec<u16> = target.encode_utf16().chain(std::iter::once(0)).collect();
        let mut info = SHELLEXECUTEINFOW {
            cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC,
            lpVerb: w!("open"),
            lpFile: PCWSTR(wide.as_ptr()),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        // lpParameters 为空；目标字符串绝不作为 shell 命令或参数列表解释。
        unsafe { ShellExecuteExW(&mut info) }.map_err(|error| format!("系统未能打开目标：{error}"))
    }
}

/// 用系统默认处理程序打开地址或本地绝对路径；仅工作线程调用。
/// 成功表示系统接纳打开请求，不代表外部应用已经完成加载。
#[cfg(windows)]
pub fn open_target(target: &str) -> Result<(), String> {
    validate_open_target(target)?;
    native::open(target)
}

/// 替换剪贴板内容为 Unicode 文本；仅工作线程调用。失败时原内容可能已经清空。
#[cfg(windows)]
pub fn write_clipboard_text(text: &str) -> Result<(), String> {
    validate_automation_text(text)?;
    clipboard::write(text, &native::Clipboard::new(true)?)
}

/// 只读取 CF_UNICODETEXT，拒绝无效/超限 UTF-16；仅工作线程调用。
#[cfg(windows)]
pub fn read_clipboard_text() -> Result<String, String> {
    clipboard::read(&native::Clipboard::new(false)?)
}

/// 向原前台窗口分批发送 Unicode 文本，换行规范为一次回车字符。
/// 仅工作线程调用；拒绝按住的修饰键，不更改焦点，不使用或覆盖剪贴板。
/// 权限隔离、前台切换或输入拦截可能导致部分提交，收尾不等于撤销文本。
#[cfg(windows)]
pub fn send_text_to(text: &str, target: crate::ShortcutTarget) -> Result<(), String> {
    let _guard = crate::shortcut::SENDING
        .lock()
        .map_err(|_| "文本发送状态不可用。")?;
    text_input::send(text, &mut native::TextInput(target))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        collections::VecDeque,
    };

    #[test]
    fn open_target_whitelist_preserves_local_spaces_without_shell_parsing() {
        for target in [
            "https://example.com/a?q=one%20two",
            "HTTP://example.com",
            "mailto:user@example.com",
            "about:blank",
            r"C:\Program Files\a&b.txt",
            "d:/folder/",
        ] {
            assert!(validate_open_target(target).is_ok(), "{target}");
        }
        for target in [
            "",
            " ",
            "https://",
            "http:example.com",
            "https:///x",
            "https://:80",
            "https://user@",
            "https://example.com/\"argument\"",
            "https://x/\n",
            "custom-app:run?id=1",
            "javascript:alert(1)",
            "file:///c:/x",
            "C:relative",
            r"\rooted",
            r"\\server\share",
            r"\\?\C:\x",
            "../x",
            " C:/x",
            "C:/x\0",
            "C:/a:stream",
        ] {
            assert!(validate_open_target(target).is_err(), "{target:?}");
        }
        assert!(
            validate_open_target(&format!("https://x/{}", "a".repeat(MAX_TARGET_UNITS))).is_err()
        );
    }

    #[test]
    fn text_limit_counts_utf16_units_and_rejects_nul() {
        assert!(validate_automation_text("").is_ok());
        assert!(validate_automation_text(&"😀".repeat(MAX_TEXT_UNITS / 2)).is_ok());
        assert!(validate_automation_text(&"😀".repeat(MAX_TEXT_UNITS / 2 + 1)).is_err());
        assert!(validate_automation_text("a\0b").is_err());
    }

    #[test]
    fn unicode_plan_pairs_surrogates_and_normalizes_each_newline_once() {
        let batches = text_input::plan("A😀\r\nB\nC\rD").unwrap();
        let strokes: Vec<_> = batches.iter().flatten().copied().collect();
        let expected: Vec<_> = "A😀\rB\rC\rD".encode_utf16().collect();
        assert_eq!(strokes.len(), expected.len() * 2);
        for (pair, scan) in strokes.chunks_exact(2).zip(expected) {
            assert_eq!(
                pair,
                [
                    text_input::UnicodeStroke { scan, up: false },
                    text_input::UnicodeStroke { scan, up: true }
                ]
            );
        }
        let batches = text_input::plan(&format!("{}😀z", "a".repeat(31))).unwrap();
        assert_eq!(batches[0].len(), 62);
        assert_eq!(batches[1].len(), 6);
        assert!(batches.iter().all(
            |batch| batch.len() <= 64 && !matches!(batch.last().unwrap().scan, 0xd800..=0xdbff)
        ));
    }

    #[test]
    fn every_partial_unicode_prefix_cleans_only_its_unfinished_press() {
        let batch = &text_input::plan("😀aa").unwrap()[0];
        for sent in 0..=batch.len() {
            let releases = text_input::cleanup(batch, sent);
            assert_eq!(releases.len(), sent % 2);
            assert!(releases.iter().all(|stroke| stroke.up));
            if sent % 2 == 1 {
                assert_eq!(releases[0].scan, batch[sent - 1].scan);
            }
        }
        assert!(text_input::cleanup(batch, usize::MAX).is_empty());
    }

    #[derive(Default)]
    struct MockInput {
        sends: Vec<Vec<text_input::UnicodeStroke>>,
        accepted: VecDeque<usize>,
        guards: usize,
        fail_guard: Option<usize>,
    }
    impl text_input::Input for MockInput {
        fn guard(&mut self) -> Result<(), String> {
            self.guards += 1;
            if self.fail_guard == Some(self.guards) {
                Err("mock foreground changed".into())
            } else {
                Ok(())
            }
        }
        fn send(&mut self, strokes: &[text_input::UnicodeStroke]) -> usize {
            self.sends.push(strokes.to_vec());
            self.accepted.pop_front().unwrap_or(strokes.len())
        }
    }

    #[test]
    fn partial_text_send_releases_only_owned_scan_without_retyping() {
        let mut input = MockInput {
            accepted: VecDeque::from([3, 0, 1]),
            ..Default::default()
        };
        let error = text_input::send("ab", &mut input).unwrap_err();
        assert!(error.contains("3/4"));
        assert_eq!(input.sends.len(), 3);
        assert_eq!(
            input.sends[1],
            [text_input::UnicodeStroke {
                scan: b'b'.into(),
                up: true
            }]
        );
        assert_eq!(input.sends[1], input.sends[2]);
        let mut input = MockInput {
            accepted: VecDeque::from([1, 0, 0, 0, 0, 0]),
            ..Default::default()
        };
        assert!(text_input::send("a", &mut input)
            .unwrap_err()
            .contains("无法确认"));
        assert_eq!(input.sends.len(), 6);
    }

    #[test]
    fn text_guard_prevents_initial_or_later_batches_without_cleanup_user_keys() {
        let mut input = MockInput {
            fail_guard: Some(1),
            ..Default::default()
        };
        assert!(text_input::send("a", &mut input).is_err());
        assert!(input.sends.is_empty());
        let mut input = MockInput {
            fail_guard: Some(3),
            ..Default::default()
        };
        assert!(text_input::send(&"a".repeat(64), &mut input).is_err());
        assert_eq!(input.sends.len(), 1);
        assert!(input.sends[0].last().unwrap().up);
    }

    #[derive(Default)]
    struct MockClipboard {
        log: RefCell<Vec<&'static str>>,
        fail: Option<&'static str>,
        busy: Cell<usize>,
        data: RefCell<Vec<u16>>,
    }
    impl MockClipboard {
        fn step(&self, name: &'static str) -> Result<(), String> {
            self.log.borrow_mut().push(name);
            if self.fail == Some(name) {
                Err(format!("mock {name}"))
            } else {
                Ok(())
            }
        }
    }
    impl clipboard::Backend for MockClipboard {
        type Handle = usize;
        fn allocate(&self, units: &[u16]) -> Result<usize, String> {
            self.step("allocate")?;
            *self.data.borrow_mut() = units.to_vec();
            Ok(7)
        }
        fn free(&self, handle: usize) {
            assert_eq!(handle, 7);
            self.log.borrow_mut().push("free");
        }
        fn open(&self) -> Result<(), String> {
            self.step("open")?;
            let busy = self.busy.get();
            if busy > 0 {
                self.busy.set(busy - 1);
                Err("busy".into())
            } else {
                Ok(())
            }
        }
        fn close(&self) -> Result<(), String> {
            self.step("close")
        }
        fn empty(&self) -> Result<(), String> {
            self.step("empty")
        }
        fn set_unicode(&self, handle: usize) -> Result<(), String> {
            assert_eq!(handle, 7);
            self.step("set")
        }
        fn read_unicode(&self) -> Result<Vec<u16>, String> {
            self.step("read_unicode")?;
            Ok(self.data.borrow().clone())
        }
        fn wait_retry(&self) {
            self.log.borrow_mut().push("wait");
        }
    }

    #[test]
    fn clipboard_success_transfers_memory_and_closes_once() {
        let backend = MockClipboard::default();
        clipboard::write("中文😀", &backend).unwrap();
        assert_eq!(
            *backend.log.borrow(),
            ["allocate", "open", "empty", "set", "close"]
        );
        assert_eq!(clipboard::decode(&backend.data.borrow()).unwrap(), "中文😀");
        let backend = MockClipboard {
            fail: Some("close"),
            ..Default::default()
        };
        assert!(clipboard::write("x", &backend)
            .unwrap_err()
            .contains("文本已写入"));
        assert!(!backend.log.borrow().contains(&"free"));
    }

    #[test]
    fn clipboard_failures_free_only_untransferred_memory_and_close_open_lease() {
        for failed in ["allocate", "empty", "set"] {
            let backend = MockClipboard {
                fail: Some(failed),
                ..Default::default()
            };
            assert!(clipboard::write("x", &backend).is_err());
            let log = backend.log.borrow();
            assert_eq!(
                log.iter().filter(|step| **step == "free").count(),
                usize::from(failed != "allocate")
            );
            assert_eq!(
                log.iter().filter(|step| **step == "close").count(),
                usize::from(failed != "allocate")
            );
        }
        let backend = MockClipboard {
            busy: Cell::new(100),
            ..Default::default()
        };
        assert!(clipboard::write("x", &backend)
            .unwrap_err()
            .contains("重试 8 次"));
        let log = backend.log.borrow();
        assert_eq!(log.iter().filter(|step| **step == "open").count(), 8);
        assert_eq!(log.iter().filter(|step| **step == "wait").count(), 7);
        assert!(!log.contains(&"close"));
        assert!(!log.contains(&"empty"));
        assert_eq!(log.last(), Some(&"free"));
    }

    #[test]
    fn clipboard_reads_only_unicode_and_validates_bounds_and_surrogates() {
        let backend = MockClipboard {
            busy: Cell::new(2),
            data: RefCell::new(vec![b'x'.into(), 0]),
            ..Default::default()
        };
        assert_eq!(clipboard::read(&backend).unwrap(), "x");
        assert_eq!(
            *backend.log.borrow(),
            [
                "open",
                "wait",
                "open",
                "wait",
                "open",
                "read_unicode",
                "close"
            ]
        );
        let backend = MockClipboard {
            fail: Some("read_unicode"),
            ..Default::default()
        };
        assert!(clipboard::read(&backend).is_err());
        assert_eq!(backend.log.borrow().last(), Some(&"close"));
        assert!(clipboard::decode(&[0xd800, 0]).is_err());
        assert!(clipboard::decode(&[65]).is_err());
        assert!(clipboard::decode(&vec![65; MAX_TEXT_UNITS + 1]).is_err());
        let mut maximum = vec![65; MAX_TEXT_UNITS];
        maximum.push(0);
        assert_eq!(clipboard::decode(&maximum).unwrap().len(), MAX_TEXT_UNITS);
        maximum.insert(0, 65);
        assert!(clipboard::decode(&maximum).is_err());
    }
}
