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
    UnavailableAfterDirectory,
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
            Self::UnavailableAfterDirectory => "selection_unavailable_after_directory",
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

// 仅显式阶段读取使用此编排；默认快照及其错误顺序保持原实现。
#[cfg(any(windows, test))]
mod failure_stage {
    use super::{validate_explorer_context, ExplorerContextSnapshot, ExplorerReadError};

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(super) struct DirectoryProof {
        pub root: usize,
        pub pid: u32,
        pub identity: usize,
        pub view: usize,
        pub folder_id: Vec<u8>,
        pub current_directory: Option<String>,
    }

    // 仅 opt-in 主选区链保留 HRESULT，不能把控制失败当成普通 provider 不可用。
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum ProviderClass {
        OrdinaryUnavailable,
        Aborted,
        TimedOut,
        ControlOrUnknown,
    }
    fn classify_provider(code: i32) -> ProviderClass {
        match code as u32 {
            0x80004005 => ProviderClass::OrdinaryUnavailable, // E_FAIL；唯一允许升级的普通失败
            0x80004004 | 0x80010002 => ProviderClass::Aborted, // E_ABORT / RPC_E_CALL_CANCELED
            0x8001011f => ProviderClass::TimedOut,            // RPC_E_TIMEOUT
            _ => ProviderClass::ControlOrUnknown,
        }
    }
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum PrimarySelectionError {
        Read(ExplorerReadError),
        Hresult(i32),
    }
    impl From<ExplorerReadError> for PrimarySelectionError {
        fn from(error: ExplorerReadError) -> Self {
            Self::Read(error)
        }
    }
    impl PrimarySelectionError {
        fn ordinary_unavailable(self) -> bool {
            matches!(self, Self::Hresult(code) if classify_provider(code) == ProviderClass::OrdinaryUnavailable)
        }
        fn into_read_error(self) -> ExplorerReadError {
            match self {
                Self::Read(error) => error,
                Self::Hresult(code) if classify_provider(code) == ProviderClass::TimedOut => {
                    ExplorerReadError::Timeout
                }
                // 原错误协议没有独立 COM abort/control 码；保留失败，不授予阶段证明。
                Self::Hresult(_) => ExplorerReadError::Unavailable,
            }
        }
    }

    // 文件内测试缝；真实 Directory 持有 COM 引用，不能仅保留裸地址。
    pub(super) trait Reader {
        type Directory;
        fn directory(&mut self) -> Result<Self::Directory, ExplorerReadError>;
        fn proof(directory: &Self::Directory) -> DirectoryProof;
        fn selection(
            &mut self,
            directory: &Self::Directory,
        ) -> Result<Vec<String>, PrimarySelectionError>;
        fn verify(
            &mut self,
            directory: &Self::Directory,
            selected: &[String],
        ) -> Result<(), ExplorerReadError>;
        fn guard(&mut self) -> Result<(), ExplorerReadError>;
    }

    fn directory<R: Reader>(reader: &mut R) -> Result<R::Directory, ExplorerReadError> {
        let value = reader.directory()?;
        validate_explorer_context(&ExplorerContextSnapshot {
            current_directory: R::proof(&value).current_directory,
            selected_paths: vec![],
        })?;
        Ok(value)
    }

    fn selection<R: Reader>(
        reader: &mut R,
        value: &R::Directory,
    ) -> Result<Vec<String>, ExplorerReadError> {
        match reader.selection(value) {
            Err(error) if error.ordinary_unavailable() => {
                let proof = R::proof(value);
                if proof
                    .current_directory
                    .as_ref()
                    .is_none_or(|path| path.is_empty())
                {
                    return Err(ExplorerReadError::Unavailable);
                }
                // 原 Directory 仍存活；复读只验证目录/身份，绝不重试选区。
                let confirmation = directory(reader)?;
                if proof != R::proof(&confirmation) {
                    return Err(ExplorerReadError::SelectionChanged);
                }
                reader.guard()?;
                Err(ExplorerReadError::UnavailableAfterDirectory)
            }
            Err(error) => Err(error.into_read_error()),
            Ok(paths) => Ok(paths),
        }
    }

    pub(super) fn read<R: Reader>(
        reader: &mut R,
    ) -> Result<ExplorerContextSnapshot, ExplorerReadError> {
        let first = directory(reader)?;
        let first_paths = selection(reader, &first)?;
        reader.verify(&first, &first_paths)?;
        let first_value = ExplorerContextSnapshot {
            current_directory: R::proof(&first).current_directory,
            selected_paths: first_paths,
        };
        validate_explorer_context(&first_value)?;
        let second = directory(reader)?;
        // 已知目录或标签变化不能被第二次选区失败掩盖。
        if R::proof(&first) != R::proof(&second) {
            return Err(ExplorerReadError::SelectionChanged);
        }
        let second_paths = selection(reader, &second)?;
        reader.verify(&second, &second_paths)?;
        let second_value = ExplorerContextSnapshot {
            current_directory: R::proof(&second).current_directory,
            selected_paths: second_paths,
        };
        validate_explorer_context(&second_value)?;
        reader.guard()?;
        if first_value != second_value {
            return Err(ExplorerReadError::SelectionChanged);
        }
        Ok(second_value)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::collections::VecDeque;

        struct Fake {
            directories: VecDeque<Result<DirectoryProof, ExplorerReadError>>,
            selections: VecDeque<Result<Vec<String>, PrimarySelectionError>>,
            verification: VecDeque<Result<(), ExplorerReadError>>,
            final_guard: Result<(), ExplorerReadError>,
            calls: Vec<&'static str>,
        }
        impl Reader for Fake {
            type Directory = DirectoryProof;
            fn directory(&mut self) -> Result<DirectoryProof, ExplorerReadError> {
                self.calls.push("directory");
                self.directories.pop_front().expect("未预期的目录复读")
            }
            fn proof(value: &DirectoryProof) -> DirectoryProof {
                value.clone()
            }
            fn selection(
                &mut self,
                _: &DirectoryProof,
            ) -> Result<Vec<String>, PrimarySelectionError> {
                self.calls.push("selection");
                self.selections.pop_front().expect("未预期的选区重试")
            }
            fn verify(
                &mut self,
                _: &DirectoryProof,
                _: &[String],
            ) -> Result<(), ExplorerReadError> {
                self.calls.push("verify");
                self.verification.pop_front().expect("未预期的选区复核")
            }
            fn guard(&mut self) -> Result<(), ExplorerReadError> {
                self.calls.push("guard");
                self.final_guard
            }
        }
        fn proof() -> DirectoryProof {
            DirectoryProof {
                root: 101,
                pid: 10,
                identity: 200,
                view: 201,
                folder_id: vec![2, 0],
                current_directory: Some("D:\\fixture".into()),
            }
        }
        fn failed_selection() -> Fake {
            Fake {
                directories: [Ok(proof()), Ok(proof())].into(),
                selections: [Err(PrimarySelectionError::Hresult(0x80004005_u32 as i32))].into(),
                verification: [].into(),
                final_guard: Ok(()),
                calls: vec![],
            }
        }
        #[test]
        fn primary_hresult_abort_timeout_and_control_failures_never_gain_directory_proof() {
            // 目录与后备复读均可成功，末尾 guard 也为成功；HRESULT 本身必须否决升级。
            for (raw, class, expected) in [
                (
                    0x80004004_u32,
                    ProviderClass::Aborted,
                    ExplorerReadError::Unavailable,
                ), // E_ABORT
                (
                    0x80010002,
                    ProviderClass::Aborted,
                    ExplorerReadError::Unavailable,
                ), // RPC_E_CALL_CANCELED
                (
                    0x8001011f,
                    ProviderClass::TimedOut,
                    ExplorerReadError::Timeout,
                ), // RPC_E_TIMEOUT
                (
                    0x80010001,
                    ProviderClass::ControlOrUnknown,
                    ExplorerReadError::Unavailable,
                ), // RPC_E_CALL_REJECTED
                (
                    0x80010108,
                    ProviderClass::ControlOrUnknown,
                    ExplorerReadError::Unavailable,
                ), // RPC_E_DISCONNECTED
                (
                    0x8001010e,
                    ProviderClass::ControlOrUnknown,
                    ExplorerReadError::Unavailable,
                ), // RPC_E_WRONG_THREAD
                (
                    0x8001010a,
                    ProviderClass::ControlOrUnknown,
                    ExplorerReadError::Unavailable,
                ), // RPC_E_SERVERCALL_RETRYLATER
                (
                    0x80070005,
                    ProviderClass::ControlOrUnknown,
                    ExplorerReadError::Unavailable,
                ), // E_ACCESSDENIED
                (
                    0x8000ffff,
                    ProviderClass::ControlOrUnknown,
                    ExplorerReadError::Unavailable,
                ), // E_UNEXPECTED
                (
                    0x81234567,
                    ProviderClass::ControlOrUnknown,
                    ExplorerReadError::Unavailable,
                ), // 未识别，失败关闭
            ] {
                let mut value = failed_selection();
                value.selections = [Err(PrimarySelectionError::Hresult(raw as i32))].into();
                assert_eq!(classify_provider(raw as i32), class);
                assert_eq!(read(&mut value), Err(expected));
                assert_eq!(value.calls, ["directory", "selection"]);
                assert_eq!(value.directories.len(), 1);
            }
            // 已折叠的 Unavailable 也不能冒充明确的 E_FAIL。
            let mut folded = failed_selection();
            folded.selections = [Err(PrimarySelectionError::Read(
                ExplorerReadError::Unavailable,
            ))]
            .into();
            assert_eq!(read(&mut folded), Err(ExplorerReadError::Unavailable));
            assert_eq!(folded.calls, ["directory", "selection"]);
            assert_eq!(
                classify_provider(0x80004005_u32 as i32),
                ProviderClass::OrdinaryUnavailable
            );
        }

        #[test]
        fn directory_failure_and_primary_selection_failure_are_distinct() {
            let mut missing = failed_selection();
            missing.directories = [Err(ExplorerReadError::Unavailable)].into();
            assert_eq!(read(&mut missing), Err(ExplorerReadError::Unavailable));
            assert_eq!(missing.calls, ["directory"]);
            let mut selected = failed_selection();
            assert_eq!(
                read(&mut selected),
                Err(ExplorerReadError::UnavailableAfterDirectory)
            );
            assert_eq!(
                selected.calls,
                ["directory", "selection", "directory", "guard"]
            );
            assert!(selected.selections.is_empty());
            for path in [None, Some(String::new()), Some("shell:::virtual".into())] {
                let mut value = failed_selection();
                let mut invalid = proof();
                invalid.current_directory = path.clone();
                value.directories = [Ok(invalid)].into();
                let expected = if path.is_none() {
                    ExplorerReadError::Unavailable
                } else {
                    ExplorerReadError::InvalidResponse
                };
                assert_eq!(read(&mut value), Err(expected));
                assert!(!value.calls.contains(&"guard"));
            }
        }
        #[test]
        fn directory_recheck_and_terminal_guard_can_only_veto() {
            let mut changes = vec![proof(); 6];
            changes[0].root += 1;
            changes[1].pid += 1;
            changes[2].identity += 1;
            changes[3].view += 1;
            changes[4].folder_id.push(0);
            changes[5].current_directory = Some("D:\\other".into());
            for changed in changes {
                let mut value = failed_selection();
                value.directories = [Ok(proof()), Ok(changed)].into();
                assert_eq!(read(&mut value), Err(ExplorerReadError::SelectionChanged));
                assert!(!value.calls.contains(&"guard"));
            }
            for error in [
                ExplorerReadError::Unavailable,
                ExplorerReadError::Timeout,
                ExplorerReadError::TargetChanged,
                ExplorerReadError::UnsupportedTarget,
                ExplorerReadError::AmbiguousTarget,
                ExplorerReadError::SelectionChanged,
                ExplorerReadError::InvalidResponse,
                ExplorerReadError::UnsupportedItem,
                ExplorerReadError::TooLarge,
            ] {
                let mut recheck = failed_selection();
                recheck.directories = [Ok(proof()), Err(error)].into();
                assert_eq!(read(&mut recheck), Err(error));
                let mut primary = failed_selection();
                if error != ExplorerReadError::Unavailable {
                    primary.selections = [Err(error.into())].into();
                    assert_eq!(read(&mut primary), Err(error));
                    assert_eq!(primary.calls, ["directory", "selection"]);
                }
            }
            for error in [ExplorerReadError::Timeout, ExplorerReadError::TargetChanged] {
                let mut value = failed_selection();
                value.final_guard = Err(error);
                assert_eq!(read(&mut value), Err(error));
            }
        }
        #[test]
        fn verification_and_second_directory_failures_never_become_primary_failure() {
            let mut verification = failed_selection();
            verification.selections = [Ok(vec![])].into();
            verification.verification = [Err(ExplorerReadError::Unavailable)].into();
            assert_eq!(read(&mut verification), Err(ExplorerReadError::Unavailable));
            assert_eq!(verification.calls, ["directory", "selection", "verify"]);
            let mut second = failed_selection();
            second.selections = [Ok(vec![])].into();
            second.verification = [Ok(())].into();
            second.directories = [Ok(proof()), Err(ExplorerReadError::Unavailable)].into();
            assert_eq!(read(&mut second), Err(ExplorerReadError::Unavailable));
            let mut changed = failed_selection();
            changed.selections = [Ok(vec![])].into();
            changed.verification = [Ok(())].into();
            let mut other = proof();
            other.identity += 1;
            changed.directories = [Ok(proof()), Ok(other)].into();
            assert_eq!(read(&mut changed), Err(ExplorerReadError::SelectionChanged));
            assert_eq!(
                changed.calls,
                ["directory", "selection", "verify", "directory"]
            );
        }
        #[test]
        fn second_primary_failure_rechecks_directory_without_retrying_selection() {
            let mut value = failed_selection();
            value.directories = [Ok(proof()), Ok(proof()), Ok(proof())].into();
            value.selections = [
                Ok(vec![]),
                Err(PrimarySelectionError::Hresult(0x80004005_u32 as i32)),
            ]
            .into();
            value.verification = [Ok(())].into();
            assert_eq!(
                read(&mut value),
                Err(ExplorerReadError::UnavailableAfterDirectory)
            );
            assert_eq!(
                value.calls,
                [
                    "directory",
                    "selection",
                    "verify",
                    "directory",
                    "selection",
                    "directory",
                    "guard"
                ]
            );
        }
        #[test]
        fn successful_pair_keeps_snapshot_shape_and_rejects_changed_selection() {
            for changed in [false, true] {
                let mut value = failed_selection();
                value.selections = [
                    Ok(vec![]),
                    Ok(if changed {
                        vec!["D:\\fixture\\a".into()]
                    } else {
                        vec![]
                    }),
                ]
                .into();
                value.verification = [Ok(()), Ok(())].into();
                let expected = if changed {
                    Err(ExplorerReadError::SelectionChanged)
                } else {
                    Ok(ExplorerContextSnapshot {
                        current_directory: proof().current_directory,
                        selected_paths: vec![],
                    })
                };
                assert_eq!(read(&mut value), expected);
                assert_eq!(
                    value.calls,
                    [
                        "directory",
                        "selection",
                        "verify",
                        "directory",
                        "selection",
                        "verify",
                        "guard"
                    ]
                );
            }
        }
    }
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
    use super::failure_stage::PrimarySelectionError;
    fn primary_selection_provider<T>(
        guard: &mut impl FnMut() -> Result<(), ExplorerReadError>,
        call: impl FnOnce() -> windows::core::Result<T>,
    ) -> Result<T, PrimarySelectionError> {
        guard()?;
        let value = call();
        guard()?;
        value.map_err(|error| PrimarySelectionError::Hresult(error.code().0))
    }
    fn primary_selection_path(
        item: &IShellItem,
        guard: &mut impl FnMut() -> Result<(), ExplorerReadError>,
    ) -> Result<Option<String>, PrimarySelectionError> {
        let attrs =
            primary_selection_provider(guard, || unsafe { item.GetAttributes(SFGAO_FILESYSTEM) })?;
        if attrs.0 & SFGAO_FILESYSTEM.0 == 0 {
            return Ok(None);
        }
        let raw: PWSTR = primary_selection_provider(guard, || unsafe {
            item.GetDisplayName(SIGDN_FILESYSPATH)
        })?;
        let _memory = TaskMem(raw.0.cast());
        if raw.is_null() {
            return Err(ExplorerReadError::InvalidResponse.into());
        }
        let mut units = Vec::new();
        for offset in 0..=1024 {
            let unit = unsafe { *raw.0.add(offset) };
            if unit == 0 {
                return String::from_utf16(&units)
                    .map(Some)
                    .map_err(|_| ExplorerReadError::InvalidResponse.into());
            }
            units.push(unit);
        }
        Err(ExplorerReadError::TooLarge.into())
    }

    // 仅 opt-in 使用；默认 snapshot 与 read_explorer_context_if 不经过此路径。
    pub(super) struct StagedDirectory {
        identity: IUnknown,
        view_hwnd: HWND,
        view: IShellView,
        folder: IFolderView2,
        proof: super::failure_stage::DirectoryProof,
    }
    struct StagedReader<'a, G> {
        windows: &'a IShellWindows,
        root: HWND,
        pid: u32,
        guard: &'a mut G,
    }
    impl<G: FnMut() -> Result<(), ExplorerReadError>> super::failure_stage::Reader
        for StagedReader<'_, G>
    {
        type Directory = StagedDirectory;
        fn directory(&mut self) -> Result<StagedDirectory, ExplorerReadError> {
            let (view_hwnd, view, folder, identity) =
                bind(self.windows, self.root, self.pid, self.guard)?;
            let persist: IPersistFolder2 = provider(self.guard, || unsafe { folder.GetFolder() })?;
            let pidl = provider(self.guard, || unsafe { persist.GetCurFolder() })?;
            let _pidl_memory = TaskMem(pidl.cast());
            if pidl.is_null() {
                return Err(ExplorerReadError::InvalidResponse);
            }
            let size = unsafe { ILGetSize(Some(pidl)) } as usize;
            if !(2..=65536).contains(&size) {
                return Err(ExplorerReadError::InvalidResponse);
            }
            let folder_id = unsafe { std::slice::from_raw_parts(pidl.cast::<u8>(), size) }.to_vec();
            let current: IShellItem =
                provider(self.guard, || unsafe { SHCreateItemFromIDList(pidl) })?;
            let current_directory = path(&current, self.guard)?;
            if visible_view(&view, self.root, self.pid, self.guard)? != Some(view_hwnd) {
                return Err(ExplorerReadError::SelectionChanged);
            }
            let proof = super::failure_stage::DirectoryProof {
                root: self.root.0 as usize,
                pid: self.pid,
                identity: identity.as_raw() as usize,
                view: view_hwnd.0 as usize,
                folder_id,
                current_directory,
            };
            Ok(StagedDirectory {
                identity,
                view_hwnd,
                view,
                folder,
                proof,
            })
        }
        fn proof(directory: &StagedDirectory) -> super::failure_stage::DirectoryProof {
            // 持有 canonical 引用直到所有比较完成，禁止裸地址释放后复用。
            let mut proof = directory.proof.clone();
            proof.identity = directory.identity.as_raw() as usize;
            proof
        }
        fn selection(
            &mut self,
            directory: &StagedDirectory,
        ) -> Result<Vec<String>, PrimarySelectionError> {
            let count = primary_selection_provider(self.guard, || unsafe {
                directory.folder.ItemCount(SVGIO_SELECTION)
            })?;
            if count < 0 {
                return Err(ExplorerReadError::InvalidResponse.into());
            }
            if count > 32 {
                return Err(ExplorerReadError::TooLarge.into());
            }
            let mut selected_paths = Vec::new();
            if count > 0 {
                let items = primary_selection_provider(self.guard, || unsafe {
                    directory.folder.GetSelection(false)
                })?;
                if primary_selection_provider(self.guard, || unsafe { items.GetCount() })?
                    != count as u32
                {
                    return Err(ExplorerReadError::SelectionChanged.into());
                }
                for index in 0..count as u32 {
                    let item = primary_selection_provider(self.guard, || unsafe {
                        items.GetItemAt(index)
                    })?;
                    selected_paths.push(
                        primary_selection_path(&item, self.guard)?
                            .ok_or(ExplorerReadError::UnsupportedItem)?,
                    );
                    // 已知坏路径/重复/超限必须先失败，不能被后续 provider 故障盖过。
                    validate_explorer_context(&ExplorerContextSnapshot {
                        current_directory: directory.proof.current_directory.clone(),
                        selected_paths: selected_paths.clone(),
                    })?;
                }
            }
            Ok(selected_paths)
        }
        fn verify(
            &mut self,
            directory: &StagedDirectory,
            selected: &[String],
        ) -> Result<(), ExplorerReadError> {
            if provider(self.guard, || unsafe {
                directory.folder.ItemCount(SVGIO_SELECTION)
            })? != selected.len() as i32
                || visible_view(&directory.view, self.root, self.pid, self.guard)?
                    != Some(directory.view_hwnd)
            {
                return Err(ExplorerReadError::SelectionChanged);
            }
            Ok(())
        }
        fn guard(&mut self) -> Result<(), ExplorerReadError> {
            (self.guard)()
        }
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

        /// 显式阶段证据读取；普通调用继续使用原入口，不改变其错误顺序。
        pub fn read_explorer_context_with_selection_failure_stage_if(
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
            super::failure_stage::read(&mut StagedReader {
                windows: &windows,
                root,
                pid: target.process_id,
                guard: &mut guard,
            })
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
