//! 按次桥接固定来源的常驻协议；不包含安装器。
use omnitrigger_actions::{ActionError, CancellationToken};
#[cfg(windows)]
use serde::Deserialize;
#[cfg(windows)]
use serde_json::json;
use serde_json::Value;
use std::time::Instant;
#[cfg(windows)]
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(windows)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Resident {
    window_handle: String,
    process_id: u32,
    thread_id: u32,
    process_created_file_time: String,
    expires_at_unix_ms: u64,
}
#[cfg(windows)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Region {
    resident: Resident,
}

fn error(code: &str, method: &str, effect: &str) -> ActionError {
    let mut value = ActionError::new(
        code,
        "RaiShot 常驻请求未获完整确认；请检查错误代码和效果状态。",
    );
    value.method = Some(method.into());
    value.effect_state = Some(effect.into());
    value
}
#[cfg(windows)]
fn now_ms() -> Result<u64, &'static str> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "clock_unavailable")?
        .as_millis()
        .try_into()
        .map_err(|_| "clock_unavailable")
}
fn check(cancellation: &CancellationToken, deadline: Option<Instant>) -> Result<(), &'static str> {
    if cancellation.is_cancelled() {
        return Err("cancelled");
    }
    if deadline.is_some_and(|end| Instant::now() >= end) {
        return Err("timeout");
    }
    Ok(())
}
#[cfg(windows)]
fn positive(value: &str) -> Result<u64, &'static str> {
    if value.is_empty() || value.len() > 20 || !value.bytes().all(|v| v.is_ascii_digit()) {
        return Err("invalid_input");
    }
    let parsed = value.parse::<u64>().map_err(|_| "invalid_input")?;
    if parsed == 0 {
        return Err("invalid_input");
    }
    Ok(parsed)
}

pub(super) fn execute(
    method: &str,
    params: Value,
    cancellation: &CancellationToken,
    deadline: Option<Instant>,
) -> Result<Value, ActionError> {
    check(cancellation, deadline).map_err(|code| error(code, method, "not_started"))?;
    #[cfg(not(windows))]
    {
        let _ = params;
        Err(error("unsupported_platform", method, "not_started"))
    }
    #[cfg(windows)]
    {
        if method == "raishot.resident.find" {
            if !params.as_object().is_some_and(|p| p.is_empty()) {
                return Err(error("invalid_input", method, "not_started"));
            }
            let resident = omnitrigger_windows::find_raishot_resident()
                .map_err(|code| error(code, method, "not_started"))?;
            check(cancellation, deadline).map_err(|code| error(code, method, "not_started"))?;
            let Some(value) = resident else {
                return Ok(Value::Null);
            };
            let expires = now_ms()
                .map_err(|code| error(code, method, "not_started"))?
                .saturating_add(5000);
            return Ok(
                json!({"window_handle":value.window_handle.to_string(),"process_id":value.process_id,"thread_id":value.thread_id,
                "process_created_file_time":value.process_created_file_time.to_string(),"expires_at_unix_ms":expires}),
            );
        }
        if method != "raishot.resident.region" {
            return Err(error("invalid_input", method, "not_started"));
        }
        let input: Region = serde_json::from_value(params)
            .map_err(|_| error("invalid_input", method, "not_started"))?;
        let expected = omnitrigger_windows::RaiShotResident {
            window_handle: positive(&input.resident.window_handle)
                .map_err(|code| error(code, method, "not_started"))?,
            process_id: input.resident.process_id,
            thread_id: input.resident.thread_id,
            process_created_file_time: positive(&input.resident.process_created_file_time)
                .map_err(|code| error(code, method, "not_started"))?,
        };
        if expected.process_id == 0 || expected.thread_id == 0 {
            return Err(error("invalid_input", method, "not_started"));
        }
        let validate = || {
            check(cancellation, deadline)?;
            let now = now_ms()?;
            if input.resident.expires_at_unix_ms <= now
                || input.resident.expires_at_unix_ms - now > 5000
            {
                return Err("raishot_resident_expired");
            }
            Ok(())
        };
        omnitrigger_windows::request_raishot_resident_region(&expected, validate)
            .map_err(|code| error(code, method, "not_started"))?;
        // 投递后取消/超时为 unknown，不能自动重试。
        check(cancellation, deadline).map_err(|code| error(code, method, "unknown"))?;
        Ok(json!({"queued":true,"screenshot_completed":false}))
    }
}

#[cfg(all(test, windows))]
#[path = "raishot_resident_native_tests.rs"]
mod native_tests;
