//! 输出通知动作包的正式服务回归；显式运行忽略测试，复用真实进程捕获后端。
use super::WithProcessCapture;
use omnitrigger_actions::{
    ActionCall, ActionError, ActionOutcome, ActionService, CancellationToken, NativeHost,
    NodeRuntime, PackageCatalog, RuntimeRegistry, ScriptHostRuntime,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows::Win32::{
    Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::Threading::{
        OpenProcess, WaitForSingleObject, PROCESS_ACCESS_RIGHTS, PROCESS_QUERY_LIMITED_INFORMATION,
    },
};

const STDOUT: &str = "\u{3000} \t中文😀 \r\n";
const STDERR: &str = "合成诊断\n";
const FILES: &[(&str, &[u8])] = &[
    (
        "action.json",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/actions/process-output/action.json"
        )),
    ),
    (
        "main.mjs",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/actions/process-output/main.mjs"
        )),
    ),
    (
        "main-capture.mjs",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/actions/process-output/main-capture.mjs"
        )),
    ),
    (
        "main-script.mjs",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/actions/process-output/main-script.mjs"
        )),
    ),
    (
        "workflow.mjs",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/actions/process-output/workflow.mjs"
        )),
    ),
];
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Node,
    Script,
}
impl Mode {
    fn selected() -> Self {
        match std::env::var("OMNITRIGGER_FALSE_CONTEXT_RUNTIME") {
            Ok(v) if v == "node" => Self::Node,
            Ok(v) if v == "script" => Self::Script,
            _ => panic!("必须显式选择node/script"),
        }
    }
    fn wait(self) -> Duration {
        Duration::from_secs(if self == Self::Node { 64 } else { 16 })
    }
    fn program(self) -> PathBuf {
        explicit(if self == Self::Node {
            "OMNITRIGGER_TEST_NODE"
        } else {
            "OMNITRIGGER_TEST_SCRIPT_HOST"
        })
    }
    fn action(self) -> &'static str {
        if self == Self::Node {
            "notify-capture"
        } else {
            "notify-script"
        }
    }
}
fn local(path: &Path) {
    use std::os::windows::fs::MetadataExt;
    assert!(path.is_absolute());
    for part in path.ancestors() {
        let info = fs::symlink_metadata(part).unwrap();
        let name = part
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase();
        assert!(!name.starts_with("onedrive") && !name.starts_with("icloud") && name != "dropbox");
        assert!(!info.file_type().is_symlink() && info.file_attributes() & 0x441400 == 0);
    }
}
fn explicit(name: &str) -> PathBuf {
    let path = PathBuf::from(std::env::var_os(name).expect("必须显式提供本地正常程序"));
    local(&path);
    assert!(path.is_file());
    assert!(path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("exe")));
    path
}
struct Fixture {
    root: PathBuf,
    actions: PathBuf,
    package: PathBuf,
    token: String,
}
impl Fixture {
    fn new() -> Self {
        let parent = std::env::temp_dir();
        local(&parent);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let token = format!("{:032x}", nonce ^ u128::from(std::process::id()));
        let root = parent.join(format!("capture-case-q5-{}-{token}", std::process::id()));
        fs::create_dir(&root).unwrap();
        local(&root);
        let actions = root.join("actions");
        fs::create_dir(&actions).unwrap();
        let package = actions.join("process-output");
        fs::create_dir(&package).unwrap();
        for (name, bytes) in FILES {
            fs::write(package.join(name), bytes).unwrap();
        }
        Self {
            root,
            actions,
            package,
            token,
        }
    }
    fn input(&self, program: &Path, case: Case) -> Value {
        let args = match case {
            Case::Nonzero | Case::Allowed => json!(["exit", "7"]),
            Case::InvalidUtf8 => json!(["invalid-utf8"]),
            Case::Timeout => json!(["sleep", "15000"]),
            Case::Cancel => json!(["descendant", self.root, self.token]),
            _ => json!(["q5-utf8-trim"]),
        };
        json!({"program":program,"args":args,"cwd":self.root,"allowNonZero":case==Case::Allowed,"trimOutput":case!=Case::Untrimmed,"timeoutMs":if case==Case::Timeout {200} else {10000}})
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if thread::panicking() {
            return;
        }
        for (name, _) in FILES {
            fs::remove_file(self.package.join(name)).unwrap();
        }
        fs::remove_dir(&self.package).unwrap();
        fs::remove_dir(&self.actions).unwrap();
        for name in ["owned-pids.json", "fixture-ready"] {
            let p = self.root.join(name);
            if p.exists() {
                fs::remove_file(p).unwrap();
            }
        }
        fs::remove_dir(&self.root).unwrap();
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Case {
    Trimmed,
    Untrimmed,
    Nonzero,
    Allowed,
    InvalidUtf8,
    Timeout,
    NotifyUnknown,
    Cancel,
}
#[derive(Default)]
struct State {
    captures: Mutex<Vec<(Value, Result<Value, ActionError>)>>,
    notifications: Mutex<Vec<Value>>,
}
struct NotifyHost {
    state: Arc<State>,
    fail: bool,
}
impl NativeHost for NotifyHost {
    fn call(
        &self,
        method: &str,
        params: Value,
        _: &CancellationToken,
    ) -> Result<Value, ActionError> {
        assert_eq!(method, "ui.notify", "通知Host不允许其它Native方法");
        self.state.notifications.lock().unwrap().push(params);
        if self.fail {
            let mut error = ActionError::new("ui_unavailable", "合成通知拒绝");
            error.method = Some(method.into());
            error.effect_state = Some("not_started".into());
            Err(error)
        } else {
            Ok(json!({"submitted":true}))
        }
    }
}
struct RecordingHost {
    inner: WithProcessCapture<NotifyHost>,
    state: Arc<State>,
    expected: Value,
}
impl NativeHost for RecordingHost {
    fn call(
        &self,
        method: &str,
        params: Value,
        cancel: &CancellationToken,
    ) -> Result<Value, ActionError> {
        self.call_with_deadline(
            method,
            params,
            cancel,
            Instant::now() + Duration::from_secs(60),
        )
    }
    fn call_with_deadline(
        &self,
        method: &str,
        params: Value,
        cancel: &CancellationToken,
        deadline: Instant,
    ) -> Result<Value, ActionError> {
        if method == "process.capture" {
            assert_eq!(params, self.expected);
            let result = self
                .inner
                .call_with_deadline(method, params.clone(), cancel, deadline);
            self.state
                .captures
                .lock()
                .unwrap()
                .push((params, result.clone()));
            result
        } else {
            self.inner
                .call_with_deadline(method, params, cancel, deadline)
        }
    }
}
fn setup(
    fixture: &Fixture,
    mode: Mode,
    input: &Value,
    legacy: bool,
    case: Case,
) -> (ActionService, Arc<State>, u64) {
    let program = mode.program();
    assert!(!program.starts_with(&fixture.root));
    let mut runtimes = RuntimeRegistry::new();
    match mode {
        Mode::Node => runtimes
            .register("node", Arc::new(NodeRuntime::new(program)))
            .unwrap(),
        Mode::Script => runtimes
            .register(
                "script",
                Arc::new(ScriptHostRuntime::new(program).with_timeout(Duration::from_secs(12))),
            )
            .unwrap(),
    }
    let state = Arc::new(State::default());
    let expected = json!({"program":input["program"],"args":input["args"],"cwd":input["cwd"],"timeout_ms":input["timeoutMs"],"allow_non_zero":input["allowNonZero"]});
    let host: Arc<dyn NativeHost> = Arc::new(RecordingHost {
        inner: WithProcessCapture::new(NotifyHost {
            state: state.clone(),
            fail: case == Case::NotifyUnknown,
        }),
        state: state.clone(),
        expected,
    });
    let service = ActionService::new(PackageCatalog::load(&fixture.actions), runtimes);
    let invocation = service
        .submit_with_host(
            77,
            ActionCall {
                package_id: "examples.process-output".into(),
                action_id: if legacy { "notify" } else { mode.action() }.into(),
                action_version: 1,
                input: input.clone(),
            },
            Some(host),
        )
        .unwrap();
    (service, state, invocation)
}
fn terminal(service: &ActionService, invocation: u64, mode: Mode) -> ActionOutcome {
    let began = Instant::now();
    let mut started = 0;
    loop {
        assert!(
            began.elapsed() < mode.wait(),
            "Service终态超时 mode={mode:?} elapsed={:?}",
            began.elapsed()
        );
        match service.try_recv() {
            Ok(event) => {
                assert_eq!(event.invocation_id, invocation);
                match event.outcome {
                    ActionOutcome::Started => started += 1,
                    outcome => {
                        assert_eq!(started, 1);
                        eprintln!(
                            "普通Service终态 mode={mode:?} elapsed={:?}",
                            began.elapsed()
                        );
                        return outcome;
                    }
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => thread::sleep(Duration::from_millis(2)),
            Err(error) => panic!("事件通道：{error}"),
        }
    }
}
fn verify(outcome: ActionOutcome, state: &State, case: Case, legacy: bool) {
    let captures = state.captures.lock().unwrap();
    assert_eq!(captures.len(), usize::from(!legacy));
    let notifications = state.notifications.lock().unwrap();
    if matches!(case, Case::Nonzero | Case::InvalidUtf8 | Case::Timeout) {
        let ActionOutcome::Failed { error } = outcome else {
            panic!("应为明确失败终态")
        };
        let code = match case {
            Case::Nonzero if legacy => "process_failed",
            Case::Nonzero => "process_exit_nonzero",
            Case::InvalidUtf8 => "process_invalid_utf8",
            _ => "process_timeout",
        };
        assert_eq!(error.code, code);
        assert_eq!(
            error.method.as_deref(),
            Some(if legacy {
                "process.run"
            } else {
                "process.capture"
            })
        );
        assert_eq!(error.effect_state.as_deref(), Some("unknown"));
        assert!(notifications.is_empty());
        if !legacy {
            let error = captures[0].1.as_ref().unwrap_err();
            assert_eq!(error.code, code);
            assert_eq!(error.effect_state.as_deref(), Some("unknown"));
        }
        return;
    }
    let (stdout, stderr, code) = if case == Case::Allowed {
        ("controlled output", "", 7)
    } else {
        (STDOUT, STDERR, 0)
    };
    if !legacy {
        assert_eq!(
            captures[0].1.as_ref().unwrap(),
            &json!({"code":code,"stdout":stdout,"stderr":stderr})
        );
    }
    assert_eq!(notifications.len(), 1);
    assert_eq!(
        notifications[0],
        json!({"message":if case==Case::Untrimmed {stdout} else {stdout.trim()},"title":"程序输出","level":if code==0 {"info"} else {"warning"},"placement":"topRight","durationMs":3000})
    );
    if case == Case::NotifyUnknown {
        let ActionOutcome::Failed { error } = outcome else {
            panic!("通知拒绝应失败")
        };
        assert_eq!(error.code, "ui_unavailable");
        assert_eq!(error.method.as_deref(), Some("ui.notify"));
        assert_eq!(error.effect_state.as_deref(), Some("unknown"));
    } else {
        let ActionOutcome::Succeeded { output } = outcome else {
            panic!("应为成功摘要")
        };
        assert_eq!(
            output,
            json!({"completed":true,"exitCode":code,"stdoutUtf16Units":stdout.encode_utf16().count(),"stderrUtf16Units":stderr.encode_utf16().count(),"notified":true})
        );
        assert!(!output.as_object().unwrap().contains_key("stdout"));
    }
}
fn cases(mode: Mode, legacy: bool) {
    let program = explicit("OMNITRIGGER_TEST_PROCESS_OUTPUT_FIXTURE");
    let mut cases = vec![
        Case::Trimmed,
        Case::Untrimmed,
        Case::Nonzero,
        Case::Allowed,
        Case::NotifyUnknown,
    ];
    if !legacy {
        cases.extend([Case::InvalidUtf8, Case::Timeout]);
    }
    for case in cases {
        let fixture = Fixture::new();
        assert!(!program.starts_with(&fixture.root));
        let mut input = fixture.input(&program, case);
        if legacy {
            input.as_object_mut().unwrap().remove("timeoutMs");
        }
        let (service, state, invocation) = setup(&fixture, mode, &input, legacy, case);
        verify(terminal(&service, invocation, mode), &state, case, legacy);
        eprintln!(
            "entry={} case={case:?}真实程序返回后核对完成",
            if legacy { "notify" } else { mode.action() }
        );
        drop(service);
        drop(state);
    }
}

struct ProcessHandle(HANDLE);
impl ProcessHandle {
    fn open(pid: u32) -> Self {
        assert!(pid != 0 && pid != std::process::id());
        Self(
            unsafe {
                OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_ACCESS_RIGHTS(0x0010_0000),
                    false,
                    pid,
                )
            }
            .unwrap(),
        )
    }
    fn exited(&self) -> bool {
        let state = unsafe { WaitForSingleObject(self.0, 0) };
        if state == WAIT_OBJECT_0 {
            true
        } else if state == WAIT_TIMEOUT {
            false
        } else {
            panic!("只读进程观察失败：{state:?}")
        }
    }
}
impl Drop for ProcessHandle {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Pids {
    schema_version: u32,
    token: String,
    fixture: u32,
    descendant: u32,
    started_at_unix_ms: u64,
}
fn ready(fixture: &Fixture) -> [ProcessHandle; 2] {
    let marker = fixture.root.join("fixture-ready");
    let began = Instant::now();
    loop {
        assert!(began.elapsed() < Duration::from_secs(8), "子进程就绪超时");
        if marker.is_file() {
            local(&marker);
            assert!(fs::metadata(&marker).unwrap().len() <= 32);
            let value = fs::read_to_string(&marker).unwrap();
            if value == fixture.token {
                break;
            }
            assert!(fixture.token.starts_with(&value), "就绪标记不属于本次夹具");
        }
        thread::sleep(Duration::from_millis(2));
    }
    let record = fixture.root.join("owned-pids.json");
    local(&record);
    assert!(fs::metadata(&record).unwrap().len() <= 2048);
    let record: Pids = serde_json::from_slice(&fs::read(record).unwrap()).unwrap();
    assert_eq!(record.schema_version, 1);
    assert_eq!(record.token, fixture.token);
    assert_ne!(record.fixture, record.descendant);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    assert!(
        now >= u128::from(record.started_at_unix_ms)
            && now - u128::from(record.started_at_unix_ms) < 5000
    );
    let handles = [
        ProcessHandle::open(record.fixture),
        ProcessHandle::open(record.descendant),
    ];
    assert!(handles.iter().all(|handle| !handle.exited()));
    handles
}

#[test]
#[ignore = "现包真实进程回归；须正常Node/ScriptHost与显式自有fixture"]
fn capture_package_cases() {
    cases(Mode::selected(), false);
}

#[test]
#[ignore = "真实Service用户取消；真实捕获与自有Job子进程清理，不终止外部PID"]
fn capture_package_service_cancel() {
    let mode = Mode::selected();
    let program = explicit("OMNITRIGGER_TEST_PROCESS_OUTPUT_FIXTURE");
    let fixture = Fixture::new();
    assert!(!program.starts_with(&fixture.root));
    let input = fixture.input(&program, Case::Cancel);
    let (service, state, invocation) = setup(&fixture, mode, &input, false, Case::Cancel);
    let handles = ready(&fixture);
    let cancelled = Instant::now();
    assert_eq!(service.cancel_id(77), 1);
    while handles.iter().any(|handle| !handle.exited()) {
        assert!(
            cancelled.elapsed() < Duration::from_secs(5),
            "取消未在自然15秒期限前清理自有子进程"
        );
        thread::sleep(Duration::from_millis(2));
    }
    assert!(matches!(
        terminal(&service, invocation, mode),
        ActionOutcome::Cancelled
    ));
    let captures = state.captures.lock().unwrap();
    assert_eq!(captures.len(), 1);
    let error = captures[0].1.as_ref().unwrap_err();
    assert_eq!(error.code, "cancelled");
    assert_eq!(error.method.as_deref(), Some("process.capture"));
    assert_eq!(error.effect_state.as_deref(), Some("unknown"));
    assert!(state.notifications.lock().unwrap().is_empty());
    drop(captures);
    drop(handles);
    drop(service);
    drop(state);
}

#[test]
#[ignore = "原Node run入口5场景；只在node选择器运行，Script仅选capture_过滤"]
fn node_run_package_cases() {
    assert_eq!(Mode::selected(), Mode::Node);
    cases(Mode::Node, true);
}
