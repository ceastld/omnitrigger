//! 真实NodeRuntime/ScriptHostRuntime与合成NativeHost的集成测试源码。
//! 仅在显式指定新宿主后运行ignored；不创建窗口、请求前台或调用Win32。
use omnitrigger_actions::{
    ActionCall, ActionDefinition, ActionError, ActionRuntime, CancellationToken, InvocationContext,
    NativeHost, NodeRuntime, ResolvedAction, ScriptHostRuntime,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};

static NEXT: AtomicU64 = AtomicU64::new(1);
const WINDOW_METHODS: [&str; 4] = [
    "window.geometry",
    "window.set_rect",
    "window.action",
    "window.drag",
];
const SDK_ERROR: &str = "显式窗口租约调用未获确认；不会自动重试。";

#[derive(Clone, Copy)]
enum RuntimeKind {
    Node,
    Script,
}
impl RuntimeKind {
    fn name(self) -> &'static str {
        match self {
            Self::Node => "node",
            Self::Script => "script",
        }
    }
    fn source(self) -> &'static str {
        match self {
            Self::Node => include_str!("fixtures/captured-window-runtimes/node.mjs"),
            Self::Script => include_str!("fixtures/captured-window-runtimes/script.mjs"),
        }
    }
    fn execute(self, action: &ResolvedAction, host: &Host) -> Result<Value, ActionError> {
        let context = InvocationContext {
            invocation_id: 9_007_199_254_740_993,
            binding_id: u64::MAX,
        };
        let token = CancellationToken::new();
        match self {
            Self::Node => NodeRuntime::new(program("OMNITRIGGER_CAPTURED_WINDOW_TEST_NODE"))
                .execute_with_host(action, &context, &token, Some(host)),
            Self::Script => ScriptHostRuntime::new(program("OMNITRIGGER_TEST_SCRIPT_HOST"))
                .execute_with_host(action, &context, &token, Some(host)),
        }
    }
}
fn local(path: &Path) {
    assert!(path.is_absolute(), "测试路径必须为明确绝对本地路径");
    for part in path.ancestors() {
        assert!(
            !part.file_name().is_some_and(|name| {
                let name = name.to_string_lossy().to_ascii_lowercase();
                name.starts_with("onedrive") || name == "dropbox" || name.starts_with("icloud")
            }),
            "拒绝云盘路径"
        );
        let metadata = fs::symlink_metadata(part).unwrap();
        assert!(!metadata.file_type().is_symlink(), "拒绝链接");
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            assert_eq!(
                metadata.file_attributes() & 0x441400,
                0,
                "拒绝重解析点和云占位"
            );
        }
    }
}
fn program(key: &str) -> PathBuf {
    let path = PathBuf::from(
        std::env::var_os(key).unwrap_or_else(|| panic!("必须显式设置{key}，不静默跳过")),
    );
    local(&path);
    assert!(path.is_file());
    path
}
struct Fixture {
    root: PathBuf,
    action: ResolvedAction,
}
impl Fixture {
    fn new(kind: RuntimeKind) -> Self {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        local(&repo);
        let parent = repo.join(".tmp");
        if !parent.exists() {
            fs::create_dir(&parent).unwrap();
        }
        local(&parent);
        let root = parent.join(format!(
            "captured-window-runtime-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        local(&root);
        let entry_path = root.join("main.mjs");
        fs::write(&entry_path, kind.source()).unwrap();
        let action = ResolvedAction {
            package_root: root.clone(),
            entry_path,
            call: ActionCall {
                package_id: "test.captured-window-runtime".into(),
                action_id: "check".into(),
                action_version: 1,
                input: json!({}),
            },
            definition: ActionDefinition {
                id: "check".into(),
                title: "显式窗口租约合成桥接测试".into(),
                version: 1,
                runtime: kind.name().into(),
                entry: "main.mjs".into(),
                single_instance: false,
            },
        };
        Self { root, action }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.root.join("main.mjs"));
        let _ = fs::remove_dir(&self.root);
    }
}
fn a() -> Value {
    json!({"window_handle":"101","process_id":10,"thread_id":11,"process_created_file_time":"134000000000000001","expires_at_unix_ms":9999999999999u64})
}
fn b_identity() -> Value {
    json!({"window_handle":"202","process_id":20,"thread_id":21,"process_created_file_time":"18446744073709551615"})
}
fn b_token() -> Value {
    json!({"capture_id":7,"context":{"window_handle":"202","process_id":20,"thread_id":21,
        "process_created_file_time":"18446744073709551615","expires_at_unix_ms":9999999999998u64}})
}
fn geometry() -> Value {
    json!({"coordinateSpace":"physical-screen","outerRect":{"left":-808,"top":92,"width":816,"height":638},
        "visibleFrameRect":{"left":-800,"top":100,"width":800,"height":600},
        "monitorRect":{"left":-1920,"top":0,"width":1920,"height":1080},
        "workArea":{"left":-1920,"top":0,"width":1920,"height":1040},"primaryScreen":{"width":2560,"height":1440},"windowDpi":144,"maximized":false})
}
fn drag() -> Value {
    json!({"stopped":true,"reason":"update_limit","acceptedUpdates":1,"pollSamples":2,"effectState":"unknown","dragCompleted":false})
}
#[derive(Clone, Copy)]
enum Scenario {
    Success,
    ToggleOnly,
    Original,
    NoNativeContext,
    NativeError(&'static str),
    BadReply(&'static str),
    HandoffRejected,
    HandoffTimeout,
}
struct Host {
    calls: Mutex<Vec<(String, Value)>>,
    scenario: Scenario,
}
impl Host {
    fn new(scenario: Scenario) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            scenario,
        }
    }
    fn methods(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|(method, _)| method.clone())
            .collect()
    }
}
impl NativeHost for Host {
    fn native_context_available(&self) -> bool {
        !matches!(self.scenario, Scenario::NoNativeContext)
    }
    fn call(
        &self,
        method: &str,
        params: Value,
        _: &CancellationToken,
    ) -> Result<Value, ActionError> {
        self.calls
            .lock()
            .unwrap()
            .push((method.into(), params.clone()));
        if WINDOW_METHODS.contains(&method) && !matches!(self.scenario, Scenario::Original) {
            // 任一默认A回退、重捕获、token换代或上下文精度丢失都会失败。
            assert_eq!(params.get("target"), Some(&b_token()));
        }
        if let Scenario::NativeError(failed) = self.scenario {
            if method == failed {
                let mut error = ActionError::new("target_expired", "合成NativeHost拒绝");
                error.method = Some(method.into());
                error.effect_state = Some(
                    if method == "window.geometry" {
                        "not_started"
                    } else {
                        "unknown"
                    }
                    .into(),
                );
                return Err(error);
            }
        }
        if let Scenario::BadReply(failed) = self.scenario {
            if method == failed {
                return Ok(json!({"submitted":false}));
            }
        }
        match method {
            "target.context" => {
                assert_eq!(params, json!({}));
                Ok(a())
            }
            "pointer.root" => {
                assert_eq!(params, json!({}));
                Ok(
                    json!({"sampledAt":"call-time","coordinateSpace":"physical-screen","point":{"x":-700,"y":200},
                    "inputSource":{"kind":"polled-pointer","injected":null,"selfInjected":null},
                    "pointingWindow":b_identity(),"rootWindow":b_identity()}),
                )
            }
            "target.handoff_pointing_root" => {
                assert_eq!(
                    params,
                    json!({"expected_root":b_identity(),"valid_for_ms":5000,"confirm_timeout_ms":50})
                );
                match self.scenario {
                    Scenario::HandoffRejected => Ok(
                        json!({"requested":true,"requestAccepted":false,"foregroundConfirmed":true,"target":null,"reason":"os_rejected","effectState":"unknown"}),
                    ),
                    Scenario::HandoffTimeout => Ok(
                        json!({"requested":true,"requestAccepted":true,"foregroundConfirmed":false,"target":null,"reason":"confirmation_timeout","effectState":"unknown"}),
                    ),
                    _ => Ok(
                        json!({"requested":true,"requestAccepted":true,"foregroundConfirmed":true,"target":b_token(),"reason":null,"effectState":"unknown"}),
                    ),
                }
            }
            "pointer.state" => {
                assert!(matches!(self.scenario, Scenario::Original));
                assert_eq!(params, json!({}));
                Ok(pointer_state())
            }
            "window.geometry" => {
                assert_eq!(
                    params,
                    if matches!(self.scenario, Scenario::Original) {
                        json!({})
                    } else {
                        json!({"target":b_token()})
                    }
                );
                Ok(geometry())
            }
            "window.set_rect" => {
                assert_eq!(
                    params,
                    if matches!(self.scenario, Scenario::Original) {
                        json!({"rect":{"left":-700,"top":100,"width":720,"height":480},"frame":"visible"})
                    } else {
                        json!({"target":b_token(),"rect":{"left":-700,"top":100,"width":720,"height":480},"frame":"visible"})
                    }
                );
                Ok(json!({"submitted":true,"effectState":"unknown"}))
            }
            "window.action" => {
                let operation = if matches!(self.scenario, Scenario::ToggleOnly) {
                    "toggle_maximize"
                } else {
                    "restore"
                };
                assert_eq!(
                    params,
                    if matches!(self.scenario, Scenario::Original) {
                        json!({"operation":operation})
                    } else {
                        json!({"target":b_token(),"operation":operation})
                    }
                );
                Ok(json!({"submitted":true}))
            }
            "window.drag" => {
                assert_eq!(
                    params,
                    if matches!(self.scenario, Scenario::Original) {
                        json!({"mode":"move","button":"left","max_duration_ms":20,"max_updates":1})
                    } else {
                        json!({"target":b_token(),"mode":"move","button":"left","max_duration_ms":20,"max_updates":1})
                    }
                );
                Ok(drag())
            }
            _ => panic!("禁止默认A回退、capture、额外Native方法或重选窗口：{method}"),
        }
    }
}
fn prefix(window_count: usize) -> Vec<String> {
    [
        "target.context",
        "pointer.root",
        "target.handoff_pointing_root",
    ]
    .into_iter()
    .chain(WINDOW_METHODS.iter().copied().take(window_count))
    .map(str::to_owned)
    .collect()
}
fn success(kind: RuntimeKind) {
    let fixture = Fixture::new(kind);
    let host = Host::new(Scenario::Success);
    let output = kind.execute(&fixture.action, &host).unwrap();
    let mut expected = prefix(4);
    expected.push("target.context".into());
    assert_eq!(host.methods(), expected);
    let shapes = vec![matches!(kind, RuntimeKind::Node); 4];
    assert_eq!(
        output,
        json!({"status":"completed","before":a(),"after":a(),"target":b_token(),
        "shapes":shapes,"geometry":geometry(),
        "rect":{"submitted":true,"effectState":"unknown"},"restore":{"submitted":true},"drag":drag()})
    );
}
fn faults(kind: RuntimeKind, malformed: bool) {
    for (index, method) in WINDOW_METHODS.into_iter().enumerate() {
        let fixture = Fixture::new(kind);
        let host = Host::new(if malformed {
            Scenario::BadReply(method)
        } else {
            Scenario::NativeError(method)
        });
        let fault = kind.execute(&fixture.action, &host).unwrap_err();
        assert_eq!(
            fault.code,
            if malformed {
                "invalid_response"
            } else {
                "target_expired"
            }
        );
        assert_eq!(fault.method.as_deref(), Some(method));
        assert_eq!(
            fault.effect_state.as_deref(),
            Some(if method == "window.geometry" {
                "not_started"
            } else {
                "unknown"
            })
        );
        assert!(
            fault.message.contains(SDK_ERROR),
            "应保留SDK规整后的原消息：{}",
            fault.message
        );
        if matches!(kind, RuntimeKind::Node) {
            assert_eq!(
                fault.message, SDK_ERROR,
                "Node结构化错误不得替换为宿主实现栈"
            );
        }
        assert_eq!(
            host.methods(),
            prefix(index + 1),
            "错误后不得继续、重捕获或回退A"
        );
    }
}
fn handoff_stops(kind: RuntimeKind) {
    for scenario in [Scenario::HandoffRejected, Scenario::HandoffTimeout] {
        let fixture = Fixture::new(kind);
        let host = Host::new(scenario);
        let output = kind.execute(&fixture.action, &host).unwrap();
        assert_eq!(output["status"], "handoff_stopped");
        assert_eq!(output["handoff"]["target"], Value::Null);
        assert_eq!(output["shapes"], json!([]));
        assert_eq!(host.methods(), prefix(0));
    }
}
#[test]
#[ignore = "需显式新Node路径；仅真实运行时+合成Host，不调用Windows"]
fn node_captured_window_four_methods_keep_the_same_b_token() {
    success(RuntimeKind::Node);
}
#[test]
#[ignore = "需显式新ScriptHost路径；仅真实运行时+合成Host，不调用Windows"]
fn script_captured_window_four_methods_keep_the_same_b_token() {
    success(RuntimeKind::Script);
}
#[test]
#[ignore = "需显式新Node路径；合成各方法错误"]
fn node_captured_window_native_errors_stop_the_chain() {
    faults(RuntimeKind::Node, false);
}
#[test]
#[ignore = "需显式新ScriptHost路径；合成各方法错误"]
fn script_captured_window_native_errors_stop_the_chain() {
    faults(RuntimeKind::Script, false);
}
#[test]
#[ignore = "需显式新Node路径；合成各方法坏回执"]
fn node_captured_window_bad_receipts_stop_the_chain() {
    faults(RuntimeKind::Node, true);
}
#[test]
#[ignore = "需显式新ScriptHost路径；合成各方法坏回执"]
fn script_captured_window_bad_receipts_stop_the_chain() {
    faults(RuntimeKind::Script, true);
}
#[test]
#[ignore = "需显式新Node路径；拒绝/超时不进入窗口方法"]
fn node_captured_window_rejected_handoff_never_enters_window_methods() {
    handoff_stops(RuntimeKind::Node);
}
#[test]
#[ignore = "需显式新ScriptHost路径；拒绝/超时不进入窗口方法"]
fn script_captured_window_rejected_handoff_never_enters_window_methods() {
    handoff_stops(RuntimeKind::Script);
}

fn toggle_contract(kind: RuntimeKind) {
    for scenario in [
        Scenario::ToggleOnly,
        Scenario::NativeError("window.action"),
        Scenario::BadReply("window.action"),
    ] {
        let mut fixture = Fixture::new(kind);
        fixture.action.call.input = json!({"toggleOnly":true});
        let host = Host::new(scenario);
        let output = kind.execute(&fixture.action, &host);
        let mut expected = prefix(0);
        expected.push("window.action".into());
        if matches!(scenario, Scenario::ToggleOnly) {
            assert_eq!(
                output.unwrap(),
                json!({"status":"toggle_completed","before":a(),"after":a(),"target":b_token(),"shape":matches!(kind, RuntimeKind::Node),"toggle":{"submitted":true}})
            );
            expected.push("target.context".into());
        } else {
            let error = output.unwrap_err();
            assert_eq!(
                error.code,
                if matches!(scenario, Scenario::NativeError(_)) {
                    "target_expired"
                } else {
                    "invalid_response"
                }
            );
            assert_eq!(error.method.as_deref(), Some("window.action"));
            assert_eq!(error.effect_state.as_deref(), Some("unknown"));
            assert!(error.message.contains(SDK_ERROR));
        }
        assert_eq!(
            host.methods(),
            expected,
            "toggle后失败不重试、不回退A或重捕获"
        );
    }
}
#[test]
#[ignore = "需显式新Node路径；真实运行时与录制Host，不调用Windows"]
fn node_captured_toggle_keeps_b_and_rejects_unknown_or_bad_reply_without_retry() {
    toggle_contract(RuntimeKind::Node);
}
#[test]
#[ignore = "需显式新ScriptHost路径；真实运行时与录制Host，不调用Windows"]
fn script_captured_toggle_keeps_b_and_rejects_unknown_or_bad_reply_without_retry() {
    toggle_contract(RuntimeKind::Script);
}

fn pointer_state() -> Value {
    json!({"coordinateSpace":"physical-screen","x":-700,"y":200,"buttons":{"left":"down","right":"not_down_or_unavailable","middle":"not_down_or_unavailable"},"inputSource":{"kind":"polled-key-state","injected":null,"selfInjected":null}})
}
fn original_contract(kind: RuntimeKind) {
    let mut fixture = Fixture::new(kind);
    fixture.action.call.input = json!({"originalOnly":true});
    let host = Host::new(Scenario::Original);
    let result = kind.execute(&fixture.action, &host).unwrap();
    assert_eq!(
        result,
        json!({"status":"original_completed","before":a(),"after":a(),"pointer":pointer_state(),"geometry":geometry(),"rect":{"submitted":true,"effectState":"unknown"},"restore":{"submitted":true},"drag":drag()})
    );
    assert_eq!(
        host.methods(),
        [
            "target.context",
            "pointer.state",
            "window.geometry",
            "window.set_rect",
            "window.action",
            "window.drag",
            "target.context"
        ]
    );
}
fn false_native_contract(kind: RuntimeKind) {
    let mut fixture = Fixture::new(kind);
    fixture.action.call.input = json!({"markerOnly":true});
    let host = Host::new(Scenario::NoNativeContext);
    let error = kind.execute(&fixture.action, &host).unwrap_err();
    assert_eq!(error.code, "native_bridge_unavailable");
    assert_eq!(error.method.as_deref(), Some("window.geometry"));
    assert_eq!(error.effect_state.as_deref(), Some("not_started"));
    assert!(host.methods().is_empty());
}
#[test]
#[ignore = "需显式Node路径；真实运行时与录制Host，不调用Windows"]
fn node_original_window_pointer_contract_keeps_default_target() {
    original_contract(RuntimeKind::Node);
}
#[test]
#[ignore = "需显式ScriptHost路径；真实运行时与录制Host，不调用Windows"]
fn script_original_window_pointer_contract_keeps_default_target() {
    original_contract(RuntimeKind::Script);
}
#[test]
#[ignore = "需显式Node路径；false Native标记不得进入Host"]
fn node_false_native_marker_rejects_window_before_dispatch() {
    false_native_contract(RuntimeKind::Node);
}
#[test]
#[ignore = "需显式ScriptHost路径；false Native标记不得进入Host"]
fn script_false_native_marker_rejects_window_before_dispatch() {
    false_native_contract(RuntimeKind::Script);
}
