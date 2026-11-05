use super::*;

const CAPS: Button = Button::Key(20);
const SPACE: Button = Button::Key(32);
const G: Button = Button::Key(71);
const X: Button = Button::Key(88);
const X2: Button = Button::Mouse(MouseButton::X2);
const RIGHT: Button = Button::Mouse(MouseButton::Right);

fn owner(id: u64, leader: Button) -> TriggerSpec {
    TriggerSpec {
        id,
        held: vec![leader],
        activation: Activation::Press(G),
        keyboard: KeyboardPolicy {
            dedicated_prefix: Some(leader),
            matching: KeyboardMatching::Captured { priority: 0 },
            ..Default::default()
        },
        wheel: Default::default(),
        conditions: vec![],
        process_scope: Default::default(),
        pointer_scope: Default::default(),
    }
}
fn retained() -> TriggerSpec {
    let mut row = owner(1, SPACE);
    row.keyboard.dedicated_prefix = None;
    row.keyboard.retained_prefix = Some(Box::new(RetainedPrefix {
        key: SPACE,
        capture_scope: Default::default(),
        quiet_ms: 50,
        fast_ms: 50,
        block_with_primary_mouse: false,
    }));
    row.keyboard.restore_on_tap = true;
    row
}
fn watcher(id: u64, leader: Button, activation: Activation) -> TriggerSpec {
    TriggerSpec {
        id,
        held: vec![leader],
        activation,
        keyboard: KeyboardPolicy {
            matching: KeyboardMatching::MouseCapturedLeader {
                priority: id as u16,
                pointer_scope_priority: false,
            },
            ..Default::default()
        },
        wheel: if matches!(activation, Activation::Wheel(_)) {
            WheelPolicy::Packet
        } else {
            WheelPolicy::Notch
        },
        conditions: vec![],
        process_scope: Default::default(),
        pointer_scope: Default::default(),
    }
}
fn wheel_watch(leader: Button) -> TriggerSpec {
    watcher(2, leader, Activation::Wheel(WheelDirection::Up))
}
fn input(engine: &mut Engine, kind: InputKind, time: u64) -> Decision {
    engine.process_at(
        InputEvent {
            kind,
            injected: false,
        },
        time,
    )
}
fn scroll(engine: &mut Engine, delta: i32, time: u64) -> Decision {
    input(
        engine,
        InputKind::Wheel {
            axis: Axis::Vertical,
            delta,
        },
        time,
    )
}
fn hit(id: u64) -> Decision {
    Decision {
        consume: true,
        triggered: vec![id],
        ..Default::default()
    }
}

#[test]
fn mouse_leader_requires_an_explicit_owner_and_cannot_create_capture_itself() {
    let watch = wheel_watch(CAPS);
    assert_eq!(
        Engine::new(vec![watch.clone()]).err().unwrap(),
        RegistrationError::MissingCapturedLeader {
            id: 2,
            button: CAPS
        }
    );
    let mut plain = owner(1, CAPS);
    plain.keyboard = Default::default();
    assert!(matches!(
        Engine::new(vec![plain, watch.clone()]),
        Err(RegistrationError::MissingCapturedLeader { .. })
    ));
    let mut scoped = owner(1, CAPS);
    scoped.process_scope.allow = vec!["editor".into()];
    let mut e = Engine::new(vec![scoped, watch]).unwrap();
    e.set_foreground_process(Some("reader.exe")).unwrap();
    assert!(!input(&mut e, InputKind::Down(CAPS), 0).consume);
    e.set_foreground_process(Some("editor.exe")).unwrap();
    assert_eq!(scroll(&mut e, 120, 1), Decision::default());
    assert!(!input(&mut e, InputKind::Up(CAPS), 2).consume);
    assert!(input(&mut e, InputKind::Down(CAPS), 3).consume);
    assert_eq!(scroll(&mut e, 1, 4), hit(2));
    assert!(input(&mut e, InputKind::Up(CAPS), 5).consume);
}

#[test]
fn mouse_leader_accepts_both_dedicated_matchers_and_keeps_keyboard_ownership() {
    for matching in [
        KeyboardMatching::Exact,
        KeyboardMatching::Captured { priority: 0 },
    ] {
        let mut row = owner(1, CAPS);
        row.keyboard.matching = matching;
        let mut e = Engine::new(vec![row, wheel_watch(CAPS)]).unwrap();
        assert!(input(&mut e, InputKind::Down(CAPS), 0).consume);
        assert_eq!(scroll(&mut e, 20, 1), hit(2));
        assert!(input(&mut e, InputKind::Down(X), 2).consume);
        assert_eq!(scroll(&mut e, 120, 3), hit(2));
        assert!(input(&mut e, InputKind::Up(X), 4).consume);
        assert_eq!(input(&mut e, InputKind::Down(G), 5), hit(1));
        assert!(input(&mut e, InputKind::Up(G), 6).consume);
        assert_eq!(scroll(&mut e, 120, 7), hit(2));
        assert!(input(&mut e, InputKind::Up(CAPS), 8).consume);
        assert_eq!(scroll(&mut e, 120, 9), Decision::default());
    }
}

#[test]
fn captured_adorn_key_is_not_an_active_leader_for_the_mouse() {
    let mut space = owner(1, SPACE);
    space.held.push(CAPS);
    let caps = owner(3, CAPS);
    let mut e = Engine::new(vec![space, caps, wheel_watch(CAPS)]).unwrap();
    input(&mut e, InputKind::Down(SPACE), 0);
    assert!(input(&mut e, InputKind::Down(CAPS), 1).consume);
    assert_eq!(scroll(&mut e, 120, 2), Decision::default());
    input(&mut e, InputKind::Up(CAPS), 3);
    input(&mut e, InputKind::Up(SPACE), 4);
    input(&mut e, InputKind::Down(CAPS), 5);
    assert_eq!(scroll(&mut e, 120, 6), hit(2));
}

#[test]
fn retained_quiet_and_fast_fallback_paths_never_lend_their_passed_leader() {
    let mut e = Engine::new(vec![retained(), wheel_watch(SPACE)]).unwrap();
    input(&mut e, InputKind::Down(X), 0);
    input(&mut e, InputKind::Up(X), 1);
    assert!(!input(&mut e, InputKind::Down(SPACE), 49).consume);
    assert_eq!(scroll(&mut e, 120, 50), Decision::default());
    assert!(!input(&mut e, InputKind::Up(SPACE), 51).consume);
    assert!(input(&mut e, InputKind::Down(SPACE), 100).consume);
    let fallback = input(&mut e, InputKind::Down(X), 110);
    assert!(!fallback.consume);
    assert_eq!(fallback.replay, [InputKind::Down(SPACE)]);
    assert_eq!(scroll(&mut e, 120, 111), Decision::default());
    assert!(!input(&mut e, InputKind::Up(SPACE), 120).consume);
    assert!(!input(&mut e, InputKind::Up(X), 121).consume);
    assert!(input(&mut e, InputKind::Down(SPACE), 200).consume);
    assert_eq!(scroll(&mut e, 120, 201), hit(2));
    assert!(input(&mut e, InputKind::Up(SPACE), 202).replay.is_empty());
}

#[test]
fn releasing_ctrl_before_a_passed_space_never_creates_a_mouse_session() {
    let mut e = Engine::new(vec![retained(), wheel_watch(SPACE)]).unwrap();
    let ctrl = Button::Key(162);
    assert!(!input(&mut e, InputKind::Down(ctrl), 0).consume);
    assert!(!input(&mut e, InputKind::Down(SPACE), 1).consume);
    assert!(!input(&mut e, InputKind::Up(ctrl), 2).consume);
    assert_eq!(scroll(&mut e, 120, 3), Decision::default());
    assert!(!input(&mut e, InputKind::Up(SPACE), 4).consume);
    assert!(input(&mut e, InputKind::Down(SPACE), 100).consume);
    assert_eq!(scroll(&mut e, 120, 101), hit(2));
}

#[test]
fn retained_capture_scope_can_supply_a_session_without_an_applicable_keyboard_action() {
    let mut row = retained();
    row.process_scope.allow = vec!["editor".into()];
    let mut e = Engine::new(vec![row, wheel_watch(SPACE)]).unwrap();
    e.set_foreground_process(Some("reader.exe")).unwrap();
    assert!(input(&mut e, InputKind::Down(SPACE), 100).consume);
    assert_eq!(scroll(&mut e, 120, 101), hit(2));
    assert!(input(&mut e, InputKind::Up(SPACE), 102).replay.is_empty());
}

#[test]
fn mouse_leader_distinguishes_passed_mouse_presses_from_owned_terminals() {
    let mut e = Engine::new(vec![
        owner(1, CAPS),
        wheel_watch(CAPS),
        watcher(3, CAPS, Activation::Press(X2)),
    ])
    .unwrap();
    input(&mut e, InputKind::Down(CAPS), 0);
    assert!(!input(&mut e, InputKind::Down(RIGHT), 1).consume);
    assert_eq!(scroll(&mut e, 120, 2), hit(2));
    assert!(!input(&mut e, InputKind::Up(RIGHT), 3).consume);
    assert_eq!(input(&mut e, InputKind::Down(X2), 4), hit(3));
    let repeating = input(&mut e, InputKind::Down(X2), 4);
    assert!(repeating.consume && repeating.triggered.is_empty());
    assert_eq!(scroll(&mut e, 120, 5), Decision::default());
    assert!(input(&mut e, InputKind::Up(X2), 6).consume);
    assert_eq!(scroll(&mut e, 120, 7), hit(2));
}

#[test]
fn cancelled_mouse_sessions_drain_owned_releases_without_rearming() {
    for mode in 0..4 {
        let mut e = Engine::new(vec![
            owner(1, CAPS),
            watcher(2, CAPS, Activation::Press(X2)),
        ])
        .unwrap();
        input(&mut e, InputKind::Down(CAPS), 0);
        assert_eq!(input(&mut e, InputKind::Down(X2), 1), hit(2));
        match mode {
            0 => {
                e.cancel();
            }
            1 => {
                e.set_foreground_process(Some("other.exe")).unwrap();
            }
            2 => {
                e.set_enabled(false);
                e.set_enabled(true);
            }
            _ => {
                e.replace_bindings(vec![
                    owner(1, CAPS),
                    watcher(2, CAPS, Activation::Press(X2)),
                ])
                .unwrap();
            }
        }
        assert!(input(&mut e, InputKind::Down(X2), 2).triggered.is_empty());
        assert!(input(&mut e, InputKind::Up(X2), 3).consume);
        assert!(input(&mut e, InputKind::Up(CAPS), 4).consume);
        input(&mut e, InputKind::Down(CAPS), 5);
        assert_eq!(input(&mut e, InputKind::Down(X2), 6), hit(2));
    }
}

#[test]
fn mouse_leader_owner_region_loss_and_reentry_require_a_fresh_press() {
    let mut row = owner(1, CAPS);
    row.conditions = vec![TriggerCondition::PointerRegion(PointerRegion::Rectangle(
        ScreenRect {
            left: 0,
            top: 0,
            right: 50,
            bottom: 50,
        },
    ))];
    let mut e = Engine::new(vec![row, wheel_watch(CAPS)]).unwrap();
    e.pointer_moved((1, 1), 0);
    input(&mut e, InputKind::Down(CAPS), 1);
    assert_eq!(scroll(&mut e, 120, 2), hit(2));
    e.pointer_moved((100, 100), 3);
    e.pointer_moved((1, 1), 4);
    assert_eq!(scroll(&mut e, 120, 5), Decision::default());
    assert!(input(&mut e, InputKind::Up(CAPS), 6).consume);
    input(&mut e, InputKind::Down(CAPS), 7);
    assert_eq!(scroll(&mut e, 120, 8), hit(2));
}

#[test]
fn observer_dependency_failure_keeps_a_live_wheel_remainder_and_capture_unchanged() {
    let mut watch = wheel_watch(CAPS);
    watch.wheel = WheelPolicy::Notch;
    let mut e = Engine::new(vec![owner(1, CAPS), watch.clone()]).unwrap();
    input(&mut e, InputKind::Down(CAPS), 0);
    assert!(scroll(&mut e, 60, 1).consume);
    assert!(matches!(
        e.replace_bindings(vec![watch]),
        Err(RegistrationError::MissingCapturedLeader { .. })
    ));
    assert_eq!(scroll(&mut e, 60, 2), hit(2));
    assert!(scroll(&mut e, 40, 3).consume);
    let up = input(&mut e, InputKind::Up(CAPS), 4);
    assert!(up.consume);
    assert_eq!(
        up.replay,
        [InputKind::Wheel {
            axis: Axis::Vertical,
            delta: 40
        }]
    );
}

#[test]
fn invalid_mouse_leader_policies_are_structured_errors_not_implicit_capture() {
    for key in [Button::Key(17), Button::Key(162), RIGHT] {
        assert!(matches!(
            Engine::new(vec![wheel_watch(key)]),
            Err(RegistrationError::InvalidMouseCapturedLeader { id: 2 })
        ));
    }
    for mode in 0..5 {
        let mut row = wheel_watch(CAPS);
        match mode {
            0 => row.held.push(X),
            1 => row.keyboard.dedicated_prefix = Some(CAPS),
            2 => row.keyboard.repeat = true,
            3 => row.activation = Activation::Press(X),
            _ => row.keyboard.restore_on_tap = true,
        }
        assert!(matches!(
            Engine::new(vec![owner(1, CAPS), row]),
            Err(RegistrationError::InvalidMouseCapturedLeader { id: 2 })
        ));
    }
}

#[test]
fn injected_leader_or_mouse_input_cannot_create_or_use_a_session() {
    let mut e = Engine::new(vec![owner(1, CAPS), wheel_watch(CAPS)]).unwrap();
    assert_eq!(
        e.process(InputEvent {
            kind: InputKind::Down(CAPS),
            injected: true
        }),
        Decision::default()
    );
    assert_eq!(scroll(&mut e, 120, 0), Decision::default());
    input(&mut e, InputKind::Down(CAPS), 1);
    assert_eq!(
        e.process(InputEvent {
            kind: InputKind::Wheel {
                axis: Axis::Vertical,
                delta: 120
            },
            injected: true
        }),
        Decision::default()
    );
    assert_eq!(scroll(&mut e, 120, 2), hit(2));
}
