//! 音量脚本API的唯一wire解析；复用既有CoreAudio算法和共同守卫。
use crate::adjustment_guard::AdjustmentControl;
use omnitrigger_actions::{ActionError, CancellationToken};
use omnitrigger_windows::{
    AdjustmentResult, SystemAdjustment, SystemAdjustmentError, SystemAdjustmentErrorCode,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Instant;
const METHOD: &str = "audio.volume_adjust";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    delta_percent: i16,
}
fn fault(code: &str, effect: &str) -> ActionError {
    let mut error = ActionError::new(code, "音量调节未获完整确认；请检查错误代码和效果状态。");
    error.method = Some(METHOD.into());
    error.effect_state = Some(effect.into());
    error
}
fn code(error: &SystemAdjustmentError) -> &'static str {
    match error.code {
        SystemAdjustmentErrorCode::InvalidDelta => "invalid_input",
        SystemAdjustmentErrorCode::Initialization => "initialization_failed",
        SystemAdjustmentErrorCode::AudioUnavailable => "audio_unavailable",
        SystemAdjustmentErrorCode::ConfirmationUnknown => "audio_unknown",
        _ => "audio_failed",
    }
}
fn execute_with(
    params: Value,
    cancellation: &CancellationToken,
    deadline: Option<Instant>,
    invoke: impl FnOnce(
        i16,
        &mut dyn FnMut(bool) -> Result<(), SystemAdjustmentError>,
    ) -> Result<AdjustmentResult, SystemAdjustmentError>,
    now: impl Fn() -> Instant,
) -> Result<Value, ActionError> {
    if !params.is_object() {
        return Err(fault("invalid_input", "not_started"));
    }
    let args: Args =
        serde_json::from_value(params).map_err(|_| fault("invalid_input", "not_started"))?;
    omnitrigger_windows::validate_system_adjustment(SystemAdjustment::Volume {
        delta_percent: args.delta_percent,
    })
    .map_err(|_| fault("invalid_input", "not_started"))?;
    let guard = AdjustmentControl::new(cancellation, deadline, &now);
    let mut check = |write| guard.check(write);
    check(false).map_err(|_| fault(guard.reason().unwrap_or("audio_failed"), guard.effect()))?;
    let result = invoke(args.delta_percent, &mut check);
    let _ = check(false);
    if let Some(reason) = guard.reason() {
        return Err(fault(reason, guard.effect()));
    }
    let value = result.map_err(|error| fault(code(&error), guard.effect()))?;
    Ok(
        json!({"effectState":guard.effect(),"previousPercent":value.previous_percent,
        "requestedPercent":value.requested_percent,"currentPercent":value.current_percent}),
    )
}
pub(super) fn execute(
    params: Value,
    cancellation: &CancellationToken,
    deadline: Option<Instant>,
) -> Result<Value, ActionError> {
    execute_with(
        params,
        cancellation,
        deadline,
        omnitrigger_windows::adjust_volume_if,
        Instant::now,
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_parameters_and_prestart_cancel_never_enter_audio() {
        for value in [
            json!([]),
            json!({}),
            json!({"delta_percent":0}),
            json!({"delta_percent":101}),
            json!({"delta_percent":-101}),
            json!({"delta_percent":1.5}),
            json!({"delta_percent":null}),
            json!({"delta_percent":true}),
            json!({"delta_percent":5,"device":"synthetic"}),
        ] {
            let error = execute_with(
                value,
                &CancellationToken::new(),
                None,
                |_, _| panic!("不得访问设备"),
                Instant::now,
            )
            .unwrap_err();
            assert_eq!(error.code, "invalid_input");
            assert_eq!(error.effect_state.as_deref(), Some("not_started"));
        }
        let token = CancellationToken::new();
        token.cancel();
        let error = execute_with(
            json!({"delta_percent":5}),
            &token,
            None,
            |_, _| panic!("不得执行"),
            Instant::now,
        )
        .unwrap_err();
        assert_eq!(error.code, "cancelled");
        assert_eq!(error.effect_state.as_deref(), Some("not_started"));
    }
    #[test]
    fn postwrite_timeout_is_unknown_and_native_errors_never_expose_device_text() {
        use std::cell::Cell;
        let start = Instant::now();
        let now = Cell::new(start);
        let deadline = start + std::time::Duration::from_millis(1);
        let error = execute_with(
            json!({"delta_percent":-5}),
            &CancellationToken::new(),
            Some(deadline),
            |delta, check| {
                assert_eq!(delta, -5);
                check(true).unwrap();
                now.set(deadline);
                Err(SystemAdjustmentError {
                    code: SystemAdjustmentErrorCode::NativeFailure,
                    message: "SYNTHETIC_PRIVATE_DEVICE".into(),
                })
            },
            || now.get(),
        )
        .unwrap_err();
        assert_eq!(error.code, "timeout");
        assert_eq!(error.effect_state.as_deref(), Some("unknown"));
        assert!(!error.message.contains("PRIVATE"));
    }
    #[test]
    fn success_keeps_actual_readback_without_claiming_exact_target_and_omits_labels() {
        let value = execute_with(
            json!({"delta_percent":5}),
            &CancellationToken::new(),
            None,
            |_, check| {
                check(false)?;
                check(true)?;
                Ok(AdjustmentResult {
                    kind: omnitrigger_windows::AdjustmentKind::Volume,
                    previous_percent: 40,
                    requested_percent: 45,
                    current_percent: 44,
                    target_label: "SYNTHETIC_PRIVATE_DEVICE".into(),
                    monitor_device: None,
                    brightness_method: None,
                })
            },
            Instant::now,
        )
        .unwrap();
        assert_eq!(
            value,
            json!({"effectState":"unknown","previousPercent":40,"requestedPercent":45,"currentPercent":44})
        );
    }
}
