//! 每次调用独立的本机能力桥接。只监听回环地址，不使用脚本 stdout。
use crate::{ActionError, CancellationToken};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

// 65,536 个 UTF-16 单元即使全部需要 JSON 转义也能完整传输。
const LIMIT: usize = 512 * 1024;
const RESPONSE_LIMIT: usize = 512 * 1024;
const BROWSER_RESPONSE_LIMIT: usize = 32 * 1024 * 1024 + 1024;
const CLIENTS: usize = 4;
const METHODS: [&str; 25] = [
    "system.open",
    "process.capture",
    "history.list",
    "history.get",
    "history.append",
    "history.remove",
    "fs.stat",
    "ui.choose",
    "ui.notify",
    "ui.open_history",
    "ui.toggle_history",
    "browser.profiles",
    "browser.call",
    "target.info",
    "target.context",
    "keyboard.shortcut",
    "keyboard.sequence",
    "keyboard.text",
    "selection.copy",
    "selection.copy_data",
    "clipboard.read",
    "clipboard.read_data",
    "clipboard.write",
    "window.action",
    "display.brightness_step",
];

/// 由应用为本次调用捕获的本机上下文。不是沙盒或权限隔离。
/// 回调在动作工作线程执行，须有界返回，并在副作用前核验取消和目标身份。
pub trait NativeHost: Send + Sync {
    fn call(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
    ) -> Result<Value, ActionError>;

    /// 长时间原生调用可据此遵守外层动作时限；默认沿用原有有界回调。
    fn call_with_deadline(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
        _deadline: Instant,
    ) -> Result<Value, ActionError> {
        self.call(method, params, cancellation)
    }
}

pub(crate) struct NativeBridge {
    listener: TcpListener,
    token: String,
    clients: Vec<Client>,
    next_id: u32,
}

struct Client {
    stream: TcpStream,
    began: Instant,
    input: Vec<u8>,
    output: Option<Vec<u8>>,
    written: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    token: String,
    id: u32,
    method: String,
    params: Value,
}

fn io_error(error: impl ToString) -> ActionError {
    let mut error = ActionError::new(
        "native_bridge_unavailable",
        format!("本机能力桥接不可用：{}", error.to_string()),
    );
    error.effect_state = Some("not_started".into());
    error
}

/// 交互 UI 和复制等待可能暂时停留在本次 poll 栈中，不能依赖父运行时
/// 继续轮询子进程才能发现宿主退出。旁路只检测连接和取消，不读走协议数据。
/// 无法强制抢占已经进入的阻塞系统调用；实现仍须在调用前后复验取消。
fn interactive_call(
    stream: &TcpStream,
    parent: &CancellationToken,
    call: impl FnOnce(&CancellationToken) -> Result<Value, ActionError>,
) -> Result<Value, ActionError> {
    let stream = stream.try_clone().map_err(io_error)?;
    let cancellation = CancellationToken::new();
    let done = AtomicBool::new(false);
    struct Finish<'a>(&'a AtomicBool);
    impl Drop for Finish<'_> {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }
    thread::scope(|scope| {
        let token = &cancellation;
        let stopped = &done;
        scope.spawn(move || {
            while !stopped.load(Ordering::Acquire) {
                let mut byte = [0];
                let disconnected = match stream.peek(&mut byte) {
                    Ok(_) => true, // EOF，或违反一连接一帧约定的尾随数据。
                    Err(error) => error.kind() != std::io::ErrorKind::WouldBlock,
                };
                if parent.is_cancelled() || disconnected {
                    token.cancel();
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
        });
        let _finish = Finish(&done);
        if parent.is_cancelled() {
            cancellation.cancel();
        }
        call(&cancellation)
    })
}

impl NativeBridge {
    pub(crate) fn new() -> Result<Self, ActionError> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(io_error)?;
        listener.set_nonblocking(true).map_err(io_error)?;
        let mut random = [0u8; 32];
        getrandom::fill(&mut random).map_err(io_error)?;
        let token = random.iter().map(|byte| format!("{byte:02x}")).collect();
        Ok(Self {
            listener,
            token,
            clients: Vec::new(),
            next_id: 1,
        })
    }

    pub(crate) fn configuration(&self) -> Result<Value, ActionError> {
        Ok(
            json!({"port": self.listener.local_addr().map_err(io_error)?.port(), "token": self.token}),
        )
    }

    /// 每次只接纳一个连接，每客户端最多读取一帧；全部缓冲和在途连接有上限。
    #[cfg(test)]
    pub(crate) fn poll(&mut self, host: &dyn NativeHost, cancellation: &CancellationToken) {
        self.poll_until(host, cancellation, None);
    }

    pub(crate) fn poll_until(
        &mut self,
        host: &dyn NativeHost,
        cancellation: &CancellationToken,
        deadline: Option<Instant>,
    ) {
        if let Ok((stream, _)) = self.listener.accept() {
            if self.clients.len() < CLIENTS && stream.set_nonblocking(true).is_ok() {
                let _ = stream.set_nodelay(true);
                self.clients.push(Client {
                    stream,
                    began: Instant::now(),
                    input: Vec::new(),
                    output: None,
                    written: 0,
                });
            }
        }
        let token = &self.token;
        let next_id = &mut self.next_id;
        self.clients.retain_mut(|client| {
            let lifetime = if client.output.is_some() {
                Duration::from_secs(5)
            } else {
                Duration::from_secs(2)
            };
            if cancellation.is_cancelled() || client.began.elapsed() > lifetime {
                return false;
            }
            if client.output.is_none() {
                let mut bytes = [0u8; 4096];
                for _ in 0..16 {
                    match client.stream.read(&mut bytes) {
                        Ok(0) => return false,
                        Ok(count) => client.input.extend_from_slice(&bytes[..count]),
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(_) => return false,
                    }
                    if client.input.len() > LIMIT {
                        return false;
                    }
                    if client.input.contains(&b'\n') {
                        break;
                    }
                }
                if client.input.len() > LIMIT {
                    return false;
                }
                if let Some(end) = client.input.iter().position(|byte| *byte == b'\n') {
                    if end + 1 != client.input.len() {
                        return false;
                    }
                    let Ok(request) = serde_json::from_slice::<Request>(&client.input[..end])
                    else {
                        return false;
                    };
                    if request.token != *token || request.id != *next_id || request.id > 128 {
                        return false;
                    }
                    *next_id += 1;
                    let opening = request.method == "system.open";
                    let capture = request.method == "process.capture";
                    let brightness = request.method == "display.brightness_step";
                    let data_read = request.method == "clipboard.read_data";
                    let data_copy = request.method == "selection.copy_data";
                    let history_read =
                        matches!(request.method.as_str(), "history.list" | "history.get");
                    let history = matches!(
                        request.method.as_str(),
                        "history.list" | "history.get" | "history.append" | "history.remove"
                    );
                    let result = if !METHODS.contains(&request.method.as_str()) {
                        let mut error = ActionError::new("invalid_input", "未提供此本机调用方法。");
                        error.effect_state = Some("not_started".into());
                        Err(error)
                    } else if !request.params.is_object()
                        || (data_read
                            && request
                                .params
                                .as_object()
                                .is_some_and(|params| !params.is_empty()))
                    {
                        let mut error = ActionError::new("invalid_input", "本机调用参数无效。");
                        error.effect_state = Some("not_started".into());
                        Err(error)
                    } else if cancellation.is_cancelled() {
                        let mut error =
                            ActionError::new("cancelled", "动作已取消，未开始本机调用。");
                        if opening || data_read || data_copy || history || capture || brightness {
                            error.effect_state = Some("not_started".into());
                        }
                        Err(error)
                    } else {
                        let call = |cancellation: &CancellationToken| match deadline {
                            Some(deadline) => host.call_with_deadline(
                                &request.method,
                                request.params,
                                cancellation,
                                deadline,
                            ),
                            None => host.call(&request.method, request.params, cancellation),
                        };
                        if opening
                            || request.method == "ui.choose"
                            || request.method == "ui.open_history"
                            || request.method == "ui.toggle_history"
                            || data_copy
                            || history
                            || capture
                            || brightness
                        {
                            interactive_call(&client.stream, cancellation, call)
                        } else {
                            call(cancellation)
                        }
                    }
                    .map_err(|mut error| {
                        if brightness {
                            error.message = "亮度步进未获完整确认；请检查结构化回执。".into();
                            error.method = Some("display.brightness_step".into());
                            error.required_permissions.clear();
                            if !matches!(
                                error.effect_state.as_deref(),
                                Some("not_started" | "unknown")
                            ) {
                                error.effect_state = Some("unknown".into());
                            }
                        }
                        if opening {
                            error.message = "打开请求未确认，请检查错误代码。".into();
                            error.method = Some("system.open".into());
                            error.required_permissions.clear();
                            if error.effect_state.as_deref() != Some("not_started") {
                                error.effect_state = Some("unknown".into());
                            }
                        }
                        if capture {
                            error.message = "程序输出采集未完成，请检查错误代码。".into();
                            error.method = Some(request.method.clone());
                            error.required_permissions.clear();
                            if error.effect_state.as_deref() != Some("not_started") {
                                error.effect_state = Some("unknown".into());
                            }
                        } else if data_read || data_copy || history {
                            error.message = if history {
                                "剪贴板历史调用失败；请检查错误代码。"
                            } else {
                                "剪贴板或选区数据调用失败；请检查错误代码。"
                            }
                            .into();
                            error.method = Some(request.method.clone());
                            error.required_permissions.clear();
                            if data_read || history_read {
                                error.effect_state = Some("not_started".into());
                            } else if error.effect_state.as_deref() != Some("not_started") {
                                error.effect_state = Some("unknown".into());
                            }
                        }
                        error
                    });
                    let response = match result {
                        Ok(value) => json!({"id":request.id, "ok":true, "value":value}),
                        Err(error) => json!({"id":request.id, "ok":false, "error":error}),
                    };
                    let Ok(mut bytes) = serde_json::to_vec(&response) else {
                        return false;
                    };
                    let limit = if request.method.starts_with("browser.") {
                        BROWSER_RESPONSE_LIMIT
                    } else {
                        RESPONSE_LIMIT
                    };
                    if bytes.len() > limit {
                        return false;
                    }
                    bytes.push(b'\n');
                    client.output = Some(bytes);
                    client.began = Instant::now();
                }
            }
            if let Some(bytes) = &client.output {
                match client.stream.write(&bytes[client.written..]) {
                    Ok(0) => return false,
                    Ok(count) => client.written += count,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return true,
                    Err(_) => return false,
                }
                return client.written < bytes.len();
            }
            true
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn interactive_wait_observes_parent_cancel_and_host_disconnect_without_polling_runtime() {
        for disconnect in [false, true] {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            let peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
            let (stream, _) = listener.accept().unwrap();
            stream.set_nonblocking(true).unwrap();
            let parent = CancellationToken::new();
            let token = parent.clone();
            let (started, ready) = std::sync::mpsc::sync_channel(1);
            let worker = thread::spawn(move || {
                interactive_call(&stream, &token, |cancellation| {
                    started.send(()).unwrap();
                    let deadline = Instant::now() + Duration::from_secs(2);
                    while !cancellation.is_cancelled() && Instant::now() < deadline {
                        thread::sleep(Duration::from_millis(1));
                    }
                    assert!(
                        cancellation.is_cancelled(),
                        "交互窗口未观察到宿主断连或动作取消"
                    );
                    Ok(Value::Null)
                })
            });
            ready.recv().unwrap();
            if disconnect {
                drop(peer);
            } else {
                parent.cancel();
            }
            assert_eq!(worker.join().unwrap().unwrap(), Value::Null);
            // 断连仅取消本次交互调用，不污染可供动作服务查询的外层令牌。
            assert_eq!(parent.is_cancelled(), !disconnect);
        }
    }

    #[test]
    fn interactive_monitor_stops_after_a_normal_response() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let _peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        stream.set_nonblocking(true).unwrap();
        let value = interactive_call(&stream, &CancellationToken::new(), |_| {
            Ok(json!({"status":"selected","id":"stable"}))
        })
        .unwrap();
        assert_eq!(value["id"], "stable");
    }

    struct Host(AtomicUsize);
    impl NativeHost for Host {
        fn call(&self, _: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"submitted":true}))
        }
    }
    fn request(bridge: &NativeBridge, id: u32, token: &str) -> TcpStream {
        let mut client = TcpStream::connect(bridge.listener.local_addr().unwrap()).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let frame =
            json!({"id":id,"token":token,"method":"keyboard.shortcut","params":{"keys":[65]}});
        writeln!(client, "{frame}").unwrap();
        client
    }

    fn drain(bridge: &mut NativeBridge, host: &Host, cancel: &CancellationToken) {
        let began = Instant::now();
        loop {
            bridge.poll(host, cancel);
            if bridge.clients.is_empty() {
                break;
            }
            assert!(began.elapsed() < Duration::from_secs(3));
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn native_bridge_authenticates_once_and_rejects_replay_oversize_and_cancelled_requests() {
        let mut bridge = NativeBridge::new().unwrap();
        let host = Host(AtomicUsize::new(0));
        let cancel = CancellationToken::new();
        let mut wrong = request(&bridge, 1, "wrong");
        drain(&mut bridge, &host, &cancel);
        assert_eq!(host.0.load(Ordering::SeqCst), 0);
        assert_eq!(wrong.read(&mut [0u8; 1]).unwrap(), 0);
        let mut accepted = request(&bridge, 1, &bridge.token);
        drain(&mut bridge, &host, &cancel);
        let mut response = String::new();
        accepted.read_to_string(&mut response).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&response).unwrap()["value"]["submitted"],
            true
        );
        assert_eq!(host.0.load(Ordering::SeqCst), 1);
        let _replay = request(&bridge, 1, &bridge.token);
        drain(&mut bridge, &host, &cancel);
        assert_eq!(host.0.load(Ordering::SeqCst), 1);
        let mut huge = TcpStream::connect(bridge.listener.local_addr().unwrap()).unwrap();
        let writer = std::thread::spawn(move || {
            let _ = huge.write_all(&vec![b'x'; LIMIT + 1]);
        });
        drain(&mut bridge, &host, &cancel);
        writer.join().unwrap();
        assert!(bridge.clients.is_empty());
        let _cancelled = request(&bridge, 2, &bridge.token);
        cancel.cancel();
        drain(&mut bridge, &host, &cancel);
        assert_eq!(host.0.load(Ordering::SeqCst), 1);
        let address = bridge.listener.local_addr().unwrap();
        drop(bridge);
        assert!(TcpStream::connect(address).is_err());
    }
}
