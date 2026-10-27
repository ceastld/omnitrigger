//! 仅轮盘使用的无系统边框窗口；不改变 GPUI 的透明合成、输入和 DPI 消息。
use std::mem::size_of;
use windows::Win32::{
    Foundation::{GetLastError, SetLastError, HWND, LPARAM, LRESULT, RECT, WIN32_ERROR, WPARAM},
    Graphics::Dwm::{
        DwmSetWindowAttribute, DWMNCRP_DISABLED, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE,
        DWMWA_NCRENDERING_POLICY, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
    },
    System::Threading::GetCurrentThreadId,
    UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            GetClientRect, GetWindowLongPtrW, GetWindowRect, GetWindowThreadProcessId, IsWindow,
            SetWindowLongPtrW, SetWindowPos, GWL_STYLE, SWP_FRAMECHANGED, SWP_NOACTIVATE,
            SWP_NOCOPYBITS, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WM_ACTIVATE, WM_DPICHANGED,
            WM_DWMCOMPOSITIONCHANGED, WM_NCCALCSIZE, WM_NCDESTROY, WM_SETTINGCHANGE,
            WM_THEMECHANGED, WS_BORDER, WS_CAPTION, WS_DLGFRAME, WS_THICKFRAME,
        },
    },
};

const SUBCLASS_ID: usize = 0x4f54_5746;

pub(crate) fn prepare_borderless(hwnd: isize) -> Result<(), String> {
    let hwnd = HWND(hwnd as *mut std::ffi::c_void);
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return Err("轨道轮盘窗口句柄无效。".into());
    }
    if unsafe { GetWindowThreadProcessId(hwnd, None) } != unsafe { GetCurrentThreadId() } {
        return Err("轨道轮盘窗口不属于当前界面线程。".into());
    }
    unsafe {
        SetLastError(WIN32_ERROR(0));
    }
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) };
    let error = unsafe { GetLastError() };
    if style == 0 && error.0 != 0 {
        return Err(format!("无法读取轨道轮盘窗口样式：{error:?}"));
    }
    let next = style & !((WS_CAPTION | WS_THICKFRAME | WS_BORDER | WS_DLGFRAME).0 as isize);
    if next != style {
        unsafe {
            SetLastError(WIN32_ERROR(0));
        }
        let previous = unsafe { SetWindowLongPtrW(hwnd, GWL_STYLE, next) };
        let error = unsafe { GetLastError() };
        if previous == 0 && error.0 != 0 {
            return Err(format!("无法移除轨道轮盘系统边框：{error:?}"));
        }
    }
    if !unsafe { SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, 0) }.as_bool() {
        return Err("无法接管轨道轮盘客户区边界。".into());
    }
    apply_dwm_preferences(hwnd);
    unsafe {
        SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED
                | SWP_NOACTIVATE
                | SWP_NOMOVE
                | SWP_NOSIZE
                | SWP_NOZORDER
                | SWP_NOCOPYBITS,
        )
    }
    .map_err(|error| format!("无法刷新轨道轮盘客户区：{error}"))?;
    verify_client_area(hwnd)
}

fn verify_client_area(hwnd: HWND) -> Result<(), String> {
    let (mut window, mut client) = (RECT::default(), RECT::default());
    unsafe { GetWindowRect(hwnd, &mut window) }
        .map_err(|error| format!("无法读取轨道轮盘窗口范围：{error}"))?;
    unsafe { GetClientRect(hwnd, &mut client) }
        .map_err(|error| format!("无法读取轨道轮盘客户区：{error}"))?;
    if window.right - window.left != client.right - client.left
        || window.bottom - window.top != client.bottom - client.top
    {
        return Err("轨道轮盘仍保留系统边框，无法准备完整客户区。".into());
    }
    Ok(())
}

fn apply_dwm_preferences(hwnd: HWND) {
    // 部分系统不支持这些外观属性；不因此禁用透明合成或使轮盘不可用。
    let border = DWMWA_COLOR_NONE;
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            &border as *const u32 as *const _,
            size_of::<u32>() as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &DWMWCP_DONOTROUND as *const _ as *const _,
            size_of_val(&DWMWCP_DONOTROUND) as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_NCRENDERING_POLICY,
            &DWMNCRP_DISABLED as *const _ as *const _,
            size_of_val(&DWMNCRP_DISABLED) as u32,
        );
    }
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    match message {
        // GPUI 对隐藏标题栏的窗口也会保留系统 inset；轮盘必须让整个提议矩形成为客户区。
        // 不改写 RECT/NCCALCSIZE_PARAMS，不再交给后续窗口过程缩小客户区。
        WM_NCCALCSIZE => LRESULT(0),
        WM_NCDESTROY => {
            let _ = unsafe { RemoveWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID) };
            unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
        }
        WM_ACTIVATE
        | WM_DPICHANGED
        | WM_SETTINGCHANGE
        | WM_THEMECHANGED
        | WM_DWMCOMPOSITIONCHANGED => {
            // 先完整转发，让 GPUI 更新 DPI、客户区和渲染资源，再保留轮盘专属的外观策略。
            let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
            apply_dwm_preferences(hwnd);
            result
        }
        _ => unsafe { DefSubclassProc(hwnd, message, wparam, lparam) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::{
        core::w,
        Win32::{
            Foundation::{HINSTANCE, POINT},
            Graphics::Gdi::ClientToScreen,
            System::LibraryLoader::GetModuleHandleW,
            UI::WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, GetForegroundWindow,
                IsWindowVisible, RegisterClassW, UnregisterClassW, NCCALCSIZE_PARAMS, WNDCLASSW,
                WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_OVERLAPPEDWINDOW,
            },
        },
    };

    const CLASS_NAME: windows::core::PCWSTR = w!("OmniTrigger.HiddenWheelFrame.Test");

    // 模拟 GPUI 隐藏标题栏后每次又加入 1px 顶边和系统侧边的消息路径。
    unsafe extern "system" fn inset_window(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if message == WM_NCCALCSIZE && wparam.0 != 0 {
            let params = unsafe { &mut *(lparam.0 as *mut NCCALCSIZE_PARAMS) };
            params.rgrc[0].left += 8;
            params.rgrc[0].right -= 8;
            params.rgrc[0].top += 1;
            params.rgrc[0].bottom -= 8;
            return LRESULT(0);
        }
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }

    struct HiddenWindow(HWND, HINSTANCE);
    impl Drop for HiddenWindow {
        fn drop(&mut self) {
            let _ = unsafe { DestroyWindow(self.0) };
            let _ = unsafe { UnregisterClassW(CLASS_NAME, Some(self.1)) };
        }
    }

    fn assert_full_client(hwnd: HWND) {
        verify_client_area(hwnd).unwrap();
        let mut window = RECT::default();
        unsafe { GetWindowRect(hwnd, &mut window) }.unwrap();
        let mut origin = POINT::default();
        assert!(unsafe { ClientToScreen(hwnd, &mut origin) }.as_bool());
        assert_eq!((origin.x, origin.y), (window.left, window.top));
    }

    #[test]
    fn hidden_wheel_owns_the_full_client_rect_after_repeated_frame_changes_without_activation() {
        let foreground = unsafe { GetForegroundWindow() };
        let module = HINSTANCE(unsafe { GetModuleHandleW(None) }.unwrap().0);
        let class = WNDCLASSW {
            lpfnWndProc: Some(inset_window),
            hInstance: module,
            lpszClassName: CLASS_NAME,
            ..Default::default()
        };
        assert_ne!(unsafe { RegisterClassW(&class) }, 0);
        let window = HiddenWindow(
            unsafe {
                CreateWindowExW(
                    WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                    CLASS_NAME,
                    w!("隐藏轮盘边框测试"),
                    WS_OVERLAPPEDWINDOW,
                    100,
                    100,
                    340,
                    220,
                    None,
                    None,
                    Some(module),
                    None,
                )
            }
            .unwrap(),
            module,
        );
        assert!(!unsafe { IsWindowVisible(window.0) }.as_bool());
        assert!(verify_client_area(window.0).is_err());
        crate::prepare_wheel_window(window.0 .0 as isize).unwrap();
        assert_full_client(window.0);
        assert_eq!(
            unsafe { GetWindowLongPtrW(window.0, GWL_STYLE) } as u32
                & (WS_CAPTION | WS_THICKFRAME | WS_BORDER | WS_DLGFRAME).0,
            0
        );
        for (width, height) in [(340, 220), (641, 479), (520, 520), (840, 840)] {
            unsafe {
                SetWindowPos(
                    window.0,
                    None,
                    0,
                    0,
                    width,
                    height,
                    SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                )
            }
            .unwrap();
            assert_full_client(window.0);
            assert!(!unsafe { IsWindowVisible(window.0) }.as_bool());
        }
        // 重复准备不会叠加 subclass，也不会激活、显示或改变别的窗口。
        crate::prepare_wheel_window(window.0 .0 as isize).unwrap();
        assert_full_client(window.0);
        assert_eq!(unsafe { GetForegroundWindow() }, foreground);
    }
}
