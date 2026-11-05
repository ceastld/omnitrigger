//! 显式授权后复用的只读历史辅助进程。空闲不采集；每个请求只发送一次。
//! 构造不创建进程；start只握手，capture才读取。stop关闭授权并回收自己的进程。
//! 所有调用在后台执行；同步Win32创建进程等系统调用仍需原生验收，预算不代表性能。
#![cfg(any(windows, test))]

use crate::clipboard_history_worker_protocol as protocol;
pub use protocol::{
    Budgets as HistoryWorkerBudgets, CaptureFailure as HistoryCaptureFailure,
    ProtocolError as HistoryWorkerProtocolError, Reply as HistoryWorkerReply,
    SkipReason as HistoryWorkerSkip,
};
use std::fmt;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardScope {
    pub station: String,
    pub desktop: String,
}

impl ClipboardScope {
    fn validate(&self) -> Result<(), HistoryWorkerError> {
        if [&self.station, &self.desktop].iter().any(|name| {
            name.is_empty()
                || name.encode_utf16().count() > 256
                || name
                    .chars()
                    .any(|character| matches!(character, '\0' | '\\' | '/'))
        }) {
            return Err(HistoryWorkerError::InvalidOptions);
        }
        Ok(())
    }
}

/// 可执行文件必须由应用提供，不能来自脚本或历史项正文。
pub struct HistoryWorkerOptions {
    pub helper_executable: PathBuf,
    pub scope: ClipboardScope,
    pub budgets: HistoryWorkerBudgets,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryWorkerError {
    InvalidOptions,
    NotStarted,
    Cancelled,
    SpawnFailed,
    TransportFailed,
    PeerExited,
    CleanupFailed,
    Protocol(protocol::ProtocolError),
}

impl fmt::Display for HistoryWorkerError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output.write_str(match self {
            Self::InvalidOptions => "history_worker_invalid_options",
            Self::NotStarted => "history_worker_not_started",
            Self::Cancelled => "history_worker_cancelled",
            Self::SpawnFailed => "history_worker_spawn_failed",
            Self::TransportFailed => "history_worker_transport_failed",
            Self::PeerExited => "history_worker_peer_exited",
            Self::CleanupFailed => "history_worker_cleanup_failed",
            Self::Protocol(_) => "history_worker_protocol_failed",
        })
    }
}
impl std::error::Error for HistoryWorkerError {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HistoryWorkerStats {
    pub spawned: u64,
    pub reaped: u64,
    pub captures: u64,
    pub failures: u64,
    pub timeouts: u64,
}

enum ReadProgress {
    Bytes(usize),
    Pending,
    Eof,
}
enum WriteProgress {
    Bytes(usize),
    Pending,
}

/// start失败也可能持有部分资源，terminate_and_reap必须幂等并报告未收尾状态。
trait Transport {
    fn now_ms(&self) -> u64;
    fn process_present(&self) -> bool;
    fn start(&mut self) -> Result<(), HistoryWorkerError>;
    fn write(&mut self, bytes: &[u8]) -> Result<WriteProgress, HistoryWorkerError>;
    fn read(&mut self, bytes: &mut [u8]) -> Result<ReadProgress, HistoryWorkerError>;
    fn alive(&mut self) -> Result<bool, HistoryWorkerError>;
    fn wait_tick(&mut self);
    fn terminate_and_reap(&mut self) -> Result<(), HistoryWorkerError>;
}

struct Session<B: Transport> {
    backend: B,
    machine: protocol::WorkerMachine,
    decoder: protocol::Decoder,
    enabled: bool,
    has_process: bool,
    cleanup_failed: bool,
    stop_grace_ms: u64,
    stats: HistoryWorkerStats,
}

impl<B: Transport> Session<B> {
    fn new(backend: B, budgets: HistoryWorkerBudgets) -> Result<Self, HistoryWorkerError> {
        let machine =
            protocol::WorkerMachine::new(budgets).map_err(HistoryWorkerError::Protocol)?;
        Ok(Self {
            backend,
            machine,
            decoder: protocol::Decoder::default(),
            enabled: false,
            has_process: false,
            cleanup_failed: false,
            stop_grace_ms: budgets.stop_grace_ms,
            stats: HistoryWorkerStats::default(),
        })
    }

    fn cleanup(&mut self) -> Result<(), HistoryWorkerError> {
        self.machine.peer_exited();
        self.decoder = protocol::Decoder::default();
        if self.has_process {
            let had_process = self.backend.process_present();
            if self.backend.terminate_and_reap().is_err() {
                self.cleanup_failed = true;
                return Err(HistoryWorkerError::CleanupFailed);
            }
            self.has_process = false;
            if had_process {
                self.stats.reaped = self.stats.reaped.saturating_add(1);
            }
        }
        self.cleanup_failed = false;
        Ok(())
    }

    fn record_failure(&mut self, error: HistoryWorkerError) {
        self.stats.failures = self.stats.failures.saturating_add(1);
        if matches!(
            error,
            HistoryWorkerError::Protocol(
                protocol::ProtocolError::StartupTimeout
                    | protocol::ProtocolError::ReadTimeout
                    | protocol::ProtocolError::StopTimeout
            )
        ) {
            self.stats.timeouts = self.stats.timeouts.saturating_add(1);
        }
    }

    fn failed<T>(&mut self, error: HistoryWorkerError) -> Result<T, HistoryWorkerError> {
        self.record_failure(error);
        self.cleanup()?;
        Err(error)
    }

    fn check(&mut self, cancel: &AtomicBool) -> Result<(), HistoryWorkerError> {
        if cancel.load(Ordering::Acquire) {
            return Err(HistoryWorkerError::Cancelled);
        }
        self.machine
            .poll(self.backend.now_ms())
            .map_err(HistoryWorkerError::Protocol)
    }

    fn eof(&mut self) -> HistoryWorkerError {
        match self.decoder.end_of_stream() {
            Ok(()) => HistoryWorkerError::PeerExited,
            Err(error) => HistoryWorkerError::Protocol(error),
        }
    }

    fn receive(&mut self, cancel: &AtomicBool) -> Result<protocol::Reply, HistoryWorkerError> {
        loop {
            self.check(cancel)?;
            let mut bytes = [0; 4096];
            match self.backend.read(&mut bytes)? {
                ReadProgress::Bytes(count) if count > 0 && count <= bytes.len() => {
                    self.check(cancel)?;
                    if let Some(reply) = self
                        .decoder
                        .push(&bytes[..count])
                        .map_err(HistoryWorkerError::Protocol)?
                    {
                        return Ok(reply);
                    }
                }
                ReadProgress::Pending => {
                    if !self.backend.alive()? {
                        return Err(self.eof());
                    }
                    self.backend.wait_tick();
                }
                ReadProgress::Eof => return Err(self.eof()),
                _ => return Err(HistoryWorkerError::TransportFailed),
            }
        }
    }

    fn connect(&mut self, cancel: &AtomicBool) -> Result<(), HistoryWorkerError> {
        self.check(cancel)?;
        self.machine
            .begin_start(self.backend.now_ms())
            .map_err(HistoryWorkerError::Protocol)?;
        // 在调用前认领部分启动资源，确保失败返回也必经收尾。
        self.has_process = true;
        let started = self.backend.start();
        if self.backend.process_present() {
            self.stats.spawned = self.stats.spawned.saturating_add(1);
        }
        started?;
        let reply = self.receive(cancel)?;
        self.machine
            .accept(reply, self.backend.now_ms())
            .map_err(HistoryWorkerError::Protocol)?;
        Ok(())
    }

    fn start(&mut self, cancel: &AtomicBool) -> Result<(), HistoryWorkerError> {
        if self.cleanup_failed {
            return Err(HistoryWorkerError::CleanupFailed);
        }
        self.enabled = true;
        let result = if cancel.load(Ordering::Acquire) {
            Err(HistoryWorkerError::Cancelled)
        } else if self.has_process && self.machine.state() == protocol::WorkerState::Ready {
            self.backend.alive().and_then(|alive| {
                if alive {
                    Ok(())
                } else {
                    Err(HistoryWorkerError::PeerExited)
                }
            })
        } else {
            self.connect(cancel)
        };
        match result {
            Ok(()) => Ok(()),
            Err(error) => self.failed(error),
        }
    }

    fn capture_once(
        &mut self,
        previous: u32,
        cancel: &AtomicBool,
    ) -> Result<protocol::Reply, HistoryWorkerError> {
        self.check(cancel)?;
        if !self.has_process {
            self.connect(cancel)?;
        }
        // Ready期间不应有未请求的输出；不能将其留给下一次请求冒充成功。
        let mut unsolicited = [0; 1];
        match self.backend.read(&mut unsolicited)? {
            ReadProgress::Pending => {}
            ReadProgress::Eof => return Err(self.eof()),
            _ => {
                return Err(HistoryWorkerError::Protocol(
                    protocol::ProtocolError::UnexpectedReply,
                ))
            }
        }
        let request = self
            .machine
            .capture(previous, self.backend.now_ms())
            .map_err(HistoryWorkerError::Protocol)?;
        self.send_command(request, cancel)?;
        let reply = self.receive(cancel)?;
        let reply = self
            .machine
            .accept(reply, self.backend.now_ms())
            .map_err(HistoryWorkerError::Protocol)?
            .ok_or(HistoryWorkerError::Protocol(
                protocol::ProtocolError::UnexpectedReply,
            ))?;
        if let protocol::Reply::Failed {
            reason:
                reason @ (protocol::CaptureFailure::ScopeMismatch | protocol::CaptureFailure::Backend),
            ..
        } = &reply
        {
            return Err(HistoryWorkerError::Protocol(
                protocol::ProtocolError::PeerRefused(*reason),
            ));
        }
        if matches!(reply, protocol::Reply::Failed { .. }) {
            self.stats.failures = self.stats.failures.saturating_add(1);
        }
        Ok(reply)
    }

    fn send_command(
        &mut self,
        request: protocol::Request,
        cancel: &AtomicBool,
    ) -> Result<(), HistoryWorkerError> {
        let bytes = protocol::encode_request(request).map_err(HistoryWorkerError::Protocol)?;
        let mut sent = 0;
        while sent < bytes.len() {
            self.check(cancel)?;
            match self.backend.write(&bytes[sent..])? {
                WriteProgress::Bytes(count) if count > 0 && count <= bytes.len() - sent => {
                    sent += count
                }
                WriteProgress::Pending => self.backend.wait_tick(),
                _ => return Err(HistoryWorkerError::TransportFailed),
            }
        }
        Ok(())
    }

    fn capture(
        &mut self,
        previous: u32,
        cancel: &AtomicBool,
    ) -> Result<protocol::Reply, HistoryWorkerError> {
        if !self.enabled {
            return Err(HistoryWorkerError::NotStarted);
        }
        if self.cleanup_failed {
            return Err(HistoryWorkerError::CleanupFailed);
        }
        self.stats.captures = self.stats.captures.saturating_add(1);
        match self.capture_once(previous, cancel) {
            Ok(reply) => Ok(reply),
            Err(error) => self.failed(error),
        }
    }

    fn stop_idle(&mut self) -> Result<(), HistoryWorkerError> {
        let request = self
            .machine
            .begin_stop(self.backend.now_ms())
            .map_err(HistoryWorkerError::Protocol)?;
        let Some(request) = request else {
            return Ok(());
        };
        let cancel = AtomicBool::new(false);
        self.send_command(request, &cancel)?;
        let reply = self.receive(&cancel)?;
        self.machine
            .accept(reply, self.backend.now_ms())
            .map_err(HistoryWorkerError::Protocol)?;
        if self.machine.state() != protocol::WorkerState::Stopped {
            return Err(HistoryWorkerError::Protocol(
                protocol::ProtocolError::UnexpectedReply,
            ));
        }
        Ok(())
    }

    fn stop(&mut self) -> Result<HistoryWorkerStats, HistoryWorkerError> {
        self.enabled = false;
        // 只对健康空闲进程尝试有界Stop/Ack；渲染中的取消/错误及Drop仍直接kill。
        if self.has_process
            && !self.cleanup_failed
            && self.stop_grace_ms > 0
            && self.machine.state() == protocol::WorkerState::Ready
        {
            if let Err(error) = self.stop_idle() {
                self.record_failure(error);
            }
        }
        // Ack不是进程已退出的证据，任何路径都必须等待退出或终止后reap。
        self.cleanup()?;
        Ok(self.stats)
    }
}

impl<B: Transport> Drop for Session<B> {
    fn drop(&mut self) {
        self.enabled = false;
        let _ = self.cleanup();
    }
}

enum SnapshotContent {
    Text(String),
    Skip(protocol::SkipReason),
}
struct Snapshot {
    sequence: u32,
    content: SnapshotContent,
}

trait SnapshotSource {
    fn verify_scope(&mut self) -> Result<(), protocol::CaptureFailure>;
    fn capture(&mut self, previous: u32) -> Result<Snapshot, protocol::CaptureFailure>;
}

fn send_reply(output: &mut impl Write, reply: &protocol::Reply) -> Result<(), ()> {
    let bytes = protocol::encode_reply(reply).map_err(|_| ())?;
    output
        .write_all(&bytes)
        .and_then(|()| output.flush())
        .map_err(|_| ())
}

/// helper循环只操作显式输入。EOF在命令边界正常退出，半命令、重复ID明确失败。
fn serve(source: &mut impl SnapshotSource, input: &mut impl Read, output: &mut impl Write) -> i32 {
    if let Err(reason) = source.verify_scope() {
        let _ = send_reply(output, &protocol::Reply::StartFailed { reason });
        return 1;
    }
    if send_reply(output, &protocol::Reply::Ready).is_err() {
        return 2;
    }
    let mut previous_request = 0;
    loop {
        let mut bytes = [0; protocol::REQUEST_BYTES];
        match input.read_exact(&mut bytes[..1]) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return 0,
            Err(_) => return 2,
        }
        if input.read_exact(&mut bytes[1..]).is_err() {
            return 2;
        }
        let request = match protocol::decode_request(&bytes) {
            Ok(value) => value,
            Err(_) => return 2,
        };
        let protocol::Request::Capture {
            request_id,
            previous_sequence,
        } = request
        else {
            return if send_reply(output, &protocol::Reply::Stopped).is_ok() {
                0
            } else {
                2
            };
        };
        if request_id <= previous_request {
            return 2;
        }
        previous_request = request_id;
        if let Err(reason) = source.verify_scope() {
            let _ = send_reply(output, &protocol::Reply::Failed { request_id, reason });
            return 1;
        }
        let mut reply = match source.capture(previous_sequence) {
            Ok(Snapshot {
                sequence,
                content: SnapshotContent::Text(text),
            }) => protocol::Reply::Text {
                request_id,
                sequence,
                text,
            },
            Ok(Snapshot {
                sequence,
                content: SnapshotContent::Skip(reason),
            }) => protocol::Reply::Skipped {
                request_id,
                sequence,
                reason,
            },
            Err(reason) => protocol::Reply::Failed { request_id, reason },
        };
        if let Err(error) = protocol::encode_reply(&reply) {
            let reason = if error == protocol::ProtocolError::TooLarge {
                protocol::CaptureFailure::TooLarge
            } else {
                protocol::CaptureFailure::InvalidText
            };
            reply = protocol::Reply::Failed { request_id, reason };
        }
        if send_reply(output, &reply).is_err() {
            return 2;
        }
        if matches!(
            reply,
            protocol::Reply::Failed {
                reason: protocol::CaptureFailure::ScopeMismatch | protocol::CaptureFailure::Backend,
                ..
            }
        ) {
            return 1;
        }
    }
}

#[cfg(windows)]
pub use native::{run_clipboard_history_worker_helper, ClipboardHistoryWorker};

#[cfg(windows)]
mod native {
    use super::*;
    use std::ffi::c_void;
    use std::os::windows::{io::AsRawHandle, process::CommandExt};
    use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
    use std::time::{Duration, Instant};
    use windows::{
        core::w,
        Win32::{
            Foundation::HGLOBAL,
            System::{
                DataExchange::{
                    CloseClipboard, GetClipboardData, GetClipboardSequenceNumber,
                    IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW,
                },
                Memory::{GlobalLock, GlobalSize, GlobalUnlock},
                Threading::GetCurrentThreadId,
            },
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

    struct Process {
        child: Child,
        input: Option<ChildStdin>,
        output: Option<ChildStdout>,
    }
    struct NativeTransport {
        options: HistoryWorkerOptions,
        epoch: Instant,
        process: Option<Process>,
    }

    impl Transport for NativeTransport {
        fn now_ms(&self) -> u64 {
            self.epoch.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
        }
        fn process_present(&self) -> bool {
            self.process.is_some()
        }
        fn start(&mut self) -> Result<(), HistoryWorkerError> {
            if self.process.is_some() {
                return Err(HistoryWorkerError::TransportFailed);
            }
            let child = Command::new(&self.options.helper_executable)
                .arg("--omni-clipboard-history-worker")
                .arg(&self.options.scope.station)
                .arg(&self.options.scope.desktop)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .creation_flags(0x0800_0000)
                .spawn()
                .map_err(|_| HistoryWorkerError::SpawnFailed)?;
            self.process = Some(Process {
                child,
                input: None,
                output: None,
            });
            let process = self.process.as_mut().unwrap();
            process.input = process.child.stdin.take();
            process.output = process.child.stdout.take();
            if process.input.is_none() || process.output.is_none() {
                return Err(HistoryWorkerError::TransportFailed);
            }
            Ok(())
        }
        fn write(&mut self, bytes: &[u8]) -> Result<WriteProgress, HistoryWorkerError> {
            // 只有一个未完成命令，最多21字节。自有helper消费完整命令后才回复，
            // 因而延迟渲染发生时此管道已空；不能在未完成读取中再发送命令。
            if bytes.len() > protocol::REQUEST_BYTES {
                return Err(HistoryWorkerError::TransportFailed);
            }
            let input = self
                .process
                .as_mut()
                .and_then(|value| value.input.as_mut())
                .ok_or(HistoryWorkerError::TransportFailed)?;
            match input.write(bytes) {
                Ok(count) => Ok(WriteProgress::Bytes(count)),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    Ok(WriteProgress::Pending)
                }
                Err(_) => Err(HistoryWorkerError::TransportFailed),
            }
        }
        fn read(&mut self, bytes: &mut [u8]) -> Result<ReadProgress, HistoryWorkerError> {
            let output = self
                .process
                .as_mut()
                .and_then(|value| value.output.as_mut())
                .ok_or(HistoryWorkerError::TransportFailed)?;
            let mut available = 0;
            if unsafe {
                PeekNamedPipe(
                    output.as_raw_handle(),
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null_mut(),
                    &mut available,
                    std::ptr::null_mut(),
                )
            } == 0
            {
                return Ok(ReadProgress::Eof);
            }
            if available == 0 {
                return Ok(ReadProgress::Pending);
            }
            let count = (available as usize).min(bytes.len());
            match output.read(&mut bytes[..count]) {
                Ok(0) => Ok(ReadProgress::Eof),
                Ok(read) => Ok(ReadProgress::Bytes(read)),
                Err(_) => Err(HistoryWorkerError::TransportFailed),
            }
        }
        fn alive(&mut self) -> Result<bool, HistoryWorkerError> {
            match self.process.as_mut() {
                Some(process) => process
                    .child
                    .try_wait()
                    .map(|status| status.is_none())
                    .map_err(|_| HistoryWorkerError::TransportFailed),
                None => Ok(false),
            }
        }
        fn wait_tick(&mut self) {
            std::thread::sleep(Duration::from_millis(5));
        }
        fn terminate_and_reap(&mut self) -> Result<(), HistoryWorkerError> {
            let Some(process) = self.process.as_mut() else {
                return Ok(());
            };
            process.input.take();
            process.output.take();
            match process.child.try_wait() {
                Ok(Some(_)) => {}
                Ok(None) => {
                    if process.child.kill().is_err()
                        && !matches!(process.child.try_wait(), Ok(Some(_)))
                    {
                        return Err(HistoryWorkerError::CleanupFailed);
                    }
                }
                Err(_) => {
                    if process.child.kill().is_err() {
                        return Err(HistoryWorkerError::CleanupFailed);
                    }
                }
            }
            process
                .child
                .wait()
                .map_err(|_| HistoryWorkerError::CleanupFailed)?;
            self.process.take();
            Ok(())
        }
    }
    impl Drop for NativeTransport {
        fn drop(&mut self) {
            let _ = self.terminate_and_reap();
        }
    }

    /// new不spawn；start仅scope握手；capture显式读一次。超时后仅下次新capture可重建。
    pub struct ClipboardHistoryWorker {
        session: Session<NativeTransport>,
    }
    impl ClipboardHistoryWorker {
        pub fn new(options: HistoryWorkerOptions) -> Result<Self, HistoryWorkerError> {
            options.scope.validate()?;
            if !options.helper_executable.is_absolute() || !options.helper_executable.is_file() {
                return Err(HistoryWorkerError::InvalidOptions);
            }
            let budgets = options.budgets;
            Ok(Self {
                session: Session::new(
                    NativeTransport {
                        options,
                        epoch: Instant::now(),
                        process: None,
                    },
                    budgets,
                )?,
            })
        }
        pub fn start(&mut self, cancel: &AtomicBool) -> Result<(), HistoryWorkerError> {
            self.session.start(cancel)
        }
        pub fn capture(
            &mut self,
            previous: u32,
            cancel: &AtomicBool,
        ) -> Result<HistoryWorkerReply, HistoryWorkerError> {
            self.session.capture(previous, cancel)
        }
        pub fn stop(&mut self) -> Result<HistoryWorkerStats, HistoryWorkerError> {
            self.session.stop()
        }
        pub fn stats(&self) -> HistoryWorkerStats {
            self.session.stats
        }
    }

    fn object_name(object: isize) -> Result<String, protocol::CaptureFailure> {
        let mut units = [0_u16; 257];
        let mut needed = 0;
        if object == 0
            || unsafe {
                GetUserObjectInformationW(
                    object,
                    2,
                    units.as_mut_ptr().cast(),
                    std::mem::size_of_val(&units) as u32,
                    &mut needed,
                )
            } == 0
        {
            return Err(protocol::CaptureFailure::ScopeMismatch);
        }
        let count = units
            .iter()
            .position(|&unit| unit == 0)
            .ok_or(protocol::CaptureFailure::ScopeMismatch)?;
        let name = String::from_utf16(&units[..count])
            .map_err(|_| protocol::CaptureFailure::ScopeMismatch)?;
        if name.is_empty() {
            return Err(protocol::CaptureFailure::ScopeMismatch);
        }
        Ok(name)
    }
    fn current_scope() -> Result<ClipboardScope, protocol::CaptureFailure> {
        Ok(ClipboardScope {
            station: object_name(unsafe { GetProcessWindowStation() })?,
            desktop: object_name(unsafe { GetThreadDesktop(GetCurrentThreadId()) })?,
        })
    }
    impl ClipboardScope {
        pub fn current() -> Result<Self, HistoryWorkerError> {
            current_scope().map_err(|reason| {
                HistoryWorkerError::Protocol(protocol::ProtocolError::PeerRefused(reason))
            })
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
    fn enter_scope(expected: &ClipboardScope) -> Result<Scope, protocol::CaptureFailure> {
        expected
            .validate()
            .map_err(|_| protocol::CaptureFailure::ScopeMismatch)?;
        let mut scope = Scope {
            old_station: unsafe { GetProcessWindowStation() },
            old_desktop: unsafe { GetThreadDesktop(GetCurrentThreadId()) },
            station: 0,
            desktop: 0,
            station_changed: false,
            desktop_changed: false,
        };
        // 仅最早helper入口允许变更进程站；桌面不同不应重设同名站点句柄。
        if object_name(scope.old_station)? != expected.station {
            let station: Vec<u16> = expected.station.encode_utf16().chain([0]).collect();
            scope.station = unsafe { OpenWindowStationW(station.as_ptr(), 0, 0x27) };
            if scope.station == 0 || unsafe { SetProcessWindowStation(scope.station) } == 0 {
                return Err(protocol::CaptureFailure::ScopeMismatch);
            }
            scope.station_changed = true;
        }
        let desktop: Vec<u16> = expected.desktop.encode_utf16().chain([0]).collect();
        scope.desktop = unsafe { OpenDesktopW(desktop.as_ptr(), 0, 0, 0x83) };
        if scope.desktop == 0 || unsafe { SetThreadDesktop(scope.desktop) } == 0 {
            return Err(protocol::CaptureFailure::ScopeMismatch);
        }
        scope.desktop_changed = true;
        if &current_scope()? != expected {
            return Err(protocol::CaptureFailure::ScopeMismatch);
        }
        Ok(scope)
    }

    struct NativeSource {
        expected: ClipboardScope,
        _scope: Scope,
        formats: [u32; 4],
    }
    impl NativeSource {
        fn new(expected: ClipboardScope) -> Result<Self, protocol::CaptureFailure> {
            let scope = enter_scope(&expected)?;
            let formats = [
                w!("OmniTrigger.TransientClipboard.v1"),
                w!("ExcludeClipboardContentFromMonitorProcessing"),
                w!("PNG"),
                w!("image/png"),
            ]
            .map(|name| unsafe { RegisterClipboardFormatW(name) });
            if formats.contains(&0) {
                return Err(protocol::CaptureFailure::Backend);
            }
            Ok(Self {
                expected,
                _scope: scope,
                formats,
            })
        }
        fn read_open(&mut self, previous: u32) -> Result<Snapshot, protocol::CaptureFailure> {
            let sequence = unsafe { GetClipboardSequenceNumber() };
            let available = |format| unsafe { IsClipboardFormatAvailable(format) }.is_ok();
            let skip = if sequence == previous {
                Some(protocol::SkipReason::Duplicate)
            } else if available(self.formats[0]) {
                Some(protocol::SkipReason::Transient)
            } else if available(self.formats[1]) {
                Some(protocol::SkipReason::Excluded)
            } else if available(15) {
                Some(protocol::SkipReason::Files)
            } else if [2, 3, 8, 14, 17, self.formats[2], self.formats[3]]
                .into_iter()
                .any(available)
            {
                Some(protocol::SkipReason::Image)
            } else if !available(13) {
                Some(protocol::SkipReason::Unsupported)
            } else {
                None
            };
            if let Some(reason) = skip {
                return Ok(Snapshot {
                    sequence,
                    content: SnapshotContent::Skip(reason),
                });
            }
            let memory = HGLOBAL(
                unsafe { GetClipboardData(13) }
                    .map_err(|_| protocol::CaptureFailure::Backend)?
                    .0,
            );
            let size = unsafe { GlobalSize(memory) };
            if size < 2 || size % 2 != 0 {
                return Err(protocol::CaptureFailure::InvalidText);
            }
            let pointer = unsafe { GlobalLock(memory) };
            if pointer.is_null() {
                return Err(protocol::CaptureFailure::Backend);
            }
            struct Unlock(HGLOBAL);
            impl Drop for Unlock {
                fn drop(&mut self) {
                    let _ = unsafe { GlobalUnlock(self.0) };
                }
            }
            let _unlock = Unlock(memory);
            let units = unsafe {
                std::slice::from_raw_parts(
                    pointer.cast::<u16>(),
                    (size / 2).min(protocol::MAX_UNITS + 1),
                )
            };
            let count = units
                .iter()
                .position(|&unit| unit == 0)
                .ok_or(protocol::CaptureFailure::TooLarge)?;
            let text = String::from_utf16(&units[..count])
                .map_err(|_| protocol::CaptureFailure::InvalidText)?;
            if unsafe { GetClipboardSequenceNumber() } != sequence {
                return Err(protocol::CaptureFailure::Changed);
            }
            Ok(Snapshot {
                sequence,
                content: SnapshotContent::Text(text),
            })
        }
    }
    impl SnapshotSource for NativeSource {
        fn verify_scope(&mut self) -> Result<(), protocol::CaptureFailure> {
            if current_scope()? == self.expected {
                Ok(())
            } else {
                Err(protocol::CaptureFailure::ScopeMismatch)
            }
        }
        fn capture(&mut self, previous: u32) -> Result<Snapshot, protocol::CaptureFailure> {
            for attempt in 0..3 {
                if unsafe { OpenClipboard(None) }.is_ok() {
                    struct Lease(bool);
                    impl Drop for Lease {
                        fn drop(&mut self) {
                            if self.0 {
                                let _ = unsafe { CloseClipboard() };
                            }
                        }
                    }
                    let mut lease = Lease(true);
                    let result = self.read_open(previous);
                    lease.0 = false;
                    let closed =
                        unsafe { CloseClipboard() }.map_err(|_| protocol::CaptureFailure::Backend);
                    return result.and_then(|snapshot| {
                        closed?;
                        Ok(snapshot)
                    });
                }
                if attempt < 2 {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            Err(protocol::CaptureFailure::Busy)
        }
    }

    /// 内部helper入口，必须在UI、插件、其他线程初始化前分派并立即以返回码退出。
    pub fn run_clipboard_history_worker_helper(station: &str, desktop: &str) -> i32 {
        let mut output = std::io::stdout().lock();
        let mut source = match NativeSource::new(ClipboardScope {
            station: station.into(),
            desktop: desktop.into(),
        }) {
            Ok(source) => source,
            Err(reason) => {
                let _ = send_reply(&mut output, &protocol::Reply::StartFailed { reason });
                return 1;
            }
        };
        serve(&mut source, &mut std::io::stdin().lock(), &mut output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    enum Action {
        Text(u32, &'static str),
        Busy,
        Raw(Vec<u8>),
        RawExit(Vec<u8>),
        Hang,
        Exit,
    }
    #[derive(Default)]
    struct Audit {
        spawns: usize,
        reaps: usize,
        requests: Vec<u64>,
        cleanup_calls: usize,
        stop_requests: usize,
    }
    struct Fake {
        now: u64,
        present: bool,
        exited: bool,
        output: VecDeque<u8>,
        input: Vec<u8>,
        actions: VecDeque<Action>,
        audit: Arc<Mutex<Audit>>,
        startup_hang: bool,
        partial_start_failure: bool,
        cleanup_failures: usize,
        write_chunk: usize,
        read_chunk: usize,
        write_pending: usize,
        cancel_at: Option<(u64, Arc<AtomicBool>)>,
        stop_reply: Option<Vec<u8>>,
    }
    impl Fake {
        fn new(actions: Vec<Action>) -> Self {
            Self {
                now: 0,
                present: false,
                exited: false,
                output: VecDeque::new(),
                input: vec![],
                actions: actions.into(),
                audit: Arc::new(Mutex::new(Audit::default())),
                startup_hang: false,
                partial_start_failure: false,
                cleanup_failures: 0,
                write_chunk: protocol::REQUEST_BYTES,
                read_chunk: 4096,
                write_pending: 0,
                cancel_at: None,
                stop_reply: Some(protocol::encode_reply(&protocol::Reply::Stopped).unwrap()),
            }
        }
        fn reply(&mut self, reply: protocol::Reply) {
            self.output.extend(protocol::encode_reply(&reply).unwrap());
        }
    }
    impl Transport for Fake {
        fn now_ms(&self) -> u64 {
            self.now
        }
        fn process_present(&self) -> bool {
            self.present
        }
        fn start(&mut self) -> Result<(), HistoryWorkerError> {
            assert!(!self.present);
            self.present = true;
            self.exited = false;
            self.audit.lock().unwrap().spawns += 1;
            if self.partial_start_failure {
                return Err(HistoryWorkerError::SpawnFailed);
            }
            if !self.startup_hang {
                self.reply(protocol::Reply::Ready);
            }
            Ok(())
        }
        fn write(&mut self, bytes: &[u8]) -> Result<WriteProgress, HistoryWorkerError> {
            if self.write_pending > 0 {
                self.write_pending -= 1;
                return Ok(WriteProgress::Pending);
            }
            let count = bytes.len().min(self.write_chunk);
            self.input.extend_from_slice(&bytes[..count]);
            if self.input.len() == protocol::REQUEST_BYTES {
                let request = protocol::decode_request(&self.input).unwrap();
                self.input.clear();
                let request_id = match request {
                    protocol::Request::Capture { request_id, .. } => request_id,
                    protocol::Request::Stop => {
                        self.audit.lock().unwrap().stop_requests += 1;
                        if let Some(bytes) = &self.stop_reply {
                            self.output.extend(bytes);
                        }
                        return Ok(WriteProgress::Bytes(count));
                    }
                };
                self.audit.lock().unwrap().requests.push(request_id);
                match self.actions.pop_front().expect("合成脚本不能被隐式重试") {
                    Action::Text(sequence, text) => self.reply(protocol::Reply::Text {
                        request_id,
                        sequence,
                        text: text.into(),
                    }),
                    Action::Busy => self.reply(protocol::Reply::Failed {
                        request_id,
                        reason: protocol::CaptureFailure::Busy,
                    }),
                    Action::Raw(bytes) => self.output.extend(bytes),
                    Action::RawExit(bytes) => {
                        self.output.extend(bytes);
                        self.exited = true;
                    }
                    Action::Hang => {}
                    Action::Exit => self.exited = true,
                }
            }
            Ok(WriteProgress::Bytes(count))
        }
        fn read(&mut self, bytes: &mut [u8]) -> Result<ReadProgress, HistoryWorkerError> {
            let count = bytes.len().min(self.read_chunk).min(self.output.len());
            if count > 0 {
                for byte in &mut bytes[..count] {
                    *byte = self.output.pop_front().unwrap();
                }
                return Ok(ReadProgress::Bytes(count));
            }
            Ok(if self.exited {
                ReadProgress::Eof
            } else {
                ReadProgress::Pending
            })
        }
        fn alive(&mut self) -> Result<bool, HistoryWorkerError> {
            Ok(self.present && !self.exited)
        }
        fn wait_tick(&mut self) {
            self.now += 1;
            if let Some((time, flag)) = &self.cancel_at {
                if self.now >= *time {
                    flag.store(true, Ordering::Release);
                }
            }
        }
        fn terminate_and_reap(&mut self) -> Result<(), HistoryWorkerError> {
            self.audit.lock().unwrap().cleanup_calls += 1;
            if self.cleanup_failures > 0 {
                self.cleanup_failures -= 1;
                return Err(HistoryWorkerError::CleanupFailed);
            }
            if self.present {
                self.audit.lock().unwrap().reaps += 1;
            }
            self.present = false;
            self.exited = false;
            self.output.clear();
            self.input.clear();
            Ok(())
        }
    }
    fn budgets() -> HistoryWorkerBudgets {
        HistoryWorkerBudgets {
            startup_ms: 10,
            read_ms: 5,
            stop_grace_ms: 0,
        }
    }
    fn new_session(actions: Vec<Action>) -> Session<Fake> {
        Session::new(Fake::new(actions), budgets()).unwrap()
    }
    fn flag() -> AtomicBool {
        AtomicBool::new(false)
    }

    #[test]
    fn constructor_is_inert_and_capture_requires_explicit_start() {
        let mut session = new_session(vec![]);
        assert_eq!(session.stats.spawned, 0);
        assert_eq!(
            session.capture(0, &flag()),
            Err(HistoryWorkerError::NotStarted)
        );
        assert_eq!(session.backend.audit.lock().unwrap().spawns, 0);
    }
    #[test]
    fn start_only_handshakes_and_many_captures_reuse_one_process() {
        let mut session = new_session(vec![
            Action::Text(1, "synthetic one"),
            Action::Text(2, ""),
            Action::Text(3, " 文😀 "),
        ]);
        session.start(&flag()).unwrap();
        session.start(&flag()).unwrap();
        assert!(session.backend.audit.lock().unwrap().requests.is_empty());
        for sequence in 1..=3 {
            assert!(
                matches!(session.capture(sequence - 1, &flag()).unwrap(), protocol::Reply::Text { request_id, sequence: value, .. }
                if request_id == u64::from(sequence) && value == sequence)
            );
        }
        let stats = session.stop().unwrap();
        assert_eq!((stats.spawned, stats.reaped, stats.captures), (1, 1, 3));
    }
    #[test]
    fn delayed_reply_timeout_reaps_once_and_does_not_retry_current_request() {
        let mut session = new_session(vec![
            Action::Hang,
            Action::Text(2, "synthetic later request"),
        ]);
        session.start(&flag()).unwrap();
        assert_eq!(
            session.capture(0, &flag()),
            Err(HistoryWorkerError::Protocol(
                protocol::ProtocolError::ReadTimeout
            ))
        );
        assert_eq!(session.backend.audit.lock().unwrap().requests, [1]);
        assert_eq!(
            (
                session.stats.spawned,
                session.stats.reaped,
                session.stats.timeouts
            ),
            (1, 1, 1)
        );
        assert!(matches!(
            session.capture(1, &flag()).unwrap(),
            protocol::Reply::Text { request_id: 2, .. }
        ));
        assert_eq!(session.stats.spawned, 2);
    }
    #[test]
    fn cancellation_before_start_spawns_nothing_and_during_read_reaps() {
        let mut session = new_session(vec![Action::Hang]);
        assert_eq!(
            session.start(&AtomicBool::new(true)),
            Err(HistoryWorkerError::Cancelled)
        );
        assert_eq!(session.stats.spawned, 0);
        session.start(&flag()).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        session.backend.cancel_at = Some((2, cancel.clone()));
        assert_eq!(
            session.capture(0, &cancel),
            Err(HistoryWorkerError::Cancelled)
        );
        assert_eq!(session.stats.reaped, 1);
    }
    #[test]
    fn startup_timeout_and_partial_start_failure_clean_up_without_second_spawn() {
        for partial in [false, true] {
            let mut backend = Fake::new(vec![]);
            backend.startup_hang = true;
            backend.partial_start_failure = partial;
            let mut session = Session::new(backend, budgets()).unwrap();
            assert!(session.start(&flag()).is_err());
            assert_eq!(
                (
                    session.backend.audit.lock().unwrap().spawns,
                    session.stats.reaped
                ),
                (1, 1)
            );
            assert!(!session.backend.present);
        }
    }
    #[test]
    fn malformed_or_wrong_id_reply_always_reaps_and_discards_all_old_bytes() {
        for bytes in [
            b"bad-magic".to_vec(),
            protocol::encode_reply(&protocol::Reply::Text {
                request_id: 99,
                sequence: 1,
                text: "synthetic wrong id".into(),
            })
            .unwrap(),
            [
                protocol::encode_reply(&protocol::Reply::Ready).unwrap(),
                vec![0],
            ]
            .concat(),
        ] {
            let mut session =
                new_session(vec![Action::Raw(bytes), Action::Text(2, "synthetic valid")]);
            session.start(&flag()).unwrap();
            assert!(session.capture(0, &flag()).is_err());
            assert!(!session.backend.present);
            assert!(matches!(
                session.capture(1, &flag()).unwrap(),
                protocol::Reply::Text { request_id: 2, .. }
            ));
        }
    }
    #[test]
    fn partial_frame_hang_and_eof_do_not_return_partial_body() {
        let frame = protocol::encode_reply(&protocol::Reply::Text {
            request_id: 1,
            sequence: 1,
            text: "synthetic".into(),
        })
        .unwrap();
        let mut session = new_session(vec![Action::Raw(
            frame[..protocol::REPLY_HEADER + 1].to_vec(),
        )]);
        session.start(&flag()).unwrap();
        assert_eq!(
            session.capture(0, &flag()),
            Err(HistoryWorkerError::Protocol(
                protocol::ProtocolError::ReadTimeout
            ))
        );
        let mut session = new_session(vec![Action::Exit]);
        session.start(&flag()).unwrap();
        assert_eq!(
            session.capture(0, &flag()),
            Err(HistoryWorkerError::PeerExited)
        );
    }
    #[test]
    fn healthy_busy_error_waits_for_next_explicit_request_without_respawn() {
        let mut session = new_session(vec![Action::Busy, Action::Text(2, "synthetic after busy")]);
        session.start(&flag()).unwrap();
        assert!(matches!(
            session.capture(0, &flag()).unwrap(),
            protocol::Reply::Failed {
                reason: protocol::CaptureFailure::Busy,
                ..
            }
        ));
        assert_eq!(session.backend.audit.lock().unwrap().requests, [1]);
        assert!(session.capture(1, &flag()).is_ok());
        assert_eq!(session.stats.spawned, 1);
    }
    #[test]
    fn scope_error_destroys_connection_and_cannot_be_treated_as_healthy_busy() {
        let bytes = protocol::encode_reply(&protocol::Reply::Failed {
            request_id: 1,
            reason: protocol::CaptureFailure::ScopeMismatch,
        })
        .unwrap();
        let mut session = new_session(vec![Action::Raw(bytes)]);
        session.start(&flag()).unwrap();
        assert_eq!(
            session.capture(0, &flag()),
            Err(HistoryWorkerError::Protocol(
                protocol::ProtocolError::PeerRefused(protocol::CaptureFailure::ScopeMismatch)
            ))
        );
        assert_eq!(session.stats.reaped, 1);
    }
    #[test]
    fn partial_pipe_writes_and_bytewise_responses_do_not_duplicate_commands() {
        let mut backend = Fake::new(vec![Action::Text(1, "synthetic")]);
        backend.write_chunk = 1;
        backend.read_chunk = 1;
        backend.write_pending = 2;
        let mut session = Session::new(backend, budgets()).unwrap();
        session.start(&flag()).unwrap();
        assert!(session.capture(0, &flag()).is_ok());
        assert_eq!(session.backend.audit.lock().unwrap().requests, [1]);
    }
    #[test]
    fn unsolicited_idle_output_is_not_borrowed_for_a_future_request() {
        let mut session = new_session(vec![]);
        session.start(&flag()).unwrap();
        session.backend.reply(protocol::Reply::Text {
            request_id: 1,
            sequence: 1,
            text: "synthetic unsolicited".into(),
        });
        assert_eq!(
            session.capture(0, &flag()),
            Err(HistoryWorkerError::Protocol(
                protocol::ProtocolError::UnexpectedReply
            ))
        );
        assert!(session.backend.audit.lock().unwrap().requests.is_empty());
    }
    #[test]
    fn failed_reap_blocks_rebuild_until_explicit_stop_succeeds() {
        let mut session = new_session(vec![Action::Hang]);
        session.start(&flag()).unwrap();
        session.backend.cleanup_failures = 1;
        assert_eq!(
            session.capture(0, &flag()),
            Err(HistoryWorkerError::CleanupFailed)
        );
        assert_eq!(
            session.capture(0, &flag()),
            Err(HistoryWorkerError::CleanupFailed)
        );
        assert_eq!(session.backend.audit.lock().unwrap().spawns, 1);
        session.stop().unwrap();
        assert_eq!(
            session.capture(0, &flag()),
            Err(HistoryWorkerError::NotStarted)
        );
    }
    #[test]
    fn drop_stops_idle_process_and_stop_is_idempotent() {
        let mut session = new_session(vec![]);
        let audit = session.backend.audit.clone();
        session.start(&flag()).unwrap();
        session.stop().unwrap();
        session.stop().unwrap();
        drop(session);
        assert_eq!(audit.lock().unwrap().reaps, 1);
        let mut session = new_session(vec![]);
        let audit = session.backend.audit.clone();
        session.start(&flag()).unwrap();
        drop(session);
        assert_eq!(audit.lock().unwrap().reaps, 1);
    }

    struct Source {
        captures: Vec<u32>,
        scopes: VecDeque<Result<(), protocol::CaptureFailure>>,
        values: VecDeque<Result<Snapshot, protocol::CaptureFailure>>,
    }
    impl Source {
        fn new() -> Self {
            Self {
                captures: vec![],
                scopes: VecDeque::new(),
                values: VecDeque::new(),
            }
        }
    }
    impl SnapshotSource for Source {
        fn verify_scope(&mut self) -> Result<(), protocol::CaptureFailure> {
            self.scopes.pop_front().unwrap_or(Ok(()))
        }
        fn capture(&mut self, previous: u32) -> Result<Snapshot, protocol::CaptureFailure> {
            self.captures.push(previous);
            self.values.pop_front().expect("不能发生未请求采集")
        }
    }
    fn commands(requests: &[protocol::Request]) -> Vec<u8> {
        requests
            .iter()
            .flat_map(|&request| protocol::encode_request(request).unwrap())
            .collect()
    }
    fn frames(bytes: &[u8]) -> Vec<protocol::Reply> {
        let mut result = vec![];
        let mut offset = 0;
        while offset < bytes.len() {
            let count =
                u32::from_le_bytes(bytes[offset + 21..offset + 25].try_into().unwrap()) as usize;
            let end = offset + protocol::REPLY_HEADER + count * 2;
            result.push(
                protocol::Decoder::default()
                    .push(&bytes[offset..end])
                    .unwrap()
                    .unwrap(),
            );
            offset = end;
        }
        result
    }
    #[test]
    fn helper_ready_eof_and_stop_do_not_capture_existing_clipboard() {
        for input in [vec![], commands(&[protocol::Request::Stop])] {
            let mut source = Source::new();
            let mut output = vec![];
            assert_eq!(serve(&mut source, &mut input.as_slice(), &mut output), 0);
            assert!(source.captures.is_empty());
            assert_eq!(frames(&output)[0], protocol::Reply::Ready);
        }
    }
    #[test]
    fn helper_serves_multiple_requests_preserving_ids_text_and_skip() {
        let mut source = Source::new();
        source.values = VecDeque::from([
            Ok(Snapshot {
                sequence: 1,
                content: SnapshotContent::Text(" 合成😀 ".into()),
            }),
            Ok(Snapshot {
                sequence: 2,
                content: SnapshotContent::Skip(protocol::SkipReason::Transient),
            }),
        ]);
        let input = commands(&[
            protocol::Request::Capture {
                request_id: 1,
                previous_sequence: 0,
            },
            protocol::Request::Capture {
                request_id: 2,
                previous_sequence: 1,
            },
            protocol::Request::Stop,
        ]);
        let mut output = vec![];
        assert_eq!(serve(&mut source, &mut input.as_slice(), &mut output), 0);
        assert_eq!(source.captures, [0, 1]);
        assert_eq!(
            frames(&output),
            [
                protocol::Reply::Ready,
                protocol::Reply::Text {
                    request_id: 1,
                    sequence: 1,
                    text: " 合成😀 ".into()
                },
                protocol::Reply::Skipped {
                    request_id: 2,
                    sequence: 2,
                    reason: protocol::SkipReason::Transient
                },
                protocol::Reply::Stopped
            ]
        );
    }
    #[test]
    fn helper_refuses_start_or_changed_scope_before_any_capture() {
        for startup in [false, true] {
            let mut source = Source::new();
            source.scopes = if startup {
                VecDeque::from([Err(protocol::CaptureFailure::ScopeMismatch)])
            } else {
                VecDeque::from([Ok(()), Err(protocol::CaptureFailure::ScopeMismatch)])
            };
            let input = commands(&[protocol::Request::Capture {
                request_id: 1,
                previous_sequence: 0,
            }]);
            let mut output = vec![];
            assert_eq!(serve(&mut source, &mut input.as_slice(), &mut output), 1);
            assert!(source.captures.is_empty());
        }
    }
    #[test]
    fn helper_rejects_partial_duplicate_or_decreasing_commands_without_replay() {
        let mut source = Source::new();
        let input = commands(&[protocol::Request::Capture {
            request_id: 1,
            previous_sequence: 0,
        }]);
        assert_eq!(serve(&mut source, &mut &input[..5], &mut vec![]), 2);
        assert!(source.captures.is_empty());
        for second in [1, 2] {
            let mut source = Source::new();
            source.values.push_back(Ok(Snapshot {
                sequence: 1,
                content: SnapshotContent::Text("synthetic".into()),
            }));
            let input = commands(&[
                protocol::Request::Capture {
                    request_id: 2,
                    previous_sequence: 0,
                },
                protocol::Request::Capture {
                    request_id: second,
                    previous_sequence: 1,
                },
            ]);
            assert_eq!(serve(&mut source, &mut input.as_slice(), &mut vec![]), 2);
            assert_eq!(source.captures, [0]);
        }
    }
    #[test]
    fn helper_sanitizes_invalid_source_text_into_fixed_error_without_body() {
        for text in [
            "synthetic\0private".into(),
            "x".repeat(protocol::MAX_UNITS + 1),
        ] {
            let mut source = Source::new();
            source.values.push_back(Ok(Snapshot {
                sequence: 1,
                content: SnapshotContent::Text(text),
            }));
            let input = commands(&[protocol::Request::Capture {
                request_id: 1,
                previous_sequence: 0,
            }]);
            let mut output = vec![];
            assert_eq!(serve(&mut source, &mut input.as_slice(), &mut output), 0);
            assert_eq!(output.len(), protocol::REPLY_HEADER * 2);
            assert!(matches!(frames(&output)[1], protocol::Reply::Failed { .. }));
        }
    }
    #[test]
    fn invalid_scope_names_are_rejected_without_platform_calls() {
        for station in ["", "bad\\station", "bad\0station"] {
            assert_eq!(
                ClipboardScope {
                    station: station.into(),
                    desktop: "Default".into()
                }
                .validate(),
                Err(HistoryWorkerError::InvalidOptions)
            );
        }
    }

    #[test]
    fn healthy_idle_stop_sends_one_stop_accepts_ack_and_still_reaps() {
        let mut options = budgets();
        options.stop_grace_ms = 5;
        let mut session = Session::new(Fake::new(vec![]), options).unwrap();
        session.start(&flag()).unwrap();
        let stats = session.stop().unwrap();
        assert_eq!(
            (stats.spawned, stats.reaped, stats.captures, stats.failures),
            (1, 1, 0, 0)
        );
        assert_eq!(session.backend.audit.lock().unwrap().stop_requests, 1);
        assert!(session.backend.audit.lock().unwrap().requests.is_empty());
        assert_eq!(session.stop().unwrap(), stats);
        assert_eq!(session.backend.audit.lock().unwrap().stop_requests, 1);
    }

    #[test]
    fn unresponsive_idle_stop_has_bounded_grace_then_force_reaps_without_retry() {
        let mut backend = Fake::new(vec![]);
        backend.stop_reply = None;
        let mut options = budgets();
        options.stop_grace_ms = 5;
        let mut session = Session::new(backend, options).unwrap();
        session.start(&flag()).unwrap();
        let stats = session.stop().unwrap();
        assert_eq!(session.backend.now, 5);
        assert_eq!((stats.reaped, stats.failures, stats.timeouts), (1, 1, 1));
        assert_eq!(session.backend.audit.lock().unwrap().stop_requests, 1);
        assert!(session.backend.audit.lock().unwrap().requests.is_empty());
    }

    #[test]
    fn text_instead_of_stop_ack_is_discarded_and_process_is_force_reaped() {
        let mut backend = Fake::new(vec![]);
        backend.stop_reply = Some(
            protocol::encode_reply(&protocol::Reply::Text {
                request_id: 99,
                sequence: 1,
                text: "synthetic unwanted shutdown body".into(),
            })
            .unwrap(),
        );
        let mut options = budgets();
        options.stop_grace_ms = 5;
        let mut session = Session::new(backend, options).unwrap();
        session.start(&flag()).unwrap();
        let stats = session.stop().unwrap();
        assert_eq!((stats.captures, stats.reaped, stats.failures), (0, 1, 1));
    }

    #[test]
    fn rendering_cancellation_never_waits_for_stop_ack() {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut backend = Fake::new(vec![Action::Hang]);
        backend.cancel_at = Some((2, cancel.clone()));
        let mut options = budgets();
        options.stop_grace_ms = 5000;
        let mut session = Session::new(backend, options).unwrap();
        session.start(&flag()).unwrap();
        assert_eq!(
            session.capture(0, &cancel),
            Err(HistoryWorkerError::Cancelled)
        );
        assert_eq!(session.backend.now, 2);
        assert_eq!(session.backend.audit.lock().unwrap().stop_requests, 0);
        assert_eq!(session.stats.reaped, 1);
    }

    #[test]
    fn eof_after_partial_reply_runs_decoder_end_of_stream_and_never_returns_body() {
        let frame = protocol::encode_reply(&protocol::Reply::Text {
            request_id: 1,
            sequence: 1,
            text: "synthetic".into(),
        })
        .unwrap();
        let mut session = new_session(vec![Action::RawExit(
            frame[..protocol::REPLY_HEADER + 1].to_vec(),
        )]);
        session.start(&flag()).unwrap();
        assert_eq!(
            session.capture(0, &flag()),
            Err(HistoryWorkerError::Protocol(
                protocol::ProtocolError::Truncated
            ))
        );
        assert_eq!(session.stats.reaped, 1);
    }
}
