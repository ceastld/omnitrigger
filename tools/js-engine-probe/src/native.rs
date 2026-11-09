//! 轻量宿主的同步客户端；本机操作仍由父进程捕获的 NativeHost 执行。
use crate::api::ApiError;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    cell::Cell,
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpStream},
    time::{Duration, Instant},
};

const LIMIT: usize = 512 * 1024;
pub const METHODS: [&str; 50] = [
    "process.capture",
    "system.open",
    "system.open_file_in_parent",
    "actions.stop_other_instances",
    "state.read",
    "state.compare_and_swap",
    "history.list",
    "history.get",
    "history.append",
    "history.remove",
    "fs.stat",
    "fs.read_directory",
    "fs.sort_logical_paths",
    "ui.choose",
    "ui.notify",
    "ui.open_history",
    "ui.toggle_history",
    "ui.action_hint",
    "browser.profiles",
    "browser.call",
    "target.info",
    "target.context",
    "target.capture_foreground",
    "target.keyboard.shortcut",
    "target.keyboard.text",
    "keyboard.shortcut",
    "keyboard.sequence",
    "keyboard.text",
    "win32_edit.replace_selection",
    "raishot.resident.find",
    "raishot.resident.region",
    "raishot.capture_result",
    "raishot.release_result",
    "selection.copy",
    "selection.copy_data",
    "selection.read_text",
    "explorer.context",
    "clipboard.read",
    "clipboard.read_data",
    "clipboard.write",
    "clipboard.write_files",
    "window.action",
    "window.geometry",
    "window.set_rect",
    "pointer.state",
    "pointer.root",
    "target.handoff_pointing_root",
    "window.drag",
    "display.brightness_step",
    "audio.volume_adjust",
];

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    port: u16,
    token: String,
}

fn file_result_method(method: &str) -> bool {
    matches!(method, "raishot.capture_result" | "raishot.release_result")
}
fn result_wait_budget(method: &str, params: &Value) -> Result<Option<Duration>, ApiError> {
    if method == "raishot.capture_result" {
        let timeout = params
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .filter(|value| (1..=30_000).contains(value))
            .ok_or_else(|| {
                fault(
                    "invalid_input",
                    "文件结果超时参数无效。",
                    method,
                    "not_started",
                )
            })?;
        return Ok(Some(Duration::from_millis(timeout + 1500)));
    }
    Ok((method == "raishot.release_result").then(|| Duration::from_secs(5)))
}

fn fault(code: &str, message: impl ToString, method: &str, effect: &str) -> ApiError {
    let logical_sort = method == "fs.sort_logical_paths";
    let stat = matches!(method, "fs.stat" | "fs.read_directory");
    let data_read = matches!(
        method,
        "clipboard.read_data"
            | "explorer.context"
            | "window.geometry"
            | "pointer.state"
            | "pointer.root"
    );

    let data_copy = method == "selection.copy_data";
    let history_read = matches!(method, "history.list" | "history.get");
    let history = matches!(
        method,
        "history.list" | "history.get" | "history.append" | "history.remove"
    );
    ApiError {
        code: (if logical_sort {
            match code {
                "invalid_input" | "cancelled" | "timeout" | "fs_unavailable" => code,
                _ => "fs_unavailable",
            }
        } else if history && code == "native_bridge_unavailable" {
            "history_unavailable"
        } else if stat && code == "native_bridge_unavailable" {
            "fs_unavailable"
        } else if stat && code == "native_bridge_failed" {
            "fs_error"
        } else {
            code
        })
        .into(),
        message: if logical_sort {
            "字符串列表排序失败；请检查错误代码。".into()
        } else if file_result_method(method) {
            "截图文件结果未获确认；不会自动重试。".into()
        } else if method == "process.capture" {
            "程序输出采集未完成，请检查错误代码。".into()
        } else if method == "system.open" || method == "system.open_file_in_parent" {
            "打开请求未获完整确认；不会本地回退或重试。".into()
        } else if method == "ui.action_hint" {
            "动作版本提示更新未确认；请检查错误代码和效果状态。".into()
        } else if matches!(
            method,
            "window.geometry"
                | "window.set_rect"
                | "pointer.state"
                | "pointer.root"
                | "window.drag"
                | "target.handoff_pointing_root"
        ) {
            "窗口矩形调用未获完整确认；请检查错误代码和效果状态。".into()
        } else if method.starts_with("raishot.resident.") {
            "RaiShot 常驻请求未获完整确认；请检查错误代码和效果状态。".into()
        } else if method == "clipboard.write_files" {
            "文件列表剪贴板写入未获完整确认；请检查错误代码和效果状态。".into()
        } else if method == "win32_edit.replace_selection" {
            "Win32 Edit选区替换未获完整确认；请检查错误代码和效果状态。".into()
        } else if method == "audio.volume_adjust" {
            "音量调节未获完整确认；请检查错误代码和效果状态。".into()
        } else if method == "display.brightness_step" {
            "亮度步进未获完整确认；请检查结构化回执。".into()
        } else if stat {
            "本地路径状态查询失败；请检查错误代码。".into()
        } else if history {
            "剪贴板历史调用失败；请检查错误代码。".into()
        } else if data_read || data_copy {
            "剪贴板或选区数据调用失败；请检查错误代码。".into()
        } else {
            message.to_string()
        },
        effect_state:
            Some(
                if logical_sort
                    || code != "window_class_blocked" && (stat || data_read || history_read)
                {
                    "not_started"
                } else {
                    effect
                }
                .into(),
            ),
        method: Some(method.into()),
        required_permissions: Vec::new(),
    }
}

// 有副作用的请求写入尝试前可证明未提交；开始写入后仅保守返回unknown。
fn brightness_transport_effect(
    method: &str,
    write_began: bool,
    fallback: &'static str,
) -> &'static str {
    if method == "fs.sort_logical_paths" {
        return "not_started";
    }
    if matches!(
        method,
        "display.brightness_step"
            | "audio.volume_adjust"
            | "win32_edit.replace_selection"
            | "raishot.resident.find"
            | "raishot.resident.region"
            | "raishot.capture_result"
            | "raishot.release_result"
            | "system.open"
            | "system.open_file_in_parent"
            | "process.capture"
            | "clipboard.write_files"
            | "window.geometry"
            | "window.set_rect"
            | "pointer.state"
            | "pointer.root"
            | "window.drag"
            | "actions.stop_other_instances"
            | "state.read"
            | "state.compare_and_swap"
            | "ui.action_hint"
            | "target.handoff_pointing_root"
    ) && !write_began
    {
        "not_started"
    } else {
        fallback
    }
}

// 回执排空余量不扩张实际采集timeout或外层动作deadline。
fn method_wait_budget(method: &str, params: &Value) -> Result<Duration, ApiError> {
    if let Some(budget) = result_wait_budget(method, params)? {
        return Ok(budget);
    }
    if method == "process.capture" {
        let timeout = match params.get("timeout_ms") {
            None => 10_000,
            Some(value) => value
                .as_u64()
                .filter(|value| (1..=60_000).contains(value))
                .ok_or_else(|| {
                    fault("invalid_input", "采集超时参数无效。", method, "not_started")
                })?,
        };
        return Ok(Duration::from_millis(timeout + 1500));
    }
    Ok(Duration::from_secs(
        if method.starts_with("browser.")
            || method == "ui.choose"
            || method == "system.open"
            || method == "system.open_file_in_parent"
            || matches!(method, "display.brightness_step" | "audio.volume_adjust")
        {
            66
        } else if method == "selection.copy_data" {
            8
        } else {
            5
        },
    ))
}

pub fn call(
    connection: Configuration,
    id: u32,
    method: String,
    params: Value,
) -> Result<Value, ApiError> {
    call_controlled(connection, id, method, params, None)
}

fn call_controlled(
    connection: Configuration,
    id: u32,
    method: String,
    params: Value,
    cancellation: Option<&std::sync::atomic::AtomicBool>,
) -> Result<Value, ApiError> {
    let cancelled =
        || cancellation.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire));
    if cancelled() {
        return Err(fault(
            "cancelled",
            "异步浏览器请求已取消。",
            &method,
            "not_started",
        ));
    }
    if connection.port == 0
        || connection.token.len() != 64
        || !connection.token.bytes().all(|b| b.is_ascii_hexdigit())
        || !(1..=128).contains(&id)
        || !METHODS.contains(&method.as_str())
        || !params.is_object()
    {
        return Err(fault(
            "invalid_input",
            "本机调用配置或参数无效。",
            &method,
            "not_started",
        ));
    }
    if method == "fs.sort_logical_paths" && !cfg!(windows) {
        return Err(fault(
            "fs_unavailable",
            "此平台没有应用逻辑排序能力。",
            &method,
            "not_started",
        ));
    }
    let mut request = serde_json::to_vec(
        &json!({"token":connection.token,"id":id,"method":method,"params":params}),
    )
    .map_err(|cause| fault("invalid_input", cause, &method, "not_started"))?;
    request.push(b'\n');
    if request.len() > LIMIT {
        return Err(fault(
            "invalid_input",
            "本机调用超过 512 KiB。",
            &method,
            "not_started",
        ));
    }
    let wait_budget = method_wait_budget(&method, &params)?;
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, connection.port));
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(500))
        .map_err(|cause| fault("native_bridge_unavailable", cause, &method, "not_started"))?;
    let browser = method.starts_with("browser.");
    let deadline = Instant::now() + wait_budget;
    let response_limit = if browser {
        32 * 1024 * 1024 + 1024
    } else {
        LIMIT
    };
    let write_began = Cell::new(false);
    let network = |cause| {
        fault(
            "native_bridge_failed",
            cause,
            &method,
            brightness_transport_effect(&method, write_began.get(), "unknown"),
        )
    };
    let transport = |cause: std::io::Error| {
        if matches!(
            method.as_str(),
            "fs.stat"
                | "fs.read_directory"
                | "fs.sort_logical_paths"
                | "ui.toggle_history"
                | "ui.action_hint"
                | "clipboard.read_data"
                | "explorer.context"
                | "selection.copy_data"
                | "display.brightness_step"
                | "audio.volume_adjust"
                | "win32_edit.replace_selection"
                | "raishot.resident.find"
                | "raishot.resident.region"
                | "raishot.capture_result"
                | "raishot.release_result"
                | "system.open"
                | "system.open_file_in_parent"
                | "process.capture"
                | "clipboard.write_files"
                | "window.geometry"
                | "window.set_rect"
                | "pointer.state"
                | "pointer.root"
                | "window.drag"
                | "actions.stop_other_instances"
                | "state.read"
                | "state.compare_and_swap"
                | "target.handoff_pointing_root"
                | "history.list"
                | "history.get"
                | "history.append"
                | "history.remove"
        ) && matches!(
            cause.kind(),
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
        ) {
            fault(
                "timeout",
                "本机调用等待超时。",
                &method,
                if matches!(
                    method.as_str(),
                    "selection.copy_data"
                        | "display.brightness_step"
                        | "audio.volume_adjust"
                        | "win32_edit.replace_selection"
                        | "raishot.resident.find"
                        | "raishot.resident.region"
                        | "raishot.capture_result"
                        | "raishot.release_result"
                        | "system.open"
                        | "system.open_file_in_parent"
                        | "process.capture"
                        | "clipboard.write_files"
                        | "window.geometry"
                        | "window.set_rect"
                        | "pointer.state"
                        | "pointer.root"
                        | "window.drag"
                        | "actions.stop_other_instances"
                        | "state.read"
                        | "state.compare_and_swap"
                        | "target.handoff_pointing_root"
                        | "history.append"
                        | "history.remove"
                        | "ui.toggle_history"
                        | "ui.action_hint"
                ) {
                    brightness_transport_effect(&method, write_began.get(), "unknown")
                } else {
                    "not_started"
                },
            )
        } else {
            network(cause.to_string())
        }
    };
    // 写入后不能证明服务端是否已执行；只读 stat 仍无副作用，其他方法为 unknown，均不重试。
    let remaining = || {
        if cancelled() {
            return Err(fault(
                "cancelled",
                "异步浏览器请求已取消。",
                &method,
                if write_began.get() {
                    "unknown"
                } else {
                    "not_started"
                },
            ));
        }
        deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| {
                if matches!(
                    method.as_str(),
                    "fs.stat"
                        | "fs.read_directory"
                        | "fs.sort_logical_paths"
                        | "ui.toggle_history"
                        | "ui.action_hint"
                        | "clipboard.read_data"
                        | "explorer.context"
                        | "selection.copy_data"
                        | "display.brightness_step"
                        | "audio.volume_adjust"
                        | "win32_edit.replace_selection"
                        | "raishot.resident.find"
                        | "raishot.resident.region"
                        | "raishot.capture_result"
                        | "raishot.release_result"
                        | "system.open"
                        | "system.open_file_in_parent"
                        | "process.capture"
                        | "clipboard.write_files"
                        | "window.geometry"
                        | "window.set_rect"
                        | "pointer.state"
                        | "pointer.root"
                        | "window.drag"
                        | "actions.stop_other_instances"
                        | "state.read"
                        | "state.compare_and_swap"
                        | "target.handoff_pointing_root"
                        | "history.list"
                        | "history.get"
                        | "history.append"
                        | "history.remove"
                ) {
                    fault(
                        "timeout",
                        "本机调用等待超时。",
                        &method,
                        if matches!(
                            method.as_str(),
                            "selection.copy_data"
                                | "display.brightness_step"
                                | "audio.volume_adjust"
                                | "win32_edit.replace_selection"
                                | "raishot.resident.find"
                                | "raishot.resident.region"
                                | "raishot.capture_result"
                                | "raishot.release_result"
                                | "system.open"
                                | "system.open_file_in_parent"
                                | "process.capture"
                                | "clipboard.write_files"
                                | "window.geometry"
                                | "window.set_rect"
                                | "pointer.state"
                                | "pointer.root"
                                | "window.drag"
                                | "actions.stop_other_instances"
                                | "state.read"
                                | "state.compare_and_swap"
                                | "target.handoff_pointing_root"
                                | "history.append"
                                | "history.remove"
                                | "ui.toggle_history"
                                | "ui.action_hint"
                        ) {
                            brightness_transport_effect(&method, write_began.get(), "unknown")
                        } else {
                            "not_started"
                        },
                    )
                } else {
                    network("本机调用等待超时。".to_string())
                }
            })
    };
    let mut written = 0;
    while written < request.len() {
        stream
            .set_write_timeout(Some(if cancellation.is_some() {
                remaining()?.min(Duration::from_millis(100))
            } else {
                remaining()?
            }))
            .map_err(transport)?;
        write_began.set(true);
        let count = stream.write(&request[written..]).map_err(transport)?;
        if count == 0 {
            return Err(network("本机调用写入中断。".into()));
        }
        written += count;
    }
    let mut bytes = Vec::new();
    loop {
        stream
            .set_read_timeout(Some(if cancellation.is_some() {
                remaining()?.min(Duration::from_millis(25))
            } else {
                remaining()?
            }))
            .map_err(transport)?;
        let mut chunk = [0u8; 4096];
        let count = match stream.read(&mut chunk) {
            Err(cause)
                if cancellation.is_some()
                    && matches!(
                        cause.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) =>
            {
                continue
            }
            result => result.map_err(transport)?,
        };
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.len() > response_limit + 1 {
            return Err(network("本机回执超过方法的结果大小限制。".into()));
        }
        // 服务端每连接一帧并关闭；读到 EOF 才能排除多帧或尾随内容。
    }
    if cancelled() {
        return Err(fault(
            "cancelled",
            "异步浏览器请求已取消。",
            &method,
            if write_began.get() {
                "unknown"
            } else {
                "not_started"
            },
        ));
    }
    parse_response(&bytes, id, &method)
}

fn parse_response(bytes: &[u8], id: u32, method: &str) -> Result<Value, ApiError> {
    let invalid = || {
        fault(
            if file_result_method(method)
                || method == "win32_edit.replace_selection"
                || method.starts_with("raishot.resident.")
                || method == "system.open"
                || method == "system.open_file_in_parent"
                || method == "process.capture"
                || method == "clipboard.write_files"
            {
                "invalid_response"
            } else {
                "native_bridge_failed"
            },
            "本机回执格式或调用身份不匹配。",
            method,
            "unknown",
        )
    };
    let frame = bytes.strip_suffix(b"\n").ok_or_else(invalid)?;
    if frame.contains(&b'\n') {
        return Err(invalid());
    }
    let response: Value = serde_json::from_slice(frame).map_err(|_| invalid())?;
    let object = response.as_object().ok_or_else(invalid)?;
    if object.len() != 3 || response["id"] != id {
        return Err(invalid());
    }
    match response["ok"].as_bool() {
        Some(true) => response.get("value").cloned().ok_or_else(invalid),
        Some(false) => {
            let error = response
                .get("error")
                .and_then(Value::as_object)
                .ok_or_else(invalid)?;
            let code = error
                .get("code")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty() && s.len() <= 64)
                .ok_or_else(invalid)?;
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty() && s.len() <= 16384)
                .ok_or_else(invalid)?;
            if code == "window_class_blocked"
                && method != "fs.sort_logical_paths"
                && (!matches!(
                    method,
                    "window.geometry" | "window.set_rect" | "window.drag"
                ) || error.get("method").and_then(Value::as_str) != Some(method)
                    || !matches!(
                        error.get("effect_state").and_then(Value::as_str),
                        Some("not_started" | "unknown")
                    ))
            {
                let mut invalid_class = fault(
                    "invalid_response",
                    "专用类名拒绝字段不完整或调用方法不匹配。",
                    method,
                    "unknown",
                );
                invalid_class.effect_state = Some("unknown".into());
                return Err(invalid_class);
            }
            let effect = match error.get("effect_state").and_then(Value::as_str) {
                Some("not_started") => "not_started",
                Some("unknown") | None => "unknown",
                _ => return Err(invalid()),
            };
            let reported_method = match error.get("method") {
                Some(value) => value
                    .as_str()
                    .filter(|method| !method.is_empty() && method.len() <= 256)
                    .ok_or_else(invalid)?,
                None => method,
            };
            if matches!(
                method,
                "fs.stat"
                    | "fs.read_directory"
                    | "fs.sort_logical_paths"
                    | "clipboard.read_data"
                    | "explorer.context"
                    | "selection.copy_data"
                    | "display.brightness_step"
                    | "audio.volume_adjust"
                    | "win32_edit.replace_selection"
                    | "raishot.resident.find"
                    | "raishot.resident.region"
                    | "raishot.capture_result"
                    | "raishot.release_result"
                    | "system.open"
                    | "system.open_file_in_parent"
                    | "process.capture"
                    | "clipboard.write_files"
                    | "window.geometry"
                    | "window.set_rect"
                    | "pointer.state"
                    | "pointer.root"
                    | "window.drag"
                    | "actions.stop_other_instances"
                    | "state.read"
                    | "state.compare_and_swap"
                    | "target.handoff_pointing_root"
                    | "history.list"
                    | "history.get"
                    | "history.append"
                    | "history.remove"
            ) && reported_method != method
            {
                return Err(invalid());
            }
            let permissions = match error.get("required_permissions") {
                Some(value) => value
                    .as_array()
                    .filter(|values| values.len() <= 32)
                    .ok_or_else(invalid)?
                    .iter()
                    .map(|value| {
                        value
                            .as_str()
                            .filter(|s| !s.is_empty() && s.len() <= 128)
                            .map(str::to_owned)
                            .ok_or_else(invalid)
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                None => Vec::new(),
            };
            let mut error = fault(code, message, reported_method, effect);
            if !matches!(
                method,
                "fs.sort_logical_paths"
                    | "history.list"
                    | "history.get"
                    | "history.append"
                    | "history.remove"
                    | "win32_edit.replace_selection"
                    | "raishot.resident.find"
                    | "raishot.resident.region"
                    | "raishot.capture_result"
                    | "raishot.release_result"
                    | "system.open"
                    | "system.open_file_in_parent"
                    | "process.capture"
                    | "clipboard.write_files"
                    | "window.geometry"
                    | "window.set_rect"
                    | "pointer.state"
                    | "pointer.root"
                    | "window.drag"
                    | "actions.stop_other_instances"
                    | "state.read"
                    | "state.compare_and_swap"
                    | "target.handoff_pointing_root"
            ) {
                error.required_permissions = permissions;
            }
            Err(error)
        }
        None => Err(invalid()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_sort_transport_and_host_errors_are_pure_and_redacted() {
        assert!(METHODS.contains(&"fs.sort_logical_paths"));
        for wrote in [false, true] {
            assert_eq!(
                brightness_transport_effect("fs.sort_logical_paths", wrote, "unknown"),
                "not_started"
            );
        }
        for (code, expected) in [
            ("invalid_input", "invalid_input"),
            ("cancelled", "cancelled"),
            ("timeout", "timeout"),
            ("fs_unavailable", "fs_unavailable"),
            ("native_bridge_failed", "fs_unavailable"),
            ("private-os-code", "fs_unavailable"),
            ("window_class_blocked", "fs_unavailable"),
        ] {
            let frame = format!(
                "{}\n",
                json!({"id":1,"ok":false,"error":{"code":code,"message":"private-input private-os-details","effect_state":"unknown","method":"fs.sort_logical_paths","required_permissions":["private-input"]}})
            );
            let error = parse_response(frame.as_bytes(), 1, "fs.sort_logical_paths").unwrap_err();
            assert_eq!(error.code, expected);
            assert_eq!(error.method.as_deref(), Some("fs.sort_logical_paths"));
            assert_eq!(error.effect_state.as_deref(), Some("not_started"));
            assert!(error.required_permissions.is_empty());
            assert!(!error.message.contains("private-input"));
            assert!(!error.message.contains("private-os-details"));
        }
        let frame = b"{\"id\":1,\"ok\":false,\"error\":{\"code\":\"timeout\",\"message\":\"private-input\",\"method\":\"fs.stat\"}}\n";
        let error = parse_response(frame, 1, "fs.sort_logical_paths").unwrap_err();
        assert_eq!(error.code, "fs_unavailable");
        assert_eq!(error.method.as_deref(), Some("fs.sort_logical_paths"));
        assert_eq!(error.effect_state.as_deref(), Some("not_started"));
    }

    #[test]
    #[cfg(windows)]
    fn logical_sort_rpc_only_forwards_to_the_app_bridge_and_preserves_full_strings() {
        use std::io::BufRead;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let connection = Configuration {
            port: listener.local_addr().unwrap().port(),
            token: "a".repeat(64),
        };
        let input = json!(["", "relative20", "a\0tail", "relative3", "relative3"]);
        let expected = input.clone();
        let server = thread::spawn(move || {
            let until = Instant::now() + Duration::from_secs(2);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < until, "未发出应用桥请求");
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => panic!("{error}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut line = String::new();
            std::io::BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let frame: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(frame["method"], "fs.sort_logical_paths");
            assert_eq!(frame["params"], json!({"paths":expected}));
            // 这里只证明转发与数组ABI；排序结果由应用后端测试和Framework oracle覆盖。
            writeln!(
                stream,
                "{}",
                json!({"id":frame["id"],"ok":true,"value":expected})
            )
            .unwrap();
        });
        let result = call(
            connection,
            1,
            "fs.sort_logical_paths".into(),
            json!({"paths":input}),
        );
        server.join().unwrap();
        assert_eq!(result.unwrap(), input);
    }

    #[test]
    #[cfg(not(windows))]
    fn logical_sort_non_windows_does_not_connect_to_a_local_substitute() {
        let error = call(
            Configuration {
                port: 1,
                token: "a".repeat(64),
            },
            1,
            "fs.sort_logical_paths".into(),
            json!({"paths":["relative"]}),
        )
        .unwrap_err();
        assert_eq!(error.code, "fs_unavailable");
        assert_eq!(error.effect_state.as_deref(), Some("not_started"));
    }

    #[test]
    fn file_result_budget_covers_capture_without_using_the_default_five_seconds() {
        assert_eq!(
            method_wait_budget("raishot.capture_result", &json!({"timeout_ms":30000})).unwrap(),
            Duration::from_millis(31500)
        );
        assert_eq!(
            method_wait_budget("raishot.release_result", &json!({})).unwrap(),
            Duration::from_secs(5)
        );
        assert_eq!(
            result_wait_budget("raishot.capture_result", &json!({"timeout_ms":30000})).unwrap(),
            Some(Duration::from_millis(31500))
        );
        assert_eq!(
            result_wait_budget("raishot.release_result", &json!({})).unwrap(),
            Some(Duration::from_secs(5))
        );
        for timeout in [0, 30001] {
            let error =
                result_wait_budget("raishot.capture_result", &json!({"timeout_ms":timeout}))
                    .unwrap_err();
            assert_eq!(error.code, "invalid_input");
            assert_eq!(error.effect_state.as_deref(), Some("not_started"));
        }
        assert_eq!(
            result_wait_budget("keyboard.text", &json!({})).unwrap(),
            None
        );
    }

    #[test]
    fn process_capture_wait_budget_uses_timeout_without_network_or_execution() {
        for (params, expected) in [
            (json!({}), 11500),
            (json!({"timeout_ms":1500}), 3000),
            (json!({"timeout_ms":25000}), 26500),
            (json!({"timeout_ms":60000}), 61500),
        ] {
            assert_eq!(
                method_wait_budget("process.capture", &params).unwrap(),
                Duration::from_millis(expected)
            );
        }
        for timeout in [
            json!(0),
            json!(60001),
            json!("1000"),
            json!(true),
            json!(null),
        ] {
            let error =
                method_wait_budget("process.capture", &json!({"timeout_ms":timeout})).unwrap_err();
            assert_eq!(error.effect_state.as_deref(), Some("not_started"));
            assert_eq!(error.method.as_deref(), Some("process.capture"));
        }
        assert_eq!(
            method_wait_budget("clipboard.read", &json!({})).unwrap(),
            Duration::from_secs(5)
        );
        assert_eq!(
            method_wait_budget("system.open", &json!({})).unwrap(),
            Duration::from_secs(66)
        );
    }
    #[test]
    fn process_capture_response_identity_and_submission_effects_are_checked_in_memory() {
        assert!(METHODS.contains(&"process.capture"));
        assert_eq!(
            brightness_transport_effect("process.capture", false, "unknown"),
            "not_started"
        );
        assert_eq!(
            brightness_transport_effect("process.capture", true, "unknown"),
            "unknown"
        );
        let body = json!({"id":1,"ok":false,"error":{"code":"process_timeout","message":"private argv/output",
            "method":"process.capture","effect_state":"unknown","required_permissions":["private"]}});
        let bytes = format!("{body}\n");
        let error = parse_response(bytes.as_bytes(), 1, "process.capture").unwrap_err();
        assert_eq!(error.code, "process_timeout");
        assert_eq!(error.effect_state.as_deref(), Some("unknown"));
        assert!(!error.message.contains("private"));
        assert!(error.required_permissions.is_empty());
        let mut mismatch = body;
        mismatch["error"]["method"] = "system.open".into();
        let error =
            parse_response(format!("{mismatch}\n").as_bytes(), 1, "process.capture").unwrap_err();
        assert_eq!(error.code, "invalid_response");
        assert_eq!(error.method.as_deref(), Some("process.capture"));
        assert_eq!(error.effect_state.as_deref(), Some("unknown"));
    }

    #[test]
    fn opening_native_policy_preserves_submission_boundary_without_network() {
        for method in ["system.open", "system.open_file_in_parent"] {
            assert!(METHODS.contains(&method));
            assert_eq!(
                brightness_transport_effect(method, false, "unknown"),
                "not_started"
            );
            assert_eq!(
                brightness_transport_effect(method, true, "unknown"),
                "unknown"
            );
            let response = json!({"id":1,"ok":false,"error":{"code":"open_failed","message":"synthetic",
                "method":method,"effect_state":"unknown"}});
            let bytes = format!("{response}\n");
            let error = parse_response(bytes.as_bytes(), 1, method).unwrap_err();
            assert_eq!(error.code, "open_failed");
            assert_eq!(error.effect_state.as_deref(), Some("unknown"));
            assert_eq!(error.method.as_deref(), Some(method));
            let other = if method == "system.open" {
                "system.open_file_in_parent"
            } else {
                "system.open"
            };
            assert!(parse_response(bytes.as_bytes(), 1, other).is_err());
        }
    }
    use std::{net::TcpListener, thread};

    #[test]
    fn refusal_and_invalid_configuration_are_not_started() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let unavailable = call(
            Configuration {
                port,
                token: "a".repeat(64),
            },
            1,
            "target.info".into(),
            json!({}),
        )
        .unwrap_err();
        assert_eq!(unavailable.code, "native_bridge_unavailable");
        assert_eq!(unavailable.effect_state.as_deref(), Some("not_started"));
        let invalid = call(
            Configuration {
                port: 0,
                token: "invalid".into(),
            },
            1,
            "target.info".into(),
            json!({}),
        )
        .unwrap_err();
        assert_eq!(invalid.code, "invalid_input");
        assert_eq!(invalid.effect_state.as_deref(), Some("not_started"));
    }

    #[test]
    fn frames_require_identity_single_response_and_preserve_native_faults() {
        assert_eq!(
            parse_response(b"{\"id\":1,\"ok\":true,\"value\":null}\n", 1, "target.info").unwrap(),
            Value::Null
        );
        for frame in [
            b"{\"id\":2,\"ok\":true,\"value\":1}\n".as_slice(),
            b"{\"id\":1,\"ok\":true,\"error\":{}}\n",
            b"{\"id\":1,\"ok\":true,\"value\":1}\n{}\n",
            b"{\"id\":1,\"ok\":true,\"value\":1}",
        ] {
            assert_eq!(
                parse_response(frame, 1, "target.info")
                    .unwrap_err()
                    .effect_state
                    .as_deref(),
                Some("unknown")
            );
        }
        let error = parse_response(b"{\"id\":1,\"ok\":false,\"error\":{\"code\":\"target_changed\",\"message\":\"changed\",\"effect_state\":\"not_started\"}}\n", 1, "keyboard.text").unwrap_err();
        assert_eq!(error.code, "target_changed");
        assert_eq!(error.effect_state.as_deref(), Some("not_started"));
        assert_eq!(error.method.as_deref(), Some("keyboard.text"));
    }

    #[test]
    fn tcp_handles_fragmented_response_and_does_not_retry_after_disconnect() {
        for disconnect in [false, true] {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            let connection = Configuration {
                port: listener.local_addr().unwrap().port(),
                token: "a".repeat(64),
            };
            let server = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\n") {
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                assert_eq!(
                    serde_json::from_slice::<Value>(&request).unwrap()["method"],
                    "target.info"
                );
                if !disconnect {
                    for chunk in [b"{\"id\":1,".as_slice(), b"\"ok\":true,\"value\":\"ok\"}\n"] {
                        stream.write_all(chunk).unwrap();
                    }
                }
                drop(stream);
                listener.set_nonblocking(true).unwrap();
                assert!(listener.accept().is_err());
            });
            let result = call(connection, 1, "target.info".into(), json!({}));
            if disconnect {
                assert_eq!(result.unwrap_err().effect_state.as_deref(), Some("unknown"));
            } else {
                assert_eq!(result.unwrap(), "ok");
            }
            server.join().unwrap();
        }
    }
    #[test]
    fn brightness_timeout_before_request_write_is_not_started_and_after_attempt_is_unknown() {
        assert_eq!(
            brightness_transport_effect("audio.volume_adjust", false, "unknown"),
            "not_started"
        );
        assert_eq!(
            brightness_transport_effect("audio.volume_adjust", true, "unknown"),
            "unknown"
        );

        assert_eq!(
            brightness_transport_effect("display.brightness_step", false, "unknown"),
            "not_started"
        );
        assert_eq!(
            brightness_transport_effect("display.brightness_step", true, "unknown"),
            "unknown"
        );
        assert_eq!(
            brightness_transport_effect("keyboard.text", false, "unknown"),
            "unknown"
        );
        assert_eq!(
            brightness_transport_effect("fs.stat", true, "not_started"),
            "not_started"
        );
    }
    #[test]
    fn edit_invalid_frames_are_unknown_without_changing_legacy_classification() {
        for frame in [
            b"{}\n".as_slice(),
            b"{not-json}\n",
            b"{\"id\":2,\"ok\":true,\"value\":null}\n",
            b"{\"id\":1,\"ok\":true,\"value\":null}\n{}\n",
        ] {
            let edit = parse_response(frame, 1, "win32_edit.replace_selection").unwrap_err();
            assert_eq!(edit.code, "invalid_response");
            assert_eq!(edit.effect_state.as_deref(), Some("unknown"));
            assert_eq!(edit.method.as_deref(), Some("win32_edit.replace_selection"));
            let legacy = parse_response(frame, 1, "keyboard.text").unwrap_err();
            assert_eq!(legacy.code, "native_bridge_failed");
        }
    }

    #[test]
    fn edit_transport_and_response_errors_preserve_effect_without_private_body() {
        let method = "win32_edit.replace_selection";
        assert_eq!(
            brightness_transport_effect(method, false, "unknown"),
            "not_started"
        );
        assert_eq!(
            brightness_transport_effect(method, true, "unknown"),
            "unknown"
        );
        let error = parse_response(b"{\"id\":1,\"ok\":false,\"error\":{\"code\":\"timeout\",\"message\":\"private original\",\"method\":\"win32_edit.replace_selection\",\"effect_state\":\"unknown\",\"required_permissions\":[\"private\"]}}\n", 1, method).unwrap_err();
        assert_eq!(error.code, "timeout");
        assert_eq!(error.effect_state.as_deref(), Some("unknown"));
        assert_eq!(error.method.as_deref(), Some(method));
        assert!(error.required_permissions.is_empty());
        assert!(!error.message.contains("private"));
        let error = parse_response(b"{\"id\":1,\"ok\":false,\"error\":{\"code\":\"edit_unsupported\",\"message\":\"private original\",\"method\":\"wrong.method\",\"effect_state\":\"not_started\"}}\n", 1, method).unwrap_err();
        assert_eq!(error.code, "invalid_response");
        assert_eq!(error.effect_state.as_deref(), Some("unknown"));
    }
    #[test]
    fn raishot_post_transport_preserves_unknown_after_write() {
        let method = "raishot.resident.region";
        assert_eq!(
            brightness_transport_effect(method, false, "unknown"),
            "not_started"
        );
        assert_eq!(
            brightness_transport_effect(method, true, "unknown"),
            "unknown"
        );
        let invalid =
            parse_response(b"{\"id\":2,\"ok\":true,\"value\":null}\n", 1, method).unwrap_err();
        assert_eq!(invalid.code, "invalid_response");
        assert_eq!(invalid.effect_state.as_deref(), Some("unknown"));
        let failure = parse_response(b"{\"id\":1,\"ok\":false,\"error\":{\"code\":\"timeout\",\"message\":\"PRIVATE\",\"method\":\"raishot.resident.region\",\"effect_state\":\"unknown\",\"required_permissions\":[\"PRIVATE\"]}}\n", 1, method).unwrap_err();
        assert_eq!(failure.effect_state.as_deref(), Some("unknown"));
        assert!(!failure.message.contains("PRIVATE"));
        assert!(failure.required_permissions.is_empty());
    }
}

#[cfg(test)]
mod class_rejection_wire_tests {
    use super::*;
    #[test]
    fn class_rejection_wire_preserves_unknown_and_requires_original_fields() {
        for effect in [Some("not_started"), Some("unknown"), None] {
            for method in [Some("window.geometry"), Some("pointer.root"), None] {
                let mut error = json!({"code":"window_class_blocked","message":"same"});
                if let Some(effect) = effect {
                    error["effect_state"] = effect.into();
                }
                if let Some(method) = method {
                    error["method"] = method.into();
                }
                let bytes = format!("{}\n", json!({"id":1,"ok":false,"error":error}));
                let actual = parse_response(bytes.as_bytes(), 1, "window.geometry").unwrap_err();
                let valid = method == Some("window.geometry") && effect.is_some();
                assert_eq!(
                    actual.code,
                    if valid {
                        "window_class_blocked"
                    } else {
                        "invalid_response"
                    }
                );
                assert_eq!(
                    actual.effect_state.as_deref(),
                    if valid { effect } else { Some("unknown") }
                );
                assert_eq!(actual.method.as_deref(), Some("window.geometry"));
            }
        }
    }
}

// 只有 JSON/ApiError 和取消标记跨线程；socket 在 call_controlled 的工作线程内创建并销毁。
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserAsyncRequest {
    id: u32,
    method: String,
    params: Value,
}

struct BrowserCompletion {
    id: u32,
    result: Result<Value, ApiError>,
}

pub(crate) struct BrowserAsyncPool {
    configuration: Option<Value>,
    pending: std::collections::BTreeMap<
        u32,
        (
            std::thread::JoinHandle<()>,
            std::sync::Arc<std::sync::atomic::AtomicBool>,
        ),
    >,
    accepted: std::collections::BTreeSet<u32>,
    send: std::sync::mpsc::Sender<BrowserCompletion>,
    receive: std::sync::mpsc::Receiver<BrowserCompletion>,
    ready: std::collections::VecDeque<BrowserCompletion>,
}

impl BrowserAsyncPool {
    pub(crate) fn new(configuration: Option<Value>) -> Self {
        let (send, receive) = std::sync::mpsc::channel();
        Self {
            configuration,
            pending: Default::default(),
            accepted: Default::default(),
            send,
            receive,
            ready: Default::default(),
        }
    }

    fn submit_with(
        &mut self,
        request: BrowserAsyncRequest,
        execute: impl FnOnce(
                Configuration,
                u32,
                String,
                Value,
                &std::sync::atomic::AtomicBool,
            ) -> Result<Value, ApiError>
            + Send
            + 'static,
    ) -> Result<u32, ApiError> {
        let method = request.method.clone();
        // 连接只取自可信 Request.native_context；脚本 JSON 无 connection 字段。
        let configuration: Configuration =
            serde_json::from_value(self.configuration.clone().ok_or_else(|| {
                fault(
                    "browser_async_unavailable",
                    "本次父宿主没有异步浏览器连接。",
                    &method,
                    "not_started",
                )
            })?)
            .map_err(|_| {
                fault(
                    "invalid_input",
                    "异步浏览器连接格式无效。",
                    &method,
                    "not_started",
                )
            })?;
        if configuration.port == 0
            || configuration.token.len() != 64
            || !configuration
                .token
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || !matches!(method.as_str(), "browser.call" | "browser.profiles")
            || !request.params.is_object()
            || !(1..=128).contains(&request.id)
            || self.accepted.contains(&request.id)
        {
            return Err(fault(
                "invalid_input",
                "异步浏览器请求或调用标识无效。",
                &method,
                "not_started",
            ));
        }
        if self.accepted.len() >= 32 {
            return Err(fault(
                "desktop_call_limit",
                "单次脚本最多接纳32次异步宿主调用。",
                &method,
                "not_started",
            ));
        }
        // 已完成但尚未交回 JS 的结果仍占槽位，没有隐式队列。
        if self.pending.len() >= 4 {
            return Err(fault(
                "browser_async_capacity",
                "异步浏览器最多四个在途请求。",
                &method,
                "not_started",
            ));
        }
        let id = request.id;
        let cancellation = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_cancellation = cancellation.clone();
        let send = self.send.clone();
        let handle = std::thread::Builder::new()
            .name(format!("script-browser-{id}"))
            .spawn(move || {
                let method = request.method.clone();
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    execute(
                        configuration,
                        id,
                        request.method,
                        request.params,
                        &worker_cancellation,
                    )
                }))
                .unwrap_or_else(|_| {
                    Err(fault(
                        "native_bridge_failed",
                        "异步浏览器工作线程异常。",
                        &method,
                        "unknown",
                    ))
                });
                let _ = send.send(BrowserCompletion { id, result });
            })
            .map_err(|_| {
                fault(
                    "worker_start_failed",
                    "无法启动异步浏览器工作线程。",
                    &method,
                    "not_started",
                )
            })?;
        self.accepted.insert(id);
        self.pending.insert(id, (handle, cancellation));
        Ok(id)
    }

    pub(crate) fn dispatch(&mut self, request: String) -> String {
        let result = if request.len() > LIMIT {
            Err(fault(
                "invalid_input",
                "异步浏览器请求超过512 KiB。",
                "browser.call",
                "not_started",
            ))
        } else {
            serde_json::from_str::<BrowserAsyncRequest>(&request)
                .map_err(|_| {
                    fault(
                        "invalid_input",
                        "异步浏览器请求格式无效。",
                        "browser.call",
                        "not_started",
                    )
                })
                .and_then(|request| {
                    self.submit_with(request, |connection, id, method, params, cancellation| {
                        call_controlled(connection, id, method, params, Some(cancellation))
                    })
                })
        };
        match result {
            Ok(id) => json!({"ok":true,"id":id}).to_string(),
            Err(error) => json!({"ok":false,"error":error}).to_string(),
        }
    }

    pub(crate) fn pending(&self) -> usize {
        self.pending.len()
    }

    pub(crate) fn poll(&mut self) -> String {
        let Some(completion) = self
            .ready
            .pop_front()
            .or_else(|| self.receive.try_recv().ok())
        else {
            return "null".into();
        };
        if let Some((handle, _)) = self.pending.remove(&completion.id) {
            let began = Instant::now();
            let _ = handle.join();
            crate::api::exclude_host_wait(began.elapsed());
        }
        match completion.result {
            Ok(value) => json!({"id":completion.id,"ok":true,"value":value}).to_string(),
            Err(error) => json!({"id":completion.id,"ok":false,"error":error}).to_string(),
        }
    }

    pub(crate) fn wait(&mut self, duration: Duration) {
        if self.ready.is_empty() && !self.pending.is_empty() {
            if let Ok(completion) = self.receive.recv_timeout(duration) {
                self.ready.push_back(completion);
            }
        }
    }
}

impl Drop for BrowserAsyncPool {
    fn drop(&mut self) {
        for (_, cancellation) in self.pending.values() {
            cancellation.store(true, std::sync::atomic::Ordering::Release);
        }
        for (_, (handle, _)) in std::mem::take(&mut self.pending) {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod browser_async_tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc,
    };
    const WAIT: Duration = Duration::from_secs(3);
    fn pool() -> BrowserAsyncPool {
        BrowserAsyncPool::new(Some(json!({"port":1,"token":"a".repeat(64)})))
    }
    fn request(id: u32) -> BrowserAsyncRequest {
        BrowserAsyncRequest {
            id,
            method: "browser.call".into(),
            params: json!({}),
        }
    }
    fn take(pool: &mut BrowserAsyncPool) -> Value {
        pool.wait(WAIT);
        serde_json::from_str(&pool.poll()).unwrap()
    }

    #[test]
    fn two_requests_are_dispatched_before_first_completion_and_finish_in_reverse_order() {
        let mut pool = pool();
        let (entered, started) = mpsc::channel();
        let mut release = Vec::new();
        for id in 1..=2 {
            let (send, receive) = mpsc::channel();
            release.push(send);
            let entered = entered.clone();
            pool.submit_with(request(id), move |_, id, _, _, _| {
                entered.send(id).unwrap();
                receive.recv_timeout(WAIT).unwrap();
                Ok(json!({"result":id}))
            })
            .unwrap();
        }
        let mut ids = [
            started.recv_timeout(WAIT).unwrap(),
            started.recv_timeout(WAIT).unwrap(),
        ];
        ids.sort();
        assert_eq!(ids, [1, 2]);
        assert_eq!(pool.pending(), 2);
        release[1].send(()).unwrap();
        let second = take(&mut pool);
        assert_eq!(second["id"], 2);
        release[0].send(()).unwrap();
        let first = take(&mut pool);
        assert_eq!(first["id"], 1);
        assert_eq!(pool.pending(), 0);
    }

    #[test]
    fn fifth_request_is_not_started_and_drop_cancels_and_joins_all_owners() {
        let mut pool = pool();
        let live = Arc::new(AtomicUsize::new(0));
        let stopped = Arc::new(AtomicUsize::new(0));
        let (send, receive) = mpsc::channel();
        for id in 1..=4 {
            let live = live.clone();
            let stopped = stopped.clone();
            let send = send.clone();
            pool.submit_with(request(id), move |_, _, _, _, stop| {
                live.fetch_add(1, Ordering::SeqCst);
                send.send(()).unwrap();
                let until = Instant::now() + WAIT;
                while !stop.load(Ordering::Acquire) {
                    assert!(Instant::now() < until);
                    std::thread::yield_now();
                }
                live.fetch_sub(1, Ordering::SeqCst);
                stopped.fetch_add(1, Ordering::SeqCst);
                Ok(Value::Null)
            })
            .unwrap();
        }
        for _ in 0..4 {
            receive.recv_timeout(WAIT).unwrap();
        }
        assert_eq!(
            pool.submit_with(request(5), |_, _, _, _, _| panic!("容量拒绝不能执行IO"))
                .unwrap_err()
                .code,
            "browser_async_capacity"
        );
        drop(pool);
        assert_eq!(live.load(Ordering::SeqCst), 0);
        assert_eq!(stopped.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn missing_parent_configuration_and_non_browser_methods_reject_before_executor() {
        let mut missing = BrowserAsyncPool::new(None);
        let error = missing
            .submit_with(request(1), |_, _, _, _, _| panic!("旧父进程不能联网"))
            .unwrap_err();
        assert_eq!(error.code, "browser_async_unavailable");
        assert_eq!(error.effect_state.as_deref(), Some("not_started"));
        let mut pool = pool();
        let mut other = request(1);
        other.method = "keyboard.text".into();
        assert_eq!(
            pool.submit_with(other, |_, _, _, _, _| panic!("不能派发非浏览器"))
                .unwrap_err()
                .code,
            "invalid_input"
        );
        assert_eq!(pool.pending(), 0);
    }

    #[test]
    fn completed_unpolled_slot_unique_ids_total_budget_and_errors_are_preserved() {
        let mut pool = pool();
        for id in 1..=32 {
            pool.submit_with(request(id), |_, _, _, _, _| {
                Err(ApiError {
                    code: "permission_required".into(),
                    message: "synthetic".into(),
                    effect_state: Some("not_started".into()),
                    method: Some("tabs.move".into()),
                    required_permissions: vec!["tabs".into()],
                })
            })
            .unwrap();
            pool.wait(WAIT);
            assert_eq!(pool.pending(), 1);
            let reply = take(&mut pool);
            assert_eq!(reply["error"]["required_permissions"], json!(["tabs"]));
            assert_eq!(reply["error"]["method"], "tabs.move");
        }
        assert_eq!(
            pool.submit_with(request(33), |_, _, _, _, _| panic!("总预算不能派发"))
                .unwrap_err()
                .code,
            "desktop_call_limit"
        );
        assert_eq!(
            pool.submit_with(request(1), |_, _, _, _, _| panic!("不能复用ID"))
                .unwrap_err()
                .code,
            "invalid_input"
        );
    }
}
