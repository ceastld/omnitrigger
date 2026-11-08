//! 普通 NodeRuntime/ScriptHostRuntime 加记录型 NativeHost；不操作真实窗口。
use omnitrigger_actions::{
    ActionCall, ActionDefinition, ActionError, ActionRuntime, CancellationToken, InvocationContext,
    NativeHost, NodeRuntime, ResolvedAction, ScriptHostRuntime,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Mutex,
    },
};

#[derive(Clone, Copy, Debug)]
enum Scenario {
    GoodA(&'static str),
    GoodB(&'static str),
    RestoreB,
    BlockedClass,
    UnicodeClass,
    LongSClass,
    KelvinClass,
    UnicodeSubstring,
    ExactClassA,
    ExactClassB,
    GenericUnsupported,
    WrongClassB,
    UnknownClassA,
    BadClassFieldsA(u8),
    BadClassFieldsB(u8),
    LateClassReject,
    WriteClassReject,
    MissingClass,
    NegativeSize,
    BadWrite,
    ExpiredB,
    UnconfirmedB,
    IdentityChanged,
}
impl Scenario {
    fn captured(self) -> bool {
        matches!(
            self,
            Self::GoodB(_)
                | Self::RestoreB
                | Self::ExpiredB
                | Self::UnconfirmedB
                | Self::ExactClassB
                | Self::WrongClassB
                | Self::BadClassFieldsB(_)
        )
    }
    fn operation(self) -> &'static str {
        match self {
            Self::GoodA(operation) | Self::GoodB(operation) => operation,
            Self::NegativeSize => "decrease",
            _ => "increase",
        }
    }
}

fn identity(b: bool) -> Value {
    if b {
        json!({"window_handle":"202","process_id":20,"thread_id":21,"process_created_file_time":"18446744073709551615"})
    } else {
        json!({"window_handle":"101","process_id":10,"thread_id":11,"process_created_file_time":"134000000000000001"})
    }
}
fn original(b: bool) -> Value {
    let mut value = identity(b);
    value["expires_at_unix_ms"] = 9_999_999_999_999u64.into();
    value
}
fn token() -> Value {
    json!({"capture_id":7,"context":original(true)})
}
fn geometry(class_name: &str, maximized: bool, width: i32, height: i32) -> Value {
    json!({"coordinateSpace":"physical-screen","className":class_name,
        "outerRect":{"left":-808,"top":92,"width":width+16,"height":height+38},
        "visibleFrameRect":{"left":-800,"top":100,"width":width,"height":height},
        "monitorRect":{"left":-1920,"top":0,"width":1920,"height":1080},
        "workArea":{"left":-1920,"top":0,"width":1920,"height":1040},
        "primaryScreen":{"width":2560,"height":1440},"windowDpi":144,"maximized":maximized})
}
struct Host {
    scenario: Scenario,
    calls: Mutex<Vec<(String, Value)>>,
    restored: AtomicBool,
    target_reads: AtomicUsize,
}
impl Host {
    fn new(scenario: Scenario) -> Self {
        Self {
            scenario,
            calls: Mutex::new(Vec::new()),
            restored: AtomicBool::new(false),
            target_reads: AtomicUsize::new(0),
        }
    }
}
impl NativeHost for Host {
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
        if method.starts_with("window.") {
            if self.scenario.captured() {
                assert_eq!(
                    params.get("target"),
                    Some(&token()),
                    "B必须沿同一Lease，不得退回A"
                );
            } else {
                assert!(params.get("target").is_none());
            }
        }
        match method {
            "target.context" => {
                let changed = matches!(self.scenario, Scenario::IdentityChanged)
                    && self.target_reads.fetch_add(1, Ordering::SeqCst) > 0;
                Ok(original(changed))
            }
            "pointer.root" => Ok(
                json!({"sampledAt":"call-time","coordinateSpace":"physical-screen",
                "point":{"x":-750,"y":110},"inputSource":{"kind":"polled-pointer","injected":null,"selfInjected":null},
                "pointingWindow":identity(self.scenario.captured()),"rootWindow":identity(self.scenario.captured())}),
            ),
            "target.handoff_pointing_root" => {
                assert!(self.scenario.captured());
                assert_eq!(params["expected_root"], identity(true));
                if matches!(self.scenario, Scenario::UnconfirmedB) {
                    Ok(
                        json!({"requested":true,"requestAccepted":true,"foregroundConfirmed":false,
                        "target":null,"reason":"confirmation_timeout","effectState":"unknown"}),
                    )
                } else {
                    Ok(
                        json!({"requested":true,"requestAccepted":true,"foregroundConfirmed":true,
                        "target":token(),"reason":null,"effectState":"unknown"}),
                    )
                }
            }
            "window.geometry" => {
                let class_rejected = matches!(
                    self.scenario,
                    Scenario::ExactClassA
                        | Scenario::ExactClassB
                        | Scenario::GenericUnsupported
                        | Scenario::WrongClassB
                        | Scenario::UnknownClassA
                        | Scenario::BadClassFieldsA(_)
                        | Scenario::BadClassFieldsB(_)
                ) || matches!(self.scenario, Scenario::LateClassReject)
                    && self.restored.load(Ordering::SeqCst);
                if class_rejected {
                    let mut error = ActionError::new(
                        if matches!(self.scenario, Scenario::GenericUnsupported) {
                            "unsupported_target"
                        } else {
                            "window_class_blocked"
                        },
                        "不调整桌面或任务栏宿主窗口。",
                    );
                    error.method = match self.scenario {
                        Scenario::BadClassFieldsA(1) | Scenario::BadClassFieldsB(1) => None,
                        Scenario::BadClassFieldsA(2) | Scenario::BadClassFieldsB(2) => {
                            Some("pointer.root".into())
                        }
                        _ => Some(method.into()),
                    };
                    error.effect_state = Some(
                        if matches!(
                            self.scenario,
                            Scenario::WrongClassB | Scenario::UnknownClassA
                        ) {
                            "unknown"
                        } else {
                            "not_started"
                        }
                        .into(),
                    );
                    if matches!(
                        self.scenario,
                        Scenario::BadClassFieldsA(3) | Scenario::BadClassFieldsB(3)
                    ) {
                        error.effect_state = None;
                    }
                    return Err(error);
                }
                if matches!(self.scenario, Scenario::ExpiredB) {
                    let mut error = ActionError::new("target_expired", "记录Host拒绝过期Lease");
                    error.method = Some(method.into());
                    error.effect_state = Some("not_started".into());
                    return Err(error);
                }
                let restored = self.restored.load(Ordering::SeqCst);
                let class_name = match self.scenario {
                    Scenario::BlockedClass => "Custom.WorkerW.Proxy",
                    Scenario::UnicodeClass => "窗口类",
                    Scenario::LongSClass => "ſhell_TrayWnd",
                    Scenario::KelvinClass => "WorKerW",
                    Scenario::UnicodeSubstring => "窗口WorkerW代理",
                    _ => "TestWindow",
                };
                let (width, height) = if matches!(self.scenario, Scenario::NegativeSize) {
                    (30, 100)
                } else if restored {
                    (400, 300)
                } else {
                    (801, 601)
                };
                let mut value = geometry(
                    class_name,
                    matches!(
                        self.scenario,
                        Scenario::RestoreB | Scenario::LateClassReject
                    ) && !restored,
                    width,
                    height,
                );
                if matches!(self.scenario, Scenario::MissingClass) {
                    value.as_object_mut().unwrap().remove("className");
                }
                Ok(value)
            }
            "window.action" => {
                assert!(matches!(
                    self.scenario,
                    Scenario::RestoreB | Scenario::LateClassReject
                ));
                assert_eq!(params["operation"], "restore");
                assert!(!self.restored.swap(true, Ordering::SeqCst), "不得重复还原");
                Ok(json!({"submitted":true}))
            }
            "window.set_rect" => {
                if matches!(self.scenario, Scenario::WriteClassReject) {
                    let mut error =
                        ActionError::new("window_class_blocked", "不调整桌面或任务栏宿主窗口。");
                    error.method = Some(method.into());
                    error.effect_state = Some("not_started".into());
                    return Err(error);
                }
                assert_eq!(params["frame"], "visible");
                let delta = if self.scenario.operation() == "increase" {
                    50
                } else {
                    -50
                };
                assert_eq!(
                    params["rect"],
                    json!({"left":-800,"top":100,"width":801+delta,"height":601+delta})
                );
                Ok(
                    json!({"submitted":!matches!(self.scenario, Scenario::BadWrite),"effectState":"unknown"}),
                )
            }
            _ => panic!("不得新增捕获、输入、State或其他Native调用：{method}"),
        }
    }
}

fn local(path: &Path) {
    assert!(path.is_absolute());
    for part in path.ancestors() {
        if let Some(name) = part.file_name() {
            let name = name.to_string_lossy().to_ascii_lowercase();
            assert!(
                !name.starts_with("onedrive") && name != "dropbox" && !name.starts_with("icloud")
            );
        }
        let metadata = fs::symlink_metadata(part).unwrap();
        assert!(!metadata.file_type().is_symlink());
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            assert_eq!(metadata.file_attributes() & 0x441400, 0);
        }
    }
}
fn program(key: &str) -> PathBuf {
    let value = PathBuf::from(std::env::var_os(key).unwrap_or_else(|| panic!("必须显式设置{key}")));
    local(&value);
    assert!(value.is_file());
    value
}
fn run_cases(runtime: &dyn ActionRuntime, kind: &str) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/actions/window-size-step")
        .canonicalize()
        .unwrap();
    local(&root);
    for scenario in [
        Scenario::GoodA("increase"),
        Scenario::GoodA("decrease"),
        Scenario::GoodB("increase"),
        Scenario::GoodB("decrease"),
        Scenario::RestoreB,
        Scenario::BlockedClass,
        Scenario::UnicodeClass,
        Scenario::LongSClass,
        Scenario::KelvinClass,
        Scenario::UnicodeSubstring,
        Scenario::ExactClassA,
        Scenario::ExactClassB,
        Scenario::GenericUnsupported,
        Scenario::WrongClassB,
        Scenario::UnknownClassA,
        Scenario::BadClassFieldsA(1),
        Scenario::BadClassFieldsA(2),
        Scenario::BadClassFieldsA(3),
        Scenario::BadClassFieldsB(1),
        Scenario::BadClassFieldsB(2),
        Scenario::BadClassFieldsB(3),
        Scenario::LateClassReject,
        Scenario::WriteClassReject,
        Scenario::MissingClass,
        Scenario::NegativeSize,
        Scenario::BadWrite,
        Scenario::ExpiredB,
        Scenario::UnconfirmedB,
        Scenario::IdentityChanged,
    ] {
        let id = format!(
            "{}{}",
            scenario.operation(),
            if kind == "script" { "-script" } else { "" }
        );
        let entry = format!("{id}.mjs");
        local(&root.join(&entry));
        let mut input =
            json!({"trigger":"AdvancedMouseAction","step":50,"targetPolicy":"original-only"});
        if scenario.captured() {
            input["targetPolicy"] = "handoff-pointing-root".into();
            input["handoff"] = json!({"validForMs":5000,"confirmTimeoutMs":1000});
        }
        let action = ResolvedAction {
            package_root: root.clone(),
            entry_path: root.join(&entry),
            call: ActionCall {
                package_id: "examples.window-size-step".into(),
                action_id: id.clone(),
                action_version: 1,
                input,
            },
            definition: ActionDefinition {
                id,
                title: "窗口大小来源步进".into(),
                version: 1,
                runtime: kind.into(),
                entry,
                single_instance: false,
            },
        };
        let host = Host::new(scenario);
        let result = runtime.execute_with_host(
            &action,
            &InvocationContext {
                invocation_id: 9_007_199_254_740_993,
                binding_id: u64::MAX,
            },
            &CancellationToken::new(),
            Some(&host),
        );
        let calls = host.calls.lock().unwrap();
        let writes = calls.iter().filter(|(m, _)| m == "window.set_rect").count();
        match scenario {
            Scenario::GoodA(_)
            | Scenario::GoodB(_)
            | Scenario::RestoreB
            | Scenario::UnicodeClass
            | Scenario::LongSClass => {
                let value = result.unwrap_or_else(|error| panic!("{kind} {scenario:?}: {error}"));
                assert_eq!(value["status"], "size_submitted", "{scenario:?}");
                assert_eq!(
                    value["className"],
                    match scenario {
                        Scenario::UnicodeClass => "窗口类",
                        Scenario::LongSClass => "ſhell_TrayWnd",
                        _ => "TestWindow",
                    }
                );
                assert_eq!(value["effectState"], "unknown");
                assert_eq!(value["windowChangeConfirmed"], false);
                assert_eq!(value["primaryScreen"], json!({"width":2560,"height":1440}));
                assert_eq!(writes, 1);
            }
            Scenario::BlockedClass
            | Scenario::UnicodeSubstring
            | Scenario::ExactClassA
            | Scenario::ExactClassB => {
                let value = result.unwrap_or_else(|error| panic!("{kind} {scenario:?}: {error}"));
                assert_eq!(value["status"], "source_class_blocked");
                assert_eq!(value["submitted"], false);
                let exact = matches!(scenario, Scenario::ExactClassA | Scenario::ExactClassB);
                assert_eq!(
                    value["noOpReason"],
                    if exact {
                        "exact_platform_class"
                    } else {
                        "source_pattern"
                    }
                );
                assert_eq!(
                    value["effectState"],
                    if scenario.captured() {
                        "unknown"
                    } else {
                        "not_started"
                    }
                );
                if exact {
                    assert!(value["className"].is_null());
                }
                assert_eq!(writes, 0);
                assert!(!calls.iter().any(|(method, _)| method == "window.action"));
            }
            Scenario::UnconfirmedB => {
                assert_eq!(
                    result.unwrap_or_else(|error| panic!("{kind} {scenario:?}: {error}"))["status"],
                    "handoff_not_confirmed"
                );
                assert_eq!(writes, 0);
            }
            _ => {
                let error = match result {
                    Err(error) => error,
                    Ok(value) => panic!("{kind} {scenario:?}: 预期错误却成功：{value}"),
                };
                let expected = match scenario {
                    Scenario::KelvinClass => "source_regex_culture_unavailable",
                    Scenario::UnknownClassA => "window_class_blocked",
                    Scenario::GenericUnsupported => "unsupported_target",
                    Scenario::LateClassReject | Scenario::WriteClassReject => {
                        "window_class_blocked"
                    }
                    Scenario::NegativeSize => "source_size_semantics_unavailable",
                    Scenario::ExpiredB => "target_expired",
                    Scenario::IdentityChanged => "target_changed",
                    _ => "invalid_response",
                };
                assert_eq!(error.code, expected, "{scenario:?}: {error}");
                let effect = if matches!(
                    scenario,
                    Scenario::ExpiredB
                        | Scenario::BadWrite
                        | Scenario::WrongClassB
                        | Scenario::UnknownClassA
                        | Scenario::BadClassFieldsB(_)
                        | Scenario::LateClassReject
                ) {
                    "unknown"
                } else {
                    "not_started"
                };
                assert_eq!(error.effect_state.as_deref(), Some(effect));
                assert_eq!(
                    writes,
                    usize::from(matches!(
                        scenario,
                        Scenario::BadWrite | Scenario::WriteClassReject
                    ))
                );
            }
        }
    }
}
#[test]
#[ignore = "显式选择Node；普通入口+记录Host29场景，非真实窗口"]
fn window_size_step_node_ordinary_entries() {
    run_cases(
        &NodeRuntime::new(program("OMNITRIGGER_WINDOW_SIZE_TEST_NODE")),
        "node",
    );
}
#[test]
#[ignore = "显式选择新Script宿主；普通入口+记录Host29场景，非真实窗口"]
fn window_size_step_script_ordinary_entries() {
    run_cases(
        &ScriptHostRuntime::new(program("OMNITRIGGER_TEST_SCRIPT_HOST")),
        "script",
    );
}
