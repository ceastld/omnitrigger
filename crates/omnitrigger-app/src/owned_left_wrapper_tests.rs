// 由action_packages.rs现有tests模块include；复用其Fixture，不加载个人设置或执行Native输入。
mod owned_left_wrapper_regressions {
    use super::*;
    use omnitrigger_actions::{
        owned_mouse::{self, OwnedInputOwner, OwnedLeftBackend, OwnedLeftSession, DOWN, UP},
        ActionOutcome, NativeHost,
    };
    use std::{
        sync::{atomic::AtomicUsize, Mutex},
        thread,
        time::{Duration, Instant},
    };

    #[derive(Default)]
    struct Backend {
        held: Mutex<Option<u64>>,
        edges: Mutex<Vec<(u64, bool)>>,
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
            if cancelled() {
                return Err(owned_mouse::fault(DOWN, "cancelled", false));
            }
            let mut held = self.held.lock().unwrap();
            if held.is_some() {
                return Err(owned_mouse::fault(DOWN, "input_busy", false));
            }
            *held = Some(owner.id());
            self.edges.lock().unwrap().push((owner.id(), true));
            Ok(())
        }
        fn release_owned(&self, owner: OwnedInputOwner) -> Result<(), ActionError> {
            let mut held = self.held.lock().unwrap();
            if *held == Some(owner.id()) {
                *held = None;
                self.edges.lock().unwrap().push((owner.id(), false));
            }
            Ok(())
        }
    }
    struct Factory {
        current: Mutex<Arc<Backend>>,
        calls: AtomicUsize,
        owners: Mutex<Vec<u64>>,
    }
    impl NativeHost for Factory {
        fn create_owned_left_input(
            &self,
            owner: OwnedInputOwner,
        ) -> Result<Arc<OwnedLeftSession>, ActionError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.owners.lock().unwrap().push(owner.id());
            let backend = self.current.lock().unwrap().clone();
            Ok(Arc::new(OwnedLeftSession::new(owner, backend)))
        }
        fn call(&self, _: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
            panic!("鼠标应由原InvocationScopedHost拦截")
        }
    }
    struct FactoryHost(Arc<Factory>);
    impl NativeHost for FactoryHost {
        fn create_owned_left_input(
            &self,
            owner: OwnedInputOwner,
        ) -> Result<Arc<OwnedLeftSession>, ActionError> {
            self.0.create_owned_left_input(owner)
        }
        fn call(&self, _: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
            panic!("不应走未知Native方法")
        }
    }
    struct Runtime {
        replacement: Option<(Arc<Factory>, Arc<Backend>)>,
    }
    impl ActionRuntime for Runtime {
        fn execute(
            &self,
            _: &ResolvedAction,
            _: &InvocationContext,
            _: &CancellationToken,
        ) -> Result<Value, ActionError> {
            panic!("需要真实Service作用域")
        }
        fn execute_with_host(
            &self,
            _: &ResolvedAction,
            _: &InvocationContext,
            token: &CancellationToken,
            host: Option<&dyn NativeHost>,
        ) -> Result<Value, ActionError> {
            let host = host.unwrap();
            host.call_with_deadline(
                DOWN,
                json!({}),
                token,
                Instant::now() + Duration::from_secs(60),
            )?;
            if let Some((factory, replacement)) = &self.replacement {
                *factory.current.lock().unwrap() = replacement.clone();
            }
            host.call_with_deadline(
                UP,
                json!({}),
                token,
                Instant::now() + Duration::from_secs(60),
            )?;
            // 新按压周期允许自己的5次预算，但同一会话不重建工厂或换Backend。
            host.call_with_deadline(
                DOWN,
                json!({}),
                token,
                Instant::now() + Duration::from_secs(60),
            )?;
            host.call_with_deadline(
                UP,
                json!({}),
                token,
                Instant::now() + Duration::from_secs(60),
            )?;
            Ok(Value::Null)
        }
    }
    fn invoke(fixture: &Fixture, host: Arc<dyn NativeHost>, runtime: Runtime) -> ActionOutcome {
        let call = fixture.package(Some(json!(true)));
        let catalog = PackageCatalog::load(&root(&fixture.config));
        assert!(catalog.diagnostics.is_empty());
        let mut runtimes = RuntimeRegistry::new();
        runtimes.register("app-test", Arc::new(runtime)).unwrap();
        let service = ActionService::new(catalog, runtimes);
        service.submit_with_host(7, call, Some(host)).unwrap();
        let until = Instant::now() + Duration::from_secs(5);
        let mut started = false;
        loop {
            assert!(Instant::now() < until, "纯包装链回归超时");
            match service.try_recv() {
                Ok(event) => match event.outcome {
                    ActionOutcome::Started => {
                        assert!(!started);
                        started = true;
                    }
                    terminal => {
                        assert!(started && terminal.is_terminal());
                        assert!(!service.is_running(7));
                        return terminal;
                    }
                },
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    thread::sleep(Duration::from_millis(1))
                }
                Err(error) => panic!("Service事件断开：{error}"),
            }
        }
    }

    #[test]
    fn full_production_chain_reaches_the_original_desktop_factory_without_native_calls() {
        let fixture = Fixture::new();
        let desktop = crate::desktop_host::DesktopHost::owned_input_factory_probe();
        let browser = crate::script_browser::ActionHost::new(fixture.config.clone(), desktop);
        let ui = crate::script_ui::Host::new(browser, None);
        let wrapped = with_native_stores(
            ui,
            None,
            Arc::new(crate::raishot_result_host::Store::new(
                fixture.config.clone(),
            )),
        );
        match invoke(&fixture, Arc::new(wrapped), Runtime { replacement: None }) {
            ActionOutcome::Failed { error } => assert_eq!(error.code, "owned_factory_probe"),
            other => panic!("必须沿所有真实包装层到原Desktop工厂：{other:?}"),
        }
    }

    #[test]
    fn production_store_wrappers_create_one_owner_and_never_switch_backend_on_release() {
        let fixture = Fixture::new();
        let original = Arc::new(Backend::default());
        let replacement = Arc::new(Backend::default());
        let factory = Arc::new(Factory {
            current: Mutex::new(original.clone()),
            calls: AtomicUsize::new(0),
            owners: Mutex::new(Vec::new()),
        });
        let ui = crate::script_ui::Host::new(FactoryHost(factory.clone()), None);
        let wrapped = with_native_stores(
            ui,
            None,
            Arc::new(crate::raishot_result_host::Store::new(
                fixture.config.clone(),
            )),
        );
        let outcome = invoke(
            &fixture,
            Arc::new(wrapped),
            Runtime {
                replacement: Some((factory.clone(), replacement.clone())),
            },
        );
        assert!(matches!(outcome, ActionOutcome::Succeeded { .. }));
        assert_eq!(factory.calls.load(Ordering::Relaxed), 1);
        let owners = factory.owners.lock().unwrap();
        assert_eq!(owners.len(), 1);
        assert_ne!(owners[0], 0);
        assert_eq!(
            *original.edges.lock().unwrap(),
            [
                (owners[0], true),
                (owners[0], false),
                (owners[0], true),
                (owners[0], false)
            ]
        );
        assert!(replacement.edges.lock().unwrap().is_empty());
        assert_eq!(*original.held.lock().unwrap(), None);
    }
}
