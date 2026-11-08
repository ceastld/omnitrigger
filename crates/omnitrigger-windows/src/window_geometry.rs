//! 原目标窗口的物理屏幕矩形；不进入Hook、不激活窗口、不等待目标处理异步调整。

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowRect {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowRectFrame {
    Outer,
    Visible,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowGeometry {
    /// 同一目标几何检查中的原始窗口类名；业务匹配不进入平台守卫。
    pub class_name: String,
    pub outer_rect: WindowRect,
    pub visible_rect: WindowRect,
    pub monitor_rect: WindowRect,
    pub work_area: WindowRect,
    /// 主显示器的物理像素尺寸；与目标所在/最近显示器和工作区分开。
    pub primary_screen_width: i32,
    pub primary_screen_height: i32,
    /// GetDpiForWindow的原值，受目标DPI感知方式影响；不用于换算物理坐标。
    pub window_dpi: u32,
    pub maximized: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowGeometryError {
    pub code: &'static str,
    pub message: String,
    pub effect_state: &'static str,
}

impl WindowGeometryError {
    pub fn new(code: &'static str, message: impl Into<String>, effect_state: &'static str) -> Self {
        Self {
            code,
            message: message.into(),
            effect_state,
        }
    }

    #[cfg(any(windows, test))]
    fn after_attempt(mut self) -> Self {
        self.effect_state = "unknown";
        self
    }
}

impl WindowRect {
    /// 调整请求：有符号32位屏幕坐标；宽高1–65535，右/下边界不得溢出。
    pub fn validate(self) -> Result<Self, WindowGeometryError> {
        if !(1..=65_535).contains(&self.width)
            || !(1..=65_535).contains(&self.height)
            || self.left.checked_add(self.width).is_none()
            || self.top.checked_add(self.height).is_none()
        {
            return Err(WindowGeometryError::new(
                "invalid_input",
                "矩形宽高需要1–65535，且边界不得超出有符号32位坐标。",
                "not_started",
            ));
        }
        Ok(self)
    }
}

// 只标记既有完整类名守卫的确切拒绝；不加入来源substring业务。
#[cfg(any(windows, test))]
fn ensure_geometry_class_supported(name: &str) -> Result<(), WindowGeometryError> {
    if ["Progman", "WorkerW", "Shell_TrayWnd"]
        .iter()
        .any(|blocked| name.eq_ignore_ascii_case(blocked))
    {
        return Err(WindowGeometryError::new(
            "window_class_blocked",
            "不调整桌面或任务栏宿主窗口。",
            "not_started",
        ));
    }
    Ok(())
}

#[cfg(any(windows, test))]
fn read_with_primary(
    mut check: impl FnMut() -> Result<(), WindowGeometryError>,
    mut sample: impl FnMut() -> Result<(i32, i32), WindowGeometryError>,
    inspect: impl FnOnce((i32, i32)) -> Result<WindowGeometry, WindowGeometryError>,
) -> Result<WindowGeometry, WindowGeometryError> {
    check()?;
    let before = sample()?;
    if before.0 <= 0 || before.1 <= 0 {
        return Err(WindowGeometryError::new(
            "geometry_unavailable",
            "主屏物理尺寸无效。",
            "not_started",
        ));
    }
    check()?;
    let value = inspect(before)?;
    check()?;
    let after = sample()?;
    check()?;
    if before != after {
        return Err(WindowGeometryError::new(
            "geometry_changed",
            "读取期间主屏尺寸变化，未返回部分几何。",
            "not_started",
        ));
    }
    Ok(value)
}

#[cfg(any(windows, test))]
fn visible_to_outer(
    request: WindowRect,
    geometry: WindowGeometry,
) -> Result<WindowRect, WindowGeometryError> {
    let outer = geometry.outer_rect;
    let visible = geometry.visible_rect;
    let left = i64::from(visible.left) - i64::from(outer.left);
    let top = i64::from(visible.top) - i64::from(outer.top);
    let right = i64::from(outer.left) + i64::from(outer.width)
        - i64::from(visible.left)
        - i64::from(visible.width);
    let bottom = i64::from(outer.top) + i64::from(outer.height)
        - i64::from(visible.top)
        - i64::from(visible.height);
    if [left, top, right, bottom].iter().any(|n| *n < 0) {
        return Err(WindowGeometryError::new(
            "geometry_changed",
            "可见矩形与外框不一致，未调整窗口。",
            "not_started",
        ));
    }
    let values = [
        i64::from(request.left) - left,
        i64::from(request.top) - top,
        i64::from(request.width) + left + right,
        i64::from(request.height) + top + bottom,
    ];
    let [left, top, width, height] = values.map(i32::try_from);
    let converted = match (left, top, width, height) {
        (Ok(left), Ok(top), Ok(width), Ok(height)) => WindowRect {
            left,
            top,
            width,
            height,
        },
        _ => {
            return Err(WindowGeometryError::new(
                "invalid_input",
                "边框换算后的矩形坐标溢出。",
                "not_started",
            ))
        }
    };
    converted.validate()
}

#[cfg(any(windows, test))]
fn apply_with(
    rect: WindowRect,
    frame: WindowRectFrame,
    inspect: impl FnOnce() -> Result<WindowGeometry, WindowGeometryError>,
    submit: impl FnOnce(WindowRect) -> Result<(), WindowGeometryError>,
    mut check: impl FnMut() -> Result<(), WindowGeometryError>,
) -> Result<(), WindowGeometryError> {
    let rect = rect.validate()?;
    check()?;
    let geometry = inspect()?;
    check()?;
    if geometry.maximized {
        return Err(WindowGeometryError::new(
            "window_not_normal",
            "最大化窗口需由调用方显式还原后再调整。",
            "not_started",
        ));
    }
    let outer = match frame {
        WindowRectFrame::Outer => rect,
        WindowRectFrame::Visible => visible_to_outer(rect, geometry)?,
    };
    check()?;
    // 从进入SetWindowPos起，即使返回错误也不能证明目标未处理部分窗口消息。
    let result = submit(outer).map_err(WindowGeometryError::after_attempt);
    check().map_err(WindowGeometryError::after_attempt)?;
    result
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::{cell::Cell, ffi::c_void, mem::size_of};
    use windows::Win32::{
        Foundation::{HWND, POINT, RECT},
        Graphics::{
            Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS},
            Gdi::{
                GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, MONITORINFO,
                MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY,
            },
        },
        UI::{
            HiDpi::{
                GetDpiForWindow, SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT,
                DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
            },
            WindowsAndMessaging::{
                GetAncestor, GetClassNameW, GetForegroundWindow, GetWindowRect, IsIconic, IsWindow,
                IsWindowVisible, IsZoomed, SetWindowPos, GA_ROOT, MONITORINFOF_PRIMARY,
                SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOZORDER,
            },
        },
    };

    fn failed(code: &'static str, message: impl Into<String>) -> WindowGeometryError {
        WindowGeometryError::new(code, message, "not_started")
    }

    struct DpiScope {
        previous: DPI_AWARENESS_CONTEXT,
        restored: bool,
    }
    impl Drop for DpiScope {
        fn drop(&mut self) {
            if !self.restored {
                let _ = unsafe { SetThreadDpiAwarenessContext(self.previous) };
            }
        }
    }

    fn physical<T>(
        attempted: &Cell<bool>,
        action: impl FnOnce() -> Result<T, WindowGeometryError>,
    ) -> Result<T, WindowGeometryError> {
        let previous =
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        if previous.0.is_null() {
            return Err(failed(
                "dpi_unavailable",
                "无法为本次同步调用建立物理像素DPI上下文。",
            ));
        }
        let mut scope = DpiScope {
            previous,
            restored: false,
        };
        let result = action();
        scope.restored = !unsafe { SetThreadDpiAwarenessContext(scope.previous) }
            .0
            .is_null();
        if !scope.restored {
            return Err(WindowGeometryError::new(
                "dpi_restore_failed",
                "无法确认已恢复调用线程原DPI上下文。",
                if attempted.get() {
                    "unknown"
                } else {
                    "not_started"
                },
            ));
        }
        result
    }

    fn rect(value: RECT) -> Result<WindowRect, WindowGeometryError> {
        let width = value.right.checked_sub(value.left);
        let height = value.bottom.checked_sub(value.top);
        match (width, height) {
            (Some(width), Some(height)) if width > 0 && height > 0 => Ok(WindowRect {
                left: value.left,
                top: value.top,
                width,
                height,
            }),
            _ => Err(failed("geometry_unavailable", "系统矩形为空或坐标溢出。")),
        }
    }

    fn primary_size() -> Result<(i32, i32), WindowGeometryError> {
        // 在既有PMv2线程scope内查询(0,0)并验证PRIMARY标记，绝不使用目标的nearest monitor替代。
        let monitor = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if monitor.is_invalid()
            || !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool()
            || info.dwFlags & MONITORINFOF_PRIMARY == 0
        {
            return Err(failed(
                "geometry_unavailable",
                "无法明确读取主显示器物理尺寸。",
            ));
        }
        let value = rect(info.rcMonitor)?;
        Ok((value.width, value.height))
    }

    fn inspect(hwnd: HWND, primary: (i32, i32)) -> Result<WindowGeometry, WindowGeometryError> {
        if !unsafe { IsWindow(Some(hwnd)) }.as_bool() || unsafe { GetForegroundWindow() } != hwnd {
            return Err(failed("target_changed", "原目标已失效或不再位于前台。"));
        }
        if !unsafe { IsWindowVisible(hwnd) }.as_bool()
            || unsafe { IsIconic(hwnd) }.as_bool()
            || unsafe { GetAncestor(hwnd, GA_ROOT) } != hwnd
        {
            return Err(failed(
                "unsupported_target",
                "仅支持可见、非最小化的原顶层窗口。",
            ));
        }
        let mut name = [0u16; 257];
        let length = unsafe { GetClassNameW(hwnd, &mut name) };
        if length <= 0 || length > 255 {
            return Err(failed("geometry_unavailable", "无法读取目标窗口类型。"));
        }
        let name = String::from_utf16(&name[..length as usize])
            .map_err(|_| failed("geometry_unavailable", "窗口类名不是有效UTF-16。"))?;
        ensure_geometry_class_supported(&name)?;
        let mut cloaked = 0u32;
        unsafe {
            DwmGetWindowAttribute(
                hwnd,
                DWMWA_CLOAKED,
                &mut cloaked as *mut _ as *mut c_void,
                size_of::<u32>() as u32,
            )
        }
        .map_err(|_| failed("geometry_unavailable", "无法确认窗口DWM可见状态。"))?;
        if cloaked != 0 {
            return Err(failed("unsupported_target", "目标窗口当前被DWM隐藏。"));
        }
        let mut outer = RECT::default();
        unsafe { GetWindowRect(hwnd, &mut outer) }
            .map_err(|_| failed("geometry_unavailable", "无法读取原窗口外框。"))?;
        let mut visible = RECT::default();
        unsafe {
            DwmGetWindowAttribute(
                hwnd,
                DWMWA_EXTENDED_FRAME_BOUNDS,
                &mut visible as *mut _ as *mut c_void,
                size_of::<RECT>() as u32,
            )
        }
        .map_err(|_| {
            failed(
                "geometry_unavailable",
                "无法读取不含不可见边框的窗口矩形；不回退为外框。",
            )
        })?;
        let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if monitor.is_invalid() || !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
            return Err(failed(
                "geometry_unavailable",
                "无法读取窗口所在或最近显示器的工作区。",
            ));
        }
        let window_dpi = unsafe { GetDpiForWindow(hwnd) };
        if window_dpi == 0 {
            return Err(failed("geometry_unavailable", "无法读取目标窗口DPI。"));
        }
        let mut after = RECT::default();
        unsafe { GetWindowRect(hwnd, &mut after) }
            .map_err(|_| failed("geometry_changed", "读取期间原窗口已改变。"))?;
        if (outer.left, outer.top, outer.right, outer.bottom)
            != (after.left, after.top, after.right, after.bottom)
        {
            return Err(failed(
                "geometry_changed",
                "读取期间窗口矩形变化，快照已丢弃。",
            ));
        }
        Ok(WindowGeometry {
            class_name: name,
            outer_rect: rect(outer)?,
            visible_rect: rect(visible)?,
            monitor_rect: rect(info.rcMonitor)?,
            work_area: rect(info.rcWork)?,
            primary_screen_width: primary.0,
            primary_screen_height: primary.1,
            window_dpi,
            maximized: unsafe { IsZoomed(hwnd) }.as_bool(),
        })
    }

    pub(crate) fn read(
        target: crate::ShortcutTarget,
        mut check: impl FnMut() -> Result<(), WindowGeometryError>,
    ) -> Result<WindowGeometry, WindowGeometryError> {
        let attempted = Cell::new(false);
        physical(&attempted, || {
            read_with_primary(&mut check, primary_size, |primary| {
                inspect(HWND(target.0 as *mut c_void), primary)
            })
        })
    }

    pub(crate) fn set(
        target: crate::ShortcutTarget,
        rect: WindowRect,
        frame: WindowRectFrame,
        check: impl FnMut() -> Result<(), WindowGeometryError>,
    ) -> Result<(), WindowGeometryError> {
        rect.validate()?;
        let hwnd = HWND(target.0 as *mut c_void);
        let attempted = Cell::new(false);
        physical(&attempted, || {
            apply_with(
                rect,
                frame,
                || inspect(hwnd, primary_size()?),
                |outer| {
                    attempted.set(true);
                    unsafe {
                        SetWindowPos(
                            hwnd,
                            None,
                            outer.left,
                            outer.top,
                            outer.width,
                            outer.height,
                            SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_ASYNCWINDOWPOS,
                        )
                    }
                    .map_err(|error| {
                        failed(
                            "window_adjust_failed",
                            format!("系统窗口调整请求未获确认：{error}"),
                        )
                    })
                },
                check,
            )
        })
    }
}

#[cfg(windows)]
pub(crate) use native::{read as read_checked, set as set_checked};

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    fn rect(left: i32, top: i32, width: i32, height: i32) -> WindowRect {
        WindowRect {
            left,
            top,
            width,
            height,
        }
    }
    fn geometry() -> WindowGeometry {
        WindowGeometry {
            class_name: "TestWindow".into(),
            outer_rect: rect(-808, 92, 816, 616),
            visible_rect: rect(-800, 100, 800, 600),
            monitor_rect: rect(-1920, 0, 1920, 1080),
            work_area: rect(-1920, 0, 1920, 1040),
            primary_screen_width: 2560,
            primary_screen_height: 1440,
            window_dpi: 144,
            maximized: false,
        }
    }
    #[test]
    fn exact_builtin_class_rejection_has_a_distinct_structured_code() {
        for name in [
            "Progman",
            "progman",
            "WorkerW",
            "WORKERW",
            "Shell_TrayWnd",
            "shell_traywnd",
        ] {
            let error = ensure_geometry_class_supported(name).unwrap_err();
            assert_eq!(error.code, "window_class_blocked");
            assert_eq!(error.effect_state, "not_started");
        }
        for name in [
            "TestWindow",
            "Custom.WorkerW.Proxy",
            "窗口类",
            "ſhell_TrayWnd",
            "WorKerW",
        ] {
            ensure_geometry_class_supported(name).unwrap();
        }
    }

    #[test]
    fn physical_negative_coordinates_and_visible_border_conversion_are_exact() {
        assert_eq!(
            visible_to_outer(rect(-500, 200, 700, 500), geometry()).unwrap(),
            rect(-508, 192, 716, 516)
        );
        assert!(rect(i32::MAX, 0, 1, 1).validate().is_err());
        assert!(rect(0, 0, 0, 1).validate().is_err());
        assert!(rect(0, 0, 65536, 1).validate().is_err());
        let mut bad = geometry();
        bad.visible_rect.left = -900;
        assert_eq!(
            visible_to_outer(rect(0, 0, 100, 100), bad)
                .unwrap_err()
                .code,
            "geometry_changed"
        );
    }
    #[test]
    fn every_guard_rejection_before_attempt_prevents_submission() {
        for stop in 1..=3 {
            let count = Cell::new(0);
            let error = apply_with(
                rect(1, 2, 100, 100),
                WindowRectFrame::Outer,
                || Ok(geometry()),
                |_| panic!("守卫拒绝不得调整"),
                || {
                    count.set(count.get() + 1);
                    if count.get() == stop {
                        Err(WindowGeometryError::new(
                            "target_changed",
                            "身份/前台/取消拒绝",
                            "not_started",
                        ))
                    } else {
                        Ok(())
                    }
                },
            )
            .unwrap_err();
            assert_eq!(error.effect_state, "not_started");
        }
    }
    #[test]
    fn maximized_and_overflow_never_submit_or_restore() {
        let mut maximized = geometry();
        maximized.maximized = true;
        assert_eq!(
            apply_with(
                rect(0, 0, 100, 100),
                WindowRectFrame::Outer,
                || Ok(maximized),
                |_| panic!("不隐式还原"),
                || Ok(())
            )
            .unwrap_err()
            .code,
            "window_not_normal"
        );
        assert!(visible_to_outer(rect(i32::MIN, 0, 100, 100), geometry()).is_err());
    }
    #[test]
    fn attempted_failure_or_late_cancel_is_unknown_without_retry() {
        for late in [false, true] {
            let calls = Cell::new(0);
            let checks = Cell::new(0);
            let error = apply_with(
                rect(0, 0, 100, 100),
                WindowRectFrame::Outer,
                || Ok(geometry()),
                |_| {
                    calls.set(calls.get() + 1);
                    Err(WindowGeometryError::new(
                        "window_adjust_failed",
                        "原生返回失败",
                        "not_started",
                    ))
                },
                || {
                    checks.set(checks.get() + 1);
                    if late && checks.get() == 4 {
                        Err(WindowGeometryError::new("cancelled", "取消", "not_started"))
                    } else {
                        Ok(())
                    }
                },
            )
            .unwrap_err();
            assert_eq!(calls.get(), 1);
            assert_eq!(error.effect_state, "unknown");
            assert_eq!(
                error.code,
                if late {
                    "cancelled"
                } else {
                    "window_adjust_failed"
                }
            );
        }
    }
    #[test]
    fn primary_read_keeps_explicit_primary_size_distinct_from_nearest_monitor() {
        let value = read_with_primary(
            || Ok(()),
            || Ok((2560, 1440)),
            |size| {
                let g = geometry();
                assert_eq!((g.primary_screen_width, g.primary_screen_height), size);
                Ok(g)
            },
        )
        .unwrap();
        assert_eq!(value.monitor_rect.width, 1920);
        assert_eq!(value.primary_screen_width, 2560);
        assert_eq!(value.primary_screen_height, 1440);
    }
    #[test]
    fn primary_change_or_invalid_size_returns_no_partial_geometry() {
        let calls = Cell::new(0);
        let error = read_with_primary(
            || Ok(()),
            || {
                calls.set(calls.get() + 1);
                Ok(if calls.get() == 1 {
                    (2560, 1440)
                } else {
                    (1920, 1080)
                })
            },
            |_| Ok(geometry()),
        )
        .unwrap_err();
        assert_eq!(error.code, "geometry_changed");
        assert_eq!(error.effect_state, "not_started");
        let error = read_with_primary(
            || Ok(()),
            || Ok((0, 1440)),
            |_| panic!("invalid primary must stop before geometry"),
        )
        .unwrap_err();
        assert_eq!(error.code, "geometry_unavailable");
    }
    #[test]
    fn primary_read_checks_cancel_deadline_and_identity_at_each_boundary() {
        for stop in 1..=4 {
            let checks = Cell::new(0);
            let error = read_with_primary(
                || {
                    checks.set(checks.get() + 1);
                    if checks.get() == stop {
                        Err(WindowGeometryError::new(
                            "cancelled",
                            "synthetic",
                            "not_started",
                        ))
                    } else {
                        Ok(())
                    }
                },
                || Ok((2560, 1440)),
                |_| Ok(geometry()),
            )
            .unwrap_err();
            assert_eq!(error.code, "cancelled");
            assert_eq!(error.effect_state, "not_started");
            assert_eq!(checks.get(), stop);
        }
    }
}
