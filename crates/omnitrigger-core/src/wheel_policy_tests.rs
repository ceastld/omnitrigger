use crate::*;

const SIDE: Button = Button::Mouse(MouseButton::X1);

fn binding(id: u64, direction: WheelDirection, wheel: WheelPolicy) -> TriggerSpec {
    TriggerSpec {
        pointer_scope: Default::default(),
        id,
        held: vec![SIDE],
        activation: Activation::Wheel(direction),
        wheel,
        keyboard: Default::default(),
        process_scope: Default::default(),
        conditions: vec![],
    }
}

fn event(kind: InputKind) -> InputEvent {
    InputEvent {
        kind,
        injected: false,
    }
}

fn scroll(engine: &mut Engine, axis: Axis, delta: i32) -> Decision {
    engine.process(event(InputKind::Wheel { axis, delta }))
}

#[test]
fn packet_wheel_fires_once_for_each_nonzero_event_on_all_axes() {
    for (axis, positive, negative) in [
        (Axis::Vertical, WheelDirection::Up, WheelDirection::Down),
        (
            Axis::Horizontal,
            WheelDirection::Right,
            WheelDirection::Left,
        ),
    ] {
        let mut engine = Engine::new(vec![
            binding(1, positive, WheelPolicy::Packet),
            binding(2, negative, WheelPolicy::Packet),
        ])
        .unwrap();
        assert!(engine.process(event(InputKind::Down(SIDE))).consume);
        for delta in [1, 15, 60, 120, 240, i32::MAX, -1, -15, -240, i32::MIN] {
            let decision = scroll(&mut engine, axis, delta);
            assert!(decision.consume);
            assert_eq!(decision.triggered, [if delta > 0 { 1 } else { 2 }]);
            assert!(decision.replay.is_empty());
        }
        assert_eq!(scroll(&mut engine, axis, 0), Decision::default());
        let up = engine.process(event(InputKind::Up(SIDE)));
        assert!(up.consume && up.replay.is_empty());
    }
}

#[test]
fn packet_wheel_commit_preserves_another_directions_notch_tail() {
    let mut packet = binding(1, WheelDirection::Up, WheelPolicy::Packet);
    let region = TriggerCondition::PointerRegion(PointerRegion::ScreenEdge {
        edge: ScreenEdge::Left,
        thickness_px: 1,
    });
    packet.conditions = vec![region];
    let mut notch = binding(2, WheelDirection::Down, WheelPolicy::Notch);
    notch.conditions = vec![region];
    let mut engine = Engine::new(vec![packet, notch]).unwrap();
    engine
        .set_screens(vec![ScreenRect {
            left: 0,
            top: 0,
            right: 100,
            bottom: 100,
        }])
        .unwrap();
    engine.pointer_moved((0, 50), 0);
    engine.process(event(InputKind::Down(SIDE)));
    assert!(
        scroll(&mut engine, Axis::Vertical, -60)
            .triggered
            .is_empty()
    );
    assert_eq!(scroll(&mut engine, Axis::Vertical, 1).triggered, [1]);
    let outside = engine.pointer_moved((1, 50), 1);
    assert_eq!(
        outside.replay,
        [InputKind::Wheel {
            axis: Axis::Vertical,
            delta: -60
        }]
    );
    assert!(!scroll(&mut engine, Axis::Vertical, 240).consume);
    assert!(engine.process(event(InputKind::Up(SIDE))).consume);
}

#[test]
fn packet_wheel_obeys_process_conditions_injection_and_enable_state() {
    let mut row = binding(1, WheelDirection::Up, WheelPolicy::Packet);
    row.process_scope.allow = vec!["editor".into()];
    let mut engine = Engine::new(vec![row]).unwrap();
    engine.set_foreground_process(Some("editor")).unwrap();
    engine.process(event(InputKind::Down(SIDE)));
    let ignored = engine.process(InputEvent {
        kind: InputKind::Wheel {
            axis: Axis::Vertical,
            delta: 15,
        },
        injected: true,
    });
    assert_eq!(ignored, Decision::default());
    assert_eq!(scroll(&mut engine, Axis::Vertical, 15).triggered, [1]);
    engine.set_foreground_process(Some("other")).unwrap();
    assert!(!scroll(&mut engine, Axis::Vertical, 240).consume);
    assert!(engine.process(event(InputKind::Up(SIDE))).consume);
    engine.set_foreground_process(Some("editor")).unwrap();
    engine.process(event(InputKind::Down(SIDE)));
    engine.set_enabled(false);
    assert_eq!(
        scroll(&mut engine, Axis::Vertical, 240),
        Decision::default()
    );
}

#[test]
fn packet_wheel_can_consume_a_small_event_in_a_retained_layer_without_tap_restore() {
    let caps = Button::Key(20);
    let mut row = binding(1, WheelDirection::Up, WheelPolicy::Packet);
    row.held = vec![caps];
    row.keyboard = KeyboardPolicy {
        matching: KeyboardMatching::Captured { priority: 1 },
        retained_prefix: Some(Box::new(RetainedPrefix {
            key: caps,
            capture_scope: Default::default(),
            quiet_ms: 50,
            fast_ms: 50,
            block_with_primary_mouse: true,
        })),
        restore_on_tap: true,
        tap_timeout_ms: Some(1500),
        ..Default::default()
    };
    let mut engine = Engine::new(vec![row]).unwrap();
    assert!(engine.process_at(event(InputKind::Down(caps)), 100).consume);
    let decision = engine.process_at(
        event(InputKind::Wheel {
            axis: Axis::Vertical,
            delta: 1,
        }),
        160,
    );
    assert_eq!(decision.triggered, [1]);
    let up = engine.process_at(event(InputKind::Up(caps)), 170);
    assert!(up.consume && up.replay.is_empty());
}

#[test]
fn packet_wheel_is_not_a_separate_conflict_domain_and_invalid_replacement_is_atomic() {
    let packet = binding(1, WheelDirection::Up, WheelPolicy::Packet);
    assert!(matches!(
        Engine::new(vec![
            packet.clone(),
            binding(2, WheelDirection::Up, WheelPolicy::Notch)
        ]),
        Err(RegistrationError::DuplicateTrigger { .. })
    ));
    let mut engine = Engine::new(vec![packet.clone()]).unwrap();
    engine.process(event(InputKind::Down(SIDE)));
    let mut invalid = packet;
    invalid.activation = Activation::Press(Button::Key(75));
    assert_eq!(
        engine.replace_bindings(vec![invalid]),
        Err(RegistrationError::InvalidWheelPolicy { id: 1 })
    );
    assert_eq!(scroll(&mut engine, Axis::Vertical, 1).triggered, [1]);
}

#[test]
fn notch_wheel_keeps_120_unit_default_and_packet_replacement_drops_old_remainder() {
    let mut engine = Engine::new(vec![binding(1, WheelDirection::Up, WheelPolicy::Notch)]).unwrap();
    engine.process(event(InputKind::Down(SIDE)));
    assert!(scroll(&mut engine, Axis::Vertical, 15).triggered.is_empty());
    assert_eq!(scroll(&mut engine, Axis::Vertical, 240).triggered, [1, 1]);
    engine
        .replace_bindings(vec![binding(1, WheelDirection::Up, WheelPolicy::Packet)])
        .unwrap();
    engine.process(event(InputKind::Up(SIDE)));
    engine.process(event(InputKind::Down(SIDE)));
    assert_eq!(scroll(&mut engine, Axis::Vertical, 240).triggered, [1]);
    engine
        .replace_bindings(vec![binding(1, WheelDirection::Up, WheelPolicy::Notch)])
        .unwrap();
    engine.process(event(InputKind::Up(SIDE)));
    engine.process(event(InputKind::Down(SIDE)));
    assert!(
        scroll(&mut engine, Axis::Vertical, 105)
            .triggered
            .is_empty()
    );
    assert_eq!(scroll(&mut engine, Axis::Vertical, 15).triggered, [1]);
}
