use crate::*;

const CAPS: Button = Button::Key(20);
const SPACE: Button = Button::Key(32);
const E: Button = Button::Key(69);
const J: Button = Button::Key(74);
const P: Button = Button::Key(80);
const TAB: Button = Button::Key(9);
const X: Button = Button::Key(88);

fn rule(id: u64, adorn: Option<Button>, terminal: Button, priority: u16) -> TriggerSpec {
    TriggerSpec {
        id,
        held: [Some(CAPS), adorn].into_iter().flatten().collect(),
        activation: Activation::Press(terminal),
        conditions: vec![],
        process_scope: Default::default(),
        keyboard: KeyboardPolicy {
            dedicated_prefix: Some(CAPS),
            matching: KeyboardMatching::Captured { priority },
            repeat: true,
            ..Default::default()
        },
    }
}
fn event(engine: &mut Engine, kind: InputKind) -> Decision {
    engine.process(InputEvent {
        kind,
        injected: false,
    })
}
fn down(engine: &mut Engine, key: Button) -> Decision {
    event(engine, InputKind::Down(key))
}
fn up(engine: &mut Engine, key: Button) -> Decision {
    event(engine, InputKind::Up(key))
}
fn quiet(value: Decision) {
    assert!(value.consume && value.triggered.is_empty() && value.replay.is_empty());
}

#[test]
fn captured_default_rules_ignore_extra_adorns_unknown_keys_and_old_terminals() {
    let mut engine = Engine::new(vec![rule(1, Some(SPACE), J, 0), rule(2, None, TAB, 10)]).unwrap();
    quiet(down(&mut engine, CAPS));
    quiet(down(&mut engine, SPACE));
    quiet(down(&mut engine, X));
    assert_eq!(down(&mut engine, TAB).triggered, vec![2]);
    // 仍按住 Tab 和未知的 X；只有 Space 是捕获前缀。
    assert_eq!(down(&mut engine, J).triggered, vec![1]);
    for key in [J, TAB, X, SPACE, CAPS] {
        quiet(up(&mut engine, key));
    }
}

#[test]
fn captured_app_rules_precede_global_adorns_and_unknown_process_disables_the_layer() {
    let mut specific = rule(1, None, P, 20);
    specific.process_scope.allow = vec!["editor".into()];
    let rules = vec![specific, rule(2, Some(E), P, 0)];
    let mut engine = Engine::new(rules).unwrap();
    assert_eq!(down(&mut engine, CAPS), Decision::default());
    up(&mut engine, CAPS);
    engine.set_foreground_process(Some("editor")).unwrap();
    quiet(down(&mut engine, CAPS));
    quiet(down(&mut engine, E));
    assert_eq!(down(&mut engine, P).triggered, vec![1]);
    for key in [P, E, CAPS] {
        up(&mut engine, key);
    }
    engine.set_foreground_process(Some("other")).unwrap();
    down(&mut engine, CAPS);
    down(&mut engine, E);
    assert_eq!(down(&mut engine, P).triggered, vec![2]);
}

#[test]
fn captured_multiple_adorns_use_explicit_order_and_release_only_the_current_adorn() {
    for reverse in [false, true] {
        let mut rules = vec![
            rule(1, Some(E), P, 8),
            rule(2, Some(SPACE), P, 2),
            rule(3, None, P, 30),
        ];
        if reverse {
            rules.reverse();
        }
        let mut engine = Engine::new(rules).unwrap();
        down(&mut engine, CAPS);
        down(&mut engine, E);
        down(&mut engine, SPACE);
        assert_eq!(down(&mut engine, P).triggered, vec![2]);
        up(&mut engine, P);
        up(&mut engine, SPACE);
        assert_eq!(down(&mut engine, P).triggered, vec![1]);
        up(&mut engine, P);
        up(&mut engine, E);
        assert_eq!(down(&mut engine, P).triggered, vec![3]);
    }
}

#[test]
fn captured_action_wins_before_capture_and_terminal_repeat_does_not_capture_it() {
    let mut short = rule(1, None, E, 10);
    short.process_scope.allow = vec!["editor".into()];
    let mut engine = Engine::new(vec![short, rule(2, Some(E), P, 0)]).unwrap();
    // E 的指定应用动作也会改变 P 的附加键是否能被捕获，进程未知时整层不猜测。
    assert_eq!(down(&mut engine, CAPS), Decision::default());
    up(&mut engine, CAPS);
    engine.set_foreground_process(Some("editor")).unwrap();
    down(&mut engine, CAPS);
    assert_eq!(down(&mut engine, E).triggered, vec![1]);
    assert_eq!(down(&mut engine, E).triggered, vec![1]);
    quiet(down(&mut engine, P));
    for key in [P, E, CAPS] {
        up(&mut engine, key);
    }
    engine.set_foreground_process(Some("other")).unwrap();
    down(&mut engine, CAPS);
    quiet(down(&mut engine, E));
    assert_eq!(down(&mut engine, P).triggered, vec![2]);
}

#[test]
fn captured_repeats_reselect_current_rules_and_can_keep_an_existing_capture_role() {
    let mut engine = Engine::new(vec![
        rule(1, Some(E), P, 1),
        rule(2, Some(SPACE), E, 0),
        rule(3, None, P, 10),
    ])
    .unwrap();
    down(&mut engine, CAPS);
    quiet(down(&mut engine, E));
    assert_eq!(down(&mut engine, P).triggered, vec![1]);
    quiet(down(&mut engine, SPACE));
    // E 在首次按下时已经捕获；重复 E 触发 Space+E，不撤销原 E 捕获。
    assert_eq!(down(&mut engine, E).triggered, vec![2]);
    assert_eq!(down(&mut engine, P).triggered, vec![1]);
    up(&mut engine, E);
    assert_eq!(down(&mut engine, P).triggered, vec![3]);
}

#[test]
fn captured_disabled_repeat_does_not_fall_through_to_a_lower_rule() {
    let mut higher = rule(1, Some(E), P, 0);
    higher.keyboard.repeat = false;
    let mut engine = Engine::new(vec![higher, rule(2, None, P, 10)]).unwrap();
    down(&mut engine, CAPS);
    down(&mut engine, E);
    assert_eq!(down(&mut engine, P).triggered, vec![1]);
    quiet(down(&mut engine, P));
}

#[test]
fn captured_cancel_replace_and_leader_release_drain_without_recapturing_old_presses() {
    for mode in 0..3 {
        let rules = vec![rule(1, Some(E), P, 0)];
        let mut engine = Engine::new(rules.clone()).unwrap();
        down(&mut engine, CAPS);
        down(&mut engine, E);
        match mode {
            0 => {
                engine.cancel();
            }
            1 => {
                engine.replace_bindings(rules).unwrap();
            }
            _ => {
                up(&mut engine, CAPS);
            }
        }
        assert!(down(&mut engine, P).triggered.is_empty());
        up(&mut engine, P);
        quiet(up(&mut engine, E));
        up(&mut engine, CAPS);
        down(&mut engine, CAPS);
        down(&mut engine, E);
        assert_eq!(down(&mut engine, P).triggered, vec![1]);
    }
}

#[test]
fn captured_injected_events_cannot_capture_release_or_reselect_actions() {
    let mut engine = Engine::new(vec![rule(1, Some(E), P, 0), rule(2, None, TAB, 1)]).unwrap();
    down(&mut engine, CAPS);
    assert_eq!(
        engine.process(InputEvent {
            kind: InputKind::Down(E),
            injected: true
        }),
        Decision::default()
    );
    quiet(down(&mut engine, P));
    up(&mut engine, P);
    down(&mut engine, E);
    engine.process(InputEvent {
        kind: InputKind::Up(E),
        injected: true,
    });
    assert_eq!(down(&mut engine, P).triggered, vec![1]);
}

#[test]
fn captured_conditions_revoke_the_capture_owner_until_a_fresh_press() {
    let mut conditional = rule(1, Some(E), P, 0);
    conditional.conditions = vec![TriggerCondition::PointerRegion(PointerRegion::Rectangle(
        ScreenRect {
            left: 0,
            top: 0,
            right: 100,
            bottom: 100,
        },
    ))];
    let mut engine = Engine::new(vec![conditional, rule(2, None, TAB, 1)]).unwrap();
    engine.pointer_moved((5, 5), 0);
    down(&mut engine, CAPS);
    down(&mut engine, E);
    engine.pointer_moved((150, 5), 1);
    engine.pointer_moved((5, 5), 2);
    quiet(down(&mut engine, P));
    up(&mut engine, P);
    quiet(up(&mut engine, E));
    down(&mut engine, E);
    assert_eq!(down(&mut engine, P).triggered, vec![1]);
}

#[test]
fn captured_validation_requires_explicit_layer_supported_shape_and_unique_order() {
    let base = rule(1, Some(E), P, 0);
    for invalid in [
        TriggerSpec {
            keyboard: KeyboardPolicy {
                dedicated_prefix: None,
                ..base.keyboard.clone()
            },
            ..base.clone()
        },
        TriggerSpec {
            held: vec![CAPS, E, SPACE],
            ..base.clone()
        },
        TriggerSpec {
            activation: Activation::Hold {
                button: P,
                duration_ms: 300,
            },
            ..base.clone()
        },
        TriggerSpec {
            keyboard: KeyboardPolicy {
                matching: KeyboardMatching::Captured { priority: 512 },
                ..base.keyboard.clone()
            },
            ..base.clone()
        },
    ] {
        assert!(matches!(
            Engine::new(vec![invalid]),
            Err(RegistrationError::InvalidCapturedMatching { .. })
        ));
    }
    assert!(matches!(
        Engine::new(vec![base.clone(), rule(2, Some(SPACE), P, 0)]),
        Err(RegistrationError::CapturedPriorityConflict { .. })
    ));
    assert!(Engine::new(vec![base.clone(), rule(2, Some(E), P, 1)]).is_ok());
    let mut exact = rule(2, None, TAB, 1);
    exact.keyboard.matching = KeyboardMatching::Exact;
    assert!(matches!(
        Engine::new(vec![base, exact]),
        Err(RegistrationError::ConflictingPrefixPolicy { .. })
    ));
}
