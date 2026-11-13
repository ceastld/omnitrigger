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
                NIM_DELETE, NIM_SETFOCUS, NIM_SETVERSION, NINF_KEY, NIN_SELECT, NOTIFYICONDATAW,
                NOTIFYICON_VERSION_4,
            },
            WindowsAndMessaging::{
                AppendMenuW, CreatePopupMenu, DestroyMenu, IsIconic, IsWindowVisible, LoadIconW,
                PostMessageW, RegisterWindowMessageW, SetForegroundWindow, ShowWindow,
                TrackPopupMenu, IDI_APPLICATION, MF_STRING, SW_HIDE, SW_RESTORE, SW_SHOW,
                TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_APP, WM_CLOSE, WM_CONTEXTMENU,
                WM_NCDESTROY, WM_NULL,
            },
        },
    },
};

const SUBCLASS: usize = 0x4f54_5452;
const CALLBACK: u32 = WM_APP + 0x34a;
const ICON_ID: u32 = 1;
const OPEN: usize = 1;
const EXIT: usize = 2;

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
    if !Shell_NotifyIconW(NIM_ADD, &data).as_bool() {
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
            let _ = RemoveWindowSubclass(hwnd, Some(tray_proc), SUBCLASS);
            return Err("系统托盘图标未能创建，关闭窗口将退出程序。".into());
        }
    }
    Ok(())
}

pub fn hide_to_tray(raw: isize) -> bool {
    let hwnd = HWND(raw as _);
    unsafe {
        if !GetWindowSubclass(hwnd, Some(tray_proc), SUBCLASS, None).as_bool() {
            return false;
        }
        let _ = ShowWindow(hwnd, SW_HIDE);
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
    let _ = RemoveWindowSubclass(hwnd, Some(tray_proc), SUBCLASS);
    let _ = Shell_NotifyIconW(NIM_DELETE, &icon_data(hwnd));
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
    match selection {
        OPEN => show_main(hwnd),
        EXIT => {
            // 撤下托盘拦截后，把关闭请求交回 GPUI 的既有宿主停机与 Hook 收尾路径。
            remove_tray(hwnd);
            show_main(hwnd);
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
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
    restart: usize,
) -> LRESULT {
    if msg == CALLBACK && (lp.0 as u32 >> 16) == ICON_ID {
        match lp.0 as u16 as u32 {
            event if event == NIN_SELECT || event == (NIN_SELECT | NINF_KEY) => show_main(hwnd),
            WM_CONTEXTMENU => show_menu(hwnd, wp),
            _ => {}
        }
        return LRESULT(0);
    }
    if msg == restart as u32 && !add_icon(hwnd) {
        let _ = RemoveWindowSubclass(hwnd, Some(tray_proc), SUBCLASS);
        if !IsWindowVisible(hwnd).as_bool() {
            show_main(hwnd);
        }
    } else if msg == WM_NCDESTROY {
        remove_tray(hwnd);
    }
    DefSubclassProc(hwnd, msg, wp, lp)
}
