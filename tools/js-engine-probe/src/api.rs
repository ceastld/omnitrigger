//! 两个引擎共用的宿主能力；这里不安装 Hook，不访问 GPUI。
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const MAX_TEXT: usize = 64 * 1024;

#[derive(Serialize, Debug)]
pub struct ApiError {
    pub(crate) code: String,
    pub(crate) message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) effect_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) method: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) required_permissions: Vec<String>,
}

fn error(code: &'static str, message: impl ToString) -> ApiError {
    ApiError {
        code: code.into(),
        message: message.to_string(),
        effect_state: None,
        method: None,
        required_permissions: Vec::new(),
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    Delay {
        milliseconds: u64,
    },
    Native {
        connection: crate::native::Configuration,
        id: u32,
        method: String,
        params: Value,
    },
    ReadText {
        path: String,
    },
    WriteText {
        path: String,
        text: String,
        overwrite: bool,
    },
    Run {
        program: String,
        args: Vec<String>,
        timeout_ms: u64,
    },
    Start {
        program: String,
        args: Vec<String>,
    },
    ReadClipboard,
    WriteClipboard {
        text: String,
    },
    Open {
        target: String,
    },
}

pub fn dispatch(request: String) -> String {
    let result = if request.len() > 512 * 1024 {
        Err(error("request_too_large", "API 请求超过 512 KiB。"))
    } else {
        serde_json::from_str::<Call>(&request)
            .map_err(|cause| error("invalid_arguments", cause))
            .and_then(|call| {
                if request.len() > 256 * 1024 && !matches!(call, Call::Native { .. }) {
                    return Err(error("request_too_large", "API 请求超过 256 KiB。"));
                }
                execute(call)
            })
    };
    match result {
        Ok(value) => json!({"ok":true,"value":value}).to_string(),
        Err(cause) => json!({"ok":false,"error":cause}).to_string(),
    }
}

fn absolute_path(path: &str) -> Result<&Path, ApiError> {
    let path = Path::new(path);
    if !path.is_absolute() {
        return Err(error("invalid_path", "文件路径必须为绝对路径。"));
    }
    Ok(path)
}

fn read_text(path: &str) -> Result<String, ApiError> {
    let file =
        File::open(absolute_path(path)?).map_err(|cause| error("file_read_failed", cause))?;
    let mut bytes = Vec::new();
    file.take(MAX_TEXT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|cause| error("file_read_failed", cause))?;
    if bytes.len() > MAX_TEXT {
        return Err(error("text_too_large", "文件超过 64 KiB。"));
    }
    String::from_utf8(bytes).map_err(|cause| error("invalid_utf8", cause))
}

fn check_text(text: &str) -> Result<(), ApiError> {
    if text.len() > MAX_TEXT {
        return Err(error("text_too_large", "文本超过 64 KiB。"));
    }
    Ok(())
}

fn command(program: &str, args: &[String]) -> Result<Command, ApiError> {
    if program.trim().is_empty()
        || program.contains('\0')
        || args.iter().any(|arg| arg.contains('\0'))
    {
        return Err(error(
            "invalid_arguments",
            "程序与参数不能包含 NUL，程序不能为空。",
        ));
    }
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    Ok(command)
}

struct ManagedChild(Child);
impl Drop for ManagedChild {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn execute(call: Call) -> Result<Value, ApiError> {
    match call {
        Call::Delay { milliseconds } => {
            if milliseconds > 60_000 {
                return Err(error(
                    "invalid_arguments",
                    "等待时间必须为 0–60000 毫秒的整数。",
                ));
            }
            thread::sleep(Duration::from_millis(milliseconds));
            Ok(Value::Null)
        }
        Call::Native {
            connection,
            id,
            method,
            params,
        } => crate::native::call(connection, id, method, params),
        Call::ReadText { path } => Ok(json!(read_text(&path)?)),
        Call::WriteText {
            path,
            text,
            overwrite,
        } => {
            check_text(&text)?;
            let mut options = OpenOptions::new();
            options.write(true);
            if overwrite {
                options.create(true).truncate(true);
            } else {
                options.create_new(true);
            }
            let mut file = options
                .open(absolute_path(&path)?)
                .map_err(|cause| error("file_write_failed", cause))?;
            file.write_all(text.as_bytes())
                .map_err(|cause| error("file_write_failed", cause))?;
            Ok(Value::Null)
        }
        Call::Run {
            program,
            args,
            timeout_ms,
        } => {
            if !(1..=1000).contains(&timeout_ms) {
                return Err(error(
                    "invalid_arguments",
                    "原型进程等待上限为 1–1000 毫秒。",
                ));
            }
            let mut child = ManagedChild(
                command(&program, &args)?
                    .spawn()
                    .map_err(|cause| error("process_start_failed", cause))?,
            );
            let pid = child.0.id();
            let deadline = Instant::now() + Duration::from_millis(timeout_ms);
            loop {
                if let Some(status) = child
                    .0
                    .try_wait()
                    .map_err(|cause| error("process_wait_failed", cause))?
                {
                    return Ok(
                        json!({"pid":pid,"exitCode":status.code(),"success":status.success()}),
                    );
                }
                if Instant::now() >= deadline {
                    return Err(error(
                        "process_timeout",
                        "进程等待超时，已请求终止直接子进程。",
                    ));
                }
                thread::sleep(Duration::from_millis(5));
            }
        }
        Call::Start { program, args } => {
            let child = command(&program, &args)?
                .spawn()
                .map_err(|cause| error("process_start_failed", cause))?;
            let pid = child.id();
            // 后台回收，不等待独立程序退出；独立程序不随脚本结束而关闭。
            thread::spawn(move || {
                let mut child = child;
                let _ = child.wait();
            });
            Ok(json!({"pid":pid}))
        }
        Call::ReadClipboard => desktop::read_clipboard().map(Value::String),
        Call::WriteClipboard { text } => {
            check_text(&text)?;
            desktop::write_clipboard(&text)?;
            Ok(Value::Null)
        }
        Call::Open { target } => {
            desktop::open(&target)?;
            Ok(Value::Null)
        }
    }
}

#[cfg(windows)]
mod desktop {
    use super::*;
    pub fn read_clipboard() -> Result<String, ApiError> {
        let text = omnitrigger_windows::read_clipboard_text()
            .map_err(|cause| error("clipboard_read_failed", cause))?;
        check_text(&text)?;
        Ok(text)
    }
    pub fn write_clipboard(text: &str) -> Result<(), ApiError> {
        omnitrigger_windows::write_clipboard_text(text)
            .map_err(|cause| error("clipboard_write_failed", cause))
    }
    pub fn open(target: &str) -> Result<(), ApiError> {
        omnitrigger_windows::open_target(target).map_err(|cause| error("open_failed", cause))
    }
}

#[cfg(not(windows))]
mod desktop {
    use super::*;
    pub fn read_clipboard() -> Result<String, ApiError> {
        Err(error("unsupported_platform", "此 API 需要 Windows。"))
    }
    pub fn write_clipboard(_: &str) -> Result<(), ApiError> {
        read_clipboard().map(|_| ())
    }
    pub fn open(_: &str) -> Result<(), ApiError> {
        read_clipboard().map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_wire_rejects_invalid_values_without_relying_on_sdk_validation() {
        for value in [
            json!(60001),
            json!(-1),
            json!(1.5),
            json!("1"),
            json!(null),
            json!(true),
        ] {
            let request = json!({"kind":"delay","milliseconds":value});
            let response: Value = serde_json::from_str(&dispatch(request.to_string())).unwrap();
            assert_eq!(response["ok"], false);
            assert_eq!(response["error"]["code"], "invalid_arguments");
        }
        let response: Value = serde_json::from_str(&dispatch(
            json!({"kind":"delay","milliseconds":0}).to_string(),
        ))
        .unwrap();
        assert_eq!(response, json!({"ok":true,"value":null}));
    }
}
