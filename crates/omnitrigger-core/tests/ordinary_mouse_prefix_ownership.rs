//! 普通键与鼠标组合的范围所有权；只有显式事件和合成上下文。
use omnitrigger_core::*;

const CAPS: Button = Button::Key(20);
const SPACE: Button = Button::Key(32);
const W: Button = Button::Key(87);
const X1: Button = Button::Mouse(MouseButton::X1);
const X2: Button = Button::Mouse(MouseButton::X2);

fn rule(id: u64, prefix: Button, activation: Activation, scoped: bool) -> TriggerSpec {
    TriggerSpec {
        id,
        held: vec![prefix],
        activation,
        conditions: vec![],
        process_scope: Default::default(),
        pointer_scope: if scoped {
            ProcessScope {
                deny: vec!["blocked".into()],
                ..Default::default()
            }
        } else {
            Default::default()
        },
        wheel: if matches!(activation, Activation::Wheel(_)) {
            WheelPolicy::Packet
        } else {
            WheelPolicy::Notch
        },
        keyboard: Default::default(),
    }
}

fn rules(prefix: Button, keyboard: bool) -> Vec<TriggerSpec> {
    let mut rules = vec![
        rule(198, prefix, Activation::Wheel(WheelDirection::Down), true),
        rule(199, prefix, Activation::Wheel(WheelDirection::Up), true),
        rule(208, prefix, Activation::Press(X2), true),
    ];
    if keyboard {
        rules.push(rule(9, prefix, Activation::Press(W), false));
    }
    rules
}

fn scope(engine: &mut Engine, name: Option<&str>) -> Decision {
    engine
        .set_pointer_context(name, WindowRole::Application)
        .unwrap()
}

fn ready(prefix: Button, keyboard: bool) -> Engine {
    let mut engine = Engine::new(rules(prefix, keyboard)).unwrap();
    scope(&mut engine, Some("receiver"));
    engine
}

fn event(engine: &mut Engine, kind: InputKind, injected: bool) -> Decision {
    engine.process_at(InputEvent { kind, injected }, 1)
}

fn down(engine: &mut Engine, button: Button) -> Decision {
    event(engine, InputKind::Down(button), false)
}

fn up(engine: &mut Engine, button: Button) -> Decision {
    event(engine, InputKind::Up(button), false)
}

fn wheel(engine: &mut Engine, delta: i32) -> Decision {
    event(
        engine,
        InputKind::Wheel {
            axis: Axis::Vertical,
            delta,
        },
        false,
    )
}

fn owned() -> Decision {
    Decision {
        consume: true,
        ..Default::default()
    }
}

fn hit(id: u64) -> Decision {
    Decision {
        consume: true,
        triggered: vec![id],
        ..Default::default()
    }
}

#[test]
fn caps_packet_directions_and_existing_keyboard_shortcut_share_the_original_press() {
    let mut engine = ready(CAPS, true);
    assert_eq!(down(&mut engine, CAPS), owned());
    assert_eq!(down(&mut engine, CAPS), owned());
    assert_eq!(wheel(&mut engine, -1), hit(198));
    assert_eq!(wheel(&mut engine, 240), hit(199));
    assert_eq!(down(&mut engine, W), hit(9));
    assert_eq!(down(&mut engine, W), owned());
    assert_eq!(up(&mut engine, W), owned());
    assert_eq!(down(&mut engine, X2), hit(208));
    assert_eq!(down(&mut engine, X2), owned());
    assert_eq!(up(&mut engine, X2), owned());
    assert_eq!(up(&mut engine, CAPS), owned());
    assert_eq!(up(&mut engine, CAPS), Decision::default());
}

#[test]
fn committed_ordinary_prefix_cannot_rearm_when_pointer_scope_returns() {
    for prefix in [CAPS, SPACE] {
        for unavailable in [None, Some("blocked")] {
            let mut engine = ready(prefix, false);
            assert_eq!(down(&mut engine, prefix), owned());
            assert_eq!(wheel(&mut engine, -1), hit(198));
            assert_eq!(scope(&mut engine, unavailable), Decision::default());
            assert_eq!(scope(&mut engine, Some("receiver")), Decision::default());
            assert_eq!(down(&mut engine, prefix), owned());
            assert_eq!(wheel(&mut engine, 1), Decision::default());
            assert_eq!(down(&mut engine, X2), Decision::default());
            assert_eq!(up(&mut engine, X2), Decision::default());
            assert_eq!(up(&mut engine, prefix), owned());
            assert_eq!(down(&mut engine, prefix), owned());
            assert_eq!(wheel(&mut engine, 1), hit(199));
            assert_eq!(up(&mut engine, prefix), owned());
        }
    }
}

#[test]
fn revoked_mouse_owner_stays_revoked_while_original_keyboard_owner_survives() {
    let mut engine = ready(CAPS, true);
    assert_eq!(down(&mut engine, CAPS), owned());
    assert_eq!(wheel(&mut engine, -1), hit(198));
    assert_eq!(scope(&mut engine, Some("blocked")), Decision::default());
    assert_eq!(down(&mut engine, W), hit(9));
    assert_eq!(up(&mut engine, W), owned());
    assert_eq!(scope(&mut engine, Some("receiver")), Decision::default());
    assert_eq!(wheel(&mut engine, 1), Decision::default());
    assert_eq!(down(&mut engine, X2), Decision::default());
    assert_eq!(up(&mut engine, X2), Decision::default());
    assert_eq!(down(&mut engine, W), hit(9));
    assert_eq!(up(&mut engine, W), owned());
    assert_eq!(up(&mut engine, CAPS), owned());
    assert_eq!(down(&mut engine, CAPS), owned());
    assert_eq!(wheel(&mut engine, 1), hit(199));
    assert_eq!(up(&mut engine, CAPS), owned());
}

#[test]
fn pending_key_does_not_admit_new_or_previously_revoked_mouse_owner() {
    for initially_allowed in [false, true] {
        let mut engine = ready(CAPS, true);
        if !initially_allowed {
            scope(&mut engine, Some("blocked"));
        }
        assert_eq!(down(&mut engine, CAPS), owned());
        scope(&mut engine, Some("blocked"));
        scope(&mut engine, Some("receiver"));
        assert_eq!(
            wheel(&mut engine, -1),
            Decision {
                replay: vec![InputKind::Down(CAPS)],
                ..Default::default()
            }
        );
        assert_eq!(down(&mut engine, CAPS), Decision::default());
        assert_eq!(down(&mut engine, W), Decision::default());
        assert_eq!(up(&mut engine, W), Decision::default());
        assert_eq!(up(&mut engine, CAPS), Decision::default());
        assert_eq!(down(&mut engine, CAPS), owned());
        assert_eq!(wheel(&mut engine, -1), hit(198));
        assert_eq!(up(&mut engine, CAPS), owned());
    }
}

#[test]
fn unmatched_tap_and_fallback_preserve_delivered_down_up_pairing() {
    let mut engine = ready(CAPS, false);
    assert_eq!(down(&mut engine, CAPS), owned());
    assert_eq!(
        up(&mut engine, CAPS),
        Decision {
            consume: true,
            replay: vec![InputKind::Down(CAPS), InputKind::Up(CAPS)],
            ..Default::default()
        }
    );
    assert_eq!(down(&mut engine, CAPS), owned());
    assert_eq!(
        down(&mut engine, W),
        Decision {
            replay: vec![InputKind::Down(CAPS)],
            ..Default::default()
        }
    );
    assert_eq!(up(&mut engine, W), Decision::default());
    assert_eq!(wheel(&mut engine, -1), Decision::default());
    assert_eq!(up(&mut engine, CAPS), Decision::default());
}

#[test]
fn injected_events_cannot_create_release_revoke_or_rearm_a_physical_prefix() {
    let mut engine = ready(CAPS, false);
    assert_eq!(
        event(&mut engine, InputKind::Down(CAPS), true),
        Decision::default()
    );
    assert_eq!(wheel(&mut engine, -1), Decision::default());
    assert_eq!(down(&mut engine, CAPS), owned());
    assert_eq!(wheel(&mut engine, -1), hit(198));
    scope(&mut engine, Some("blocked"));
    scope(&mut engine, Some("receiver"));
    for kind in [
        InputKind::Up(CAPS),
        InputKind::Down(CAPS),
        InputKind::Down(X2),
    ] {
        assert_eq!(event(&mut engine, kind, true), Decision::default());
    }
    assert_eq!(wheel(&mut engine, -1), Decision::default());
    assert_eq!(up(&mut engine, CAPS), owned());
    assert_eq!(down(&mut engine, CAPS), owned());
    assert_eq!(wheel(&mut engine, -1), hit(198));
    assert_eq!(up(&mut engine, CAPS), owned());
}

#[test]
fn cancel_disable_replace_and_focus_loss_drain_prefix_and_terminal_in_either_order() {
    for boundary in 0..4 {
        for releases in [[CAPS, X2], [X2, CAPS]] {
            let mut engine = ready(CAPS, true);
            assert_eq!(down(&mut engine, CAPS), owned());
            assert_eq!(down(&mut engine, X2), hit(208));
            match boundary {
                0 => {
                    engine.cancel();
                }
                1 => {
                    engine.set_enabled(false);
                    engine.set_enabled(true);
                }
                2 => {
                    engine.replace_bindings(rules(CAPS, true)).unwrap();
                }
                _ => {
                    engine.set_foreground_process(Some("other")).unwrap();
                    engine.set_foreground_process(Some("receiver")).unwrap();
                }
            }
            assert_eq!(down(&mut engine, CAPS), owned());
            assert_eq!(down(&mut engine, X2), owned());
            assert_eq!(wheel(&mut engine, -1), Decision::default());
            for button in releases {
                assert_eq!(up(&mut engine, button), owned());
                assert_eq!(up(&mut engine, button), Decision::default());
            }
            assert_eq!(down(&mut engine, CAPS), owned());
            assert_eq!(down(&mut engine, X2), hit(208));
            assert_eq!(up(&mut engine, X2), owned());
            assert_eq!(up(&mut engine, CAPS), owned());
        }
    }
}

#[test]
fn x1_mouse_owner_and_left_observation_keep_their_existing_contracts() {
    for prefix in [X1, Button::Mouse(MouseButton::Left)] {
        let mut engine = ready(prefix, false);
        assert_eq!(
            down(&mut engine, prefix),
            if prefix == X1 {
                owned()
            } else {
                Decision::default()
            }
        );
        assert_eq!(wheel(&mut engine, -1), hit(198));
        scope(&mut engine, Some("blocked"));
        scope(&mut engine, Some("receiver"));
        assert_eq!(
            wheel(&mut engine, 1),
            if prefix == X1 {
                Decision::default()
            } else {
                hit(199)
            }
        );
        assert_eq!(
            up(&mut engine, prefix),
            if prefix == X1 {
                owned()
            } else {
                Decision::default()
            }
        );
    }
}

#[test]
fn extra_physical_key_or_mouse_button_does_not_expand_exact_matching() {
    for extra in [Button::Key(162), X1] {
        let mut engine = ready(CAPS, true);
        assert_eq!(down(&mut engine, CAPS), owned());
        assert_eq!(wheel(&mut engine, -1), hit(198));
        assert_eq!(down(&mut engine, extra), Decision::default());
        assert_eq!(wheel(&mut engine, -1), Decision::default());
        assert_eq!(up(&mut engine, extra), Decision::default());
        assert_eq!(wheel(&mut engine, -1), hit(198));
        assert_eq!(up(&mut engine, CAPS), owned());
    }
}

#[test]
fn pending_prefix_losing_all_owners_is_replayed_before_a_different_scope_can_claim_it() {
    let mut first = rule(198, CAPS, Activation::Wheel(WheelDirection::Down), false);
    first.pointer_scope.allow = vec!["first".into()];
    let mut second = rule(199, CAPS, Activation::Wheel(WheelDirection::Up), false);
    second.pointer_scope.allow = vec!["second".into()];
    let mut engine = Engine::new(vec![first, second]).unwrap();
    scope(&mut engine, Some("first"));
    assert_eq!(down(&mut engine, CAPS), owned());
    assert_eq!(
        scope(&mut engine, Some("second")),
        Decision {
            replay: vec![InputKind::Down(CAPS)],
            ..Default::default()
        }
    );
    assert_eq!(wheel(&mut engine, 1), Decision::default());
    assert_eq!(up(&mut engine, CAPS), Decision::default());
    assert_eq!(down(&mut engine, CAPS), owned());
    assert_eq!(wheel(&mut engine, 1), hit(199));
    assert_eq!(up(&mut engine, CAPS), owned());
}
