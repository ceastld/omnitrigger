//! 真实 Node/QuickJS、ActionService 与回环 RPC；仅操作新建合成包和内存状态。
use omnitrigger_actions::{
    state::ActionStateStore, ActionCall, ActionError, ActionOutcome, ActionRuntime, ActionService,
    CancellationToken, InvocationContext, NodeRuntime, PackageCatalog, RuntimeRegistry,
    ScriptHostRuntime,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

fn registry() -> RuntimeRegistry {
    let mut registry = RuntimeRegistry::new();
    registry
        .register(
            "node",
            Arc::new(NodeRuntime::new(PathBuf::from(
                std::env::var_os("OMNITRIGGER_TEST_NODE").expect("显式 Node 路径"),
            ))),
        )
        .unwrap();
    registry
        .register(
            "script",
            Arc::new(
                ScriptHostRuntime::new(PathBuf::from(
                    std::env::var_os("OMNITRIGGER_TEST_SCRIPT_HOST").expect("显式 QuickJS 路径"),
                ))
                .with_required_capabilities(vec!["state.read_cas.v1".into()]),
            ),
        )
        .unwrap();
    registry
}

fn fixture(name: &str) -> PathBuf {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.tmp");
    for ancestor in base.ancestors().filter(|p| p.exists()) {
        let metadata = fs::symlink_metadata(ancestor).unwrap();
        assert!(!metadata.file_type().is_symlink());
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            assert_eq!(metadata.file_attributes() & 0x441400, 0);
        }
    }
    fs::create_dir_all(&base).unwrap();
    let root = base.join(format!("state-real-{}-{name}", std::process::id()));
    fs::create_dir(&root).unwrap();
    for package in ["state-one", "state-two"] {
        let path = root.join(package);
        fs::create_dir(&path).unwrap();
        fs::write(path.join("node.mjs"), "export default async function main(ctx){const x=ctx.input;if(x.op==='read')return await ctx.state.read(x.area,x.key);if(x.op==='unawaited'){ctx.state.read(x.area,x.key);return 'returned';}if(x.op==='unawaitedCas'){ctx.state.compareAndSwap(x.area,x.key,x.revision,x.next);return 'returned';}return await ctx.state.compareAndSwap(x.area,x.key,x.revision,x.next);}\n").unwrap();
        fs::write(path.join("script.mjs"), "export function main(ctx){const x=ctx.input;if(x.op==='read')return ctx.state.read(x.area,x.key);return ctx.state.compareAndSwap(x.area,x.key,x.revision,x.next);}\n").unwrap();
    }
    root
}

fn catalog(root: &std::path::Path, runtime: &str, version: u32) -> PackageCatalog {
    for package in ["state-one", "state-two"] {
        fs::write(root.join(package).join("action.json"),serde_json::to_vec(&json!({
            "schema_version":1,"id":package,"name":"合成状态包","version":"1.0.0",
            "actions":(["work","other"].map(|id|json!({"id":id,"title":"合成状态调用","version":version,"runtime":runtime,"entry":format!("{runtime}.mjs"),"single_instance":false})))
        })).unwrap()).unwrap();
    }
    let catalog = PackageCatalog::load(root);
    assert!(catalog.diagnostics.is_empty(), "{:?}", catalog.diagnostics);
    catalog
}
fn call(package: &str, action: &str, version: u32, input: Value) -> ActionCall {
    ActionCall {
        package_id: package.into(),
        action_id: action.into(),
        action_version: version,
        input,
    }
}
fn outputs(service: &ActionService, count: usize) -> Vec<Result<Value, ActionError>> {
    let end = Instant::now() + Duration::from_secs(90);
    let mut results = Vec::new();
    while results.len() < count {
        if let Ok(event) = service.try_recv() {
            match event.outcome {
                ActionOutcome::Started => {}
                ActionOutcome::Succeeded { output } => results.push(Ok(output)),
                ActionOutcome::Failed { error } => results.push(Err(error)),
                ActionOutcome::Cancelled => panic!("非预期取消"),
            }
        }
        assert!(Instant::now() < end, "真实运行时未给出终态");
        thread::sleep(Duration::from_millis(1));
    }
    results
}
fn one(
    service: &ActionService,
    binding: u64,
    package: &str,
    action: &str,
    version: u32,
    input: Value,
) -> Result<Value, ActionError> {
    service
        .submit(binding, call(package, action, version, input))
        .unwrap();
    outputs(service, 1).pop().unwrap()
}
fn read(service: &ActionService, binding: u64, package: &str, action: &str, version: u32) -> Value {
    one(
        service,
        binding,
        package,
        action,
        version,
        json!({"op":"read","area":"session","key":"clock"}),
    )
    .unwrap()
}

fn shared(first: &str, second: &str) {
    let root = fixture(first);
    let store = Arc::new(ActionStateStore::new(None).unwrap());
    let mut service =
        ActionService::new(catalog(&root, first, 1), registry()).with_state_store(store);
    let missing = read(&service, 1, "state-one", "work", 1);
    assert_eq!(missing["entry"], json!({"present":false}));
    let committed=one(&service,2,"state-one","work",1,json!({"area":"session","key":"clock","revision":missing["revision"],"next":{"present":true,"value":null}})).unwrap();
    assert_eq!(committed["status"], "committed");
    assert_eq!(
        committed["current"]["entry"],
        json!({"present":true,"value":null})
    );
    let stale=one(&service,3,"state-one","work",1,json!({"area":"session","key":"clock","revision":missing["revision"],"next":{"present":true,"value":99}})).unwrap();
    assert_eq!(
        stale,
        json!({"status":"conflict","current":committed["current"]})
    );
    service.reload(catalog(&root, second, 2), registry());
    assert_eq!(
        read(&service, 4, "state-one", "work", 2),
        committed["current"]
    );
    assert_eq!(
        read(&service, 5, "state-one", "other", 2)["entry"],
        json!({"present":false})
    );
    assert_eq!(
        read(&service, 6, "state-two", "work", 2)["entry"],
        json!({"present":false})
    );
    let zero=one(&service,7,"state-one","work",2,json!({"area":"session","key":"clock","revision":committed["current"]["revision"],"next":{"present":true,"value":0}})).unwrap();
    assert_eq!(zero["status"], "committed");
    assert_eq!(zero["current"]["entry"], json!({"present":true,"value":0}));
    let deleted=one(&service,8,"state-one","work",2,json!({"area":"session","key":"clock","revision":zero["current"]["revision"],"next":{"present":false}})).unwrap();
    assert_eq!(deleted["status"], "committed");
    assert_eq!(deleted["current"]["entry"], json!({"present":false}));
    assert_ne!(deleted["current"]["revision"], missing["revision"]);
    let no_durable = one(
        &service,
        9,
        "state-one",
        "work",
        2,
        json!({"op":"read","area":"durable","key":"clock"}),
    )
    .unwrap_err();
    assert_eq!(no_durable.code, "state_unavailable");
    assert_eq!(no_durable.effect_state.as_deref(), Some("not_started"));
    // 原始运行时元数据不能伪造服务作用域。
    let request = call(
        "state-one",
        "work",
        2,
        json!({"op":"read","area":"session","key":"clock"}),
    );
    let resolved = service.catalog().resolve(&request).unwrap();
    let runtime: Arc<dyn ActionRuntime> = if second == "node" {
        Arc::new(NodeRuntime::new(PathBuf::from(
            std::env::var_os("OMNITRIGGER_TEST_NODE").unwrap(),
        )))
    } else {
        Arc::new(ScriptHostRuntime::new(PathBuf::from(
            std::env::var_os("OMNITRIGGER_TEST_SCRIPT_HOST").unwrap(),
        )))
    };
    let error = runtime
        .execute(
            &resolved,
            &InvocationContext {
                invocation_id: 999,
                binding_id: 555,
            },
            &CancellationToken::new(),
        )
        .unwrap_err();
    assert_eq!(error.code, "state_unavailable");
    assert_eq!(error.method.as_deref(), Some("state.read"));
    assert_eq!(error.effect_state.as_deref(), Some("not_started"));
    let no_store = ActionService::new(catalog(&root, second, 2), registry());
    let error = one(
        &no_store,
        10,
        "state-one",
        "work",
        2,
        json!({"op":"read","area":"session","key":"clock"}),
    )
    .unwrap_err();
    assert_eq!(error.code, "state_unavailable");
    assert_eq!(error.method.as_deref(), Some("state.read"));
    assert_eq!(error.effect_state.as_deref(), Some("not_started"));
}

#[test]
#[ignore = "需显式指定真实Node和新QuickJS宿主"]
fn node_then_script_share_state_after_reload_and_preserve_scope() {
    shared("node", "script")
}
#[test]
#[ignore = "需显式指定真实Node和新QuickJS宿主"]
fn script_then_node_share_state_after_reload_and_preserve_scope() {
    shared("script", "node")
}

fn race(runtime: &str) {
    let root = fixture(&format!("race-{runtime}"));
    let service = ActionService::new(catalog(&root, runtime, 1), registry())
        .with_state_store(Arc::new(ActionStateStore::new(None).unwrap()));
    let before = read(&service, 1, "state-one", "work", 1);
    for (binding, value) in [(2, 111), (3, 222)] {
        service.submit(binding,call("state-one","work",1,json!({"area":"session","key":"clock","revision":before["revision"],"next":{"present":true,"value":value}}))).unwrap();
    }
    let out = outputs(&service, 2)
        .into_iter()
        .map(Result::unwrap)
        .collect::<Vec<_>>();
    let mut status = out
        .iter()
        .map(|x| x["status"].as_str().unwrap())
        .collect::<Vec<_>>();
    status.sort();
    assert_eq!(status, ["committed", "conflict"]);
    let win = &out.iter().find(|x| x["status"] == "committed").unwrap()["current"];
    assert_eq!(read(&service, 4, "state-one", "work", 1), *win);
    assert_eq!(
        out.iter().find(|x| x["status"] == "conflict").unwrap()["current"],
        *win
    );
}
#[test]
#[ignore = "需显式指定真实Node"]
fn node_concurrent_calls_have_one_cas_winner() {
    race("node")
}
#[test]
#[ignore = "需显式指定真实QuickJS"]
fn script_concurrent_calls_have_one_cas_winner() {
    race("script")
}

#[test]
#[ignore = "需显式指定真实Node"]
fn node_unawaited_state_read_is_explicitly_rejected() {
    let root = fixture("unawaited");
    let service = ActionService::new(catalog(&root, "node", 1), registry())
        .with_state_store(Arc::new(ActionStateStore::new(None).unwrap()));
    let error = one(
        &service,
        1,
        "state-one",
        "work",
        1,
        json!({"op":"unawaited","area":"session","key":"clock"}),
    )
    .unwrap_err();
    assert_eq!(error.code, "pending_state_call");
    assert_eq!(error.effect_state.as_deref(), Some("not_started"));
    assert_eq!(error.method.as_deref(), Some("state.read"));
}

#[test]
#[ignore = "需显式指定真实Node"]
fn node_unawaited_cas_retains_unknown_effect_and_does_not_retry() {
    let root = fixture("unawaited-cas");
    let service = ActionService::new(catalog(&root, "node", 1), registry())
        .with_state_store(Arc::new(ActionStateStore::new(None).unwrap()));
    let before = read(&service, 1, "state-one", "work", 1);
    let error = one(
        &service,
        2,
        "state-one",
        "work",
        1,
        json!({
            "op":"unawaitedCas","area":"session","key":"clock",
            "revision":before["revision"],"next":{"present":true,"value":731}
        }),
    )
    .unwrap_err();
    assert_eq!(error.code, "pending_state_call");
    assert_eq!(error.effect_state.as_deref(), Some("unknown"));
    assert_eq!(error.method.as_deref(), Some("state.compare_and_swap"));
    // 发送后取消可能已经提交；未知回执不能伪造回滚或自动补发。
    let after = read(&service, 3, "state-one", "work", 1);
    assert!(
        after["entry"] == json!({"present":false})
            || after["entry"] == json!({"present":true,"value":731})
    );
    if after["entry"]["present"] == false {
        assert_eq!(after, before);
    } else {
        assert_ne!(after["revision"], before["revision"]);
    }
}
