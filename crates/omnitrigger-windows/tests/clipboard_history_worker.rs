//! 持续历史辅助进程的真实协议验收；只在CREATE_ONLY非交互窗口站使用合成载荷。
//! 默认忽略，须显式指定已构建的自有helper并在独立测试进程中串行执行；禁止交互回退。
#![cfg(windows)]

use omnitrigger_windows::{
    ClipboardHistoryWorker, ClipboardScope, HistoryCaptureFailure, HistoryWorkerBudgets,
    HistoryWorkerError, HistoryWorkerOptions, HistoryWorkerProtocolError, HistoryWorkerReply,
    HistoryWorkerSkip,
};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use windows::{
    core::w,
    Win32::{
        Foundation::{GetLastError, GlobalFree, SetLastError, HANDLE, HWND, WIN32_ERROR},
        System::{
            DataExchange::{
                CloseClipboard, EmptyClipboard, GetClipboardSequenceNumber, OpenClipboard,
                RegisterClipboardFormatW, SetClipboardData,
            },
            Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE},
        },
        UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE,
        },
    },
};

#[path = "support/private_clipboard.rs"]
mod private_clipboard;
use private_clipboard::PrivateClipboard;

struct Owner(HWND);
impl Owner {
    fn create(scope: &PrivateClipboard) -> Self {
        scope.verify();
        Self(
            unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("STATIC"),
                    w!("Synthetic persistent history owner"),
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
            .unwrap(),
        )
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        let _ = unsafe { DestroyWindow(self.0) };
    }
}
struct Lease;
impl Drop for Lease {
    fn drop(&mut self) {
        let _ = unsafe { CloseClipboard() };
    }
}

fn helper() -> PathBuf {
    let path = PathBuf::from(
        std::env::var_os("OMNITRIGGER_HISTORY_HELPER_EXE")
            .expect("必须显式提供已构建且接入持续历史入口的自有程序，不能猜测或回退"),
    );
    assert!(path.is_absolute() && path.is_file());
    path
}
fn worker(scope: &PrivateClipboard, read_ms: u64) -> ClipboardHistoryWorker {
    scope.verify();
    ClipboardHistoryWorker::new(HistoryWorkerOptions {
        helper_executable: helper(),
        scope: ClipboardScope::current().unwrap(),
        // 验收预算，不表示已验证产品冷启动或读取性能。
        budgets: HistoryWorkerBudgets {
            startup_ms: 5000,
            read_ms,
            stop_grace_ms: 200,
        },
    })
    .unwrap()
}
fn wide(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect()
}
fn sequence(scope: &PrivateClipboard) -> u32 {
    scope.verify();
    unsafe { GetClipboardSequenceNumber() }
}
fn publish(scope: &PrivateClipboard, owner: &Owner, formats: &[(u32, Vec<u8>)]) {
    scope.verify();
    unsafe {
        OpenClipboard(Some(owner.0)).unwrap();
        let _lease = Lease;
        EmptyClipboard().unwrap();
        for (format, bytes) in formats {
            let memory = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(2)).unwrap();
            let pointer = GlobalLock(memory);
            if pointer.is_null() {
                let _ = GlobalFree(Some(memory));
                panic!("合成内存不可用");
            }
            std::ptr::write_bytes(pointer.cast::<u8>(), 0x61, GlobalSize(memory));
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast::<u8>(), bytes.len());
            let _ = GlobalUnlock(memory);
            if SetClipboardData(*format, Some(HANDLE(memory.0))).is_err() {
                let _ = GlobalFree(Some(memory));
                panic!("无法发布合成格式");
            }
        }
    }
    scope.verify();
}
fn delayed(scope: &PrivateClipboard, owner: &Owner) {
    scope.verify();
    unsafe {
        OpenClipboard(Some(owner.0)).unwrap();
        let _lease = Lease;
        EmptyClipboard().unwrap();
        SetLastError(WIN32_ERROR(0));
        let result = SetClipboardData(13, None);
        assert!(result.is_ok() || GetLastError() == WIN32_ERROR(0));
    }
    scope.verify();
}
fn assert_text(reply: HistoryWorkerReply, expected: &str) {
    let HistoryWorkerReply::Text { text, .. } = reply else {
        panic!("预期合成文本回复");
    };
    assert_eq!(text, expected);
}

#[test]
#[ignore = "需要自有持续helper与非交互窗口站权限；单独进程串行执行"]
fn isolated_worker_ready_does_not_import_and_multiple_captures_reuse_process() {
    let scope = PrivateClipboard::create();
    let owner = Owner::create(&scope);
    publish(&scope, &owner, &[(13, wide("synthetic existing body"))]);
    let baseline = sequence(&scope);
    let cancel = AtomicBool::new(false);
    let mut worker = worker(&scope, 5000);
    assert_eq!(worker.stats().spawned, 0);
    worker.start(&cancel).unwrap();
    worker.start(&cancel).unwrap();
    scope.verify();
    assert_eq!(worker.stats().captures, 0);
    assert!(matches!(
        worker.capture(baseline, &cancel).unwrap(),
        HistoryWorkerReply::Skipped {
            reason: HistoryWorkerSkip::Duplicate,
            ..
        }
    ));
    let mut previous = baseline;
    for value in ["合成 文😀", " \r\n\t ", ""] {
        publish(&scope, &owner, &[(13, wide(value))]);
        assert_text(worker.capture(previous, &cancel).unwrap(), value);
        previous = sequence(&scope);
        scope.verify();
    }
    let stats = worker.stop().unwrap();
    assert_eq!((stats.spawned, stats.reaped), (1, 1));
    assert_eq!(worker.stop().unwrap(), stats);
    assert_eq!(
        worker.capture(previous, &cancel),
        Err(HistoryWorkerError::NotStarted)
    );
    scope.verify();
}

#[test]
#[ignore = "需要自有持续helper与非交互窗口站权限；不读取文件或图片载荷"]
fn isolated_worker_skips_transient_excluded_files_and_images_before_text() {
    let scope = PrivateClipboard::create();
    let owner = Owner::create(&scope);
    let mut previous = sequence(&scope);
    let cancel = AtomicBool::new(false);
    let mut worker = worker(&scope, 5000);
    worker.start(&cancel).unwrap();
    let transient = unsafe { RegisterClipboardFormatW(w!("OmniTrigger.TransientClipboard.v1")) };
    let excluded =
        unsafe { RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing")) };
    let png = unsafe { RegisterClipboardFormatW(w!("PNG")) };
    assert!(![transient, excluded, png].contains(&0));
    let mut files = vec![0; 20];
    files[..4].copy_from_slice(&20_u32.to_le_bytes());
    files[16..20].copy_from_slice(&1_u32.to_le_bytes());
    files.extend(wide("Z:\\synthetic-never-open.txt"));
    files.extend([0, 0]);
    for (format, bytes, expected) in [
        (
            transient,
            1_u32.to_le_bytes().to_vec(),
            HistoryWorkerSkip::Transient,
        ),
        (
            excluded,
            1_u32.to_le_bytes().to_vec(),
            HistoryWorkerSkip::Excluded,
        ),
        (15, files, HistoryWorkerSkip::Files),
        (
            png,
            b"synthetic image bytes".to_vec(),
            HistoryWorkerSkip::Image,
        ),
    ] {
        publish(
            &scope,
            &owner,
            &[
                (format, bytes),
                (13, wide("synthetic must not be imported")),
            ],
        );
        assert!(
            matches!(worker.capture(previous, &cancel).unwrap(), HistoryWorkerReply::Skipped { reason, .. } if reason == expected)
        );
        previous = sequence(&scope);
        scope.verify();
    }
    assert_eq!(worker.stop().unwrap().spawned, 1);
}

#[test]
#[ignore = "需要自有持续helper与非交互窗口站权限；合成Unicode/长度边界"]
fn isolated_worker_strict_unicode_and_utf16_limit_do_not_poison_healthy_process() {
    let scope = PrivateClipboard::create();
    let owner = Owner::create(&scope);
    let cancel = AtomicBool::new(false);
    let mut previous = sequence(&scope);
    let mut worker = worker(&scope, 5000);
    worker.start(&cancel).unwrap();
    let invalid = [0xd800_u16, 0]
        .into_iter()
        .flat_map(u16::to_le_bytes)
        .collect();
    publish(&scope, &owner, &[(13, invalid)]);
    assert!(matches!(
        worker.capture(previous, &cancel).unwrap(),
        HistoryWorkerReply::Failed {
            reason: HistoryCaptureFailure::InvalidText,
            ..
        }
    ));
    previous = sequence(&scope);
    let maximum = "😀".repeat(32768);
    publish(&scope, &owner, &[(13, wide(&maximum))]);
    assert_text(worker.capture(previous, &cancel).unwrap(), &maximum);
    previous = sequence(&scope);
    publish(&scope, &owner, &[(13, wide(&"x".repeat(65537)))]);
    assert!(matches!(
        worker.capture(previous, &cancel).unwrap(),
        HistoryWorkerReply::Failed {
            reason: HistoryCaptureFailure::TooLarge,
            ..
        }
    ));
    previous = sequence(&scope);
    publish(&scope, &owner, &[(13, wide("synthetic recovered"))]);
    assert_text(
        worker.capture(previous, &cancel).unwrap(),
        "synthetic recovered",
    );
    assert_eq!(worker.stop().unwrap().spawned, 1);
    scope.verify();
}

#[test]
#[ignore = "需要自有持续helper与非交互窗口站权限；合成延迟渲染超时后新请求重建"]
fn isolated_worker_render_timeout_reaps_then_next_explicit_capture_rebuilds_once() {
    let scope = PrivateClipboard::create();
    let owner = Owner::create(&scope);
    publish(&scope, &owner, &[(13, wide("synthetic before delay"))]);
    let previous = sequence(&scope);
    let cancel = AtomicBool::new(false);
    let mut worker = worker(&scope, 200);
    worker.start(&cancel).unwrap();
    delayed(&scope, &owner);
    // 当前线程不处理WM_RENDERFORMAT，helper必须由预算终止，而不是等待来源窗口。
    assert_eq!(
        worker.capture(previous, &cancel),
        Err(HistoryWorkerError::Protocol(
            HistoryWorkerProtocolError::ReadTimeout
        ))
    );
    let timed_out = worker.stats();
    assert_eq!(
        (timed_out.spawned, timed_out.reaped, timed_out.captures),
        (1, 1, 1)
    );
    assert_eq!(timed_out.timeouts, 1);
    scope.verify();
    let previous = sequence(&scope);
    publish(&scope, &owner, &[(13, wide("synthetic new request"))]);
    assert_text(
        worker.capture(previous, &cancel).unwrap(),
        "synthetic new request",
    );
    let stats = worker.stop().unwrap();
    assert_eq!((stats.spawned, stats.reaped, stats.captures), (2, 2, 2));
    scope.verify();
}

#[test]
#[ignore = "需要自有持续helper与非交互窗口站权限；取消只终止本次测试创建的子进程"]
fn isolated_worker_cancel_during_pending_render_reaps_and_stop_is_complete() {
    let scope = PrivateClipboard::create();
    let owner = Owner::create(&scope);
    publish(&scope, &owner, &[(13, wide("synthetic before cancel"))]);
    let previous = sequence(&scope);
    let mut worker = worker(&scope, 5000);
    let cancel = Arc::new(AtomicBool::new(false));
    worker.start(&cancel).unwrap();
    delayed(&scope, &owner);
    let thread_cancel = cancel.clone();
    let (entered, ready) = std::sync::mpsc::sync_channel(1);
    let reading = std::thread::spawn(move || {
        entered.send(()).unwrap();
        let result = worker.capture(previous, &thread_cancel);
        let stats = worker.stop();
        (result, stats)
    });
    ready.recv_timeout(Duration::from_secs(2)).unwrap();
    // 此等待仅安排取消；不声称准确测量helper进入GetClipboardData的时刻。
    std::thread::sleep(Duration::from_millis(50));
    let began = Instant::now();
    cancel.store(true, Ordering::Release);
    let (result, stats) = reading.join().unwrap();
    assert_eq!(result, Err(HistoryWorkerError::Cancelled));
    let stats = stats.unwrap();
    assert_eq!((stats.spawned, stats.reaped), (1, 1));
    assert!(began.elapsed() < Duration::from_secs(2));
    scope.verify();
}

#[test]
#[ignore = "需要自有持续helper与非交互窗口站权限；错误scope不得回退"]
fn isolated_worker_wrong_scope_is_rejected_during_handshake_before_capture() {
    let scope = PrivateClipboard::create();
    let mut target = ClipboardScope::current().unwrap();
    target.station = format!(
        "SyntheticMissingStation_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let mut worker = ClipboardHistoryWorker::new(HistoryWorkerOptions {
        helper_executable: helper(),
        scope: target,
        budgets: HistoryWorkerBudgets {
            startup_ms: 5000,
            read_ms: 5000,
            stop_grace_ms: 0,
        },
    })
    .unwrap();
    assert_eq!(
        worker.start(&AtomicBool::new(false)),
        Err(HistoryWorkerError::Protocol(
            HistoryWorkerProtocolError::PeerRefused(HistoryCaptureFailure::ScopeMismatch)
        ))
    );
    let stats = worker.stop().unwrap();
    assert_eq!((stats.spawned, stats.reaped, stats.captures), (1, 1, 0));
    scope.verify();
}
