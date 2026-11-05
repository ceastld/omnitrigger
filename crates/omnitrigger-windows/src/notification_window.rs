//! 脚本通知仅定位主显示器工作区；窗口非交互属性复用既有 HUD 适配。
use crate::SystemHudPlacement;
use omnitrigger_core::ScreenRect;
use std::mem::size_of;
use windows::Win32::{
    Foundation::{HWND, POINT},
    Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFOEXW, MONITOR_DEFAULTTOPRIMARY},
    UI::{
        HiDpi::{
            GetDpiForMonitor, SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT,
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, MDT_EFFECTIVE_DPI,
        },
        WindowsAndMessaging::{SetWindowPos, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOZORDER},
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationPlacement {
    TopRight,
    BottomRight,
    Center,
}

fn placement_rect(
    work: ScreenRect,
    scale: f64,
    logical_width: f32,
    logical_height: f32,
    placement: NotificationPlacement,
) -> Result<(i32, i32, i32, i32), String> {
    let work_width = i64::from(work.right) - i64::from(work.left);
    let work_height = i64::from(work.bottom) - i64::from(work.top);
    if work_width <= 0
        || work_height <= 0
        || !scale.is_finite()
        || scale <= 0.
        || !logical_width.is_finite()
        || logical_width <= 0.
        || !logical_height.is_finite()
        || logical_height <= 0.
    {
        return Err("通知工作区、缩放或尺寸无效。".into());
    }
    let horizontal_margin = (20. * scale).round().clamp(0., (work_width / 4) as f64) as i64;
    let vertical_margin = (20. * scale).round().clamp(0., (work_height / 4) as f64) as i64;
    let width = (f64::from(logical_width) * scale).round().clamp(
        1.,
        (work_width - horizontal_margin * 2).min(i64::from(i32::MAX)) as f64,
    ) as i64;
    let height = (f64::from(logical_height) * scale).round().clamp(
        1.,
        (work_height - vertical_margin * 2).min(i64::from(i32::MAX)) as f64,
    ) as i64;
    let (x, y) = match placement {
        NotificationPlacement::TopRight => (
            i64::from(work.right) - width - horizontal_margin,
            i64::from(work.top) + vertical_margin,
        ),
        NotificationPlacement::BottomRight => (
            i64::from(work.right) - width - horizontal_margin,
            i64::from(work.bottom) - height - vertical_margin,
        ),
        NotificationPlacement::Center => (
            i64::from(work.left) + (work_width - width) / 2,
            i64::from(work.top) + (work_height - height) / 2,
        ),
    };
    Ok((x as i32, y as i32, width as i32, height as i32))
}

/// 明确使用主显示器，不读取鼠标、前台或动作目标；返回统一物理坐标。
pub fn primary_notification_placement(
    placement: NotificationPlacement,
    logical_width: f32,
    logical_height: f32,
) -> Result<SystemHudPlacement, String> {
    struct RestoreDpi(DPI_AWARENESS_CONTEXT);
    impl Drop for RestoreDpi {
        fn drop(&mut self) {
            let _ = unsafe { SetThreadDpiAwarenessContext(self.0) };
        }
    }
    let previous =
        unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    if previous.0.is_null() {
        return Err("无法设置通知物理坐标上下文。".into());
    }
    let _restore = RestoreDpi(previous);
    let monitor = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
    if monitor.is_invalid() {
        return Err("主显示器不可用。".into());
    }
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    if !unsafe { GetMonitorInfoW(monitor, &mut info.monitorInfo) }.as_bool() {
        return Err("无法读取主显示器工作区。".into());
    }
    let rect = info.monitorInfo.rcWork;
    let work = ScreenRect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    };
    let (mut dpi_x, mut dpi_y) = (0, 0);
    unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) }
        .map_err(|error| format!("无法读取主显示器缩放：{error}"))?;
    if dpi_x == 0 || dpi_x != dpi_y {
        return Err("主显示器缩放无效。".into());
    }
    let (x, y, width, height) = placement_rect(
        work,
        f64::from(dpi_x) / 96.,
        logical_width,
        logical_height,
        placement,
    )?;
    let rect = info.monitorInfo.rcMonitor;
    let length = info
        .szDevice
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(info.szDevice.len());
    Ok(SystemHudPlacement {
        monitor_device: String::from_utf16_lossy(&info.szDevice[..length]),
        monitor_bounds: ScreenRect {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        },
        x,
        y,
        width,
        height,
    })
}

/// 首帧绘制前设置隐藏窗口的真实尺寸，不显示、激活或改动前台。
pub fn prepare_notification_window(
    hwnd: isize,
    placement: &SystemHudPlacement,
) -> Result<(), String> {
    if placement.width <= 0 || placement.height <= 0 {
        return Err("通知尺寸无效。".into());
    }
    crate::prewarm_system_hud_window(hwnd)?;
    for _ in 0..2 {
        unsafe {
            SetWindowPos(
                HWND(hwnd as *mut std::ffi::c_void),
                None,
                placement.x,
                placement.y,
                placement.width,
                placement.height,
                SWP_NOACTIVATE | SWP_NOZORDER | SWP_FRAMECHANGED,
            )
        }
        .map_err(|error| format!("无法准备通知位置：{error}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_positions_use_work_area_including_taskbar_offsets_and_dpi() {
        let work = ScreenRect {
            left: 40,
            top: 30,
            right: 1920,
            bottom: 1040,
        };
        assert_eq!(
            placement_rect(work, 1., 400., 120., NotificationPlacement::TopRight).unwrap(),
            (1500, 50, 400, 120)
        );
        assert_eq!(
            placement_rect(work, 1., 400., 120., NotificationPlacement::BottomRight).unwrap(),
            (1500, 900, 400, 120)
        );
        assert_eq!(
            placement_rect(work, 1., 400., 120., NotificationPlacement::Center).unwrap(),
            (780, 475, 400, 120)
        );
        assert_eq!(
            placement_rect(work, 1.5, 400., 120., NotificationPlacement::TopRight).unwrap(),
            (1290, 60, 600, 180)
        );
    }

    #[test]
    fn placement_fits_small_negative_work_areas_and_rejects_invalid_geometry() {
        let work = ScreenRect {
            left: -800,
            top: -600,
            right: -700,
            bottom: -520,
        };
        assert_eq!(
            placement_rect(work, 1., 460., 1000., NotificationPlacement::BottomRight).unwrap(),
            (-780, -580, 60, 40)
        );
        for (scale, width, height) in [
            (0., 100., 100.),
            (f64::NAN, 100., 100.),
            (1., -1., 100.),
            (1., 100., f32::INFINITY),
        ] {
            assert!(
                placement_rect(work, scale, width, height, NotificationPlacement::Center).is_err()
            );
        }
    }
}
