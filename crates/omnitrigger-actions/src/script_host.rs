//! 轻量脚本宿主的进程适配；引擎作为独立可执行文件，不链接进输入引擎或桌面程序。
use crate::{ActionError, ActionRuntime, CancellationToken, InvocationContext, ResolvedAction};
use serde_json::{json, Value};
use std::{
    fs::File,
    io::{Read, Seek, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const SOURCE_LIMIT: u64 = 128 * 1024;
const RESPONSE_LIMIT: u64 = 96 * 1024;

pub struct ScriptHostRuntime {
    program: PathBuf,
    timeout: Duration,
}

impl ScriptHostRuntime {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            timeout: Duration::from_secs(60),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

impl ActionRuntime for ScriptHostRuntime {
    fn execute(
        &self,
        action: &ResolvedAction,
        context: &InvocationContext,
        cancellation: &CancellationToken,
    ) -> Result<Value, ActionError> {
        if cancellation.is_cancelled() {
            return Err(error("cancelled", "动作提交前已取消。", "not_started"));
        }
        let (root, entry) = crate::node::checked_entry(action)?;
        let mut source = String::new();
        File::open(entry)
            .and_then(|file| file.take(SOURCE_LIMIT + 1).read_to_string(&mut source))
            .map_err(|cause| error("entry_unavailable", cause.to_string(), "not_started"))?;
        if source.len() as u64 > SOURCE_LIMIT {
            return Err(error(
                "source_too_large",
                "脚本入口超过 128 KiB。",
                "not_started",
            ));
        }
        // 动作文件保持标准 JS 语法，供编辑器与类型提示使用；CLI 原型的函数体协议留在适配层。
        let source = format!("{source}\nif (typeof main !== 'function') throw new Error('脚本入口须声明 function main(ctx)');\nreturn main(ctx);");
        let request = serde_json::to_vec(&json!({
            "script":source,"input":action.call.input,"package_root":root,
            "invocation":{"invocation_id":context.invocation_id.to_string(),"binding_id":context.binding_id.to_string()}
        })).map_err(|cause| error("invalid_request", cause.to_string(), "not_started"))?;
        if request.len() > 256 * 1024 {
            return Err(error(
                "invalid_request",
                "脚本宿主请求超过 256 KiB。",
                "not_started",
            ));
        }
        let scratch = crate::node::ResponseDirectory::new()?;
        let mut output = File::options()
            .create_new(true)
            .read(true)
            .write(true)
            .open(scratch.response())
            .map_err(|cause| error("runtime_io", cause.to_string(), "not_started"))?;
        let mut command = Command::new(&self.program);
        command
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(
                output
                    .try_clone()
                    .map_err(|cause| error("runtime_io", cause.to_string(), "not_started"))?,
            )
            .stderr(Stdio::null());
        crate::node::hide_window(&mut command);
        let mut child = OwnedHost(command.spawn().map_err(|cause| {
            error(
                "runtime_start_failed",
                format!("无法启动轻量脚本宿主：{cause}"),
                "not_started",
            )
        })?);
        let mut input = child.0.stdin.take().expect("piped stdin");
        let writer = thread::Builder::new()
            .name("omnitrigger-script-input".into())
            .spawn(move || input.write_all(&request))
            .map_err(|cause| error("runtime_io", cause.to_string(), "unknown"))?;
        let began = Instant::now();
        let status = loop {
            let reason = if cancellation.is_cancelled() {
                Some(("cancelled", "动作已取消；已发生的副作用不会撤销。"))
            } else if began.elapsed() >= self.timeout {
                Some(("timeout", "脚本宿主超时；不会自动重试。"))
            } else if output
                .metadata()
                .map(|m| m.len() > RESPONSE_LIMIT)
                .unwrap_or(false)
            {
                Some(("invalid_response", "脚本宿主输出超过 96 KiB。"))
            } else {
                None
            };
            if let Some((code, message)) = reason {
                drop(child);
                let _ = writer.join();
                return Err(error(code, message, "unknown"));
            }
            match child.0.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => thread::sleep(Duration::from_millis(10)),
                Err(cause) => {
                    drop(child);
                    let _ = writer.join();
                    return Err(error("runtime_io", cause.to_string(), "unknown"));
                }
            }
        };
        // 真实 OS 退出后再读取唯一的结果文件；不用 stdout 管道等待可能仍存活的后代。
        child
            .0
            .wait()
            .map_err(|cause| error("runtime_io", cause.to_string(), "unknown"))?;
        let written = writer
            .join()
            .map_err(|_| error("runtime_io", "脚本请求写入线程异常。", "unknown"))?;
        if let Err(cause) = written {
            return Err(error(
                "runtime_io",
                format!("脚本请求未写完：{cause}"),
                "unknown",
            ));
        }
        output
            .rewind()
            .map_err(|cause| error("runtime_io", cause.to_string(), "unknown"))?;
        let mut bytes = Vec::new();
        output
            .take(RESPONSE_LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|cause| error("runtime_io", cause.to_string(), "unknown"))?;
        if bytes.len() as u64 > RESPONSE_LIMIT {
            return Err(error("invalid_response", "脚本结果过大。", "unknown"));
        }
        parse_response(&bytes, status.success())
    }
}

fn error(code: &str, message: impl Into<String>, effect: &str) -> ActionError {
    let mut value = ActionError::new(code, message);
    value.effect_state = Some(effect.into());
    value
}

fn parse_response(bytes: &[u8], exited_successfully: bool) -> Result<Value, ActionError> {
    let invalid = || {
        error(
            "invalid_response",
            "脚本结果版本、终态或退出码不匹配。",
            "unknown",
        )
    };
    let value: Value = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let object = value.as_object().ok_or_else(invalid)?;
    if object.len() != 3
        || (exited_successfully && object.contains_key("error"))
        || (!exited_successfully && object.contains_key("result"))
    {
        return Err(invalid());
    }
    if value["schema_version"] != 1 || value["ok"].as_bool() != Some(exited_successfully) {
        return Err(invalid());
    }
    if exited_successfully {
        return value.get("result").cloned().ok_or_else(invalid);
    }
    let message = value["error"]
        .as_str()
        .filter(|text| !text.is_empty() && text.len() <= 16 * 1024)
        .ok_or_else(invalid)?;
    Err(error("script_failed", message, "unknown"))
}

struct OwnedHost(Child);
impl Drop for OwnedHost {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn result_requires_version_and_matching_os_exit() {
        assert_eq!(
            parse_response(br#"{"schema_version":1,"ok":true,"result":null}"#, true).unwrap(),
            Value::Null
        );
        for bytes in [
            br#"{"ok":true,"result":1}"#.as_slice(),
            br#"{"schema_version":1,"ok":false,"result":1}"#,
            br#"{"schema_version":1,"ok":true}"#,
            br#"{"schema_version":1,"ok":true,"result":1,"error":"ambiguous"}"#,
        ] {
            assert_eq!(
                parse_response(bytes, true).unwrap_err().code,
                "invalid_response"
            );
        }
        let failed = parse_response(
            br#"{"schema_version":1,"ok":false,"error":"failed"}"#,
            false,
        )
        .unwrap_err();
        assert_eq!(failed.code, "script_failed");
        assert_eq!(failed.effect_state.as_deref(), Some("unknown"));
    }
}
