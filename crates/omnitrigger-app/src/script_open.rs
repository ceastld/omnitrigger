//! Node显式打开目标；复用轻量宿主已有平台校验与打开，不拼接命令行。
use omnitrigger_actions::{ActionError, CancellationToken};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Instant;

const METHOD: &str = "system.open";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Params {
    target: String,
}
fn fault(code: &str, started: bool) -> ActionError {
    let mut error = ActionError::new(code, "打开请求未确认，请检查错误代码。");
    error.method = Some(METHOD.into());
    error.effect_state = Some(if started { "unknown" } else { "not_started" }.into());
    error
}
fn check(
    cancellation: &CancellationToken,
    deadline: Instant,
    started: bool,
) -> Result<(), ActionError> {
    if cancellation.is_cancelled() {
        Err(fault("cancelled", started))
    } else if Instant::now() >= deadline {
        Err(fault("timeout", started))
    } else {
        Ok(())
    }
}
fn open_with(
    params: Value,
    cancellation: &CancellationToken,
    deadline: Instant,
    submit: impl FnOnce(&str) -> Result<(), String>,
) -> Result<Value, ActionError> {
    let params: Params =
        serde_json::from_value(params).map_err(|_| fault("invalid_input", false))?;
    omnitrigger_windows::validate_open_target(&params.target)
        .map_err(|_| fault("invalid_input", false))?;
    check(cancellation, deadline, false)?;
    let submitted = submit(&params.target);
    // 单次系统打开无法被强制抢占；取消和迟到返回不意味着已提交请求已撤销。
    check(cancellation, deadline, true)?;
    submitted.map_err(|_| fault("open_failed", true))?;
    Ok(json!({"submitted":true}))
}
pub fn open(
    params: Value,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<Value, ActionError> {
    #[cfg(windows)]
    {
        open_with(
            params,
            cancellation,
            deadline,
            omnitrigger_windows::open_target,
        )
    }
    #[cfg(not(windows))]
    {
        let _ = (params, cancellation, deadline);
        Err(fault("open_unavailable", false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, time::Duration};
    fn deadline() -> Instant {
        Instant::now() + Duration::from_secs(60)
    }
    fn assert_fault(error: ActionError, code: &str, started: bool) {
        assert_eq!(error.code, code);
        assert_eq!(error.method.as_deref(), Some(METHOD));
        assert_eq!(
            error.effect_state.as_deref(),
            Some(if started { "unknown" } else { "not_started" })
        );
        assert!(error.required_permissions.is_empty());
        assert!(!error.message.contains("PRIVATE"));
    }
    #[test]
    fn accepted_targets_use_existing_validator_and_preserve_exact_text() {
        for target in [
            "https://example.invalid/?q=%E4%B8%AD",
            "mailto:person@example.invalid",
            "about:blank",
            "C:/Owned/space & unicode 中.txt",
        ] {
            let calls = Cell::new(0);
            let result = open_with(
                json!({"target":target}),
                &CancellationToken::new(),
                deadline(),
                |actual| {
                    calls.set(calls.get() + 1);
                    assert_eq!(actual, target);
                    Ok(())
                },
            );
            assert_eq!(result.unwrap(), json!({"submitted":true}));
            assert_eq!(calls.get(), 1);
        }
    }
    #[test]
    fn invalid_shape_or_target_never_submits() {
        for params in [
            Value::Null,
            json!({"target":42}),
            json!({"target":"https://example.invalid","args":[]}),
            json!({"target":"https://example.invalid","verb":"runas"}),
            json!({"target":"javascript:PRIVATE"}),
            json!({"target":"relative-PRIVATE.txt"}),
            json!({"target":" https://example.invalid"}),
            json!({"target":"C:/PRIVATE\u{0}.txt"}),
        ] {
            assert_fault(
                open_with(params, &CancellationToken::new(), deadline(), |_| {
                    panic!("无效请求不能打开目标")
                })
                .unwrap_err(),
                "invalid_input",
                false,
            );
        }
    }
    #[test]
    fn pre_dispatch_cancel_or_deadline_never_submits() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert_fault(
            open_with(
                json!({"target":"about:blank"}),
                &cancellation,
                deadline(),
                |_| panic!("取消不能提交"),
            )
            .unwrap_err(),
            "cancelled",
            false,
        );
        assert_fault(
            open_with(
                json!({"target":"about:blank"}),
                &CancellationToken::new(),
                Instant::now(),
                |_| panic!("过期不能提交"),
            )
            .unwrap_err(),
            "timeout",
            false,
        );
    }
    #[test]
    fn after_submit_cancel_and_platform_failure_are_unknown_and_never_retried() {
        let cancellation = CancellationToken::new();
        let calls = Cell::new(0);
        assert_fault(
            open_with(
                json!({"target":"about:blank"}),
                &cancellation,
                deadline(),
                |_| {
                    calls.set(calls.get() + 1);
                    cancellation.cancel();
                    Ok(())
                },
            )
            .unwrap_err(),
            "cancelled",
            true,
        );
        assert_eq!(calls.get(), 1);
        assert_fault(
            open_with(
                json!({"target":"about:blank"}),
                &CancellationToken::new(),
                deadline(),
                |_| Err("PRIVATE system error".into()),
            )
            .unwrap_err(),
            "open_failed",
            true,
        );
    }
}
