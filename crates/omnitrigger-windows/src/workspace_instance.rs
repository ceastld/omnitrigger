//! 每个 Windows 会话为同一配置窗口保留一个事件。重复启动时请求显示原窗口。
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

pub struct WorkspaceInstance(OwnedHandle);

impl WorkspaceInstance {
    pub fn open(key: &str) -> Result<Option<Self>, String> {
        let name: Vec<u16> = format!("Local\\OmniTrigger.Workspace.{key}\0")
            .encode_utf16()
            .collect();
        unsafe {
            SetLastError(ERROR_SUCCESS);
            let event = CreateEventW(None, false, false, PCWSTR(name.as_ptr()))
                .map_err(|error| format!("打开配置窗口事件失败：{error}"))?;
            let existing = GetLastError() == ERROR_ALREADY_EXISTS;
            let handle = OwnedHandle::from_raw_handle(event.0);
            if existing {
                SetEvent(event).map_err(|error| format!("通知原配置窗口失败：{error}"))?;
                Ok(None)
            } else {
                Ok(Some(Self(handle)))
            }
        }
    }

    pub fn take_open_request(&self) -> bool {
        unsafe { WaitForSingleObject(HANDLE(self.0.as_raw_handle()), 0) == WAIT_OBJECT_0 }
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
        drop(first);
        assert!(WorkspaceInstance::open(&key).unwrap().is_some());
    }
}
