use crate::*;

const PREFIX: Button = Button::Key(32);
const G: Button = Button::Key(71);
const K: Button = Button::Key(75);
const X: Button = Button::Key(88);

fn spec(id: u64, terminal: Button, adorn: Option<Button>) -> TriggerSpec {
    TriggerSpec {
        wheel: Default::default(),
        id,
        held: std::iter::once(PREFIX).chain(adorn).collect(),
        activation: Activation::Press(terminal),
        conditions: vec![],
        process_scope: Default::default(),
        keyboard: KeyboardPolicy {
            retained_prefix: Some(Box::new(RetainedPrefix {
                key: PREFIX,
                capture_scope: Default::default(),
                quiet_ms: 50,
                fast_ms: 50,
                block_with_primary_mouse: false,
            })),
            matching: KeyboardMatching::Captured {
                priority: id as u16,
            },
            repeat: true,
            restore_on_tap: true,
            tap_timeout_ms: Some(1500),
            ..Default::default()
        },
    }
}
fn input(engine: &mut Engine, key: Button, down: bool, time: u64) -> Decision {
    engine.process_at(
        InputEvent {
            kind: if down {
                InputKind::Down(key)
            } else {
                InputKind::Up(key)
            },
            injected: false,
        },
        time,
    )
}
fn tap(key: Button) -> Vec<InputKind> {
    vec![InputKind::Down(key), InputKind::Up(key)]
}

#[test]
fn retained_tap_has_inclusive_timeout_and_quiet_time_uses_restored_release() {
    for (elapsed, restore) in [(0, true), (1500, true), (1501, false)] {
        let mut e = Engine::new(vec![spec(1, K, None)]).unwrap();
        assert!(input(&mut e, PREFIX, true, 100).consume);
        let up = input(&mut e, PREFIX, false, 100 + elapsed);
        assert!(up.consume);
        assert_eq!(up.replay, if restore { tap(PREFIX) } else { vec![] });
        assert!(!input(&mut e, PREFIX, true, 149 + elapsed).consume);
        assert!(!input(&mut e, PREFIX, false, 150 + elapsed).consume);
    }
}

#[test]
fn retained_quiet_boundary_preserves_prior_passed_keys_and_repeats() {
    for (time, captured) in [(49, false), (50, true)] {
        let mut e = Engine::new(vec![spec(1, K, None)]).unwrap();
        assert!(!input(&mut e, X, true, 100).consume);
        assert!(!input(&mut e, X, true, 110).consume);
        assert_eq!(input(&mut e, PREFIX, true, 110 + time).consume, captured);
        assert!(!input(&mut e, X, false, 180).consume);
    }
    let mut e = Engine::new(vec![spec(1, X, None)]).unwrap();
    input(&mut e, X, true, 0);
    input(&mut e, PREFIX, true, 100);
    assert_eq!(input(&mut e, X, true, 160).triggered, vec![1]);
    assert!(!input(&mut e, X, false, 170).consume);
    assert!(input(&mut e, PREFIX, false, 180).replay.is_empty());
}

#[test]
fn retained_fast_terminal_falls_back_but_boundary_triggers() {
    for (elapsed, hit) in [(49, false), (50, true)] {
        let mut e = Engine::new(vec![spec(1, K, None)]).unwrap();
        input(&mut e, PREFIX, true, 100);
        let decision = input(&mut e, K, true, 100 + elapsed);
        assert_eq!(decision.consume, hit);
        assert_eq!(decision.triggered, if hit { vec![1] } else { vec![] });
        assert_eq!(
            decision.replay,
            if hit {
                vec![]
            } else {
                vec![InputKind::Down(PREFIX)]
            }
        );
        assert_eq!(input(&mut e, PREFIX, false, 200).consume, hit);
        assert_eq!(input(&mut e, K, false, 201).consume, hit);
    }
}

#[test]
fn retained_temporary_key_restores_two_taps_if_prefix_releases_first() {
    let mut e = Engine::new(vec![spec(1, K, Some(G))]).unwrap();
    input(&mut e, PREFIX, true, 100);
    input(&mut e, G, true, 110);
    let up = input(&mut e, PREFIX, false, 120);
    assert_eq!(up.replay, [tap(PREFIX), tap(G)].concat());
    assert!(up.consume);
    let tail = input(&mut e, G, false, 130);
    assert!(tail.consume);
    assert!(tail.replay.is_empty());
}

#[test]
fn retained_temporary_key_can_execute_its_bare_rule_on_release() {
    let mut e = Engine::new(vec![spec(1, K, Some(G)), spec(2, G, None)]).unwrap();
    input(&mut e, PREFIX, true, 100);
    assert!(input(&mut e, G, true, 110).triggered.is_empty());
    assert_eq!(input(&mut e, G, false, 120).triggered, vec![2]);
    assert!(input(&mut e, PREFIX, false, 130).replay.is_empty());
}

#[test]
fn retained_third_key_promotes_temporary_then_repeat_reselects_after_release() {
    let mut e = Engine::new(vec![spec(1, K, Some(G)), spec(2, K, None)]).unwrap();
    input(&mut e, PREFIX, true, 100);
    input(&mut e, G, true, 110);
    assert_eq!(input(&mut e, K, true, 120).triggered, vec![1]);
    assert!(input(&mut e, G, false, 130).consume);
    assert_eq!(input(&mut e, K, true, 160).triggered, vec![2]);
    assert!(input(&mut e, PREFIX, false, 170).consume);
    assert!(input(&mut e, K, false, 180).consume);
}

#[test]
fn retained_unmatched_third_key_keeps_temporary_capture_until_prefix_ends() {
    let mut e = Engine::new(vec![spec(1, K, Some(G))]).unwrap();
    input(&mut e, PREFIX, true, 100);
    input(&mut e, G, true, 110);
    assert!(input(&mut e, X, true, 120).consume);
    assert!(input(&mut e, G, false, 130).triggered.is_empty());
    assert_eq!(input(&mut e, K, true, 160).triggered, vec![1]);
    e.cancel();
    for key in [PREFIX, X, K] {
        let up = input(&mut e, key, false, 180);
        assert!(up.consume);
        assert!(up.replay.is_empty());
    }
    input(&mut e, PREFIX, true, 300);
    assert!(input(&mut e, K, true, 360).triggered.is_empty());
}

#[test]
fn retained_unknown_key_returns_prefix_down_and_drains_earlier_consumed_keys() {
    let mut e = Engine::new(vec![spec(1, K, None)]).unwrap();
    input(&mut e, PREFIX, true, 100);
    input(&mut e, K, true, 160);
    let fallback = input(&mut e, X, true, 170);
    assert!(!fallback.consume);
    assert_eq!(fallback.replay, vec![InputKind::Down(PREFIX)]);
    assert!(input(&mut e, K, false, 180).consume);
    assert!(!input(&mut e, PREFIX, false, 190).consume);
    assert!(!input(&mut e, X, false, 200).consume);
}

#[test]
fn retained_primary_mouse_restores_down_and_keeps_release_balanced() {
    for (elapsed, used) in [(100, false), (1501, false), (100, true)] {
        let mut e = Engine::new(vec![spec(1, K, None)]).unwrap();
        input(&mut e, PREFIX, true, 100);
        let mouse = input(&mut e, Button::Mouse(MouseButton::Left), true, 160);
        assert_eq!(mouse.replay, vec![InputKind::Down(PREFIX)]);
        assert!(!mouse.consume);
        if used {
            assert_eq!(input(&mut e, K, true, 170).triggered, vec![1]);
        }
        let up = input(&mut e, PREFIX, false, 100 + elapsed);
        assert_eq!(
            up.replay,
            if elapsed <= 1500 && !used {
                tap(PREFIX)
            } else {
                vec![]
            }
        );
        assert_eq!(up.consume, elapsed <= 1500 && !used);
        assert!(!input(&mut e, Button::Mouse(MouseButton::Left), false, 2000).consume);
    }
    let mut e = Engine::new(vec![spec(1, K, None)]).unwrap();
    input(&mut e, PREFIX, true, 100);
    input(&mut e, Button::Mouse(MouseButton::Left), true, 160);
    e.cancel();
    assert!(!input(&mut e, PREFIX, false, 180).consume);
}

#[test]
fn retained_mouse_action_suppresses_tap_and_primary_mouse_can_block_capture() {
    let right = Button::Mouse(MouseButton::Right);
    let mut mouse = spec(2, right, None);
    mouse.keyboard.repeat = false;
    let mut e = Engine::new(vec![spec(1, K, None), mouse]).unwrap();
    input(&mut e, PREFIX, true, 100);
    assert_eq!(input(&mut e, right, true, 160).triggered, vec![2]);
    assert!(input(&mut e, PREFIX, false, 200).replay.is_empty());
    assert!(input(&mut e, right, false, 210).consume);
    let mut blocked = spec(1, K, None);
    blocked
        .keyboard
        .retained_prefix
        .as_mut()
        .unwrap()
        .block_with_primary_mouse = true;
    let mut e = Engine::new(vec![blocked]).unwrap();
    input(&mut e, Button::Mouse(MouseButton::Left), true, 0);
    assert!(!input(&mut e, PREFIX, true, 100).consume);
    assert!(!input(&mut e, PREFIX, false, 110).consume);
}

#[test]
fn retained_context_changes_and_injection_cannot_revive_old_session() {
    let mut row = spec(1, K, Some(G));
    row.process_scope.allow = vec!["editor".into()];
    row.keyboard
        .retained_prefix
        .as_mut()
        .unwrap()
        .capture_scope
        .allow = vec!["editor".into()];
    let mut e = Engine::new(vec![row]).unwrap();
    e.set_foreground_process(Some("editor")).unwrap();
    input(&mut e, PREFIX, true, 100);
    input(&mut e, G, true, 110);
    assert_eq!(
        e.process_at(
            InputEvent {
                kind: InputKind::Up(PREFIX),
                injected: true
            },
            9999
        ),
        Decision::default()
    );
    e.set_foreground_process(Some("other")).unwrap();
    assert!(input(&mut e, PREFIX, false, 120).consume);
    assert!(input(&mut e, G, false, 130).consume);
    assert!(!input(&mut e, PREFIX, true, 300).consume);
}

#[test]
fn retained_and_dedicated_captured_layers_follow_the_first_leader() {
    let caps = Button::Key(20);
    let mut dedicated = spec(10, PREFIX, None);
    dedicated.held = vec![caps];
    dedicated.keyboard.retained_prefix = None;
    dedicated.keyboard.dedicated_prefix = Some(caps);
    dedicated.keyboard.restore_on_tap = false;
    dedicated.keyboard.tap_timeout_ms = None;
    let retained = spec(1, K, Some(caps));
    let mut e = Engine::new(vec![dedicated, retained]).unwrap();
    input(&mut e, caps, true, 100);
    assert_eq!(input(&mut e, PREFIX, true, 110).triggered, vec![10]);
    input(&mut e, PREFIX, false, 120);
    input(&mut e, caps, false, 130);
    input(&mut e, PREFIX, true, 300);
    input(&mut e, caps, true, 310);
    assert_eq!(input(&mut e, K, true, 320).triggered, vec![1]);
    for key in [caps, K, PREFIX] {
        assert!(input(&mut e, key, false, 330).consume);
    }
    let mut a = spec(20, K, None);
    a.held = vec![caps, PREFIX];
    a.keyboard.retained_prefix = None;
    a.keyboard.dedicated_prefix = Some(caps);
    a.keyboard.restore_on_tap = false;
    a.keyboard.tap_timeout_ms = None;
    let b = spec(21, K, Some(caps));
    let mut e = Engine::new(vec![a, b]).unwrap();
    input(&mut e, caps, true, 100);
    input(&mut e, PREFIX, true, 160);
    assert_eq!(input(&mut e, K, true, 170).triggered, vec![20]);
    for key in [K, PREFIX, caps] {
        input(&mut e, key, false, 180);
    }
    input(&mut e, PREFIX, true, 300);
    input(&mut e, caps, true, 310);
    assert_eq!(input(&mut e, K, true, 320).triggered, vec![21]);
}

#[test]
fn retained_capture_scope_is_independent_of_action_scope_and_checked_across_disjoint_actions() {
    let mut row = spec(1, K, None);
    row.process_scope.allow = vec!["editor".into()];
    let mut e = Engine::new(vec![row.clone()]).unwrap();
    e.set_foreground_process(Some("other")).unwrap();
    assert!(input(&mut e, PREFIX, true, 100).consume);
    let key = input(&mut e, K, true, 160);
    assert!(key.triggered.is_empty());
    assert_eq!(key.replay, vec![InputKind::Down(PREFIX)]);
    assert!(!input(&mut e, PREFIX, false, 170).consume);
    let mut other = spec(2, X, None);
    other.process_scope.allow = vec!["other".into()];
    other.keyboard.retained_prefix.as_mut().unwrap().quiet_ms = 30;
    assert!(matches!(
        Engine::new(vec![row.clone(), other]),
        Err(RegistrationError::ConflictingPrefixPolicy { .. })
    ));
    let mut dedicated = spec(3, X, None);
    dedicated.process_scope.allow = vec!["other".into()];
    dedicated.keyboard.retained_prefix = None;
    dedicated.keyboard.dedicated_prefix = Some(PREFIX);
    assert!(matches!(
        Engine::new(vec![row, dedicated]),
        Err(RegistrationError::ConflictingPrefixPolicy { .. })
    ));
}

#[test]
fn retained_wheel_preserves_partial_delta_and_completed_action_suppresses_tap() {
    let mut wheel = spec(1, K, None);
    wheel.activation = Activation::Wheel(WheelDirection::Up);
    wheel.keyboard.repeat = false;
    for delta in [60, 120] {
        let mut e = Engine::new(vec![wheel.clone()]).unwrap();
        input(&mut e, PREFIX, true, 100);
        let decision = e.process_at(
            InputEvent {
                kind: InputKind::Wheel {
                    axis: Axis::Vertical,
                    delta,
                },
                injected: false,
            },
            160,
        );
        assert!(decision.consume);
        let up = input(&mut e, PREFIX, false, 170);
        if delta == 60 {
            assert_eq!(
                up.replay,
                [
                    vec![InputKind::Wheel {
                        axis: Axis::Vertical,
                        delta
                    }],
                    tap(PREFIX)
                ]
                .concat()
            );
        } else {
            assert_eq!(decision.triggered, vec![1]);
            assert!(up.replay.is_empty());
        }
    }
}

#[test]
fn retained_modifiers_bypass_capture_and_do_not_steal_existing_shortcuts() {
    for code in [16, 17, 18, 91, 92, 160, 161, 162, 163, 164, 165] {
        let modifier = Button::Key(code);
        let mut e = Engine::new(vec![spec(1, K, None)]).unwrap();
        input(&mut e, modifier, true, 0);
        assert!(!input(&mut e, PREFIX, true, 100).consume);
        input(&mut e, modifier, false, 110);
        input(&mut e, PREFIX, false, 120);
        // 被放行的修饰组合不会延长前键间隔。
        assert!(input(&mut e, PREFIX, true, 130).consume);
    }
    let ctrl = Button::Key(162);
    let normal = TriggerSpec {
        wheel: Default::default(),
        id: 2,
        held: vec![ctrl],
        activation: Activation::Press(PREFIX),
        keyboard: Default::default(),
        process_scope: Default::default(),
        conditions: vec![],
    };
    let mut e = Engine::new(vec![spec(1, K, None), normal]).unwrap();
    input(&mut e, ctrl, true, 0);
    assert_eq!(input(&mut e, PREFIX, true, 100).triggered, vec![2]);
    assert!(input(&mut e, PREFIX, false, 110).consume);
    assert!(input(&mut e, ctrl, false, 120).consume);
}

#[test]
fn retained_registration_rejects_implicit_or_conflicting_policy() {
    for kind in 0..6 {
        let mut row = spec(1, K, None);
        match kind {
            0 => row.keyboard.restore_on_tap = false,
            1 => row.keyboard.dedicated_prefix = Some(PREFIX),
            2 => row.keyboard.matching = KeyboardMatching::Exact,
            3 => row.keyboard.retained_prefix.as_mut().unwrap().fast_ms = 60001,
            4 => row.keyboard.retained_prefix.as_mut().unwrap().key = Button::Key(17),
            _ => row.keyboard.retained_prefix.as_mut().unwrap().key = X,
        }
        assert!(matches!(
            Engine::new(vec![row]),
            Err(RegistrationError::InvalidRetainedPrefix { .. })
        ));
    }
    let a = spec(1, K, None);
    let mut b = spec(2, X, None);
    b.keyboard.retained_prefix.as_mut().unwrap().quiet_ms = 30;
    assert!(matches!(
        Engine::new(vec![a, b]),
        Err(RegistrationError::ConflictingPrefixPolicy { .. })
    ));
}
