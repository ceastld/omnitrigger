//! 窗口动作由外层执行器调用；不进入输入 Hook，不模拟键盘，也不主动获取前台。

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowOperation {
    ToggleMaximize,
    /// 显式请求正常显示，不激活、不按当前最大化状态切换。
    Restore,
    ToggleTopmost,
    Minimize,
    /// 请求窗口按系统菜单“关闭”处理，等同 Alt+F4；应用可以询问保存或拒绝。
    Close,
}

#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, Default)]
struct TargetState {
    valid: bool,
    visible: bool,
    foreground: bool,
    top_level: bool,
    maximized: bool,
    topmost: bool,
    minimized: bool,
    can_maximize: bool,
    can_minimize: bool,
}

#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WindowCommand {
    SetTopmost(bool),
    Maximize,
    Restore,
    RestoreNoActivate,
    Minimize,
    Close,
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
        WindowOperation::Restore => Ok(WindowCommand::RestoreNoActivate),
        WindowOperation::ToggleMaximize if state.maximized => Ok(WindowCommand::Restore),
        WindowOperation::ToggleMaximize if state.can_maximize => Ok(WindowCommand::Maximize),
        WindowOperation::Minimize if state.can_minimize => Ok(WindowCommand::Minimize),
        WindowOperation::ToggleTopmost => Ok(WindowCommand::SetTopmost(!state.topmost)),
        WindowOperation::Close => Ok(WindowCommand::Close),
        WindowOperation::ToggleMaximize => Err("目标窗口不支持最大化。".into()),
        WindowOperation::Minimize => Err("目标窗口不支持最小化。".into()),
    }
}

#[cfg(any(windows, test))]
fn perform_with(
    operation: WindowOperation,
    mut inspect: impl FnMut() -> TargetState,
    submit: impl FnOnce(WindowCommand) -> bool,
    mut check: impl FnMut() -> Result<(), String>,
) -> Result<(), String> {
    check()?;
    plan(inspect(), operation)?;
    // 提交前重新核验；切换前台或窗口在检查期间关闭时不向旧目标发送请求。
    check()?;
    let command = plan(inspect(), operation)?;
    check()?;
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
    perform_window_action_checked(target, operation, || Ok(()))
}

#[cfg(windows)]
pub(crate) fn perform_window_action_checked(
    target: crate::ShortcutTarget,
    operation: WindowOperation,
    check: impl FnMut() -> Result<(), String>,
) -> Result<(), String> {
    use windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{
            GetAncestor, GetForegroundWindow, GetWindowLongPtrW, IsIconic, IsWindow,
            IsWindowVisible, IsZoomed, PostMessageW, SetWindowPos, ShowWindowAsync, GA_ROOT,
            GWL_EXSTYLE, GWL_STYLE, HWND_NOTOPMOST, HWND_TOPMOST, SC_CLOSE, SWP_ASYNCWINDOWPOS,
            SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SW_MAXIMIZE, SW_MINIMIZE,
            SW_RESTORE, SW_SHOWNOACTIVATE, WM_SYSCOMMAND, WS_EX_TOPMOST, WS_MAXIMIZEBOX,
            WS_MINIMIZEBOX,
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
                topmost: unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32 & WS_EX_TOPMOST.0
                    != 0,
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
                WindowCommand::SetTopmost(topmost) => {
                    // 只改原窗口Z序；不激活、不移动或缩放，不修改整个扩展样式。
                    return unsafe {
                        SetWindowPos(
                            hwnd,
                            Some(if topmost {
                                HWND_TOPMOST
                            } else {
                                HWND_NOTOPMOST
                            }),
                            0,
                            0,
                            0,
                            0,
                            SWP_NOMOVE
                                | SWP_NOSIZE
                                | SWP_NOACTIVATE
                                | SWP_NOOWNERZORDER
                                | SWP_ASYNCWINDOWPOS,
                        )
                    }
                    .is_ok();
                }
                WindowCommand::Close => {
                    return unsafe {
                        PostMessageW(
                            Some(hwnd),
                            WM_SYSCOMMAND,
                            windows::Win32::Foundation::WPARAM(SC_CLOSE as usize),
                            windows::Win32::Foundation::LPARAM(0),
                        )
                    }
                    .is_ok();
                }
                WindowCommand::Maximize => SW_MAXIMIZE,
                WindowCommand::Restore => SW_RESTORE,
                // 显式还原只请求正常显示，不激活；正常窗口重复调用也不会最大化。
                WindowCommand::RestoreNoActivate => SW_SHOWNOACTIVATE,
                WindowCommand::Minimize => SW_MINIMIZE,
            };
            unsafe { ShowWindowAsync(hwnd, command) }.as_bool()
        },
        check,
    )
}

/// 读取捕获窗口的标题文本，目标不必仍为前台；空标题返回空字符串。
#[cfg(windows)]
pub fn shortcut_target_title(target: crate::ShortcutTarget) -> Result<String, String> {
    use windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{GetWindowTextLengthW, GetWindowTextW, IsWindow},
    };
    let hwnd = HWND(target.0 as *mut std::ffi::c_void);
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return Err("目标窗口已关闭或失效，无法读取标题。".into());
    }
    let length = unsafe { GetWindowTextLengthW(hwnd) }.clamp(0, 4096) as usize;
    let mut buffer = vec![0u16; length + 1];
    let copied = unsafe { GetWindowTextW(hwnd, &mut buffer) }.max(0) as usize;
    Ok(String::from_utf16_lossy(&buffer[..copied.min(length)]))
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
    fn close_needs_only_a_visible_foreground_top_level_target() {
        let fixed = TargetState {
            can_maximize: false,
            can_minimize: false,
            ..ready()
        };
        assert_eq!(
            plan(fixed, WindowOperation::Close).unwrap(),
            WindowCommand::Close
        );
        for state in [
            TargetState {
                foreground: false,
                ..ready()
            },
            TargetState {
                top_level: false,
                ..ready()
            },
            TargetState::default(),
        ] {
            assert!(plan(state, WindowOperation::Close).is_err());
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
                },
                || Ok(()),
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
            },
            || Ok(()),
        )
        .is_err());
        assert_eq!(checks.get(), 2);
        assert!(!submitted.get());
        assert!(perform_with(WindowOperation::Minimize, ready, |_| false, || Ok(())).is_err());
        assert!(perform_with(
            WindowOperation::Minimize,
            ready,
            |command| command == WindowCommand::Minimize,
            || Ok(())
        )
        .is_ok());
    }

    #[test]
    fn identity_and_cancellation_checks_prevent_submission_at_each_boundary() {
        for rejected_check in 1..=3 {
            let checks = Cell::new(0);
            let submitted = Cell::new(false);
            let error = perform_with(
                WindowOperation::Close,
                ready,
                |_| {
                    submitted.set(true);
                    true
                },
                || {
                    checks.set(checks.get() + 1);
                    if checks.get() == rejected_check {
                        Err("原目标或取消状态已改变".into())
                    } else {
                        Ok(())
                    }
                },
            )
            .unwrap_err();
            assert_eq!(error, "原目标或取消状态已改变");
            assert_eq!(checks.get(), rejected_check);
            assert!(!submitted.get());
        }
    }
}

#[cfg(test)]
mod topmost_tests {
    use super::*;
    use std::cell::Cell;

    fn ready() -> TargetState {
        TargetState {
            valid: true,
            visible: true,
            foreground: true,
            top_level: true,
            ..Default::default()
        }
    }

    #[test]
    fn topmost_plan_uses_current_state_without_requiring_resize_support() {
        for topmost in [false, true] {
            assert_eq!(
                plan(
                    TargetState { topmost, ..ready() },
                    WindowOperation::ToggleTopmost
                )
                .unwrap(),
                WindowCommand::SetTopmost(!topmost)
            );
        }
    }

    #[test]
    fn topmost_is_reinspected_before_the_single_submission() {
        let inspections = Cell::new(0);
        let submitted = Cell::new(0);
        perform_with(
            WindowOperation::ToggleTopmost,
            || {
                let count = inspections.get();
                inspections.set(count + 1);
                TargetState {
                    topmost: count != 0,
                    ..ready()
                }
            },
            |command| {
                assert_eq!(command, WindowCommand::SetTopmost(false));
                submitted.set(submitted.get() + 1);
                true
            },
            || Ok(()),
        )
        .unwrap();
        assert_eq!(inspections.get(), 2);
        assert_eq!(submitted.get(), 1);
    }

    #[test]
    fn invalid_hidden_child_minimized_or_changed_target_never_submits_topmost() {
        for state in [
            TargetState::default(),
            TargetState {
                visible: false,
                ..ready()
            },
            TargetState {
                foreground: false,
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
            assert!(perform_with(
                WindowOperation::ToggleTopmost,
                || state,
                |_| panic!("失效或失焦原目标不能提交置顶"),
                || Ok(())
            )
            .is_err());
        }
    }

    #[test]
    fn topmost_identity_cancel_or_deadline_guard_stops_at_each_real_boundary() {
        for stop in 1..=3 {
            let checks = Cell::new(0);
            let error = perform_with(
                WindowOperation::ToggleTopmost,
                ready,
                |_| panic!("检查失败不得提交置顶"),
                || {
                    checks.set(checks.get() + 1);
                    if checks.get() == stop {
                        Err("原目标/取消/期限守卫拒绝".into())
                    } else {
                        Ok(())
                    }
                },
            )
            .unwrap_err();
            assert_eq!(error, "原目标/取消/期限守卫拒绝");
            assert_eq!(checks.get(), stop);
        }
    }

    #[test]
    fn native_topmost_rejection_is_returned_without_retry() {
        let calls = Cell::new(0);
        assert!(perform_with(
            WindowOperation::ToggleTopmost,
            ready,
            |command| {
                assert_eq!(command, WindowCommand::SetTopmost(true));
                calls.set(calls.get() + 1);
                false
            },
            || Ok(()),
        )
        .is_err());
        assert_eq!(calls.get(), 1);
    }
}

#[cfg(test)]
mod restore_tests {
    use super::*;
    use std::cell::Cell;

    fn ready(maximized: bool) -> TargetState {
        TargetState {
            valid: true,
            visible: true,
            foreground: true,
            top_level: true,
            maximized,
            // 显式还原不依赖最大化/最小化按钮能力。
            ..Default::default()
        }
    }

    #[test]
    fn restore_is_idempotent_for_normal_and_maximized_fixed_windows() {
        for maximized in [false, true] {
            assert_eq!(
                plan(ready(maximized), WindowOperation::Restore).unwrap(),
                WindowCommand::RestoreNoActivate
            );
        }
    }

    #[test]
    fn restore_never_becomes_maximize_when_state_changes_before_submission() {
        for initially_maximized in [false, true] {
            let inspections = Cell::new(0);
            let submissions = Cell::new(0);
            perform_with(
                WindowOperation::Restore,
                || {
                    let index = inspections.get();
                    inspections.set(index + 1);
                    ready(if index == 0 {
                        initially_maximized
                    } else {
                        !initially_maximized
                    })
                },
                |command| {
                    assert_eq!(command, WindowCommand::RestoreNoActivate);
                    submissions.set(submissions.get() + 1);
                    true
                },
                || Ok(()),
            )
            .unwrap();
            assert_eq!(inspections.get(), 2);
            assert_eq!(submissions.get(), 1);
        }
    }

    #[test]
    fn restore_invalid_hidden_child_minimized_or_changed_target_never_submits() {
        for state in [
            TargetState::default(),
            TargetState {
                visible: false,
                ..ready(true)
            },
            TargetState {
                foreground: false,
                ..ready(true)
            },
            TargetState {
                top_level: false,
                ..ready(true)
            },
            TargetState {
                minimized: true,
                ..ready(true)
            },
        ] {
            assert!(perform_with(
                WindowOperation::Restore,
                || state,
                |_| panic!("失效或失焦原目标不得提交还原"),
                || Ok(()),
            )
            .is_err());
        }
    }

    #[test]
    fn restore_checks_identity_cancel_and_deadline_at_every_existing_boundary() {
        for stop in 1..=3 {
            let checks = Cell::new(0);
            assert!(perform_with(
                WindowOperation::Restore,
                || ready(true),
                |_| panic!("守卫失败不得提交还原"),
                || {
                    checks.set(checks.get() + 1);
                    if checks.get() == stop {
                        Err("守卫拒绝".into())
                    } else {
                        Ok(())
                    }
                },
            )
            .is_err());
            assert_eq!(checks.get(), stop);
        }
    }

    #[test]
    fn restore_native_rejection_is_returned_after_one_submission_without_retry() {
        let submissions = Cell::new(0);
        assert!(perform_with(
            WindowOperation::Restore,
            || ready(true),
            |command| {
                assert_eq!(command, WindowCommand::RestoreNoActivate);
                submissions.set(submissions.get() + 1);
                false
            },
            || Ok(()),
        )
        .is_err());
        assert_eq!(submissions.get(), 1);
    }
}

#[cfg(test)]
mod captured_toggle_contract_tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    fn ready(maximized: bool) -> TargetState {
        TargetState {
            valid: true,
            visible: true,
            foreground: true,
            top_level: true,
            maximized,
            can_maximize: true,
            ..Default::default()
        }
    }

    #[test]
    fn toggle_replans_from_the_second_inspection_and_keeps_all_pre_submit_guards() {
        let reads = Cell::new(0);
        let events = RefCell::new(Vec::new());
        perform_with(
            WindowOperation::ToggleMaximize,
            || {
                events.borrow_mut().push("inspect");
                reads.set(reads.get() + 1);
                ready(reads.get() == 2)
            },
            |command| {
                events.borrow_mut().push("submit");
                assert_eq!(command, WindowCommand::Restore);
                true
            },
            || {
                events.borrow_mut().push("guard");
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(
            *events.borrow(),
            ["guard", "inspect", "guard", "inspect", "guard", "submit"]
        );
        perform_with(
            WindowOperation::ToggleMaximize,
            || ready(false),
            |command| command == WindowCommand::Maximize,
            || Ok(()),
        )
        .unwrap();
    }

    #[test]
    fn captured_toggle_never_submits_after_any_guard_or_foreground_boundary_fails() {
        for boundary in 1..=3 {
            let guards = Cell::new(0);
            let submitted = Cell::new(false);
            assert!(perform_with(
                WindowOperation::ToggleMaximize,
                || ready(false),
                |_| {
                    submitted.set(true);
                    true
                },
                || {
                    guards.set(guards.get() + 1);
                    if guards.get() == boundary {
                        Err("合成身份/租约/取消/期限守卫拒绝".into())
                    } else {
                        Ok(())
                    }
                },
            )
            .is_err());
            assert!(!submitted.get());
        }
        let reads = Cell::new(0);
        let submitted = Cell::new(false);
        assert!(perform_with(
            WindowOperation::ToggleMaximize,
            || {
                reads.set(reads.get() + 1);
                TargetState {
                    foreground: reads.get() == 1,
                    ..ready(false)
                }
            },
            |_| {
                submitted.set(true);
                true
            },
            || Ok(()),
        )
        .is_err());
        assert_eq!(reads.get(), 2);
        assert!(!submitted.get());
    }
}
