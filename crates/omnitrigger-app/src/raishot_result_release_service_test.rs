// 同一宿主进程、ActionService和Store，经逐字普通入口连续捕获和释放。
// 只替换既有定位/采集端口，不启动RaiShot，不访问屏幕、剪贴板或个人配置。
use super::super::RELEASE;
use super::*;

struct LifecycleNative {
    inner: WithRaiShotResult<RejectOtherNative>,
    calls: Mutex<Vec<Value>>,
    no_deadline: AtomicUsize,
    submitted_at: Mutex<Option<Instant>>,
    release_id: Mutex<Option<String>>,
}
impl NativeHost for LifecycleNative {
    fn call(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
    ) -> Result<Value, ActionError> {
        self.no_deadline.fetch_add(1, Ordering::SeqCst);
        self.inner.call(method, params, cancellation)
    }
    fn call_with_deadline(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<Value, ActionError> {
        let submitted = self
            .submitted_at
            .lock()
            .unwrap()
            .expect("需先记录正式提交时间");
        let index = {
            let mut calls = self.calls.lock().unwrap();
            let index = calls.len();
            calls.push(json!({"method":method,"params":params.clone(),
                "deadlineRemainingMs":deadline.saturating_duration_since(Instant::now()).as_millis(),
                "deadlineFromSubmissionMs":deadline.saturating_duration_since(submitted).as_millis()}));
            index
        };
        match index {
            0 => {
                assert_eq!(method, CAPTURE);
                assert_eq!(params, json!({"timeout_ms":30000}));
            }
            1 => {
                assert_eq!(method, RELEASE);
                let id = self
                    .release_id
                    .lock()
                    .unwrap()
                    .clone()
                    .expect("先由捕获终态取得请求ID");
                assert_eq!(params, json!({"request_id":id}));
            }
            _ => panic!("仅允许一次捕获和一次显式释放，不增加错误或重试矩阵"),
        }
        let response = self
            .inner
            .call_with_deadline(method, params, cancellation, deadline);
        self.calls.lock().unwrap()[index]["response"] = match &response {
            Ok(value) => json!({"ok":true,"value":value}),
            Err(error) => {
                json!({"ok":false,"code":error.code,"effectState":error.effect_state,"method":error.method})
            }
        };
        response
    }
}

#[test]
#[ignore = "需显式Node和本轮自有目录；串行运行同源普通入口，不启动RaiShot"]
fn node_service_raishot_capture_then_release_removes_owned_png_and_registration() {
    assert_source_pins();
    let program = explicit("OMNITRIGGER_RAISHOT_SERVICE_NODE", true);
    let root = owned_case_root("node-release-lifecycle", Case::Complete);
    let catalog_root = root.join("actions");
    let package = catalog_root.join("raishot");
    fs::create_dir(&catalog_root).unwrap();
    fs::create_dir(&package).unwrap();
    write_new(&package.join("result-script.mjs"), ENTRY);
    write_new(&package.join("action.json"), json!({"schema_version":1,"id":"examples.raishot","name":"RaiShot 文件释放接缝","version":"0.1.0",
        "actions":[{"id":"result-script","title":"图像文件结果","version":1,"runtime":"node","entry":"result-script.mjs","single_instance":true}]}).to_string().as_bytes());
    let catalog = PackageCatalog::load(&catalog_root);
    assert!(catalog.diagnostics.is_empty());
    assert_eq!(catalog.actions.len(), 1);
    let mut runtimes = RuntimeRegistry::new();
    // 使用原NodeRuntime，不预热，不调用with_timeout；每次调用默认60秒。
    runtimes
        .register("node", Arc::new(NodeRuntime::new(program)))
        .unwrap();
    let fixture_program = root.join("recording-port.exe");
    write_new(
        &fixture_program,
        b"OmniTrigger record-only port; not an executable image\n",
    );
    let config = root.join("settings.json");
    let port = Arc::new(RecordingPort {
        program: fixture_program,
        config: config.clone(),
        case: Case::Complete,
        locate_count: AtomicUsize::new(0),
        calls: Mutex::new(Vec::new()),
        image_path: Mutex::new(None),
    });
    let mut store = Store::new(config);
    store.test_port = Some(port.clone());
    let store = Arc::new(store);
    let unexpected = Arc::new(AtomicUsize::new(0));
    let host = Arc::new(LifecycleNative {
        inner: WithRaiShotResult::new(
            RejectOtherNative {
                calls: unexpected.clone(),
            },
            store.clone(),
        ),
        calls: Mutex::new(Vec::new()),
        no_deadline: AtomicUsize::new(0),
        submitted_at: Mutex::new(None),
        release_id: Mutex::new(None),
    });
    assert!(Arc::ptr_eq(&store, &host.inner.store));
    assert!(store.sessions.lock().unwrap().is_empty());
    let service = ActionService::new(catalog, runtimes);
    let began = Instant::now();
    *host.submitted_at.lock().unwrap() = Some(began);
    let capture_invocation = service
        .submit_with_host(
            1,
            ActionCall {
                package_id: "examples.raishot".into(),
                action_id: "result-script".into(),
                action_version: 1,
                input: json!({"operation":"capture","timeoutMs":30000}),
            },
            Some(host.clone()),
        )
        .unwrap();
    let capture_outcome = terminal(&service, capture_invocation);
    let ActionOutcome::Succeeded { output: capture } = capture_outcome else {
        panic!("捕获应通过真实Node入口返回成功终态：{capture_outcome:?}")
    };
    assert_eq!(capture["status"], "completed");
    assert_eq!(capture["artifactVerified"], true);
    assert_eq!(capture["imageEffectState"], "confirmed");
    assert_eq!(capture["completionScope"], "png_file_only");
    assert_eq!(capture["retryAllowed"], false);
    for field in [
        "automaticClipboardWritten",
        "automaticHistoryAdded",
        "sourceBusinessComplete",
        "sourceAutoAdsorbVerified",
    ] {
        assert_eq!(capture[field], false);
    }
    let request_id = capture["requestId"]
        .as_str()
        .expect("生产捕获返回请求ID")
        .to_owned();
    assert!(super::super::protocol::hex(&request_id, 32));
    let image = port
        .image_path
        .lock()
        .unwrap()
        .clone()
        .expect("记录端口创建自有静态PNG");
    let directory = root
        .join("raishot-results")
        .join(format!("capture-{request_id}"));
    assert_eq!(image, directory.join("image.png"));
    assert_eq!(capture["image"]["path"], image.to_str().unwrap());
    assert_eq!(capture["image"]["sha256"], PNG_SHA);
    assert_eq!(capture["image"]["byteLength"], PNG_LENGTH.to_string());
    assert_eq!(capture["image"]["width"], 1);
    assert_eq!(capture["image"]["height"], 1);
    assert_eq!(hash(&fs::read(&image).unwrap()), PNG_SHA);
    assert!(
        OpenOptions::new().write(true).open(&image).is_err(),
        "释放前必须由真实验证器保管只读PNG句柄"
    );
    assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
    let directory_path = fs::canonicalize(&directory).unwrap();
    {
        let entries = store.sessions.lock().unwrap();
        assert_eq!(entries.len(), 1);
        let entry = entries.get(&request_id).expect("生产Store已登记捕获引用");
        assert!(!entry.quarantined);
        assert!(entry.session.is_some());
    }
    assert_eq!(port.locate_count.load(Ordering::SeqCst), 1);
    assert_eq!(port.calls.lock().unwrap().len(), 2);
    assert_eq!(host.calls.lock().unwrap().len(), 1);
    *host.release_id.lock().unwrap() = Some(request_id.clone());

    // 同一Service/host/Arc<Store>，仅改变普通入口的operation和requestId输入。
    *host.submitted_at.lock().unwrap() = Some(Instant::now());
    let release_invocation = service
        .submit_with_host(
            1,
            ActionCall {
                package_id: "examples.raishot".into(),
                action_id: "result-script".into(),
                action_version: 1,
                input: json!({"operation":"release","requestId":request_id}),
            },
            Some(host.clone()),
        )
        .unwrap();
    assert_ne!(capture_invocation, release_invocation);
    let release_outcome = terminal(&service, release_invocation);
    let ActionOutcome::Succeeded { output: release } = release_outcome else {
        panic!("释放应通过同一Store的真实Node入口成功：{release_outcome:?}")
    };
    assert_eq!(
        release,
        json!({"released":true,"requestId":request_id,"scope":"owned_png_file","sessionDirectoryRetained":true})
    );
    assert!(
        matches!(fs::symlink_metadata(&image), Err(error) if error.kind() == std::io::ErrorKind::NotFound),
        "必须确认PNG项不存在，不能把权限错误当作已删除"
    );
    let directory_metadata = fs::symlink_metadata(&directory).expect("会话目录应保留");
    assert!(directory_metadata.is_dir());
    assert!(!directory_metadata.file_type().is_symlink());
    crate::script_runtime::check_local_path(&directory).unwrap();
    assert_eq!(fs::canonicalize(&directory).unwrap(), directory_path);
    assert!(fs::read_dir(&directory).unwrap().next().is_none());
    assert!(Arc::ptr_eq(&store, &host.inner.store));
    {
        let entries = store.sessions.lock().unwrap();
        assert!(
            !entries.contains_key(&request_id),
            "显式释放必须移除原注册引用"
        );
        assert!(entries.is_empty());
    }
    assert_eq!(unexpected.load(Ordering::SeqCst), 0);
    assert_eq!(host.no_deadline.load(Ordering::SeqCst), 0);
    assert_eq!(
        port.locate_count.load(Ordering::SeqCst),
        1,
        "释放不重新定位或启动程序"
    );
    let launches = port.calls.lock().unwrap().clone();
    assert_eq!(launches.len(), 2, "释放不能产生新的能力探测或采集");
    let native = host.calls.lock().unwrap().clone();
    assert_eq!(native.len(), 2);
    for (call, output) in native.iter().zip([&capture, &release]) {
        assert_eq!(call["response"]["ok"], true);
        assert_eq!(
            call["response"]["value"], *output,
            "普通入口原样返回生产结果"
        );
        assert!(call["deadlineRemainingMs"]
            .as_u64()
            .is_some_and(|n| n > 0 && n <= 60000));
        assert!(
            call["deadlineFromSubmissionMs"]
                .as_u64()
                .is_some_and(|n| n >= 60000),
            "两次调用均保留默认Runtime60秒"
        );
        assert!(serde_json::to_vec(output).unwrap().len() < 4096);
    }
    assert!(matches!(
        service.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    let receipt = json!({"schemaVersion":1,"case":"node_capture_then_release","realActionService":true,
        "sameHostProcessId":std::process::id(),"sameActionService":true,"sameStoreArc":true,
        "ordinaryEntryPinned":true,"captureInvocation":capture_invocation,"releaseInvocation":release_invocation,
        "runtimeDefaultSeconds":60,"observerSecondsPerInvocation":64,"observedMs":began.elapsed().as_millis(),
        "native":native,"launchPort":launches,"capture":capture,"release":release,
        "imageMissing":true,"sessionDirectoryRetained":true,"registryEntryRemoved":true,"remainingEntries":0,
        "staticFixturePngSha256":PNG_SHA,"resultFactoryExactSha256":RESULT_FACTORY_SHA,
        "productionSources":[
            {"path":"raishot_result_host.rs","sha256":hash(include_bytes!("raishot_result_host.rs"))},
            {"path":"raishot_owned_png.rs","sha256":hash(include_bytes!("raishot_owned_png.rs"))},
            {"path":"raishot_result_protocol.rs","sha256":hash(include_bytes!("raishot_result_protocol.rs"))}],
        "actualSdkSources":SDK_SOURCES.iter().map(|(name,bytes)|json!({"path":name,"bytes":bytes.len(),"sha256":hash(bytes)})).collect::<Vec<_>>(),
        "rustshotStarted":false,"screenCaptured":false,"clipboardAccessed":false,"historyAccessed":false,"uiStarted":false});
    write_new(
        &root.join("release-service-receipt.json"),
        serde_json::to_vec_pretty(&receipt).unwrap().as_slice(),
    );
}
