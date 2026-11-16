// 只测真实提交状态；不构造GPUI窗口，不调用平台或NativeHost。
use super::*;
use std::cell::Cell;

#[test]
fn action_hint_cancelled_or_expired_pending_request_never_applies_late() {
    for cancelled in [false, true] {
        let token = CancellationToken::new();
        if cancelled {
            token.cancel();
        }
        let deadline = Instant::now()
            + if cancelled {
                Duration::from_secs(1)
            } else {
                Duration::ZERO
            };
        let submission = ActionHintSubmission::new();
        let calls = Cell::new(0);
        let error = submission.result(&token, deadline).unwrap().unwrap_err();
        assert_eq!(error.effect_state.as_deref(), Some("not_started"));
        submission.apply(
            &CancellationToken::new(),
            Instant::now() + Duration::from_secs(1),
            || {
                calls.set(calls.get() + 1);
                Ok(())
            },
        );
        assert_eq!(calls.get(), 0);
    }
}

#[test]
fn action_hint_success_means_model_update_once_and_later_interrupt_is_unknown() {
    let token = CancellationToken::new();
    let deadline = Instant::now() + Duration::from_secs(1);
    let submission = ActionHintSubmission::new();
    let calls = Cell::new(0);
    submission.apply(&token, deadline, || {
        calls.set(calls.get() + 1);
        Ok(())
    });
    submission.apply(&token, deadline, || {
        calls.set(calls.get() + 1);
        Ok(())
    });
    assert_eq!(calls.get(), 1);
    assert_eq!(
        submission.result(&token, deadline).unwrap().unwrap(),
        json!({"updated":true})
    );
    token.cancel();
    let error = submission.result(&token, deadline).unwrap().unwrap_err();
    assert_eq!(error.effect_state.as_deref(), Some("unknown"));
    assert_eq!(error.method.as_deref(), Some("ui.action_hint"));
}

#[test]
fn action_hint_scope_failure_preserves_error_and_does_not_retry_update() {
    let token = CancellationToken::new();
    let deadline = Instant::now() + Duration::from_secs(1);
    let submission = ActionHintSubmission::new();
    submission.apply(&token, deadline, || {
        Err(hint_fault("ui_hint_scope_expired", "not_started"))
    });
    submission.apply(&token, deadline, || panic!("不得重试"));
    assert_eq!(
        submission
            .result(&token, deadline)
            .unwrap()
            .unwrap_err()
            .code,
        "ui_hint_scope_expired"
    );
}
