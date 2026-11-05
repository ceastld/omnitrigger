//! 显式启动的纯文本历史观察器。通知只唤醒容量为一的队列；正文读取不在窗口过程内。
//!
//! Windows 读取在专用辅助进程中进行，避免延迟渲染阻止通知线程退出。
//! 调用方须在应用最早入口分派 `--omni-clipboard-history-capture`，调用
//! [`run_clipboard_history_capture_helper`] 后立即退出，不能先启动 UI 或其他线程。
//! 本模块不自行启动监听、不落盘、不写剪贴板、不发送输入。
#![cfg(any(windows, test))]

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

const MAX_UNITS: usize = 65_536;
const OUTPUT_CAPACITY: usize = 16;
const MAGIC: &[u8; 8] = b"OTHIST01";
const HEADER: usize = 17;
const MAX_FRAME: usize = HEADER + MAX_UNITS * 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryListenerError {
    InvalidOptions,
    StartFailed,
    WindowFailed,
    WorkerFailed,
    ScopeMismatch,
    CaptureFailed,
    CaptureTimedOut,
    CaptureCancelled,
    ClipboardBusy,
    ClipboardChanged,
    InvalidText,
    TextTooLarge,
    InvalidReply,
}

impl HistoryListenerError {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidOptions => "history_listener_invalid_options",
            Self::StartFailed => "history_listener_start_failed",
            Self::WindowFailed => "history_listener_window_failed",
            Self::WorkerFailed => "history_listener_worker_failed",
            Self::ScopeMismatch => "history_listener_scope_mismatch",
            Self::CaptureFailed => "history_listener_capture_failed",
            Self::CaptureTimedOut => "history_listener_capture_timeout",
            Self::CaptureCancelled => "history_listener_cancelled",
            Self::ClipboardBusy => "history_listener_clipboard_busy",
            Self::ClipboardChanged => "history_listener_clipboard_changed",
            Self::InvalidText => "history_listener_invalid_text",
            Self::TextTooLarge => "history_listener_text_too_large",
            Self::InvalidReply => "history_listener_invalid_reply",
        }
    }
}

impl fmt::Display for HistoryListenerError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output.write_str(self.code())
    }
}

impl std::error::Error for HistoryListenerError {}

/// 正文仅供历史存储接线使用。Debug 不输出正文；序号不表示来源身份。
#[derive(Clone, PartialEq, Eq)]
pub struct HistoryText {
    pub session: u64,
    pub sequence_number: u32,
    pub text: String,
}

impl fmt::Debug for HistoryText {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output
            .debug_struct("HistoryText")
            .field("session", &self.session)
            .field("sequence_number", &self.sequence_number)
            .field("utf16_units", &self.text.encode_utf16().count())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Skip {
    Duplicate = 1,
    Transient = 2,
    Excluded = 3,
    Files = 4,
    Image = 5,
    Unsupported = 6,
}

enum Content {
    Text(String),
    Skip(Skip),
}

struct Observation {
    sequence: u32,
    content: Content,
}

#[derive(Default)]
struct Counters {
    notifications: AtomicU64,
    coalesced: AtomicU64,
    captures: AtomicU64,
    skipped: AtomicU64,
    delivered: AtomicU64,
    dropped: AtomicU64,
    failed: AtomicU64,
    timeouts: AtomicU64,
    helpers_started: AtomicU64,
    helpers_finished: AtomicU64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HistoryListenerStats {
    pub notifications: u64,
    pub coalesced: u64,
    pub captures: u64,
    pub skipped: u64,
    pub delivered: u64,
    pub dropped: u64,
    pub failed: u64,
    pub timeouts: u64,
    pub helpers_started: u64,
    pub helpers_finished: u64,
}

impl Counters {
    fn snapshot(&self) -> HistoryListenerStats {
        HistoryListenerStats {
            notifications: self.notifications.load(Ordering::Relaxed),
            coalesced: self.coalesced.load(Ordering::Relaxed),
            captures: self.captures.load(Ordering::Relaxed),
            skipped: self.skipped.load(Ordering::Relaxed),
            delivered: self.delivered.load(Ordering::Relaxed),
            dropped: self.dropped.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
            timeouts: self.timeouts.load(Ordering::Relaxed),
            helpers_started: self.helpers_started.load(Ordering::Relaxed),
            helpers_finished: self.helpers_finished.load(Ordering::Relaxed),
        }
    }
}

struct Signal {
    stop: AtomicBool,
    wake: mpsc::SyncSender<()>,
    counters: Counters,
}

impl Signal {
    fn notify(&self) {
        if self.stop.load(Ordering::Acquire) {
            return;
        }
        self.counters.notifications.fetch_add(1, Ordering::Relaxed);
        if matches!(self.wake.try_send(()), Err(mpsc::TrySendError::Full(()))) {
            self.counters.coalesced.fetch_add(1, Ordering::Relaxed);
        }
    }
}

trait CaptureReader {
    /// 必须响应 stop；生产实现通过终止本次读取进程解除延迟渲染等待。
    fn capture(
        &mut self,
        previous_sequence: u32,
        stop: &AtomicBool,
    ) -> Result<Observation, HistoryListenerError>;
}

#[derive(Clone, Copy, Default)]
struct Formats {
    transient: bool,
    excluded: bool,
    files: bool,
    image: bool,
    unicode: bool,
}

trait TextBackend {
    fn open(&mut self) -> Result<(), HistoryListenerError>;
    fn close(&mut self) -> Result<(), HistoryListenerError>;
    fn sequence(&mut self) -> u32;
    fn formats(&mut self) -> Formats;
    fn text(&mut self) -> Result<String, HistoryListenerError>;
}

fn capture_with(
    backend: &mut impl TextBackend,
    previous: u32,
) -> Result<Observation, HistoryListenerError> {
    struct Lease<'a, B: TextBackend> {
        backend: &'a mut B,
        active: bool,
    }
    impl<B: TextBackend> Drop for Lease<'_, B> {
        fn drop(&mut self) {
            if self.active {
                let _ = self.backend.close();
            }
        }
    }
    backend.open()?;
    let mut lease = Lease {
        backend,
        active: true,
    };
    let result = (|| {
        let sequence = lease.backend.sequence();
        if sequence == previous {
            return Ok(Observation {
                sequence,
                content: Content::Skip(Skip::Duplicate),
            });
        }
        let formats = lease.backend.formats();
        let reason = if formats.transient {
            Some(Skip::Transient)
        } else if formats.excluded {
            Some(Skip::Excluded)
        } else if formats.files {
            Some(Skip::Files)
        } else if formats.image {
            Some(Skip::Image)
        } else if !formats.unicode {
            Some(Skip::Unsupported)
        } else {
            None
        };
        if let Some(reason) = reason {
            return Ok(Observation {
                sequence,
                content: Content::Skip(reason),
            });
        }
        let text = lease.backend.text()?;
        valid_text(&text)?;
        if lease.backend.sequence() != sequence {
            return Err(HistoryListenerError::ClipboardChanged);
        }
        Ok(Observation {
            sequence,
            content: Content::Text(text),
        })
    })();
    lease.active = false;
    let closed = lease.backend.close();
    result.and_then(|value| {
        closed?;
        Ok(value)
    })
}

fn valid_text(text: &str) -> Result<(), HistoryListenerError> {
    if text.contains('\0') {
        return Err(HistoryListenerError::InvalidText);
    }
    if text.encode_utf16().count() > MAX_UNITS {
        return Err(HistoryListenerError::TextTooLarge);
    }
    Ok(())
}

fn worker(
    mut reader: impl CaptureReader,
    signal: &Signal,
    wakes: mpsc::Receiver<()>,
    output: mpsc::SyncSender<HistoryText>,
    session: u64,
    mut previous: u32,
) {
    struct StopOnExit<'a>(&'a Signal);
    impl Drop for StopOnExit<'_> {
        fn drop(&mut self) {
            self.0.stop.store(true, Ordering::Release);
        }
    }
    let _exit = StopOnExit(signal);
    while !signal.stop.load(Ordering::Acquire) {
        match wakes.recv_timeout(Duration::from_millis(20)) {
            Ok(()) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if signal.stop.load(Ordering::Acquire) {
            break;
        }
        signal.counters.captures.fetch_add(1, Ordering::Relaxed);
        let observation = reader.capture(previous, &signal.stop);
        if signal.stop.load(Ordering::Acquire) {
            break;
        }
        let observation = match observation {
            Ok(value) => value,
            Err(error) => {
                signal.counters.failed.fetch_add(1, Ordering::Relaxed);
                if error == HistoryListenerError::CaptureTimedOut {
                    signal.counters.timeouts.fetch_add(1, Ordering::Relaxed);
                }
                continue;
            }
        };
        let duplicate = previous == observation.sequence;
        previous = observation.sequence;
        let Content::Text(text) = observation.content else {
            signal.counters.skipped.fetch_add(1, Ordering::Relaxed);
            continue;
        };
        if duplicate {
            signal.counters.skipped.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        if valid_text(&text).is_err() {
            signal.counters.failed.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        match output.try_send(HistoryText {
            session,
            sequence_number: observation.sequence,
            text,
        }) {
            Ok(()) => {
                signal.counters.delivered.fetch_add(1, Ordering::Relaxed);
            }
            Err(mpsc::TrySendError::Full(_)) => {
                signal.counters.dropped.fetch_add(1, Ordering::Relaxed);
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                signal.stop.store(true, Ordering::Release);
                break;
            }
        }
    }
}

fn encode_reply(result: Result<Observation, HistoryListenerError>) -> Vec<u8> {
    let (sequence, tag, units): (u32, u8, Vec<u16>) = match result {
        Ok(Observation {
            sequence,
            content: Content::Text(text),
        }) => {
            if let Err(error) = valid_text(&text) {
                return encode_reply(Err(error));
            }
            (sequence, 0, text.encode_utf16().collect())
        }
        Ok(Observation {
            sequence,
            content: Content::Skip(reason),
        }) => (sequence, reason as u8, vec![]),
        Err(error) => (
            0,
            match error {
                HistoryListenerError::ScopeMismatch => 128,
                HistoryListenerError::ClipboardBusy => 129,
                HistoryListenerError::ClipboardChanged => 130,
                HistoryListenerError::InvalidText => 131,
                HistoryListenerError::TextTooLarge => 132,
                _ => 133,
            },
            vec![],
        ),
    };
    let mut bytes = Vec::with_capacity(HEADER + units.len() * 2);
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&sequence.to_le_bytes());
    bytes.push(tag);
    bytes.extend_from_slice(&(units.len() as u32).to_le_bytes());
    for unit in units {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
}

fn decode_reply(bytes: &[u8]) -> Result<Observation, HistoryListenerError> {
    if bytes.len() < HEADER || bytes.len() > MAX_FRAME || &bytes[..8] != MAGIC {
        return Err(HistoryListenerError::InvalidReply);
    }
    let sequence = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    let count = u32::from_le_bytes(bytes[13..17].try_into().unwrap()) as usize;
    if count > MAX_UNITS || bytes.len() != HEADER + count * 2 {
        return Err(HistoryListenerError::InvalidReply);
    }
    let content = match bytes[12] {
        0 => {
            let units: Vec<_> = bytes[HEADER..]
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            let text = String::from_utf16(&units).map_err(|_| HistoryListenerError::InvalidText)?;
            valid_text(&text)?;
            Content::Text(text)
        }
        tag if count == 0 => Content::Skip(match tag {
            1 => Skip::Duplicate,
            2 => Skip::Transient,
            3 => Skip::Excluded,
            4 => Skip::Files,
            5 => Skip::Image,
            6 => Skip::Unsupported,
            128 => return Err(HistoryListenerError::ScopeMismatch),
            129 => return Err(HistoryListenerError::ClipboardBusy),
            130 => return Err(HistoryListenerError::ClipboardChanged),
            131 => return Err(HistoryListenerError::InvalidText),
            132 => return Err(HistoryListenerError::TextTooLarge),
            133 => return Err(HistoryListenerError::CaptureFailed),
            _ => return Err(HistoryListenerError::InvalidReply),
        }),
        _ => return Err(HistoryListenerError::InvalidReply),
    };
    Ok(Observation { sequence, content })
}

#[cfg(windows)]
pub use native::{
    run_clipboard_history_capture_helper, run_clipboard_history_gated_capture_helper,
    ClipboardHistoryListener,
};

#[cfg(windows)]
mod native {
    use super::*;
    use std::ffi::c_void;
    use std::io::{Read, Write};
    use std::os::windows::{io::AsRawHandle, process::CommandExt};
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::thread::{self, JoinHandle};
    use std::time::Instant;
    use windows::core::{w, PCWSTR};
    use windows::Win32::{
        Foundation::{HGLOBAL, HWND, LPARAM, LRESULT, WPARAM},
        System::{
            DataExchange::{
                AddClipboardFormatListener, CloseClipboard, GetClipboardData,
                GetClipboardSequenceNumber, IsClipboardFormatAvailable, OpenClipboard,
                RegisterClipboardFormatW, RemoveClipboardFormatListener,
            },
            LibraryLoader::GetModuleHandleW,
            Memory::{GlobalLock, GlobalSize, GlobalUnlock},
            Threading::GetCurrentThreadId,
        },
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetWindowLongPtrW,
            PeekMessageW, RegisterClassW, SetWindowLongPtrW, TranslateMessage, UnregisterClassW,
            CREATESTRUCTW, GWLP_USERDATA, HWND_MESSAGE, MSG, PM_REMOVE, WINDOW_EX_STYLE,
            WINDOW_STYLE, WM_CLIPBOARDUPDATE, WM_CLOSE, WM_NCCREATE, WM_NCDESTROY, WM_QUIT,
            WNDCLASSW,
        },
    };

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn PeekNamedPipe(
            pipe: *mut c_void,
            buffer: *mut c_void,
            size: u32,
            read: *mut u32,
            available: *mut u32,
            left: *mut u32,
        ) -> i32;
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetProcessWindowStation() -> isize;
        fn GetThreadDesktop(thread: u32) -> isize;
        fn GetUserObjectInformationW(
            object: isize,
            index: i32,
            info: *mut c_void,
            length: u32,
            needed: *mut u32,
        ) -> i32;
        fn OpenWindowStationW(name: *const u16, inherit: i32, access: u32) -> isize;
        fn SetProcessWindowStation(station: isize) -> i32;
        fn CloseWindowStation(station: isize) -> i32;
        fn OpenDesktopW(name: *const u16, flags: u32, inherit: i32, access: u32) -> isize;
        fn SetThreadDesktop(desktop: isize) -> i32;
        fn CloseDesktop(desktop: isize) -> i32;
    }

    const CAPTURE_BUDGET: Duration = Duration::from_millis(1000);
    static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);

    /// 此对象只有显式 start 才创建线程。队列满时丢弃新结果并记 dropped，不阻塞退出。
    /// stop 后接收器可能仍含旧结果；接线方须以 session 和启用状态拒绝过期落盘。
    pub struct ClipboardHistoryListener {
        session: u64,
        signal: Arc<Signal>,
        window: Option<JoinHandle<Result<(), HistoryListenerError>>>,
        worker: Option<JoinHandle<()>>,
        persistent: Option<Arc<std::sync::Mutex<PersistentState>>>,
    }

    impl ClipboardHistoryListener {
        /// helper_executable 必须由应用提供自己的可执行文件，不能来自脚本参数。
        pub fn start(
            helper_executable: &Path,
        ) -> Result<(Self, mpsc::Receiver<HistoryText>), HistoryListenerError> {
            Self::start_with_reader(helper_executable, None)
        }
        /// 显式选择持续helper；仍只由后续通知触发采集，不在启动时读取旧正文。
        pub fn start_persistent(
            helper_executable: &Path,
            budgets: crate::clipboard_history_worker::HistoryWorkerBudgets,
        ) -> Result<(Self, mpsc::Receiver<HistoryText>), HistoryListenerError> {
            crate::clipboard_history_worker_protocol::WorkerMachine::new(budgets)
                .map_err(|_| HistoryListenerError::InvalidOptions)?;
            Self::start_with_reader(helper_executable, Some(budgets))
        }
        fn start_with_reader(
            helper_executable: &Path,
            budgets: Option<crate::clipboard_history_worker::HistoryWorkerBudgets>,
        ) -> Result<(Self, mpsc::Receiver<HistoryText>), HistoryListenerError> {
            if !helper_executable.is_absolute() || !helper_executable.is_file() {
                return Err(HistoryListenerError::InvalidOptions);
            }
            let session = NEXT_SESSION
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                    value.checked_add(1)
                })
                .map_err(|_| HistoryListenerError::StartFailed)?;
            let listener_scope = ListenerScope::capture()?;
            let station = listener_scope.station_name.clone();
            let desktop = listener_scope.desktop_name.clone();
            let (wake, wakes) = mpsc::sync_channel(1);
            let (output, results) = mpsc::sync_channel(OUTPUT_CAPACITY);
            let signal = Arc::new(Signal {
                stop: AtomicBool::new(false),
                wake,
                counters: Counters::default(),
            });
            let (ready, initial) = mpsc::sync_channel(1);
            let window_signal = signal.clone();
            let window = thread::Builder::new()
                .name("clipboard-history-window".into())
                .spawn(move || notification_window(window_signal, ready, listener_scope))
                .map_err(|_| HistoryListenerError::StartFailed)?;
            let mut listener = Self {
                session,
                signal,
                window: Some(window),
                worker: None,
                persistent: None,
            };
            let baseline = initial
                .recv_timeout(Duration::from_secs(2))
                .map_err(|_| HistoryListenerError::StartFailed)??;
            let worker_signal = listener.signal.clone();
            let reader = match budgets {
                None => SelectedReader::OneShot(ProcessReader {
                    executable: helper_executable.to_owned(),
                    station,
                    desktop,
                    signal: listener.signal.clone(),
                }),
                Some(budgets) => {
                    let state = Arc::new(std::sync::Mutex::new(PersistentState::new(
                        helper_executable.to_owned(),
                        station,
                        desktop,
                        budgets,
                    )?));
                    listener.persistent = Some(state.clone());
                    SelectedReader::Persistent(Box::new(PersistentReader {
                        state,
                        signal: listener.signal.clone(),
                    }))
                }
            };
            listener.worker = Some(
                thread::Builder::new()
                    .name("clipboard-history-reader".into())
                    .spawn(move || worker(reader, &worker_signal, wakes, output, session, baseline))
                    .map_err(|_| HistoryListenerError::StartFailed)?,
            );
            Ok((listener, results))
        }

        pub fn session(&self) -> u64 {
            self.session
        }
        pub fn stats(&self) -> HistoryListenerStats {
            self.signal.counters.snapshot()
        }

        /// 主动停机和 Drop 共用收尾；不等待消费者，不在窗口线程读正文。
        pub fn stop(&mut self) -> Result<HistoryListenerStats, HistoryListenerError> {
            self.signal.stop.store(true, Ordering::Release);
            let _ = self.signal.wake.try_send(());
            let mut error = None;
            if let Some(window) = self.window.take() {
                match window.join() {
                    Ok(Ok(())) => {}
                    Ok(Err(cause)) => error = Some(cause),
                    Err(_) => error = Some(HistoryListenerError::WindowFailed),
                }
            }
            if self
                .worker
                .take()
                .is_some_and(|handle| handle.join().is_err())
            {
                error = Some(HistoryListenerError::WorkerFailed);
            }
            if let Err(cause) = self.cleanup_persistent() {
                error = Some(cause);
            }
            match error {
                Some(cause) => Err(cause),
                None => Ok(self.stats()),
            }
        }
    }

    impl Drop for ClipboardHistoryListener {
        fn drop(&mut self) {
            let _ = self.stop();
        }
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if message == WM_NCCREATE {
            let create = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            }
            return LRESULT(1);
        }
        let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const Signal;
        if !pointer.is_null() {
            let signal = unsafe { &*pointer };
            if message == WM_CLIPBOARDUPDATE {
                signal.notify();
                return LRESULT(0);
            }
            if message == WM_CLOSE {
                signal.stop.store(true, Ordering::Release);
                return LRESULT(0);
            }
        }
        if message == WM_NCDESTROY {
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
        }
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }

    fn notification_window(
        signal: Arc<Signal>,
        ready: mpsc::SyncSender<Result<u32, HistoryListenerError>>,
        scope: ListenerScope,
    ) -> Result<(), HistoryListenerError> {
        let result = scope
            .attach(&mut NativeListenerScope)
            .and_then(|()| notification_window_inner(&signal, &ready, &scope));
        if let Err(error) = result {
            let _ = ready.try_send(Err(error));
        }
        signal.stop.store(true, Ordering::Release);
        result
    }

    fn notification_window_inner(
        signal: &Signal,
        ready: &mpsc::SyncSender<Result<u32, HistoryListenerError>>,
        scope: &ListenerScope,
    ) -> Result<(), HistoryListenerError> {
        let instance =
            unsafe { GetModuleHandleW(None) }.map_err(|_| HistoryListenerError::WindowFailed)?;
        let name: Vec<u16> = format!("OmniTriggerHistory_{}_{}", std::process::id(), unsafe {
            GetCurrentThreadId()
        })
        .encode_utf16()
        .chain([0])
        .collect();
        let class = WNDCLASSW {
            hInstance: instance.into(),
            lpszClassName: PCWSTR(name.as_ptr()),
            lpfnWndProc: Some(window_proc),
            ..Default::default()
        };
        if unsafe { RegisterClassW(&class) } == 0 {
            return Err(HistoryListenerError::WindowFailed);
        }
        struct Class<'a> {
            name: &'a [u16],
            instance: windows::Win32::Foundation::HINSTANCE,
        }
        impl Drop for Class<'_> {
            fn drop(&mut self) {
                let _ =
                    unsafe { UnregisterClassW(PCWSTR(self.name.as_ptr()), Some(self.instance)) };
            }
        }
        let _class = Class {
            name: &name,
            instance: instance.into(),
        };
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR(name.as_ptr()),
                w!(""),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(instance.into()),
                Some((signal as *const Signal).cast()),
            )
        }
        .map_err(|_| HistoryListenerError::WindowFailed)?;
        struct Window(HWND);
        impl Drop for Window {
            fn drop(&mut self) {
                let _ = unsafe { DestroyWindow(self.0) };
            }
        }
        let _window = Window(hwnd);
        unsafe { AddClipboardFormatListener(hwnd) }
            .map_err(|_| HistoryListenerError::WindowFailed)?;
        struct Subscription(HWND);
        impl Drop for Subscription {
            fn drop(&mut self) {
                let _ = unsafe { RemoveClipboardFormatListener(self.0) };
            }
        }
        let _subscription = Subscription(hwnd);
        ready
            .try_send(Ok(unsafe { GetClipboardSequenceNumber() }))
            .map_err(|_| HistoryListenerError::StartFailed)?;
        while !signal.stop.load(Ordering::Acquire) {
            scope.verify(&NativeListenerScope)?;
            let mut message = MSG::default();
            for _ in 0..64 {
                if !unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
                    break;
                }
                if message.message == WM_QUIT {
                    return Ok(());
                }
                unsafe {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }

    // 仅插入现有listener的native模块；不改默认模式，不依赖共享读取重构。
    enum SelectedReader {
        OneShot(ProcessReader),
        Persistent(Box<PersistentReader>),
    }
    impl CaptureReader for SelectedReader {
        fn capture(
            &mut self,
            previous: u32,
            stop: &AtomicBool,
        ) -> Result<Observation, HistoryListenerError> {
            match self {
                Self::OneShot(reader) => reader.capture(previous, stop),
                Self::Persistent(reader) => reader.capture(previous, stop),
            }
        }
    }
    // 仅此历史适配的测试接缝，不调度任务，也不定义另一个进程协议。
    trait PersistentClient: Send {
        fn start(
            &mut self,
            stop: &AtomicBool,
        ) -> Result<(), crate::clipboard_history_worker::HistoryWorkerError>;
        fn capture(
            &mut self,
            previous: u32,
            stop: &AtomicBool,
        ) -> Result<
            crate::clipboard_history_worker::HistoryWorkerReply,
            crate::clipboard_history_worker::HistoryWorkerError,
        >;
        fn stop(
            &mut self,
        ) -> Result<
            crate::clipboard_history_worker::HistoryWorkerStats,
            crate::clipboard_history_worker::HistoryWorkerError,
        >;
        fn stats(&self) -> crate::clipboard_history_worker::HistoryWorkerStats;
    }
    impl PersistentClient for crate::clipboard_history_worker::ClipboardHistoryWorker {
        fn start(
            &mut self,
            stop: &AtomicBool,
        ) -> Result<(), crate::clipboard_history_worker::HistoryWorkerError> {
            crate::clipboard_history_worker::ClipboardHistoryWorker::start(self, stop)
        }
        fn capture(
            &mut self,
            previous: u32,
            stop: &AtomicBool,
        ) -> Result<
            crate::clipboard_history_worker::HistoryWorkerReply,
            crate::clipboard_history_worker::HistoryWorkerError,
        > {
            crate::clipboard_history_worker::ClipboardHistoryWorker::capture(self, previous, stop)
        }
        fn stop(
            &mut self,
        ) -> Result<
            crate::clipboard_history_worker::HistoryWorkerStats,
            crate::clipboard_history_worker::HistoryWorkerError,
        > {
            crate::clipboard_history_worker::ClipboardHistoryWorker::stop(self)
        }
        fn stats(&self) -> crate::clipboard_history_worker::HistoryWorkerStats {
            crate::clipboard_history_worker::ClipboardHistoryWorker::stats(self)
        }
    }
    fn persistent_error(
        error: crate::clipboard_history_worker::HistoryWorkerError,
    ) -> HistoryListenerError {
        use crate::clipboard_history_worker::{
            HistoryCaptureFailure as F, HistoryWorkerError as E, HistoryWorkerProtocolError as P,
        };
        match error {
            E::Cancelled => HistoryListenerError::CaptureCancelled,
            E::CleanupFailed => HistoryListenerError::WorkerFailed,
            E::InvalidOptions | E::Protocol(P::InvalidBudget) => {
                HistoryListenerError::InvalidOptions
            }
            E::Protocol(P::StartupTimeout | P::ReadTimeout | P::StopTimeout) => {
                HistoryListenerError::CaptureTimedOut
            }
            E::Protocol(P::PeerRefused(F::ScopeMismatch)) => HistoryListenerError::ScopeMismatch,
            _ => HistoryListenerError::CaptureFailed,
        }
    }
    struct PersistentReader<
        C: PersistentClient = crate::clipboard_history_worker::ClipboardHistoryWorker,
    > {
        state: Arc<std::sync::Mutex<PersistentState<C>>>,
        signal: Arc<Signal>,
    }
    struct PersistentState<
        C: PersistentClient = crate::clipboard_history_worker::ClipboardHistoryWorker,
    > {
        client: C,
        reported: crate::clipboard_history_worker::HistoryWorkerStats,
    }
    impl PersistentState {
        fn new(
            executable: PathBuf,
            station: String,
            desktop: String,
            budgets: crate::clipboard_history_worker::HistoryWorkerBudgets,
        ) -> Result<Self, HistoryListenerError> {
            use crate::clipboard_history_worker::{
                ClipboardHistoryWorker, ClipboardScope, HistoryWorkerOptions,
            };
            let client = ClipboardHistoryWorker::new(HistoryWorkerOptions {
                helper_executable: executable,
                scope: ClipboardScope { station, desktop },
                budgets,
            })
            .map_err(persistent_error)?;
            Ok(Self {
                client,
                reported: Default::default(),
            })
        }
    }
    impl<C: PersistentClient> PersistentState<C> {
        fn report(&mut self, signal: &Signal) {
            let stats = self.client.stats();
            signal.counters.helpers_started.fetch_add(
                stats.spawned.saturating_sub(self.reported.spawned),
                Ordering::Relaxed,
            );
            signal.counters.helpers_finished.fetch_add(
                stats.reaped.saturating_sub(self.reported.reaped),
                Ordering::Relaxed,
            );
            self.reported = stats;
        }
        fn shutdown(&mut self, signal: &Signal) -> Result<(), HistoryListenerError> {
            let before = self.client.stats();
            let result = self.client.stop();
            let after = self.client.stats();
            self.report(signal);
            signal.counters.failed.fetch_add(
                after.failures.saturating_sub(before.failures),
                Ordering::Relaxed,
            );
            signal.counters.timeouts.fetch_add(
                after.timeouts.saturating_sub(before.timeouts),
                Ordering::Relaxed,
            );
            result.map_err(persistent_error).map(|_| ())
        }
    }
    impl<C: PersistentClient> CaptureReader for PersistentReader<C> {
        fn capture(
            &mut self,
            previous: u32,
            stop: &AtomicBool,
        ) -> Result<Observation, HistoryListenerError> {
            use crate::clipboard_history_worker::{
                HistoryCaptureFailure as F, HistoryWorkerReply as R, HistoryWorkerSkip as S,
            };
            let mut state = self
                .state
                .lock()
                .map_err(|_| HistoryListenerError::WorkerFailed)?;
            let result = state
                .client
                .start(stop)
                .and_then(|()| state.client.capture(previous, stop));
            state.report(&self.signal);
            match result.map_err(persistent_error)? {
                R::Text { sequence, text, .. } => Ok(Observation {
                    sequence,
                    content: Content::Text(text),
                }),
                R::Skipped {
                    sequence, reason, ..
                } => Ok(Observation {
                    sequence,
                    content: Content::Skip(match reason {
                        S::Duplicate => Skip::Duplicate,
                        S::Transient => Skip::Transient,
                        S::Excluded => Skip::Excluded,
                        S::Files => Skip::Files,
                        S::Image => Skip::Image,
                        S::Unsupported => Skip::Unsupported,
                    }),
                }),
                R::Failed { reason, .. } => Err(match reason {
                    F::ScopeMismatch => HistoryListenerError::ScopeMismatch,
                    F::Busy => HistoryListenerError::ClipboardBusy,
                    F::Changed => HistoryListenerError::ClipboardChanged,
                    F::InvalidText => HistoryListenerError::InvalidText,
                    F::TooLarge => HistoryListenerError::TextTooLarge,
                    F::Backend => HistoryListenerError::CaptureFailed,
                }),
                _ => Err(HistoryListenerError::InvalidReply),
            }
        }
    }
    fn stop_persistent_state<C: PersistentClient>(
        owner: &mut Option<Arc<std::sync::Mutex<PersistentState<C>>>>,
        signal: &Signal,
    ) -> Result<(), HistoryListenerError> {
        let shared = match owner.as_ref() {
            Some(shared) => Arc::clone(shared),
            None => return Ok(()),
        };
        let stopped = {
            // join之后仅为退出接回毒化状态；数据路径仍拒绝毒化锁。
            let mut state = match shared.lock() {
                Ok(state) => state,
                Err(poison) => poison.into_inner(),
            };
            state.shutdown(signal)
        };
        if stopped.is_ok() {
            owner.take();
        }
        // 失败时父对象保留同一client，下一次明确stop重试，不新建替代。
        stopped
    }
    impl ClipboardHistoryListener {
        fn cleanup_persistent(&mut self) -> Result<(), HistoryListenerError> {
            stop_persistent_state(&mut self.persistent, &self.signal)
        }
    }

    struct ProcessReader {
        executable: PathBuf,
        station: String,
        desktop: String,
        signal: Arc<Signal>,
    }
    struct Process(
        Child,
        Arc<Signal>,
        crate::clipboard_history_child_job::HistoryChildJob,
        bool,
    );
    impl Process {
        fn finish(&mut self) -> Result<(), HistoryListenerError> {
            if self.3 {
                return Ok(());
            }
            if !matches!(self.0.try_wait(), Ok(Some(_)))
                && self.0.kill().is_err()
                && !matches!(self.0.try_wait(), Ok(Some(_)))
            {
                self.1.counters.failed.fetch_add(1, Ordering::Relaxed);
                return Err(HistoryListenerError::WorkerFailed);
            }
            if self.0.wait().is_err() {
                self.1.counters.failed.fetch_add(1, Ordering::Relaxed);
                return Err(HistoryListenerError::WorkerFailed);
            }
            self.3 = true;
            self.1
                .counters
                .helpers_finished
                .fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }
    impl Drop for Process {
        fn drop(&mut self) {
            // 正常路径明确finish并传播错误；Drop只兜底。Job关闭也不冒充wait已成功。
            let _ = self.finish();
        }
    }

    impl CaptureReader for ProcessReader {
        fn capture(
            &mut self,
            previous: u32,
            stop: &AtomicBool,
        ) -> Result<Observation, HistoryListenerError> {
            if stop.load(Ordering::Acquire) {
                return Err(HistoryListenerError::CaptureCancelled);
            }
            let deadline = Instant::now() + CAPTURE_BUDGET;
            let job = crate::clipboard_history_child_job::HistoryChildJob::new()
                .map_err(|_| HistoryListenerError::CaptureFailed)?;
            let child = Command::new(&self.executable)
                .arg("--omni-clipboard-history-capture-gated")
                .arg(&self.station)
                .arg(&self.desktop)
                .arg(previous.to_string())
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .creation_flags(0x0800_0000)
                .spawn()
                .map_err(|_| HistoryListenerError::CaptureFailed)?;
            self.signal
                .counters
                .helpers_started
                .fetch_add(1, Ordering::Relaxed);
            let mut process = Process(child, self.signal.clone(), job, false);
            let result = (|| {
                process
                    .2
                    .assign(&process.0)
                    .map_err(|_| HistoryListenerError::CaptureFailed)?;
                if stop.load(Ordering::Acquire) {
                    return Err(HistoryListenerError::CaptureCancelled);
                }
                if Instant::now() >= deadline {
                    return Err(HistoryListenerError::CaptureTimedOut);
                }
                let mut input = process
                    .0
                    .stdin
                    .take()
                    .ok_or(HistoryListenerError::CaptureFailed)?;
                // helper在此之前只读授权管道；Job绑定失败或父退出均不允许触碰剪贴板。
                input
                    .write_all(crate::clipboard_history_child_job::CAPTURE_GATE)
                    .and_then(|()| input.flush())
                    .map_err(|_| HistoryListenerError::CaptureFailed)?;
                drop(input);
                let mut stdout = process
                    .0
                    .stdout
                    .take()
                    .ok_or(HistoryListenerError::CaptureFailed)?;
                let mut bytes = Vec::with_capacity(4096);
                loop {
                    if stop.load(Ordering::Acquire) {
                        return Err(HistoryListenerError::CaptureCancelled);
                    }
                    if Instant::now() >= deadline {
                        return Err(HistoryListenerError::CaptureTimedOut);
                    }
                    let mut available = 0;
                    let peek = unsafe {
                        PeekNamedPipe(
                            stdout.as_raw_handle(),
                            std::ptr::null_mut(),
                            0,
                            std::ptr::null_mut(),
                            &mut available,
                            std::ptr::null_mut(),
                        )
                    };
                    if peek != 0 && available > 0 {
                        if bytes.len() == MAX_FRAME {
                            return Err(HistoryListenerError::InvalidReply);
                        }
                        let mut buffer = [0; 4096];
                        let count = (available as usize)
                            .min(buffer.len())
                            .min(MAX_FRAME - bytes.len());
                        let read = stdout
                            .read(&mut buffer[..count])
                            .map_err(|_| HistoryListenerError::CaptureFailed)?;
                        if read == 0 {
                            return Err(HistoryListenerError::InvalidReply);
                        }
                        bytes.extend_from_slice(&buffer[..read]);
                        continue;
                    }
                    if let Some(status) = process
                        .0
                        .try_wait()
                        .map_err(|_| HistoryListenerError::CaptureFailed)?
                    {
                        if !status.success() {
                            return Err(HistoryListenerError::CaptureFailed);
                        }
                        return decode_reply(&bytes);
                    }
                    thread::sleep(Duration::from_millis(5));
                }
            })();
            // 清理失败优先返回；helpers_finished只在实际wait成功后增加。
            process.finish().and(result)
        }
    }

    fn object_name(object: isize) -> Result<String, HistoryListenerError> {
        let mut name = [0_u16; 257];
        let mut needed = 0;
        if object == 0
            || unsafe {
                GetUserObjectInformationW(
                    object,
                    2,
                    name.as_mut_ptr().cast(),
                    std::mem::size_of_val(&name) as u32,
                    &mut needed,
                )
            } == 0
        {
            return Err(HistoryListenerError::ScopeMismatch);
        }
        let count = name
            .iter()
            .position(|&unit| unit == 0)
            .ok_or(HistoryListenerError::ScopeMismatch)?;
        let name =
            String::from_utf16(&name[..count]).map_err(|_| HistoryListenerError::ScopeMismatch)?;
        if name.is_empty() {
            return Err(HistoryListenerError::ScopeMismatch);
        }
        Ok(name)
    }

    fn current_scope() -> Result<(String, String), HistoryListenerError> {
        Ok((
            object_name(unsafe { GetProcessWindowStation() })?,
            object_name(unsafe { GetThreadDesktop(GetCurrentThreadId()) })?,
        ))
    }

    // 监听线程只能借用启动方的站点/桌面，不拥有也不关闭这些句柄。
    // 调用方必须在切换进程窗口站之前stop并join；检查不替代进程全局同步。
    struct ListenerScope {
        station: isize,
        desktop: isize,
        station_name: String,
        desktop_name: String,
    }
    trait ListenerScopeBackend {
        fn station(&self) -> isize;
        fn desktop(&self) -> isize;
        fn name(&self, handle: isize) -> Result<String, HistoryListenerError>;
        fn set_desktop(&mut self, handle: isize) -> Result<(), HistoryListenerError>;
    }
    struct NativeListenerScope;
    impl ListenerScopeBackend for NativeListenerScope {
        fn station(&self) -> isize {
            unsafe { GetProcessWindowStation() }
        }
        fn desktop(&self) -> isize {
            unsafe { GetThreadDesktop(GetCurrentThreadId()) }
        }
        fn name(&self, handle: isize) -> Result<String, HistoryListenerError> {
            object_name(handle)
        }
        fn set_desktop(&mut self, handle: isize) -> Result<(), HistoryListenerError> {
            if unsafe { SetThreadDesktop(handle) } == 0 {
                Err(HistoryListenerError::ScopeMismatch)
            } else {
                Ok(())
            }
        }
    }
    impl ListenerScope {
        fn capture() -> Result<Self, HistoryListenerError> {
            let backend = NativeListenerScope;
            let station = backend.station();
            let desktop = backend.desktop();
            let scope = Self {
                station,
                desktop,
                station_name: backend.name(station)?,
                desktop_name: backend.name(desktop)?,
            };
            scope.verify(&backend)?;
            Ok(scope)
        }
        fn verify_station(
            &self,
            backend: &impl ListenerScopeBackend,
        ) -> Result<(), HistoryListenerError> {
            if self.station == 0
                || self.desktop == 0
                || backend.station() != self.station
                || backend.name(self.station)? != self.station_name
            {
                return Err(HistoryListenerError::ScopeMismatch);
            }
            Ok(())
        }
        fn verify(&self, backend: &impl ListenerScopeBackend) -> Result<(), HistoryListenerError> {
            self.verify_station(backend)?;
            if backend.desktop() != self.desktop || backend.name(self.desktop)? != self.desktop_name
            {
                return Err(HistoryListenerError::ScopeMismatch);
            }
            Ok(())
        }
        fn attach(
            &self,
            backend: &mut impl ListenerScopeBackend,
        ) -> Result<(), HistoryListenerError> {
            self.verify_station(backend)?;
            if backend.name(self.desktop)? != self.desktop_name {
                return Err(HistoryListenerError::ScopeMismatch);
            }
            // 使用明确的借用句柄，不因桌面不同而重开/重设进程站；线程退出释放其桌面引用。
            backend.set_desktop(self.desktop)?;
            self.verify(backend)
        }
    }

    struct Scope {
        old_station: isize,
        old_desktop: isize,
        station: isize,
        desktop: isize,
        station_changed: bool,
        desktop_changed: bool,
    }
    impl Drop for Scope {
        fn drop(&mut self) {
            unsafe {
                if self.station_changed {
                    let _ = SetProcessWindowStation(self.old_station);
                }
                if self.desktop_changed {
                    let _ = SetThreadDesktop(self.old_desktop);
                }
                if self.desktop != 0 {
                    let _ = CloseDesktop(self.desktop);
                }
                if self.station != 0 {
                    let _ = CloseWindowStation(self.station);
                }
            }
        }
    }

    fn enter_scope(station: &str, desktop: &str) -> Result<Scope, HistoryListenerError> {
        if [station, desktop].iter().any(|name| {
            name.is_empty()
                || name.encode_utf16().count() > 256
                || name
                    .chars()
                    .any(|character| matches!(character, '\0' | '\\' | '/'))
        }) {
            return Err(HistoryListenerError::ScopeMismatch);
        }
        let mut scope = Scope {
            old_station: unsafe { GetProcessWindowStation() },
            old_desktop: unsafe { GetThreadDesktop(GetCurrentThreadId()) },
            station: 0,
            desktop: 0,
            station_changed: false,
            desktop_changed: false,
        };
        // 仅供最早辅助进程入口使用：独占进程时允许进入明确站点。
        // 站点已正确时不能仅因线程桌面不同而替换进程全局句柄。
        if object_name(scope.old_station)? != station {
            let station_name: Vec<u16> = station.encode_utf16().chain([0]).collect();
            // 枚举桌面、读属性、剪贴板及全局原子（格式注册）；不请求创建桌面或改安全描述符。
            scope.station = unsafe { OpenWindowStationW(station_name.as_ptr(), 0, 0x27) };
            if scope.station == 0 || unsafe { SetProcessWindowStation(scope.station) } == 0 {
                return Err(HistoryListenerError::ScopeMismatch);
            }
            scope.station_changed = true;
        }
        let desktop_name: Vec<u16> = desktop.encode_utf16().chain([0]).collect();
        scope.desktop = unsafe { OpenDesktopW(desktop_name.as_ptr(), 0, 0, 0x83) };
        if scope.desktop == 0 || unsafe { SetThreadDesktop(scope.desktop) } == 0 {
            return Err(HistoryListenerError::ScopeMismatch);
        }
        scope.desktop_changed = true;
        if current_scope()? != (station.into(), desktop.into()) {
            return Err(HistoryListenerError::ScopeMismatch);
        }
        Ok(scope)
    }

    /// 内部受管一次性入口；未收到固定授权帧时不进入scope或读取剪贴板。
    /// 原run_clipboard_history_capture_helper保持直接读取含义，显式调用方自行拥有生命周期。
    pub fn run_clipboard_history_gated_capture_helper(
        station: &str,
        desktop: &str,
        previous: u32,
    ) -> i32 {
        if crate::clipboard_history_child_job::await_capture_gate(&mut std::io::stdin().lock())
            .is_err()
        {
            return 2;
        }
        run_clipboard_history_capture_helper(station, desktop, previous)
    }

    /// 仅用于应用内部辅助进程入口。接线方须在任何 UI、插件和脚本初始化之前调用。
    /// 参数是父进程明确采样的窗口站/桌面名；不能以失败为由回退当前交互窗口站。
    pub fn run_clipboard_history_capture_helper(
        station: &str,
        desktop: &str,
        previous: u32,
    ) -> i32 {
        let result = enter_scope(station, desktop).and_then(|_scope| capture_text(previous));
        let bytes = encode_reply(result);
        let mut output = std::io::stdout().lock();
        match output.write_all(&bytes).and_then(|()| output.flush()) {
            Ok(()) => 0,
            Err(_) => 2,
        }
    }

    fn capture_text(previous: u32) -> Result<Observation, HistoryListenerError> {
        let formats = [
            w!("OmniTrigger.TransientClipboard.v1"),
            w!("ExcludeClipboardContentFromMonitorProcessing"),
            w!("PNG"),
            w!("image/png"),
        ]
        .map(|name| unsafe { RegisterClipboardFormatW(name) });
        if formats.contains(&0) {
            return Err(HistoryListenerError::CaptureFailed);
        }
        capture_with(&mut NativeText { formats }, previous)
    }

    struct NativeText {
        formats: [u32; 4],
    }
    impl TextBackend for NativeText {
        fn open(&mut self) -> Result<(), HistoryListenerError> {
            for attempt in 0..3 {
                if unsafe { OpenClipboard(None) }.is_ok() {
                    return Ok(());
                }
                if attempt < 2 {
                    thread::sleep(Duration::from_millis(5));
                }
            }
            Err(HistoryListenerError::ClipboardBusy)
        }
        fn close(&mut self) -> Result<(), HistoryListenerError> {
            unsafe { CloseClipboard() }.map_err(|_| HistoryListenerError::CaptureFailed)
        }
        fn sequence(&mut self) -> u32 {
            unsafe { GetClipboardSequenceNumber() }
        }
        fn formats(&mut self) -> Formats {
            let available = |format| unsafe { IsClipboardFormatAvailable(format) }.is_ok();
            Formats {
                transient: available(self.formats[0]),
                excluded: available(self.formats[1]),
                files: available(15),
                image: [2, 3, 8, 14, 17, self.formats[2], self.formats[3]]
                    .into_iter()
                    .any(available),
                unicode: available(13),
            }
        }
        fn text(&mut self) -> Result<String, HistoryListenerError> {
            let memory = HGLOBAL(
                unsafe { GetClipboardData(13) }
                    .map_err(|_| HistoryListenerError::CaptureFailed)?
                    .0,
            );
            let size = unsafe { GlobalSize(memory) };
            if size < 2 || size % 2 != 0 {
                return Err(HistoryListenerError::InvalidText);
            }
            let pointer = unsafe { GlobalLock(memory) };
            if pointer.is_null() {
                return Err(HistoryListenerError::CaptureFailed);
            }
            struct Unlock(HGLOBAL);
            impl Drop for Unlock {
                fn drop(&mut self) {
                    let _ = unsafe { GlobalUnlock(self.0) };
                }
            }
            let _unlock = Unlock(memory);
            let units = unsafe {
                std::slice::from_raw_parts(pointer.cast::<u16>(), (size / 2).min(MAX_UNITS + 1))
            };
            let count = units
                .iter()
                .position(|&unit| unit == 0)
                .ok_or(HistoryListenerError::TextTooLarge)?;
            String::from_utf16(&units[..count]).map_err(|_| HistoryListenerError::InvalidText)
        }
    }
    #[cfg(test)]
    mod scope_tests {
        use super::*;
        struct Backend {
            station: isize,
            desktop: isize,
            sets: Vec<isize>,
            change_during_set: bool,
            refuse: bool,
        }
        impl ListenerScopeBackend for Backend {
            fn station(&self) -> isize {
                self.station
            }
            fn desktop(&self) -> isize {
                self.desktop
            }
            fn name(&self, handle: isize) -> Result<String, HistoryListenerError> {
                match handle {
                    1 | 2 => Ok("synthetic-private-station".into()),
                    10 => Ok("Default".into()),
                    11 | 12 => Ok("Def".into()),
                    _ => Err(HistoryListenerError::ScopeMismatch),
                }
            }
            fn set_desktop(&mut self, handle: isize) -> Result<(), HistoryListenerError> {
                self.sets.push(handle);
                if self.refuse {
                    return Err(HistoryListenerError::ScopeMismatch);
                }
                self.desktop = handle;
                if self.change_during_set {
                    self.station = 2;
                }
                Ok(())
            }
        }
        fn target() -> ListenerScope {
            ListenerScope {
                station: 1,
                desktop: 11,
                station_name: "synthetic-private-station".into(),
                desktop_name: "Def".into(),
            }
        }
        fn backend() -> Backend {
            Backend {
                station: 1,
                desktop: 10,
                sets: vec![],
                change_during_set: false,
                refuse: false,
            }
        }
        #[test]
        fn same_station_different_thread_desktop_only_assigns_borrowed_desktop() {
            let mut backend = backend();
            {
                let target = target();
                target.attach(&mut backend).unwrap();
                target.verify(&backend).unwrap();
            }
            assert_eq!(backend.station, 1);
            assert_eq!(backend.sets, [11]);
            assert_eq!((backend.station, backend.desktop), (1, 11));
        }
        #[test]
        fn same_station_name_with_changed_handle_is_rejected_before_thread_assignment() {
            let mut backend = backend();
            backend.station = 2;
            assert_eq!(
                target().attach(&mut backend),
                Err(HistoryListenerError::ScopeMismatch)
            );
            assert!(backend.sets.is_empty());
            assert_eq!(backend.station, 2);
        }
        #[test]
        fn external_station_change_during_attach_is_detected_without_restoring_global_state() {
            let mut backend = backend();
            backend.change_during_set = true;
            assert_eq!(
                target().attach(&mut backend),
                Err(HistoryListenerError::ScopeMismatch)
            );
            assert_eq!(backend.station, 2);
        }
        #[test]
        fn failed_desktop_assignment_does_not_enter_or_rewrite_process_station() {
            let mut backend = backend();
            backend.refuse = true;
            assert_eq!(
                target().attach(&mut backend),
                Err(HistoryListenerError::ScopeMismatch)
            );
            assert_eq!((backend.station, backend.desktop), (1, 10));
        }
        #[test]
        fn same_desktop_name_without_target_handle_is_not_enough() {
            let mut backend = backend();
            backend.desktop = 12;
            assert_eq!(
                target().verify(&backend),
                Err(HistoryListenerError::ScopeMismatch)
            );
            assert!(backend.sets.is_empty());
        }
        // 全部为内存假客户端：不启动线程/进程，不访问剪贴板或窗口站。
        use crate::clipboard_history_worker::{
            HistoryCaptureFailure as ClientFailure, HistoryWorkerError as ClientError,
            HistoryWorkerProtocolError as ClientProtocolError, HistoryWorkerReply as ClientReply,
            HistoryWorkerSkip as ClientSkip, HistoryWorkerStats as ClientStats,
        };
        use std::collections::VecDeque;
        use std::sync::Mutex;

        #[derive(Default)]
        struct ClientAudit {
            starts: usize,
            previous: Vec<u32>,
            stops: usize,
            alive: bool,
            stats: ClientStats,
        }
        struct ClientStep {
            result: Result<ClientReply, ClientError>,
            cancel_after: bool,
            wake_next: bool,
        }
        impl ClientStep {
            fn reply(reply: ClientReply) -> Self {
                Self {
                    result: Ok(reply),
                    cancel_after: false,
                    wake_next: false,
                }
            }
            fn error(error: ClientError) -> Self {
                Self {
                    result: Err(error),
                    cancel_after: false,
                    wake_next: false,
                }
            }
        }
        struct MemoryClient {
            audit: Arc<Mutex<ClientAudit>>,
            steps: VecDeque<ClientStep>,
            signal: std::sync::Weak<Signal>,
            stop_failures: usize,
            startup_error: Option<ClientError>,
        }
        impl PersistentClient for MemoryClient {
            fn start(&mut self, cancel: &AtomicBool) -> Result<(), ClientError> {
                let mut audit = self.audit.lock().unwrap();
                audit.starts += 1;
                if cancel.load(Ordering::Acquire) {
                    return Err(ClientError::Cancelled);
                }
                if let Some(error) = self.startup_error {
                    return Err(error);
                }
                if !audit.alive {
                    audit.alive = true;
                    audit.stats.spawned += 1;
                }
                Ok(())
            }
            fn capture(
                &mut self,
                previous: u32,
                cancel: &AtomicBool,
            ) -> Result<ClientReply, ClientError> {
                let step = self.steps.pop_front().expect("一次通知只消费一步");
                let mut audit = self.audit.lock().unwrap();
                audit.previous.push(previous);
                audit.stats.captures += 1;
                if step.result.is_err() {
                    // 假客户端报告已回收；真实kill/reap由worker自身测试和native验收证明。
                    audit.stats.failures += 1;
                    if matches!(
                        step.result,
                        Err(ClientError::Protocol(ClientProtocolError::ReadTimeout))
                    ) {
                        audit.stats.timeouts += 1;
                    }
                    if audit.alive {
                        audit.stats.reaped += 1;
                        audit.alive = false;
                    }
                }
                drop(audit);
                if step.cancel_after {
                    cancel.store(true, Ordering::Release);
                }
                if step.wake_next {
                    self.signal.upgrade().unwrap().wake.try_send(()).unwrap();
                }
                step.result
            }
            fn stop(&mut self) -> Result<ClientStats, ClientError> {
                let mut audit = self.audit.lock().unwrap();
                audit.stops += 1;
                if self.stop_failures > 0 {
                    self.stop_failures -= 1;
                    audit.stats.failures += 1;
                    return Err(ClientError::CleanupFailed);
                }
                if audit.alive {
                    audit.stats.reaped += 1;
                    audit.alive = false;
                }
                Ok(audit.stats)
            }
            fn stats(&self) -> ClientStats {
                self.audit.lock().unwrap().stats
            }
        }
        struct ReaderHarness {
            reader: PersistentReader<MemoryClient>,
            owner: Option<Arc<Mutex<PersistentState<MemoryClient>>>>,
            signal: Arc<Signal>,
            wakes: mpsc::Receiver<()>,
            audit: Arc<Mutex<ClientAudit>>,
        }
        fn memory_reader(steps: Vec<ClientStep>) -> ReaderHarness {
            let (wake, wakes) = mpsc::sync_channel(1);
            let signal = Arc::new(Signal {
                stop: AtomicBool::new(false),
                wake,
                counters: Counters::default(),
            });
            let audit = Arc::new(Mutex::new(ClientAudit::default()));
            let state = Arc::new(Mutex::new(PersistentState {
                client: MemoryClient {
                    audit: audit.clone(),
                    steps: steps.into(),
                    signal: Arc::downgrade(&signal),
                    stop_failures: 0,
                    startup_error: None,
                },
                reported: ClientStats::default(),
            }));
            ReaderHarness {
                reader: PersistentReader {
                    state: state.clone(),
                    signal: signal.clone(),
                },
                owner: Some(state),
                signal,
                wakes,
                audit,
            }
        }
        fn synthetic_text(sequence: u32) -> ClientReply {
            ClientReply::Text {
                request_id: u64::from(sequence) + 1,
                sequence,
                text: "synthetic".into(),
            }
        }

        #[test]
        fn persistent_adapter_is_inert_until_explicit_capture_and_reuses_client() {
            let mut h = memory_reader(vec![
                ClientStep::reply(synthetic_text(5)),
                ClientStep::reply(synthetic_text(6)),
            ]);
            assert_eq!(h.audit.lock().unwrap().starts, 0);
            assert_eq!(h.signal.counters.snapshot().captures, 0);
            assert_eq!(h.reader.capture(4, &h.signal.stop).unwrap().sequence, 5);
            assert_eq!(h.reader.capture(5, &h.signal.stop).unwrap().sequence, 6);
            {
                let a = h.audit.lock().unwrap();
                assert_eq!(a.stats.spawned, 1);
                assert_eq!(a.previous, [4, 5]);
            }
            assert_eq!(h.signal.counters.snapshot().helpers_started, 1);
            stop_persistent_state(&mut h.owner, &h.signal).unwrap();
            assert!(h.owner.is_none());
            assert_eq!(h.signal.counters.snapshot().helpers_finished, 1);
        }
        #[test]
        fn persistent_adapter_maps_all_skip_reasons_without_creating_new_client() {
            let pairs = [
                (ClientSkip::Duplicate, Skip::Duplicate),
                (ClientSkip::Transient, Skip::Transient),
                (ClientSkip::Excluded, Skip::Excluded),
                (ClientSkip::Files, Skip::Files),
                (ClientSkip::Image, Skip::Image),
                (ClientSkip::Unsupported, Skip::Unsupported),
            ];
            let mut h = memory_reader(
                pairs
                    .iter()
                    .map(|(reason, _)| {
                        ClientStep::reply(ClientReply::Skipped {
                            request_id: 1,
                            sequence: 8,
                            reason: *reason,
                        })
                    })
                    .collect(),
            );
            for (_, expected) in pairs {
                let result = h.reader.capture(7, &h.signal.stop).unwrap();
                assert_eq!(result.sequence, 8);
                assert!(matches!(result.content, Content::Skip(actual) if actual == expected));
            }
            assert_eq!(h.audit.lock().unwrap().stats.spawned, 1);
        }
        #[test]
        fn persistent_adapter_maps_capture_failures_without_adapter_retry() {
            let pairs = [
                (
                    ClientFailure::ScopeMismatch,
                    HistoryListenerError::ScopeMismatch,
                ),
                (ClientFailure::Busy, HistoryListenerError::ClipboardBusy),
                (
                    ClientFailure::Changed,
                    HistoryListenerError::ClipboardChanged,
                ),
                (
                    ClientFailure::InvalidText,
                    HistoryListenerError::InvalidText,
                ),
                (ClientFailure::TooLarge, HistoryListenerError::TextTooLarge),
                (ClientFailure::Backend, HistoryListenerError::CaptureFailed),
            ];
            let mut h = memory_reader(
                pairs
                    .iter()
                    .map(|(reason, _)| {
                        ClientStep::reply(ClientReply::Failed {
                            request_id: 1,
                            reason: *reason,
                        })
                    })
                    .collect(),
            );
            for (_, expected) in pairs {
                assert_eq!(h.reader.capture(7, &h.signal.stop).err(), Some(expected));
            }
            assert_eq!(h.audit.lock().unwrap().stats.spawned, 1);
        }
        #[test]
        fn persistent_adapter_protocol_failure_is_not_retried_inside_capture() {
            let mut h = memory_reader(vec![
                ClientStep::error(ClientError::Protocol(ClientProtocolError::InvalidFrame)),
                ClientStep::reply(synthetic_text(9)),
            ]);
            assert_eq!(
                h.reader.capture(7, &h.signal.stop).err(),
                Some(HistoryListenerError::CaptureFailed)
            );
            {
                let a = h.audit.lock().unwrap();
                assert_eq!(a.previous, [7]);
                assert_eq!(a.stats.reaped, 1);
            }
            assert_eq!(h.signal.counters.snapshot().helpers_finished, 1);
            // 下一次明确采集才可调用客户端重建；失败请求本身没有补发。
            assert_eq!(h.reader.capture(8, &h.signal.stop).unwrap().sequence, 9);
            assert_eq!(h.audit.lock().unwrap().stats.spawned, 2);
        }
        #[test]
        fn persistent_adapter_start_failure_does_not_issue_capture() {
            let mut h = memory_reader(vec![]);
            h.reader.state.lock().unwrap().client.startup_error = Some(ClientError::Protocol(
                ClientProtocolError::PeerRefused(ClientFailure::ScopeMismatch),
            ));
            assert_eq!(
                h.reader.capture(7, &h.signal.stop).err(),
                Some(HistoryListenerError::ScopeMismatch)
            );
            let a = h.audit.lock().unwrap();
            assert_eq!(a.starts, 1);
            assert!(a.previous.is_empty());
        }
        #[test]
        fn persistent_adapter_cancelled_before_start_does_not_capture_or_spawn() {
            let mut h = memory_reader(vec![]);
            h.signal.stop.store(true, Ordering::Release);
            assert_eq!(
                h.reader.capture(7, &h.signal.stop).err(),
                Some(HistoryListenerError::CaptureCancelled)
            );
            let a = h.audit.lock().unwrap();
            assert_eq!(a.stats.spawned, 0);
            assert!(a.previous.is_empty());
        }
        #[test]
        fn persistent_adapter_cleanup_failure_retains_same_client_for_explicit_retry() {
            let mut h = memory_reader(vec![ClientStep::reply(synthetic_text(8))]);
            h.reader.capture(7, &h.signal.stop).unwrap();
            h.reader.state.lock().unwrap().client.stop_failures = 1;
            let identity = Arc::as_ptr(h.owner.as_ref().unwrap());
            drop(h.reader);
            assert_eq!(
                stop_persistent_state(&mut h.owner, &h.signal),
                Err(HistoryListenerError::WorkerFailed)
            );
            assert_eq!(Arc::as_ptr(h.owner.as_ref().unwrap()), identity);
            assert_eq!(h.signal.counters.snapshot().helpers_finished, 0);
            stop_persistent_state(&mut h.owner, &h.signal).unwrap();
            stop_persistent_state(&mut h.owner, &h.signal).unwrap();
            let a = h.audit.lock().unwrap();
            assert_eq!(a.stops, 2);
            assert_eq!(a.stats.spawned, 1);
            assert_eq!(a.stats.reaped, 1);
            assert_eq!(h.signal.counters.snapshot().failed, 1);
            assert_eq!(h.signal.counters.snapshot().helpers_finished, 1);
        }
        #[test]
        fn persistent_adapter_poison_rejects_capture_but_cleanup_recovers_owned_client() {
            let mut h = memory_reader(vec![ClientStep::reply(synthetic_text(8))]);
            h.reader.capture(7, &h.signal.stop).unwrap();
            let shared = h.owner.as_ref().unwrap().clone();
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _guard = shared.lock().unwrap();
                panic!("synthetic poisoning");
            }));
            assert_eq!(
                h.reader.capture(8, &h.signal.stop).err(),
                Some(HistoryListenerError::WorkerFailed)
            );
            stop_persistent_state(&mut h.owner, &h.signal).unwrap();
            let a = h.audit.lock().unwrap();
            assert_eq!(a.previous, [7]);
            assert_eq!(a.stats.reaped, 1);
        }
        #[test]
        fn persistent_adapter_late_text_after_cancel_is_discarded_by_existing_worker() {
            let mut step = ClientStep::reply(synthetic_text(8));
            step.cancel_after = true;
            let mut h = memory_reader(vec![step]);
            let (output, results) = mpsc::sync_channel(1);
            h.signal.wake.try_send(()).unwrap();
            worker(h.reader, &h.signal, h.wakes, output, 42, 7);
            assert!(results.try_recv().is_err());
            assert_eq!(h.signal.counters.snapshot().delivered, 0);
            assert_eq!(h.audit.lock().unwrap().previous, [7]);
            stop_persistent_state(&mut h.owner, &h.signal).unwrap();
        }
        #[test]
        fn persistent_adapter_uses_bounded_output_and_drops_new_result_when_full() {
            let mut first = ClientStep::reply(synthetic_text(8));
            first.wake_next = true;
            let mut second = ClientStep::reply(synthetic_text(9));
            second.wake_next = true;
            let mut last = ClientStep::reply(synthetic_text(10));
            last.cancel_after = true;
            let mut h = memory_reader(vec![first, second, last]);
            let (output, results) = mpsc::sync_channel(1);
            h.signal.wake.try_send(()).unwrap();
            worker(h.reader, &h.signal, h.wakes, output, 42, 7);
            assert_eq!(results.try_recv().unwrap().sequence_number, 8);
            assert!(results.try_recv().is_err());
            let stats = h.signal.counters.snapshot();
            assert_eq!(stats.delivered, 1);
            assert_eq!(stats.dropped, 1);
            assert_eq!(h.audit.lock().unwrap().stats.spawned, 1);
            stop_persistent_state(&mut h.owner, &h.signal).unwrap();
        }
        #[test]
        fn persistent_adapter_timeout_counts_once_in_existing_worker() {
            let mut first =
                ClientStep::error(ClientError::Protocol(ClientProtocolError::ReadTimeout));
            first.wake_next = true;
            let mut last = ClientStep::reply(synthetic_text(9));
            last.cancel_after = true;
            let mut h = memory_reader(vec![first, last]);
            let (output, results) = mpsc::sync_channel(1);
            h.signal.wake.try_send(()).unwrap();
            worker(h.reader, &h.signal, h.wakes, output, 42, 7);
            assert!(results.try_recv().is_err());
            let stats = h.signal.counters.snapshot();
            assert_eq!(stats.failed, 1);
            assert_eq!(stats.timeouts, 1);
            assert_eq!(stats.helpers_started, 2);
            assert_eq!(stats.helpers_finished, 1);
            stop_persistent_state(&mut h.owner, &h.signal).unwrap();
            assert_eq!(h.signal.counters.snapshot().helpers_finished, 2);
        }
        #[test]
        fn persistent_adapter_rejects_control_reply_on_capture_path() {
            let mut h = memory_reader(vec![ClientStep::reply(ClientReply::Ready)]);
            assert_eq!(
                h.reader.capture(7, &h.signal.stop).err(),
                Some(HistoryListenerError::InvalidReply)
            );
            assert_eq!(h.audit.lock().unwrap().previous, [7]);
        }
        #[test]
        fn persistent_adapter_accepts_text_decoded_across_every_wire_split() {
            use crate::clipboard_history_worker_protocol::{encode_reply, Decoder};
            let frame = encode_reply(&synthetic_text(8)).unwrap();
            for split in 1..frame.len() {
                let mut decoder = Decoder::default();
                assert_eq!(decoder.push(&frame[..split]).unwrap(), None);
                let decoded = decoder.push(&frame[split..]).unwrap().unwrap();
                let mut h = memory_reader(vec![ClientStep::reply(decoded)]);
                let observation = h.reader.capture(7, &h.signal.stop).unwrap();
                assert_eq!(observation.sequence, 8);
                assert!(
                    matches!(observation.content, Content::Text(ref text) if text == "synthetic")
                );
                assert_eq!(h.audit.lock().unwrap().previous, [7]);
            }
        }
        #[test]
        fn persistent_adapter_propagates_decoder_rejection_once_without_fallback() {
            use crate::clipboard_history_worker_protocol::{encode_reply, Decoder, MAX_UNITS};
            let frame = encode_reply(&synthetic_text(8)).unwrap();
            let mut bad_magic = frame.clone();
            bad_magic[0] ^= 1;
            let mut trailing = frame.clone();
            trailing.push(0);
            let mut too_large = frame[..25].to_vec();
            too_large[21..25].copy_from_slice(&((MAX_UNITS + 1) as u32).to_le_bytes());
            for (bytes, expected) in [
                (bad_magic, ClientProtocolError::InvalidFrame),
                (trailing, ClientProtocolError::TrailingData),
                (too_large, ClientProtocolError::TooLarge),
            ] {
                let mut decoder = Decoder::default();
                let error = decoder.push(&bytes).unwrap_err();
                assert_eq!(error, expected);
                assert_eq!(decoder.push(&frame), Err(ClientProtocolError::InvalidState));
                let mut h = memory_reader(vec![ClientStep::error(ClientError::Protocol(error))]);
                assert_eq!(
                    h.reader.capture(7, &h.signal.stop).err(),
                    Some(HistoryListenerError::CaptureFailed)
                );
                let a = h.audit.lock().unwrap();
                assert_eq!(a.previous, [7]);
                assert_eq!(a.stats.reaped, 1);
            }
        }
        #[test]
        fn persistent_adapter_propagates_truncated_eof_without_body_delivery() {
            use crate::clipboard_history_worker_protocol::{encode_reply, Decoder};
            let frame = encode_reply(&synthetic_text(8)).unwrap();
            let mut decoder = Decoder::default();
            assert_eq!(decoder.push(&frame[..frame.len() - 1]), Ok(None));
            let error = decoder.end_of_stream().unwrap_err();
            assert_eq!(error, ClientProtocolError::Truncated);
            let mut h = memory_reader(vec![ClientStep::error(ClientError::Protocol(error))]);
            assert_eq!(
                h.reader.capture(7, &h.signal.stop).err(),
                Some(HistoryListenerError::CaptureFailed)
            );
            assert_eq!(h.signal.counters.snapshot().helpers_finished, 1);
        }
        #[test]
        fn persistent_adapter_sequence_wrap_uses_equality_and_delivers_new_zero() {
            let mut first = ClientStep::reply(synthetic_text(0));
            first.wake_next = true;
            let mut last = ClientStep::reply(synthetic_text(1));
            last.cancel_after = true;
            let mut h = memory_reader(vec![first, last]);
            let (output, results) = mpsc::sync_channel(1);
            h.signal.wake.try_send(()).unwrap();
            worker(h.reader, &h.signal, h.wakes, output, 42, u32::MAX);
            let record = results.try_recv().unwrap();
            assert_eq!((record.session, record.sequence_number), (42, 0));
            assert!(results.try_recv().is_err());
            assert_eq!(h.audit.lock().unwrap().previous, [u32::MAX, 0]);
            stop_persistent_state(&mut h.owner, &h.signal).unwrap();
        }
        #[test]
        fn persistent_adapter_stop_before_worker_rejects_even_queued_notification() {
            let mut h = memory_reader(vec![]);
            let (output, results) = mpsc::sync_channel(1);
            h.signal.wake.try_send(()).unwrap();
            h.signal.stop.store(true, Ordering::Release);
            worker(h.reader, &h.signal, h.wakes, output, 42, 7);
            assert!(results.try_recv().is_err());
            assert_eq!(h.audit.lock().unwrap().starts, 0);
            assert_eq!(h.signal.counters.snapshot().captures, 0);
            stop_persistent_state(&mut h.owner, &h.signal).unwrap();
            assert_eq!(h.signal.counters.snapshot().helpers_started, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    struct MemoryClipboard {
        formats: Formats,
        sequences: VecDeque<u32>,
        text: Result<String, HistoryListenerError>,
        open_error: Option<HistoryListenerError>,
        close_error: Option<HistoryListenerError>,
        opened: usize,
        closed: usize,
        reads: usize,
        panic_on_read: bool,
    }

    impl MemoryClipboard {
        fn text(value: &str) -> Self {
            Self {
                formats: Formats {
                    unicode: true,
                    ..Default::default()
                },
                sequences: VecDeque::from([10, 10]),
                text: Ok(value.into()),
                open_error: None,
                close_error: None,
                opened: 0,
                closed: 0,
                reads: 0,
                panic_on_read: false,
            }
        }
    }

    impl TextBackend for MemoryClipboard {
        fn open(&mut self) -> Result<(), HistoryListenerError> {
            self.opened += 1;
            self.open_error.map_or(Ok(()), Err)
        }
        fn close(&mut self) -> Result<(), HistoryListenerError> {
            self.closed += 1;
            self.close_error.map_or(Ok(()), Err)
        }
        fn sequence(&mut self) -> u32 {
            self.sequences.pop_front().unwrap()
        }
        fn formats(&mut self) -> Formats {
            self.formats
        }
        fn text(&mut self) -> Result<String, HistoryListenerError> {
            self.reads += 1;
            assert!(!self.panic_on_read, "合成后端异常");
            self.text.clone()
        }
    }

    fn text(sequence: u32, value: &str) -> Observation {
        Observation {
            sequence,
            content: Content::Text(value.into()),
        }
    }

    fn assert_text(observation: Observation, sequence: u32, expected: &str) {
        assert_eq!(observation.sequence, sequence);
        let Content::Text(value) = observation.content else {
            panic!("预期文本");
        };
        assert_eq!(value, expected);
    }

    #[test]
    fn exclusion_and_nontext_formats_skip_before_any_payload_read() {
        for (formats, expected) in [
            (
                Formats {
                    transient: true,
                    unicode: true,
                    ..Default::default()
                },
                Skip::Transient,
            ),
            (
                Formats {
                    excluded: true,
                    unicode: true,
                    ..Default::default()
                },
                Skip::Excluded,
            ),
            (
                Formats {
                    files: true,
                    unicode: true,
                    ..Default::default()
                },
                Skip::Files,
            ),
            (
                Formats {
                    image: true,
                    unicode: true,
                    ..Default::default()
                },
                Skip::Image,
            ),
            (Formats::default(), Skip::Unsupported),
        ] {
            let mut backend = MemoryClipboard::text("synthetic body must remain unread");
            backend.formats = formats;
            let observation = capture_with(&mut backend, 9).unwrap();
            let Content::Skip(reason) = observation.content else {
                panic!("预期跳过");
            };
            assert_eq!(reason, expected);
            assert_eq!(backend.reads, 0);
            assert_eq!((backend.opened, backend.closed), (1, 1));
        }
    }

    #[test]
    fn unchanged_sequence_does_not_read_existing_contents_at_start() {
        let mut backend = MemoryClipboard::text("synthetic old text");
        let observation = capture_with(&mut backend, 10).unwrap();
        assert!(matches!(
            observation.content,
            Content::Skip(Skip::Duplicate)
        ));
        assert_eq!(backend.reads, 0);
        assert_eq!(backend.closed, 1);
    }

    #[test]
    fn valid_text_preserves_empty_whitespace_and_supplementary_unicode() {
        for value in ["", " \r\n\t ", "合成 文😀"] {
            let mut backend = MemoryClipboard::text(value);
            assert_text(capture_with(&mut backend, 9).unwrap(), 10, value);
            assert_eq!((backend.reads, backend.closed), (1, 1));
        }
    }

    #[test]
    fn text_limits_count_utf16_without_trimming_or_truncation() {
        for value in ["x".repeat(MAX_UNITS), "😀".repeat(MAX_UNITS / 2)] {
            let mut backend = MemoryClipboard::text(&value);
            assert_text(capture_with(&mut backend, 9).unwrap(), 10, &value);
        }
        for (value, expected) in [
            (
                "x".repeat(MAX_UNITS + 1),
                HistoryListenerError::TextTooLarge,
            ),
            (
                "synthetic\0hidden".into(),
                HistoryListenerError::InvalidText,
            ),
        ] {
            let mut backend = MemoryClipboard::text(&value);
            assert_eq!(capture_with(&mut backend, 9).err(), Some(expected));
            assert_eq!(backend.closed, 1);
        }
    }

    #[test]
    fn failed_open_read_close_and_changed_sequence_never_return_partial_text() {
        let mut backend = MemoryClipboard::text("synthetic");
        backend.open_error = Some(HistoryListenerError::ClipboardBusy);
        assert_eq!(
            capture_with(&mut backend, 9).err(),
            Some(HistoryListenerError::ClipboardBusy)
        );
        assert_eq!((backend.closed, backend.reads), (0, 0));
        let mut backend = MemoryClipboard::text("synthetic");
        backend.text = Err(HistoryListenerError::CaptureFailed);
        assert_eq!(
            capture_with(&mut backend, 9).err(),
            Some(HistoryListenerError::CaptureFailed)
        );
        assert_eq!(backend.closed, 1);
        let mut backend = MemoryClipboard::text("synthetic");
        backend.close_error = Some(HistoryListenerError::CaptureFailed);
        assert_eq!(
            capture_with(&mut backend, 9).err(),
            Some(HistoryListenerError::CaptureFailed)
        );
        assert_eq!(backend.closed, 1);
        let mut backend = MemoryClipboard::text("synthetic");
        backend.sequences = VecDeque::from([10, 11]);
        assert_eq!(
            capture_with(&mut backend, 9).err(),
            Some(HistoryListenerError::ClipboardChanged)
        );
        assert_eq!(backend.closed, 1);
    }

    #[test]
    fn protocol_roundtrips_text_skip_and_fixed_errors_without_error_bodies() {
        for value in ["", " \r\n文😀 "] {
            assert_text(
                decode_reply(&encode_reply(Ok(text(u32::MAX, value)))).unwrap(),
                u32::MAX,
                value,
            );
        }
        for reason in [
            Skip::Duplicate,
            Skip::Transient,
            Skip::Excluded,
            Skip::Files,
            Skip::Image,
            Skip::Unsupported,
        ] {
            let observation = decode_reply(&encode_reply(Ok(Observation {
                sequence: 1,
                content: Content::Skip(reason),
            })))
            .unwrap();
            assert!(matches!(observation.content, Content::Skip(value) if value == reason));
        }
        for error in [
            HistoryListenerError::ScopeMismatch,
            HistoryListenerError::ClipboardBusy,
            HistoryListenerError::ClipboardChanged,
            HistoryListenerError::InvalidText,
            HistoryListenerError::TextTooLarge,
            HistoryListenerError::CaptureFailed,
        ] {
            let bytes = encode_reply(Err(error));
            assert_eq!(bytes.len(), HEADER);
            assert_eq!(decode_reply(&bytes).err(), Some(error));
        }
    }

    #[test]
    fn protocol_rejects_truncation_trailing_fields_bad_tags_nul_and_invalid_utf16() {
        let valid = encode_reply(Ok(text(1, "x")));
        for length in 0..valid.len() {
            assert!(decode_reply(&valid[..length]).is_err());
        }
        let mut bytes = valid.clone();
        bytes.push(0);
        assert_eq!(
            decode_reply(&bytes).err(),
            Some(HistoryListenerError::InvalidReply)
        );
        let mut bytes = valid.clone();
        bytes[0] = b'!';
        assert_eq!(
            decode_reply(&bytes).err(),
            Some(HistoryListenerError::InvalidReply)
        );
        let mut bytes = valid.clone();
        bytes[12] = 7;
        assert_eq!(
            decode_reply(&bytes).err(),
            Some(HistoryListenerError::InvalidReply)
        );
        let mut bytes = valid.clone();
        bytes[HEADER..].copy_from_slice(&0xd800_u16.to_le_bytes());
        assert_eq!(
            decode_reply(&bytes).err(),
            Some(HistoryListenerError::InvalidText)
        );
        let mut bytes = valid;
        bytes[HEADER..].fill(0);
        assert_eq!(
            decode_reply(&bytes).err(),
            Some(HistoryListenerError::InvalidText)
        );
        assert_eq!(
            decode_reply(&vec![0; MAX_FRAME + 1]).err(),
            Some(HistoryListenerError::InvalidReply)
        );
    }

    struct Reader {
        values: VecDeque<Result<Observation, HistoryListenerError>>,
        signal: Arc<Signal>,
        calls: Arc<AtomicU64>,
        cancel_during_capture: bool,
    }

    impl CaptureReader for Reader {
        fn capture(&mut self, _: u32, _: &AtomicBool) -> Result<Observation, HistoryListenerError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if self.cancel_during_capture {
                self.signal.stop.store(true, Ordering::Release);
            }
            match self.values.pop_front() {
                Some(value) => {
                    self.signal.notify();
                    value
                }
                None => {
                    self.signal.stop.store(true, Ordering::Release);
                    Err(HistoryListenerError::CaptureCancelled)
                }
            }
        }
    }

    fn signal() -> (Arc<Signal>, mpsc::Receiver<()>) {
        let (wake, wakes) = mpsc::sync_channel(1);
        (
            Arc::new(Signal {
                stop: AtomicBool::new(false),
                wake,
                counters: Counters::default(),
            }),
            wakes,
        )
    }

    fn run(
        values: Vec<Result<Observation, HistoryListenerError>>,
        capacity: usize,
        previous: u32,
    ) -> (Vec<HistoryText>, HistoryListenerStats) {
        let (signal, wakes) = signal();
        signal.notify();
        let reader = Reader {
            values: values.into(),
            signal: signal.clone(),
            calls: Arc::new(AtomicU64::new(0)),
            cancel_during_capture: false,
        };
        let (output, results) = mpsc::sync_channel(capacity);
        worker(reader, &signal, wakes, output, 42, previous);
        (results.try_iter().collect(), signal.counters.snapshot())
    }

    #[test]
    fn sequence_is_equality_only_and_wrap_does_not_suppress_new_observations() {
        let (results, stats) = run(
            vec![
                Ok(text(0, "zero")),
                Ok(text(0, "duplicate")),
                Ok(text(1, "one")),
                Ok(text(u32::MAX, "older numeric sequence")),
            ],
            OUTPUT_CAPACITY,
            u32::MAX,
        );
        assert_eq!(
            results
                .iter()
                .map(|value| value.sequence_number)
                .collect::<Vec<_>>(),
            [0, 1, u32::MAX]
        );
        assert!(results.iter().all(|value| value.session == 42));
        assert_eq!(stats.skipped, 1);
        assert_eq!(stats.delivered, 3);
    }

    #[test]
    fn notification_storm_is_coalesced_to_a_single_pending_wakeup() {
        let (signal, wakes) = signal();
        for _ in 0..1000 {
            signal.notify();
        }
        assert_eq!(wakes.try_recv(), Ok(()));
        assert_eq!(wakes.try_recv(), Err(mpsc::TryRecvError::Empty));
        assert_eq!(signal.counters.snapshot().coalesced, 999);
        signal.stop.store(true, Ordering::Release);
        signal.notify();
        assert_eq!(wakes.try_recv(), Err(mpsc::TryRecvError::Empty));
    }

    #[test]
    fn full_output_queue_never_blocks_the_reader_or_retries_delivery() {
        let values = (1..=20)
            .map(|sequence| Ok(text(sequence, "synthetic")))
            .collect();
        let (results, stats) = run(values, OUTPUT_CAPACITY, 0);
        assert_eq!(results.len(), OUTPUT_CAPACITY);
        assert_eq!(stats.delivered, OUTPUT_CAPACITY as u64);
        assert_eq!(stats.dropped, 4);
    }

    #[test]
    fn stop_before_or_during_capture_delivers_no_late_record() {
        for during in [false, true] {
            let (signal, wakes) = signal();
            signal.notify();
            if !during {
                signal.stop.store(true, Ordering::Release);
            }
            let calls = Arc::new(AtomicU64::new(0));
            let reader = Reader {
                values: VecDeque::from([Ok(text(1, "synthetic"))]),
                signal: signal.clone(),
                calls: calls.clone(),
                cancel_during_capture: during,
            };
            let (output, results) = mpsc::sync_channel(OUTPUT_CAPACITY);
            worker(reader, &signal, wakes, output, 1, 0);
            assert_eq!(results.try_recv(), Err(mpsc::TryRecvError::Disconnected));
            assert_eq!(calls.load(Ordering::Relaxed), u64::from(during));
        }
    }

    #[test]
    fn unavailable_consumer_stops_without_spinning_or_losing_control_of_window() {
        let (signal, wakes) = signal();
        signal.notify();
        let reader = Reader {
            values: VecDeque::from([Ok(text(1, "synthetic"))]),
            signal: signal.clone(),
            calls: Arc::new(AtomicU64::new(0)),
            cancel_during_capture: false,
        };
        let (output, results) = mpsc::sync_channel(1);
        drop(results);
        worker(reader, &signal, wakes, output, 1, 0);
        assert!(signal.stop.load(Ordering::Acquire));
    }

    #[test]
    fn skip_and_errors_do_not_deliver_body_and_later_notification_can_recover() {
        let (results, stats) = run(
            vec![
                Err(HistoryListenerError::ClipboardBusy),
                Ok(Observation {
                    sequence: 1,
                    content: Content::Skip(Skip::Transient),
                }),
                Ok(text(2, "synthetic recovered")),
            ],
            OUTPUT_CAPACITY,
            0,
        );
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].text, "synthetic recovered");
        assert_eq!(stats.failed, 1);
        assert_eq!(stats.skipped, 1);
    }

    #[test]
    fn debug_and_errors_do_not_echo_text() {
        let value = HistoryText {
            session: 1,
            sequence_number: 2,
            text: "synthetic-private-正文".into(),
        };
        assert!(!format!("{value:?}").contains(&value.text));
        assert_eq!(
            HistoryListenerError::CaptureFailed.to_string(),
            "history_listener_capture_failed"
        );
    }

    #[test]
    fn clipboard_lease_closes_once_when_a_synthetic_reader_panics() {
        let mut backend = MemoryClipboard::text("synthetic");
        backend.panic_on_read = true;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            capture_with(&mut backend, 9)
        }));
        assert!(result.is_err());
        assert_eq!((backend.opened, backend.closed), (1, 1));
    }

    #[test]
    fn oversized_helper_text_is_replaced_with_a_fixed_error_frame() {
        let bytes = encode_reply(Ok(text(1, &"x".repeat(MAX_UNITS + 1))));
        assert_eq!(bytes.len(), HEADER);
        assert_eq!(
            decode_reply(&bytes).err(),
            Some(HistoryListenerError::TextTooLarge)
        );
    }

    #[test]
    fn timeout_is_counted_without_replaying_the_failed_read_or_blocking_later_work() {
        let (results, stats) = run(
            vec![
                Err(HistoryListenerError::CaptureTimedOut),
                Ok(text(1, "synthetic after timeout")),
            ],
            OUTPUT_CAPACITY,
            0,
        );
        assert_eq!(results.len(), 1);
        assert_eq!(stats.timeouts, 1);
        assert_eq!(stats.failed, 1);
        assert_eq!(stats.delivered, 1);
    }

    #[test]
    fn panicking_reader_still_stops_the_window_signal_and_closes_output() {
        struct PanicReader;
        impl CaptureReader for PanicReader {
            fn capture(
                &mut self,
                _: u32,
                _: &AtomicBool,
            ) -> Result<Observation, HistoryListenerError> {
                panic!("合成读取器异常");
            }
        }
        let (signal, wakes) = signal();
        signal.notify();
        let (output, results) = mpsc::sync_channel(1);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            worker(PanicReader, &signal, wakes, output, 1, 0);
        }));
        assert!(result.is_err());
        assert!(signal.stop.load(Ordering::Acquire));
        assert_eq!(results.try_recv(), Err(mpsc::TryRecvError::Disconnected));
    }
}
