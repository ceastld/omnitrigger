//! 显式历史监听的非交互窗口站原生验收；所有载荷均由夹具合成。
//! 默认忽略；单独进程串行执行，须显式提供自有辅助进程路径，不能回退交互剪贴板。
#![cfg(windows)]

use omnitrigger_windows::{ClipboardHistoryListener, HistoryText};
use std::ffi::c_void;
use std::io::Read;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};
use windows::{
    core::w,
    Win32::{
        Foundation::{GetLastError, GlobalFree, SetLastError, HANDLE, HWND, WIN32_ERROR},
        System::{
            DataExchange::{
                CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW,
                SetClipboardData,
            },
            Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE},
            Threading::GetCurrentThreadId,
        },
        UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE,
        },
    },
};

#[path = "support/private_clipboard.rs"]
mod private_clipboard;
use private_clipboard::PrivateClipboard;

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
}

struct Owner(HWND);
impl Owner {
    fn new(scope: &PrivateClipboard) -> Self {
        scope.verify();
        Self(
            unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("STATIC"),
                    w!("Synthetic history fixture"),
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

fn executable() -> PathBuf {
    let path = PathBuf::from(
        std::env::var_os("OMNITRIGGER_HISTORY_HELPER_EXE")
            .expect("必须显式提供已接入历史读取入口的自有程序路径；禁止猜测路径或回退"),
    );
    assert!(path.is_absolute() && path.is_file());
    path
}

fn publish(scope: &PrivateClipboard, owner: &Owner, formats: &[(u32, Vec<u8>)]) {
    scope.verify();
    unsafe {
        OpenClipboard(Some(owner.0)).unwrap();
        let _lease = Lease;
        EmptyClipboard().unwrap();
        for (format, bytes) in formats {
            let memory = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)).unwrap();
            let pointer = GlobalLock(memory);
            if pointer.is_null() {
                let _ = GlobalFree(Some(memory));
                panic!("合成载荷分配失败");
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast::<u8>(), bytes.len());
            let _ = GlobalUnlock(memory);
            if SetClipboardData(*format, Some(HANDLE(memory.0))).is_err() {
                let _ = GlobalFree(Some(memory));
                panic!("合成载荷发布失败");
            }
        }
    }
    scope.verify();
}

fn wide(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect()
}

fn publish_delayed(scope: &PrivateClipboard, owner: &Owner) {
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

fn wait(listener: &ClipboardHistoryListener, check: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !check() {
        assert!(
            Instant::now() < deadline,
            "独立监听验收超时，状态={:?}",
            listener.stats()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn receive(scope: &PrivateClipboard, receiver: &Receiver<HistoryText>) -> HistoryText {
    scope.verify();
    let record = receiver
        .recv_timeout(Duration::from_secs(3))
        .expect("未收到合成历史项");
    scope.verify();
    record
}

#[test]
#[ignore = "需要OMNITRIGGER_HISTORY_HELPER_EXE与非交互窗口站权限；单独进程串行执行"]
fn isolated_history_preserves_text_skips_marked_files_images_and_stops() {
    let scope = PrivateClipboard::create();
    let owner = Owner::new(&scope);
    publish(
        &scope,
        &owner,
        &[(13, wide("synthetic before explicit start"))],
    );
    let (mut listener, results) = ClipboardHistoryListener::start(&executable()).unwrap();
    assert_eq!(
        results.recv_timeout(Duration::from_millis(150)),
        Err(RecvTimeoutError::Timeout)
    );
    publish(&scope, &owner, &[(13, wide(" \r\n合成😀 "))]);
    let record = receive(&scope, &results);
    assert_eq!(record.text, " \r\n合成😀 ");
    assert_eq!(record.session, listener.session());

    let transient = unsafe { RegisterClipboardFormatW(w!("OmniTrigger.TransientClipboard.v1")) };
    let excluded =
        unsafe { RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing")) };
    assert_ne!(transient, 0);
    assert_ne!(excluded, 0);
    let mut drop_files = vec![0; 20];
    drop_files[..4].copy_from_slice(&20_u32.to_le_bytes());
    drop_files[16..20].copy_from_slice(&1_u32.to_le_bytes());
    drop_files.extend(wide("Z:\\synthetic-never-open.txt"));
    drop_files.extend([0, 0]);
    let mut dib = vec![0; 44];
    dib[..4].copy_from_slice(&40_u32.to_le_bytes());
    dib[4..8].copy_from_slice(&1_i32.to_le_bytes());
    dib[8..12].copy_from_slice(&1_i32.to_le_bytes());
    dib[12..14].copy_from_slice(&1_u16.to_le_bytes());
    dib[14..16].copy_from_slice(&32_u16.to_le_bytes());
    for (format, body) in [
        (transient, 1_u32.to_le_bytes().to_vec()),
        (excluded, 1_u32.to_le_bytes().to_vec()),
        (15, drop_files),
        (8, dib),
    ] {
        let previous = listener.stats().skipped;
        publish(
            &scope,
            &owner,
            &[
                (format, body),
                (13, wide("synthetic must not become history")),
            ],
        );
        wait(&listener, || listener.stats().skipped > previous);
        assert_eq!(
            results.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        );
    }
    publish(&scope, &owner, &[(13, wide(""))]);
    assert_eq!(receive(&scope, &results).text, "");
    let began = Instant::now();
    listener.stop().unwrap();
    assert!(began.elapsed() < Duration::from_secs(2));
    publish(&scope, &owner, &[(13, wide("synthetic after stop"))]);
    assert_eq!(
        results.recv_timeout(Duration::from_millis(150)),
        Err(RecvTimeoutError::Disconnected)
    );
    scope.verify();
}

#[test]
#[ignore = "需要自有辅助进程路径与非交互窗口站权限；验证单独非交互窗口站重入和完全退出"]
fn isolated_history_repeated_start_stop_has_fresh_sessions_and_no_old_replay() {
    let scope = PrivateClipboard::create();
    let owner = Owner::new(&scope);
    let mut previous_session = None;
    for index in 0..3 {
        publish(&scope, &owner, &[(13, wide("synthetic prior session"))]);
        let (mut listener, results) = ClipboardHistoryListener::start(&executable()).unwrap();
        assert_ne!(Some(listener.session()), previous_session);
        previous_session = Some(listener.session());
        assert_eq!(
            results.recv_timeout(Duration::from_millis(100)),
            Err(RecvTimeoutError::Timeout)
        );
        let expected = format!("synthetic session {index}");
        publish(&scope, &owner, &[(13, wide(&expected))]);
        assert_eq!(receive(&scope, &results).text, expected);
        listener.stop().unwrap();
        listener.stop().unwrap();
        assert_eq!(
            results.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Disconnected)
        );
        scope.verify();
    }
}

fn scope_name(object: isize) -> String {
    let mut name = [0_u16; 257];
    let mut needed = 0;
    assert_ne!(
        unsafe {
            GetUserObjectInformationW(
                object,
                2,
                name.as_mut_ptr().cast(),
                std::mem::size_of_val(&name) as u32,
                &mut needed,
            )
        },
        0
    );
    String::from_utf16(&name[..name.iter().position(|&unit| unit == 0).unwrap()]).unwrap()
}

struct OwnedHelper(Option<Child>);
impl Drop for OwnedHelper {
    fn drop(&mut self) {
        if let Some(process) = self.0.as_mut() {
            if !matches!(process.try_wait(), Ok(Some(_))) {
                let _ = process.kill();
            }
            let _ = process.wait();
        }
    }
}

fn helper_frame(station: &str, desktop: &str, previous: u32) -> Vec<u8> {
    let process = Command::new(executable())
        .arg("--omni-clipboard-history-capture")
        .arg(station)
        .arg(desktop)
        .arg(previous.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(0x0800_0000)
        .spawn()
        .unwrap();
    let mut process = OwnedHelper(Some(process));
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = process.0.as_mut().unwrap().try_wait().unwrap() {
            assert!(status.success());
            let mut bytes = Vec::new();
            process
                .0
                .as_mut()
                .unwrap()
                .stdout
                .take()
                .unwrap()
                .take(18)
                .read_to_end(&mut bytes)
                .unwrap();
            assert_eq!(bytes.len(), 17);
            assert_eq!(&bytes[..8], b"OTHIST01");
            return bytes;
        }
        if Instant::now() >= deadline {
            panic!("独立辅助进程没有按期退出");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "需要自有辅助进程路径与非交互窗口站权限；scope失败不可回退交互剪贴板，busy只返回固定帧"]
fn isolated_history_helper_fails_closed_for_missing_scope_and_busy_clipboard() {
    let scope = PrivateClipboard::create();
    let owner = Owner::new(&scope);
    let station = scope_name(unsafe { GetProcessWindowStation() });
    let desktop = scope_name(unsafe { GetThreadDesktop(GetCurrentThreadId()) });
    scope.verify();
    let missing = format!(
        "SyntheticMissingStation_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    assert_eq!(helper_frame(&missing, &desktop, 0)[12], 128);
    unsafe {
        OpenClipboard(Some(owner.0)).unwrap();
    }
    let lease = Lease;
    assert_eq!(helper_frame(&station, &desktop, 0)[12], 129);
    drop(lease);
    scope.verify();
}

#[test]
#[ignore = "需要自有辅助进程路径与非交互窗口站权限；合成延迟渲染所有者不泵消息，必须终止读取子进程"]
fn isolated_history_delayed_rendering_does_not_prevent_stop() {
    let scope = PrivateClipboard::create();
    let owner = Owner::new(&scope);
    publish(&scope, &owner, &[(13, wide("synthetic before delay"))]);
    let (mut listener, _results) = ClipboardHistoryListener::start(&executable()).unwrap();
    let previous = listener.stats().helpers_started;
    publish_delayed(&scope, &owner);
    // 不处理此线程的WM_RENDERFORMAT；内容完全属于本夹具的私有窗口站。
    wait(&listener, || listener.stats().helpers_started > previous);
    let began = Instant::now();
    let stopped = listener.stop().unwrap();
    assert!(began.elapsed() < Duration::from_secs(2));
    assert_eq!(stopped.helpers_started, stopped.helpers_finished);
    scope.verify();
}

#[test]
#[ignore = "需要自有辅助进程路径与非交互窗口站权限；合成延迟渲染超时后恢复，不访问交互剪贴板"]
fn isolated_history_delayed_render_timeout_is_bounded_and_next_update_recovers() {
    let scope = PrivateClipboard::create();
    let owner = Owner::new(&scope);
    publish(&scope, &owner, &[(13, wide("synthetic before timeout"))]);
    let (mut listener, results) = ClipboardHistoryListener::start(&executable()).unwrap();
    publish_delayed(&scope, &owner);
    wait(&listener, || listener.stats().timeouts > 0);
    assert_eq!(
        results.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    );
    publish(&scope, &owner, &[(13, wide("synthetic after timeout"))]);
    assert_eq!(receive(&scope, &results).text, "synthetic after timeout");
    let stopped = listener.stop().unwrap();
    assert_eq!(stopped.helpers_started, stopped.helpers_finished);
    scope.verify();
}
