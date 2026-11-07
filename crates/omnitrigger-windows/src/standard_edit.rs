//! 显式标准Edit选区替换候选：不发送按键，不借剪贴板，不自动切换适配器。
//! 消息处理成功不证明控件接受了全部正文，也不与此前UIA读取构成原子操作。
use std::time::{Duration, Instant};

const MAX_UNITS: usize = 8192;
const MAX_MESSAGE_WAIT: Duration = Duration::from_millis(1000);

/// 调用方必须明确选择。WinForms仅接受系统Edit派生的已知窗口类命名。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StandardEditAdapter {
    Win32Edit,
    WinFormsEdit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StandardEditError {
    pub code: &'static str,
    pub effect_state: &'static str,
}

impl StandardEditError {
    pub fn not_started(code: &'static str) -> Self {
        Self {
            code,
            effect_state: "not_started",
        }
    }

    fn unknown(mut self) -> Self {
        self.effect_state = "unknown";
        self
    }
}

/// 只表示消息已返回；EM_REPLACESEL没有正文接受数量或业务完成回执。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StandardEditReceipt {
    pub message_processed: bool,
    pub effect_state: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RootIdentity {
    hwnd: usize,
    pid: u32,
    thread: u32,
    birth: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Observation {
    root: RootIdentity,
    child: usize,
    child_pid: u32,
    child_thread: u32,
    foreground: usize,
    is_descendant: bool,
    unicode: bool,
    visible: bool,
    read_only_or_password_or_disabled: bool,
    same_process: bool,
    class_name: String,
    real_class: String,
}

fn validate_observation(
    root: RootIdentity,
    adapter: StandardEditAdapter,
    view: &Observation,
) -> Result<(), StandardEditError> {
    if view.root != root
        || view.foreground != root.hwnd
        || view.child == 0
        || view.child_pid != root.pid
        || view.child_thread != root.thread
        || !view.is_descendant
    {
        return Err(StandardEditError::not_started("target_changed"));
    }
    // 同进程超时不能依赖跨进程的系统消息封送管理指针生命周期，首版明确拒绝。
    if view.same_process || !view.unicode || !view.visible || view.read_only_or_password_or_disabled
    {
        return Err(StandardEditError::not_started("edit_unsupported"));
    }
    let registered = match adapter {
        StandardEditAdapter::Win32Edit => view.class_name.eq_ignore_ascii_case("Edit"),
        StandardEditAdapter::WinFormsEdit => view
            .class_name
            .get(.."WindowsForms10.EDIT.".len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("WindowsForms10.EDIT.")),
    };
    if !registered || !view.real_class.eq_ignore_ascii_case("Edit") {
        return Err(StandardEditError::not_started("edit_unsupported"));
    }
    Ok(())
}

// 包含child到原root；断链或128层内无法到达原root时保守拒绝。
fn enabled_ancestry_with(
    mut child: usize,
    root: usize,
    mut inspect: impl FnMut(usize) -> Option<(usize, bool)>,
) -> bool {
    for _ in 0..128 {
        let Some((parent, enabled)) = inspect(child) else {
            return false;
        };
        if !enabled {
            return false;
        }
        if child == root {
            return true;
        }
        child = parent;
    }
    false
}

trait Backend {
    fn inspect(&mut self) -> Result<Observation, StandardEditError>;
    /// 恰好一次写消息。失败可能发生在目标已处理消息后，调用方统一标unknown。
    fn replace_once(
        &mut self,
        child: usize,
        units: &[u16],
        timeout_ms: u32,
    ) -> Result<(), StandardEditError>;
}

fn guard(
    deadline: Instant,
    now: &mut impl FnMut() -> Instant,
    check: &mut impl FnMut() -> Result<(), StandardEditError>,
) -> Result<(), StandardEditError> {
    check()?;
    if now() >= deadline {
        return Err(StandardEditError::not_started("timeout"));
    }
    Ok(())
}

fn execute(
    root: RootIdentity,
    adapter: StandardEditAdapter,
    text: &str,
    deadline: Instant,
    mut now: impl FnMut() -> Instant,
    mut check: impl FnMut() -> Result<(), StandardEditError>,
    backend: &mut impl Backend,
) -> Result<StandardEditReceipt, StandardEditError> {
    if text.contains('\0')
        || text.len() > MAX_UNITS * 4
        || text.encode_utf16().take(MAX_UNITS + 1).count() > MAX_UNITS
    {
        return Err(StandardEditError::not_started("invalid_input"));
    }
    // 允许空字符串删除当前选区；若选区为空，Win32按其原生语义在插入点操作。
    let units: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    guard(deadline, &mut now, &mut check)?;
    let original = backend.inspect()?;
    validate_observation(root, adapter, &original)?;
    guard(deadline, &mut now, &mut check)?;
    let current = backend.inspect()?;
    validate_observation(root, adapter, &current)?;
    if current != original {
        return Err(StandardEditError::not_started("target_changed"));
    }
    guard(deadline, &mut now, &mut check)?;
    let remaining = deadline.saturating_duration_since(now());
    if remaining.is_zero() {
        return Err(StandardEditError::not_started("timeout"));
    }
    let budget = remaining.min(MAX_MESSAGE_WAIT).as_millis().max(1) as u32;
    backend
        .replace_once(original.child, &units, budget)
        .map_err(StandardEditError::unknown)?;
    guard(deadline, &mut now, &mut check).map_err(StandardEditError::unknown)?;
    let after = backend.inspect().map_err(StandardEditError::unknown)?;
    validate_observation(root, adapter, &after).map_err(StandardEditError::unknown)?;
    if after != original {
        return Err(StandardEditError::not_started("target_changed").unknown());
    }
    guard(deadline, &mut now, &mut check).map_err(StandardEditError::unknown)?;
    Ok(StandardEditReceipt {
        message_processed: true,
        effect_state: "unknown",
    })
}

#[cfg(windows)]
mod native {
    use super::*;
    use crate::ScriptTarget;
    use std::mem::size_of;
    use windows::Win32::{
        Foundation::{GetLastError, SetLastError, HWND, LPARAM, WIN32_ERROR, WPARAM},
        System::Threading::GetCurrentProcessId,
        UI::WindowsAndMessaging::{
            GetClassNameW, GetForegroundWindow, GetGUIThreadInfo, GetWindowLongPtrW,
            GetWindowThreadProcessId, IsChild, IsWindow, IsWindowUnicode, IsWindowVisible,
            RealGetWindowClassW, SendMessageTimeoutW, ES_PASSWORD, ES_READONLY, GUITHREADINFO,
            GWL_STYLE, SMTO_ABORTIFHUNG, SMTO_BLOCK, SMTO_ERRORONEXIT,
        },
    };

    use windows::Win32::UI::{
        Input::KeyboardAndMouse::IsWindowEnabled,
        WindowsAndMessaging::{GetAncestor, GA_PARENT},
    };

    const EM_REPLACESEL: u32 = 0x00c2; // 系统消息，低于WM_USER，跨进程由系统封送字符串。
    const WS_DISABLED_BIT: isize = 0x0800_0000;

    struct Native<'a> {
        target: &'a ScriptTarget,
        root: RootIdentity,
    }

    impl Backend for Native<'_> {
        fn inspect(&mut self) -> Result<Observation, StandardEditError> {
            self.target
                .ensure_foreground()
                .map_err(|_| StandardEditError::not_started("target_changed"))?;
            let saved = self
                .target
                .snapshot()
                .map_err(|_| StandardEditError::not_started("target_changed"))?;
            let root = RootIdentity {
                hwnd: saved.window_handle,
                pid: saved.process_id,
                thread: saved.thread_id,
                birth: saved.process_created_file_time,
            };
            if root != self.root {
                return Err(StandardEditError::not_started("target_changed"));
            }
            let mut gui = GUITHREADINFO {
                cbSize: size_of::<GUITHREADINFO>() as u32,
                ..Default::default()
            };
            unsafe { GetGUIThreadInfo(root.thread, &mut gui) }
                .map_err(|_| StandardEditError::not_started("target_changed"))?;
            let child = gui.hwndFocus;
            if child.0.is_null() || !unsafe { IsWindow(Some(child)) }.as_bool() {
                return Err(StandardEditError::not_started("target_changed"));
            }
            let mut pid = 0;
            let thread = unsafe { GetWindowThreadProcessId(child, Some(&mut pid)) };
            let mut registered = [0_u16; 256];
            let mut real = [0_u16; 256];
            let n = unsafe { GetClassNameW(child, &mut registered) };
            let m = unsafe { RealGetWindowClassW(child, &mut real) };
            if n <= 0
                || n as usize >= registered.len() - 1
                || m == 0
                || m as usize >= real.len() - 1
            {
                return Err(StandardEditError::not_started("edit_unsupported"));
            }
            unsafe { SetLastError(WIN32_ERROR(0)) };
            let style = unsafe { GetWindowLongPtrW(child, GWL_STYLE) };
            if style == 0 && unsafe { GetLastError() }.0 != 0 {
                return Err(StandardEditError::not_started("edit_unsupported"));
            }
            Ok(Observation {
                root,
                child: child.0 as usize,
                child_pid: pid,
                child_thread: thread,
                foreground: unsafe { GetForegroundWindow() }.0 as usize,
                is_descendant: unsafe { IsChild(HWND(root.hwnd as *mut _), child) }.as_bool(),
                unicode: unsafe { IsWindowUnicode(child) }.as_bool(),
                visible: unsafe { IsWindowVisible(child) }.as_bool(),
                read_only_or_password_or_disabled: style
                    & (ES_READONLY as isize | ES_PASSWORD as isize | WS_DISABLED_BIT)
                    != 0
                    || !enabled_ancestry_with(child.0 as usize, root.hwnd, |value| {
                        let hwnd = HWND(value as *mut _);
                        if hwnd.0.is_null() || !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
                            return None;
                        }
                        Some((
                            unsafe { GetAncestor(hwnd, GA_PARENT) }.0 as usize,
                            unsafe { IsWindowEnabled(hwnd) }.as_bool(),
                        ))
                    }),
                same_process: pid == unsafe { GetCurrentProcessId() },
                class_name: String::from_utf16_lossy(&registered[..n as usize]),
                real_class: String::from_utf16_lossy(&real[..m as usize]),
            })
        }

        fn replace_once(
            &mut self,
            child: usize,
            units: &[u16],
            timeout_ms: u32,
        ) -> Result<(), StandardEditError> {
            let mut ignored = 0;
            unsafe { SetLastError(WIN32_ERROR(0)) };
            // 仅跨进程系统Edit；指针属于当前units，存活到同步调用返回。
            // 不能泛化到同进程超时或>=WM_USER的自定义指针消息。
            let processed = unsafe {
                SendMessageTimeoutW(
                    HWND(child as *mut _),
                    EM_REPLACESEL,
                    WPARAM(1),
                    LPARAM(units.as_ptr() as isize),
                    SMTO_ABORTIFHUNG | SMTO_BLOCK | SMTO_ERRORONEXIT,
                    timeout_ms,
                    Some(&mut ignored),
                )
            };
            if processed.0 != 0 {
                return Ok(());
            }
            let code = match unsafe { GetLastError() }.0 {
                1460 => "timeout",
                5 => "permission_denied",
                _ => "edit_message_unconfirmed",
            };
            Err(StandardEditError::not_started(code).unknown())
        }
    }

    /// deadline沿用动作绝对期限；单条写消息另外最多请求1000ms，不抢占已进入的系统调用。
    pub fn replace(
        target: &ScriptTarget,
        adapter: StandardEditAdapter,
        text: &str,
        deadline: Instant,
        mut check: impl FnMut() -> Result<(), StandardEditError>,
    ) -> Result<StandardEditReceipt, StandardEditError> {
        guard(deadline, &mut Instant::now, &mut check)?;
        let saved = target
            .snapshot()
            .map_err(|_| StandardEditError::not_started("target_changed"))?;
        let root = RootIdentity {
            hwnd: saved.window_handle,
            pid: saved.process_id,
            thread: saved.thread_id,
            birth: saved.process_created_file_time,
        };
        execute(
            root,
            adapter,
            text,
            deadline,
            Instant::now,
            check,
            &mut Native { target, root },
        )
    }
}

#[cfg(windows)]
pub use native::replace as replace_focused_standard_edit_selection;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    fn root() -> RootIdentity {
        RootIdentity {
            hwnd: 10,
            pid: 20,
            thread: 30,
            birth: 40,
        }
    }
    fn view() -> Observation {
        Observation {
            root: root(),
            child: 11,
            child_pid: 20,
            child_thread: 30,
            foreground: 10,
            is_descendant: true,
            unicode: true,
            visible: true,
            read_only_or_password_or_disabled: false,
            same_process: false,
            class_name: "Edit".into(),
            real_class: "Edit".into(),
        }
    }
    struct Fake {
        views: VecDeque<Observation>,
        calls: Vec<(usize, Vec<u16>, u32)>,
        fail: bool,
    }
    impl Fake {
        fn new() -> Self {
            Self {
                views: VecDeque::from([view(), view(), view()]),
                calls: vec![],
                fail: false,
            }
        }
    }
    impl Backend for Fake {
        fn inspect(&mut self) -> Result<Observation, StandardEditError> {
            self.views
                .pop_front()
                .ok_or_else(|| StandardEditError::not_started("target_changed"))
        }
        fn replace_once(
            &mut self,
            child: usize,
            units: &[u16],
            timeout: u32,
        ) -> Result<(), StandardEditError> {
            self.calls.push((child, units.to_vec(), timeout));
            if self.fail {
                Err(StandardEditError::not_started("timeout"))
            } else {
                Ok(())
            }
        }
    }
    fn run(
        fake: &mut Fake,
        adapter: StandardEditAdapter,
        text: &str,
    ) -> Result<StandardEditReceipt, StandardEditError> {
        let now = Instant::now();
        execute(
            root(),
            adapter,
            text,
            now + Duration::from_secs(5),
            || now,
            || Ok(()),
            fake,
        )
    }

    #[test]
    fn exact_unicode_and_single_write_do_not_claim_content_acceptance() {
        let mut f = Fake::new();
        let receipt = run(&mut f, StandardEditAdapter::Win32Edit, "A\r\nB😀").unwrap();
        assert_eq!(
            f.calls,
            [(11, vec![65, 13, 10, 66, 0xd83d, 0xde00, 0], 1000)]
        );
        assert!(receipt.message_processed);
        assert_eq!(receipt.effect_state, "unknown");
    }

    #[test]
    fn adapter_is_explicit_and_prefix_without_real_edit_is_rejected() {
        let mut f = Fake::new();
        for v in &mut f.views {
            v.class_name = "WindowsForms10.EDIT.app.0.synthetic".into();
        }
        assert_eq!(
            run(&mut f, StandardEditAdapter::Win32Edit, "x")
                .unwrap_err()
                .code,
            "edit_unsupported"
        );
        assert!(f.calls.is_empty());
        let mut f = Fake::new();
        for v in &mut f.views {
            v.class_name = "WindowsForms10.EDIT.app.0.synthetic".into();
        }
        assert!(run(&mut f, StandardEditAdapter::WinFormsEdit, "x").is_ok());
        let mut f = Fake::new();
        f.views[0].real_class = "Chrome_WidgetWin_1".into();
        assert_eq!(
            run(&mut f, StandardEditAdapter::Win32Edit, "x")
                .unwrap_err()
                .code,
            "edit_unsupported"
        );
        assert!(f.calls.is_empty());
    }

    #[test]
    fn observed_winforms_class_casing_is_accepted_without_matching_other_controls() {
        for name in [
            "WindowsForms10.Edit.app.0.4e69f0_r3_ad1",
            "WindowsForms10.EDIT.app.0.synthetic",
            "windowsforms10.edit.app.0.synthetic",
        ] {
            let mut f = Fake::new();
            for view in &mut f.views {
                view.class_name = name.into();
            }
            assert!(run(&mut f, StandardEditAdapter::WinFormsEdit, "A😀").is_ok());
            assert_eq!(f.calls.len(), 1);
        }
        for name in [
            "WindowsForms10.EditX.app.0.synthetic",
            "WindowsForms10.Edit",
            "OtherWindowsForms10.Edit.app.0.synthetic",
            "界面界面界面界面界面界面界面界面",
        ] {
            let mut f = Fake::new();
            f.views[0].class_name = name.into();
            let error = run(&mut f, StandardEditAdapter::WinFormsEdit, "x").unwrap_err();
            assert_eq!(error.code, "edit_unsupported");
            assert_eq!(error.effect_state, "not_started");
            assert!(f.calls.is_empty());
        }
        let mut f = Fake::new();
        f.views[0].class_name = "WindowsForms10.Edit.app.0.synthetic".into();
        f.views[0].real_class = "CustomEdit".into();
        assert!(run(&mut f, StandardEditAdapter::WinFormsEdit, "x").is_err());
        assert!(f.calls.is_empty());
    }

    #[test]
    fn all_identity_and_control_preconditions_fail_before_write() {
        for index in 0..12 {
            let mut f = Fake::new();
            let v = &mut f.views[0];
            match index {
                0 => v.root.birth += 1,
                1 => v.child = 0,
                2 => v.child_pid += 1,
                3 => v.child_thread += 1,
                4 => v.foreground += 1,
                5 => v.is_descendant = false,
                6 => v.unicode = false,
                7 => v.visible = false,
                8 => v.read_only_or_password_or_disabled = true,
                9 => v.same_process = true,
                10 => v.root.hwnd += 1,
                _ => v.class_name = "CustomEdit".into(),
            }
            let error = run(&mut f, StandardEditAdapter::Win32Edit, "x").unwrap_err();
            assert_eq!(error.effect_state, "not_started");
            assert!(f.calls.is_empty());
        }
    }

    #[test]
    fn focused_child_change_is_never_recaptured_for_write() {
        let mut f = Fake::new();
        f.views[1].child += 1;
        assert_eq!(
            run(&mut f, StandardEditAdapter::Win32Edit, "x")
                .unwrap_err()
                .effect_state,
            "not_started"
        );
        assert!(f.calls.is_empty());
        let mut f = Fake::new();
        f.views[2].child += 1;
        assert_eq!(
            run(&mut f, StandardEditAdapter::Win32Edit, "x")
                .unwrap_err()
                .effect_state,
            "unknown"
        );
        assert_eq!(f.calls.len(), 1);
    }

    #[test]
    fn message_timeout_has_one_attempt_and_unknown_effect() {
        let mut f = Fake::new();
        f.fail = true;
        let error = run(&mut f, StandardEditAdapter::Win32Edit, "x").unwrap_err();
        assert_eq!(error.code, "timeout");
        assert_eq!(error.effect_state, "unknown");
        assert_eq!(f.calls.len(), 1);
    }

    #[test]
    fn budget_is_remaining_action_time_and_does_not_reset() {
        let now = Instant::now();
        let mut f = Fake::new();
        execute(
            root(),
            StandardEditAdapter::Win32Edit,
            "x",
            now + Duration::from_millis(23),
            || now,
            || Ok(()),
            &mut f,
        )
        .unwrap();
        assert_eq!(f.calls[0].2, 23);
        let mut f = Fake::new();
        let error = execute(
            root(),
            StandardEditAdapter::Win32Edit,
            "x",
            now,
            || now,
            || Ok(()),
            &mut f,
        )
        .unwrap_err();
        assert_eq!(error.code, "timeout");
        assert_eq!(error.effect_state, "not_started");
        assert!(f.calls.is_empty());
    }

    #[test]
    fn cancellation_before_commit_is_not_started_and_after_commit_unknown() {
        for fail_at in [1, 3, 4, 5] {
            let now = Instant::now();
            let mut f = Fake::new();
            let mut checks = 0;
            let error = execute(
                root(),
                StandardEditAdapter::Win32Edit,
                "x",
                now + Duration::from_secs(5),
                || now,
                || {
                    checks += 1;
                    if checks == fail_at {
                        Err(StandardEditError::not_started("cancelled"))
                    } else {
                        Ok(())
                    }
                },
                &mut f,
            )
            .unwrap_err();
            assert_eq!(error.code, "cancelled");
            assert_eq!(
                error.effect_state,
                if fail_at >= 4 {
                    "unknown"
                } else {
                    "not_started"
                }
            );
            assert_eq!(f.calls.len(), usize::from(fail_at >= 4));
        }
    }

    #[test]
    fn expiry_during_final_identity_observation_is_unknown() {
        let began = Instant::now();
        let deadline = began + Duration::from_millis(100);
        let mut ticks = 0;
        let mut f = Fake::new();
        let error = execute(
            root(),
            StandardEditAdapter::Win32Edit,
            "x",
            deadline,
            || {
                ticks += 1;
                if ticks >= 6 {
                    deadline
                } else {
                    began
                }
            },
            || Ok(()),
            &mut f,
        )
        .unwrap_err();
        assert_eq!(error.code, "timeout");
        assert_eq!(error.effect_state, "unknown");
        assert_eq!(f.calls.len(), 1);
    }

    #[test]
    fn bounds_nul_empty_and_surrogates_are_not_changed() {
        for text in ["x".repeat(8193), "x\0y".into()] {
            let mut f = Fake::new();
            assert_eq!(
                run(&mut f, StandardEditAdapter::Win32Edit, &text)
                    .unwrap_err()
                    .code,
                "invalid_input"
            );
            assert!(f.calls.is_empty());
        }
        let mut f = Fake::new();
        run(&mut f, StandardEditAdapter::Win32Edit, "").unwrap();
        assert_eq!(f.calls[0].1, [0]);
        let mut f = Fake::new();
        run(&mut f, StandardEditAdapter::Win32Edit, &"😀".repeat(4096)).unwrap();
        assert_eq!(f.calls[0].1.len(), 8193);
    }
    #[test]
    fn enabled_ancestry_includes_root_and_rejects_implicit_parent_disable() {
        let sample = |disabled| {
            enabled_ancestry_with(3, 1, |h| match h {
                3 => Some((2, disabled != 3)),
                2 => Some((1, disabled != 2)),
                1 => Some((0, disabled != 1)),
                _ => None,
            })
        };
        assert!(sample(0));
        assert!(!sample(1));
        assert!(!sample(2));
        assert!(!sample(3));
        assert!(!enabled_ancestry_with(3, 1, |_| None));
        assert!(!enabled_ancestry_with(3, 1, |_| Some((3, true))));
    }
}
