use super::*;

fn options() -> Options {
    Options::parse(json!({"message":"通知正文"})).unwrap()
}
fn stopped() -> AtomicBool {
    AtomicBool::new(false)
}
fn later() -> Instant {
    Instant::now() + Duration::from_secs(1)
}

fn assert_error(error: ActionError, code: &str, effect: &str) {
    assert_eq!(error.code, code);
    assert_eq!(error.method.as_deref(), Some("ui.notify"));
    assert_eq!(error.effect_state.as_deref(), Some(effect));
}

#[test]
fn notify_wire_keeps_text_and_accepts_exact_utf16_boundaries() {
    let parsed = options();
    assert_eq!(parsed.title, "OmniTrigger");
    assert_eq!(parsed.level, Level::Info);
    assert_eq!(parsed.placement, Placement::TopRight);
    assert_eq!(parsed.duration_ms, 3000);
    let message = format!("{}\r\n\t文字\u{2028}", "😀".repeat(500));
    let parsed = Options::parse(json!({"message":message,"title":"😀".repeat(64),"level":"error","placement":"center","durationMs":30000})).unwrap();
    assert_eq!(parsed.message, message);
    assert_eq!(parsed.title.encode_utf16().count(), 128);
    assert_eq!(parsed.level, Level::Error);
    assert_eq!(parsed.placement, Placement::Center);
    assert!(Options::parse(json!({"message":"😀".repeat(512)})).is_ok());
    assert!(Options::parse(json!({"message":"x".repeat(1024),"level":"warning","placement":"bottomRight","durationMs":1000})).is_ok());
    assert!(
        Options::parse(json!({"message":"<script>不是代码</script>","level":"success"})).is_ok()
    );
}

#[test]
fn notify_wire_rejects_null_unknown_fields_types_and_invalid_unicode() {
    for params in [
        json!(null),
        json!([]),
        json!({}),
        json!({"message":null}),
        json!({"message":true}),
        json!({"message":""}),
        json!({"message":" \r\n\t"}),
        json!({"message":"x".repeat(1025)}),
        json!({"message":"😀".repeat(513)}),
        json!({"message":"ok","title":null}),
        json!({"message":"ok","title":" "}),
        json!({"message":"ok","title":"x".repeat(129)}),
        json!({"message":"ok","title":"a\u{2028}b"}),
        json!({"message":"ok","title":"a\u{2029}b"}),
        json!({"message":"ok","title":"a\nb"}),
        json!({"message":"ok","level":null}),
        json!({"message":"ok","level":"Info"}),
        json!({"message":"ok","placement":null}),
        json!({"message":"ok","placement":"bottom_right"}),
        json!({"message":"ok","durationMs":null}),
        json!({"message":"ok","durationMs":"3000"}),
        json!({"message":"ok","durationMs":true}),
        json!({"message":"ok","durationMs":1.5}),
        json!({"message":"ok","durationMs":-1}),
        json!({"message":"ok","durationMs":999}),
        json!({"message":"ok","durationMs":30001}),
        json!({"message":"ok","unknown":true}),
        json!({"message":"ok","duration_ms":3000}),
    ] {
        assert_error(
            Options::parse(params).unwrap_err(),
            "invalid_input",
            "not_started",
        );
    }
    for control in (0..=0x9f)
        .filter_map(char::from_u32)
        .filter(|ch| ch.is_control())
    {
        if !matches!(control, '\r' | '\n' | '\t') {
            assert_error(
                Options::parse(json!({"message":format!("a{control}b")})).unwrap_err(),
                "invalid_input",
                "not_started",
            );
        }
        assert_error(
            Options::parse(json!({"message":"ok","title":format!("a{control}b")})).unwrap_err(),
            "invalid_input",
            "not_started",
        );
    }
    // JSON 在到达 Options 前就拒绝孤立代理项；Rust String 不可能包含这些单元。
    for source in [
        r#"{"message":"\ud800"}"#,
        r#"{"message":"\udc00"}"#,
        r#"{"message":"ok","title":"\ud800"}"#,
    ] {
        assert!(serde_json::from_str::<Value>(source).is_err());
    }
}

#[test]
fn cancellation_and_timeout_before_creation_withdraw_the_request_atomically() {
    for cancelled in [false, true] {
        let submission = Submission::new();
        let token = CancellationToken::new();
        if cancelled {
            token.cancel();
        }
        let deadline = if cancelled { later() } else { Instant::now() };
        assert_error(
            submission.wait(&token, &stopped(), deadline).unwrap_err(),
            if cancelled { "cancelled" } else { "timeout" },
            "not_started",
        );
        assert!(
            !submission.begin(&CancellationToken::new(), false, later()),
            "已撤回请求不能在调用返回后创建窗口"
        );
        assert!(matches!(*submission.lock(), Phase::Failed(_)));
    }
}

#[test]
fn creation_races_and_post_creation_failures_never_report_not_started() {
    for created in [false, true] {
        for cancelled in [false, true] {
            let submission = Submission::new();
            assert!(submission.begin(&CancellationToken::new(), false, later()));
            if created {
                submission.created();
            }
            let token = CancellationToken::new();
            if cancelled {
                token.cancel();
            }
            assert_error(
                submission
                    .wait(
                        &token,
                        &stopped(),
                        if cancelled { later() } else { Instant::now() },
                    )
                    .unwrap_err(),
                if cancelled { "cancelled" } else { "timeout" },
                "unknown",
            );
            // 超时/取消不能假装撤回已经开始的原生创建，也不能触发自动重试。
            submission.created();
            submission.submitted();
            assert!(matches!(*submission.lock(), Phase::Submitted));
        }
    }
    let failed = Submission::new();
    failed.fail("主显示器不可用");
    assert_error(
        failed
            .wait(&CancellationToken::new(), &stopped(), later())
            .unwrap_err(),
        "ui_unavailable",
        "not_started",
    );
    let failed = Submission::new();
    assert!(failed.begin(&CancellationToken::new(), false, later()));
    failed.created();
    failed.fail("原生显示失败");
    assert_error(
        failed
            .wait(&CancellationToken::new(), &stopped(), later())
            .unwrap_err(),
        "ui_unavailable",
        "unknown",
    );
}

#[test]
fn submitting_does_not_wait_for_duration_or_release_the_visible_slot_with_the_action() {
    let (sender, receiver) = mpsc::sync_channel(1);
    let busy = Arc::new(AtomicBool::new(false));
    let broker = Broker {
        sender,
        busy: busy.clone(),
        stopped: Arc::new(stopped()),
    };
    let worker_broker = broker.clone();
    let token = CancellationToken::new();
    let worker_token = token.clone();
    let worker =
        std::thread::spawn(move || submit(options(), &worker_token, later(), worker_broker));
    let request = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(busy.load(Ordering::Acquire));
    assert_error(
        submit(
            options(),
            &CancellationToken::new(),
            later(),
            broker.clone(),
        )
        .unwrap_err(),
        "ui_busy",
        "not_started",
    );
    assert!(request
        .submission
        .begin(&request.cancellation, false, request.deadline));
    request.submission.created();
    request.submission.submitted();
    assert_eq!(worker.join().unwrap().unwrap(), json!({"submitted":true}));
    token.cancel(); // 动作线程已结束/令牌后续变动均不影响应用持有的通知名额。
    assert!(busy.load(Ordering::Acquire));
    assert_error(
        Slot::acquire(busy.clone()).err().unwrap(),
        "ui_busy",
        "not_started",
    );
    drop(request); // 对应真实 UI 到期移除窗口后释放 Request。
    assert!(!busy.load(Ordering::Acquire));
    let next = Slot::acquire(busy.clone()).unwrap();
    assert!(busy.load(Ordering::Acquire));
    drop(next);
    assert!(!busy.load(Ordering::Acquire));
}

#[test]
fn app_shutdown_and_dropped_ui_requests_wake_workers_with_honest_effect_state() {
    let shutdown = AtomicBool::new(true);
    let before = Submission::new();
    assert_error(
        before
            .wait(&CancellationToken::new(), &shutdown, later())
            .unwrap_err(),
        "ui_unavailable",
        "not_started",
    );
    let after = Submission::new();
    assert!(after.begin(&CancellationToken::new(), false, later()));
    after.created();
    after.submitted();
    assert_error(
        after
            .wait(&CancellationToken::new(), &shutdown, later())
            .unwrap_err(),
        "ui_unavailable",
        "unknown",
    );
    let submission = Arc::new(Submission::new());
    let busy = Arc::new(AtomicBool::new(false));
    drop(Request {
        options: options(),
        submission: submission.clone(),
        cancellation: CancellationToken::new(),
        deadline: later(),
        _slot: Slot::acquire(busy.clone()).unwrap(),
    });
    assert_error(
        submission
            .wait(&CancellationToken::new(), &stopped(), later())
            .unwrap_err(),
        "ui_unavailable",
        "not_started",
    );
    assert!(!busy.load(Ordering::Acquire));
}

#[test]
fn disconnected_ui_and_pre_cancelled_calls_never_create_or_reserve_a_window() {
    for cancelled in [false, true] {
        let (sender, receiver) = mpsc::sync_channel(1);
        drop(receiver);
        let busy = Arc::new(AtomicBool::new(false));
        let broker = Broker {
            sender,
            busy: busy.clone(),
            stopped: Arc::new(stopped()),
        };
        let token = CancellationToken::new();
        if cancelled {
            token.cancel();
        }
        assert_error(
            submit(options(), &token, later(), broker).unwrap_err(),
            if cancelled {
                "cancelled"
            } else {
                "ui_unavailable"
            },
            "not_started",
        );
        assert!(!busy.load(Ordering::Acquire));
    }
}

#[test]
fn host_routes_notify_without_calling_or_recapturing_the_original_desktop_target() {
    use omnitrigger_actions::NativeHost;
    struct Desktop;
    impl NativeHost for Desktop {
        fn call(&self, _: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
            panic!("通知不能调用桌面输入或重新捕获目标")
        }
    }
    let host = crate::script_ui::Host::new(Desktop, None);
    assert_error(
        host.call_with_deadline(
            "ui.notify",
            json!({"message":null}),
            &CancellationToken::new(),
            later(),
        )
        .unwrap_err(),
        "invalid_input",
        "not_started",
    );
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert_error(
        host.call_with_deadline("ui.notify", json!({"message":"ok"}), &cancellation, later())
            .unwrap_err(),
        "cancelled",
        "not_started",
    );
    assert_error(
        host.call_with_deadline(
            "ui.notify",
            json!({"message":"ok"}),
            &CancellationToken::new(),
            Instant::now(),
        )
        .unwrap_err(),
        "timeout",
        "not_started",
    );
}
