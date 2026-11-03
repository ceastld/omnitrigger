//! 每次调用独立的本机能力桥接。只监听回环地址，不使用脚本 stdout。
use crate::{ActionError, CancellationToken};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    time::{Duration, Instant},
};

const LIMIT: usize = 16 * 1024;
const RESPONSE_LIMIT: usize = 64 * 1024;
const CLIENTS: usize = 4;

/// 由应用为本次调用捕获的本机上下文。不是沙盒或权限隔离。
/// 回调在动作工作线程执行，须有界返回，并在副作用前核验取消和目标身份。
pub trait NativeHost: Send + Sync {
    fn call(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
    ) -> Result<Value, ActionError>;
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
    pub(crate) fn poll(&mut self, host: &dyn NativeHost, cancellation: &CancellationToken) {
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
            if cancellation.is_cancelled() || client.began.elapsed() > Duration::from_secs(2) {
                return false;
            }
            if client.output.is_none() {
                let mut bytes = [0u8; 4096];
                match client.stream.read(&mut bytes) {
                    Ok(0) => return false,
                    Ok(count) => client.input.extend_from_slice(&bytes[..count]),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return true,
                    Err(_) => return false,
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
                    let result = if cancellation.is_cancelled() {
                        Err(ActionError::new(
                            "cancelled",
                            "动作已取消，未开始本机调用。",
                        ))
                    } else {
                        host.call(&request.method, request.params, cancellation)
                    };
                    let response = match result {
                        Ok(value) => json!({"id":request.id, "ok":true, "value":value}),
                        Err(error) => json!({"id":request.id, "ok":false, "error":error}),
                    };
                    let Ok(mut bytes) = serde_json::to_vec(&response) else {
                        return false;
                    };
                    if bytes.len() > RESPONSE_LIMIT {
                        return false;
                    }
                    bytes.push(b'\n');
                    client.output = Some(bytes);
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
        huge.write_all(&vec![b'x'; LIMIT + 1]).unwrap();
        drain(&mut bridge, &host, &cancel);
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
