use crate::{model::valid_id, ActionCall, ActionError, PackageCatalog, ResolvedAction};
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
}

struct Reservations {
    next_id: u64,
    entries: BTreeMap<u64, Reservation>,
}

/// 有界的应用调用服务；正在运行和终态尚未被消费的调用共用配额。
/// 不在 Drop 中等待用户运行时，不重试已经接纳的带副作用调用。
pub struct ActionService {
    catalog: PackageCatalog,
    runtimes: RuntimeRegistry,
    reservations: Mutex<Reservations>,
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
            reservations: Mutex::new(Reservations {
                next_id: 1,
                entries: BTreeMap::new(),
            }),
            events_tx,
            events_rx,
        }
    }

    pub fn catalog(&self) -> &PackageCatalog {
        &self.catalog
    }

    pub fn validate(&self, call: &ActionCall) -> Result<(), ActionError> {
        let resolved = self.catalog.resolve(call)?;
        self.runtimes.get(&resolved.definition.runtime)?;
        Ok(())
    }

    pub fn submit(&self, id: u64, call: ActionCall) -> Result<u64, ActionError> {
        let action = self.catalog.resolve(&call)?;
        let runtime = self.runtimes.get(&action.definition.runtime)?;
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
                },
            );
            invocation_id
        };
        let events = self.events_tx.clone();
        let context = InvocationContext {
            invocation_id,
            binding_id: id,
        };
        let spawned = thread::Builder::new()
            .name(format!("omnitrigger-action-{invocation_id}"))
            .spawn(move || {
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
                let outcome = if cancellation.is_cancelled() {
                    ActionOutcome::Cancelled
                } else {
                    let result = catch_unwind(AssertUnwindSafe(|| {
                        runtime.execute(&action, &context, &cancellation)
                    }));
                    if cancellation.is_cancelled() {
                        // 取消不撤销已经发生的副作用；实现必须先结束自己的资源再返回。
                        ActionOutcome::Cancelled
                    } else {
                        match result {
                            Ok(Ok(output)) => ActionOutcome::Succeeded { output },
                            Ok(Err(error)) if error.code == "cancelled" => ActionOutcome::Cancelled,
                            Ok(Err(error)) => ActionOutcome::Failed { error },
                            Err(_) => ActionOutcome::Failed {
                                error: ActionError::new(
                                    "runtime_panic",
                                    "动作运行时异常退出，本次调用未重试。",
                                ),
                            },
                        }
                    }
                };
                let _ = events.send(ActionEvent {
                    id,
                    invocation_id,
                    outcome,
                });
            });
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
