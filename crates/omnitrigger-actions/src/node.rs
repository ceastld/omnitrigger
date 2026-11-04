//! 每次调用一个 Node 进程；运行代码的权限与当前用户相同，不构成沙盒。
//! 协议结果走独立临时文件，普通 stdout/stderr 不进入协议或应用历史。
use crate::{
    native::NativeBridge, ActionError, ActionRuntime, CancellationToken, InvocationContext,
    NativeHost, ResolvedAction,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

const HOST: &str = include_str!("../runtime/host.mjs");
const OUTPUT_LIMIT: u64 = 64 * 1024;
const POLL: Duration = Duration::from_millis(20);
const CANCEL_GRACE: Duration = Duration::from_millis(250);
static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

pub struct NodeRuntime {
    program: PathBuf,
    timeout: Duration,
    browser_connector: Option<PathBuf>,
}

impl NodeRuntime {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            timeout: Duration::from_secs(60),
            browser_connector: None,
        }
    }

    /// 由应用提供固定版本连接器目录；仅执行时核验，非浏览器动作无需安装连接器。
    pub fn with_browser_connector(mut self, directory: PathBuf) -> Self {
        self.browser_connector = Some(directory);
        self
    }
}

impl Default for NodeRuntime {
    fn default() -> Self {
        Self::new("node")
    }
}

impl ActionRuntime for NodeRuntime {
    fn execute(
        &self,
        action: &ResolvedAction,
        context: &InvocationContext,
        cancellation: &CancellationToken,
    ) -> Result<Value, ActionError> {
        self.execute_with_host(action, context, cancellation, None)
    }

    fn execute_with_host(
        &self,
        action: &ResolvedAction,
        context: &InvocationContext,
        cancellation: &CancellationToken,
        host: Option<&dyn NativeHost>,
    ) -> Result<Value, ActionError> {
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        self.execute_with_native(
            action,
            context,
            cancellation,
            host,
            host.map(|_| NativeBridge::new()),
        )
    }
}

impl NodeRuntime {
    fn execute_with_native(
        &self,
        action: &ResolvedAction,
        context: &InvocationContext,
        cancellation: &CancellationToken,
        host: Option<&dyn NativeHost>,
        native: Option<Result<NativeBridge, ActionError>>,
    ) -> Result<Value, ActionError> {
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        // 目录是加载时快照，执行前再次核验入口，避免文件被替换为链接后越界。
        let (root, entry) = checked_entry(action)?;
        let scratch = ResponseDirectory::new()?;
        let native =
            native.map(|result| result.and_then(|bridge| Ok((bridge.configuration()?, bridge))));
        let (mut native, native_configuration, native_error) = match native {
            Some(Ok((configuration, bridge))) => (Some(bridge), Some(configuration), None),
            Some(Err(error)) => (None, None, Some(error)),
            None => (None, None, None),
        };
        let request = json!({
            "api_version": 1,
            "invocation": {
                "invocation_id": context.invocation_id.to_string(),
                "binding_id": context.binding_id.to_string(),
            },
            "input": action.call.input,
            "package_root": root,
            "entry_path": entry,
            "response_path": scratch.response(),
            "timeout_ms": self.timeout.as_millis(),
            "browser": self.browser_connector.as_deref().and_then(browser_configuration),
            "native": native_configuration,
            "native_error": native_error,
        });
        let mut bytes = serde_json::to_vec(&request)
            .map_err(|error| ActionError::new("invalid_request", error.to_string()))?;
        if bytes.len() > 96 * 1024 {
            return Err(ActionError::new("invalid_request", "动作请求超过 96 KiB。"));
        }
        bytes.push(b'\n');
        let mut command = Command::new(&self.program);
        command
            .args(["--input-type=module", "--eval", HOST])
            .current_dir(&root)
            .env_remove("NODE_OPTIONS")
            .env_remove("OMNITRIGGER_BROWSER_PIPE")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        hide_window(&mut command);
        let mut child = ChildGuard(command.spawn().map_err(|error| {
            ActionError::new(
                "runtime_start_failed",
                format!("无法启动 Node 运行时：{error}"),
            )
        })?);
        let stdin = child.0.stdin.take().expect("piped stdin");
        // 即使配置的可执行文件不读取 stdin，后台运行时仍能按时取消/超时。
        let (written_tx, written_rx) = mpsc::sync_channel(1);
        let writer = thread::Builder::new()
            .name("omnitrigger-node-input".into())
            .spawn(move || {
                let mut stdin = stdin;
                let result = stdin.write_all(&bytes).map(|_| stdin);
                let _ = written_tx.send(result);
            })
            .map_err(|error| {
                ActionError::new("runtime_io", format!("无法写入动作请求：{error}"))
            })?;
        let began = Instant::now();
        let mut input: Option<ChildStdin> = None;
        let mut cancel_writer = None;
        let mut stopping: Option<(Instant, ActionError)> = None;
        let mut input_pending = true;
        let result = loop {
            if input_pending {
                match written_rx.try_recv() {
                    Ok(Ok(stdin)) => {
                        input = Some(stdin);
                        input_pending = false;
                    }
                    Ok(Err(error)) => {
                        break Err(ActionError::new(
                            "runtime_io",
                            format!("动作请求写入失败：{error}"),
                        ))
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        break Err(ActionError::new("runtime_io", "动作请求写入线程退出。"))
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                }
            }
            if stopping.is_none() {
                let reason = if cancellation.is_cancelled() {
                    Some(cancelled())
                } else if began.elapsed() >= self.timeout {
                    let mut error =
                        ActionError::new("timeout", "动作超过 60 秒运行时限；不会自动重试。");
                    error.effect_state = Some("unknown".into());
                    Some(error)
                } else {
                    None
                };
                if let Some(reason) = reason {
                    if let Some(mut stdin) = input.take() {
                        // 不在轮询线程写管道；错误解释器不读 stdin 时也能强制停止。
                        cancel_writer = thread::Builder::new()
                            .name("omnitrigger-node-cancel".into())
                            .spawn(move || {
                                let _ = stdin.write_all(b"cancel\n");
                            })
                            .ok();
                    }
                    stopping = Some((Instant::now(), reason));
                }
            }
            match child.0.try_wait() {
                Ok(Some(status)) => {
                    if let Some((_, reason)) = stopping {
                        break Err(reason);
                    }
                    if !status.success() {
                        break Err(ActionError::new(
                            "runtime_exit",
                            format!("Node 宿主异常退出（{}）；业务结果未确认。", status),
                        ));
                    }
                    break read_response(&scratch.response(), context.invocation_id);
                }
                Err(error) => {
                    break Err(ActionError::new(
                        "runtime_io",
                        format!("无法读取 Node 运行状态：{error}"),
                    ))
                }
                Ok(None) => {}
            }
            if let (true, Some(bridge), Some(host)) = (stopping.is_none(), &mut native, host) {
                bridge.poll(host, cancellation);
            }
            if let Some((since, reason)) = &stopping {
                if since.elapsed() >= CANCEL_GRACE {
                    terminate_host(&mut child.0);
                    break Err(reason.clone());
                }
            }
            thread::sleep(POLL);
        };
        drop(input);
        drop(child);
        // 宿主退出会关闭管道，写线程不会被子程序继承的 stdout 挂起。
        let _ = writer.join();
        if let Some(writer) = cancel_writer {
            let _ = writer.join();
        }
        result
    }
}

fn cancelled() -> ActionError {
    let mut error = ActionError::new("cancelled", "动作已取消；已经完成的操作不会撤销。");
    error.effect_state = Some("unknown".into());
    error
}

fn browser_configuration(directory: &Path) -> Option<Value> {
    let directory = std::path::absolute(directory).ok()?;
    let sdk = directory.join("sdk-js").join("index.mjs");
    let config = directory.join("connection.json");
    for file in [
        &sdk,
        &config,
        &directory.join("sdk-js").join("protocol.mjs"),
    ] {
        for ancestor in file.ancestors() {
            if restricted(&fs::symlink_metadata(ancestor).ok()?) {
                return None;
            }
        }
        if !file.is_file() {
            return None;
        }
    }
    Some(json!({"sdk_path": sdk, "config_path": config}))
}

fn checked_entry(action: &ResolvedAction) -> Result<(PathBuf, PathBuf), ActionError> {
    let fail = |error| ActionError::new("entry_unavailable", format!("动作入口不可用：{error}"));
    if !action.package_root.is_absolute() || !action.entry_path.is_absolute() {
        return Err(ActionError::new(
            "entry_unavailable",
            "动作包与入口必须是已解析的绝对路径。",
        ));
    }
    for ancestor in action.package_root.ancestors() {
        if restricted(&fs::symlink_metadata(ancestor).map_err(fail)?) {
            return Err(ActionError::new(
                "entry_unavailable",
                "动作包不能经过链接或云端占位目录。",
            ));
        }
    }
    let root_meta = fs::symlink_metadata(&action.package_root).map_err(fail)?;
    if restricted(&root_meta) || !root_meta.is_dir() {
        return Err(ActionError::new(
            "entry_unavailable",
            "动作包目录不能是链接或云端占位目录。",
        ));
    }
    let root = fs::canonicalize(&action.package_root).map_err(fail)?;
    let relative = action
        .entry_path
        .strip_prefix(&action.package_root)
        .map_err(|_| ActionError::new("entry_unavailable", "动作入口必须位于动作包内。"))?;
    let mut path = action.package_root.clone();
    for component in relative.components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return Err(ActionError::new(
                "entry_unavailable",
                "动作入口路径包含无效片段。",
            ));
        }
        path.push(component);
        if restricted(&fs::symlink_metadata(&path).map_err(fail)?) {
            return Err(ActionError::new(
                "entry_unavailable",
                "动作入口不能经过链接或云端占位文件。",
            ));
        }
    }
    let entry = fs::canonicalize(&path).map_err(fail)?;
    if !entry.starts_with(&root) || !fs::metadata(&entry).map_err(fail)?.is_file() {
        return Err(ActionError::new(
            "entry_unavailable",
            "动作入口必须是动作包内的文件。",
        ));
    }
    Ok((root, entry))
}

fn restricted(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // REPARSE_POINT, OFFLINE, RECALL_ON_OPEN, RECALL_ON_DATA_ACCESS。
        metadata.file_attributes() & (0x400 | 0x1000 | 0x40000 | 0x400000) != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum Response {
    Ok {
        api_version: u32,
        invocation_id: String,
        output: Value,
    },
    Error {
        api_version: u32,
        invocation_id: String,
        error: ActionError,
    },
}

fn read_response(path: &Path, invocation_id: u64) -> Result<Value, ActionError> {
    let invalid = |message: String| ActionError::new("invalid_response", message);
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| invalid(format!("Node 未返回业务结果：{error}")))?;
    if !metadata.is_file() || restricted(&metadata) || metadata.len() > OUTPUT_LIMIT {
        return Err(invalid("Node 结果不是有效文件或超过 64 KiB。".into()));
    }
    let mut data = Vec::new();
    File::open(path)
        .and_then(|file| file.take(OUTPUT_LIMIT + 1).read_to_end(&mut data))
        .map_err(|error| invalid(format!("无法读取 Node 结果：{error}")))?;
    if data.len() > OUTPUT_LIMIT as usize {
        return Err(invalid("Node 结果超过 64 KiB。".into()));
    }
    let response: Response = serde_json::from_slice(&data)
        .map_err(|error| invalid(format!("Node 结果协议无效：{error}")))?;
    let (version, id) = match &response {
        Response::Ok {
            api_version,
            invocation_id,
            ..
        }
        | Response::Error {
            api_version,
            invocation_id,
            ..
        } => (*api_version, invocation_id),
    };
    if version != 1 || *id != invocation_id.to_string() {
        return Err(invalid("Node 结果版本或调用 ID 不匹配。".into()));
    }
    match response {
        Response::Ok { output, .. } => Ok(output),
        Response::Error { error, .. }
            if !error.code.is_empty()
                && error.code.len() <= 64
                && error.message.len() <= 16 * 1024 =>
        {
            Err(error)
        }
        _ => Err(invalid("Node 结果缺少终态或包含相互冲突的字段。".into())),
    }
}

struct ResponseDirectory(PathBuf);

impl ResponseDirectory {
    fn new() -> Result<Self, ActionError> {
        for _ in 0..32 {
            let path = std::env::temp_dir().join(format!(
                "omnitrigger-action-{}-{}",
                std::process::id(),
                NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
            ));
            let builder = fs::DirBuilder::new();
            #[cfg(unix)]
            let builder = {
                use std::os::unix::fs::DirBuilderExt;
                let mut builder = builder;
                builder.mode(0o700);
                builder
            };
            match builder.create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(ActionError::new(
                        "runtime_io",
                        format!("无法创建动作临时目录：{error}"),
                    ))
                }
            }
        }
        Err(ActionError::new("runtime_io", "无法分配动作临时目录。"))
    }

    fn response(&self) -> PathBuf {
        self.0.join("response.json")
    }
}

impl Drop for ResponseDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.response());
        let _ = fs::remove_file(self.0.join("response.json.tmp"));
        // 仅移除自己创建的空目录，不递归删除脚本可能写入的其他内容。
        let _ = fs::remove_dir(&self.0);
    }
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            terminate_host(&mut self.0);
        }
        let _ = self.0.wait();
    }
}

fn hide_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    #[cfg(not(windows))]
    {
        let _ = command;
    }
}

fn terminate_host(child: &mut Child) {
    // 只终止本次宿主，不能连带终止 start 已独立启动的程序。
    // run 子程序由协作取消清理；JS 阻塞事件循环时无法保证这些子程序收尾。
    let _ = child.kill();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ActionCall, ActionDefinition};

    #[test]
    fn unavailable_native_bridge_only_fails_the_requested_capability() {
        if !node_available() {
            return;
        }
        let fixture = Fixture::new(
            r#"export default async ctx => {
            const arithmetic = 6 * 7;
            let failure;
            try { await ctx.desktop.target(); } catch(e) { failure={code:e.code,effect:e.effectState}; }
            return {arithmetic,failure};
        }"#,
            json!({}),
        );
        let mut error = ActionError::new("native_bridge_unavailable", "本机桥接被禁用。");
        error.effect_state = Some("not_started".into());
        let output = NodeRuntime::default()
            .execute_with_native(
                &fixture.action,
                &context(),
                &CancellationToken::new(),
                Some(&DesktopHost::default()),
                Some(Err(error)),
            )
            .unwrap();
        assert_eq!(
            output,
            json!({"arithmetic":42,"failure":{"code":"native_bridge_unavailable","effect":"not_started"}})
        );
    }

    #[test]
    fn text_api_preserves_maximum_json_escaped_text_and_rejects_invalid_unicode() {
        if !node_available() {
            return;
        }
        #[derive(Default)]
        struct TextHost(std::sync::Mutex<String>);
        impl NativeHost for TextHost {
            fn call(
                &self,
                method: &str,
                params: Value,
                _: &CancellationToken,
            ) -> Result<Value, ActionError> {
                match method {
                    "clipboard.write" => {
                        *self.0.lock().unwrap() = params["text"].as_str().unwrap().into();
                        Ok(json!({"written":true}))
                    }
                    "clipboard.read" => Ok(json!(*self.0.lock().unwrap())),
                    "keyboard.text" => {
                        assert_eq!(params["text"], "文😀\r\n");
                        Ok(json!({"submitted":true}))
                    }
                    "selection.copy" => Ok(json!(" \t ")),
                    _ => panic!("未知方法"),
                }
            }
        }
        let fixture = Fixture::new(
            r#"export default async ctx => {
            const text='\u0001'.repeat(65536);
            await ctx.clipboard.writeText(text);
            const read=await ctx.clipboard.readText();
            const submitted=await ctx.desktop.sendText('文😀\r\n');
            const selection=await ctx.desktop.copySelectionText();
            let invalid=0;
            for (const text of ['\ud800','x'.repeat(65537),'a\0b']) {
                try { await ctx.desktop.sendText(text); } catch(e) { if(e.code==='invalid_input' && e.effectState==='not_started') invalid++; }
            }
            return {length:read.length,equal:read===text,submitted,selection,invalid};
        }"#,
            json!({}),
        );
        let output = NodeRuntime::default()
            .execute_with_host(
                &fixture.action,
                &context(),
                &CancellationToken::new(),
                Some(&TextHost::default()),
            )
            .unwrap();
        assert_eq!(
            output,
            json!({"length":65536,"equal":true,"submitted":{"submitted":true},"selection":" \t ","invalid":3})
        );
    }

    #[derive(Default)]
    struct DesktopHost(std::sync::Mutex<Vec<Value>>);
    impl NativeHost for DesktopHost {
        fn call(
            &self,
            method: &str,
            params: Value,
            _: &CancellationToken,
        ) -> Result<Value, ActionError> {
            self.0
                .lock()
                .unwrap()
                .push(json!({"method":method,"params":params}));
            match method {
                "target.info" => Ok(
                    json!({"processName":"Acrobat.exe","title":"测试文档","processId":12,"isForeground":true}),
                ),
                "keyboard.shortcut" if params["keys"] == json!([162, 87]) => {
                    Ok(json!({"submitted":true}))
                }
                _ => {
                    let mut error = ActionError::new("target_changed", "目标已切换。");
                    error.effect_state = Some("not_started".into());
                    Err(error)
                }
            }
        }
    }

    #[test]
    fn desktop_host_routes_captured_target_shortcuts_and_structured_errors_without_stdout_protocol()
    {
        if !node_available() {
            return;
        }
        let fixture = Fixture::new(
            r#"export default async ctx => {
            console.log('ordinary output is not a native request');
            const target = await ctx.desktop.target();
            const keys = target.processName.toLowerCase() === 'acrobat.exe' ? [162,87] : [164,115];
            const sent = await ctx.desktop.sendShortcut(keys);
            let failure;
            try { await ctx.desktop.sendShortcut([65]); } catch(e) { failure = {code:e.code,effect:e.effectState}; }
            return {target,sent,failure};
        }"#,
            json!({}),
        );
        let host = DesktopHost::default();
        let result = NodeRuntime::default()
            .execute_with_host(
                &fixture.action,
                &context(),
                &CancellationToken::new(),
                Some(&host),
            )
            .unwrap();
        assert_eq!(result["target"]["title"], "测试文档");
        assert_eq!(result["sent"], json!({"submitted":true}));
        assert_eq!(
            result["failure"],
            json!({"code":"target_changed","effect":"not_started"})
        );
        let calls = host.0.lock().unwrap();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[1]["params"]["keys"], json!([162, 87]));
    }

    #[test]
    fn target_context_export_keeps_portable_precision_and_clipboard_receipt() {
        if !node_available() {
            return;
        }
        struct Host;
        impl NativeHost for Host {
            fn call(
                &self,
                method: &str,
                params: Value,
                _: &CancellationToken,
            ) -> Result<Value, ActionError> {
                match method {
                    "target.context" if params == json!({}) => {
                        Ok(json!({"window_handle":"123", "process_id":7,
                        "thread_id":8,"process_created_file_time":"134386985330123456","expires_at_unix_ms":1793760000000u64}))
                    }
                    "clipboard.write" if params == json!({"text":"literal"}) => {
                        Ok(json!({"written":true,"sequenceNumber":42}))
                    }
                    _ => Err(ActionError::new("unexpected_method", method)),
                }
            }
        }
        let fixture = Fixture::new("export default async ctx => ({target:await ctx.desktop.targetContext(), receipt:await ctx.clipboard.writeText('literal')});", json!({}));
        let output = NodeRuntime::default()
            .execute_with_host(
                &fixture.action,
                &context(),
                &CancellationToken::new(),
                Some(&Host),
            )
            .unwrap();
        assert_eq!(
            output["target"]["process_created_file_time"],
            "134386985330123456"
        );
        assert_eq!(output["receipt"]["sequenceNumber"], 42);
    }

    #[test]
    fn keyboard_recipe_keeps_literal_text_and_single_key_independent_of_profile_delay() {
        if !node_available() {
            return;
        }
        #[derive(Default)]
        struct Host(std::sync::Mutex<Vec<Value>>);
        impl NativeHost for Host {
            fn call(
                &self,
                method: &str,
                params: Value,
                _: &CancellationToken,
            ) -> Result<Value, ActionError> {
                self.0
                    .lock()
                    .unwrap()
                    .push(json!({"method":method,"params":params}));
                if !matches!(method, "keyboard.text" | "keyboard.sequence") {
                    return Err(ActionError::new("unexpected_method", method));
                }
                Ok(json!({"submitted":true}))
            }
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/actions");
        let action = crate::PackageCatalog::load(&root).resolve(&ActionCall {
            package_id:"examples.keyboard-sequence".into(), action_id:"recipe".into(), action_version:1,
            input:json!({"keyDownDelayMs":10,"steps":[{"kind":"text","text":"${literal} $$"},{"kind":"tap","key":35}]}),
        }).unwrap();
        let host = Host::default();
        NodeRuntime::default()
            .execute_with_host(&action, &context(), &CancellationToken::new(), Some(&host))
            .unwrap();
        assert_eq!(
            *host.0.lock().unwrap(),
            vec![
                json!({"method":"keyboard.text","params":{"text":"${literal} $$"}}),
                json!({"method":"keyboard.sequence","params":{"modifiers":[],"keys":[35],"key_down_delay_ms":0}}),
            ]
        );
    }

    #[test]
    fn keyboard_recipe_example_preserves_profile_timing_and_stops_after_cancellation() {
        if !node_available() {
            return;
        }
        struct Host {
            calls: std::sync::Mutex<Vec<(String, Value, Instant)>>,
            cancel_after_first_input: bool,
        }
        impl NativeHost for Host {
            fn call(
                &self,
                method: &str,
                params: Value,
                cancellation: &CancellationToken,
            ) -> Result<Value, ActionError> {
                self.calls
                    .lock()
                    .unwrap()
                    .push((method.into(), params.clone(), Instant::now()));
                match method {
                    "target.info" => Ok(json!({"processName":"WPS.EXE"})),
                    "keyboard.sequence" => {
                        if self.cancel_after_first_input {
                            cancellation.cancel();
                        }
                        Ok(json!({"submitted":true}))
                    }
                    _ => Err(ActionError::new("unexpected_method", method)),
                }
            }
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/actions");
        let call: ActionCall = serde_json::from_slice(
            &fs::read(root.join("keyboard-sequence/delete-line-start.json")).unwrap(),
        )
        .unwrap();
        let action = crate::PackageCatalog::load(&root).resolve(&call).unwrap();
        for cancel_after_first_input in [false, true] {
            let host = Host {
                calls: Default::default(),
                cancel_after_first_input,
            };
            let result = NodeRuntime::default().execute_with_host(
                &action,
                &context(),
                &CancellationToken::new(),
                Some(&host),
            );
            let calls = host.calls.lock().unwrap();
            assert_eq!(calls[0].0, "target.info");
            assert_eq!(calls[1].0, "keyboard.sequence");
            assert_eq!(
                calls[1].1,
                json!({"modifiers":[160],"keys":[36],"key_down_delay_ms":10})
            );
            if cancel_after_first_input {
                assert_eq!(result.unwrap_err().code, "cancelled");
                assert_eq!(calls.len(), 2, "取消后不能继续发送 Backspace");
            } else {
                assert!(result.is_ok(), "{result:?}");
                assert_eq!(calls.len(), 3);
                assert_eq!(calls[2].0, "keyboard.sequence");
                assert_eq!(
                    calls[2].1,
                    json!({"modifiers":[],"keys":[8],"key_down_delay_ms":10})
                );
                assert!(calls[2].2.duration_since(calls[1].2) >= Duration::from_millis(80));
            }
        }
    }

    #[test]
    fn timed_keyboard_api_preserves_order_and_rejects_invalid_options_before_dispatch() {
        if !node_available() {
            return;
        }
        #[derive(Default)]
        struct Host(std::sync::Mutex<Vec<Value>>);
        impl NativeHost for Host {
            fn call(
                &self,
                method: &str,
                params: Value,
                _: &CancellationToken,
            ) -> Result<Value, ActionError> {
                self.0
                    .lock()
                    .unwrap()
                    .push(json!({"method":method,"params":params}));
                Ok(json!({"submitted":true}))
            }
        }
        let fixture = Fixture::new(
            r#"export default async ctx => {
            await ctx.desktop.sendKeySequence([163,165,161],[75,67],{keyDownDelayMs:10});
            await ctx.desktop.sendShortcut([163,65],{keyDownDelayMs:20});
            await ctx.desktop.sendShortcut([163,9],{timing:{process_overrides:{'WPS.exe':10}}});
            const invalid=[];
            for (const options of [{keyDownDelayMs:null},{keyDownDelayMs:-1},{keyDownDelayMs:1.5},{keyDownDelayMs:1001},{delay:10},null,
                {timing:null},{timing:{},keyDownDelayMs:0},{timing:{process_overrides:{wps:null}}},{timing:{key_down_delay_ms:1001}}]) {
                try { await ctx.desktop.sendShortcut([65],options); } catch(e) { invalid.push([e.code,e.effectState]); }
            }
            try { await ctx.desktop.sendKeySequence([163],Array(16).fill(65),{keyDownDelayMs:200}); } catch(e) { invalid.push([e.code,e.effectState]); }
            return invalid;
        }"#,
            json!({}),
        );
        let host = Host::default();
        let result = NodeRuntime::default()
            .execute_with_host(
                &fixture.action,
                &context(),
                &CancellationToken::new(),
                Some(&host),
            )
            .unwrap();
        assert_eq!(
            result,
            json!(vec![vec!["invalid_input", "not_started"]; 11])
        );
        let calls = host.0.lock().unwrap();
        assert_eq!(calls.len(), 3);
        assert_eq!(
            calls[0],
            json!({"method":"keyboard.sequence","params":{"modifiers":[163,165,161],"keys":[75,67],"key_down_delay_ms":10}})
        );
        assert_eq!(
            calls[1],
            json!({"method":"keyboard.shortcut","params":{"keys":[163,65],"key_down_delay_ms":20}})
        );
        assert_eq!(
            calls[2],
            json!({"method":"keyboard.shortcut","params":{"keys":[163,9],"timing":{"process_overrides":{"WPS.exe":10}}}})
        );
    }

    #[test]
    fn desktop_host_requires_await_and_reports_unavailable_before_operation() {
        if !node_available() {
            return;
        }
        let fixture = Fixture::new(
            "export default async ctx => await ctx.desktop.target();",
            json!({}),
        );
        let error = fixture.run().unwrap_err();
        assert_eq!(error.code, "desktop_unavailable");
        assert_eq!(error.effect_state.as_deref(), Some("not_started"));
        let fixture = Fixture::new(
            r#"export default async ctx => {
            const first = ctx.desktop.target();
            let busy;
            try { await ctx.desktop.sendShortcut([162,87]); } catch(e) { busy=e.code; }
            await first;
            return busy;
        }"#,
            json!({}),
        );
        let host = DesktopHost::default();
        assert_eq!(
            NodeRuntime::default()
                .execute_with_host(
                    &fixture.action,
                    &context(),
                    &CancellationToken::new(),
                    Some(&host)
                )
                .unwrap(),
            "desktop_busy"
        );
        let fixture = Fixture::new(
            "export default ctx => { void ctx.desktop.target(); return {}; };",
            json!({}),
        );
        assert_eq!(
            NodeRuntime::default()
                .execute_with_host(
                    &fixture.action,
                    &context(),
                    &CancellationToken::new(),
                    Some(&host)
                )
                .unwrap_err()
                .code,
            "pending_desktop_calls"
        );
    }

    #[test]
    fn desktop_host_cancellation_prevents_a_later_input_step() {
        if !node_available() {
            return;
        }
        let fixture = Fixture::new(
            r#"export default async ctx => {
            await ctx.desktop.target();
            await ctx.delay(2000);
            await ctx.desktop.sendShortcut([162,87]);
        }"#,
            json!({}),
        );
        #[derive(Default)]
        struct CancelAfterFirst(std::sync::Mutex<Vec<String>>);
        impl NativeHost for CancelAfterFirst {
            fn call(
                &self,
                method: &str,
                _: Value,
                cancellation: &CancellationToken,
            ) -> Result<Value, ActionError> {
                self.0.lock().unwrap().push(method.into());
                cancellation.cancel();
                Ok(json!({}))
            }
        }
        let host = CancelAfterFirst::default();
        let cancellation = CancellationToken::new();
        assert_eq!(
            NodeRuntime::default()
                .execute_with_host(&fixture.action, &context(), &cancellation, Some(&host))
                .unwrap_err()
                .code,
            "cancelled"
        );
        assert_eq!(*host.0.lock().unwrap(), vec!["target.info"]);
    }

    struct Fixture {
        directory: ResponseDirectory,
        action: ResolvedAction,
    }
    impl Fixture {
        fn new(script: &str, input: Value) -> Self {
            let directory = ResponseDirectory::new().unwrap();
            let root = fs::canonicalize(&directory.0).unwrap();
            let entry_path = root.join("main.mjs");
            fs::write(&entry_path, script).unwrap();
            Self {
                directory,
                action: ResolvedAction {
                    call: ActionCall {
                        package_id: "test.node".into(),
                        action_id: "run".into(),
                        action_version: 1,
                        input,
                    },
                    definition: ActionDefinition {
                        id: "run".into(),
                        title: "测试".into(),
                        version: 1,
                        runtime: "node".into(),
                        entry: "main.mjs".into(),
                    },
                    package_root: root,
                    entry_path,
                },
            }
        }
        fn run(&self) -> Result<Value, ActionError> {
            NodeRuntime::default().execute(&self.action, &context(), &CancellationToken::new())
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_file(self.directory.0.join("main.mjs"));
            for file in ["sdk-js/index.mjs", "sdk-js/protocol.mjs", "connection.json"] {
                let _ = fs::remove_file(self.directory.0.join(file));
            }
            let _ = fs::remove_dir(self.directory.0.join("sdk-js"));
        }
    }

    fn browser_runtime(fixture: &Fixture, sdk: &str) -> NodeRuntime {
        let root = &fixture.action.package_root;
        fs::create_dir(root.join("sdk-js")).unwrap();
        fs::write(root.join("sdk-js/index.mjs"), sdk).unwrap();
        fs::write(root.join("sdk-js/protocol.mjs"), "").unwrap();
        fs::write(root.join("connection.json"), "{}").unwrap();
        NodeRuntime::default().with_browser_connector(root.clone())
    }

    #[test]
    fn browser_host_reuses_client_preserves_values_and_structured_errors() {
        if !node_available() {
            return;
        }
        let fixture = Fixture::new(
            r#"export default async ctx => {
            const profiles = await ctx.browser.profiles();
            const value = await ctx.browser.chrome.bookmarks.getTree();
            const nothing = await ctx.browser.call('tabs.remove', [1], {profile:'test', timeoutMs:100});
            return { profiles, value, undefinedResult: nothing === undefined };
        }"#,
            json!({}),
        );
        let runtime = browser_runtime(
            &fixture,
            r#"
            let connections = 0;
            export async function connect() {
                if (++connections !== 1) throw Error('must reuse connection');
                return { profiles: async () => [{browser_instance_id:'test'}], close() {},
                    async call(method, args, options) {
                        if (!options.signal || options.timeoutMs > 10000) throw Error('missing bounds');
                        if (method === 'bookmarks.getTree') return [{id:'0', title:'中文'}];
                        if (args[0] !== 1 || options.profile !== 'test') throw Error('argument mismatch');
                    } };
            }
        "#,
        );
        let result = runtime
            .execute(&fixture.action, &context(), &CancellationToken::new())
            .unwrap();
        assert_eq!(result["value"][0]["title"], "中文");
        assert_eq!(result["undefinedResult"], true);

        fs::write(
            &fixture.action.entry_path,
            "export default async ctx => ctx.browser.call('sessions.restore')",
        )
        .unwrap();
        fs::write(
            fixture.action.package_root.join("sdk-js/index.mjs"),
            r#"
            export async function connect() { return { close() {}, async call(method) {
                throw Object.assign(Error('需要会话权限'), {code:'permission_required', method,
                    effectState:'not_started', requiredPermissions:['sessions']});
            } }; }
        "#,
        )
        .unwrap();
        let error = runtime
            .execute(&fixture.action, &context(), &CancellationToken::new())
            .unwrap_err();
        assert_eq!(error.code, "permission_required");
        assert_eq!(error.effect_state.as_deref(), Some("not_started"));
        assert_eq!(error.method.as_deref(), Some("sessions.restore"));
        assert_eq!(error.required_permissions, ["sessions"]);
    }

    #[test]
    fn browser_host_reports_missing_connector_and_rejects_unawaited_calls() {
        if !node_available() {
            return;
        }
        let fixture = Fixture::new(
            "export default async ctx => ctx.browser.profiles()",
            json!({}),
        );
        let error = fixture.run().unwrap_err();
        assert_eq!(error.code, "browser_unavailable");
        assert_eq!(error.effect_state.as_deref(), Some("not_started"));
        let runtime = browser_runtime(&fixture, "export async function connect() { return { close() {}, call: async () => new Promise(() => {}) }; }");
        fs::write(
            &fixture.action.entry_path,
            "export default ctx => { ctx.browser.call('tabs.create', [{}]); return {}; }",
        )
        .unwrap();
        assert_eq!(
            runtime
                .execute(&fixture.action, &context(), &CancellationToken::new())
                .unwrap_err()
                .code,
            "pending_browser_calls"
        );
        fs::write(
            &fixture.action.entry_path,
            "export default async ctx => ctx.browser.call('tabs.query', [], {timeoutMs:0})",
        )
        .unwrap();
        assert_eq!(
            runtime
                .execute(&fixture.action, &context(), &CancellationToken::new())
                .unwrap_err()
                .code,
            "invalid_input"
        );
    }

    #[test]
    fn browser_host_bounds_calls_before_connection_is_ready() {
        if !node_available() {
            return;
        }
        let fixture = Fixture::new(
            r#"export default async ctx => {
                const results = await Promise.allSettled(Array.from({length:9}, () => ctx.browser.call('tabs.query', [])));
                return results.map(result => result.status === 'fulfilled' ? result.value : {code:result.reason.code, effectState:result.reason.effectState});
            }"#,
            json!({}),
        );
        let runtime = browser_runtime(
            &fixture,
            r#"
            export async function connect() {
                await new Promise(resolve => setTimeout(resolve, 30));
                let calls = 0;
                return {close() {}, async call() {return ++calls;}};
            }
        "#,
        );
        let output = runtime
            .execute(&fixture.action, &context(), &CancellationToken::new())
            .unwrap();
        assert_eq!(output[7], 8);
        assert_eq!(
            output[8],
            json!({"code":"busy","effectState":"not_started"})
        );
    }

    #[test]
    fn browser_host_timeout_and_cancellation_bound_connection_and_calls() {
        if !node_available() {
            return;
        }
        let fixture = Fixture::new(
            "export default async ctx => ctx.browser.call('tabs.query', [])",
            json!({}),
        );
        let mut runtime = browser_runtime(&fixture, "export async function connect() { return {close() {}, call: async () => new Promise(() => {})}; }");
        runtime.timeout = Duration::from_millis(250);
        let began = Instant::now();
        assert_eq!(
            runtime
                .execute(&fixture.action, &context(), &CancellationToken::new())
                .unwrap_err()
                .code,
            "timeout"
        );
        assert!(began.elapsed() < Duration::from_secs(5));
        fs::write(
            fixture.action.package_root.join("sdk-js/index.mjs"),
            "export async function connect() { return new Promise(() => {}); }",
        )
        .unwrap();
        let token = CancellationToken::new();
        let cancel = token.clone();
        let cancelling = thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            cancel.cancel();
        });
        assert_eq!(
            runtime
                .execute(&fixture.action, &context(), &token)
                .unwrap_err()
                .code,
            "cancelled"
        );
        cancelling.join().unwrap();
    }
    fn context() -> InvocationContext {
        InvocationContext {
            invocation_id: u64::MAX,
            binding_id: 42,
        }
    }

    fn node_available() -> bool {
        let mut command = Command::new("node");
        command
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        hide_window(&mut command);
        command.status().is_ok_and(|status| status.success())
    }

    #[test]
    fn node_round_trip_runs_arguments_without_shell_and_preserves_large_ids() {
        if !node_available() {
            eprintln!("跳过 Node 集成测试：未安装 Node。");
            return;
        }
        let fixture = Fixture::new(
            r#"export default async ctx => {
            console.log('日志不会污染响应');
            const result = await ctx.process.run(process.execPath, ['-e', 'process.stdout.write(JSON.stringify(process.argv.slice(1)))', ...ctx.input.args]);
            await ctx.delay(1);
            return { args: JSON.parse(result.stdout), id: ctx.invocation.invocation_id, code: result.code };
        }"#,
            json!({"args":["有 空格", "a\"b", "$env:HOME", "$(echo nope)"]}),
        );
        let output = fixture.run().unwrap();
        assert_eq!(output["args"], fixture.action.call.input["args"]);
        assert_eq!(output["id"], u64::MAX.to_string());
        assert_eq!(output["code"], 0);
    }

    #[test]
    fn node_failure_stops_sequence_and_rejects_missing_or_oversized_results() {
        if !node_available() {
            eprintln!("跳过 Node 集成测试：未安装 Node。");
            return;
        }
        for (script, expected) in [
            ("export default async ctx => { await ctx.process.run(process.execPath, ['-e', 'process.exit(7)']); throw new Error('should not run'); }", "process_failed"),
            ("export default async () => { process.exit(0); }", "invalid_response"),
            ("export default async () => 'x'.repeat(70000)", "output_limit"),
            ("export default async () => 1n", "invalid_output"),
            ("export default async ctx => ctx.process.run(process.execPath, ['-e', 'process.stdout.write(\"x\".repeat(70000))'])", "output_limit"),
        ] {
            assert_eq!(Fixture::new(script, json!({})).run().unwrap_err().code, expected);
        }
    }

    #[test]
    fn node_cancel_and_cpu_timeout_finish_without_waiting_for_script() {
        if !node_available() {
            eprintln!("跳过 Node 集成测试：未安装 Node。");
            return;
        }
        let fixture = Fixture::new(
            "export default async ctx => { await ctx.delay(60000); return 'bad'; }",
            json!({}),
        );
        let token = CancellationToken::new();
        let cancel = token.clone();
        let cancelling = thread::spawn(move || {
            thread::sleep(Duration::from_millis(150));
            cancel.cancel();
        });
        let began = Instant::now();
        let result = NodeRuntime::default().execute(&fixture.action, &context(), &token);
        assert_eq!(result.unwrap_err().code, "cancelled");
        assert!(began.elapsed() < Duration::from_secs(5));
        cancelling.join().unwrap();
        let fixture = Fixture::new("export default async () => { while (true) {} }", json!({}));
        let runtime = NodeRuntime {
            program: "node".into(),
            timeout: Duration::from_millis(100),
            browser_connector: None,
        };
        assert_eq!(
            runtime
                .execute(&fixture.action, &context(), &CancellationToken::new())
                .unwrap_err()
                .code,
            "timeout"
        );
    }

    #[test]
    fn node_start_returns_before_independent_program_finishes() {
        if !node_available() {
            eprintln!("跳过 Node 集成测试：未安装 Node。");
            return;
        }
        let mut fixture = Fixture::new(
            r#"export default async ctx => {
            return await ctx.process.start(process.execPath, ['-e',
                'const fs = require("node:fs"); const timer = setInterval(() => { if (fs.existsSync(process.argv[1] + ".release")) { fs.writeFileSync(process.argv[1], "continued"); clearInterval(timer); process.exit(0); } }, 20); setTimeout(() => process.exit(9), 30000)',
                ctx.input.output]);
        }"#,
            json!({}),
        );
        let output = fixture.action.package_root.join("continued.txt");
        fixture.action.call.input = json!({"output":output});
        let result = fixture.run().unwrap();
        assert!(result["pid"].as_u64().is_some_and(|pid| pid > 0));
        let release = output.with_extension("txt.release");
        fs::write(&release, "release").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while fs::read_to_string(&output).ok().as_deref() != Some("continued")
            && Instant::now() < deadline
        {
            thread::sleep(POLL);
        }
        assert_eq!(fs::read_to_string(&output).unwrap(), "continued");
        fs::remove_file(output).unwrap();
        fs::remove_file(release).unwrap();
    }

    #[test]
    fn node_cooperative_cancel_terminates_a_managed_run_child() {
        if !node_available() {
            eprintln!("跳过 Node 集成测试：未安装 Node。");
            return;
        }
        let mut fixture = Fixture::new(
            r#"export default async ctx => {
            return await ctx.process.run(process.execPath, ['-e',
                'require("node:fs").writeFileSync(process.argv[1], String(process.pid)); setTimeout(() => {}, 10000)',
                ctx.input.started]);
        }"#,
            json!({}),
        );
        let started = fixture.action.package_root.join("started.txt");
        fixture.action.call.input = json!({"started":started});
        let token = CancellationToken::new();
        let cancel = token.clone();
        let observed = started.clone();
        let cancelling = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            while fs::read_to_string(&observed)
                .ok()
                .and_then(|value| value.parse::<u32>().ok())
                .is_none()
                && Instant::now() < deadline
            {
                thread::sleep(POLL);
            }
            cancel.cancel();
        });
        let result = NodeRuntime::default().execute(&fixture.action, &context(), &token);
        cancelling.join().unwrap();
        assert_eq!(result.unwrap_err().code, "cancelled");
        let pid = fs::read_to_string(&started).unwrap();
        let mut probe = Command::new("node");
        probe.args(["-e", "try { process.kill(Number(process.argv[1]), 0); process.exit(1); } catch { process.exit(0); }", &pid]);
        hide_window(&mut probe);
        assert!(
            probe.status().unwrap().success(),
            "被取消的 run 子程序仍然运行"
        );
        fs::remove_file(started).unwrap();
    }

    #[test]
    fn response_requires_matching_id_version_and_terminal_envelope() {
        let scratch = ResponseDirectory::new().unwrap();
        for value in [
            json!({"api_version":2,"invocation_id":"1","status":"ok","output":null}),
            json!({"api_version":1,"invocation_id":"2","status":"ok","output":null}),
            json!({"api_version":1,"invocation_id":"1","status":"error"}),
            json!({"api_version":1,"invocation_id":"1","status":"ok","error":{"code":"x","message":"x"}}),
        ] {
            fs::write(scratch.response(), serde_json::to_vec(&value).unwrap()).unwrap();
            assert_eq!(
                read_response(&scratch.response(), 1).unwrap_err().code,
                "invalid_response"
            );
        }
    }

    #[test]
    fn missing_runtime_and_replaced_entry_are_explicit_failures() {
        let fixture = Fixture::new("export default async () => null", json!({}));
        let runtime = NodeRuntime::new(fixture.action.package_root.join("no-such-node.exe"));
        assert_eq!(
            runtime
                .execute(&fixture.action, &context(), &CancellationToken::new())
                .unwrap_err()
                .code,
            "runtime_start_failed"
        );
        fs::remove_file(&fixture.action.entry_path).unwrap();
        assert_eq!(fixture.run().unwrap_err().code, "entry_unavailable");
    }
}
