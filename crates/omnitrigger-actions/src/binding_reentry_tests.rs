//! 内存目录、真实ActionService与可控Rust端口的测试源码；不执行JS、文件或桌面接口。
use super::*;
use crate::{catalog::test_catalog, ActionDefinition, CatalogAction};
use serde_json::json;
use std::sync::{
    atomic::AtomicUsize,
    mpsc::{channel, Sender},
};

use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(5);

enum Command {
    Finish,
    Stop,
    State(Option<u64>),
}
struct Entered {
    id: u64,
    token: CancellationToken,
    command: Sender<Command>,
}
struct Runtime {
    entered: Sender<Entered>,
    replies: Sender<(u64, Result<Value, ActionError>)>,
    after_stop: AtomicUsize,
}
impl ActionRuntime for Runtime {
    fn execute(
        &self,
        _: &ResolvedAction,
        _: &InvocationContext,
        _: &CancellationToken,
    ) -> Result<Value, ActionError> {
        panic!("必须复用真实execute_with_host调用路径")
    }
    fn execute_with_host(
        &self,
        action: &ResolvedAction,
        context: &InvocationContext,
        cancellation: &CancellationToken,
        host: Option<&dyn NativeHost>,
    ) -> Result<Value, ActionError> {
        let (command, receive) = channel();
        self.entered
            .send(Entered {
                id: context.invocation_id,
                token: cancellation.clone(),
                command,
            })
            .unwrap();
        match receive.recv_timeout(WAIT) {
            Ok(Command::Finish) => Ok(json!({"synthetic":true})),
            Ok(Command::Stop) => {
                let timeout = action.call.input["stop_timeout_ms"]
                    .as_u64()
                    .unwrap_or(2000);
                let result = host.unwrap().call(
                    "actions.stop_other_instances",
                    json!({"timeout_ms":timeout}),
                    cancellation,
                );
                // 只见证是否到达停止后的业务位置；不是实际CAS或窗口效果。
                if result.is_ok() {
                    self.after_stop.fetch_add(1, Ordering::SeqCst);
                }
                self.replies
                    .send((context.invocation_id, result.clone()))
                    .unwrap();
                result
            }
            Ok(Command::State(write)) => {
                let host = host.unwrap();
                let read = host.call(
                    "state.read",
                    json!({"area":"session","key":"reentry.choice"}),
                    cancellation,
                )?;
                let result = match write {
                    None => Ok(read),
                    Some(value) => host.call(
                        "state.compare_and_swap",
                        json!({"area":"session","key":"reentry.choice","expectedRevision":read["revision"],"next":{"present":true,"value":value}}),
                        cancellation,
                    ),
                };
                self.replies
                    .send((context.invocation_id, result.clone()))
                    .unwrap();
                // 两个同绑定调用都保持活跃，不能用旧终态已读后重启冒充再入。
                match receive.recv_timeout(WAIT) {
                    Ok(Command::Finish) => result,
                    _ => Err(ActionError::new("test_timeout", "状态见证未结束。")),
                }
            }
            Err(_) => Err(ActionError::new("test_timeout", "未收到合成运行许可。")),
        }
    }
}
struct Harness {
    service: ActionService,
    runtime: Arc<Runtime>,
    entered: Receiver<Entered>,
    replies: Receiver<(u64, Result<Value, ActionError>)>,
}
fn catalog(single: bool, allow_binding_reentry: bool) -> PackageCatalog {
    let actions = ["example", "other-package"]
        .into_iter()
        .flat_map(|package| {
            ["run", "other"]
                .into_iter()
                .map(move |action| CatalogAction {
                    package_id: package.into(),
                    package_name: "合成动作".into(),
                    definition: ActionDefinition {
                        id: action.into(),
                        title: "合成入口".into(),
                        version: 1,
                        runtime: "controlled".into(),
                        entry: "not-opened.mjs".into(),
                        single_instance: single,
                        allow_binding_reentry,
                    },
                })
        })
        .collect();
    test_catalog(actions)
}
fn registry(runtime: Arc<Runtime>) -> RuntimeRegistry {
    let mut registry = RuntimeRegistry::new();
    registry.register("controlled", runtime).unwrap();
    registry
}
fn call() -> ActionCall {
    ActionCall {
        package_id: "example".into(),
        action_id: "run".into(),
        action_version: 1,
        input: json!({}),
    }
}
impl Harness {
    fn new(single: bool, allow_binding_reentry: bool) -> Self {
        let (entered_tx, entered) = channel();
        let (replies_tx, replies) = channel();
        let runtime = Arc::new(Runtime {
            entered: entered_tx,
            replies: replies_tx,
            after_stop: AtomicUsize::new(0),
        });
        Self {
            service: ActionService::new(
                catalog(single, allow_binding_reentry),
                registry(runtime.clone()),
            ),
            runtime,
            entered,
            replies,
        }
    }
    fn start(&self, binding: u64, host: Option<Arc<dyn NativeHost>>) -> Entered {
        self.start_call(binding, call(), host)
    }
    fn start_call(
        &self,
        binding: u64,
        call: ActionCall,
        host: Option<Arc<dyn NativeHost>>,
    ) -> Entered {
        let id = self.service.submit_with_host(binding, call, host).unwrap();
        let entered = self.entered.recv_timeout(WAIT).unwrap();
        assert_eq!(entered.id, id);
        entered
    }
    fn terminal(&self) -> ActionEvent {
        let deadline = Instant::now() + WAIT;
        loop {
            match self.service.try_recv() {
                Ok(event) if event.outcome.is_terminal() => return event,
                Ok(_) | Err(TryRecvError::Empty) => {}
                Err(error) => panic!("事件通道关闭：{error}"),
            }
            assert!(Instant::now() < deadline);
            thread::yield_now();
        }
    }
    fn inactive(&self, id: u64) {
        let deadline = Instant::now() + WAIT;
        while self.service.reservations.lock().unwrap().entries[&id].active {
            assert!(Instant::now() < deadline);
            thread::yield_now();
        }
    }
}
fn until_cancelled(token: &CancellationToken) {
    let deadline = Instant::now() + WAIT;
    while !token.is_cancelled() {
        assert!(Instant::now() < deadline);
        thread::yield_now();
    }
}

struct Cleanup {
    owner: u64,
    entered: Sender<u64>,
    release: Mutex<Receiver<()>>,
    released: Arc<Mutex<Vec<u64>>>,
}
impl NativeHost for Cleanup {
    fn call(&self, _: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
        panic!("合成清理宿主不调用本机接口")
    }
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        self.entered.send(self.owner).unwrap();
        self.release.get_mut().unwrap().recv_timeout(WAIT).unwrap();
        self.released.lock().unwrap().push(self.owner);
    }
}
fn cleanup(
    owner: u64,
    released: Arc<Mutex<Vec<u64>>>,
) -> (Arc<dyn NativeHost>, Receiver<u64>, Sender<()>) {
    let (entered, observe) = channel();
    let (release, wait) = channel();
    (
        Arc::new(Cleanup {
            owner,
            entered,
            release: Mutex::new(wait),
            released,
        }),
        observe,
        release,
    )
}

#[test]
fn ordinary_false_still_rejects_same_binding_without_cancelling() {
    let mut h = Harness::new(false, false);
    let mut request = call();
    request.input = json!({"allow_binding_reentry":true});
    let old = h.start_call(7, request.clone(), None);
    let snapshot = serde_json::to_value(h.service.describe_catalog()).unwrap();
    assert!(snapshot["actions"][0]
        .get("allow_binding_reentry")
        .is_none());
    assert_eq!(
        h.service.submit(7, call()).unwrap_err().code,
        "binding_busy"
    );
    assert_eq!(
        h.service
            .submit_with_host(7, request, None)
            .unwrap_err()
            .code,
        "binding_busy"
    );
    h.service
        .reload(catalog(false, true), registry(h.runtime.clone()));
    assert_eq!(
        h.service.submit(7, call()).unwrap_err().code,
        "binding_busy"
    );
    assert!(!old.token.is_cancelled());
    assert_eq!(h.service.reservations.lock().unwrap().entries.len(), 1);
    old.command.send(Command::Finish).unwrap();
    h.terminal();
}

#[test]
fn explicit_reentry_starts_business_judgment_without_automatic_cancel_or_stop() {
    let h = Harness::new(false, true);
    let first = h.start(7, None);
    let mut request = call();
    request.input = json!({"allow_binding_reentry":false});
    let second = h.start_call(7, request, None);
    assert_ne!(first.id, second.id);
    assert!(!first.token.is_cancelled() && !second.token.is_cancelled());
    assert_eq!(h.runtime.after_stop.load(Ordering::SeqCst), 0);
    assert!(h.service.is_running(7));
    let snapshot = serde_json::to_value(h.service.describe_catalog()).unwrap();
    assert!(snapshot["actions"]
        .as_array()
        .unwrap()
        .iter()
        .all(|entry| entry["allow_binding_reentry"] == true));
    let third_id = h.service.submit(7, call()).unwrap();
    let third = h.entered.recv_timeout(WAIT).unwrap();
    assert_eq!(third.id, third_id);
    assert!(!third.token.is_cancelled());
    third.command.send(Command::Finish).unwrap();
    assert_eq!(h.terminal().invocation_id, third.id);
    first.command.send(Command::Finish).unwrap();
    assert_eq!(h.terminal().invocation_id, first.id);
    assert!(h.service.is_running(7));
    assert!(!second.token.is_cancelled());
    second.command.send(Command::Finish).unwrap();
    assert_eq!(h.terminal().invocation_id, second.id);
    assert!(!h.service.is_running(7));
}

#[test]
fn matched_business_stop_waits_for_old_owned_host_cleanup_before_continuation() {
    let h = Harness::new(false, true);
    let released = Arc::new(Mutex::new(Vec::new()));
    let (host, cleanup_entered, release) = cleanup(101, released.clone());
    let old = h.start(7, Some(host));
    let new = h.start(7, None);
    new.command.send(Command::Stop).unwrap();
    until_cancelled(&old.token);
    assert!(!new.token.is_cancelled());
    old.command.send(Command::Finish).unwrap();
    assert_eq!(cleanup_entered.recv_timeout(WAIT).unwrap(), 101);
    assert!(h.service.reservations.lock().unwrap().entries[&old.id].active);
    assert_eq!(h.runtime.after_stop.load(Ordering::SeqCst), 0);
    assert!(matches!(h.replies.try_recv(), Err(TryRecvError::Empty)));
    assert!(released.lock().unwrap().is_empty());
    release.send(()).unwrap();
    let (id, receipt) = h.replies.recv_timeout(WAIT).unwrap();
    assert_eq!(id, new.id);
    assert_eq!(
        receipt.unwrap(),
        json!({"stoppedCount":1,"effectState":"unknown"})
    );
    assert_eq!(*released.lock().unwrap(), [101]); // 只清理旧宿主自己的合成所有权。
    assert_eq!(h.runtime.after_stop.load(Ordering::SeqCst), 1);
    let outcomes: BTreeMap<_, _> = (0..2)
        .map(|_| {
            let event = h.terminal();
            (event.invocation_id, event.outcome)
        })
        .collect();
    assert_eq!(outcomes[&old.id], ActionOutcome::Cancelled);
    assert!(matches!(outcomes[&new.id], ActionOutcome::Succeeded { .. }));
}

#[test]
fn cleanup_timeout_is_unknown_and_does_not_reach_after_stop_or_release_old_ownership() {
    let h = Harness::new(false, true);
    let released = Arc::new(Mutex::new(Vec::new()));
    let (host, cleanup_entered, release) = cleanup(101, released.clone());
    let old = h.start(7, Some(host));
    let mut request = call();
    request.input = json!({"stop_timeout_ms":1000});
    let new = h.start_call(7, request, None);
    new.command.send(Command::Stop).unwrap();
    let (id, result) = h.replies.recv_timeout(WAIT).unwrap();
    assert_eq!(id, new.id);
    let error = result.unwrap_err();
    assert_eq!(error.code, "stop_timeout");
    assert_eq!(error.effect_state.as_deref(), Some("unknown"));
    assert!(old.token.is_cancelled());
    assert_eq!(h.runtime.after_stop.load(Ordering::SeqCst), 0);
    old.command.send(Command::Finish).unwrap();
    cleanup_entered.recv_timeout(WAIT).unwrap();
    assert!(released.lock().unwrap().is_empty());
    assert!(h.service.reservations.lock().unwrap().entries[&old.id].active);
    release.send(()).unwrap();
    h.terminal();
    h.terminal();
}

#[test]
fn single_instance_true_and_reload_of_old_true_remain_rejections_without_cancel() {
    let mut h = Harness::new(true, false);
    let old = h.start(7, None);
    assert_eq!(
        h.service
            .submit_with_host(7, call(), None)
            .unwrap_err()
            .code,
        "binding_busy"
    );
    assert_eq!(h.service.submit(8, call()).unwrap_err().code, "action_busy");
    h.service
        .reload(catalog(false, true), registry(h.runtime.clone()));
    assert_eq!(
        h.service
            .submit_with_host(8, call(), None)
            .unwrap_err()
            .code,
        "action_busy"
    );
    assert!(!old.token.is_cancelled());
    old.command.send(Command::Finish).unwrap();
    h.terminal();
    h.service
        .reload(catalog(true, true), registry(h.runtime.clone()));
    for error in [
        h.service.validate(&call()).unwrap_err(),
        h.service.submit(7, call()).unwrap_err(),
    ] {
        assert_eq!(error.code, "invalid_action_policy");
        assert_eq!(error.effect_state.as_deref(), Some("not_started"));
    }
    assert!(h.service.reservations.lock().unwrap().entries.is_empty());
    assert!(matches!(h.entered.try_recv(), Err(TryRecvError::Empty)));
}

#[test]
fn same_binding_reentry_keeps_four_reservations_and_never_evicts_unread_terminals() {
    let h = Harness::new(false, true);
    let runs: Vec<_> = (0..CAPACITY).map(|_| h.start(7, None)).collect();
    assert_eq!(
        h.service
            .submit_with_host(7, call(), None)
            .unwrap_err()
            .code,
        "capacity_exceeded"
    );
    assert!(runs.iter().all(|run| !run.token.is_cancelled()));
    for run in &runs {
        run.command.send(Command::Finish).unwrap();
    }
    for run in &runs {
        h.inactive(run.id);
    }
    assert_eq!(
        h.service.reservations.lock().unwrap().entries.len(),
        CAPACITY
    );
    assert!(h.service.is_running(7));
    assert_eq!(
        h.service
            .submit_with_host(7, call(), None)
            .unwrap_err()
            .code,
        "binding_busy"
    );
    for remaining in (0..CAPACITY).rev() {
        h.terminal();
        assert_eq!(h.service.is_running(7), remaining != 0);
    }
}

#[test]
fn cancel_id_cancels_all_same_binding_tokens_without_early_removal() {
    let h = Harness::new(false, true);
    let a = h.start(7, None);
    let b = h.start(7, None);
    let other = h.start(8, None);
    assert_eq!(h.service.cancel_id(7), 2);
    assert!(a.token.is_cancelled() && b.token.is_cancelled());
    assert!(!other.token.is_cancelled());
    assert_eq!(h.service.reservations.lock().unwrap().entries.len(), 3);
    assert!(h.service.is_running(7));
    assert_eq!(h.service.cancel_all(), 3);
    assert!(other.token.is_cancelled());
    for run in [a, b, other] {
        run.command.send(Command::Finish).unwrap();
    }
    for _ in 0..3 {
        assert_eq!(h.terminal().outcome, ActionOutcome::Cancelled);
    }
}

#[test]
fn reload_preserves_old_tokens_hosts_and_unread_ids_but_new_true_cannot_reenter() {
    let mut h = Harness::new(false, true);
    let old = h.start(7, None);
    h.service
        .reload(catalog(true, false), registry(h.runtime.clone()));
    assert!(h.service.is_running(7));
    assert_eq!(
        h.service
            .submit_with_host(7, call(), None)
            .unwrap_err()
            .code,
        "binding_busy"
    );
    assert!(!old.token.is_cancelled());
    assert_eq!(h.service.cancel_id(7), 1);
    old.command.send(Command::Finish).unwrap();
    h.inactive(old.id);
    assert!(h.service.is_running(7));
    assert_eq!(h.terminal().invocation_id, old.id);
    let next = h.start(7, None);
    assert!(next.id > old.id);
    next.command.send(Command::Finish).unwrap();
    h.terminal();
}

#[test]
fn cross_identity_or_ordinary_old_occupancy_cannot_be_borrowed_for_reentry() {
    let h = Harness::new(false, true);
    let old = h.start(7, None);
    for (package, action) in [("other-package", "run"), ("example", "other")] {
        let request = ActionCall {
            package_id: package.into(),
            action_id: action.into(),
            ..call()
        };
        assert_eq!(
            h.service
                .submit_with_host(7, request, None)
                .unwrap_err()
                .code,
            "binding_busy"
        );
    }
    assert!(!old.token.is_cancelled());
    old.command.send(Command::Finish).unwrap();
    h.terminal();
}

#[test]
fn unread_terminal_still_blocks_even_if_another_opted_in_call_is_active() {
    let h = Harness::new(false, true);
    let old = h.start(7, None);
    let active = h.start(7, None);
    old.command.send(Command::Finish).unwrap();
    h.inactive(old.id);
    assert_eq!(
        h.service
            .submit_with_host(7, call(), None)
            .unwrap_err()
            .code,
        "binding_busy"
    );
    assert_eq!(h.terminal().invocation_id, old.id);
    let next = h.start(7, None);
    assert!(!active.token.is_cancelled());
    active.command.send(Command::Finish).unwrap();
    next.command.send(Command::Finish).unwrap();
    h.terminal();
    h.terminal();
}

#[test]
fn same_binding_reentry_preserves_real_scoped_state_read_cas_and_action_scope() {
    let mut h = Harness::new(false, true);
    let store = Arc::new(crate::state::ActionStateStore::new(None).unwrap());
    h.service = h.service.with_state_store(store);
    let old = h.start(7, None);
    old.command.send(Command::State(Some(1000))).unwrap();
    let (id, committed) = h.replies.recv_timeout(WAIT).unwrap();
    assert_eq!(id, old.id);
    let committed = committed.unwrap();
    assert_eq!(committed["status"], "committed");
    let new = h.start(7, None);
    new.command.send(Command::State(None)).unwrap();
    let (id, snapshot) = h.replies.recv_timeout(WAIT).unwrap();
    assert_eq!(id, new.id);
    let snapshot = snapshot.unwrap();
    assert_eq!(snapshot["entry"], json!({"present":true,"value":1000}));
    assert_eq!(snapshot["revision"], committed["current"]["revision"]);
    assert!(!old.token.is_cancelled() && !new.token.is_cancelled());
    assert!(h.service.reservations.lock().unwrap().entries[&old.id].active);
    assert!(h.service.reservations.lock().unwrap().entries[&new.id].active);
    old.command.send(Command::Finish).unwrap();
    new.command.send(Command::Finish).unwrap();
    h.terminal();
    h.terminal();
}

#[test]
fn reentry_spawn_failure_removes_only_new_slot_without_cancelling_or_replaying_old() {
    let h = Harness::new(false, true);
    let old = h.start(7, None);
    let error = h
        .service
        .submit_with_host_and_spawn(7, call(), None, |_, _| {
            Err(std::io::Error::other("合成线程启动失败"))
        })
        .unwrap_err();
    assert_eq!(error.code, "worker_start_failed");
    let state = h.service.reservations.lock().unwrap();
    assert_eq!(state.entries.len(), 1);
    assert!(state.entries[&old.id].active);
    drop(state);
    assert!(!old.token.is_cancelled());
    assert!(matches!(h.entered.try_recv(), Err(TryRecvError::Empty)));
    old.command.send(Command::Finish).unwrap();
    h.terminal();
}

#[test]
fn action_version_reload_does_not_borrow_old_reentry_admission() {
    let mut h = Harness::new(false, true);
    let old = h.start(7, None);
    let mut actions = catalog(false, true).actions;
    for action in &mut actions {
        action.definition.version = 2;
    }
    h.service
        .reload(test_catalog(actions), registry(h.runtime.clone()));
    let request = ActionCall {
        action_version: 2,
        ..call()
    };
    assert_eq!(
        h.service
            .submit_with_host(7, request.clone(), None)
            .unwrap_err()
            .code,
        "binding_busy"
    );
    assert!(!old.token.is_cancelled());
    old.command.send(Command::Finish).unwrap();
    assert_eq!(h.terminal().invocation_id, old.id);
    let next = h.start_call(7, request, None);
    next.command.send(Command::Finish).unwrap();
    assert_eq!(h.terminal().invocation_id, next.id);
}
