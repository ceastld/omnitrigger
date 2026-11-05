//! 显式修饰快照的公共事件序列；全部状态由夹具提供，不读取系统状态。
use omnitrigger_core::*;

fn policy(required: ModifierRequirement) -> MouseModifierPolicy {
    MouseModifierPolicy {
        required,
        allow_other_modifiers: required == ModifierRequirement::NoControlShiftAlt,
        allow_extra_passed_keys: true,
        allow_extra_passed_mouse: false,
    }
}

fn rule(
    id: u64,
    activation: Activation,
    required: ModifierRequirement,
    priority: u16,
) -> TriggerSpec {
    TriggerSpec {
        id,
        activation,
        held: vec![],
        conditions: vec![],
        pointer_scope: Default::default(),
        process_scope: Default::default(),
        wheel: if matches!(activation, Activation::Wheel(_)) {
            WheelPolicy::Packet
        } else {
            WheelPolicy::Notch
        },
        keyboard: KeyboardPolicy {
            matching: KeyboardMatching::MouseSampled {
                policy: policy(required),
                priority,
                pointer_scope_priority: true,
            },
            ..Default::default()
        },
    }
}

fn event(kind: InputKind) -> InputEvent {
    InputEvent {
        kind,
        injected: false,
    }
}
fn wheel() -> InputEvent {
    event(InputKind::Wheel {
        axis: Axis::Vertical,
        delta: 120,
    })
}
fn engine(bindings: Vec<TriggerSpec>) -> Engine {
    let mut engine = Engine::new(bindings).unwrap();
    engine.configure_mouse_modifier_snapshot(7, 5).unwrap();
    engine
}
fn input(engine: &Engine, serial: u64, at: u64, modifiers: ModifierSides) -> MouseModifierInput {
    let epoch = engine.mouse_modifier_snapshot_epoch().unwrap();
    MouseModifierInput {
        stream_id: 7,
        serial,
        epoch,
        snapshot: Some(MouseModifierSnapshot {
            stream_id: 7,
            event_serial: serial,
            epoch,
            sampled_at_ms: at,
            modifiers,
        }),
    }
}
fn ctrl() -> ModifierSides {
    ModifierSides {
        left_control: true,
        ..Default::default()
    }
}

#[test]
fn six_mouse_candidates_accept_matching_samples_and_leave_extra_key_repeats_and_releases_passed() {
    for (required, activation, kind) in [
        (
            ModifierRequirement::Control,
            Activation::Wheel(WheelDirection::Up),
            InputKind::Wheel {
                axis: Axis::Vertical,
                delta: 120,
            },
        ),
        (
            ModifierRequirement::Control,
            Activation::Wheel(WheelDirection::Down),
            InputKind::Wheel {
                axis: Axis::Vertical,
                delta: -120,
            },
        ),
        (
            ModifierRequirement::Control,
            Activation::Press(Button::Mouse(MouseButton::Middle)),
            InputKind::Down(Button::Mouse(MouseButton::Middle)),
        ),
        (
            ModifierRequirement::Control,
            Activation::Press(Button::Mouse(MouseButton::X1)),
            InputKind::Down(Button::Mouse(MouseButton::X1)),
        ),
        (
            ModifierRequirement::LeftAlt,
            Activation::Press(Button::Mouse(MouseButton::X2)),
            InputKind::Down(Button::Mouse(MouseButton::X2)),
        ),
        (
            ModifierRequirement::Control,
            Activation::Press(Button::Mouse(MouseButton::X2)),
            InputKind::Down(Button::Mouse(MouseButton::X2)),
        ),
    ] {
        let mut e = engine(vec![rule(1, activation, required, 0)]);
        let ordinary = Button::Key(65);
        assert_eq!(
            e.process_at(event(InputKind::Down(ordinary)), 1),
            Decision::default()
        );
        let modifiers = if required == ModifierRequirement::LeftAlt {
            ModifierSides {
                left_alt: true,
                ..Default::default()
            }
        } else {
            ctrl()
        };
        let sample = input(&e, 1, 10, modifiers);
        let hit = e.process_with_mouse_snapshot_at(event(kind), 10, sample);
        assert!(hit.consume);
        assert_eq!(hit.triggered, [1]);
        assert!(hit.replay.is_empty());
        assert_eq!(
            e.process_at(event(InputKind::Down(ordinary)), 11),
            Decision::default()
        );
        assert_eq!(
            e.process_at(event(InputKind::Up(ordinary)), 12),
            Decision::default()
        );
        if let InputKind::Down(button) = kind {
            assert!(e.process_at(event(InputKind::Up(button)), 13).consume);
        }
    }
}

#[test]
fn same_snapshot_separates_unmodified_control_and_left_alt_without_changing_priority() {
    let mut e = engine(vec![
        rule(
            1,
            Activation::Wheel(WheelDirection::Up),
            ModifierRequirement::NoControlShiftAlt,
            0,
        ),
        rule(
            2,
            Activation::Wheel(WheelDirection::Up),
            ModifierRequirement::Control,
            2,
        ),
        rule(
            3,
            Activation::Wheel(WheelDirection::Up),
            ModifierRequirement::LeftAlt,
            19,
        ),
    ]);
    for (serial, modifiers, expected) in [
        (1, ctrl(), 2),
        (2, ModifierSides::default(), 1),
        (
            3,
            ModifierSides {
                left_alt: true,
                ..Default::default()
            },
            3,
        ),
    ] {
        let sample = input(&e, serial, 10 + serial, modifiers);
        assert_eq!(
            e.process_with_mouse_snapshot_at(wheel(), 10 + serial, sample)
                .triggered,
            [expected]
        );
    }
}

#[test]
fn required_modifier_is_not_synthesized_into_core_pressed_and_legacy_rule_is_not_reinterpreted() {
    let mut legacy = rule(
        1,
        Activation::Wheel(WheelDirection::Up),
        ModifierRequirement::NoControlShiftAlt,
        0,
    );
    legacy.keyboard.matching = KeyboardMatching::MouseUnmodified {
        priority: 0,
        pointer_scope_priority: true,
    };
    assert!(matches!(
        Engine::new(vec![
            legacy.clone(),
            rule(
                2,
                Activation::Wheel(WheelDirection::Up),
                ModifierRequirement::Control,
                0
            )
        ]),
        Err(RegistrationError::DuplicateTrigger {
            first: 1,
            second: 2
        })
    ));
    let mut e = engine(vec![
        legacy,
        rule(
            2,
            Activation::Wheel(WheelDirection::Up),
            ModifierRequirement::Control,
            2,
        ),
    ]);
    let sample = input(&e, 1, 10, ctrl());
    // 样本有Ctrl而核心没有Down：保留旧模式语义，不能偷偷让它改读异步状态。
    assert_eq!(
        e.process_with_mouse_snapshot_at(wheel(), 10, sample)
            .triggered,
        [1]
    );
    assert_eq!(
        e.process_at(event(InputKind::Up(Button::Key(0xa2))), 11),
        Decision::default()
    );
}

#[test]
fn old_entry_missing_expired_and_mismatched_snapshots_cannot_reuse_a_previous_hit() {
    let mut e = engine(vec![rule(
        1,
        Activation::Wheel(WheelDirection::Up),
        ModifierRequirement::Control,
        0,
    )]);
    let first = input(&e, 1, 10, ctrl());
    assert_eq!(
        e.process_with_mouse_snapshot_at(wheel(), 10, first)
            .triggered,
        [1]
    );
    assert!(e.process_at(wheel(), 11).triggered.is_empty());
    let mut missing = input(&e, 2, 12, ctrl());
    missing.snapshot = None;
    assert!(
        e.process_with_mouse_snapshot_at(wheel(), 12, missing)
            .triggered
            .is_empty()
    );
    let old = input(&e, 3, 6, ctrl());
    assert!(
        e.process_with_mouse_snapshot_at(wheel(), 12, old)
            .triggered
            .is_empty()
    );
    let mut mismatch = input(&e, 4, 12, ctrl());
    mismatch.snapshot.as_mut().unwrap().event_serial = 5;
    assert!(
        e.process_with_mouse_snapshot_at(wheel(), 12, mismatch)
            .triggered
            .is_empty()
    );
    let next = input(&e, 5, 13, ctrl());
    assert_eq!(
        e.process_with_mouse_snapshot_at(wheel(), 13, next)
            .triggered,
        [1]
    );
}

#[test]
fn cancellation_and_reconfiguration_keep_release_ownership_and_revoke_old_context() {
    let button = Button::Mouse(MouseButton::X1);
    let mut e = engine(vec![rule(
        1,
        Activation::Press(button),
        ModifierRequirement::Control,
        0,
    )]);
    let first = input(&e, 1, 10, ctrl());
    assert_eq!(
        e.process_with_mouse_snapshot_at(event(InputKind::Down(button)), 10, first)
            .triggered,
        [1]
    );
    e.cancel();
    assert!(e.process_at(event(InputKind::Down(button)), 11).consume);
    assert!(e.process_at(event(InputKind::Up(button)), 12).consume);
    assert_eq!(
        e.configure_mouse_modifier_snapshot(7, 0).unwrap_err(),
        MouseSnapshotError::StaleStream
    );
    e.configure_mouse_modifier_snapshot(8, 0).unwrap();
    assert!(
        e.process_with_mouse_snapshot_at(event(InputKind::Down(button)), 13, first)
            .triggered
            .is_empty()
    );
    assert!(!e.process_at(event(InputKind::Up(button)), 14).consume);
}

#[test]
fn injected_candidate_never_advances_clock_or_snapshot_serial() {
    let mut e = engine(vec![rule(
        1,
        Activation::Wheel(WheelDirection::Up),
        ModifierRequirement::Control,
        0,
    )]);
    let sample = input(&e, 1, 10, ctrl());
    assert_eq!(
        e.process_with_mouse_snapshot_at(
            InputEvent {
                injected: true,
                ..wheel()
            },
            1000,
            sample
        ),
        Decision::default()
    );
    assert_eq!(
        e.process_with_mouse_snapshot_at(wheel(), 10, sample)
            .triggered,
        [1]
    );
}
