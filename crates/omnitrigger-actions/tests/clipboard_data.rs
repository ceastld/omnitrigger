//! 真正 Node/Boa/QuickJS 运行时与 NativeHost 的数据契约集成。
//! NativeHost 只收发内存中的合成 JSON；不链接/调用 Win32 剪贴板或输入实现。
//! 真实进程用例显式 ignored，由 CI/串行验证指定运行时后执行。
use omnitrigger_actions::{
    ActionCall, ActionDefinition, ActionError, ActionRuntime, CancellationToken, InvocationContext,
    NativeHost, NodeRuntime, PackageCatalog, ResolvedAction, ScriptHostRuntime,
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

const READ: &str = "clipboard.read_data";
const COPY: &str = "selection.copy_data";
const PAYLOAD: &str = "synthetic_private_payload_合成😀";
const EXAMPLE: &str = include_str!("../../../examples/actions/selection-data/main.js");
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
        for name in [
            ".tmp",
            "parallel-selection-data",
            "integration",
            "test-cases",
        ] {
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
        Self { base, root, engine }
    }
    fn action(&self, input: Value) -> ResolvedAction {
        ResolvedAction {
            package_root: self.root.clone(),
            entry_path: self.root.join(self.engine.entry()),
            call: ActionCall {
                package_id: "clipboard-data-fixture".into(),
                action_id: "run".into(),
                action_version: 1,
                input,
            },
            definition: ActionDefinition {
                id: "run".into(),
                title: "合成数据契约测试".into(),
                version: 1,
                runtime: self.engine.id().into(),
                entry: self.engine.entry().into(),
                single_instance: true,
                allow_binding_reentry: false,
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
        let entry = root.join(self.engine.entry());
        assert!(local_metadata(&entry).is_file());
        fs::remove_file(entry).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
fn context() -> InvocationContext {
    InvocationContext {
        invocation_id: u64::MAX,
        binding_id: 9_007_199_254_740_993,
    }
}
fn defaults() -> Value {
    json!({"timeout_ms":600,"poll_interval_ms":15,"settle_ms":30,"allow_unchanged":false})
}
fn options() -> Value {
    json!({"timeout_ms":400,"poll_interval_ms":10,"settle_ms":0,"allow_unchanged":true})
}
fn error(code: &str, method: &str, effect: &str) -> ActionError {
    let mut failure = ActionError::new(code, format!("{PAYLOAD} X:\\SyntheticOnly\\file.txt"));
    failure.method = Some(method.into());
    failure.effect_state = Some(effect.into());
    failure
}

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
}
struct Host {
    expected: Mutex<VecDeque<Expected>>,
    calls: Mutex<Vec<(String, Value)>>,
}
impl Host {
    fn new(expected: Vec<Expected>) -> Self {
        Self {
            expected: Mutex::new(expected.into()),
            calls: Mutex::new(Vec::new()),
        }
    }
    fn finished(&self, count: usize) {
        assert!(self.expected.lock().unwrap().is_empty());
        assert_eq!(self.calls.lock().unwrap().len(), count);
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
            .expect("不得多发、回退或重试本机方法");
        assert_eq!(
            method, expected.method,
            "不能转成真实系统、旧文本、目标查询或其它方法"
        );
        assert_eq!(params, expected.params);
        self.calls.lock().unwrap().push((method.into(), params));
        expected.reply
    }
}
fn execute(
    engine: Engine,
    body: &str,
    host: Option<&dyn NativeHost>,
) -> Result<Value, ActionError> {
    let fixture = Fixture::new(engine, &engine.source(body));
    engine.runtime().execute_with_host(
        &fixture.action(json!({})),
        &context(),
        &CancellationToken::new(),
        host,
    )
}
fn assert_fault(error: &ActionError, code: &str, method: &str, effect: &str) {
    assert_eq!(error.code, code);
    assert_eq!(error.method.as_deref(), Some(method));
    assert_eq!(error.effect_state.as_deref(), Some(effect));
    assert!(!error.message.contains(PAYLOAD));
    assert!(!error.message.contains("SyntheticOnly"));
}

fn roundtrip(engine: Engine) {
    let text = json!({"kind":"text","text":PAYLOAD,"sequenceNumber":u32::MAX});
    let empty = json!({"kind":"text","text":"","sequenceNumber":0});
    let files = json!({"kind":"files","paths":["relative synthetic.txt", "\\\\synthetic.invalid\\share\\中文😀.txt", "C:\\OneDrive-synthetic\\file.txt"],"sequenceNumber":17});
    let empty_files = json!({"kind":"files","paths":[],"sequenceNumber":18});
    let copied = json!({"kind":"text","text":PAYLOAD,"sequenceNumber":0,"changed":true});
    let unchanged = json!({"kind":"files","paths":[],"sequenceNumber":u32::MAX,"changed":false});
    let host = Host::new(vec![
        Expected::value(READ, json!({}), text.clone()),
        Expected::value(READ, json!({}), empty.clone()),
        Expected::value(READ, json!({}), files.clone()),
        Expected::value(READ, json!({}), empty_files.clone()),
        Expected::value(COPY, defaults(), copied.clone()),
        Expected::value(COPY, options(), unchanged.clone()),
    ]);
    let read = engine.call("ctx.clipboard.readData()");
    let copy = engine.call("ctx.desktop.copySelectionData()");
    let explicit=engine.call("ctx.desktop.copySelectionData({timeoutMs:400,pollIntervalMs:10,settleMs:0,allowUnchanged:true})");
    let body=format!("const text={read}; const empty={read}; const files={read}; const emptyFiles={read}; const copied={copy}; const unchanged={explicit}; return {{text,empty,files,emptyFiles,copied,unchanged}};");
    let output = execute(engine, &body, Some(&host)).unwrap();
    assert_eq!(
        output,
        json!({"text":text,"empty":empty,"files":files,"emptyFiles":empty_files,"copied":copied,"unchanged":unchanged})
    );
    host.finished(6);
}

fn large_values(engine: Engine) {
    let host = Host::new(vec![
        Expected::value(
            READ,
            json!({}),
            json!({"kind":"text","text":"😀".repeat(32768),"sequenceNumber":u32::MAX}),
        ),
        Expected::value(
            COPY,
            options(),
            json!({"kind":"files","paths":vec!["x".repeat(1024);64],"sequenceNumber":0,"changed":false}),
        ),
    ]);
    let read = engine.call("ctx.clipboard.readData()");
    let copy=engine.call("ctx.desktop.copySelectionData({timeoutMs:400,pollIntervalMs:10,settleMs:0,allowUnchanged:true})");
    let body=format!("const text={read}; const files={copy}; return {{kind:text.kind,units:text.text.length,sequence:text.sequenceNumber,items:files.paths.length,fileUnits:files.paths.reduce((n,path)=>n+path.length,0),changed:files.changed}};");
    assert_eq!(
        execute(engine, &body, Some(&host)).unwrap(),
        json!({"kind":"text","units":65536,"sequence":u32::MAX,"items":64,"fileUnits":65536,"changed":false})
    );
    host.finished(2);
}

fn invalid_options(engine: Engine) {
    let expression = engine.call("ctx.desktop.copySelectionData(options)");
    let read = engine.call("ctx.clipboard.readData({extra:'synthetic'})");
    let body = format!(
        r#"
const rejected=[];
for (const options of [null,[],true,{{timeout_ms:400}},{{timeoutMs:0}},{{timeoutMs:5001}},{{timeoutMs:10,pollIntervalMs:11}},{{pollIntervalMs:0}},{{settleMs:1001}},{{allowUnchanged:1}}]) {{
  try {{ {expression}; throw new Error('应拒绝'); }} catch(e) {{ if(e.code!=='invalid_input'||e.method!=='selection.copy_data'||e.effectState!=='not_started') throw e; rejected.push(e.code); }}
}}
try {{ {read}; throw new Error('应拒绝'); }} catch(e) {{ if(e.code!=='invalid_input'||e.method!=='clipboard.read_data'||e.effectState!=='not_started') throw e; }}
return {{rejected:rejected.length}};
"#
    );
    let host = Host::new(vec![]);
    assert_eq!(
        execute(engine, &body, Some(&host)).unwrap(),
        json!({"rejected":10})
    );
    host.finished(0);
}

fn malformed_results(engine: Engine) {
    for method in [READ, COPY] {
        let copy = method == COPY;
        let mut valid = json!({"kind":"text","text":PAYLOAD,"sequenceNumber":1});
        if copy {
            valid["changed"] = json!(true);
        }
        let mut extra = valid.clone();
        extra["private"] = json!(PAYLOAD);
        let mut bad_sequence = valid.clone();
        bad_sequence["sequenceNumber"] = json!(4294967296_u64);
        let mut nul = valid.clone();
        nul["text"] = json!("x\0y");
        let mut wrong_data = valid.clone();
        wrong_data["text"] = json!(null);
        for value in [json!(null), extra, bad_sequence, nul, wrong_data] {
            let host = Host::new(vec![Expected::value(
                method,
                if copy { defaults() } else { json!({}) },
                value,
            )]);
            let expression = engine.call(if copy {
                "ctx.desktop.copySelectionData()"
            } else {
                "ctx.clipboard.readData()"
            });
            let failure =
                execute(engine, &format!("return {expression};"), Some(&host)).unwrap_err();
            assert_fault(
                &failure,
                "invalid_response",
                method,
                if copy { "unknown" } else { "not_started" },
            );
            host.finished(1);
        }
    }
}

fn platform_errors(engine: Engine) {
    for (method, code, effect) in [
        (READ, "unsupported_clipboard_format", "not_started"),
        (READ, "cancelled", "not_started"),
        (COPY, "selection_unchanged", "unknown"),
        (COPY, "target_changed", "not_started"),
        (COPY, "cancelled", "unknown"),
        (COPY, "timeout", "unknown"),
    ] {
        let host = Host::new(vec![Expected {
            method,
            params: if method == COPY {
                defaults()
            } else {
                json!({})
            },
            reply: Err(error(code, method, effect)),
        }]);
        let expression = engine.call(if method == COPY {
            "ctx.desktop.copySelectionData()"
        } else {
            "ctx.clipboard.readData()"
        });
        let failure = execute(engine, &format!("return {expression};"), Some(&host)).unwrap_err();
        assert_fault(&failure, code, method, effect);
        host.finished(1);
    }
}

fn no_bridge_and_old_methods(engine: Engine) {
    let read = engine.call("ctx.clipboard.readData()");
    let copy = engine.call("ctx.desktop.copySelectionData()");
    let body = format!(
        r#"
const failures=[];
try {{ {read}; throw new Error('缺桥接应失败'); }} catch(e) {{ failures.push({{code:e.code,method:e.method,effect:e.effectState}}); }}
try {{ {copy}; throw new Error('缺桥接应失败'); }} catch(e) {{ failures.push({{code:e.code,method:e.method,effect:e.effectState}}); }}
return {{readType:typeof ctx.clipboard.readData,copyType:typeof ctx.desktop.copySelectionData,failures}};
"#
    );
    assert_eq!(
        execute(engine, &body, None).unwrap(),
        json!({"readType":"function","copyType":"function","failures":[
            {"code":"native_bridge_unavailable","method":READ,"effect":"not_started"},
            {"code":"native_bridge_unavailable","method":COPY,"effect":"not_started"},
        ]})
    );
    // 旧文本API在有桥接时也只由内存Host接收，不调用平台剪贴板或输入。
    let host = Host::new(vec![
        Expected::value("clipboard.read", json!({}), json!("合成旧文本")),
        Expected::value("selection.copy", json!({}), json!("合成旧选区")),
    ]);
    let old_read = engine.call("ctx.clipboard.readText()");
    let old_copy = engine.call("ctx.desktop.copySelectionText()");
    assert_eq!(
        execute(
            engine,
            &format!("const text={old_read}; const selected={old_copy}; return {{text,selected}};"),
            Some(&host)
        )
        .unwrap(),
        json!({"text":"合成旧文本","selected":"合成旧选区"})
    );
    host.finished(2);
}

struct CancelHost {
    entered: SyncSender<()>,
    finished: AtomicBool,
    observed_cancel: AtomicBool,
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
        self.observed_cancel
            .store(cancellation.is_cancelled(), Ordering::Release);
        self.finished.store(true, Ordering::Release);
        Err(error(
            "cancelled",
            method,
            if method == READ {
                "not_started"
            } else {
                "unknown"
            },
        ))
    }
}
fn cancellation(engine: Engine) {
    for method in [READ, COPY] {
        let expression = engine.call(if method == COPY {
            "ctx.desktop.copySelectionData()"
        } else {
            "ctx.clipboard.readData()"
        });
        let fixture = Fixture::new(engine, &engine.source(&format!("return {expression};")));
        let action = fixture.action(json!({}));
        let cancellation = CancellationToken::new();
        let signal = cancellation.clone();
        let (entered, ready) = mpsc::sync_channel(1);
        let host = Arc::new(CancelHost {
            entered,
            finished: AtomicBool::new(false),
            observed_cancel: AtomicBool::new(false),
            method,
        });
        let worker_host = Arc::clone(&host);
        let runtime = engine.runtime();
        let worker = thread::spawn(move || {
            runtime.execute_with_host(&action, &context(), &signal, Some(worker_host.as_ref()))
        });
        let began = ready.recv_timeout(Duration::from_secs(10));
        // 即使前序失败也先取消并join；不能留下测试启动的运行时进程。
        cancellation.cancel();
        let result = worker.join().unwrap();
        assert!(began.is_ok(), "真实宿主未进入可控NativeHost：{result:?}");
        assert_eq!(result.unwrap_err().code, "cancelled");
        assert!(host.finished.load(Ordering::Acquire));
        assert!(host.observed_cancel.load(Ordering::Acquire));
    }
}

struct SlowCopy(AtomicU64);
impl NativeHost for SlowCopy {
    fn call(
        &self,
        method: &str,
        params: Value,
        _: &CancellationToken,
    ) -> Result<Value, ActionError> {
        assert_eq!(method, COPY);
        assert_eq!(params, defaults());
        assert_eq!(
            self.0.fetch_add(1, Ordering::SeqCst),
            0,
            "不能重复提交复制请求"
        );
        // 有意超过旧5秒传输预算，用真实计时核验新copy_data的8秒预算；不模拟输入或系统读取。
        let deadline = Instant::now() + Duration::from_millis(5200);
        while Instant::now() < deadline {
            thread::park_timeout(
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(20)),
            );
        }
        Ok(json!({"kind":"text","text":"合成延迟回执","sequenceNumber":1,"changed":true}))
    }
}
fn copy_budget(engine: Engine) {
    let expression = engine.call("ctx.desktop.copySelectionData()");
    let host = SlowCopy(AtomicU64::new(0));
    assert_eq!(
        execute(
            engine,
            &format!("const data={expression}; return {{kind:data.kind,changed:data.changed}};"),
            Some(&host)
        )
        .unwrap(),
        json!({"kind":"text","changed":true})
    );
    assert_eq!(host.0.load(Ordering::SeqCst), 1);
}

macro_rules! real_engines {
    ($node:ident, $light:ident, $check:ident) => {
        #[test]
        #[ignore = "根对话串行运行真实Node；全部本机回执由合成NativeHost提供"]
        fn $node() {
            $check(Engine::Node);
        }
        #[test]
        #[ignore = "根对话设置OMNITRIGGER_TEST_SCRIPT_HOST后分别以Boa/QuickJS串行执行"]
        fn $light() {
            $check(Engine::Light);
        }
    };
}
real_engines!(node_roundtrip, light_roundtrip, roundtrip);
real_engines!(node_large_values, light_large_values, large_values);
real_engines!(node_invalid_options, light_invalid_options, invalid_options);
real_engines!(
    node_malformed_results,
    light_malformed_results,
    malformed_results
);
real_engines!(node_platform_errors, light_platform_errors, platform_errors);
real_engines!(
    node_no_bridge_and_old_methods,
    light_no_bridge_and_old_methods,
    no_bridge_and_old_methods
);
real_engines!(node_cancellation, light_cancellation, cancellation);
real_engines!(node_copy_budget, light_copy_budget, copy_budget);

#[test]
fn selection_data_example_resolves_and_declares_action_single_instance() {
    let examples = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/actions");
    assert!(local_metadata(&examples).is_dir());
    let call: ActionCall = serde_json::from_str(include_str!(
        "../../../examples/actions/selection-data/call.json"
    ))
    .unwrap();
    let action = PackageCatalog::load(&examples).resolve(&call).unwrap();
    assert_eq!(action.definition.runtime, "script");
    assert!(action.definition.single_instance);
    assert_eq!(action.definition.entry, "main.js");
    assert_eq!(call.input, json!({"mode":"read"}));
}

#[test]
#[ignore = "根对话以真实Boa/QuickJS运行实际示例；只读合成NativeHost，不读系统剪贴板"]
fn light_actual_example_outputs_summary_without_payload() {
    for (input, value, expected, method, params) in [
        (
            json!({"mode":"read"}),
            json!({"kind":"text","text":"合成😀","sequenceNumber":1}),
            json!({"kind":"text","utf16Units":4,"itemCount":1}),
            READ,
            json!({}),
        ),
        (
            json!({"mode":"read"}),
            json!({"kind":"files","paths":[],"sequenceNumber":0}),
            json!({"kind":"files","utf16Units":0,"itemCount":0}),
            READ,
            json!({}),
        ),
        (
            json!({"mode":"copy"}),
            json!({"kind":"text","text":"","sequenceNumber":2,"changed":true}),
            json!({"kind":"text","utf16Units":0,"itemCount":1,"changed":true}),
            COPY,
            defaults(),
        ),
        (
            json!({"mode":"copy","options":{"allowUnchanged":true}}),
            json!({"kind":"files","paths":["X:\\a","合成😀"],"sequenceNumber":0,"changed":false}),
            json!({"kind":"files","utf16Units":8,"itemCount":2,"changed":false}),
            COPY,
            json!({"timeout_ms":600,"poll_interval_ms":15,"settle_ms":30,"allow_unchanged":true}),
        ),
    ] {
        let fixture = Fixture::new(Engine::Light, EXAMPLE);
        let host = Host::new(vec![Expected::value(method, params, value)]);
        let output = Engine::Light
            .runtime()
            .execute_with_host(
                &fixture.action(input),
                &context(),
                &CancellationToken::new(),
                Some(&host),
            )
            .unwrap();
        assert_eq!(output, expected);
        assert!(
            output.get("text").is_none()
                && output.get("paths").is_none()
                && output.get("sequenceNumber").is_none()
        );
        host.finished(1);
    }
    for input in [
        json!({}),
        json!({"mode":"text"}),
        json!({"mode":"read","options":{}}),
        json!({"mode":"read","extra":true}),
    ] {
        let fixture = Fixture::new(Engine::Light, EXAMPLE);
        let host = Host::new(vec![]);
        let failure = Engine::Light
            .runtime()
            .execute_with_host(
                &fixture.action(input),
                &context(),
                &CancellationToken::new(),
                Some(&host),
            )
            .unwrap_err();
        assert_fault(
            &failure,
            "invalid_input",
            "script.parameters",
            "not_started",
        );
        host.finished(0);
    }
}
