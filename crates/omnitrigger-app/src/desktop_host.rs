//! 应用向脚本提供的按次桌面上下文；不在 Hook 中读取进程或发送输入。
use omnitrigger_actions::{ActionError, CancellationToken, NativeHost};
use omnitrigger_windows::{
    capture_shortcut_target, ClipboardData, ClipboardDataError, ClipboardDataFormat,
    ClipboardDataSnapshot, ScriptTarget, ScriptTargetSnapshot, SelectionCopyOptions,
    SelectionDataError, SelectionDataSnapshot, ShortcutTarget,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Instant;

pub struct DesktopHost {
    target: Result<ScriptTarget, ActionError>,
}

fn error(code: &str, message: impl ToString, effect: &str) -> ActionError {
    let mut error = ActionError::new(code, message.to_string());
    error.effect_state = Some(effect.into());
    error
}

impl DesktopHost {
    fn copy_data(
        &self,
        params: Value,
        cancellation: &CancellationToken,
        deadline: Option<Instant>,
    ) -> Result<Value, ActionError> {
        let method = "selection.copy_data";
        let options = copy_data_options(params)?;
        data_check(method, cancellation, deadline, "not_started")?;
        let target = self
            .target
            .as_ref()
            .map_err(|error| data_error(method, &error.code, "not_started"))?;
        target
            .ensure_foreground()
            .map_err(|_| data_error(method, "target_changed", "not_started"))?;
        data_check(method, cancellation, deadline, "not_started")?;
        let result = target.copy_selection_data_if(options, || {
            data_check(method, cancellation, deadline, "not_started").map_err(|error| error.message)
        });
        copy_data_result(result, cancellation, deadline)
    }

    /// 选择窗口只可恢复本次捕获且身份仍有效的目标，绝不重新抓取前台。
    pub fn choice_target(&self) -> Option<ScriptTargetSnapshot> {
        self.target.as_ref().ok()?.snapshot().ok()
    }

    pub fn from_snapshot(snapshot: ScriptTargetSnapshot) -> Result<Self, ActionError> {
        Ok(Self {
            target: Ok(ScriptTarget::from_snapshot(snapshot)
                .map_err(|message| error("target_unavailable", message, "not_started"))?),
        })
    }

    pub fn capture(target: Option<ShortcutTarget>) -> Self {
        // 捕获失败只影响桌面调用，浏览器或纯代码动作仍可运行。
        let target = target
            .map(Ok)
            .unwrap_or_else(capture_shortcut_target)
            .and_then(ScriptTarget::capture)
            .map_err(|message| error("target_unavailable", message, "not_started"));
        Self { target }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ShortcutArgs {
    keys: Vec<u16>,
    #[serde(default)]
    key_down_delay_ms: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SequenceArgs {
    modifiers: Vec<u16>,
    keys: Vec<u16>,
    #[serde(default)]
    key_down_delay_ms: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TextArgs {
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClipboardWriteArgs {
    text: String,
    #[serde(default)]
    exclude_from_history: bool,
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct CopyDataArgs {
    format: String,
    timeout_ms: u32,
    poll_interval_ms: u32,
    settle_ms: u32,
    allow_unchanged: bool,
}
impl Default for CopyDataArgs {
    fn default() -> Self {
        let options = SelectionCopyOptions::default();
        Self {
            format: "auto".into(),
            timeout_ms: options.timeout_ms,
            poll_interval_ms: options.poll_interval_ms,
            settle_ms: options.settle_ms,
            allow_unchanged: options.allow_unchanged,
        }
    }
}

fn copy_data_options(params: Value) -> Result<SelectionCopyOptions, ActionError> {
    let invalid = || data_error("selection.copy_data", "invalid_input", "not_started");
    // serde 的结构体也可从数组反序列化；wire 契约只接受对象。
    if !params.is_object() {
        return Err(invalid());
    }
    let args: CopyDataArgs = serde_json::from_value(params).map_err(|_| invalid())?;
    if !(1..=5000).contains(&args.timeout_ms)
        || !(1..=1000).contains(&args.poll_interval_ms)
        || args.poll_interval_ms > args.timeout_ms
        || args.settle_ms > 1000
    {
        return Err(invalid());
    }
    let format = match args.format.as_str() {
        "auto" => ClipboardDataFormat::Auto,
        "text" => ClipboardDataFormat::Text,
        "files" => ClipboardDataFormat::Files,
        _ => return Err(invalid()),
    };
    let options = SelectionCopyOptions {
        format,
        timeout_ms: args.timeout_ms,
        poll_interval_ms: args.poll_interval_ms,
        settle_ms: args.settle_ms,
        allow_unchanged: args.allow_unchanged,
    };
    options.validate().map_err(|_| invalid())?;
    Ok(options)
}

fn data_error(method: &str, code: &str, effect: &str) -> ActionError {
    let code = if !code.is_empty()
        && code.len() <= 64
        && code
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        code
    } else if method == "clipboard.read_data" {
        "clipboard_failed"
    } else {
        "selection_failed"
    };
    let message = match code {
        "invalid_input" if method == "clipboard.read_data" => "读取剪贴板数据只接受空对象参数。",
        "invalid_input" => "选区复制参数无效，需要有效等待时限、轮询间隔、稳定等待和布尔选项。",
        "cancelled" => "动作已取消。",
        "timeout" => "动作超过运行时限。",
        "target_unavailable" => "原目标不可用。",
        "target_changed" => "原目标已变化或不再位于前台。",
        "unsupported_clipboard_format" => "剪贴板没有支持的文本或文件列表格式。",
        "selection_unchanged" => "等待结束时剪贴板序号未改变。",
        "selection_busy" => "另一次输入提交尚未结束。",
        "invalid_clipboard_data" => "剪贴板数据不符合文本或文件列表的边界要求。",
        _ if method == "clipboard.read_data" => "无法读取剪贴板数据。",
        _ => "无法复制并读取选区数据。",
    };
    let effect = if method == "clipboard.read_data" || effect == "not_started" {
        "not_started"
    } else {
        "unknown"
    };
    let mut error = error(code, message, effect);
    error.method = Some(method.into());
    error
}

fn data_check(
    method: &str,
    cancellation: &CancellationToken,
    deadline: Option<Instant>,
    effect: &str,
) -> Result<(), ActionError> {
    if cancellation.is_cancelled() {
        return Err(data_error(method, "cancelled", effect));
    }
    if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
        return Err(data_error(method, "timeout", effect));
    }
    Ok(())
}

fn clipboard_data_json(
    snapshot: ClipboardDataSnapshot,
    method: &str,
    effect: &str,
) -> Result<Value, ActionError> {
    let invalid = || data_error(method, "invalid_clipboard_data", effect);
    match snapshot.data {
        ClipboardData::Text(text) => {
            if text.encode_utf16().take(65537).count() > 65536 || text.contains('\0') {
                return Err(invalid());
            }
            Ok(json!({"kind":"text","text":text,"sequenceNumber":snapshot.sequence_number}))
        }
        ClipboardData::Files(paths) => {
            if paths.len() > 64 {
                return Err(invalid());
            }
            let mut total = 0usize;
            for path in &paths {
                let length = path.encode_utf16().take(65537).count();
                if path.is_empty() || length > 65536 - total || path.contains('\0') {
                    return Err(invalid());
                }
                total += length;
            }
            Ok(json!({"kind":"files","paths":paths,"sequenceNumber":snapshot.sequence_number}))
        }
    }
}

/// 本接缝也用于测试；有效参数用假 reader，绝不依赖交互剪贴板或前台窗口。
fn read_data_with(
    params: Value,
    cancellation: &CancellationToken,
    deadline: Option<Instant>,
    reader: impl FnOnce() -> Result<ClipboardDataSnapshot, ClipboardDataError>,
) -> Result<Value, ActionError> {
    let method = "clipboard.read_data";
    if params != json!({}) {
        return Err(data_error(method, "invalid_input", "not_started"));
    }
    data_check(method, cancellation, deadline, "not_started")?;
    let result = reader();
    data_check(method, cancellation, deadline, "not_started")?;
    result
        .map_err(|error| data_error(method, error.code, "not_started"))
        .and_then(|snapshot| clipboard_data_json(snapshot, method, "not_started"))
}

fn copy_data_result(
    result: Result<SelectionDataSnapshot, SelectionDataError>,
    cancellation: &CancellationToken,
    deadline: Option<Instant>,
) -> Result<Value, ActionError> {
    let method = "selection.copy_data";
    let effect = match &result {
        Ok(_) => "unknown",
        Err(error) => error.effect_state,
    };
    data_check(method, cancellation, deadline, effect)?;
    let snapshot = result.map_err(|error| data_error(method, error.code, error.effect_state))?;
    let mut value = clipboard_data_json(snapshot.clipboard, method, "unknown")?;
    value["changed"] = json!(snapshot.changed);
    Ok(value)
}

fn not_cancelled(cancellation: &CancellationToken) -> Result<(), String> {
    if cancellation.is_cancelled() {
        Err("动作已取消，未继续本机调用。".into())
    } else {
        Ok(())
    }
}

impl NativeHost for DesktopHost {
    fn call(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
    ) -> Result<Value, ActionError> {
        match method {
            "clipboard.read_data" => {
                return read_data_with(
                    params,
                    cancellation,
                    None,
                    omnitrigger_windows::read_clipboard_data,
                )
            }
            "selection.copy_data" => return self.copy_data(params, cancellation, None),
            _ => {}
        }
        if cancellation.is_cancelled() {
            return Err(error(
                "cancelled",
                "动作已取消，未开始本机调用。",
                "not_started",
            ));
        }
        match method {
            "clipboard.read" => {
                if params != json!({}) {
                    return Err(error(
                        "invalid_input",
                        "剪贴板读取不接受参数。",
                        "not_started",
                    ));
                }
                let text = omnitrigger_windows::read_clipboard_text()
                    .map_err(|message| error("clipboard_failed", message, "not_started"))?;
                not_cancelled(cancellation)
                    .map_err(|message| error("cancelled", message, "not_started"))?;
                Ok(json!(text))
            }
            "clipboard.write" => {
                let args: ClipboardWriteArgs = serde_json::from_value(params)
                    .map_err(|message| error("invalid_input", message, "not_started"))?;
                omnitrigger_windows::validate_automation_text(&args.text)
                    .map_err(|message| error("invalid_input", message, "not_started"))?;
                let check = || not_cancelled(cancellation);
                let write_id = if args.exclude_from_history {
                    Some(
                        omnitrigger_windows::write_clipboard_text_excluding_history_if(
                            &args.text, check,
                        )
                        .map_err(|message| error("clipboard_failed", message, "unknown"))?,
                    )
                } else {
                    omnitrigger_windows::write_clipboard_text_if(&args.text, check)
                        .map_err(|message| error("clipboard_failed", message, "unknown"))?;
                    None
                };
                let sequence = omnitrigger_windows::clipboard_sequence_number();
                let mut receipt = json!({"written":true,"sequenceNumber":sequence});
                if let Some(write_id) = write_id {
                    receipt["writeId"] = json!(write_id);
                }
                Ok(receipt)
            }
            "keyboard.text" => {
                let args: TextArgs = serde_json::from_value(params)
                    .map_err(|message| error("invalid_input", message, "not_started"))?;
                omnitrigger_windows::validate_automation_text(&args.text)
                    .map_err(|message| error("invalid_input", message, "not_started"))?;
                let target = self.target.as_ref().map_err(Clone::clone)?;
                target
                    .ensure_foreground()
                    .map_err(|message| error("target_changed", message, "not_started"))?;
                target
                    .send_text_if(&args.text, || not_cancelled(cancellation))
                    .map_err(|message| error("input_failed", message, "unknown"))?;
                Ok(json!({"submitted":true}))
            }
            "selection.copy" => {
                if params != json!({}) {
                    return Err(error(
                        "invalid_input",
                        "复制选区不接受参数。",
                        "not_started",
                    ));
                }
                let target = self.target.as_ref().map_err(Clone::clone)?;
                target
                    .ensure_foreground()
                    .map_err(|message| error("target_changed", message, "not_started"))?;
                let text = target
                    .copy_selection_text_if(|| not_cancelled(cancellation))
                    .map_err(|message| error("selection_failed", message, "unknown"))?;
                Ok(json!(text))
            }
            "target.context" => {
                if params != json!({}) {
                    return Err(error(
                        "invalid_input",
                        "目标上下文导出不接受参数。",
                        "not_started",
                    ));
                }
                let snapshot = self
                    .target
                    .as_ref()
                    .map_err(Clone::clone)?
                    .snapshot()
                    .map_err(|message| error("target_changed", message, "not_started"))?;
                crate::action_target_context::export(snapshot)
                    .map_err(|message| error("target_unavailable", message, "not_started"))
            }
            "target.info" => {
                if params != json!({}) {
                    return Err(error(
                        "invalid_input",
                        "目标信息查询不接受参数。",
                        "not_started",
                    ));
                }
                let info = self
                    .target
                    .as_ref()
                    .map_err(Clone::clone)?
                    .info()
                    .map_err(|message| error("target_changed", message, "not_started"))?;
                Ok(
                    json!({"processName":info.process_name,"windowRole":info.window_role.as_str(),"title":info.title,"processId":info.process_id,"isForeground":info.is_foreground}),
                )
            }
            "keyboard.shortcut" => {
                let action: crate::shortcut::ShortcutAction = if params.get("timing").is_some() {
                    serde_json::from_value(params)
                        .map_err(|message| error("invalid_input", message, "not_started"))?
                } else {
                    let args: ShortcutArgs = serde_json::from_value(params)
                        .map_err(|message| error("invalid_input", message, "not_started"))?;
                    crate::shortcut::ShortcutAction {
                        keys: args.keys,
                        timing: crate::shortcut::KeyboardTiming {
                            key_down_delay_ms: args.key_down_delay_ms,
                            ..Default::default()
                        },
                    }
                };
                action
                    .validate()
                    .map_err(|message| error("invalid_input", message, "not_started"))?;
                let target = self.target.as_ref().map_err(Clone::clone)?;
                target
                    .ensure_foreground()
                    .map_err(|message| error("target_changed", message, "not_started"))?;
                let delay = if action.timing.process_overrides.is_empty() {
                    action.timing.key_down_delay_ms
                } else {
                    let info = target
                        .info()
                        .map_err(|message| error("target_changed", message, "not_started"))?;
                    action
                        .timing
                        .resolve(&info.process_name)
                        .map_err(|message| error("invalid_input", message, "not_started"))?
                };
                if cancellation.is_cancelled() {
                    return Err(error("cancelled", "动作已取消。", "not_started"));
                }
                target
                    .send_shortcut_timed_if(&action.keys, delay, || {
                        if cancellation.is_cancelled() {
                            Err("动作已取消，未提交输入。".into())
                        } else {
                            Ok(())
                        }
                    })
                    .map_err(|message| error("input_failed", message, "unknown"))?;
                Ok(json!({"submitted":true}))
            }
            "window.action" => {
                let action: crate::window_action::WindowAction = serde_json::from_value(params)
                    .map_err(|message| error("invalid_input", message, "not_started"))?;
                let target = self.target.as_ref().map_err(Clone::clone)?;
                target
                    .ensure_foreground()
                    .map_err(|message| error("target_changed", message, "not_started"))?;
                not_cancelled(cancellation)
                    .map_err(|message| error("cancelled", message, "not_started"))?;
                target
                    .window_action_if(action.platform_operation(), || not_cancelled(cancellation))
                    .map_err(|message| error("window_action_failed", message, "unknown"))?;
                Ok(json!({"submitted":true}))
            }
            "keyboard.sequence" => {
                let args: SequenceArgs = serde_json::from_value(params)
                    .map_err(|message| error("invalid_input", message, "not_started"))?;
                omnitrigger_windows::validate_key_sequence(&args.modifiers, &args.keys)
                    .and_then(|_| {
                        omnitrigger_windows::validate_key_timing(
                            args.keys.len(),
                            args.key_down_delay_ms,
                        )
                    })
                    .map_err(|message| error("invalid_input", message, "not_started"))?;
                let target = self.target.as_ref().map_err(Clone::clone)?;
                target
                    .ensure_foreground()
                    .map_err(|message| error("target_changed", message, "not_started"))?;
                target
                    .send_key_sequence_if(
                        &args.modifiers,
                        &args.keys,
                        args.key_down_delay_ms,
                        || not_cancelled(cancellation),
                    )
                    .map_err(|message| error("input_failed", message, "unknown"))?;
                Ok(json!({"submitted":true}))
            }
            _ => Err(error(
                "unknown_method",
                "本机宿主不支持此方法。",
                "not_started",
            )),
        }
    }

    fn call_with_deadline(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<Value, ActionError> {
        match method {
            "clipboard.read_data" => read_data_with(
                params,
                cancellation,
                Some(deadline),
                omnitrigger_windows::read_clipboard_data,
            ),
            "selection.copy_data" => self.copy_data(params, cancellation, Some(deadline)),
            _ => self.call(method, params, cancellation),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 这一组仅使用合成快照/注入 reader，或不可用目标；禁止访问交互剪贴板。
    mod data_adapter {
        use super::*;
        use std::cell::Cell;

        fn host_without_target() -> DesktopHost {
            DesktopHost {
                target: Err(error(
                    "target_unavailable",
                    "synthetic_payload_do_not_echo",
                    "not_started",
                )),
            }
        }

        fn snapshot(data: ClipboardData, sequence_number: u32) -> ClipboardDataSnapshot {
            ClipboardDataSnapshot {
                data,
                sequence_number,
            }
        }

        fn assert_fault(error: ActionError, method: &str, code: &str, effect: &str) {
            assert_eq!(error.method.as_deref(), Some(method));
            assert_eq!(error.code, code);
            assert_eq!(error.effect_state.as_deref(), Some(effect));
            assert!(!error.message.contains("synthetic_payload_do_not_echo"));
        }

        #[test]
        fn copy_data_defaults_and_boundaries_are_pure_and_do_not_touch_a_target() {
            let defaults = copy_data_options(json!({})).unwrap();
            assert_eq!(defaults.format, ClipboardDataFormat::Auto);
            for (format, expected) in [
                ("auto", ClipboardDataFormat::Auto),
                ("text", ClipboardDataFormat::Text),
                ("files", ClipboardDataFormat::Files),
            ] {
                assert_eq!(
                    copy_data_options(json!({"format":format})).unwrap().format,
                    expected
                );
            }
            assert_eq!(
                (
                    defaults.timeout_ms,
                    defaults.poll_interval_ms,
                    defaults.settle_ms,
                    defaults.allow_unchanged
                ),
                (600, 15, 30, false)
            );
            let minimum = copy_data_options(
                json!({"timeout_ms":1,"poll_interval_ms":1,"settle_ms":0,"allow_unchanged":false}),
            )
            .unwrap();
            assert_eq!(
                (
                    minimum.timeout_ms,
                    minimum.poll_interval_ms,
                    minimum.settle_ms
                ),
                (1, 1, 0)
            );
            let maximum = copy_data_options(json!({"timeout_ms":5000,"poll_interval_ms":1000,"settle_ms":1000,"allow_unchanged":true})).unwrap();
            assert_eq!(
                (
                    maximum.timeout_ms,
                    maximum.poll_interval_ms,
                    maximum.settle_ms,
                    maximum.allow_unchanged
                ),
                (5000, 1000, 1000, true)
            );
            // settle 是独立预算，不擅自约束到序号等待 timeout。
            assert!(copy_data_options(
                json!({"timeout_ms":1,"poll_interval_ms":1,"settle_ms":1000})
            )
            .is_ok());
        }

        #[test]
        fn copy_data_invalid_wire_fields_are_rejected_before_unavailable_target() {
            let host = host_without_target();
            let token = CancellationToken::new();
            for params in [
                json!(null),
                json!([]),
                json!(true),
                json!({"extra":true}),
                json!({"format":null}),
                json!({"format":1}),
                json!({"format":""}),
                json!({"format":"AUTO"}),
                json!({"format":{}}),
                json!({"timeout_ms":0}),
                json!({"timeout_ms":5001}),
                json!({"timeout_ms":u64::MAX}),
                json!({"timeout_ms":null}),
                json!({"timeout_ms":"600"}),
                json!({"timeout_ms":1.5}),
                json!({"timeout_ms":-1}),
                json!({"poll_interval_ms":0}),
                json!({"poll_interval_ms":1001}),
                json!({"poll_interval_ms":null}),
                json!({"timeout_ms":1}),
                json!({"timeout_ms":10,"poll_interval_ms":11}),
                json!({"settle_ms":1001}),
                json!({"settle_ms":-1}),
                json!({"settle_ms":false}),
                json!({"settle_ms":null}),
                json!({"allow_unchanged":null}),
                json!({"allow_unchanged":1}),
                json!({"allow_unchanged":"false"}),
                json!({"timeoutMs":600}),
                json!({"pollIntervalMs":15}),
                json!({"settleMs":30}),
                json!({"allowUnchanged":true}),
            ] {
                assert_fault(
                    host.call("selection.copy_data", params, &token)
                        .unwrap_err(),
                    "selection.copy_data",
                    "invalid_input",
                    "not_started",
                );
            }
            token.cancel();
            assert_fault(
                host.call(
                    "selection.copy_data",
                    json!({"allow_unchanged":null}),
                    &token,
                )
                .unwrap_err(),
                "selection.copy_data",
                "invalid_input",
                "not_started",
            );
        }

        #[test]
        fn copy_data_valid_options_cancel_and_deadline_do_not_fall_back_to_foreground() {
            let host = host_without_target();
            let token = CancellationToken::new();
            for params in [
                json!({}),
                json!({"allow_unchanged":true}),
                json!({"timeout_ms":400,"poll_interval_ms":10,"settle_ms":0}),
            ] {
                assert_fault(
                    host.call("selection.copy_data", params, &token)
                        .unwrap_err(),
                    "selection.copy_data",
                    "target_unavailable",
                    "not_started",
                );
            }
            assert_fault(
                host.call_with_deadline("selection.copy_data", json!({}), &token, Instant::now())
                    .unwrap_err(),
                "selection.copy_data",
                "timeout",
                "not_started",
            );
            token.cancel();
            assert_fault(
                host.call("selection.copy_data", json!({}), &token)
                    .unwrap_err(),
                "selection.copy_data",
                "cancelled",
                "not_started",
            );
        }

        #[test]
        fn read_data_strict_params_do_not_call_the_injected_reader() {
            for params in [
                json!(null),
                json!([]),
                json!(true),
                json!({"text":"synthetic_payload_do_not_echo"}),
                json!({"sequenceNumber":0}),
            ] {
                let result = read_data_with(params, &CancellationToken::new(), None, || {
                    panic!("非法参数不得调用 reader")
                });
                assert_fault(
                    result.unwrap_err(),
                    "clipboard.read_data",
                    "invalid_input",
                    "not_started",
                );
            }
        }

        #[test]
        fn read_data_injected_snapshots_keep_empty_text_files_and_arbitrary_path_data() {
            let token = CancellationToken::new();
            let called = Cell::new(0);
            let empty = read_data_with(json!({}), &token, None, || {
                called.set(called.get() + 1);
                Ok(snapshot(ClipboardData::Text(String::new()), 0))
            })
            .unwrap();
            assert_eq!(empty, json!({"kind":"text","text":"","sequenceNumber":0}));
            let text = "  中文😀\r\n\t";
            let output = read_data_with(json!({}), &token, None, || {
                Ok(snapshot(ClipboardData::Text(text.into()), u32::MAX))
            })
            .unwrap();
            assert_eq!(
                output,
                json!({"kind":"text","text":text,"sequenceNumber":u32::MAX})
            );
            let paths = vec![
                "relative/只是数据".to_string(),
                "C:/OneDrive/not-opened.txt".into(),
                "\\\\server\\share\\not-opened.txt".into(),
                "  literal path  ".into(),
            ];
            let expected = json!({"kind":"files","paths":paths,"sequenceNumber":7});
            let files = read_data_with(json!({}), &token, None, || {
                Ok(snapshot(ClipboardData::Files(paths), 7))
            })
            .unwrap();
            assert_eq!(files, expected);
            let empty_files = read_data_with(json!({}), &token, None, || {
                Ok(snapshot(ClipboardData::Files(vec![]), 0))
            })
            .unwrap();
            assert_eq!(
                empty_files,
                json!({"kind":"files","paths":[],"sequenceNumber":0})
            );
            assert_eq!(called.get(), 1);
        }

        #[test]
        fn data_conversion_accepts_exact_utf16_limits_without_trimming_or_deduplicating() {
            let text = clipboard_data_json(
                snapshot(ClipboardData::Text("😀".repeat(32768)), u32::MAX),
                "clipboard.read_data",
                "not_started",
            )
            .unwrap();
            assert_eq!(text["text"].as_str().unwrap().encode_utf16().count(), 65536);
            let files = clipboard_data_json(
                snapshot(ClipboardData::Files(vec!["😀".repeat(512); 64]), 0),
                "clipboard.read_data",
                "not_started",
            )
            .unwrap();
            assert_eq!(files["paths"].as_array().unwrap().len(), 64);
            let whitespace = clipboard_data_json(
                snapshot(ClipboardData::Files(vec![" ".into()]), 0),
                "clipboard.read_data",
                "not_started",
            )
            .unwrap();
            assert_eq!(whitespace["paths"][0], " ");
        }

        #[test]
        fn data_conversion_refuses_invalid_shape_and_keeps_copy_effect_unknown() {
            for (method, effect) in [
                ("clipboard.read_data", "not_started"),
                ("selection.copy_data", "unknown"),
            ] {
                for data in [
                    ClipboardData::Text("x".repeat(65537)),
                    ClipboardData::Text("a\0b".into()),
                    ClipboardData::Files(vec![String::new()]),
                    ClipboardData::Files(vec!["a\0b".into()]),
                    ClipboardData::Files(vec!["p".into(); 65]),
                    ClipboardData::Files(vec!["a".repeat(32768), "b".repeat(32769)]),
                ] {
                    assert_fault(
                        clipboard_data_json(snapshot(data, 0), method, effect).unwrap_err(),
                        method,
                        "invalid_clipboard_data",
                        effect,
                    );
                }
            }
        }

        #[test]
        fn read_data_cancel_deadline_and_native_errors_never_expose_payload() {
            let token = CancellationToken::new();
            token.cancel();
            assert_fault(
                read_data_with(json!({}), &token, None, || panic!("取消后不得读剪贴板"))
                    .unwrap_err(),
                "clipboard.read_data",
                "cancelled",
                "not_started",
            );
            assert_fault(
                read_data_with(
                    json!({}),
                    &CancellationToken::new(),
                    Some(Instant::now()),
                    || panic!("超时后不得读剪贴板"),
                )
                .unwrap_err(),
                "clipboard.read_data",
                "timeout",
                "not_started",
            );
            let token = CancellationToken::new();
            assert_fault(
                read_data_with(json!({}), &token, None, || {
                    token.cancel();
                    Ok(snapshot(
                        ClipboardData::Text("synthetic_payload_do_not_echo".into()),
                        1,
                    ))
                })
                .unwrap_err(),
                "clipboard.read_data",
                "cancelled",
                "not_started",
            );
            for code in [
                "unsupported_clipboard_format",
                "clipboard_busy",
                "invalid_clipboard_data",
            ] {
                assert_fault(
                    read_data_with(json!({}), &CancellationToken::new(), None, || {
                        Err(ClipboardDataError {
                            code,
                            message: "synthetic_payload_do_not_echo".into(),
                        })
                    })
                    .unwrap_err(),
                    "clipboard.read_data",
                    code,
                    "not_started",
                );
            }
        }

        #[test]
        fn copy_data_result_keeps_changed_and_native_stage_without_text_error_matching() {
            for (data, changed, sequence) in [
                (ClipboardData::Text(String::new()), false, 0),
                (ClipboardData::Files(vec![]), true, u32::MAX),
            ] {
                let value = copy_data_result(
                    Ok(SelectionDataSnapshot {
                        clipboard: snapshot(data, sequence),
                        changed,
                    }),
                    &CancellationToken::new(),
                    None,
                )
                .unwrap();
                assert_eq!(value["changed"], changed);
                assert_eq!(value["sequenceNumber"], sequence);
                assert_eq!(value.as_object().unwrap().len(), 4);
            }
            for (code, effect) in [
                ("selection_unchanged", "unknown"),
                ("selection_busy", "not_started"),
                ("target_changed", "unknown"),
                ("operation_guard_failed", "unknown"),
            ] {
                let result = Err(SelectionDataError {
                    code,
                    message: "动作已取消 synthetic_payload_do_not_echo".into(),
                    effect_state: effect,
                });
                assert_fault(
                    copy_data_result(result, &CancellationToken::new(), None).unwrap_err(),
                    "selection.copy_data",
                    code,
                    effect,
                );
            }
            for effect in ["not_started", "unknown"] {
                let token = CancellationToken::new();
                token.cancel();
                let result = Err(SelectionDataError {
                    code: "operation_guard_failed",
                    message: "synthetic_payload_do_not_echo".into(),
                    effect_state: effect,
                });
                assert_fault(
                    copy_data_result(result, &token, None).unwrap_err(),
                    "selection.copy_data",
                    "cancelled",
                    effect,
                );
                let result = Err(SelectionDataError {
                    code: "operation_guard_failed",
                    message: "synthetic_payload_do_not_echo".into(),
                    effect_state: effect,
                });
                assert_fault(
                    copy_data_result(result, &CancellationToken::new(), Some(Instant::now()))
                        .unwrap_err(),
                    "selection.copy_data",
                    "timeout",
                    effect,
                );
            }
            let token = CancellationToken::new();
            token.cancel();
            let result = Ok(SelectionDataSnapshot {
                clipboard: snapshot(
                    ClipboardData::Text("synthetic_payload_do_not_echo".into()),
                    1,
                ),
                changed: true,
            });
            assert_fault(
                copy_data_result(result, &token, None).unwrap_err(),
                "selection.copy_data",
                "cancelled",
                "unknown",
            );
        }
    }

    #[test]
    fn target_context_export_keeps_original_hidden_window_identity() {
        use windows::{
            core::w,
            Win32::UI::WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WINDOW_STYLE,
            },
        };
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("Owned context export"),
                WINDOW_STYLE(0),
                0,
                0,
                1,
                1,
                None,
                None,
                None,
                None,
            )
        }
        .unwrap();
        struct Owned(windows::Win32::Foundation::HWND);
        impl Drop for Owned {
            fn drop(&mut self) {
                let _ = unsafe { DestroyWindow(self.0) };
            }
        }
        let owned = Owned(hwnd);
        // 使用本进程真实窗口构造快照，不读取当前前台。
        let mut pid = 0;
        let tid = unsafe {
            windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(hwnd, Some(&mut pid))
        };
        use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
        let (mut created, mut exited, mut kernel, mut user) = Default::default();
        unsafe {
            GetProcessTimes(
                GetCurrentProcess(),
                &mut created,
                &mut exited,
                &mut kernel,
                &mut user,
            )
        }
        .unwrap();
        let snapshot = ScriptTargetSnapshot {
            window_handle: hwnd.0 as usize,
            process_id: pid,
            thread_id: tid,
            process_created_file_time: (u64::from(created.dwHighDateTime) << 32)
                | u64::from(created.dwLowDateTime),
        };
        let host = DesktopHost::from_snapshot(snapshot).unwrap();
        let token = CancellationToken::new();
        for operation in ["toggle_maximize", "minimize", "close"] {
            let error = host
                .call("window.action", json!({"operation":operation}), &token)
                .unwrap_err();
            assert_eq!(error.code, "target_changed");
            assert_eq!(error.effect_state.as_deref(), Some("not_started"));
        }
        let value = host.call("target.context", json!({}), &token).unwrap();
        assert_eq!(value["window_handle"], snapshot.window_handle.to_string());
        assert_eq!(
            value["process_created_file_time"],
            snapshot.process_created_file_time.to_string()
        );
        assert_eq!(
            host.call("target.context", json!({"other":true}), &token)
                .unwrap_err()
                .code,
            "invalid_input"
        );
        drop(owned);
        assert_eq!(
            host.call("target.context", json!({}), &token)
                .unwrap_err()
                .code,
            "target_changed"
        );
    }

    #[test]
    fn desktop_validation_and_cancellation_do_not_fall_back_to_another_target() {
        let host = DesktopHost {
            target: Err(error(
                "target_unavailable",
                "测试目标不存在。",
                "not_started",
            )),
        };
        let token = CancellationToken::new();
        for (method, params) in [
            (
                "keyboard.shortcut",
                json!({"keys":[65],"key_down_delay_ms":1001}),
            ),
            ("keyboard.sequence", json!({"modifiers":[17],"keys":[65]})),
            ("keyboard.sequence", json!({"modifiers":[162],"keys":[]})),
            (
                "keyboard.sequence",
                json!({"modifiers":[162],"keys":vec![65;16],"key_down_delay_ms":200}),
            ),
            (
                "keyboard.sequence",
                json!({"modifiers":[],"keys":[65],"extra":1}),
            ),
            ("clipboard.read", json!({"unexpected":true})),
            ("window.action", json!({"operation":"maximize"})),
            (
                "window.action",
                json!({"operation":"close","window_handle":"1"}),
            ),
            ("window.action", json!({"operation":null})),
            ("clipboard.write", json!({"text":"a\0b"})),
            (
                "clipboard.write",
                json!({"text":"literal", "exclude_from_history":"true"}),
            ),
            (
                "clipboard.write",
                json!({"text":"literal", "exclude_from_history":null}),
            ),
            (
                "clipboard.write",
                json!({"text":"literal", "excludeFromHistory":true}),
            ),
            (
                "keyboard.text",
                json!({"text":"literal", "exclude_from_history":true}),
            ),
            ("keyboard.text", json!({"text":5})),
            ("keyboard.shortcut", json!({"keys":[163,9],"timing":null})),
            (
                "keyboard.shortcut",
                json!({"keys":[163,9],"timing":{},"key_down_delay_ms":0}),
            ),
            (
                "keyboard.shortcut",
                json!({"keys":[163,9],"timing":{"process_overrides":{"bad/path":10}}}),
            ),
            (
                "keyboard.shortcut",
                json!({"keys":[163,9],"timing":{"process_overrides":{"wps":10,"WPS.exe":20}}}),
            ),
            ("selection.copy", json!([])),
        ] {
            assert_eq!(
                host.call(method, params, &token).unwrap_err().code,
                "invalid_input"
            );
        }
        assert_eq!(
            host.call("keyboard.shortcut", json!({"keys":[65,162]}), &token)
                .unwrap_err()
                .code,
            "invalid_input"
        );
        assert_eq!(
            host.call("target.info", json!({}), &token)
                .unwrap_err()
                .code,
            "target_unavailable"
        );
        assert_eq!(
            host.call("window.action", json!({"operation":"close"}), &token)
                .unwrap_err()
                .code,
            "target_unavailable"
        );
        assert_eq!(
            host.call(
                "keyboard.shortcut",
                json!({"keys":[163,9],"timing":{"process_overrides":{"wps":10}}}),
                &token
            )
            .unwrap_err()
            .code,
            "target_unavailable"
        );
        assert_eq!(
            host.call("target.info", json!({"other":true}), &token)
                .unwrap_err()
                .code,
            "invalid_input"
        );
        assert_eq!(
            host.call("other", json!({}), &token).unwrap_err().code,
            "unknown_method"
        );
        token.cancel();
        assert_eq!(
            host.call("window.action", json!({"operation":"close"}), &token)
                .unwrap_err()
                .code,
            "cancelled"
        );
        assert_eq!(
            host.call("keyboard.shortcut", json!({"keys":[65]}), &token)
                .unwrap_err()
                .code,
            "cancelled"
        );
    }

    /// 显式在本机交互桌面运行；只向本测试创建的输入框注入按键。
    #[test]
    #[ignore = "短暂激活专用输入框，验证 Node 桌面 API 的真实效果"]
    fn interactive_desktop_api_smoke() {
        desktop_line_smoke(
            "node",
            std::sync::Arc::new(omnitrigger_actions::NodeRuntime::default()),
            include_str!("../../../examples/actions/desktop-line-edit/call.json"),
        );
    }

    #[test]
    #[ignore = "短暂激活专用输入框，须显式指定真实轻量宿主，验证脚本行编辑效果"]
    fn interactive_script_line_edit_smoke() {
        let program =
            std::env::var_os("OMNITRIGGER_TEST_SCRIPT_HOST").expect("设置真实脚本宿主路径");
        assert!(std::path::Path::new(&program).is_file());
        desktop_line_smoke(
            "script",
            std::sync::Arc::new(omnitrigger_actions::ScriptHostRuntime::new(program)),
            include_str!("../../../examples/actions/desktop-line-edit/call-script.json"),
        );
    }

    fn desktop_line_smoke(
        runtime_name: &str,
        runtime: std::sync::Arc<dyn omnitrigger_actions::ActionRuntime>,
        call_source: &str,
    ) {
        use omnitrigger_actions::{
            ActionCall, ActionOutcome, ActionService, PackageCatalog, RuntimeRegistry,
        };
        use std::{
            thread,
            time::{Duration, Instant},
        };
        use windows::{
            core::{w, PCWSTR},
            Win32::{
                Foundation::{HWND, LPARAM, WPARAM},
                UI::{
                    Input::KeyboardAndMouse::SetFocus,
                    WindowsAndMessaging::{
                        CreateWindowExW, DestroyWindow, DispatchMessageW, GetForegroundWindow,
                        GetWindowTextW, PeekMessageW, SendMessageW, SetForegroundWindow,
                        SetWindowTextW, ShowWindow, TranslateMessage, MSG, PM_REMOVE,
                        SW_SHOWNOACTIVATE, WINDOW_EX_STYLE, WINDOW_STYLE, WS_BORDER, WS_CHILD,
                        WS_OVERLAPPEDWINDOW, WS_VISIBLE,
                    },
                },
            },
        };

        struct Windows {
            previous: HWND,
            owned: Vec<HWND>,
        }
        impl Drop for Windows {
            fn drop(&mut self) {
                let restore = self.owned.contains(&unsafe { GetForegroundWindow() });
                for window in self.owned.iter().rev() {
                    let _ = unsafe { DestroyWindow(*window) };
                }
                if restore && !self.previous.is_invalid() {
                    let _ = unsafe { SetForegroundWindow(self.previous) };
                }
            }
        }
        fn pump() {
            let mut msg = MSG::default();
            while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
                let _ = unsafe { TranslateMessage(&msg) };
                unsafe { DispatchMessageW(&msg) };
            }
        }
        fn text(edit: HWND) -> String {
            let mut buffer = [0u16; 32];
            let count = unsafe { GetWindowTextW(edit, &mut buffer) }.max(0) as usize;
            String::from_utf16(&buffer[..count]).unwrap()
        }
        fn selected(edit: HWND) -> bool {
            let (mut first, mut last) = (0u32, 0u32);
            unsafe {
                SendMessageW(
                    edit,
                    0x00b0,
                    Some(WPARAM((&mut first as *mut u32) as usize)),
                    Some(LPARAM((&mut last as *mut u32) as isize)),
                )
            };
            (first, last) == (0, 4)
        }

        let mut windows = Windows {
            previous: unsafe { GetForegroundWindow() },
            owned: vec![],
        };
        let parent = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("OmniTrigger 脚本 API 实测"),
                WS_OVERLAPPEDWINDOW,
                100,
                100,
                500,
                160,
                None,
                None,
                None,
                None,
            )
        }
        .unwrap();
        windows.owned.push(parent);
        let edit = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("EDIT"),
                w!("abcd"),
                WS_CHILD | WS_VISIBLE | WS_BORDER | WINDOW_STYLE(0x0044),
                20,
                35,
                440,
                80,
                Some(parent),
                None,
                None,
                None,
            )
        }
        .unwrap();
        let decoy = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("失焦验证：不应收到输入"),
                WS_OVERLAPPEDWINDOW,
                120,
                120,
                500,
                160,
                None,
                None,
                None,
                None,
            )
        }
        .unwrap();
        windows.owned.push(decoy);
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/actions");
        let catalog = PackageCatalog::load(&root);
        let mut registry = RuntimeRegistry::new();
        registry.register(runtime_name, runtime).unwrap();
        let service = ActionService::new(catalog, registry);
        let call: ActionCall = serde_json::from_str(call_source).unwrap();
        let mut evidence = Vec::new();
        for (mode, before, cursor, expected) in [
            ("success", "abcd", 4, ""),
            (
                "middle_of_line",
                "head\r\nabcDEF\r\ntail",
                9,
                "head\r\nDEF\r\ntail",
            ),
            (
                "end_of_line",
                "head\r\nabcDEF\r\ntail",
                12,
                "head\r\n\r\ntail",
            ),
            (
                "start_of_line",
                "head\r\nabcDEF\r\ntail",
                6,
                "headabcDEF\r\ntail",
            ),
            ("focus_changed", "abcd", 4, "abcd"),
            ("cancelled", "abcd", 4, "abcd"),
        ] {
            let encoded = before
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect::<Vec<_>>();
            unsafe { SetWindowTextW(edit, PCWSTR(encoded.as_ptr())) }.unwrap();
            let _ = unsafe { ShowWindow(parent, SW_SHOWNOACTIVATE) };
            let _ = unsafe { SetForegroundWindow(parent) };
            // Windows 前台锁可能拒绝进程自行激活；允许操作者激活专用窗口，
            // 等待期间只泵消息，不发送任何输入，也不反复抢占前台。
            if unsafe { GetForegroundWindow() } != parent {
                println!(
                    "等待专用测试窗口前台：pid={} hwnd={}",
                    std::process::id(),
                    parent.0 as usize
                );
                let wait = Instant::now();
                while unsafe { GetForegroundWindow() } != parent
                    && wait.elapsed() < Duration::from_secs(45)
                {
                    pump();
                    thread::sleep(Duration::from_millis(10));
                }
            }
            assert_eq!(
                unsafe { GetForegroundWindow() },
                parent,
                "未获取测试窗口前台，未提交动作"
            );
            let _ = unsafe { SetFocus(Some(edit)) };
            unsafe {
                SendMessageW(
                    edit,
                    0x00b1,
                    Some(WPARAM(cursor)),
                    Some(LPARAM(cursor as isize)),
                )
            };
            let target = capture_shortcut_target().unwrap();
            crate::action_packages::submit(&service, 1, call.clone(), Some(target)).unwrap();
            let began = Instant::now();
            let mut altered = false;
            let outcome = loop {
                pump();
                if !altered && matches!(mode, "focus_changed" | "cancelled") && selected(edit) {
                    if mode == "focus_changed" {
                        let _ = unsafe { ShowWindow(decoy, SW_SHOWNOACTIVATE) };
                        let _ = unsafe { SetForegroundWindow(decoy) };
                        assert_eq!(unsafe { GetForegroundWindow() }, decoy);
                    } else {
                        service.cancel_id(1);
                    }
                    altered = true;
                }
                let expected_foreground = if mode == "focus_changed" && altered {
                    decoy
                } else {
                    parent
                };
                if unsafe { GetForegroundWindow() } != expected_foreground {
                    service.cancel_all();
                    panic!("行编辑实测 {mode} 失焦，已请求取消");
                }
                if let Ok(event) = service.try_recv() {
                    if event.outcome.is_terminal() {
                        break event.outcome;
                    }
                }
                if began.elapsed() > Duration::from_secs(10) {
                    service.cancel_all();
                    panic!("脚本 API 实测超时");
                }
                thread::sleep(Duration::from_millis(1));
            };
            pump();
            match mode {
                "success" | "middle_of_line" | "end_of_line" | "start_of_line" => {
                    assert!(
                        matches!(outcome, ActionOutcome::Succeeded { .. }),
                        "{outcome:?}"
                    );
                    assert_eq!(text(edit), expected);
                }
                "focus_changed" => {
                    assert!(
                        matches!(&outcome, ActionOutcome::Failed { error } if error.code=="target_changed" && error.effect_state.as_deref()==Some("not_started")),
                        "{outcome:?}"
                    );
                    assert_eq!(text(edit), "abcd");
                }
                _ => {
                    assert_eq!(outcome, ActionOutcome::Cancelled);
                    assert_eq!(text(edit), "abcd");
                }
            }
            evidence.push(json!({"case":mode,"cursor":cursor,"expected":expected,"text":text(edit),"outcome":format!("{outcome:?}")}));
        }
        if let Some(path) = std::env::var_os("OMNITRIGGER_DESKTOP_SMOKE_REPORT") {
            std::fs::write(
                path,
                serde_json::to_vec_pretty(&json!({"runtime":runtime_name,"cases":evidence,"clipboard_accessed":false,"scope":"owned_edit_not_physical_trigger_or_original_target_application"})).unwrap(),
            )
            .unwrap();
        }
    }
}
