//! 日常主窗口的通知区域图标。所有操作均在窗口线程上，随窗口销毁清理。
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Shell::{
                DefSubclassProc, GetWindowSubclass, RemoveWindowSubclass, SetWindowSubclass,
                Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD,
                NIM_DELETE, NIM_MODIFY, NIM_SETFOCUS, NIM_SETVERSION, NINF_KEY, NIN_SELECT,
                NOTIFYICONDATAW, NOTIFYICON_VERSION_4,
            },
            WindowsAndMessaging::{
                AppendMenuW, CreatePopupMenu, DestroyMenu, IsIconic, IsWindowVisible, KillTimer,
                LoadIconW, PostMessageW, RegisterWindowMessageW, SetForegroundWindow, SetTimer,
                ShowWindow, TrackPopupMenu, ENDSESSION_CLOSEAPP, IDI_APPLICATION, MF_STRING,
                SW_HIDE, SW_RESTORE, SW_SHOW, SW_SHOWMINNOACTIVE, TPM_NONOTIFY, TPM_RETURNCMD,
                TPM_RIGHTBUTTON, WM_APP, WM_CLOSE, WM_CONTEXTMENU, WM_ENDSESSION, WM_NCDESTROY,
                WM_NULL, WM_QUERYENDSESSION, WM_TIMER,
            },
        },
    },
};

const SUBCLASS: usize = 0x4f54_5452;
const CALLBACK: u32 = WM_APP + 0x34a;
const ICON_ID: u32 = 1;
const OPEN: usize = 1;
const EXIT: usize = 2;
const RETRY_TIMER: usize = SUBCLASS;
const RETRY_DELAYS_MS: [u32; 3] = [250, 1000, 3000];
const NO_ICON: usize = RETRY_DELAYS_MS.len() + 1;

// 注册消息占低 16 位，剩余位存已安排的重试次数。状态随 HWND 的子类登记一起销毁。
#[derive(Clone, Copy)]
struct TrayRegistration {
    restart: u32,
    retries: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IconRecovery {
    Ready,
    AwaitTimer,
    RetryAfter(u32),
    Unavailable,
}

impl TrayRegistration {
    fn from_data(data: usize) -> Self {
        Self {
            restart: (data & 0xffff) as u32,
            retries: data >> 16,
        }
    }

    fn data(self) -> usize {
        self.restart as usize | (self.retries << 16)
    }

    fn timer_pending(self) -> bool {
        (1..=RETRY_DELAYS_MS.len()).contains(&self.retries)
    }

    fn after_attempt(&mut self, added: bool, timer: bool) -> IconRecovery {
        if added {
            self.retries = 0;
            return IconRecovery::Ready;
        }
        if !timer && self.timer_pending() {
            return IconRecovery::AwaitTimer;
        }
        if self.retries == NO_ICON {
            self.retries = 0; // 下一次任务栏重建开始新一轮恢复。
        }
        let delay = RETRY_DELAYS_MS.get(self.retries);
        self.retries += 1;
        delay.map_or(IconRecovery::Unavailable, |ms| {
            IconRecovery::RetryAfter(*ms)
        })
    }
}

fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ICON_ID,
        ..Default::default()
    }
}

unsafe fn add_icon(hwnd: HWND) -> bool {
    let mut data = icon_data(hwnd);
    let icon = GetModuleHandleW(None)
        .and_then(|module| LoadIconW(Some(module.into()), PCWSTR(std::ptr::without_provenance(1))))
        .or_else(|_| LoadIconW(None, IDI_APPLICATION));
    let Ok(icon) = icon else { return false };
    data.hIcon = icon;
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
    data.uCallbackMessage = CALLBACK;
    for (slot, ch) in data.szTip.iter_mut().zip("OmniTrigger".encode_utf16()) {
        *slot = ch;
    }
    // 任务栏通知可能重复到达。已有图标可直接更新，再协商回调版本。
    if !Shell_NotifyIconW(NIM_ADD, &data).as_bool()
        && !Shell_NotifyIconW(NIM_MODIFY, &data).as_bool()
    {
        return false;
    }
    data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
    if !Shell_NotifyIconW(NIM_SETVERSION, &data).as_bool() {
        let _ = Shell_NotifyIconW(NIM_DELETE, &data);
        return false;
    }
    true
}

pub fn install_tray(raw: isize) -> Result<(), String> {
    let hwnd = HWND(raw as _);
    unsafe {
        let restart = RegisterWindowMessageW(w!("TaskbarCreated"));
        if restart == 0
            || !SetWindowSubclass(hwnd, Some(tray_proc), SUBCLASS, restart as usize).as_bool()
        {
            return Err("无法接收系统托盘消息，关闭窗口将退出程序。".into());
        }
        if !add_icon(hwnd) {
            update_registration(
                hwnd,
                TrayRegistration {
                    restart,
                    retries: 0,
                },
                false,
                false,
            );
            return Err("系统托盘暂不可用，可从开始菜单重新打开主窗口。".into());
        }
    }
    Ok(())
}

/// 日常窗口优先收至托盘；图标恢复失败时，关闭仍保留任务栏入口和输入监听。
pub fn hide_to_tray(raw: isize) -> bool {
    let hwnd = HWND(raw as _);
    unsafe {
        let mut data = 0;
        if !GetWindowSubclass(hwnd, Some(tray_proc), SUBCLASS, Some(&mut data)).as_bool() {
            return false;
        }
        let mode = if TrayRegistration::from_data(data).retries == NO_ICON {
            SW_SHOWMINNOACTIVE
        } else {
            SW_HIDE
        };
        let _ = ShowWindow(hwnd, mode);
    }
    true
}

pub fn show_workspace_window(raw: isize) {
    unsafe { show_main(HWND(raw as _)) }
}

unsafe fn show_main(hwnd: HWND) {
    let cmd = if IsIconic(hwnd).as_bool() {
        SW_RESTORE
    } else {
        SW_SHOW
    };
    let _ = ShowWindow(hwnd, cmd);
    let _ = SetForegroundWindow(hwnd);
}

unsafe fn remove_tray(hwnd: HWND) {
    let _ = KillTimer(Some(hwnd), RETRY_TIMER);
    let _ = RemoveWindowSubclass(hwnd, Some(tray_proc), SUBCLASS);
    let _ = Shell_NotifyIconW(NIM_DELETE, &icon_data(hwnd));
}

/// 让下一条 WM_CLOSE 进入 GPUI 原有的宿主停机与 Hook 收尾路径。
unsafe fn request_exit(hwnd: HWND) {
    remove_tray(hwnd);
    let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
}

unsafe fn show_menu(hwnd: HWND, position: WPARAM) {
    let Ok(menu) = CreatePopupMenu() else { return };
    let items = AppendMenuW(menu, MF_STRING, OPEN, w!("打开主窗口"))
        .and_then(|()| AppendMenuW(menu, MF_STRING, EXIT, w!("退出 OmniTrigger")));
    let x = position.0 as i16 as i32;
    let y = (position.0 >> 16) as i16 as i32;
    let selection = if items.is_ok() {
        let _ = SetForegroundWindow(hwnd);
        TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            x,
            y,
            None,
            hwnd,
            None,
        )
        .0 as usize
    } else {
        0
    };
    let _ = DestroyMenu(menu);
    let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
    handle_menu_selection(hwnd, selection);
}

unsafe fn handle_menu_selection(hwnd: HWND, selection: usize) {
    match selection {
        OPEN => show_main(hwnd),
        EXIT => request_exit(hwnd),
        _ => {
            let _ = Shell_NotifyIconW(NIM_SETFOCUS, &icon_data(hwnd));
        }
    }
}

unsafe extern "system" fn tray_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _: usize,
    data: usize,
) -> LRESULT {
    let registration = TrayRegistration::from_data(data);
    if lp.0 as u32 & ENDSESSION_CLOSEAPP != 0 {
        match msg {
            // Restart Manager 的查询可能被取消。只有确认结束才撤下托盘。
            WM_QUERYENDSESSION => return LRESULT(1),
            WM_ENDSESSION => {
                if wp.0 != 0 {
                    request_exit(hwnd);
                }
                return LRESULT(0);
            }
            _ => {}
        }
    }
    if msg == CALLBACK && (lp.0 as u32 >> 16) == ICON_ID {
        match lp.0 as u16 as u32 {
            event if event == NIN_SELECT || event == (NIN_SELECT | NINF_KEY) => show_main(hwnd),
            WM_CONTEXTMENU => show_menu(hwnd, wp),
            _ => {}
        }
        return LRESULT(0);
    }
    let retry_timer = msg == WM_TIMER && wp.0 == RETRY_TIMER;
    if msg == registration.restart || retry_timer {
        if retry_timer && !registration.timer_pending() {
            return LRESULT(0); // 已取消的计时器消息可能仍在队列中。
        }
        update_registration(hwnd, registration, add_icon(hwnd), retry_timer);
        return LRESULT(0);
    }
    if msg == WM_NCDESTROY {
        remove_tray(hwnd);
    }
    DefSubclassProc(hwnd, msg, wp, lp)
}

unsafe fn update_registration(
    hwnd: HWND,
    mut registration: TrayRegistration,
    added: bool,
    timer: bool,
) {
    let action = registration.after_attempt(added, timer);
    let restored_or_scheduled = match action {
        IconRecovery::AwaitTimer => return,
        IconRecovery::Ready => {
            let _ = KillTimer(Some(hwnd), RETRY_TIMER);
            true
        }
        IconRecovery::RetryAfter(delay) => {
            let scheduled = SetTimer(Some(hwnd), RETRY_TIMER, delay, None) != 0;
            if !scheduled {
                registration.retries = NO_ICON;
            }
            scheduled
        }
        IconRecovery::Unavailable => false,
    };
    if !restored_or_scheduled {
        let _ = KillTimer(Some(hwnd), RETRY_TIMER);
    }
    if !SetWindowSubclass(hwnd, Some(tray_proc), SUBCLASS, registration.data()).as_bool() {
        remove_tray(hwnd);
    } else if restored_or_scheduled {
        return;
    }
    // 图标持续不可用时保留一个任务栏入口。下次重建仍可重新注册。
    if !IsWindowVisible(hwnd).as_bool() {
        let _ = ShowWindow(hwnd, SW_SHOWMINNOACTIVE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, PeekMessageW, MSG, PM_REMOVE, WINDOW_EX_STYLE,
        WS_OVERLAPPEDWINDOW,
    };

    #[test]
    #[ignore = "仅在隔离的 Windows 桌面验证真实窗口的最小化和退出"]
    fn native_tray_close_and_exit() {
        unsafe {
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("OmniTrigger 托盘关闭测试"),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                200,
                100,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            let raw = hwnd.0 as isize;
            assert!(!hide_to_tray(raw));
            let restart = RegisterWindowMessageW(w!("TaskbarCreated"));
            assert_ne!(restart, 0);
            let mut registration = TrayRegistration {
                restart,
                retries: 0,
            };
            assert!(
                SetWindowSubclass(hwnd, Some(tray_proc), SUBCLASS, registration.data()).as_bool()
            );
            let _ = ShowWindow(hwnd, SW_SHOW);
            assert!(hide_to_tray(raw));
            assert!(!IsWindowVisible(hwnd).as_bool());

            for timer in [false, true, true, true] {
                update_registration(hwnd, registration, false, timer);
                let mut data = 0;
                assert!(
                    GetWindowSubclass(hwnd, Some(tray_proc), SUBCLASS, Some(&mut data)).as_bool()
                );
                registration = TrayRegistration::from_data(data);
            }
            assert_eq!(registration.retries, NO_ICON);
            assert!(IsIconic(hwnd).as_bool());
            show_main(hwnd);
            assert!(hide_to_tray(raw), "关闭时应继续保留日常实例");
            assert!(IsWindowVisible(hwnd).as_bool() && IsIconic(hwnd).as_bool());

            update_registration(hwnd, registration, true, false);
            handle_menu_selection(hwnd, OPEN);
            assert!(IsWindowVisible(hwnd).as_bool() && !IsIconic(hwnd).as_bool());
            assert!(hide_to_tray(raw));
            handle_menu_selection(hwnd, EXIT);
            assert!(!IsWindowVisible(hwnd).as_bool(), "托盘退出时不应弹出主窗口");
            let mut message = MSG::default();
            assert!(
                PeekMessageW(&mut message, Some(hwnd), WM_CLOSE, WM_CLOSE, PM_REMOVE).as_bool()
            );
            assert!(!hide_to_tray(raw), "明确退出后应交回正常关闭流程");
            DestroyWindow(hwnd).unwrap();
        }
    }

    #[test]
    fn tray_recovery_keeps_hidden_on_transient_failure_and_ignores_duplicate_notifications() {
        let mut registration = TrayRegistration {
            restart: 0xc153,
            retries: 0,
        };
        assert_eq!(
            registration.after_attempt(false, false),
            IconRecovery::RetryAfter(RETRY_DELAYS_MS[0])
        );
        for _ in 0..8 {
            assert_eq!(
                registration.after_attempt(false, false),
                IconRecovery::AwaitTimer
            );
            assert!(registration.timer_pending());
            registration = TrayRegistration::from_data(registration.data());
            assert_eq!(registration.restart, 0xc153);
        }
        assert_eq!(registration.after_attempt(true, true), IconRecovery::Ready);
        assert!(!registration.timer_pending());
        assert_eq!(registration.data(), 0xc153);
    }

    #[test]
    fn tray_recovery_stops_after_exhaustion_and_recovers_on_the_next_taskbar() {
        let mut registration = TrayRegistration {
            restart: 0xffff,
            retries: 0,
        };
        for (attempt, delay) in RETRY_DELAYS_MS.into_iter().enumerate() {
            assert_eq!(
                registration.after_attempt(false, attempt != 0),
                IconRecovery::RetryAfter(delay)
            );
            assert!(registration.timer_pending());
        }
        assert_eq!(
            registration.after_attempt(false, true),
            IconRecovery::Unavailable
        );
        assert!(!registration.timer_pending());
        assert_eq!(
            TrayRegistration::from_data(registration.data()).retries,
            NO_ICON
        );
        assert_eq!(
            registration.after_attempt(false, false),
            IconRecovery::RetryAfter(RETRY_DELAYS_MS[0])
        );
        assert_eq!(registration.after_attempt(true, false), IconRecovery::Ready);
        assert!(!registration.timer_pending());
    }
}
