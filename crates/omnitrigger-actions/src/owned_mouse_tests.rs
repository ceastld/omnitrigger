use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize};

#[derive(Default)]
struct Backend {
    events: Mutex<Vec<&'static str>>,
    held: Mutex<Option<u64>>,
    target_changed: AtomicBool,
    down_error_after_delivery: AtomicBool,
    failed_ups: AtomicUsize,
}
impl OwnedLeftBackend for Backend {
    fn check_original_target(&self) -> Result<(), ActionError> {
        self.events.lock().unwrap().push("check-original");
        if self.target_changed.load(Ordering::Relaxed) {
            Err(fault(DOWN, "target_changed", false))
        } else {
            Ok(())
        }
    }
    fn down_owned(
        &self,
        owner: OwnedInputOwner,
        _: Instant,
        _: &dyn Fn() -> bool,
    ) -> Result<(), ActionError> {
        let mut held = self.held.lock().unwrap();
        if held.is_some() {
            return Err(fault(DOWN, "input_busy", false));
        }
        *held = Some(owner.id());
        self.events.lock().unwrap().push("mouse-left-down");
        if self.down_error_after_delivery.load(Ordering::Relaxed) {
            Err(fault(DOWN, "input_failed", true))
        } else {
            Ok(())
        }
    }
    fn release_owned(&self, owner: OwnedInputOwner) -> Result<(), ActionError> {
        let mut held = self.held.lock().unwrap();
        if *held != Some(owner.id()) {
            return Ok(());
        }
        self.events.lock().unwrap().push("mouse-left-up");
        if self
            .failed_ups
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
            .is_ok()
        {
            return Err(fault(UP, "input_failed", true));
        }
        *held = None;
        Ok(())
    }
}
fn session(backend: &Arc<Backend>) -> OwnedLeftSession {
    OwnedLeftSession::new(new_owner().unwrap(), backend.clone())
}
fn run(
    session: &OwnedLeftSession,
    method: &str,
    token: &CancellationToken,
) -> Result<Value, ActionError> {
    session.call(
        method,
        json!({}),
        token,
        Instant::now() + std::time::Duration::from_secs(1),
    )
}
#[test]
fn normal_sequence_has_one_mouse_down_one_owned_up_and_no_extra_drop_input() {
    let backend = Arc::new(Backend::default());
    let input = session(&backend);
    let token = CancellationToken::new();
    run(&input, DOWN, &token).unwrap();
    run(&input, UP, &token).unwrap();
    input.finish().unwrap();
    drop(input);
    assert_eq!(
        *backend.events.lock().unwrap(),
        [
            "check-original",
            "mouse-left-down",
            "check-original",
            "check-original",
            "mouse-left-up"
        ]
    );
}
#[test]
fn target_change_before_press_emits_nothing_after_press_releases_same_owner() {
    let backend = Arc::new(Backend::default());
    let input = session(&backend);
    let token = CancellationToken::new();
    backend.target_changed.store(true, Ordering::Relaxed);
    assert_eq!(
        run(&input, DOWN, &token).unwrap_err().code,
        "target_changed"
    );
    assert_eq!(*backend.held.lock().unwrap(), None);
    backend.target_changed.store(false, Ordering::Relaxed);
    run(&input, DOWN, &token).unwrap();
    backend.target_changed.store(true, Ordering::Relaxed);
    let error = run(&input, UP, &token).unwrap_err();
    assert_eq!(error.code, "target_changed");
    assert_eq!(error.effect_state.as_deref(), Some("unknown"));
    assert_eq!(*backend.held.lock().unwrap(), None);
    assert_eq!(
        backend
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| **e == "mouse-left-down")
            .count(),
        1
    );
    assert_eq!(
        backend
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| **e == "mouse-left-up")
            .count(),
        1
    );
}
#[test]
fn cancellation_after_press_does_not_skip_owned_release() {
    let backend = Arc::new(Backend::default());
    let input = session(&backend);
    let token = CancellationToken::new();
    run(&input, DOWN, &token).unwrap();
    token.cancel();
    assert_eq!(run(&input, UP, &token).unwrap_err().code, "cancelled");
    assert_eq!(*backend.held.lock().unwrap(), None);
    assert_eq!(
        *backend.events.lock().unwrap(),
        [
            "check-original",
            "mouse-left-down",
            "check-original",
            "mouse-left-up"
        ]
    );
}
#[test]
fn runtime_exception_or_uncertain_down_keeps_cleanup_obligation() {
    let backend = Arc::new(Backend::default());
    backend
        .down_error_after_delivery
        .store(true, Ordering::Relaxed);
    {
        let input = session(&backend);
        assert!(run(&input, DOWN, &CancellationToken::new()).is_err());
        // 不确定Down的错误分支立即清理；Drop不得重复释放。
    }
    assert_eq!(*backend.held.lock().unwrap(), None);
    assert_eq!(
        *backend.events.lock().unwrap(),
        ["check-original", "mouse-left-down", "mouse-left-up"]
    );
    let backend = Arc::new(Backend::default());
    {
        let input = session(&backend);
        run(&input, DOWN, &CancellationToken::new()).unwrap();
        // 模拟成功Down后运行时异常提前退出，由Drop兜底。
    }
    assert_eq!(*backend.held.lock().unwrap(), None);
    assert_eq!(
        *backend.events.lock().unwrap(),
        [
            "check-original",
            "mouse-left-down",
            "check-original",
            "mouse-left-up"
        ]
    );
}
#[test]
fn failed_release_is_five_attempts_total_and_never_replays_down() {
    let backend = Arc::new(Backend::default());
    backend.failed_ups.store(100, Ordering::Relaxed);
    let input = session(&backend);
    run(&input, DOWN, &CancellationToken::new()).unwrap();
    assert_eq!(input.finish().unwrap_err().code, "input_cleanup_failed");
    drop(input);
    assert_eq!(
        backend
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| **e == "mouse-left-up")
            .count(),
        5
    );
    assert_eq!(
        backend
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| **e == "mouse-left-down")
            .count(),
        1
    );
    assert!(
        backend.held.lock().unwrap().is_some(),
        "未释放义务不能伪装已清除"
    );
}
#[test]
fn second_owner_cannot_press_or_release_the_first_owners_input() {
    let backend = Arc::new(Backend::default());
    let first = session(&backend);
    let second = session(&backend);
    run(&first, DOWN, &CancellationToken::new()).unwrap();
    let owner = *backend.held.lock().unwrap();
    assert_eq!(
        run(&second, DOWN, &CancellationToken::new())
            .unwrap_err()
            .code,
        "input_busy"
    );
    second.finish().unwrap();
    assert_eq!(*backend.held.lock().unwrap(), owner);
    first.finish().unwrap();
    assert_eq!(*backend.held.lock().unwrap(), None);
}

#[test]
fn a_confirmed_new_press_cycle_has_its_own_five_attempt_budget() {
    let backend = Arc::new(Backend::default());
    let input = session(&backend);
    let token = CancellationToken::new();
    for _ in 0..2 {
        backend.failed_ups.store(4, Ordering::Relaxed);
        run(&input, DOWN, &token).unwrap();
        run(&input, UP, &token).unwrap();
    }
    input.finish().unwrap();
    assert_eq!(
        backend
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| **e == "mouse-left-down")
            .count(),
        2
    );
    assert_eq!(
        backend
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| **e == "mouse-left-up")
            .count(),
        10
    );
    assert_eq!(*backend.held.lock().unwrap(), None);
}

// Down内部清理失败必须按Down应答；finish主动清理仍属于Up，严格桥接无需放宽。
fn assert_owned_down_cleanup_error_method(down_error: bool) {
    let backend = Arc::new(Backend::default());
    backend
        .down_error_after_delivery
        .store(down_error, Ordering::Relaxed);
    backend.failed_ups.store(100, Ordering::Relaxed);
    let input = session(&backend);
    let checks = AtomicUsize::new(0);
    let error = input
        .call_if(
            DOWN,
            json!({}),
            Instant::now() + std::time::Duration::from_secs(1),
            || checks.fetch_add(1, Ordering::Relaxed) != 0,
        )
        .unwrap_err();
    assert_eq!(error.code, "input_cleanup_failed");
    assert_eq!(error.method.as_deref(), Some(DOWN));
    assert_eq!(error.effect_state.as_deref(), Some("unknown"));
    let cleanup = input.finish().unwrap_err();
    assert_eq!(cleanup.code, "input_cleanup_failed");
    assert_eq!(cleanup.method.as_deref(), Some(UP));
    assert_eq!(cleanup.effect_state.as_deref(), Some("unknown"));
    drop(input);
    let events = backend.events.lock().unwrap();
    assert_eq!(
        events.iter().filter(|e| **e == "mouse-left-down").count(),
        1
    );
    assert_eq!(events.iter().filter(|e| **e == "mouse-left-up").count(), 5);
    assert!(backend.held.lock().unwrap().is_some());
}
#[test]
fn owned_down_backend_error_cleanup_failure_keeps_down_response_method() {
    assert_owned_down_cleanup_error_method(true);
}
#[test]
fn owned_down_post_delivery_guard_cleanup_failure_keeps_down_response_method() {
    assert_owned_down_cleanup_error_method(false);
}
