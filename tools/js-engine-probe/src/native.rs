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
pub const METHODS: [&str; 28] = [
    "history.list",
    "history.get",
    "history.append",
    "history.remove",
    "fs.stat",
    "ui.choose",
    "ui.notify",
    "ui.open_history",
    "ui.toggle_history",
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
    "selection.copy",
    "selection.copy_data",
    "selection.read_text",
    "clipboard.read",
    "clipboard.read_data",
    "clipboard.write",
    "window.action",
    "display.brightness_step",
    "audio.volume_adjust",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    port: u16,
    token: String,
}

fn fault(code: &str, message: impl ToString, method: &str, effect: &str) -> ApiError {
    let stat = method == "fs.stat";
    let data_read = method == "clipboard.read_data";
    let data_copy = method == "selection.copy_data";
    let history_read = matches!(method, "history.list" | "history.get");
    let history = matches!(
        method,
        "history.list" | "history.get" | "history.append" | "history.remove"
    );
    ApiError {
        code: (if history && code == "native_bridge_unavailable" {
            "history_unavailable"
        } else if stat && code == "native_bridge_unavailable" {
            "fs_unavailable"
        } else if stat && code == "native_bridge_failed" {
            "fs_error"
        } else {
            code
        })
        .into(),
        message: if method == "audio.volume_adjust" {
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
        effect_state: Some(
            if stat || data_read || history_read {
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

// 请求写入尝试前可证明未提交；开始写入后仅保守返回unknown。
fn brightness_transport_effect(
    method: &str,
    write_began: bool,
    fallback: &'static str,
) -> &'static str {
    if matches!(method, "display.brightness_step" | "audio.volume_adjust") && !write_began {
        "not_started"
    } else {
        fallback
    }
}

pub fn call(
    connection: Configuration,
    id: u32,
    method: String,
    params: Value,
) -> Result<Value, ApiError> {
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
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, connection.port));
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(500))
        .map_err(|cause| fault("native_bridge_unavailable", cause, &method, "not_started"))?;
    let browser = method.starts_with("browser.");
    let deadline = Instant::now()
        + Duration::from_secs(
            if browser
                || method == "ui.choose"
                || matches!(
                    method.as_str(),
                    "display.brightness_step" | "audio.volume_adjust"
                )
            {
                66
            } else if method == "selection.copy_data" {
                8
            } else {
                5
            },
        );
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
                | "ui.toggle_history"
                | "clipboard.read_data"
                | "selection.copy_data"
                | "display.brightness_step"
                | "audio.volume_adjust"
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
                        | "history.append"
                        | "history.remove"
                        | "ui.toggle_history"
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
        deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| {
                if matches!(
                    method.as_str(),
                    "fs.stat"
                        | "ui.toggle_history"
                        | "clipboard.read_data"
                        | "selection.copy_data"
                        | "display.brightness_step"
                        | "audio.volume_adjust"
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
                                | "history.append"
                                | "history.remove"
                                | "ui.toggle_history"
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
            .set_write_timeout(Some(remaining()?))
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
            .set_read_timeout(Some(remaining()?))
            .map_err(transport)?;
        let mut chunk = [0u8; 4096];
        let count = stream.read(&mut chunk).map_err(transport)?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.len() > response_limit + 1 {
            return Err(network("本机回执超过方法的结果大小限制。".into()));
        }
        // 服务端每连接一帧并关闭；读到 EOF 才能排除多帧或尾随内容。
    }
    parse_response(&bytes, id, &method)
}

fn parse_response(bytes: &[u8], id: u32, method: &str) -> Result<Value, ApiError> {
    let invalid = || {
        fault(
            "native_bridge_failed",
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
                    | "clipboard.read_data"
                    | "selection.copy_data"
                    | "display.brightness_step"
                    | "audio.volume_adjust"
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
                "history.list" | "history.get" | "history.append" | "history.remove"
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
}
