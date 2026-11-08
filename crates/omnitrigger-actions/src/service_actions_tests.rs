//! 内存目录+真实ActionService调用路径；不启动脚本、进程、文件系统或桌面接口。
use super::*;
use crate::{
    catalog::test_catalog, ActionCall, ActionDefinition, ActionEvent, ActionOutcome, ActionRuntime,
    ActionService, CatalogAction, InvocationContext, ResolvedAction, RuntimeRegistry,
};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

const WAIT: Duration = Duration::from_secs(5);
struct Entered {
    id: u64,
    token: CancellationToken,
    go: Sender<()>,
}
type Reply = (u64, Result<Value, ActionError>);

struct Runtime {
    entered: Sender<Entered>,
    replies: Sender<Reply>,
    commits: AtomicUsize,
    calls: AtomicUsize,
}
impl ActionRuntime for Runtime {
    fn execute(
        &self,
        _: &ResolvedAction,
        _: &InvocationContext,
        _: &CancellationToken,
    ) -> Result<Value, ActionError> {
        panic!("服务必须沿真实execute_with_host路径调用")
    }
    fn execute_with_host(
        &self,
        action: &ResolvedAction,
        context: &InvocationContext,
        cancellation: &CancellationToken,
        host: Option<&dyn NativeHost>,
    ) -> Result<Value, ActionError> {
        let host = host.expect("ActionService必须提供作用域宿主");
        assert!(host.action_scope_available());
        let (go, ready) = mpsc::channel();
        self.entered
            .send(Entered {
                id: context.invocation_id,
                token: cancellation.clone(),
                go,
            })
            .unwrap();
        match action.call.input["role"].as_str().unwrap() {
            "old" => {
                let deadline = Instant::now() + WAIT;
                while !cancellation.is_cancelled() {
                    if Instant::now() >= deadline {
                        return Err(ActionError::new("test_timeout", "测试旧调用未取消。"));
                    }
                    thread::yield_now();
                }
                Ok(json!({"old_effect_may_have_happened":true}))
            }
            "idle" => Ok(json!({})),
            "stop" => {
                ready
                    .recv_timeout(WAIT)
                    .map_err(|_| ActionError::new("test_timeout", "未许可调用。"))?;
                self.calls.fetch_add(1, Ordering::SeqCst);
                let params = action
                    .call
                    .input
                    .get("params")
                    .cloned()
                    .unwrap_or(json!({"timeout_ms":2000}));
                let deadline = if action.call.input["expired"] == true {
                    Instant::now()
                } else {
                    Instant::now() + WAIT
                };
                let result = host.call_with_deadline(METHOD, params, cancellation, deadline);
                if result.is_ok() {
                    self.commits.fetch_add(1, Ordering::SeqCst);
                }
                self.replies
                    .send((context.invocation_id, result.clone()))
                    .unwrap();
                result
            }
            _ => panic!("未知测试角色"),
        }
    }
}

struct Harness {
    service: ActionService,
    runtime: Arc<Runtime>,
    entered: Receiver<Entered>,
    replies: Receiver<Reply>,
}
impl Harness {
    fn new(single: bool) -> Self {
        let actions = ["example", "other-package"]
            .into_iter()
            .flat_map(|package| {
                ["work", "other"].into_iter().map(move |id| CatalogAction {
                    package_id: package.into(),
                    package_name: "合成动作".into(),
                    definition: ActionDefinition {
                        id: id.into(),
                        title: "合成动作".into(),
                        version: 1,
                        runtime: "controlled".into(),
                        entry: "not-opened.mjs".into(),
                        single_instance: single,
                        allow_binding_reentry: false,
                    },
                })
            })
            .collect();
        let (entered_tx, entered) = mpsc::channel();
        let (replies_tx, replies) = mpsc::channel();
        let runtime = Arc::new(Runtime {
            entered: entered_tx,
            replies: replies_tx,
            commits: AtomicUsize::new(0),
            calls: AtomicUsize::new(0),
        });
        let mut registry = RuntimeRegistry::new();
        registry.register("controlled", runtime.clone()).unwrap();
        Self {
            service: ActionService::new(test_catalog(actions), registry),
            runtime,
            entered,
            replies,
        }
    }
    fn start(&self, binding: u64, call: ActionCall, host: Option<Arc<dyn NativeHost>>) -> Entered {
        let id = self.service.submit_with_host(binding, call, host).unwrap();
        let entered = self.entered.recv_timeout(WAIT).unwrap();
        assert_eq!(entered.id, id);
        entered
    }
    fn reply(&self) -> Reply {
        self.replies.recv_timeout(WAIT).unwrap()
    }
    fn terminal(&self) -> ActionEvent {
        let deadline = Instant::now() + WAIT;
        loop {
            match self.service.try_recv() {
                Ok(event) if event.outcome.is_terminal() => return event,
                Ok(_) | Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(error) => panic!("事件通道关闭：{error}"),
            }
            assert!(Instant::now() < deadline);
            thread::yield_now();
        }
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        self.service.cancel_all();
    }
}
fn call(role: &str) -> ActionCall {
    ActionCall {
        package_id: "example".into(),
        action_id: "work".into(),
        action_version: 1,
        input: json!({"role":role}),
    }
}
fn active(h: &Harness, id: u64) -> bool {
    h.service
        .reservations
        .lock()
        .unwrap()
        .entries
        .get(&id)
        .is_some_and(|entry| entry.active)
}
fn wait_until(predicate: impl Fn() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !predicate() {
        assert!(Instant::now() < deadline);
        thread::yield_now();
    }
}
struct Cleanup {
    entered: Sender<()>,
    release: Mutex<Receiver<()>>,
}
impl NativeHost for Cleanup {
    fn call(&self, _: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
        panic!("不得访问桌面接口")
    }
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = self.entered.send(());
        let _ = self.release.get_mut().unwrap().recv_timeout(WAIT);
    }
}
fn cleanup_host() -> (Arc<dyn NativeHost>, Receiver<()>, Sender<()>) {
    let (entered_tx, entered) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    (
        Arc::new(Cleanup {
            entered: entered_tx,
            release: Mutex::new(release_rx),
        }),
        entered,
        release,
    )
}
fn assert_error(result: Result<Value, ActionError>, code: &str, effect: &str) {
    let error = result.unwrap_err();
    assert_eq!(error.code, code);
    assert_eq!(error.effect_state.as_deref(), Some(effect));
    assert_eq!(error.method.as_deref(), Some(METHOD));
}

#[test]
fn current_runtime_stops_only_peers_and_commits_only_after_their_real_host_cleanup() {
    let h = Harness::new(false);
    let (host, cleanup, release) = cleanup_host();
    let old = h.start(1, call("old"), Some(host));
    let current = h.start(2, call("stop"), None);
    current.go.send(()).unwrap();
    cleanup.recv_timeout(WAIT).unwrap();
    assert!(old.token.is_cancelled());
    assert!(!current.token.is_cancelled());
    assert!(active(&h, old.id));
    assert_eq!(h.runtime.commits.load(Ordering::SeqCst), 0);
    release.send(()).unwrap();
    let (id, result) = h.reply();
    assert_eq!(id, current.id);
    assert_eq!(
        result.unwrap(),
        json!({"stoppedCount":1,"effectState":"unknown"})
    );
    assert_eq!(h.runtime.commits.load(Ordering::SeqCst), 1);
    let outcomes: BTreeMap<_, _> = (0..2)
        .map(|_| {
            let event = h.terminal();
            (event.invocation_id, event.outcome)
        })
        .collect();
    assert_eq!(outcomes[&old.id], ActionOutcome::Cancelled);
    assert!(matches!(
        outcomes[&current.id],
        ActionOutcome::Succeeded { .. }
    ));
}

#[test]
fn all_peer_tokens_are_cancelled_before_waiting_and_current_is_excluded() {
    let h = Harness::new(false);
    let mut olds = Vec::new();
    let mut cleanup = Vec::new();
    for binding in 1..=2 {
        let (host, entered, release) = cleanup_host();
        olds.push(h.start(binding, call("old"), Some(host)));
        cleanup.push((entered, release));
    }
    let current = h.start(3, call("stop"), None);
    current.go.send(()).unwrap();
    for (entered, release) in cleanup {
        entered.recv_timeout(WAIT).unwrap();
        assert!(olds.iter().all(|old| old.token.is_cancelled()));
        assert!(!current.token.is_cancelled());
        assert_eq!(h.runtime.commits.load(Ordering::SeqCst), 0);
        release.send(()).unwrap();
    }
    assert_eq!(h.reply().1.unwrap()["stoppedCount"], 2);
    for _ in 0..3 {
        h.terminal();
    }
}

#[test]
fn timeout_is_unknown_no_business_commit_and_no_automatic_retry() {
    let h = Harness::new(false);
    let (host, cleanup, release) = cleanup_host();
    let old = h.start(1, call("old"), Some(host));
    let mut request = call("stop");
    request.input["params"] = json!({"timeout_ms":10});
    let current = h.start(2, request, None);
    current.go.send(()).unwrap();
    assert_error(h.reply().1, "stop_timeout", "unknown");
    cleanup.recv_timeout(WAIT).unwrap();
    assert!(old.token.is_cancelled());
    assert!(!current.token.is_cancelled());
    assert!(active(&h, old.id));
    assert_eq!(h.runtime.commits.load(Ordering::SeqCst), 0);
    assert_eq!(h.runtime.calls.load(Ordering::SeqCst), 1);
    release.send(()).unwrap();
    for _ in 0..2 {
        h.terminal();
    }
    assert_eq!(h.runtime.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn single_instance_keeps_admission_rejection_and_denies_control_before_effects() {
    let h = Harness::new(true);
    let current = h.start(1, call("stop"), None);
    assert_eq!(
        h.service.submit(2, call("idle")).unwrap_err().code,
        "action_busy"
    );
    current.go.send(()).unwrap();
    assert_error(h.reply().1, "action_policy_conflict", "not_started");
    assert!(!current.token.is_cancelled());
    assert_eq!(h.runtime.commits.load(Ordering::SeqCst), 0);
    h.terminal();
}

#[test]
fn caller_cancellation_during_cleanup_wait_is_unknown_and_does_not_commit() {
    let h = Harness::new(false);
    let (host, cleanup, release) = cleanup_host();
    let old = h.start(1, call("old"), Some(host));
    let current = h.start(2, call("stop"), None);
    current.go.send(()).unwrap();
    cleanup.recv_timeout(WAIT).unwrap();
    assert_eq!(h.service.cancel_id(2), 1);
    assert_error(h.reply().1, "cancelled", "unknown");
    assert!(active(&h, old.id));
    assert_eq!(h.runtime.commits.load(Ordering::SeqCst), 0);
    release.send(()).unwrap();
    for _ in 0..2 {
        h.terminal();
    }
}

#[test]
fn disconnected_bridge_token_interrupts_wait_without_cancelling_current_service_token() {
    let h = Harness::new(false);
    let (host, cleanup, release) = cleanup_host();
    let old = h.start(1, call("old"), Some(host));
    let current = h.start(2, call("stop"), None);
    let scope = scoped_host(
        None,
        h.service.reservations.clone(),
        h.service.active_changed.clone(),
        current.id,
        None,
    );
    let bridge = CancellationToken::new();
    let worker_token = bridge.clone();
    let (reply_tx, reply) = mpsc::channel();
    let worker = thread::spawn(move || {
        reply_tx
            .send(scope.call(METHOD, json!({"timeout_ms":2000}), &worker_token))
            .unwrap();
    });
    cleanup.recv_timeout(WAIT).unwrap();
    bridge.cancel();
    assert_error(reply.recv_timeout(WAIT).unwrap(), "cancelled", "unknown");
    worker.join().unwrap();
    assert!(!current.token.is_cancelled());
    assert!(active(&h, old.id));
    release.send(()).unwrap();
    h.service.cancel_id(2);
    current.go.send(()).unwrap();
    assert_error(h.reply().1, "cancelled", "not_started");
    for _ in 0..2 {
        h.terminal();
    }
}

#[test]
fn invalid_ids_expired_deadline_and_pre_cancelled_caller_do_not_cancel_peers() {
    for variant in ["ids", "expired", "cancelled"] {
        let h = Harness::new(false);
        let old = h.start(1, call("old"), None);
        let mut request = call("stop");
        if variant == "ids" {
            request.input["params"] = json!({"timeout_ms":100,"invocation_id":old.id});
        }
        if variant == "expired" {
            request.input["expired"] = true.into();
        }
        let current = h.start(2, request, None);
        if variant == "cancelled" {
            h.service.cancel_id(2);
        }
        current.go.send(()).unwrap();
        let code = match variant {
            "ids" => "invalid_input",
            "expired" => "timeout",
            _ => "cancelled",
        };
        assert_error(h.reply().1, code, "not_started");
        assert!(!old.token.is_cancelled());
        assert_eq!(h.runtime.commits.load(Ordering::SeqCst), 0);
        h.service.cancel_all();
        for _ in 0..2 {
            h.terminal();
        }
    }
}

#[test]
fn stale_scope_or_readable_invocation_metadata_cannot_select_a_live_identity() {
    let h = Harness::new(false);
    let old = h.start(1, call("old"), None);
    let finished = h.start(2, call("idle"), None);
    wait_until(|| !active(&h, finished.id));
    let stale = scoped_host(
        None,
        h.service.reservations.clone(),
        h.service.active_changed.clone(),
        finished.id,
        None,
    );
    assert!(!stale.native_context_available());
    assert_error(
        stale.call(METHOD, json!({"timeout_ms":100}), &CancellationToken::new()),
        "action_inactive",
        "not_started",
    );
    assert_error(
        stale.call(
            METHOD,
            json!({"timeout_ms":100,"current_id":old.id}),
            &CancellationToken::new(),
        ),
        "invalid_input",
        "not_started",
    );
    assert!(!old.token.is_cancelled());
    h.service.cancel_all();
    for _ in 0..2 {
        h.terminal();
    }
}

#[test]
fn other_package_and_other_action_are_not_cancelled_and_zero_peers_is_explicit() {
    let h = Harness::new(false);
    let mut other_package = call("old");
    other_package.package_id = "other-package".into();
    let mut other_action = call("old");
    other_action.action_id = "other".into();
    let a = h.start(1, other_package, None);
    let b = h.start(2, other_action, None);
    let current = h.start(3, call("stop"), None);
    current.go.send(()).unwrap();
    assert_eq!(
        h.reply().1.unwrap(),
        json!({"stoppedCount":0,"effectState":"not_started"})
    );
    assert!(!a.token.is_cancelled());
    assert!(!b.token.is_cancelled());
    assert!(!current.token.is_cancelled());
    h.service.cancel_all();
    for _ in 0..3 {
        h.terminal();
    }
}

#[test]
fn two_internal_stop_calls_cannot_mutually_wait_after_one_caller_was_cancelled() {
    let h = Harness::new(false);
    let b = h.start(1, call("stop"), None);
    let a = h.start(2, call("stop"), None);
    a.go.send(()).unwrap();
    wait_until(|| b.token.is_cancelled());
    b.go.send(()).unwrap();
    let replies: BTreeMap<_, _> = (0..2).map(|_| h.reply()).collect();
    assert_error(replies[&b.id].clone(), "cancelled", "not_started");
    assert_eq!(
        replies[&a.id].clone().unwrap(),
        json!({"stoppedCount":1,"effectState":"unknown"})
    );
    assert!(!a.token.is_cancelled());
    assert_eq!(h.runtime.commits.load(Ordering::SeqCst), 1);
    for _ in 0..2 {
        h.terminal();
    }
}

#[test]
fn unrelated_inner_host_calls_keep_original_deadline_dispatch() {
    struct Inner(AtomicUsize);
    impl NativeHost for Inner {
        fn call(&self, _: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(json!("plain"))
        }
        fn call_with_deadline(
            &self,
            _: &str,
            _: Value,
            _: &CancellationToken,
            _: Instant,
        ) -> Result<Value, ActionError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(json!("deadline"))
        }
    }
    let h = Harness::new(false);
    let current = h.start(1, call("old"), None);
    let inner = Arc::new(Inner(AtomicUsize::new(0)));
    assert!(!inner.action_scope_available());
    let scoped = scoped_host(
        Some(inner.clone()),
        h.service.reservations.clone(),
        h.service.active_changed.clone(),
        current.id,
        None,
    );
    assert!(scoped.action_scope_available());
    let token = CancellationToken::new();
    assert_eq!(
        scoped.call("other", json!({}), &token).unwrap(),
        json!("plain")
    );
    assert_eq!(
        scoped
            .call_with_deadline("other", json!({}), &token, Instant::now() + WAIT)
            .unwrap(),
        json!("deadline")
    );
    assert_eq!(inner.0.load(Ordering::SeqCst), 2);
    h.service.cancel_all();
    h.terminal();
}
