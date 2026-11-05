//! 保留层恢复、原按压次序和已消费按压收尾的事件回归。
use omnitrigger_core::*;

const SPACE: Button = Button::Key(32);
const H: Button = Button::Key(72);
const UNKNOWN: Button = Button::Key(0x87);
const LEFT: Button = Button::Mouse(MouseButton::Left);
const X2: Button = Button::Mouse(MouseButton::X2);

fn owner() -> TriggerSpec {
    TriggerSpec {
        id: 1,
        held: vec![SPACE],
        activation: Activation::Press(H),
        keyboard: KeyboardPolicy {
            matching: KeyboardMatching::Captured { priority: 0 },
            retained_prefix: Some(Box::new(RetainedPrefix {
                key: SPACE,
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

fn engine() -> Engine {
    let watcher = TriggerSpec {
        id: 2,
        held: vec![SPACE],
        activation: Activation::Press(X2),
        keyboard: KeyboardPolicy {
            matching: KeyboardMatching::MouseCapturedLeader {
                priority: 1,
                pointer_scope_priority: false,
            },
            ..Default::default()
        },
        wheel: Default::default(),
        conditions: vec![],
        process_scope: Default::default(),
        pointer_scope: Default::default(),
    };
    Engine::new(vec![owner(), watcher]).unwrap()
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

fn consumed() -> Decision {
    Decision {
        consume: true,
        ..Default::default()
    }
}

fn restored(engine: &mut Engine) {
    assert_eq!(input(engine, InputKind::Down(SPACE), 1000), consumed());
    assert_eq!(
        input(engine, InputKind::Down(LEFT), 1060),
        Decision {
            replay: vec![InputKind::Down(SPACE)],
            ..Default::default()
        }
    );
}

#[test]
fn mouse_restored_leader_release_passes_without_a_second_tap() {
    let mut e = engine();
    restored(&mut e);
    assert_eq!(
        input(&mut e, InputKind::Up(SPACE), 1100),
        Decision::default()
    );
    assert_eq!(
        input(&mut e, InputKind::Up(LEFT), 1101),
        Decision::default()
    );
}

#[test]
fn mouse_restored_leader_repeat_passes_to_the_application() {
    let mut e = engine();
    restored(&mut e);
    assert_eq!(
        input(&mut e, InputKind::Down(SPACE), 1061),
        Decision::default()
    );
    assert_eq!(
        input(&mut e, InputKind::Up(SPACE), 1100),
        Decision::default()
    );
}

#[test]
fn mouse_recovery_ends_keyboard_interception_but_preserves_mouse_leader_identity() {
    let mut e = engine();
    restored(&mut e);
    assert_eq!(input(&mut e, InputKind::Down(H), 1061), Decision::default());
    assert_eq!(input(&mut e, InputKind::Up(H), 1062), Decision::default());
    assert_eq!(
        input(&mut e, InputKind::Up(LEFT), 1063),
        Decision::default()
    );
    assert_eq!(
        input(&mut e, InputKind::Down(X2), 1064),
        Decision {
            consume: true,
            triggered: vec![2],
            ..Default::default()
        }
    );
    assert_eq!(input(&mut e, InputKind::Up(X2), 1065), consumed());
    assert_eq!(
        input(&mut e, InputKind::Up(SPACE), 1066),
        Decision::default()
    );
}

#[test]
fn unknown_key_after_mouse_restore_does_not_replay_leader_or_destroy_mouse_identity() {
    let mut e = engine();
    restored(&mut e);
    assert_eq!(
        input(&mut e, InputKind::Down(UNKNOWN), 1061),
        Decision::default()
    );
    assert_eq!(
        input(&mut e, InputKind::Up(LEFT), 1062),
        Decision::default()
    );
    assert_eq!(
        input(&mut e, InputKind::Down(X2), 1063),
        Decision {
            consume: true,
            triggered: vec![2],
            ..Default::default()
        }
    );
    assert_eq!(input(&mut e, InputKind::Up(X2), 1064), consumed());
    assert_eq!(
        input(&mut e, InputKind::Up(SPACE), 1065),
        Decision::default()
    );
    assert_eq!(
        input(&mut e, InputKind::Up(UNKNOWN), 1066),
        Decision::default()
    );
}

#[test]
fn keyboard_press_consumed_before_recovery_remains_owned_until_release() {
    let mut e = engine();
    input(&mut e, InputKind::Down(SPACE), 1000);
    assert_eq!(input(&mut e, InputKind::Down(H), 1060).triggered, vec![1]);
    assert_eq!(
        input(&mut e, InputKind::Down(LEFT), 1061).replay,
        vec![InputKind::Down(SPACE)]
    );
    assert_eq!(input(&mut e, InputKind::Down(H), 1062), consumed());
    assert_eq!(
        input(&mut e, InputKind::Up(SPACE), 1063),
        Decision::default()
    );
    assert_eq!(input(&mut e, InputKind::Up(H), 1064), consumed());
    assert_eq!(
        input(&mut e, InputKind::Up(LEFT), 1065),
        Decision::default()
    );
}

#[test]
fn injection_cannot_restore_or_cancel_a_physical_leader() {
    let mut e = engine();
    input(&mut e, InputKind::Down(SPACE), 1000);
    assert_eq!(
        e.process_at(
            InputEvent {
                kind: InputKind::Down(LEFT),
                injected: true
            },
            9000
        ),
        Decision::default()
    );
    assert_eq!(input(&mut e, InputKind::Down(H), 1060).triggered, vec![1]);
    assert!(input(&mut e, InputKind::Up(H), 1061).consume);
    assert!(input(&mut e, InputKind::Up(SPACE), 1062).consume);
}

#[test]
fn explicit_cancel_after_mouse_restore_does_not_replay_or_rearm_the_leader() {
    let mut e = engine();
    restored(&mut e);
    e.cancel();
    assert_eq!(
        input(&mut e, InputKind::Up(SPACE), 1100),
        Decision::default()
    );
    assert_eq!(
        input(&mut e, InputKind::Down(X2), 1101),
        Decision::default()
    );
    assert_eq!(input(&mut e, InputKind::Up(X2), 1102), Decision::default());
}

#[test]
fn mouse_recovery_restores_pending_and_captured_adorns_without_replaying_consumed_actions() {
    let adorn = Button::Key(71);
    for adorn_time in [1010, 1060] {
        let mut nested = owner();
        nested.id = 3;
        nested.held.push(adorn);
        nested.activation = Activation::Press(Button::Key(73));
        nested.keyboard.matching = KeyboardMatching::Captured { priority: 1 };
        let mut e = Engine::new(vec![owner(), nested]).unwrap();
        assert_eq!(input(&mut e, InputKind::Down(SPACE), 1000), consumed());
        assert_eq!(
            input(&mut e, InputKind::Down(adorn), adorn_time),
            consumed()
        );
        assert_eq!(
            input(&mut e, InputKind::Down(LEFT), adorn_time + 1),
            Decision {
                replay: vec![InputKind::Down(SPACE), InputKind::Down(adorn)],
                ..Default::default()
            }
        );
        assert_eq!(
            input(&mut e, InputKind::Up(adorn), adorn_time + 2),
            Decision::default()
        );
        assert_eq!(
            input(&mut e, InputKind::Up(SPACE), adorn_time + 3),
            Decision::default()
        );
    }
}

fn multiple_adorns() -> (Engine, Button, Button) {
    let high = Button::Key(90);
    let low = Button::Key(65);
    let mut bindings = vec![owner()];
    for (id, adorn, terminal) in [(3, high, Button::Key(73)), (4, low, Button::Key(74))] {
        let mut nested = owner();
        nested.id = id;
        nested.held.push(adorn);
        nested.activation = Activation::Press(terminal);
        nested.keyboard.matching = KeyboardMatching::Captured {
            priority: id as u16,
        };
        bindings.push(nested);
    }
    (Engine::new(bindings).unwrap(), high, low)
}

#[test]
fn multiple_adorns_restore_in_physical_down_order_even_when_vk_order_is_reversed() {
    let (mut e, high, low) = multiple_adorns();
    input(&mut e, InputKind::Down(SPACE), 1000);
    assert_eq!(input(&mut e, InputKind::Down(high), 1060), consumed());
    assert_eq!(input(&mut e, InputKind::Down(low), 1061), consumed());
    // 重复按下不应把high移动到low之后。
    assert_eq!(input(&mut e, InputKind::Down(high), 1062), consumed());
    assert_eq!(
        input(&mut e, InputKind::Down(LEFT), 1063),
        Decision {
            replay: vec![
                InputKind::Down(SPACE),
                InputKind::Down(high),
                InputKind::Down(low)
            ],
            ..Default::default()
        }
    );
    for (offset, button) in [low, high, SPACE, LEFT].into_iter().enumerate() {
        assert_eq!(
            input(&mut e, InputKind::Up(button), 1070 + offset as u64),
            Decision::default()
        );
    }
}

#[test]
fn release_and_repress_append_adorn_to_the_end_of_the_current_physical_order() {
    let (mut e, high, low) = multiple_adorns();
    input(&mut e, InputKind::Down(SPACE), 1000);
    input(&mut e, InputKind::Down(high), 1060);
    input(&mut e, InputKind::Down(low), 1061);
    assert_eq!(input(&mut e, InputKind::Up(high), 1062), consumed());
    assert_eq!(input(&mut e, InputKind::Down(high), 1063), consumed());
    assert_eq!(input(&mut e, InputKind::Down(high), 1064), consumed());
    assert_eq!(
        input(&mut e, InputKind::Down(LEFT), 1065).replay,
        vec![
            InputKind::Down(SPACE),
            InputKind::Down(low),
            InputKind::Down(high)
        ]
    );
    for (offset, button) in [SPACE, high, low, LEFT].into_iter().enumerate() {
        assert_eq!(
            input(&mut e, InputKind::Up(button), 1070 + offset as u64),
            Decision::default()
        );
    }
}

#[test]
fn temporary_adorn_repeats_do_not_create_duplicate_restore_downs() {
    let (mut e, high, _) = multiple_adorns();
    input(&mut e, InputKind::Down(SPACE), 1000);
    assert_eq!(input(&mut e, InputKind::Down(high), 1010), consumed());
    assert_eq!(input(&mut e, InputKind::Down(high), 1011), consumed());
    assert_eq!(
        input(&mut e, InputKind::Down(LEFT), 1012).replay,
        vec![InputKind::Down(SPACE), InputKind::Down(high)]
    );
    assert_eq!(
        input(&mut e, InputKind::Up(high), 1013),
        Decision::default()
    );
    assert_eq!(
        input(&mut e, InputKind::Up(SPACE), 1014),
        Decision::default()
    );
}

#[test]
fn unmatched_fast_third_key_falls_back_before_mouse_restore_even_if_it_is_another_adorn() {
    // 不是把第三键追加到adorn列表：来源pending分支ResolveAction失败后立即回退。
    for third_is_adorn in [false, true] {
        let (mut e, high, low) = multiple_adorns();
        let third = if third_is_adorn { low } else { UNKNOWN };
        input(&mut e, InputKind::Down(SPACE), 1000);
        input(&mut e, InputKind::Down(high), 1010);
        assert_eq!(
            input(&mut e, InputKind::Down(third), 1011),
            Decision {
                replay: vec![InputKind::Down(SPACE), InputKind::Down(high)],
                ..Default::default()
            }
        );
        assert_eq!(
            input(&mut e, InputKind::Down(LEFT), 1012),
            Decision::default()
        );
        for (offset, button) in [SPACE, third, high, LEFT].into_iter().enumerate() {
            assert_eq!(
                input(&mut e, InputKind::Up(button), 1020 + offset as u64),
                Decision::default()
            );
        }
    }
}

#[test]
fn cancellation_and_new_session_do_not_retain_previous_adorn_order() {
    let (mut e, high, low) = multiple_adorns();
    input(&mut e, InputKind::Down(SPACE), 1000);
    input(&mut e, InputKind::Down(high), 1060);
    input(&mut e, InputKind::Down(low), 1061);
    e.cancel();
    for key in [SPACE, high, low] {
        assert_eq!(input(&mut e, InputKind::Up(key), 1070), consumed());
    }
    input(&mut e, InputKind::Down(SPACE), 1200);
    input(&mut e, InputKind::Down(low), 1260);
    assert_eq!(
        input(&mut e, InputKind::Down(LEFT), 1261).replay,
        vec![InputKind::Down(SPACE), InputKind::Down(low)]
    );
}

#[test]
fn preexisting_passed_adorn_repeat_cannot_enter_the_restore_order() {
    let (mut e, high, _) = multiple_adorns();
    assert_eq!(input(&mut e, InputKind::Down(high), 0), Decision::default());
    assert_eq!(input(&mut e, InputKind::Down(SPACE), 1000), consumed());
    assert_eq!(
        input(&mut e, InputKind::Down(high), 1010),
        Decision::default()
    );
    assert_eq!(
        input(&mut e, InputKind::Down(LEFT), 1060).replay,
        vec![InputKind::Down(SPACE)]
    );
    assert_eq!(
        input(&mut e, InputKind::Up(high), 1061),
        Decision::default()
    );
    assert_eq!(
        input(&mut e, InputKind::Up(SPACE), 1062),
        Decision::default()
    );
}

#[test]
fn temporary_adorn_release_without_bare_action_restores_typing_and_ends_mouse_identity() {
    let (mut e, adorn, _) = multiple_adorns();
    input(&mut e, InputKind::Down(SPACE), 1000);
    assert_eq!(input(&mut e, InputKind::Down(adorn), 1010), consumed());
    assert_eq!(
        input(&mut e, InputKind::Up(adorn), 1011),
        Decision {
            consume: true,
            replay: vec![
                InputKind::Down(SPACE),
                InputKind::Down(adorn),
                InputKind::Up(adorn),
            ],
            ..Decision::default()
        }
    );
    assert_eq!(
        input(&mut e, InputKind::Down(LEFT), 1012),
        Decision::default()
    );
    assert_eq!(
        input(&mut e, InputKind::Up(SPACE), 1013),
        Decision::default()
    );
    assert_eq!(
        input(&mut e, InputKind::Up(LEFT), 1014),
        Decision::default()
    );
}

#[test]
fn leader_released_before_temporary_adorn_keeps_that_adorn_down_until_physical_release() {
    let (mut e, adorn, _) = multiple_adorns();
    input(&mut e, InputKind::Down(SPACE), 1000);
    input(&mut e, InputKind::Down(adorn), 1010);
    assert_eq!(
        input(&mut e, InputKind::Up(SPACE), 1011),
        Decision {
            consume: true,
            replay: vec![
                InputKind::Down(SPACE),
                InputKind::Up(SPACE),
                InputKind::Down(adorn)
            ],
            ..Decision::default()
        }
    );
    assert_eq!(
        input(&mut e, InputKind::Up(adorn), 1012),
        Decision::default()
    );
}

#[test]
fn captured_adorn_repeat_inside_fast_window_cannot_become_pending_again() {
    let (mut e, adorn, _) = multiple_adorns();
    let terminal = Button::Key(73);
    input(&mut e, InputKind::Down(SPACE), 1000);
    input(&mut e, InputKind::Down(adorn), 1010);
    assert_eq!(
        input(&mut e, InputKind::Down(terminal), 1011).triggered,
        vec![3]
    );
    assert_eq!(input(&mut e, InputKind::Up(terminal), 1012), consumed());
    assert_eq!(input(&mut e, InputKind::Down(adorn), 1013), consumed());
    assert_eq!(input(&mut e, InputKind::Up(SPACE), 1014), consumed());
    assert_eq!(input(&mut e, InputKind::Up(adorn), 1015), consumed());
}
