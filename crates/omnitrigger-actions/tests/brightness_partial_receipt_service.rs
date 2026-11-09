//! 真实 ActionService → Production Runtime → NativeBridge → Synthetic Native 往返。
//! 不访问屏幕、个人配置或GUI；显式指定Node/QuickJS/Boa二进制后顺序执行。
use omnitrigger_actions::{
    ActionCall, ActionError, ActionOutcome, ActionRuntime, ActionService, CancellationToken,
    InvocationContext, NativeHost, NodeRuntime, PackageCatalog, ResolvedAction, RuntimeRegistry,
    ScriptHostRuntime,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{mpsc::TryRecvError, Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

const PACKAGE: &str = "test.brightness-partial-receipt";
const METHOD: &str = "display.brightness_step";
const PRIVATE: &str = "SYNTHETIC_PRIVATE_NATIVE_MESSAGE";

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
fn configured_path(key: &str) -> PathBuf {
    let path = PathBuf::from(std::env::var_os(key).unwrap_or_else(|| panic!("显式指定{key}")));
    local(&path);
    path
}

#[derive(Clone, Copy, Debug)]
enum Engine {
    Node,
    Script,
}
impl Engine {
    fn id(self) -> &'static str {
        match self {
            Self::Node => "node",
            Self::Script => "script",
        }
    }
    fn runtime(self) -> (Arc<dyn ActionRuntime>, Value) {
        match self {
            Self::Node => {
                let program = configured_path("OMNITRIGGER_TEST_NODE");
                assert!(program.is_file());
                (
                    Arc::new(NodeRuntime::new(&program)),
                    json!({"engine":"node","program":program}),
                )
            }
            Self::Script => {
                let program = configured_path("OMNITRIGGER_TEST_SCRIPT_HOST");
                assert!(program.is_file());
                let expected = std::env::var("OMNITRIGGER_TEST_SCRIPT_ENGINE").unwrap();
                assert!(matches!(expected.as_str(), "quickjs" | "boa"));
                let output = Command::new(&program)
                    .arg("--describe-runtime")
                    .output()
                    .unwrap();
                assert!(output.status.success());
                let description: Value = serde_json::from_slice(&output.stdout).unwrap();
                assert_eq!(description["engine"], expected);
                assert_eq!(description["product"], "omnitrigger-script-host");
                (
                    Arc::new(ScriptHostRuntime::new(&program)),
                    json!({"engine":expected,"program":program,"description":description}),
                )
            }
        }
    }
}

// 委托并记录真实Runtime输出；新增场景仅在返回后取消，不伪造错误或桥接。
struct ObserveProduction {
    inner: Arc<dyn ActionRuntime>,
    result: Mutex<Option<Result<Value, ActionError>>>,
    cancel_after_result: bool,
}
impl ActionRuntime for ObserveProduction {
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
        let result = self
            .inner
            .execute_with_host(action, context, cancellation, host);
        *self.result.lock().unwrap() = Some(result.clone());
        if self.cancel_after_result {
            // Production已真实返回完整错误后才取消，不能借此声称断链也能送达。
            assert!(matches!(&result, Err(error)
                if error.method.as_deref() == Some(METHOD)
                    && error.effect_state.as_deref() == Some("unknown")
                    && error.brightness_receipt.is_some()));
            cancellation.cancel();
        }
        result
    }
}

fn receipt() -> Value {
    json!({"strategy":"physical_then_wmi","effectState":"unknown","physical":{
        "members":[
            {"index":0,"status":"confirmed","previousRaw":40,"requestedRaw":45,"currentRaw":45,"errorCode":null},
            {"index":1,"status":"set_unconfirmed","previousRaw":20,"requestedRaw":25,"currentRaw":null,"errorCode":"brightness_unknown"},
            {"index":2,"status":"read_rejected","previousRaw":null,"requestedRaw":null,"currentRaw":null,"errorCode":"brightness_unknown"}
        ],"errorCode":null},"wmi":null})
}
struct SyntheticBrightness {
    code: &'static str,
    with_receipt: bool,
    calls: Mutex<Vec<(String, Value)>>,
}
impl NativeHost for SyntheticBrightness {
    fn call(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
    ) -> Result<Value, ActionError> {
        assert!(
            !cancellation.is_cancelled(),
            "此用例注入已交付的操作错误，不取消外层传输"
        );
        assert_eq!(method, METHOD, "不得真实读写亮度或调用其他Native方法");
        assert_eq!(
            params,
            json!({"direction":"up","strategy":"physical_then_wmi"})
        );
        let mut calls = self.calls.lock().unwrap();
        calls.push((method.into(), params));
        assert_eq!(calls.len(), 1, "部分写入后不得重试或额外调用WMI");
        let mut error = ActionError::new(self.code, PRIVATE);
        error.method = Some(METHOD.into());
        error.effect_state = Some("unknown".into());
        if self.with_receipt {
            error.brightness_receipt = Some(Box::new(receipt()));
        }
        Err(error)
    }
}

fn fixture(engine: Engine) -> PathBuf {
    let parent = configured_path("OMNITRIGGER_TEST_BRIGHTNESS_FIXTURE_PARENT");
    assert!(parent.is_dir());
    // 原子创建，已有目录不复用、不清理；保留失败现场。
    let root = (0..1024)
        .find_map(|attempt| {
            let path = parent.join(format!(
                "brightness-service-{}-{}-{attempt}",
                std::process::id(),
                engine.id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => Some(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(error) => panic!("新建合成Fixture失败：{error}"),
            }
        })
        .expect("无法分配独占Fixture目录");
    local(&root);
    let package = root.join("synthetic");
    fs::create_dir(&package).unwrap();
    let async_kw = if matches!(engine, Engine::Node) {
        "async "
    } else {
        ""
    };
    let await_kw = if matches!(engine, Engine::Node) {
        "await "
    } else {
        ""
    };
    let entry = format!(
        "export {async_kw}function main(ctx) {{\n\
        try {{\n\
          {await_kw}ctx.desktop.stepBrightness({{direction:'up',strategy:'physical_then_wmi'}});\n\
          throw new Error('Synthetic Native必须返回错误');\n\
        }} catch (error) {{\n\
          if (ctx.input.mode==='throw') throw error;\n\
          return {{code:error.code,effectState:error.effectState,method:error.method,brightnessReceipt:error.brightnessReceipt??null}};\n\
        }}\n\
        }}\nexport default main;\n"
    );
    fs::write(package.join("main.mjs"), entry).unwrap();
    fs::write(package.join("action.json"), serde_json::to_vec_pretty(&json!({
        "schema_version":1,"id":PACKAGE,"name":"亮度匿名回执真实桥接测试","version":"1.0.0",
        "actions":[{"id":"work","title":"合成亮度","version":1,"runtime":engine.id(),"entry":"main.mjs"}]
    })).unwrap()).unwrap();
    root
}

fn assert_error(error: &ActionError, code: &str, with_receipt: bool) {
    assert_eq!(error.code, code);
    assert_eq!(error.method.as_deref(), Some(METHOD));
    assert_eq!(error.effect_state.as_deref(), Some("unknown"));
    assert!(!error.message.contains(PRIVATE));
    assert_eq!(
        error.brightness_receipt,
        with_receipt.then(|| Box::new(receipt()))
    );
}

fn run(engine: Engine) {
    let root = fixture(engine);
    let catalog = PackageCatalog::load(&root);
    assert!(catalog.diagnostics.is_empty(), "{:?}", catalog.diagnostics);
    assert_eq!(catalog.actions.len(), 1);
    let mut evidence = Vec::new();
    // 先验证timeout，再验证cancelled和外层迟到取消，分别检查回执保留。
    for (index, (mode, code, with_receipt, cancel_after_result)) in [
        ("catch", "timeout", true, false),
        ("catch", "cancelled", true, false),
        ("throw", "cancelled", false, false),
        ("throw", "timeout", true, false),
        ("throw", "cancelled", true, false),
        ("throw", "timeout", true, true),
        ("throw", "cancelled", true, true),
    ]
    .into_iter()
    .enumerate()
    {
        let (production, runtime_identity) = engine.runtime();
        let observed = Arc::new(ObserveProduction {
            inner: production,
            result: Mutex::new(None),
            cancel_after_result,
        });
        let mut registry = RuntimeRegistry::new();
        registry.register(engine.id(), observed.clone()).unwrap();
        let service = ActionService::new(catalog.clone(), registry);
        let host = Arc::new(SyntheticBrightness {
            code,
            with_receipt,
            calls: Mutex::new(Vec::new()),
        });
        let binding = 7100 + index as u64;
        let call = ActionCall {
            package_id: PACKAGE.into(),
            action_id: "work".into(),
            action_version: 1,
            input: json!({"mode":mode}),
        };
        let invocation = service
            .submit_with_host(binding, call, Some(host.clone()))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(64);
        let mut started = false;
        let terminal = loop {
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
                    assert!(
                        Instant::now() < deadline,
                        "{engine:?} {mode} {code} 等待Service终态超时"
                    );
                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("{error}"),
            }
        };
        assert!(!service.is_running(binding));
        assert_eq!(host.calls.lock().unwrap().len(), 1);
        let runtime_result = observed
            .result
            .lock()
            .unwrap()
            .clone()
            .expect("必须经过真实Production Runtime");
        let projected = match &terminal {
            ActionOutcome::Succeeded { output } => json!({"kind":"succeeded","output":output}),
            ActionOutcome::Failed { error } => json!({"kind":"failed","error":error}),
            ActionOutcome::Cancelled => json!({"kind":"cancelled"}),
            ActionOutcome::Started => unreachable!(),
        };
        evidence.push(json!({"mode":mode,"code":code,"withReceipt":with_receipt,
            "runtime":runtime_identity,"nativeCalls":1,"runtimeResult":runtime_result,"serviceTerminal":projected,"outerCancelAfterProductionResult":cancel_after_result}));
        fs::write(
            root.join("acceptance.json"),
            serde_json::to_vec_pretty(&json!({
                "syntheticNativeOnly":true,"hardwareCalled":false,"productionNativeBridge":true,
                "transportAlreadyDisconnectedDeliveryGuaranteed":false,"cases":evidence
            }))
            .unwrap(),
        )
        .unwrap();
        if mode == "catch" {
            let expected = json!({"code":code,"effectState":"unknown","method":METHOD,"brightnessReceipt":receipt()});
            assert_eq!(runtime_result.unwrap(), expected);
            assert_eq!(terminal, ActionOutcome::Succeeded { output: expected });
        } else {
            let error = runtime_result.unwrap_err();
            assert_error(&error, code, with_receipt);
            if code == "cancelled" && !with_receipt {
                assert_eq!(terminal, ActionOutcome::Cancelled, "保持既有无回执取消投影");
            } else {
                let ActionOutcome::Failed { error } = terminal else {
                    panic!("真实Runtime已交付{code}/unknown与回执，但Service终态丢失：{terminal:?}；见acceptance.json");
                };
                assert_error(&error, code, with_receipt);
            }
        }
    }
}

#[test]
#[ignore = "需显式配置Node与自有数据目录；不访问显示器"]
fn node_brightness_partial_receipt_ordinary_service_roundtrip() {
    run(Engine::Node);
}

#[test]
#[ignore = "需显式配置QuickJS或Boa与自有数据目录；不访问显示器"]
fn script_brightness_partial_receipt_ordinary_service_roundtrip() {
    run(Engine::Script);
}
