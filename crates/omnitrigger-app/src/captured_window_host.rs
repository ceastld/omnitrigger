//! 显式窗口租约适配；复用既有参数解释和ForegroundTargets守卫，不改变默认A。
use super::{CapturedTargetRef, ForegroundLease, ForegroundTargets};
use omnitrigger_actions::{ActionError, CancellationToken};
use omnitrigger_windows::{ScriptTarget, WindowGeometryError, WindowOperation};
use serde_json::Value;
use std::time::Instant;

fn fault(method: &str, code: &str, effect: &str) -> ActionError {
    let mut error = ActionError::new(code, "显式窗口租约调用未获确认。");
    error.method = Some(method.into());
    error.effect_state = Some(effect.into());
    error
}
fn split(method: &str, mut params: Value) -> Result<(CapturedTargetRef, Value), ActionError> {
    let invalid = || fault(method, "invalid_input", "not_started");
    let object = params.as_object_mut().ok_or_else(invalid)?;
    let value = object.remove("target").ok_or_else(invalid)?;
    if !value.is_object() || !value.get("context").is_some_and(Value::is_object) {
        return Err(invalid());
    }
    let reference: CapturedTargetRef = serde_json::from_value(value).map_err(|_| invalid())?;
    if !(1..=128).contains(&reference.capture_id) {
        return Err(invalid());
    }
    Ok((reference, params))
}
fn resolve(
    targets: &ForegroundTargets,
    reference: CapturedTargetRef,
    token: &CancellationToken,
    deadline: Option<Instant>,
) -> Result<(ForegroundLease, ScriptTarget), ActionError> {
    let pair = targets.resolve(reference)?;
    targets.guard(&pair.0, &pair.1, token, deadline)?;
    Ok(pair)
}
fn geometry_error(error: ActionError) -> WindowGeometryError {
    let code = match error.code.as_str() {
        "cancelled" => "cancelled",
        "timeout" => "timeout",
        "target_expired" => "target_expired",
        "target_unavailable" => "target_unavailable",
        _ => "target_changed",
    };
    WindowGeometryError::new(code, "窗口租约已失效、被替代或不在前台。", "not_started")
}
fn guard(
    targets: &ForegroundTargets,
    lease: &ForegroundLease,
    target: &ScriptTarget,
    token: &CancellationToken,
    deadline: Option<Instant>,
    check: &mut dyn FnMut() -> Result<(), WindowGeometryError>,
) -> Result<(), WindowGeometryError> {
    check()?;
    targets
        .guard(lease, target, token, deadline)
        .map_err(geometry_error)?;
    check()
}
fn method_fault(method: &str, mut error: ActionError) -> ActionError {
    error.method = Some(method.into());
    error
}

pub(super) fn geometry(
    targets: &ForegroundTargets,
    method: &str,
    params: Value,
    token: &CancellationToken,
    deadline: Option<Instant>,
) -> Result<Value, ActionError> {
    let (reference, input) = split(method, params)?;
    super::window_geometry_host::execute_with(
        method,
        input,
        token,
        deadline,
        || resolve(targets, reference, token, deadline),
        |(lease, target), check| {
            let mut check = || guard(targets, &lease, &target, token, deadline, check);
            let result = target.window_geometry_if(&mut check);
            check()?;
            result
        },
        |(lease, target), rect, frame, check| {
            let mut check = || guard(targets, &lease, &target, token, deadline, check);
            let result = target.set_window_rect_if(rect, frame, &mut check);
            let effect = result
                .as_ref()
                .map_or_else(|e| e.effect_state, |_| "unknown");
            check().map_err(|mut e| {
                e.effect_state = effect;
                e
            })?;
            result
        },
    )
    .map_err(|error| method_fault(method, error))
}
pub(super) fn drag(
    targets: &ForegroundTargets,
    params: Value,
    token: &CancellationToken,
    deadline: Option<Instant>,
) -> Result<Value, ActionError> {
    let method = "window.drag";
    let (reference, input) = split(method, params)?;
    super::window_drag_host::execute_with(
        method,
        input,
        token,
        deadline,
        || resolve(targets, reference, token, deadline),
        |_, _| unreachable!("这里只处理window.drag"),
        |(lease, target), options, check| {
            let mut check = || guard(targets, &lease, &target, token, deadline, check);
            let result = omnitrigger_windows::track_window_drag_if(&target, options, &mut check);
            let effect = result
                .as_ref()
                .map_or_else(|e| e.effect_state, |result| result.effect_state);
            check().map_err(|mut e| {
                e.effect_state = effect;
                e
            })?;
            result
        },
    )
    .map_err(|error| method_fault(method, error))
}
pub(super) fn action(
    targets: &ForegroundTargets,
    params: Value,
    token: &CancellationToken,
    deadline: Option<Instant>,
) -> Result<Value, ActionError> {
    let method = "window.action";
    let (reference, input) = split(method, params)?;
    if !matches!(
        input.get("operation").and_then(Value::as_str),
        Some("restore" | "toggle_maximize")
    ) {
        return Err(fault(method, "invalid_input", "not_started"));
    }
    let mut rejected = None;
    let result = crate::window_action::execute_with(
        input,
        token,
        deadline,
        || resolve(targets, reference, token, deadline),
        |(lease, target), operation, check| {
            debug_assert!(matches!(
                operation,
                WindowOperation::Restore | WindowOperation::ToggleMaximize
            ));
            let result = {
                let mut before = || {
                    check()?;
                    targets
                        .guard(&lease, &target, token, deadline)
                        .map_err(|error| {
                            rejected = Some(method_fault(method, error));
                            "窗口租约在提交前失效。".to_owned()
                        })
                };
                target.window_action_if(operation, &mut before)
            };
            if result.is_ok() {
                if let Err(mut error) = targets.guard(&lease, &target, token, deadline) {
                    error.effect_state = Some("unknown".into());
                    rejected = Some(method_fault(method, error));
                    return Err("窗口租约在提交后失效。".into());
                }
            }
            result
        },
    );
    if let Some(error) = rejected {
        return Err(error);
    }
    result.map_err(|error| method_fault(method, error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn reference() -> Value {
        json!({"capture_id":1,"context":{"window_handle":"12","process_id":22,"thread_id":32,"process_created_file_time":"42","expires_at_unix_ms":1}})
    }
    #[test]
    fn target_shape_is_explicit_and_cannot_be_a_raw_handle_or_positional_array() {
        for target in [
            json!("12"),
            json!([1, {}]),
            json!({"hwnd":"12"}),
            json!({"capture_id":1,"context":[]}),
        ] {
            assert_eq!(
                split("window.geometry", json!({"target":target}))
                    .err()
                    .unwrap()
                    .code,
                "invalid_input"
            );
        }
    }
    #[test]
    fn parameter_validation_precedes_missing_lease_for_all_four_operations() {
        let targets = ForegroundTargets::default();
        let token = CancellationToken::new();
        let errors = [
            geometry(
                &targets,
                "window.geometry",
                json!({"target":reference(),"extra":true}),
                &token,
                None,
            ),
            geometry(
                &targets,
                "window.set_rect",
                json!({"target":reference(),"rect":[0,0,100,100],"frame":"outer"}),
                &token,
                None,
            ),
            drag(
                &targets,
                json!({"target":reference(),"mode":"move","button":"left","max_duration_ms":2001,"max_updates":1}),
                &token,
                None,
            ),
            action(
                &targets,
                json!({"target":reference(),"operation":"toggle_topmost"}),
                &token,
                None,
            ),
        ];
        for error in errors {
            assert_eq!(error.unwrap_err().code, "invalid_input");
        }
    }
    #[test]
    fn missing_lease_rejects_all_real_adapters_before_any_platform_identity_query() {
        let targets = ForegroundTargets::default();
        let token = CancellationToken::new();
        let cases = [
            geometry(
                &targets,
                "window.geometry",
                json!({"target":reference()}),
                &token,
                None,
            ),
            geometry(
                &targets,
                "window.set_rect",
                json!({"target":reference(),"rect":{"left":0,"top":0,"width":100,"height":100},"frame":"outer"}),
                &token,
                None,
            ),
            drag(
                &targets,
                json!({"target":reference(),"mode":"move","button":"left","max_duration_ms":100,"max_updates":1}),
                &token,
                None,
            ),
            action(
                &targets,
                json!({"target":reference(),"operation":"restore"}),
                &token,
                None,
            ),
        ];
        for error in cases {
            let error = error.unwrap_err();
            assert_eq!(error.code, "target_changed");
            assert_eq!(error.effect_state.as_deref(), Some("not_started"));
        }
        assert_eq!(targets.active.lock().unwrap().0, 0);
    }
    #[test]
    fn lease_fault_codes_remain_structured_without_hiding_expiry_or_cancellation() {
        for code in [
            "cancelled",
            "timeout",
            "target_expired",
            "target_changed",
            "target_unavailable",
        ] {
            let error = geometry_error(ActionError::new(code, "合成守卫"));
            assert_eq!(error.code, code);
            assert_eq!(error.effect_state, "not_started");
        }
    }

    #[test]
    fn toggle_uses_the_same_lease_resolution_and_strict_action_shape_as_restore() {
        let targets = ForegroundTargets::default();
        let token = CancellationToken::new();
        for operation in ["restore", "toggle_maximize"] {
            let error = action(
                &targets,
                json!({"target":reference(),"operation":operation}),
                &token,
                None,
            )
            .unwrap_err();
            assert_eq!(error.code, "target_changed");
            assert_eq!(error.method.as_deref(), Some("window.action"));
            assert_eq!(error.effect_state.as_deref(), Some("not_started"));
        }
        for input in [
            json!({"target":reference(),"operation":"toggle_maximize","extra":true}),
            json!({"target":reference(),"operation":"toggle_topmost"}),
            json!({"target":reference(),"operation":"close"}),
        ] {
            assert_eq!(
                action(&targets, input, &token, None).unwrap_err().code,
                "invalid_input"
            );
        }
        assert_eq!(targets.active.lock().unwrap().0, 0);
    }

    #[test]
    fn toggle_cancel_and_deadline_fail_before_missing_lease_resolution() {
        let targets = ForegroundTargets::default();
        let token = CancellationToken::new();
        let params = json!({"target":reference(),"operation":"toggle_maximize"});
        let timeout = action(&targets, params.clone(), &token, Some(Instant::now())).unwrap_err();
        assert_eq!(timeout.code, "timeout");
        assert_eq!(timeout.effect_state.as_deref(), Some("not_started"));
        token.cancel();
        for deadline in [None, Some(Instant::now())] {
            let error = action(&targets, params.clone(), &token, deadline).unwrap_err();
            assert_eq!(error.code, "cancelled");
            assert_eq!(error.effect_state.as_deref(), Some("not_started"));
        }
        assert_eq!(targets.active.lock().unwrap().0, 0);
    }

    #[test]
    fn drag_initial_pointer_shape_is_validated_before_explicit_lease_resolution() {
        let targets = ForegroundTargets::default();
        for point in [
            json!([0, 0]),
            json!(null),
            json!({"x":0,"y":0,"button":"down"}),
        ] {
            let error = drag(
                &targets,
                json!({"target":reference(),"mode":"move","button":"left","max_duration_ms":100,"max_updates":1,"initial_pointer":point}),
                &CancellationToken::new(), None,
            ).unwrap_err();
            assert_eq!(error.code, "invalid_input");
            assert_eq!(error.method.as_deref(), Some("window.drag"));
            assert_eq!(error.effect_state.as_deref(), Some("not_started"));
        }
        let error = drag(
            &targets,
            json!({"target":reference(),"mode":"move","button":"left","max_duration_ms":100,"max_updates":1,"initial_pointer":{"x":-700,"y":200}}),
            &CancellationToken::new(), None,
        ).unwrap_err();
        assert_eq!(error.code, "target_changed");
        assert_eq!(error.effect_state.as_deref(), Some("not_started"));
    }
}
