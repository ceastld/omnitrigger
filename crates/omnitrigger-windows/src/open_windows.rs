use std::{ffi::c_void, mem::size_of};

use windows::{
    core::BOOL,
    Win32::{
        Foundation::{HWND, LPARAM},
        Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED},
        UI::WindowsAndMessaging::{
            EnumWindows, GetForegroundWindow, GetShellWindow, GetWindow, GetWindowLongPtrW,
            GetWindowThreadProcessId, IsIconic, IsWindowVisible, SetForegroundWindow,
            ShowWindowAsync, GWL_EXSTYLE, GW_OWNER, SW_RESTORE, WS_EX_APPWINDOW, WS_EX_NOACTIVATE,
            WS_EX_TOOLWINDOW,
        },
    },
};

use crate::{shortcut_target_process, shortcut_target_title, ShortcutTarget};

/// Windows 窗口搜索的一项。句柄只由适配层读取、校验和使用。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenWindow {
    pub title: String,
    pub process: Option<String>,
    target: ShortcutTarget,
    identity: (u32, u32),
}

impl OpenWindow {
    /// 比较 Windows 窗口标识，保留列表刷新期间仍然匹配的窗口。
    pub fn same_window(&self, other: &Self) -> bool {
        self.target == other.target && self.identity == other.identity
    }

    /// 由用户选中窗口后请求切到前台，最小化的窗口先请求还原。
    pub fn activate(&self) -> Result<(), String> {
        let handle = HWND(self.target.0 as *mut c_void);
        if window_identity(handle) != Some(self.identity) || !usable_window(handle) {
            return Err("窗口已关闭或隐藏，请重新打开搜索框。".into());
        }
        if unsafe { IsIconic(handle) }.as_bool()
            && !unsafe { ShowWindowAsync(handle, SW_RESTORE) }.as_bool()
        {
            return Err("Windows 未能还原目标窗口。".into());
        }
        if unsafe { GetForegroundWindow() } == handle
            || unsafe { SetForegroundWindow(handle) }.as_bool()
        {
            Ok(())
        } else {
            Err("Windows 未能切换到目标窗口。".into())
        }
    }
}

/// 读取当前桌面可见或最小化的普通顶层窗口，不访问窗口内容。
pub fn list_open_windows() -> Result<Vec<OpenWindow>, String> {
    let mut windows = Vec::new();
    // EnumWindows 在返回前调用回调，参数始终指向此函数中独占的 Vec。
    unsafe {
        EnumWindows(
            Some(collect_window),
            LPARAM(&mut windows as *mut Vec<OpenWindow> as isize),
        )
    }
    .map_err(|error| format!("无法读取当前窗口列表。{error}"))?;
    Ok(windows)
}

unsafe extern "system" fn collect_window(handle: HWND, data: LPARAM) -> BOOL {
    if let Some(window) = inspect_window(handle) {
        // 只有 list_open_windows 会注册此回调并提供这个同线程指针。
        unsafe { &mut *(data.0 as *mut Vec<OpenWindow>) }.push(window);
    }
    BOOL(1)
}

fn inspect_window(handle: HWND) -> Option<OpenWindow> {
    let identity = window_identity(handle)?;
    if identity.0 == std::process::id() || !usable_window(handle) {
        return None;
    }
    let target = ShortcutTarget(handle.0 as isize);
    let title = shortcut_target_title(target).ok()?;
    let title = title.trim();
    if title.is_empty() {
        return None;
    }
    let process = shortcut_target_process(target).ok();
    (window_identity(handle) == Some(identity)).then(|| OpenWindow {
        title: title.to_owned(),
        process,
        target,
        identity,
    })
}

fn window_identity(handle: HWND) -> Option<(u32, u32)> {
    let mut process = 0;
    let thread = unsafe { GetWindowThreadProcessId(handle, Some(&mut process)) };
    (process != 0 && thread != 0).then_some((process, thread))
}

fn usable_window(handle: HWND) -> bool {
    if handle == unsafe { GetShellWindow() } || !unsafe { IsWindowVisible(handle) }.as_bool() {
        return false;
    }
    let style = unsafe { GetWindowLongPtrW(handle, GWL_EXSTYLE) } as u32;
    if style & (WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE).0 != 0
        || (style & WS_EX_APPWINDOW.0 == 0 && unsafe { GetWindow(handle, GW_OWNER) }.is_ok())
    {
        return false;
    }
    let mut cloaked = 0u32;
    // 输出缓冲区为一个 DWORD，按窗口当前的 DWM 状态写入。
    let visibility = unsafe {
        DwmGetWindowAttribute(
            handle,
            DWMWA_CLOAKED,
            (&mut cloaked as *mut u32).cast(),
            size_of::<u32>() as u32,
        )
    };
    visibility.is_ok() && cloaked == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_window_identity_when_its_title_changes() {
        let original = OpenWindow {
            title: "初始标题".to_owned(),
            process: None,
            target: ShortcutTarget(42),
            identity: (10, 20),
        };
        let mut current = original.clone();
        current.title = "新标题".to_owned();
        assert!(original.same_window(&current));
        current.identity = (10, 21);
        assert!(!original.same_window(&current));
        current.identity = original.identity;
        current.target = ShortcutTarget(43);
        assert!(!original.same_window(&current));
    }
}
