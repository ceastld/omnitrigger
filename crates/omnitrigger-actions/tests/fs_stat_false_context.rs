//! 实际ActionService→可选NodeRuntime/ScriptHostRuntime的fs.stat只读合同；严格FakeHost不查询真实目标。
//! 原目标false保持原值，path为夹具独占根下明确不存在的文件。
#![cfg(windows)]
use omnitrigger_actions::{
    ActionCall, ActionError, ActionOutcome, ActionService, CancellationToken, NativeHost,
    NodeRuntime, PackageCatalog, RuntimeRegistry, ScriptHostRuntime,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const MANIFEST: &[u8] = include_bytes!("fixtures/fs_stat_false_context/action.json");
const AUTHOR: &[u8] = include_bytes!("fixtures/fs_stat_false_context/main.mjs");

fn local(path: &Path) {
    use std::os::windows::fs::MetadataExt;
    assert!(path.is_absolute());
    for part in path.ancestors() {
        let metadata = fs::symlink_metadata(part).unwrap();
        let name = part
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase();
        assert!(!name.starts_with("onedrive") && name != "dropbox" && !name.starts_with("icloud"));
        assert!(!metadata.file_type().is_symlink() && metadata.file_attributes() & 0x441400 == 0);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimeMode {
    Script,
    Node,
}
impl RuntimeMode {
    fn selected() -> Self {
        match std::env::var("OMNITRIGGER_FALSE_CONTEXT_RUNTIME") {
            Err(std::env::VarError::NotPresent) => Self::Script,
            Ok(value) if value == "script" => Self::Script,
            Ok(value) if value == "node" => Self::Node,
            _ => panic!("OMNITRIGGER_FALSE_CONTEXT_RUNTIME仅接受node/script，省略为script"),
        }
    }
    fn program_env(self) -> &'static str {
        match self {
            Self::Script => "OMNITRIGGER_TEST_SCRIPT_HOST",
            Self::Node => "OMNITRIGGER_TEST_NODE",
        }
    }
    fn write_package(self, package: &Path) {
        match self {
            Self::Script => {
                // 保持原Script夹具字节，不因Node测试改写它。
                fs::write(package.join("action.json"), MANIFEST).unwrap();
                fs::write(package.join("main.mjs"), AUTHOR).unwrap();
            }
            Self::Node => {
                let mut manifest: Value = serde_json::from_slice(MANIFEST).unwrap();
                let actions = manifest["actions"].as_array_mut().unwrap();
                assert_eq!(actions.len(), 1);
                assert_eq!(actions[0]["runtime"], "script");
                actions[0]["runtime"] = "node".into();
                fs::write(
                    package.join("action.json"),
                    serde_json::to_vec(&manifest).unwrap(),
                )
                .unwrap();
                let mut author = AUTHOR.to_vec();
                author.extend_from_slice(b"\nexport default main;\n");
                fs::write(package.join("main.mjs"), author).unwrap();
            }
        }
    }
}

struct Fixture {
    root: PathBuf,
    actions: PathBuf,
    package: PathBuf,
    program: PathBuf,
}
impl Fixture {
    fn new(runtime: RuntimeMode) -> Self {
        let temporary = std::env::temp_dir();
        local(&temporary);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = temporary.join(format!(
            "omni-stat-false-context-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        local(&root);
        let actions = root.join("actions");
        let package = actions.join("stat-api");
        fs::create_dir(&actions).unwrap();
        fs::create_dir(&package).unwrap();
        runtime.write_package(&package);
        let fixture = Self {
            program: root.join("never.txt"),
            root,
            actions,
            package,
        };
        fixture.no_program();
        fixture
    }
    fn no_program(&self) {
        assert_eq!(
            fs::symlink_metadata(&self.program).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
        assert_eq!(fs::read_dir(&self.root).unwrap().count(), 1);
        assert_eq!(fs::read_dir(&self.package).unwrap().count(), 2);
        for name in ["action.json", "main.mjs"] {
            local(&self.package.join(name));
        }
    }
    fn input(&self) -> Value {
        json!({"path":self.program.to_str().unwrap()})
    }
    fn expected(&self) -> Value {
        self.input()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if thread::panicking() {
            eprintln!(
                "保留失败夹具，避免清理异常覆盖原始错误：{}",
                self.root.display()
            );
            return;
        }
        // 仅移除本夹具精确创建的两文件和三目录，无递归删除、无进程收尾。
        self.no_program();
        for name in ["main.mjs", "action.json"] {
            fs::remove_file(self.package.join(name)).unwrap();
        }
        fs::remove_dir(&self.package).unwrap();
        fs::remove_dir(&self.actions).unwrap();
        fs::remove_dir(&self.root).unwrap();
    }
}

#[derive(Clone, Copy, Debug)]
enum Case {
    Success(&'static str),
    Unsupported,
    Timeout,
    BadReceipt,
    MissingHost,
}

struct Host {
    case: Case,
    runtime: RuntimeMode,
    expected: Value,
    method: &'static str,
    calls: Mutex<Vec<Value>>,
}
impl Host {
    fn respond(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
        deadline: Option<Instant>,
    ) -> Result<Value, ActionError> {
        assert_eq!(method, self.method);
        assert!(!cancellation.is_cancelled());
        assert_eq!(params, self.expected);
        let deadline = deadline.expect("实际桥必须保留外层deadline");
        if self.runtime == RuntimeMode::Script {
            assert!(
                deadline > Instant::now() && deadline <= Instant::now() + Duration::from_secs(8)
            );
        } else {
            // NodeRuntime公开构造器的真实默认预算为60秒，没有虚构with_timeout接口。
            assert!(
                deadline > Instant::now() && deadline <= Instant::now() + Duration::from_secs(60)
            );
        }
        let mut calls = self.calls.lock().unwrap();
        assert!(
            calls.is_empty(),
            "唯一stat请求；禁止本地查询/重试/窗口或剪贴板API"
        );
        calls.push(json!({"method":method,"params":params}));
        match self.case {
            Case::Success(kind) => Ok(json!({"kind":kind})),
            Case::BadReceipt => Ok(json!({"kind":"missing","extra":true})),
            Case::Unsupported | Case::Timeout => {
                let (code, effect) = if matches!(self.case, Case::Unsupported) {
                    ("fs_unavailable", "not_started")
                } else {
                    ("timeout", "not_started")
                };
                let mut error = ActionError::new(code, "仅合成Host响应，无实际路径查询。");
                error.method = Some(method.into());
                error.effect_state = Some(effect.into());
                Err(error)
            }
            Case::MissingHost => panic!("MissingHost没有内层Host"),
        }
    }
}
impl NativeHost for Host {
    fn native_context_available(&self) -> bool {
        false
    }
    fn call(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
    ) -> Result<Value, ActionError> {
        self.respond(method, params, cancellation, None)
    }
    fn call_with_deadline(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<Value, ActionError> {
        self.respond(method, params, cancellation, Some(deadline))
    }
}

fn run(case: Case) {
    let method = "fs.stat";
    let runtime = RuntimeMode::selected();
    let began = Instant::now();
    eprintln!("服务场景开始：runtime={runtime:?} case={case:?}");
    let fixture = Fixture::new(runtime);
    let host = (!matches!(case, Case::MissingHost)).then(|| {
        Arc::new(Host {
            case,
            runtime,
            expected: fixture.expected(),
            method,
            calls: Mutex::new(vec![]),
        })
    });
    let program = PathBuf::from(
        std::env::var_os(runtime.program_env())
            .expect("显式指定本次选择的实际Node/ScriptHost绝对路径，不静默跳过"),
    );
    local(&program);
    assert!(
        !program.starts_with(&fixture.root),
        "夹具根不得放可执行Host"
    );
    let mut runtimes = RuntimeRegistry::new();
    match runtime {
        RuntimeMode::Script => runtimes
            .register(
                "script",
                Arc::new(ScriptHostRuntime::new(program).with_timeout(Duration::from_secs(8))),
            )
            .unwrap(),
        RuntimeMode::Node => runtimes
            .register("node", Arc::new(NodeRuntime::new(program)))
            .unwrap(),
    }
    let service = ActionService::new(PackageCatalog::load(&fixture.actions), runtimes);
    let native = host
        .as_ref()
        .map(|host| host.clone() as Arc<dyn NativeHost>);
    let invocation = service
        .submit_with_host(
            17,
            ActionCall {
                package_id: "test.fs-stat-false-context".into(),
                action_id: "stat".into(),
                action_version: 1,
                input: fixture.input(),
            },
            native,
        )
        .unwrap();
    // Script沿原8秒运行时预算加收尾余量；Node默认60秒，分别约束服务等待。
    let until = Instant::now()
        + Duration::from_secs(if runtime == RuntimeMode::Script {
            12
        } else {
            64
        });
    let mut started = 0;
    let result = loop {
        assert!(Instant::now() < until, "API测试未有界完成：{case:?}");
        match service.try_recv() {
            Ok(event) => {
                assert_eq!(event.invocation_id, invocation);
                match event.outcome {
                    ActionOutcome::Started => started += 1,
                    ActionOutcome::Succeeded { output } => break Ok(output),
                    ActionOutcome::Failed { error } => break Err(error),
                    ActionOutcome::Cancelled => panic!("未请求取消：{case:?}"),
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => thread::sleep(Duration::from_millis(2)),
            Err(error) => panic!("事件通道失败：{error}"),
        }
    };
    eprintln!(
        "服务场景终态：runtime={runtime:?} case={case:?} elapsed={:?}",
        began.elapsed()
    );
    assert_eq!(started, 1);
    if let Some(host) = host {
        assert!(!host.native_context_available());
        assert_eq!(
            *host.calls.lock().unwrap(),
            vec![json!({"method":method,"params":fixture.expected()})]
        );
    }
    match case {
        Case::Success(kind) => assert_eq!(result.unwrap(), json!({"kind":kind})),
        _ => {
            let error = result.unwrap_err();
            let (code, effect) = match case {
                Case::Unsupported => ("fs_unavailable", "not_started"),
                Case::MissingHost => ("fs_error", "not_started"),
                Case::Timeout => ("timeout", "not_started"),
                Case::BadReceipt => ("fs_error", "not_started"),
                Case::Success(_) => unreachable!(),
            };
            assert_eq!(error.code, code, "{case:?}");
            assert_eq!(error.method.as_deref(), Some(method));
            assert_eq!(error.effect_state.as_deref(), Some(effect));
        }
    }
    fixture.no_program();
}

#[test]
#[ignore = "显式选择实际Node/ScriptHost；只合成Host，不查询目标路径"]
fn fs_stat_false_context_success_crosses_the_real_service_bridge() {
    for kind in ["file", "directory", "missing", "other"] {
        run(Case::Success(kind));
    }
}
#[test]
#[ignore = "显式选择实际Node/ScriptHost；只合成Host，不查询目标路径"]
fn fs_stat_false_context_errors_do_not_fall_back_or_retry() {
    for case in [
        Case::Unsupported,
        Case::Timeout,
        Case::BadReceipt,
        Case::MissingHost,
    ] {
        run(case);
    }
}
