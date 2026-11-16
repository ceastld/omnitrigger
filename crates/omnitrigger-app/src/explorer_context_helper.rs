//! 单次原Explorer上下文辅助进程。父侧仅传固定身份和期限，不进入Shell COM。
use omnitrigger_actions::{ActionError, CancellationToken};
#[cfg(any(windows, test))]
use omnitrigger_windows::{validate_explorer_context, ExplorerContextSnapshot};
use serde::{Deserialize, Serialize};
#[cfg(any(windows, test))]
use serde_json::json;
use serde_json::Value;
#[cfg(windows)]
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{io::Write, time::Instant};
const FLAG: &str = "--omni-explorer-context-once";
const MAX_MS: u64 = 2_000;
const MAX_REQUEST: usize = 1024;
const MAX_REPLY: usize = 60 * 1024;
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Request {
    v: u8,
    budget_ms: u32,
    target: Value,
    #[serde(
        default,
        deserialize_with = "explicit_selection_failure_stage",
        skip_serializing_if = "is_false"
    )]
    selection_failure_stage: bool,
}
#[derive(Deserialize, Serialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum Reply {
    Ok {
        v: u8,
        target: Value,
        #[serde(deserialize_with = "required_nullable")]
        current_directory: Option<String>,
        selected_paths: Vec<String>,
    },
    Error {
        v: u8,
        code: String,
    },
}
fn required_nullable<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(deserializer)
}
pub(crate) fn fault(code: &str) -> ActionError {
    let mut error = ActionError::new(code, "原资源管理器上下文读取未获确认；请检查错误代码。");
    error.method = Some("explorer.context".into());
    error.effect_state = Some("not_started".into());
    error
}
pub(crate) fn options(params: Value) -> Result<bool, ActionError> {
    if params.as_object().is_some_and(|params| params.is_empty()) {
        Ok(false)
    } else if params.as_object().is_some_and(|params| {
        params.len() == 1 && params.get("selection_failure_stage") == Some(&Value::Bool(true))
    }) {
        Ok(true)
    } else {
        Err(fault("invalid_input"))
    }
}
pub(crate) fn check_action(
    token: &CancellationToken,
    deadline: Option<Instant>,
) -> Result<(), ActionError> {
    if token.is_cancelled() {
        return Err(fault("cancelled"));
    }
    if deadline.is_some_and(|end| Instant::now() >= end) {
        return Err(fault("timeout"));
    }
    Ok(())
}
#[cfg(any(windows, test))]
fn decode(
    captured: omnitrigger_windows::ProcessCaptureResult,
    expected: &Value,
) -> Result<Value, ActionError> {
    if captured.code != 0 || !captured.stderr.is_empty() || captured.stdout.len() > MAX_REPLY {
        return Err(fault("explorer_helper_failed"));
    }
    let reply: Reply =
        serde_json::from_str(&captured.stdout).map_err(|_| fault("invalid_response"))?;
    match reply {
        Reply::Ok {
            v: 1,
            target,
            current_directory,
            selected_paths,
        } if target == *expected => {
            let value = ExplorerContextSnapshot {
                current_directory,
                selected_paths,
            };
            validate_explorer_context(&value).map_err(|error| fault(error.code()))?;
            Ok(
                json!({"currentDirectory":value.current_directory,"selectedPaths":value.selected_paths}),
            )
        }
        Reply::Error { v: 1, code }
            if matches!(
                code.as_str(),
                "timeout"
                    | "target_changed"
                    | "unsupported_target"
                    | "ambiguous_target"
                    | "selection_changed"
                    | "selection_unavailable"
                    | "unsupported_item"
                    | "selection_too_large"
                    | "invalid_response"
                    | "explorer_helper_scope_rejected"
            ) =>
        {
            Err(fault(&code))
        }
        _ => Err(fault("invalid_response")),
    }
}
#[cfg(windows)]
pub(crate) fn read(
    target: &omnitrigger_windows::ScriptTarget,
    token: &CancellationToken,
    deadline: Option<Instant>,
) -> Result<Value, ActionError> {
    let (captured, expected, end) = crate::read_once_helper::capture(
        target,
        token,
        deadline,
        FLAG,
        "explorer",
        fault,
        |target, budget_ms| json!({"v":1,"budget_ms":budget_ms,"target":target}),
    )?;
    let value = decode(captured, &expected)?;
    crate::read_once_helper::check_live(target, token, end).map_err(fault)?;
    Ok(value)
}
#[cfg(any(windows, test))]
const SELECTION_FAILURE_STAGE: &str = "selection_unavailable_after_directory";
fn explicit_selection_failure_stage<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<bool, D::Error> {
    if bool::deserialize(deserializer)? {
        Ok(true)
    } else {
        Err(serde::de::Error::custom(
            "selection_failure_stage 只允许 true；默认应省略",
        ))
    }
}
fn is_false(value: &bool) -> bool {
    !*value
}
#[cfg(any(windows, test))]
fn decode_selection_failure_stage(
    captured: omnitrigger_windows::ProcessCaptureResult,
    expected: &Value,
) -> Result<Value, ActionError> {
    if captured.code == 0 && captured.stderr.is_empty() && captured.stdout.len() <= MAX_REPLY {
        if let Ok(Reply::Error { v: 1, code }) = serde_json::from_str::<Reply>(&captured.stdout) {
            if code == SELECTION_FAILURE_STAGE {
                return Err(fault(SELECTION_FAILURE_STAGE));
            }
        }
    }
    // 所有旧结果及坏回执沿用原解码，默认解码器仍拒绝新码。
    decode(captured, expected)
}
#[cfg(any(windows, test))]
fn finish_selection_failure_stage(
    result: Result<Value, ActionError>,
    check_live: impl FnOnce() -> Result<(), &'static str>,
) -> Result<Value, ActionError> {
    if result.is_ok()
        || result
            .as_ref()
            .is_err_and(|error| error.code == SELECTION_FAILURE_STAGE)
    {
        check_live().map_err(fault)?;
    }
    result
}
#[cfg(windows)]
pub(crate) fn read_with_selection_failure_stage(
    target: &omnitrigger_windows::ScriptTarget,
    token: &CancellationToken,
    deadline: Option<Instant>,
    selection_failure_stage: bool,
) -> Result<Value, ActionError> {
    if !selection_failure_stage {
        return read(target, token, deadline);
    }
    let (captured, expected, end) = crate::read_once_helper::capture(
        target,
        token,
        deadline,
        FLAG,
        "explorer",
        fault,
        |target, budget_ms| json!({"v":1,"budget_ms":budget_ms,"target":target,"selection_failure_stage":true}),
    )?;
    finish_selection_failure_stage(decode_selection_failure_stage(captured, &expected), || {
        crate::read_once_helper::check_live(target, token, end)
    })
}

fn parse(args: &[String]) -> Option<Result<Request, ()>> {
    if !args.iter().skip(1).any(|arg| {
        arg == FLAG
            || arg
                .strip_prefix(FLAG)
                .is_some_and(|tail| tail.starts_with('='))
    }) {
        return None;
    }
    if args.len() != 3 || args[1] != FLAG || args[2].len() > MAX_REQUEST {
        return Some(Err(()));
    }
    Some(
        serde_json::from_str::<Request>(&args[2])
            .map_err(|_| ())
            .and_then(|r| {
                if r.v == 1 && (1..=MAX_MS as u32).contains(&r.budget_ms) {
                    Ok(r)
                } else {
                    Err(())
                }
            }),
    )
}
#[cfg(windows)]
fn child(request: Request) -> Reply {
    let fail = |code: &str| Reply::Error {
        v: 1,
        code: code.into(),
    };
    if !crate::read_once_helper::owned_capture_child() {
        return fail("explorer_helper_scope_rejected");
    }
    let Some(now) = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|v| u64::try_from(v.as_millis()).ok())
    else {
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
    let Ok(target) = omnitrigger_windows::ScriptTarget::from_snapshot(snapshot) else {
        return fail("target_changed");
    };
    let deadline = Instant::now() + Duration::from_millis(budget);
    let result = if request.selection_failure_stage {
        target.read_explorer_context_with_selection_failure_stage_if(deadline)
    } else {
        target.read_explorer_context_if(deadline)
    };
    match result {
        Ok(value) => Reply::Ok {
            v: 1,
            target: request.target,
            current_directory: value.current_directory,
            selected_paths: value.selected_paths,
        },
        Err(error) => fail(error.code()),
    }
}
#[cfg(not(windows))]
fn child(_: Request) -> Reply {
    Reply::Error {
        v: 1,
        code: "unsupported_target".into(),
    }
}
pub(crate) fn cli(args: &[String]) -> Option<i32> {
    parse(args).map(|request| {
        let Ok(request) = request else {
            return 2;
        };
        let Ok(mut bytes) = serde_json::to_vec(&child(request)) else {
            return 2;
        };
        if bytes.len() + 1 > MAX_REPLY {
            return 2;
        }
        bytes.push(b'\n');
        let mut out = std::io::stdout().lock();
        if out.write_all(&bytes).and_then(|()| out.flush()).is_ok() {
            0
        } else {
            2
        }
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn expected() -> Value {
        json!({"window_handle":"101","process_id":10,"thread_id":11,"process_created_file_time":"123","expires_at_unix_ms":1000})
    }
    fn captured(value: Value) -> omnitrigger_windows::ProcessCaptureResult {
        omnitrigger_windows::ProcessCaptureResult {
            code: 0,
            stdout: value.to_string(),
            stderr: String::new(),
        }
    }
    fn reply(paths: Value) -> Value {
        json!({"status":"ok","v":1,"target":expected(),"current_directory":null,"selected_paths":paths})
    }
    #[test]
    fn real_decoder_empty_selection_does_not_invent_current_directory() {
        assert_eq!(
            decode(captured(reply(json!([]))), &expected()).unwrap(),
            json!({"currentDirectory":null,"selectedPaths":[]})
        );
        let full = "D:\\fixture\\长文件名 😀.txt";
        assert_eq!(
            decode(captured(reply(json!([full]))), &expected()).unwrap()["selectedPaths"][0],
            full
        );
    }
    #[test]
    fn real_decoder_rejects_identity_extra_fields_and_partial_output() {
        let mut stale = reply(json!([]));
        stale["target"]["process_id"] = json!(20);
        assert_eq!(
            decode(captured(stale), &expected()).unwrap_err().code,
            "invalid_response"
        );
        let mut extra = reply(json!([]));
        extra["truncated"] = json!(true);
        assert_eq!(
            decode(captured(extra), &expected()).unwrap_err().code,
            "invalid_response"
        );
        let mut missing = reply(json!([]));
        missing.as_object_mut().unwrap().remove("current_directory");
        assert_eq!(
            decode(captured(missing), &expected()).unwrap_err().code,
            "invalid_response"
        );
        let mut failed = captured(reply(json!([])));
        failed.stderr = "synthetic".into();
        assert_eq!(
            decode(failed, &expected()).unwrap_err().code,
            "explorer_helper_failed"
        );
        assert_eq!(
            decode(captured(reply(json!(vec!["D:\\x"; 33]))), &expected())
                .unwrap_err()
                .code,
            "selection_too_large"
        );
    }
    #[test]
    fn real_options_and_cli_parser_refuse_target_override_or_mixed_launch() {
        assert!(options(json!({})).is_ok());
        for params in [
            json!({"hwnd":"1"}),
            json!([]),
            json!(null),
            json!({"truncate":true}),
        ] {
            assert_eq!(options(params).unwrap_err().code, "invalid_input");
        }
        let data = json!({"v":1,"target":expected(),"budget_ms":1000}).to_string();
        assert!(parse(&["self.exe".into(), FLAG.into(), data.clone()])
            .unwrap()
            .is_ok());
        assert!(parse(&[
            "self.exe".into(),
            FLAG.into(),
            data,
            "--config".into(),
            "never-read".into()
        ])
        .unwrap()
        .is_err());
        assert!(parse(&["self.exe".into(), format!("{FLAG}=bad")])
            .unwrap()
            .is_err());
    }
}

#[cfg(test)]
mod selection_failure_stage_tests {
    use super::*;
    use std::cell::Cell;

    fn target() -> Value {
        json!({"window_handle":"101","process_id":10,"thread_id":11,"process_created_file_time":"123","expires_at_unix_ms":1000})
    }
    fn captured(value: Value) -> omnitrigger_windows::ProcessCaptureResult {
        omnitrigger_windows::ProcessCaptureResult {
            code: 0,
            stdout: value.to_string(),
            stderr: String::new(),
        }
    }
    fn stage_error() -> Value {
        json!({"v":1,"status":"error","code":SELECTION_FAILURE_STAGE})
    }
    #[test]
    fn private_wire_rejects_explicit_false_or_malformed_opt_in() {
        for value in [
            json!(false),
            json!(null),
            json!(1),
            json!("true"),
            json!([]),
        ] {
            let request =
                json!({"v":1,"target":target(),"budget_ms":1000,"selection_failure_stage":value});
            assert!(serde_json::from_value::<Request>(request.clone()).is_err());
            assert!(
                parse(&["self.exe".into(), FLAG.into(), request.to_string()])
                    .unwrap()
                    .is_err()
            );
        }
    }

    #[test]
    fn native_options_accept_only_empty_default_or_exact_true_opt_in() {
        assert!(!options(json!({})).unwrap());
        assert!(options(json!({"selection_failure_stage":true})).unwrap());
        for params in [
            json!({"selection_failure_stage":false}),
            json!({"selection_failure_stage":null}),
            json!({"selection_failure_stage":1}),
            json!({"selection_failure_stage":"true"}),
            json!({"selection_failure_stage":true,"hwnd":"101"}),
            json!({"selectionFailureStage":true}),
        ] {
            assert_eq!(options(params).unwrap_err().code, "invalid_input");
        }
        let value = json!({"v":1,"target":target(),"budget_ms":1000});
        let default: Request = serde_json::from_value(value.clone()).unwrap();
        assert!(!default.selection_failure_stage);
        assert_eq!(serde_json::to_value(default).unwrap(), value);
        let mut opted = value;
        opted["selection_failure_stage"] = json!(true);
        let decoded: Request = serde_json::from_value(opted.clone()).unwrap();
        assert!(decoded.selection_failure_stage);
        assert_eq!(serde_json::to_value(decoded).unwrap(), opted);
    }
    #[test]
    fn default_decoder_rejects_new_code_and_opt_in_still_rejects_bad_envelopes() {
        assert_eq!(
            decode(captured(stage_error()), &target()).unwrap_err().code,
            "invalid_response"
        );
        let error = decode_selection_failure_stage(captured(stage_error()), &target()).unwrap_err();
        assert_eq!(error.code, SELECTION_FAILURE_STAGE);
        assert_eq!(error.method.as_deref(), Some("explorer.context"));
        assert_eq!(error.effect_state.as_deref(), Some("not_started"));
        let mut extra = stage_error();
        extra["current_directory"] = json!("D:\\fixture");
        let mut bad_version = stage_error();
        bad_version["v"] = json!(2);
        for value in [extra, bad_version] {
            assert_eq!(
                decode_selection_failure_stage(captured(value), &target())
                    .unwrap_err()
                    .code,
                "invalid_response"
            );
        }
        let mut bad_process = captured(stage_error());
        bad_process.stderr = "synthetic".into();
        assert_eq!(
            decode_selection_failure_stage(bad_process, &target())
                .unwrap_err()
                .code,
            "explorer_helper_failed"
        );
        let success = json!({"v":1,"status":"ok","target":target(),"current_directory":"D:\\fixture","selected_paths":[]});
        assert_eq!(
            decode_selection_failure_stage(captured(success.clone()), &target()).unwrap(),
            decode(captured(success), &target()).unwrap()
        );
        assert_eq!(
            decode_selection_failure_stage(
                captured(json!({"v":1,"status":"error","code":"selection_unavailable"})),
                &target()
            )
            .unwrap_err()
            .code,
            "selection_unavailable"
        );
    }
    #[test]
    fn parent_live_check_overrides_stage_proof_but_keeps_ordinary_failures() {
        for code in ["cancelled", "timeout", "target_changed"] {
            let calls = Cell::new(0);
            let error = finish_selection_failure_stage(Err(fault(SELECTION_FAILURE_STAGE)), || {
                calls.set(calls.get() + 1);
                Err(code)
            })
            .unwrap_err();
            assert_eq!(calls.get(), 1);
            assert_eq!(error.code, code);
        }
        let calls = Cell::new(0);
        let error = finish_selection_failure_stage(Err(fault(SELECTION_FAILURE_STAGE)), || {
            calls.set(calls.get() + 1);
            Ok(())
        })
        .unwrap_err();
        assert_eq!(error.code, SELECTION_FAILURE_STAGE);
        assert_eq!(calls.get(), 1);
        let error = finish_selection_failure_stage(Err(fault("selection_unavailable")), || {
            panic!("普通错误不可新加复核")
        })
        .unwrap_err();
        assert_eq!(error.code, "selection_unavailable");
        let value = finish_selection_failure_stage(
            Ok(json!({"currentDirectory":null,"selectedPaths":[]})),
            || Ok(()),
        )
        .unwrap();
        assert_eq!(value, json!({"currentDirectory":null,"selectedPaths":[]}));
    }
}

#[cfg(all(test, windows))]
mod native_acceptance {
    use super::*;
    #[test]
    #[ignore = "Root显式提供原目标快照、期望路径；真实Shell COM，不能自动运行"]
    fn exact_original_explorer_via_actual_platform_function() {
        // 这里只验真实平台函数；helper进程/Job收尾须Root另走实际应用入口验收。
        let supplied =
            std::env::var("OMNITRIGGER_EXPLORER_ORIGINAL_CONTEXT_JSON").expect("需显式原目标快照");
        let expected: Value = serde_json::from_str(
            &std::env::var("OMNITRIGGER_EXPLORER_EXPECTED_JSON").expect("需显式期望上下文"),
        )
        .unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let snapshot = crate::action_target_context::validate_value(
            serde_json::from_str(&supplied).unwrap(),
            now,
        )
        .unwrap();
        let target = omnitrigger_windows::ScriptTarget::from_snapshot(snapshot).unwrap();
        let value = target
            .read_explorer_context_if(Instant::now() + Duration::from_millis(MAX_MS))
            .unwrap();
        assert_eq!(
            json!({"currentDirectory":value.current_directory,"selectedPaths":value.selected_paths}),
            expected
        );
    }
}
