//! 平台接线的合成样本测试；不调用异步键状态读取或安装Hook。
use super::*;
use omnitrigger_core::{
    Activation, KeyboardMatching, KeyboardPolicy, ModifierRequirement, ModifierSides,
    MouseModifierPolicy, MouseModifierSnapshot, WheelDirection, WheelPolicy,
};

fn sampled_rule(id: u64, required: ModifierRequirement, priority: u16) -> TriggerSpec {
    TriggerSpec {
        id,
        held: vec![],
        activation: Activation::Wheel(WheelDirection::Up),
        conditions: vec![],
        pointer_scope: Default::default(),
        process_scope: Default::default(),
        wheel: WheelPolicy::Packet,
        keyboard: KeyboardPolicy {
            matching: KeyboardMatching::MouseSampled {
                policy: MouseModifierPolicy {
                    required,
                    allow_other_modifiers: false,
                    allow_extra_passed_keys: true,
                    allow_extra_passed_mouse: false,
                },
                priority,
                pointer_scope_priority: true,
            },
            ..Default::default()
        },
    }
}

fn fixture(bindings: Vec<TriggerSpec>) -> ThreadState {
    let (tx, _rx) = mpsc::sync_channel(32);
    let mut state = ThreadState::new(tx);
    state
        .with_engine(|engine| engine.replace_bindings(bindings))
        .unwrap();
    assert!(state.mouse_stream_id.is_some());
    state
}

fn wheel_event() -> InputEvent {
    InputEvent {
        kind: InputKind::Wheel {
            axis: Axis::Vertical,
            delta: 120,
        },
        injected: false,
    }
}

fn control() -> ModifierSides {
    ModifierSides {
        left_control: true,
        ..Default::default()
    }
}

fn snapshot(
    state: &ThreadState,
    sampled_at_ms: u64,
    modifiers: ModifierSides,
) -> MouseModifierSnapshot {
    MouseModifierSnapshot {
        stream_id: state.mouse_stream_id.unwrap(),
        event_serial: state.mouse_event_serial + 1,
        epoch: state.engine.mouse_modifier_snapshot_epoch().unwrap(),
        sampled_at_ms,
        modifiers,
    }
}

fn enqueue(state: &mut ThreadState, sampled_at_ms: u64, modifiers: ModifierSides) {
    let sample = snapshot(state, sampled_at_ms, modifiers);
    state.mouse_modifier_test_samples.push_back(Some(sample));
}

#[test]
fn all_eight_sides_share_one_sample_and_compete_in_original_rank_order() {
    let mut state = fixture(vec![
        sampled_rule(1, ModifierRequirement::Shift, 1),
        sampled_rule(2, ModifierRequirement::Control, 2),
        sampled_rule(3, ModifierRequirement::Alt, 3),
        sampled_rule(4, ModifierRequirement::LeftWindows, 4),
        sampled_rule(5, ModifierRequirement::RightWindows, 5),
    ]);
    for (index, (sides, expected)) in [
        (
            ModifierSides {
                left_shift: true,
                ..Default::default()
            },
            1,
        ),
        (
            ModifierSides {
                right_shift: true,
                ..Default::default()
            },
            1,
        ),
        (
            ModifierSides {
                left_control: true,
                ..Default::default()
            },
            2,
        ),
        (
            ModifierSides {
                right_control: true,
                ..Default::default()
            },
            2,
        ),
        (
            ModifierSides {
                left_alt: true,
                ..Default::default()
            },
            3,
        ),
        (
            ModifierSides {
                right_alt: true,
                ..Default::default()
            },
            3,
        ),
        (
            ModifierSides {
                left_windows: true,
                ..Default::default()
            },
            4,
        ),
        (
            ModifierSides {
                right_windows: true,
                ..Default::default()
            },
            5,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let now = 100 + index as u64;
        enqueue(&mut state, now, sides);
        let (decision, observed_at) = state.process_engine_input(wheel_event(), now);
        assert_eq!(decision.triggered, [expected]);
        assert_eq!(observed_at, now);
        assert!(state.mouse_modifier_test_samples.is_empty());
    }
    let mut first = sampled_rule(10, ModifierRequirement::Shift, 9);
    let mut second = sampled_rule(11, ModifierRequirement::Control, 2);
    for binding in [&mut first, &mut second] {
        if let KeyboardMatching::MouseSampled { policy, .. } = &mut binding.keyboard.matching {
            policy.allow_other_modifiers = true;
        }
    }
    state
        .with_engine(|engine| engine.replace_bindings(vec![first, second]))
        .unwrap();
    enqueue(
        &mut state,
        200,
        ModifierSides {
            left_shift: true,
            right_shift: true,
            left_control: true,
            right_control: true,
            left_alt: true,
            right_alt: true,
            left_windows: true,
            right_windows: true,
        },
    );
    state.mouse_modifier_test_samples.push_back(None);
    assert_eq!(
        state.process_engine_input(wheel_event(), 200).0.triggered,
        [11]
    );
    assert_eq!(
        state.mouse_modifier_test_samples.len(),
        1,
        "每个事件只能消费一份样本，不能按规则重采"
    );
}

#[test]
fn age_sixteen_is_valid_and_missing_older_wrong_epoch_or_stream_rejects() {
    assert_eq!(MOUSE_SNAPSHOT_MAX_AGE_MS, 16);
    let mut state = fixture(vec![sampled_rule(1, ModifierRequirement::Control, 0)]);
    enqueue(&mut state, 84, control());
    assert_eq!(
        state.process_engine_input(wheel_event(), 100).0.triggered,
        [1]
    );
    enqueue(&mut state, 84, control());
    assert!(state
        .process_engine_input(wheel_event(), 101)
        .0
        .triggered
        .is_empty());
    state.mouse_modifier_test_samples.push_back(None);
    assert!(state
        .process_engine_input(wheel_event(), 102)
        .0
        .triggered
        .is_empty());
    let mut wrong_epoch = snapshot(&state, 103, control());
    wrong_epoch.epoch -= 1;
    state
        .mouse_modifier_test_samples
        .push_back(Some(wrong_epoch));
    assert!(state
        .process_engine_input(wheel_event(), 103)
        .0
        .triggered
        .is_empty());
    let mut wrong_stream = snapshot(&state, 104, control());
    wrong_stream.stream_id += 1;
    state
        .mouse_modifier_test_samples
        .push_back(Some(wrong_stream));
    assert!(state
        .process_engine_input(wheel_event(), 104)
        .0
        .triggered
        .is_empty());
    enqueue(&mut state, 105, control());
    assert_eq!(
        state.process_engine_input(wheel_event(), 105).0.triggered,
        [1]
    );
}

#[test]
fn injected_events_leave_test_sample_serial_and_engine_time_untouched() {
    let mut state = fixture(vec![sampled_rule(1, ModifierRequirement::Control, 0)]);
    enqueue(&mut state, 100, control());
    let serial = state.mouse_event_serial;
    assert_eq!(
        state
            .process_engine_input(
                InputEvent {
                    injected: true,
                    ..wheel_event()
                },
                10_000
            )
            .0,
        Decision::default()
    );
    assert_eq!(state.mouse_event_serial, serial);
    assert_eq!(state.mouse_modifier_test_samples.len(), 1);
    assert_eq!(
        state.process_engine_input(wheel_event(), 100).0.triggered,
        [1]
    );
}

#[test]
fn keyboard_down_repeat_and_release_pass_normally_without_consuming_mouse_sample() {
    let mut state = fixture(vec![sampled_rule(1, ModifierRequirement::Control, 0)]);
    enqueue(&mut state, 103, control());
    let key = Button::Key(65);
    for (now, kind) in [
        (100, InputKind::Down(key)),
        (101, InputKind::Down(key)),
        (102, InputKind::Up(key)),
    ] {
        assert_eq!(
            state
                .process_engine_input(
                    InputEvent {
                        kind,
                        injected: false
                    },
                    now
                )
                .0,
            Decision::default()
        );
    }
    assert_eq!(state.mouse_event_serial, 0);
    assert_eq!(state.mouse_modifier_test_samples.len(), 1);
    assert_eq!(
        state.process_engine_input(wheel_event(), 103).0.triggered,
        [1]
    );
}

#[test]
fn cancel_keeps_consumed_mouse_repeat_and_release_owned_and_never_reuses_old_epoch() {
    let button = Button::Mouse(MouseButton::X1);
    let mut binding = sampled_rule(1, ModifierRequirement::Control, 0);
    binding.activation = Activation::Press(button);
    binding.wheel = WheelPolicy::Notch;
    let mut state = fixture(vec![binding]);
    enqueue(&mut state, 100, control());
    let down = InputEvent {
        kind: InputKind::Down(button),
        injected: false,
    };
    assert_eq!(state.process_engine_input(down, 100).0.triggered, [1]);
    let mut stale = snapshot(&state, 103, control());
    state.with_engine(Engine::cancel);
    assert!(state.process_engine_input(down, 101).0.consume);
    assert!(
        state
            .process_engine_input(
                InputEvent {
                    kind: InputKind::Up(button),
                    injected: false
                },
                102
            )
            .0
            .consume
    );
    stale.event_serial = state.mouse_event_serial + 1;
    state.mouse_modifier_test_samples.push_back(Some(stale));
    assert!(state.process_engine_input(down, 103).0.triggered.is_empty());
    assert!(
        !state
            .process_engine_input(
                InputEvent {
                    kind: InputKind::Up(button),
                    injected: false
                },
                104
            )
            .0
            .consume
    );
    enqueue(&mut state, 105, control());
    assert_eq!(state.process_engine_input(down, 105).0.triggered, [1]);
}

#[test]
fn removing_restoring_and_pausing_sampled_bindings_revokes_old_samples() {
    let bindings = vec![sampled_rule(1, ModifierRequirement::Control, 0)];
    let mut state = fixture(bindings.clone());
    let stale = snapshot(&state, 100, control());
    state
        .with_engine(|engine| engine.replace_bindings(vec![]))
        .unwrap();
    state.mouse_modifier_test_samples.push_back(Some(stale));
    assert_eq!(
        state.process_engine_input(wheel_event(), 100).0,
        Decision::default()
    );
    assert_eq!(state.mouse_modifier_test_samples.len(), 1);
    state
        .with_engine(|engine| engine.replace_bindings(bindings))
        .unwrap();
    assert!(state
        .process_engine_input(wheel_event(), 101)
        .0
        .triggered
        .is_empty());
    state.with_engine(|engine| engine.set_enabled(false));
    enqueue(&mut state, 102, control());
    assert!(state
        .process_engine_input(wheel_event(), 102)
        .0
        .triggered
        .is_empty());
    state.with_engine(|engine| engine.set_enabled(true));
    enqueue(&mut state, 103, control());
    assert_eq!(
        state.process_engine_input(wheel_event(), 103).0.triggered,
        [1]
    );
}
