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

/// 跨进程传递的值快照；恢复时核验原身份，不重新选取窗口。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScriptTargetSnapshot {
    pub window_handle: usize,
    pub process_id: u32,
    pub thread_id: u32,
    pub process_created_file_time: u64,
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
    pub window_role: omnitrigger_core::WindowRole,
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

    pub fn snapshot(&self) -> Result<ScriptTargetSnapshot, String> {
        self.ensure_identity()?;
        Ok(ScriptTargetSnapshot {
            window_handle: self.target.0 as usize,
            process_id: self.identity.pid,
            thread_id: self.identity.thread_id,
            process_created_file_time: self.identity.process_created,
        })
    }

    pub fn from_snapshot(snapshot: ScriptTargetSnapshot) -> Result<Self, String> {
        if snapshot.window_handle == 0
            || snapshot.window_handle > isize::MAX as usize
            || snapshot.process_id == 0
            || snapshot.thread_id == 0
            || snapshot.process_created_file_time == 0
        {
            return Err("目标快照必须包含有效窗口、进程、线程和进程创建时间。".into());
        }
        let target = Self {
            target: ShortcutTarget(snapshot.window_handle as isize),
            identity: Identity {
                pid: snapshot.process_id,
                thread_id: snapshot.thread_id,
                process_created: snapshot.process_created_file_time,
            },
        };
        target.ensure_identity()?;
        Ok(target)
    }

    fn ensure_identity(&self) -> Result<(), String> {
        if identity(self.target)? != self.identity {
            return Err("目标窗口或进程身份已改变，已取消本机调用。".into());
        }
        Ok(())
    }

    pub fn info(&self) -> Result<ScriptTargetInfo, String> {
        self.ensure_identity()?;
        let process_name = shortcut_target_process(self.target)?;
        let window_role = crate::window_context::role(
            HWND(self.target.0 as *mut _),
            self.identity.pid,
            &process_name,
        );
        let info = ScriptTargetInfo {
            process_name,
            window_role,
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

    /// 仅当前台仍是调用方提供的界面窗口时，尝试交还原目标。
    /// 保留原进程身份核验；窗口检查与系统激活不是原子事务。
    pub fn restore_foreground_if(&self, current_window: isize) -> Result<bool, String> {
        self.ensure_identity()?;
        if current_window == 0 || !crate::is_foreground_window(current_window) {
            return Ok(false);
        }
        self.ensure_identity()?;
        let restored = crate::restore_shortcut_target(self.target)?;
        self.ensure_identity()?;
        Ok(restored)
    }

    /// 每次状态检查及提交前核验原窗口身份和取消条件，不选择新的前台。
    pub fn window_action_if(
        &self,
        operation: crate::WindowOperation,
        mut check: impl FnMut() -> Result<(), String>,
    ) -> Result<(), String> {
        crate::window_action::perform_window_action_checked(self.target, operation, || {
            self.ensure_foreground()?;
            check()
        })
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

    pub fn send_shortcut_timed_if(
        &self,
        keys: &[u16],
        delay_ms: u32,
        mut check: impl FnMut() -> Result<(), String>,
    ) -> Result<(), String> {
        crate::validate_shortcut(keys)?;
        crate::validate_key_timing(1, delay_ms)?;
        if delay_ms == 0 {
            return self.send_shortcut_if(keys, check);
        }
        self.send_key_sequence_if(
            &keys[..keys.len() - 1],
            &keys[keys.len() - 1..],
            delay_ms,
            &mut check,
        )
    }

    pub fn send_text_if(
        &self,
        text: &str,
        mut check: impl FnMut() -> Result<(), String>,
    ) -> Result<(), String> {
        crate::automation_basics::send_text_checked(text, self.target, || {
            self.ensure_foreground()?;
            check()
        })
    }

    pub fn send_key_sequence_if(
        &self,
        modifiers: &[u16],
        keys: &[u16],
        delay_ms: u32,
        mut check: impl FnMut() -> Result<(), String>,
    ) -> Result<(), String> {
        crate::shortcut::send_key_sequence_checked(modifiers, keys, delay_ms, self.target, || {
            self.ensure_foreground()?;
            check()
        })
    }

    pub fn copy_selection_text_if(
        &self,
        mut check: impl FnMut() -> Result<(), String>,
    ) -> Result<String, String> {
        crate::automation_basics::copy_selection_text_checked(self.target, || {
            self.ensure_foreground()?;
            check()
        })
    }

    /// 向原目标复制一次并读取文本或文件列表；不重新捕获前台，不恢复剪贴板。
    /// check的字符串错误统一映射operation_guard_failed，不根据消息猜测取消原因。
    pub fn copy_selection_data_if(
        &self,
        options: crate::selection_data::SelectionCopyOptions,
        mut check: impl FnMut() -> Result<(), String>,
    ) -> Result<
        crate::selection_data::SelectionDataSnapshot,
        crate::selection_data::SelectionDataError,
    > {
        use crate::selection_data::{copy_selection_data_checked, SelectionDataError};
        options.validate()?;
        copy_selection_data_checked(self.target, options, || {
            check().map_err(|_| {
                SelectionDataError::new(
                    "operation_guard_failed",
                    "复制选区的调用检查未通过。",
                    "not_started",
                )
            })?;
            self.ensure_foreground().map_err(|_| {
                SelectionDataError::new(
                    "target_changed",
                    "原目标窗口已失效或不再位于前台。",
                    "not_started",
                )
            })
        })
    }
}

use crate::selection_text::{selection_has_range, selection_range_has_text, selection_text};
use crate::{SelectionOverflow, SelectionReadError, SelectionTextSnapshot};

impl ScriptTarget {
    /// 只读取原前台目标中当前UIA焦点元素的单一TextPattern选区。
    /// 不做父节点pattern发现、整篇读取、模拟复制、剪贴板访问或焦点设置。
    pub fn read_selection_text_if(
        &self,
        deadline: std::time::Instant,
        overflow: SelectionOverflow,
        mut check: impl FnMut() -> Result<(), SelectionReadError>,
    ) -> Result<SelectionTextSnapshot, SelectionReadError> {
        use windows::{
            core::Result as WinResult,
            Win32::{
                System::Com::{
                    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
                    COINIT_MULTITHREADED,
                },
                UI::Accessibility::{
                    CUIAutomation8, IUIAutomation2, IUIAutomationElement, IUIAutomationTextPattern,
                    SupportedTextSelection_None, TextPatternRangeEndpoint_End,
                    TextPatternRangeEndpoint_Start, UIA_TextPatternId,
                },
            },
        };
        fn provider<T>(
            guard: &mut impl FnMut() -> Result<(), SelectionReadError>,
            operation: impl FnOnce() -> WinResult<T>,
        ) -> Result<T, SelectionReadError> {
            guard()?;
            let result = operation();
            guard()?; // provider阻塞返回后重新核验取消/期限/原窗口；不回显provider正文。
            result.map_err(|_| SelectionReadError::Unavailable)
        }
        let mut guard = || {
            check()?;
            self.ensure_foreground()
                .map_err(|_| SelectionReadError::TargetChanged)?;
            check() // 原身份核验也可能耗时；核验后复查取消/期限，再进入UIA。
        };
        let wait_ms = || {
            let left = deadline
                .saturating_duration_since(std::time::Instant::now())
                .as_millis();
            if left == 0 {
                return Err(SelectionReadError::Timeout);
            }
            Ok(left.min(150) as u32)
        };
        guard()?;
        let snapshot = self
            .snapshot()
            .map_err(|_| SelectionReadError::TargetChanged)?;
        // 已有动作工作线程必须能初始化MTA；STA冲突明确拒绝，不另开线程。
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
            .ok()
            .map_err(|_| SelectionReadError::Unavailable)?;
        struct Apartment;
        impl Drop for Apartment {
            fn drop(&mut self) {
                unsafe { CoUninitialize() };
            }
        }
        let _apartment = Apartment;
        let automation: IUIAutomation2 = provider(&mut guard, || unsafe {
            CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)
        })?;
        provider(&mut guard, || unsafe { automation.SetAutoSetFocus(false) })?;
        let wait = wait_ms()?;
        provider(&mut guard, || unsafe {
            automation.SetConnectionTimeout(wait)
        })?;
        let wait = wait_ms()?;
        provider(&mut guard, || unsafe {
            automation.SetTransactionTimeout(wait)
        })?;
        let root = provider(&mut guard, || unsafe {
            automation.ElementFromHandle(HWND(snapshot.window_handle as *mut std::ffi::c_void))
        })?;
        let focused = provider(&mut guard, || unsafe { automation.GetFocusedElement() })?;
        let walker = provider(&mut guard, || unsafe { automation.RawViewWalker() })?;
        let mut current: IUIAutomationElement = focused.clone();
        let mut owned = false;
        // 只走当前焦点到原窗口的祖先链验证归属，不枚举子树或读取节点文本属性。
        for depth in 0..=32 {
            if provider(&mut guard, || unsafe {
                automation.CompareElements(&root, &current)
            })?
            .as_bool()
            {
                owned = true;
                break;
            }
            if depth == 32 {
                break;
            }
            current = provider(&mut guard, || unsafe { walker.GetParentElement(&current) })?;
        }
        if !owned {
            return Err(SelectionReadError::TargetChanged);
        }
        if provider(&mut guard, || unsafe { focused.CurrentIsPassword() })?.as_bool() {
            return Err(SelectionReadError::Protected);
        }
        let pattern: IUIAutomationTextPattern = provider(&mut guard, || unsafe {
            focused.GetCurrentPatternAs(UIA_TextPatternId)
        })?;
        if provider(&mut guard, || unsafe { pattern.SupportedTextSelection() })?
            == SupportedTextSelection_None
        {
            return Err(SelectionReadError::Unavailable);
        }
        let ranges = provider(&mut guard, || unsafe { pattern.GetSelection() })?;
        if !selection_has_range(provider(&mut guard, || unsafe { ranges.Length() })?)? {
            // 成功确认无选区；再次核验焦点及零范围，再交付空成功值。没有文本读取。
            let after = provider(&mut guard, || unsafe { automation.GetFocusedElement() })?;
            if !provider(&mut guard, || unsafe {
                automation.CompareElements(&after, &focused)
            })?
            .as_bool()
            {
                return Err(SelectionReadError::SelectionChanged);
            }
            let latest = provider(&mut guard, || unsafe { pattern.GetSelection() })?;
            if provider(&mut guard, || unsafe { latest.Length() })? != 0 {
                return Err(SelectionReadError::SelectionChanged);
            }
            guard()?;
            return selection_text(&[], overflow);
        }
        let range = provider(&mut guard, || unsafe { ranges.GetElement(0) })?;
        let has_text = selection_range_has_text(provider(&mut guard, || unsafe {
            range.CompareEndpoints(
                TextPatternRangeEndpoint_Start,
                &range,
                TextPatternRangeEndpoint_End,
            )
        })?)?;
        let before = provider(&mut guard, || unsafe { automation.GetFocusedElement() })?;
        if !provider(&mut guard, || unsafe {
            automation.CompareElements(&before, &focused)
        })?
        .as_bool()
        {
            return Err(SelectionReadError::SelectionChanged);
        }
        let raw = if has_text {
            // 仅非退化范围请求129；GetText失败通过?返回错误，绝不转换为空文本。
            let wait = wait_ms()?;
            provider(&mut guard, || unsafe {
                automation.SetConnectionTimeout(wait)
            })?;
            let wait = wait_ms()?;
            provider(&mut guard, || unsafe {
                automation.SetTransactionTimeout(wait)
            })?;
            Some(provider(&mut guard, || unsafe { range.GetText(129) })?)
        } else {
            None
        };
        let after = provider(&mut guard, || unsafe { automation.GetFocusedElement() })?;
        if !provider(&mut guard, || unsafe {
            automation.CompareElements(&after, &focused)
        })?
        .as_bool()
        {
            return Err(SelectionReadError::SelectionChanged);
        }
        let latest = provider(&mut guard, || unsafe { pattern.GetSelection() })?;
        if provider(&mut guard, || unsafe { latest.Length() })? != 1 {
            return Err(SelectionReadError::SelectionChanged);
        }
        let latest = provider(&mut guard, || unsafe { latest.GetElement(0) })?;
        for endpoint in [TextPatternRangeEndpoint_Start, TextPatternRangeEndpoint_End] {
            if provider(&mut guard, || unsafe {
                range.CompareEndpoints(endpoint, &latest, endpoint)
            })? != 0
            {
                return Err(SelectionReadError::SelectionChanged);
            }
        }
        // windows-strings0.4.2 BSTR Deref<Target=[u16]>，长度来自SysStringLen。
        let result = selection_text(raw.as_deref().unwrap_or(&[]), overflow)?;
        guard()?;
        Ok(result)
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
        assert_eq!(info.window_role, omnitrigger_core::WindowRole::Application);
        assert!(target.ensure_foreground().is_err());
        assert!(!target.restore_foreground_if(0).unwrap());
        assert!(!target.restore_foreground_if(hwnd.0 as isize).unwrap());
        let checked = std::cell::Cell::new(false);
        assert!(target
            .window_action_if(crate::WindowOperation::Close, || {
                checked.set(true);
                Ok(())
            })
            .is_err());
        assert!(!checked.get());
        let mut stale = target;
        stale.identity.process_created ^= 1;
        assert!(stale.info().is_err());
        assert!(stale.restore_foreground_if(0).is_err());
        stale = target;
        stale.identity.thread_id ^= 1;
        assert!(stale.info().is_err());
        unsafe { DestroyWindow(hwnd) }.unwrap();
        assert!(target.info().is_err());
        assert!(target.send_shortcut(&[65]).is_err());
        assert!(target
            .window_action_if(crate::WindowOperation::Minimize, || Ok(()))
            .is_err());
    }
    #[test]
    fn explicit_snapshot_restores_only_the_supplied_identity_and_rejects_closed_target() {
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("Owned snapshot target"),
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
        struct Owned(HWND);
        impl Drop for Owned {
            fn drop(&mut self) {
                let _ = unsafe { DestroyWindow(self.0) };
            }
        }
        let owned = Owned(hwnd);
        let target = ScriptTarget::capture(ShortcutTarget(hwnd.0 as isize)).unwrap();
        let original = target.snapshot().unwrap();
        assert_eq!(
            ScriptTarget::from_snapshot(original)
                .unwrap()
                .snapshot()
                .unwrap(),
            original
        );
        for changed in [
            ScriptTargetSnapshot {
                process_id: original.process_id + 1,
                ..original
            },
            ScriptTargetSnapshot {
                thread_id: original.thread_id + 1,
                ..original
            },
            ScriptTargetSnapshot {
                process_created_file_time: original.process_created_file_time + 1,
                ..original
            },
            ScriptTargetSnapshot {
                window_handle: 0,
                ..original
            },
        ] {
            assert!(ScriptTarget::from_snapshot(changed).is_err());
        }
        drop(owned);
        assert!(ScriptTarget::from_snapshot(original).is_err());
    }
}
