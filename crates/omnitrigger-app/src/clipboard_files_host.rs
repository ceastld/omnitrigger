//! 文件列表写入宿主；错误不回显路径；Native调用只在动作工作线程。
use omnitrigger_actions::{ActionError, CancellationToken};
use omnitrigger_windows::{validate_clipboard_file_paths, ClipboardFilesError};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
const METHOD: &str = "clipboard.write_files";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    paths: Vec<String>,
}
fn fault(error: ClipboardFilesError) -> ActionError {
    let mut result = ActionError::new(
        error.code,
        "文件列表剪贴板写入未获完整确认；请检查错误代码和效果状态。",
    );
    result.method = Some(METHOD.into());
    result.effect_state = Some(error.effect_state.into());
    result
}
fn check(token: &CancellationToken, deadline: Instant) -> Result<(), ClipboardFilesError> {
    if token.is_cancelled() {
        Err(ClipboardFilesError::before("cancelled"))
    } else if Instant::now() >= deadline {
        Err(ClipboardFilesError::before("timeout"))
    } else {
        Ok(())
    }
}
pub(super) fn execute_with(
    params: Value,
    token: &CancellationToken,
    outer: Option<Instant>,
    write: impl FnOnce(
        &[String],
        &mut dyn FnMut() -> Result<(), ClipboardFilesError>,
    ) -> Result<(), ClipboardFilesError>,
    sequence: impl FnOnce() -> u32,
) -> Result<Value, ActionError> {
    let args: Args = serde_json::from_value(params)
        .map_err(|_| fault(ClipboardFilesError::before("invalid_input")))?;
    validate_clipboard_file_paths(&args.paths).map_err(fault)?;
    let local = Instant::now() + Duration::from_secs(2);
    let deadline = outer.map_or(local, |value| value.min(local));
    check(token, deadline).map_err(fault)?;
    write(&args.paths, &mut || check(token, deadline)).map_err(fault)?;
    check(token, deadline).map_err(|error| fault(error.unknown()))?;
    Ok(json!({"written":true,"sequenceNumber":sequence()}))
}
pub(super) fn execute(
    params: Value,
    token: &CancellationToken,
    deadline: Option<Instant>,
) -> Result<Value, ActionError> {
    #[cfg(windows)]
    {
        execute_with(
            params,
            token,
            deadline,
            |paths, check| omnitrigger_windows::write_clipboard_files_if(paths, check),
            omnitrigger_windows::clipboard_sequence_number,
        )
    }
    #[cfg(not(windows))]
    {
        execute_with(
            params,
            token,
            deadline,
            |_, _| Err(ClipboardFilesError::before("unsupported_target")),
            || 0,
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_cancelled_and_expired_never_write() {
        for params in [
            json!({"paths":[]}),
            json!({"paths":["C:\\a"],"extra":1}),
            json!({"paths":["relative"]}),
        ] {
            let error = execute_with(
                params,
                &CancellationToken::new(),
                None,
                |_, _| panic!("不得写"),
                || 0,
            )
            .unwrap_err();
            assert_eq!(error.code, "invalid_input");
            assert_eq!(error.effect_state.as_deref(), Some("not_started"));
        }
        for cancelled in [false, true] {
            let token = CancellationToken::new();
            if cancelled {
                token.cancel();
            }
            let error = execute_with(
                json!({"paths":["C:\\a"]}),
                &token,
                Some(Instant::now()),
                |_, _| panic!("不得写"),
                || 0,
            )
            .unwrap_err();
            assert_eq!(error.code, if cancelled { "cancelled" } else { "timeout" });
        }
    }
    #[test]
    fn post_write_cancel_is_unknown_and_receipt_contains_no_paths() {
        let token = CancellationToken::new();
        let error = execute_with(
            json!({"paths":["C:\\private"]}),
            &token,
            None,
            |_, check| {
                check()?;
                token.cancel();
                Ok(())
            },
            || 0,
        )
        .unwrap_err();
        assert_eq!(error.effect_state.as_deref(), Some("unknown"));
        assert!(!error.message.contains("private"));
        let result = execute_with(
            json!({"paths":["C:\\合成😀"]}),
            &CancellationToken::new(),
            None,
            |paths, check| {
                assert_eq!(paths.len(), 1);
                check()
            },
            || u32::MAX,
        )
        .unwrap();
        assert_eq!(result, json!({"written":true,"sequenceNumber":u32::MAX}));
    }
}
