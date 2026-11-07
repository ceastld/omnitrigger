//! 真实运行时与回环RPC；UI由记录宿主代替，不启动窗口或输入设备。
use omnitrigger_actions::{
    action_hint::{ScopedActionHint, SCOPED_METHOD},
    ActionCall, ActionError, ActionOutcome, ActionService, CancellationToken, NativeHost,
    NodeRuntime, PackageCatalog, RuntimeRegistry, ScriptHostRuntime,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

struct RecordedUi(Mutex<Vec<ScopedActionHint>>);
impl NativeHost for RecordedUi {
    fn native_context_available(&self) -> bool {
        false
    }
    fn call(
        &self,
        method: &str,
        params: Value,
        _: &CancellationToken,
    ) -> Result<Value, ActionError> {
        assert_eq!(method, SCOPED_METHOD, "仅内部提示调用可以到达UI宿主");
        self.0
            .lock()
            .unwrap()
            .push(ScopedActionHint::parse(params)?);
        Ok(json!({"updated":true}))
    }
}
fn terminal(service: &ActionService) -> Value {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        if let Ok(event) = service.try_recv() {
            match event.outcome {
                ActionOutcome::Started => {}
                ActionOutcome::Succeeded { output } => return output,
                other => panic!("实际运行时失败：{other:?}"),
            }
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
}
fn check(runtime: &str) {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.tmp");
    for ancestor in base.ancestors().filter(|p| p.exists()) {
        let meta = fs::symlink_metadata(ancestor).unwrap();
        assert!(!meta.file_type().is_symlink());
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            assert_eq!(meta.file_attributes() & 0x441400, 0);
        }
    }
    fs::create_dir_all(&base).unwrap();
    let root = base.join(format!("hint-real-{}-{runtime}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let pkg = root.join("hint-example");
    fs::create_dir(&pkg).unwrap();
    let await_kw = if runtime == "node" { "await " } else { "" };
    let async_kw = if runtime == "node" { "async " } else { "" };
    let default_kw = if runtime == "node" { "default " } else { "" };
    let module=format!("export {default_kw}{async_kw}function main(ctx){{const x=ctx.input;try{{if(x.op==='desktop')return {await_kw}ctx.desktop.sendShortcut([65]);return {await_kw}ctx.ui.setActionHint(x.options);}}catch(e){{return {{code:e.code,method:e.method,effectState:e.effectState}};}}}}\n");
    fs::write(pkg.join("main.mjs"), module).unwrap();
    fs::write(pkg.join("action.json"),serde_json::to_vec(&json!({"schema_version":1,"id":"hint-example","name":"合成提示","version":"1.0.0","actions":[{"id":"work","title":"合成提示","version":9,"runtime":runtime,"entry":"main.mjs","single_instance":false}]})).unwrap()).unwrap();
    let mut registry = RuntimeRegistry::new();
    if runtime == "node" {
        registry
            .register(
                "node",
                Arc::new(NodeRuntime::new(PathBuf::from(
                    std::env::var_os("OMNITRIGGER_TEST_NODE").expect("显式Node"),
                ))),
            )
            .unwrap();
    } else {
        registry
            .register(
                "script",
                Arc::new(
                    ScriptHostRuntime::new(PathBuf::from(
                        std::env::var_os("OMNITRIGGER_TEST_SCRIPT_HOST").expect("显式Script宿主"),
                    ))
                    .with_required_capabilities(vec!["ui.action_hint.v1".into()]),
                ),
            )
            .unwrap();
    }
    let catalog = PackageCatalog::load(&root);
    assert!(catalog.diagnostics.is_empty(), "{:?}", catalog.diagnostics);
    let service = ActionService::new(catalog, registry);
    let ui = Arc::new(RecordedUi(Mutex::new(Vec::new())));
    let submit = |binding, input, host| {
        service
            .submit_with_host(
                binding,
                ActionCall {
                    package_id: "hint-example".into(),
                    action_id: "work".into(),
                    action_version: 9,
                    input,
                },
                host,
            )
            .unwrap()
    };
    let invocation = submit(
        731,
        json!({"options":{"text":"n10"}}),
        Some(ui.clone() as Arc<dyn NativeHost>),
    );
    assert_eq!(terminal(&service), json!({"updated":true}));
    {
        let updates = ui.0.lock().unwrap();
        assert_eq!(updates.len(), 1);
        let item = &updates[0];
        assert_eq!(item.text, "n10");
        assert_eq!(item.scope.binding_id, 731);
        assert_eq!(item.scope.invocation_id, invocation);
        assert_eq!(item.scope.package_id, "hint-example");
        assert_eq!(item.scope.action_id, "work");
        assert_eq!(item.scope.action_version, 9);
    }
    submit(
        732,
        json!({"options":{"text":"n10","scope":{"binding_id":1}}}),
        Some(ui.clone() as Arc<dyn NativeHost>),
    );
    assert_eq!(terminal(&service)["code"], "invalid_input");
    submit(
        733,
        json!({"op":"desktop"}),
        Some(ui.clone() as Arc<dyn NativeHost>),
    );
    let rejected = terminal(&service);
    assert_ne!(rejected["submitted"], true);
    assert!(rejected["code"].is_string(), "{rejected}");
    assert_eq!(ui.0.lock().unwrap().len(), 1, "作用域不能打开桌面能力");
    submit(734, json!({"options":{"text":"pre"}}), None);
    let unavailable = terminal(&service);
    assert_eq!(unavailable["code"], "ui_unavailable");
    assert_eq!(unavailable["method"], "ui.action_hint");
    assert_eq!(unavailable["effectState"], "not_started");
}
#[test]
#[ignore = "显式真实Node及合成UI记录宿主"]
fn node_action_hint_rpc_uses_trusted_scope_without_desktop_context() {
    check("node")
}
#[test]
#[ignore = "显式真实Script宿主及合成UI记录宿主"]
fn script_action_hint_rpc_uses_trusted_scope_without_desktop_context() {
    check("script")
}
