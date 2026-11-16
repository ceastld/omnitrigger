//! 待定X1前缀的原规则失效后，先回退，不允许新范围接手旧按压。
use omnitrigger_core::*;

const X1: Button = Button::Mouse(MouseButton::X1);
const RIGHT: Button = Button::Mouse(MouseButton::Right);

fn rule(id: u64, activation: Activation, scope: Option<&str>) -> TriggerSpec {
    TriggerSpec {
        id,
        held: vec![X1],
        activation,
        conditions: vec![],
        process_scope: Default::default(),
        pointer_scope: ProcessScope {
            allow: scope.into_iter().map(str::to_owned).collect(),
            ..Default::default()
        },
        wheel: if matches!(activation, Activation::Wheel(_)) {
            WheelPolicy::Packet
        } else {
            WheelPolicy::Notch
        },
        keyboard: Default::default(),
    }
}

fn rules() -> Vec<TriggerSpec> {
    vec![
        rule(201, Activation::Wheel(WheelDirection::Up), Some("first")),
        rule(202, Activation::Wheel(WheelDirection::Down), Some("second")),
        rule(203, Activation::Press(RIGHT), Some("second")),
    ]
}

fn scope(engine: &mut Engine, name: Option<&str>) -> Decision {
    engine
        .set_pointer_context(name, WindowRole::Application)
        .unwrap()
}

fn ready() -> Engine {
    let mut engine = Engine::new(rules()).unwrap();
    scope(&mut engine, Some("first"));
    engine
}

fn physical(engine: &mut Engine, kind: InputKind) -> Decision {
    engine.process_at(
        InputEvent {
            kind,
            injected: false,
        },
        1,
    )
}

fn down(engine: &mut Engine, button: Button) -> Decision {
    physical(engine, InputKind::Down(button))
}

fn up(engine: &mut Engine, button: Button) -> Decision {
    physical(engine, InputKind::Up(button))
}

fn wheel(engine: &mut Engine, delta: i32) -> Decision {
    physical(
        engine,
        InputKind::Wheel {
            axis: Axis::Vertical,
            delta,
        },
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

fn replay_down() -> Decision {
    Decision {
        replay: vec![InputKind::Down(X1)],
        ..Default::default()
    }
}

#[test]
fn pending_x1_replays_before_new_pointer_scope_and_cannot_be_borrowed_back() {
    let mut engine = ready();
    assert_eq!(down(&mut engine, X1), owned());
    assert_eq!(down(&mut engine, X1), owned());
    assert_eq!(scope(&mut engine, Some("second")), replay_down());
    assert_eq!(scope(&mut engine, Some("first")), Decision::default());
    assert_eq!(scope(&mut engine, Some("second")), Decision::default());
    assert_eq!(down(&mut engine, X1), Decision::default());
    assert_eq!(wheel(&mut engine, -1), Decision::default());
    assert_eq!(down(&mut engine, RIGHT), Decision::default());
    assert_eq!(up(&mut engine, RIGHT), Decision::default());
    assert_eq!(up(&mut engine, X1), Decision::default());
    assert_eq!(down(&mut engine, X1), owned());
    assert_eq!(wheel(&mut engine, -1), hit(202));
    assert_eq!(up(&mut engine, X1), owned());
}

#[test]
fn new_scope_mouse_terminal_waits_for_fresh_x1_then_owns_both_release_orders() {
    for releases in [[X1, RIGHT], [RIGHT, X1]] {
        let mut engine = ready();
        assert_eq!(down(&mut engine, X1), owned());
        assert_eq!(scope(&mut engine, Some("second")), replay_down());
        assert_eq!(up(&mut engine, X1), Decision::default());
        assert_eq!(down(&mut engine, X1), owned());
        assert_eq!(down(&mut engine, RIGHT), hit(203));
        assert_eq!(down(&mut engine, RIGHT), owned());
        for button in releases {
            assert_eq!(up(&mut engine, button), owned());
            assert_eq!(up(&mut engine, button), Decision::default());
        }
    }
}

#[test]
fn committed_x1_is_drained_without_replay_or_new_scope_rearming() {
    let mut engine = ready();
    assert_eq!(down(&mut engine, X1), owned());
    assert_eq!(wheel(&mut engine, 1), hit(201));
    assert_eq!(scope(&mut engine, Some("second")), Decision::default());
    assert_eq!(down(&mut engine, X1), owned());
    assert_eq!(wheel(&mut engine, -1), Decision::default());
    assert_eq!(up(&mut engine, X1), owned());
    assert_eq!(down(&mut engine, X1), owned());
    assert_eq!(wheel(&mut engine, -1), hit(202));
    assert_eq!(up(&mut engine, X1), owned());
}

#[test]
fn surviving_original_owner_stays_usable_while_new_owner_cannot_join() {
    for use_survivor in [false, true] {
        let mut rules = rules();
        rules.push(rule(204, Activation::Wheel(WheelDirection::Right), None));
        let mut engine = Engine::new(rules).unwrap();
        scope(&mut engine, Some("first"));
        assert_eq!(down(&mut engine, X1), owned());
        assert_eq!(scope(&mut engine, Some("second")), Decision::default());
        if use_survivor {
            // 原来的全局横向规则仍有效，可以提交原X1，而不准入新的纵向规则。
            assert_eq!(
                physical(
                    &mut engine,
                    InputKind::Wheel {
                        axis: Axis::Horizontal,
                        delta: 1
                    }
                ),
                hit(204)
            );
        }
        assert_eq!(
            wheel(&mut engine, -1),
            if use_survivor {
                Decision::default()
            } else {
                replay_down()
            }
        );
        assert_eq!(
            up(&mut engine, X1),
            if use_survivor {
                owned()
            } else {
                Decision::default()
            }
        );
        assert_eq!(down(&mut engine, X1), owned());
        assert_eq!(wheel(&mut engine, -1), hit(202));
        assert_eq!(up(&mut engine, X1), owned());
    }
}

#[test]
fn injected_release_or_down_never_rebuilds_the_replayed_physical_session() {
    let mut engine = ready();
    assert_eq!(down(&mut engine, X1), owned());
    assert_eq!(scope(&mut engine, Some("second")), replay_down());
    for kind in [
        InputKind::Up(X1),
        InputKind::Down(X1),
        InputKind::Down(RIGHT),
    ] {
        assert_eq!(
            engine.process_at(
                InputEvent {
                    kind,
                    injected: true
                },
                2
            ),
            Decision::default()
        );
    }
    assert_eq!(wheel(&mut engine, -1), Decision::default());
    assert_eq!(up(&mut engine, X1), Decision::default());
    assert_eq!(down(&mut engine, X1), owned());
    assert_eq!(wheel(&mut engine, -1), hit(202));
    assert_eq!(up(&mut engine, X1), owned());
}

#[test]
fn cancel_disable_replace_and_focus_changes_drain_pending_x1_without_transferring_it() {
    for boundary in 0..4 {
        let mut engine = ready();
        assert_eq!(down(&mut engine, X1), owned());
        match boundary {
            0 => {
                engine.cancel();
            }
            1 => {
                engine.set_enabled(false);
                engine.set_enabled(true);
            }
            2 => {
                engine.replace_bindings(rules()).unwrap();
            }
            _ => {
                engine.set_foreground_process(Some("other")).unwrap();
            }
        }
        assert_eq!(scope(&mut engine, Some("second")), Decision::default());
        assert_eq!(down(&mut engine, X1), owned());
        assert_eq!(wheel(&mut engine, -1), Decision::default());
        assert_eq!(up(&mut engine, X1), owned());
        assert_eq!(down(&mut engine, X1), owned());
        assert_eq!(wheel(&mut engine, -1), hit(202));
        assert_eq!(up(&mut engine, X1), owned());
    }
}

#[test]
fn original_three_shared_scope_rules_keep_tap_repeat_fallback_and_scope_loss_pairing() {
    let original: Vec<_> = rules()
        .into_iter()
        .map(|mut binding| {
            binding.pointer_scope.allow = vec!["first".into()];
            binding
        })
        .collect();
    let mut engine = Engine::new(original).unwrap();
    scope(&mut engine, Some("first"));
    assert_eq!(down(&mut engine, X1), owned());
    assert_eq!(down(&mut engine, X1), owned());
    assert_eq!(
        up(&mut engine, X1),
        Decision {
            consume: true,
            replay: vec![InputKind::Down(X1), InputKind::Up(X1)],
            ..Default::default()
        }
    );
    assert_eq!(down(&mut engine, X1), owned());
    assert_eq!(scope(&mut engine, None), replay_down());
    scope(&mut engine, Some("first"));
    assert_eq!(wheel(&mut engine, 1), Decision::default());
    assert_eq!(up(&mut engine, X1), Decision::default());
    assert_eq!(down(&mut engine, X1), owned());
    assert_eq!(wheel(&mut engine, 1), hit(201));
    assert_eq!(wheel(&mut engine, -1), hit(202));
    assert_eq!(down(&mut engine, RIGHT), hit(203));
    assert_eq!(down(&mut engine, RIGHT), owned());
    assert_eq!(up(&mut engine, RIGHT), owned());
    assert_eq!(up(&mut engine, X1), owned());
}
