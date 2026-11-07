//! 沿用DesktopHost的原目标与期限；不从参数接收HWND，不重新捕获前台。
use omnitrigger_actions::{ActionError, CancellationToken};
use omnitrigger_windows::{WindowGeometry, WindowGeometryError, WindowRect, WindowRectFrame};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyArgs {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RectArgs {
    left: i32,
    top: i32,
    width: i32,
    height: i32,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Frame {
    Outer,
    Visible,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetArgs {
    rect: RectArgs,
    frame: Frame,
}

fn fault(method: &str, error: WindowGeometryError) -> ActionError {
    let mut value = ActionError::new(error.code, error.message);
    value.method = Some(method.into());
    value.effect_state = Some(error.effect_state.into());
    value
}
fn check(
    cancellation: &CancellationToken,
    deadline: Option<Instant>,
    effect: &'static str,
) -> Result<(), WindowGeometryError> {
    if cancellation.is_cancelled() {
        return Err(WindowGeometryError::new(
            "cancelled",
            "窗口矩形调用已取消。",
            effect,
        ));
    }
    if deadline.is_some_and(|d| Instant::now() >= d) {
        return Err(WindowGeometryError::new(
            "timeout",
            "窗口矩形调用超过本次动作期限。",
            effect,
        ));
    }
    Ok(())
}
fn rect_json(rect: WindowRect) -> Value {
    json!({"left":rect.left,"top":rect.top,"width":rect.width,"height":rect.height})
}
fn geometry_json(value: WindowGeometry) -> Value {
    json!({"coordinateSpace":"physical-screen","outerRect":rect_json(value.outer_rect),
        "visibleFrameRect":rect_json(value.visible_rect),"monitorRect":rect_json(value.monitor_rect),
        "workArea":rect_json(value.work_area),"primaryScreen":{"width":value.primary_screen_width,"height":value.primary_screen_height},"windowDpi":value.window_dpi,"maximized":value.maximized})
}

pub(super) fn execute_with<T>(
    method: &str,
    params: Value,
    cancellation: &CancellationToken,
    deadline: Option<Instant>,
    resolve: impl FnOnce() -> Result<T, ActionError>,
    read: impl FnOnce(
        T,
        &mut dyn FnMut() -> Result<(), WindowGeometryError>,
    ) -> Result<WindowGeometry, WindowGeometryError>,
    set: impl FnOnce(
        T,
        WindowRect,
        WindowRectFrame,
        &mut dyn FnMut() -> Result<(), WindowGeometryError>,
    ) -> Result<(), WindowGeometryError>,
) -> Result<Value, ActionError> {
    // 单次调用预算与两种传输的5秒上限一致；不能硬性抢占已经进入的Win32调用。
    let limit = Instant::now() + Duration::from_secs(5);
    let deadline = Some(deadline.map_or(limit, |value| value.min(limit)));
    // serde命名struct也可从序列反序列化；raw-wire合同只接受对象，不接受位置参数。
    if !params.is_object()
        || (method == "window.set_rect" && !params.get("rect").is_some_and(Value::is_object))
    {
        return Err(fault(
            method,
            WindowGeometryError::new(
                "invalid_input",
                "窗口矩形参数及rect必须为JSON对象。",
                "not_started",
            ),
        ));
    }
    // 完整解析和几何校验都在解析原目标之前，结构错误不会调用平台。
    let request = match method {
        "window.geometry" => {
            serde_json::from_value::<EmptyArgs>(params).map_err(|e| {
                fault(
                    method,
                    WindowGeometryError::new("invalid_input", e.to_string(), "not_started"),
                )
            })?;
            None
        }
        "window.set_rect" => {
            let args: SetArgs = serde_json::from_value(params).map_err(|e| {
                fault(
                    method,
                    WindowGeometryError::new("invalid_input", e.to_string(), "not_started"),
                )
            })?;
            let rect = WindowRect {
                left: args.rect.left,
                top: args.rect.top,
                width: args.rect.width,
                height: args.rect.height,
            }
            .validate()
            .map_err(|e| fault(method, e))?;
            Some((
                rect,
                match args.frame {
                    Frame::Outer => WindowRectFrame::Outer,
                    Frame::Visible => WindowRectFrame::Visible,
                },
            ))
        }
        _ => {
            return Err(fault(
                method,
                WindowGeometryError::new("unknown_method", "不支持的窗口矩形方法。", "not_started"),
            ))
        }
    };
    check(cancellation, deadline, "not_started").map_err(|e| fault(method, e))?;
    let target = resolve().map_err(|mut e| {
        e.method = Some(method.into());
        e
    })?;
    check(cancellation, deadline, "not_started").map_err(|e| fault(method, e))?;
    let mut guard = || check(cancellation, deadline, "not_started");
    let result = match request {
        None => read(target, &mut guard).map(geometry_json),
        Some((rect, frame)) => set(target, rect, frame, &mut guard)
            .map(|()| json!({"submitted":true,"effectState":"unknown"})),
    };
    let effect = match &result {
        Ok(_) => {
            if request.is_some() {
                "unknown"
            } else {
                "not_started"
            }
        }
        Err(e) => e.effect_state,
    };
    check(cancellation, deadline, effect).map_err(|e| fault(method, e))?;
    result.map_err(|e| fault(method, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn resolve() -> Result<(), ActionError> {
        panic!("参数或早期取消拒绝不得解析目标")
    }
    #[test]
    fn invalid_rectangles_and_unknown_fields_stop_before_target_resolution() {
        for params in [
            json!({}),
            json!({"rect":{"left":0,"top":0,"width":0,"height":1},"frame":"outer"}),
            json!({"rect":{"left":2147483647i64,"top":0,"width":1,"height":1},"frame":"visible"}),
            json!({"rect":{"left":0,"top":0,"width":10,"height":10,"hwnd":1},"frame":"outer"}),
        ] {
            let e = execute_with(
                "window.set_rect",
                params,
                &CancellationToken::new(),
                None,
                resolve,
                |_, _| panic!(),
                |_, _, _, _| panic!(),
            )
            .unwrap_err();
            assert_eq!(e.code, "invalid_input");
            assert_eq!(e.effect_state.as_deref(), Some("not_started"));
        }
    }
    #[test]
    fn early_cancel_or_deadline_and_read_parameters_do_not_reach_platform() {
        for cancelled in [false, true] {
            let token = CancellationToken::new();
            if cancelled {
                token.cancel();
            }
            let e = execute_with(
                "window.geometry",
                json!({}),
                &token,
                Some(Instant::now()),
                resolve,
                |_, _| panic!(),
                |_, _, _, _| panic!(),
            )
            .unwrap_err();
            assert_eq!(e.code, if cancelled { "cancelled" } else { "timeout" });
        }
        assert_eq!(
            execute_with(
                "window.geometry",
                json!({"hwnd":1}),
                &CancellationToken::new(),
                None,
                resolve,
                |_, _| panic!(),
                |_, _, _, _| panic!()
            )
            .unwrap_err()
            .code,
            "invalid_input"
        );
    }
    #[test]
    fn late_set_cancel_is_unknown_and_pre_submit_target_change_stays_not_started() {
        for late in [false, true] {
            let token = CancellationToken::new();
            let e = execute_with(
                "window.set_rect",
                json!({"rect":{"left":-100,"top":0,"width":10,"height":10},"frame":"visible"}),
                &token,
                None,
                || Ok(()),
                |_, _| panic!(),
                |_, _, _, guard| {
                    guard()?;
                    if late {
                        token.cancel();
                        Ok(())
                    } else {
                        Err(WindowGeometryError::new(
                            "target_changed",
                            "原目标已改变",
                            "not_started",
                        ))
                    }
                },
            )
            .unwrap_err();
            assert_eq!(e.code, if late { "cancelled" } else { "target_changed" });
            assert_eq!(
                e.effect_state.as_deref(),
                Some(if late { "unknown" } else { "not_started" })
            );
        }
    }
    #[test]
    fn geometry_serializes_primary_separately_without_dropping_existing_fields() {
        let r = WindowRect {
            left: -1920,
            top: 0,
            width: 1920,
            height: 1080,
        };
        let value = geometry_json(WindowGeometry {
            outer_rect: r,
            visible_rect: r,
            monitor_rect: r,
            work_area: r,
            primary_screen_width: 2560,
            primary_screen_height: 1440,
            window_dpi: 144,
            maximized: false,
        });
        assert_eq!(value["primaryScreen"], json!({"width":2560,"height":1440}));
        assert_eq!(value["monitorRect"]["width"], 1920);
        assert_eq!(value.as_object().unwrap().len(), 8);
        assert_eq!(value["coordinateSpace"], "physical-screen");
    }
}
