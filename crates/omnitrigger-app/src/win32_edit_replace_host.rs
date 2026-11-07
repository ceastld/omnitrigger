//! 显式Win32 Edit替换，不回退键盘、剪贴板或其他类；错误不含原文。
use super::{ActionError, CancellationToken};
use omnitrigger_windows::{StandardEditAdapter, StandardEditError, StandardEditReceipt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

const METHOD: &str = "win32_edit.replace_selection";
const MESSAGE: &str = "Win32 Edit选区替换未获完整确认；请检查错误代码和效果状态。";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Adapter {
    NativeEdit,
    WinformsEdit,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    text: String,
    adapter: Adapter,
}

fn fault(code: &str, effect: &str) -> ActionError {
    let mut error = ActionError::new(code, MESSAGE);
    error.method = Some(METHOD.into());
    error.effect_state = Some(effect.into());
    error
}

fn parse(params: Value) -> Result<(String, StandardEditAdapter), ActionError> {
    let args: Args =
        serde_json::from_value(params).map_err(|_| fault("invalid_input", "not_started"))?;
    if args.text.contains('\0')
        || args.text.len() > 8192 * 4
        || args.text.encode_utf16().take(8193).count() > 8192
    {
        return Err(fault("invalid_input", "not_started"));
    }
    let adapter = match args.adapter {
        Adapter::NativeEdit => StandardEditAdapter::Win32Edit,
        Adapter::WinformsEdit => StandardEditAdapter::WinFormsEdit,
    };
    Ok((args.text, adapter))
}

fn check(
    cancellation: &CancellationToken,
    deadline: Instant,
    now: Instant,
) -> Result<(), StandardEditError> {
    if cancellation.is_cancelled() {
        Err(StandardEditError::not_started("cancelled"))
    } else if now >= deadline {
        Err(StandardEditError::not_started("timeout"))
    } else {
        Ok(())
    }
}

fn mapped(error: StandardEditError) -> ActionError {
    let code = match error.code {
        "invalid_input"
        | "cancelled"
        | "timeout"
        | "target_changed"
        | "edit_unsupported"
        | "permission_denied"
        | "edit_message_unconfirmed" => error.code,
        _ => "edit_message_unconfirmed",
    };
    fault(
        code,
        if error.effect_state == "not_started" {
            "not_started"
        } else {
            "unknown"
        },
    )
}

pub(super) fn execute_with<T>(
    params: Value,
    cancellation: &CancellationToken,
    action_deadline: Option<Instant>,
    mut now: impl FnMut() -> Instant,
    target: impl FnOnce() -> Result<T, ActionError>,
    send: impl FnOnce(
        T,
        &str,
        StandardEditAdapter,
        Instant,
        &mut dyn FnMut() -> Result<(), StandardEditError>,
    ) -> Result<StandardEditReceipt, StandardEditError>,
) -> Result<Value, ActionError> {
    let began = now();
    let local = began
        .checked_add(Duration::from_secs(5))
        .ok_or_else(|| fault("timeout", "not_started"))?;
    let deadline = action_deadline.map_or(local, |outer| outer.min(local));
    let (text, adapter) = parse(params)?;
    check(cancellation, deadline, now()).map_err(mapped)?;
    let target = target().map_err(|_| fault("target_changed", "not_started"))?;
    check(cancellation, deadline, now()).map_err(mapped)?;
    let receipt = {
        let mut guard = || check(cancellation, deadline, now());
        send(target, &text, adapter, deadline, &mut guard).map_err(mapped)?
    };
    // 平台执行负责区分写前拒绝与移交消息后的unknown；此处不推测接受正文数量。
    if cancellation.is_cancelled() {
        return Err(fault("cancelled", "unknown"));
    }
    if now() >= deadline {
        return Err(fault("timeout", "unknown"));
    }
    if !receipt.message_processed || receipt.effect_state != "unknown" {
        return Err(fault("invalid_response", "unknown"));
    }
    Ok(json!({"messageProcessed":true,"effectState":"unknown"}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    fn receipt() -> StandardEditReceipt {
        StandardEditReceipt {
            message_processed: true,
            effect_state: "unknown",
        }
    }

    #[test]
    fn strict_adapter_text_shape_and_utf16_bounds() {
        assert_eq!(
            parse(json!({"text":"😀".repeat(4096),"adapter":"native-edit"}))
                .unwrap()
                .0
                .encode_utf16()
                .count(),
            8192
        );
        assert!(parse(json!({"text":"","adapter":"winforms-edit"})).is_ok());
        for params in [
            json!({"text":"x"}),
            json!({"text":"x","adapter":"auto"}),
            json!({"text":5,"adapter":"native-edit"}),
            json!({"text":"x","adapter":"native-edit","hwnd":10}),
            json!({"text":"x\0y","adapter":"native-edit"}),
            json!({"text":"x".repeat(8193),"adapter":"native-edit"}),
        ] {
            let error = parse(params).unwrap_err();
            assert_eq!(error.code, "invalid_input");
            assert_eq!(error.effect_state.as_deref(), Some("not_started"));
            assert_eq!(error.message, MESSAGE);
        }
    }

    #[test]
    fn pre_cancel_and_expired_deadline_do_not_resolve_target_or_send() {
        let now = Instant::now();
        let token = CancellationToken::new();
        token.cancel();
        let error = execute_with(
            json!({"text":"x","adapter":"native-edit"}),
            &token,
            None,
            || now,
            || -> Result<(), ActionError> { panic!("预取消不能取目标") },
            |_, _, _, _, _| panic!("不能写消息"),
        )
        .unwrap_err();
        assert_eq!(error.code, "cancelled");
        let error = execute_with(
            json!({"text":"x","adapter":"native-edit"}),
            &CancellationToken::new(),
            Some(now),
            || now,
            || -> Result<(), ActionError> { panic!("已到期不能取目标") },
            |_, _, _, _, _| panic!("不能写消息"),
        )
        .unwrap_err();
        assert_eq!(error.code, "timeout");
    }

    #[test]
    fn one_target_one_send_and_receipt_contains_no_text_or_identity() {
        let now = Instant::now();
        let calls = Cell::new(0);
        let value = execute_with(
            json!({"text":"A\r\nB😀","adapter":"winforms-edit"}),
            &CancellationToken::new(),
            Some(now + Duration::from_millis(40)),
            || now,
            || {
                calls.set(calls.get() + 1);
                Ok(17)
            },
            |target, text, adapter, deadline, check| {
                assert_eq!(target, 17);
                assert_eq!(text, "A\r\nB😀");
                assert_eq!(adapter, StandardEditAdapter::WinFormsEdit);
                assert_eq!(deadline, now + Duration::from_millis(40));
                check()?;
                calls.set(calls.get() + 1);
                Ok(receipt())
            },
        )
        .unwrap();
        assert_eq!(calls.get(), 2);
        assert_eq!(
            value,
            json!({"messageProcessed":true,"effectState":"unknown"})
        );
    }

    #[test]
    fn none_and_long_action_deadline_never_extend_five_second_operation_budget() {
        let now = Instant::now();
        for outer in [None, Some(now + Duration::from_secs(30))] {
            execute_with(
                json!({"text":"x","adapter":"native-edit"}),
                &CancellationToken::new(),
                outer,
                || now,
                || Ok(()),
                |_, _, _, deadline, _| {
                    assert_eq!(deadline, now + Duration::from_secs(5));
                    Ok(receipt())
                },
            )
            .unwrap();
        }
    }

    #[test]
    fn preflight_and_post_message_failures_preserve_effect_and_never_retry() {
        let now = Instant::now();
        for (code, effect) in [
            ("edit_unsupported", "not_started"),
            ("target_changed", "not_started"),
            ("permission_denied", "unknown"),
            ("timeout", "unknown"),
        ] {
            let count = Cell::new(0);
            let error = execute_with(
                json!({"text":"private","adapter":"native-edit"}),
                &CancellationToken::new(),
                None,
                || now,
                || Ok(()),
                |_, _, _, _, _| {
                    count.set(count.get() + 1);
                    Err(StandardEditError {
                        code,
                        effect_state: effect,
                    })
                },
            )
            .unwrap_err();
            assert_eq!(count.get(), 1);
            assert_eq!(error.effect_state.as_deref(), Some(effect));
            assert_eq!(error.method.as_deref(), Some(METHOD));
            assert_eq!(error.message, MESSAGE);
            assert!(error.required_permissions.is_empty());
        }
    }

    #[test]
    fn cancellation_and_expiry_after_message_are_unknown() {
        let now = Instant::now();
        let token = CancellationToken::new();
        let error = execute_with(
            json!({"text":"x","adapter":"native-edit"}),
            &token,
            None,
            || now,
            || Ok(()),
            |_, _, _, _, _| {
                token.cancel();
                Ok(receipt())
            },
        )
        .unwrap_err();
        assert_eq!(error.code, "cancelled");
        assert_eq!(error.effect_state.as_deref(), Some("unknown"));
        let clock = Cell::new(now);
        let error = execute_with(
            json!({"text":"x","adapter":"native-edit"}),
            &CancellationToken::new(),
            Some(now + Duration::from_millis(40)),
            || clock.get(),
            || Ok(()),
            |_, _, _, deadline, _| {
                clock.set(deadline);
                Ok(receipt())
            },
        )
        .unwrap_err();
        assert_eq!(error.code, "timeout");
        assert_eq!(error.effect_state.as_deref(), Some("unknown"));
    }

    #[test]
    fn target_failure_redacts_details_and_fails_before_message() {
        let now = Instant::now();
        let error = execute_with(
            json!({"text":"private","adapter":"native-edit"}),
            &CancellationToken::new(),
            None,
            || now,
            || -> Result<(), ActionError> { Err(ActionError::new("sensitive", "private title")) },
            |_, _, _, _, _| panic!("不能写消息"),
        )
        .unwrap_err();
        assert_eq!(error.code, "target_changed");
        assert_eq!(error.message, MESSAGE);
        assert_eq!(error.effect_state.as_deref(), Some("not_started"));
    }
}
