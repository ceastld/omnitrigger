//! 可持久化的窗口操作；目标捕获与执行校验由 Windows 适配层完成。
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowOperation {
    ToggleMaximize,
    Restore,
    ToggleTopmost,
    Minimize,
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowAction {
    pub operation: WindowOperation,
}

impl WindowAction {
    pub fn display(self) -> &'static str {
        match self.operation {
            WindowOperation::ToggleMaximize => "最大化 / 恢复",
            WindowOperation::Restore => "恢复窗口",
            WindowOperation::ToggleTopmost => "置顶 / 取消置顶",
            WindowOperation::Minimize => "最小化",
            WindowOperation::Close => "关闭窗口",
        }
    }

    pub fn platform_operation(self) -> omnitrigger_windows::WindowOperation {
        match self.operation {
            WindowOperation::ToggleMaximize => omnitrigger_windows::WindowOperation::ToggleMaximize,
            WindowOperation::Restore => omnitrigger_windows::WindowOperation::Restore,
            WindowOperation::ToggleTopmost => omnitrigger_windows::WindowOperation::ToggleTopmost,
            WindowOperation::Minimize => omnitrigger_windows::WindowOperation::Minimize,
            WindowOperation::Close => omnitrigger_windows::WindowOperation::Close,
        }
    }
}

fn fault(code: &str, message: impl ToString, effect: &str) -> omnitrigger_actions::ActionError {
    let mut error = omnitrigger_actions::ActionError::new(code, message.to_string());
    error.method = Some("window.action".into());
    error.effect_state = Some(effect.into());
    error
}

fn check_action(
    cancellation: &omnitrigger_actions::CancellationToken,
    deadline: Option<std::time::Instant>,
    effect: &str,
) -> Result<(), omnitrigger_actions::ActionError> {
    if cancellation.is_cancelled() {
        return Err(fault("cancelled", "窗口操作已取消。", effect));
    }
    if deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline) {
        return Err(fault("timeout", "窗口操作超过本次动作期限。", effect));
    }
    Ok(())
}

/// 真实DesktopHost共用此接缝；resolve只能返回本次原目标，不可重抓前台。
/// 平台层每次检查和提交前都调用guard；提交返回后的取消/迟到仅能标记unknown。
pub(crate) fn execute_with<T>(
    params: serde_json::Value,
    cancellation: &omnitrigger_actions::CancellationToken,
    deadline: Option<std::time::Instant>,
    resolve: impl FnOnce() -> Result<T, omnitrigger_actions::ActionError>,
    submit: impl FnOnce(
        T,
        omnitrigger_windows::WindowOperation,
        &mut dyn FnMut() -> Result<(), String>,
    ) -> Result<(), String>,
) -> Result<serde_json::Value, omnitrigger_actions::ActionError> {
    // 命名结构体的序列形式不属于既有 window.action 对象合同。
    if !params.is_object() {
        return Err(fault(
            "invalid_input",
            "窗口操作参数必须为 JSON 对象。",
            "not_started",
        ));
    }
    let action: WindowAction = serde_json::from_value(params)
        .map_err(|message| fault("invalid_input", message, "not_started"))?;
    check_action(cancellation, deadline, "not_started")?;
    let target = resolve().map_err(|mut error| {
        error.method = Some("window.action".into());
        error
    })?;
    check_action(cancellation, deadline, "not_started")?;
    let mut guard_error = None;
    let result = submit(target, action.platform_operation(), &mut || {
        check_action(cancellation, deadline, "not_started").map_err(|error| {
            let message = error.message.clone();
            guard_error = Some(error);
            message
        })
    });
    if let Some(error) = guard_error {
        return Err(error);
    }
    check_action(cancellation, deadline, "unknown")?;
    result.map_err(|message| fault("window_action_failed", message, "unknown"))?;
    Ok(serde_json::json!({"submitted":true}))
}

#[cfg(test)]
mod topmost_tests {
    use super::*;
    use omnitrigger_actions::{ActionError, CancellationToken};
    use serde_json::json;
    use std::{cell::Cell, time::Instant};

    #[test]
    fn topmost_roundtrip_display_and_platform_mapping_are_real() {
        let action: WindowAction =
            serde_json::from_value(json!({"operation":"toggle_topmost"})).unwrap();
        assert_eq!(action.operation, WindowOperation::ToggleTopmost);
        assert_eq!(action.display(), "置顶 / 取消置顶");
        assert_eq!(
            action.platform_operation(),
            omnitrigger_windows::WindowOperation::ToggleTopmost
        );
        assert_eq!(
            serde_json::to_value(action).unwrap(),
            json!({"operation":"toggle_topmost"})
        );
        for operation in ["toggle_maximize", "minimize", "close"] {
            assert!(serde_json::from_value::<WindowAction>(json!({"operation":operation})).is_ok());
        }
    }

    #[test]
    fn invalid_window_parameters_never_resolve_a_target() {
        for params in [
            json!([]),
            json!({}),
            json!({"operation":"topmost"}),
            json!({"operation":"toggle_topmost","hwnd":1}),
        ] {
            let error = execute_with(
                params,
                &CancellationToken::new(),
                None,
                || -> Result<(), ActionError> { panic!("无效参数不得解析目标") },
                |_, _, _| panic!("无效参数不得提交"),
            )
            .unwrap_err();
            assert_eq!(error.code, "invalid_input");
            assert_eq!(error.effect_state.as_deref(), Some("not_started"));
        }
    }

    #[test]
    fn cancelled_or_expired_window_calls_never_resolve_a_target() {
        for cancelled in [false, true] {
            let token = CancellationToken::new();
            if cancelled {
                token.cancel();
            }
            let error = execute_with(
                json!({"operation":"toggle_topmost"}),
                &token,
                Some(Instant::now()),
                || -> Result<(), ActionError> { panic!("取消/过期不得解析目标") },
                |_, _, _| panic!("取消/过期不得提交"),
            )
            .unwrap_err();
            assert_eq!(error.code, if cancelled { "cancelled" } else { "timeout" });
            assert_eq!(error.method.as_deref(), Some("window.action"));
            assert_eq!(error.effect_state.as_deref(), Some("not_started"));
        }
    }

    #[test]
    fn cancellation_while_resolving_original_target_stops_before_submission() {
        let token = CancellationToken::new();
        let error = execute_with(
            json!({"operation":"toggle_topmost"}),
            &token,
            None,
            || {
                token.cancel();
                Ok(())
            },
            |_, _, _| panic!("原目标核验后取消不得提交"),
        )
        .unwrap_err();
        assert_eq!(error.code, "cancelled");
        assert_eq!(error.effect_state.as_deref(), Some("not_started"));
    }

    #[test]
    fn platform_guard_cancellation_keeps_structured_not_started_fault() {
        let token = CancellationToken::new();
        let error = execute_with(
            json!({"operation":"toggle_topmost"}),
            &token,
            None,
            || Ok(()),
            |_, operation, check| {
                assert_eq!(
                    operation,
                    omnitrigger_windows::WindowOperation::ToggleTopmost
                );
                token.cancel();
                check()?;
                panic!("平台最终检查取消后不得原生提交")
            },
        )
        .unwrap_err();
        assert_eq!(error.code, "cancelled");
        assert_eq!(error.effect_state.as_deref(), Some("not_started"));
    }

    #[test]
    fn successful_or_rejected_submission_runs_once_and_late_cancel_is_unknown() {
        for scenario in ["accepted", "rejected", "late_cancel"] {
            let token = CancellationToken::new();
            let calls = Cell::new(0);
            let result = execute_with(
                json!({"operation":"toggle_topmost"}),
                &token,
                None,
                || Ok(17),
                |target, operation, check| {
                    assert_eq!(target, 17);
                    assert_eq!(
                        operation,
                        omnitrigger_windows::WindowOperation::ToggleTopmost
                    );
                    check()?;
                    calls.set(calls.get() + 1);
                    if scenario == "late_cancel" {
                        token.cancel();
                    }
                    if scenario == "rejected" {
                        Err("系统未接纳".into())
                    } else {
                        Ok(())
                    }
                },
            );
            assert_eq!(calls.get(), 1);
            if scenario == "accepted" {
                assert_eq!(result.unwrap(), json!({"submitted":true}));
            } else {
                let error = result.unwrap_err();
                assert_eq!(
                    error.code,
                    if scenario == "rejected" {
                        "window_action_failed"
                    } else {
                        "cancelled"
                    }
                );
                assert_eq!(error.effect_state.as_deref(), Some("unknown"));
            }
        }
    }
}

#[cfg(test)]
mod restore_tests {
    use super::*;
    use omnitrigger_actions::{ActionError, CancellationToken};
    use serde_json::json;
    use std::{cell::Cell, time::Instant};

    #[test]
    fn restore_roundtrips_and_maps_to_the_explicit_platform_operation() {
        let action: WindowAction = serde_json::from_value(json!({"operation":"restore"})).unwrap();
        assert_eq!(action.operation, WindowOperation::Restore);
        assert_eq!(action.display(), "恢复窗口");
        assert_eq!(
            action.platform_operation(),
            omnitrigger_windows::WindowOperation::Restore
        );
        assert_eq!(
            serde_json::to_value(action).unwrap(),
            json!({"operation":"restore"})
        );
    }

    #[test]
    fn restore_raw_wire_invalid_shapes_never_resolve_or_submit() {
        for params in [
            json!(["restore"]),
            json!("restore"),
            json!({"operation":"restore","hwnd":17}),
        ] {
            let error = execute_with(
                params,
                &CancellationToken::new(),
                None,
                || -> Result<(), ActionError> { panic!("无效还原参数不得解析目标") },
                |_, _, _| panic!("无效还原参数不得提交"),
            )
            .unwrap_err();
            assert_eq!(error.code, "invalid_input");
            assert_eq!(error.method.as_deref(), Some("window.action"));
            assert_eq!(error.effect_state.as_deref(), Some("not_started"));
        }
    }

    #[test]
    fn restore_cancel_or_deadline_before_resolution_preserves_not_started() {
        for cancelled in [false, true] {
            let token = CancellationToken::new();
            if cancelled {
                token.cancel();
            }
            let error = execute_with(
                json!({"operation":"restore"}),
                &token,
                Some(Instant::now()),
                || -> Result<(), ActionError> { panic!("取消或过期不得解析目标") },
                |_, _, _| panic!("取消或过期不得提交"),
            )
            .unwrap_err();
            assert_eq!(error.code, if cancelled { "cancelled" } else { "timeout" });
            assert_eq!(error.method.as_deref(), Some("window.action"));
            assert_eq!(error.effect_state.as_deref(), Some("not_started"));
        }
    }

    #[test]
    fn restore_submits_once_and_does_not_claim_completion_or_retry_after_failure() {
        for scenario in ["accepted", "rejected", "late_cancel", "guard_cancel"] {
            let token = CancellationToken::new();
            let calls = Cell::new(0);
            let result = execute_with(
                json!({"operation":"restore"}),
                &token,
                None,
                || Ok(17),
                |target, operation, check| {
                    assert_eq!(target, 17);
                    assert_eq!(operation, omnitrigger_windows::WindowOperation::Restore);
                    if scenario == "guard_cancel" {
                        token.cancel();
                    }
                    check()?;
                    calls.set(calls.get() + 1);
                    if scenario == "late_cancel" {
                        token.cancel();
                    }
                    if scenario == "rejected" {
                        Err("未接纳".into())
                    } else {
                        Ok(())
                    }
                },
            );
            assert_eq!(calls.get(), if scenario == "guard_cancel" { 0 } else { 1 });
            if scenario == "accepted" {
                assert_eq!(result.unwrap(), json!({"submitted":true}));
            } else {
                let error = result.unwrap_err();
                assert_eq!(
                    error.code,
                    if scenario == "rejected" {
                        "window_action_failed"
                    } else {
                        "cancelled"
                    }
                );
                assert_eq!(error.method.as_deref(), Some("window.action"));
                assert_eq!(
                    error.effect_state.as_deref(),
                    Some(if scenario == "guard_cancel" {
                        "not_started"
                    } else {
                        "unknown"
                    })
                );
            }
        }
    }
}
