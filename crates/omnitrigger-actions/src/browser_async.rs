//! 每次脚本调用独立的浏览器异步桥；同步桌面桥的顺序和方法集不变。
use crate::{ActionError, CancellationToken, NativeHost};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    panic::{catch_unwind, AssertUnwindSafe},
    sync::atomic::{AtomicBool, Ordering},
    thread::{self, Scope, ScopedJoinHandle},
    time::{Duration, Instant},
};

const CLIENT_LIMIT: usize = 4;
const REQUEST_LIMIT: usize = 512 * 1024;
const RESPONSE_LIMIT: usize = 32 * 1024 * 1024 + 1024;

pub(crate) struct AsyncBrowserBridge<'scope> {
    listener: TcpListener,
    token: String,
    seen: [bool; 129],
    clients: Vec<Client>,
    workers: Vec<ScopedJoinHandle<'scope, ()>>,
    stopped: CancellationToken,
}

struct Client {
    stream: TcpStream,
    began: Instant,
    input: Vec<u8>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    token: String,
    id: u32,
    method: String,
    params: Value,
}

fn fault(code: &str, method: &str, effect: &str) -> ActionError {
    let mut error = ActionError::new(code, "异步浏览器调用未获完整确认；不会自动重试。");
    error.method = Some(method.into());
    error.effect_state = Some(effect.into());
    error
}

impl<'scope> AsyncBrowserBridge<'scope> {
    pub(crate) fn new() -> Result<Self, ActionError> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .map_err(|_| fault("browser_async_unavailable", "browser.call", "not_started"))?;
        listener
            .set_nonblocking(true)
            .map_err(|_| fault("browser_async_unavailable", "browser.call", "not_started"))?;
        let mut random = [0u8; 32];
        getrandom::fill(&mut random)
            .map_err(|_| fault("browser_async_unavailable", "browser.call", "not_started"))?;
        Ok(Self {
            listener,
            token: random.iter().map(|byte| format!("{byte:02x}")).collect(),
            seen: [false; 129],
            clients: Vec::new(),
            workers: Vec::new(),
            stopped: CancellationToken::new(),
        })
    }

    pub(crate) fn configuration(&self) -> Result<Value, ActionError> {
        let port = self
            .listener
            .local_addr()
            .map_err(|_| fault("browser_async_unavailable", "browser.call", "not_started"))?
            .port();
        Ok(json!({"port":port,"token":self.token}))
    }

    pub(crate) fn poll<'env: 'scope>(
        &mut self,
        scope: &'scope Scope<'scope, 'env>,
        host: &'env dyn NativeHost,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) {
        let mut index = 0;
        while index < self.workers.len() {
            if self.workers[index].is_finished() {
                let _ = self.workers.swap_remove(index).join();
            } else {
                index += 1;
            }
        }
        if cancellation.is_cancelled() || Instant::now() >= deadline {
            self.stopped.cancel();
            self.clients.clear();
            return;
        }
        if let Ok((stream, _)) = self.listener.accept() {
            // 子宿主在连接前执行同样的四项容量检查；违规的第五连接不派发。
            if self.clients.len() + self.workers.len() < CLIENT_LIMIT
                && stream.set_nonblocking(true).is_ok()
            {
                let _ = stream.set_nodelay(true);
                self.clients.push(Client {
                    stream,
                    began: Instant::now(),
                    input: Vec::new(),
                });
            }
        }
        let mut index = 0;
        while index < self.clients.len() {
            let frame = read_frame(&mut self.clients[index]);
            match frame {
                Ok(None) => index += 1,
                Err(()) => {
                    self.clients.swap_remove(index);
                }
                Ok(Some(request)) => {
                    let client = self.clients.swap_remove(index);
                    if request.token != self.token
                        || !(1..=128).contains(&request.id)
                        || self.seen[request.id as usize]
                    {
                        continue;
                    }
                    // 已认证的ID只消费一次，失败的方法也不能复用ID重放。
                    self.seen[request.id as usize] = true;
                    if !matches!(request.method.as_str(), "browser.call" | "browser.profiles")
                        || !request.params.is_object()
                    {
                        continue;
                    }
                    let parent = cancellation.clone();
                    let stopped = self.stopped.clone();
                    if let Ok(worker) = thread::Builder::new()
                        .name("omnitrigger-browser-async".into())
                        .spawn_scoped(scope, move || {
                            serve(client.stream, request, host, parent, stopped, deadline);
                        })
                    {
                        self.workers.push(worker);
                    }
                }
            }
        }
    }
}

impl Drop for AsyncBrowserBridge<'_> {
    fn drop(&mut self) {
        // execute外围的thread::scope随后等待全部worker，早于ActionService释放调用资源。
        self.stopped.cancel();
    }
}

fn read_frame(client: &mut Client) -> Result<Option<Request>, ()> {
    if client.began.elapsed() >= Duration::from_secs(2) {
        return Err(());
    }
    let mut chunk = [0u8; 4096];
    for _ in 0..16 {
        match client.stream.read(&mut chunk) {
            Ok(0) => return Err(()),
            Ok(count) => client.input.extend_from_slice(&chunk[..count]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(_) => return Err(()),
        }
        if client.input.len() > REQUEST_LIMIT {
            return Err(());
        }
        if client.input.contains(&b'\n') {
            break;
        }
    }
    let Some(end) = client.input.iter().position(|byte| *byte == b'\n') else {
        return Ok(None);
    };
    if end + 1 != client.input.len() {
        return Err(());
    }
    serde_json::from_slice(&client.input[..end])
        .map(Some)
        .map_err(|_| ())
}

fn invoke(
    stream: &TcpStream,
    request: &Request,
    host: &dyn NativeHost,
    parent: &CancellationToken,
    stopped: &CancellationToken,
    deadline: Instant,
) -> Result<Value, ActionError> {
    let monitor = stream
        .try_clone()
        .map_err(|_| fault("browser_async_unavailable", &request.method, "not_started"))?;
    let cancellation = CancellationToken::new();
    let done = AtomicBool::new(false);
    struct Finish<'a>(&'a AtomicBool);
    impl Drop for Finish<'_> {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }
    thread::scope(|scope| {
        scope.spawn(|| {
            while !done.load(Ordering::Acquire) {
                let mut byte = [0u8];
                let disconnected = match monitor.peek(&mut byte) {
                    Ok(_) => true, // EOF或一连接一帧之外的额外输入。
                    Err(error) => error.kind() != std::io::ErrorKind::WouldBlock,
                };
                if parent.is_cancelled()
                    || stopped.is_cancelled()
                    || Instant::now() >= deadline
                    || disconnected
                {
                    cancellation.cancel();
                    break;
                }
                thread::sleep(Duration::from_millis(5));
            }
        });
        let _finish = Finish(&done);
        if parent.is_cancelled() || stopped.is_cancelled() || Instant::now() >= deadline {
            return Err(fault("cancelled", &request.method, "not_started"));
        }
        host.call_browser_async(
            &request.method,
            request.params.clone(),
            &cancellation,
            deadline,
        )
    })
}

fn serve(
    mut stream: TcpStream,
    request: Request,
    host: &dyn NativeHost,
    parent: CancellationToken,
    stopped: CancellationToken,
    deadline: Instant,
) {
    let result = catch_unwind(AssertUnwindSafe(|| {
        invoke(&stream, &request, host, &parent, &stopped, deadline)
    }))
    .unwrap_or_else(|_| Err(fault("native_host_failed", &request.method, "unknown")))
    .map_err(|mut error| {
        if error.method.is_none() {
            error.method = Some(request.method.clone());
        }
        if !matches!(
            error.effect_state.as_deref(),
            Some("not_started" | "unknown")
        ) {
            error.effect_state = Some("unknown".into());
        }
        error
    });
    let frame = match result {
        Ok(value) => json!({"id":request.id,"ok":true,"value":value}),
        Err(error) => json!({"id":request.id,"ok":false,"error":error}),
    };
    let Ok(mut output) = serde_json::to_vec(&frame) else {
        return;
    };
    if output.len() > RESPONSE_LIMIT {
        let error = fault("invalid_response", &request.method, "unknown");
        let Ok(short) = serde_json::to_vec(&json!({"id":request.id,"ok":false,"error":error}))
        else {
            return;
        };
        output = short;
    }
    output.push(b'\n');
    let Some(write_deadline) = Instant::now().checked_add(Duration::from_secs(5)) else {
        return;
    };
    let write_deadline = write_deadline.min(deadline);
    let mut written = 0;
    while written < output.len() {
        if parent.is_cancelled() || stopped.is_cancelled() || Instant::now() >= write_deadline {
            return;
        }
        match stream.write(&output[written..]) {
            Ok(0) => return,
            Ok(count) => written += count,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(2));
            }
            Err(_) => return,
        }
    }
}

#[cfg(test)]
#[path = "browser_async_tests.rs"]
mod tests;
