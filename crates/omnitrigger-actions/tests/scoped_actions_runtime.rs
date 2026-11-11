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
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};
static NEXT: AtomicU64 = AtomicU64::new(1);
const WAIT: Duration = Duration::from_secs(90);

struct OldHost {
    entered: mpsc::Sender<()>,
    dropped: Arc<AtomicBool>,
}
impl NativeHost for OldHost {
    fn call(
        &self,
        method: &str,
        _: Value,
        cancellation: &CancellationToken,
    ) -> Result<Value, ActionError> {
        assert_eq!(method, "target.info");
        self.entered.send(()).unwrap();
        let deadline = Instant::now() + WAIT;
        while !cancellation.is_cancelled() {
            assert!(Instant::now() < deadline, "合成旧调用未收到取消");
            thread::sleep(Duration::from_millis(1));
        }
        Err(ActionError::new("cancelled", "合成旧调用已取消。"))
    }
}
impl Drop for OldHost {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

fn run(runtime_name: &str, runtime: Arc<dyn ActionRuntime>) {
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
    let root = (0..1024)
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
        .expect("未找到可原子创建的合成验收目录");
    let package = root.join("scoped-real");
    fs::create_dir(&package).unwrap();
    let source = if runtime_name == "node" {
        "export default async function main(ctx){if(ctx.input.role==='old')return await ctx.desktop.target();return await ctx.actions.stopOtherInstances({timeoutMs:2000});}\n"
    } else {
        "export function main(ctx){if(ctx.input.role==='old')return ctx.desktop.target();return ctx.actions.stopOtherInstances({timeoutMs:2000});}\n"
    };
    fs::write(package.join("main.mjs"), source).unwrap();
    fs::write(package.join("action.json"),serde_json::to_vec(&json!({"schema_version":1,"id":"scoped-real","name":"真实调用作用域合成验收","version":"1.0.0","actions":[{"id":"run","title":"停止其他实例","version":1,"runtime":runtime_name,"entry":"main.mjs","single_instance":false}]})).unwrap()).unwrap();
    let catalog = PackageCatalog::load(&root);
    assert!(catalog.diagnostics.is_empty(), "{:?}", catalog.diagnostics);
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
    let old = service
        .submit_with_host(
            1,
            call("old"),
            Some(Arc::new(OldHost {
                entered: entered_tx,
                dropped: dropped.clone(),
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
    let controller = service.submit(2, call("controller")).unwrap();
    let deadline = Instant::now() + WAIT;
    let mut terminals = 0;
    let mut saw_controller = false;
    while terminals < 2 {
        if let Ok(event) = service.try_recv() {
            match event.outcome {
                ActionOutcome::Started => {}
                ActionOutcome::Succeeded { output } => {
                    assert_eq!(event.invocation_id, controller);
                    assert!(dropped.load(Ordering::SeqCst), "回执早于旧宿主清理");
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
    );
}
