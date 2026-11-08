//! 真实 Node/QuickJS 子进程、真实服务和回环桥接；桌面回调为合成替身。
use omnitrigger_actions::{
    ActionCall, ActionError, ActionOutcome, ActionRuntime, ActionService, CancellationToken,
    NativeHost, NodeRuntime, PackageCatalog, RuntimeRegistry, ScriptHostRuntime,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
static NEXT: AtomicU64 = AtomicU64::new(1);
const WAIT: Duration = Duration::from_secs(90);

struct OldHost {
    entered: mpsc::Sender<()>,
    dropped: Arc<AtomicBool>,
    capture: bool,
    capture_cwd: Option<String>,
    cancelled: Arc<AtomicBool>,
    calls: Arc<Mutex<Vec<(String, Value)>>>,
}
impl NativeHost for OldHost {
    fn native_context_available(&self) -> bool {
        !self.capture
    }

    fn call(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
    ) -> Result<Value, ActionError> {
        assert_eq!(
            method,
            if self.capture {
                "process.capture"
            } else {
                "target.info"
            }
        );
        if self.capture {
            let mut expected = json!({"program":"C:\\Synthetic\\must-not-start.exe","args":[],"timeout_ms":60000,"allow_non_zero":false});
            if let Some(cwd) = &self.capture_cwd {
                expected["cwd"] = json!(cwd);
            }
            assert_eq!(params, expected);
        }
        {
            let mut calls = self.calls.lock().unwrap();
            assert!(calls.is_empty(), "记录Host只接受一次请求，不执行程序或重试");
            calls.push((method.into(), params));
        }
        // 真实Runtime已进入记录Host后才通知测试线程启动worker2，不用固定sleep猜就绪。
        self.entered.send(()).unwrap();
        let deadline = Instant::now() + WAIT;
        while !cancellation.is_cancelled() {
            assert!(Instant::now() < deadline, "合成旧调用未收到取消");
            thread::sleep(Duration::from_millis(1));
        }
        self.cancelled.store(true, Ordering::SeqCst);
        Err(ActionError::new("cancelled", "合成旧调用已取消。"))
    }
}
impl Drop for OldHost {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

fn fixture_root() -> PathBuf {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.tmp");
    for ancestor in base.ancestors().filter(|p| p.exists()) {
        let info = fs::symlink_metadata(ancestor).unwrap();
        assert!(!info.file_type().is_symlink());
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            assert_eq!(info.file_attributes() & 0x441400, 0);
        }
    }
    fs::create_dir_all(&base).unwrap();
    // 保留的验收包可能来自被复用的进程 ID；只原子创建新目录，不复用或删除旧包。
    (0..1024)
        .find_map(|_| {
            let candidate = base.join(format!(
                "scoped-runtime-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&candidate) {
                Ok(()) => Some(candidate),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(error) => panic!("无法创建新的合成验收目录：{error}"),
            }
        })
        .expect("未找到可原子创建的合成验收目录")
}

fn run(runtime_name: &str, runtime: Arc<dyn ActionRuntime>, same_binding: bool) {
    let root = fixture_root();
    let package = root.join("scoped-real");
    fs::create_dir(&package).unwrap();
    let source = if same_binding && runtime_name == "node" {
        "export default async function main(ctx){if(ctx.input.role==='old')return await ctx.process.capture('C:\\\\Synthetic\\\\must-not-start.exe',[],{timeoutMs:60000});return await ctx.actions.stopOtherInstances({timeoutMs:2000});}\n"
    } else if same_binding {
        "export function main(ctx){if(ctx.input.role==='old')return ctx.process.capture('C:\\\\Synthetic\\\\must-not-start.exe',[],{timeoutMs:60000});return ctx.actions.stopOtherInstances({timeoutMs:2000});}\n"
    } else if runtime_name == "node" {
        "export default async function main(ctx){if(ctx.input.role==='old')return await ctx.desktop.target();return await ctx.actions.stopOtherInstances({timeoutMs:2000});}\n"
    } else {
        "export function main(ctx){if(ctx.input.role==='old')return ctx.desktop.target();return ctx.actions.stopOtherInstances({timeoutMs:2000});}\n"
    };
    fs::write(package.join("main.mjs"), source).unwrap();
    let mut manifest = json!({"schema_version":1,"id":"scoped-real","name":"真实调用作用域合成验收","version":"1.0.0","actions":[{"id":"run","title":"停止其他实例","version":1,"runtime":runtime_name,"entry":"main.mjs","single_instance":false}]});
    if same_binding {
        manifest["actions"][0]["allow_binding_reentry"] = json!(true);
    }
    fs::write(
        package.join("action.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let catalog = PackageCatalog::load(&root);
    assert!(catalog.diagnostics.is_empty(), "{:?}", catalog.diagnostics);
    assert_eq!(catalog.actions.len(), 1);
    assert!(!catalog.actions[0].definition.single_instance);
    assert_eq!(
        catalog.actions[0].definition.allow_binding_reentry,
        same_binding
    );
    let mut registry = RuntimeRegistry::new();
    registry.register(runtime_name, runtime.clone()).unwrap();
    let service = ActionService::new(catalog, registry);
    struct CancelOnDrop<'a>(&'a ActionService);
    impl Drop for CancelOnDrop<'_> {
        fn drop(&mut self) {
            self.0.cancel_all();
        }
    }
    let _cancel = CancelOnDrop(&service);
    let call = |role: &str| ActionCall {
        package_id: "scoped-real".into(),
        action_id: "run".into(),
        action_version: 1,
        input: json!({"role":role}),
    };
    let (entered_tx, entered) = mpsc::channel();
    let dropped = Arc::new(AtomicBool::new(false));
    let cancelled = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let old = service
        .submit_with_host(
            1,
            call("old"),
            Some(Arc::new(OldHost {
                entered: entered_tx,
                dropped: dropped.clone(),
                capture: same_binding,
                capture_cwd: (same_binding && runtime_name == "node").then(|| {
                    let absolute = fs::canonicalize(&package).unwrap();
                    let path = absolute.to_str().unwrap();
                    path.strip_prefix(r"\\?\").unwrap_or(path).to_owned()
                }),
                cancelled: cancelled.clone(),
                calls: calls.clone(),
            })),
        )
        .unwrap();
    if let Err(cause) = entered.recv_timeout(WAIT) {
        let until = Instant::now() + Duration::from_secs(1);
        loop {
            if let Ok(event) = service.try_recv() {
                eprintln!("旧脚本进入宿主前的服务事件：{event:?}");
                if event.outcome.is_terminal() {
                    break;
                }
            }
            if Instant::now() >= until {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        panic!("真实旧脚本未进入合成宿主：{cause:?}");
    }
    assert!(!dropped.load(Ordering::SeqCst));
    assert!(service.is_running(1));
    let controller = service
        .submit(if same_binding { 1 } else { 2 }, call("controller"))
        .unwrap();
    assert_ne!(old, controller, "同绑定仍必须获得独立invocation");
    let deadline = Instant::now() + WAIT;
    let mut terminals = 0;
    let mut saw_controller = false;
    while terminals < 2 {
        if let Ok(event) = service.try_recv() {
            if same_binding {
                assert_eq!(event.id, 1, "两个worker始终属于同一个binding");
            }
            match event.outcome {
                ActionOutcome::Started => {}
                ActionOutcome::Succeeded { output } => {
                    assert_eq!(event.invocation_id, controller);
                    assert!(dropped.load(Ordering::SeqCst), "回执早于旧宿主清理");
                    assert!(
                        cancelled.load(Ordering::SeqCst),
                        "旧Native等待必须实际观察取消"
                    );
                    assert_eq!(output, json!({"stoppedCount":1,"effectState":"unknown"}));
                    saw_controller = true;
                    terminals += 1;
                }
                ActionOutcome::Cancelled => {
                    assert_eq!(event.invocation_id, old);
                    terminals += 1;
                }
                ActionOutcome::Failed { error } => panic!("真实{runtime_name}调用失败：{error:?}"),
            }
        }
        assert!(Instant::now() < deadline, "真实调用未结束");
        thread::sleep(Duration::from_millis(1));
    }
    assert!(saw_controller);
    assert!(dropped.load(Ordering::SeqCst));
    assert!(!service.is_running(1) && !service.is_running(2));
    if same_binding {
        let recorded = calls.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].0, "process.capture");
        // 正场景到此为止；不为新策略复制原有无scope负向矩阵。
        return;
    }
    // 同一真实运行时直接执行时，invocation元数据也不能代替服务作用域。
    let resolved = service.catalog().resolve(&call("controller")).unwrap();
    let error = runtime
        .execute(
            &resolved,
            &omnitrigger_actions::InvocationContext {
                invocation_id: 999,
                binding_id: 777,
            },
            &CancellationToken::new(),
        )
        .unwrap_err();
    assert_eq!(error.code, "actions_unavailable");
    assert_eq!(error.effect_state.as_deref(), Some("not_started"));
    assert_eq!(
        error.method.as_deref(),
        Some("actions.stop_other_instances")
    );
    // 保留合成包用于核对，不删除作者目录或其他测试产物。
}

#[test]
#[ignore = "需要显式指定已安装的真实 Node"]
fn node_scoped_stop_other_instances_reaches_service_and_waits_for_cleanup() {
    run(
        "node",
        Arc::new(NodeRuntime::new(PathBuf::from(
            std::env::var_os("OMNITRIGGER_TEST_NODE").expect("设置Node路径"),
        ))),
        false,
    );
}
#[test]
#[ignore = "需要显式指定新构建的真实 QuickJS 宿主"]
fn script_scoped_stop_other_instances_reaches_service_and_waits_for_cleanup() {
    run(
        "script",
        Arc::new(
            ScriptHostRuntime::new(PathBuf::from(
                std::env::var_os("OMNITRIGGER_TEST_SCRIPT_HOST").expect("设置Script宿主路径"),
            ))
            .with_required_capabilities(vec!["actions.stop_other_instances.v1".into()]),
        ),
        false,
    );
}

#[cfg(windows)]
#[test]
#[ignore = "显式Node路径；真实Service/Runtime与记录capture Host，无业务进程"]
fn node_same_binding_reentry_capture_stops_other_and_waits_cleanup() {
    run(
        "node",
        Arc::new(NodeRuntime::new(PathBuf::from(
            std::env::var_os("OMNITRIGGER_TEST_NODE").expect("设置Node路径"),
        ))),
        true,
    );
}

#[cfg(windows)]
#[test]
#[ignore = "显式Windows ScriptHost路径；真实Service/Runtime与记录capture Host，无业务进程"]
fn script_same_binding_reentry_capture_stops_other_and_waits_cleanup() {
    run(
        "script",
        Arc::new(
            ScriptHostRuntime::new(PathBuf::from(
                std::env::var_os("OMNITRIGGER_TEST_SCRIPT_HOST").expect("设置Script宿主路径"),
            ))
            .with_required_capabilities(vec![
                "actions.stop_other_instances.v1".into(),
                "process.capture.v1".into(),
            ]),
        ),
        true,
    );
}

// 以下仅扩充真实 ScriptHostRuntime 的服务夹具；不实现另一套运行器或父桥。
#[derive(Default)]
struct AsyncWitness {
    entered: AtomicBool,
    active: std::sync::atomic::AtomicUsize,
    synchronous_calls: std::sync::atomic::AtomicUsize,
    cancelled: AtomicBool,
    callback_returned: AtomicBool,
    dropped: AtomicBool,
    returned_order: Mutex<Vec<u64>>,
    continued_before_cleanup: AtomicBool,
    second_delivered: AtomicBool,
}

#[derive(Debug, PartialEq, Eq)]
enum AsyncPhase {
    Entered(u64),
    Returned(u64),
    Cancelled,
    Dropping,
    SecondDelivered,
}

struct CallbackActive {
    state: Arc<AsyncWitness>,
    events: mpsc::Sender<AsyncPhase>,
    sequence: u64,
}
impl CallbackActive {
    fn enter(state: Arc<AsyncWitness>, events: mpsc::Sender<AsyncPhase>, sequence: u64) -> Self {
        state.entered.store(true, Ordering::SeqCst);
        state.active.fetch_add(1, Ordering::SeqCst);
        let _ = events.send(AsyncPhase::Entered(sequence));
        Self {
            state,
            events,
            sequence,
        }
    }
}
impl Drop for CallbackActive {
    fn drop(&mut self) {
        self.state
            .returned_order
            .lock()
            .unwrap()
            .push(self.sequence);
        self.state.callback_returned.store(true, Ordering::SeqCst);
        self.state.active.fetch_sub(1, Ordering::SeqCst);
        let _ = self.events.send(AsyncPhase::Returned(self.sequence));
    }
}

struct OpenGatesOnDrop(Vec<mpsc::Sender<()>>);
impl Drop for OpenGatesOnDrop {
    fn drop(&mut self) {
        for gate in &self.0 {
            let _ = gate.send(());
        }
    }
}
struct CancelServiceOnDrop<'a>(&'a ActionService);
impl Drop for CancelServiceOnDrop<'_> {
    fn drop(&mut self) {
        self.0.cancel_all();
    }
}

fn async_error(code: &str, method: &str) -> ActionError {
    let mut error = ActionError::new(code, "合成浏览器回调，不调用Chrome或桌面。");
    error.method = Some(method.into());
    error.effect_state = Some("unknown".into());
    error
}
fn wait_async_gate(
    gate: &Mutex<mpsc::Receiver<()>>,
    cancellation: &CancellationToken,
    deadline: Instant,
    observe_cancel: bool,
) -> Result<(), ActionError> {
    loop {
        if observe_cancel && cancellation.is_cancelled() {
            return Err(async_error("cancelled", "browser.call"));
        }
        if Instant::now() >= deadline {
            return Err(async_error("timeout", "browser.call"));
        }
        match gate.lock().unwrap().recv_timeout(Duration::from_millis(5)) {
            Ok(()) => return Ok(()),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(error) => panic!("合成清理门丢失：{error}"),
        }
    }
}

struct FanoutHost {
    state: Arc<AsyncWitness>,
    events: mpsc::Sender<AsyncPhase>,
    release: [Mutex<mpsc::Receiver<()>>; 2],
}
impl NativeHost for FanoutHost {
    fn native_context_available(&self) -> bool {
        false
    }
    fn browser_async_available(&self) -> bool {
        true
    }
    fn call(&self, method: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
        self.state.synchronous_calls.fetch_add(1, Ordering::SeqCst);
        Err(async_error("unexpected_sync_path", method))
    }
    fn call_browser_async(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<Value, ActionError> {
        assert_eq!(method, "browser.call");
        if params["method"] == "test.secondDelivered" {
            assert_eq!(
                params,
                json!({"method":"test.secondDelivered","args":[],"options":{}})
            );
            assert_eq!(self.state.active.load(Ordering::SeqCst), 1);
            assert_eq!(*self.state.returned_order.lock().unwrap(), [2]);
            assert!(!self.state.second_delivered.swap(true, Ordering::SeqCst));
            let _ = self.events.send(AsyncPhase::SecondDelivered);
            return Ok(json!({"has_value":true,"value":null}));
        }
        let sequence = params["args"][0].as_u64().unwrap();
        assert!((1..=2).contains(&sequence));
        assert_eq!(
            params,
            json!({"method":"tabs.move","args":[sequence,{}],"options":{}})
        );
        let _active = CallbackActive::enter(self.state.clone(), self.events.clone(), sequence);
        wait_async_gate(
            &self.release[sequence as usize - 1],
            cancellation,
            deadline,
            true,
        )?;
        Ok(json!({"has_value":true,"value":{"sequence":sequence}}))
    }
}
impl Drop for FanoutHost {
    fn drop(&mut self) {
        assert_eq!(self.state.active.load(Ordering::SeqCst), 0);
        self.state.dropped.store(true, Ordering::SeqCst);
    }
}

struct ParkedAsyncHost {
    state: Arc<AsyncWitness>,
    events: mpsc::Sender<AsyncPhase>,
    callback_cleanup: Mutex<mpsc::Receiver<()>>,
    host_cleanup: Mutex<mpsc::Receiver<()>>,
}
impl NativeHost for ParkedAsyncHost {
    fn native_context_available(&self) -> bool {
        false
    }
    fn browser_async_available(&self) -> bool {
        true
    }
    fn call(&self, method: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
        self.state.synchronous_calls.fetch_add(1, Ordering::SeqCst);
        Err(async_error("unexpected_sync_path", method))
    }
    fn call_browser_async(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<Value, ActionError> {
        assert_eq!(method, "browser.call");
        assert_eq!(
            params,
            json!({"method":"tabs.move","args":[1,{}],"options":{}})
        );
        let _active = CallbackActive::enter(self.state.clone(), self.events.clone(), 1);
        while !cancellation.is_cancelled() {
            assert!(Instant::now() < deadline, "旧异步回调没有收到服务取消");
            thread::sleep(Duration::from_millis(1));
        }
        self.state.cancelled.store(true, Ordering::SeqCst);
        let _ = self.events.send(AsyncPhase::Cancelled);
        // 此门表示取消后的本次资源清理：取消令牌不等于已退出/已清理。
        wait_async_gate(&self.callback_cleanup, cancellation, deadline, false)?;
        Err(async_error("cancelled", method))
    }
}
impl Drop for ParkedAsyncHost {
    fn drop(&mut self) {
        // 旧宿主缺cap而从未进入回调时不阻塞，才能直接暴露预检失败，不能误记通过。
        if self.state.entered.load(Ordering::SeqCst) {
            assert!(self.state.callback_returned.load(Ordering::SeqCst));
            assert_eq!(self.state.active.load(Ordering::SeqCst), 0);
            let _ = self.events.send(AsyncPhase::Dropping);
            self.host_cleanup
                .get_mut()
                .unwrap()
                .recv_timeout(WAIT)
                .expect("合成本次Host资源释放门");
        }
        self.state.dropped.store(true, Ordering::SeqCst);
    }
}

struct AfterStopHost {
    old: Arc<AsyncWitness>,
    continued: mpsc::Sender<Value>,
}
impl NativeHost for AfterStopHost {
    fn native_context_available(&self) -> bool {
        false
    }
    fn browser_async_available(&self) -> bool {
        true
    }
    fn call(&self, method: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
        self.old.synchronous_calls.fetch_add(1, Ordering::SeqCst);
        Err(async_error("unexpected_sync_path", method))
    }
    fn call_browser_async(
        &self,
        method: &str,
        params: Value,
        _: &CancellationToken,
        _: Instant,
    ) -> Result<Value, ActionError> {
        assert_eq!(method, "browser.call");
        assert_eq!(params["method"], "test.afterStop");
        assert_eq!(
            params["args"],
            json!([{"stoppedCount":1,"effectState":"unknown"}])
        );
        let cleaned = self.old.dropped.load(Ordering::SeqCst)
            && self.old.active.load(Ordering::SeqCst) == 0
            && self.old.callback_returned.load(Ordering::SeqCst);
        if !cleaned {
            self.old
                .continued_before_cleanup
                .store(true, Ordering::SeqCst);
        }
        let _ = self.continued.send(params["args"][0].clone());
        if !cleaned {
            return Err(async_error("cleanup_not_complete", method));
        }
        Ok(json!({"has_value":true,"value":null}))
    }
}

fn async_service(source: &str, reentry: bool) -> ActionService {
    let root = fixture_root();
    let package = root.join("scoped-browser-real");
    fs::create_dir(&package).unwrap();
    fs::write(package.join("main.mjs"), source).unwrap();
    let manifest = json!({"schema_version":1,"id":"scoped-browser-real","name":"合成浏览器真实Script作用域","version":"1.0.0","actions":[{"id":"run","title":"浏览器回调收尾","version":1,"runtime":"script","entry":"main.mjs","single_instance":false,"allow_binding_reentry":reentry}]});
    fs::write(
        package.join("action.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let catalog = PackageCatalog::load(&root);
    assert!(catalog.diagnostics.is_empty(), "{:?}", catalog.diagnostics);
    assert_eq!(catalog.actions.len(), 1);
    assert_eq!(catalog.actions[0].definition.allow_binding_reentry, reentry);
    let mut required_capabilities = vec!["browser.async.v1".into()];
    if reentry {
        required_capabilities.push("actions.stop_other_instances.v1".into());
    }
    let runtime = ScriptHostRuntime::new(PathBuf::from(
        std::env::var_os("OMNITRIGGER_TEST_SCRIPT_HOST").expect("设置本次真实Script宿主路径"),
    ))
    .with_required_capabilities(required_capabilities);
    let mut registry = RuntimeRegistry::new();
    registry.register("script", Arc::new(runtime)).unwrap();
    ActionService::new(catalog, registry)
}
fn async_call(role: &str) -> ActionCall {
    // role仅选合成业务分支；身份、scope和再入策略来自服务、Host和清单。
    ActionCall {
        package_id: "scoped-browser-real".into(),
        action_id: "run".into(),
        action_version: 1,
        input: json!({"role":role}),
    }
}
fn no_async_terminal(service: &ActionService) {
    while let Ok(event) = service.try_recv() {
        assert!(
            !event.outcome.is_terminal(),
            "清理门未释放就收到终态（或旧宿主缺能力）：{event:?}"
        );
    }
}
fn async_phase(receiver: &mpsc::Receiver<AsyncPhase>, service: &ActionService) -> AsyncPhase {
    let until = Instant::now() + WAIT;
    loop {
        if let Ok(phase) = receiver.try_recv() {
            return phase;
        }
        no_async_terminal(service);
        assert!(Instant::now() < until, "真实Script未进入所要求的异步阶段");
        thread::sleep(Duration::from_millis(1));
    }
}
fn wait_controller_still_blocked(service: &ActionService, continued: &mpsc::Receiver<Value>) {
    // 已用真实回调阶段确定就绪；这是短负向观察窗，不用sleep猜进程是否启动。
    assert!(
        matches!(
            continued.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ),
        "旧资源仍被持有时控制脚本已越过stopOtherInstances"
    );
    no_async_terminal(service);
}

#[test]
#[ignore = "显式新Script宿主；要求browser.async.v1；真实Service/进程/本地TCP，记录Host不调用Chrome"]
fn script_browser_async_fanout_uses_opted_in_hook_and_matches_reverse_replies() {
    let service=async_service("export function main(ctx){const first=ctx.browser.callAsync('tabs.move',[1,{}]);const second=ctx.browser.callAsync('tabs.move',[2,{}]);return Promise.all([first,second.then(async value=>{await ctx.browser.callAsync('test.secondDelivered',[]);return value;})]);}\n",false);
    let _cancel = CancelServiceOnDrop(&service);
    let (first, first_gate) = mpsc::channel();
    let (second, second_gate) = mpsc::channel();
    let _open = OpenGatesOnDrop(vec![first.clone(), second.clone()]);
    let (events, phases) = mpsc::channel();
    let state = Arc::new(AsyncWitness::default());
    let host = Arc::new(FanoutHost {
        state: state.clone(),
        events,
        release: [Mutex::new(first_gate), Mutex::new(second_gate)],
    });
    assert!(!host.native_context_available());
    assert!(host.browser_async_available());
    let weak = Arc::downgrade(&host);
    let invocation = service
        .submit_with_host(7001, async_call("fanout"), Some(host))
        .unwrap();
    let mut entered = [
        async_phase(&phases, &service),
        async_phase(&phases, &service),
    ];
    entered.sort_by_key(|phase| match phase {
        AsyncPhase::Entered(value) => *value,
        _ => panic!("首个回执前必须两个回调都已进入"),
    });
    assert_eq!(entered, [AsyncPhase::Entered(1), AsyncPhase::Entered(2)]);
    assert_eq!(state.active.load(Ordering::SeqCst), 2);
    second.send(()).unwrap();
    assert_eq!(async_phase(&phases, &service), AsyncPhase::Returned(2));
    assert_eq!(async_phase(&phases, &service), AsyncPhase::SecondDelivered);
    assert!(state.second_delivered.load(Ordering::SeqCst));
    assert_eq!(state.active.load(Ordering::SeqCst), 1);
    no_async_terminal(&service);
    first.send(()).unwrap();
    let until = Instant::now() + WAIT;
    loop {
        if let Ok(event) = service.try_recv() {
            match event.outcome {
                ActionOutcome::Started => {}
                ActionOutcome::Succeeded { output } => {
                    assert_eq!(event.invocation_id, invocation);
                    assert_eq!(output, json!([{"sequence":1},{"sequence":2}]));
                    break;
                }
                outcome => panic!("真实异步Script失败：{outcome:?}"),
            }
        }
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(*state.returned_order.lock().unwrap(), [2, 1]);
    assert_eq!(state.synchronous_calls.load(Ordering::SeqCst), 0);
    assert_eq!(state.active.load(Ordering::SeqCst), 0);
    assert!(state.dropped.load(Ordering::SeqCst));
    assert!(weak.upgrade().is_none());
    assert!(!service.is_running(7001));
}

#[test]
#[ignore = "显式新Script宿主；真实同绑定再入/stopOther与受控异步worker、Host资源门，不调用Chrome"]
fn script_browser_async_same_binding_stop_waits_for_runtime_worker_and_host_cleanup() {
    let source="export function main(ctx){if(ctx.input.role==='old')return ctx.browser.callAsync('tabs.move',[1,{}]);return(async()=>{const receipt=ctx.actions.stopOtherInstances({timeoutMs:2000});await ctx.browser.callAsync('test.afterStop',[receipt]);return receipt;})();}\n";
    let service = async_service(source, true);
    let _cancel = CancelServiceOnDrop(&service);
    let (callback_release, callback_gate) = mpsc::channel();
    let (host_release, host_gate) = mpsc::channel();
    let _open = OpenGatesOnDrop(vec![callback_release.clone(), host_release.clone()]);
    let (events, phases) = mpsc::channel();
    let state = Arc::new(AsyncWitness::default());
    let host = Arc::new(ParkedAsyncHost {
        state: state.clone(),
        events,
        callback_cleanup: Mutex::new(callback_gate),
        host_cleanup: Mutex::new(host_gate),
    });
    let weak = Arc::downgrade(&host);
    let old = service
        .submit_with_host(7002, async_call("old"), Some(host))
        .unwrap();
    assert_eq!(async_phase(&phases, &service), AsyncPhase::Entered(1));
    assert_eq!(state.active.load(Ordering::SeqCst), 1);
    let (continued, continuation) = mpsc::channel();
    let controller = service
        .submit_with_host(
            7002,
            async_call("controller"),
            Some(Arc::new(AfterStopHost {
                old: state.clone(),
                continued,
            })),
        )
        .unwrap();
    assert_ne!(old, controller);
    assert_eq!(async_phase(&phases, &service), AsyncPhase::Cancelled);
    assert!(state.cancelled.load(Ordering::SeqCst));
    assert_eq!(state.active.load(Ordering::SeqCst), 1);
    assert!(!state.dropped.load(Ordering::SeqCst));
    wait_controller_still_blocked(&service, &continuation);
    callback_release.send(()).unwrap();
    assert_eq!(async_phase(&phases, &service), AsyncPhase::Returned(1));
    assert_eq!(async_phase(&phases, &service), AsyncPhase::Dropping);
    assert_eq!(state.active.load(Ordering::SeqCst), 0);
    assert!(!state.dropped.load(Ordering::SeqCst));
    wait_controller_still_blocked(&service, &continuation);
    host_release.send(()).unwrap();
    let until = Instant::now() + WAIT;
    let mut old_finished = false;
    let mut controller_finished = false;
    while !old_finished || !controller_finished {
        if let Ok(event) = service.try_recv() {
            assert_eq!(event.id, 7002);
            match event.outcome {
                ActionOutcome::Started => {}
                ActionOutcome::Cancelled => {
                    assert_eq!(event.invocation_id, old);
                    assert!(!old_finished);
                    old_finished = true;
                }
                ActionOutcome::Succeeded { output } => {
                    assert_eq!(event.invocation_id, controller);
                    assert!(!controller_finished);
                    assert_eq!(output, json!({"stoppedCount":1,"effectState":"unknown"}));
                    assert!(state.dropped.load(Ordering::SeqCst));
                    assert_eq!(state.active.load(Ordering::SeqCst), 0);
                    assert!(weak.upgrade().is_none());
                    controller_finished = true;
                }
                ActionOutcome::Failed { error } => panic!("真实同绑定异步stop失败：{error:?}"),
            }
        }
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        continuation.recv_timeout(WAIT).unwrap(),
        json!({"stoppedCount":1,"effectState":"unknown"})
    );
    assert!(!state.continued_before_cleanup.load(Ordering::SeqCst));
    assert_eq!(state.synchronous_calls.load(Ordering::SeqCst), 0);
    assert!(!service.is_running(7002));
}
