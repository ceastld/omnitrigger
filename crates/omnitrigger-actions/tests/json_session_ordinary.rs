//! 正式ActionService与ScriptHostRuntime逐轮执行JSON会话入口。
//! 每轮只传显式state/event。NativeHost是禁止调用的哨兵，无UI/Clipboard/后台会话。
#![cfg(windows)]
use omnitrigger_actions::{
    ActionCall, ActionError, ActionOutcome, ActionService, CancellationToken, NativeHost,
    PackageCatalog, RuntimeRegistry, ScriptHostRuntime,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const FILES: &[(&str, &[u8])] = &[
    (
        "action.json",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/actions/json-text/action.json"
        )),
    ),
    (
        "main.mjs",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/actions/json-text/main.mjs"
        )),
    ),
    (
        "main-script.mjs",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/actions/json-text/main-script.mjs"
        )),
    ),
    (
        "select.mjs",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/actions/json-text/select.mjs"
        )),
    ),
    (
        "select-script.mjs",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/actions/json-text/select-script.mjs"
        )),
    ),
    (
        "json.mjs",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/actions/json-text/json.mjs"
        )),
    ),
    (
        "workflow.mjs",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/actions/json-text/workflow.mjs"
        )),
    ),
    (
        "select-core.mjs",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/actions/json-text/select-core.mjs"
        )),
    ),
    (
        "json-types.d.ts",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/actions/json-text/json-types.d.ts"
        )),
    ),
    (
        "session-core.mjs",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/actions/json-text/session-core.mjs"
        )),
    ),
    (
        "session-core.d.mts",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/actions/json-text/session-core.d.mts"
        )),
    ),
    (
        "session-step-script.mjs",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/actions/json-text/session-step-script.mjs"
        )),
    ),
    (
        "session-types.d.ts",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/actions/json-text/session-types.d.ts"
        )),
    ),
];
static NEXT: AtomicU64 = AtomicU64::new(1);
fn local(path: &Path) {
    use std::os::windows::fs::MetadataExt;
    assert!(path.is_absolute());
    for p in path.ancestors() {
        let name = p
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase();
        assert!(!name.starts_with("onedrive") && name != "dropbox" && !name.starts_with("icloud"));
        let meta = fs::symlink_metadata(p).unwrap();
        assert!(!meta.file_type().is_symlink() && meta.file_attributes() & 0x441400 == 0);
    }
}
struct Fixture {
    root: PathBuf,
    actions: PathBuf,
    package: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let parent = PathBuf::from(
            std::env::var_os("OMNITRIGGER_JSON_SESSION_TEST_ROOT")
                .expect("须明确提供已存在的本地自有测试父目录，不默认个人目录"),
        );
        local(&parent);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = parent.join(format!(
            "json-session-{}-{nonce}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let actions = root.join("actions");
        fs::create_dir(&actions).unwrap();
        let package = actions.join("json-text");
        fs::create_dir(&package).unwrap();
        for (name, bytes) in FILES {
            fs::write(package.join(name), bytes).unwrap();
        }
        Self {
            root,
            actions,
            package,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if thread::panicking() {
            eprintln!("保留失败夹具：{}", self.root.display());
            return;
        }
        for (name, _) in FILES {
            fs::remove_file(self.package.join(name)).unwrap();
        }
        fs::remove_dir(&self.package).unwrap();
        fs::remove_dir(&self.actions).unwrap();
        fs::remove_dir(&self.root).unwrap();
    }
}
#[derive(Default)]
struct NoNative {
    calls: AtomicUsize,
}
impl NativeHost for NoNative {
    fn native_context_available(&self) -> bool {
        false
    }
    fn call(
        &self,
        method: &str,
        _params: Value,
        _token: &CancellationToken,
    ) -> Result<Value, ActionError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut e = ActionError::new(
            "unexpected_native_call",
            "纯会话普通入口不允许任何Native/UI/Clipboard调用。",
        );
        e.method = Some(method.into());
        e.effect_state = Some("not_started".into());
        Err(e)
    }
    // 显式保留外层调用期限。
    fn call_with_deadline(
        &self,
        method: &str,
        params: Value,
        token: &CancellationToken,
        _deadline: Instant,
    ) -> Result<Value, ActionError> {
        self.call(method, params, token)
    }
}
struct Harness {
    service: ActionService,
    host: Arc<NoNative>,
    _fixture: Fixture,
}
impl Harness {
    fn new() -> Self {
        let fixture = Fixture::new();
        let program = PathBuf::from(
            std::env::var_os("OMNITRIGGER_TEST_SCRIPT_HOST")
                .expect("显式选择真实QuickJS/Boa宿主，不静默跳过"),
        );
        local(&program);
        assert!(program.is_file() && !program.starts_with(&fixture.root));
        let mut runtimes = RuntimeRegistry::new();
        runtimes
            .register("script", Arc::new(ScriptHostRuntime::new(program)))
            .unwrap();
        // 沿普通默认60秒预算，不提供伪运行时或改产品预算。
        let service = ActionService::new(PackageCatalog::load(&fixture.actions), runtimes);
        Self {
            service,
            host: Arc::new(NoNative::default()),
            _fixture: fixture,
        }
    }
    fn call(&self, input: Value) -> Result<Value, ActionError> {
        let id = self.service.submit_with_host(
            17,
            ActionCall {
                package_id: "examples.json-text".into(),
                action_id: "session-step-script".into(),
                action_version: 1,
                input,
            },
            Some(self.host.clone()),
        )?;
        let until = Instant::now() + Duration::from_secs(64);
        let mut started = 0;
        let result = loop {
            assert!(Instant::now() < until, "普通会话调用未在原预算内终止");
            match self.service.try_recv() {
                Ok(event) => {
                    assert_eq!(event.invocation_id, id);
                    match event.outcome {
                        ActionOutcome::Started => started += 1,
                        ActionOutcome::Succeeded { output } => break Ok(output),
                        ActionOutcome::Failed { error } => break Err(error),
                        ActionOutcome::Cancelled => panic!("未请求取消，纯入口不可伪造Cancelled"),
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    thread::sleep(Duration::from_millis(2))
                }
                Err(error) => panic!("Service事件通道失败：{error}"),
            }
        };
        assert_eq!(started, 1);
        assert_eq!(self.host.calls.load(Ordering::SeqCst), 0);
        if let Ok(value) = &result {
            assert_eq!(value["schema_version"], 1);
            assert_eq!(value["effectsExecuted"], false);
        }
        result
    }
    fn create(&self, text: &str) -> Value {
        self.call(json!({"mode":"create","initial":{"sessionId":"service_demo","text":text,"continuous":true}})).unwrap()
    }
    fn step(&self, state: &Value, kind: &str, extra: Value) -> Value {
        let mut event = json!({"type":kind,"revision":state["revision"]});
        for (k, v) in extra.as_object().unwrap() {
            event[k] = v.clone();
        }
        self.call(json!({"mode":"step","state":state,"event":event}))
            .unwrap()
    }
    fn bad(&self, input: Value, code: &str) {
        let e = self.call(input).unwrap_err();
        assert_eq!(e.code, code);
        assert_eq!(e.method.as_deref(), Some("script.parameters"));
        assert_eq!(e.effect_state.as_deref(), Some("not_started"));
        assert!(!e.message.contains("SECRET"));
    }
}

#[test]
#[ignore = "实际普通Script Service，NoNative哨兵必须零调用"]
fn json_session_create_edit_output_ack_each_use_one_real_service_call() {
    let h = Harness::new();
    let created = h.create("{\"n\":9007199254740993,\"a\":[false,null]}");
    let mut s = created["state"].clone();
    assert_eq!(created["status"], "created");
    s = h.step(&s, "select", json!({"path":["n"]}))["state"].clone();
    s = h.step(&s, "begin_edit", json!({}))["state"].clone();
    s = h.step(
        &s,
        "commit_edit",
        json!({"replacementText":"9223372036854775807"}),
    )["state"]
        .clone();
    assert_eq!(s["text"], "{\"n\":9223372036854775807,\"a\":[false,null]}");
    assert_eq!(s["revision"], 1);
    let requested = h.step(
        &s,
        "request_output",
        json!({"kind":"document","format":"compact"}),
    );
    assert_eq!(requested["requests"][0]["text"], s["text"]);
    assert_eq!(requested["state"]["mode"], "pending");
    let done = h
        .call(
            json!({"mode":"step","state":requested["state"],"event":{"type":"ack_output",
        "requestId":requested["state"]["pending"]["id"],"status":"confirmed"}}),
        )
        .unwrap();
    assert_eq!(done["state"]["mode"], "closed");
    assert_eq!(done["requests"], json!([]));
}

#[test]
#[ignore = "真实Service对序列化状态的导航/取消/修订拒绝"]
fn json_session_navigation_delete_cancel_and_stale_events_keep_lossless_state() {
    let h = Harness::new();
    let original = "{\"rows\":[9007199254740993,null,false],\"label\":\"own\"}";
    let mut s = h.create(original)["state"].clone();
    s = h.step(&s, "select", json!({"path":["label"]}))["state"].clone();
    s = h.step(&s, "begin_edit", json!({}))["state"].clone();
    s = h.step(&s, "cancel_edit", json!({}))["state"].clone();
    assert_eq!(s["text"], original);
    s = h.step(&s, "select", json!({"path":["rows"]}))["state"].clone();
    s = h.step(&s, "open_selected", json!({}))["state"].clone();
    s = h.step(&s, "select", json!({"path":["rows",1]}))["state"].clone();
    s = h.step(&s, "delete_selected", json!({}))["state"].clone();
    assert_eq!(
        s["text"],
        "{\"rows\":[9007199254740993,false],\"label\":\"own\"}"
    );
    h.bad(
        json!({"mode":"step","state":s,"event":{"type":"home","revision":0}}),
        "stale_revision",
    );
    let cancelled = h
        .call(json!({"mode":"step","state":s,"event":{"type":"cancel"}}))
        .unwrap();
    assert_eq!(cancelled["state"]["mode"], "closed");
}

#[test]
#[ignore = "分页/真实96KiB响应界限前的结构化错误"]
fn json_session_paging_and_budget_failures_never_truncate_or_call_native() {
    let h = Harness::new();
    let doc = format!(
        "[{}]",
        (0..70).map(|n| n.to_string()).collect::<Vec<_>>().join(",")
    );
    let result=h.call(json!({"mode":"create","initial":{"sessionId":"pages","text":doc,"continuous":true},"page":{"offset":64,"limit":6}})).unwrap();
    assert_eq!(result["view"]["totalRows"], 70);
    assert_eq!(result["view"]["rows"].as_array().unwrap().len(), 6);
    assert_eq!(result["state"]["text"], doc);
    let doc = serde_json::to_string(&json!({"text":"界".repeat(12000)})).unwrap();
    let mut s = h.create(&doc)["state"].clone();
    s = h.step(&s, "select", json!({"path":["text"]}))["state"].clone();
    h.bad(json!({"mode":"step","state":s,"event":{"type":"request_output","revision":0,"kind":"value","format":"raw-string"}}),"session_output_too_large");
    assert_eq!(s["pending"], Value::Null);
    assert_eq!(s["text"], doc);
    h.bad(json!({"mode":"create","initial":{"sessionId":"too_large","text":"x".repeat(61 * 1024),"continuous":true}}),"session_input_too_large");
    let error = h.call(json!({"mode":"create","initial":{"sessionId":"service_too_large","text":"x".repeat(110000),"continuous":true}})).unwrap_err();
    assert_eq!(error.code, "input_too_large");
    assert_eq!(error.method, None);
    assert!(!h.service.is_running(17));
    assert!(matches!(
        h.service.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    assert_eq!(h.host.calls.load(Ordering::SeqCst), 0);
    let doc =
        serde_json::to_string(&json!({"payload":"a".repeat(12000),"padding":"b".repeat(38000)}))
            .unwrap();
    let state = h.create(&doc)["state"].clone();
    let state = h.step(&state, "select", json!({"path":["payload"]}))["state"].clone();
    h.bad(json!({"mode":"step","state":state,"event":{"type":"request_output","revision":0,"kind":"value","format":"raw-string"}}), "session_state_too_large");
    h.bad(json!({"mode":"create","initial":{"sessionId":"bad_json","text":"{\"SECRET\":","continuous":true}}),"invalid_json");
    h.bad(json!({"mode":"create","initial":{"sessionId":"dup","text":"{\"x\":1,\"x\":2}","continuous":true}}),"json_duplicate_key");
}

#[test]
#[ignore = "输出请求只是参数流，unknown/取消不冒称写入"]
fn json_session_unknown_and_late_ack_do_not_resume_or_repeat_output() {
    let h = Harness::new();
    let mut s = h.create("{\"v\":\"synthetic\"}")["state"].clone();
    s = h.step(&s, "select", json!({"path":["v"]}))["state"].clone();
    let p = h.step(
        &s,
        "request_output",
        json!({"kind":"value","format":"raw-string"}),
    );
    let cancelled = h
        .call(json!({"mode":"step","state":p["state"],"event":{"type":"cancel"}}))
        .unwrap();
    let late=h.call(json!({"mode":"step","state":cancelled["state"],"event":{"type":"ack_output","requestId":p["state"]["pending"]["id"],"status":"confirmed"}})).unwrap();
    assert_eq!(late["state"]["mode"], "closed");
    assert_eq!(late["state"]["endReason"], "cancelled");
    assert_eq!(late["requests"], json!([]));
    let unknown=h.call(json!({"mode":"step","state":p["state"],"event":{"type":"ack_output","requestId":p["state"]["pending"]["id"],"status":"unknown"}})).unwrap();
    assert_eq!(unknown["state"]["endReason"], "output_unknown");
    assert_eq!(unknown["requests"], json!([]));
}
