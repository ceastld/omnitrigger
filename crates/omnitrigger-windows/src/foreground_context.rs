//! 前台通知只唤醒输入循环；映像查询在 Hook 回调和状态借用之外执行。
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    System::Threading::GetCurrentThreadId,
    UI::{
        Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK},
        WindowsAndMessaging::{
            GetForegroundWindow, GetWindowThreadProcessId, PostThreadMessageW,
            EVENT_SYSTEM_FOREGROUND, WINEVENT_OUTOFCONTEXT,
        },
    },
};

thread_local! {
    static CHANGED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(super) fn take_changed() -> bool {
    CHANGED.with(|changed| changed.replace(false))
}

pub(super) fn mark_changed() {
    CHANGED.with(|changed| changed.set(true));
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct Identity {
    pub hwnd: usize,
    pub pid: u32,
}

pub(super) fn snapshot() -> Identity {
    let hwnd = unsafe { GetForegroundWindow() };
    let mut pid = 0;
    if !hwnd.0.is_null() {
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    }
    Identity {
        hwnd: hwnd.0 as usize,
        pid,
    }
}

pub(super) fn process_name(identity: Identity) -> Option<String> {
    if identity.hwnd == 0 || identity.pid == 0 {
        return None;
    }
    let hwnd = HWND(identity.hwnd as *mut _);
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid != identity.pid {
        return None;
    }
    let name = crate::window_action::shortcut_target_process(crate::ShortcutTarget(
        identity.hwnd as isize,
    ))
    .ok()?;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    (pid == identity.pid).then_some(name)
}

pub(super) struct ForegroundEvents(HWINEVENTHOOK);

impl ForegroundEvents {
    pub fn new() -> Option<Self> {
        let hook = unsafe {
            SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                None,
                Some(changed),
                0,
                0,
                WINEVENT_OUTOFCONTEXT,
            )
        };
        if hook.is_invalid() {
            None
        } else {
            Some(Self(hook))
        }
    }
}

impl Drop for ForegroundEvents {
    fn drop(&mut self) {
        let _ = unsafe { UnhookWinEvent(self.0) };
    }
}

unsafe extern "system" fn changed(
    _: HWINEVENTHOOK,
    _: u32,
    _: HWND,
    _: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    // OUTOFCONTEXT 回调交付给注册线程，不查询进程，也不借用核心状态。
    mark_changed();
    let _ = unsafe {
        PostThreadMessageW(
            GetCurrentThreadId(),
            super::COMMAND_MESSAGE,
            WPARAM(0),
            LPARAM(0),
        )
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foreground_query_rejects_missing_or_mismatched_identity() {
        assert_eq!(process_name(Identity::default()), None);
        let identity = snapshot();
        if identity.hwnd != 0 && identity.pid != 0 {
            assert_eq!(
                process_name(Identity {
                    pid: identity.pid.wrapping_add(1),
                    ..identity
                }),
                None
            );
        }
    }

    #[test]
    fn owned_hidden_window_resolves_to_the_test_process() {
        use windows::{
            core::w,
            Win32::{
                System::Threading::GetCurrentProcessId,
                UI::WindowsAndMessaging::{
                    CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_OVERLAPPED,
                },
            },
        };
        struct Window(HWND);
        impl Drop for Window {
            fn drop(&mut self) {
                let _ = unsafe { DestroyWindow(self.0) };
            }
        }
        let window = Window(
            unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("STATIC"),
                    w!("OmniTrigger scope test"),
                    WS_OVERLAPPED,
                    0,
                    0,
                    10,
                    10,
                    None,
                    None,
                    None,
                    None,
                )
            }
            .unwrap(),
        );
        let name = process_name(Identity {
            hwnd: window.0 .0 as usize,
            pid: unsafe { GetCurrentProcessId() },
        })
        .unwrap();
        assert!(std::env::current_exe()
            .unwrap()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .eq_ignore_ascii_case(&name));
    }
}
