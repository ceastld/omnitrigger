//! 正常ScriptRuntime执行现有N10入口；所有clip/角标端口都是严格合成Host。
//! 不创建业务进程、剪贴板或GUI；显式运行ignored才启动真实脚本宿主。
use omnitrigger_actions::{
    ActionCall, ActionError, ActionOutcome, ActionService, CancellationToken, NativeHost,
    PackageCatalog, RuntimeRegistry, ScriptHostRuntime,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{mpsc::TryRecvError, Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
const ACTION: &str = "32c12786-9bb8-4b0c-8d55-7e6a4c8a5d10";
fn local(path: &Path) {
    assert!(path.is_absolute());
    for part in path.ancestors() {
        assert!(!part.file_name().is_some_and(|name| {
            let name = name.to_string_lossy().to_ascii_lowercase();
            name.starts_with("onedrive") || name == "dropbox" || name.starts_with("icloud")
        }));
        let metadata = fs::symlink_metadata(part).unwrap();
        assert!(!metadata.file_type().is_symlink());
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            assert_eq!(metadata.file_attributes() & 0x441400, 0);
        }
    }
}
fn program() -> PathBuf {
    let path = PathBuf::from(
        std::env::var_os("OMNITRIGGER_TEST_SCRIPT_HOST").expect("必须显式指定配套正常Script宿主"),
    );
    local(&path);
    assert!(path.is_file());
    path
}
fn input() -> Value {
    json!({"cliExecutable":"C:\\Synthetic\\eaclip-cli.exe","hostExecutable":"C:\\Synthetic\\cliphost.exe","dataRoot":"C:\\Synthetic\\data","profile":"script-test","timeoutMs":1000,"actionParameter":{"present":true,"value":null}})
}
#[derive(Clone, Copy, Debug)]
enum Scenario {
    Success,
    BadHint,
    HintRejected,
    ClipUnknown,
}
struct Host {
    scenario: Scenario,
    binding: u64,
    target: Value,
    calls: Mutex<Vec<(String, Value)>>,
}
impl Host {
    fn new(scenario: Scenario, binding: u64) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        Self {
            scenario,
            binding,
            target: json!({"window_handle":"101","process_created_file_time":"134000000000000001","expires_at_unix_ms":now+29000}),
            calls: Mutex::new(Vec::new()),
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
        let mut calls = self.calls.lock().unwrap();
        calls.push((method.to_owned(), params.clone()));
        match calls.len() {
            1 => {
                assert_eq!(method, "target.context");
                assert_eq!(params, json!({}));
                Ok(self.target.clone())
            }
            2 => {
                assert_eq!(method, "process.capture");
                assert_eq!(
                    params,
                    json!({"program":"C:\\Synthetic\\eaclip-cli.exe","args":["verb","clip toggle","--json","--host","C:\\Synthetic\\cliphost.exe","--no-start","--test-data-root","C:\\Synthetic\\data","--target-context",serde_json::to_string(&self.target).unwrap(),"--timeout-ms","1000"],"timeout_ms":2000,"allow_non_zero":true})
                );
                let reply = if matches!(self.scenario, Scenario::ClipUnknown) {
                    json!({"ok":false,"state":"unknown","submitted":true,"error":{"code":"synthetic_failure"}})
                } else {
                    json!({"ok":true,"state":"accepted","submitted":true,"result":{"started":false,"targetContextVersion":1,"targetContextVerified":true,"executable":"C:\\Synthetic\\cliphost.exe","dataRoot":"C:\\Synthetic\\data","testProfile":true,"captureEnabled":false}})
                };
                Ok(
                    json!({"code":if matches!(self.scenario,Scenario::ClipUnknown){1}else{0},"stdout":serde_json::to_string(&reply).unwrap(),"stderr":""}),
                )
            }
            3 => {
                assert!(!matches!(self.scenario, Scenario::ClipUnknown));
                assert_eq!(method, "ui.action_hint.scoped");
                assert_eq!(params.as_object().unwrap().len(), 3);
                assert_eq!(params["text"], "n10");
                assert_eq!(params["placement"], "topRight");
                assert_eq!(params["scope"]["binding_id"], self.binding);
                assert_eq!(params["scope"]["package_id"], "examples.n10-rust");
                assert_eq!(params["scope"]["action_id"], ACTION);
                assert_eq!(params["scope"]["action_version"], 1);
                assert_eq!(params["scope"].as_object().unwrap().len(), 5);
                match self.scenario {
                    Scenario::HintRejected => {
                        let mut error =
                            ActionError::new("ui_unavailable", "secret original Host body");
                        error.method = Some("ui.action_hint".into());
                        error.effect_state = Some("not_started".into());
                        Err(error)
                    }
                    Scenario::BadHint => Ok(json!({"updated":false})),
                    _ => Ok(json!({"updated":true})),
                }
            }
            _ => panic!("禁止重试clip、重新捕获目标或额外调用"),
        }
    }
}
#[test]
#[ignore = "正常ScriptHost与严格合成Host；不执行clip/剪贴板/可见角标"]
fn n10_normal_entry_uses_actual_service_scope_and_clip_then_corner_hint() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/actions")
        .canonicalize()
        .unwrap();
    local(&root);
    let catalog = PackageCatalog::load(&root);
    for (index, scenario) in [
        Scenario::Success,
        Scenario::BadHint,
        Scenario::HintRejected,
        Scenario::ClipUnknown,
    ]
    .into_iter()
    .enumerate()
    {
        let binding = 700 + index as u64;
        let call = ActionCall {
            package_id: "examples.n10-rust".into(),
            action_id: ACTION.into(),
            action_version: 1,
            input: input(),
        };
        let resolved = catalog.resolve(&call).unwrap();
        assert_eq!(resolved.definition.runtime, "script");
        assert_eq!(resolved.definition.entry, "n10-rust-bound.mjs");
        assert!(resolved.definition.single_instance);
        local(&resolved.entry_path);
        let mut registry = RuntimeRegistry::new();
        registry
            .register("script", Arc::new(ScriptHostRuntime::new(program())))
            .unwrap();
        let service = ActionService::new(catalog.clone(), registry);
        let host = Arc::new(Host::new(scenario, binding));
        let invocation = service
            .submit_with_host(binding, call, Some(host.clone()))
            .unwrap();
        // 等待略长于正常宿主的60秒期限；不修改生产预算。
        let deadline = Instant::now() + Duration::from_secs(64);
        let mut started = false;
        let outcome = loop {
            match service.try_recv() {
                Ok(event) => {
                    assert_eq!(event.id, binding);
                    assert_eq!(event.invocation_id, invocation);
                    if matches!(event.outcome, ActionOutcome::Started) {
                        assert!(!started);
                        started = true;
                    } else {
                        assert!(started);
                        break event.outcome;
                    }
                }
                Err(TryRecvError::Empty) => {
                    assert!(Instant::now() < deadline);
                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("{error}"),
            }
        };
        assert!(!service.is_running(binding));
        let calls = host.calls.lock().unwrap();
        assert_eq!(
            calls.len(),
            if matches!(scenario, Scenario::ClipUnknown) {
                2
            } else {
                3
            }
        );
        if calls.len() == 3 {
            assert_eq!(calls[2].1["scope"]["invocation_id"], invocation);
        }
        match scenario {
            Scenario::Success => {
                let ActionOutcome::Succeeded { output } = outcome else {
                    panic!("应成功")
                };
                assert_eq!(output["commandEffectState"], "accepted_only");
                assert_eq!(output["uiRendered"], false);
            }
            _ => {
                let ActionOutcome::Failed { error } = outcome else {
                    panic!("应失败：{scenario:?}")
                };
                assert_eq!(error.effect_state.as_deref(), Some("unknown"));
                assert!(!error.message.contains("secret original Host body"));
                let detail = error
                    .message
                    .lines()
                    .find_map(|line| line.strip_prefix("N10_DETAIL="))
                    .expect("必须保留结构化详情及正常源码诊断");
                let detail: Value = serde_json::from_str(detail).unwrap();
                assert_eq!(detail["submitted"], true);
                assert_eq!(detail["retryAllowed"], false);
                if matches!(scenario, Scenario::ClipUnknown) {
                    assert_eq!(detail["phase"], "business");
                    assert_eq!(
                        error.method.as_deref(),
                        Some("cliphost.runVerbWithTargetV1")
                    );
                } else {
                    assert_eq!(detail["phase"], "action_hint");
                    assert_eq!(detail["businessAccepted"], true);
                    assert_eq!(detail["businessEffectState"], "accepted_only");
                    assert_eq!(error.method.as_deref(), Some("ui.action_hint"));
                }
            }
        }
    }
}
