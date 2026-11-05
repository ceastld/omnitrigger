const ON: DispatchEnablement = DispatchEnablement::Enabled;
#[derive(Default)]
struct FakePort {
    next: u64,
    attempts: Vec<(u64, u64, Vec<u16>, u32)>,
    cancelled: Vec<u64>,
    reject: bool,
    zero: bool,
}
impl Port for FakePort {
    fn submit(&mut self, binding: u64, work: Work) -> Result<u64, String> {
        assert!(work.prepared.is_none());
        let (original, keys, delay) = work.fixture.unwrap();
        self.attempts.push((binding, original, keys, delay));
        if self.reject {
            self.reject = false;
            return Err("受控入队前拒绝".into());
        }
        if self.zero {
            return Ok(0);
        }
        self.next += 1;
        Ok(self.next)
    }
    fn cancel(&mut self, invocation: u64) -> Result<ShortcutCancelState, String> {
        self.cancelled.push(invocation);
        Ok(ShortcutCancelState::Requested)
    }
}
fn queue(capacity: usize, budget: u32) -> MouseShortcutDispatch {
    MouseShortcutDispatch::new(Plan {
        bindings: vec![201, 202, 203],
        policy: BusyMousePolicy::Delay {
            capacity,
            max_wait_ms: budget,
        },
    })
    .unwrap()
}
fn work(original: u64, keys: &[u16], delay: u32) -> Work {
    Work {
        prepared: None,
        fixture: Some((original, keys.to_vec(), delay)),
    }
}
fn event(lease: Lease, outcome: ExecutionOutcome) -> ExecutionEvent {
    ExecutionEvent {
        id: lease.binding,
        invocation_id: lease.invocation,
        outcome,
    }
}
#[test]
fn fifo_keeps_each_original_payload_and_only_exact_shortcut_terminal_advances() {
    let mut q = queue(2, 20);
    let mut p = FakePort::default();
    q.offer_port(&mut p, 201, work(11, &[163, 161, 9], 10), 0, ON)
        .unwrap();
    let first = q.pump_port(&mut p, 0, ON).unwrap().submitted.unwrap();
    q.offer_port(&mut p, 202, work(22, &[163, 9], 0), 1, ON)
        .unwrap();
    q.offer_port(&mut p, 203, work(33, &[163, 87], 10), 2, ON)
        .unwrap();
    let terminal = event(first, ExecutionOutcome::ShortcutSent);
    assert!(!q
        .observe_port(false, &terminal, &mut p, 3, ON)
        .unwrap()
        .changed());
    let mut wrong = event(first, ExecutionOutcome::ShortcutSent);
    wrong.id = 202;
    assert!(!q
        .observe_port(true, &wrong, &mut p, 3, ON)
        .unwrap()
        .changed());
    assert!(!q
        .observe_port(
            true,
            &event(
                first,
                ExecutionOutcome::Unknown {
                    message: "未知".into()
                }
            ),
            &mut p,
            3,
            ON
        )
        .unwrap()
        .changed());
    assert_eq!(q.lease, Some(first));
    assert_eq!(p.attempts.len(), 1);
    q.observe_port(true, &terminal, &mut p, 3, ON).unwrap();
    let second = q.pump_port(&mut p, 3, ON).unwrap().submitted.unwrap();
    q.observe_port(true, &terminal, &mut p, 4, ON).unwrap();
    assert_eq!(q.lease, Some(second));
    q.observe_port(
        true,
        &event(second, ExecutionOutcome::ShortcutSent),
        &mut p,
        4,
        ON,
    )
    .unwrap();
    let third = q.pump_port(&mut p, 4, ON).unwrap().submitted.unwrap();
    q.observe_port(
        true,
        &event(third, ExecutionOutcome::ShortcutSent),
        &mut p,
        5,
        ON,
    )
    .unwrap();
    assert!(q.queue.active().is_none());
    assert_eq!(
        p.attempts,
        vec![
            (201, 11, vec![163, 161, 9], 10),
            (202, 22, vec![163, 9], 0),
            (203, 33, vec![163, 87], 10)
        ]
    );
}
#[test]
fn full_result_is_structured_and_cancel_before_submit_never_enters_port() {
    let mut q = queue(1, 10);
    let mut p = FakePort::default();
    q.offer_port(&mut p, 201, work(11, &[163, 161, 9], 0), 0, ON)
        .unwrap();
    q.offer_port(&mut p, 202, work(22, &[163, 9], 0), 0, ON)
        .unwrap();
    let full = q
        .offer_port(&mut p, 203, work(33, &[163, 87], 0), 0, ON)
        .unwrap();
    assert!(matches!(
        full.admission,
        Some(MouseDispatchAdmission::Dropped(
            MouseDispatchDropReason::QueueFull
        ))
    ));
    let stopped = q.cancel_all_port(&mut p, 1, ON).unwrap();
    assert_eq!(stopped.dropped.len(), 2);
    assert!(q.queue.active().is_none());
    q.pump_port(&mut p, 2, ON).unwrap();
    assert!(p.attempts.is_empty());
    assert!(p.cancelled.is_empty());
    q.offer_port(&mut p, 203, work(44, &[163, 87], 0), 2, ON)
        .unwrap();
    assert_eq!(
        q.pump_port(&mut p, 2, ON)
            .unwrap()
            .submitted
            .unwrap()
            .binding,
        203
    );
    q.offer_port(&mut p, 202, work(22, &[163, 9], 0), 2, ON)
        .unwrap();
    let replacement = q
        .offer_port(&mut p, 201, work(55, &[163, 161, 9], 0), 12, ON)
        .unwrap();
    assert!(matches!(
        replacement.admission,
        Some(MouseDispatchAdmission::Delayed { .. })
    ));
    assert_eq!(replacement.dropped.len(), 1);
    assert_eq!(
        replacement.dropped[0].reason,
        MouseDispatchDropReason::Expired
    );
}
#[test]
fn stopping_one_rule_drops_its_waiting_calls_and_cancels_only_its_exact_invocation() {
    let mut q = queue(3, 20);
    let mut p = FakePort::default();
    q.offer_port(&mut p, 201, work(11, &[163, 161, 9], 0), 0, ON)
        .unwrap();
    let first = q.pump_port(&mut p, 0, ON).unwrap().submitted.unwrap();
    q.offer_port(&mut p, 201, work(12, &[163, 161, 9], 0), 1, ON)
        .unwrap();
    q.offer_port(&mut p, 202, work(22, &[163, 9], 0), 1, ON)
        .unwrap();
    let stopped = q.cancel_rule_port(201, &mut p, 2, ON).unwrap();
    assert_eq!(stopped.dropped.len(), 1);
    assert_eq!(q.waiting.len(), 1);
    assert_eq!(p.cancelled, vec![first.invocation]);
    assert_eq!(q.lease, Some(first));
    q.pump_port(&mut p, 3, ON).unwrap();
    assert_eq!(p.attempts.len(), 1);
    q.observe_port(
        true,
        &event(
            first,
            ExecutionOutcome::Failed {
                message: "取消返回".into(),
            },
        ),
        &mut p,
        4,
        ON,
    )
    .unwrap();
    let next = q.pump_port(&mut p, 4, ON).unwrap().submitted.unwrap();
    assert_eq!(next.binding, 202);
    assert_eq!(p.attempts.len(), 2);
    assert_eq!(p.attempts[1].1, 22);
}
#[test]
fn deadline_applies_to_waiting_and_ready_before_submit_without_retarget_or_retry() {
    for ready_before_expiry in [false, true] {
        let mut q = queue(1, 5);
        let mut p = FakePort::default();
        q.offer_port(&mut p, 201, work(11, &[163, 161, 9], 0), 0, ON)
            .unwrap();
        let first = q.pump_port(&mut p, 0, ON).unwrap().submitted.unwrap();
        q.offer_port(&mut p, 202, work(22, &[163, 9], 10), 0, ON)
            .unwrap();
        if ready_before_expiry {
            q.observe_port(
                true,
                &event(first, ExecutionOutcome::ShortcutSent),
                &mut p,
                4,
                ON,
            )
            .unwrap();
        }
        let expired = q.pump_port(&mut p, 5, ON).unwrap();
        assert_eq!(expired.dropped.len(), 1);
        assert_eq!(expired.dropped[0].reason, MouseDispatchDropReason::Expired);
        assert_eq!(p.attempts.len(), 1);
        assert!(q.waiting.is_empty());
    }
}
#[test]
fn submission_rejection_does_not_retry_and_zero_receipt_keeps_ambiguous_occupancy() {
    let mut q = queue(1, 10);
    let mut p = FakePort {
        reject: true,
        ..Default::default()
    };
    q.offer_port(&mut p, 201, work(11, &[163, 161, 9], 0), 0, ON)
        .unwrap();
    q.offer_port(&mut p, 202, work(22, &[163, 9], 0), 0, ON)
        .unwrap();
    assert!(q
        .pump_port(&mut p, 0, ON)
        .unwrap()
        .rejected_before_start
        .is_some());
    assert_eq!(p.attempts.len(), 1);
    assert_eq!(
        q.pump_port(&mut p, 1, ON)
            .unwrap()
            .submitted
            .unwrap()
            .binding,
        202
    );
    assert_eq!(
        p.attempts.iter().map(|entry| entry.0).collect::<Vec<_>>(),
        vec![201, 202]
    );
    let mut q = queue(1, 10);
    let mut p = FakePort {
        zero: true,
        ..Default::default()
    };
    q.offer_port(&mut p, 201, work(11, &[163, 161, 9], 0), 0, ON)
        .unwrap();
    assert!(matches!(
        q.pump_port(&mut p, 0, ON),
        Err(Error::InvalidReceipt)
    ));
    q.cancel_all_port(&mut p, 1, ON).unwrap();
    assert!(q.queue.active().is_some());
    assert!(p.cancelled.is_empty());
    assert_eq!(p.attempts.len(), 1);
}
#[test]
fn unknown_enablement_drops_waiting_but_never_releases_active_on_cancel_request() {
    let mut q = queue(1, 10);
    let mut p = FakePort::default();
    q.offer_port(&mut p, 201, work(11, &[163, 161, 9], 0), 0, ON)
        .unwrap();
    let first = q.pump_port(&mut p, 0, ON).unwrap().submitted.unwrap();
    q.offer_port(&mut p, 202, work(22, &[163, 9], 0), 0, ON)
        .unwrap();
    let disabled = q.pump_port(&mut p, 1, DispatchEnablement::Unknown).unwrap();
    assert_eq!(disabled.dropped.len(), 1);
    assert_eq!(q.lease, Some(first));
    assert_eq!(p.cancelled, vec![first.invocation]);
    q.observe_port(
        true,
        &event(first, ExecutionOutcome::ShortcutSent),
        &mut p,
        2,
        DispatchEnablement::Unknown,
    )
    .unwrap();
    assert!(q.queue.active().is_none());
    assert_eq!(p.attempts.len(), 1);
}

fn argv(values: &[&str]) -> Vec<String> {
    std::iter::once("omnitrigger")
        .chain(values.iter().copied())
        .map(str::to_owned)
        .collect()
}
#[test]
fn absent_queue_flag_preserves_other_modes_without_reading_any_file() {
    assert_eq!(
        parse_plan_path(&argv(&["--text-history", "--config", "missing.json"])).unwrap(),
        None
    );
    assert_eq!(parse_plan_path(&argv(&["--check-config"])).unwrap(), None);
}
#[test]
fn explicit_queue_cli_rejects_invalid_combinations_before_file_reads() {
    for values in [
        vec![PLAN_FLAG],
        vec![PLAN_FLAG, ""],
        vec![PLAN_FLAG, "--config"],
        vec!["--mouse-shortcut-queue=missing.json"],
        vec![PLAN_FLAG, "missing.json", PLAN_FLAG, "other.json"],
        vec![PLAN_FLAG, "missing.json", "--text-history"],
        vec!["--text-history", PLAN_FLAG, "missing.json"],
        vec![PLAN_FLAG, "missing.json", "--smoke-test"],
        vec![PLAN_FLAG, "missing.json", "--action-run"],
        vec![
            PLAN_FLAG,
            "missing.json",
            "--wheel-input-policy",
            "unfiltered",
        ],
        vec![PLAN_FLAG, "missing.json", "--check-config", "--panel"],
        vec![PLAN_FLAG, "missing.json", "--config", "a", "--config", "b"],
        vec![
            PLAN_FLAG,
            "missing.json",
            "--check-config",
            "--check-config",
        ],
    ] {
        assert_eq!(
            parse_plan_path(&argv(&values)).unwrap_err().code,
            "invalid_mouse_queue_arguments"
        );
    }
    for tail in [
        vec![],
        vec!["--panel"],
        vec!["--check-config"],
        vec!["--config", "real.json", "--check-config"],
    ] {
        let mut values = vec![PLAN_FLAG, "chosen.json"];
        values.extend(tail);
        assert_eq!(
            parse_plan_path(&argv(&values)).unwrap(),
            Some("chosen.json".into())
        );
    }
}
#[test]
fn strict_plan_dto_accepts_only_explicit_drop_or_bounded_delay() {
    let drop = decode_plan(br#"{"bindings":[71],"policy":{"kind":"drop"}}"#).unwrap();
    assert_eq!(drop.bindings, vec![71]);
    assert_eq!(drop.policy, BusyMousePolicy::Drop);
    let delay=decode_plan(br#"{"bindings":[71,72],"policy":{"kind":"delay","capacity":64,"max_submit_wait_ms":60000}}"#).unwrap();
    assert_eq!(
        delay.policy,
        BusyMousePolicy::Delay {
            capacity: 64,
            max_wait_ms: 60000
        }
    );
    for body in [
        r#"{"bindings":[71],"policy":{"kind":"drop","capacity":1}}"#,
        r#"{"bindings":[71],"policy":{"kind":"drop"},"enabled":true}"#,
        r#"{"bindings":[71],"bindings":[72],"policy":{"kind":"drop"}}"#,
        r#"{"bindings":[71],"policy":{"kind":"delay","capacity":1}}"#,
        r#"{"bindings":[71],"policy":{"kind":"delay","capacity":1.5,"max_submit_wait_ms":1}}"#,
        r#"{"bindings":[71],"policy":{"kind":"delay","capacity":1,"max_submit_wait_ms":null}}"#,
        r#"{"bindings":[71],"policy":{"kind":"delay","capacity":-1,"max_submit_wait_ms":1}}"#,
        r#"{"bindings":[71],"policy":{"kind":"source_default"}}"#,
        r#"{"bindings":[71],"policy":{"kind":"drop","kind":"drop"}}"#,
    ] {
        assert!(decode_plan(body.as_bytes()).is_err(), "{body}");
    }
}
#[test]
fn plan_rejects_duplicate_zero_large_binding_sets_and_out_of_range_policy() {
    for ids in [vec![], vec![0], vec![71, 71], (1..=65).collect()] {
        let value = serde_json::json!({"bindings":ids,"policy":{"kind":"drop"}});
        assert_eq!(
            decode_plan(&serde_json::to_vec(&value).unwrap())
                .err()
                .unwrap()
                .code,
            "invalid_mouse_queue_bindings"
        );
    }
    for (capacity, budget) in [(0, 1), (65, 1), (1, 0), (1, 60001)] {
        let value = serde_json::json!({"bindings":[71],"policy":{"kind":"delay","capacity":capacity,"max_submit_wait_ms":budget}});
        assert_eq!(
            decode_plan(&serde_json::to_vec(&value).unwrap())
                .err()
                .unwrap()
                .code,
            "invalid_mouse_queue_policy"
        );
    }
}
#[test]
fn actual_binding_validation_rejects_missing_non_shortcut_and_keyboard_rules() {
    use crate::settings::ActionSpec;
    use omnitrigger_core::{Activation, Button, MouseButton, WheelDirection};
    let mut binding = crate::settings::defaults().remove(0);
    binding.spec.id = 71;
    binding.enabled = false;
    binding.action = ActionSpec::SendShortcut(crate::shortcut::ShortcutAction {
        keys: vec![0xA2, 0x09],
        timing: Default::default(),
    });
    let plan = decode_plan(br#"{"bindings":[71],"policy":{"kind":"drop"}}"#).unwrap();
    assert_eq!(
        validate_plan_bindings(&plan, &[]).unwrap_err().code,
        "mouse_queue_binding_missing"
    );
    for activation in [
        Activation::Press(Button::Mouse(MouseButton::X1)),
        Activation::Wheel(WheelDirection::Up),
    ] {
        binding.spec.activation = activation;
        let before = binding.clone();
        validate_plan_bindings(&plan, std::slice::from_ref(&binding)).unwrap();
        assert_eq!(binding, before);
        assert!(!binding.enabled);
    }
    binding.spec.activation = Activation::Press(Button::Key(0x41));
    assert_eq!(
        validate_plan_bindings(&plan, std::slice::from_ref(&binding))
            .unwrap_err()
            .code,
        "mouse_queue_binding_unsupported"
    );
    binding.spec.activation = Activation::Press(Button::Mouse(MouseButton::X1));
    binding.action = ActionSpec::Notification;
    assert_eq!(
        validate_plan_bindings(&plan, &[binding]).unwrap_err().code,
        "mouse_queue_binding_unsupported"
    );
}
#[test]
fn explicit_plan_file_errors_and_size_limits_are_never_ignored() {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let folder = std::env::temp_dir().join(format!(
        "omnitrigger-queue-plan-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir(&folder).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(folder.clone());
    let file = folder.join("plan.json");
    assert_eq!(
        read_plan(&file).err().unwrap().code,
        "mouse_queue_plan_read_failed"
    );
    assert!(read_plan(&folder).is_err());
    std::fs::write(&file, b"{").unwrap();
    assert_eq!(
        read_plan(&file).err().unwrap().code,
        "invalid_mouse_queue_plan"
    );
    std::fs::write(&file, vec![b' '; MAX_PLAN_BYTES as usize + 1]).unwrap();
    assert_eq!(
        read_plan(&file).err().unwrap().code,
        "invalid_mouse_queue_plan_file"
    );
    std::fs::write(&file, br#"{"bindings":[71],"policy":{"kind":"drop"}}"#).unwrap();
    assert_eq!(read_plan(&file).unwrap().bindings, vec![71]);
}
