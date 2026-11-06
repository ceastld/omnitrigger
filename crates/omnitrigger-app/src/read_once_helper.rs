//! UIA与Shell只读辅助进程共用传输；不在父进程进入COM。
use omnitrigger_actions::{ActionError, CancellationToken};
use omnitrigger_windows::{
    capture_process_if, ProcessCaptureControl as Control, ProcessCaptureOptions as CaptureOptions,
    ProcessCaptureResult, ScriptTarget,
};
use serde_json::{json, Value};
use std::{
    cell::Cell,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
const MAX_MS: u64 = 2_000;
const MAX_REQUEST: usize = 1024;
#[cfg(windows)]
fn clock(fault: fn(&str) -> ActionError) -> Result<u64, ActionError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .ok_or_else(|| fault("selection_unavailable"))
}
#[cfg(windows)]
fn remaining(end: Instant, fault: fn(&str) -> ActionError) -> Result<u64, ActionError> {
    let left = end
        .saturating_duration_since(Instant::now())
        .as_millis()
        .min(u128::from(MAX_MS)) as u64;
    if left == 0 {
        Err(fault("timeout"))
    } else {
        Ok(left)
    }
}
#[cfg(windows)]
pub(crate) fn check_live(
    target: &ScriptTarget,
    cancellation: &CancellationToken,
    end: Instant,
) -> Result<(), &'static str> {
    if cancellation.is_cancelled() {
        return Err("cancelled");
    }
    if Instant::now() >= end {
        return Err("timeout");
    }
    target.ensure_foreground().map_err(|_| "target_changed")?;
    // 身份/前台核验可能耗时；核验后复查取消与动作期限。
    if cancellation.is_cancelled() {
        return Err("cancelled");
    }
    if Instant::now() >= end {
        return Err("timeout");
    }
    Ok(())
}

pub(crate) fn capture(
    target: &ScriptTarget,
    cancellation: &CancellationToken,
    deadline: Option<Instant>,
    flag: &'static str,
    kind: &'static str,
    fault: fn(&str) -> ActionError,
    make_request: impl FnOnce(Value, u32) -> Value,
) -> Result<(ProcessCaptureResult, Value, Instant), ActionError> {
    let end = Instant::now() + Duration::from_millis(MAX_MS);
    let end = deadline.map_or(end, |outer| outer.min(end));
    check_live(target, cancellation, end).map_err(fault)?;
    let snapshot = target.snapshot().map_err(|_| fault("target_changed"))?;
    check_live(target, cancellation, end).map_err(fault)?;
    let mut context = crate::action_target_context::export(snapshot)
        .map_err(|_| fault("selection_unavailable"))?;
    let budget_ms = remaining(end, fault)?;
    context["expires_at_unix_ms"] = json!(clock(fault)?
        .checked_add(budget_ms)
        .ok_or_else(|| fault("timeout"))?);
    let request = make_request(context.clone(), budget_ms as u32);
    let serialized = serde_json::to_string(&request).map_err(|_| fault("invalid_response"))?;
    if serialized.len() > MAX_REQUEST {
        return Err(fault("invalid_response"));
    }
    let executable =
        std::env::current_exe().map_err(|_| fault(&format!("{kind}_helper_start_failed")))?;
    // 不搜索PATH，不使用解释器或配置中的可执行文件。普通本地盘符路径交由现有验证器复验。
    let program = executable
        .to_str()
        .ok_or_else(|| fault(&format!("{kind}_helper_start_failed")))?;
    let cwd = executable
        .parent()
        .and_then(|p| p.to_str())
        .ok_or_else(|| fault(&format!("{kind}_helper_start_failed")))?;
    let options = CaptureOptions {
        timeout_ms: remaining(end, fault)?,
        allow_non_zero: true,
        cwd: Some(cwd.into()),
    };
    let args = [flag.to_owned(), serialized];
    let stop_reason = Cell::new(None);
    let mut control = || match check_live(target, cancellation, end) {
        Ok(()) => Control::Continue,
        Err(code) => {
            stop_reason.set(Some(code));
            if code == "timeout" {
                Control::Deadline
            } else {
                Control::Cancelled
            }
        }
    };
    let captured =
        capture_process_if(program, &args, &options, &mut control).map_err(|failure| {
            // 收尾无法确认优先于原取消/超时；不得冒称helper已终止或交付任何stdout。
            if failure.code == "process_cleanup_failed" {
                return fault(&format!("{kind}_helper_cleanup_failed"));
            }
            if let Some(code) = stop_reason.get() {
                return fault(code);
            }
            match failure.code {
                "process_timeout" | "timeout" => fault("timeout"),
                "cancelled" => fault("cancelled"),
                "invalid_arguments" | "process_setup_failed" | "process_start_failed" => {
                    fault(&format!("{kind}_helper_start_failed"))
                }
                _ => fault(&format!("{kind}_helper_failed")),
            }
        })?;
    check_live(target, cancellation, end).map_err(fault)?;
    Ok((captured, context, end))
}

#[path = "selected_text_helper_scope.rs"]
mod scope;
pub(crate) fn owned_capture_child() -> bool {
    scope::owned_capture_child()
}
