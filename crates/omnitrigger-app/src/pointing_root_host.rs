//! 同一DesktopHost的只读调用时观察；不创建输入租约、不修改本次原目标。
use omnitrigger_actions::{ActionError, CancellationToken};
use omnitrigger_windows::{PointingRootObservation, ScriptTargetSnapshot, WindowGeometryError};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

fn fault(code: &str, message: impl ToString) -> ActionError {
    let mut value = ActionError::new(code, message.to_string());
    value.method = Some("pointer.root".into());
    value.effect_state = Some("not_started".into());
    value
}
fn check(token: &CancellationToken, deadline: Instant) -> Result<(), WindowGeometryError> {
    if token.is_cancelled() {
        return Err(WindowGeometryError::new(
            "cancelled",
            "指向根窗查询已取消。",
            "not_started",
        ));
    }
    if Instant::now() >= deadline {
        return Err(WindowGeometryError::new(
            "timeout",
            "指向根窗查询超过期限。",
            "not_started",
        ));
    }
    Ok(())
}
fn identity(value: ScriptTargetSnapshot) -> Value {
    json!({"window_handle":value.window_handle.to_string(),"process_id":value.process_id,
        "thread_id":value.thread_id,"process_created_file_time":value.process_created_file_time.to_string()})
}
fn export(value: PointingRootObservation) -> Value {
    json!({"sampledAt":"call-time","coordinateSpace":"physical-screen","point":{"x":value.x,"y":value.y},
        "inputSource":{"kind":"polled-pointer","injected":null,"selfInjected":null},
        "pointingWindow":identity(value.pointing_window),"rootWindow":identity(value.root_window)})
}

pub(crate) fn execute_with<T>(
    params: Value,
    token: &CancellationToken,
    deadline: Option<Instant>,
    resolve: impl FnOnce() -> Result<T, ActionError>,
    read: impl FnOnce(
        T,
        &mut dyn FnMut() -> Result<(), WindowGeometryError>,
    ) -> Result<PointingRootObservation, WindowGeometryError>,
) -> Result<Value, ActionError> {
    if params != json!({}) {
        return Err(fault(
            "invalid_input",
            "指向根窗查询只接受空对象，不接收坐标或窗口句柄。",
        ));
    }
    let limit = Instant::now() + Duration::from_secs(5);
    let deadline = deadline.map_or(limit, |deadline| deadline.min(limit));
    check(token, deadline).map_err(|e| fault(e.code, e.message))?;
    let original = resolve().map_err(|mut e| {
        e.method = Some("pointer.root".into());
        e.effect_state = Some("not_started".into());
        e
    })?;
    check(token, deadline).map_err(|e| fault(e.code, e.message))?;
    let result = read(original, &mut || check(token, deadline));
    check(token, deadline).map_err(|e| fault(e.code, e.message))?;
    result.map(export).map_err(|e| fault(e.code, e.message))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observed() -> PointingRootObservation {
        PointingRootObservation {
            x: -500,
            y: 80,
            pointing_window: ScriptTargetSnapshot {
                window_handle: 11,
                process_id: 21,
                thread_id: 31,
                process_created_file_time: 41,
            },
            root_window: ScriptTargetSnapshot {
                window_handle: 12,
                process_id: 22,
                thread_id: 32,
                process_created_file_time: 42,
            },
        }
    }
    #[test]
    fn invalid_params_never_resolve_or_accept_an_arbitrary_target() {
        for params in [
            json!([]),
            json!(null),
            json!({"point":{"x":1,"y":2}}),
            json!({"hwnd":"12"}),
        ] {
            let error = execute_with(
                params,
                &CancellationToken::new(),
                None,
                || -> Result<(), ActionError> { panic!("非法参数不能解析原目标") },
                |_, _| panic!("非法参数不能读平台"),
            )
            .unwrap_err();
            assert_eq!(error.code, "invalid_input");
            assert_eq!(error.effect_state.as_deref(), Some("not_started"));
        }
    }
    #[test]
    fn cancellation_or_deadline_never_resolves_the_original_target() {
        for cancelled in [false, true] {
            let token = CancellationToken::new();
            if cancelled {
                token.cancel();
            }
            let error = execute_with(
                json!({}),
                &token,
                Some(Instant::now()),
                || -> Result<(), ActionError> { panic!("取消/过期不能解析目标") },
                |_, _| panic!("不能读平台"),
            )
            .unwrap_err();
            assert_eq!(error.code, if cancelled { "cancelled" } else { "timeout" });
            assert_eq!(error.method.as_deref(), Some("pointer.root"));
        }
    }
    #[test]
    fn same_handler_keeps_original_context_and_exports_only_read_observation() {
        let value = execute_with(
            json!({}),
            &CancellationToken::new(),
            None,
            || Ok(99),
            |original, guard| {
                assert_eq!(original, 99);
                guard()?;
                Ok(observed())
            },
        )
        .unwrap();
        assert_eq!(value["sampledAt"], "call-time");
        assert_eq!(value["rootWindow"]["window_handle"], "12");
        assert_eq!(value["pointingWindow"]["window_handle"], "11");
        assert_eq!(value["inputSource"]["injected"], Value::Null);
        assert!(value.get("capture_id").is_none());
        assert!(value.get("context").is_none());
    }
    #[test]
    fn late_cancel_or_native_error_never_returns_partial_data_or_unknown_effect() {
        for cancel in [false, true] {
            let token = CancellationToken::new();
            let error = execute_with(
                json!({}),
                &token,
                None,
                || Ok(()),
                |_, _| {
                    if cancel {
                        token.cancel();
                        Ok(observed())
                    } else {
                        Err(WindowGeometryError::new(
                            "pointing_root_changed",
                            "合成变化",
                            "unknown",
                        ))
                    }
                },
            )
            .unwrap_err();
            assert_eq!(
                error.code,
                if cancel {
                    "cancelled"
                } else {
                    "pointing_root_changed"
                }
            );
            assert_eq!(error.method.as_deref(), Some("pointer.root"));
            assert_eq!(error.effect_state.as_deref(), Some("not_started"));
        }
    }
}
