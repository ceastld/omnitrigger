//! 原目标发送的合成候选。尚无主页面原目标交接入口，仅编入测试，不接入输入执行。

use crate::clipboard_history_store::{preview, HistorySummary};
use omnitrigger_actions::{CancellationToken, NativeHost};
use omnitrigger_windows::ScriptTargetSnapshot;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

const TARGET_LIFETIME: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Effect {
    NotStarted,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SendError {
    code: &'static str,
    effect: Effect,
}

impl SendError {
    fn before(code: &'static str) -> Self {
        Self {
            code,
            effect: Effect::NotStarted,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct SendReceipt {
    submitted_utf16_units: usize,
}

struct HistorySendAttempt {
    target: ScriptTargetSnapshot,
    own_process: u32,
    captured: Instant,
    expires: Instant,
    selected: HistorySummary,
    revision: String,
    used: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextDto {
    window_handle: String,
    process_id: u32,
    thread_id: u32,
    process_created_file_time: String,
    expires_at_unix_ms: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ItemDto {
    id: String,
    text: String,
    created_at: String,
    revision: String,
}

fn decimal(value: &str, zero: bool) -> bool {
    !value.is_empty()
        && value.len() <= 20
        && (value.len() == 1 || !value.starts_with('0'))
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && value.parse::<u64>().is_ok_and(|number| zero || number != 0)
}

impl HistorySendAttempt {
    /// 只接受进入界面前由适配层明确传入的完整身份，不提供捕获当前前台的回退。
    fn new(
        target: Option<ScriptTargetSnapshot>,
        own_process: u32,
        captured: Instant,
        selected: HistorySummary,
        revision: String,
    ) -> Result<Self, SendError> {
        let target = target.ok_or_else(|| SendError::before("target_missing"))?;
        if own_process == 0
            || target.window_handle == 0
            || target.window_handle > isize::MAX as usize
            || target.process_id == 0
            || target.thread_id == 0
            || target.process_created_file_time == 0
        {
            return Err(SendError::before("target_invalid"));
        }
        if target.process_id == own_process {
            return Err(SendError::before("target_is_own_window"));
        }
        if !decimal(&selected.id, false)
            || !decimal(&revision, true)
            || !decimal(&selected.created_at, true)
            || selected.utf16_units > 65_536
        {
            return Err(SendError::before("selection_invalid"));
        }
        let expires = captured
            .checked_add(TARGET_LIFETIME)
            .ok_or_else(|| SendError::before("target_expired"))?;
        Ok(Self {
            target,
            own_process,
            captured,
            expires,
            selected,
            revision,
            used: false,
        })
    }

    fn check(&self, now: Instant, cancellation: &CancellationToken) -> Result<(), SendError> {
        if cancellation.is_cancelled() {
            return Err(SendError::before("cancelled"));
        }
        if now < self.captured || now >= self.expires {
            return Err(SendError::before("target_expired"));
        }
        Ok(())
    }

    fn check_host_target(
        &self,
        host: &impl NativeHost,
        cancellation: &CancellationToken,
    ) -> Result<(), SendError> {
        let value = host
            .call_with_deadline("target.context", json!({}), cancellation, self.expires)
            .map_err(|_| SendError::before("target_unavailable"))?;
        let value: ContextDto =
            serde_json::from_value(value).map_err(|_| SendError::before("target_invalid"))?;
        if value.process_id == self.own_process {
            return Err(SendError::before("target_is_own_window"));
        }
        if value.window_handle != self.target.window_handle.to_string()
            || value.process_id != self.target.process_id
            || value.thread_id != self.target.thread_id
            || value.process_created_file_time != self.target.process_created_file_time.to_string()
            || value.expires_at_unix_ms == 0
        {
            return Err(SendError::before("target_changed"));
        }
        Ok(())
    }

    /// 一次明确操作只尝试一次；Host须绑定同一快照并在keyboard.text内部复验前台。
    /// 此候选不激活窗口、不复制选区、不补选当前前台。失败不重试。
    fn submit(
        &mut self,
        host: &impl NativeHost,
        cancellation: &CancellationToken,
        mut clock: impl FnMut() -> Instant,
    ) -> Result<SendReceipt, SendError> {
        if self.used {
            return Err(SendError::before("attempt_already_used"));
        }
        self.used = true;
        self.check(clock(), cancellation)?;
        self.check_host_target(host, cancellation)?;
        self.check(clock(), cancellation)?;
        let value = host
            .call_with_deadline(
                "history.get",
                json!({"id":self.selected.id}),
                cancellation,
                self.expires,
            )
            .map_err(|_| SendError::before("history_unavailable"))?;
        let item: ItemDto = serde_json::from_value(value)
            .map_err(|_| SendError::before("history_invalid_result"))?;
        if item.id != self.selected.id
            || item.revision != self.revision
            || item.created_at != self.selected.created_at
            || item.text.encode_utf16().count() != self.selected.utf16_units
            || preview(&item.text) != self.selected.preview
        {
            return Err(SendError::before("history_selection_changed"));
        }
        if item.text.is_empty() || item.text.contains('\0') || self.selected.utf16_units > 65_536 {
            return Err(SendError::before("history_text_invalid"));
        }
        self.check(clock(), cancellation)?;
        self.check_host_target(host, cancellation)?;
        self.check(clock(), cancellation)?;
        let value = host
            .call_with_deadline(
                "keyboard.text",
                json!({"text":item.text}),
                cancellation,
                self.expires,
            )
            .map_err(|error| SendError {
                code: match error.code.as_str() {
                    "target_changed" => "target_changed",
                    "target_unavailable" => "target_unavailable",
                    "cancelled" => "cancelled",
                    "timeout" => "timeout",
                    _ => "input_failed",
                },
                effect: if error.effect_state.as_deref() == Some("not_started") {
                    Effect::NotStarted
                } else {
                    Effect::Unknown
                },
            })?;
        // 宿主未证实时限/取消或回执形状不正确，不能从返回值推断输入未发生。
        if self.check(clock(), cancellation).is_err()
            || value.as_object().is_none_or(|object| object.len() != 1)
            || value.get("submitted") != Some(&Value::Bool(true))
        {
            return Err(SendError {
                code: "input_result_unknown",
                effect: Effect::Unknown,
            });
        }
        Ok(SendReceipt {
            submitted_utf16_units: self.selected.utf16_units,
        })
    }
}

mod tests {
    use super::*;
    use omnitrigger_actions::ActionError;
    use std::sync::Mutex;

    struct FakeHost {
        calls: Mutex<Vec<String>>,
        context: Mutex<Value>,
        item: Mutex<Value>,
        keyboard_result: Mutex<Result<Value, ActionError>>,
        submitted: Mutex<Vec<String>>,
    }

    fn target() -> ScriptTargetSnapshot {
        ScriptTargetSnapshot {
            window_handle: 123,
            process_id: 7,
            thread_id: 8,
            process_created_file_time: 134_386_985_330_123_456,
        }
    }

    fn summary() -> HistorySummary {
        HistorySummary {
            id: "1".into(),
            preview: preview(" A😀\n"),
            utf16_units: 5,
            created_at: "10".into(),
        }
    }

    fn attempt(now: Instant) -> HistorySendAttempt {
        HistorySendAttempt::new(Some(target()), 99, now, summary(), "2".into()).unwrap()
    }

    fn host() -> FakeHost {
        let target = target();
        FakeHost {
            calls: Mutex::new(Vec::new()),
            context: Mutex::new(json!({"window_handle":target.window_handle.to_string(),
                "process_id":target.process_id,"thread_id":target.thread_id,
                "process_created_file_time":target.process_created_file_time.to_string(),"expires_at_unix_ms":100})),
            item: Mutex::new(json!({"id":"1","text":" A😀\n","createdAt":"10","revision":"2"})),
            keyboard_result: Mutex::new(Ok(json!({"submitted":true}))),
            submitted: Mutex::new(Vec::new()),
        }
    }

    impl NativeHost for FakeHost {
        fn call(
            &self,
            method: &str,
            params: Value,
            _: &CancellationToken,
        ) -> Result<Value, ActionError> {
            self.calls.lock().unwrap().push(method.into());
            match method {
                "target.context" => Ok(self.context.lock().unwrap().clone()),
                "history.get" => {
                    assert_eq!(params, json!({"id":"1"}));
                    Ok(self.item.lock().unwrap().clone())
                }
                "keyboard.text" => {
                    self.submitted
                        .lock()
                        .unwrap()
                        .push(params["text"].as_str().unwrap().into());
                    self.keyboard_result.lock().unwrap().clone()
                }
                _ => panic!("合成Host不支持该方法"),
            }
        }
    }

    #[test]
    fn missing_and_incomplete_target_have_no_foreground_fallback() {
        let now = Instant::now();
        assert_eq!(
            HistorySendAttempt::new(None, 99, now, summary(), "2".into())
                .err()
                .unwrap()
                .code,
            "target_missing"
        );
        for target in [
            ScriptTargetSnapshot {
                window_handle: 0,
                ..target()
            },
            ScriptTargetSnapshot {
                thread_id: 0,
                ..target()
            },
            ScriptTargetSnapshot {
                process_created_file_time: 0,
                ..target()
            },
        ] {
            assert_eq!(
                HistorySendAttempt::new(Some(target), 99, now, summary(), "2".into())
                    .err()
                    .unwrap()
                    .code,
                "target_invalid"
            );
        }
    }

    #[test]
    fn every_window_in_own_process_is_rejected() {
        assert_eq!(
            HistorySendAttempt::new(Some(target()), 7, Instant::now(), summary(), "2".into())
                .err()
                .unwrap()
                .code,
            "target_is_own_window"
        );
    }

    #[test]
    fn invalid_selection_identity_or_revision_is_rejected_before_host() {
        let now = Instant::now();
        for revision in ["", "02", "18446744073709551616"] {
            assert_eq!(
                HistorySendAttempt::new(Some(target()), 99, now, summary(), revision.into())
                    .err()
                    .unwrap()
                    .code,
                "selection_invalid"
            );
        }
        let selected = HistorySummary {
            id: "0".into(),
            ..summary()
        };
        assert!(HistorySendAttempt::new(Some(target()), 99, now, selected, "2".into()).is_err());
    }

    #[test]
    fn expiration_and_cancellation_happen_before_any_host_call() {
        let now = Instant::now();
        for cancelled in [false, true] {
            let host = host();
            let token = CancellationToken::new();
            if cancelled {
                token.cancel();
            }
            let result = attempt(now).submit(&host, &token, || now + TARGET_LIFETIME);
            assert_eq!(result.unwrap_err().effect, Effect::NotStarted);
            assert!(host.calls.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn changed_or_own_host_identity_cannot_read_history_or_send() {
        let now = Instant::now();
        for pid in [9, 99] {
            let host = host();
            host.context.lock().unwrap()["process_id"] = json!(pid);
            assert!(attempt(now)
                .submit(&host, &CancellationToken::new(), || now)
                .is_err());
            assert_eq!(*host.calls.lock().unwrap(), ["target.context"]);
        }
    }

    #[test]
    fn changed_item_or_revision_never_reaches_keyboard() {
        let now = Instant::now();
        for (field, value) in [
            ("id", json!("2")),
            ("revision", json!("3")),
            ("createdAt", json!("11")),
            ("text", json!("changed")),
            ("text", json!(" B😀\n")),
        ] {
            let host = host();
            host.item.lock().unwrap()[field] = value;
            assert_eq!(
                attempt(now)
                    .submit(&host, &CancellationToken::new(), || now)
                    .unwrap_err()
                    .effect,
                Effect::NotStarted
            );
            assert!(host.submitted.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn empty_nul_or_extra_body_fields_never_reach_keyboard() {
        let now = Instant::now();
        for mode in ["empty", "nul", "extra"] {
            let host = host();
            let mut attempt = attempt(now);
            match mode {
                "empty" => {
                    attempt.selected.utf16_units = 0;
                    attempt.selected.preview.clear();
                    host.item.lock().unwrap()["text"] = json!("");
                }
                "nul" => {
                    attempt.selected.preview = preview(" A\0 \n");
                    host.item.lock().unwrap()["text"] = json!(" A\0 \n");
                }
                _ => host.item.lock().unwrap()["private"] = json!("合成正文"),
            }
            let error = attempt
                .submit(&host, &CancellationToken::new(), || now)
                .unwrap_err();
            assert_eq!(error.effect, Effect::NotStarted);
            assert!(!format!("{error:?}").contains("合成正文"));
            assert!(host.submitted.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn expired_during_history_read_never_reaches_keyboard() {
        let now = Instant::now();
        let host = host();
        let mut ticks = [now, now, now + TARGET_LIFETIME].into_iter();
        let error = attempt(now)
            .submit(&host, &CancellationToken::new(), || ticks.next().unwrap())
            .unwrap_err();
        assert_eq!(error.code, "target_expired");
        assert!(host.submitted.lock().unwrap().is_empty());
    }

    #[test]
    fn target_replaced_during_history_read_is_checked_again() {
        let now = Instant::now();
        let host = host();
        let mut tick = 0;
        let error = attempt(now)
            .submit(&host, &CancellationToken::new(), || {
                tick += 1;
                if tick == 3 {
                    host.context.lock().unwrap()["process_created_file_time"] =
                        json!("134386985330123457");
                }
                now
            })
            .unwrap_err();
        assert_eq!(error.code, "target_changed");
        assert_eq!(error.effect, Effect::NotStarted);
        assert!(host.submitted.lock().unwrap().is_empty());
    }

    #[test]
    fn cancelled_during_history_read_is_not_submitted() {
        let now = Instant::now();
        let host = host();
        let token = CancellationToken::new();
        let mut tick = 0;
        let error = attempt(now)
            .submit(&host, &token, || {
                tick += 1;
                if tick == 3 {
                    token.cancel();
                }
                now
            })
            .unwrap_err();
        assert_eq!(error.code, "cancelled");
        assert!(host.submitted.lock().unwrap().is_empty());
    }

    #[test]
    fn synthetic_submission_preserves_text_and_is_one_shot() {
        let now = Instant::now();
        let host = host();
        let mut attempt = attempt(now);
        assert_eq!(
            attempt
                .submit(&host, &CancellationToken::new(), || now)
                .unwrap()
                .submitted_utf16_units,
            5
        );
        assert_eq!(
            *host.calls.lock().unwrap(),
            [
                "target.context",
                "history.get",
                "target.context",
                "keyboard.text"
            ]
        );
        assert_eq!(*host.submitted.lock().unwrap(), [" A😀\n"]);
        assert_eq!(
            attempt
                .submit(&host, &CancellationToken::new(), || now)
                .unwrap_err()
                .code,
            "attempt_already_used"
        );
        assert_eq!(host.submitted.lock().unwrap().len(), 1);
    }

    #[test]
    fn foreground_refusal_stays_not_started_and_is_never_retried() {
        let now = Instant::now();
        let host = host();
        let mut error = ActionError::new("target_changed", "合成拒绝");
        error.effect_state = Some("not_started".into());
        *host.keyboard_result.lock().unwrap() = Err(error);
        let mut attempt = attempt(now);
        let error = attempt
            .submit(&host, &CancellationToken::new(), || now)
            .unwrap_err();
        assert_eq!(error.effect, Effect::NotStarted);
        assert_eq!(error.code, "target_changed");
        assert!(attempt
            .submit(&host, &CancellationToken::new(), || now)
            .is_err());
        assert_eq!(host.submitted.lock().unwrap().len(), 1);
    }

    #[test]
    fn input_error_and_malformed_acknowledgement_preserve_unknown() {
        let now = Instant::now();
        for result in [
            Err(ActionError::new("private_details", "合成正文不应进入错误")),
            Ok(json!({"submitted":true,"text":"合成正文"})),
            Ok(json!({"submitted":false})),
        ] {
            let host = host();
            *host.keyboard_result.lock().unwrap() = result;
            let error = attempt(now)
                .submit(&host, &CancellationToken::new(), || now)
                .unwrap_err();
            assert_eq!(error.effect, Effect::Unknown);
            assert!(!format!("{error:?}").contains("合成正文"));
        }
    }

    #[test]
    fn expiry_after_keyboard_acknowledgement_cannot_claim_no_effect() {
        let now = Instant::now();
        let host = host();
        let mut ticks = [now, now, now, now, now + TARGET_LIFETIME].into_iter();
        let error = attempt(now)
            .submit(&host, &CancellationToken::new(), || ticks.next().unwrap())
            .unwrap_err();
        assert_eq!(error.effect, Effect::Unknown);
        assert_eq!(host.submitted.lock().unwrap().len(), 1);
    }

    #[test]
    fn cancellation_after_keyboard_acknowledgement_preserves_unknown() {
        let now = Instant::now();
        let host = host();
        let token = CancellationToken::new();
        let mut tick = 0;
        let error = attempt(now)
            .submit(&host, &token, || {
                tick += 1;
                if tick == 5 {
                    token.cancel();
                }
                now
            })
            .unwrap_err();
        assert_eq!(error.effect, Effect::Unknown);
        assert_eq!(host.submitted.lock().unwrap().len(), 1);
    }
}
