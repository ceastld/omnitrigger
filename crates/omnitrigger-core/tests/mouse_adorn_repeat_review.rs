use omnitrigger_core::*;

fn rule(id: u64, adorn: Option<Button>, terminal: Button) -> TriggerSpec {
    let space = Button::Key(32);
    TriggerSpec {
        id,
        held: std::iter::once(space).chain(adorn).collect(),
        activation: Activation::Press(terminal),
        keyboard: KeyboardPolicy {
            matching: KeyboardMatching::Captured {
                priority: id as u16,
            },
            retained_prefix: Some(Box::new(RetainedPrefix {
                key: space,
                capture_scope: Default::default(),
                quiet_ms: 50,
                fast_ms: 50,
                block_with_primary_mouse: false,
            })),
            restore_on_tap: true,
            tap_timeout_ms: Some(1500),
            ..Default::default()
        },
        wheel: Default::default(),
        conditions: vec![],
        process_scope: Default::default(),
        pointer_scope: Default::default(),
    }
}

fn input(e: &mut Engine, kind: InputKind, at: u64) -> Decision {
    e.process_at(
        InputEvent {
            kind,
            injected: false,
        },
        at,
    )
}

#[test]
fn captured_adorn_repeat_keeps_its_role_and_original_restore_order() {
    let space = Button::Key(32);
    let high = Button::Key(90);
    let low = Button::Key(65);
    let left = Button::Mouse(MouseButton::Left);
    let mut e = Engine::new(vec![
        rule(1, None, Button::Key(72)),
        rule(3, Some(high), Button::Key(73)),
        rule(4, Some(low), high),
    ])
    .unwrap();
    input(&mut e, InputKind::Down(space), 1000);
    input(&mut e, InputKind::Down(high), 1060);
    input(&mut e, InputKind::Down(low), 1061);
    assert_eq!(
        input(&mut e, InputKind::Down(high), 1062),
        Decision {
            consume: true,
            ..Default::default()
        }
    );
    assert_eq!(
        input(&mut e, InputKind::Down(left), 1063).replay,
        vec![
            InputKind::Down(space),
            InputKind::Down(high),
            InputKind::Down(low)
        ]
    );
    for (offset, key) in [space, high, low, left].into_iter().enumerate() {
        assert_eq!(
            input(&mut e, InputKind::Up(key), 1070 + offset as u64),
            Decision::default()
        );
    }
}
