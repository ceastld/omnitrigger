//! 仅运行本测试生成的无副作用脚本，验证真实Runtime的返回值序列化诊断。
//! Node运行一次；同一Script测试由Root分别指定正常QuickJS和Boa宿主。
use omnitrigger_actions::{
    ActionCall, ActionError, ActionOutcome, ActionService, CancellationToken, NativeHost,
    NodeRuntime, PackageCatalog, RuntimeRegistry, ScriptHostRuntime,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{mpsc::TryRecvError, Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const PACKAGE: &str = "test.result-serialization-diagnostic";
const SENTINEL: &str = "SYNTHETIC_SERIALIZATION_THROW";
const ENTRY: &str = "import {result} from './lib/serialize.mjs';\nexport function main(ctx) { return ctx.input.mode==='plain'?{value:3}:result(ctx.input.mode); }\nexport default main;\n";
// throw在原始相对模块第4行；不插入包装，不人工拼造调用栈。
const HELPER: &str = "export function result(mode) {\n  return {\n    toJSON() {\n      if(mode==='throw-null') throw null; if(mode==='throw-undefined') throw undefined; throw new Error('SYNTHETIC_SERIALIZATION_THROW');\n    }\n  };\n}\n";

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
fn program(key: &str) -> PathBuf {
    let path = PathBuf::from(std::env::var_os(key).unwrap_or_else(|| panic!("必须显式指定{key}")));
    local(&path);
    assert!(path.is_file());
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
    fn registry(self) -> RuntimeRegistry {
        let mut registry = RuntimeRegistry::new();
        match self {
            Self::Node => registry
                .register(
                    self.id(),
                    Arc::new(NodeRuntime::new(program("OMNITRIGGER_TEST_NODE"))),
                )
                .unwrap(),
            Self::Script => registry
                .register(
                    self.id(),
                    Arc::new(ScriptHostRuntime::new(program(
                        "OMNITRIGGER_TEST_SCRIPT_HOST",
                    ))),
                )
                .unwrap(),
        }
        registry
    }
}

#[derive(Default)]
struct NoNative(Mutex<Vec<String>>);
impl NativeHost for NoNative {
    fn call(&self, method: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
        self.0.lock().unwrap().push(method.into());
        Err(ActionError::new(
            "unexpected_native_call",
            "合成脚本不得调用Native。",
        ))
    }
}

fn fixture(engine: Engine) -> PathBuf {
    let parent = std::env::temp_dir();
    local(&parent);
    let root = parent.join(format!(
        "omnitrigger-result-diagnostic-{}-{}-{}",
        std::process::id(),
        engine.id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    let root = root.canonicalize().unwrap();
    local(&root);
    let package = root.join("synthetic");
    fs::create_dir(&package).unwrap();
    fs::create_dir(package.join("lib")).unwrap();
    fs::write(package.join("main.mjs"), ENTRY).unwrap();
    fs::write(package.join("lib/serialize.mjs"), HELPER).unwrap();
    fs::write(
        package.join("action.json"),
        serde_json::to_vec(&json!({
            "schema_version":1,"id":PACKAGE,"name":"无副作用序列化诊断","version":"1.0.0",
            "actions":[
                {"id":"node","title":"Node","version":1,"runtime":"node","entry":"main.mjs"},
                {"id":"script","title":"Script","version":1,"runtime":"script","entry":"main.mjs"}
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    root
}

fn run(engine: Engine) {
    let root = fixture(engine);
    let catalog = PackageCatalog::load(&root);
    assert!(catalog.diagnostics.is_empty(), "{:?}", catalog.diagnostics);
    assert_eq!(catalog.actions.len(), 2);
    let modes: &[&str] = match engine {
        Engine::Node => &[
            "plain",
            "throw-serialization",
            "throw-null",
            "throw-undefined",
        ],
        Engine::Script => &["plain", "throw-serialization"],
    };
    for (index, &mode) in modes.iter().enumerate() {
        let service = ActionService::new(catalog.clone(), engine.registry());
        let host = Arc::new(NoNative::default());
        let binding = 920 + index as u64;
        let call = ActionCall {
            package_id: PACKAGE.into(),
            action_id: engine.id().into(),
            action_version: 1,
            input: json!({"mode":mode}),
        };
        let resolved = catalog.resolve(&call).unwrap();
        assert_eq!(resolved.definition.entry, "main.mjs");
        assert_eq!(resolved.definition.runtime, engine.id());
        let invocation = service
            .submit_with_host(binding, call, Some(host.clone()))
            .unwrap();
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
                    assert!(Instant::now() < deadline, "{engine:?} {mode} 超时");
                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("{error}"),
            }
        };
        assert!(!service.is_running(binding));
        assert!(host.0.lock().unwrap().is_empty());
        if mode == "plain" {
            assert_eq!(
                outcome,
                ActionOutcome::Succeeded {
                    output: json!({"value":3})
                }
            );
        } else if matches!(mode, "throw-null" | "throw-undefined") {
            let ActionOutcome::Failed { error } = outcome else {
                panic!("应交付序列化失败而非宿主失联：{outcome:?}")
            };
            assert_eq!(error.code, "invalid_output");
            let value = if mode == "throw-null" {
                "null"
            } else {
                "undefined"
            };
            assert_eq!(
                error.message,
                format!("动作结果必须可序列化为 JSON。\n返回值序列化：{value}")
            );
            assert!(error.effect_state.is_none());
            assert!(error.method.is_none());
        } else {
            let ActionOutcome::Failed { error } = outcome else {
                panic!("应得到序列化失败：{outcome:?}")
            };
            assert!(error.message.contains(SENTINEL), "{}", error.message);
            assert!(
                error.message.contains("serialize.mjs:4"),
                "必须保留实际相对模块第4行：{}",
                error.message
            );
            match engine {
                Engine::Node => {
                    assert_eq!(error.code, "invalid_output");
                    assert!(error
                        .message
                        .starts_with("动作结果必须可序列化为 JSON。\n返回值序列化："));
                    assert!(error.message.encode_utf16().count() <= 4096);
                    assert!(error.effect_state.is_none());
                    assert!(error.method.is_none());
                }
                // Script现有引擎诊断已保留toJSON的源码帧；不改其错误代码或构造新字段。
                Engine::Script => assert_eq!(error.code, "script_failed"),
            }
        }
    }
}

#[test]
#[ignore = "正常Node，仅运行本文件生成的无副作用脚本"]
fn node_result_serialization_keeps_real_helper_location() {
    run(Engine::Node);
}
#[test]
#[ignore = "由Root分别指定正常QuickJS和Boa宿主；仅运行合成脚本"]
fn script_result_serialization_keeps_real_helper_location() {
    run(Engine::Script);
}
