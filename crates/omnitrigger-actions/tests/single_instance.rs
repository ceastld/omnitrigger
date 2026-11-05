//! 动作目录 → ActionService → 运行时/宿主的单实例边界。
//! 通道与标记是明确的同步见证；轮询只等待已知状态，不按固定延时猜测重叠。
use omnitrigger_actions::{
    ActionCall, ActionDefinition, ActionError, ActionEvent, ActionOutcome, ActionRuntime,
    ActionService, CancellationToken, InvocationContext, NativeHost, PackageCatalog,
    ResolvedAction, RuntimeRegistry,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver, Sender, SyncSender, TryRecvError},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

const WAIT: Duration = Duration::from_secs(10);
static NEXT: AtomicU64 = AtomicU64::new(1);

fn local_metadata(path: &Path) -> fs::Metadata {
    for component in path.components() {
        let name = component.as_os_str().to_string_lossy().to_ascii_lowercase();
        assert!(!name.starts_with("onedrive") && !name.starts_with("icloud") && name != "dropbox");
    }
    let mut ancestors: Vec<_> = path.ancestors().collect();
    ancestors.reverse();
    for ancestor in ancestors {
        let metadata = fs::symlink_metadata(ancestor).unwrap();
        assert!(!metadata.file_type().is_symlink());
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            assert_eq!(
                metadata.file_attributes() & (0x400 | 0x1000 | 0x40000 | 0x400000),
                0
            );
        }
    }
    fs::symlink_metadata(path).unwrap()
}

struct Fixture {
    base: PathBuf,
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        assert!(local_metadata(&manifest).is_dir());
        let repo = manifest.parent().unwrap().parent().unwrap();
        assert!(local_metadata(repo).is_dir());
        let repo = repo.canonicalize().unwrap();
        let mut base = repo;
        for name in [".tmp", "parallel-single-instance", "integration"] {
            base.push(name);
            match fs::symlink_metadata(&base) {
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    match fs::create_dir(&base) {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                        Err(error) => panic!("无法创建隔离目录：{error}"),
                    }
                }
                Err(error) => panic!("无法检查隔离目录：{error}"),
            }
            assert!(local_metadata(&base).is_dir());
        }
        let root = base.join(format!(
            "case-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let fixture = Self { base, root };
        fixture.package("fixture.a", true, 1, "controlled");
        fixture.package("fixture.b", true, 1, "controlled");
        fixture
    }
    fn package(&self, id: &str, single: bool, version: u32, runtime: &str) {
        let root = self.root.join(id);
        match fs::symlink_metadata(&root) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&root).unwrap();
            }
            Err(error) => panic!("无法检查隔离包：{error}"),
        }
        assert!(local_metadata(&root).is_dir());
        fs::write(root.join("main.js"), "// 此入口不由可控运行时执行。\n").unwrap();
        let actions: Vec<_> = ["work", "other"].into_iter().map(|action| json!({
            "id":action,"title":"集成测试动作","version":version,"runtime":runtime,"entry":"main.js","single_instance":single
        })).collect();
        fs::write(
            root.join("action.json"),
            serde_json::to_vec(&json!({
                "schema_version":1,"id":id,"name":"集成测试包","version":"1.0.0","actions":actions
            }))
            .unwrap(),
        )
        .unwrap();
    }
    fn catalog(&self) -> PackageCatalog {
        let catalog = PackageCatalog::load(&self.root);
        assert!(catalog.diagnostics.is_empty(), "{:?}", catalog.diagnostics);
        catalog
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let root = self.root.canonicalize().unwrap();
        let base = self.base.canonicalize().unwrap();
        assert_eq!(root.parent(), Some(base.as_path()));
        assert!(root
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("case-"));
        fs::remove_dir_all(root).unwrap();
    }
}

fn call(package: &str, action: &str, version: u32, tag: &str) -> ActionCall {
    ActionCall {
        package_id: package.into(),
        action_id: action.into(),
        action_version: version,
        input: json!({"tag":tag}),
    }
}
fn work(tag: &str) -> ActionCall {
    call("fixture.a", "work", 1, tag)
}

#[derive(Clone, Copy)]
enum Exit {
    Success,
    Failure,
    Panic,
}

struct Running {
    context: InvocationContext,
    call: ActionCall,
    definition: ActionDefinition,
    cancellation: CancellationToken,
    finish: SyncSender<Exit>,
    cleanup_entered: Receiver<()>,
    cleanup_permit: SyncSender<()>,
    cleaned: Receiver<()>,
}
impl Running {
    fn begin_cleanup(&self, exit: Exit) {
        self.finish.send(exit).unwrap();
        self.cleanup_entered
            .recv_timeout(WAIT)
            .expect("运行时未进入资源清理");
    }
    fn finish_cleanup(&self) {
        self.cleanup_permit.send(()).unwrap();
        self.cleaned.recv_timeout(WAIT).expect("资源清理没有结束");
    }
    fn complete(&self, exit: Exit) {
        self.begin_cleanup(exit);
        self.finish_cleanup();
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.finish.try_send(Exit::Success);
        let _ = self.cleanup_permit.try_send(());
    }
}

struct ExecutionResource {
    entered: Sender<()>,
    permit: Receiver<()>,
    cleaned: Sender<()>,
}
impl Drop for ExecutionResource {
    fn drop(&mut self) {
        if self.entered.send(()).is_ok() {
            let _ = self.permit.recv_timeout(WAIT);
        }
        let _ = self.cleaned.send(());
    }
}

struct Controlled {
    entered: Sender<Running>,
    executions: AtomicUsize,
}
impl ActionRuntime for Controlled {
    fn execute(
        &self,
        action: &ResolvedAction,
        context: &InvocationContext,
        cancellation: &CancellationToken,
    ) -> Result<Value, ActionError> {
        self.executions.fetch_add(1, Ordering::SeqCst);
        let (finish, finished) = mpsc::sync_channel(1);
        let (entered, cleanup_entered) = mpsc::channel();
        let (cleanup_permit, permit) = mpsc::sync_channel(1);
        let (cleaned, clean_done) = mpsc::channel();
        let _resource = ExecutionResource {
            entered,
            permit,
            cleaned,
        };
        self.entered
            .send(Running {
                context: *context,
                call: action.call.clone(),
                definition: action.definition.clone(),
                cancellation: cancellation.clone(),
                finish,
                cleanup_entered,
                cleanup_permit,
                cleaned: clean_done,
            })
            .map_err(|_| ActionError::new("test_disconnected", "测试接收端关闭"))?;
        match finished
            .recv_timeout(WAIT)
            .map_err(|_| ActionError::new("test_timeout", "测试未释放运行时"))?
        {
            Exit::Success => {
                Ok(json!({"tag":action.call.input["tag"],"binding":context.binding_id}))
            }
            Exit::Failure => Err(ActionError::new("test_failure", "可控运行时失败")),
            Exit::Panic => panic!("可控运行时异常"),
        }
    }
}

fn controlled() -> (Arc<Controlled>, Receiver<Running>) {
    let (entered, receiver) = mpsc::channel();
    (
        Arc::new(Controlled {
            entered,
            executions: AtomicUsize::new(0),
        }),
        receiver,
    )
}
fn registry(id: &str, runtime: Arc<dyn ActionRuntime>) -> RuntimeRegistry {
    let mut registry = RuntimeRegistry::new();
    registry.register(id, runtime).unwrap();
    registry
}
fn entered(receiver: &Receiver<Running>, invocation: u64) -> Running {
    let run = receiver.recv_timeout(WAIT).expect("已接纳调用未进入运行时");
    assert_eq!(run.context.invocation_id, invocation);
    run
}
fn busy(result: Result<u64, ActionError>, code: &str) {
    let error = result.unwrap_err();
    assert_eq!(error.code, code);
    if code == "action_busy" {
        assert_eq!(error.effect_state.as_deref(), Some("not_started"));
    }
}

/// 只在已放行清理后用公开 submit 观察服务完成边界，不读取终态以制造解锁。
fn admitted_after_cleanup(service: &ActionService, binding: u64, call: ActionCall) -> u64 {
    admitted_with_host(service, binding, call, None)
}
fn admitted_with_host(
    service: &ActionService,
    binding: u64,
    call: ActionCall,
    host: Option<Arc<dyn NativeHost>>,
) -> u64 {
    let deadline = Instant::now() + WAIT;
    loop {
        match service.submit_with_host(binding, call.clone(), host.clone()) {
            Ok(id) => return id,
            Err(error) if error.code == "action_busy" => {
                assert!(Instant::now() < deadline, "资源已清理但单实例占用未释放");
                thread::yield_now();
            }
            Err(error) => panic!("完成边界出现非预期拒绝：{error}"),
        }
    }
}

fn terminals(service: &ActionService, count: usize) -> BTreeMap<u64, ActionEvent> {
    let deadline = Instant::now() + WAIT;
    let mut events = BTreeMap::new();
    while events.len() < count {
        match service.try_recv() {
            Ok(event) if event.outcome.is_terminal() => {
                assert!(events.insert(event.invocation_id, event).is_none());
            }
            Ok(_) | Err(TryRecvError::Empty) => {
                assert!(Instant::now() < deadline, "等待终态超时");
                thread::yield_now();
            }
            Err(error) => panic!("事件通道退出：{error}"),
        }
    }
    events
}

#[test]
fn catalog_to_runtime_rejects_same_action_across_bindings_and_inputs() {
    let fixture = Fixture::new();
    let (runtime, receiver) = controlled();
    let service = ActionService::new(fixture.catalog(), registry("controlled", runtime.clone()));
    let first = service.submit(11, work("first")).unwrap();
    let run = entered(&receiver, first);
    assert_eq!(run.call.input["tag"], "first");
    busy(service.submit(11, work("same-binding")), "binding_busy");
    busy(service.submit(12, work("different-input")), "action_busy");
    assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
    assert_eq!(runtime.executions.load(Ordering::SeqCst), 1);
    run.complete(Exit::Success);
    assert!(matches!(
        terminals(&service, 1)[&first].outcome,
        ActionOutcome::Succeeded { .. }
    ));
    assert_eq!(runtime.executions.load(Ordering::SeqCst), 1);
}

#[test]
fn distinct_actions_and_packages_enter_together_before_any_release() {
    let fixture = Fixture::new();
    let (runtime, receiver) = controlled();
    let service = ActionService::new(fixture.catalog(), registry("controlled", runtime));
    let calls = [
        work("a-work"),
        call("fixture.a", "other", 1, "a-other"),
        call("fixture.b", "work", 1, "b-work"),
    ];
    let mut accepted = Vec::new();
    for (index, call) in calls.into_iter().enumerate() {
        accepted.push(service.submit(20 + index as u64, call).unwrap());
    }
    let mut runs = Vec::new();
    for _ in 0..3 {
        runs.push(receiver.recv_timeout(WAIT).unwrap());
    }
    let mut entered_ids: Vec<_> = runs.iter().map(|run| run.context.invocation_id).collect();
    entered_ids.sort_unstable();
    accepted.sort_unstable();
    assert_eq!(entered_ids, accepted);
    assert_eq!(
        runs.iter()
            .filter(|run| run.call.package_id == "fixture.a")
            .count(),
        2
    );
    for run in &runs {
        run.complete(Exit::Success);
    }
    assert_eq!(terminals(&service, 3).len(), 3);
}

#[test]
fn cancel_request_does_not_release_action_while_execution_resource_cleanup_is_blocked() {
    let fixture = Fixture::new();
    let (runtime, receiver) = controlled();
    let service = ActionService::new(fixture.catalog(), registry("controlled", runtime));
    let first = service.submit(31, work("cancelled")).unwrap();
    let run = entered(&receiver, first);
    assert_eq!(service.cancel_id(31), 1);
    assert!(run.cancellation.is_cancelled());
    busy(service.submit(32, work("too-early")), "action_busy");
    run.begin_cleanup(Exit::Success);
    busy(
        service.submit(32, work("cleanup-still-live")),
        "action_busy",
    );
    run.finish_cleanup();
    let next = admitted_after_cleanup(&service, 32, work("after-cleanup"));
    assert!(service.is_running(31), "未读取旧终态的绑定占用仍须保留");
    entered(&receiver, next).complete(Exit::Success);
    let outcomes = terminals(&service, 2);
    assert_eq!(outcomes[&first].outcome, ActionOutcome::Cancelled);
    assert!(matches!(
        outcomes[&next].outcome,
        ActionOutcome::Succeeded { .. }
    ));
}

#[test]
fn unread_terminal_retains_binding_and_four_slot_quota_but_not_action_lock() {
    let fixture = Fixture::new();
    let (runtime, receiver) = controlled();
    let service = ActionService::new(fixture.catalog(), registry("controlled", runtime));
    let mut ids = Vec::new();
    for binding in 40..44 {
        let id = if ids.is_empty() {
            service.submit(binding, work("unread")).unwrap()
        } else {
            admitted_after_cleanup(&service, binding, work("unread"))
        };
        entered(&receiver, id).complete(Exit::Success);
        ids.push(id);
        assert!(service.is_running(40));
        busy(service.submit(40, work("old-binding")), "binding_busy");
    }
    // 使用不同动作排除尚在发布最后一个终态时的 action_busy 优先级差异。
    busy(
        service.submit(44, call("fixture.a", "other", 1, "quota")),
        "capacity_exceeded",
    );
    let outcomes = terminals(&service, 4);
    assert!(ids.iter().all(|id| outcomes.contains_key(id)));
    let next = service.submit(40, work("quota-returned")).unwrap();
    entered(&receiver, next).complete(Exit::Success);
    terminals(&service, 1);
}

#[test]
fn reload_policy_matrix_uses_action_identity_across_version_and_runtime_changes() {
    for (old_single, new_single) in [(true, false), (false, true), (true, true), (false, false)] {
        let fixture = Fixture::new();
        fixture.package("fixture.a", old_single, 1, "controlled");
        let (runtime, receiver) = controlled();
        let mut service =
            ActionService::new(fixture.catalog(), registry("controlled", runtime.clone()));
        let first = service.submit(51, work("old")).unwrap();
        let old = entered(&receiver, first);
        fixture.package("fixture.a", new_single, 2, "replacement");
        service.reload(fixture.catalog(), registry("replacement", runtime));
        let new_call = call("fixture.a", "work", 2, "new-input");
        if old_single || new_single {
            busy(service.submit(52, new_call.clone()), "action_busy");
            old.complete(Exit::Success);
            let next = admitted_after_cleanup(&service, 52, new_call);
            let new = entered(&receiver, next);
            assert_eq!(new.definition.version, 2);
            assert_eq!(new.definition.runtime, "replacement");
            new.complete(Exit::Success);
        } else {
            let next = service.submit(52, new_call).unwrap();
            let new = entered(&receiver, next);
            // 两个运行时都已进入且未释放，证明 false → false 仍允许并行。
            new.complete(Exit::Success);
            old.complete(Exit::Success);
        }
        assert_eq!(terminals(&service, 2).len(), 2);
    }
}

#[test]
fn enabling_policy_waits_for_all_already_running_unrestricted_calls() {
    let fixture = Fixture::new();
    fixture.package("fixture.a", false, 1, "controlled");
    let (runtime, receiver) = controlled();
    let mut service =
        ActionService::new(fixture.catalog(), registry("controlled", runtime.clone()));
    let first = service.submit(61, work("old-a")).unwrap();
    let a = entered(&receiver, first);
    let second = service.submit(62, work("old-b")).unwrap();
    let b = entered(&receiver, second);
    fixture.package("fixture.a", true, 1, "controlled");
    service.reload(fixture.catalog(), registry("controlled", runtime));
    busy(service.submit(63, work("new-exclusive")), "action_busy");
    a.complete(Exit::Success);
    busy(
        service.submit(63, work("one-old-still-live")),
        "action_busy",
    );
    b.complete(Exit::Success);
    let third = admitted_after_cleanup(&service, 63, work("all-old-cleaned"));
    entered(&receiver, third).complete(Exit::Success);
    terminals(&service, 3);
}

#[test]
fn runtime_error_and_unwind_keep_resource_cleanup_and_then_allow_new_binding() {
    for exit in [Exit::Failure, Exit::Panic] {
        let fixture = Fixture::new();
        let (runtime, receiver) = controlled();
        let service = ActionService::new(fixture.catalog(), registry("controlled", runtime));
        let first = service.submit(71, work("failing")).unwrap();
        let failed = entered(&receiver, first);
        failed.begin_cleanup(exit);
        busy(service.submit(72, work("during-cleanup")), "action_busy");
        failed.finish_cleanup();
        let next = admitted_after_cleanup(&service, 72, work("recovered"));
        entered(&receiver, next).complete(Exit::Success);
        let outcomes = terminals(&service, 2);
        let ActionOutcome::Failed { error } = &outcomes[&first].outcome else {
            panic!("预期失败终态")
        };
        assert_eq!(
            error.code,
            match exit {
                Exit::Failure => "test_failure",
                Exit::Panic => "runtime_panic",
                Exit::Success => unreachable!(),
            }
        );
    }
}

struct ResourceWitness {
    name: &'static str,
    entered: Sender<&'static str>,
    permit: Mutex<Receiver<()>>,
}
impl Drop for ResourceWitness {
    fn drop(&mut self) {
        if self.entered.send(self.name).is_ok() {
            let _ = self.permit.get_mut().unwrap().recv_timeout(WAIT);
        }
    }
}
struct ResourcePermits(BTreeMap<&'static str, SyncSender<()>>);
impl ResourcePermits {
    fn new() -> Self {
        Self(BTreeMap::new())
    }
    fn witness(&mut self, name: &'static str, entered: &Sender<&'static str>) -> ResourceWitness {
        let (send, permit) = mpsc::sync_channel(1);
        assert!(self.0.insert(name, send).is_none());
        ResourceWitness {
            name,
            entered: entered.clone(),
            permit: Mutex::new(permit),
        }
    }
    fn release(&self, name: &str) {
        self.0[name].send(()).unwrap();
    }
}
impl Drop for ResourcePermits {
    fn drop(&mut self) {
        for permit in self.0.values() {
            let _ = permit.try_send(());
        }
    }
}
struct TrackedRuntime {
    inner: Arc<dyn ActionRuntime>,
    _resource: ResourceWitness,
}
impl ActionRuntime for TrackedRuntime {
    fn execute(
        &self,
        action: &ResolvedAction,
        context: &InvocationContext,
        cancellation: &CancellationToken,
    ) -> Result<Value, ActionError> {
        self.inner.execute(action, context, cancellation)
    }
    fn execute_with_host(
        &self,
        action: &ResolvedAction,
        context: &InvocationContext,
        cancellation: &CancellationToken,
        host: Option<&dyn NativeHost>,
    ) -> Result<Value, ActionError> {
        self.inner
            .execute_with_host(action, context, cancellation, host)
    }
}
struct UnusedHost {
    _resource: ResourceWitness,
}
impl NativeHost for UnusedHost {
    fn call(&self, _: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
        panic!("可控运行时不调用本机 API")
    }
}

#[test]
fn reload_removing_last_old_runtime_and_host_does_not_unlock_during_their_destructors() {
    let fixture = Fixture::new();
    let (runtime, receiver) = controlled();
    let (dropping, dropped) = mpsc::channel();
    let mut permits = ResourcePermits::new();
    let old = Arc::new(TrackedRuntime {
        inner: runtime.clone(),
        _resource: permits.witness("runtime", &dropping),
    });
    let mut service = ActionService::new(fixture.catalog(), registry("controlled", old));
    let host = Arc::new(UnusedHost {
        _resource: permits.witness("host", &dropping),
    });
    let first = service
        .submit_with_host(81, work("old-provider"), Some(host))
        .unwrap();
    let run = entered(&receiver, first);
    fixture.package("fixture.a", false, 2, "replacement");
    service.reload(fixture.catalog(), registry("replacement", runtime));
    run.complete(Exit::Success);
    for _ in 0..2 {
        let resource = dropped
            .recv_timeout(WAIT)
            .expect("旧 provider 未进入最终清理");
        busy(
            service.submit(82, call("fixture.a", "work", 2, "not-yet-cleaned")),
            "action_busy",
        );
        permits.release(resource);
    }
    let next = admitted_after_cleanup(&service, 82, call("fixture.a", "work", 2, "new-provider"));
    entered(&receiver, next).complete(Exit::Success);
    terminals(&service, 2);
}

#[test]
fn independent_services_do_not_share_an_implicit_process_global_action_lock() {
    let fixture = Fixture::new();
    let (runtime, receiver) = controlled();
    let first = ActionService::new(fixture.catalog(), registry("controlled", runtime.clone()));
    let second = ActionService::new(fixture.catalog(), registry("controlled", runtime));
    first.submit(91, work("service-a")).unwrap();
    second.submit(92, work("service-b")).unwrap();
    let a = receiver.recv_timeout(WAIT).unwrap();
    let b = receiver.recv_timeout(WAIT).unwrap();
    assert_ne!(a.context.binding_id, b.context.binding_id);
    a.complete(Exit::Success);
    b.complete(Exit::Success);
    terminals(&first, 1);
    terminals(&second, 1);
}

#[cfg(windows)]
mod real {
    use super::*;
    use omnitrigger_actions::{NodeRuntime, ScriptHostRuntime};
    use std::io::Write;

    const NODE_ENTRY: &str =
        "export default async function main(ctx) { return await ctx.fs.stat(ctx.input.path); }\n";
    const LIGHT_ENTRY: &str = "function main(ctx) { return ctx.fs.stat(ctx.input.path); }\n";

    #[derive(Clone, Copy)]
    enum Engine {
        Node,
        Light,
    }
    impl Engine {
        fn runtime_id(self) -> &'static str {
            match self {
                Self::Node => "node",
                Self::Light => "script",
            }
        }
        fn runtime(self) -> Arc<dyn ActionRuntime> {
            match self {
                Self::Node => {
                    let program = match std::env::var_os("OMNITRIGGER_TEST_NODE") {
                        Some(program) => {
                            let program = PathBuf::from(program);
                            assert!(program.is_absolute());
                            assert!(local_metadata(&program).is_file());
                            program
                        }
                        None => PathBuf::from("node"),
                    };
                    Arc::new(NodeRuntime::new(program))
                }
                Self::Light => {
                    let program = PathBuf::from(
                        std::env::var_os("OMNITRIGGER_TEST_SCRIPT_HOST")
                            .expect("显式指定已有真实轻量宿主"),
                    );
                    assert!(program.is_absolute());
                    assert!(local_metadata(&program).is_file());
                    Arc::new(ScriptHostRuntime::new(program))
                }
            }
        }
        fn install(self, fixture: &Fixture, single: bool, version: u32) {
            fixture.package("fixture.a", single, version, self.runtime_id());
            let (entry, source) = match self {
                Self::Node => ("main.mjs", NODE_ENTRY),
                Self::Light => ("main.js", LIGHT_ENTRY),
            };
            let folder = fixture.root.join("fixture.a");
            fs::write(folder.join(entry), source).unwrap();
            let actions: Vec<_> = ["work", "other"].into_iter().map(|id| json!({
                "id":id,"title":"真实宿主集成动作","version":version,"runtime":self.runtime_id(),"entry":entry,"single_instance":single
            })).collect();
            fs::write(folder.join("action.json"), serde_json::to_vec(&json!({
                "schema_version":1,"id":"fixture.a","name":"真实宿主集成包","version":"1.0.0","actions":actions
            })).unwrap()).unwrap();
        }
    }

    fn create_marker(path: &Path) -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        file.write_all(b"single-instance-test\n")
    }
    fn path_for_script(path: &Path) -> String {
        let text = path.to_str().expect("隔离测试路径必须可表示为 Unicode");
        text.strip_prefix("\\\\?\\").unwrap_or(text).to_owned()
    }

    struct MarkerHost {
        query: String,
        ready: PathBuf,
        released: PathBuf,
        cleaned: PathBuf,
        entered: SyncSender<()>,
        permit: Mutex<Receiver<()>>,
        calls: Arc<AtomicUsize>,
        _resource: ResourceWitness,
    }
    impl NativeHost for MarkerHost {
        fn call(
            &self,
            method: &str,
            params: Value,
            _: &CancellationToken,
        ) -> Result<Value, ActionError> {
            assert_eq!(method, "fs.stat");
            assert_eq!(params, json!({"path":self.query}));
            self.calls.fetch_add(1, Ordering::SeqCst);
            create_marker(&self.ready)
                .map_err(|_| ActionError::new("test_marker", "创建进入标记失败"))?;
            self.entered
                .send(())
                .map_err(|_| ActionError::new("test_disconnected", "测试通道关闭"))?;
            self.permit
                .lock()
                .unwrap()
                .recv_timeout(WAIT)
                .map_err(|_| ActionError::new("test_timeout", "未收到显式释放"))?;
            assert!(local_metadata(&self.released).is_file());
            create_marker(&self.cleaned)
                .map_err(|_| ActionError::new("test_marker", "创建清理标记失败"))?;
            // 内存回执只证明真实 JS → 本机桥；不查询 query，更不是平台 stat 业务测试。
            Ok(json!({"kind":"file"}))
        }
    }

    struct ReleaseMarker {
        path: PathBuf,
        permit: SyncSender<()>,
    }
    impl ReleaseMarker {
        fn release(&self) {
            create_marker(&self.path).unwrap();
            self.permit.send(()).unwrap();
        }
    }
    impl Drop for ReleaseMarker {
        fn drop(&mut self) {
            let _ = create_marker(&self.path);
            let _ = self.permit.try_send(());
        }
    }
    struct ImmediateHost;
    impl NativeHost for ImmediateHost {
        fn call(
            &self,
            method: &str,
            _: Value,
            _: &CancellationToken,
        ) -> Result<Value, ActionError> {
            assert_eq!(method, "fs.stat");
            Ok(json!({"kind":"directory"}))
        }
    }

    fn real_service_sequence(engine: Engine) {
        let fixture = Fixture::new();
        engine.install(&fixture, true, 1);
        let query = fixture.root.join("query.marker");
        create_marker(&query).unwrap();
        let query = path_for_script(&query);
        let ready = fixture.root.join("entered.marker");
        let released = fixture.root.join("release.marker");
        let cleaned = fixture.root.join("cleaned.marker");
        let (dropping, resources) = mpsc::channel();
        let mut permits = ResourcePermits::new();
        let old_runtime = Arc::new(TrackedRuntime {
            inner: engine.runtime(),
            _resource: permits.witness("runtime", &dropping),
        });
        let mut service = ActionService::new(
            fixture.catalog(),
            registry(engine.runtime_id(), old_runtime),
        );
        let (entered_tx, entered_rx) = mpsc::sync_channel(1);
        let (permit_tx, permit_rx) = mpsc::sync_channel(1);
        let release = ReleaseMarker {
            path: released.clone(),
            permit: permit_tx,
        };
        let host_calls = Arc::new(AtomicUsize::new(0));
        let host = Arc::new(MarkerHost {
            query: query.clone(),
            ready: ready.clone(),
            released,
            cleaned: cleaned.clone(),
            entered: entered_tx,
            permit: Mutex::new(permit_rx),
            calls: host_calls.clone(),
            _resource: permits.witness("host", &dropping),
        });
        let mut first_call = work("real-first");
        first_call.input = json!({"path":query});
        let first = service
            .submit_with_host(101, first_call.clone(), Some(host))
            .unwrap();
        entered_rx
            .recv_timeout(WAIT)
            .expect("真实脚本没有到达内存宿主 barrier");
        assert!(local_metadata(&ready).is_file());
        assert!(!cleaned.exists());
        let mut rejected = first_call.clone();
        rejected.input =
            json!({"path":path_for_script(&fixture.root.join("never-entered.marker"))});
        busy(service.submit(102, rejected), "action_busy");
        assert_eq!(host_calls.load(Ordering::SeqCst), 1);

        // 第一脚本仍阻塞在明确 barrier 时，不同动作必须可真实完成。
        let mut other = first_call.clone();
        other.action_id = "other".into();
        let independent = service
            .submit_with_host(103, other, Some(Arc::new(ImmediateHost)))
            .unwrap();
        let completed = terminals(&service, 1);
        assert_eq!(
            completed[&independent].outcome,
            ActionOutcome::Succeeded {
                output: json!({"kind":"directory"})
            }
        );

        // 移除 registry 对旧 provider 的最后引用；新版本关掉标志也不能绕过旧活跃调用。
        engine.install(&fixture, false, 2);
        service.reload(
            fixture.catalog(),
            registry(engine.runtime_id(), engine.runtime()),
        );
        let mut replacement = first_call;
        replacement.action_version = 2;
        busy(service.submit(104, replacement.clone()), "action_busy");
        release.release();
        for _ in 0..2 {
            let resource = resources
                .recv_timeout(WAIT)
                .expect("真实 provider 没有进入最终清理");
            assert!(local_metadata(&cleaned).is_file());
            busy(service.submit(104, replacement.clone()), "action_busy");
            permits.release(resource);
        }
        assert_eq!(host_calls.load(Ordering::SeqCst), 1);
        let next = admitted_with_host(&service, 104, replacement, Some(Arc::new(ImmediateHost)));
        assert!(
            service.is_running(101),
            "真实脚本旧终态未读时绑定占用仍存在"
        );
        let completed = terminals(&service, 2);
        assert_eq!(
            completed[&first].outcome,
            ActionOutcome::Succeeded {
                output: json!({"kind":"file"})
            }
        );
        assert_eq!(
            completed[&next].outcome,
            ActionOutcome::Succeeded {
                output: json!({"kind":"directory"})
            }
        );
    }

    #[test]
    #[ignore = "真实 Node 脚本进程；根对话串行执行，Node 可由 OMNITRIGGER_TEST_NODE 指定"]
    fn node_script_service_mutex_reaches_native_barrier_and_cleans_old_provider() {
        real_service_sequence(Engine::Node);
    }

    #[test]
    #[ignore = "需要 OMNITRIGGER_TEST_SCRIPT_HOST 指向已有真实轻量宿主；根对话串行执行"]
    fn light_script_service_mutex_reaches_native_barrier_and_cleans_old_provider() {
        real_service_sequence(Engine::Light);
    }
}
