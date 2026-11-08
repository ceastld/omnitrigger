//! 仅显式ignored原生验收。父测试进程自然存活，不修改CLI或生产生命周期。
use super::WithProcessCapture;
use omnitrigger_actions::{
    ActionCall, ActionDefinition, ActionError, ActionRuntime, CancellationToken, InvocationContext,
    NativeHost, NodeRuntime, ResolvedAction,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows::Win32::{
    Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::Threading::{
        OpenProcess, WaitForSingleObject, PROCESS_ACCESS_RIGHTS, PROCESS_QUERY_LIMITED_INFORMATION,
    },
};
static NEXT: AtomicU64 = AtomicU64::new(1);
const SIGNATURE: &str = "omnitrigger-process-capture-fixture-v1";
const WARM: &str = r#"
export default async ctx => {
  const value = await ctx.process.capture(ctx.input.program, ['warmup'], {timeoutMs:10000});
  return {ready:value.code===0&&value.stdout===ctx.input.signature&&value.stderr===''};
};
"#;
const SELF_EXIT: &str = r#"
import {existsSync,readFileSync,writeFileSync,renameSync} from 'node:fs';
import {join} from 'node:path';
const pause=ms=>new Promise(done=>setTimeout(done,ms));
function publish(root,name,text) {
  writeFileSync(join(root,name+'.writing'),text,{flag:'wx'});
  renameSync(join(root,name+'.writing'),join(root,name));
}
export default async ctx => {
  if(process.ppid!==ctx.input.parent)throw Error('owned_parent_mismatch');
  const pending=ctx.process.capture(ctx.input.program,['descendant',ctx.input.root,ctx.input.token],{timeoutMs:10000});
  pending.catch(()=>{});
  const until=performance.now()+8000;
  const ready=join(ctx.input.root,'fixture-ready');
  while(!existsSync(ready)||readFileSync(ready,'utf8')!==ctx.input.token) {
    if(performance.now()>=until)throw Error('fixture_ready_timeout');
    await pause(2);
  }
  publish(ctx.input.root,'node-owner.json',JSON.stringify({schemaVersion:1,token:ctx.input.token,node:process.pid,app:process.ppid}));
  publish(ctx.input.root,'node-armed',ctx.input.token);
  const release=join(ctx.input.root,'release-control');
  while(!existsSync(release)||readFileSync(release,'utf8')!==ctx.input.token) {
    if(performance.now()>=until)throw Error('control_release_timeout');
    await pause(2);
  }
  publish(ctx.input.root,'node-control-request',ctx.input.token);
  process.kill(process.pid,'SIGKILL'); // 唯一终止目标是作者自己，不按记录PID终止。
  throw Error('self_exit_returned');
};
"#;

fn ordinary(path: &Path, file: bool) -> Result<(), &'static str> {
    crate::script_runtime::check_local_path(path).map_err(|_| "metadata_rejected")?;
    let info = fs::symlink_metadata(path).map_err(|_| "metadata_unavailable")?;
    if info.file_type().is_symlink() || (file && !info.is_file()) || (!file && !info.is_dir()) {
        return Err("metadata_kind");
    }
    Ok(())
}
fn explicit(name: &str, file: bool) -> PathBuf {
    let path = PathBuf::from(std::env::var_os(name).expect("必须显式提供本机自有路径"));
    assert!(path.is_absolute(), "需要明确绝对路径");
    ordinary(&path, file).expect("路径必须是非云盘普通本地文件或目录");
    if file {
        assert!(
            path.extension()
                .and_then(|v| v.to_str())
                .is_some_and(|v| v.eq_ignore_ascii_case("exe")),
            "需要自有EXE"
        );
    }
    path
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), &'static str> {
    crate::script_runtime::check_local_path(path).map_err(|_| "metadata_rejected")?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| "create_owned_file")?;
    file.write_all(bytes).map_err(|_| "write_owned_file")?;
    file.sync_all().map_err(|_| "sync_owned_file")
}
fn read_owned(path: &Path, limit: u64) -> Result<String, &'static str> {
    ordinary(path, true)?;
    let file = File::open(path).map_err(|_| "read_owned_file")?;
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "read_owned_file")?;
    if bytes.len() as u64 > limit {
        return Err("owned_file_limit");
    }
    String::from_utf8(bytes).map_err(|_| "owned_file_utf8")
}
fn action(root: &Path, source: &str, input: Value) -> ResolvedAction {
    let entry = root.join(format!(
        "action-{}.mjs",
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    write_new(&entry, source.as_bytes()).expect("仅创建本轮动作源码");
    ResolvedAction {
        call: ActionCall {
            package_id: "validation.node-native-capture".into(),
            action_id: "run".into(),
            action_version: 1,
            input,
        },
        definition: ActionDefinition {
            id: "run".into(),
            title: "Node原生采集验收".into(),
            version: 1,
            runtime: "node".into(),
            entry: entry.file_name().unwrap().to_string_lossy().into_owned(),
            single_instance: false,
            allow_binding_reentry: false,
        },
        package_root: root.to_path_buf(),
        entry_path: entry,
    }
}
struct NoOtherMethods;
impl NativeHost for NoOtherMethods {
    fn call(&self, _: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
        Err(ActionError::new(
            "unexpected_native_method",
            "该原生验收只接受process.capture。",
        ))
    }
}
struct ReadOnlyProcess(HANDLE);
impl ReadOnlyProcess {
    fn open(pid: u32) -> Result<Self, &'static str> {
        // 只有查询与同步权限；不申请PROCESS_TERMINATE，也不按PID终止进程。
        unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_ACCESS_RIGHTS(0x0010_0000),
                false,
                pid,
            )
        }
        .map(Self)
        .map_err(|_| "readonly_process_open")
    }
    fn exited(&self) -> Result<bool, &'static str> {
        let state = unsafe { WaitForSingleObject(self.0, 0) };
        if state == WAIT_OBJECT_0 {
            Ok(true)
        } else if state == WAIT_TIMEOUT {
            Ok(false)
        } else {
            Err("readonly_wait_unknown")
        }
    }
}
impl Drop for ReadOnlyProcess {
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
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NodeOwner {
    schema_version: u32,
    token: String,
    node: u32,
    app: u32,
}
fn observe(root: &Path, token: &str, parent: u32) -> Result<Value, &'static str> {
    let began = Instant::now();
    while !root.join("node-armed").is_file() {
        if began.elapsed() >= Duration::from_secs(8) {
            return Err("node_ready_timeout");
        }
        thread::sleep(Duration::from_millis(2));
    }
    if read_owned(&root.join("node-armed"), 128)? != token
        || read_owned(&root.join("fixture-ready"), 128)? != token
    {
        return Err("owned_ready_token");
    }
    let pids: Pids = serde_json::from_str(&read_owned(&root.join("owned-pids.json"), 2048)?)
        .map_err(|_| "owned_pid_record")?;
    let node: NodeOwner = serde_json::from_str(&read_owned(&root.join("node-owner.json"), 2048)?)
        .map_err(|_| "owned_node_record")?;
    if pids.schema_version != 1
        || node.schema_version != 1
        || pids.token != token
        || node.token != token
        || node.app != parent
    {
        return Err("owned_record_identity");
    }
    let ids = [parent, node.node, pids.fixture, pids.descendant];
    if ids.contains(&0)
        || ids
            .iter()
            .enumerate()
            .any(|(index, id)| ids[..index].contains(id))
    {
        return Err("owned_pid_collision");
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "wall_clock")?
        .as_millis();
    if now < u128::from(pids.started_at_unix_ms)
        || now - u128::from(pids.started_at_unix_ms) >= 5000
    {
        return Err("fixture_age_unproven");
    }
    // 全部句柄在Node强杀之前获取，观察中不再重新按PID打开，避免PID复用混淆。
    let handles = [
        ReadOnlyProcess::open(node.node)?,
        ReadOnlyProcess::open(pids.fixture)?,
        ReadOnlyProcess::open(pids.descendant)?,
    ];
    for handle in &handles {
        if handle.exited()? {
            return Err("owned_process_not_alive");
        }
    }
    let temporary = root.join("release-control.writing");
    write_new(&temporary, token.as_bytes())?;
    if root.join("release-control").exists() {
        return Err("release_exists");
    }
    fs::rename(temporary, root.join("release-control")).map_err(|_| "publish_release")?;
    let released = Instant::now();
    loop {
        let states = handles
            .iter()
            .map(ReadOnlyProcess::exited)
            .collect::<Result<Vec<_>, _>>()?;
        if states.iter().all(|value| *value) {
            if std::process::id() != parent {
                return Err("parent_identity_changed");
            }
            if read_owned(&root.join("node-control-request"), 128)? != token {
                return Err("self_exit_request_unproven");
            }
            return Ok(
                json!({"nodeExited":true,"fixtureExited":true,"descendantExited":true,
                "parentTestProcessStillAlive":true,"handlesOpenedBeforeRelease":true,
                "observedMs":released.elapsed().as_millis(),"naturalExpiryUsed":false,"bodyRecorded":false}),
            );
        }
        if released.elapsed() >= Duration::from_secs(3) {
            return Err("cooperative_cleanup_not_observed");
        }
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
#[ignore = "需要显式自有Node、夹具和父目录；真实原生验收须显式运行"]
fn node_self_exit_cleans_capture_job_while_parent_test_process_lives() {
    let node = explicit("OMNITRIGGER_APP_CAPTURE_NODE", true);
    let fixture = explicit("OMNITRIGGER_CAPTURE_FIXTURE", true);
    let parent_root = explicit("OMNITRIGGER_APP_CAPTURE_CASE_ROOT", false);
    let parent = std::process::id();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        ^ (u128::from(parent) << 64)
        ^ u128::from(NEXT.fetch_add(1, Ordering::Relaxed));
    let token = format!("{nonce:032x}");
    let root = parent_root.join(format!("capture-case-app-node-native-{token}"));
    fs::create_dir(&root).expect("只创建全新本轮目录，不覆盖或删除旧目录");
    ordinary(&root, false).unwrap();
    let host = WithProcessCapture::new(NoOtherMethods);
    let context = InvocationContext {
        invocation_id: 1,
        binding_id: 1,
    };
    let warm = action(
        &root,
        WARM,
        json!({"program":fixture,"signature":SIGNATURE}),
    );
    let warm_result = NodeRuntime::new(&node).execute_with_host(
        &warm,
        &context,
        &CancellationToken::new(),
        Some(&host),
    );
    assert!(
        warm_result.is_ok_and(|value| value["ready"] == true),
        "冷启动预热或夹具签名失败；不回显正文"
    );
    let target = action(
        &root,
        SELF_EXIT,
        json!({"program":fixture,"root":root,"token":token,"parent":parent}),
    );
    let (outcome, observation) = thread::scope(|scope| {
        let observer = scope.spawn(|| observe(&root, &token, parent));
        let outcome = NodeRuntime::new(&node).execute_with_host(
            &target,
            &context,
            &CancellationToken::new(),
            Some(&host),
        );
        (outcome, observer.join().expect("只读观察线程必须正常返回"))
    });
    assert!(
        outcome
            .as_ref()
            .is_err_and(|error| error.code == "runtime_exit"),
        "Node必须异常退出，不能以别的错误或成功替代"
    );
    let observation = observation.expect("未在自然期限前证明协作清理；不重试");
    assert_eq!(std::process::id(), parent);
    let receipt = json!({"passed":true,"runtime":"node","terminalCode":"runtime_exit",
        "parentKind":"retained_app_test_process","observation":observation,
        "nodeExitUsesCooperativeCancellation":true,"equivalentToLightweightOsJobClose":false,"attempts":1});
    write_new(
        &root.join("native-node-capture-receipt.json"),
        &serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
}

const USER_CANCEL: &str = r#"
import {existsSync,readFileSync,writeFileSync,renameSync} from 'node:fs';
import {join} from 'node:path';
const pause=ms=>new Promise(done=>setTimeout(done,ms));
function publish(root,name,text) {
  writeFileSync(join(root,name+'.writing'),text,{flag:'wx'});
  renameSync(join(root,name+'.writing'),join(root,name));
}
export default async ctx => {
  if(process.ppid!==ctx.input.parent)throw Error('owned_parent_mismatch');
  const pending=ctx.process.capture(ctx.input.program,['descendant',ctx.input.root,ctx.input.token],{timeoutMs:10000});
  pending.catch(()=>{});
  const until=performance.now()+8000;
  const ready=join(ctx.input.root,'fixture-ready');
  while(!existsSync(ready)||readFileSync(ready,'utf8')!==ctx.input.token) {
    if(performance.now()>=until)throw Error('fixture_ready_timeout');
    await pause(2);
  }
  publish(ctx.input.root,'node-owner.json',JSON.stringify({schemaVersion:1,token:ctx.input.token,node:process.pid,app:process.ppid}));
  publish(ctx.input.root,'node-armed',ctx.input.token);
  const release=join(ctx.input.root,'release-control');
  while(!existsSync(release)||readFileSync(release,'utf8')!==ctx.input.token) {
    if(performance.now()>=until)throw Error('control_release_timeout');
    await pause(2);
  }
  publish(ctx.input.root,'node-control-request',ctx.input.token);
  return await pending;
};
"#;

// ActionService在令牌被取消后会覆盖终态；另证实实际capture返回取消，避免仅凭终态通过。
struct CancelledCaptureHost {
    inner: WithProcessCapture<NoOtherMethods>,
    observed: std::sync::Mutex<(usize, bool)>,
}
impl CancelledCaptureHost {
    fn remember(
        &self,
        result: Result<Value, ActionError>,
        cancellation: &CancellationToken,
    ) -> Result<Value, ActionError> {
        let valid = cancellation.is_cancelled()
            && result.as_ref().is_err_and(|error| {
                error.code == "cancelled"
                    && error.effect_state.as_deref() == Some("unknown")
                    && error.method.as_deref() == Some("process.capture")
                    && error.required_permissions.is_empty()
            });
        let mut observed = self.observed.lock().unwrap();
        observed.0 += 1;
        observed.1 &= valid;
        result
    }
}
impl NativeHost for CancelledCaptureHost {
    fn call(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
    ) -> Result<Value, ActionError> {
        self.remember(self.inner.call(method, params, cancellation), cancellation)
    }
    fn call_with_deadline(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<Value, ActionError> {
        self.remember(
            self.inner
                .call_with_deadline(method, params, cancellation, deadline),
            cancellation,
        )
    }
}

fn wait_user_cancel_request(root: &Path, token: &str) -> Result<(), &'static str> {
    let began = Instant::now();
    let path = root.join("node-control-request");
    while !path.exists() {
        if began.elapsed() >= Duration::from_secs(8) {
            return Err("user_cancel_ready_timeout");
        }
        thread::sleep(Duration::from_millis(2));
    }
    if read_owned(&path, 128)? != token {
        return Err("user_cancel_ready_token");
    }
    Ok(())
}

fn cancelled_terminal(
    service: &omnitrigger_actions::ActionService,
    invocation: u64,
) -> Result<(), &'static str> {
    use omnitrigger_actions::ActionOutcome;
    let began = Instant::now();
    let mut started = false;
    loop {
        match service.try_recv() {
            Ok(event) => {
                if event.id != 1 || event.invocation_id != invocation {
                    return Err("service_event_identity");
                }
                match event.outcome {
                    ActionOutcome::Started if !started => started = true,
                    ActionOutcome::Cancelled if started => return Ok(()),
                    _ => return Err("service_event_not_cancelled"),
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                return Err("service_events_disconnected");
            }
        }
        if began.elapsed() >= Duration::from_secs(4) {
            return Err("service_cancel_terminal_timeout");
        }
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
#[ignore = "需要显式自有Node、夹具和父目录；真实ActionService取消验收须显式运行"]
fn action_service_cancel_cleans_capture_job_while_parent_test_process_lives() {
    use omnitrigger_actions::{ActionService, PackageCatalog, RuntimeRegistry};
    use std::sync::Arc;
    let node = explicit("OMNITRIGGER_APP_CAPTURE_NODE", true);
    let fixture = explicit("OMNITRIGGER_CAPTURE_FIXTURE", true);
    let parent_root = explicit("OMNITRIGGER_APP_CAPTURE_CASE_ROOT", false);
    let parent = std::process::id();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        ^ (u128::from(parent) << 64)
        ^ u128::from(NEXT.fetch_add(1, Ordering::Relaxed));
    let token = format!("{nonce:032x}");
    let root = parent_root.join(format!("capture-case-app-service-cancel-{token}"));
    fs::create_dir(&root).expect("只创建全新本轮目录，不覆盖或删除旧目录");
    ordinary(&root, false).unwrap();
    let warm = action(
        &root,
        WARM,
        json!({"program":fixture,"signature":SIGNATURE}),
    );
    let warm_result = NodeRuntime::new(&node).execute_with_host(
        &warm,
        &InvocationContext {
            invocation_id: 1,
            binding_id: 1,
        },
        &CancellationToken::new(),
        Some(&WithProcessCapture::new(NoOtherMethods)),
    );
    assert!(
        warm_result.is_ok_and(|value| value["ready"] == true),
        "冷启动预热或夹具签名失败；不回显正文"
    );
    let catalog_root = root.join("catalog");
    fs::create_dir(&catalog_root).unwrap();
    let package = catalog_root.join("owned");
    fs::create_dir(&package).unwrap();
    write_new(&package.join("main.mjs"), USER_CANCEL.as_bytes()).unwrap();
    let manifest = json!({"schema_version":1,"id":"validation.node-service-cancel",
        "name":"Node动作服务取消验收","version":"1.0.0","actions":[{
        "id":"run","title":"取消采集","version":1,"runtime":"node","entry":"main.mjs"}]});
    write_new(
        &package.join("action.json"),
        &serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let catalog = PackageCatalog::load(&catalog_root);
    assert!(catalog.diagnostics.is_empty(), "合成目录加载失败");
    assert_eq!(catalog.actions.len(), 1);
    let mut runtimes = RuntimeRegistry::new();
    runtimes
        .register("node", Arc::new(NodeRuntime::new(&node)))
        .unwrap();
    let service = ActionService::new(catalog, runtimes);
    let host = Arc::new(CancelledCaptureHost {
        inner: WithProcessCapture::new(NoOtherMethods),
        observed: std::sync::Mutex::new((0, true)),
    });
    let call = ActionCall {
        package_id: "validation.node-service-cancel".into(),
        action_id: "run".into(),
        action_version: 1,
        input: json!({"program":fixture,"root":root,"token":token,"parent":parent}),
    };
    let (ready, count, terminal, observation) = thread::scope(|scope| {
        let observer = scope.spawn(|| observe(&root, &token, parent));
        let invocation = service
            .submit_with_host(1, call, Some(host.clone()))
            .expect("合成动作必须提交成功");
        let ready = wait_user_cancel_request(&root, &token);
        // 就绪失败也取消本次提交；所有断言留在观察线程有界返回之后。
        let count = service.cancel_id(1);
        let terminal = cancelled_terminal(&service, invocation);
        (
            ready,
            count,
            terminal,
            observer.join().expect("只读观察线程必须正常返回"),
        )
    });
    ready.expect("必须在fixture存活且句柄就绪后发出用户取消");
    assert_eq!(count, 1, "必须命中本轮唯一动作");
    terminal.expect("必须收到本轮Started后Cancelled终态");
    assert_eq!(*host.observed.lock().unwrap(), (1, true));
    assert!(!service.is_running(1), "读取终态后应释放本轮占用");
    let observation = observation.expect("未在自然期限前证明协作清理；不重试");
    assert_eq!(std::process::id(), parent);
    let receipt = json!({"passed":true,"runtime":"node","terminalStatus":"cancelled",
        "cancelSource":"ActionService.cancel_id","cancelledInvocations":count,
        "captureReturnedCancelledUnknown":true,"captureCalls":1,
        "parentKind":"retained_app_test_process","observation":observation,
        "physicalUiClickExercised":false,"authorInjectsCancelFrame":false,
        "nodeSelfExitRequested":false,"attempts":1});
    write_new(
        &root.join("native-service-cancel-capture-receipt.json"),
        &serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
}

const OUTER_DEADLINE: &str = r#"
import {existsSync,readFileSync,writeFileSync,renameSync} from 'node:fs';
import {join} from 'node:path';
const pause=ms=>new Promise(done=>setTimeout(done,ms));
function publish(root,name,text) {
  writeFileSync(join(root,name+'.writing'),text,{flag:'wx'});
  renameSync(join(root,name+'.writing'),join(root,name));
}
export default async ctx => {
  if(process.ppid!==ctx.input.parent)throw Error('owned_parent_mismatch');
  await ctx.delay(58000); // 沿用默认60秒；不修改运行时限或注入取消帧。
  const pending=ctx.process.capture(ctx.input.program,['descendant',ctx.input.root,ctx.input.token],{timeoutMs:10000});
  pending.catch(()=>{});
  const until=performance.now()+8000;
  const ready=join(ctx.input.root,'fixture-ready');
  while(!existsSync(ready)||readFileSync(ready,'utf8')!==ctx.input.token) {
    if(performance.now()>=until)throw Error('fixture_ready_timeout');
    await pause(2);
  }
  publish(ctx.input.root,'node-owner.json',JSON.stringify({schemaVersion:1,token:ctx.input.token,node:process.pid,app:process.ppid}));
  publish(ctx.input.root,'node-armed',ctx.input.token);
  const release=join(ctx.input.root,'release-control');
  while(!existsSync(release)||readFileSync(release,'utf8')!==ctx.input.token) {
    if(performance.now()>=until)throw Error('control_release_timeout');
    await pause(2);
  }
  publish(ctx.input.root,'node-control-request',ctx.input.token);
  return await pending;
};
"#;

// 仅测试观察：原参数、实际外层deadline、令牌与结果照常转交，不缩短期限。
struct DeadlineCaptureHost {
    inner: WithProcessCapture<NoOtherMethods>,
    observed: std::sync::Mutex<Vec<Value>>,
}
impl NativeHost for DeadlineCaptureHost {
    fn call(&self, _: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
        Err(ActionError::new(
            "missing_outer_deadline",
            "本验收必须收到运行时传入的真实外层deadline。",
        ))
    }
    fn call_with_deadline(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<Value, ActionError> {
        let began = Instant::now();
        let remaining = deadline.saturating_duration_since(began);
        let request_matches = method == "process.capture"
            && params["timeout_ms"] == 10000
            && params["args"][0] == "descendant";
        let uncancelled_at_entry = !cancellation.is_cancelled();
        let result = self
            .inner
            .call_with_deadline(method, params, cancellation, deadline);
        let valid_error = result.as_ref().is_err_and(|error| {
            error.code == "timeout"
                && error.effect_state.as_deref() == Some("unknown")
                && error.method.as_deref() == Some("process.capture")
                && error.required_permissions.is_empty()
        });
        let observation = json!({
            "requestMatches":request_matches,
            "remainingMs":remaining.as_millis(),
            "positiveOuterBudget":remaining >= Duration::from_millis(250),
            "outerBudgetBelowThreeSeconds":remaining <= Duration::from_secs(3),
            "deadlineReached":Instant::now() >= deadline,
            "apiTenSecondsNotElapsed":began.elapsed() < Duration::from_secs(10),
            "uncancelledAtEntry":uncancelled_at_entry,
            "uncancelledAtReturn":!cancellation.is_cancelled(),
            "captureReturnedTimeoutUnknown":valid_error,
            "bodyRecorded":false,
        });
        self.observed.lock().unwrap().push(observation);
        result
    }
}

fn observe_after_slow_node_ready(
    root: &Path,
    token: &str,
    parent: u32,
) -> Result<Value, &'static str> {
    let began = Instant::now();
    // 只延长测试观察器等作者开始采集的时间，不更改任何生产期限。
    while !root.join("node-armed").is_file() {
        if began.elapsed() >= Duration::from_secs(62) {
            return Err("outer_deadline_node_ready_unproven");
        }
        thread::sleep(Duration::from_millis(5));
    }
    observe(root, token, parent)
}

#[test]
#[ignore = "慢速约60–63秒；显式自有Node和夹具；默认60秒真实外层deadline"]
fn node_outer_deadline_cleans_capture_job_while_parent_test_process_lives() {
    let node = explicit("OMNITRIGGER_APP_CAPTURE_NODE", true);
    let fixture = explicit("OMNITRIGGER_CAPTURE_FIXTURE", true);
    let parent_root = explicit("OMNITRIGGER_APP_CAPTURE_CASE_ROOT", false);
    let parent = std::process::id();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        ^ (u128::from(parent) << 64)
        ^ u128::from(NEXT.fetch_add(1, Ordering::Relaxed));
    let token = format!("{nonce:032x}");
    let root = parent_root.join(format!("capture-case-app-node-deadline-{token}"));
    fs::create_dir(&root).expect("只创建全新本轮目录，不覆盖或删除旧目录");
    ordinary(&root, false).unwrap();
    let context = InvocationContext {
        invocation_id: 1,
        binding_id: 1,
    };
    let warm = action(
        &root,
        WARM,
        json!({"program":fixture,"signature":SIGNATURE}),
    );
    let warm_result = NodeRuntime::new(&node).execute_with_host(
        &warm,
        &context,
        &CancellationToken::new(),
        Some(&WithProcessCapture::new(NoOtherMethods)),
    );
    assert!(
        warm_result.is_ok_and(|value| value["ready"] == true),
        "冷启动预热或夹具签名失败；不回显正文"
    );
    let host = DeadlineCaptureHost {
        inner: WithProcessCapture::new(NoOtherMethods),
        observed: std::sync::Mutex::new(Vec::new()),
    };
    let target = action(
        &root,
        OUTER_DEADLINE,
        json!({"program":fixture,"root":root,"token":token,"parent":parent}),
    );
    let cancellation = CancellationToken::new();
    let began = Instant::now();
    let (outcome, observation) = thread::scope(|scope| {
        let observer = scope.spawn(|| observe_after_slow_node_ready(&root, &token, parent));
        let outcome = NodeRuntime::new(&node).execute_with_host(
            &target,
            &context,
            &cancellation,
            Some(&host),
        );
        (outcome, observer.join().expect("只读观察线程必须正常返回"))
    });
    assert!(
        outcome.as_ref().is_err_and(|error| {
            error.code == "timeout" && error.effect_state.as_deref() == Some("unknown")
        }),
        "必须收到默认外层timeout/unknown，不能以取消或单次采集成功替代"
    );
    assert!(!cancellation.is_cancelled(), "测试不得取消父令牌");
    assert!(began.elapsed() >= Duration::from_secs(60));
    assert!(began.elapsed() < Duration::from_secs(65));
    let records = host.observed.lock().unwrap();
    assert_eq!(records.len(), 1, "必须只有一次真实采集");
    let deadline_observation = records[0].clone();
    for key in [
        "requestMatches",
        "positiveOuterBudget",
        "outerBudgetBelowThreeSeconds",
        "deadlineReached",
        "apiTenSecondsNotElapsed",
        "uncancelledAtEntry",
        "uncancelledAtReturn",
        "captureReturnedTimeoutUnknown",
    ] {
        assert_eq!(deadline_observation[key], true, "外层deadline证据不足");
    }
    let observation = observation.expect("未在自然期限前证明协作清理；不重试");
    assert_eq!(std::process::id(), parent);
    let receipt = json!({"passed":true,"runtime":"node","terminalCode":"timeout",
        "outerRuntimeDefaultSeconds":60,"captureTimeoutMs":10000,
        "deadlineObservation":deadline_observation,"observation":observation,
        "elapsedMs":began.elapsed().as_millis(),"parentKind":"retained_app_test_process",
        "parentTokenCancelled":false,"authorInjectsCancelFrame":false,
        "nodeSelfExitRequested":false,"attempts":1});
    write_new(
        &root.join("native-node-deadline-capture-receipt.json"),
        &serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
}
