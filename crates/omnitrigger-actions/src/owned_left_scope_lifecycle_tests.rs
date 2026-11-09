//! 作为service_actions.rs的测试子模块引入，使用现有scoped_host/Reservations。
//! 无新Runtime/包目录/线程/输入；只注入确定的NativeHost与OwnedLeftBackend回调。
use super::super::Reservation;
use super::*;
use crate::owned_mouse::{self, OwnedInputOwner, OwnedLeftBackend, OwnedLeftSession, DOWN};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

struct Backend {
    service_token: CancellationToken,
    cancel_inside_down: AtomicBool,
    cancel_after_delivery: AtomicBool,
    caller_to_cancel: Mutex<Option<CancellationToken>>,
    delivered: AtomicUsize,
    released: AtomicUsize,
    held: Mutex<Option<u64>>,
}
impl OwnedLeftBackend for Backend {
    fn check_original_target(&self) -> Result<(), ActionError> {
        Ok(())
    }
    fn down_owned(
        &self,
        owner: OwnedInputOwner,
        _: Instant,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), ActionError> {
        if self.cancel_inside_down.load(Ordering::Relaxed) {
            self.service_token.cancel();
        }
        if let Some(caller) = self.caller_to_cancel.lock().unwrap().as_ref() {
            caller.cancel();
        }
        // 对应实际client/worker最后一次检查：这里必须看得到刚发生的Service取消。
        if cancelled() {
            return Err(owned_mouse::fault(DOWN, "cancelled", false));
        }
        *self.held.lock().unwrap() = Some(owner.id());
        self.delivered.fetch_add(1, Ordering::Relaxed);
        if self.cancel_after_delivery.load(Ordering::Relaxed) {
            self.service_token.cancel();
        }
        Ok(())
    }
    fn release_owned(&self, owner: OwnedInputOwner) -> Result<(), ActionError> {
        let mut held = self.held.lock().unwrap();
        if *held == Some(owner.id()) {
            *held = None;
            self.released.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }
}
struct Factory {
    backend: Arc<Backend>,
    calls: AtomicUsize,
}
impl NativeHost for Factory {
    fn create_owned_left_input(
        &self,
        owner: OwnedInputOwner,
    ) -> Result<Arc<OwnedLeftSession>, ActionError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(Arc::new(OwnedLeftSession::new(owner, self.backend.clone())))
    }
    fn call(&self, _: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
        panic!("必须由原InvocationScopedHost处理鼠标方法")
    }
}
fn fixture() -> (Arc<dyn NativeHost>, Arc<Factory>) {
    let service_token = CancellationToken::new();
    let backend = Arc::new(Backend {
        service_token: service_token.clone(),
        cancel_inside_down: AtomicBool::new(false),
        cancel_after_delivery: AtomicBool::new(false),
        caller_to_cancel: Mutex::new(None),
        delivered: AtomicUsize::new(0),
        released: AtomicUsize::new(0),
        held: Mutex::new(None),
    });
    let factory = Arc::new(Factory {
        backend,
        calls: AtomicUsize::new(0),
    });
    let reservations = Arc::new(Mutex::new(Reservations {
        next_id: 2,
        entries: BTreeMap::from([(
            1,
            Reservation {
                binding_id: 7,
                cancellation: service_token,
                identity: ActionIdentity {
                    package_id: "test.review".into(),
                    action_id: "work".into(),
                },
                action_version: 1,
                single_instance: true,
                binding_reentry: false,
                active: true,
            },
        )]),
    }));
    let host = scoped_host(
        Some(factory.clone()),
        reservations,
        Arc::new(Condvar::new()),
        1,
        None,
    );
    (host, factory)
}

#[test]
fn finishing_an_empty_active_scope_prevents_late_factory_creation() {
    let (host, factory) = fixture();
    let retained = host.clone();
    host.finish_owned_inputs().unwrap();
    let result = retained.call_with_deadline(
        DOWN,
        json!({}),
        &CancellationToken::new(),
        Instant::now() + Duration::from_secs(60),
    );
    assert_eq!(result.unwrap_err().code, "action_inactive");
    assert_eq!(factory.calls.load(Ordering::Relaxed), 0);
    assert_eq!(factory.backend.delivered.load(Ordering::Relaxed), 0);
}

#[test]
fn a_service_cancel_after_scope_check_is_live_even_with_an_independent_call_token() {
    let (host, factory) = fixture();
    factory
        .backend
        .cancel_inside_down
        .store(true, Ordering::Relaxed);
    let independent = CancellationToken::new();
    let result = host.call_with_deadline(
        DOWN,
        json!({}),
        &independent,
        Instant::now() + Duration::from_secs(60),
    );
    host.finish_owned_inputs().unwrap();
    assert!(factory.backend.service_token.is_cancelled());
    assert!(!independent.is_cancelled());
    assert!(result.is_err(), "作用域不能只在入口时快照Service取消状态");
    assert_eq!(factory.backend.delivered.load(Ordering::Relaxed), 0);
}

#[test]
fn independent_caller_cancel_during_down_is_also_live() {
    let (host, factory) = fixture();
    let caller = CancellationToken::new();
    *factory.backend.caller_to_cancel.lock().unwrap() = Some(caller.clone());
    let result = host.call_with_deadline(
        DOWN,
        json!({}),
        &caller,
        Instant::now() + Duration::from_secs(60),
    );
    host.finish_owned_inputs().unwrap();
    assert_eq!(result.unwrap_err().code, "cancelled");
    assert!(caller.is_cancelled());
    assert!(!factory.backend.service_token.is_cancelled());
    assert_eq!(factory.backend.delivered.load(Ordering::Relaxed), 0);
}

#[test]
fn service_cancel_after_delivery_cleans_the_original_owner_before_returning_error() {
    let (host, factory) = fixture();
    factory
        .backend
        .cancel_after_delivery
        .store(true, Ordering::Relaxed);
    let caller = CancellationToken::new();
    let result = host.call_with_deadline(
        DOWN,
        json!({}),
        &caller,
        Instant::now() + Duration::from_secs(60),
    );
    assert_eq!(result.unwrap_err().code, "cancelled");
    assert_eq!(factory.backend.delivered.load(Ordering::Relaxed), 1);
    assert_eq!(factory.backend.released.load(Ordering::Relaxed), 1);
    host.finish_owned_inputs().unwrap();
    assert_eq!(factory.backend.released.load(Ordering::Relaxed), 1);
}
