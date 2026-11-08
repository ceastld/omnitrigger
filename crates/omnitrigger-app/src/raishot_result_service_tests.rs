//! 正式 ActionService 接缝；只替换启动端口，真实 PNG 验证器保持生产实现。
//! ignored：仅由操作者提供同源新 SDK 宿主后串行运行，不加载或启动 RaiShot。
use super::{LaunchPort, Store, WithRaiShotResult, CAPTURE};
use omnitrigger_actions::{
    script_host::ScriptHostRuntime, ActionCall, ActionError, ActionOutcome, ActionService,
    CancellationToken, NativeHost, NodeRuntime, PackageCatalog, RuntimeRegistry,
};
use omnitrigger_windows::{
    ProcessCaptureControl as Control, ProcessCaptureFailure as Failure,
    ProcessCaptureOptions as Options, ProcessCaptureResult as Captured,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
const ENTRY: &[u8] = include_bytes!("../../../examples/actions/raishot/result-script.mjs");
include!("raishot_result_service_pins.rs");
const OBSERVE: Duration = Duration::from_secs(64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Case {
    Unsupported,
    Complete,
    BadPath,
    BadHash,
    Unknown,
}
impl Case {
    fn name(self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported",
            Self::Complete => "complete",
            Self::BadPath => "bad_path",
            Self::BadHash => "bad_hash",
            Self::Unknown => "unknown",
        }
    }
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn assert_source_pins() {
    assert_eq!(ENTRY.len(), ENTRY_LENGTH);
    assert_eq!(hash(ENTRY), ENTRY_SHA);
    assert_eq!(hash(RESULT_FACTORY), RESULT_FACTORY_SHA);
    let desktop = SDK_SOURCES[0].1;
    assert_eq!(
        desktop
            .windows(RESULT_FACTORY.len())
            .filter(|w| *w == RESULT_FACTORY)
            .count(),
        1,
        "实际共享 SDK 必须含固定版本的结果工厂"
    );
    let wiring = b"...createRaiShotResultApi(invoke),";
    assert_eq!(
        desktop
            .windows(wiring.len())
            .filter(|w| *w == wiring)
            .count(),
        1,
        "普通 API 必须接到同一真实工厂"
    );
    assert_eq!(PNG.len(), PNG_LENGTH);
    assert_eq!(hash(PNG), PNG_SHA);
}
fn write_new(path: &Path, bytes: &[u8]) {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .expect("仅写本轮全新文件");
    file.write_all(bytes).unwrap();
    file.sync_all().unwrap();
}
fn explicit(name: &str, file: bool) -> PathBuf {
    let path = PathBuf::from(std::env::var_os(name).expect("需要显式提供自有本机路径"));
    assert!(path.is_absolute());
    crate::script_runtime::check_local_path(&path).expect("拒绝云盘、占位、联接点及不安全祖先");
    let info = fs::symlink_metadata(&path).unwrap();
    assert!(!info.file_type().is_symlink());
    assert_eq!(info.is_file(), file);
    assert_eq!(info.is_dir(), !file);
    if file {
        assert!(path
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("exe")));
    }
    path
}
fn owned_case_root(runtime: &str, case: Case) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent = explicit("OMNITRIGGER_RAISHOT_SERVICE_ROOT", false);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = parent.join(format!(
        "raishot-service-{runtime}-{}-{}-{nonce}-{}",
        case.name(),
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).expect("只创建本轮独占目录，不覆盖、复用或删除旧目录");
    crate::script_runtime::check_local_path(&root).unwrap();
    root
}

struct RecordingPort {
    program: PathBuf,
    config: PathBuf,
    case: Case,
    locate_count: AtomicUsize,
    calls: Mutex<Vec<Value>>,
    image_path: Mutex<Option<PathBuf>>,
}
impl LaunchPort for RecordingPort {
    fn locate(&self, config: &Path) -> Result<PathBuf, &'static str> {
        assert_eq!(config, self.config.as_path());
        assert_eq!(self.locate_count.fetch_add(1, Ordering::SeqCst), 0);
        Ok(self.program.clone())
    }
    fn capture(
        &self,
        program: &str,
        args: &[String],
        options: &Options,
        control: &mut dyn FnMut() -> Control,
    ) -> Result<Captured, Failure> {
        assert_eq!(program, self.program.to_str().unwrap());
        assert_eq!(
            options.cwd.as_deref(),
            self.program.parent().and_then(Path::to_str)
        );
        assert!(options.allow_non_zero);
        assert_eq!(control(), Control::Continue);
        let mut calls = self.calls.lock().unwrap();
        let index = calls.len();
        calls.push(json!({"program":program,"args":args,"timeoutMs":options.timeout_ms,"cwd":options.cwd.as_deref(),"portOnly":true}));
        assert!(index < 2, "未知结果绝不重试或回退第三次采集");
        if index == 0 {
            assert_eq!(args.len(), 1);
            assert_eq!(args[0], "--capture-result-capabilities");
            assert!((1..=3000).contains(&options.timeout_ms));
            let capabilities = if self.case == Case::Unsupported {
                json!({"protocol":"raishot.capture-copy-json","version":1,"command":"--capture-copy-json","in_process":true})
            } else {
                json!({"protocol":"raishot.capture-result-json","version":1,"command":"--capture-result-json","in_process":true,
                "max_lifetime_ms":30000,"max_png_bytes":67108864,"max_pixels":32000000,"image_transport":"owned_png_file"})
            };
            return Ok(Captured {
                code: 0,
                stdout: capabilities.to_string(),
                stderr: String::new(),
            });
        }
        assert_ne!(
            self.case,
            Case::Unsupported,
            "旧能力必须在创建会话和图像执行前拒绝"
        );
        assert_eq!(args.len(), 7);
        assert_eq!(args[0], "--capture-result-json");
        assert_eq!(args[1], "--request-id");
        assert_eq!(args[3], "--session-dir");
        assert_eq!(args[5], "--deadline-ms");
        assert!(super::protocol::hex(&args[2], 32));
        assert!((1..=30000).contains(&options.timeout_ms));
        let directory = PathBuf::from(&args[4]);
        let expected_parent = self.config.parent().unwrap().join("raishot-results");
        assert_eq!(directory.parent(), Some(expected_parent.as_path()));
        assert_eq!(
            directory.file_name().unwrap().to_str().unwrap(),
            format!("capture-{}", args[2])
        );
        assert!(fs::read_dir(&directory).unwrap().next().is_none());
        let deadline = args[6].parse::<u64>().unwrap();
        let now = u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap();
        assert!(deadline > now && deadline - now <= 30000);
        let image = directory.join("image.png");
        write_new(&image, PNG); // 静态夹具的真实文件，由生产 Session::verify 打开并读 SHA/PNG。
        *self.image_path.lock().unwrap() = Some(image);
        if self.case == Case::Unknown {
            return Err(Failure {
                code: "process_timeout",
                message: "记录端口返回未知结果",
                effect_state: "unknown",
            });
        }
        let descriptor = json!({"file_name":if self.case==Case::BadPath {"../image.png"} else {"image.png"},
            "format":"png","width":1,"height":1,"byte_length":PNG.len().to_string(),
            "sha256":if self.case==Case::BadHash {"0".repeat(64)} else {hash(PNG)}});
        let terminal = json!({"protocol":"raishot.capture-result-json","version":1,"request_id":args[2],"completion_scope":"png_file_only",
            "status":"completed","image":descriptor,"error_code":null,"image_effect_state":"confirmed","automatic_clipboard_written":false,
            "automatic_history_added":false,"source_auto_adsorb_verified":false,"retention":"caller_owned_until_explicit_release","warning_code":null});
        Ok(Captured {
            code: 0,
            stdout: terminal.to_string(),
            stderr: String::new(),
        })
    }
}
struct RejectOtherNative {
    calls: Arc<AtomicUsize>,
}
impl NativeHost for RejectOtherNative {
    fn call(&self, method: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut error = ActionError::new("unexpected_native_method", "测试不提供任何其它本机能力");
        error.method = Some(method.into());
        error.effect_state = Some("not_started".into());
        Err(error)
    }
}
struct RecordingNative {
    inner: WithRaiShotResult<RejectOtherNative>,
    calls: Mutex<Vec<Value>>,
    no_deadline: AtomicUsize,
    submitted_at: Mutex<Option<Instant>>,
}
impl NativeHost for RecordingNative {
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
            calls.push(json!({"method":method,"params":params.clone(),"deadlineRemainingMs":deadline.saturating_duration_since(Instant::now()).as_millis(),
                "deadlineFromSubmissionMs":deadline.saturating_duration_since(submitted).as_millis()}));
            index
        };
        assert_eq!(method, CAPTURE, "只允许本差集的图像结果调用");
        assert_eq!(params, json!({"timeout_ms":30000}));
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
fn terminal(service: &ActionService, invocation: u64) -> ActionOutcome {
    let end = Instant::now() + OBSERVE;
    let mut started = false;
    loop {
        match service.try_recv() {
            Ok(event) => {
                assert_eq!(event.id, 1);
                assert_eq!(event.invocation_id, invocation);
                match event.outcome {
                    ActionOutcome::Started => {
                        assert!(!started);
                        started = true;
                    }
                    outcome => {
                        assert!(started);
                        assert!(outcome.is_terminal());
                        return outcome;
                    }
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(error) => panic!("Service 事件通道异常：{error}"),
        }
        if Instant::now() >= end {
            service.cancel_id(1);
            panic!("64 秒外观察结束仍未拿到终态；未缩短默认 runtime 60 秒");
        }
        thread::sleep(Duration::from_millis(10));
    }
}
fn run_case(runtime: &str, program: &Path, case: Case) -> Value {
    assert_source_pins();
    let root = owned_case_root(runtime, case);
    let catalog_root = root.join("actions");
    let package = catalog_root.join("raishot");
    fs::create_dir(&catalog_root).unwrap();
    fs::create_dir(&package).unwrap();
    write_new(&package.join("result-script.mjs"), ENTRY);
    write_new(&package.join("action.json"),json!({"schema_version":1,"id":"examples.raishot","name":"RaiShot 文件结果接缝","version":"0.1.0",
        "actions":[{"id":"result-script","title":"图像文件结果","version":1,"runtime":runtime,"entry":"result-script.mjs","single_instance":true}]}).to_string().as_bytes());
    let catalog = PackageCatalog::load(&catalog_root);
    assert!(catalog.diagnostics.is_empty(), "动作目录必须无诊断");
    assert_eq!(catalog.actions.len(), 1);
    let mut runtimes = RuntimeRegistry::new();
    // 不调用 with_timeout、不预热、不改默认运行时期限。
    match runtime {
        "script" => runtimes
            .register("script", Arc::new(ScriptHostRuntime::new(program)))
            .unwrap(),
        "node" => runtimes
            .register("node", Arc::new(NodeRuntime::new(program)))
            .unwrap(),
        _ => panic!("固定差集不接受其它 runtime"),
    }
    let fixture_program = root.join("recording-port.exe");
    write_new(
        &fixture_program,
        b"OmniTrigger record-only port; not an executable image\n",
    );
    let config = root.join("settings.json");
    let port = Arc::new(RecordingPort {
        program: fixture_program,
        config: config.clone(),
        case,
        locate_count: AtomicUsize::new(0),
        calls: Mutex::new(Vec::new()),
        image_path: Mutex::new(None),
    });
    let mut store = Store::new(config);
    store.test_port = Some(port.clone());
    let unexpected = Arc::new(AtomicUsize::new(0));
    let host = Arc::new(RecordingNative {
        inner: WithRaiShotResult::new(
            RejectOtherNative {
                calls: unexpected.clone(),
            },
            Arc::new(store),
        ),
        calls: Mutex::new(Vec::new()),
        no_deadline: AtomicUsize::new(0),
        submitted_at: Mutex::new(None),
    });
    let service = ActionService::new(catalog, runtimes);
    let began = Instant::now();
    *host.submitted_at.lock().unwrap() = Some(began);
    let invocation = service
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
    let outcome = terminal(&service, invocation);
    assert_eq!(unexpected.load(Ordering::SeqCst), 0);
    assert_eq!(host.no_deadline.load(Ordering::SeqCst), 0);
    let native = host.calls.lock().unwrap().clone();
    assert_eq!(native.len(), 1, "actual Service outcome: {outcome:?}");
    assert!(native[0]["deadlineRemainingMs"]
        .as_u64()
        .is_some_and(|n| n > 0 && n <= 60000));
    assert!(
        native[0]["deadlineFromSubmissionMs"]
            .as_u64()
            .is_some_and(|n| n >= 60000),
        "不得通过缩短默认 runtime 时限跑通接缝"
    );
    assert_eq!(port.locate_count.load(Ordering::SeqCst), 1);
    let launches = port.calls.lock().unwrap().clone();
    assert_eq!(
        launches.len(),
        if case == Case::Unsupported { 1 } else { 2 }
    );
    let image = port.image_path.lock().unwrap().clone();
    let output = if case == Case::Unsupported {
        let ActionOutcome::Failed { error } = outcome else {
            panic!("旧能力不得通过 Service")
        };
        assert_eq!(error.code, "capture_result_unsupported");
        assert_eq!(error.effect_state.as_deref(), Some("not_started"));
        assert_eq!(error.method.as_deref(), Some(CAPTURE));
        assert_eq!(native[0]["response"]["ok"], false);
        assert_eq!(native[0]["response"]["code"], error.code);
        assert!(image.is_none());
        assert!(!root.join("raishot-results").exists());
        json!({"service":"failed","code":error.code,"effectState":error.effect_state})
    } else {
        let ActionOutcome::Succeeded { output } = outcome else {
            panic!("普通入口应返回业务状态")
        };
        assert_eq!(output["retryAllowed"], false);
        assert_eq!(output["completionScope"], "png_file_only");
        assert_eq!(output["automaticClipboardWritten"], false);
        assert_eq!(output["automaticHistoryAdded"], false);
        assert_eq!(output["sourceBusinessComplete"], false);
        assert_eq!(output["sourceAutoAdsorbVerified"], false);
        let image = image.as_ref().expect("记录端口应已创建静态 PNG");
        assert_eq!(hash(&fs::read(image).unwrap()), PNG_SHA);
        if case == Case::Complete {
            assert_eq!(output["status"], "completed");
            assert_eq!(output["artifactVerified"], true);
            assert_eq!(output["imageEffectState"], "confirmed");
            assert_eq!(output["image"]["sha256"], PNG_SHA);
            assert_eq!(output["image"]["byteLength"], PNG_LENGTH.to_string());
            assert_eq!(output["image"]["path"], image.to_str().unwrap());
            assert_eq!(output["image"]["width"], 1);
            assert_eq!(output["image"]["height"], 1);
            // 生产 verify 成功后才保管只读句柄；不能用伪造 artifactVerified 布尔值代替。
            assert!(
                OpenOptions::new().write(true).open(image).is_err(),
                "应由真实验证器保管 PNG，禁止写入"
            );
        } else {
            assert_eq!(output["status"], "failed");
            assert_eq!(output["artifactVerified"], false);
            assert_eq!(output["imageEffectState"], "unknown");
            assert!(output["image"].is_null());
            assert_eq!(
                output["errorCode"],
                if case == Case::Unknown {
                    "process_timeout"
                } else {
                    "capture_result_unconfirmed"
                }
            );
            if case == Case::BadHash {
                assert!(
                    OpenOptions::new().write(true).open(image).is_ok(),
                    "坏 SHA 必须使真实 verify 拒绝并放下临时读句柄"
                );
            }
        }
        assert!(serde_json::to_vec(&output).unwrap().len() < 4096);
        assert_eq!(native[0]["response"]["ok"], true);
        assert_eq!(
            native[0]["response"]["value"], output,
            "普通入口必须原样返回真实生产验证结果"
        );
        output
    };
    let receipt = json!({"schemaVersion":1,"runtime":runtime,"case":case.name(),"realActionService":true,"ordinaryEntryPinned":true,
        "runtimeDefaultSeconds":60,"observerSeconds":64,"observedMs":began.elapsed().as_millis(),"native":native,"launchPort":launches,
        "productionPngVerifierUnchanged":true,"staticFixturePngSha256":PNG_SHA,"output":output,
        "resultFactoryExactSha256":RESULT_FACTORY_SHA,"actualSdkSources":SDK_SOURCES.iter().map(|(name,bytes)|json!({"path":name,"bytes":bytes.len(),"sha256":hash(bytes)})).collect::<Vec<_>>(),
        "rustshotStarted":false,"screenCaptured":false,"clipboardAccessed":false,"historyAccessed":false,"uiStarted":false});
    write_new(
        &root.join("service-receipt.json"),
        serde_json::to_vec_pretty(&receipt).unwrap().as_slice(),
    );
    receipt
}

#[test]
#[ignore = "需同源新 SDK Script 宿主和本轮自有目录；需串行运行"]
fn script_service_raishot_new_rejections_and_unknown_use_real_entry() {
    let program = explicit("OMNITRIGGER_RAISHOT_SERVICE_SCRIPT_HOST", true);
    for case in [Case::Unsupported, Case::BadPath, Case::Unknown] {
        run_case("script", &program, case);
    }
}
#[test]
#[ignore = "需显式 Node 和本轮自有目录；需串行运行，不启动 RaiShot"]
fn node_service_raishot_completion_and_bad_hash_use_real_png_verifier() {
    let program = explicit("OMNITRIGGER_RAISHOT_SERVICE_NODE", true);
    for case in [Case::Complete, Case::BadHash] {
        run_case("node", &program, case);
    }
}

#[cfg(test)]
mod release_lifecycle {
    include!("raishot_result_release_service_test.rs");
}
