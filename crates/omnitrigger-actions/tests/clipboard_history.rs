//! 历史 API 的真实 Node/Boa/QuickJS 与合成 NativeHost 集成测试。
//! NativeHost 只收发内存中的合成 JSON；不链接/调用 Win32 剪贴板或输入实现。
//! 真实进程用例显式 ignored，由 CI/串行验证指定运行时后执行。
use omnitrigger_actions::{
    ActionCall, ActionDefinition, ActionError, ActionRuntime, CancellationToken, InvocationContext,
    NativeHost, NodeRuntime, ResolvedAction, ScriptHostRuntime,
};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, SyncSender},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

const PAYLOAD: &str = "synthetic_private_history_body_合成😀";
static NEXT: AtomicU64 = AtomicU64::new(1);

fn local_metadata(path: &Path) -> fs::Metadata {
    for component in path.components() {
        let name = component.as_os_str().to_string_lossy().to_ascii_lowercase();
        assert!(!name.starts_with("onedrive") && !name.starts_with("icloud") && name != "dropbox");
    }
    let mut ancestors: Vec<_> = path.ancestors().collect();
    ancestors.reverse();
    for ancestor in ancestors {
        let metadata = fs::symlink_metadata(ancestor).unwrap();
        assert!(!metadata.file_type().is_symlink());
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            assert_eq!(
                metadata.file_attributes() & (0x400 | 0x1000 | 0x40000 | 0x400000),
                0
            );
        }
    }
    fs::symlink_metadata(path).unwrap()
}

#[derive(Clone, Copy)]
enum Engine {
    Node,
    Light,
}
impl Engine {
    fn id(self) -> &'static str {
        match self {
            Self::Node => "node",
            Self::Light => "script",
        }
    }
    fn entry(self) -> &'static str {
        match self {
            Self::Node => "main.mjs",
            Self::Light => "main.js",
        }
    }
    fn call(self, expression: &str) -> String {
        match self {
            Self::Node => format!("await {expression}"),
            Self::Light => expression.into(),
        }
    }
    fn source(self, body: &str) -> String {
        match self {
            Self::Node => format!("export default async function main(ctx) {{\n{body}\n}}"),
            Self::Light => format!("function main(ctx) {{\n{body}\n}}"),
        }
    }
    fn runtime(self) -> Box<dyn ActionRuntime> {
        match self {
            Self::Node => {
                let program = std::env::var_os("OMNITRIGGER_TEST_NODE")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("node"));
                if program.is_absolute() {
                    assert!(local_metadata(&program).is_file());
                }
                Box::new(NodeRuntime::new(program))
            }
            Self::Light => {
                let program = PathBuf::from(
                    std::env::var_os("OMNITRIGGER_TEST_SCRIPT_HOST")
                        .expect("设置实际构建的 Boa 或 QuickJS 宿主绝对路径"),
                );
                assert!(program.is_absolute() && local_metadata(&program).is_file());
                Box::new(ScriptHostRuntime::new(program))
            }
        }
    }
}

struct Fixture {
    base: PathBuf,
    root: PathBuf,
    engine: Engine,
    entry: &'static str,
    workflow: bool,
}
impl Fixture {
    fn new(engine: Engine, source: &str) -> Self {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        assert!(local_metadata(&manifest).is_dir());
        let repo = manifest
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .canonicalize()
            .unwrap();
        let mut base = repo;
        for name in [".tmp", "continuous-actions", "sdk", "runtime-cases"] {
            base.push(name);
            match fs::symlink_metadata(&base) {
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    match fs::create_dir(&base) {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                        Err(error) => panic!("无法创建隔离测试目录：{error}"),
                    }
                }
                Err(error) => panic!("无法检查隔离测试目录：{error}"),
            }
            assert!(local_metadata(&base).is_dir());
        }
        let root = base.join(format!(
            "case-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::write(root.join(engine.entry()), source).unwrap();
        Self {
            base,
            root,
            engine,
            entry: engine.entry(),
            workflow: false,
        }
    }
    fn action(&self, input: Value) -> ResolvedAction {
        ResolvedAction {
            package_root: self.root.clone(),
            entry_path: self.root.join(self.entry),
            call: ActionCall {
                package_id: "clipboard-history-fixture".into(),
                action_id: "run".into(),
                action_version: 1,
                input,
            },
            definition: ActionDefinition {
                id: "run".into(),
                title: "合成历史契约测试".into(),
                version: 1,
                runtime: self.engine.id().into(),
                entry: self.entry.into(),
                single_instance: true,
            },
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // 只删除本测试创建的一个入口和空目录；不递归遍历或清理共享目录。
        let root = self.root.canonicalize().unwrap();
        let base = self.base.canonicalize().unwrap();
        assert_eq!(root.parent(), Some(base.as_path()));
        assert!(root
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("case-"));
        let entry = root.join(self.entry);
        assert!(local_metadata(&entry).is_file());
        fs::remove_file(entry).unwrap();
        if self.workflow {
            let workflow = root.join("workflow.mjs");
            assert!(local_metadata(&workflow).is_file());
            fs::remove_file(workflow).unwrap();
        }
        fs::remove_dir(root).unwrap();
    }
}
fn context() -> InvocationContext {
    InvocationContext {
        invocation_id: u64::MAX,
        binding_id: 9_007_199_254_740_993,
    }
}
// 此文件由 prepare-runtime-draft.py 接在本项目现有隔离夹具后；不是独立 Cargo 测试。
struct Expected {
    method: &'static str,
    params: Value,
    reply: Result<Value, ActionError>,
}
impl Expected {
    fn value(method: &'static str, params: Value, value: Value) -> Self {
        Self {
            method,
            params,
            reply: Ok(value),
        }
    }
    fn error(method: &'static str, params: Value, code: &str, effect: &str) -> Self {
        let mut error = ActionError::new(code, PAYLOAD);
        error.method = Some(method.into());
        error.effect_state = Some(effect.into());
        error.required_permissions = vec![PAYLOAD.into()];
        Self {
            method,
            params,
            reply: Err(error),
        }
    }
}
struct Host {
    expected: Mutex<VecDeque<Expected>>,
    calls: AtomicU64,
}
impl Host {
    fn new(expected: Vec<Expected>) -> Self {
        Self {
            expected: Mutex::new(expected.into()),
            calls: AtomicU64::new(0),
        }
    }
    fn finished(&self, count: u64) {
        assert!(self.expected.lock().unwrap().is_empty());
        assert_eq!(self.calls.load(Ordering::Acquire), count);
    }
}
impl NativeHost for Host {
    fn call(
        &self,
        method: &str,
        params: Value,
        _: &CancellationToken,
    ) -> Result<Value, ActionError> {
        let expected = self
            .expected
            .lock()
            .unwrap()
            .pop_front()
            .expect("不得回退或自动重试");
        self.calls.fetch_add(1, Ordering::AcqRel);
        assert_eq!(
            method, expected.method,
            "不应请求真实剪贴板、前台或其他本机能力"
        );
        assert_eq!(params, expected.params);
        expected.reply
    }
}
fn run(engine: Engine, body: &str, host: Option<&dyn NativeHost>) -> Result<Value, ActionError> {
    let fixture = Fixture::new(engine, &engine.source(body));
    engine.runtime().execute_with_host(
        &fixture.action(json!({})),
        &context(),
        &CancellationToken::new(),
        host,
    )
}
fn fault(error: &ActionError, code: &str, method: &str, effect: &str) {
    assert_eq!(error.code, code);
    assert_eq!(error.method.as_deref(), Some(method));
    assert_eq!(error.effect_state.as_deref(), Some(effect));
    assert!(error.required_permissions.is_empty());
    assert!(!serde_json::to_string(error).unwrap().contains(PAYLOAD));
}
fn summary(id: &str, units: usize) -> Value {
    json!({"id":id,"preview":"合成摘要","utf16Units":units,"createdAt":"0"})
}

fn roundtrip(engine: Engine) {
    let host = Host::new(vec![
        Expected::value(
            "history.append",
            json!({"text":PAYLOAD}),
            json!({"id":"9007199254740993","revision":"1","inserted":true}),
        ),
        Expected::value(
            "history.list",
            json!({"limit":100}),
            json!({"items":[summary("9007199254740993",PAYLOAD.encode_utf16().count())],"revision":"1"}),
        ),
        Expected::value(
            "history.get",
            json!({"id":"9007199254740993"}),
            json!({"id":"9007199254740993","text":PAYLOAD,"createdAt":"0","revision":"1"}),
        ),
        Expected::value(
            "history.remove",
            json!({"id":"9007199254740993"}),
            json!({"removed":true,"revision":"2"}),
        ),
    ]);
    let append = engine.call(&format!(
        "ctx.clipboard.history.append({{text:{}}})",
        serde_json::to_string(PAYLOAD).unwrap()
    ));
    let list = engine.call("ctx.clipboard.history.list()");
    let get = engine.call("ctx.clipboard.history.get({id:'9007199254740993'})");
    let remove = engine.call("ctx.clipboard.history.remove({id:'9007199254740993'})");
    let output=run(engine,&format!("const saved={append};const listing={list};const item={get};const removed={remove};return {{id:saved.id,inserted:saved.inserted,itemCount:listing.items.length,units:item.text.length,removed:removed.removed,revision:removed.revision}};"),Some(&host)).unwrap();
    assert_eq!(
        output,
        json!({"id":"9007199254740993","inserted":true,"itemCount":1,"units":PAYLOAD.encode_utf16().count(),"removed":true,"revision":"2"})
    );
    assert!(!output.to_string().contains(PAYLOAD));
    assert!(output.get("text").is_none());
    host.finished(4);
}

fn large_values(engine: Engine) {
    let text = "😀".repeat(32768);
    let items: Vec<_> = (1..=200).map(|id| summary(&id.to_string(), 4096)).collect();
    let host = Host::new(vec![
        Expected::value(
            "history.list",
            json!({"limit":200}),
            json!({"items":items,"revision":"200"}),
        ),
        Expected::value(
            "history.get",
            json!({"id":"1"}),
            json!({"id":"1","text":text.clone(),"createdAt":"0","revision":"200"}),
        ),
        Expected::value(
            "history.append",
            json!({"text":text}),
            json!({"id":"201","revision":"201","inserted":true}),
        ),
    ]);
    let list = engine.call("ctx.clipboard.history.list({limit:200})");
    let get = engine.call("ctx.clipboard.history.get({id:'1'})");
    let append = engine.call("ctx.clipboard.history.append({text:'😀'.repeat(32768)})");
    let output=run(engine,&format!("const listing={list};const item={get};const saved={append};return {{itemCount:listing.items.length,units:item.text.length,inserted:saved.inserted}};"),Some(&host)).unwrap();
    assert_eq!(
        output,
        json!({"itemCount":200,"units":65536,"inserted":true})
    );
    host.finished(3);
}

fn invalid_inputs(engine: Engine) {
    let cases = [
        ("list", "{limit:0}"),
        ("list", "{limit:201}"),
        ("list", "{limit:1.5}"),
        ("list", "{extra:true}"),
        ("get", "{id:1}"),
        ("get", "{id:'0'}"),
        ("get", "{id:'01'}"),
        ("get", "{id:'18446744073709551616'}"),
        ("append", "{text:'\\0'}"),
        ("append", "{text:'\\ud800'}"),
        ("append", "{text:'x'.repeat(65537)}"),
        ("append", "{text:'',path:'synthetic'}"),
        ("remove", "{id:'1',extra:true}"),
    ];
    for (method, args) in cases {
        let host = Host::new(vec![]);
        let expression = engine.call(&format!("ctx.clipboard.history.{method}({args})"));
        let error = run(
            engine,
            &format!("{expression};return {{unexpected:true}};"),
            Some(&host),
        )
        .unwrap_err();
        fault(
            &error,
            "invalid_input",
            &format!("history.{method}"),
            "not_started",
        );
        host.finished(0);
    }
}

fn malformed_results(engine: Engine) {
    let invalid = vec![
        ("list", json!({"items":[],"revision":"1","text":PAYLOAD})),
        (
            "list",
            json!({"items":[summary("1",10),summary("1",10)],"revision":"1"}),
        ),
        (
            "list",
            json!({"items":[{"id":"1","preview":"a\nb","utf16Units":3,"createdAt":"0"}],"revision":"1"}),
        ),
        (
            "list",
            json!({"items":[{"id":"1","preview":"long","utf16Units":0,"createdAt":"0"}],"revision":"1"}),
        ),
        ("list", json!({"items":[],"revision":1})),
        (
            "get",
            json!({"id":"2","text":PAYLOAD,"createdAt":"0","revision":"1"}),
        ),
        (
            "get",
            json!({"id":"1","text":"a\0b","createdAt":"0","revision":"1"}),
        ),
        (
            "get",
            json!({"id":"1","text":"x".repeat(65537),"createdAt":"0","revision":"1"}),
        ),
        ("append", json!({"id":"1","revision":"1","inserted":1})),
        (
            "append",
            json!({"id":"1","revision":"1","inserted":true,"text":PAYLOAD}),
        ),
        ("remove", json!({"removed":true})),
    ];
    for (method, reply) in invalid {
        let (params, args) = match method {
            "list" => (json!({"limit":100}), ""),
            "append" => (json!({"text":""}), "{text:''}"),
            _ => (json!({"id":"1"}), "{id:'1'}"),
        };
        let full_method = match method {
            "list" => "history.list",
            "get" => "history.get",
            "append" => "history.append",
            _ => "history.remove",
        };
        let host = Host::new(vec![Expected::value(full_method, params, reply)]);
        let expression = engine.call(&format!("ctx.clipboard.history.{method}({args})"));
        let error = run(
            engine,
            &format!("{expression};return {{unexpected:true}};"),
            Some(&host),
        )
        .unwrap_err();
        fault(
            &error,
            "invalid_response",
            full_method,
            if matches!(method, "append" | "remove") {
                "unknown"
            } else {
                "not_started"
            },
        );
        host.finished(1);
    }
}

fn failure_stages(engine: Engine) {
    for method in [
        "history.list",
        "history.get",
        "history.append",
        "history.remove",
    ] {
        for effect in ["not_started", "unknown"] {
            let (params, args) = match method {
                "history.list" => (json!({"limit":100}), ""),
                "history.append" => (json!({"text":""}), "{text:''}"),
                _ => (json!({"id":"1"}), "{id:'1'}"),
            };
            let host = Host::new(vec![Expected::error(
                method,
                params,
                "history_conflict",
                effect,
            )]);
            let expression =
                engine.call(&format!("ctx.clipboard.history.{}({args})", &method[8..]));
            let error = run(
                engine,
                &format!("{expression};return {{unexpected:true}};"),
                Some(&host),
            )
            .unwrap_err();
            fault(
                &error,
                "history_conflict",
                method,
                if matches!(method, "history.list" | "history.get") {
                    "not_started"
                } else {
                    effect
                },
            );
            host.finished(1);
        }
    }
}

fn unavailable(engine: Engine) {
    for (method, args) in [
        ("list", ""),
        ("get", "{id:'1'}"),
        ("append", "{text:''}"),
        ("remove", "{id:'1'}"),
    ] {
        let expression = engine.call(&format!("ctx.clipboard.history.{method}({args})"));
        let error = run(
            engine,
            &format!("{expression};return {{unexpected:true}};"),
            None,
        )
        .unwrap_err();
        fault(
            &error,
            "history_unavailable",
            &format!("history.{method}"),
            "not_started",
        );
    }
}

struct CancelHost {
    entered: SyncSender<()>,
    finished: AtomicBool,
    observed: AtomicBool,
    method: &'static str,
}
impl NativeHost for CancelHost {
    fn call(
        &self,
        method: &str,
        _: Value,
        cancellation: &CancellationToken,
    ) -> Result<Value, ActionError> {
        assert_eq!(method, self.method);
        self.entered.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !cancellation.is_cancelled() && Instant::now() < deadline {
            thread::park_timeout(Duration::from_millis(2));
        }
        self.observed
            .store(cancellation.is_cancelled(), Ordering::Release);
        self.finished.store(true, Ordering::Release);
        Err(Expected::error(
            self.method,
            json!({}),
            "cancelled",
            if matches!(method, "history.list" | "history.get") {
                "not_started"
            } else {
                "unknown"
            },
        )
        .reply
        .unwrap_err())
    }
}
fn cancellation(engine: Engine) {
    for (method, args) in [
        ("history.list", ""),
        ("history.get", "{id:'1'}"),
        ("history.append", "{text:''}"),
        ("history.remove", "{id:'1'}"),
    ] {
        let expression = engine.call(&format!("ctx.clipboard.history.{}({args})", &method[8..]));
        let fixture = Fixture::new(
            engine,
            &engine.source(&format!("{expression};return {{finished:true}};")),
        );
        let action = fixture.action(json!({}));
        let cancellation = CancellationToken::new();
        let signal = cancellation.clone();
        let (entered, ready) = mpsc::sync_channel(1);
        let host = Arc::new(CancelHost {
            entered,
            finished: AtomicBool::new(false),
            observed: AtomicBool::new(false),
            method,
        });
        let worker_host = host.clone();
        let runtime = engine.runtime();
        let worker = thread::spawn(move || {
            runtime.execute_with_host(&action, &context(), &signal, Some(worker_host.as_ref()))
        });
        let began = ready.recv_timeout(Duration::from_secs(10));
        cancellation.cancel();
        let result = worker.join().unwrap(); // 失败也必须取消并收尾自己启动的运行时。
        assert!(began.is_ok(), "真实宿主未进入合成回调");
        assert_eq!(result.unwrap_err().code, "cancelled");
        assert!(host.finished.load(Ordering::Acquire));
        assert!(host.observed.load(Ordering::Acquire));
    }
    let host = Host::new(vec![]);
    let fixture = Fixture::new(engine, &engine.source("return {finished:true};"));
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let error = engine
        .runtime()
        .execute_with_host(
            &fixture.action(json!({})),
            &context(),
            &cancellation,
            Some(&host),
        )
        .unwrap_err();
    assert_eq!(error.code, "cancelled");
    host.finished(0);
}

struct DeadlineHost(AtomicBool);
impl NativeHost for DeadlineHost {
    fn call(&self, _: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
        panic!("历史调用应转发动作 deadline");
    }
    fn call_with_deadline(
        &self,
        method: &str,
        params: Value,
        _: &CancellationToken,
        deadline: Instant,
    ) -> Result<Value, ActionError> {
        assert_eq!(method, "history.list");
        assert_eq!(params, json!({"limit":100}));
        assert!(deadline > Instant::now());
        self.0.store(true, Ordering::Release);
        Ok(json!({"items":[],"revision":"0"}))
    }
}
fn deadline_forwarded(engine: Engine) {
    let host = DeadlineHost(AtomicBool::new(false));
    let list = engine.call("ctx.clipboard.history.list()");
    assert_eq!(
        run(
            engine,
            &format!("const result={list};return {{itemCount:result.items.length}};"),
            Some(&host)
        )
        .unwrap(),
        json!({"itemCount":0})
    );
    assert!(host.0.load(Ordering::Acquire));
}

fn actual_recipe_summary(engine: Engine) {
    let source = match engine {
        Engine::Node => include_str!("../../../examples/actions/clipboard-history/main.mjs"),
        Engine::Light => {
            include_str!("../../../examples/actions/clipboard-history/main-script.mjs")
        }
    };
    let mut fixture = Fixture::new(engine, source);
    let entry = match engine {
        Engine::Node => "main.mjs",
        Engine::Light => "main-script.mjs",
    };
    if fixture.entry != entry {
        fs::rename(fixture.root.join(fixture.entry), fixture.root.join(entry)).unwrap();
        fixture.entry = entry;
    }
    fs::write(
        fixture.root.join("workflow.mjs"),
        include_str!("../../../examples/actions/clipboard-history/workflow.mjs"),
    )
    .unwrap();
    fixture.workflow = true;
    let host = Host::new(vec![
        Expected::value(
            "history.list",
            json!({"limit":100}),
            json!({"items":[summary("1",PAYLOAD.encode_utf16().count())],"revision":"1"}),
        ),
        Expected::value(
            "ui.choose",
            json!({"title":"选择要发送的历史文本","items":[{"id":"1","label":"1. 合成摘要"}]}),
            json!({"status":"selected","id":"1"}),
        ),
        Expected::value(
            "history.get",
            json!({"id":"1"}),
            json!({"id":"1","text":PAYLOAD,"createdAt":"0","revision":"1"}),
        ),
        Expected::value(
            "keyboard.text",
            json!({"text":PAYLOAD}),
            json!({"submitted":true}),
        ),
    ]);
    let output = engine
        .runtime()
        .execute_with_host(
            &fixture.action(json!({"mode":"send"})),
            &context(),
            &CancellationToken::new(),
            Some(&host),
        )
        .unwrap();
    assert_eq!(
        output,
        json!({"operation":"send","status":"submitted","itemCount":1,"selectedUtf16Units":PAYLOAD.encode_utf16().count(),"inputSubmitted":true})
    );
    assert!(!output.to_string().contains(PAYLOAD));
    assert!(output.get("text").is_none());
    host.finished(4);
}

macro_rules! engines {
    ($node:ident,$light:ident,$check:ident) => {
        #[test]
        #[ignore = "显式串行运行真实Node；所有本机响应都是合成内存数据"]
        fn $node() {
            $check(Engine::Node);
        }
        #[test]
        #[ignore = "指定实际Boa/QuickJS宿主串行运行；不读系统剪贴板或发输入"]
        fn $light() {
            $check(Engine::Light);
        }
    };
}
engines!(node_history_roundtrip, light_history_roundtrip, roundtrip);
engines!(node_history_large, light_history_large, large_values);
engines!(
    node_history_invalid_input,
    light_history_invalid_input,
    invalid_inputs
);
engines!(
    node_history_malformed,
    light_history_malformed,
    malformed_results
);
engines!(
    node_history_failure_stages,
    light_history_failure_stages,
    failure_stages
);
engines!(
    node_history_unavailable,
    light_history_unavailable,
    unavailable
);
engines!(node_history_cancel, light_history_cancel, cancellation);
engines!(
    node_history_deadline,
    light_history_deadline,
    deadline_forwarded
);
engines!(
    node_history_recipe_summary,
    light_history_recipe_summary,
    actual_recipe_summary
);
