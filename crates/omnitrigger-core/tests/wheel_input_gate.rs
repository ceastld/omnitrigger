use omnitrigger_core::*;

const INACTIVE: WheelSessionSnapshot = WheelSessionSnapshot {
    captured_mouse_buttons: false,
    holding_move_key: false,
};
const BUTTON_SESSION: WheelSessionSnapshot = WheelSessionSnapshot {
    captured_mouse_buttons: true,
    holding_move_key: false,
};
const KEY_SESSION: WheelSessionSnapshot = WheelSessionSnapshot {
    captured_mouse_buttons: false,
    holding_move_key: true,
};

fn event(kind: InputKind) -> InputEvent {
    InputEvent {
        kind,
        injected: false,
    }
}

fn vertical(delta: i32) -> InputEvent {
    event(InputKind::Wheel {
        axis: Axis::Vertical,
        delta,
    })
}

fn gate() -> WheelInputNormalizer {
    WheelInputNormalizer::new(WheelInputPolicy::SmallVertical150ms)
}

fn binding(
    id: u64,
    direction: WheelDirection,
    held: Vec<Button>,
    policy: WheelPolicy,
) -> TriggerSpec {
    TriggerSpec {
        id,
        held,
        activation: Activation::Wheel(direction),
        wheel: policy,
        keyboard: Default::default(),
        conditions: vec![],
        process_scope: Default::default(),
        pointer_scope: Default::default(),
    }
}

fn plain_engine(policy: WheelPolicy) -> Engine {
    Engine::new(vec![
        binding(1, WheelDirection::Up, vec![], policy),
        binding(2, WheelDirection::Down, vec![], policy),
    ])
    .unwrap()
}

#[test]
fn default_normalizer_preserves_packet_and_notch_contracts() {
    let mut n = WheelInputNormalizer::default();
    let mut packet = plain_engine(WheelPolicy::Packet);
    for at in 0..4 {
        assert_eq!(
            n.process_at(&mut packet, vertical(1), at, at as u64, INACTIVE)
                .triggered,
            vec![1]
        );
    }
    let mut notch = plain_engine(WheelPolicy::Notch);
    assert!(
        n.process_at(&mut notch, vertical(60), 0, 0, INACTIVE)
            .triggered
            .is_empty()
    );
    assert_eq!(
        n.process_at(&mut notch, vertical(60), 1, 1, INACTIVE)
            .triggered,
        vec![1]
    );
}

#[test]
fn absolute_clock_initial_previous_is_zero_not_first_packet_wins() {
    let mut n = gate();
    assert_eq!(
        n.admit_at(vertical(1), 0, INACTIVE),
        WheelInputAdmission::PassThrough
    );
    assert_eq!(
        n.admit_at(vertical(1), 149, INACTIVE),
        WheelInputAdmission::PassThrough
    );
    assert_eq!(
        n.admit_at(vertical(1), 150, INACTIVE),
        WheelInputAdmission::Forward
    );
}

#[test]
fn small_gate_has_exact_150ms_boundary_and_dropped_packets_do_not_slide_it() {
    let mut n = gate();
    assert_eq!(
        n.admit_at(vertical(1), 1000, INACTIVE),
        WheelInputAdmission::Forward
    );
    for at in [1001, 1100, 1149] {
        assert_eq!(
            n.admit_at(vertical(1), at, INACTIVE),
            WheelInputAdmission::PassThrough
        );
    }
    assert_eq!(
        n.admit_at(vertical(1), 1150, INACTIVE),
        WheelInputAdmission::Forward
    );
}

#[test]
fn strict_delta_boundaries_and_extreme_values_never_overflow() {
    for delta in [-9, -1, 0, 1, 9] {
        let mut n = gate();
        assert_eq!(
            n.admit_at(vertical(delta), 1000, INACTIVE),
            WheelInputAdmission::Forward
        );
        assert_eq!(
            n.admit_at(vertical(delta), 1001, INACTIVE),
            WheelInputAdmission::PassThrough
        );
    }
    for delta in [i32::MIN, -120, -10, 10, 120, i32::MAX] {
        let mut n = gate();
        assert_eq!(
            n.admit_at(vertical(delta), 1000, INACTIVE),
            WheelInputAdmission::Forward
        );
        assert_eq!(
            n.admit_at(vertical(delta), 1001, INACTIVE),
            WheelInputAdmission::Forward
        );
    }
}

#[test]
fn directions_share_one_history_without_delta_accumulation() {
    let mut n = gate();
    let mut e = plain_engine(WheelPolicy::Packet);
    assert_eq!(
        n.process_at(&mut e, vertical(9), 1000, 0, INACTIVE)
            .triggered,
        vec![1]
    );
    assert_eq!(
        n.process_at(&mut e, vertical(-9), 1001, 1, INACTIVE),
        Decision::default()
    );
    assert_eq!(
        n.process_at(&mut e, vertical(-1), 1150, 150, INACTIVE)
            .triggered,
        vec![2]
    );
}

#[test]
fn horizontal_keyboard_and_large_vertical_events_do_not_advance_small_history() {
    let mut n = gate();
    n.admit_at(vertical(1), 1000, INACTIVE);
    for (at, kind) in [
        (
            1100,
            InputKind::Wheel {
                axis: Axis::Horizontal,
                delta: 1,
            },
        ),
        (1110, InputKind::Down(Button::Key(65))),
        (1120, InputKind::Up(Button::Key(65))),
        (
            1149,
            InputKind::Wheel {
                axis: Axis::Vertical,
                delta: 10,
            },
        ),
    ] {
        assert_eq!(
            n.admit_at(event(kind), at, INACTIVE),
            WheelInputAdmission::Forward
        );
    }
    assert_eq!(
        n.admit_at(vertical(1), 1150, INACTIVE),
        WheelInputAdmission::Forward
    );
}

#[test]
fn captured_mouse_and_holding_move_key_each_bypass_without_updating_history() {
    for session in [BUTTON_SESSION, KEY_SESSION] {
        let mut n = gate();
        n.admit_at(vertical(1), 1000, INACTIVE);
        assert_eq!(
            n.admit_at(vertical(1), 1100, session),
            WheelInputAdmission::Forward
        );
        assert_eq!(
            n.admit_at(vertical(-1), 1149, session),
            WheelInputAdmission::Forward
        );
        assert_eq!(
            n.admit_at(vertical(1), 1150, INACTIVE),
            WheelInputAdmission::Forward
        );
    }
}

#[test]
fn session_release_does_not_reset_gate_or_use_its_bypassed_events_as_history() {
    let mut n = gate();
    n.admit_at(vertical(1), 1000, INACTIVE);
    n.admit_at(
        event(InputKind::Down(Button::Mouse(MouseButton::X1))),
        1010,
        INACTIVE,
    );
    n.admit_at(vertical(1), 1020, BUTTON_SESSION);
    n.admit_at(
        event(InputKind::Up(Button::Mouse(MouseButton::X1))),
        1030,
        BUTTON_SESSION,
    );
    assert_eq!(
        n.admit_at(vertical(1), 1031, INACTIVE),
        WheelInputAdmission::PassThrough
    );
    assert_eq!(
        n.admit_at(vertical(1), 1150, INACTIVE),
        WheelInputAdmission::Forward
    );
}

#[test]
fn an_unmatched_packet_updates_history_before_pointer_region_and_rule_selection() {
    let mut row = binding(1, WheelDirection::Up, vec![], WheelPolicy::Packet);
    row.conditions = vec![TriggerCondition::PointerRegion(
        PointerRegion::ScreenBoundary {
            edge: ScreenEdge::Top,
        },
    )];
    let mut e = Engine::new(vec![row]).unwrap();
    e.set_screens(vec![ScreenRect {
        left: 0,
        top: 0,
        right: 1000,
        bottom: 800,
    }])
    .unwrap();
    e.pointer_moved((500, 400), 0);
    let mut n = gate();
    assert_eq!(
        n.process_at(&mut e, vertical(1), 1000, 0, INACTIVE),
        Decision::default()
    );
    e.pointer_moved((500, 0), 1);
    assert_eq!(
        n.process_at(&mut e, vertical(1), 1010, 10, INACTIVE),
        Decision::default()
    );
    assert_eq!(
        n.process_at(&mut e, vertical(1), 1150, 150, INACTIVE)
            .triggered,
        vec![1]
    );
}

#[test]
fn paused_events_still_advance_upstream_history_and_resume_does_not_reset_it() {
    let mut n = gate();
    let mut e = plain_engine(WheelPolicy::Packet);
    e.set_enabled(false);
    assert_eq!(
        n.process_at(&mut e, vertical(1), 1000, 0, INACTIVE),
        Decision::default()
    );
    e.set_enabled(true);
    assert_eq!(
        n.process_at(&mut e, vertical(1), 1010, 10, INACTIVE),
        Decision::default()
    );
    assert_eq!(
        n.process_at(&mut e, vertical(1), 1150, 150, INACTIVE)
            .triggered,
        vec![1]
    );
}

#[test]
fn injected_packets_update_opt_in_gate_before_engine_ignores_them() {
    let mut n = gate();
    let mut e = plain_engine(WheelPolicy::Packet);
    let mut injected = vertical(1);
    injected.injected = true;
    assert_eq!(
        n.process_at(&mut e, injected, 1000, 0, INACTIVE),
        Decision::default()
    );
    assert_eq!(
        n.process_at(&mut e, vertical(1), 1010, 10, INACTIVE),
        Decision::default()
    );
    assert_eq!(
        n.process_at(&mut e, vertical(1), 1150, 150, INACTIVE)
            .triggered,
        vec![1]
    );
}

#[test]
fn cancellation_foreground_and_binding_replacement_do_not_reset_adapter_history() {
    let mut n = gate();
    let mut e = plain_engine(WheelPolicy::Packet);
    n.process_at(&mut e, vertical(1), 1000, 0, INACTIVE);
    e.cancel();
    e.set_foreground_process(Some("editor.exe")).unwrap();
    e.set_screens(vec![ScreenRect {
        left: 0,
        top: 0,
        right: 100,
        bottom: 100,
    }])
    .unwrap();
    e.replace_bindings(vec![binding(
        1,
        WheelDirection::Up,
        vec![],
        WheelPolicy::Packet,
    )])
    .unwrap();
    assert_eq!(
        n.process_at(&mut e, vertical(1), 1149, 149, INACTIVE),
        Decision::default()
    );
    assert_eq!(
        n.process_at(&mut e, vertical(1), 1150, 150, INACTIVE)
            .triggered,
        vec![1]
    );
}

#[test]
fn source_clock_rollback_blocks_until_old_timestamp_plus_interval() {
    let mut n = gate();
    n.admit_at(vertical(1), 1000, INACTIVE);
    for at in [999, -1, i64::MIN, 1149] {
        assert_eq!(
            n.admit_at(vertical(1), at, INACTIVE),
            WheelInputAdmission::PassThrough
        );
    }
    assert_eq!(
        n.admit_at(vertical(1), 1150, INACTIVE),
        WheelInputAdmission::Forward
    );
    assert_eq!(
        n.admit_at(vertical(1), i64::MAX, INACTIVE),
        WheelInputAdmission::Forward
    );
    assert_eq!(
        n.admit_at(vertical(1), i64::MIN, INACTIVE),
        WheelInputAdmission::PassThrough
    );
}

#[test]
fn explicit_policy_changes_preserve_history_but_unfiltered_is_default() {
    let mut n = gate();
    n.admit_at(vertical(1), 1000, INACTIVE);
    n.set_policy(WheelInputPolicy::Unfiltered);
    assert_eq!(
        n.admit_at(vertical(1), 1010, INACTIVE),
        WheelInputAdmission::Forward
    );
    n.set_policy(WheelInputPolicy::SmallVertical150ms);
    assert_eq!(
        n.admit_at(vertical(1), 1020, INACTIVE),
        WheelInputAdmission::PassThrough
    );
}

#[test]
fn recording_receives_raw_event_and_preserves_trigger_spec_without_gate_policy() {
    let mut n = gate();
    n.admit_at(vertical(1), 1000, INACTIVE);
    let raw = vertical(1);
    let mut recorder = Recorder::new();
    let recorded = recorder.process(raw).unwrap();
    assert_eq!(recorded.activation, Activation::Wheel(WheelDirection::Up));
    assert_eq!(recorded.wheel, WheelPolicy::Notch);
    assert_eq!(recorded.keyboard, KeyboardPolicy::default());
    assert_eq!(
        n.admit_at(raw, 1010, INACTIVE),
        WheelInputAdmission::PassThrough
    );
}

#[test]
fn pass_through_packet_does_not_cancel_or_replay_a_pending_owned_press() {
    let side = Button::Mouse(MouseButton::X1);
    let mut e = Engine::new(vec![binding(
        1,
        WheelDirection::Up,
        vec![side],
        WheelPolicy::Packet,
    )])
    .unwrap();
    let mut n = gate();
    n.admit_at(vertical(1), 1000, INACTIVE);
    assert!(e.process_at(event(InputKind::Down(side)), 0).consume);
    // 显式inactive用来验证PassThrough本身不进入Engine；不冒充真实侧键会话快照。
    assert_eq!(
        n.process_at(&mut e, vertical(1), 1010, 10, INACTIVE),
        Decision::default()
    );
    let up = e.process_at(event(InputKind::Up(side)), 11);
    assert_eq!(up.replay, vec![InputKind::Down(side), InputKind::Up(side)]);
}

#[test]
fn explicit_active_prefix_session_keeps_all_small_packet_actions_and_owns_release() {
    let side = Button::Mouse(MouseButton::X1);
    let mut e = Engine::new(vec![binding(
        1,
        WheelDirection::Up,
        vec![side],
        WheelPolicy::Packet,
    )])
    .unwrap();
    let mut n = gate();
    assert!(e.process_at(event(InputKind::Down(side)), 0).consume);
    for at in [1000, 1001, 1002] {
        assert_eq!(
            n.process_at(&mut e, vertical(1), at, (at - 1000) as u64, BUTTON_SESSION)
                .triggered,
            vec![1]
        );
    }
    let up = e.process_at(event(InputKind::Up(side)), 3);
    assert!(up.consume && up.replay.is_empty());
}

#[test]
fn filtered_packet_does_not_advance_or_cancel_a_keyboard_hold_timer() {
    let key = Button::Key(65);
    let mut hold = binding(3, WheelDirection::Up, vec![], WheelPolicy::Notch);
    hold.activation = Activation::Hold {
        button: key,
        duration_ms: 100,
    };
    let mut e = Engine::new(vec![hold]).unwrap();
    let mut n = gate();
    n.admit_at(vertical(1), 1000, INACTIVE);
    assert!(e.process_at(event(InputKind::Down(key)), 0).consume);
    assert_eq!(
        n.process_at(&mut e, vertical(1), 1010, 100, INACTIVE),
        Decision::default()
    );
    assert_eq!(e.advance_time(100).triggered, vec![3]);
}

#[test]
fn recording_and_runtime_admission_are_independent_during_a_mouse_session() {
    let side = Button::Mouse(MouseButton::X1);
    let mut recorder = Recorder::new();
    assert!(recorder.process(event(InputKind::Down(side))).is_none());
    let recorded = recorder.process(vertical(1)).unwrap();
    assert_eq!(recorded.held, vec![side]);
    assert_eq!(recorded.activation, Activation::Wheel(WheelDirection::Up));
    let mut n = gate();
    assert_eq!(
        n.admit_at(vertical(1), 1, BUTTON_SESSION),
        WheelInputAdmission::Forward
    );
    assert_eq!(
        n.admit_at(vertical(1), 2, BUTTON_SESSION),
        WheelInputAdmission::Forward
    );
}

#[test]
fn captured_keyboard_leader_alone_does_not_bypass_the_vertical_gate() {
    let caps = Button::Key(20);
    let owner = TriggerSpec {
        id: 3,
        held: vec![caps],
        activation: Activation::Press(Button::Key(72)),
        keyboard: KeyboardPolicy {
            dedicated_prefix: Some(caps),
            matching: KeyboardMatching::Captured { priority: 0 },
            ..Default::default()
        },
        wheel: Default::default(),
        conditions: vec![],
        process_scope: Default::default(),
        pointer_scope: Default::default(),
    };
    let mut watcher = binding(2, WheelDirection::Up, vec![caps], WheelPolicy::Packet);
    watcher.keyboard.matching = KeyboardMatching::MouseCapturedLeader {
        priority: 1,
        pointer_scope_priority: false,
    };
    let mut e = Engine::new(vec![owner, watcher]).unwrap();
    let mut n = gate();
    assert!(e.process_at(event(InputKind::Down(caps)), 0).consume);
    assert_eq!(
        n.process_at(&mut e, vertical(1), 1000, 1, INACTIVE)
            .triggered,
        vec![2]
    );
    assert_eq!(
        n.process_at(&mut e, vertical(1), 1010, 2, INACTIVE),
        Decision::default()
    );
    assert!(e.process_at(event(InputKind::Up(caps)), 3).consume);
}
