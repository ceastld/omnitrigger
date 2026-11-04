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
    initial_sequence: u32,
    owner: HWND,
    data: Vec<(u32, Vec<u8>)>,
    last_owned: Arc<AtomicU32>,
    active: bool,
}
impl ClipboardBackup {
    fn new(owner: HWND) -> Result<Self, String> {
        let before = unsafe { GetClipboardSequenceNumber() };
        let data = snapshot(owner)?;
        let sequence = unsafe { GetClipboardSequenceNumber() };
        if sequence != before {
            return Err("备份期间剪贴板发生变化；未开始写入".into());
        }
        Ok(Self {
            initial_sequence: sequence,
            owner,
            data,
            last_owned: Arc::new(AtomicU32::new(sequence)),
            active: true,
        })
    }
    fn restore(&mut self) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        let current = unsafe { GetClipboardSequenceNumber() };
        if current == self.initial_sequence
            && self.last_owned.load(Ordering::Acquire) == self.initial_sequence
        {
            self.active = false;
            return Ok(()); // 只有读操作或提前拒绝时，不重写原剪贴板。
        }
        if current != self.last_owned.load(Ordering::Acquire) {
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

fn cli_run(
    exe: &std::path::Path,
    config: &std::path::Path,
    call: &std::path::Path,
    context: &std::path::Path,
) -> std::process::Output {
    use std::{
        os::windows::process::CommandExt,
        process::{Command, Stdio},
    };
    let spawn_began = Instant::now();
    let mut child = Command::new(exe)
        .args(["--config"])
        .arg(config)
        .arg("--run-action")
        .arg(call)
        .arg("--target-context")
        .arg(context)
        .creation_flags(0x08000000)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    eprintln!(
        "CLI child created in {} ms",
        spawn_began.elapsed().as_millis()
    );
    let began = Instant::now();
    loop {
        pump();
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if began.elapsed() > Duration::from_secs(20) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("自有目标 CLI 验收超时；已结束本次子进程，不重试");
        }
        thread::sleep(Duration::from_millis(2));
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
    if let Some(path) = std::env::var_os("OMNITRIGGER_TEXT_SMOKE_WAIT_FOR_FOCUS") {
        let _ = unsafe { ShowWindow(window.0, SW_SHOWNOACTIVATE) };
        std::fs::write(
            path,
            serde_json::to_vec(
                &json!({"pid":std::process::id(), "window":window.0.0 as usize,
            "title":"OmniTrigger 文本 API 实测", "input_sent":false}),
            )
            .unwrap(),
        )
        .unwrap();
        let began = Instant::now();
        while unsafe { GetForegroundWindow() } != window.0 {
            pump();
            assert!(
                began.elapsed() < Duration::from_secs(60),
                "等待自有输入框激活超时；没有提交输入"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }
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
    let saved_projection = std::env::var_os("OMNITRIGGER_LEGACY_TEXT_FIXTURE").map(|path| {
        let bytes = std::fs::read(path).unwrap();
        assert!(bytes.len() <= 128 * 1024);
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["simulate"], false);
        assert_eq!(value["simulate_delay_ms"], 0);
        value
    });
    if clipboard_cases {
        if let Some(value) = &saved_projection {
            let text = value["text"].as_str().unwrap();
            assert!(!text.is_empty());
            cases.push(("paste", "", json!({"text":text}), text));
        }
    }
    let cli_exe = std::env::var_os("OMNITRIGGER_CLI_SMOKE_EXE").map(std::path::PathBuf::from);
    let cli_root = cli_exe.as_ref().map(|_| {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.tmp")
            .join(format!(
                "target-context-cli-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        let package = root.join("actions/desktop-text");
        std::fs::create_dir_all(&package).unwrap();
        for name in [
            "action.json",
            "type.mjs",
            "paste.mjs",
            "wrap.mjs",
            "newline.mjs",
        ] {
            std::fs::copy(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../examples/actions/desktop-text")
                    .join(name),
                package.join(name),
            )
            .unwrap();
        }
        std::fs::write(root.join("bindings.json"), r#"{"version":1,"bindings":[]}"#).unwrap();
        root
    });
    let mut evidence = Vec::new();
    for (case_index, (action_id, before, input, expected)) in cases.into_iter().enumerate() {
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
        let snapshot = omnitrigger_windows::ScriptTarget::capture(target)
            .unwrap()
            .snapshot()
            .unwrap();
        let host = TrackedHost {
            inner: DesktopHost::from_snapshot(snapshot).unwrap(),
            sequence: backup.as_ref().map(|backup| backup.last_owned.clone()),
        };
        let call = ActionCall {
            package_id: "examples.desktop-text".into(),
            action_id: action_id.into(),
            action_version: 1,
            input,
        };
        if case_index == 0 {
            if let (Some(exe), Some(root)) = (&cli_exe, &cli_root) {
                let call_path = root.join("call.json");
                let target_path = root.join("target.json");
                std::fs::write(&call_path, serde_json::to_vec(&call).unwrap()).unwrap();
                let mut context = json!({"window_handle":snapshot.window_handle.to_string(),
                    "process_id":snapshot.process_id,"thread_id":snapshot.thread_id,
                    "process_created_file_time":snapshot.process_created_file_time.to_string(),
                    "expires_at_unix_ms":std::time::SystemTime::now().duration_since(std::time::SystemTime::UNIX_EPOCH).unwrap().as_millis() as u64+30_000});
                std::fs::write(&target_path, serde_json::to_vec(&context).unwrap()).unwrap();
                let output = cli_run(exe, &root.join("bindings.json"), &call_path, &target_path);
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                let result: Value = serde_json::from_slice(&output.stdout).unwrap();
                assert_eq!(result["status"], "succeeded");
                pump();
                assert_eq!(text(edit), expected);
                evidence.push(json!({"action":"CLI explicit target", "observed":text(edit)}));
                context["process_id"] = json!(snapshot.process_id + 1);
                context["expires_at_unix_ms"] = json!(
                    std::time::SystemTime::now()
                        .duration_since(std::time::SystemTime::UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as u64
                        + 30_000
                );
                std::fs::write(&target_path, serde_json::to_vec(&context).unwrap()).unwrap();
                let rejected = cli_run(exe, &root.join("bindings.json"), &call_path, &target_path);
                assert!(!rejected.status.success());
                let failure: Value = serde_json::from_slice(&rejected.stderr).unwrap();
                assert_eq!(failure["error"]["code"], "invalid_target_context");
                pump();
                assert_eq!(text(edit), expected, "错误目标不能回退到当前前台");
                evidence.push(json!({"action":"CLI rejects changed PID", "effect":"not_started"}));
                continue;
            }
        }
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
    let report = json!({"cases":evidence,"clipboard_cases":clipboard_cases,"restored_clipboard_formats":formats,"explicit_snapshot_hosts":true,"cli_tested":cli_exe.is_some(),"saved_legacy_projection_pasted":saved_projection.is_some() && clipboard_cases});
    if let Some(path) = std::env::var_os("OMNITRIGGER_TEXT_SMOKE_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    assert!(
        clipboard_cases,
        "剪贴板格式无法安全备份；文本输入已验证，未执行剪贴板用例"
    );
}
