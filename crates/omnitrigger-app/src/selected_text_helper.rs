//! 单次内部选区辅助进程；不是通用脚本宿主。父侧不调用UIA。
use omnitrigger_actions::{ActionError, CancellationToken};
#[cfg(windows)]
use omnitrigger_windows::{
    capture_process_if, ProcessCaptureControl as Control, ProcessCaptureOptions as CaptureOptions,
    ScriptTarget, SelectionOverflow, SelectionReadError,
};
use serde::{Deserialize, Serialize};
#[cfg(any(windows, test))]
use serde_json::json;
use serde_json::Value;
#[cfg(windows)]
use std::{
    cell::Cell,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use std::{io::Write, time::Instant};

const FLAG: &str = "--omni-selected-text-once";
// 本机实测正常一次进程启动及 UIA 往返约半秒，预留冷初始化与调度余量。
// 这是上限；原动作期限更短时仍取较早者，成功不等待余下时间。
const MAX_MS: u64 = 2_000;
const MAX_REQUEST: usize = 1024;
const MAX_REPLY: usize = 2048;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Overflow {
    #[default]
    Reject,
    Truncate,
}
impl Overflow {
    #[cfg(windows)]
    fn platform(self) -> SelectionOverflow {
        match self {
            Self::Reject => SelectionOverflow::Reject,
            Self::Truncate => SelectionOverflow::Truncate,
        }
    }
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadOptions {
    #[serde(default)]
    overflow: Overflow,
}
pub(crate) fn options(params: Value) -> Result<Overflow, ActionError> {
    if !params.is_object() {
        return Err(fault("invalid_input"));
    }
    serde_json::from_value::<ReadOptions>(params)
        .map(|value| value.overflow)
        .map_err(|_| fault("invalid_input"))
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Request {
    v: u8,
    budget_ms: u32,
    target: Value,
    overflow: Overflow,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum Reply {
    Ok {
        v: u8,
        target: Value,
        overflow: Overflow,
        text: String,
        truncated: bool,
    },
    Error {
        v: u8,
        code: String,
    },
}

pub(crate) fn fault(code: &str) -> ActionError {
    let mut error = ActionError::new(code, "只读选区读取未获确认；请检查错误代码。");
    error.method = Some("selection.read_text".into());
    // 只读能力不产生输入/剪贴板写入，不将此字段解释为未发生过UIA读取。
    error.effect_state = Some("not_started".into());
    error
}
#[cfg(windows)]
fn clock() -> Result<u64, ActionError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .ok_or_else(|| fault("selection_unavailable"))
}
#[cfg(windows)]
fn remaining(end: Instant) -> Result<u64, ActionError> {
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
fn check_live(
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
#[cfg(any(windows, test))]
fn read_code(code: &str) -> bool {
    matches!(
        code,
        "cancelled"
            | "timeout"
            | "target_changed"
            | "selection_changed"
            | "selection_unavailable"
            | "selection_empty"
            | "selection_multiple"
            | "selection_protected"
            | "selection_too_long"
            | "invalid_response"
            | "selection_helper_scope_rejected"
    )
}
#[cfg(any(windows, test))]
fn valid_text(text: &str) -> bool {
    !text.contains('\0') && text.encode_utf16().count() <= 128
}
#[cfg(any(windows, test))]
fn decode(
    captured: omnitrigger_windows::ProcessCaptureResult,
    expected: &Value,
    policy: Overflow,
) -> Result<Value, ActionError> {
    // capture_process_if仅在Job收尾成功后交付Captured；所有失败无部分输出。
    if captured.code != 0 || !captured.stderr.is_empty() || captured.stdout.len() > MAX_REPLY {
        return Err(fault("selection_helper_failed"));
    }
    let reply: Reply =
        serde_json::from_str(&captured.stdout).map_err(|_| fault("invalid_response"))?;
    match reply {
        Reply::Ok {
            v: 2,
            target,
            overflow,
            text,
            truncated,
        } if target == *expected
            && overflow == policy
            && valid_text(&text)
            && (!truncated
                || policy == Overflow::Truncate && text.encode_utf16().count() == 128) =>
        {
            Ok(json!({"text":text,"truncated":truncated}))
        }
        Reply::Error { v: 2, code } if read_code(&code) => Err(fault(&code)),
        _ => Err(fault("invalid_response")),
    }
}

/// 父进程：仅启动自己的同一exe内部入口，复用创建期Job关联/非阻塞采集/有界收尾。
#[cfg(windows)]
pub(crate) fn read(
    target: &ScriptTarget,
    overflow: Overflow,
    cancellation: &CancellationToken,
    deadline: Option<Instant>,
) -> Result<Value, ActionError> {
    let end = Instant::now() + Duration::from_millis(MAX_MS);
    let end = deadline.map_or(end, |outer| outer.min(end));
    check_live(target, cancellation, end).map_err(fault)?;
    let snapshot = target.snapshot().map_err(|_| fault("target_changed"))?;
    check_live(target, cancellation, end).map_err(fault)?;
    let mut context = crate::action_target_context::export(snapshot)
        .map_err(|_| fault("selection_unavailable"))?;
    let budget_ms = remaining(end)?;
    context["expires_at_unix_ms"] = json!(clock()?
        .checked_add(budget_ms)
        .ok_or_else(|| fault("timeout"))?);
    let request = Request {
        v: 2,
        budget_ms: budget_ms as u32,
        target: context.clone(),
        overflow,
    };
    let serialized = serde_json::to_string(&request).map_err(|_| fault("invalid_response"))?;
    if serialized.len() > MAX_REQUEST {
        return Err(fault("invalid_response"));
    }
    let executable = std::env::current_exe().map_err(|_| fault("selection_helper_start_failed"))?;
    // 不搜索PATH，不使用解释器或配置中的可执行文件。普通本地盘符路径交由现有验证器复验。
    let program = executable
        .to_str()
        .ok_or_else(|| fault("selection_helper_start_failed"))?;
    let cwd = executable
        .parent()
        .and_then(|p| p.to_str())
        .ok_or_else(|| fault("selection_helper_start_failed"))?;
    let options = CaptureOptions {
        timeout_ms: remaining(end)?,
        allow_non_zero: true,
        cwd: Some(cwd.into()),
    };
    let args = [FLAG.to_owned(), serialized];
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
                return fault("selection_helper_cleanup_failed");
            }
            if let Some(code) = stop_reason.get() {
                return fault(code);
            }
            match failure.code {
                "process_timeout" | "timeout" => fault("timeout"),
                "cancelled" => fault("cancelled"),
                "invalid_arguments" | "process_setup_failed" | "process_start_failed" => {
                    fault("selection_helper_start_failed")
                }
                _ => fault("selection_helper_failed"),
            }
        })?;
    check_live(target, cancellation, end).map_err(fault)?;
    let result = decode(captured, &context, overflow)?;
    check_live(target, cancellation, end).map_err(fault)?;
    Ok(result)
}

fn parse(args: &[String]) -> Option<Result<Request, ()>> {
    if !args.iter().skip(1).any(|arg| {
        arg == FLAG
            || arg
                .strip_prefix(FLAG)
                .is_some_and(|rest| rest.starts_with('='))
    }) {
        return None;
    }
    if args.len() != 3 || args[1] != FLAG || args[2].len() > MAX_REQUEST {
        return Some(Err(()));
    }
    Some(
        serde_json::from_str::<Request>(&args[2])
            .map_err(|_| ())
            .and_then(|request| {
                if request.v != 2 || !(1..=MAX_MS as u32).contains(&request.budget_ms) {
                    Err(())
                } else {
                    Ok(request)
                }
            }),
    )
}
fn emit(reply: Reply) -> i32 {
    let Ok(mut bytes) = serde_json::to_vec(&reply) else {
        return 2;
    };
    if bytes.len() + 1 > MAX_REPLY {
        return 2;
    }
    bytes.push(b'\n');
    let mut output = std::io::stdout().lock();
    if output
        .write_all(&bytes)
        .and_then(|()| output.flush())
        .is_ok()
    {
        0
    } else {
        2
    }
}
#[cfg(windows)]
fn child(request: Request) -> Reply {
    if !scope::owned_capture_child() {
        return Reply::Error {
            v: 2,
            code: "selection_helper_scope_rejected".into(),
        };
    }
    let fail = |code: &str| Reply::Error {
        v: 2,
        code: code.into(),
    };
    let Ok(now) = clock() else {
        return fail("selection_unavailable");
    };
    let Ok(snapshot) = crate::action_target_context::validate_value(request.target.clone(), now)
    else {
        return fail("target_changed");
    };
    let Some(expires) = request
        .target
        .get("expires_at_unix_ms")
        .and_then(Value::as_u64)
    else {
        return fail("invalid_response");
    };
    let budget = u64::from(request.budget_ms).min(expires.saturating_sub(now));
    if budget == 0 {
        return fail("timeout");
    }
    let end = Instant::now() + Duration::from_millis(budget);
    let Ok(target) = ScriptTarget::from_snapshot(snapshot) else {
        return fail("target_changed");
    };
    let result = target.read_selection_text_if(end, request.overflow.platform(), || {
        if Instant::now() >= end {
            Err(SelectionReadError::Timeout)
        } else {
            Ok(())
        }
    });
    match result {
        Ok(value) => Reply::Ok {
            v: 2,
            target: request.target,
            overflow: request.overflow,
            text: value.text,
            truncated: value.truncated,
        },
        Err(error) => fail(error.code()),
    }
}

/// 必须在main的任何配置/legacy/历史/Hook启动代码之前调用；出现flag但混用参数即退出。
pub(crate) fn cli(args: &[String]) -> Option<i32> {
    parse(args).map(|request| match request {
        Ok(request) => emit(child(request)),
        Err(()) => 2,
    })
}

pub(crate) fn check_action(
    cancellation: &CancellationToken,
    deadline: Option<Instant>,
) -> Result<(), ActionError> {
    if cancellation.is_cancelled() {
        return Err(fault("cancelled"));
    }
    if deadline.is_some_and(|end| Instant::now() >= end) {
        return Err(fault("timeout"));
    }
    Ok(())
}
#[cfg(not(windows))]
fn child(_: Request) -> Reply {
    Reply::Error {
        v: 2,
        code: "selection_unavailable".into(),
    }
}
#[cfg(windows)]
#[path = "selected_text_helper_scope.rs"]
mod scope;

#[cfg(test)]
mod selection_helper_protocol_tests {
    use super::*;
    fn expected() -> Value {
        json!({"window_handle":"1","process_id":2,"thread_id":3,"process_created_file_time":"4","expires_at_unix_ms":1000})
    }
    fn captured(value: Value) -> omnitrigger_windows::ProcessCaptureResult {
        omnitrigger_windows::ProcessCaptureResult {
            code: 0,
            stdout: value.to_string(),
            stderr: String::new(),
        }
    }
    fn reply(text: &str, mode: Overflow, truncated: bool) -> Value {
        json!({"status":"ok","v":2,"target":expected(),"overflow":mode,"text":text,"truncated":truncated})
    }
    #[test]
    fn default_reject_explicit_truncate_and_unknown_options_are_strict() {
        assert_eq!(options(json!({})).unwrap(), Overflow::Reject);
        assert_eq!(
            options(json!({"overflow":"truncate"})).unwrap(),
            Overflow::Truncate
        );
        for value in [
            json!({"overflow":"lossy"}),
            json!({"overflow":null}),
            json!({"maxLength":128}),
            json!([]),
            json!(["truncate"]),
            json!(["reject"]),
            json!(null),
        ] {
            assert_eq!(options(value).unwrap_err().code, "invalid_input");
        }
    }
    #[test]
    fn helper_v2_requires_policy_and_rejects_stale_or_mixed_requests() {
        let payload =
            json!({"v":2,"budget_ms":800,"target":expected(),"overflow":"truncate"}).to_string();
        assert!(parse(&["self.exe".into(), FLAG.into(), payload.clone()])
            .unwrap()
            .is_ok());
        assert!(parse(&[
            "self.exe".into(),
            "--config".into(),
            "never-read.json".into(),
            FLAG.into(),
            payload
        ])
        .unwrap()
        .is_err());
        for value in [
            json!({"v":1,"budget_ms":800,"target":expected()}),
            json!({"v":2,"budget_ms":800,"target":expected()}),
            json!({"v":2,"budget_ms":800,"target":expected(),"overflow":"repair"}),
        ] {
            assert!(parse(&["self.exe".into(), FLAG.into(), value.to_string()])
                .unwrap()
                .is_err());
        }
    }
    #[test]
    fn helper_budget_accepts_the_limit_and_rejects_zero_or_over_limit() {
        for (budget, valid) in [(1, true), (2_000, true), (0, false), (2_001, false)] {
            let payload = json!({"v":2,"budget_ms":budget,"target":expected(),"overflow":"reject"});
            assert_eq!(
                parse(&["self.exe".into(), FLAG.into(), payload.to_string()])
                    .unwrap()
                    .is_ok(),
                valid
            );
        }
    }
    #[test]
    fn reply_binds_policy_flag_target_and_utf16_length() {
        let target = expected();
        assert_eq!(
            decode(
                captured(reply("合成😀", Overflow::Reject, false)),
                &target,
                Overflow::Reject
            )
            .unwrap(),
            json!({"text":"合成😀","truncated":false})
        );
        assert_eq!(
            decode(
                captured(reply(&"😀".repeat(64), Overflow::Truncate, true)),
                &target,
                Overflow::Truncate
            )
            .unwrap(),
            json!({"text":"😀".repeat(64),"truncated":true})
        );
        for value in [
            reply("short", Overflow::Truncate, true),
            reply(&"A".repeat(128), Overflow::Reject, true),
            reply("ok", Overflow::Reject, false),
            reply(&"😀".repeat(65), Overflow::Truncate, false),
        ] {
            assert_eq!(
                decode(captured(value), &target, Overflow::Truncate)
                    .unwrap_err()
                    .code,
                "invalid_response"
            );
        }
        let mut foreign = reply("ok", Overflow::Truncate, false);
        foreign["target"] = json!({});
        assert_eq!(
            decode(captured(foreign), &target, Overflow::Truncate)
                .unwrap_err()
                .code,
            "invalid_response"
        );
    }
    #[test]
    fn private_cli_has_no_default_request_and_output_is_bounded() {
        for args in [
            vec!["self.exe", FLAG],
            vec!["self.exe", FLAG, "{}"],
            vec!["self.exe", "--omni-selected-text-once={}"],
            vec!["self.exe", FLAG, "{}", "--run-action"],
        ] {
            assert!(
                parse(&args.into_iter().map(String::from).collect::<Vec<_>>())
                    .unwrap()
                    .is_err()
            );
        }
        assert!(parse(&["self.exe".into(), "--run-action".into(), "call.json".into()]).is_none());
        let target = expected();
        let mut data = captured(reply("ok", Overflow::Reject, false));
        data.stdout = " ".repeat(MAX_REPLY + 1);
        assert_eq!(
            decode(data, &target, Overflow::Reject).unwrap_err().code,
            "selection_helper_failed"
        );
        assert_eq!(
            decode(
                captured(reply("A\0B", Overflow::Reject, false)),
                &target,
                Overflow::Reject
            )
            .unwrap_err()
            .code,
            "invalid_response"
        );
        let mut bad = reply("ok", Overflow::Reject, false);
        bad["extra"] = json!("no");
        assert_eq!(
            decode(captured(bad), &target, Overflow::Reject)
                .unwrap_err()
                .code,
            "invalid_response"
        );
    }
    #[test]
    fn confirmed_empty_reply_is_success_under_both_policies_but_never_truncated() {
        let target = expected();
        for policy in [Overflow::Reject, Overflow::Truncate] {
            assert_eq!(
                decode(captured(reply("", policy, false)), &target, policy).unwrap(),
                json!({"text":"","truncated":false})
            );
            assert_eq!(
                decode(captured(reply("", policy, true)), &target, policy)
                    .unwrap_err()
                    .code,
                "invalid_response"
            );
        }
        let mut foreign = reply("", Overflow::Reject, false);
        foreign["target"] = json!({});
        assert_eq!(
            decode(captured(foreign), &target, Overflow::Reject)
                .unwrap_err()
                .code,
            "invalid_response"
        );
        assert_eq!(
            decode(
                captured(reply("", Overflow::Truncate, false)),
                &target,
                Overflow::Reject
            )
            .unwrap_err()
            .code,
            "invalid_response"
        );
    }
    #[test]
    fn error_or_missing_stdout_is_never_an_empty_selection() {
        let target = expected();
        for code in [
            "selection_unavailable",
            "target_changed",
            "selection_changed",
            "timeout",
            "selection_empty",
        ] {
            let error = decode(
                captured(json!({"status":"error","v":2,"code":code})),
                &target,
                Overflow::Reject,
            )
            .unwrap_err();
            assert_eq!(error.code, code);
        }
        for stdout in ["", "{}", "null"] {
            let data = omnitrigger_windows::ProcessCaptureResult {
                code: 0,
                stdout: stdout.into(),
                stderr: String::new(),
            };
            assert_eq!(
                decode(data, &target, Overflow::Reject).unwrap_err().code,
                "invalid_response"
            );
        }
        let mut data = captured(reply("", Overflow::Reject, false));
        data.stderr = "provider failure".into();
        assert_eq!(
            decode(data, &target, Overflow::Reject).unwrap_err().code,
            "selection_helper_failed"
        );
        let mut data = captured(reply("", Overflow::Reject, false));
        data.code = 2;
        assert_eq!(
            decode(data, &target, Overflow::Reject).unwrap_err().code,
            "selection_helper_failed"
        );
    }
    #[test]
    fn response_v2_keeps_stderr_stale_version_and_fixed_error_hygiene() {
        let target = expected();
        let mut value = captured(reply("ok", Overflow::Reject, false));
        value.stderr = "private provider text".into();
        let error = decode(value, &target, Overflow::Reject).unwrap_err();
        assert_eq!(error.code, "selection_helper_failed");
        assert!(!error.message.contains("private"));
        let stale = json!({"status":"ok","v":1,"target":target,"text":"old"});
        assert_eq!(
            decode(captured(stale), &target, Overflow::Reject)
                .unwrap_err()
                .code,
            "invalid_response"
        );
        let empty = json!({"status":"error","v":2,"code":"selection_empty"});
        assert_eq!(
            decode(captured(empty), &target, Overflow::Truncate)
                .unwrap_err()
                .code,
            "selection_empty"
        );
    }
}
