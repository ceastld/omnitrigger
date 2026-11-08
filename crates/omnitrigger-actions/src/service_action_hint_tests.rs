//! 只走内存目录与实际ActionService/InvocationScopedHost；不启动脚本、UI或平台。
use super::*;
use crate::{
    catalog::test_catalog, ActionCall, ActionDefinition, ActionEvent, ActionOutcome, ActionRuntime,
    ActionService, CatalogAction, InvocationContext, ResolvedAction, RuntimeRegistry,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc,
};
struct HintRuntime;
impl ActionRuntime for HintRuntime {
    fn execute(
        &self,
        _: &ResolvedAction,
        _: &InvocationContext,
        _: &CancellationToken,
    ) -> Result<Value, ActionError> {
        panic!("必须提供服务作用域宿主")
    }
    fn execute_with_host(
        &self,
        action: &ResolvedAction,
        context: &InvocationContext,
        cancellation: &CancellationToken,
        host: Option<&dyn NativeHost>,
    ) -> Result<Value, ActionError> {
        let host = host.unwrap();
        assert!(host.action_scope_available());
        assert!(!host.native_context_available());
        let mut options = json!({"text":"n10"});
        if let Some(placement) = action.call.input.get("placement") {
            options["placement"] = placement.clone();
        }
        let receipt = host.call("ui.action_hint", options, cancellation)?;
        assert_eq!(receipt["binding"], context.binding_id);
        assert_eq!(receipt["invocation"], context.invocation_id);
        Ok(receipt)
    }
}
struct UiOnlyHost;
impl NativeHost for UiOnlyHost {
    fn native_context_available(&self) -> bool {
        false
    }
    fn call(
        &self,
        method: &str,
        params: Value,
        _: &CancellationToken,
    ) -> Result<Value, ActionError> {
        assert_eq!(method, crate::action_hint::SCOPED_METHOD);
        let update = crate::action_hint::ScopedActionHint::parse(params)?;
        assert_eq!(update.scope.package_id, "hint-example");
        assert_eq!(update.scope.action_id, "work");
        assert_eq!(update.scope.action_version, 3);
        assert_eq!(update.text, "n10");
        Ok(
            json!({"updated":true,"binding":update.scope.binding_id,"invocation":update.scope.invocation_id}),
        )
    }
}
fn service() -> ActionService {
    let catalog = test_catalog(vec![CatalogAction {
        package_id: "hint-example".into(),
        package_name: "合成".into(),
        definition: ActionDefinition {
            id: "work".into(),
            title: "合成".into(),
            version: 3,
            runtime: "hint-test".into(),
            entry: "not-opened.mjs".into(),
            single_instance: false,
            allow_binding_reentry: false,
        },
    }]);
    let mut registry = RuntimeRegistry::new();
    registry
        .register("hint-test", Arc::new(HintRuntime))
        .unwrap();
    ActionService::new(catalog, registry)
}
fn call() -> ActionCall {
    ActionCall {
        package_id: "hint-example".into(),
        action_id: "work".into(),
        action_version: 3,
        input: json!({"not":"forwarded"}),
    }
}
fn terminal(service: &ActionService) -> ActionEvent {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match service.try_recv() {
            Ok(event) if event.outcome.is_terminal() => return event,
            Ok(_) | Err(mpsc::TryRecvError::Empty) => {}
            Err(error) => panic!("{error}"),
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn action_hint_actual_service_uses_retained_identity_and_preserves_false_native_availability() {
    let service = service();
    let invocation = service
        .submit_with_host(73, call(), Some(Arc::new(UiOnlyHost)))
        .unwrap();
    let ActionOutcome::Succeeded { output } = terminal(&service).outcome else {
        panic!("提示应成功")
    };
    assert_eq!(output["binding"], 73);
    assert_eq!(output["invocation"], invocation);
}
#[test]
fn state_only_scope_has_no_ui_and_never_becomes_a_desktop_context() {
    let service = service();
    service.submit(74, call()).unwrap();
    let ActionOutcome::Failed { error } = terminal(&service).outcome else {
        panic!("仅作用域宿主没有UI")
    };
    assert_eq!(error.code, "ui_unavailable");
    assert_eq!(error.effect_state.as_deref(), Some("not_started"));
    assert_eq!(error.method.as_deref(), Some("ui.action_hint"));
}
#[test]
fn scoped_hint_rejects_forgery_inactive_scope_and_both_cancellation_sources_before_delegation() {
    struct Count(AtomicUsize);
    impl NativeHost for Count {
        fn call(&self, _: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"updated":true}))
        }
    }
    let inner = Arc::new(Count(AtomicUsize::new(0)));
    let service_token = CancellationToken::new();
    let reservations = Arc::new(Mutex::new(Reservations {
        next_id: 2,
        entries: std::collections::BTreeMap::from([(
            1,
            super::super::Reservation {
                binding_id: 73,
                cancellation: service_token.clone(),
                identity: ActionIdentity {
                    package_id: "hint-example".into(),
                    action_id: "work".into(),
                },
                action_version: 3,
                single_instance: false,
                binding_reentry: false,
                active: true,
            },
        )]),
    }));
    let host = scoped_host(
        Some(inner.clone()),
        reservations.clone(),
        Arc::new(Condvar::new()),
        1,
        None,
    );
    let token = CancellationToken::new();
    let deadline = Instant::now() + Duration::from_secs(2);
    for (method, params) in [
        ("ui.action_hint", json!({"text":"n10","scope":{}})),
        ("ui.action_hint.scoped", json!({"scope":{},"text":"n10"})),
    ] {
        assert_eq!(
            host.call_with_deadline(method, params, &token, deadline)
                .unwrap_err()
                .code,
            "invalid_input"
        );
    }
    assert_eq!(
        host.call_with_deadline(
            "ui.action_hint",
            json!({"text":"n10"}),
            &token,
            Instant::now()
        )
        .unwrap_err()
        .code,
        "timeout"
    );
    let assert_cancelled = |error: ActionError| {
        assert_eq!(error.code, "cancelled");
        assert_eq!(error.method.as_deref(), Some("ui.action_hint"));
        assert_eq!(error.effect_state.as_deref(), Some("not_started"));
    };
    token.cancel();
    assert_cancelled(
        host.call_with_deadline("ui.action_hint", json!({"text":"n10"}), &token, deadline)
            .unwrap_err(),
    );
    service_token.cancel();
    assert_cancelled(
        host.call_with_deadline(
            "ui.action_hint",
            json!({"text":"n10"}),
            &CancellationToken::new(),
            deadline,
        )
        .unwrap_err(),
    );
    reservations
        .lock()
        .unwrap()
        .entries
        .get_mut(&1)
        .unwrap()
        .active = false;
    assert_eq!(
        host.call_with_deadline(
            "ui.action_hint",
            json!({"text":"n10"}),
            &CancellationToken::new(),
            deadline
        )
        .unwrap_err()
        .code,
        "ui_hint_scope_expired"
    );
    assert_eq!(inner.0.load(Ordering::SeqCst), 0);
}

#[test]
fn corner_hint_actual_service_forwards_only_placement_with_retained_identity() {
    struct PlacementHost;
    impl NativeHost for PlacementHost {
        fn native_context_available(&self) -> bool {
            false
        }
        fn call(
            &self,
            method: &str,
            params: Value,
            _: &CancellationToken,
        ) -> Result<Value, ActionError> {
            assert_eq!(method, crate::action_hint::SCOPED_METHOD);
            assert_eq!(params.as_object().unwrap().len(), 3);
            let update = crate::action_hint::ScopedActionHint::parse(params)?;
            assert_eq!(
                update.placement,
                crate::action_hint::ActionHintPlacement::TopRight
            );
            assert_eq!(update.text, "n10");
            assert_eq!(update.scope.binding_id, 91);
            assert_eq!(update.scope.package_id, "hint-example");
            assert_eq!(update.scope.action_id, "work");
            assert_eq!(update.scope.action_version, 3);
            Ok(
                json!({"updated":true,"binding":update.scope.binding_id,"invocation":update.scope.invocation_id,"placement":"topRight"}),
            )
        }
    }
    let service = service();
    let mut request = call();
    request.input = json!({"placement":"topRight","scope":"不得转交"});
    let invocation = service
        .submit_with_host(91, request, Some(Arc::new(PlacementHost)))
        .unwrap();
    let ActionOutcome::Succeeded { output } = terminal(&service).outcome else {
        panic!("角标应成功")
    };
    assert_eq!(output["invocation"], invocation);
    assert_eq!(output["placement"], "topRight");
}
