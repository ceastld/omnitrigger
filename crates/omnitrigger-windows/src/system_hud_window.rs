//! 独立系统调节 HUD 的目标屏幕定位和无交互窗口；不读取亮度或改变面板焦点状态。
use omnitrigger_core::ScreenRect;
use std::mem::size_of;
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{
            GetLastError, SetLastError, HANDLE, HWND, LPARAM, LRESULT, POINT, RECT, WIN32_ERROR,
            WPARAM,
        },
        Graphics::{
            Dwm::{
                DwmSetWindowAttribute, DWMNCRP_DISABLED, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE,
                DWMWA_NCRENDERING_POLICY, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
            },
            Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFOEXW, MONITOR_DEFAULTTONULL},
        },
        System::Threading::GetCurrentThreadId,
        UI::{
            HiDpi::{
                GetDpiForMonitor, SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT,
                DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, MDT_EFFECTIVE_DPI,
            },
            Input::KeyboardAndMouse::{EnableWindow, IsWindowEnabled},
            Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
            WindowsAndMessaging::{
                GetClientRect, GetPropW, GetWindowLongPtrW, GetWindowRect,
                GetWindowThreadProcessId, IsWindow, IsWindowVisible, RemovePropW, SetPropW,
                SetWindowLongPtrW, SetWindowPos, SetWindowTextW, ShowWindow, GWL_EXSTYLE,
                GWL_STYLE, HTTRANSPARENT, HWND_TOPMOST, MA_NOACTIVATE, SWP_FRAMECHANGED,
                SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, SW_HIDE,
                WINDOW_LONG_PTR_INDEX, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_DWMCOMPOSITIONCHANGED,
                WM_MOUSEACTIVATE, WM_NCCALCSIZE, WM_NCDESTROY, WM_NCHITTEST, WM_SETTINGCHANGE,
                WM_THEMECHANGED, WS_BORDER, WS_CAPTION, WS_DLGFRAME, WS_EX_APPWINDOW,
                WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_THICKFRAME,
            },
        },
    },
};

const SUBCLASS_ID: usize = 0x4f54_5348;
const STATE_PROPERTY: PCWSTR = w!("OmniTrigger.SystemHud.WindowState");
const PREPARED: usize = 1;
const NEEDS_PLACEMENT: usize = 2;
const BOTTOM_MARGIN: f64 = 56.;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemHudPlacement {
    pub monitor_device: String,
    pub monitor_bounds: ScreenRect,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

fn screen_rect(rect: RECT) -> ScreenRect {
    ScreenRect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    }
}

fn require_monitor(device: &str, expected: Option<&str>) -> Result<(), String> {
    if expected.is_some_and(|expected| !expected.eq_ignore_ascii_case(device)) {
        return Err("亮度目标显示器已变化，已取消 HUD 显示。".into());
    }
    Ok(())
}

/// 所有坐标在同一个物理坐标系中计算；逻辑尺寸和间距仅在这里按目标 DPI 换算。
fn placement_rect(
    work: ScreenRect,
    scale: f64,
    logical_width: f32,
    logical_height: f32,
) -> Result<(i32, i32, i32, i32), String> {
    if !work.is_valid()
        || !scale.is_finite()
        || scale <= 0.
        || !logical_width.is_finite()
        || logical_width <= 0.
        || !logical_height.is_finite()
        || logical_height <= 0.
    {
        return Err("HUD 工作区、缩放或尺寸无效。".into());
    }
    let work_width = i64::from(work.right) - i64::from(work.left);
    let work_height = i64::from(work.bottom) - i64::from(work.top);
    let extent = |logical: f32, available: i64| {
        (f64::from(logical) * scale)
            .round()
            .clamp(1., available.min(i64::from(i32::MAX)) as f64) as i64
    };
    let width = extent(logical_width, work_width);
    let height = extent(logical_height, work_height);
    let margin = (BOTTOM_MARGIN * scale)
        .round()
        .clamp(0., (work_height - height) as f64) as i64;
    let x = i64::from(work.left) + (work_width - width) / 2;
    let y = i64::from(work.bottom) - height - margin;
    Ok((x as i32, y as i32, width as i32, height as i32))
}

/// 只接受实际包含冻结触发点的显示器；目标身份改变或屏幕消失时不改选其他屏幕。
pub fn system_hud_placement(
    point: (i32, i32),
    expected_monitor: Option<&str>,
    logical_width: f32,
    logical_height: f32,
) -> Result<SystemHudPlacement, String> {
    // 原生预览不启动 GPUI，仍须采用物理坐标；只临时改变当前线程并恢复。
    struct RestoreDpi(DPI_AWARENESS_CONTEXT);
    impl Drop for RestoreDpi {
        fn drop(&mut self) {
            let _ = unsafe { SetThreadDpiAwarenessContext(self.0) };
        }
    }
    let previous =
        unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    if previous.0.is_null() {
        return Err(format!(
            "无法设置 HUD 物理坐标上下文：{}",
            std::io::Error::last_os_error()
        ));
    }
    let _restore_dpi = RestoreDpi(previous);
    let monitor = unsafe {
        MonitorFromPoint(
            POINT {
                x: point.0,
                y: point.1,
            },
            MONITOR_DEFAULTTONULL,
        )
    };
    if monitor.is_invalid() {
        return Err("亮度目标显示器已不包含触发位置，已取消 HUD 显示。".into());
    }
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    if !unsafe { GetMonitorInfoW(monitor, &mut info.monitorInfo) }.as_bool() {
        return Err(format!(
            "无法读取 HUD 目标显示器：{}",
            std::io::Error::last_os_error()
        ));
    }
    let length = info
        .szDevice
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(info.szDevice.len());
    let device = String::from_utf16_lossy(&info.szDevice[..length]);
    require_monitor(&device, expected_monitor)?;
    let monitor_bounds = screen_rect(info.monitorInfo.rcMonitor);
    let work = screen_rect(info.monitorInfo.rcWork);
    if !monitor_bounds.is_valid() {
        return Err("HUD 目标显示器边界无效。".into());
    }
    let work = ScreenRect {
        left: work.left.max(monitor_bounds.left),
        top: work.top.max(monitor_bounds.top),
        right: work.right.min(monitor_bounds.right),
        bottom: work.bottom.min(monitor_bounds.bottom),
    };
    let (mut dpi_x, mut dpi_y) = (0, 0);
    unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) }
        .map_err(|error| format!("无法读取 HUD 目标显示器缩放：{error}"))?;
    if dpi_x == 0 || dpi_x != dpi_y {
        return Err("HUD 目标显示器缩放无效或不一致。".into());
    }
    let (x, y, width, height) =
        placement_rect(work, f64::from(dpi_x) / 96., logical_width, logical_height)?;
    Ok(SystemHudPlacement {
        monitor_device: device,
        monitor_bounds,
        x,
        y,
        width,
        height,
    })
}

fn update_style(
    hwnd: HWND,
    index: WINDOW_LONG_PTR_INDEX,
    remove: u32,
    add: u32,
) -> Result<bool, String> {
    unsafe { SetLastError(WIN32_ERROR(0)) };
    let value = unsafe { GetWindowLongPtrW(hwnd, index) };
    let error = unsafe { GetLastError() };
    if value == 0 && error.0 != 0 {
        return Err(format!("无法读取 HUD 窗口样式：{error:?}"));
    }
    let next = (value & !(remove as isize)) | add as isize;
    if value != next {
        unsafe { SetLastError(WIN32_ERROR(0)) };
        let previous = unsafe { SetWindowLongPtrW(hwnd, index, next) };
        let error = unsafe { GetLastError() };
        if previous == 0 && error.0 != 0 {
            return Err(format!("无法设置 HUD 窗口样式：{error:?}"));
        }
    }
    Ok(value != next)
}

fn apply_dwm_preferences(hwnd: HWND) {
    // 不支持这些外观属性的系统仍可显示 HUD；不改变 GPUI 的合成方式。
    unsafe {
        let border = DWMWA_COLOR_NONE;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            &border as *const u32 as *const _,
            size_of::<u32>() as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_NCRENDERING_POLICY,
            &DWMNCRP_DISABLED as *const _ as *const _,
            size_of_val(&DWMNCRP_DISABLED) as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &DWMWCP_DONOTROUND as *const _ as *const _,
            size_of_val(&DWMWCP_DONOTROUND) as u32,
        );
    }
}

/// 预先准备 HUD 样式和完整客户区；不显示、移动、激活窗口或查询显示器数值。
pub fn prewarm_system_hud_window(hwnd: isize) -> Result<(), String> {
    prepare_system_hud_window(HWND(hwnd as *mut std::ffi::c_void))
}

fn prepare_system_hud_window(hwnd: HWND) -> Result<(), String> {
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool()
        || unsafe { GetWindowThreadProcessId(hwnd, None) } != unsafe { GetCurrentThreadId() }
    {
        return Err("HUD 窗口句柄无效或不属于当前界面线程。".into());
    }
    let installed = preparation_state(hwnd).is_some();
    let frame_changed = update_style(
        hwnd,
        GWL_STYLE,
        (WS_CAPTION | WS_THICKFRAME | WS_BORDER | WS_DLGFRAME).0,
        0,
    )?;
    let extended_changed = update_style(
        hwnd,
        GWL_EXSTYLE,
        WS_EX_APPWINDOW.0,
        (WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT).0,
    )?;
    if !installed
        && !unsafe { SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, 0) }.as_bool()
    {
        return Err("无法设置 HUD 无交互窗口过程。".into());
    }
    // 禁用顶层窗口会绕过 WM_NCHITTEST，反而使此处的透明命中失效。
    // HUD 保持原生命中过程可用，依靠 HTTRANSPARENT、NOACTIVATE 和无交互界面透传鼠标。
    if !unsafe { IsWindowEnabled(hwnd) }.as_bool() {
        let _ = unsafe { EnableWindow(hwnd, true) };
    }
    if !unsafe { IsWindowEnabled(hwnd) }.as_bool() {
        return Err("无法启用 HUD 透明命中过程。".into());
    }
    if !installed || frame_changed || extended_changed {
        set_preparation_state(hwnd, NEEDS_PLACEMENT)?;
        apply_dwm_preferences(hwnd);
        unsafe {
            SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER,
            )
        }
        .map_err(|error| format!("无法准备 HUD 客户区：{error}"))?;
    }
    verify_full_client(hwnd)
}

fn preparation_state(hwnd: HWND) -> Option<usize> {
    match unsafe { GetPropW(hwnd, STATE_PROPERTY) }.0 as usize {
        state @ (PREPARED | NEEDS_PLACEMENT) => Some(state),
        _ => None,
    }
}

fn set_preparation_state(hwnd: HWND, state: usize) -> Result<(), String> {
    unsafe {
        SetPropW(
            hwnd,
            STATE_PROPERTY,
            Some(HANDLE(state as *mut std::ffi::c_void)),
        )
    }
    .map_err(|error| format!("无法保存 HUD 窗口准备状态：{error}"))
}

fn mark_placement_dirty(hwnd: HWND) {
    if preparation_state(hwnd).is_some() && set_preparation_state(hwnd, NEEDS_PLACEMENT).is_err() {
        // 无法更新标记时保守地重新准备，不能继续使用上次的快速路径。
        let _ = unsafe { RemovePropW(hwnd, STATE_PROPERTY) };
    }
}

fn matches_placement(hwnd: HWND, placement: &SystemHudPlacement) -> Result<bool, String> {
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect) }
        .map_err(|error| format!("无法读取 HUD 窗口位置：{error}"))?;
    Ok(rect.left == placement.x
        && rect.top == placement.y
        && i64::from(rect.right) - i64::from(rect.left) == i64::from(placement.width)
        && i64::from(rect.bottom) - i64::from(rect.top) == i64::from(placement.height))
}

fn verify_full_client(hwnd: HWND) -> Result<(), String> {
    let (mut window, mut client) = (RECT::default(), RECT::default());
    unsafe { GetWindowRect(hwnd, &mut window) }
        .map_err(|error| format!("无法读取 HUD 窗口尺寸：{error}"))?;
    unsafe { GetClientRect(hwnd, &mut client) }
        .map_err(|error| format!("无法读取 HUD 客户区：{error}"))?;
    if i64::from(window.right) - i64::from(window.left)
        != i64::from(client.right) - i64::from(client.left)
        || i64::from(window.bottom) - i64::from(window.top)
            != i64::from(client.bottom) - i64::from(client.top)
    {
        return Err("HUD 仍保留系统边框或客户区缩进。".into());
    }
    Ok(())
}

/// 设置完整客户区、不激活和鼠标穿透后显示；不更改进程 DPI 感知或面板焦点恢复目标。
pub fn show_system_hud_window(
    hwnd: isize,
    placement: &SystemHudPlacement,
    title: &str,
) -> Result<(), String> {
    if placement.width <= 0
        || placement.height <= 0
        || !placement.monitor_bounds.is_valid()
        || i64::from(placement.x) < i64::from(placement.monitor_bounds.left)
        || i64::from(placement.y) < i64::from(placement.monitor_bounds.top)
        || i64::from(placement.x) + i64::from(placement.width)
            > i64::from(placement.monitor_bounds.right)
        || i64::from(placement.y) + i64::from(placement.height)
            > i64::from(placement.monitor_bounds.bottom)
        || title.contains('\0')
    {
        return Err("HUD 定位或窗口标题无效。".into());
    }
    let hwnd = HWND(hwnd as *mut std::ffi::c_void);
    prepare_system_hud_window(hwnd)?;
    let title: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
    unsafe { SetWindowTextW(hwnd, PCWSTR(title.as_ptr())) }
        .map_err(|error| format!("无法设置 HUD 标题：{error}"))?;
    // 已可见且物理范围未变时只重申置顶；DPI、显示布局和主题消息会使快速路径失效。
    if unsafe { IsWindowVisible(hwnd) }.as_bool()
        && preparation_state(hwnd) == Some(PREPARED)
        && matches_placement(hwnd, placement)?
    {
        unsafe {
            SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
            )
        }
        .map_err(|error| format!("无法更新 HUD：{error}"))?;
        if preparation_state(hwnd) == Some(PREPARED) && matches_placement(hwnd, placement)? {
            return verify_full_client(hwnd);
        }
    }
    // 首次跨屏移动可触发 GPUI 的 WM_DPICHANGED；再次应用目标物理范围，覆盖系统建议矩形。
    for _ in 0..2 {
        unsafe {
            SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                placement.x,
                placement.y,
                placement.width,
                placement.height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_FRAMECHANGED,
            )
        }
        .map_err(|error| format!("无法显示 HUD：{error}"))?;
    }
    verify_full_client(hwnd)?;
    set_preparation_state(hwnd, PREPARED)?;
    Ok(())
}

/// HUD 从未持有焦点；收起时只隐藏，不执行前台窗口恢复。
pub fn hide_system_hud_window(hwnd: isize) {
    let _ = unsafe { ShowWindow(HWND(hwnd as *mut std::ffi::c_void), SW_HIDE) };
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
        WM_NCCALCSIZE => LRESULT(0),
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_NCDESTROY => {
            let _ = unsafe { RemovePropW(hwnd, STATE_PROPERTY) };
            let _ = unsafe { RemoveWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID) };
            unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
        }
        WM_DISPLAYCHANGE
        | WM_DPICHANGED
        | WM_SETTINGCHANGE
        | WM_THEMECHANGED
        | WM_DWMCOMPOSITIONCHANGED => {
            let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
            mark_placement_dirty(hwnd);
            apply_dwm_preferences(hwnd);
            result
        }
        _ => unsafe { DefSubclassProc(hwnd, message, wparam, lparam) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::Cell,
        sync::mpsc::{self, Sender},
        thread::{self, JoinHandle},
        time::Duration,
    };
    use windows::{
        core::w,
        Win32::{
            Foundation::HINSTANCE,
            Graphics::Gdi::ClientToScreen,
            System::LibraryLoader::GetModuleHandleW,
            UI::WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
                GetForegroundWindow, PeekMessageW, RegisterClassW, SendMessageW, TranslateMessage,
                UnregisterClassW, WindowFromPoint, MSG, NCCALCSIZE_PARAMS, PM_REMOVE, WINDOWPOS,
                WM_WINDOWPOSCHANGING, WNDCLASSW, WS_EX_CONTROLPARENT, WS_EX_LAYERED,
                WS_EX_NOREDIRECTIONBITMAP, WS_OVERLAPPEDWINDOW, WS_POPUP,
            },
        },
    };

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> ScreenRect {
        ScreenRect {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn placement_uses_target_dpi_and_negative_work_area_without_primary_display_assumptions() {
        let work = rect(-1920, 0, 0, 1040);
        assert_eq!(
            placement_rect(work, 1., 320., 112.).unwrap(),
            (-1120, 872, 320, 112)
        );
        assert_eq!(
            placement_rect(work, 1.5, 320., 112.).unwrap(),
            (-1200, 788, 480, 168)
        );
        assert_eq!(
            placement_rect(rect(0, -1080, 1920, -40), 1.25, 320., 112.).unwrap(),
            (760, -250, 400, 140)
        );
        assert_eq!(
            placement_rect(rect(-20, -10, 0, 0), 2., 320., 112.).unwrap(),
            (-20, -10, 20, 10)
        );
    }

    #[test]
    fn placement_rejects_invalid_values_and_bounds_extreme_coordinates() {
        let work = rect(0, 0, 100, 100);
        for (scale, width, height) in [
            (0., 1., 1.),
            (f64::NAN, 1., 1.),
            (1., 0., 1.),
            (1., f32::INFINITY, 1.),
            (1., 1., -1.),
        ] {
            assert!(placement_rect(work, scale, width, height).is_err());
        }
        assert!(placement_rect(rect(1, 0, 0, 1), 1., 1., 1.).is_err());
        let work = rect(i32::MIN, i32::MIN, i32::MAX, i32::MAX);
        let (x, y, width, height) =
            placement_rect(work, f64::from(u32::MAX) / 96., f32::MAX, f32::MAX).unwrap();
        assert!(x >= work.left && y >= work.top);
        assert!(i64::from(x) + i64::from(width) <= i64::from(work.right));
        assert!(i64::from(y) + i64::from(height) <= i64::from(work.bottom));
        assert_eq!((width, height), (i32::MAX, i32::MAX));
    }

    #[test]
    fn expected_monitor_requires_exact_device_identity_without_fallback() {
        assert!(require_monitor(r"\\.\DISPLAY1", None).is_ok());
        assert!(require_monitor(r"\\.\DISPLAY1", Some(r"\\.\display1")).is_ok());
        assert!(require_monitor(r"\\.\DISPLAY1", Some(r"\\.\DISPLAY2")).is_err());
        assert!(require_monitor(r"\\.\DISPLAY10", Some(r"\\.\DISPLAY1")).is_err());
        assert!(require_monitor(r"\\.\DISPLAY1", Some("")).is_err());
    }

    const CLASS_NAME: PCWSTR = w!("OmniTrigger.HiddenSystemHud.Test");
    thread_local! {
        static POSITION_CHANGES: Cell<usize> = const { Cell::new(0) };
        static FRAME_CHANGES: Cell<usize> = const { Cell::new(0) };
    }

    fn reset_position_changes() {
        POSITION_CHANGES.set(0);
        FRAME_CHANGES.set(0);
    }

    fn assert_position_changes(positions: usize, frames: usize) {
        assert_eq!(POSITION_CHANGES.get(), positions);
        assert_eq!(FRAME_CHANGES.get(), frames);
    }

    unsafe extern "system" fn inset_window(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if message == WM_WINDOWPOSCHANGING {
            let position = unsafe { &*(lparam.0 as *const WINDOWPOS) };
            POSITION_CHANGES.set(POSITION_CHANGES.get() + 1);
            if position.flags.contains(SWP_FRAMECHANGED) {
                FRAME_CHANGES.set(FRAME_CHANGES.get() + 1);
            }
        }
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

    /// 位于另一原生线程的实际接收窗口；测试只查询命中，不移动鼠标或注入输入。
    struct BackgroundWindow {
        hwnd: HWND,
        stop: Sender<()>,
        worker: Option<JoinHandle<()>>,
    }
    impl BackgroundWindow {
        fn new() -> Self {
            let (ready, receive) = mpsc::sync_channel(1);
            let (stop, stopped) = mpsc::channel();
            let worker = thread::spawn(move || {
                let module = HINSTANCE(unsafe { GetModuleHandleW(None) }.unwrap().0);
                let hwnd = unsafe {
                    CreateWindowExW(
                        WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                        CLASS_NAME,
                        w!("桌面外 HUD 命中接收窗口"),
                        WS_POPUP,
                        -30000,
                        -30000,
                        640,
                        320,
                        None,
                        None,
                        Some(module),
                        None,
                    )
                }
                .unwrap();
                unsafe {
                    SetWindowPos(
                        hwnd,
                        Some(HWND_TOPMOST),
                        -30000,
                        -30000,
                        640,
                        320,
                        SWP_NOACTIVATE | SWP_SHOWWINDOW,
                    )
                }
                .unwrap();
                ready.send(hwnd.0 as isize).unwrap();
                while matches!(stopped.try_recv(), Err(mpsc::TryRecvError::Empty)) {
                    let mut message = MSG::default();
                    while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
                        let _ = unsafe { TranslateMessage(&message) };
                        unsafe { DispatchMessageW(&message) };
                    }
                    thread::sleep(Duration::from_millis(1));
                }
                let _ = unsafe { DestroyWindow(hwnd) };
            });
            Self {
                hwnd: HWND(
                    receive.recv_timeout(Duration::from_secs(5)).unwrap() as *mut std::ffi::c_void
                ),
                stop,
                worker: Some(worker),
            }
        }
    }
    impl Drop for BackgroundWindow {
        fn drop(&mut self) {
            let _ = self.stop.send(());
            if let Some(worker) = self.worker.take() {
                worker.join().unwrap();
            }
        }
    }

    fn assert_full_client(hwnd: HWND) {
        verify_full_client(hwnd).unwrap();
        let mut window = RECT::default();
        unsafe { GetWindowRect(hwnd, &mut window) }.unwrap();
        let mut origin = POINT::default();
        assert!(unsafe { ClientToScreen(hwnd, &mut origin) }.as_bool());
        assert_eq!((origin.x, origin.y), (window.left, window.top));
    }

    #[test]
    fn hidden_hud_preserves_focus_ignores_input_and_keeps_full_client_after_frame_changes() {
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
                    WS_EX_NOACTIVATE
                        | WS_EX_CONTROLPARENT
                        | WS_EX_APPWINDOW
                        | WS_EX_NOREDIRECTIONBITMAP,
                    CLASS_NAME,
                    w!("隐藏 HUD 测试"),
                    WS_OVERLAPPEDWINDOW,
                    -30000,
                    -30000,
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
        assert!(verify_full_client(window.0).is_err());
        for attempt in 0..2 {
            reset_position_changes();
            prewarm_system_hud_window(window.0 .0 as isize).unwrap();
            assert_position_changes(usize::from(attempt == 0), usize::from(attempt == 0));
            assert_full_client(window.0);
            assert!(!unsafe { IsWindowVisible(window.0) }.as_bool());
            assert_eq!(unsafe { GetForegroundWindow() }, foreground);
            assert!(unsafe { IsWindowEnabled(window.0) }.as_bool());
            let extended = unsafe { GetWindowLongPtrW(window.0, GWL_EXSTYLE) } as u32;
            let required = (WS_EX_NOACTIVATE
                | WS_EX_TOOLWINDOW
                | WS_EX_TRANSPARENT
                | WS_EX_CONTROLPARENT
                | WS_EX_NOREDIRECTIONBITMAP)
                .0;
            assert_eq!(extended & required, required);
            assert_eq!(extended & WS_EX_APPWINDOW.0, 0);
            assert_eq!(extended & WS_EX_LAYERED.0, 0);
            assert_eq!(
                unsafe { GetWindowLongPtrW(window.0, GWL_STYLE) } as u32
                    & (WS_CAPTION | WS_THICKFRAME | WS_BORDER | WS_DLGFRAME).0,
                0
            );
            assert_eq!(
                unsafe { SendMessageW(window.0, WM_NCHITTEST, None, None) }.0,
                HTTRANSPARENT as isize
            );
            assert_eq!(
                unsafe { SendMessageW(window.0, WM_MOUSEACTIVATE, None, None) }.0,
                MA_NOACTIVATE as isize
            );
        }
        let background = BackgroundWindow::new();
        assert!(prepare_system_hud_window(background.hwnd).is_err());
        let probe = POINT {
            x: -29900,
            y: -29900,
        };
        assert_eq!(unsafe { WindowFromPoint(probe) }, background.hwnd);
        // 真实原生显示/收起路径使用桌面外位置；不向用户桌面注入输入或绘制测试浮层。
        let placement = SystemHudPlacement {
            monitor_device: "test".into(),
            monitor_bounds: rect(-31000, -31000, -29000, -29000),
            x: -30000,
            y: -30000,
            width: 480,
            height: 168,
        };
        reset_position_changes();
        show_system_hud_window(window.0 .0 as isize, &placement, "系统 HUD 测试").unwrap();
        assert_position_changes(2, 2);
        assert!(unsafe { IsWindowVisible(window.0) }.as_bool());
        assert_full_client(window.0);
        assert!(unsafe { IsWindowEnabled(window.0) }.as_bool());
        assert_eq!(unsafe { WindowFromPoint(probe) }, background.hwnd);
        assert_eq!(unsafe { GetForegroundWindow() }, foreground);
        // 连续调整只需要一次不激活的置顶重申，不再重算边框或重新定位。
        reset_position_changes();
        show_system_hud_window(window.0 .0 as isize, &placement, "连续调整 HUD").unwrap();
        assert_position_changes(1, 0);
        assert_full_client(window.0);
        assert_eq!(unsafe { WindowFromPoint(probe) }, background.hwnd);
        assert_eq!(unsafe { GetForegroundWindow() }, foreground);
        // 显示布局变化使同一矩形也回退完整定位，避免沿用变化前的 DPI/边框状态。
        let _ = unsafe { SendMessageW(window.0, WM_DISPLAYCHANGE, None, None) };
        reset_position_changes();
        show_system_hud_window(window.0 .0 as isize, &placement, "显示布局变化 HUD").unwrap();
        assert_position_changes(2, 2);
        assert_full_client(window.0);
        assert_eq!(unsafe { GetForegroundWindow() }, foreground);
        let moved = SystemHudPlacement {
            monitor_device: "test2".into(),
            monitor_bounds: rect(-30500, -30500, -28500, -28500),
            x: -29980,
            y: -29980,
            width: 400,
            height: 140,
        };
        reset_position_changes();
        show_system_hud_window(window.0 .0 as isize, &moved, "变化范围 HUD").unwrap();
        assert_position_changes(2, 2);
        assert!(matches_placement(window.0, &moved).unwrap());
        assert_full_client(window.0);
        assert_eq!(unsafe { WindowFromPoint(probe) }, background.hwnd);
        assert_eq!(unsafe { GetForegroundWindow() }, foreground);
        // 回归原先的错误策略：禁用顶层 HUD 会阻止透明命中；再次显示必须修复它。
        let _ = unsafe { EnableWindow(window.0, false) };
        reset_position_changes();
        show_system_hud_window(window.0 .0 as isize, &moved, "系统 HUD 测试").unwrap();
        assert_position_changes(1, 0);
        assert!(unsafe { IsWindowEnabled(window.0) }.as_bool());
        assert_eq!(unsafe { WindowFromPoint(probe) }, background.hwnd);
        assert_eq!(unsafe { GetForegroundWindow() }, foreground);
        hide_system_hud_window(window.0 .0 as isize);
        assert!(!unsafe { IsWindowVisible(window.0) }.as_bool());
        assert_eq!(unsafe { GetForegroundWindow() }, foreground);
    }
}
