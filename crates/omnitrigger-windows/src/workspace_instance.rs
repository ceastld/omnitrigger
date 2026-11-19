//! 同一配置窗口共享唤回事件，设置入口在唤回前另行标记目标页。
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{
            GetLastError, SetLastError, ERROR_ALREADY_EXISTS, ERROR_SUCCESS, HANDLE, WAIT_OBJECT_0,
        },
        System::Threading::{CreateEventW, ResetEvent, SetEvent, WaitForSingleObject},
    },
};

pub struct WorkspaceInstance {
    show: OwnedHandle,
    pages: [(WorkspacePage, OwnedHandle); 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspacePage {
    Current,
    Settings,
    WheelEditor,
    BrowserConnector,
}

fn take_event(event: &OwnedHandle) -> bool {
    unsafe { WaitForSingleObject(HANDLE(event.as_raw_handle()), 0) == WAIT_OBJECT_0 }
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
        Self::open_page(
            key,
            if settings_page {
                WorkspacePage::Settings
            } else {
                WorkspacePage::Current
            },
        )
    }

    pub fn open_page(key: &str, page: WorkspacePage) -> Result<Option<Self>, String> {
        Self::acquire(key, Some(page))
    }

    pub fn open_in_tray(key: &str) -> Result<Option<Self>, String> {
        Self::acquire(key, None)
    }

    fn acquire(key: &str, request: Option<WorkspacePage>) -> Result<Option<Self>, String> {
        let name = format!("Local\\OmniTrigger.Workspace.{key}");
        // 先打开各页事件。另一个进程一旦看到窗口身份，就能转交请求。
        let pages = [
            (
                WorkspacePage::Settings,
                workspace_event(&format!("{name}.Settings"))?.0,
            ),
            (
                WorkspacePage::WheelEditor,
                workspace_event(&format!("{name}.WheelEditor"))?.0,
            ),
            (
                WorkspacePage::BrowserConnector,
                workspace_event(&format!("{name}.BrowserConnector"))?.0,
            ),
        ];
        let (show, existing) = workspace_event(&name)?;
        if !existing {
            return Ok(Some(Self { show, pages }));
        }
        let Some(page) = request else {
            return Ok(None);
        };
        unsafe {
            if page != WorkspacePage::Current {
                // 唤回当前页时保留已提交的选页请求。连续指定不同页时采用最后一页。
                for (target, event) in &pages {
                    if *target != page {
                        ResetEvent(HANDLE(event.as_raw_handle()))
                            .map_err(|error| format!("重置窗口页面请求失败：{error}"))?;
                    }
                }
                for (target, event) in &pages {
                    if *target == page {
                        SetEvent(HANDLE(event.as_raw_handle()))
                            .map_err(|error| format!("通知原窗口切换页面失败：{error}"))?;
                    }
                }
            }
            SetEvent(HANDLE(show.as_raw_handle()))
                .map_err(|error| format!("通知原配置窗口失败：{error}"))?;
        }
        Ok(None)
    }

    pub fn take_open_request(&self) -> bool {
        take_event(&self.show)
    }

    pub fn take_settings_request(&self) -> bool {
        self.pages
            .iter()
            .any(|(page, event)| *page == WorkspacePage::Settings && take_event(event))
    }

    pub fn take_page_request(&self) -> Option<WorkspacePage> {
        if !self.take_open_request() {
            return None;
        }
        let mut requested = WorkspacePage::Current;
        for (page, event) in &self.pages {
            if take_event(event) {
                requested = *page;
            }
        }
        Some(requested)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_pages_coalesce_and_plain_launch_keeps_the_pending_page() {
        use WorkspacePage::*;
        let key = format!("page-routing-{}", std::process::id());
        let owner = WorkspaceInstance::open_page(&key, WheelEditor)
            .unwrap()
            .unwrap();
        assert_eq!(owner.take_page_request(), None);
        for (first, last) in [
            (Settings, WheelEditor),
            (WheelEditor, BrowserConnector),
            (BrowserConnector, Settings),
        ] {
            for page in [first, last, Current] {
                assert!(WorkspaceInstance::open_page(&key, page).unwrap().is_none());
            }
            assert!(WorkspaceInstance::open_in_tray(&key).unwrap().is_none());
            assert_eq!(owner.take_page_request(), Some(last));
            assert_eq!(owner.take_page_request(), None);
        }
        assert!(WorkspaceInstance::open(&key).unwrap().is_none());
        assert_eq!(owner.take_page_request(), Some(Current));
    }

    #[test]
    fn duplicate_launch_wakes_only_its_workspace_and_exit_allows_reopen() {
        let key = format!("test-{}", std::process::id());
        let first = WorkspaceInstance::open(&key).unwrap().unwrap();
        let other = WorkspaceInstance::open(&format!("{key}-other"))
            .unwrap()
            .unwrap();
        assert!(!first.take_open_request());
        assert!(WorkspaceInstance::open_in_tray(&key).unwrap().is_none());
        assert!(!first.take_open_request() && !first.take_settings_request());
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
