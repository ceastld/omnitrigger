//! 原Explorer目标的只读文件系统选区。平台COM入口只能由受控一次性helper调用。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExplorerContextSnapshot {
    pub current_directory: Option<String>,
    pub selected_paths: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExplorerReadError {
    Timeout,
    TargetChanged,
    UnsupportedTarget,
    AmbiguousTarget,
    SelectionChanged,
    Unavailable,
    UnsupportedItem,
    TooLarge,
    InvalidResponse,
}
impl ExplorerReadError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::TargetChanged => "target_changed",
            Self::UnsupportedTarget => "unsupported_target",
            Self::AmbiguousTarget => "ambiguous_target",
            Self::SelectionChanged => "selection_changed",
            Self::Unavailable => "selection_unavailable",
            Self::UnsupportedItem => "unsupported_item",
            Self::TooLarge => "selection_too_large",
            Self::InvalidResponse => "invalid_response",
        }
    }
}

fn valid_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    let local = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && bytes[2] == b'\\'
        && !path[2..].contains(':');
    let unc = path.strip_prefix("\\\\").is_some_and(|rest| {
        let mut parts = rest.split('\\');
        let server = parts.next().unwrap_or("");
        let share = parts.next().unwrap_or("");
        !server.is_empty()
            && !share.is_empty()
            && server != "."
            && server != "?"
            && !path.contains(':')
            && !path.contains('/')
    });
    (local || unc)
        && !path.chars().any(|c| c < ' ' || "<>\"|?*".contains(c))
        && !path.split('\\').any(|part| part == "." || part == "..")
}
pub fn validate_explorer_context(value: &ExplorerContextSnapshot) -> Result<(), ExplorerReadError> {
    if value.selected_paths.len() > 32 {
        return Err(ExplorerReadError::TooLarge);
    }
    let mut total = 0;
    let mut unique = std::collections::BTreeSet::new();
    for path in value
        .current_directory
        .iter()
        .chain(value.selected_paths.iter())
    {
        let length = path.encode_utf16().count();
        if length > 1024 {
            return Err(ExplorerReadError::TooLarge);
        }
        if !valid_path(path) {
            return Err(ExplorerReadError::InvalidResponse);
        }
        total += length;
    }
    if total > 8192 {
        return Err(ExplorerReadError::TooLarge);
    }
    for path in &value.selected_paths {
        if !unique.insert(path.to_lowercase()) {
            return Err(ExplorerReadError::InvalidResponse);
        }
    }
    Ok(())
}

#[cfg(windows)]
mod platform {
    use super::*;
    use crate::ScriptTarget;
    use std::{ffi::c_void, time::Instant};
    use windows::{
        core::{IUnknown, Interface, PWSTR},
        Win32::{
            Foundation::HWND,
            System::{
                Com::{
                    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize,
                    IServiceProvider, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
                },
                SystemServices::SFGAO_FILESYSTEM,
                Variant::VARIANT,
            },
            UI::{
                Shell::{
                    IFolderView2, ILGetSize, IPersistFolder2, IShellBrowser, IShellItem,
                    IShellView, IShellWindows, IWebBrowser2, SHCreateItemFromIDList,
                    SID_STopLevelBrowser, ShellWindows, SIGDN_FILESYSPATH, SVGIO_SELECTION,
                },
                WindowsAndMessaging::{
                    GetAncestor, GetClassNameW, GetWindowThreadProcessId, IsWindowVisible, GA_ROOT,
                },
            },
        },
    };
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }
    struct TaskMem(*mut c_void);
    impl Drop for TaskMem {
        fn drop(&mut self) {
            unsafe { CoTaskMemFree(Some(self.0)) };
        }
    }
    fn provider<T>(
        guard: &mut impl FnMut() -> Result<(), ExplorerReadError>,
        call: impl FnOnce() -> windows::core::Result<T>,
    ) -> Result<T, ExplorerReadError> {
        guard()?;
        let value = call();
        guard()?;
        value.map_err(|_| ExplorerReadError::Unavailable)
    }
    fn path(
        item: &IShellItem,
        guard: &mut impl FnMut() -> Result<(), ExplorerReadError>,
    ) -> Result<Option<String>, ExplorerReadError> {
        let attrs = provider(guard, || unsafe { item.GetAttributes(SFGAO_FILESYSTEM) })?;
        if attrs.0 & SFGAO_FILESYSTEM.0 == 0 {
            return Ok(None);
        }
        let raw: PWSTR = provider(guard, || unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) })?;
        let _memory = TaskMem(raw.0.cast());
        if raw.is_null() {
            return Err(ExplorerReadError::InvalidResponse);
        }
        let mut units = Vec::new();
        // COM分配的NUL结尾UTF16；只取1025哨兵，超限拒绝，不返回前缀。
        for offset in 0..=1024 {
            let unit = unsafe { *raw.0.add(offset) };
            if unit == 0 {
                return String::from_utf16(&units)
                    .map(Some)
                    .map_err(|_| ExplorerReadError::InvalidResponse);
            }
            units.push(unit);
        }
        Err(ExplorerReadError::TooLarge)
    }
    fn visible_view(
        view: &IShellView,
        root: HWND,
        pid: u32,
        guard: &mut impl FnMut() -> Result<(), ExplorerReadError>,
    ) -> Result<Option<HWND>, ExplorerReadError> {
        let hwnd = provider(guard, || unsafe { view.GetWindow() })?;
        guard()?;
        let mut actual_pid = 0;
        let thread = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut actual_pid)) };
        let owned =
            thread != 0 && actual_pid == pid && unsafe { GetAncestor(hwnd, GA_ROOT) } == root;
        let visible = unsafe { IsWindowVisible(hwnd) }.as_bool();
        guard()?;
        if !owned {
            return Err(ExplorerReadError::TargetChanged);
        }
        Ok(visible.then_some(hwnd))
    }
    fn bind(
        windows: &IShellWindows,
        root: HWND,
        pid: u32,
        guard: &mut impl FnMut() -> Result<(), ExplorerReadError>,
    ) -> Result<(HWND, IShellView, IFolderView2, IUnknown), ExplorerReadError> {
        let count = provider(guard, || unsafe { windows.Count() })?;
        if !(0..=128).contains(&count) {
            return Err(ExplorerReadError::Unavailable);
        }
        let mut found: Option<(HWND, IShellView, IFolderView2, IUnknown)> = None;
        for index in 0..count {
            let dispatch = provider(guard, || unsafe { windows.Item(&VARIANT::from(index)) })?;
            // ShellWindows可含其他对象；只尝试其浏览器接口，不读取其路径/选区。
            let Ok(browser) = dispatch.cast::<IWebBrowser2>() else {
                continue;
            };
            let hwnd = provider(guard, || unsafe { browser.HWND() })?;
            if hwnd.0 as usize != root.0 as usize {
                continue;
            }
            let services: IServiceProvider = provider(guard, || browser.cast())?;
            let browser: IShellBrowser = provider(guard, || unsafe {
                services.QueryService(&SID_STopLevelBrowser)
            })?;
            let view = provider(guard, || unsafe { browser.QueryActiveShellView() })?;
            let Some(view_hwnd) = visible_view(&view, root, pid, guard)? else {
                continue;
            };
            let folder: IFolderView2 = provider(guard, || view.cast())?;
            // 同一原窗口的多个ShellWindows条目可投影到同一个活动view。
            // 必须同时是同一view HWND和同一canonical IUnknown；不按父窗口或路径猜标签。
            let identity: IUnknown = provider(guard, || view.cast())?;
            if let Some((previous_hwnd, _, _, previous_identity)) = found.as_ref() {
                if *previous_hwnd != view_hwnd || previous_identity.as_raw() != identity.as_raw() {
                    return Err(ExplorerReadError::AmbiguousTarget);
                }
            } else {
                found = Some((view_hwnd, view, folder, identity));
            }
        }
        if provider(guard, || unsafe { windows.Count() })? != count {
            return Err(ExplorerReadError::SelectionChanged);
        }
        found.ok_or(ExplorerReadError::UnsupportedTarget)
    }
    struct Snapshot {
        // 持有canonical引用跨越两次快照，避免仅保存裸地址后的释放/地址复用。
        identity: IUnknown,
        view: usize,
        folder_id: Vec<u8>,
        value: ExplorerContextSnapshot,
    }
    impl Snapshot {
        fn agrees_with(&self, other: &Self) -> bool {
            self.identity.as_raw() == other.identity.as_raw()
                && self.view == other.view
                && self.folder_id == other.folder_id
                && self.value == other.value
        }
    }
    fn snapshot(
        windows: &IShellWindows,
        root: HWND,
        pid: u32,
        guard: &mut impl FnMut() -> Result<(), ExplorerReadError>,
    ) -> Result<Snapshot, ExplorerReadError> {
        let (view_hwnd, view, folder, identity) = bind(windows, root, pid, guard)?;
        let persist: IPersistFolder2 = provider(guard, || unsafe { folder.GetFolder() })?;
        let pidl = provider(guard, || unsafe { persist.GetCurFolder() })?;
        let _pidl_memory = TaskMem(pidl.cast());
        if pidl.is_null() {
            return Err(ExplorerReadError::InvalidResponse);
        }
        let size = unsafe { ILGetSize(Some(pidl)) } as usize;
        if !(2..=65536).contains(&size) {
            return Err(ExplorerReadError::InvalidResponse);
        }
        let folder_id = unsafe { std::slice::from_raw_parts(pidl.cast::<u8>(), size) }.to_vec();
        let current: IShellItem = provider(guard, || unsafe { SHCreateItemFromIDList(pidl) })?;
        let current_directory = path(&current, guard)?;
        let count = provider(guard, || unsafe { folder.ItemCount(SVGIO_SELECTION) })?;
        if count < 0 {
            return Err(ExplorerReadError::InvalidResponse);
        }
        if count > 32 {
            return Err(ExplorerReadError::TooLarge);
        }
        let mut selected_paths = Vec::new();
        if count > 0 {
            // FALSE明确禁止空选区隐式变成当前目录；成功的0数量单独返回[]。
            let items = provider(guard, || unsafe { folder.GetSelection(false) })?;
            if provider(guard, || unsafe { items.GetCount() })? != count as u32 {
                return Err(ExplorerReadError::SelectionChanged);
            }
            for index in 0..count as u32 {
                let item = provider(guard, || unsafe { items.GetItemAt(index) })?;
                selected_paths.push(path(&item, guard)?.ok_or(ExplorerReadError::UnsupportedItem)?);
            }
        }
        if provider(guard, || unsafe { folder.ItemCount(SVGIO_SELECTION) })? != count
            || visible_view(&view, root, pid, guard)? != Some(view_hwnd)
        {
            return Err(ExplorerReadError::SelectionChanged);
        }
        let value = ExplorerContextSnapshot {
            current_directory,
            selected_paths,
        };
        validate_explorer_context(&value)?;
        Ok(Snapshot {
            identity,
            view: view_hwnd.0 as usize,
            folder_id,
            value,
        })
    }
    impl ScriptTarget {
        /// 受控helper专用；调用可能阻塞，由外层Job的硬期限/取消收尾。
        /// 两次快照与身份检查可发现变化，不是窗口/选区生命周期的原子锁。
        pub fn read_explorer_context_if(
            &self,
            deadline: Instant,
        ) -> Result<ExplorerContextSnapshot, ExplorerReadError> {
            let mut guard = || {
                if Instant::now() >= deadline {
                    return Err(ExplorerReadError::Timeout);
                }
                self.ensure_foreground()
                    .map_err(|_| ExplorerReadError::TargetChanged)?;
                if Instant::now() >= deadline {
                    return Err(ExplorerReadError::Timeout);
                }
                Ok(())
            };
            guard()?;
            let target = self
                .snapshot()
                .map_err(|_| ExplorerReadError::TargetChanged)?;
            let process_name = crate::shortcut_target_process(crate::ShortcutTarget(
                target.window_handle as isize,
            ))
            .map_err(|_| ExplorerReadError::TargetChanged)?;
            if !process_name.eq_ignore_ascii_case("explorer.exe")
                && !process_name.eq_ignore_ascii_case("explorer")
            {
                return Err(ExplorerReadError::UnsupportedTarget);
            }
            let root = HWND(target.window_handle as *mut c_void);
            let mut class = [0u16; 128];
            let length = unsafe { GetClassNameW(root, &mut class) };
            let class = String::from_utf16(&class[..length.max(0) as usize])
                .map_err(|_| ExplorerReadError::InvalidResponse)?;
            if class != "CabinetWClass" && class != "ExploreWClass" {
                return Err(ExplorerReadError::UnsupportedTarget);
            }
            guard()?;
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
                .ok()
                .map_err(|_| ExplorerReadError::Unavailable)?;
            let _apartment = Apartment;
            let windows: IShellWindows = provider(&mut guard, || unsafe {
                CoCreateInstance(&ShellWindows, None, CLSCTX_ALL)
            })?;
            let first = snapshot(&windows, root, target.process_id, &mut guard)?;
            let second = snapshot(&windows, root, target.process_id, &mut guard)?;
            guard()?;
            if !first.agrees_with(&second) {
                return Err(ExplorerReadError::SelectionChanged);
            }
            Ok(second.value)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_validator_empty_virtual_directory_and_complete_paths() {
        let empty = ExplorerContextSnapshot {
            current_directory: None,
            selected_paths: vec![],
        };
        assert_eq!(validate_explorer_context(&empty), Ok(()));
        let paths = ExplorerContextSnapshot {
            current_directory: Some("D:\\fixture".into()),
            selected_paths: vec![
                "D:\\fixture\\A 😀.txt".into(),
                "\\\\server\\share\\B.txt".into(),
            ],
        };
        assert_eq!(validate_explorer_context(&paths), Ok(()));
        assert_eq!(paths.selected_paths[0], "D:\\fixture\\A 😀.txt");
    }
    #[test]
    fn real_validator_rejects_whole_oversized_or_invalid_result() {
        for paths in [
            vec!["D:\\x".into(); 33],
            vec![format!("D:\\{}", "x".repeat(1022))],
            (0..9)
                .map(|i| format!("D:\\{i}{}", "x".repeat(1000)))
                .collect(),
        ] {
            assert_eq!(
                validate_explorer_context(&ExplorerContextSnapshot {
                    current_directory: None,
                    selected_paths: paths
                }),
                Err(ExplorerReadError::TooLarge)
            );
        }
        for paths in [
            vec!["shell:::{virtual}".into()],
            vec!["D:\\A".into(), "d:\\a".into()],
            vec!["D:\\..\\x".into()],
            vec!["D:\\x\0".into()],
        ] {
            assert_eq!(
                validate_explorer_context(&ExplorerContextSnapshot {
                    current_directory: None,
                    selected_paths: paths
                }),
                Err(ExplorerReadError::InvalidResponse)
            );
        }
    }
}
