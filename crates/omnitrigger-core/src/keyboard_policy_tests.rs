use crate::*;

const CAPS: Button = Button::Key(20);
const K: Button = Button::Key(75);
const J: Button = Button::Key(74);
const E: Button = Button::Key(69);
const X: Button = Button::Key(88);
const SIDE: Button = Button::Mouse(MouseButton::X1);

fn binding(id: u64, held: &[Button], activation: Activation) -> TriggerSpec {
    TriggerSpec {
        wheel: Default::default(),
        id,
        held: held.to_vec(),
        activation,
        conditions: vec![],
        process_scope: Default::default(),
        keyboard: KeyboardPolicy {
            dedicated_prefix: Some(CAPS),
            repeat: false,
            ..Default::default()
        },
    }
}
fn press() -> TriggerSpec {
    binding(1, &[CAPS], Activation::Press(K))
}
fn input(engine: &mut Engine, kind: InputKind, now: u64) -> Decision {
    engine.process_at(
        InputEvent {
            kind,
            injected: false,
        },
        now,
    )
}
fn swallowed(decision: Decision) {
    assert!(decision.consume);
    assert!(decision.replay.is_empty());
    assert!(decision.triggered.is_empty());
}

#[test]
fn dedicated_tap_unknown_key_and_its_repeats_never_replay() {
    let mut engine = Engine::new(vec![press()]).unwrap();
    swallowed(input(&mut engine, InputKind::Down(CAPS), 0));
    swallowed(input(&mut engine, InputKind::Down(CAPS), 1));
    swallowed(input(&mut engine, InputKind::Up(CAPS), 2));
    swallowed(input(&mut engine, InputKind::Down(CAPS), 3));
    for time in 4..10 {
        swallowed(input(&mut engine, InputKind::Down(X), time));
    }
    swallowed(input(&mut engine, InputKind::Up(X), 10));
    assert_eq!(
        input(&mut engine, InputKind::Down(K), 11).triggered,
        vec![1]
    );
    swallowed(input(&mut engine, InputKind::Up(K), 12));
    swallowed(input(&mut engine, InputKind::Up(CAPS), 13));
    assert_eq!(
        input(&mut engine, InputKind::Down(X), 14),
        Decision::default()
    );
}

#[test]
fn dedicated_adorn_can_be_reused_after_an_unknown_combination() {
    let adorn = binding(2, &[CAPS, E], Activation::Press(J));
    let mut engine = Engine::new(vec![press(), adorn]).unwrap();
    swallowed(input(&mut engine, InputKind::Down(CAPS), 0));
    swallowed(input(&mut engine, InputKind::Down(E), 1));
    swallowed(input(&mut engine, InputKind::Down(X), 2));
    swallowed(input(&mut engine, InputKind::Up(X), 3));
    assert_eq!(input(&mut engine, InputKind::Down(J), 4).triggered, vec![2]);
    swallowed(input(&mut engine, InputKind::Up(J), 5));
    swallowed(input(&mut engine, InputKind::Up(E), 6));
    assert_eq!(input(&mut engine, InputKind::Down(K), 7).triggered, vec![1]);
}

#[test]
fn releasing_leader_first_drains_unknown_keys_without_lending_them_to_other_chords() {
    let ordinary = TriggerSpec {
        id: 2,
        keyboard: Default::default(),
        held: vec![X],
        activation: Activation::Press(J),
        ..press()
    };
    let mut engine = Engine::new(vec![press(), ordinary]).unwrap();
    swallowed(input(&mut engine, InputKind::Down(CAPS), 0));
    swallowed(input(&mut engine, InputKind::Down(X), 1));
    swallowed(input(&mut engine, InputKind::Up(CAPS), 2));
    assert_eq!(
        input(&mut engine, InputKind::Down(J), 3),
        Decision::default()
    );
    assert_eq!(input(&mut engine, InputKind::Up(J), 4), Decision::default());
    swallowed(input(&mut engine, InputKind::Down(X), 5));
    swallowed(input(&mut engine, InputKind::Up(X), 6));
}

#[test]
fn dedicated_prefix_pressed_after_passed_input_is_suppressed_without_starting_a_layer() {
    let mut engine = Engine::new(vec![press()]).unwrap();
    assert_eq!(
        input(&mut engine, InputKind::Down(X), 0),
        Decision::default()
    );
    swallowed(input(&mut engine, InputKind::Down(CAPS), 1));
    assert_eq!(
        input(&mut engine, InputKind::Down(K), 2),
        Decision::default()
    );
    swallowed(input(&mut engine, InputKind::Up(CAPS), 3));
    assert_eq!(input(&mut engine, InputKind::Up(K), 4), Decision::default());
    assert_eq!(input(&mut engine, InputKind::Up(X), 5), Decision::default());
}

#[test]
fn dedicated_hold_short_release_and_interruption_keep_keyboard_input_suppressed() {
    for interrupt in [false, true] {
        let timed = binding(
            1,
            &[CAPS],
            Activation::Hold {
                button: K,
                duration_ms: 100,
            },
        );
        let mut engine = Engine::new(vec![timed]).unwrap();
        swallowed(input(&mut engine, InputKind::Down(CAPS), 0));
        swallowed(input(&mut engine, InputKind::Down(K), 1));
        if interrupt {
            swallowed(input(&mut engine, InputKind::Down(X), 20));
        }
        swallowed(input(&mut engine, InputKind::Up(K), 30));
        assert!(engine.advance_time(200).triggered.is_empty());
        if interrupt {
            swallowed(input(&mut engine, InputKind::Up(X), 201));
        }
        swallowed(input(&mut engine, InputKind::Up(CAPS), 202));
    }
    let mut engine = Engine::new(vec![binding(
        1,
        &[CAPS],
        Activation::Hold {
            button: K,
            duration_ms: 100,
        },
    )])
    .unwrap();
    input(&mut engine, InputKind::Down(CAPS), 0);
    input(&mut engine, InputKind::Down(K), 1);
    assert_eq!(engine.advance_time(101).triggered, vec![1]);
    swallowed(input(&mut engine, InputKind::Up(CAPS), 102));
    swallowed(input(&mut engine, InputKind::Up(K), 103));
}

#[test]
fn mouse_pass_through_and_failed_mouse_prefix_keep_their_paired_edges() {
    let mut engine = Engine::new(vec![
        press(),
        binding(2, &[CAPS, SIDE], Activation::Press(J)),
    ])
    .unwrap();
    engine.pointer_moved((0, 0), 0);
    swallowed(input(&mut engine, InputKind::Down(CAPS), 0));
    let left = Button::Mouse(MouseButton::Left);
    assert_eq!(
        input(&mut engine, InputKind::Down(left), 1),
        Decision::default()
    );
    assert_eq!(
        input(&mut engine, InputKind::Up(left), 2),
        Decision::default()
    );
    swallowed(input(&mut engine, InputKind::Down(SIDE), 3));
    assert_eq!(
        engine.pointer_moved((9, 0), 4).replay,
        vec![InputKind::Down(SIDE)]
    );
    assert_eq!(
        input(&mut engine, InputKind::Up(SIDE), 5),
        Decision::default()
    );
    swallowed(input(&mut engine, InputKind::Up(CAPS), 6));
}

#[test]
fn dedicated_wheel_remainders_return_without_replaying_the_leader() {
    let wheel = binding(1, &[CAPS], Activation::Wheel(WheelDirection::Up));
    let mut engine = Engine::new(vec![wheel]).unwrap();
    for (delta, expected, tail) in [(60, vec![], 60), (140, vec![1], 20)] {
        swallowed(input(&mut engine, InputKind::Down(CAPS), 0));
        let result = input(
            &mut engine,
            InputKind::Wheel {
                axis: Axis::Vertical,
                delta,
            },
            1,
        );
        assert_eq!(result.triggered, expected);
        assert!(result.consume && result.replay.is_empty());
        let release = input(&mut engine, InputKind::Up(CAPS), 2);
        assert!(release.consume);
        assert_eq!(
            release.replay,
            vec![InputKind::Wheel {
                axis: Axis::Vertical,
                delta: tail
            }]
        );
    }
}

#[test]
fn pause_focus_change_and_replacement_drain_layer_and_repeat_owners() {
    for mode in 0..4 {
        let mut spec = press();
        spec.keyboard.repeat = true;
        spec.process_scope.allow = vec!["editor".into()];
        let mut engine = Engine::new(vec![spec.clone()]).unwrap();
        engine.set_foreground_process(Some("editor")).unwrap();
        input(&mut engine, InputKind::Down(CAPS), 0);
        assert_eq!(input(&mut engine, InputKind::Down(K), 1).triggered, vec![1]);
        match mode {
            0 => {
                engine.cancel();
            }
            1 => {
                engine.set_enabled(false);
            }
            2 => {
                engine.set_foreground_process(Some("other")).unwrap();
            }
            _ => {
                engine.replace_bindings(vec![spec]).unwrap();
            }
        }
        swallowed(input(&mut engine, InputKind::Down(K), 2));
        swallowed(input(&mut engine, InputKind::Up(CAPS), 3));
        swallowed(input(&mut engine, InputKind::Up(K), 4));
        assert_eq!(
            input(&mut engine, InputKind::Down(X), 5),
            Decision::default()
        );
    }
}

#[test]
fn scoped_layer_is_inactive_outside_the_application_and_after_region_exit() {
    let mut spec = press();
    spec.process_scope.allow = vec!["editor".into()];
    spec.conditions = vec![TriggerCondition::PointerRegion(PointerRegion::Rectangle(
        ScreenRect {
            left: 0,
            top: 0,
            right: 100,
            bottom: 100,
        },
    ))];
    let mut engine = Engine::new(vec![spec]).unwrap();
    assert_eq!(
        input(&mut engine, InputKind::Down(CAPS), 0),
        Decision::default()
    );
    engine.set_foreground_process(Some("editor")).unwrap();
    engine.pointer_moved((10, 10), 1);
    assert_eq!(
        input(&mut engine, InputKind::Down(K), 2),
        Decision::default()
    );
    input(&mut engine, InputKind::Up(K), 3);
    input(&mut engine, InputKind::Up(CAPS), 4);
    swallowed(input(&mut engine, InputKind::Down(CAPS), 5));
    engine.pointer_moved((110, 10), 6);
    engine.pointer_moved((10, 10), 7);
    assert_eq!(
        input(&mut engine, InputKind::Down(K), 8),
        Decision::default()
    );
    swallowed(input(&mut engine, InputKind::Up(CAPS), 9));
}

#[test]
fn repeat_is_explicit_and_stays_with_the_original_rule() {
    let mut global = press();
    global.keyboard.repeat = true;
    let mut scoped = global.clone();
    scoped.id = 2;
    scoped.keyboard.repeat = false;
    scoped.process_scope.allow = vec!["editor".into()];
    let mut engine = Engine::new(vec![global, scoped]).unwrap();
    engine.set_foreground_process(Some("other")).unwrap();
    input(&mut engine, InputKind::Down(CAPS), 0);
    for time in 1..10 {
        assert_eq!(
            input(&mut engine, InputKind::Down(K), time).triggered,
            vec![1]
        );
    }
    engine.set_foreground_process(Some("editor")).unwrap();
    swallowed(input(&mut engine, InputKind::Down(K), 10));
    swallowed(input(&mut engine, InputKind::Up(K), 11));
    swallowed(input(&mut engine, InputKind::Up(CAPS), 12));
    input(&mut engine, InputKind::Down(CAPS), 13);
    assert_eq!(
        input(&mut engine, InputKind::Down(K), 14).triggered,
        vec![2]
    );
    swallowed(input(&mut engine, InputKind::Down(K), 15));
}

#[test]
fn repeat_stops_on_extra_held_input_and_cannot_resume_after_its_scope_is_lost() {
    let mut spec = press();
    spec.keyboard.repeat = true;
    spec.conditions = vec![TriggerCondition::PointerRegion(PointerRegion::Rectangle(
        ScreenRect {
            left: 0,
            top: 0,
            right: 100,
            bottom: 100,
        },
    ))];
    let mut engine = Engine::new(vec![spec]).unwrap();
    engine.pointer_moved((10, 10), 0);
    input(&mut engine, InputKind::Down(CAPS), 0);
    input(&mut engine, InputKind::Down(K), 1);
    swallowed(input(&mut engine, InputKind::Down(X), 2));
    swallowed(input(&mut engine, InputKind::Down(K), 3));
    swallowed(input(&mut engine, InputKind::Up(X), 4));
    assert_eq!(input(&mut engine, InputKind::Down(K), 5).triggered, vec![1]);
    engine.pointer_moved((110, 10), 6);
    engine.pointer_moved((10, 10), 7);
    swallowed(input(&mut engine, InputKind::Down(K), 8));
}

#[test]
fn injected_events_do_not_end_the_layer_repeat_actions_or_advance_a_hold() {
    let mut engine = Engine::new(vec![binding(
        1,
        &[CAPS],
        Activation::Hold {
            button: K,
            duration_ms: 100,
        },
    )])
    .unwrap();
    input(&mut engine, InputKind::Down(CAPS), 0);
    input(&mut engine, InputKind::Down(K), 0);
    for kind in [InputKind::Up(CAPS), InputKind::Down(X), InputKind::Down(K)] {
        assert_eq!(
            engine.process_at(
                InputEvent {
                    kind,
                    injected: true
                },
                1000
            ),
            Decision::default()
        );
    }
    assert!(engine.advance_time(99).triggered.is_empty());
    assert_eq!(engine.advance_time(100).triggered, vec![1]);
}

#[test]
fn keyboard_policy_validation_and_failed_replacement_keep_live_state() {
    let mut engine = Engine::new(vec![press()]).unwrap();
    input(&mut engine, InputKind::Down(CAPS), 0);
    let mut ordinary = press();
    ordinary.id = 2;
    ordinary.activation = Activation::Press(J);
    ordinary.keyboard = Default::default();
    assert!(matches!(
        engine.replace_bindings(vec![press(), ordinary.clone()]),
        Err(RegistrationError::ConflictingPrefixPolicy { .. })
    ));
    assert_eq!(input(&mut engine, InputKind::Down(K), 1).triggered, vec![1]);
    for button in [SIDE, K, Button::Key(17), Button::Key(0)] {
        let mut invalid = press();
        invalid.keyboard.dedicated_prefix = Some(button);
        assert!(matches!(
            Engine::new(vec![invalid]),
            Err(RegistrationError::InvalidDedicatedPrefix { .. })
        ));
    }
    for activation in [
        Activation::Press(SIDE),
        Activation::Hold {
            button: K,
            duration_ms: 100,
        },
        Activation::Wheel(WheelDirection::Up),
    ] {
        let mut invalid = press();
        invalid.activation = activation;
        invalid.keyboard.repeat = true;
        assert!(matches!(
            Engine::new(vec![invalid]),
            Err(RegistrationError::InvalidKeyboardRepeat { .. })
        ));
    }
    let mut dedicated = press();
    dedicated.process_scope.allow = vec!["one".into()];
    ordinary.process_scope.allow = vec!["two".into()];
    assert!(Engine::new(vec![dedicated, ordinary]).is_ok());
}

#[test]
fn tap_restore_is_explicit_bounded_and_cancelled_by_keyboard_or_bound_wheel() {
    for (interruption, expected) in [
        (0, true),
        (1, false),
        (2, false),
        (3, true),
        (4, false),
        (5, false),
    ] {
        let mut spec = press();
        spec.keyboard.restore_on_tap = true;
        spec.keyboard.tap_timeout_ms = Some(100);
        let mut wheel = binding(2, &[CAPS], Activation::Wheel(WheelDirection::Up));
        wheel.keyboard = spec.keyboard.clone();
        let mut engine = Engine::new(vec![spec, wheel]).unwrap();
        swallowed(input(&mut engine, InputKind::Down(CAPS), 0));
        match interruption {
            1 => {
                input(&mut engine, InputKind::Down(X), 1);
                input(&mut engine, InputKind::Up(X), 2);
            }
            2 => {
                engine.cancel();
            }
            3 => {
                // 未绑定鼠标输入不占用这个仅按键盘定义的单按。
                let left = Button::Mouse(MouseButton::Left);
                assert!(!input(&mut engine, InputKind::Down(left), 1).consume);
                assert!(!input(&mut engine, InputKind::Up(left), 2).consume);
            }
            4 => {
                engine.advance_time(100);
            }
            5 => {
                input(
                    &mut engine,
                    InputKind::Wheel {
                        axis: Axis::Vertical,
                        delta: 60,
                    },
                    1,
                );
            }
            _ => {
                swallowed(input(&mut engine, InputKind::Down(CAPS), 1));
            }
        }
        let released = input(&mut engine, InputKind::Up(CAPS), 50);
        assert!(released.consume);
        if expected {
            assert_eq!(
                released.replay,
                vec![InputKind::Down(CAPS), InputKind::Up(CAPS)]
            );
        } else {
            assert!(!released.replay.iter().any(|kind| matches!(kind, InputKind::Down(button) | InputKind::Up(button) if *button == CAPS)));
        }
    }
}

#[test]
fn injected_keyboard_does_not_cancel_tap_and_a_successful_action_does() {
    let mut spec = press();
    spec.keyboard.restore_on_tap = true;
    let mut engine = Engine::new(vec![spec]).unwrap();
    input(&mut engine, InputKind::Down(CAPS), 0);
    engine.process_at(
        InputEvent {
            kind: InputKind::Down(X),
            injected: true,
        },
        1000,
    );
    assert_eq!(
        input(&mut engine, InputKind::Up(CAPS), 1).replay,
        vec![InputKind::Down(CAPS), InputKind::Up(CAPS)]
    );
    input(&mut engine, InputKind::Down(CAPS), 2);
    assert_eq!(input(&mut engine, InputKind::Down(K), 3).triggered, vec![1]);
    swallowed(input(&mut engine, InputKind::Up(K), 4));
    swallowed(input(&mut engine, InputKind::Up(CAPS), 5));
}

#[test]
fn tap_policy_must_be_consistent_and_region_change_cannot_switch_it_mid_press() {
    let mut restoring = press();
    restoring.keyboard.restore_on_tap = true;
    let mut ordinary_layer = press();
    ordinary_layer.id = 2;
    ordinary_layer.activation = Activation::Press(J);
    assert!(matches!(
        Engine::new(vec![restoring.clone(), ordinary_layer.clone()]),
        Err(RegistrationError::ConflictingPrefixPolicy { .. })
    ));
    for (spec, left) in [(&mut restoring, 0), (&mut ordinary_layer, 100)] {
        spec.conditions = vec![TriggerCondition::PointerRegion(PointerRegion::Rectangle(
            ScreenRect {
                left,
                top: 0,
                right: left + 100,
                bottom: 100,
            },
        ))];
    }
    let mut engine = Engine::new(vec![restoring, ordinary_layer]).unwrap();
    engine.pointer_moved((10, 10), 0);
    input(&mut engine, InputKind::Down(CAPS), 0);
    engine.pointer_moved((110, 10), 1);
    swallowed(input(&mut engine, InputKind::Up(CAPS), 2));
    for policy in [
        KeyboardPolicy {
            restore_on_tap: true,
            ..Default::default()
        },
        KeyboardPolicy {
            dedicated_prefix: Some(CAPS),
            tap_timeout_ms: Some(100),
            ..Default::default()
        },
        KeyboardPolicy {
            dedicated_prefix: Some(CAPS),
            restore_on_tap: true,
            tap_timeout_ms: Some(0),
            ..Default::default()
        },
    ] {
        let mut invalid = press();
        invalid.keyboard = policy;
        assert!(matches!(
            Engine::new(vec![invalid]),
            Err(RegistrationError::InvalidTapRestore { .. })
        ));
    }
}

#[test]
fn tap_restore_scope_is_independent_of_layer_scope_and_fails_closed_when_unknown() {
    for (process, restore) in [
        (None, false),
        (Some("editor"), false),
        (Some("WORMHOLE.exe"), true),
    ] {
        let mut spec = press();
        spec.keyboard.restore_on_tap = true;
        spec.keyboard.tap_scope.allow = vec!["Wormhole".into()];
        let mut engine = Engine::new(vec![spec]).unwrap();
        assert!(engine.needs_process_context());
        engine.set_foreground_process(process).unwrap();
        swallowed(input(&mut engine, InputKind::Down(CAPS), 0));
        let released = input(&mut engine, InputKind::Up(CAPS), 1);
        assert!(released.consume);
        assert_eq!(!released.replay.is_empty(), restore);
        swallowed(input(&mut engine, InputKind::Down(CAPS), 2));
        swallowed(input(&mut engine, InputKind::Down(X), 3));
        swallowed(input(&mut engine, InputKind::Up(X), 4));
        swallowed(input(&mut engine, InputKind::Up(CAPS), 5));
    }
}
