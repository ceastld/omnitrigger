//! 同步脚本入口到既有异步浏览器客户端的适配，连接只在动作工作线程按需创建。
use omnitrigger_actions::{ActionError, CancellationToken, NativeHost};
use omnitrigger_browser::client::BrowserClient;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    cell::Cell,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};

pub struct ActionHost {
    desktop: crate::desktop_host::DesktopHost,
    browser: BrowserHost,
}
impl ActionHost {
    pub fn new(config: PathBuf, desktop: crate::desktop_host::DesktopHost) -> Self {
        Self {
            desktop,
            browser: BrowserHost {
                config,
                state: Mutex::new(None),
            },
        }
    }
}
impl NativeHost for ActionHost {
    fn call(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
    ) -> Result<Value, ActionError> {
        self.call_with_deadline(
            method,
            params,
            cancellation,
            Instant::now() + Duration::from_secs(60),
        )
    }
    fn call_with_deadline(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<Value, ActionError> {
        if method.starts_with("browser.") {
            self.browser.call(method, params, cancellation, deadline)
        } else {
            self.desktop
                .call_with_deadline(method, params, cancellation, deadline)
        }
    }
}

struct BrowserHost {
    config: PathBuf,
    state: Mutex<Option<State>>,
}
struct State {
    runtime: tokio::runtime::Runtime,
    session: Session,
}
#[derive(Default)]
struct Session {
    endpoint: String,
    profile: Option<String>,
    client: Option<BrowserClient>,
}

fn fault(code: &str, message: impl Into<String>, method: &str, effect: &str) -> ActionError {
    let mut error = ActionError::new(code, message);
    error.method = Some(method.into());
    error.effect_state = Some(effect.into());
    error
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Options {
    profile: Option<String>,
    #[serde(rename = "timeoutMs")]
    timeout_ms: Option<u64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfilesArgs {
    #[serde(default)]
    options: Options,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CallArgs {
    method: String,
    args: Vec<Value>,
    #[serde(default)]
    options: Options,
}

fn valid_method(method: &str) -> bool {
    method.len() <= 256
        && !method.starts_with("chrome.")
        && method.contains('.')
        && method.split('.').all(|part| {
            !part.is_empty()
                && !matches!(part, "__proto__" | "prototype" | "constructor")
                && (part.as_bytes()[0].is_ascii_alphabetic()
                    || matches!(part.as_bytes()[0], b'_' | b'$'))
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$'))
        })
}

impl BrowserHost {
    fn call(
        &self,
        operation: &str,
        params: Value,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<Value, ActionError> {
        let invalid = || {
            fault(
                "invalid_input",
                "浏览器参数或选项无效。",
                operation,
                "not_started",
            )
        };
        if let Some(options) = params.get("options") {
            let object = options.as_object().ok_or_else(invalid)?;
            if object
                .get("profile")
                .is_some_and(|value| !value.is_string())
                || object
                    .get("timeoutMs")
                    .is_some_and(|value| value.as_u64().is_none())
            {
                return Err(invalid());
            }
        }
        let (method, args, options) = match operation {
            "browser.profiles" => {
                let request: ProfilesArgs =
                    serde_json::from_value(params).map_err(|_| invalid())?;
                (None, Value::Null, request.options)
            }
            "browser.call" => {
                let request: CallArgs = serde_json::from_value(params).map_err(|_| invalid())?;
                if !valid_method(&request.method) {
                    return Err(invalid());
                }
                (Some(request.method), json!(request.args), request.options)
            }
            _ => return Err(invalid()),
        };
        let method_name = method.as_deref().unwrap_or("browser.profiles");
        let timeout = options.timeout_ms.unwrap_or(10000);
        if !(1..=60000).contains(&timeout)
            || options
                .profile
                .as_ref()
                .is_some_and(|profile| profile.is_empty() || profile.encode_utf16().count() > 128)
        {
            return Err(invalid());
        }
        if cancellation.is_cancelled() {
            return Err(fault(
                "cancelled",
                "浏览器调用在派发前已取消。",
                method_name,
                "not_started",
            ));
        }
        if Instant::now() >= deadline {
            return Err(fault(
                "timeout",
                "动作时限已到，未开始浏览器调用。",
                method_name,
                "not_started",
            ));
        }
        if method.is_some() {
            // 在派发前校验完整 RPC 帧，避免参数超限被误报成已派发的未知结果。
            let frame = json!({"v":1,"type":"call","id":"rust:18446744073709551615","method":method_name,"args":args,"browser_instance_id":options.profile,"timeout_ms":timeout});
            omnitrigger_browser::framing::encode(&frame).map_err(|cause| {
                fault(
                    "invalid_input",
                    cause.to_string(),
                    method_name,
                    "not_started",
                )
            })?;
        }
        let endpoint =
            crate::browser_actions::script_endpoint(&self.config).map_err(|mut error| {
                error.method = Some(method_name.into());
                error
            })?;
        let mut guard = self.state.try_lock().map_err(|_| {
            fault(
                "busy",
                "本次动作已有浏览器调用正在执行。",
                method_name,
                "not_started",
            )
        })?;
        if guard.is_none() {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| {
                    fault(
                        "browser_unavailable",
                        error.to_string(),
                        method_name,
                        "not_started",
                    )
                })?;
            *guard = Some(State {
                runtime,
                session: Session::default(),
            });
        }
        let State { runtime, session } = guard.as_mut().expect("initialized");
        let profile = if method.is_some() {
            options.profile
        } else {
            session.profile.clone()
        };
        if endpoint != session.endpoint || profile != session.profile {
            session.client = None;
            session.endpoint = endpoint;
            session.profile = profile;
        }
        let dispatched = Cell::new(false);
        let stopped = |code: &str| {
            fault(
                code,
                "浏览器调用等待结束；不会自动重试或撤销已经发生的操作。",
                method_name,
                if dispatched.get() {
                    "unknown"
                } else {
                    "not_started"
                },
            )
        };
        let result = runtime.block_on(async {
            let work = async {
                if Instant::now() >= deadline { return Err(stopped("timeout")); }
                if session.client.is_none() {
                    let connect = BrowserClient::connect(&session.endpoint, session.profile.clone());
                    let client = tokio::time::timeout(Duration::from_secs(5), connect).await
                        .map_err(|_| fault("timeout", "浏览器连接握手超时。", method_name, "not_started"))?
                        .map_err(|error| fault(&error.code, error.message, method_name, "not_started"))?;
                    session.client = Some(client);
                }
                if cancellation.is_cancelled() { return Err(stopped("cancelled")); }
                if Instant::now() >= deadline { return Err(stopped("timeout")); }
                dispatched.set(true);
                let client = session.client.as_mut().expect("connected");
                let call = async {
                    if let Some(method) = method.as_deref() {
                        match client.call_value(method, args, timeout).await {
                            Ok(Some(value)) => Ok(json!({"has_value":true,"value":value})),
                            Ok(None) => Ok(json!({"has_value":false})),
                            Err(error) => {
                                let mut fault = fault(&error.fault.code, error.fault.message, method,
                                    error.fault.effect_state.as_deref().unwrap_or("unknown"));
                                fault.required_permissions = error.required_permissions;
                                Err(fault)
                            }
                        }
                    } else {
                        client.profiles().await.map_err(|error| fault(&error.code, error.message,
                            method_name, error.effect_state.as_deref().unwrap_or("unknown")))
                    }
                };
                tokio::time::timeout(Duration::from_millis(timeout), call).await
                    .map_err(|_| stopped("timeout"))?
            };
            tokio::select! {
                result = tokio::time::timeout(deadline.saturating_duration_since(Instant::now()), work) => result.map_err(|_| stopped("timeout"))?,
                _ = async {
                    loop {
                        if cancellation.is_cancelled() { break; }
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                } => Err(stopped("cancelled")),
            }
        });
        if result.is_err() {
            session.client = None;
        }
        let value = result?;
        // 与浏览器客户端的原始结果上限一致；JS 引擎自身的堆内存限制仍独立生效。
        let bytes = serde_json::to_vec(&value).map_err(|error| {
            fault(
                "invalid_response",
                error.to_string(),
                method_name,
                "unknown",
            )
        })?;
        if bytes.len() > omnitrigger_browser::MAX_RESULT + 512 {
            return Err(fault(
                "result_too_large",
                "浏览器结果超过传输上限。",
                method_name,
                "unknown",
            ));
        }
        Ok(value)
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use omnitrigger_browser::{broker, framing, platform, CHUNK_BYTES};
    use std::{
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            mpsc, Arc,
        },
        thread,
        time::{SystemTime, UNIX_EPOCH},
    };

    struct Fixture {
        root: PathBuf,
        runtime: PathBuf,
        host: Arc<BrowserHost>,
        stop: Arc<AtomicBool>,
        worker: Option<thread::JoinHandle<()>>,
        calls: Arc<AtomicUsize>,
        slow: mpsc::Receiver<()>,
    }
    impl Fixture {
        fn new() -> Self {
            let nonce = format!(
                "{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            );
            let root = std::env::temp_dir().join(format!("omnitrigger-script-browser-{nonce}"));
            let config = root.join("bindings.json");
            let runtime = crate::browser_connector::runtime_root(&config)
                .join(crate::browser_connector::CONNECTOR_VERSION);
            std::fs::create_dir_all(&runtime).unwrap();
            let snapshot = crate::browser_connector::Snapshot {
                enabled: true,
                registered: true,
                installed: true,
                extension_id: "abcdefghijklmnopabcdefghijklmnop".into(),
                version: crate::browser_connector::CONNECTOR_VERSION.into(),
                cli_path: runtime.join("omnitrigger-browser.exe"),
                ..Default::default()
            };
            std::fs::write(&snapshot.cli_path, []).unwrap();
            std::fs::write(runtime.join("omnitrigger-browser-host.exe"), []).unwrap();
            std::fs::write(
                crate::browser_connector::state_path(&config),
                serde_json::to_vec(&snapshot).unwrap(),
            )
            .unwrap();
            let pipe = format!(r"\\.\pipe\omnitrigger-script-browser-{nonce}");
            std::fs::write(
                runtime.join("connection.json"),
                json!({"pipe":pipe,"extension_ids":[snapshot.extension_id]}).to_string(),
            )
            .unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let calls = Arc::new(AtomicUsize::new(0));
            let signal = Arc::clone(&stop);
            let count = Arc::clone(&calls);
            let (ready_tx, ready_rx) = mpsc::channel();
            let (slow_tx, slow) = mpsc::channel();
            let worker = thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                rt.block_on(async {
                    let endpoint=pipe.clone();let bridge=tokio::spawn(async move{broker::serve(&endpoint).await});
                    let mut connections=Vec::new();
                    for profile in ["work","other"] {
                        let began=Instant::now();
                        let mut extension=loop {match platform::connect(&pipe).await {Ok(stream)=>break stream,Err(_) if began.elapsed()<Duration::from_secs(5)=>tokio::time::sleep(Duration::from_millis(5)).await,Err(error)=>panic!("bridge connect: {error}")}};
                        framing::write(&mut extension,&json!({"v":1,"type":"hello","role":"extension","name":profile,"browser_instance_id":profile,"session_id":format!("s-{profile}"),"capabilities":{"methods":["tabs.query","tabs.get","tabs.update","bookmarks.getTree","sessions.restore"]}})).await.unwrap();
                        assert_eq!(framing::read(&mut extension).await.unwrap()["ok"],true);
                        connections.push(extension);
                    }
                    let mut extension=connections.remove(0);
                    ready_tx.send(()).unwrap();
                    let work=async {
                        loop {
                            let message=tokio::select! {
                                response=framing::read(&mut extension)=>response.unwrap(),
                                _=async {while !signal.load(Ordering::SeqCst){tokio::time::sleep(Duration::from_millis(5)).await;}}=>break,
                            };
                            if message["type"]!="call" {continue;}
                            count.fetch_add(1,Ordering::SeqCst);
                            assert_eq!(message["browser_instance_id"],"work");
                            let id=message["id"].clone();
                            let method=message["method"].as_str().unwrap();
                            if method=="tabs.query" && message["args"]==json!(["slow"]) {slow_tx.send(()).unwrap();continue;}
                            if method=="tabs.query" && message["args"]==json!(["reported-unknown"]) {
                                framing::write(&mut extension,&json!({"v":1,"type":"result","id":id,"session_id":"s-work","ok":false,"error":{"code":"result_unknown","message":"上游未确认已派发操作的结果","effect_state":"unknown"}})).await.unwrap();continue;
                            }
                            if method=="bookmarks.getTree" {
                                let bytes=serde_json::to_vec(&json!([{"title":"中文😀".repeat(110000)}])).unwrap();
                                framing::write(&mut extension,&json!({"v":1,"type":"result.begin","id":id,"session_id":"s-work","total_bytes":bytes.len(),"chunk_count":bytes.len().div_ceil(CHUNK_BYTES)})).await.unwrap();
                                for (index,chunk) in bytes.chunks(CHUNK_BYTES).enumerate() {
                                    framing::write(&mut extension,&json!({"v":1,"type":"result.chunk","id":id,"session_id":"s-work","index":index,"data":base64(chunk)})).await.unwrap();
                                    let ack=framing::read(&mut extension).await.unwrap();assert_eq!(ack["type"],"result.ack");assert_eq!(ack["index"],index);
                                }
                                framing::write(&mut extension,&json!({"v":1,"type":"result.end","id":id,"session_id":"s-work"})).await.unwrap();continue;
                            }
                            let reply=match method {
                                "tabs.update"=>json!({"v":1,"type":"result","id":id,"session_id":"s-work","ok":true,"has_value":false}),
                                "tabs.get"=>json!({"v":1,"type":"result","id":id,"session_id":"s-work","ok":true,"has_value":true,"value":null}),
                                "sessions.restore"=>json!({"v":1,"type":"result","id":id,"session_id":"s-work","ok":false,"error":{"code":"permission_required","message":"need sessions","effect_state":"not_started","required_permissions":["sessions"]}}),
                                _=>json!({"v":1,"type":"result","id":id,"session_id":"s-work","ok":true,"has_value":true,"value":{"done":true}}),
                            };
                            framing::write(&mut extension,&reply).await.unwrap();
                        }
                    };
                    tokio::time::timeout(Duration::from_secs(30),work).await.expect("fixture timeout");
                    drop(connections);bridge.abort();let _=bridge.await;
                });
            });
            ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            Self {
                root,
                runtime,
                host: Arc::new(BrowserHost {
                    config,
                    state: Mutex::new(None),
                }),
                stop,
                worker: Some(worker),
                calls,
                slow,
            }
        }
        fn call(
            &self,
            method: &str,
            args: Value,
            timeout: u64,
            deadline: Instant,
            cancel: &CancellationToken,
        ) -> Result<Value, ActionError> {
            self.host.call("browser.call",json!({"method":method,"args":args,"options":{"profile":"work","timeoutMs":timeout}}),cancel,deadline)
        }
        fn finish(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            self.worker.take().unwrap().join().unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
            for file in [
                self.root.join("browser-connector.json"),
                self.runtime.join("connection.json"),
                self.runtime.join("omnitrigger-browser.exe"),
                self.runtime.join("omnitrigger-browser-host.exe"),
            ] {
                let _ = std::fs::remove_file(file);
            }
            let mut directory = self.runtime.clone();
            while directory.starts_with(&self.root) {
                let _ = std::fs::remove_dir(&directory);
                if directory == self.root {
                    break;
                }
                directory.pop();
            }
        }
    }
    fn base64(bytes: &[u8]) -> String {
        const ABC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let a = chunk[0];
            let b = chunk.get(1).copied().unwrap_or(0);
            let c = chunk.get(2).copied().unwrap_or(0);
            out.push(ABC[(a >> 2) as usize] as char);
            out.push(ABC[(((a & 3) << 4) | (b >> 4)) as usize] as char);
            out.push(if chunk.len() > 1 {
                ABC[(((b & 15) << 2) | (c >> 6)) as usize] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                ABC[(c & 63) as usize] as char
            } else {
                '='
            });
        }
        out
    }

    #[test]
    fn script_browser_routes_profiles_preserves_chunks_and_cancels_without_replay() {
        let mut fixture = Fixture::new();
        let token = CancellationToken::new();
        let deadline = || Instant::now() + Duration::from_secs(10);
        let profiles = fixture
            .host
            .call("browser.profiles", json!({}), &token, deadline())
            .unwrap();
        assert_eq!(profiles.as_array().unwrap().len(), 2);
        let ambiguous = fixture
            .host
            .call(
                "browser.call",
                json!({"method":"tabs.query","args":[],"options":{}}),
                &token,
                deadline(),
            )
            .unwrap_err();
        assert_eq!(ambiguous.code, "target_ambiguous");
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            fixture
                .call("tabs.get", json!([]), 10000, deadline(), &token)
                .unwrap(),
            json!({"has_value":true,"value":null})
        );
        assert_eq!(
            fixture
                .call("tabs.update", json!([]), 10000, deadline(), &token)
                .unwrap(),
            json!({"has_value":false})
        );
        let large = fixture
            .call("bookmarks.getTree", json!([]), 10000, deadline(), &token)
            .unwrap();
        assert_eq!(
            large["value"][0]["title"]
                .as_str()
                .unwrap()
                .encode_utf16()
                .count(),
            440000
        );
        let error = fixture
            .call("sessions.restore", json!([]), 10000, deadline(), &token)
            .unwrap_err();
        assert_eq!(error.code, "permission_required");
        assert_eq!(error.required_permissions, vec!["sessions"]);
        let count = fixture.calls.load(Ordering::SeqCst);
        assert_eq!(count, 4);
        let invalid = fixture
            .host
            .call(
                "browser.call",
                json!({"method":"tabs.query","args":[],"options":{"profile":null}}),
                &token,
                deadline(),
            )
            .unwrap_err();
        assert_eq!(invalid.effect_state.as_deref(), Some("not_started"));
        let expired = fixture
            .call("tabs.query", json!([]), 10000, Instant::now(), &token)
            .unwrap_err();
        assert_eq!(expired.effect_state.as_deref(), Some("not_started"));
        assert_eq!(fixture.calls.load(Ordering::SeqCst), count);
        // 上游错误必须原样保留。不要让本地与代理的同期限计时器竞争，
        // 再把谁先唤醒当作错误码契约；本地截止由下方 outer 场景单独验证。
        let reported = fixture
            .call(
                "tabs.query",
                json!(["reported-unknown"]),
                10000,
                deadline(),
                &token,
            )
            .unwrap_err();
        assert_eq!(reported.code, "result_unknown");
        assert_eq!(reported.effect_state.as_deref(), Some("unknown"));
        assert_eq!(reported.method.as_deref(), Some("tabs.query"));
        assert_eq!(fixture.calls.load(Ordering::SeqCst), count + 1);
        let host = Arc::clone(&fixture.host);
        let cancel = CancellationToken::new();
        let signal = cancel.clone();
        let worker = thread::spawn(move || {
            host.call("browser.call",json!({"method":"tabs.query","args":["slow"],"options":{"profile":"work","timeoutMs":60000}}),&signal,Instant::now()+Duration::from_secs(60))
        });
        fixture.slow.recv_timeout(Duration::from_secs(5)).unwrap();
        let began = Instant::now();
        cancel.cancel();
        let error = worker.join().unwrap().unwrap_err();
        assert_eq!(error.code, "cancelled");
        assert_eq!(error.effect_state.as_deref(), Some("unknown"));
        assert!(began.elapsed() < Duration::from_secs(2));
        let outer = fixture
            .call(
                "tabs.query",
                json!(["slow"]),
                60000,
                Instant::now() + Duration::from_millis(500),
                &token,
            )
            .unwrap_err();
        assert_eq!(outer.code, "timeout");
        assert_eq!(outer.effect_state.as_deref(), Some("unknown"));
        assert_eq!(outer.method.as_deref(), Some("tabs.query"));
        fixture.slow.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(
            fixture
                .call("tabs.query", json!([]), 10000, deadline(), &token)
                .unwrap(),
            json!({"has_value":true,"value":{"done":true}})
        );
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 8);
        fixture.finish();
    }
}
