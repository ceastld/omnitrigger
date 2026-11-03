//! 只在输入消息循环的 Hook 外查询；快照不授予动作向此窗口发送输入的权限。
use super::foreground_context::{self, Identity};
use omnitrigger_core::WindowRole;
use std::time::{Duration, Instant};
use windows::Win32::{
    Foundation::{HWND, POINT},
    UI::WindowsAndMessaging::{GetWindowThreadProcessId, WindowFromPoint},
};

pub(super) const MAX_AGE: Duration = Duration::from_millis(250);
const REFRESH_AFTER: Duration = Duration::from_millis(125);

pub(super) struct Snapshot {
    pub point: (i32, i32),
    pub identity: Identity,
    pub name: Option<String>,
    pub role: WindowRole,
    pub observed: Instant,
}

impl Snapshot {
    pub fn needs_refresh(&self, point: Option<(i32, i32)>, now: Instant) -> bool {
        point != Some(self.point) || now.saturating_duration_since(self.observed) >= REFRESH_AFTER
    }
    pub fn valid_at(&self, point: Option<(i32, i32)>, now: Instant) -> bool {
        point == Some(self.point) && now.saturating_duration_since(self.observed) < MAX_AGE
    }
}

fn identity_at(point: (i32, i32)) -> Identity {
    let hwnd = unsafe {
        WindowFromPoint(POINT {
            x: point.0,
            y: point.1,
        })
    };
    let mut pid = 0;
    if !hwnd.0.is_null() {
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    }
    Identity {
        hwnd: hwnd.0 as usize,
        pid,
    }
}

/// 前后核验同一点的窗口与进程，保留真实进程名和独立窗口角色。
pub(super) fn sample_at(point: (i32, i32)) -> Snapshot {
    let observed = Instant::now();
    let identity = identity_at(point);
    let mut name = foreground_context::process_name(identity);
    let mut role = name.as_deref().map_or(WindowRole::Unknown, |name| {
        crate::window_context::role(HWND(identity.hwnd as *mut _), identity.pid, name)
    });
    if identity_at(point) != identity {
        name = None;
        role = WindowRole::Unknown;
    }
    Snapshot {
        point,
        identity,
        name,
        role,
        observed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_snapshot_requires_same_point_and_unexpired_observation() {
        let observed = Instant::now();
        let snapshot = Snapshot {
            point: (-1, 2),
            identity: Identity::default(),
            name: None,
            role: WindowRole::Unknown,
            observed,
        };
        assert!(snapshot.valid_at(Some((-1, 2)), observed + Duration::from_millis(249)));
        assert!(!snapshot.valid_at(Some((-1, 2)), observed + MAX_AGE));
        assert!(!snapshot.valid_at(Some((0, 2)), observed));
        assert!(!snapshot.valid_at(None, observed));
        assert!(!snapshot.needs_refresh(Some((-1, 2)), observed + Duration::from_millis(124)));
        assert!(snapshot.needs_refresh(Some((-1, 2)), observed + REFRESH_AFTER));
    }

    #[test]
    #[ignore = "显式交互验收：创建两个测试窗口并向自己的编辑框输入固定测试文字"]
    fn pointer_context_native_two_windows_keep_input_on_the_original_foreground() {
        use windows::{
            core::{w, PCWSTR},
            Win32::{
                Foundation::{HWND, RECT},
                UI::{
                    Input::KeyboardAndMouse::SetFocus,
                    WindowsAndMessaging::{
                        CreateWindowExW, DestroyWindow, DispatchMessageW, GetForegroundWindow,
                        GetWindowRect, GetWindowTextW, PeekMessageW, SetForegroundWindow,
                        SetWindowPos, ShowWindow, TranslateMessage, HWND_TOPMOST, MSG, PM_REMOVE,
                        SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_SHOWNOACTIVATE, WINDOW_EX_STYLE,
                        WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
                    },
                },
            },
        };
        struct Owned {
            a: HWND,
            b: HWND,
            previous: HWND,
        }
        impl Drop for Owned {
            fn drop(&mut self) {
                let foreground = unsafe { GetForegroundWindow() };
                let restore = foreground == self.a || foreground == self.b;
                let _ = unsafe { DestroyWindow(self.a) };
                let _ = unsafe { DestroyWindow(self.b) };
                if restore {
                    let _ = unsafe { SetForegroundWindow(self.previous) };
                }
            }
        }
        fn window(title: PCWSTR, x: i32) -> HWND {
            let hwnd = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("STATIC"),
                    title,
                    WS_OVERLAPPEDWINDOW,
                    x,
                    100,
                    420,
                    160,
                    None,
                    None,
                    None,
                    None,
                )
            }
            .unwrap();
            let _ = unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
            unsafe {
                SetWindowPos(
                    hwnd,
                    Some(HWND_TOPMOST),
                    x,
                    100,
                    420,
                    160,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                )
            }
            .unwrap();
            hwnd
        }
        fn edit(parent: HWND) -> HWND {
            unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("EDIT"),
                    w!(""),
                    WS_CHILD | WS_VISIBLE,
                    10,
                    10,
                    380,
                    80,
                    Some(parent),
                    None,
                    None,
                    None,
                )
            }
            .unwrap()
        }
        fn pump() {
            let until = Instant::now() + Duration::from_millis(40);
            while Instant::now() < until {
                let mut msg = MSG::default();
                while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
                    let _ = unsafe { TranslateMessage(&msg) };
                    unsafe { DispatchMessageW(&msg) };
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        fn text(hwnd: HWND) -> String {
            let mut units = [0u16; 64];
            let len = unsafe { GetWindowTextW(hwnd, &mut units) };
            String::from_utf16(&units[..len as usize]).unwrap()
        }
        let previous = unsafe { GetForegroundWindow() };
        let owned = Owned {
            a: window(w!("OmniTrigger pointer test A"), 100),
            b: window(w!("OmniTrigger pointer test B"), 700),
            previous,
        };
        let edit_a = edit(owned.a);
        let edit_b = edit(owned.b);
        let _ = unsafe { SetForegroundWindow(owned.a) };
        let _ = unsafe { SetFocus(Some(edit_a)) };
        pump();
        assert_eq!(
            unsafe { GetForegroundWindow() },
            owned.a,
            "未取得测试焦点，不发送输入"
        );
        let mut rect = RECT::default();
        unsafe { GetWindowRect(edit_b, &mut rect) }.unwrap();
        let point = ((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2);
        let snapshot = sample_at(point);
        assert_eq!(snapshot.identity.hwnd, edit_b.0 as usize);
        assert!(snapshot.name.is_some());
        assert_eq!(snapshot.role, WindowRole::Application);
        let target = crate::capture_matching_shortcut_target(owned.a.0 as isize).unwrap();
        crate::send_text_to("pointer-scope", target).unwrap();
        pump();
        assert_eq!(text(edit_a), "pointer-scope");
        assert_eq!(text(edit_b), "");
        let _ = unsafe { SetForegroundWindow(owned.b) };
        let _ = unsafe { SetFocus(Some(edit_b)) };
        pump();
        assert_eq!(unsafe { GetForegroundWindow() }, owned.b);
        assert!(crate::capture_matching_shortcut_target(owned.a.0 as isize).is_err());
        assert!(crate::send_text_to("must-not-arrive", target).is_err());
        pump();
        assert_eq!(text(edit_a), "pointer-scope");
        assert_eq!(text(edit_b), "");
        println!("native pointer_scope verified: pointer window B; text delivered only to foreground A; focus change rejected without retargeting");
    }
}
