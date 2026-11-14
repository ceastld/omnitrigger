//! 同一配置窗口共享唤回事件，设置入口在唤回前另行标记目标页。
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{
            GetLastError, SetLastError, ERROR_ALREADY_EXISTS, ERROR_SUCCESS, HANDLE, WAIT_OBJECT_0,
        },
        System::Threading::{CreateEventW, SetEvent, WaitForSingleObject},
    },
};

pub struct WorkspaceInstance {
    show: OwnedHandle,
    settings: OwnedHandle,
}

fn workspace_event(name: &str) -> Result<(OwnedHandle, bool), String> {
    let name: Vec<u16> = name.encode_utf16().chain([0]).collect();
    unsafe {
        SetLastError(ERROR_SUCCESS);
        let event = CreateEventW(None, false, false, PCWSTR(name.as_ptr()))
            .map_err(|error| format!("打开配置窗口事件失败：{error}"))?;
        let existing = GetLastError() == ERROR_ALREADY_EXISTS;
        Ok((OwnedHandle::from_raw_handle(event.0), existing))
    }
}

impl WorkspaceInstance {
    pub fn open(key: &str) -> Result<Option<Self>, String> {
        Self::open_with_settings(key, false)
    }

    pub fn open_with_settings(key: &str, settings_page: bool) -> Result<Option<Self>, String> {
        let name = format!("Local\\OmniTrigger.Workspace.{key}");
        // 先打开页请求事件，确保并发启动时，取得窗口身份的进程能读到最早的设置请求。
        let (settings, _) = workspace_event(&format!("{name}.Settings"))?;
        let (show, existing) = workspace_event(&name)?;
        if !existing {
            return Ok(Some(Self { show, settings }));
        }
        unsafe {
            if settings_page {
                SetEvent(HANDLE(settings.as_raw_handle()))
                    .map_err(|error| format!("通知原窗口打开设置失败：{error}"))?;
            }
            SetEvent(HANDLE(show.as_raw_handle()))
                .map_err(|error| format!("通知原配置窗口失败：{error}"))?;
        }
        Ok(None)
    }

    pub fn take_open_request(&self) -> bool {
        unsafe { WaitForSingleObject(HANDLE(self.show.as_raw_handle()), 0) == WAIT_OBJECT_0 }
    }

    pub fn take_settings_request(&self) -> bool {
        unsafe { WaitForSingleObject(HANDLE(self.settings.as_raw_handle()), 0) == WAIT_OBJECT_0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_launch_wakes_only_its_workspace_and_exit_allows_reopen() {
        let key = format!("test-{}", std::process::id());
        let first = WorkspaceInstance::open(&key).unwrap().unwrap();
        let other = WorkspaceInstance::open(&format!("{key}-other"))
            .unwrap()
            .unwrap();
        assert!(!first.take_open_request());
        assert!(WorkspaceInstance::open(&key).unwrap().is_none());
        assert!(first.take_open_request());
        assert!(!first.take_open_request());
        assert!(!other.take_open_request());
        assert!(!first.take_settings_request());
        assert!(WorkspaceInstance::open_with_settings(&key, true)
            .unwrap()
            .is_none());
        assert!(WorkspaceInstance::open(&key).unwrap().is_none());
        assert!(first.take_open_request() && first.take_settings_request());
        assert!(!first.take_settings_request());
        assert!(!other.take_open_request() && !other.take_settings_request());
        drop(first);
        assert!(WorkspaceInstance::open(&key).unwrap().is_some());
    }
}
