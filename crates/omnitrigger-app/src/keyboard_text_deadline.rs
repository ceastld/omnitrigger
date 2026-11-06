//! 普通 keyboard.text 的期限适配；复用已有发送回调，不改输入批次或释放清理。
use super::TextArgs;
use omnitrigger_actions::{ActionError, CancellationToken};
use serde_json::{json, Value};
use std::{
    cell::Cell,
    time::{Duration, Instant},
};

// 沿用两种现有 Native 客户端的普通调用等待预算；只在带动作期限的路径取较早者。
const MAX_SEND_WAIT: Duration = Duration::from_secs(5);

fn stopped(
    cancellation: &CancellationToken,
    deadline: Option<Instant>,
    now: Instant,
) -> Option<&'static str> {
    if cancellation.is_cancelled() {
        Some("cancelled")
    } else if deadline.is_some_and(|end| now >= end) {
        Some("timeout")
    } else {
        None
    }
}

fn fault(code: &str, message: impl ToString, effect: &str) -> ActionError {
    let mut error = ActionError::new(code, message.to_string());
    error.method = Some("keyboard.text".into());
    error.effect_state = Some(effect.into());
    error
}

fn check(
    cancellation: &CancellationToken,
    deadline: Option<Instant>,
    now: Instant,
    effect: &str,
) -> Result<(), ActionError> {
    match stopped(cancellation, deadline, now) {
        Some("cancelled") => Err(fault("cancelled", "动作已取消，未继续文本发送。", effect)),
        Some(_) => Err(fault("timeout", "文本发送超过动作期限。", effect)),
        None => Ok(()),
    }
}

pub(super) fn execute_with<T>(
    params: Value,
    cancellation: &CancellationToken,
    deadline: Option<Instant>,
    mut now: impl FnMut() -> Instant,
    target: impl FnOnce() -> Result<T, ActionError>,
    send: impl FnOnce(T, &str, &mut dyn FnMut() -> Result<(), String>) -> Result<(), String>,
) -> Result<Value, ActionError> {
    let began = now();
    let deadline = deadline.map(|outer| outer.min(began + MAX_SEND_WAIT));
    let args: TextArgs = serde_json::from_value(params)
        .map_err(|message| fault("invalid_input", message, "not_started"))?;
    omnitrigger_windows::validate_automation_text(&args.text)
        .map_err(|message| fault("invalid_input", message, "not_started"))?;
    check(cancellation, deadline, now(), "not_started")?;
    let target = target()?;
    check(cancellation, deadline, now(), "not_started")?;
    let stop_reason = Cell::new(None);
    let result = {
        // 现有 send_text_if 在等待 SENDING 门闩后、每批前后调用此回调。
        // String 错误没有精确提交计数；移交发送器后的任何失败均保守 unknown。
        let mut guard = || {
            check(cancellation, deadline, now(), "unknown").map_err(|error| {
                stop_reason.set(Some(if error.code == "cancelled" {
                    "cancelled"
                } else {
                    "timeout"
                }));
                error.message
            })
        };
        send(target, &args.text, &mut guard)
    };
    result.map_err(|message| {
        fault(
            stop_reason
                .get()
                .or_else(|| stopped(cancellation, deadline, now()))
                .unwrap_or("input_failed"),
            message,
            "unknown",
        )
    })?;
    check(cancellation, deadline, now(), "unknown")?;
    Ok(json!({"submitted":true}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use omnitrigger_actions::NativeHost;
    use std::{cell::Cell, time::Duration};

    fn assert_fault(error: ActionError, code: &str, effect: &str) {
        assert_eq!(error.code, code);
        assert_eq!(error.effect_state.as_deref(), Some(effect));
        assert_eq!(error.method.as_deref(), Some("keyboard.text"));
    }

    #[test]
    fn expired_before_target_never_enters_sender() {
        let start = Instant::now();
        let token = CancellationToken::new();
        let result = execute_with(
            json!({"text":"A"}),
            &token,
            Some(start),
            || start,
            || -> Result<(), ActionError> { panic!("不得核验目标") },
            |_, _, _| panic!("不得移交发送器"),
        );
        assert_fault(result.unwrap_err(), "timeout", "not_started");
    }

    #[test]
    fn cancelled_before_target_keeps_cancel_precedence() {
        let start = Instant::now();
        let token = CancellationToken::new();
        token.cancel();
        let result = execute_with(
            json!({"text":"A"}),
            &token,
            Some(start),
            || start,
            || -> Result<(), ActionError> { panic!("不得核验目标") },
            |_, _, _| panic!("不得移交发送器"),
        );
        assert_fault(result.unwrap_err(), "cancelled", "not_started");
    }

    #[test]
    fn expiry_during_identity_check_never_hands_off_input() {
        let start = Instant::now();
        let end = start + Duration::from_millis(10);
        let clock = Cell::new(start);
        let result = execute_with(
            json!({"text":"A"}),
            &CancellationToken::new(),
            Some(end),
            || clock.get(),
            || {
                clock.set(end);
                Ok(())
            },
            |_, _, _| panic!("不得移交发送器"),
        );
        assert_fault(result.unwrap_err(), "timeout", "not_started");
    }

    #[test]
    fn expiry_after_gate_wait_is_unknown_without_claiming_submission_count() {
        let start = Instant::now();
        let end = start + Duration::from_millis(10);
        let clock = Cell::new(start);
        let sends = Cell::new(0);
        let result = execute_with(
            json!({"text":"A"}),
            &CancellationToken::new(),
            Some(end),
            || clock.get(),
            || Ok(()),
            |_, _, guard| {
                clock.set(end); // 模拟已有门闩等待；第一次检查发生于获得门闩之后。
                guard()?;
                sends.set(sends.get() + 1);
                Ok(())
            },
        );
        assert_fault(result.unwrap_err(), "timeout", "unknown");
        assert_eq!(sends.get(), 0);
    }

    #[test]
    fn absolute_deadline_is_not_renewed_between_batches() {
        let start = Instant::now();
        let end = start + Duration::from_millis(10);
        let clock = Cell::new(start);
        let batches = Cell::new(0);
        let result = execute_with(
            json!({"text":"A".repeat(64)}),
            &CancellationToken::new(),
            Some(end),
            || clock.get(),
            || Ok(()),
            |_, _, guard| {
                clock.set(start + Duration::from_millis(9));
                guard()?;
                batches.set(batches.get() + 1);
                clock.set(end);
                guard()?; // 第一批之后到达原绝对期限，不能继续下一批。
                batches.set(batches.get() + 1);
                Ok(())
            },
        );
        assert_fault(result.unwrap_err(), "timeout", "unknown");
        assert_eq!(batches.get(), 1);
    }

    #[test]
    fn existing_five_second_budget_wins_over_a_later_action_deadline() {
        let start = Instant::now();
        let clock = Cell::new(start);
        let sends = Cell::new(0);
        let result = execute_with(
            json!({"text":"A"}),
            &CancellationToken::new(),
            Some(start + Duration::from_secs(60)),
            || clock.get(),
            || Ok(()),
            |_, _, guard| {
                clock.set(start + Duration::from_secs(5));
                guard()?;
                sends.set(1);
                Ok(())
            },
        );
        assert_fault(result.unwrap_err(), "timeout", "unknown");
        assert_eq!(sends.get(), 0);
    }

    #[test]
    fn cancellation_between_batches_does_not_retry() {
        let start = Instant::now();
        let token = CancellationToken::new();
        let batches = Cell::new(0);
        let result = execute_with(
            json!({"text":"A"}),
            &token,
            None,
            || start,
            || Ok(()),
            |_, _, guard| {
                guard()?;
                batches.set(1);
                token.cancel();
                guard()?;
                batches.set(2);
                Ok(())
            },
        );
        assert_fault(result.unwrap_err(), "cancelled", "unknown");
        assert_eq!(batches.get(), 1);
    }

    #[test]
    fn expiry_after_sender_returns_success_is_still_unknown() {
        let start = Instant::now();
        let end = start + Duration::from_millis(10);
        let clock = Cell::new(start);
        let result = execute_with(
            json!({"text":"A"}),
            &CancellationToken::new(),
            Some(end),
            || clock.get(),
            || Ok(()),
            |_, _, guard| {
                guard()?;
                clock.set(end);
                Ok(())
            },
        );
        assert_fault(result.unwrap_err(), "timeout", "unknown");
    }

    #[test]
    fn healthy_sender_receives_exact_text_once() {
        let start = Instant::now();
        let clock = Cell::new(start);
        let sends = Cell::new(0);
        let result = execute_with(
            json!({"text":"A😀\r\nB"}),
            &CancellationToken::new(),
            None,
            || clock.get(),
            || Ok(7),
            |target, text, guard| {
                assert_eq!(target, 7);
                assert_eq!(text, "A😀\r\nB");
                clock.set(start + Duration::from_secs(60)); // 无deadline旧直调不新增5秒预算。
                guard()?;
                sends.set(sends.get() + 1);
                guard()
            },
        );
        assert_eq!(result.unwrap(), json!({"submitted":true}));
        assert_eq!(sends.get(), 1);
    }

    #[test]
    fn platform_failure_keeps_unknown_and_is_not_retried() {
        let attempts = Cell::new(0);
        let result = execute_with(
            json!({"text":"A"}),
            &CancellationToken::new(),
            None,
            Instant::now,
            || Ok(()),
            |_, _, guard| {
                guard()?;
                attempts.set(attempts.get() + 1);
                Err("原目标/第三窗检查失败；保留已有释放清理结果".into())
            },
        );
        assert_fault(result.unwrap_err(), "input_failed", "unknown");
        assert_eq!(attempts.get(), 1);
    }

    #[test]
    fn invalid_input_never_queries_target_or_sender() {
        for params in [
            json!({"text":"A".repeat(65537)}),
            json!({"text":"A", "extra":1}),
        ] {
            let result = execute_with(
                params,
                &CancellationToken::new(),
                None,
                Instant::now,
                || -> Result<(), ActionError> { panic!("不得核验目标") },
                |_, _, _| panic!("不得移交发送器"),
            );
            assert_fault(result.unwrap_err(), "invalid_input", "not_started");
        }
    }

    #[test]
    fn native_deadline_dispatch_rejects_before_missing_original_target() {
        let host = super::super::DesktopHost {
            foreground_targets: super::super::ForegroundTargets::default(),
            target: Err(ActionError::new("sentinel_target", "不得触及原目标")),
        };
        let result = host.call_with_deadline(
            "keyboard.text",
            json!({"text":"A"}),
            &CancellationToken::new(),
            Instant::now(),
        );
        assert_fault(result.unwrap_err(), "timeout", "not_started");
    }
}
