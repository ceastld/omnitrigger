//! 原目标有界采样/调整入口；单次最多2秒worker循环，不进入Hook同步路径。
use omnitrigger_actions::{ActionError, CancellationToken};
use omnitrigger_windows::{
    WindowDragButton, WindowDragMode, WindowDragOptions, WindowDragStopped, WindowGeometryError,
    WindowPointerState, WindowRect, WindowSnapOptions,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Move,
    Resize,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Button {
    Left,
    Right,
    Middle,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rect {
    left: i32,
    top: i32,
    width: i32,
    height: i32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Snap {
    bounds: Rect,
    distance: u32,
    wait_ms: u32,
    initial_elapsed_ms: u32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InitialPointer {
    x: i32,
    y: i32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Drag {
    mode: Mode,
    button: Button,
    max_duration_ms: u32,
    max_updates: u32,
    snap: Option<Snap>,
    initial_pointer: Option<InitialPointer>,
}
fn fault(method: &str, error: WindowGeometryError) -> ActionError {
    let mut value = ActionError::new(error.code, error.message);
    value.method = Some(method.into());
    value.effect_state = Some(error.effect_state.into());
    value
}
fn check(
    token: &CancellationToken,
    deadline: Instant,
    effect: &'static str,
) -> Result<(), WindowGeometryError> {
    if token.is_cancelled() {
        return Err(WindowGeometryError::new(
            "cancelled",
            "指针/窗口调整已取消。",
            effect,
        ));
    }
    if Instant::now() >= deadline {
        return Err(WindowGeometryError::new(
            "timeout",
            "指针/窗口调整超过动作期限。",
            effect,
        ));
    }
    Ok(())
}
fn pointer_json(value: WindowPointerState) -> Value {
    json!({"coordinateSpace":"physical-screen","x":value.x,"y":value.y,"buttons":{"left":value.left.as_str(),"right":value.right.as_str(),"middle":value.middle.as_str()},"inputSource":{"kind":"polled-key-state","injected":null,"selfInjected":null}})
}
fn stopped_json(value: WindowDragStopped) -> Value {
    json!({"stopped":true,"reason":value.reason.as_str(),"acceptedUpdates":value.accepted_updates,"pollSamples":value.poll_samples,"effectState":value.effect_state,"dragCompleted":false})
}
pub(super) fn execute_with<T>(
    method: &str,
    params: Value,
    token: &CancellationToken,
    deadline: Option<Instant>,
    resolve: impl FnOnce() -> Result<T, ActionError>,
    sample: impl FnOnce(
        T,
        &mut dyn FnMut() -> Result<(), WindowGeometryError>,
    ) -> Result<WindowPointerState, WindowGeometryError>,
    track: impl FnOnce(
        T,
        WindowDragOptions,
        &mut dyn FnMut() -> Result<(), WindowGeometryError>,
    ) -> Result<WindowDragStopped, WindowGeometryError>,
) -> Result<Value, ActionError> {
    let limit = Instant::now() + Duration::from_secs(5);
    let deadline = deadline.map_or(limit, |d| d.min(limit));
    // 同一raw-wire对象合同；snap的null/省略行为不变，显式initial_pointer必须为对象。
    if !params.is_object()
        || (method == "window.drag"
            && (params.get("snap").is_some_and(|snap| {
                !snap.is_null()
                    && (!snap.is_object() || !snap.get("bounds").is_some_and(Value::is_object))
            }) || params
                .get("initial_pointer")
                .is_some_and(|point| !point.is_object())))
    {
        return Err(fault(
            method,
            WindowGeometryError::new(
                "invalid_input",
                "指针/调整参数、非空snap/bounds及显式initial_pointer必须为JSON对象。",
                "not_started",
            ),
        ));
    }
    let parse = |error: serde_json::Error| {
        fault(
            method,
            WindowGeometryError::new("invalid_input", error.to_string(), "not_started"),
        )
    };
    let options = match method {
        "pointer.state" => {
            serde_json::from_value::<Empty>(params).map_err(parse)?;
            None
        }
        "window.drag" => {
            let args: Drag = serde_json::from_value(params).map_err(parse)?;
            Some(
                WindowDragOptions {
                    mode: match args.mode {
                        Mode::Move => WindowDragMode::Move,
                        Mode::Resize => WindowDragMode::Resize,
                    },
                    button: match args.button {
                        Button::Left => WindowDragButton::Left,
                        Button::Right => WindowDragButton::Right,
                        Button::Middle => WindowDragButton::Middle,
                    },
                    max_duration_ms: args.max_duration_ms,
                    max_updates: args.max_updates,
                    initial_pointer: args.initial_pointer.map(|point| (point.x, point.y)),
                    snap: args.snap.map(|s| WindowSnapOptions {
                        bounds: WindowRect {
                            left: s.bounds.left,
                            top: s.bounds.top,
                            width: s.bounds.width,
                            height: s.bounds.height,
                        },
                        distance: s.distance,
                        wait_ms: s.wait_ms,
                        initial_elapsed_ms: s.initial_elapsed_ms,
                    }),
                }
                .validate()
                .map_err(|e| fault(method, e))?,
            )
        }
        _ => {
            return Err(fault(
                method,
                WindowGeometryError::new(
                    "unknown_method",
                    "未提供该指针/调整方法。",
                    "not_started",
                ),
            ))
        }
    };
    check(token, deadline, "not_started").map_err(|e| fault(method, e))?;
    let target = resolve().map_err(|mut e| {
        e.method = Some(method.into());
        e
    })?;
    check(token, deadline, "not_started").map_err(|e| fault(method, e))?;
    let mut guard = || check(token, deadline, "not_started");
    let result = match options {
        None => sample(target, &mut guard).map(pointer_json),
        Some(options) => track(target, options, &mut guard).map(stopped_json),
    };
    let effect = match &result {
        Ok(v) if options.is_some() && v["effectState"] == "unknown" => "unknown",
        Err(e) => e.effect_state,
        _ => "not_started",
    };
    check(token, deadline, effect).map_err(|e| fault(method, e))?;
    result.map_err(|e| fault(method, e))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_budget_or_incomplete_snap_stops_before_original_target_resolution() {
        for params in [
            json!({}),
            json!({"mode":"move","button":"left","max_duration_ms":2001,"max_updates":1}),
            json!({"mode":"move","button":"left","max_duration_ms":100,"max_updates":1,"snap":{"bounds":{"left":0,"top":0,"width":1000,"height":800}}}),
        ] {
            let error = execute_with(
                "window.drag",
                params,
                &CancellationToken::new(),
                None,
                || -> Result<(), ActionError> { panic!() },
                |_, _| panic!(),
                |_, _, _| panic!(),
            )
            .unwrap_err();
            assert_eq!(error.code, "invalid_input");
            assert_eq!(error.effect_state.as_deref(), Some("not_started"));
        }
    }
    #[test]
    fn cancellation_after_accepted_update_is_unknown_and_never_rolled_back() {
        let token = CancellationToken::new();
        let error = execute_with(
            "window.drag",
            json!({"mode":"move","button":"left","max_duration_ms":100,"max_updates":1}),
            &token,
            None,
            || Ok(()),
            |_, _| panic!(),
            |_, _, guard| {
                guard()?;
                token.cancel();
                Ok(WindowDragStopped {
                    reason: omnitrigger_windows::WindowDragStop::UpdateLimit,
                    accepted_updates: 1,
                    poll_samples: 2,
                    effect_state: "unknown",
                })
            },
        )
        .unwrap_err();
        assert_eq!(error.code, "cancelled");
        assert_eq!(error.effect_state.as_deref(), Some("unknown"));
    }

    #[test]
    fn initial_pointer_is_optional_and_arrives_as_only_i32_coordinates() {
        for point in [None, Some((i32::MIN, i32::MAX)), Some((-700, 200))] {
            let mut params =
                json!({"mode":"move","button":"left","max_duration_ms":100,"max_updates":1});
            if let Some((x, y)) = point {
                params["initial_pointer"] = json!({"x":x,"y":y});
            }
            let result = execute_with(
                "window.drag",
                params,
                &CancellationToken::new(),
                None,
                || Ok("same-existing-target"),
                |_, _| panic!("不使用独立pointer.state分支"),
                |target, options, guard| {
                    assert_eq!(target, "same-existing-target");
                    assert_eq!(options.initial_pointer, point);
                    guard()?;
                    Ok(WindowDragStopped {
                        reason: omnitrigger_windows::WindowDragStop::ButtonNotDownOrUnavailable,
                        accepted_updates: 0,
                        poll_samples: 1,
                        effect_state: "not_started",
                    })
                },
            )
            .unwrap();
            assert_eq!(result["reason"], "button_not_down_or_unavailable");
            assert_eq!(result["effectState"], "not_started");
        }
    }

    #[test]
    fn malformed_initial_pointer_rejects_before_target_resolution_or_platform_calls() {
        for point in [
            json!(null),
            json!([0, 0]),
            json!({}),
            json!({"x":0}),
            json!({"x":0,"y":0,"left":"down"}),
            json!({"x":2147483648i64,"y":0}),
            json!({"x":0,"y":-2147483649i64}),
            json!({"x":0.5,"y":0}),
            json!({"x":"0","y":0}),
        ] {
            let error = execute_with(
                "window.drag",
                json!({"mode":"move","button":"left","max_duration_ms":100,"max_updates":1,"initial_pointer":point}),
                &CancellationToken::new(), None,
                || -> Result<(), ActionError> { panic!("无效锚点不得解析目标") },
                |_, _| panic!(), |_, _, _| panic!(),
            ).unwrap_err();
            assert_eq!(error.code, "invalid_input");
            assert_eq!(error.effect_state.as_deref(), Some("not_started"));
        }
    }

    #[test]
    fn initial_pointer_never_bypasses_cancel_deadline_or_existing_target_error() {
        let params = json!({"mode":"move","button":"left","max_duration_ms":100,"max_updates":1,"initial_pointer":{"x":-700,"y":200}});
        for cancel in [false, true] {
            let token = CancellationToken::new();
            if cancel {
                token.cancel();
            }
            let error = execute_with(
                "window.drag",
                params.clone(),
                &token,
                Some(Instant::now()),
                || -> Result<(), ActionError> { panic!("先检查取消/期限") },
                |_, _| panic!(),
                |_, _, _| panic!(),
            )
            .unwrap_err();
            assert_eq!(error.code, if cancel { "cancelled" } else { "timeout" });
            assert_eq!(error.effect_state.as_deref(), Some("not_started"));
        }
        let error = execute_with(
            "window.drag",
            params,
            &CancellationToken::new(),
            None,
            || -> Result<(), ActionError> {
                let mut error = ActionError::new("target_expired", "既有租约拒绝");
                error.effect_state = Some("not_started".into());
                Err(error)
            },
            |_, _| panic!(),
            |_, _, _| panic!(),
        )
        .unwrap_err();
        assert_eq!(error.code, "target_expired");
        assert_eq!(error.method.as_deref(), Some("window.drag"));
        assert_eq!(error.effect_state.as_deref(), Some("not_started"));
    }
}
