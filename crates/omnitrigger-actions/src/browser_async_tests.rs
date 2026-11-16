use super::*;
use std::sync::{atomic::AtomicUsize, mpsc, Mutex};

fn connect(
    bridge: &AsyncBrowserBridge<'_>,
    id: u32,
    method: &str,
    token: Option<&str>,
) -> TcpStream {
    let port = bridge.configuration().unwrap()["port"].as_u64().unwrap() as u16;
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let request = json!({"token":token.unwrap_or(&bridge.token),"id":id,"method":method,"params":{"sequence":id}});
    writeln!(stream, "{request}").unwrap();
    stream
}

fn response(mut stream: TcpStream) -> Value {
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn poll_until<'scope, 'env: 'scope>(
    bridge: &mut AsyncBrowserBridge<'scope>,
    scope: &'scope Scope<'scope, 'env>,
    host: &'env dyn NativeHost,
    cancellation: &CancellationToken,
    deadline: Instant,
    ready: impl Fn() -> bool,
) {
    while !ready() {
        assert!(Instant::now() < deadline, "异步桥没有在期限内推进");
        bridge.poll(scope, host, cancellation, deadline);
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn authenticated_ids_can_arrive_out_of_order_and_calls_complete_in_reverse_order() {
    struct Host {
        entered: AtomicUsize,
        release_first: Mutex<mpsc::Receiver<()>>,
    }
    impl NativeHost for Host {
        fn browser_async_available(&self) -> bool {
            true
        }
        fn call(
            &self,
            method: &str,
            params: Value,
            cancellation: &CancellationToken,
        ) -> Result<Value, ActionError> {
            assert_eq!(method, "browser.call");
            self.entered.fetch_add(1, Ordering::SeqCst);
            while self.entered.load(Ordering::SeqCst) < 2 {
                if cancellation.is_cancelled() {
                    return Err(fault("cancelled", method, "unknown"));
                }
                thread::sleep(Duration::from_millis(2));
            }
            if params["sequence"] == 1 {
                loop {
                    if cancellation.is_cancelled() {
                        return Err(fault("cancelled", method, "unknown"));
                    }
                    match self
                        .release_first
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_millis(5))
                    {
                        Ok(()) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(error) => panic!("{error}"),
                    }
                }
            }
            Ok(params)
        }
    }
    let (release, receiver) = mpsc::channel();
    let host = Host {
        entered: AtomicUsize::new(0),
        release_first: Mutex::new(receiver),
    };
    let cancellation = CancellationToken::new();
    thread::scope(|scope| {
        let mut bridge = AsyncBrowserBridge::new().unwrap();
        let second = connect(&bridge, 2, "browser.call", None);
        let first = connect(&bridge, 1, "browser.call", None);
        let deadline = Instant::now() + Duration::from_secs(3);
        poll_until(&mut bridge, scope, &host, &cancellation, deadline, || {
            host.entered.load(Ordering::SeqCst) == 2
        });
        assert_eq!(
            response(second),
            json!({"id":2,"ok":true,"value":{"sequence":2}})
        );
        release.send(()).unwrap();
        assert_eq!(
            response(first),
            json!({"id":1,"ok":true,"value":{"sequence":1}})
        );
    });
    assert_eq!(host.entered.load(Ordering::SeqCst), 2);
}

struct WaitingHost {
    entered: AtomicUsize,
    active: AtomicUsize,
}

impl WaitingHost {
    fn new() -> Self {
        Self {
            entered: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
        }
    }
}

impl NativeHost for WaitingHost {
    fn browser_async_available(&self) -> bool {
        true
    }
    fn call(
        &self,
        method: &str,
        _: Value,
        cancellation: &CancellationToken,
    ) -> Result<Value, ActionError> {
        struct Active<'a>(&'a AtomicUsize);
        impl Drop for Active<'_> {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }
        self.active.fetch_add(1, Ordering::SeqCst);
        let _active = Active(&self.active);
        self.entered.fetch_add(1, Ordering::SeqCst);
        while !cancellation.is_cancelled() {
            thread::sleep(Duration::from_millis(2));
        }
        Err(fault("cancelled", method, "unknown"))
    }
}

#[test]
fn four_pending_calls_bound_capacity_and_drop_joins_before_scope_returns() {
    let host = WaitingHost::new();
    let cancellation = CancellationToken::new();
    thread::scope(|scope| {
        let mut bridge = AsyncBrowserBridge::new().unwrap();
        let _clients: Vec<_> = (1..=4)
            .map(|id| connect(&bridge, id, "browser.call", None))
            .collect();
        let deadline = Instant::now() + Duration::from_secs(3);
        poll_until(&mut bridge, scope, &host, &cancellation, deadline, || {
            host.entered.load(Ordering::SeqCst) == 4
        });
        let mut extra = connect(&bridge, 5, "browser.call", None);
        bridge.poll(scope, &host, &cancellation, deadline);
        let mut bytes = Vec::new();
        // 容量之外的连接会关闭或复位，不派发第五个Native调用。
        let _ = extra.read_to_end(&mut bytes);
        assert!(bytes.is_empty());
        assert_eq!(host.entered.load(Ordering::SeqCst), 4);
        drop(bridge);
    });
    assert_eq!(host.active.load(Ordering::SeqCst), 0);
    assert!(
        !cancellation.is_cancelled(),
        "桥收尾不能修改外层调用的取消状态"
    );
}

#[test]
fn user_cancellation_reaches_native_workers() {
    let host = WaitingHost::new();
    let cancellation = CancellationToken::new();
    thread::scope(|scope| {
        let mut bridge = AsyncBrowserBridge::new().unwrap();
        let _client = connect(&bridge, 1, "browser.profiles", None);
        let deadline = Instant::now() + Duration::from_secs(3);
        poll_until(&mut bridge, scope, &host, &cancellation, deadline, || {
            host.entered.load(Ordering::SeqCst) == 1
        });
        cancellation.cancel();
        poll_until(&mut bridge, scope, &host, &cancellation, deadline, || {
            host.active.load(Ordering::SeqCst) == 0
        });
    });
    assert_eq!(host.active.load(Ordering::SeqCst), 0);
}

#[test]
fn disconnected_client_cancels_only_its_worker() {
    let host = WaitingHost::new();
    let cancellation = CancellationToken::new();
    thread::scope(|scope| {
        let mut bridge = AsyncBrowserBridge::new().unwrap();
        let client = connect(&bridge, 1, "browser.call", None);
        let deadline = Instant::now() + Duration::from_secs(3);
        poll_until(&mut bridge, scope, &host, &cancellation, deadline, || {
            host.entered.load(Ordering::SeqCst) == 1
        });
        drop(client);
        poll_until(&mut bridge, scope, &host, &cancellation, deadline, || {
            host.active.load(Ordering::SeqCst) == 0
        });
        assert!(!cancellation.is_cancelled());
    });
}

#[test]
fn deadline_cancels_worker_without_needing_another_parent_poll() {
    let host = WaitingHost::new();
    let cancellation = CancellationToken::new();
    thread::scope(|scope| {
        let mut bridge = AsyncBrowserBridge::new().unwrap();
        let _client = connect(&bridge, 1, "browser.call", None);
        let deadline = Instant::now() + Duration::from_millis(500);
        poll_until(&mut bridge, scope, &host, &cancellation, deadline, || {
            host.entered.load(Ordering::SeqCst) == 1
        });
        while host.active.load(Ordering::SeqCst) != 0 {
            assert!(Instant::now() < deadline + Duration::from_secs(1));
            thread::sleep(Duration::from_millis(2));
        }
        assert!(!cancellation.is_cancelled());
    });
}

#[test]
fn confirmed_native_error_keeps_the_browser_method_and_effect() {
    struct Host(AtomicUsize);
    impl NativeHost for Host {
        fn browser_async_available(&self) -> bool {
            true
        }
        fn call(&self, _: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            let mut error = ActionError::new("result_unknown", "浏览器连接在派发后关闭。");
            error.method = Some("tabs.update".into());
            error.effect_state = Some("unknown".into());
            Err(error)
        }
    }
    let host = Host(AtomicUsize::new(0));
    let cancellation = CancellationToken::new();
    thread::scope(|scope| {
        let mut bridge = AsyncBrowserBridge::new().unwrap();
        let client = connect(&bridge, 1, "browser.call", None);
        let deadline = Instant::now() + Duration::from_secs(3);
        poll_until(&mut bridge, scope, &host, &cancellation, deadline, || {
            host.0.load(Ordering::SeqCst) == 1
        });
        let reply = response(client);
        assert_eq!(reply["id"], 1);
        assert_eq!(reply["ok"], false);
        assert_eq!(reply["error"]["code"], "result_unknown");
        assert_eq!(reply["error"]["method"], "tabs.update");
        assert_eq!(reply["error"]["effect_state"], "unknown");
    });
}

#[test]
fn token_method_and_replay_checks_precede_host_dispatch() {
    struct Host(AtomicUsize);
    impl NativeHost for Host {
        fn browser_async_available(&self) -> bool {
            true
        }
        fn call(&self, _: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(json!("accepted"))
        }
    }
    let host = Host(AtomicUsize::new(0));
    let cancellation = CancellationToken::new();
    thread::scope(|scope| {
        let mut bridge = AsyncBrowserBridge::new().unwrap();
        let wrong_method = connect(&bridge, 1, "keyboard.text", None);
        let replay = connect(&bridge, 1, "browser.call", None);
        let wrong_token = connect(&bridge, 2, "browser.call", Some(&"0".repeat(64)));
        let accepted = connect(&bridge, 2, "browser.call", None);
        let deadline = Instant::now() + Duration::from_secs(3);
        poll_until(&mut bridge, scope, &host, &cancellation, deadline, || {
            host.0.load(Ordering::SeqCst) == 1
        });
        assert_eq!(
            response(accepted),
            json!({"id":2,"ok":true,"value":"accepted"})
        );
        for mut rejected in [wrong_method, replay, wrong_token] {
            let mut bytes = Vec::new();
            let _ = rejected.read_to_end(&mut bytes);
            assert!(bytes.is_empty());
        }
    });
    assert_eq!(host.0.load(Ordering::SeqCst), 1);
}
