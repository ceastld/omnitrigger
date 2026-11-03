//! 只由显式交互测试使用。剪贴板只在能完整备份受支持格式时参与测试。
use crate::desktop_host::DesktopHost;
use omnitrigger_actions::{
    ActionCall, ActionOutcome, ActionService, CancellationToken, NativeHost, NodeRuntime,
    PackageCatalog, RuntimeRegistry,
};
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{
            GetLastError, GlobalFree, SetLastError, HANDLE, HGLOBAL, HWND, LPARAM, WIN32_ERROR,
            WPARAM,
        },
        System::{
            DataExchange::{
                CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
                GetClipboardFormatNameW, GetClipboardSequenceNumber, OpenClipboard,
                SetClipboardData,
            },
            Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE},
        },
        UI::{
            Input::KeyboardAndMouse::SetFocus,
            WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, DispatchMessageW, GetForegroundWindow,
                GetWindowTextW, PeekMessageW, SendMessageW, SetForegroundWindow, SetWindowTextW,
                ShowWindow, TranslateMessage, MSG, PM_REMOVE, SW_SHOWNOACTIVATE, WINDOW_EX_STYLE,
                WINDOW_STYLE, WS_BORDER, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
            },
        },
    },
};

struct Lease;
impl Lease {
    fn open(owner: HWND) -> Result<Self, String> {
        for _ in 0..20 {
            if unsafe { OpenClipboard(Some(owner)) }.is_ok() {
                return Ok(Self);
            }
            thread::sleep(Duration::from_millis(10));
        }
        Err("测试无法打开剪贴板；未开始写入".into())
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        let _ = unsafe { CloseClipboard() };
    }
}

struct Allocation(Option<HGLOBAL>);
impl Allocation {
    fn new(bytes: &[u8]) -> Result<Self, String> {
        let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) }
            .map_err(|error| error.to_string())?;
        let allocation = Self(Some(memory));
        let pointer = unsafe { GlobalLock(memory) };
        if pointer.is_null() {
            return Err("无法锁定剪贴板备份内存".into());
        }
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast(), bytes.len());
            let _ = GlobalUnlock(memory);
        }
        Ok(allocation)
    }
}
impl Drop for Allocation {
    fn drop(&mut self) {
        if let Some(handle) = self.0 {
            let _ = unsafe { GlobalFree(Some(handle)) };
        }
    }
}

fn snapshot(owner: HWND) -> Result<Vec<(u32, Vec<u8>)>, String> {
    let _lease = Lease::open(owner)?;
    let mut formats = Vec::new();
    let mut format = 0;
    loop {
        unsafe {
            SetLastError(WIN32_ERROR(0));
        }
        format = unsafe { EnumClipboardFormats(format) };
        if format == 0 {
            if unsafe { GetLastError() }.0 != 0 {
                return Err("剪贴板格式枚举失败".into());
            }
            break;
        }
        if formats.len() >= 128 {
            return Err("剪贴板格式过多；未开始写入".into());
        }
        let mut name = [0u16; 256];
        let count = unsafe { GetClipboardFormatNameW(format, &mut name) }.max(0) as usize;
        let name = String::from_utf16_lossy(&name[..count]);
        let allowed = matches!(format, 1 | 7 | 8 | 13 | 15 | 16 | 17)
            || matches!(
                name.as_str(),
                "HTML Format" | "Rich Text Format" | "Chromium Web Custom MIME Data Format"
            );
        if !allowed {
            return Err(format!(
                "剪贴板含未支持的格式 {format}；未读取该格式或开始写入"
            ));
        }
        formats.push(format);
    }
    let mut total = 0;
    formats
        .into_iter()
        .map(|format| {
            let handle = unsafe { GetClipboardData(format) }.map_err(|error| error.to_string())?;
            let global = HGLOBAL(handle.0);
            let size = unsafe { GlobalSize(global) };
            total += size;
            if size == 0 || total > 16 * 1024 * 1024 {
                return Err("剪贴板格式不是可备份的有界内存".into());
            }
            let pointer = unsafe { GlobalLock(global) };
            if pointer.is_null() {
                return Err("剪贴板备份读取失败".into());
            }
            let bytes = unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), size).to_vec() };
            let _ = unsafe { GlobalUnlock(global) };
            Ok((format, bytes))
        })
        .collect()
}

struct ClipboardBackup {
    owner: HWND,
    data: Vec<(u32, Vec<u8>)>,
    last_owned: Arc<AtomicU32>,
    active: bool,
}
impl ClipboardBackup {
    fn new(owner: HWND) -> Result<Self, String> {
        let data = snapshot(owner)?;
        Ok(Self {
            owner,
            data,
            last_owned: Arc::new(AtomicU32::new(unsafe { GetClipboardSequenceNumber() })),
            active: true,
        })
    }
    fn restore(&mut self) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        if unsafe { GetClipboardSequenceNumber() } != self.last_owned.load(Ordering::Acquire) {
            self.active = false;
            return Err("测试期间剪贴板已被其他来源更新；保留当前内容，未恢复旧快照".into());
        }
        let mut allocations = self
            .data
            .iter()
            .map(|(_, bytes)| Allocation::new(bytes))
            .collect::<Result<Vec<_>, _>>()?;
        let lease = Lease::open(self.owner)?;
        if unsafe { GetClipboardSequenceNumber() } != self.last_owned.load(Ordering::Acquire) {
            self.active = false;
            return Err("打开剪贴板期间内容已变化；未覆盖当前内容".into());
        }
        unsafe { EmptyClipboard() }.map_err(|error| error.to_string())?;
        for ((format, _), allocation) in self.data.iter().zip(&mut allocations) {
            let handle = allocation.0.unwrap();
            unsafe { SetClipboardData(*format, Some(HANDLE(handle.0))) }
                .map_err(|error| error.to_string())?;
            allocation.0 = None;
        }
        drop(lease);
        self.active = false;
        let restored = snapshot(self.owner)?;
        for entry in &self.data {
            if !restored.contains(entry) {
                return Err("剪贴板备份读回不一致".into());
            }
        }
        Ok(())
    }
}
impl Drop for ClipboardBackup {
    fn drop(&mut self) {
        if let Err(error) = self.restore() {
            eprintln!("{error}");
        }
    }
}

struct TrackedHost {
    inner: DesktopHost,
    sequence: Option<Arc<AtomicU32>>,
}
impl NativeHost for TrackedHost {
    fn call(
        &self,
        method: &str,
        params: Value,
        cancel: &CancellationToken,
    ) -> Result<Value, omnitrigger_actions::ActionError> {
        let result = self.inner.call(method, params, cancel);
        if matches!(method, "clipboard.write" | "selection.copy") {
            if let Some(sequence) = &self.sequence {
                sequence.store(unsafe { GetClipboardSequenceNumber() }, Ordering::Release);
            }
        }
        result
    }
}

fn pump() {
    let mut message = MSG::default();
    while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
        let _ = unsafe { TranslateMessage(&message) };
        unsafe { DispatchMessageW(&message) };
    }
}
fn text(edit: HWND) -> String {
    let mut buffer = [0u16; 512];
    let count = unsafe { GetWindowTextW(edit, &mut buffer) }.max(0) as usize;
    String::from_utf16(&buffer[..count]).unwrap()
}

struct Running<'a>(&'a ActionService);
impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.0.cancel_all();
        let began = Instant::now();
        while self.0.is_running(1) && began.elapsed() < Duration::from_secs(5) {
            pump();
            let _ = self.0.try_recv();
            thread::sleep(Duration::from_millis(1));
        }
        pump();
    }
}

#[test]
#[ignore = "激活专用输入框；支持完整备份时验证真实剪贴板并恢复"]
fn interactive_desktop_text_smoke() {
    struct Window(HWND, HWND);
    impl Drop for Window {
        fn drop(&mut self) {
            let ours = unsafe { GetForegroundWindow() } == self.0;
            let _ = unsafe { DestroyWindow(self.0) };
            if ours && !self.1.is_invalid() {
                let _ = unsafe { SetForegroundWindow(self.1) };
            }
        }
    }
    let previous = unsafe { GetForegroundWindow() };
    let window = Window(
        unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("OmniTrigger 文本 API 实测"),
                WS_OVERLAPPEDWINDOW,
                100,
                100,
                520,
                220,
                None,
                None,
                None,
                None,
            )
        }
        .unwrap(),
        previous,
    );
    let edit = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("EDIT"),
            w!(""),
            WS_CHILD | WS_VISIBLE | WS_BORDER | WINDOW_STYLE(0x0004 | 0x0040 | 0x0080 | 0x1000),
            20,
            35,
            460,
            110,
            Some(window.0),
            None,
            None,
            None,
        )
    }
    .unwrap();
    let mut backup = ClipboardBackup::new(window.0).ok();
    let clipboard_cases = backup.is_some();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/actions");
    let mut registry = RuntimeRegistry::new();
    registry
        .register("node", Arc::new(NodeRuntime::default()))
        .unwrap();
    let service = ActionService::new(PackageCatalog::load(&root), registry);
    // 异常退出时先结束动作，再恢复剪贴板和关闭目标窗口。
    let _running = Running(&service);
    let mut cases = vec![
        ("type", "", json!({"text":"$$"}), "$$"),
        ("type", "", json!({"text":"$="}), "$="),
        ("type", "", json!({"text":"文😀\n测试"}), "文😀\r\n测试"),
        ("end-newline", "abc", json!({}), "abc\r\n"),
    ];
    if clipboard_cases {
        cases.extend([
            ("paste", "", json!({"text":"\\Leftarrow "}), "\\Leftarrow "),
            (
                "paste",
                "",
                json!({"text":"\\Rightarrow "}),
                "\\Rightarrow ",
            ),
            ("wrap-selection", "abc", json!({}), "(abc)"),
            ("wrap-selection", " \t ", json!({}), "( \t )"),
        ]);
    }
    let mut evidence = Vec::new();
    for (action_id, before, input, expected) in cases {
        let wide = before
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        unsafe { SetWindowTextW(edit, PCWSTR(wide.as_ptr())) }.unwrap();
        let _ = unsafe { ShowWindow(window.0, SW_SHOWNOACTIVATE) };
        let _ = unsafe { SetForegroundWindow(window.0) };
        assert_eq!(
            unsafe { GetForegroundWindow() },
            window.0,
            "未取得测试焦点，未提交输入"
        );
        let _ = unsafe { SetFocus(Some(edit)) };
        let (first, last) = if action_id == "wrap-selection" {
            (0, before.len())
        } else {
            (0, 0)
        };
        unsafe {
            SendMessageW(
                edit,
                0x00b1,
                Some(WPARAM(first)),
                Some(LPARAM(last as isize)),
            )
        };
        let target = omnitrigger_windows::capture_shortcut_target().unwrap();
        let host = TrackedHost {
            inner: DesktopHost::capture(Some(target)),
            sequence: backup.as_ref().map(|backup| backup.last_owned.clone()),
        };
        let call = ActionCall {
            package_id: "examples.desktop-text".into(),
            action_id: action_id.into(),
            action_version: 1,
            input,
        };
        service
            .submit_with_host(1, call, Some(Arc::new(host)))
            .unwrap();
        let began = Instant::now();
        let outcome = loop {
            pump();
            if unsafe { GetForegroundWindow() } != window.0 {
                service.cancel_all();
                panic!("测试失焦，已取消后续输入");
            }
            if let Ok(event) = service.try_recv() {
                if event.outcome.is_terminal() {
                    break event.outcome;
                }
            }
            if began.elapsed() > Duration::from_secs(10) {
                service.cancel_all();
                panic!("文本 API 实测超时");
            }
            thread::sleep(Duration::from_millis(1));
        };
        pump();
        assert!(
            matches!(outcome, ActionOutcome::Succeeded { .. }),
            "{outcome:?}"
        );
        assert_eq!(text(edit), expected);
        if clipboard_cases && matches!(action_id, "paste" | "wrap-selection") {
            assert_eq!(
                omnitrigger_windows::read_clipboard_text().unwrap(),
                expected,
                "粘贴结果应留在剪贴板"
            );
        }
        evidence.push(json!({"action":action_id,"expected":expected,"observed":text(edit)}));
    }
    let formats = backup.as_ref().map(|backup| backup.data.len());
    if let Some(backup) = &mut backup {
        backup.restore().unwrap();
    }
    let report = json!({"cases":evidence,"clipboard_cases":clipboard_cases,"restored_clipboard_formats":formats});
    if let Some(path) = std::env::var_os("OMNITRIGGER_TEXT_SMOKE_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    assert!(
        clipboard_cases,
        "剪贴板格式无法安全备份；文本输入已验证，未执行剪贴板用例"
    );
}
