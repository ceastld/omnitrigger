//! 正向快照修饰终键与普通鼠标前缀的条件分支；不查询系统状态。
use omnitrigger_core::*;

const X1: Button = Button::Mouse(MouseButton::X1);
const RIGHT: Button = Button::Mouse(MouseButton::Right);

fn prefix(id: u64, activation: Activation) -> TriggerSpec {
    TriggerSpec {
        id,
        held: vec![X1],
        activation,
        conditions: vec![],
        pointer_scope: Default::default(),
        process_scope: Default::default(),
        keyboard: Default::default(),
        wheel: WheelPolicy::Notch,
    }
}

fn terminal(required: ModifierRequirement) -> TriggerSpec {
    TriggerSpec {
        id: 2,
        held: vec![],
        activation: Activation::Press(X1),
        conditions: vec![],
        pointer_scope: Default::default(),
        process_scope: Default::default(),
        wheel: WheelPolicy::Notch,
        keyboard: KeyboardPolicy {
            matching: KeyboardMatching::MouseSampled {
                policy: MouseModifierPolicy {
                    required,
                    allow_other_modifiers: false,
                    allow_extra_passed_keys: true,
                    allow_extra_passed_mouse: false,
                },
                priority: 17,
                pointer_scope_priority: false,
            },
            ..Default::default()
        },
    }
}

fn setup(bindings: Vec<TriggerSpec>) -> Engine {
    let mut engine = Engine::new(bindings).unwrap();
    engine.configure_mouse_modifier_snapshot(9, 16).unwrap();
    engine
}

fn input(engine: &mut Engine, kind: InputKind, at: u64) -> Decision {
    engine.process_at(
        InputEvent {
            kind,
            injected: false,
        },
        at,
    )
}

fn sampled_down(engine: &mut Engine, serial: u64, at: u64, modifiers: ModifierSides) -> Decision {
    let epoch = engine.mouse_modifier_snapshot_epoch().unwrap();
    engine.process_with_mouse_snapshot_at(
        InputEvent {
            kind: InputKind::Down(X1),
            injected: false,
        },
        at,
        MouseModifierInput {
            stream_id: 9,
            serial,
            epoch,
            snapshot: Some(MouseModifierSnapshot {
                stream_id: 9,
                event_serial: serial,
                epoch,
                sampled_at_ms: at,
                modifiers,
            }),
        },
    )
}

#[test]
fn no_control_sample_keeps_x1_then_right_prefix_and_consumes_both_releases() {
    let mut engine = setup(vec![
        prefix(1, Activation::Press(RIGHT)),
        terminal(ModifierRequirement::Control),
    ]);
    let pending = sampled_down(&mut engine, 1, 10, ModifierSides::default());
    assert!(pending.consume && pending.triggered.is_empty() && pending.replay.is_empty());
    assert!(
        input(&mut engine, InputKind::Down(X1), 11)
            .triggered
            .is_empty()
    );
    let action = input(&mut engine, InputKind::Down(RIGHT), 12);
    assert!(action.consume);
    assert_eq!(action.triggered, [1]);
    assert!(input(&mut engine, InputKind::Up(RIGHT), 13).consume);
    assert!(input(&mut engine, InputKind::Up(X1), 14).consume);
}

#[test]
fn control_sample_on_empty_ledger_preempts_prefix_and_terminal_is_never_borrowed() {
    let mut engine = setup(vec![
        prefix(1, Activation::Press(RIGHT)),
        terminal(ModifierRequirement::Control),
    ]);
    let hit = sampled_down(
        &mut engine,
        1,
        10,
        ModifierSides {
            left_control: true,
            ..Default::default()
        },
    );
    assert!(hit.consume);
    assert_eq!(hit.triggered, [2]);
    let right = input(&mut engine, InputKind::Down(RIGHT), 11);
    assert!(!right.consume && right.triggered.is_empty());
    assert!(!input(&mut engine, InputKind::Up(RIGHT), 12).consume);
    let repeated = input(&mut engine, InputKind::Down(X1), 13);
    assert!(repeated.consume && repeated.triggered.is_empty());
    assert!(input(&mut engine, InputKind::Up(X1), 14).consume);
    let next = sampled_down(&mut engine, 2, 15, ModifierSides::default());
    assert!(next.consume && next.triggered.is_empty());
    assert_eq!(
        input(&mut engine, InputKind::Down(RIGHT), 16).triggered,
        [1]
    );
    assert!(input(&mut engine, InputKind::Up(RIGHT), 17).consume);
    assert!(input(&mut engine, InputKind::Up(X1), 18).consume);
}

#[test]
fn all_three_plain_x1_prefix_targets_remain_reachable_with_original_order_numbers() {
    let bindings = vec![
        prefix(3, Activation::Wheel(WheelDirection::Up)),
        prefix(4, Activation::Wheel(WheelDirection::Down)),
        prefix(1, Activation::Press(RIGHT)),
        terminal(ModifierRequirement::Control),
    ];
    for (kind, expected) in [
        (
            InputKind::Wheel {
                axis: Axis::Vertical,
                delta: 120,
            },
            3,
        ),
        (
            InputKind::Wheel {
                axis: Axis::Vertical,
                delta: -120,
            },
            4,
        ),
        (InputKind::Down(RIGHT), 1),
    ] {
        let mut engine = setup(bindings.clone());
        assert!(
            sampled_down(&mut engine, 1, 10, ModifierSides::default())
                .triggered
                .is_empty()
        );
        assert_eq!(input(&mut engine, kind, 11).triggered, [expected]);
        if let InputKind::Down(button) = kind {
            assert!(input(&mut engine, InputKind::Up(button), 12).consume);
        }
        assert!(input(&mut engine, InputKind::Up(X1), 13).consume);
    }
}

#[test]
fn left_alt_positive_condition_has_the_same_two_distinct_branches() {
    for active in [false, true] {
        let mut engine = setup(vec![
            prefix(1, Activation::Press(RIGHT)),
            terminal(ModifierRequirement::LeftAlt),
        ]);
        let first = sampled_down(
            &mut engine,
            1,
            10,
            ModifierSides {
                left_alt: active,
                ..Default::default()
            },
        );
        assert_eq!(first.triggered, if active { vec![2] } else { vec![] });
        let right = input(&mut engine, InputKind::Down(RIGHT), 11);
        assert_eq!(right.triggered, if active { vec![] } else { vec![1] });
        assert_eq!(
            input(&mut engine, InputKind::Up(RIGHT), 12).consume,
            !active
        );
        assert!(input(&mut engine, InputKind::Up(X1), 13).consume);
    }
}

#[test]
fn no_csa_and_unconditional_terminals_still_protect_the_canonical_prefix_entry() {
    for matching in [
        KeyboardMatching::Exact,
        KeyboardMatching::MouseExact {
            priority: 17,
            pointer_scope_priority: false,
        },
        KeyboardMatching::MouseUnmodified {
            priority: 17,
            pointer_scope_priority: false,
        },
        terminal(ModifierRequirement::NoControlShiftAlt)
            .keyboard
            .matching,
    ] {
        let mut direct = terminal(ModifierRequirement::Control);
        direct.keyboard.matching = matching;
        assert!(matches!(
            Engine::new(vec![prefix(1, Activation::Press(RIGHT)), direct]),
            Err(RegistrationError::TerminalIsPrefix {
                terminal_binding: 2,
                prefix_binding: 1
            })
        ));
    }
    let mut alone = setup(vec![terminal(ModifierRequirement::NoControlShiftAlt)]);
    assert_eq!(
        sampled_down(&mut alone, 1, 10, ModifierSides::default()).triggered,
        [2]
    );
}

#[test]
fn positive_exception_does_not_relax_multi_held_or_keyboard_prefix_targets() {
    let mut multiple = prefix(1, Activation::Press(RIGHT));
    multiple.held.push(Button::Key(65));
    assert!(matches!(
        Engine::new(vec![multiple, terminal(ModifierRequirement::Control)]),
        Err(RegistrationError::TerminalIsPrefix {
            terminal_binding: 2,
            prefix_binding: 1
        })
    ));
    let keyboard = prefix(1, Activation::Press(Button::Key(65)));
    assert!(matches!(
        Engine::new(vec![keyboard, terminal(ModifierRequirement::Control)]),
        Err(RegistrationError::TerminalIsPrefix {
            terminal_binding: 2,
            prefix_binding: 1
        })
    ));
}

#[test]
fn unavailable_sample_never_triggers_positive_terminal_and_legacy_prefix_still_has_its_path() {
    let mut engine = setup(vec![
        prefix(1, Activation::Press(RIGHT)),
        terminal(ModifierRequirement::Control),
    ]);
    let pending = input(&mut engine, InputKind::Down(X1), 10);
    assert!(pending.consume && pending.triggered.is_empty());
    assert_eq!(
        input(&mut engine, InputKind::Down(RIGHT), 11).triggered,
        [1]
    );
    assert!(input(&mut engine, InputKind::Up(RIGHT), 12).consume);
    assert!(input(&mut engine, InputKind::Up(X1), 13).consume);
}

#[test]
fn every_public_positive_requirement_preserves_canonical_entry_and_owns_terminal_release() {
    for (required, modifiers) in [
        (
            ModifierRequirement::Shift,
            ModifierSides {
                left_shift: true,
                ..Default::default()
            },
        ),
        (
            ModifierRequirement::Control,
            ModifierSides {
                left_control: true,
                ..Default::default()
            },
        ),
        (
            ModifierRequirement::Alt,
            ModifierSides {
                left_alt: true,
                ..Default::default()
            },
        ),
        (
            ModifierRequirement::LeftShift,
            ModifierSides {
                left_shift: true,
                ..Default::default()
            },
        ),
        (
            ModifierRequirement::RightShift,
            ModifierSides {
                right_shift: true,
                ..Default::default()
            },
        ),
        (
            ModifierRequirement::LeftControl,
            ModifierSides {
                left_control: true,
                ..Default::default()
            },
        ),
        (
            ModifierRequirement::RightControl,
            ModifierSides {
                right_control: true,
                ..Default::default()
            },
        ),
        (
            ModifierRequirement::LeftAlt,
            ModifierSides {
                left_alt: true,
                ..Default::default()
            },
        ),
        (
            ModifierRequirement::RightAlt,
            ModifierSides {
                right_alt: true,
                ..Default::default()
            },
        ),
        (
            ModifierRequirement::LeftWindows,
            ModifierSides {
                left_windows: true,
                ..Default::default()
            },
        ),
        (
            ModifierRequirement::RightWindows,
            ModifierSides {
                right_windows: true,
                ..Default::default()
            },
        ),
    ] {
        for flags in 0..8 {
            for active in [false, true] {
                let mut direct = terminal(required);
                if let KeyboardMatching::MouseSampled { policy, .. } = &mut direct.keyboard.matching
                {
                    policy.allow_other_modifiers = flags & 1 != 0;
                    policy.allow_extra_passed_keys = flags & 2 != 0;
                    policy.allow_extra_passed_mouse = flags & 4 != 0;
                }
                let mut engine = setup(vec![prefix(1, Activation::Press(RIGHT)), direct]);
                let first = sampled_down(
                    &mut engine,
                    1,
                    10,
                    if active {
                        modifiers
                    } else {
                        ModifierSides::default()
                    },
                );
                assert!(first.consume);
                assert_eq!(first.triggered, if active { vec![2] } else { vec![] });
                let repeat = input(&mut engine, InputKind::Down(X1), 11);
                assert!(repeat.consume && repeat.triggered.is_empty());
                let right = input(&mut engine, InputKind::Down(RIGHT), 12);
                assert_eq!(right.triggered, if active { vec![] } else { vec![1] });
                assert_eq!(right.consume, !active);
                assert_eq!(
                    input(&mut engine, InputKind::Up(RIGHT), 13).consume,
                    !active
                );
                assert!(input(&mut engine, InputKind::Up(X1), 14).consume);
            }
        }
    }
}
