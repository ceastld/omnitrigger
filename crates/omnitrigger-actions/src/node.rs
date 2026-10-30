//! 每次调用一个 Node 进程；运行代码的权限与当前用户相同，不构成沙盒。
//! 协议结果走独立临时文件，普通 stdout/stderr 不进入协议或应用历史。
use crate::{ActionError, ActionRuntime, CancellationToken, InvocationContext, ResolvedAction};
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
}

impl NodeRuntime {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            timeout: Duration::from_secs(60),
        }
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
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        // 目录是加载时快照，执行前再次核验入口，避免文件被替换为链接后越界。
        let (root, entry) = checked_entry(action)?;
        let scratch = ResponseDirectory::new()?;
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
                    Some(ActionError::new(
                        "timeout",
                        "动作超过 60 秒运行时限；不会自动重试。",
                    ))
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
    ActionError::new("cancelled", "动作已取消；已经完成的操作不会撤销。")
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
        }
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
