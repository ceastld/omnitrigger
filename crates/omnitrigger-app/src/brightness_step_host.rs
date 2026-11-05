//! 亮度 wire 的唯一 App 参数解析与匿名回执映射；复用现有平台实现。
use crate::adjustment_guard::AdjustmentControl;
use omnitrigger_actions::{ActionError, CancellationToken};
use omnitrigger_windows::{
    BrightnessStepDirection, FirstWmiBrightnessResult, PhysicalBrightnessSetResult,
    PhysicalBrightnessStatus, SystemAdjustmentError, SystemAdjustmentErrorCode,
    WmiBrightnessStepSelection,
};
use serde::Deserialize;
use serde_json::{json, Value};
#[cfg(test)]
use std::cell::Cell;
use std::time::Instant;
const METHOD: &str = "display.brightness_step";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    direction: String,
    strategy: String,
}
fn fault(code: &str, effect: &str) -> ActionError {
    let mut value = ActionError::new(code, "亮度步进未获完整确认；请检查结构化回执。");
    value.method = Some(METHOD.into());
    value.effect_state = Some(effect.into());
    value
}
fn code(error: &SystemAdjustmentError) -> &'static str {
    match error.code {
        SystemAdjustmentErrorCode::Initialization => "initialization_failed",
        SystemAdjustmentErrorCode::MonitorUnavailable => "monitor_unavailable",
        SystemAdjustmentErrorCode::AmbiguousMonitor => "ambiguous_monitor",
        SystemAdjustmentErrorCode::UnsupportedBrightness => "brightness_unavailable",
        SystemAdjustmentErrorCode::ConfirmationUnknown => "brightness_unknown",
        _ => "brightness_failed",
    }
}
fn physical(value: Result<PhysicalBrightnessSetResult, SystemAdjustmentError>) -> Value {
    let value = match value {
        Ok(value) => value,
        Err(error) => return json!({"members":[],"errorCode":code(&error)}),
    };
    let members:Vec<_>=value.members.into_iter().map(|member|{
        use PhysicalBrightnessStatus::*;
        let(status,previous,requested,current,error)=match member.status {
            ReadRejected{error}=>("read_rejected",None,None,None,Some(code(&error))),
            Unchanged{reading}=>("unchanged",Some(reading.current),Some(reading.current),Some(reading.current),None),
            SetUnconfirmed{before,requested,error}=>("set_unconfirmed",Some(before.current),Some(requested),None,Some(code(&error))),
            ReadbackUnknown{before,requested,error}=>("readback_unknown",Some(before.current),Some(requested),None,Some(code(&error))),
            ReadbackDifferent{before,requested,observed}=>("readback_different",Some(before.current),Some(requested),Some(observed.current),None),
            Confirmed{before,requested,observed}=>("confirmed",Some(before.current),Some(requested),Some(observed.current),None),
        };
        json!({"index":member.index,"status":status,"previousRaw":previous,"requestedRaw":requested,"currentRaw":current,"errorCode":error})
    }).collect();
    json!({"members":members,"errorCode":null})
}
fn wmi(value: Result<FirstWmiBrightnessResult, SystemAdjustmentError>) -> Value {
    let value = match value {
        Ok(value) => value,
        Err(error) => {
            return json!({"status":"failed","previousPercent":null,"requestedPercent":null,"currentPercent":null,"errorCode":code(&error)})
        }
    };
    let (status, requested) = match value.step.selection {
        WmiBrightnessStepSelection::Set { requested } => ("set", Some(requested)),
        WmiBrightnessStepSelection::SkipZero => ("zero_skipped", None),
        WmiBrightnessStepSelection::NoEligibleLevel => ("no_eligible_level", None),
    };
    json!({"status":status,"previousPercent":value.step.adjustment.previous_percent,"requestedPercent":requested,
        "currentPercent":value.step.adjustment.current_percent,"errorCode":null})
}
type Physical = Option<Result<PhysicalBrightnessSetResult, SystemAdjustmentError>>;
type Wmi = Option<Result<FirstWmiBrightnessResult, SystemAdjustmentError>>;
fn execute_with(
    params: Value,
    cancellation: &CancellationToken,
    deadline: Option<Instant>,
    capture: impl FnOnce() -> Result<(i32, i32), SystemAdjustmentError>,
    invoke: impl FnOnce(
        BrightnessStepDirection,
        &str,
        Option<(i32, i32)>,
        &mut dyn FnMut(bool) -> Result<(), SystemAdjustmentError>,
    ) -> (Physical, Wmi),
) -> Result<Value, ActionError> {
    execute_with_clock(
        params,
        cancellation,
        deadline,
        capture,
        invoke,
        Instant::now,
    )
}
fn execute_with_clock(
    params: Value,
    cancellation: &CancellationToken,
    deadline: Option<Instant>,
    capture: impl FnOnce() -> Result<(i32, i32), SystemAdjustmentError>,
    invoke: impl FnOnce(
        BrightnessStepDirection,
        &str,
        Option<(i32, i32)>,
        &mut dyn FnMut(bool) -> Result<(), SystemAdjustmentError>,
    ) -> (Physical, Wmi),
    now: impl Fn() -> Instant,
) -> Result<Value, ActionError> {
    if !params.is_object() {
        return Err(fault("invalid_input", "not_started"));
    }
    let args: Args =
        serde_json::from_value(params).map_err(|_| fault("invalid_input", "not_started"))?;
    let direction = match args.direction.as_str() {
        "up" => BrightnessStepDirection::Increase,
        "down" => BrightnessStepDirection::Decrease,
        _ => return Err(fault("invalid_input", "not_started")),
    };
    if !["physical", "wmi_first", "physical_then_wmi"].contains(&args.strategy.as_str()) {
        return Err(fault("invalid_input", "not_started"));
    }
    let control = AdjustmentControl::new(cancellation, deadline, &now);
    let mut check = |write| control.check(write);
    let effect = || control.effect();
    check(false).map_err(|_| fault(control.reason().unwrap_or("brightness_failed"), effect()))?;
    let captured = if args.strategy == "wmi_first" {
        Ok(None)
    } else {
        capture().map(Some)
    };
    check(false).map_err(|_| fault(control.reason().unwrap_or("brightness_failed"), effect()))?;
    let point = captured.map_err(|error| fault(code(&error), effect()))?;
    let (physical_result, wmi_result) = invoke(direction, &args.strategy, point, &mut check);
    let _ = check(false);
    if let Some(reason) = control.reason() {
        return Err(fault(reason, effect()));
    }
    Ok(
        json!({"strategy":args.strategy,"effectState":effect(),"physical":physical_result.map(physical),"wmi":wmi_result.map(wmi)}),
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
        omnitrigger_windows::capture_adjustment_point,
        |direction, strategy, point, check| {
            use omnitrigger_windows::*;
            let wmi_direction = match direction {
                BrightnessStepDirection::Increase => WmiBrightnessStepDirection::Increase,
                BrightnessStepDirection::Decrease => WmiBrightnessStepDirection::Decrease,
            };
            match (strategy, point) {
                ("physical", Some(point)) => (
                    Some(step_physical_brightness_set_at_if(point, direction, check)),
                    None,
                ),
                ("wmi_first", None) => (
                    None,
                    Some(step_first_enumerated_wmi_brightness_if(
                        wmi_direction,
                        check,
                    )),
                ),
                ("physical_then_wmi", Some(point)) => {
                    match step_brightness_with_first_wmi_fallback_at_if(point, direction, check) {
                        Ok(BrightnessStepOutcome::Physical(value)) => (Some(value), None),
                        Ok(BrightnessStepOutcome::WmiFallback { physical, wmi }) => {
                            (Some(physical), Some(wmi))
                        }
                        Err(error) => (Some(Err(error)), None),
                    }
                }
                _ => (
                    Some(Err(SystemAdjustmentError {
                        code: SystemAdjustmentErrorCode::InvalidDelta,
                        message: "亮度策略上下文无效。".into(),
                    })),
                    None,
                ),
            }
        },
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    include!("brightness_step_host_tests.rs");
}
