//! 纯Rust Service生命周期回归；合成Backend，无进程/Hook/输入。
use super::*;
use crate::{
    ActionCall, ActionOutcome, ActionRuntime, ActionService, InvocationContext, NativeHost,
    PackageCatalog, ResolvedAction, RuntimeRegistry,
};
use std::{
    fs, thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
#[derive(Default)]
struct Backend {
    events: Mutex<Vec<&'static str>>,
    fail_release: std::sync::atomic::AtomicBool,
}
impl OwnedLeftBackend for Backend {
    fn check_original_target(&self) -> Result<(), ActionError> {
        Ok(())
    }
    fn down_owned(
        &self,
        _: OwnedInputOwner,
        _: Instant,
        _: &dyn Fn() -> bool,
    ) -> Result<(), ActionError> {
        self.events.lock().unwrap().push("down");
        Ok(())
    }
    fn release_owned(&self, _: OwnedInputOwner) -> Result<(), ActionError> {
        self.events.lock().unwrap().push("up");
        if self.fail_release.load(Ordering::Relaxed) {
            Err(fault(UP, "input_failed", true))
        } else {
            Ok(())
        }
    }
}
struct Host(Arc<Backend>);
impl NativeHost for Host {
    fn create_owned_left_input(
        &self,
        owner: OwnedInputOwner,
    ) -> Result<Arc<OwnedLeftSession>, ActionError> {
        Ok(Arc::new(OwnedLeftSession::new(owner, self.0.clone())))
    }
    fn call(&self, _: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
        panic!("必须走Service作用域")
    }
}
struct Runtime(u8);
impl ActionRuntime for Runtime {
    fn execute(
        &self,
        _: &ResolvedAction,
        _: &InvocationContext,
        _: &CancellationToken,
    ) -> Result<Value, ActionError> {
        panic!("需要真实作用域")
    }
    fn execute_with_host(
        &self,
        _: &ResolvedAction,
        _: &InvocationContext,
        token: &CancellationToken,
        host: Option<&dyn NativeHost>,
    ) -> Result<Value, ActionError> {
        let host = host.unwrap();
        if self.0 == 4 {
            // 尚未创建会话时关闭作用域，迟到的Down也不能再建会话。
            host.finish_owned_inputs()?;
            return host.call_with_deadline(
                DOWN,
                json!({}),
                token,
                Instant::now() + Duration::from_secs(1),
            );
        }
        host.call_with_deadline(
            DOWN,
            json!({}),
            token,
            Instant::now() + Duration::from_secs(1),
        )?;
        match self.0 {
            0 => Ok(Value::Null),
            1 => Err(ActionError::new("synthetic_failure", "合成异常")),
            2 | 5 => {
                token.cancel();
                Err(ActionError::new("cancelled", "合成取消"))
            }
            _ => panic!("合成运行时panic"),
        }
    }
}
fn check_local_ancestors(path: &std::path::Path) {
    for ancestor in path.ancestors() {
        let name = ancestor
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase();
        assert!(!name.starts_with("onedrive") && name != "dropbox" && !name.starts_with("icloud"));
        let metadata = fs::symlink_metadata(ancestor).unwrap();
        assert!(!metadata.file_type().is_symlink());
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            assert_eq!(metadata.file_attributes() & 0x441400, 0);
        }
    }
}
#[test]
fn ordinary_service_finishes_owned_left_before_success_failure_cancel_or_panic_terminal() {
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    check_local_ancestors(&manifest);
    let workspace = manifest.join("../..").canonicalize().unwrap();
    check_local_ancestors(&workspace);
    let parent = workspace.join(".tmp");
    if parent.exists() {
        check_local_ancestors(&parent);
    } else {
        fs::create_dir(&parent).unwrap();
    }
    for mode in 0..7 {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = parent.join(format!(
            "owned-left-service-{}-{mode}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        let package = root.join("package");
        fs::create_dir(&package).unwrap();
        fs::write(package.join("main.mjs"), "// never executed\n").unwrap();
        fs::write(package.join("action.json"),serde_json::to_vec(&json!({
            "schema_version":1,"id":"test.owned-left","name":"自有左键Service夹具","version":"0.0.0",
            "actions":[{"id":"work","title":"合成","version":1,"runtime":"script","entry":"main.mjs","single_instance":true}]
        })).unwrap()).unwrap();
        let catalog = PackageCatalog::load(&root);
        assert!(catalog.diagnostics.is_empty(), "{:?}", catalog.diagnostics);
        let mut runtimes = RuntimeRegistry::new();
        runtimes
            .register("script", Arc::new(Runtime(mode)))
            .unwrap();
        let service = ActionService::new(catalog, runtimes);
        let backend = Arc::new(Backend::default());
        backend
            .fail_release
            .store(mode == 5 || mode == 6, Ordering::Relaxed);
        let host = Arc::new(Host(backend.clone()));
        service
            .submit_with_host(
                7,
                ActionCall {
                    package_id: "test.owned-left".into(),
                    action_id: "work".into(),
                    action_version: 1,
                    input: json!({}),
                },
                Some(host),
            )
            .unwrap();
        let until = Instant::now() + Duration::from_secs(3);
        let mut started = false;
        loop {
            assert!(Instant::now() < until);
            match service.try_recv() {
                Ok(event) => match event.outcome {
                    ActionOutcome::Started => {
                        assert!(!started);
                        started = true;
                    }
                    terminal => {
                        assert!(started && terminal.is_terminal());
                        let expected: &[&str] = if mode == 4 {
                            &[]
                        } else if mode == 5 || mode == 6 {
                            &["down", "up", "up", "up", "up", "up"]
                        } else {
                            &["down", "up"]
                        };
                        assert_eq!(
                            backend.events.lock().unwrap().as_slice(),
                            expected,
                            "终态前必须收尾且不重放Down"
                        );
                        match (mode, terminal) {
                            (0, ActionOutcome::Succeeded { .. })
                            | (2, ActionOutcome::Cancelled) => {}
                            (1, ActionOutcome::Failed { error }) => {
                                assert_eq!(error.code, "synthetic_failure")
                            }
                            (3, ActionOutcome::Failed { error }) => {
                                assert_eq!(error.code, "runtime_panic")
                            }
                            (4, ActionOutcome::Failed { error }) => {
                                assert_eq!(error.code, "action_inactive")
                            }
                            (5 | 6, ActionOutcome::Failed { error }) => {
                                assert_eq!(error.code, "input_cleanup_failed");
                                assert_eq!(error.effect_state.as_deref(), Some("unknown"));
                            }
                            other => panic!("错误终态：{other:?}"),
                        }
                        assert!(!service.is_running(7));
                        break;
                    }
                },
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    thread::sleep(Duration::from_millis(1))
                }
                Err(error) => panic!("{error}"),
            }
        }
        fs::remove_file(package.join("main.mjs")).unwrap();
        fs::remove_file(package.join("action.json")).unwrap();
        fs::remove_dir(&package).unwrap();
        fs::remove_dir(&root).unwrap();
    }
}
