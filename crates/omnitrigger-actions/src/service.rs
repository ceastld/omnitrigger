use crate::{model::valid_id, ActionCall, ActionError, NativeHost, PackageCatalog, ResolvedAction};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError},
        Arc, Mutex,
    },
    thread,
};

pub const CAPACITY: usize = 4;

/// 只读的动作包目录；引用检查不执行脚本，也不证明业务参数或本机能力可用。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActionCatalogSnapshot {
    pub schema_version: u32,
    pub actions: Vec<ActionCatalogDescription>,
    pub diagnostics: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActionCatalogDescription {
    pub call: ActionCall,
    pub package_name: String,
    pub title: String,
    pub runtime: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub single_instance: bool,
    pub reference_error: Option<ActionError>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvocationContext {
    pub invocation_id: u64,
    pub binding_id: u64,
}

#[derive(Clone, Debug, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
}

/// 业务执行适配点。实现负责其自身阻塞调用、输出上限以及取消收尾。
pub trait ActionRuntime: Send + Sync {
    fn execute(
        &self,
        action: &ResolvedAction,
        context: &InvocationContext,
        cancellation: &CancellationToken,
    ) -> Result<Value, ActionError>;

    fn execute_with_host(
        &self,
        action: &ResolvedAction,
        context: &InvocationContext,
        cancellation: &CancellationToken,
        _host: Option<&dyn NativeHost>,
    ) -> Result<Value, ActionError> {
        self.execute(action, context, cancellation)
    }
}

#[derive(Default)]
pub struct RuntimeRegistry {
    entries: BTreeMap<String, Arc<dyn ActionRuntime>>,
}

impl RuntimeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        id: &str,
        runtime: Arc<dyn ActionRuntime>,
    ) -> Result<(), ActionError> {
        if !valid_id(id) {
            return Err(ActionError::new("invalid_runtime", "运行时标识无效。"));
        }
        if self.entries.contains_key(id) {
            return Err(ActionError::new(
                "duplicate_runtime",
                format!("运行时 {id} 已注册。"),
            ));
        }
        self.entries.insert(id.into(), runtime);
        Ok(())
    }

    fn get(&self, id: &str) -> Result<Arc<dyn ActionRuntime>, ActionError> {
        self.entries.get(id).cloned().ok_or_else(|| {
            ActionError::new(
                "runtime_unavailable",
                format!("未注册动作运行时 {id}，原调用已保留。"),
            )
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ActionEvent {
    pub id: u64,
    pub invocation_id: u64,
    pub outcome: ActionOutcome,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ActionOutcome {
    Started,
    Succeeded { output: Value },
    Failed { error: ActionError },
    Cancelled,
}

impl ActionOutcome {
    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::Started)
    }
}

struct Reservation {
    binding_id: u64,
    cancellation: CancellationToken,
    identity: ActionIdentity,
    single_instance: bool,
    /// 从接纳到运行时及本次调用资源清理结束；不等UI读取终态，不由取消请求清除。
    active: bool,
}

#[derive(PartialEq, Eq)]
struct ActionIdentity {
    package_id: String,
    action_id: String,
}

struct Reservations {
    next_id: u64,
    entries: BTreeMap<u64, Reservation>,
}

struct ActiveInvocation {
    reservations: Arc<Mutex<Reservations>>,
    invocation_id: u64,
}

impl Drop for ActiveInvocation {
    fn drop(&mut self) {
        let mut reserved = self
            .reservations
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(entry) = reserved.entries.get_mut(&self.invocation_id) {
            entry.active = false;
        }
    }
}

/// 字段按声明顺序清理：active必须最后，确保旧调用资源释放后才接纳同动作。
struct InvocationTask {
    action: ResolvedAction,
    runtime: Arc<dyn ActionRuntime>,
    host: Option<Arc<dyn NativeHost>>,
    context: InvocationContext,
    cancellation: CancellationToken,
    _active: ActiveInvocation,
}

impl InvocationTask {
    fn run(self, events: SyncSender<ActionEvent>) {
        let id = self.context.binding_id;
        let invocation_id = self.context.invocation_id;
        if events
            .send(ActionEvent {
                id,
                invocation_id,
                outcome: ActionOutcome::Started,
            })
            .is_err()
        {
            return;
        }
        let cancellation = self.cancellation.clone();
        let outcome = if cancellation.is_cancelled() {
            ActionOutcome::Cancelled
        } else {
            match catch_unwind(AssertUnwindSafe(|| {
                self.runtime.execute_with_host(
                    &self.action,
                    &self.context,
                    &cancellation,
                    self.host.as_deref(),
                )
            })) {
                Ok(Ok(output)) => ActionOutcome::Succeeded { output },
                Ok(Err(error)) if error.code == "cancelled" => ActionOutcome::Cancelled,
                Ok(Err(error)) => ActionOutcome::Failed { error },
                Err(_) => runtime_panic(),
            }
        };
        // 包括reload后旧运行时最后一份Arc；此处不持有服务锁，不等待UI读终态。
        let cleaned = catch_unwind(AssertUnwindSafe(|| drop(self)));
        let outcome = if cleaned.is_err() {
            runtime_panic()
        } else if cancellation.is_cancelled() {
            ActionOutcome::Cancelled
        } else {
            outcome
        };
        let _ = events.send(ActionEvent {
            id,
            invocation_id,
            outcome,
        });
    }
}

fn runtime_panic() -> ActionOutcome {
    ActionOutcome::Failed {
        error: ActionError::new("runtime_panic", "动作运行时异常退出，本次调用未重试。"),
    }
}

/// 有界的应用调用服务；正在运行和终态尚未被消费的调用共用配额。
/// 不在 Drop 中等待用户运行时，不重试已经接纳的带副作用调用。
pub struct ActionService {
    catalog: PackageCatalog,
    runtimes: RuntimeRegistry,
    reservations: Arc<Mutex<Reservations>>,
    events_tx: SyncSender<ActionEvent>,
    events_rx: Receiver<ActionEvent>,
}

impl ActionService {
    pub fn new(catalog: PackageCatalog, runtimes: RuntimeRegistry) -> Self {
        // 每个配额恰有一次 Started 和一次终态，不会产生无限进度日志。
        let (events_tx, events_rx) = mpsc::sync_channel(CAPACITY * 2);
        Self {
            catalog,
            runtimes,
            reservations: Arc::new(Mutex::new(Reservations {
                next_id: 1,
                entries: BTreeMap::new(),
            })),
            events_tx,
            events_rx,
        }
    }

    pub fn catalog(&self) -> &PackageCatalog {
        &self.catalog
    }

    /// 只替换后续调用使用的快照；已接纳的调用持有自己的运行时，不清空配额或事件。
    pub fn reload(&mut self, catalog: PackageCatalog, runtimes: RuntimeRegistry) {
        self.catalog = catalog;
        self.runtimes = runtimes;
    }

    pub fn describe_catalog(&self) -> ActionCatalogSnapshot {
        ActionCatalogSnapshot {
            schema_version: 1,
            actions: self
                .catalog
                .actions
                .iter()
                .map(|entry| {
                    let call = ActionCall {
                        package_id: entry.package_id.clone(),
                        action_id: entry.definition.id.clone(),
                        action_version: entry.definition.version,
                        input: serde_json::json!({}),
                    };
                    let reference_error = self.validate(&call).err();
                    ActionCatalogDescription {
                        call,
                        package_name: entry.package_name.clone(),
                        title: entry.definition.title.clone(),
                        runtime: entry.definition.runtime.clone(),
                        single_instance: entry.definition.single_instance,
                        reference_error,
                    }
                })
                .collect(),
            diagnostics: self.catalog.diagnostics.clone(),
        }
    }

    pub fn validate(&self, call: &ActionCall) -> Result<(), ActionError> {
        let resolved = self.catalog.resolve(call)?;
        self.runtimes.get(&resolved.definition.runtime)?;
        Ok(())
    }

    pub fn submit(&self, id: u64, call: ActionCall) -> Result<u64, ActionError> {
        self.submit_with_host(id, call, None)
    }

    pub fn submit_with_host(
        &self,
        id: u64,
        call: ActionCall,
        host: Option<Arc<dyn NativeHost>>,
    ) -> Result<u64, ActionError> {
        self.submit_with_host_and_spawn(id, call, host, |builder, task| {
            builder.spawn(task).map(|_| ())
        })
    }

    fn submit_with_host_and_spawn(
        &self,
        id: u64,
        call: ActionCall,
        host: Option<Arc<dyn NativeHost>>,
        spawn: impl FnOnce(thread::Builder, Box<dyn FnOnce() + Send>) -> std::io::Result<()>,
    ) -> Result<u64, ActionError> {
        let action = self.catalog.resolve(&call)?;
        let runtime = self.runtimes.get(&action.definition.runtime)?;
        let identity = ActionIdentity {
            package_id: action.call.package_id.clone(),
            action_id: action.call.action_id.clone(),
        };
        let cancellation = CancellationToken::new();
        let invocation_id = {
            let mut reserved = self
                .reservations
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if reserved
                .entries
                .values()
                .any(|entry| entry.binding_id == id)
            {
                return Err(ActionError::new(
                    "binding_busy",
                    "此动作的上次调用尚未结束或终态尚未读取。",
                ));
            }
            if reserved.entries.values().any(|entry| {
                entry.active
                    && entry.identity == identity
                    && (entry.single_instance || action.definition.single_instance)
            }) {
                let mut error = ActionError::new("action_busy", "此动作正在执行，请结束后再试。");
                error.effect_state = Some("not_started".into());
                return Err(error);
            }
            if reserved.entries.len() >= CAPACITY {
                return Err(ActionError::new(
                    "capacity_exceeded",
                    "动作执行配额已满，请稍后重试。",
                ));
            }
            let invocation_id = reserved.next_id;
            reserved.next_id = invocation_id.checked_add(1).ok_or_else(|| {
                ActionError::new("invocation_limit", "动作调用标识已耗尽，请重启应用。")
            })?;
            reserved.entries.insert(
                invocation_id,
                Reservation {
                    binding_id: id,
                    cancellation: cancellation.clone(),
                    identity,
                    single_instance: action.definition.single_instance,
                    active: true,
                },
            );
            invocation_id
        };
        let events = self.events_tx.clone();
        let context = InvocationContext {
            invocation_id,
            binding_id: id,
        };
        let task = InvocationTask {
            action,
            runtime,
            host,
            context,
            cancellation,
            _active: ActiveInvocation {
                reservations: self.reservations.clone(),
                invocation_id,
            },
        };
        let spawned = catch_unwind(AssertUnwindSafe(|| {
            spawn(
                thread::Builder::new().name(format!("omnitrigger-action-{invocation_id}")),
                Box::new(move || task.run(events)),
            )
        }))
        .unwrap_or_else(|_| Err(std::io::Error::other("动作工作线程启动或启动清理异常")));
        if let Err(error) = spawned {
            self.reservations
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .entries
                .remove(&invocation_id);
            return Err(ActionError::new(
                "worker_start_failed",
                format!("启动动作工作线程失败：{error}"),
            ));
        }
        Ok(invocation_id)
    }

    pub fn try_recv(&self) -> Result<ActionEvent, TryRecvError> {
        let event = self.events_rx.try_recv()?;
        if event.outcome.is_terminal() {
            self.reservations
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .entries
                .remove(&event.invocation_id);
        }
        Ok(event)
    }

    /// 绑定仍占用配额（包含终态尚未读取），与动作级实际活跃状态分开。
    pub fn is_running(&self, id: u64) -> bool {
        self.reservations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entries
            .values()
            .any(|entry| entry.binding_id == id)
    }

    pub fn cancel_id(&self, id: u64) -> usize {
        let reserved = self
            .reservations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut count = 0;
        for reservation in reserved
            .entries
            .values()
            .filter(|entry| entry.binding_id == id)
        {
            reservation.cancellation.cancel();
            count += 1;
        }
        count
    }

    pub fn cancel_all(&self) -> usize {
        let reserved = self
            .reservations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for reservation in reserved.entries.values() {
            reservation.cancellation.cancel();
        }
        reserved.entries.len()
    }
}

impl Drop for ActionService {
    fn drop(&mut self) {
        self.cancel_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::tests::Fixture;
    use serde_json::json;
    use std::time::{Duration, Instant};

    enum RuntimeExit {
        Success,
        Failure,
        Panic,
    }

    struct ControlledRuntime {
        entered: SyncSender<u64>,
        exits: Mutex<Receiver<RuntimeExit>>,
    }

    impl ActionRuntime for ControlledRuntime {
        fn execute(
            &self,
            _: &ResolvedAction,
            context: &InvocationContext,
            _: &CancellationToken,
        ) -> Result<Value, ActionError> {
            self.entered.send(context.invocation_id).unwrap();
            let exit = self
                .exits
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            match exit {
                RuntimeExit::Success => Ok(json!({"done":true})),
                RuntimeExit::Failure => Err(ActionError::new("controlled_failure", "测试失败")),
                RuntimeExit::Panic => panic!("受控运行时异常"),
            }
        }
    }

    fn controlled_runtime() -> (
        Arc<ControlledRuntime>,
        Receiver<u64>,
        SyncSender<RuntimeExit>,
    ) {
        let (entered, receiver) = mpsc::sync_channel(CAPACITY);
        let (sender, exits) = mpsc::sync_channel(CAPACITY);
        (
            Arc::new(ControlledRuntime {
                entered,
                exits: Mutex::new(exits),
            }),
            receiver,
            sender,
        )
    }

    fn policy_catalog(
        fixture: &Fixture,
        enabled: bool,
        version: u32,
        runtime: &str,
    ) -> PackageCatalog {
        let path = fixture.0.join("sample/action.json");
        let mut manifest: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        manifest["actions"][0]["single_instance"] = enabled.into();
        manifest["actions"][0]["version"] = version.into();
        manifest["actions"][0]["runtime"] = runtime.into();
        std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        PackageCatalog::load(&fixture.0)
    }

    fn policy_service(enabled: bool, runtime: Arc<dyn ActionRuntime>) -> (Fixture, ActionService) {
        let fixture = Fixture::new();
        fixture.package("sample", "example", "test");
        let mut registry = RuntimeRegistry::new();
        registry.register("test", runtime).unwrap();
        let catalog = policy_catalog(&fixture, enabled, 1, "test");
        (fixture, ActionService::new(catalog, registry))
    }

    fn wait_inactive(service: &ActionService, invocation: u64) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let active = service.reservations.lock().unwrap().entries[&invocation].active;
            if !active {
                return;
            }
            assert!(Instant::now() < deadline, "等待实际执行清理结束超时");
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn assert_action_busy(service: &ActionService, binding: u64, call: ActionCall) {
        let error = service.submit(binding, call).unwrap_err();
        assert_eq!(error.code, "action_busy");
        assert_eq!(error.effect_state.as_deref(), Some("not_started"));
    }

    #[test]
    fn single_instance_rejects_cross_binding_overlap_but_not_unread_finished_results() {
        let (runtime, entered, exits) = controlled_runtime();
        let (_fixture, service) = policy_service(true, runtime);
        let first = service.submit(1, Fixture::call()).unwrap();
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        let mut changed_input = Fixture::call();
        changed_input.input = json!({"different":true});
        assert_action_busy(&service, 2, changed_input);
        assert_eq!(
            service.submit(1, Fixture::call()).unwrap_err().code,
            "binding_busy"
        );
        exits.send(RuntimeExit::Success).unwrap();
        wait_inactive(&service, first);
        assert!(service.is_running(1)); // 旧绑定/配额仍等待读取终态，动作已可再次执行。
        let second = service.submit(2, Fixture::call()).unwrap();
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(second > first);
        exits.send(RuntimeExit::Success).unwrap();
        for _ in 0..2 {
            assert!(matches!(
                terminal(&service).outcome,
                ActionOutcome::Succeeded { .. }
            ));
        }
    }

    #[test]
    fn reloading_true_to_false_preserves_old_policy_and_ignores_version_runtime_input_for_identity()
    {
        let (runtime, entered, exits) = controlled_runtime();
        let (fixture, mut service) = policy_service(true, runtime);
        let first = service.submit(1, Fixture::call()).unwrap();
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        let catalog = policy_catalog(&fixture, false, 2, "replacement");
        let mut registry = RuntimeRegistry::new();
        registry.register("replacement", Arc::new(Echo)).unwrap();
        service.reload(catalog, registry);
        let call = ActionCall {
            action_version: 2,
            input: json!({"new":1}),
            ..Fixture::call()
        };
        assert_action_busy(&service, 2, call.clone());
        assert_eq!(service.cancel_id(1), 1);
        assert_action_busy(&service, 2, call.clone());
        exits.send(RuntimeExit::Success).unwrap();
        wait_inactive(&service, first);
        service.submit(2, call).unwrap();
        let outcomes: Vec<_> = (0..2).map(|_| terminal(&service).outcome).collect();
        assert!(outcomes
            .iter()
            .any(|outcome| *outcome == ActionOutcome::Cancelled));
        assert!(outcomes
            .iter()
            .any(|outcome| matches!(outcome, ActionOutcome::Succeeded { .. })));
    }

    #[test]
    fn reloading_false_to_true_checks_every_preexisting_parallel_invocation() {
        let (runtime, entered, exits) = controlled_runtime();
        let (fixture, mut service) = policy_service(false, runtime.clone());
        let first = service.submit(1, Fixture::call()).unwrap();
        let second = service.submit(2, Fixture::call()).unwrap();
        for _ in 0..2 {
            entered.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        let catalog = policy_catalog(&fixture, true, 1, "test");
        let mut registry = RuntimeRegistry::new();
        registry.register("test", runtime).unwrap();
        service.reload(catalog, registry);
        assert_action_busy(&service, 3, Fixture::call());
        exits.send(RuntimeExit::Success).unwrap();
        // 只等最先持有recv锁的调用清理；另一个旧false仍活跃时也不能接纳true。
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let active = service
                .reservations
                .lock()
                .unwrap()
                .entries
                .values()
                .filter(|entry| entry.active)
                .count();
            if active == 1 {
                break;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        assert_action_busy(&service, 3, Fixture::call());
        exits.send(RuntimeExit::Success).unwrap();
        wait_inactive(&service, first);
        wait_inactive(&service, second);
        service.submit(3, Fixture::call()).unwrap();
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        exits.send(RuntimeExit::Success).unwrap();
        for _ in 0..3 {
            terminal(&service);
        }
    }

    #[test]
    fn single_instance_releases_on_success_failure_and_panic_before_terminal_ack() {
        for exit in [
            RuntimeExit::Success,
            RuntimeExit::Failure,
            RuntimeExit::Panic,
        ] {
            let expected_error = match exit {
                RuntimeExit::Success => None,
                RuntimeExit::Failure => Some("controlled_failure"),
                RuntimeExit::Panic => Some("runtime_panic"),
            };
            let (runtime, entered, exits) = controlled_runtime();
            let (_fixture, service) = policy_service(true, runtime);
            let first = service.submit(1, Fixture::call()).unwrap();
            entered.recv_timeout(Duration::from_secs(5)).unwrap();
            exits.send(exit).unwrap();
            wait_inactive(&service, first);
            assert!(service.is_running(1));
            service.submit(2, Fixture::call()).unwrap();
            entered.recv_timeout(Duration::from_secs(5)).unwrap();
            exits.send(RuntimeExit::Success).unwrap();
            let outcomes: Vec<_> = (0..2).map(|_| terminal(&service)).collect();
            let original = &outcomes
                .iter()
                .find(|event| event.invocation_id == first)
                .unwrap()
                .outcome;
            match expected_error {
                Some(code) => assert!(
                    matches!(original, ActionOutcome::Failed { error } if error.code == code)
                ),
                None => assert!(matches!(original, ActionOutcome::Succeeded { .. })),
            }
        }
    }

    #[test]
    fn completed_single_instances_still_obey_original_four_unread_reservations() {
        let (_fixture, service) = policy_service(true, Arc::new(Echo));
        for binding in 1..=CAPACITY as u64 {
            let invocation = service.submit(binding, Fixture::call()).unwrap();
            wait_inactive(&service, invocation);
        }
        assert_eq!(
            service.submit(1, Fixture::call()).unwrap_err().code,
            "binding_busy"
        );
        assert_eq!(
            service.submit(99, Fixture::call()).unwrap_err().code,
            "capacity_exceeded"
        );
        for _ in 0..CAPACITY {
            terminal(&service);
        }
    }

    #[test]
    fn action_and_package_identity_are_independent_within_one_service() {
        let (runtime, entered, exits) = controlled_runtime();
        let (fixture, mut service) = policy_service(true, runtime.clone());
        fixture.package("other", "other", "test");
        let path = fixture.0.join("sample/action.json");
        let mut manifest: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let mut second = manifest["actions"][0].clone();
        second["id"] = "second".into();
        manifest["actions"].as_array_mut().unwrap().push(second);
        std::fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let mut registry = RuntimeRegistry::new();
        registry.register("test", runtime).unwrap();
        service.reload(PackageCatalog::load(&fixture.0), registry);
        for (binding, call) in [
            (1, Fixture::call()),
            (
                2,
                ActionCall {
                    action_id: "second".into(),
                    ..Fixture::call()
                },
            ),
            (
                3,
                ActionCall {
                    package_id: "other".into(),
                    ..Fixture::call()
                },
            ),
        ] {
            service.submit(binding, call).unwrap();
        }
        for _ in 0..3 {
            entered.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        assert_action_busy(&service, 4, Fixture::call());
        for _ in 0..3 {
            exits.send(RuntimeExit::Success).unwrap();
        }
        for _ in 0..3 {
            terminal(&service);
        }
    }

    struct DropHost {
        entered: SyncSender<()>,
        release: Receiver<()>,
    }

    // Receiver本身非Sync，因此把清理接收器单独放在Mutex中。
    struct ControlledDropHost(Mutex<DropHost>);
    impl NativeHost for ControlledDropHost {
        fn call(&self, _: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
            Ok(json!({}))
        }
    }
    impl Drop for ControlledDropHost {
        fn drop(&mut self) {
            let host = self.0.get_mut().unwrap();
            host.entered.send(()).unwrap();
            host.release.recv_timeout(Duration::from_secs(5)).unwrap();
        }
    }

    #[test]
    fn per_invocation_host_cleanup_keeps_action_active_and_cancel_does_not_unlock_early() {
        let (entered_tx, entered) = mpsc::sync_channel(1);
        let (release, release_rx) = mpsc::sync_channel(1);
        let host = Arc::new(ControlledDropHost(Mutex::new(DropHost {
            entered: entered_tx,
            release: release_rx,
        })));
        let (_fixture, service) = policy_service(true, Arc::new(Echo));
        let first = service
            .submit_with_host(1, Fixture::call(), Some(host))
            .unwrap();
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_action_busy(&service, 2, Fixture::call());
        service.cancel_id(1);
        assert_action_busy(&service, 2, Fixture::call());
        release.send(()).unwrap();
        wait_inactive(&service, first);
        service.submit(2, Fixture::call()).unwrap();
        let outcomes: Vec<_> = (0..2).map(|_| terminal(&service).outcome).collect();
        assert!(outcomes.contains(&ActionOutcome::Cancelled));
    }

    struct RuntimeDropCleanup {
        entered: SyncSender<()>,
        execute_release: Mutex<Receiver<()>>,
        drop_entered: SyncSender<()>,
        drop_release: Receiver<()>,
    }
    struct CleanupRuntime(Mutex<RuntimeDropCleanup>);
    impl ActionRuntime for CleanupRuntime {
        fn execute(
            &self,
            _: &ResolvedAction,
            _: &InvocationContext,
            _: &CancellationToken,
        ) -> Result<Value, ActionError> {
            let state = self.0.lock().unwrap();
            state.entered.send(()).unwrap();
            state
                .execute_release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            Ok(json!({}))
        }
    }
    impl Drop for CleanupRuntime {
        fn drop(&mut self) {
            let state = self.0.get_mut().unwrap();
            state.drop_entered.send(()).unwrap();
            state
                .drop_release
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
        }
    }

    #[test]
    fn reload_cannot_overlap_last_old_runtime_arc_cleanup_with_new_runtime_execution() {
        let (entered_tx, entered) = mpsc::sync_channel(1);
        let (execute_release, execute_rx) = mpsc::sync_channel(1);
        let (drop_tx, dropping) = mpsc::sync_channel(1);
        let (drop_release, drop_rx) = mpsc::sync_channel(1);
        let runtime = Arc::new(CleanupRuntime(Mutex::new(RuntimeDropCleanup {
            entered: entered_tx,
            execute_release: Mutex::new(execute_rx),
            drop_entered: drop_tx,
            drop_release: drop_rx,
        })));
        let (fixture, mut service) = policy_service(true, runtime);
        let first = service.submit(1, Fixture::call()).unwrap();
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        let catalog = policy_catalog(&fixture, false, 2, "replacement");
        let mut registry = RuntimeRegistry::new();
        registry.register("replacement", Arc::new(Echo)).unwrap();
        service.reload(catalog, registry);
        execute_release.send(()).unwrap();
        dropping.recv_timeout(Duration::from_secs(5)).unwrap();
        let next_call = ActionCall {
            action_version: 2,
            ..Fixture::call()
        };
        assert_action_busy(&service, 2, next_call.clone());
        assert!(service.is_running(1));
        drop_release.send(()).unwrap();
        wait_inactive(&service, first);
        service.submit(2, next_call).unwrap();
        for _ in 0..2 {
            terminal(&service);
        }
    }

    #[test]
    fn pre_start_cancellation_and_worker_start_failure_release_actual_occupancy() {
        struct CountExecutions(Arc<std::sync::atomic::AtomicUsize>);
        impl ActionRuntime for CountExecutions {
            fn execute(
                &self,
                _: &ResolvedAction,
                _: &InvocationContext,
                _: &CancellationToken,
            ) -> Result<Value, ActionError> {
                self.0.fetch_add(1, Ordering::AcqRel);
                Ok(json!({}))
            }
        }
        let executions = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (_fixture, service) =
            policy_service(true, Arc::new(CountExecutions(executions.clone())));
        let mut pending = None;
        let first = service
            .submit_with_host_and_spawn(1, Fixture::call(), None, |_, task| {
                pending = Some(task);
                Ok(())
            })
            .unwrap();
        service.cancel_id(1);
        assert_action_busy(&service, 2, Fixture::call());
        pending.take().unwrap()();
        wait_inactive(&service, first);
        assert_eq!(executions.load(Ordering::Acquire), 0);
        assert_eq!(terminal(&service).outcome, ActionOutcome::Cancelled);
        service.submit(2, Fixture::call()).unwrap();
        terminal(&service);
        let error = service
            .submit_with_host_and_spawn(3, Fixture::call(), None, |_, task| {
                drop(task);
                Err(std::io::Error::other("受控线程创建失败"))
            })
            .unwrap_err();
        assert_eq!(error.code, "worker_start_failed");
        assert!(!service.is_running(3));
        assert!(service.reservations.lock().unwrap().entries.is_empty());
        service.submit(3, Fixture::call()).unwrap();
        terminal(&service);
    }

    #[test]
    fn started_channel_failure_still_releases_worker_resources_and_active_marker() {
        let (_fixture, service) = policy_service(true, Arc::new(Echo));
        let ledger = service.reservations.clone();
        let mut pending = None;
        let first = service
            .submit_with_host_and_spawn(1, Fixture::call(), None, |_, task| {
                pending = Some(task);
                Ok(())
            })
            .unwrap();
        drop(service);
        pending.take().unwrap()();
        assert!(!ledger.lock().unwrap().entries[&first].active);
    }

    struct FlagHost(Arc<AtomicBool>);
    impl NativeHost for FlagHost {
        fn call(&self, _: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
            Ok(json!({}))
        }
    }
    impl Drop for FlagHost {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    #[test]
    fn worker_start_failure_drops_owned_host_and_removes_both_occupancies() {
        let (_fixture, service) = policy_service(true, Arc::new(Echo));
        let cleaned = Arc::new(AtomicBool::new(false));
        let error = service
            .submit_with_host_and_spawn(
                1,
                Fixture::call(),
                Some(Arc::new(FlagHost(cleaned.clone()))),
                |_, task| {
                    drop(task);
                    Err(std::io::Error::other("受控线程创建失败"))
                },
            )
            .unwrap_err();
        assert_eq!(error.code, "worker_start_failed");
        assert!(cleaned.load(Ordering::Acquire));
        assert!(service.reservations.lock().unwrap().entries.is_empty());
        service.submit(1, Fixture::call()).unwrap();
        terminal(&service);
    }

    struct PanicCleanupHost;
    impl NativeHost for PanicCleanupHost {
        fn call(&self, _: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
            Ok(json!({}))
        }
    }
    impl Drop for PanicCleanupHost {
        fn drop(&mut self) {
            panic!("受控宿主清理异常");
        }
    }

    #[test]
    fn cleanup_panic_releases_active_marker_and_publishes_one_failure() {
        let (_fixture, service) = policy_service(true, Arc::new(Echo));
        let first = service
            .submit_with_host(1, Fixture::call(), Some(Arc::new(PanicCleanupHost)))
            .unwrap();
        wait_inactive(&service, first);
        let failure = terminal(&service);
        assert_eq!(failure.invocation_id, first);
        let ActionOutcome::Failed { error } = failure.outcome else {
            panic!("应收到清理异常")
        };
        assert_eq!(error.code, "runtime_panic");
        service.submit(1, Fixture::call()).unwrap();
        terminal(&service);
        assert!(matches!(service.try_recv(), Err(TryRecvError::Empty)));
    }

    #[test]
    fn spawn_failure_cleanup_panic_does_not_leave_an_unacknowledgeable_reservation() {
        let (_fixture, service) = policy_service(true, Arc::new(Echo));
        let error = service
            .submit_with_host_and_spawn(
                1,
                Fixture::call(),
                Some(Arc::new(PanicCleanupHost)),
                |_, task| {
                    drop(task);
                    Err(std::io::Error::other("受控线程创建失败"))
                },
            )
            .unwrap_err();
        assert_eq!(error.code, "worker_start_failed");
        assert!(service.reservations.lock().unwrap().entries.is_empty());
        assert!(matches!(service.try_recv(), Err(TryRecvError::Empty)));
        service.submit(1, Fixture::call()).unwrap();
        terminal(&service);
    }

    #[test]
    fn single_instance_scope_is_one_service_not_a_global_lock() {
        let (runtime, entered, exits) = controlled_runtime();
        let (_first_fixture, first_service) = policy_service(true, runtime.clone());
        let (_second_fixture, second_service) = policy_service(true, runtime);
        first_service.submit(1, Fixture::call()).unwrap();
        second_service.submit(1, Fixture::call()).unwrap();
        for _ in 0..2 {
            entered.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        for _ in 0..2 {
            exits.send(RuntimeExit::Success).unwrap();
        }
        terminal(&first_service);
        terminal(&second_service);
    }

    #[test]
    fn catalog_single_instance_true_is_visible_and_default_false_json_is_unchanged() {
        let (fixture, mut service) = policy_service(true, Arc::new(Echo));
        let enabled = service.describe_catalog();
        assert!(enabled.actions[0].single_instance);
        assert_eq!(
            serde_json::to_value(&enabled).unwrap()["actions"][0]["single_instance"],
            true
        );
        let catalog = policy_catalog(&fixture, false, 1, "test");
        let mut registry = RuntimeRegistry::new();
        registry.register("test", Arc::new(Echo)).unwrap();
        service.reload(catalog, registry);
        let json = serde_json::to_value(service.describe_catalog()).unwrap();
        assert!(json["actions"][0].get("single_instance").is_none());
        assert!(
            !serde_json::from_value::<ActionCatalogSnapshot>(json)
                .unwrap()
                .actions[0]
                .single_instance
        );
    }

    struct Echo;
    impl ActionRuntime for Echo {
        fn execute(
            &self,
            action: &ResolvedAction,
            context: &InvocationContext,
            _: &CancellationToken,
        ) -> Result<Value, ActionError> {
            Ok(
                json!({"input":action.call.input,"binding":context.binding_id,"invocation":context.invocation_id}),
            )
        }
    }

    struct Wait;
    impl ActionRuntime for Wait {
        fn execute(
            &self,
            _: &ResolvedAction,
            _: &InvocationContext,
            cancellation: &CancellationToken,
        ) -> Result<Value, ActionError> {
            while !cancellation.is_cancelled() {
                thread::sleep(Duration::from_millis(5));
            }
            Err(ActionError::new("cancelled", "测试取消"))
        }
    }

    fn service(runtime: Arc<dyn ActionRuntime>) -> (Fixture, ActionService) {
        let fixture = Fixture::new();
        fixture.package("sample", "example", "test");
        let mut registry = RuntimeRegistry::new();
        registry.register("test", runtime).unwrap();
        let service = ActionService::new(PackageCatalog::load(&fixture.0), registry);
        (fixture, service)
    }

    fn terminal(service: &ActionService) -> ActionEvent {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match service.try_recv() {
                Ok(event) if event.outcome.is_terminal() => return event,
                Ok(_) | Err(TryRecvError::Empty) => {}
                Err(error) => panic!("动作事件通道退出：{error}"),
            }
            assert!(Instant::now() < deadline, "等待动作终态超时");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn accepted_invocations_are_distinct_and_unread_results_hold_capacity() {
        let (_fixture, service) = service(Arc::new(Echo));
        let mut invocations = Vec::new();
        for id in 1..=CAPACITY as u64 {
            invocations.push(service.submit(id, Fixture::call()).unwrap());
        }
        assert_eq!(
            service.submit(1, Fixture::call()).unwrap_err().code,
            "binding_busy"
        );
        assert_eq!(
            service.submit(99, Fixture::call()).unwrap_err().code,
            "capacity_exceeded"
        );
        let event = terminal(&service);
        assert!(invocations.contains(&event.invocation_id));
        assert!(!service.is_running(event.id));
        let ActionOutcome::Succeeded { output } = event.outcome else {
            panic!("意外终态")
        };
        assert_eq!(output["binding"], event.id);
        assert_eq!(output["invocation"], event.invocation_id);
        let next = service.submit(event.id, Fixture::call()).unwrap();
        assert!(!invocations.contains(&next));
        for _ in 0..CAPACITY {
            terminal(&service);
        }
    }

    #[test]
    fn reload_preserves_old_workers_results_reservations_and_invocation_ids() {
        let (fixture, mut service) = service(Arc::new(Wait));
        let old = service.submit(1, Fixture::call()).unwrap();
        service.reload(PackageCatalog::default(), RuntimeRegistry::new());
        assert!(service.is_running(1));
        assert_eq!(
            service.validate(&Fixture::call()).unwrap_err().code,
            "package_missing"
        );
        let mut registry = RuntimeRegistry::new();
        registry.register("test", Arc::new(Echo)).unwrap();
        service.reload(PackageCatalog::load(&fixture.0), registry);
        let next = service.submit(2, Fixture::call()).unwrap();
        assert!(next > old);
        let event = terminal(&service);
        assert_eq!(event.id, 2);
        assert_eq!(event.invocation_id, next);
        assert!(matches!(event.outcome, ActionOutcome::Succeeded { .. }));
        assert!(service.is_running(1));
        assert_eq!(service.cancel_id(1), 1);
        let event = terminal(&service);
        assert_eq!(event.id, 1);
        assert_eq!(event.invocation_id, old);
        assert_eq!(event.outcome, ActionOutcome::Cancelled);
        assert!(!service.is_running(1));
    }

    #[test]
    fn cancellation_keeps_occupancy_until_terminal_and_drop_does_not_join() {
        let (_fixture, service) = service(Arc::new(Wait));
        service.submit(1, Fixture::call()).unwrap();
        service.submit(2, Fixture::call()).unwrap();
        assert_eq!(service.cancel_id(1), 1);
        assert!(service.is_running(1));
        assert_eq!(terminal(&service).outcome, ActionOutcome::Cancelled);
        assert!(!service.is_running(1));
        assert_eq!(service.cancel_all(), 1);
        assert_eq!(terminal(&service).outcome, ActionOutcome::Cancelled);
        service.submit(3, Fixture::call()).unwrap();
        let start = Instant::now();
        drop(service);
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn runtime_panics_become_one_failure_and_do_not_leak_reservations() {
        struct Panics;
        impl ActionRuntime for Panics {
            fn execute(
                &self,
                _: &ResolvedAction,
                _: &InvocationContext,
                _: &CancellationToken,
            ) -> Result<Value, ActionError> {
                panic!("模拟运行时异常")
            }
        }
        let (_fixture, service) = service(Arc::new(Panics));
        service.submit(1, Fixture::call()).unwrap();
        let ActionOutcome::Failed { error } = terminal(&service).outcome else {
            panic!("预期异常失败")
        };
        assert_eq!(error.code, "runtime_panic");
        assert!(!service.is_running(1));
        service.submit(1, Fixture::call()).unwrap();
        terminal(&service);
    }

    #[test]
    fn unsupported_runtime_and_duplicate_registration_are_explicit() {
        let fixture = Fixture::new();
        fixture.package("sample", "example", "future");
        let mut registry = RuntimeRegistry::new();
        registry.register("test", Arc::new(Echo)).unwrap();
        assert_eq!(
            registry.register("test", Arc::new(Echo)).unwrap_err().code,
            "duplicate_runtime"
        );
        let service = ActionService::new(PackageCatalog::load(&fixture.0), registry);
        assert_eq!(
            service.validate(&Fixture::call()).unwrap_err().code,
            "runtime_unavailable"
        );
        assert_eq!(
            service.submit(1, Fixture::call()).unwrap_err().code,
            "runtime_unavailable"
        );
        assert!(!service.is_running(1));
    }

    #[test]
    fn catalog_description_is_a_serializable_snapshot_without_execution() {
        struct NeverExecute;
        impl ActionRuntime for NeverExecute {
            fn execute(
                &self,
                _: &ResolvedAction,
                _: &InvocationContext,
                _: &CancellationToken,
            ) -> Result<Value, ActionError> {
                panic!("目录描述不能执行动作")
            }
        }
        let fixture = Fixture::new();
        fixture.package("installed", "example", "test");
        fixture.package("future", "future", "missing-runtime");
        let mut registry = RuntimeRegistry::new();
        registry.register("test", Arc::new(NeverExecute)).unwrap();
        let mut catalog = PackageCatalog::load(&fixture.0);
        catalog.diagnostics.push("诊断原样保留".into());
        let service = ActionService::new(catalog, registry);
        drop(fixture); // 描述使用已加载快照，不重新读目录、脚本或用户配置。
        let snapshot = service.describe_catalog();
        assert_eq!(snapshot.schema_version, 1);
        assert_eq!(snapshot.actions.len(), 2);
        let installed = snapshot
            .actions
            .iter()
            .find(|x| x.call.package_id == "example")
            .unwrap();
        assert_eq!(installed.call, Fixture::call());
        assert!(installed.reference_error.is_none());
        let future = snapshot
            .actions
            .iter()
            .find(|x| x.call.package_id == "future")
            .unwrap();
        assert_eq!(
            future.reference_error.as_ref().unwrap().code,
            "runtime_unavailable"
        );
        assert_eq!(snapshot.diagnostics.last().unwrap(), "诊断原样保留");
        let json = serde_json::to_string(&snapshot).unwrap();
        assert_eq!(
            serde_json::from_str::<ActionCatalogSnapshot>(&json).unwrap(),
            snapshot
        );
        assert!(matches!(service.try_recv(), Err(TryRecvError::Empty)));
    }

    #[test]
    fn registering_a_new_runtime_executes_the_existing_call_and_manifest_schema() {
        struct Workflow;
        impl ActionRuntime for Workflow {
            fn execute(
                &self,
                action: &ResolvedAction,
                _: &InvocationContext,
                _: &CancellationToken,
            ) -> Result<Value, ActionError> {
                Ok(json!({"workflow_result": action.call.input["value"]}))
            }
        }
        let fixture = Fixture::new();
        fixture.package("sample", "example", "workflow-test");
        let catalog = PackageCatalog::load(&fixture.0);
        let mut call = Fixture::call();
        call.input = json!({"value": 42});
        let serialized = serde_json::to_string(&call).unwrap();
        let unavailable = ActionService::new(catalog.clone(), RuntimeRegistry::new());
        assert_eq!(
            unavailable.validate(&call).unwrap_err().code,
            "runtime_unavailable"
        );
        let mut registry = RuntimeRegistry::new();
        registry
            .register("workflow-test", Arc::new(Workflow))
            .unwrap();
        let service = ActionService::new(catalog, registry);
        service
            .submit(1, serde_json::from_str(&serialized).unwrap())
            .unwrap();
        assert_eq!(
            terminal(&service).outcome,
            ActionOutcome::Succeeded {
                output: json!({"workflow_result":42})
            }
        );
    }
}
