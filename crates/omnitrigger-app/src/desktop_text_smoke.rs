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
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{
            CloseHandle, GetLastError, GlobalFree, SetLastError, HANDLE, HGLOBAL, HWND, LPARAM,
            WAIT_OBJECT_0, WIN32_ERROR, WPARAM,
        },
        System::{
            DataExchange::{
                CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
                GetClipboardFormatNameW, GetClipboardSequenceNumber, OpenClipboard,
                RegisterClipboardFormatW, SetClipboardData,
            },
            Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE},
            Threading::{
                GetExitCodeProcess, OpenProcess, WaitForSingleObject, PROCESS_ACCESS_RIGHTS,
                PROCESS_QUERY_LIMITED_INFORMATION,
            },
        },
        UI::{
            Input::KeyboardAndMouse::SetFocus,
            WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, DispatchMessageW, GetForegroundWindow,
                GetWindowTextW, PeekMessageW, SendMessageW, SetForegroundWindow, SetWindowTextW,
                ShowWindow, TranslateMessage, HWND_MESSAGE, MSG, PM_REMOVE, SW_SHOWNOACTIVATE,
                WINDOW_EX_STYLE, WINDOW_STYLE, WS_BORDER, WS_CHILD, WS_OVERLAPPEDWINDOW,
                WS_VISIBLE,
            },
        },
    },
};

// 启动后保留内核句柄，避免退出报告先于实际退出或 PID 重用导致误判。
struct HostProcess(HANDLE);
impl Drop for HostProcess {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

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
        // 排除元数据只保留其语义；恢复时规范化为版本1，不读取旧标记载荷。
        if crate::clipboard_restore::EXCLUSION_FORMATS.contains(&name.as_str()) {
            continue;
        }
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
    last_write_id: Arc<Mutex<Option<String>>>,
    active: bool,
    report_path: Option<std::path::PathBuf>,
}

use crate::private_window_station as private_clipboard;

#[test]
#[ignore = "需要可创建命名窗口站的 Windows 管理员令牌；CI 单独串行运行，绝不使用交互剪贴板"]
fn isolated_clipboard_restore_smoke() {
    let station = private_clipboard::PrivateClipboard::create();
    station.verify();
    let owner = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!("OmniTrigger Clipboard Restore Test"),
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
    .unwrap();
    struct Window(HWND);
    impl Drop for Window {
        fn drop(&mut self) {
            let _ = unsafe { DestroyWindow(self.0) };
        }
    }
    let _window = Window(owner);
    // 合成富文本与 Unicode；恢复路径必须保留原始字节并规范化排除元数据。
    let mut plain = Allocation::new(&[65, 0, 66, 0, 0, 0]).unwrap();
    let html_format = unsafe { RegisterClipboardFormatW(w!("HTML Format")) };
    assert_ne!(html_format, 0);
    let html_bytes = b"Version:1.0\r\n<html><b>owned</b></html>\0";
    let mut html = Allocation::new(html_bytes).unwrap();
    {
        let _lease = Lease::open(owner).unwrap();
        unsafe { EmptyClipboard() }.unwrap();
        unsafe { SetClipboardData(13, Some(HANDLE(plain.0.unwrap().0))) }.unwrap();
        plain.0 = None;
        unsafe { SetClipboardData(html_format, Some(HANDLE(html.0.unwrap().0))) }.unwrap();
        html.0 = None;
    }
    let mut backup = ClipboardBackup::new(owner).unwrap();
    let restore_report = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../.tmp/clipboard-restore-native-{}.json",
        std::process::id()
    ));
    backup.report_path = Some(restore_report.clone());
    let expected = backup.data.clone();
    // 确认普通快照原本没有排除元数据。
    let backend = RestoreNative(owner);
    let marker_ids: Vec<u32> = crate::clipboard_restore::EXCLUSION_FORMATS
        .iter()
        .map(|name| crate::clipboard_restore::Backend::register(&backend, name).unwrap())
        .collect();
    assert!(marker_ids.iter().all(|format| unsafe {
        windows::Win32::System::DataExchange::IsClipboardFormatAvailable(*format)
    }
    .is_err()));
    station.verify();
    let receipt = DesktopHost::capture(None)
        .call(
            "clipboard.write",
            json!({"text":"temporary","exclude_from_history":true}),
            &CancellationToken::new(),
        )
        .unwrap();
    let write_id = receipt["writeId"].as_str().unwrap().to_owned();
    assert_eq!(write_id.len(), 32);
    *backup.last_write_id.lock().unwrap() = Some(write_id.clone());
    backup.last_owned.store(
        receipt["sequenceNumber"].as_u64().unwrap() as u32,
        Ordering::Release,
    );
    backup.restore().unwrap();
    let report: Value = serde_json::from_slice(&std::fs::read(&restore_report).unwrap()).unwrap();
    assert_eq!(report["status"], "restored_and_verified");
    assert_eq!(report["exclusion_formats"], 2);
    std::fs::remove_file(restore_report).unwrap();
    station.verify();
    let restored = snapshot(owner).unwrap();
    assert_eq!(
        restored, expected,
        "原载荷格式及字节必须相同，排除元数据另算"
    );
    {
        let _lease = Lease::open(owner).unwrap();
        for format in marker_ids {
            crate::clipboard_restore::Backend::verify(&backend, format, &1u32.to_le_bytes())
                .unwrap();
        }
    }
    assert_eq!(omnitrigger_windows::read_clipboard_text().unwrap(), "AB");
    // 后续来源更新必须被保留，不能在 Drop 中再次覆盖。
    let mut stale = ClipboardBackup::new(owner).unwrap();
    let newer_id =
        omnitrigger_windows::write_clipboard_text_excluding_history_if("newer", || Ok(())).unwrap();
    assert_ne!(newer_id, write_id);
    // 模拟外层观察到其它写入方的新序号，标识仍必须阻止覆盖。
    stale
        .last_owned
        .store(unsafe { GetClipboardSequenceNumber() }, Ordering::Release);
    *stale.last_write_id.lock().unwrap() = Some(write_id);
    assert!(stale.restore().is_err());
    assert!(!stale.active);
    drop(stale);
    assert_eq!(omnitrigger_windows::read_clipboard_text().unwrap(), "newer");
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
            last_write_id: Arc::new(Mutex::new(None)),
            active: true,
            report_path: None,
        })
    }
    fn restore(&mut self) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        let no_write = unsafe { GetClipboardSequenceNumber() } == self.initial_sequence
            && self.last_owned.load(Ordering::Acquire) == self.initial_sequence;
        let result = self.restore_once();
        if let Some(path) = &self.report_path {
            let status = if result.is_err() {
                "failed_or_refused"
            } else if no_write {
                "unchanged_no_write"
            } else {
                "restored_and_verified"
            };
            let report = json!({"status":status,"payload_formats":self.data.len(),
                "exclusion_formats":if result.is_ok() && !no_write {2}else{0},
                "exact_format_set_verified":false,"error":result.as_ref().err()});
            std::fs::write(
                path,
                serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?,
            )
            .map_err(|error| format!("恢复已结束，但收据写入失败：{error}"))?;
        }
        result
    }
    fn restore_once(&mut self) -> Result<(), String> {
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
        // 本次恢复只有一次尝试。失败后 Drop 不得再盲重试破坏性写入。
        self.active = false;
        let write_id = self
            .last_write_id
            .lock()
            .map_err(|_| "写入标识锁已失效；未恢复")?
            .clone()
            .ok_or("没有本次临时写入标识；未恢复")?;
        crate::clipboard_restore::restore(
            &self.data,
            self.last_owned.load(Ordering::Acquire),
            &write_id,
            &RestoreNative(self.owner),
        )
    }
}

struct RestoreNative(HWND);
impl crate::clipboard_restore::Backend for RestoreNative {
    type Memory = Allocation;
    fn register(&self, name: &str) -> Result<u32, String> {
        let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
        let format = unsafe { RegisterClipboardFormatW(PCWSTR(wide.as_ptr())) };
        if format == 0 {
            Err("无法注册恢复所需的排除格式；未开始写入".into())
        } else {
            Ok(format)
        }
    }
    fn allocate(&self, bytes: &[u8]) -> Result<Allocation, String> {
        Allocation::new(bytes)
    }
    fn open(&self) -> Result<(), String> {
        let lease = Lease::open(self.0)?;
        // 由恢复算法的 Lease 负责关闭；不让临时打开句柄提前关闭。
        std::mem::forget(lease);
        Ok(())
    }
    fn close(&self) -> Result<(), String> {
        unsafe { CloseClipboard() }.map_err(|error| error.to_string())
    }
    fn sequence(&self) -> u32 {
        unsafe { GetClipboardSequenceNumber() }
    }
    fn matches_write(&self, token: &str) -> Result<bool, String> {
        let format = <Self as crate::clipboard_restore::Backend>::register(
            self,
            crate::clipboard_restore::EXCLUSION_FORMATS[0],
        )?;
        let mut expected = 1u32.to_le_bytes().to_vec();
        expected.extend(token.encode_utf16().chain([0]).flat_map(u16::to_le_bytes));
        Ok(<Self as crate::clipboard_restore::Backend>::verify(self, format, &expected).is_ok())
    }
    fn empty(&self) -> Result<(), String> {
        unsafe { EmptyClipboard() }.map_err(|error| error.to_string())
    }
    fn set(&self, format: u32, allocation: &mut Allocation) -> Result<(), String> {
        let handle = allocation.0.unwrap();
        unsafe { SetClipboardData(format, Some(HANDLE(handle.0))) }
            .map_err(|error| error.to_string())?;
        allocation.0 = None;
        Ok(())
    }
    fn verify(&self, format: u32, expected: &[u8]) -> Result<(), String> {
        let handle = unsafe { GetClipboardData(format) }.map_err(|error| error.to_string())?;
        let global = HGLOBAL(handle.0);
        let size = unsafe { GlobalSize(global) };
        if size < expected.len() {
            return Err("剪贴板恢复读回长度不符".into());
        }
        let pointer = unsafe { GlobalLock(global) };
        if pointer.is_null() {
            return Err("剪贴板恢复读回锁定失败".into());
        }
        let matches =
            unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), expected.len()) } == expected;
        let _ = unsafe { GlobalUnlock(global) };
        if matches {
            Ok(())
        } else {
            Err("剪贴板恢复载荷读回不符".into())
        }
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
    write_id: Option<Arc<Mutex<Option<String>>>>,
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
            if let Some(write_id) = &self.write_id {
                *write_id.lock().unwrap() = result
                    .as_ref()
                    .ok()
                    .and_then(|receipt| receipt["writeId"].as_str())
                    .map(str::to_owned);
            }
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
fn reject_held_clipboard_smoke(path: &std::path::Path) -> Result<(), String> {
    if path.try_exists().map_err(|error| error.to_string())? {
        return Err("采集归属尚未隔离：禁止重放会改写系统剪贴板的手工实测。".into());
    }
    Ok(())
}

fn native_clipboard_smoke_preflight(profile: Option<&std::path::Path>) -> Result<(), String> {
    reject_held_clipboard_smoke(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.tmp/native-clipboard-test-hold.marker"),
    )?;
    if let Some(profile) = profile {
        reject_held_clipboard_smoke(&profile.join("native-clipboard-tests-disabled.marker"))?;
    }
    Ok(())
}

#[test]
fn held_clipboard_fixture_is_rejected_without_native_effects() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../.tmp/clipboard-hold-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let marker = root.join("hold");
    assert!(reject_held_clipboard_smoke(&marker).is_ok());
    std::fs::write(&marker, "source collector not isolated").unwrap();
    assert!(reject_held_clipboard_smoke(&marker).is_err());
    std::fs::remove_file(marker).unwrap();
    std::fs::remove_dir(root).unwrap();
}

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
    native_clipboard_smoke_preflight(None).unwrap();
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
            write_id: backup.as_ref().map(|backup| backup.last_write_id.clone()),
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

/// 由 Cua 在两个 ready 阶段点击真实菜单；仅操作本测试创建的输入框。
#[test]
#[ignore = "需要精确激活自有窗口并点击独立宿主的原菜单，核验成功及目标关闭错误"]
fn interactive_legacy_menu_click_smoke() {
    legacy_host_click_smoke("saved-text-execute");
}

#[test]
#[ignore = "需要精确激活自有窗口并点击独立宿主的真实剪贴板条目"]
fn interactive_legacy_clipboard_click_smoke() {
    legacy_host_click_smoke("clipboard-text-execute");
}

fn legacy_host_click_smoke(scenario: &str) {
    use std::path::PathBuf;
    let root = PathBuf::from(
        std::env::var_os("OMNITRIGGER_MENU_HOST_ROOT").expect("需要隔离的保存配置目录"),
    );
    let status_path = PathBuf::from(std::env::var_os("OMNITRIGGER_MENU_SMOKE_STATUS").unwrap());
    let host_exe = std::env::var("OMNITRIGGER_MENU_HOST_EXE").unwrap();
    let omni_exe = std::env::var("OMNITRIGGER_CLI_SMOKE_EXE").unwrap();
    // 通用 hold 保留；只有固定的已保护 Legacy 夹具可凭短时核验记录运行。
    crate::clipboard_smoke_protection::preflight(
        &root,
        scenario,
        std::path::Path::new(&omni_exe),
        std::path::Path::new(&host_exe),
    )
    .unwrap();
    struct Owned {
        windows: Vec<HWND>,
        previous: HWND,
        root: PathBuf,
    }
    impl Drop for Owned {
        fn drop(&mut self) {
            let _ = std::fs::write(self.root.join("cancel-requested"), "cancel");
            let restore = self.windows.contains(&unsafe { GetForegroundWindow() });
            for hwnd in self.windows.iter().rev() {
                let _ = unsafe { DestroyWindow(*hwnd) };
            }
            if restore {
                let _ = unsafe { SetForegroundWindow(self.previous) };
            }
        }
    }
    fn create(title: &str, initial: &str) -> (HWND, HWND) {
        // 普通编辑器会在重新激活时恢复子控件焦点；STATIC 默认过程没有这项行为。
        unsafe extern "system" fn fixture_proc(
            hwnd: HWND,
            message: u32,
            wparam: WPARAM,
            lparam: LPARAM,
        ) -> windows::Win32::Foundation::LRESULT {
            use windows::Win32::UI::WindowsAndMessaging::{
                DefWindowProcW, GetWindowLongPtrW, GWLP_USERDATA, WM_SETFOCUS,
            };
            if message == WM_SETFOCUS {
                let edit = HWND(unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut _);
                if !edit.is_invalid() {
                    let _ = unsafe { SetFocus(Some(edit)) };
                    return windows::Win32::Foundation::LRESULT(0);
                }
            }
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        let wide = title.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                PCWSTR(wide.as_ptr()),
                WS_OVERLAPPEDWINDOW,
                120,
                120,
                540,
                240,
                None,
                None,
                None,
                None,
            )
        }
        .unwrap();
        let wide = initial.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let edit = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("EDIT"),
                PCWSTR(wide.as_ptr()),
                WS_CHILD | WS_VISIBLE | WS_BORDER | WINDOW_STYLE(0x0004 | 0x0040 | 0x0080 | 0x1000),
                20,
                35,
                480,
                120,
                Some(hwnd),
                None,
                None,
                None,
            )
        }
        .unwrap();
        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::{
                SetWindowLongPtrW, GWLP_USERDATA, GWLP_WNDPROC,
            };
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, edit.0 as isize);
            assert_ne!(
                SetWindowLongPtrW(hwnd, GWLP_WNDPROC, fixture_proc as *const () as isize),
                0
            );
        }
        (hwnd, edit)
    }
    let mut owned = Owned {
        windows: vec![],
        previous: unsafe { GetForegroundWindow() },
        root: root.clone(),
    };
    let (window, edit) = create("OmniTrigger 文本 API 实测", "");
    owned.windows.push(window);
    let (decoy, decoy_edit) = create("OmniTrigger 失效目标验证", "unchanged");
    owned.windows.push(decoy);
    let _ = unsafe { ShowWindow(window, SW_SHOWNOACTIVATE) };
    let write_status = |stage: &str, extra: Value| {
        std::fs::write(
            &status_path,
            serde_json::to_vec(&json!({"stage":stage,"pid":std::process::id(),
        "window":window.0 as usize,"decoy":decoy.0 as usize,"root":root,"extra":extra}))
            .unwrap(),
        )
        .unwrap()
    };
    write_status("waiting_focus", json!({}));
    let began = Instant::now();
    while unsafe { GetForegroundWindow() } != window {
        pump();
        assert!(
            began.elapsed() < Duration::from_secs(60),
            "没有激活自有目标，未开始执行"
        );
        thread::sleep(Duration::from_millis(20));
    }
    let _ = unsafe { SetFocus(Some(edit)) };
    // 原目标将在失效用例中关闭；剪贴板租约使用一直存活到恢复之后的自有窗口。
    let mut backup = ClipboardBackup::new(decoy).expect("无法完整备份当前格式；未开始写入");
    backup.report_path = Some(root.join("clipboard-restore-result.json"));
    let package = root.join("native-gateway/actions/legacy-menu");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        root.join("native-gateway/bindings.json"),
        r#"{"version":1,"bindings":[]}"#,
    )
    .unwrap();
    std::fs::write(package.join("action.json"),serde_json::to_vec(&json!({"schema_version":1,"id":"legacy.menu-text","name":"Owned menu gateway","version":"1.0.0","actions":[
        {"id":"launch","title":"launch","version":1,"runtime":"node","entry":"launch.mjs"},
        {"id":"paste","title":"paste","version":1,"runtime":"node","entry":"paste.mjs"}]})).unwrap()).unwrap();
    std::fs::write(package.join("launch.mjs"),r#"import fs from 'node:fs/promises';
export default async ctx => {
  const target = await ctx.desktop.targetContext();
  await fs.writeFile(ctx.input.target_path, JSON.stringify(target), {mode:0o600});
  await fs.writeFile(ctx.input.root+'/host-launch-request.json',JSON.stringify({nodePid:process.pid,startedAt:Date.now()}));
  const child = await ctx.process.start(ctx.input.host_exe, ['--data-root',ctx.input.root,'--command','clip none','--scenario',ctx.input.scenario,'--exit-after-ms',String(ctx.input.host_lifetime_ms)]);
  await fs.writeFile(ctx.input.root+'/host-spawned.json',JSON.stringify({pid:child.pid,startedAt:Date.now()}));
  return {pid:child.pid,target};
};"#).unwrap();
    std::fs::write(package.join("paste.mjs"),r#"import fs from 'node:fs/promises';
export default async ctx => {
  const receipt = await ctx.clipboard.writeText(ctx.input.text, {excludeFromHistory:true});
  if (!Number.isInteger(receipt.sequenceNumber) || receipt.sequenceNumber===0) throw new Error('clipboard receipt missing');
  if (typeof receipt.writeId !== 'string' || !/^[0-9a-f]{32}$/.test(receipt.writeId)) throw new Error('clipboard ownership receipt missing');
  await fs.writeFile(ctx.input.phase_path, JSON.stringify(receipt), {mode:0o600});
  await ctx.delay(ctx.input.delay_ms);
  await ctx.desktop.sendShortcut([162,86]);
  return {inputSubmitted:true,clipboardSequenceNumber:receipt.sequenceNumber};
};"#).unwrap();
    std::fs::write(root.join("gateway-config.json"),serde_json::to_vec(&json!({"omnitrigger_exe":omni_exe,
        "config_path":root.join("native-gateway/bindings.json"),"expected_owner_pid":std::process::id()})).unwrap()).unwrap();
    let target = omnitrigger_windows::ScriptTarget::capture(
        omnitrigger_windows::capture_shortcut_target().unwrap(),
    )
    .unwrap()
    .snapshot()
    .unwrap();
    let mut registry = RuntimeRegistry::new();
    registry
        .register("node", Arc::new(NodeRuntime::default()))
        .unwrap();
    let service = ActionService::new(
        PackageCatalog::load(&root.join("native-gateway/actions")),
        registry,
    );
    let _running = Running(&service);
    service.submit_with_host(1,ActionCall{package_id:"legacy.menu-text".into(),action_id:"launch".into(),action_version:1,
        input:json!({"target_path":root.join("launch-target.json"),"host_exe":host_exe,"root":root,"scenario":scenario,
            "host_lifetime_ms":if scenario=="clipboard-text-execute" {300000}else{60000}})},
        Some(Arc::new(DesktopHost::from_snapshot(target).unwrap()))).unwrap();
    let began = Instant::now();
    let launch = loop {
        pump();
        if let Ok(event) = service.try_recv() {
            if let ActionOutcome::Succeeded { output } = event.outcome {
                break output;
            } else if event.outcome.is_terminal() {
                panic!("{event:?}");
            }
        }
        assert!(began.elapsed() < Duration::from_secs(15));
        thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(
        launch["target"]["window_handle"],
        target.window_handle.to_string()
    );
    std::fs::write(
        root.join("host-launch-result.json"),
        serde_json::to_vec_pretty(&launch).unwrap(),
    )
    .unwrap();
    let host_pid = u32::try_from(launch["pid"].as_u64().expect("宿主 PID 缺失")).unwrap();
    let host_process = HostProcess(unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_ACCESS_RIGHTS(0x0010_0000),
            false,
            host_pid,
        )
        .expect("无法保留本轮宿主进程句柄")
    });
    let mut destroyed = false;
    let began = Instant::now();
    let result = loop {
        pump();
        if let Ok(bytes) = std::fs::read(root.join("native-clipboard-phase.json")) {
            if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
                if let Some(sequence) = value["sequenceNumber"].as_u64() {
                    let write_id = value["writeId"].as_str().expect("临时写入必须带有标识");
                    assert!(
                        write_id.len() == 32
                            && write_id.bytes().all(|byte| byte.is_ascii_hexdigit())
                    );
                    *backup.last_write_id.lock().unwrap() = Some(write_id.to_owned());
                    backup
                        .last_owned
                        .store(u32::try_from(sequence).unwrap(), Ordering::Release);
                }
            }
        }
        if root.join("startup-error.log").exists() {
            panic!(
                "{}",
                std::fs::read_to_string(root.join("startup-error.log")).unwrap()
            );
        }
        if root.join("menu-click-ready-1.json").exists()
            && !root.join("first-click-complete.json").exists()
        {
            let ready: Value = serde_json::from_slice(
                &std::fs::read(root.join("menu-click-ready-1.json")).unwrap(),
            )
            .unwrap();
            write_status("menu_1", ready);
        }
        if root.join("first-click-complete.json").exists()
            && !root.join("first-effect-verified").exists()
        {
            let expected: Value =
                serde_json::from_slice(&std::fs::read(root.join("expected-text.json")).unwrap())
                    .unwrap();
            let expected = expected["text"].as_str().unwrap();
            let effect_limit = Instant::now();
            while text(edit) != expected && effect_limit.elapsed() < Duration::from_secs(3) {
                pump();
                thread::sleep(Duration::from_millis(10));
            }
            let mut info = windows::Win32::UI::WindowsAndMessaging::GUITHREADINFO {
                cbSize: std::mem::size_of::<windows::Win32::UI::WindowsAndMessaging::GUITHREADINFO>(
                ) as u32,
                ..Default::default()
            };
            let _ = unsafe {
                windows::Win32::UI::WindowsAndMessaging::GetGUIThreadInfo(
                    target.thread_id,
                    &mut info,
                )
            };
            std::fs::write(root.join("first-effect-diagnostic.json"),serde_json::to_vec(&json!({"foreground":unsafe{GetForegroundWindow()}.0 as usize,"focus":info.hwndFocus.0 as usize,"expected_edit":edit.0 as usize,"observed_length":text(edit).len()})).unwrap()).unwrap();
            assert_eq!(text(edit), expected, "真实菜单执行后的文本不一致");
            std::fs::write(root.join("first-effect-verified"), "verified").unwrap();
        }
        if root.join("menu-click-ready-2.json").exists() && !destroyed {
            unsafe { DestroyWindow(window) }.unwrap();
            destroyed = true;
            let _ = unsafe { ShowWindow(decoy, SW_SHOWNOACTIVATE) };
            let _ = unsafe { SetForegroundWindow(decoy) };
        }
        if destroyed {
            let ready: Value = serde_json::from_slice(
                &std::fs::read(root.join("menu-click-ready-2.json")).unwrap(),
            )
            .unwrap();
            write_status("menu_2_target_closed", ready);
        }
        if root.join("scenario.json").exists() {
            break serde_json::from_slice::<Value>(
                &std::fs::read(root.join("scenario.json")).unwrap(),
            )
            .unwrap();
        }
        assert!(
            began.elapsed()
                < Duration::from_secs(if scenario == "clipboard-text-execute" {
                    240
                } else {
                    65
                }),
            "原菜单验证未完成，不重复执行"
        );
        thread::sleep(Duration::from_millis(20));
    };
    assert!(result["passed"].as_u64().unwrap() >= 9);
    assert!(destroyed);
    assert_eq!(text(decoy_edit), "unchanged");
    write_status("error_visible", result.clone());
    let until = Instant::now();
    while !root.join("operator-finish").exists()
        && until.elapsed()
            < Duration::from_secs(if scenario == "clipboard-text-execute" {
                30
            } else {
                12
            })
    {
        pump();
        thread::sleep(Duration::from_millis(20));
    }
    let formats = backup.data.len();
    backup.restore().unwrap();
    std::fs::write(root.join("finish-test"), "finish").unwrap();
    let until = Instant::now();
    while !root.join("exit.json").exists() && until.elapsed() < Duration::from_secs(5) {
        pump();
        thread::sleep(Duration::from_millis(20));
    }
    let exit: Value =
        serde_json::from_slice(&std::fs::read(root.join("exit.json")).unwrap()).unwrap();
    assert_eq!(exit["exitCode"], 0);
    let until = Instant::now();
    while unsafe { WaitForSingleObject(host_process.0, 0) } != WAIT_OBJECT_0 {
        assert!(
            until.elapsed() < Duration::from_secs(10),
            "宿主写出退出报告但进程仍未结束"
        );
        pump();
        thread::sleep(Duration::from_millis(20));
    }
    let mut host_exit_code = u32::MAX;
    unsafe { GetExitCodeProcess(host_process.0, &mut host_exit_code) }.unwrap();
    assert_eq!(host_exit_code, 0);
    std::fs::write(root.join("outer-native-result.json"),serde_json::to_vec_pretty(&json!({"passed":true,"launch":launch,
        "menu_result":result,"decoy_unchanged":true,"restored_clipboard_formats":formats,
        "restored_payload_formats":formats,"restore_exclusion_formats":2,"exact_format_set_verified":false,"host_exit":exit,"host_process_exit_verified":true})).unwrap()).unwrap();
    write_status("complete", json!({"restored_clipboard_formats":formats}));
}
