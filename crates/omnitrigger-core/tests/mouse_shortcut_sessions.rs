//! 共享鼠标前缀的范围撤销、重复与释放契约；使用合成进程，不读取系统状态。
use omnitrigger_core::*;

const X1: Button = Button::Mouse(MouseButton::X1);
const RIGHT: Button = Button::Mouse(MouseButton::Right);
const DENY: [&str; 2] = ["blocked", "remotehost"];

fn original_rules(id_offset: u64) -> Vec<TriggerSpec> {
    [
        (201, Activation::Wheel(WheelDirection::Up)),
        (202, Activation::Wheel(WheelDirection::Down)),
        (203, Activation::Press(RIGHT)),
    ]
    .into_iter()
    .map(|(id, activation)| TriggerSpec {
        id: id + id_offset,
        held: vec![X1],
        activation,
        conditions: vec![],
        pointer_scope: ProcessScope {
            identity: ProcessIdentity::ShellContext,
            allow: vec![],
            deny: DENY.into_iter().map(str::to_owned).collect(),
        },
        process_scope: Default::default(),
        keyboard: Default::default(),
        wheel: if id == 203 {
            WheelPolicy::Notch
        } else {
            WheelPolicy::Packet
        },
    })
    .collect()
}

fn ready() -> Engine {
    let mut engine = Engine::new(original_rules(0)).unwrap();
    engine
        .set_foreground_context(Some("acceptancereceiver.exe"), WindowRole::Application)
        .unwrap();
    engine
        .set_pointer_context(Some("acceptancereceiver.exe"), WindowRole::Application)
        .unwrap();
    engine
}

fn physical(engine: &mut Engine, kind: InputKind, at: u64) -> Decision {
    engine.process_at(
        InputEvent {
            kind,
            injected: false,
        },
        at,
    )
}

fn wheel(engine: &mut Engine, delta: i32, at: u64) -> Decision {
    physical(
        engine,
        InputKind::Wheel {
            axis: Axis::Vertical,
            delta,
        },
        at,
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
fn three_mouse_shortcuts_share_one_prefix_across_packet_direction_changes_and_right_repeat() {
    let mut engine = ready();
    assert_eq!(physical(&mut engine, InputKind::Down(X1), 1), owned());
    for (at, delta, id) in [(2, 1, 201), (3, 240, 201), (4, -1, 202), (5, -240, 202)] {
        assert_eq!(wheel(&mut engine, delta, at), hit(id));
    }
    assert_eq!(wheel(&mut engine, 0, 6), Decision::default());
    assert_eq!(physical(&mut engine, InputKind::Down(RIGHT), 7), hit(203));
    assert_eq!(physical(&mut engine, InputKind::Down(RIGHT), 8), owned());
    assert_eq!(physical(&mut engine, InputKind::Up(RIGHT), 9), owned());
    assert_eq!(wheel(&mut engine, 1, 10), hit(201));
    assert_eq!(physical(&mut engine, InputKind::Up(X1), 11), owned());
    assert_eq!(
        physical(&mut engine, InputKind::Down(RIGHT), 12),
        Decision::default()
    );
    assert_eq!(
        physical(&mut engine, InputKind::Up(RIGHT), 13),
        Decision::default()
    );
}

#[test]
fn pause_after_close_drains_both_old_presses_in_either_release_order_before_new_prefix() {
    for releases in [[RIGHT, X1], [X1, RIGHT]] {
        let mut engine = ready();
        assert_eq!(physical(&mut engine, InputKind::Down(X1), 1), owned());
        assert_eq!(physical(&mut engine, InputKind::Down(RIGHT), 2), hit(203));
        engine.set_enabled(false);
        engine.set_enabled(true);
        assert_eq!(physical(&mut engine, InputKind::Down(X1), 3), owned());
        assert_eq!(physical(&mut engine, InputKind::Down(RIGHT), 4), owned());
        assert_eq!(wheel(&mut engine, -1, 5), Decision::default());
        for (index, button) in releases.into_iter().enumerate() {
            assert_eq!(
                physical(&mut engine, InputKind::Up(button), 6 + index as u64),
                owned()
            );
        }
        assert_eq!(physical(&mut engine, InputKind::Down(X1), 8), owned());
        assert_eq!(wheel(&mut engine, -1, 9), hit(202));
        assert_eq!(physical(&mut engine, InputKind::Up(X1), 10), owned());
    }
}

#[test]
fn pointer_unknown_or_denied_before_commit_replays_x1_once_and_cannot_borrow_it_back() {
    for (name, role) in [
        (None, WindowRole::Unknown),
        (Some("blocked.exe"), WindowRole::Application),
    ] {
        let mut engine = ready();
        assert_eq!(physical(&mut engine, InputKind::Down(X1), 1), owned());
        assert_eq!(
            engine.set_pointer_context(name, role).unwrap(),
            Decision {
                replay: vec![InputKind::Down(X1)],
                ..Default::default()
            }
        );
        assert_eq!(
            engine
                .set_pointer_context(Some("acceptancereceiver.exe"), WindowRole::Application)
                .unwrap(),
            Decision::default()
        );
        assert_eq!(
            physical(&mut engine, InputKind::Down(X1), 3),
            Decision::default()
        );
        assert_eq!(wheel(&mut engine, -1, 4), Decision::default());
        assert_eq!(
            physical(&mut engine, InputKind::Down(RIGHT), 5),
            Decision::default()
        );
        assert_eq!(
            physical(&mut engine, InputKind::Up(RIGHT), 6),
            Decision::default()
        );
        assert_eq!(
            physical(&mut engine, InputKind::Up(X1), 7),
            Decision::default()
        );
        assert_eq!(physical(&mut engine, InputKind::Down(X1), 8), owned());
        assert_eq!(physical(&mut engine, InputKind::Down(RIGHT), 9), hit(203));
        assert_eq!(physical(&mut engine, InputKind::Up(RIGHT), 10), owned());
        assert_eq!(physical(&mut engine, InputKind::Up(X1), 11), owned());
    }
}

#[test]
fn replacing_three_rules_does_not_transfer_a_committed_prefix_to_new_binding_ids() {
    let mut engine = ready();
    assert_eq!(physical(&mut engine, InputKind::Down(X1), 1), owned());
    assert_eq!(wheel(&mut engine, 1, 2), hit(201));
    assert_eq!(
        engine.replace_bindings(original_rules(1000)).unwrap(),
        Decision::default()
    );
    assert_eq!(physical(&mut engine, InputKind::Down(X1), 3), owned());
    assert_eq!(wheel(&mut engine, -1, 4), Decision::default());
    assert_eq!(
        physical(&mut engine, InputKind::Down(RIGHT), 5),
        Decision::default()
    );
    assert_eq!(
        physical(&mut engine, InputKind::Up(RIGHT), 6),
        Decision::default()
    );
    assert_eq!(physical(&mut engine, InputKind::Up(X1), 7), owned());
    assert_eq!(physical(&mut engine, InputKind::Down(X1), 8), owned());
    assert_eq!(wheel(&mut engine, -1, 9), hit(1202));
    assert_eq!(physical(&mut engine, InputKind::Down(RIGHT), 10), hit(1203));
    assert_eq!(physical(&mut engine, InputKind::Up(RIGHT), 11), owned());
    assert_eq!(physical(&mut engine, InputKind::Up(X1), 12), owned());
}

// 已提交动作的释放配对与范围撤销分别验证。
// 本路未执行，不能因源码推断或其他四项通过而把本场景写成通过。

#[test]
fn committed_scope_loss_keeps_consumed_x1_and_right_releases_owned_in_both_orders() {
    for (name, role) in [
        (None, WindowRole::Unknown),
        (Some("blocked.exe"), WindowRole::Application),
    ] {
        for releases in [vec![X1], vec![RIGHT, X1], vec![X1, RIGHT]] {
            let close = releases.len() == 2;
            let mut engine = ready();
            assert_eq!(physical(&mut engine, InputKind::Down(X1), 1), owned());
            if close {
                assert_eq!(physical(&mut engine, InputKind::Down(RIGHT), 2), hit(203));
            } else {
                assert_eq!(wheel(&mut engine, 1, 2), hit(201));
            }
            // 已提交，不把已消费按下补发给另一范围，也不重新执行已提交动作。
            assert_eq!(
                engine.set_pointer_context(name, role).unwrap(),
                Decision::default()
            );
            assert_eq!(physical(&mut engine, InputKind::Down(X1), 3), owned());
            if close {
                assert_eq!(physical(&mut engine, InputKind::Down(RIGHT), 4), owned());
            }
            for (index, button) in releases.into_iter().enumerate() {
                assert_eq!(
                    physical(&mut engine, InputKind::Up(button), 5 + index as u64),
                    owned()
                );
            }
            // 释放只能收尾一次，重复的Up不能继续声称拥有。
            assert_eq!(
                physical(&mut engine, InputKind::Up(X1), 7),
                Decision::default()
            );
            if close {
                assert_eq!(
                    physical(&mut engine, InputKind::Up(RIGHT), 8),
                    Decision::default()
                );
            }
            engine
                .set_pointer_context(Some("acceptancereceiver.exe"), WindowRole::Application)
                .unwrap();
            assert_eq!(physical(&mut engine, InputKind::Down(X1), 9), owned());
            assert_eq!(wheel(&mut engine, -1, 10), hit(202));
            assert_eq!(physical(&mut engine, InputKind::Up(X1), 11), owned());
        }
    }
}

#[test]
fn committed_scope_loss_cannot_rearm_old_x1_when_scope_returns_before_physical_release() {
    for (name, role) in [
        (None, WindowRole::Unknown),
        (Some("blocked.exe"), WindowRole::Application),
    ] {
        for close in [false, true] {
            let mut engine = ready();
            assert_eq!(physical(&mut engine, InputKind::Down(X1), 1), owned());
            if close {
                assert_eq!(physical(&mut engine, InputKind::Down(RIGHT), 2), hit(203));
            } else {
                assert_eq!(wheel(&mut engine, 1, 2), hit(201));
            }
            assert_eq!(
                engine.set_pointer_context(name, role).unwrap(),
                Decision::default()
            );
            if close {
                assert_eq!(physical(&mut engine, InputKind::Up(RIGHT), 3), owned());
            }
            engine
                .set_pointer_context(Some("acceptancereceiver.exe"), WindowRole::Application)
                .unwrap();
            assert_eq!(physical(&mut engine, InputKind::Down(X1), 4), owned());
            // 范围恢复不能把已撤销的旧按压重新当作新前缀。
            assert_eq!(wheel(&mut engine, -1, 5), Decision::default());
            assert_eq!(physical(&mut engine, InputKind::Up(X1), 6), owned());
            assert_eq!(physical(&mut engine, InputKind::Down(X1), 7), owned());
            assert_eq!(wheel(&mut engine, -1, 8), hit(202));
            assert_eq!(physical(&mut engine, InputKind::Up(X1), 9), owned());
        }
    }
}

#[test]
fn losing_one_scope_keeps_another_original_owner_and_passed_key_alive() {
    let mut rules = original_rules(0);
    rules[1].pointer_scope = Default::default();
    let mut engine = Engine::new(rules).unwrap();
    engine
        .set_foreground_context(Some("acceptancereceiver.exe"), WindowRole::Application)
        .unwrap();
    engine
        .set_pointer_context(Some("acceptancereceiver.exe"), WindowRole::Application)
        .unwrap();
    assert_eq!(physical(&mut engine, InputKind::Down(X1), 1), owned());
    assert_eq!(wheel(&mut engine, 1, 2), hit(201));
    assert_eq!(
        engine
            .set_pointer_context(Some("blocked.exe"), WindowRole::Application)
            .unwrap(),
        Decision::default()
    );
    assert_eq!(wheel(&mut engine, -1, 3), hit(202));
    let letter = Button::Key(65);
    assert_eq!(
        physical(&mut engine, InputKind::Down(letter), 4),
        Decision::default()
    );
    assert_eq!(
        engine
            .set_pointer_context(None, WindowRole::Unknown)
            .unwrap(),
        Decision::default()
    );
    assert_eq!(
        physical(&mut engine, InputKind::Up(letter), 5),
        Decision::default()
    );
    assert_eq!(wheel(&mut engine, -1, 6), hit(202));
    assert_eq!(physical(&mut engine, InputKind::Up(X1), 7), owned());
}
