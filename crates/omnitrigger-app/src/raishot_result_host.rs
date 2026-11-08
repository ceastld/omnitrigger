//! 普通 Script/Node 的 RaiShot 文件结果入口；复用原包定位和 owned Job 采集。
use omnitrigger_actions::{ActionError, CancellationToken, NativeHost};
use serde::Deserialize;
#[cfg(windows)]
use serde_json::json;
use serde_json::Value;
use std::{path::PathBuf, sync::Arc, time::Instant};

#[path = "raishot_owned_png.rs"]
mod owned;
#[path = "raishot_result_protocol.rs"]
#[cfg_attr(not(windows), allow(dead_code))]
mod protocol;

const CAPTURE: &str = "raishot.capture_result";
const RELEASE: &str = "raishot.release_result";
fn fault(method: &str, code: &str, effect: &str) -> ActionError {
    let mut error = ActionError::new(code, "截图文件结果未获确认；请检查错误代码，不会自动重试。");
    error.method = Some(method.into());
    error.effect_state = Some(effect.into());
    error
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureRequest {
    timeout_ms: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleaseRequest {
    request_id: String,
}

pub struct Store {
    #[cfg(all(test, windows))]
    test_port: Option<Arc<dyn LaunchPort>>,
    #[cfg_attr(not(windows), allow(dead_code))]
    config: PathBuf,
    #[cfg(windows)]
    sessions: std::sync::Mutex<std::collections::BTreeMap<String, Entry>>,
}
#[cfg(windows)]
struct Entry {
    session: Option<owned::Session>,
    quarantined: bool,
}
impl Store {
    /// 构造不读写文件；由应用显式共享同一实例，引用在本次应用进程内有效。
    pub fn new(config: PathBuf) -> Self {
        Self {
            config,
            #[cfg(all(test, windows))]
            test_port: None,
            #[cfg(windows)]
            sessions: std::sync::Mutex::new(std::collections::BTreeMap::new()),
        }
    }
    fn execute(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
        deadline: Option<Instant>,
    ) -> Result<Value, ActionError> {
        if method == CAPTURE {
            let request: CaptureRequest = serde_json::from_value(params)
                .map_err(|_| fault(method, "invalid_input", "not_started"))?;
            if !(1..=30_000).contains(&request.timeout_ms) {
                return Err(fault(method, "invalid_input", "not_started"));
            }
            #[cfg(windows)]
            {
                self.capture(request.timeout_ms, cancellation, deadline)
            }
            #[cfg(not(windows))]
            {
                let _ = (request, cancellation, deadline);
                Err(fault(method, "unsupported_platform", "not_started"))
            }
        } else {
            let request: ReleaseRequest = serde_json::from_value(params)
                .map_err(|_| fault(method, "invalid_input", "not_started"))?;
            if !protocol::hex(&request.request_id, 32) {
                return Err(fault(method, "invalid_input", "not_started"));
            }
            #[cfg(windows)]
            {
                self.release(&request.request_id, cancellation, deadline)
            }
            #[cfg(not(windows))]
            {
                let _ = (request, cancellation, deadline);
                Err(fault(method, "unsupported_platform", "not_started"))
            }
        }
    }
}

#[cfg(windows)]
fn check(cancellation: &CancellationToken, end: Instant) -> Result<(), &'static str> {
    if cancellation.is_cancelled() {
        Err("cancelled")
    } else if Instant::now() >= end {
        Err("timeout")
    } else {
        Ok(())
    }
}
#[cfg(windows)]
fn milliseconds(end: Instant) -> Result<u64, &'static str> {
    u64::try_from(end.saturating_duration_since(Instant::now()).as_millis())
        .ok()
        .filter(|n| *n > 0)
        .ok_or("timeout")
}

// 仅定位与既有采集调用形成端口；生产仍使用同一定位器和 owned Job，无新执行器。
#[cfg(windows)]
trait LaunchPort: Send + Sync {
    fn locate(&self, config: &std::path::Path) -> Result<PathBuf, &'static str>;
    fn capture(
        &self,
        program: &str,
        args: &[String],
        options: &omnitrigger_windows::ProcessCaptureOptions,
        control: &mut dyn FnMut() -> omnitrigger_windows::ProcessCaptureControl,
    ) -> Result<omnitrigger_windows::ProcessCaptureResult, omnitrigger_windows::ProcessCaptureFailure>;
}
#[cfg(windows)]
struct OriginalLaunchPort;
#[cfg(windows)]
impl LaunchPort for OriginalLaunchPort {
    fn locate(&self, config: &std::path::Path) -> Result<PathBuf, &'static str> {
        crate::plugin_install::capture_result_program(config).map_err(|_| "raishot_unavailable")
    }
    fn capture(
        &self,
        program: &str,
        args: &[String],
        options: &omnitrigger_windows::ProcessCaptureOptions,
        control: &mut dyn FnMut() -> omnitrigger_windows::ProcessCaptureControl,
    ) -> Result<omnitrigger_windows::ProcessCaptureResult, omnitrigger_windows::ProcessCaptureFailure>
    {
        omnitrigger_windows::capture_process_if(program, args, options, control)
    }
}

#[cfg(windows)]
impl Store {
    fn capture(
        &self,
        timeout_ms: u64,
        cancellation: &CancellationToken,
        deadline: Option<Instant>,
    ) -> Result<Value, ActionError> {
        #[cfg(test)]
        if let Some(port) = self.test_port.as_deref() {
            return self.capture_with_port(timeout_ms, cancellation, deadline, port);
        }
        self.capture_with_port(timeout_ms, cancellation, deadline, &OriginalLaunchPort)
    }
    fn capture_with_port(
        &self,
        timeout_ms: u64,
        cancellation: &CancellationToken,
        deadline: Option<Instant>,
        port: &dyn LaunchPort,
    ) -> Result<Value, ActionError> {
        use omnitrigger_windows::{
            ProcessCaptureControl as Control, ProcessCaptureOptions as Options,
        };
        use std::{
            sync::atomic::{AtomicU32, Ordering},
            time::{Duration, SystemTime, UNIX_EPOCH},
        };
        let own_end = Instant::now() + Duration::from_millis(timeout_ms);
        let end = deadline.map_or(own_end, |outer| outer.min(own_end));
        let before = |code| fault(CAPTURE, code, "not_started");
        check(cancellation, end).map_err(before)?;
        let cli = port.locate(&self.config).map_err(before)?;
        // 原程序钉住方式和真实 PNG 验证器均不通过测试端口替换。
        let _program_guard = owned::pin_program(&cli).map_err(before)?;
        let program = cli.to_str().ok_or_else(|| before("raishot_unavailable"))?;
        let cwd = cli
            .parent()
            .and_then(|p| p.to_str())
            .ok_or_else(|| before("raishot_unavailable"))?;
        let mut control = || match check(cancellation, end) {
            Ok(()) => Control::Continue,
            Err("cancelled") => Control::Cancelled,
            Err(_) => Control::Deadline,
        };
        let probe = port.capture(
            program,
            &["--capture-result-capabilities".into()],
            &Options {
                timeout_ms: milliseconds(end).map_err(before)?.min(3000),
                allow_non_zero: true,
                cwd: Some(cwd.into()),
            },
            &mut control,
        );
        check(cancellation, end).map_err(before)?;
        let probe = probe.map_err(|_| before("capture_result_unsupported"))?;
        protocol::capabilities(probe.code, &probe.stdout).map_err(before)?;
        // 没有新能力就止步于此，不尝试 --capture-copy-json、resident.region 或第二执行器。
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let sequence = NEXT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| before("capture_session_limit"))?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| before("process_clock_error"))?;
        let millis = u64::try_from(now.as_millis()).map_err(|_| before("process_clock_error"))?;
        let id = format!("{millis:016x}{:08x}{sequence:08x}", std::process::id());
        let root = self
            .config
            .parent()
            .ok_or_else(|| before("capture_owned_file_unavailable"))?
            .join("raishot-results");
        {
            let mut entries = self
                .sessions
                .lock()
                .map_err(|_| before("capture_store_unavailable"))?;
            if entries.len() >= 16 {
                return Err(before("capture_session_limit"));
            }
            if entries
                .values()
                .any(|entry| entry.session.is_none() || entry.quarantined)
            {
                return Err(before("capture_result_busy"));
            }
            entries.insert(
                id.clone(),
                Entry {
                    session: None,
                    quarantined: false,
                },
            );
        }
        let mut session = match owned::Session::create(&root, &id) {
            Ok(session) => session,
            Err(code) => {
                if let Ok(mut entries) = self.sessions.lock() {
                    entries.remove(&id);
                }
                return Err(before(code));
            }
        };
        let mut quarantined = false;
        let mut capture_submitted = false;
        let outcome = (|| -> Result<Value, &'static str> {
            check(cancellation, end)?;
            let left = milliseconds(end)?;
            let unix_deadline = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()
                .and_then(|now| u64::try_from(now.as_millis()).ok())
                .and_then(|now| now.checked_add(left))
                .ok_or("process_clock_error")?;
            let directory = session
                .directory
                .to_str()
                .ok_or("capture_owned_file_unavailable")?;
            let args = vec![
                "--capture-result-json".into(),
                "--request-id".into(),
                id.clone(),
                "--session-dir".into(),
                directory.into(),
                "--deadline-ms".into(),
                unix_deadline.to_string(),
            ];
            capture_submitted = true;
            let captured = match port.capture(
                program,
                &args,
                &Options {
                    timeout_ms: left,
                    allow_non_zero: true,
                    cwd: Some(cwd.into()),
                },
                &mut control,
            ) {
                Ok(value) => value,
                Err(failure) => {
                    quarantined = failure.code == "process_cleanup_failed";
                    capture_submitted = failure.effect_state != "not_started";
                    return Err(failure.code);
                }
            };
            check(cancellation, end)?;
            let receipt = protocol::terminal(&id, captured.code, &captured.stdout)?;
            if receipt.status != "completed" {
                return Ok(result(
                    &id,
                    &receipt.status,
                    receipt.error_code.as_deref(),
                    &receipt.image_effect_state,
                    None,
                    None,
                ));
            }
            let image = receipt.image.as_ref().ok_or("capture_result_unconfirmed")?;
            session.verify(image, &mut || check(cancellation, end))?;
            check(cancellation, end)?;
            Ok(result(
                &id,
                "completed",
                None,
                "confirmed",
                Some(json!({
                    "path":session.path(),"fileName":"image.png","format":"png","width":image.width,"height":image.height,
                    "byteLength":image.byte_length,"sha256":image.sha256
                })),
                receipt.warning_code.as_deref(),
            ))
        })();
        let value = match outcome {
            Ok(value) => value,
            Err(code) => result(
                &id,
                if code == "cancelled" {
                    "cancelled"
                } else {
                    "failed"
                },
                Some(code),
                if capture_submitted {
                    "unknown"
                } else {
                    "not_started"
                },
                None,
                None,
            ),
        };
        // 报错仍返回请求 ID，可显式释放；无法确认 Job 清理时隔离，不能删除潜在活动产物。
        let mut entries = self
            .sessions
            .lock()
            .map_err(|_| fault(CAPTURE, "capture_store_unavailable", "unknown"))?;
        entries.insert(
            id,
            Entry {
                session: Some(session),
                quarantined,
            },
        );
        // 发布注册表可能等待另一次显式释放；返回前再次核对同一单调期限。
        if let Err(code) = check(cancellation, end) {
            let request_id = value["requestId"]
                .as_str()
                .ok_or_else(|| fault(CAPTURE, "capture_store_unavailable", "unknown"))?;
            return Ok(result(
                request_id,
                if code == "cancelled" {
                    "cancelled"
                } else {
                    "failed"
                },
                Some(code),
                if capture_submitted {
                    "unknown"
                } else {
                    "not_started"
                },
                None,
                None,
            ));
        }
        Ok(value)
    }
    fn release(
        &self,
        id: &str,
        cancellation: &CancellationToken,
        deadline: Option<Instant>,
    ) -> Result<Value, ActionError> {
        let end =
            deadline.unwrap_or_else(|| Instant::now() + std::time::Duration::from_millis(2000));
        check(cancellation, end).map_err(|code| fault(RELEASE, code, "not_started"))?;
        let mut entries = self
            .sessions
            .lock()
            .map_err(|_| fault(RELEASE, "capture_store_unavailable", "not_started"))?;
        check(cancellation, end).map_err(|code| fault(RELEASE, code, "not_started"))?;
        let entry = entries
            .get_mut(id)
            .ok_or_else(|| fault(RELEASE, "capture_result_missing", "not_started"))?;
        if entry.quarantined || entry.session.is_none() {
            return Err(fault(RELEASE, "capture_result_busy", "not_started"));
        }
        entry
            .session
            .as_mut()
            .ok_or_else(|| fault(RELEASE, "capture_result_busy", "not_started"))?
            .release_image(&mut || check(cancellation, end))
            .map_err(|code| {
                fault(
                    RELEASE,
                    code,
                    if matches!(code, "cancelled" | "timeout") {
                        "not_started"
                    } else {
                        "unknown"
                    },
                )
            })?;
        entries.remove(id);
        Ok(
            json!({"released":true,"requestId":id,"scope":"owned_png_file","sessionDirectoryRetained":true}),
        )
    }
}
#[cfg(windows)]
fn result(
    id: &str,
    status: &str,
    error: Option<&str>,
    effect: &str,
    image: Option<Value>,
    warning: Option<&str>,
) -> Value {
    json!({"status":status,"requestId":id,"completionScope":"png_file_only","image":image,
        "errorCode":error,"imageEffectState":effect,"artifactVerified":status=="completed",
        "automaticClipboardWritten":false,"automaticHistoryAdded":false,"sourceBusinessComplete":false,
        "sourceAutoAdsorbVerified":false,"retention":"caller_owned_until_explicit_release",
        "warningCode":warning,"retryAllowed":false})
}
pub struct WithRaiShotResult<H> {
    inner: H,
    store: Arc<Store>,
}
impl<H> WithRaiShotResult<H> {
    pub fn new(inner: H, store: Arc<Store>) -> Self {
        Self { inner, store }
    }
}
impl<H: NativeHost> NativeHost for WithRaiShotResult<H> {
    fn native_context_available(&self) -> bool {
        self.inner.native_context_available()
    }

    fn call(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
    ) -> Result<Value, ActionError> {
        if matches!(method, CAPTURE | RELEASE) {
            self.store.execute(method, params, cancellation, None)
        } else {
            self.inner.call(method, params, cancellation)
        }
    }
    fn call_with_deadline(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<Value, ActionError> {
        if matches!(method, CAPTURE | RELEASE) {
            self.store
                .execute(method, params, cancellation, Some(deadline))
        } else {
            self.inner
                .call_with_deadline(method, params, cancellation, deadline)
        }
    }
}

#[cfg(all(test, windows))]
#[path = "raishot_result_service_tests.rs"]
mod service_tests;
