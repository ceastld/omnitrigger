//! 窗口动作由外层执行器调用；不进入输入 Hook，不模拟键盘，也不主动获取前台。

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowOperation {
    ToggleMaximize,
    Minimize,
}

#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, Default)]
struct TargetState {
    valid: bool,
    visible: bool,
    foreground: bool,
    top_level: bool,
    maximized: bool,
    minimized: bool,
    can_maximize: bool,
    can_minimize: bool,
}

#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WindowCommand {
    Maximize,
    Restore,
    Minimize,
}

#[cfg(any(windows, test))]
fn plan(state: TargetState, operation: WindowOperation) -> Result<WindowCommand, String> {
    if !state.valid {
        return Err("目标窗口已关闭或失效，已取消窗口动作。".into());
    }
    if !state.foreground {
        return Err("前台窗口已切换，已取消窗口动作。".into());
    }
    if !state.visible || !state.top_level || state.minimized {
        return Err("目标窗口当前无法执行窗口动作。".into());
    }
    match operation {
        WindowOperation::ToggleMaximize if state.maximized => Ok(WindowCommand::Restore),
        WindowOperation::ToggleMaximize if state.can_maximize => Ok(WindowCommand::Maximize),
        WindowOperation::Minimize if state.can_minimize => Ok(WindowCommand::Minimize),
        WindowOperation::ToggleMaximize => Err("目标窗口不支持最大化。".into()),
        WindowOperation::Minimize => Err("目标窗口不支持最小化。".into()),
    }
}

#[cfg(any(windows, test))]
fn perform_with(
    operation: WindowOperation,
    mut inspect: impl FnMut() -> TargetState,
    submit: impl FnOnce(WindowCommand) -> bool,
) -> Result<(), String> {
    plan(inspect(), operation)?;
    // 提交前重新核验；切换前台或窗口在检查期间关闭时不向旧目标发送请求。
    let command = plan(inspect(), operation)?;
    if submit(command) {
        Ok(())
    } else {
        Err("系统未接受窗口动作，请检查目标窗口状态或权限。".into())
    }
}

/// 对捕获的前台窗口提交原生窗口动作。必须由 Hook 之外的外层执行器调用。
/// 每次执行都检查目标仍有效、可见、支持该操作且仍为前台，不切换或抢占前台。
/// `Ok(())` 表示系统已接纳异步显示请求，不代表目标应用已经处理完成。
#[cfg(windows)]
pub fn perform_window_action(
    target: crate::ShortcutTarget,
    operation: WindowOperation,
) -> Result<(), String> {
    use windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{
            GetAncestor, GetForegroundWindow, GetWindowLongPtrW, IsIconic, IsWindow,
            IsWindowVisible, IsZoomed, ShowWindowAsync, GA_ROOT, GWL_STYLE, SW_MAXIMIZE,
            SW_MINIMIZE, SW_RESTORE, WS_MAXIMIZEBOX, WS_MINIMIZEBOX,
        },
    };
    let hwnd = HWND(target.0 as *mut std::ffi::c_void);
    perform_with(
        operation,
        || {
            if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
                return TargetState::default();
            }
            let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
            TargetState {
                valid: true,
                visible: unsafe { IsWindowVisible(hwnd) }.as_bool(),
                foreground: unsafe { GetForegroundWindow() } == hwnd,
                top_level: unsafe { GetAncestor(hwnd, GA_ROOT) } == hwnd,
                maximized: unsafe { IsZoomed(hwnd) }.as_bool(),
                minimized: unsafe { IsIconic(hwnd) }.as_bool(),
                can_maximize: style & WS_MAXIMIZEBOX.0 != 0,
                can_minimize: style & WS_MINIMIZEBOX.0 != 0,
            }
        },
        |command| {
            // 只操作仍在前台的目标；不通过 SetForegroundWindow 改变系统决定。
            if unsafe { GetForegroundWindow() } != hwnd {
                return false;
            }
            let command = match command {
                WindowCommand::Maximize => SW_MAXIMIZE,
                WindowCommand::Restore => SW_RESTORE,
                WindowCommand::Minimize => SW_MINIMIZE,
            };
            unsafe { ShowWindowAsync(hwnd, command) }.as_bool()
        },
    )
}

/// 读取捕获窗口所属进程的映像文件名（例如 `Code.exe`），不读取目录或进程配置。
/// 目标不必仍为前台；无法读取时返回错误，由调用方决定使用全局配置。
#[cfg(windows)]
pub fn shortcut_target_process(target: crate::ShortcutTarget) -> Result<String, String> {
    use windows::{
        core::PWSTR,
        Win32::{
            Foundation::{CloseHandle, HANDLE, HWND},
            System::Threading::{
                OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
                PROCESS_QUERY_LIMITED_INFORMATION,
            },
            UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindow},
        },
    };
    struct ProcessHandle(HANDLE);
    impl Drop for ProcessHandle {
        fn drop(&mut self) {
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
    let hwnd = HWND(target.0 as *mut std::ffi::c_void);
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return Err("目标窗口已关闭或失效，无法读取所属程序。".into());
    }
    let mut pid = 0;
    if unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) } == 0 || pid == 0 {
        return Err("无法确定目标窗口所属进程。".into());
    }
    let process = ProcessHandle(
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
            .map_err(|error| format!("无法读取目标进程：{error}"))?,
    );
    let mut current_pid = 0;
    if unsafe { GetWindowThreadProcessId(hwnd, Some(&mut current_pid)) } == 0 || current_pid != pid
    {
        return Err("目标窗口身份已改变，无法读取所属程序。".into());
    }
    let mut buffer = vec![0u16; 32_768];
    let mut length = buffer.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(
            process.0,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )
    }
    .map_err(|error| format!("无法读取目标程序名称：{error}"))?;
    let path = String::from_utf16_lossy(&buffer[..length as usize]);
    std::path::Path::new(&path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "目标进程没有可用的程序名称。".into())
}

#[cfg(any(windows, test))]
fn desktop_context(class_name: &str) -> Option<&'static str> {
    (class_name.eq_ignore_ascii_case("Progman") || class_name.eq_ignore_ascii_case("WorkerW"))
        .then_some("@desktop")
}

/// 读取轮盘应用上下文；桌面窗口使用专用标识，其余窗口保留进程映像文件名。
/// 此函数不要求目标仍为前台，不读取桌面文件、目录或窗口文字。
#[cfg(windows)]
pub fn shortcut_target_context(target: crate::ShortcutTarget) -> Result<String, String> {
    use windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{GetAncestor, GetClassNameW, IsWindow, GA_ROOT},
    };
    let hwnd = HWND(target.0 as *mut std::ffi::c_void);
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return Err("目标窗口已关闭或失效，无法读取应用上下文。".into());
    }
    let root = unsafe { GetAncestor(hwnd, GA_ROOT) };
    if root.is_invalid() {
        return Err("无法确定目标顶层窗口。".into());
    }
    let mut buffer = [0u16; 256];
    let count = unsafe { GetClassNameW(root, &mut buffer) };
    if count <= 0 {
        return Err("无法读取目标窗口类型。".into());
    }
    let class_name = String::from_utf16_lossy(&buffer[..count as usize]);
    if let Some(context) = desktop_context(&class_name) {
        Ok(context.into())
    } else {
        shortcut_target_process(target)
    }
}

#[cfg(any(windows, test))]
fn effective_scale(dpi_x: u32, dpi_y: u32) -> Result<f32, String> {
    if dpi_x == 0 || dpi_y == 0 || dpi_x != dpi_y {
        return Err("按下位置所在显示器没有可用的一致缩放比例。".into());
    }
    Ok(dpi_x as f32 / 96.)
}

/// 直接读取物理屏幕坐标所在显示器的有效缩放，不依赖浮窗的历史位置或尚未处理的 DPI 消息。
/// 只查询显示器信息；不改变鼠标、前台、窗口布局或 DPI 感知设置。
#[cfg(windows)]
pub fn pointer_scale_at_point(x: i32, y: i32) -> Result<f32, String> {
    use windows::Win32::{
        Foundation::POINT,
        Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTONEAREST},
        UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
    };
    let monitor = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) };
    if monitor.is_invalid() {
        return Err("无法确定按下位置所在的显示器。".into());
    }
    let (mut dpi_x, mut dpi_y) = (0, 0);
    unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) }
        .map_err(|error| format!("无法读取按下位置所在显示器的缩放比例：{error}"))?;
    effective_scale(dpi_x, dpi_y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn effective_scale_preserves_monitor_dpi_and_rejects_missing_or_inconsistent_values() {
        assert_eq!(effective_scale(96, 96).unwrap(), 1.);
        assert_eq!(effective_scale(144, 144).unwrap(), 1.5);
        assert_eq!(effective_scale(192, 192).unwrap(), 2.);
        assert!(effective_scale(0, 96).is_err());
        assert!(effective_scale(96, 0).is_err());
        assert!(effective_scale(144, 192).is_err());
    }

    fn ready() -> TargetState {
        TargetState {
            valid: true,
            visible: true,
            foreground: true,
            top_level: true,
            can_maximize: true,
            can_minimize: true,
            ..Default::default()
        }
    }

    #[test]
    fn desktop_context_distinguishes_desktop_hosts_from_other_explorer_windows() {
        assert_eq!(desktop_context("Progman"), Some("@desktop"));
        assert_eq!(desktop_context("WorkerW"), Some("@desktop"));
        assert_eq!(desktop_context("workerw"), Some("@desktop"));
        for class in [
            "CabinetWClass",
            "Shell_TrayWnd",
            "Chrome_WidgetWin_1",
            "WorkerWChild",
        ] {
            assert_eq!(desktop_context(class), None);
        }
    }

    #[test]
    fn toggle_uses_the_current_maximized_state_and_preserves_capability_boundaries() {
        assert_eq!(
            plan(ready(), WindowOperation::ToggleMaximize).unwrap(),
            WindowCommand::Maximize
        );
        assert_eq!(
            plan(
                TargetState {
                    maximized: true,
                    ..ready()
                },
                WindowOperation::ToggleMaximize
            )
            .unwrap(),
            WindowCommand::Restore
        );
        assert_eq!(
            plan(ready(), WindowOperation::Minimize).unwrap(),
            WindowCommand::Minimize
        );
        assert!(plan(
            TargetState {
                can_maximize: false,
                ..ready()
            },
            WindowOperation::ToggleMaximize
        )
        .is_err());
        assert!(plan(
            TargetState {
                can_minimize: false,
                ..ready()
            },
            WindowOperation::Minimize
        )
        .is_err());
    }

    #[test]
    fn invalid_hidden_child_or_changed_foreground_never_submits() {
        for state in [
            TargetState::default(),
            TargetState {
                foreground: false,
                ..ready()
            },
            TargetState {
                visible: false,
                ..ready()
            },
            TargetState {
                top_level: false,
                ..ready()
            },
            TargetState {
                minimized: true,
                ..ready()
            },
        ] {
            let submitted = Cell::new(false);
            assert!(perform_with(
                WindowOperation::Minimize,
                || state,
                |_| {
                    submitted.set(true);
                    true
                }
            )
            .is_err());
            assert!(!submitted.get());
        }
    }

    #[test]
    fn rechecks_foreground_before_submit_and_reports_native_rejection() {
        let checks = Cell::new(0);
        let submitted = Cell::new(false);
        assert!(perform_with(
            WindowOperation::Minimize,
            || {
                let count = checks.get();
                checks.set(count + 1);
                TargetState {
                    foreground: count == 0,
                    ..ready()
                }
            },
            |_| {
                submitted.set(true);
                true
            }
        )
        .is_err());
        assert_eq!(checks.get(), 2);
        assert!(!submitted.get());
        assert!(perform_with(WindowOperation::Minimize, ready, |_| false).is_err());
        assert!(
            perform_with(WindowOperation::Minimize, ready, |command| command
                == WindowCommand::Minimize)
            .is_ok()
        );
    }
}
