use crate::*;

const CAPS: Button = Button::Key(20);
const SPACE: Button = Button::Key(32);
const E: Button = Button::Key(69);
const P: Button = Button::Key(80);
const J: Button = Button::Key(74);
const RIGHT: Button = Button::Mouse(MouseButton::Right);

fn spec(id: u64, held: &[Button], activation: Activation, dedicated: bool) -> TriggerSpec {
    TriggerSpec {
        pointer_scope: Default::default(),
        wheel: Default::default(),
        id,
        held: held.to_vec(),
        activation,
        conditions: vec![],
        process_scope: Default::default(),
        keyboard: KeyboardPolicy {
            dedicated_prefix: dedicated.then_some(CAPS),
            ..Default::default()
        },
    }
}

fn input(engine: &mut Engine, kind: InputKind) -> Decision {
    engine.process(InputEvent {
        kind,
        injected: false,
    })
}

fn down(engine: &mut Engine, button: Button) -> Decision {
    input(engine, InputKind::Down(button))
}

fn up(engine: &mut Engine, button: Button) -> Decision {
    input(engine, InputKind::Up(button))
}

#[test]
fn terminal_role_registration_allows_distinct_held_contexts_but_rejects_early_actions() {
    for dedicated in [false, true] {
        for activation in [
            Activation::Press(E),
            Activation::Hold {
                button: E,
                duration_ms: 300,
            },
        ] {
            let terminal = spec(1, &[CAPS, SPACE], activation, dedicated);
            assert!(
                Engine::new(vec![
                    terminal.clone(),
                    spec(2, &[CAPS, E], Activation::Press(P), dedicated),
                ])
                .is_ok()
            );
            for held in [&[CAPS, SPACE, E][..], &[CAPS, SPACE, E, J][..]] {
                assert!(matches!(
                    Engine::new(vec![
                        terminal.clone(),
                        spec(2, held, Activation::Press(P), dedicated),
                    ]),
                    Err(RegistrationError::TerminalIsPrefix {
                        terminal_binding: 1,
                        prefix_binding: 2,
                    })
                ));
            }
        }
    }
}

#[test]
fn terminal_role_requires_a_fresh_press_before_reusing_a_terminal_as_prefix() {
    for dedicated in [false, true] {
        let mut engine = Engine::new(vec![
            spec(1, &[CAPS, SPACE], Activation::Press(E), dedicated),
            spec(2, &[CAPS, E], Activation::Press(P), dedicated),
        ])
        .unwrap();
        down(&mut engine, CAPS);
        down(&mut engine, SPACE);
        assert_eq!(down(&mut engine, E).triggered, vec![1]);
        assert!(up(&mut engine, SPACE).consume);
        // 自动重复不能把动作末键变成附加前缀。
        let repeated = down(&mut engine, E);
        assert!(repeated.consume && repeated.triggered.is_empty());
        let blocked = down(&mut engine, P);
        assert!(blocked.triggered.is_empty() && blocked.replay.is_empty());
        assert_eq!(blocked.consume, dedicated);
        assert_eq!(up(&mut engine, P).consume, dedicated);
        assert!(up(&mut engine, E).consume);
        down(&mut engine, E);
        assert_eq!(down(&mut engine, P).triggered, vec![2]);
        for button in [P, E, CAPS] {
            let release = up(&mut engine, button);
            assert!(release.consume && release.replay.is_empty());
        }
    }
}

#[test]
fn terminal_role_blocks_prefix_extension_and_wheel_reuse_after_a_terminal_action() {
    for dedicated in [false, true] {
        let mut engine = Engine::new(vec![
            spec(1, &[CAPS, SPACE], Activation::Press(E), dedicated),
            spec(2, &[CAPS, E, J], Activation::Press(P), dedicated),
            spec(
                3,
                &[CAPS, E],
                Activation::Wheel(WheelDirection::Up),
                dedicated,
            ),
        ])
        .unwrap();
        down(&mut engine, CAPS);
        down(&mut engine, SPACE);
        assert_eq!(down(&mut engine, E).triggered, vec![1]);
        up(&mut engine, SPACE);
        let extension = down(&mut engine, J);
        assert_eq!(extension.consume, dedicated);
        assert!(extension.replay.is_empty());
        assert!(down(&mut engine, P).triggered.is_empty());
        up(&mut engine, P);
        up(&mut engine, J);
        let wheel = input(
            &mut engine,
            InputKind::Wheel {
                axis: Axis::Vertical,
                delta: 120,
            },
        );
        assert_eq!(wheel, Decision::default());
        up(&mut engine, E);
        down(&mut engine, E);
        let wheel = input(
            &mut engine,
            InputKind::Wheel {
                axis: Axis::Vertical,
                delta: 120,
            },
        );
        assert_eq!(wheel.triggered, vec![3]);
    }
}

#[test]
fn terminal_role_preserves_repeat_owner_without_lending_the_key_to_another_action() {
    let mut terminal = spec(1, &[CAPS, SPACE], Activation::Press(E), true);
    terminal.keyboard.repeat = true;
    let mut engine = Engine::new(vec![
        terminal,
        spec(2, &[CAPS, E], Activation::Press(P), true),
    ])
    .unwrap();
    down(&mut engine, CAPS);
    down(&mut engine, SPACE);
    assert_eq!(down(&mut engine, E).triggered, vec![1]);
    assert_eq!(down(&mut engine, E).triggered, vec![1]);
    up(&mut engine, SPACE);
    assert!(down(&mut engine, E).triggered.is_empty());
    assert!(down(&mut engine, P).triggered.is_empty());
    up(&mut engine, P);
    up(&mut engine, E);
    down(&mut engine, E);
    assert_eq!(down(&mut engine, P).triggered, vec![2]);
}

#[test]
fn terminal_role_reserves_timed_terminals_before_and_after_they_fire() {
    for fire in [false, true] {
        let mut engine = Engine::new(vec![
            spec(
                1,
                &[CAPS, SPACE],
                Activation::Hold {
                    button: E,
                    duration_ms: 300,
                },
                true,
            ),
            spec(2, &[CAPS, E], Activation::Press(P), true),
        ])
        .unwrap();
        down(&mut engine, CAPS);
        down(&mut engine, SPACE);
        assert!(down(&mut engine, E).triggered.is_empty());
        if fire {
            assert_eq!(engine.advance_time(300).triggered, vec![1]);
        }
        up(&mut engine, SPACE);
        assert!(engine.advance_time(400).triggered.is_empty());
        assert!(down(&mut engine, P).triggered.is_empty());
        up(&mut engine, P);
        assert!(up(&mut engine, E).consume);
        down(&mut engine, E);
        assert_eq!(down(&mut engine, P).triggered, vec![2]);
    }
}

#[test]
fn terminal_role_drag_reservation_and_completed_drag_never_become_prefixes() {
    for fire in [false, true] {
        let mut engine = Engine::new(vec![
            spec(
                1,
                &[CAPS, SPACE],
                Activation::Drag {
                    button: RIGHT,
                    min_distance_px: 4,
                    max_start_ms: 200,
                },
                true,
            ),
            spec(2, &[CAPS, RIGHT], Activation::Press(P), true),
        ])
        .unwrap();
        engine.pointer_moved((0, 0), 0);
        down(&mut engine, CAPS);
        down(&mut engine, SPACE);
        down(&mut engine, RIGHT);
        if fire {
            assert_eq!(engine.pointer_moved((5, 0), 1).triggered, vec![1]);
        }
        up(&mut engine, SPACE);
        assert!(down(&mut engine, P).triggered.is_empty());
        up(&mut engine, P);
        // 未启动的鼠标拖拽按原语义回放；已启动的鼠标拖拽消费释放。
        let release = up(&mut engine, RIGHT);
        assert_eq!(release.consume, fire);
        up(&mut engine, CAPS);
        down(&mut engine, CAPS);
        down(&mut engine, RIGHT);
        assert_eq!(down(&mut engine, P).triggered, vec![2]);
    }
}

#[test]
fn terminal_role_cancel_and_binding_replacement_drain_old_presses() {
    for replace in [false, true] {
        let bindings = vec![
            spec(1, &[CAPS, SPACE], Activation::Press(E), true),
            spec(2, &[CAPS, E], Activation::Press(P), true),
        ];
        let mut engine = Engine::new(bindings.clone()).unwrap();
        down(&mut engine, CAPS);
        down(&mut engine, SPACE);
        assert_eq!(down(&mut engine, E).triggered, vec![1]);
        if replace {
            engine.replace_bindings(bindings).unwrap();
        } else {
            engine.cancel();
        }
        up(&mut engine, SPACE);
        assert!(down(&mut engine, P).triggered.is_empty());
        up(&mut engine, P);
        for key in [E, CAPS] {
            assert!(up(&mut engine, key).consume);
        }
        down(&mut engine, CAPS);
        down(&mut engine, E);
        assert_eq!(down(&mut engine, P).triggered, vec![2]);
    }
}

#[test]
fn terminal_role_injected_release_does_not_clear_the_physical_terminal_role() {
    let mut engine = Engine::new(vec![
        spec(1, &[CAPS, SPACE], Activation::Press(E), true),
        spec(2, &[CAPS, E], Activation::Press(P), true),
    ])
    .unwrap();
    down(&mut engine, CAPS);
    down(&mut engine, SPACE);
    down(&mut engine, E);
    up(&mut engine, SPACE);
    assert_eq!(
        engine.process(InputEvent {
            kind: InputKind::Up(E),
            injected: true
        }),
        Decision::default()
    );
    assert!(down(&mut engine, P).triggered.is_empty());
    up(&mut engine, P);
    up(&mut engine, E);
    down(&mut engine, E);
    assert_eq!(down(&mut engine, P).triggered, vec![2]);
}
