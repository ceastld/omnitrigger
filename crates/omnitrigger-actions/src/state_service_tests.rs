//! 真实ActionService、真实作用域宿主及CAS；受控运行时不启动Node/Script进程或Native桥。
use super::*;
use crate::{catalog::test_catalog, state::ActionStateStore, ActionDefinition, CatalogAction};
use serde_json::json;
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};

struct Controlled {
    read: mpsc::Sender<mpsc::Sender<()>>,
}
impl ActionRuntime for Controlled {
    fn execute(
        &self,
        _: &ResolvedAction,
        _: &InvocationContext,
        _: &CancellationToken,
    ) -> Result<Value, ActionError> {
        panic!("测试必须沿服务作用域宿主")
    }
    fn execute_with_host(
        &self,
        action: &ResolvedAction,
        _: &InvocationContext,
        cancellation: &CancellationToken,
        host: Option<&dyn NativeHost>,
    ) -> Result<Value, ActionError> {
        let host = host.unwrap();
        let call = |method: &str, params: Value| {
            host.call_with_deadline(
                method,
                params,
                cancellation,
                Instant::now() + Duration::from_secs(5),
            )
        };
        if action.call.input["race"] == true {
            let old = call("state.read", json!({"area":"session","key":"choice"}))?;
            let (go, wait) = mpsc::channel();
            self.read.send(go).unwrap();
            wait.recv_timeout(Duration::from_secs(5))
                .map_err(|_| ActionError::new("test_timeout", "并发门未放行"))?;
            call(
                "state.compare_and_swap",
                json!({"area":"session","key":"choice","expectedRevision":old["revision"],"next":{"present":true,"value":action.call.input["value"]}}),
            )
        } else {
            call(
                action.call.input["method"].as_str().unwrap(),
                action.call.input["params"].clone(),
            )
        }
    }
}
fn catalog(runtime: &str, version: u32) -> PackageCatalog {
    test_catalog(
        ["test", "other-package"]
            .into_iter()
            .flat_map(|package| {
                ["work", "other"].into_iter().map(move |id| CatalogAction {
                    package_id: package.into(),
                    package_name: "合成包".into(),
                    definition: ActionDefinition {
                        id: id.into(),
                        title: "合成动作".into(),
                        version,
                        runtime: runtime.into(),
                        entry: "not-opened.mjs".into(),
                        single_instance: false,
                        allow_binding_reentry: false,
                    },
                })
            })
            .collect(),
    )
}
fn registry(runtime: Arc<Controlled>) -> RuntimeRegistry {
    let mut r = RuntimeRegistry::new();
    r.register("controlled-node", runtime.clone()).unwrap();
    r.register("controlled-script", runtime).unwrap();
    r
}
fn request(package: &str, action: &str, version: u32, input: Value) -> ActionCall {
    ActionCall {
        package_id: package.into(),
        action_id: action.into(),
        action_version: version,
        input,
    }
}
fn results(service: &ActionService, count: usize) -> Vec<Result<Value, ActionError>> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut out = Vec::new();
    while out.len() < count {
        assert!(Instant::now() < deadline, "受控服务没有及时给出终态");
        match service.try_recv() {
            Ok(ActionEvent {
                outcome: ActionOutcome::Succeeded { output },
                ..
            }) => out.push(Ok(output)),
            Ok(ActionEvent {
                outcome: ActionOutcome::Failed { error },
                ..
            }) => out.push(Err(error)),
            Ok(ActionEvent {
                outcome: ActionOutcome::Cancelled,
                ..
            }) => panic!("非预期取消"),
            Ok(_) => {}
            Err(mpsc::TryRecvError::Empty) => std::thread::yield_now(),
            Err(e) => panic!("{e}"),
        }
    }
    out
}
fn one(
    service: &ActionService,
    binding: u64,
    package: &str,
    action: &str,
    version: u32,
    method: &str,
    params: Value,
) -> Result<Value, ActionError> {
    service
        .submit(
            binding,
            request(
                package,
                action,
                version,
                json!({"method":method,"params":params}),
            ),
        )
        .unwrap();
    results(service, 1).pop().unwrap()
}
fn controlled() -> (Arc<Controlled>, mpsc::Receiver<mpsc::Sender<()>>) {
    let (tx, rx) = mpsc::channel();
    (Arc::new(Controlled { read: tx }), rx)
}

#[test]
fn two_real_service_invocations_have_one_cas_winner_and_one_conflict() {
    let (runtime, read) = controlled();
    let service = ActionService::new(catalog("controlled-node", 1), registry(runtime))
        .with_state_store(Arc::new(ActionStateStore::new(None).unwrap()));
    // 先建立作用域，随后两个实际工作线程都必须先读到相同revision。
    one(
        &service,
        90,
        "test",
        "work",
        1,
        "state.read",
        json!({"area":"session","key":"choice"}),
    )
    .unwrap();
    for (id, value) in [(1, "a"), (2, "b")] {
        service
            .submit(
                id,
                request("test", "work", 1, json!({"race":true,"value":value})),
            )
            .unwrap();
    }
    let a = read.recv_timeout(Duration::from_secs(5)).unwrap();
    let b = read.recv_timeout(Duration::from_secs(5)).unwrap();
    a.send(()).unwrap();
    b.send(()).unwrap();
    let r = results(&service, 2);
    assert_eq!(
        r.iter()
            .filter(|v| v.as_ref().is_ok_and(|v| v["status"] == "committed"))
            .count(),
        1
    );
    assert_eq!(
        r.iter()
            .filter(|v| v.as_ref().is_ok_and(|v| v["status"] == "conflict"))
            .count(),
        1
    );
}

#[test]
fn store_is_shared_across_bindings_reload_and_controlled_runtime_adapters_but_not_other_actions() {
    let (runtime, _) = controlled();
    let mut service = ActionService::new(catalog("controlled-node", 1), registry(runtime.clone()))
        .with_state_store(Arc::new(ActionStateStore::new(None).unwrap()));
    let old = one(
        &service,
        1,
        "test",
        "work",
        1,
        "state.read",
        json!({"area":"session","key":"choice"}),
    )
    .unwrap();
    one(&service,2,"test","work",1,"state.compare_and_swap",json!({"area":"session","key":"choice","expectedRevision":old["revision"],"next":{"present":true,"value":"kept"}})).unwrap();
    for (package, action) in [("test", "other"), ("other-package", "work")] {
        let r = one(
            &service,
            3,
            package,
            action,
            1,
            "state.read",
            json!({"area":"session","key":"choice"}),
        )
        .unwrap();
        assert_eq!(r["entry"]["present"], false);
        assert_ne!(r["revision"], old["revision"]);
    }
    service.reload(catalog("controlled-script", 2), registry(runtime));
    let r = one(
        &service,
        4,
        "test",
        "work",
        2,
        "state.read",
        json!({"area":"session","key":"choice"}),
    )
    .unwrap();
    assert_eq!(r["entry"]["value"], "kept");
    assert_eq!(
        one(
            &service,
            5,
            "test",
            "work",
            2,
            "state.read",
            json!({"area":"session","key":"choice","actionId":"other"})
        )
        .unwrap_err()
        .code,
        "invalid_input"
    );
}

#[test]
fn default_service_without_store_is_explicitly_unavailable() {
    let (runtime, _) = controlled();
    let service = ActionService::new(catalog("controlled-node", 1), registry(runtime));
    let e = one(
        &service,
        1,
        "test",
        "work",
        1,
        "state.read",
        json!({"area":"session","key":"choice"}),
    )
    .unwrap_err();
    assert_eq!(e.code, "state_unavailable");
    assert_eq!(e.effect_state.as_deref(), Some("not_started"));
}

#[test]
fn failed_durable_commit_through_real_service_does_not_change_snapshot() {
    use crate::state::{
        ActionStateScope, DurableStateSnapshot, DurableStateStorage, StateStorageError,
    };
    struct Reject;
    impl DurableStateStorage for Reject {
        fn load(&self, _: &ActionStateScope) -> Result<DurableStateSnapshot, StateStorageError> {
            Ok(DurableStateSnapshot {
                values: Default::default(),
                version: "missing".into(),
            })
        }
        fn commit(
            &self,
            _: &ActionStateScope,
            _: &str,
            _: &std::collections::BTreeMap<String, Value>,
            before: &dyn Fn() -> Result<(), StateStorageError>,
        ) -> Result<String, StateStorageError> {
            before()?;
            Err(StateStorageError::Io)
        }
    }
    let (runtime, _) = controlled();
    let service = ActionService::new(catalog("controlled-node", 1), registry(runtime))
        .with_state_store(Arc::new(
            ActionStateStore::new(Some(Arc::new(Reject))).unwrap(),
        ));
    let before = one(
        &service,
        1,
        "test",
        "work",
        1,
        "state.read",
        json!({"area":"durable","key":"choice"}),
    )
    .unwrap();
    let error=one(&service,2,"test","work",1,"state.compare_and_swap",json!({"area":"durable","key":"choice","expectedRevision":before["revision"],"next":{"present":true,"value":7}})).unwrap_err();
    assert_eq!(error.code, "state_io");
    assert_eq!(error.effect_state.as_deref(), Some("not_started"));
    assert_eq!(
        one(
            &service,
            3,
            "test",
            "work",
            1,
            "state.read",
            json!({"area":"durable","key":"choice"})
        )
        .unwrap(),
        before
    );
}
