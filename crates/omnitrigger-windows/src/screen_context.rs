//! Hook 外的显示器拓扑采集与变化通知；窗口过程只设置合并标记。
use omnitrigger_core::ScreenRect;
use std::{cell::Cell, mem::size_of, sync::OnceLock};
use windows::{
    core::{w, BOOL},
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO},
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos,
            RegisterClassW, MSG, WM_DISPLAYCHANGE, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
            WS_POPUP,
        },
    },
};

const CLASS_NAME: windows::core::PCWSTR = w!("OmniTrigger.DisplayContext");
const MAX_SCREENS: usize = 32;

thread_local! {
    static REFRESH: Cell<bool> = const { Cell::new(false) };
}

pub(super) fn take_refresh() -> bool {
    REFRESH.with(|refresh| refresh.replace(false))
}

pub(super) struct DisplayEvents(HWND);

impl DisplayEvents {
    pub(super) fn new(module: HINSTANCE) -> Result<Self, String> {
        static REGISTERED: OnceLock<Result<(), String>> = OnceLock::new();
        REGISTERED
            .get_or_init(|| {
                let class = WNDCLASSW {
                    lpfnWndProc: Some(window_proc),
                    hInstance: module,
                    lpszClassName: CLASS_NAME,
                    ..Default::default()
                };
                if unsafe { RegisterClassW(&class) } == 0 {
                    Err(format!(
                        "无法注册显示器上下文窗口：{}",
                        std::io::Error::last_os_error()
                    ))
                } else {
                    Ok(())
                }
            })
            .clone()?;
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                CLASS_NAME,
                w!("OmniTrigger 显示器上下文"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(module),
                None,
            )
        }
        .map_err(|error| format!("无法创建显示器上下文窗口：{error}"))?;
        Ok(Self(hwnd))
    }

    pub(super) fn dispatch(&self, message: &MSG) {
        if message.hwnd == self.0 {
            unsafe { DispatchMessageW(message) };
        }
    }
}

impl Drop for DisplayEvents {
    fn drop(&mut self) {
        let _ = unsafe { DestroyWindow(self.0) };
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_DISPLAYCHANGE {
        REFRESH.with(|refresh| refresh.set(true));
        LRESULT(0)
    } else {
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }
}

pub(super) fn cursor_position() -> Result<(i32, i32), String> {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point) }
        .map_err(|error| format!("无法读取物理鼠标位置：{error}"))?;
    Ok((point.x, point.y))
}

struct ScreenCollection {
    screens: Vec<ScreenRect>,
    error: Option<String>,
}

unsafe extern "system" fn collect_monitor(
    monitor: HMONITOR,
    _: HDC,
    _: *mut RECT,
    data: LPARAM,
) -> BOOL {
    let collection = unsafe { &mut *(data.0 as *mut ScreenCollection) };
    if collection.screens.len() == MAX_SCREENS {
        collection.error = Some("显示器数量超过 32，无法建立有界输入上下文。".into());
        return BOOL(0);
    }
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        collection.error = Some(format!(
            "无法读取显示器完整边界：{}",
            std::io::Error::last_os_error()
        ));
        return BOOL(0);
    }
    let rect = info.rcMonitor;
    collection.screens.push(ScreenRect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    });
    BOOL(1)
}

pub(super) fn collect_screens() -> Result<Vec<ScreenRect>, String> {
    let mut collection = ScreenCollection {
        screens: Vec::with_capacity(MAX_SCREENS),
        error: None,
    };
    let success = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(collect_monitor),
            LPARAM((&mut collection as *mut ScreenCollection) as isize),
        )
    };
    if let Some(error) = collection.error {
        return Err(error);
    }
    if !success.as_bool() {
        return Err(format!(
            "无法枚举显示器：{}",
            std::io::Error::last_os_error()
        ));
    }
    if collection.screens.is_empty() {
        return Err("没有可用的显示器完整边界。".into());
    }
    // 拓扑顺序不影响匹配；固定顺序可避免仅枚举顺序变化时取消同一上下文。
    collection
        .screens
        .sort_by_key(|rect| (rect.left, rect.top, rect.right, rect.bottom));
    collection.screens.dedup();
    Ok(collection.screens)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::{
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{GetForegroundWindow, IsWindowVisible, SendMessageW},
    };

    #[test]
    fn hidden_display_window_coalesces_only_its_display_change_notifications_without_activation() {
        take_refresh();
        let foreground = unsafe { GetForegroundWindow() };
        let module = HINSTANCE(unsafe { GetModuleHandleW(None) }.unwrap().0);
        let window = DisplayEvents::new(module).unwrap();
        assert!(!unsafe { IsWindowVisible(window.0) }.as_bool());
        for _ in 0..3 {
            unsafe { SendMessageW(window.0, WM_DISPLAYCHANGE, None, None) };
        }
        assert!(take_refresh());
        assert!(!take_refresh());
        assert_eq!(unsafe { GetForegroundWindow() }, foreground);
    }
}
