//! 动作提交时捕获的目标身份。每次调用重新核验，不隐式改选前台。
use crate::{
    ensure_shortcut_target, shortcut_target_process, shortcut_target_title, ShortcutTarget,
};
use windows::Win32::{
    Foundation::{CloseHandle, FILETIME, HANDLE, HWND},
    System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindow},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Identity {
    pid: u32,
    thread_id: u32,
    process_created: u64,
}

/// 句柄仅留在平台层；进程创建时间用于拒绝 PID 被复用后的目标。
/// 这些核验不是对 Windows 窗口生命周期的原子锁定。
#[derive(Clone, Copy, Debug)]
pub struct ScriptTarget {
    target: ShortcutTarget,
    identity: Identity,
}

#[derive(Debug)]
pub struct ScriptTargetInfo {
    pub process_name: String,
    pub title: String,
    pub process_id: u32,
    pub is_foreground: bool,
}

fn identity(target: ShortcutTarget) -> Result<Identity, String> {
    let hwnd = HWND(target.0 as *mut std::ffi::c_void);
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return Err("本次动作的目标窗口已关闭或失效。".into());
    }
    let mut pid = 0;
    let thread_id = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if thread_id == 0 || pid == 0 {
        return Err("无法确定目标窗口的进程身份。".into());
    }
    struct Process(HANDLE);
    impl Drop for Process {
        fn drop(&mut self) {
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
    let process = Process(
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
            .map_err(|error| format!("无法捕获目标进程身份：{error}"))?,
    );
    let (mut created, mut exited, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    unsafe { GetProcessTimes(process.0, &mut created, &mut exited, &mut kernel, &mut user) }
        .map_err(|error| format!("无法读取目标进程创建时间：{error}"))?;
    let mut after_pid = 0;
    let after_thread = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut after_pid)) };
    if after_pid != pid || after_thread != thread_id {
        return Err("捕获期间目标窗口身份已改变。".into());
    }
    Ok(Identity {
        pid,
        thread_id,
        process_created: (u64::from(created.dwHighDateTime) << 32)
            | u64::from(created.dwLowDateTime),
    })
}

impl ScriptTarget {
    pub fn capture(target: ShortcutTarget) -> Result<Self, String> {
        Ok(Self {
            target,
            identity: identity(target)?,
        })
    }

    fn ensure_identity(&self) -> Result<(), String> {
        if identity(self.target)? != self.identity {
            return Err("目标窗口或进程身份已改变，已取消本机调用。".into());
        }
        Ok(())
    }

    pub fn info(&self) -> Result<ScriptTargetInfo, String> {
        self.ensure_identity()?;
        let info = ScriptTargetInfo {
            process_name: shortcut_target_process(self.target)?,
            title: shortcut_target_title(self.target)?,
            process_id: self.identity.pid,
            is_foreground: ensure_shortcut_target(self.target).is_ok(),
        };
        self.ensure_identity()?;
        Ok(info)
    }

    pub fn ensure_foreground(&self) -> Result<(), String> {
        self.ensure_identity()?;
        ensure_shortcut_target(self.target)
    }

    pub fn send_shortcut(&self, keys: &[u16]) -> Result<(), String> {
        self.send_shortcut_if(keys, || Ok(()))
    }

    /// 等待输入串行门闩后再核验目标和取消条件，避免借用等待前的旧快照。
    pub fn send_shortcut_if(
        &self,
        keys: &[u16],
        check: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        crate::shortcut::send_shortcut_checked(keys, self.target, || {
            self.ensure_foreground()?;
            check()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::{
        core::w,
        Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WINDOW_STYLE,
        },
    };

    #[test]
    fn script_target_reads_hidden_window_and_rejects_stale_identity_without_input() {
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("独立目标验证"),
                WINDOW_STYLE(0),
                0,
                0,
                1,
                1,
                None,
                None,
                None,
                None,
            )
        }
        .unwrap();
        let target = ScriptTarget::capture(ShortcutTarget(hwnd.0 as isize)).unwrap();
        let info = target.info().unwrap();
        assert_eq!(info.process_id, std::process::id());
        assert_eq!(info.title, "独立目标验证");
        assert!(!info.is_foreground);
        assert!(info.process_name.ends_with(".exe"));
        assert!(target.ensure_foreground().is_err());
        let mut stale = target;
        stale.identity.process_created ^= 1;
        assert!(stale.info().is_err());
        stale = target;
        stale.identity.thread_id ^= 1;
        assert!(stale.info().is_err());
        unsafe { DestroyWindow(hwnd) }.unwrap();
        assert!(target.info().is_err());
        assert!(target.send_shortcut(&[65]).is_err());
    }
}
