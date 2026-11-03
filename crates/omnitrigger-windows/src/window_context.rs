//! 角色采集只在 Hook 之外执行，保留真实进程名；不沿 owner 链分类。
use omnitrigger_core::WindowRole;
use windows::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{
        GetAncestor, GetClassNameW, GetDesktopWindow, GetShellWindow, GetWindowThreadProcessId,
        IsChild, GA_PARENT,
    },
};

fn ordinary_role(process: &str) -> Option<WindowRole> {
    let name = process.to_ascii_lowercase();
    let name = name.strip_suffix(".exe").unwrap_or(&name);
    if matches!(
        name,
        "desktopmgr64"
            | "desktopmgr"
            | "360desktop"
            | "360desktoplite"
            | "360desktoplite64"
            | "coodesker"
            | "coodesker-x64"
            | "xzdesktop64"
            | "xzdesktop"
    ) {
        Some(WindowRole::Desktop)
    } else if name != "explorer" {
        Some(WindowRole::Application)
    } else {
        None
    }
}

fn classify_explorer(shell_child: bool, classes: &[String], complete: bool) -> WindowRole {
    if shell_child
        || classes.first().is_some_and(|class| {
            matches!(class.as_str(), "SysListView32" | "WorkerW" | "TxMiniSkin")
        })
    {
        return WindowRole::Desktop;
    }
    // 从目标向父窗口走；先到达的明确容器决定角色，不看 owner。
    for class in classes {
        match class.as_str() {
            "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" => return WindowRole::Taskbar,
            "CabinetWClass" | "ExploreWClass" => return WindowRole::Application,
            _ => {}
        }
    }
    if complete {
        WindowRole::Application
    } else {
        WindowRole::Unknown
    }
}

/// expected_pid 必须来自同一次窗口身份捕获；采集前后再次核验线程和进程。
pub(crate) fn role(hwnd: HWND, expected_pid: u32, process: &str) -> WindowRole {
    let mut before_pid = 0;
    let before_thread = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut before_pid)) };
    if hwnd.0.is_null() || expected_pid == 0 || before_pid != expected_pid || before_thread == 0 {
        return WindowRole::Unknown;
    }
    let result = ordinary_role(process).unwrap_or_else(|| {
        let shell = unsafe { GetShellWindow() };
        let shell_child =
            !shell.0.is_null() && (shell == hwnd || unsafe { IsChild(shell, hwnd) }.as_bool());
        let desktop = unsafe { GetDesktopWindow() };
        let mut current = hwnd;
        let mut seen = Vec::with_capacity(32);
        let mut classes = Vec::with_capacity(32);
        let mut complete = false;
        for _ in 0..32 {
            if current.0.is_null() || current == desktop {
                complete = true;
                break;
            }
            if seen.contains(&current) {
                break;
            }
            seen.push(current);
            let mut class = [0u16; 256];
            let len = unsafe { GetClassNameW(current, &mut class) };
            if len <= 0 {
                break;
            }
            classes.push(String::from_utf16_lossy(&class[..len as usize]));
            current = unsafe { GetAncestor(current, GA_PARENT) };
        }
        classify_explorer(shell_child, &classes, complete)
    });
    let mut after_pid = 0;
    let after_thread = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut after_pid)) };
    if after_pid != before_pid || after_thread != before_thread {
        WindowRole::Unknown
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classify(classes: &[&str], complete: bool) -> WindowRole {
        classify_explorer(
            false,
            &classes.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            complete,
        )
    }

    #[test]
    fn shell_context_classification_uses_process_guard_and_nearest_parent() {
        assert_eq!(ordinary_role("Editor.EXE"), Some(WindowRole::Application));
        assert_eq!(
            ordinary_role("coodesker-x64.exe"),
            Some(WindowRole::Desktop)
        );
        assert_eq!(ordinary_role("Explorer.EXE"), None);
        assert_eq!(classify_explorer(true, &[], false), WindowRole::Desktop);
        for class in ["WorkerW", "SysListView32", "TxMiniSkin"] {
            assert_eq!(classify(&[class], false), WindowRole::Desktop);
        }
        assert_eq!(
            classify(&["child", "Shell_SecondaryTrayWnd"], false),
            WindowRole::Taskbar
        );
        assert_eq!(
            classify(&["CabinetWClass", "Shell_TrayWnd"], true),
            WindowRole::Application
        );
        assert_eq!(
            classify(&["Shell_TrayWnd", "CabinetWClass"], true),
            WindowRole::Taskbar
        );
        assert_eq!(classify(&["unknown"], false), WindowRole::Unknown);
        assert_eq!(classify(&["unknown"], true), WindowRole::Application);
        assert_eq!(role(HWND::default(), 0, "explorer"), WindowRole::Unknown);
    }

    #[test]
    #[ignore = "本机只读验证真实桌面、任务栏与可选文件窗口"]
    fn shell_context_live_windows() {
        use windows::{core::w, Win32::UI::WindowsAndMessaging::FindWindowW};
        fn check(hwnd: HWND, expected: WindowRole) {
            assert!(!hwnd.0.is_null());
            let mut pid = 0;
            unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
            let process =
                crate::shortcut_target_process(crate::ShortcutTarget(hwnd.0 as isize)).unwrap();
            assert_eq!(role(hwnd, pid, &process), expected);
            let info = crate::ScriptTarget::capture(crate::ShortcutTarget(hwnd.0 as isize))
                .unwrap()
                .info()
                .unwrap();
            assert_eq!(info.window_role, expected);
            assert_eq!(info.process_name, process);
            assert_eq!(info.process_id, pid);
            assert_eq!(
                role(hwnd, pid.wrapping_add(1), &process),
                WindowRole::Unknown
            );
        }
        check(unsafe { GetShellWindow() }, WindowRole::Desktop);
        check(
            unsafe { FindWindowW(w!("Shell_TrayWnd"), None) }.unwrap(),
            WindowRole::Taskbar,
        );
        if let Ok(hwnd) = unsafe { FindWindowW(w!("CabinetWClass"), None) } {
            check(hwnd, WindowRole::Application);
            eprintln!("文件窗口角色已验证");
        } else {
            eprintln!("没有现存文件窗口，未验证该项");
        }
    }
}
