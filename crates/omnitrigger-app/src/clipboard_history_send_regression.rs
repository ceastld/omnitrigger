//! 发送与界面完成回执的独立合成回归；作为clipboard_history_send的测试子模块接线。

use super::*;
use crate::clipboard_history_store::HistoryList;
use crate::clipboard_history_ui_state::HistoryUiState;
use std::sync::atomic::{AtomicUsize, Ordering};

fn identity() -> ScriptTargetSnapshot {
    ScriptTargetSnapshot {
        window_handle: 123,
        process_id: 7,
        thread_id: 8,
        process_created_file_time: 134_386_985_330_123_456,
    }
}

fn context(expires: u64) -> Value {
    let target = identity();
    json!({"window_handle":target.window_handle.to_string(),"process_id":target.process_id,
        "thread_id":target.thread_id,"process_created_file_time":target.process_created_file_time.to_string(),
        "expires_at_unix_ms":expires})
}

fn selection(id: &str) -> HistorySendSelection {
    HistorySendSelection {
        summary: HistorySummary {
            id: id.into(),
            preview: "合成".into(),
            utf16_units: 2,
            created_at: "10".into(),
        },
        revision: "2".into(),
    }
}

fn state() -> HistoryUiState {
    let mut state = HistoryUiState::new(true);
    state.listed(
        None,
        Ok(HistoryList {
            items: vec![selection("1").summary, selection("2").summary],
            revision: "2".into(),
        }),
    );
    state.select("1");
    state
}

struct CancelAfterSubmission {
    submitted: AtomicUsize,
}

impl NativeHost for CancelAfterSubmission {
    fn call(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
    ) -> Result<Value, ActionError> {
        match method {
            "target.context" => Ok(context(31_000)),
            "history.get" => {
                assert_eq!(params, json!({"id":"1"}));
                Ok(json!({"id":"1","text":"合成","createdAt":"10","revision":"2"}))
            }
            "keyboard.text" => {
                assert_eq!(params, json!({"text":"合成"}));
                self.submitted.fetch_add(1, Ordering::Relaxed);
                // 模拟输入已经提交后用户切换选择；只取消内存token，无桌面操作。
                cancellation.cancel();
                Ok(json!({"submitted":true}))
            }
            _ => panic!("回归Host不支持未声明方法"),
        }
    }
}

impl SendPort for CancelAfterSubmission {
    fn restore_original(
        &self,
        window: isize,
        _: &CancellationToken,
        _: Instant,
    ) -> Result<(), SendFailure> {
        assert_eq!(window, 456);
        Ok(())
    }
}

#[test]
fn cancellation_inside_keyboard_host_after_submission_remains_unknown() {
    let now = SendTime {
        instant: Instant::now(),
        unix_ms: 1000,
    };
    let request = HistorySendRequest {
        target: HistoryLaunchTarget {
            snapshot: identity(),
            own_process: 99,
            captured: now.instant,
            expires: now.instant + TARGET_LIFETIME,
        },
        ui_window: 456,
        selected: selection("1"),
        cancellation: CancellationToken::new(),
    };
    let host = CancelAfterSubmission {
        submitted: AtomicUsize::new(0),
    };
    let failure = request.execute(&host, || Ok(now)).unwrap_err();
    assert_eq!(failure.effect, InputEffect::Unknown);
    assert_eq!(failure.code, "input_result_unknown");
    assert_eq!(host.submitted.load(Ordering::Relaxed), 1);
}

#[test]
fn late_success_acknowledgement_preserves_a_newer_user_selection() {
    let mut state = state();
    let (id, selected) = state.begin_send_selection().unwrap();
    assert_eq!(selected.summary.id, "1");
    // 发送完成后回执尚在队列中，用户已经明确选择另一行。
    state.select("2");
    state.sent(id, Ok(SendReceipt { utf16_units: 2 }));
    assert_eq!(state.selected.as_deref(), Some("2"));
    assert!(state.can_query());
}

#[test]
fn returning_to_the_same_id_is_still_a_new_selection_after_a_completed_send() {
    let mut state = state();
    let (id, _) = state.begin_send_selection().unwrap();
    state.select("2");
    state.select("1");
    state.sent(id, Ok(SendReceipt { utf16_units: 2 }));
    assert_eq!(state.selected.as_deref(), Some("1"));
    assert!(state.can_query());
}

#[test]
fn expiry_after_a_known_success_does_not_turn_the_ack_into_no_input() {
    let now = SendTime::now().unwrap();
    let target = HistoryLaunchTarget {
        snapshot: identity(),
        own_process: 99,
        captured: now.instant,
        expires: now.instant + TARGET_LIFETIME,
    };
    let mut session = HistorySendSession::new(Some(target), Some(456));
    let mut state = state();
    let (id, selected) = state.begin_send_selection().unwrap();
    let request = session.begin(id, selected).unwrap();
    let completed = Ok(SendReceipt { utf16_units: 2 });
    // 模拟已完成的回执迟于GUI观察到期；不执行任何Host或输入。
    session.target.as_mut().unwrap().expires = now.instant;
    assert!(session.poll());
    assert!(request.cancellation.is_cancelled());
    session.finish(id);
    state.sent(id, completed);
    assert_eq!(state.message, Some("已向原窗口提交文本。"));
    assert!(state.selected.is_none());
    assert!(!session.ready());
    assert!(session.status().contains("过期"));
}
