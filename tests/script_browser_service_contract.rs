// 包含于script_browser::tests子模块；只使用现有Fixture/broker和真实Service/Runtime。
use super::Fixture;
use crate::script_browser::BrowserHost;
use omnitrigger_actions::{
    ActionCall, ActionError, ActionOutcome, ActionService, CancellationToken, NativeHost,
    NodeRuntime, PackageCatalog, RuntimeRegistry, ScriptHostRuntime,
};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, atomic::Ordering},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Script,
    Node,
}
impl Mode {
    fn selected() -> Self {
        match std::env::var("OMNITRIGGER_FALSE_CONTEXT_RUNTIME") {
            Err(std::env::VarError::NotPresent) => Self::Script,
            Ok(value) if value == "script" => Self::Script,
            Ok(value) if value == "node" => Self::Node,
            _ => panic!("只接受script/node"),
        }
    }
    fn runtime(self) -> &'static str {
        if self == Self::Script {
            "script"
        } else {
            "node"
        }
    }
    fn wait(self) -> Duration {
        // 正常 Runtime 保留60秒；外层观察须允许冷启动及宿主自然收尾。
        Duration::from_secs(64)
    }
    fn program(self) -> PathBuf {
        let name = if self == Self::Script {
            "OMNITRIGGER_TEST_SCRIPT_HOST"
        } else {
            "OMNITRIGGER_TEST_NODE"
        };
        let path = PathBuf::from(std::env::var_os(name).expect("显式匹配的正常Runtime绝对路径"));
        local(&path);
        assert!(path.is_file());
        path
    }
}
fn local(path: &Path) {
    use std::os::windows::fs::MetadataExt;
    assert!(path.is_absolute());
    for item in path.ancestors() {
        let info = fs::symlink_metadata(item).unwrap();
        let name = item
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase();
        assert!(!name.starts_with("onedrive") && !name.starts_with("icloud") && name != "dropbox");
        assert!(!info.file_type().is_symlink() && info.file_attributes() & 0x441400 == 0);
    }
}

struct Package {
    actions: PathBuf,
    package: PathBuf,
    sdk: Option<PathBuf>,
    files: Vec<String>,
}
impl Package {
    fn new(fixture: &Fixture, mode: Mode, body: &str) -> Self {
        let manifest=serde_json::to_vec(&json!({"schema_version":1,"id":"test.browser-service-contract","name":"Browser Service合同","version":"0.0.0","actions":[{"id":"invoke","title":"API合同","version":1,"runtime":mode.runtime(),"entry":"main.mjs","single_instance":true}]})).unwrap();
        let author = format!(
            "export {}function main(ctx) {{\n{}\n}}\n{}",
            if mode == Mode::Node { "async " } else { "" },
            body,
            if mode == Mode::Node {
                "export default main;\n"
            } else {
                ""
            }
        );
        Self::with_files(
            fixture,
            mode,
            &[("action.json", &manifest), ("main.mjs", author.as_bytes())],
        )
    }
    fn with_files(fixture: &Fixture, mode: Mode, files: &[(&str, &[u8])]) -> Self {
        local(&fixture.root);
        let actions = fixture.root.join("actions");
        let package = actions.join("service-api");
        fs::create_dir(&actions).unwrap();
        fs::create_dir(&package).unwrap();
        for (name, bytes) in files {
            assert!(Path::new(name).components().count() == 1 && *name != "." && *name != "..");
            fs::write(package.join(name), bytes).unwrap();
        }
        let sdk = if mode == Mode::Node {
            // 显式提供与连接器版本匹配的SDK；不查找安装目录或替换connect实现。
            let source = PathBuf::from(
                std::env::var_os("OMNITRIGGER_TEST_BROWSER_SDK")
                    .expect("显式提供与连接器版本匹配的SDK源码目录"),
            );
            local(&source);
            assert!(!source.starts_with(&fixture.root));
            let sdk = fixture.runtime.join("sdk-js");
            fs::create_dir(&sdk).unwrap();
            for name in ["index.mjs", "protocol.mjs"] {
                let from = source.join(name);
                local(&from);
                let bytes = fs::read(&from).unwrap();
                assert!(bytes.len() <= 256 * 1024);
                fs::write(sdk.join(name), bytes).unwrap();
            }
            Some(sdk)
        } else {
            None
        };
        Self {
            actions,
            package,
            sdk,
            files: files.iter().map(|(name, _)| (*name).to_owned()).collect(),
        }
    }
}
impl Drop for Package {
    fn drop(&mut self) {
        if thread::panicking() {
            return;
        }
        for name in &self.files {
            let _ = fs::remove_file(self.package.join(name));
        }
        let _ = fs::remove_dir(&self.package);
        let _ = fs::remove_dir(&self.actions);
        if let Some(sdk) = &self.sdk {
            for name in ["index.mjs", "protocol.mjs"] {
                let _ = fs::remove_file(sdk.join(name));
            }
            let _ = fs::remove_dir(sdk);
        }
    }
}
struct Host {
    browser: Arc<BrowserHost>,
    missing: String,
    calls: Mutex<Vec<(String, Value)>>,
}
impl NativeHost for Host {
    fn native_context_available(&self) -> bool {
        false
    }
    fn call(
        &self,
        method: &str,
        params: Value,
        cancel: &CancellationToken,
    ) -> Result<Value, ActionError> {
        self.call_with_deadline(
            method,
            params,
            cancel,
            Instant::now() + Duration::from_secs(60),
        )
    }
    fn call_with_deadline(
        &self,
        method: &str,
        params: Value,
        cancel: &CancellationToken,
        deadline: Instant,
    ) -> Result<Value, ActionError> {
        assert!(!cancel.is_cancelled() && deadline > Instant::now());
        self.calls
            .lock()
            .unwrap()
            .push((method.into(), params.clone()));
        if matches!(method, "browser.call" | "browser.profiles") {
            return self.browser.call(method, params, cancel, deadline);
        }
        if method == "fs.stat" {
            assert_eq!(params, json!({"path":self.missing}));
            return Ok(json!({"kind":"missing"}));
        }
        if matches!(method, "system.open" | "system.open_file_in_parent") {
            assert_eq!(
                params,
                json!({"target":if method=="system.open" {"https://synthetic.invalid/service"} else {&self.missing}})
            );
            return Ok(json!({"submitted":true}));
        }
        panic!("合同禁止调用Window/Clipboard/其它Native：{method}")
    }
}
fn invoke(fixture: &Fixture, mode: Mode, body: &str) -> (Result<Value, ActionError>, Arc<Host>) {
    let package = Package::new(fixture, mode, body);
    invoke_prepared(
        fixture,
        mode,
        package,
        ActionCall {
            package_id: "test.browser-service-contract".into(),
            action_id: "invoke".into(),
            action_version: 1,
            input: json!({}),
        },
    )
}
fn invoke_prepared(
    fixture: &Fixture,
    mode: Mode,
    package: Package,
    call: ActionCall,
) -> (Result<Value, ActionError>, Arc<Host>) {
    let mut registry = RuntimeRegistry::new();
    let program = mode.program();
    assert!(!program.starts_with(&fixture.root));
    match mode {
        Mode::Script => registry
            .register("script", Arc::new(ScriptHostRuntime::new(program)))
            .unwrap(),
        Mode::Node => registry
            .register(
                "node",
                Arc::new(NodeRuntime::new(program).with_browser_connector(fixture.runtime.clone())),
            )
            .unwrap(),
    }
    let host = Arc::new(Host {
        browser: fixture.host.clone(),
        missing: fixture.root.join("never.txt").to_str().unwrap().into(),
        calls: Mutex::new(vec![]),
    });
    assert!(!fixture.root.join("never.txt").exists());
    let service = ActionService::new(PackageCatalog::load(&package.actions), registry);
    let native: Arc<dyn NativeHost> = host.clone();
    let id = service.submit_with_host(29, call, Some(native)).unwrap();
    let began = Instant::now();
    let until = began + mode.wait();
    let mut started = 0;
    let result = loop {
        assert!(
            Instant::now() < until,
            "Service等待超时：mode={mode:?} elapsed={:?}",
            began.elapsed()
        );
        match service.try_recv() {
            Ok(event) => {
                assert_eq!(event.invocation_id, id);
                match event.outcome {
                    ActionOutcome::Started => started += 1,
                    ActionOutcome::Succeeded { output } => break Ok(output),
                    ActionOutcome::Failed { error } => break Err(error),
                    ActionOutcome::Cancelled => panic!("未请求取消"),
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => thread::sleep(Duration::from_millis(2)),
            Err(error) => panic!("Service事件通道：{error}"),
        }
    };
    assert_eq!(started, 1);
    assert!(!host.native_context_available());
    eprintln!(
        "真实Service终态：mode={mode:?} elapsed={:?} ok={}",
        began.elapsed(),
        result.is_ok()
    );
    drop(service);
    drop(package);
    (result, host)
}

#[test]
#[ignore = "真实正常Runtime与自有loopback协议；需显式真实SDK输入，不触达Chrome"]
fn service_false_original_context_browser_contract() {
    let mode = Mode::selected();
    let wait = if mode == Mode::Node { "await " } else { "" };
    for case in [
        "call",
        "profiles",
        "undefined",
        "null",
        "permission",
        "reported-unknown",
    ] {
        let mut fixture = Fixture::with_lifetime(mode.wait() + Duration::from_secs(8));
        let body = match case {
            "profiles" => format!("return {wait}ctx.browser.profiles({{timeoutMs:1000}});"),
            "undefined" | "null" => format!(
                "const value={wait}ctx.browser.call('{}',[7],{{profile:'work',timeoutMs:1000}});return {{isUndefined:value===undefined,isNull:value===null}};",
                if case == "undefined" {
                    "tabs.update"
                } else {
                    "tabs.get"
                }
            ),
            "permission" => format!(
                "return {wait}ctx.browser.call('sessions.restore',['synthetic-closed'],{{profile:'work',timeoutMs:1000}});"
            ),
            "reported-unknown" => format!(
                "return {wait}ctx.browser.call('tabs.query',['reported-unknown'],{{profile:'work',timeoutMs:1000}});"
            ),
            _ => format!(
                "return {wait}ctx.browser.call('tabs.query',[],{{profile:'work',timeoutMs:1000}});"
            ),
        };
        let (result, host) = invoke(&fixture, mode, &body);
        match case {
            "profiles" => {
                let mut ids: Vec<_> = result
                    .unwrap()
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v["browser_instance_id"].as_str().unwrap().to_owned())
                    .collect();
                ids.sort();
                assert_eq!(ids, ["other", "work"]);
            }
            "undefined" => assert_eq!(result.unwrap(), json!({"isUndefined":true,"isNull":false})),
            "null" => assert_eq!(result.unwrap(), json!({"isUndefined":false,"isNull":true})),
            "permission" | "reported-unknown" => {
                let error = result.unwrap_err();
                assert_eq!(
                    error.code,
                    if case == "permission" {
                        "permission_required"
                    } else {
                        "result_unknown"
                    }
                );
                assert_eq!(
                    error.effect_state.as_deref(),
                    Some(if case == "permission" {
                        "not_started"
                    } else {
                        "unknown"
                    })
                );
                assert_eq!(
                    error.method.as_deref(),
                    Some(if case == "permission" {
                        "sessions.restore"
                    } else {
                        "tabs.query"
                    })
                );
            }
            _ => assert_eq!(result.unwrap(), json!({"done":true})),
        }
        assert_eq!(
            fixture.calls.load(Ordering::SeqCst),
            usize::from(case != "profiles")
        );
        let requests = fixture.requests.lock().unwrap();
        assert_eq!(requests.len(), usize::from(case != "profiles"));
        if case != "profiles" {
            let (method, args) = match case {
                "undefined" => ("tabs.update", json!([7])),
                "null" => ("tabs.get", json!([7])),
                "permission" => ("sessions.restore", json!(["synthetic-closed"])),
                "reported-unknown" => ("tabs.query", json!(["reported-unknown"])),
                _ => ("tabs.query", json!([])),
            };
            assert_eq!(requests[0]["method"], method);
            assert_eq!(requests[0]["args"], args);
            assert_eq!(requests[0]["browser_instance_id"], "work");
            assert_eq!(requests[0]["session_id"], "s-work");
            assert_eq!(requests[0]["timeout_ms"], 1000);
        }
        drop(requests);
        let calls = host.calls.lock().unwrap();
        assert_eq!(calls.len(), usize::from(mode == Mode::Script));
        if mode == Mode::Script {
            assert_eq!(
                calls[0].0,
                if case == "profiles" {
                    "browser.profiles"
                } else {
                    "browser.call"
                }
            );
        }
        drop(calls);
        fixture.finish();
    }
}

#[test]
#[ignore = "真实正常Runtime的原额度边界；Native仅合成stat/open回执，不查询或打开"]
fn service_open_stat_original_budget_contract() {
    let mode = Mode::selected();
    let wait = if mode == Mode::Node { "await " } else { "" };
    for method in ["system.open", "system.open_file_in_parent", "fs.stat"] {
        for last_allowed in [true, false] {
            let mut fixture = Fixture::with_lifetime(mode.wait() + Duration::from_secs(8));
            let path =
                serde_json::to_string(fixture.root.join("never.txt").to_str().unwrap()).unwrap();
            let cap = if mode == Mode::Script { 32 } else { 128 };
            let prefix = cap - usize::from(last_allowed);
            let spend = if mode == Mode::Script {
                "ctx.delay(0);".into()
            } else {
                format!("await ctx.fs.stat({path});")
            };
            let call = match method {
                "system.open" => format!("{wait}ctx.open('https://synthetic.invalid/service')"),
                "system.open_file_in_parent" => format!("{wait}ctx.openFileInParent({path})"),
                _ => format!("{wait}ctx.fs.stat({path})"),
            };
            let body = format!("for(let i=0;i<{prefix};i++){{{spend}}}return {call};");
            let (result, host) = invoke(&fixture, mode, &body);
            if last_allowed {
                assert_eq!(
                    result.unwrap(),
                    if method == "fs.stat" {
                        json!({"kind":"missing"})
                    } else if mode == Mode::Script {
                        Value::Null
                    } else {
                        json!({"submitted":true})
                    }
                );
            } else {
                let error = result.unwrap_err();
                assert_eq!(
                    error.code,
                    if method == "fs.stat" {
                        "fs_error"
                    } else if mode == Mode::Script {
                        "desktop_call_limit"
                    } else {
                        "native_call_limit"
                    }
                );
                assert_eq!(error.method.as_deref(), Some(method));
                assert_eq!(error.effect_state.as_deref(), Some("not_started"));
            }
            let calls = host.calls.lock().unwrap();
            assert_eq!(
                calls.len(),
                if mode == Mode::Script {
                    usize::from(last_allowed)
                } else {
                    128
                }
            );
            if !last_allowed {
                assert!(calls.iter().all(|(method, _)| method == "fs.stat"));
            }
            assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
            drop(calls);
            fixture.finish();
        }
    }
}

mod duplicate_author {
    include!("script_browser_duplicate_service_contract.rs");
}
