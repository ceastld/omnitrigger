//! 单次内部选区辅助进程；不是通用脚本宿主。父侧不调用UIA。
use omnitrigger_actions::{ActionError, CancellationToken};
#[cfg(windows)]
use omnitrigger_windows::{ScriptTarget, SelectionOverflow, SelectionReadError};
use omnitrigger_windows::{DEFAULT_SELECTION_UTF16_UNITS, MAX_SELECTION_UTF16_UNITS};
use serde::{Deserialize, Serialize};
#[cfg(any(windows, test))]
use serde_json::json;
use serde_json::Value;
#[cfg(windows)]
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{io::Write, time::Instant};

const FLAG: &str = "--omni-selected-text-once";
// 本机实测正常一次进程启动及 UIA 往返约半秒，预留冷初始化与调度余量。
// 这是上限；原动作期限更短时仍取较早者，成功不等待余下时间。
const MAX_MS: u64 = 2_000;
const MAX_REQUEST: usize = 1024;
const MAX_REPLY: usize = 64 * 1024;
fn default_max_units() -> u32 {
    DEFAULT_SELECTION_UTF16_UNITS as u32
}
fn is_default_max_units(value: &u32) -> bool {
    *value == default_max_units()
}
fn reply_limit(max_utf16_units: u32) -> usize {
    // 目标已在1024字节请求内，另留256字节元数据；默认128仍沿用2048字节。
    (max_utf16_units as usize * 6 + MAX_REQUEST + 256).clamp(2048, MAX_REPLY)
}

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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadOptions {
    #[serde(default)]
    pub(crate) overflow: Overflow,
    #[serde(default = "default_max_units")]
    pub(crate) max_utf16_units: u32,
}
pub(crate) fn options(params: Value) -> Result<ReadOptions, ActionError> {
    if !params.is_object() {
        return Err(fault("invalid_input"));
    }
    let value: ReadOptions = serde_json::from_value(params).map_err(|_| fault("invalid_input"))?;
    if !(1..=MAX_SELECTION_UTF16_UNITS as u32).contains(&value.max_utf16_units) {
        return Err(fault("invalid_input"));
    }
    Ok(value)
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Request {
    v: u8,
    budget_ms: u32,
    target: Value,
    overflow: Overflow,
    #[serde(
        default = "default_max_units",
        skip_serializing_if = "is_default_max_units"
    )]
    max_utf16_units: u32,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum Reply {
    Ok {
        v: u8,
        target: Value,
        overflow: Overflow,
        #[serde(
            default = "default_max_units",
            skip_serializing_if = "is_default_max_units"
        )]
        max_utf16_units: u32,
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
fn valid_text(text: &str, max_utf16_units: u32) -> bool {
    !text.contains('\0') && text.encode_utf16().count() <= max_utf16_units as usize
}
#[cfg(any(windows, test))]
fn decode(
    captured: omnitrigger_windows::ProcessCaptureResult,
    expected: &Value,
    policy: Overflow,
) -> Result<Value, ActionError> {
    decode_with_limit(captured, expected, policy, default_max_units())
}
#[cfg(any(windows, test))]
fn decode_with_limit(
    captured: omnitrigger_windows::ProcessCaptureResult,
    expected: &Value,
    policy: Overflow,
    requested_limit: u32,
) -> Result<Value, ActionError> {
    if !(1..=MAX_SELECTION_UTF16_UNITS as u32).contains(&requested_limit) {
        return Err(fault("invalid_response"));
    }
    // capture_process_if仅在Job收尾成功后交付Captured；所有失败无部分输出。
    if captured.code != 0
        || !captured.stderr.is_empty()
        || captured.stdout.len() > reply_limit(requested_limit)
    {
        return Err(fault("selection_helper_failed"));
    }
    let reply: Reply =
        serde_json::from_str(&captured.stdout).map_err(|_| fault("invalid_response"))?;
    match reply {
        Reply::Ok {
            v: 2,
            target,
            overflow,
            max_utf16_units,
            text,
            truncated,
        } if target == *expected
            && overflow == policy
            && max_utf16_units == requested_limit
            && valid_text(&text, requested_limit)
            && (!truncated
                || policy == Overflow::Truncate
                    && text.encode_utf16().count() == requested_limit as usize) =>
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
    options: ReadOptions,
    cancellation: &CancellationToken,
    deadline: Option<Instant>,
) -> Result<Value, ActionError> {
    let overflow = options.overflow;
    let max_utf16_units = options.max_utf16_units;
    let (captured, context, end) = crate::read_once_helper::capture(
        target,
        cancellation,
        deadline,
        FLAG,
        "selection",
        fault,
        |context, budget_ms| {
            let mut request =
                json!({"v":2,"budget_ms":budget_ms,"target":context,"overflow":overflow});
            if max_utf16_units != default_max_units() {
                request["max_utf16_units"] = json!(max_utf16_units);
            }
            request
        },
    )?;
    let result = if max_utf16_units == default_max_units() {
        decode(captured, &context, overflow)?
    } else {
        decode_with_limit(captured, &context, overflow, max_utf16_units)?
    };
    crate::read_once_helper::check_live(target, cancellation, end).map_err(fault)?;
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
                if request.v != 2
                    || !(1..=MAX_MS as u32).contains(&request.budget_ms)
                    || !(1..=MAX_SELECTION_UTF16_UNITS as u32).contains(&request.max_utf16_units)
                {
                    Err(())
                } else {
                    Ok(request)
                }
            }),
    )
}
fn emit(reply: Reply) -> i32 {
    let limit = match &reply {
        Reply::Ok {
            max_utf16_units, ..
        } => reply_limit(*max_utf16_units),
        Reply::Error { .. } => 2048,
    };
    let Ok(mut bytes) = serde_json::to_vec(&reply) else {
        return 2;
    };
    if bytes.len() + 1 > limit {
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
    if !crate::read_once_helper::owned_capture_child() {
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
    let result = target.read_selection_text_with_limit_if(
        end,
        request.overflow.platform(),
        request.max_utf16_units as usize,
        || {
            if Instant::now() >= end {
                Err(SelectionReadError::Timeout)
            } else {
                Ok(())
            }
        },
    );
    match result {
        Ok(value) => Reply::Ok {
            v: 2,
            target: request.target,
            overflow: request.overflow,
            max_utf16_units: request.max_utf16_units,
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
        assert_eq!(options(json!({})).unwrap().overflow, Overflow::Reject);
        assert_eq!(
            options(json!({"overflow":"truncate"})).unwrap().overflow,
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

#[cfg(test)]
#[path = "../../../tests/selected_text_max_units.rs"]
mod max_units_tests;
