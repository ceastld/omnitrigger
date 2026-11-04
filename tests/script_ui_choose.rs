use super::*;

fn options() -> Value {
    json!({"items":[{"id":"copy","label":"复制"},{"id":"paste","label":"粘贴","description":"插入文本"}]})
}

#[test]
fn wire_validation_is_strict_and_keeps_stable_ids() {
    let parsed = Options::parse(options()).unwrap();
    assert_eq!(parsed.timeout_ms, 30_000);
    assert_eq!(parsed.title, "选择一项");
    assert_eq!(parsed.items[1].id, "paste");
    for params in [
        json!(null),
        json!([]),
        json!({"items":[]}),
        json!({"items":[{"id":"a","label":"A"},{"id":"a","label":"B"}]}),
        json!({"items":[{"id":"a","label":"A","value":1}]}),
        json!({"items":[{"id":"a","label":"A","description":null}]}),
        json!({"items":[{"id":"a","label":"A\nB"}]}),
        json!({"items":[{"id":"a","label":"A\u{2028}B"}]}),
        json!({"items":[{"id":" ","label":"A"}]}),
        json!({"items":[{"id":"😀".repeat(65),"label":"A"}]}),
        json!({"items":[{"id":"a","label":"A"}],"timeoutMs":null}),
        json!({"items":[{"id":"a","label":"A"}],"timeoutMs":0}),
        json!({"items":[{"id":"a","label":"A"}],"timeoutMs":60001}),
        json!({"items":[{"id":"a","label":"A"}],"title":""}),
        json!({"items":[{"id":"a","label":"A"}],"extra":true}),
        json!({"items":(0..201).map(|i| json!({"id":i.to_string(),"label":"A"})).collect::<Vec<_>>()}),
        json!({"items":(0..100).map(|i| json!({"id":i.to_string(),"label":"中".repeat(200)})).collect::<Vec<_>>()}),
    ] {
        let error = Options::parse(params).err().expect("必须拒绝无效参数");
        assert_eq!(error.code, "invalid_input");
        assert_eq!(error.method.as_deref(), Some("ui.choose"));
        assert_eq!(error.effect_state.as_deref(), Some("not_started"));
    }
}

#[test]
fn filtering_returns_original_indices_and_searches_id_label_description() {
    let items = Options::parse(options()).unwrap().items;
    assert_eq!(filtered(&items, ""), vec![0, 1]);
    assert_eq!(filtered(&items, " PaStE "), vec![1]);
    assert_eq!(filtered(&items, "复制"), vec![0]);
    assert_eq!(filtered(&items, "插入"), vec![1]);
    assert!(filtered(&items, "missing").is_empty());
    assert_eq!(items[filtered(&items, "插入")[0]].id, "paste");
}

#[test]
fn choice_wait_distinguishes_dismissal_timeout_cancellation_deadline_and_host_exit() {
    let token = CancellationToken::new();
    let now = Instant::now();
    let later = now + Duration::from_secs(1);
    let (sender, receiver) = mpsc::sync_channel(1);
    sender
        .send(json!({"status":"selected","id":"paste"}))
        .unwrap();
    assert_eq!(
        wait_for_choice(receiver, &token, later, later).unwrap()["id"],
        "paste"
    );
    let (sender, receiver) = mpsc::sync_channel(1);
    sender
        .send(json!({"status":"cancelled","reason":"dismissed"}))
        .unwrap();
    assert_eq!(
        wait_for_choice(receiver, &token, later, later).unwrap()["reason"],
        "dismissed"
    );
    let (_sender, receiver) = mpsc::sync_channel(1);
    assert_eq!(
        wait_for_choice(receiver, &token, later, now).unwrap(),
        json!({"status":"cancelled","reason":"timeout"})
    );
    let (_sender, receiver) = mpsc::sync_channel(1);
    assert_eq!(
        wait_for_choice(receiver, &token, now, later)
            .unwrap_err()
            .code,
        "timeout"
    );
    let (sender, receiver) = mpsc::sync_channel(1);
    drop(sender);
    assert_eq!(
        wait_for_choice(receiver, &token, later, later)
            .unwrap_err()
            .code,
        "ui_unavailable"
    );
    let (sender, receiver) = mpsc::sync_channel(1);
    sender
        .send(json!({"status":"selected","id":"copy"}))
        .unwrap();
    token.cancel();
    assert_eq!(
        wait_for_choice(receiver, &token, later, later)
            .unwrap_err()
            .code,
        "cancelled"
    );
}

#[test]
fn action_wait_checks_cancellation_while_waiting_and_releases_window_lease() {
    let token = CancellationToken::new();
    let worker_token = token.clone();
    let (_sender, receiver) = mpsc::sync_channel(1);
    let later = Instant::now() + Duration::from_secs(2);
    let worker = std::thread::spawn(move || wait_for_choice(receiver, &worker_token, later, later));
    token.cancel();
    assert_eq!(worker.join().unwrap().unwrap_err().code, "cancelled");
    let abandoned = Arc::new(AtomicBool::new(false));
    let busy = Arc::new(AtomicBool::new(true));
    drop(Waiting {
        abandoned: abandoned.clone(),
        busy: busy.clone(),
    });
    assert!(abandoned.load(Ordering::Acquire));
    assert!(!busy.load(Ordering::Acquire));
}

#[test]
fn wrapper_delegates_existing_target_and_exact_deadline() {
    struct OriginalTarget(Instant);
    impl NativeHost for OriginalTarget {
        fn call(&self, _: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
            panic!("必须保留时限")
        }
        fn call_with_deadline(
            &self,
            method: &str,
            params: Value,
            _: &CancellationToken,
            deadline: Instant,
        ) -> Result<Value, ActionError> {
            assert_eq!(deadline, self.0);
            assert_eq!(method, "keyboard.text");
            assert_eq!(params, json!({"text":"原目标"}));
            Ok(json!({"submitted":true}))
        }
    }
    let deadline = Instant::now() + Duration::from_secs(1);
    let host = Host::new(OriginalTarget(deadline), None);
    assert_eq!(
        host.call_with_deadline(
            "keyboard.text",
            json!({"text":"原目标"}),
            &CancellationToken::new(),
            deadline
        )
        .unwrap(),
        json!({"submitted":true})
    );
}
