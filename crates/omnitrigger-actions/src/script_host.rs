//! 轻量脚本宿主的进程适配；引擎作为独立可执行文件，不链接进输入引擎或桌面程序。
use crate::native::NativeBridge;
use crate::{
    ActionError, ActionRuntime, CancellationToken, InvocationContext, NativeHost, ResolvedAction,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Seek, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[path = "script_host_capability_probe.rs"]
mod capability_probe;

const SOURCE_LIMIT: u64 = 128 * 1024;
const RESPONSE_LIMIT: u64 = 96 * 1024;

// 校验、探测、创建进程和执行共用期限；阻塞中的系统调用仍只能在返回后复核。
fn remaining_budget(
    deadline: Instant,
    cancellation: &CancellationToken,
    now: Instant,
    submitted: bool,
) -> Result<Duration, ActionError> {
    let effect = if submitted { "unknown" } else { "not_started" };
    if cancellation.is_cancelled() {
        return Err(error(
            "cancelled",
            if submitted {
                "动作已取消；已发生的副作用不会撤销。"
            } else {
                "动作提交前已取消，未发送脚本输入。"
            },
            effect,
        ));
    }
    deadline
        .checked_duration_since(now)
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| {
            error(
                "timeout",
                if submitted {
                    "脚本宿主超时；不会自动重试。"
                } else {
                    "动作提交前已超过运行时限，未发送脚本输入。"
                },
                effect,
            )
        })
}

fn start_before_request<T>(
    deadline: Instant,
    cancellation: &CancellationToken,
    now: impl Fn() -> Instant,
    start: impl FnOnce() -> Result<T, ActionError>,
) -> Result<T, ActionError> {
    remaining_budget(deadline, cancellation, now(), false)?;
    let child = start()?;
    // 失败时 child 就地释放；生产调用持有的 OwnedHost 负责回收自有宿主。
    remaining_budget(deadline, cancellation, now(), false)?;
    Ok(child)
}

fn write_request(
    mut input: impl Write,
    request: &[u8],
    deadline: Instant,
    cancellation: &CancellationToken,
    now: Instant,
) -> Result<(), ActionError> {
    // 在线程真正获得调度后复核，不能只在创建写入线程前检查。
    remaining_budget(deadline, cancellation, now, false)?;
    input
        .write_all(request)
        .map_err(|cause| error("runtime_io", format!("脚本请求未写完：{cause}"), "unknown"))
}

pub struct ScriptHostRuntime {
    program: PathBuf,
    timeout: Duration,
    integrity: Option<(String, u64)>,
    required_capabilities: Vec<String>,
}

impl ScriptHostRuntime {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            timeout: Duration::from_secs(60),
            integrity: None,
            required_capabilities: Vec::new(),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// 托管安装包的宿主映像校验。手动指定解释器时不自动声称受包清单保护。
    pub fn with_integrity(mut self, sha256: String, size: u64) -> Self {
        self.integrity = Some((sha256, size));
        self
    }

    /// 仅调用方明确提出能力要求时探测宿主；不从源码或 SDK 推断要求。
    /// 未声明要求的已有动作保持原执行路径，静态引用检查不调用本方法。
    pub fn with_required_capabilities(mut self, required: Vec<String>) -> Self {
        self.required_capabilities = required;
        self
    }

    fn checked_program(&self) -> Result<Option<File>, ActionError> {
        let Some((expected, size)) = &self.integrity else {
            return Ok(None);
        };
        let invalid = || {
            error(
                "runtime_integrity",
                "脚本宿主与已安装清单不符，未启动进程。",
                "not_started",
            )
        };
        if expected.len() != 64
            || !expected.bytes().all(|byte| byte.is_ascii_hexdigit())
            || *size == 0
            || *size > 16 * 1024 * 1024
        {
            return Err(invalid());
        }
        crate::catalog::check_ancestors(&self.program).map_err(|_| invalid())?;
        let metadata = std::fs::symlink_metadata(&self.program).map_err(|_| invalid())?;
        crate::catalog::check_local(&self.program, &metadata).map_err(|_| invalid())?;
        if !metadata.is_file() || metadata.len() != *size {
            return Err(invalid());
        }
        let mut options = File::options();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1);
        }
        let mut file = options.open(&self.program).map_err(|_| invalid())?;
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 32768];
        let mut total = 0u64;
        loop {
            let count = file.read(&mut buffer).map_err(|_| invalid())?;
            if count == 0 {
                break;
            }
            total += count as u64;
            if total > *size {
                return Err(invalid());
            }
            hash.update(&buffer[..count]);
        }
        if total != *size || !format!("{:x}", hash.finalize()).eq_ignore_ascii_case(expected) {
            return Err(invalid());
        }
        // Windows 上此句柄在宿主退出前禁止替换或写入映像。
        Ok(Some(file))
    }
}

impl ActionRuntime for ScriptHostRuntime {
    fn execute(
        &self,
        action: &ResolvedAction,
        context: &InvocationContext,
        cancellation: &CancellationToken,
    ) -> Result<Value, ActionError> {
        self.execute_in_context(action, context, cancellation, None)
    }
    fn execute_with_host(
        &self,
        action: &ResolvedAction,
        context: &InvocationContext,
        cancellation: &CancellationToken,
        host: Option<&dyn NativeHost>,
    ) -> Result<Value, ActionError> {
        self.execute_in_context(action, context, cancellation, host)
    }
}

impl ScriptHostRuntime {
    fn execute_in_context(
        &self,
        action: &ResolvedAction,
        context: &InvocationContext,
        cancellation: &CancellationToken,
        host: Option<&dyn NativeHost>,
    ) -> Result<Value, ActionError> {
        let began = Instant::now();
        if cancellation.is_cancelled() {
            return Err(error("cancelled", "动作提交前已取消。", "not_started"));
        }
        let deadline = began.checked_add(self.timeout).ok_or_else(|| {
            error(
                "invalid_request",
                "脚本宿主运行时限超出支持范围。",
                "not_started",
            )
        })?;
        remaining_budget(deadline, cancellation, began, false)?;
        let _image = self.checked_program()?;
        capability_probe::check(
            &self.program,
            &self.required_capabilities,
            deadline,
            cancellation,
        )?;
        let (root, entry) = crate::node::checked_entry(action)?;
        let entry_request = entry_request(&root, &entry)?;
        let native = host
            .map(|_| NativeBridge::new().and_then(|bridge| Ok((bridge.configuration()?, bridge))));
        let (mut bridge, native_context) = match native {
            Some(Ok((configuration, bridge))) => {
                (Some(bridge), json!({"connection":configuration}))
            }
            Some(Err(error)) => (None, json!({"error":error})),
            None => (None, Value::Null),
        };
        let mut request = json!({
            "input":action.call.input,"package_root":crate::node::script_path(&root),"native_context":native_context,
            "invocation":{"invocation_id":context.invocation_id.to_string(),"binding_id":context.binding_id.to_string()}
        });
        request
            .as_object_mut()
            .expect("request object")
            .extend(entry_request);
        let request = serde_json::to_vec(&request)
            .map_err(|cause| error("invalid_request", cause.to_string(), "not_started"))?;
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
        let mut child = start_before_request(deadline, cancellation, Instant::now, || {
            command.spawn().map(OwnedHost).map_err(|cause| {
                error(
                    "runtime_start_failed",
                    format!("无法启动轻量脚本宿主：{cause}"),
                    "not_started",
                )
            })
        })?;
        let input = child.0.stdin.take().expect("piped stdin");
        let writer_cancellation = cancellation.clone();
        let writer = thread::Builder::new()
            .name("omnitrigger-script-input".into())
            .spawn(move || {
                write_request(
                    input,
                    &request,
                    deadline,
                    &writer_cancellation,
                    Instant::now(),
                )
            })
            .map_err(|cause| error("runtime_io", cause.to_string(), "not_started"))?;
        let status = loop {
            let reason = if cancellation.is_cancelled() {
                Some(("cancelled", "动作已取消；已发生的副作用不会撤销。"))
            } else if Instant::now() >= deadline {
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
                match writer.join() {
                    Ok(Err(fault)) if fault.effect_state.as_deref() == Some("not_started") => {
                        return Err(fault);
                    }
                    _ => {}
                }
                return Err(error(code, message, "unknown"));
            }
            if let (Some(bridge), Some(host)) = (&mut bridge, host) {
                bridge.poll_until(host, cancellation, Some(deadline));
            }
            // 本机调用可能耗尽剩余预算，不在同一轮接受迟到的成功退出。
            if cancellation.is_cancelled() || Instant::now() >= deadline {
                continue;
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
        written?;
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
        remaining_budget(deadline, cancellation, Instant::now(), true)?;
        parse_response(&bytes, status.success()).map_err(|mut fault| {
            if fault.code != "invalid_response" {
                fault.message = format!(
                    "脚本入口：{}\n{}",
                    crate::node::script_path(&entry).display(),
                    fault.message
                );
                const TAIL: &str = "\n…（诊断已截断）";
                if fault.message.len() > 16 * 1024 {
                    const SEPARATOR: &str = "\n…\n";
                    let budget = 16 * 1024 - TAIL.len() - SEPARATOR.len();
                    let mut end = budget / 2;
                    while !fault.message.is_char_boundary(end) {
                        end -= 1;
                    }
                    let mut start = fault.message.len() - (budget - end);
                    while !fault.message.is_char_boundary(start) {
                        start += 1;
                    }
                    fault.message = format!(
                        "{}{SEPARATOR}{}{TAIL}",
                        &fault.message[..end],
                        &fault.message[start..]
                    );
                }
            }
            fault
        })
    }
}

fn entry_request(
    root: &std::path::Path,
    entry: &std::path::Path,
) -> Result<serde_json::Map<String, Value>, ActionError> {
    if entry.extension().is_some_and(|ext| ext == "mjs") {
        let relative = entry.strip_prefix(root).map_err(|_| {
            error(
                "entry_unavailable",
                "模块入口必须位于动作包内。",
                "not_started",
            )
        })?;
        let relative = relative
            .to_str()
            .ok_or_else(|| {
                error(
                    "entry_unavailable",
                    "模块入口路径需要 UTF-8。",
                    "not_started",
                )
            })?
            .replace('\\', "/");
        // 由宿主在自己的包内加载器中再次验证入口及每个依赖；不拼接或转译模块源码。
        return Ok(serde_json::Map::from_iter([(
            "module_entry".into(),
            relative.into(),
        )]));
    }
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
    // 兼容原有 function main(ctx) 工程；函数体宿主协议不变。
    let source = format!("{source}\nif (typeof main !== 'function') throw new Error('脚本入口须声明 function main(ctx)');\nreturn main(ctx);");
    Ok(serde_json::Map::from_iter([(
        "script".into(),
        source.into(),
    )]))
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
    if value["error"].is_object() {
        let fault: ActionError =
            serde_json::from_value(value["error"].clone()).map_err(|_| invalid())?;
        if fault.code.is_empty()
            || fault.code.len() > 64
            || fault.message.is_empty()
            || fault.message.len() > 16 * 1024
            || !matches!(
                fault.effect_state.as_deref(),
                Some("not_started" | "unknown")
            )
            || fault
                .method
                .as_ref()
                .is_some_and(|method| method.len() > 256)
            || fault.required_permissions.len() > 32
            || fault
                .required_permissions
                .iter()
                .any(|permission| permission.is_empty() || permission.len() > 128)
        {
            return Err(invalid());
        }
        return Err(fault);
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
#[path = "script_host_budget_tests.rs"]
mod budget_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn module_entry_uses_a_path_and_legacy_entry_keeps_function_body_protocol() {
        let fixture = crate::catalog::tests::Fixture::new();
        let entry = fixture.0.join("main.mjs");
        let request = entry_request(&fixture.0, &entry).unwrap();
        assert_eq!(request["module_entry"], "main.mjs");
        assert!(!request.contains_key("script"));
        let entry = fixture.0.join("main.js");
        std::fs::write(&entry, "function main(ctx) { return ctx.input; }").unwrap();
        let request = entry_request(&fixture.0, &entry).unwrap();
        assert!(request["script"]
            .as_str()
            .unwrap()
            .ends_with("return main(ctx);"));
        assert!(!request.contains_key("module_entry"));
    }

    #[test]
    #[ignore = "需要显式指定实际构建的 Boa 或 QuickJS 宿主"]
    fn generated_module_project_runs_and_reports_dependency_errors() {
        let fixture = crate::catalog::tests::Fixture::new();
        let root = fixture.0.join("actions");
        let project = crate::project::create_script_project(&root, "modules").unwrap();
        let action = crate::PackageCatalog::load(&root)
            .resolve(&project.call)
            .unwrap();
        let runtime = ScriptHostRuntime::new(PathBuf::from(
            std::env::var_os("OMNITRIGGER_TEST_SCRIPT_HOST").expect("设置真实宿主路径"),
        ));
        let context = InvocationContext {
            invocation_id: u64::MAX,
            binding_id: 9_007_199_254_740_993,
        };
        let result = runtime
            .execute(&action, &context, &CancellationToken::new())
            .unwrap();
        assert_eq!(
            result,
            json!({"text":"OmniTrigger","invocation":{"invocation_id":u64::MAX.to_string(),"binding_id":"9007199254740993"}})
        );
        std::fs::write(
            project.package_root.join("lib/text.js"),
            "export function cleanText(){throw new Error('helper failed');}",
        )
        .unwrap();
        let error = runtime
            .execute(&action, &context, &CancellationToken::new())
            .unwrap_err();
        assert_eq!(error.code, "script_failed");
        assert!(error.message.contains("text.js"), "{error}");
        std::fs::write(
            project.package_root.join("lib/text.js"),
            "export const cleanText = ;",
        )
        .unwrap();
        let error = runtime
            .execute(&action, &context, &CancellationToken::new())
            .unwrap_err();
        assert_eq!(error.code, "module_parse");
        assert_eq!(error.effect_state.as_deref(), Some("not_started"));
        assert!(error.message.contains("text.js"), "{error}");
    }
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
        let native = parse_response(br#"{"schema_version":1,"ok":false,"error":{"code":"target_changed","message":"changed","effect_state":"not_started","method":"keyboard.text"}}"#, false).unwrap_err();
        assert_eq!(native.code, "target_changed");
        assert_eq!(native.effect_state.as_deref(), Some("not_started"));
        for error in [
            json!({"code":"x","message":"x","effect_state":"rolled_back"}),
            json!({"code":"","message":"x","effect_state":"not_started"}),
            json!({"code":"x","message":"x","effect_state":"unknown","extra":true}),
        ] {
            let frame = json!({"schema_version":1,"ok":false,"error":error});
            assert_eq!(
                parse_response(frame.to_string().as_bytes(), false)
                    .unwrap_err()
                    .code,
                "invalid_response"
            );
        }
    }
}
