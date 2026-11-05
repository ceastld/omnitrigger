use omnitrigger_core::*;

const SIDE: Button = Button::Mouse(MouseButton::X1);
const RIGHT: Button = Button::Mouse(MouseButton::Right);
const LEFT: Button = Button::Mouse(MouseButton::Left);
const SPACE: Button = Button::Key(32);
const KEY: Button = Button::Key(72);
const NONE: WheelSessionSnapshot = WheelSessionSnapshot {
    captured_mouse_buttons: false,
    holding_move_key: false,
};
const MOUSE: WheelSessionSnapshot = WheelSessionSnapshot {
    captured_mouse_buttons: true,
    holding_move_key: false,
};

fn spec(id: u64, held: Vec<Button>, activation: Activation) -> TriggerSpec {
    TriggerSpec {
        id,
        held,
        activation,
        keyboard: Default::default(),
        wheel: Default::default(),
        process_scope: Default::default(),
        pointer_scope: Default::default(),
        conditions: vec![],
    }
}

fn wheel(id: u64, held: Vec<Button>, direction: WheelDirection) -> TriggerSpec {
    let mut row = spec(id, held, Activation::Wheel(direction));
    row.wheel = WheelPolicy::Packet;
    row
}

fn drag(held: Vec<Button>, button: Button) -> TriggerSpec {
    spec(
        1,
        held,
        Activation::Drag {
            button,
            min_distance_px: 4,
            max_start_ms: 100,
        },
    )
}

fn input(e: &mut Engine, kind: InputKind, time: u64) -> Decision {
    e.process_at(
        InputEvent {
            kind,
            injected: false,
        },
        time,
    )
}

fn scroll(e: &mut Engine, delta: i32, time: u64) -> Decision {
    input(
        e,
        InputKind::Wheel {
            axis: Axis::Vertical,
            delta,
        },
        time,
    )
}

fn retained_owner() -> TriggerSpec {
    let mut row = spec(2, vec![SPACE], Activation::Press(KEY));
    row.keyboard = KeyboardPolicy {
        matching: KeyboardMatching::Captured { priority: 0 },
        retained_prefix: Some(Box::new(RetainedPrefix {
            key: SPACE,
            capture_scope: Default::default(),
            quiet_ms: 50,
            fast_ms: 50,
            block_with_primary_mouse: false,
        })),
        restore_on_tap: true,
        ..Default::default()
    };
    row
}

#[test]
fn arbitrary_physical_keys_mouse_and_direct_terminals_do_not_make_a_mouse_session() {
    let mut e = Engine::new(vec![]).unwrap();
    for (at, button) in [KEY, LEFT, RIGHT, SIDE].into_iter().enumerate() {
        input(&mut e, InputKind::Down(button), at as u64);
        assert_eq!(e.wheel_session_snapshot(), NONE);
    }
    for button in [RIGHT, SIDE] {
        let mut e = Engine::new(vec![spec(1, vec![], Activation::Press(button))]).unwrap();
        assert!(input(&mut e, InputKind::Down(button), 0).consume);
        assert_eq!(e.wheel_session_snapshot(), NONE);
        assert!(input(&mut e, InputKind::Up(button), 1).consume);
    }
    let mut e = Engine::new(vec![spec(
        1,
        vec![],
        Activation::Hold {
            button: RIGHT,
            duration_ms: 100,
        },
    )])
    .unwrap();
    input(&mut e, InputKind::Down(RIGHT), 0);
    assert_eq!(e.wheel_session_snapshot(), NONE);
    assert_eq!(e.advance_time(100).triggered, vec![1]);
    assert_eq!(e.wheel_session_snapshot(), NONE);
}

#[test]
fn captured_prefix_is_explicit_and_survives_commit_until_its_release() {
    let mut e = Engine::new(vec![wheel(1, vec![SIDE], WheelDirection::Up)]).unwrap();
    assert!(input(&mut e, InputKind::Down(SIDE), 0).consume);
    assert_eq!(e.wheel_session_snapshot(), MOUSE);
    assert_eq!(e.wheel_session_snapshot(), MOUSE); // 读取没有副作用。
    assert!(input(&mut e, InputKind::Down(SIDE), 1).consume);
    assert_eq!(scroll(&mut e, 1, 2).triggered, vec![1]);
    assert_eq!(e.wheel_session_snapshot(), MOUSE);
    assert_eq!(scroll(&mut e, -1, 3), Decision::default());
    assert_eq!(e.wheel_session_snapshot(), MOUSE);
    assert!(input(&mut e, InputKind::Up(SIDE), 4).consume);
    assert_eq!(e.wheel_session_snapshot(), NONE);
}

#[test]
fn observed_primary_prefix_never_claims_captured_mouse_ownership() {
    let mut e = Engine::new(vec![wheel(1, vec![LEFT], WheelDirection::Up)]).unwrap();
    assert!(!input(&mut e, InputKind::Down(LEFT), 0).consume);
    assert_eq!(e.wheel_session_snapshot(), NONE);
    assert_eq!(scroll(&mut e, 1, 1).triggered, vec![1]);
    assert_eq!(e.wheel_session_snapshot(), NONE);
    assert!(!input(&mut e, InputKind::Up(LEFT), 2).consume);
}

#[test]
fn normal_keyboard_layers_and_observer_mouse_terminals_do_not_set_holding_move_key() {
    let mut owner = spec(1, vec![SPACE], Activation::Press(KEY));
    owner.keyboard = KeyboardPolicy {
        dedicated_prefix: Some(SPACE),
        matching: KeyboardMatching::Captured { priority: 0 },
        ..Default::default()
    };
    let mut watcher = spec(3, vec![SPACE], Activation::Press(SIDE));
    watcher.keyboard.matching = KeyboardMatching::MouseCapturedLeader {
        priority: 1,
        pointer_scope_priority: false,
    };
    let mut e = Engine::new(vec![owner, watcher]).unwrap();
    input(&mut e, InputKind::Down(SPACE), 0);
    assert_eq!(e.wheel_session_snapshot(), NONE);
    assert_eq!(input(&mut e, InputKind::Down(SIDE), 1).triggered, vec![3]);
    assert_eq!(e.wheel_session_snapshot(), NONE);
    e.cancel();
    assert_eq!(e.wheel_session_snapshot(), NONE);
    assert!(input(&mut e, InputKind::Up(SIDE), 2).consume);
}

#[test]
fn actual_pending_and_active_mouse_drags_are_sessions_including_primary_button() {
    for button in [LEFT, RIGHT] {
        let mut e = Engine::new(vec![drag(vec![], button)]).unwrap();
        e.pointer_moved((0, 0), 0);
        input(&mut e, InputKind::Down(button), 0);
        assert_eq!(e.wheel_session_snapshot(), MOUSE);
        assert_eq!(
            e.pointer_moved((4, 0), 1).drags[0].phase,
            DragPhase::Started
        );
        assert_eq!(e.wheel_session_snapshot(), MOUSE);
        assert_eq!(
            input(&mut e, InputKind::Up(button), 2).drags[0].phase,
            DragPhase::Ended
        );
        assert_eq!(e.wheel_session_snapshot(), NONE);
    }
}

#[test]
fn keyboard_prefix_plus_mouse_drag_is_a_mouse_session_not_pure_key_movement() {
    let mut e = Engine::new(vec![drag(vec![KEY], RIGHT)]).unwrap();
    e.pointer_moved((0, 0), 0);
    input(&mut e, InputKind::Down(KEY), 0);
    assert_eq!(e.wheel_session_snapshot(), NONE);
    input(&mut e, InputKind::Down(RIGHT), 1);
    assert_eq!(e.wheel_session_snapshot(), MOUSE);
    e.pointer_moved((4, 0), 2);
    let cancelled = input(&mut e, InputKind::Up(KEY), 3);
    assert_eq!(cancelled.drags[0].phase, DragPhase::Cancelled);
    assert_eq!(e.wheel_session_snapshot(), NONE);
    assert!(input(&mut e, InputKind::Up(RIGHT), 4).consume);
    assert!(matches!(
        Engine::new(vec![drag(vec![], KEY)]),
        Err(RegistrationError::InvalidDragButton { .. })
    ));
}

#[test]
fn releasing_a_passed_primary_prefix_cancels_pending_drag_and_restores_terminal_down() {
    let mut e = Engine::new(vec![drag(vec![LEFT], RIGHT)]).unwrap();
    e.pointer_moved((0, 0), 0);
    assert!(!input(&mut e, InputKind::Down(LEFT), 0).consume);
    input(&mut e, InputKind::Down(RIGHT), 1);
    assert_eq!(e.wheel_session_snapshot(), MOUSE);
    let up = input(&mut e, InputKind::Up(LEFT), 2);
    assert!(!up.consume);
    assert_eq!(up.replay, vec![InputKind::Down(RIGHT)]);
    assert_eq!(e.wheel_session_snapshot(), NONE);
    assert!(e.pointer_moved((5, 0), 3).triggered.is_empty());
    assert!(!input(&mut e, InputKind::Up(RIGHT), 4).consume);
}

#[test]
fn drag_timeout_clears_session_even_when_a_hold_terminal_still_waits() {
    let hold = spec(
        2,
        vec![],
        Activation::Hold {
            button: RIGHT,
            duration_ms: 200,
        },
    );
    let mut e = Engine::new(vec![drag(vec![], RIGHT), hold]).unwrap();
    input(&mut e, InputKind::Down(RIGHT), 0);
    assert_eq!(e.wheel_session_snapshot(), MOUSE);
    e.advance_time(100);
    assert_eq!(e.wheel_session_snapshot(), NONE);
    assert_eq!(e.advance_time(200).triggered, vec![2]);
    assert_eq!(e.wheel_session_snapshot(), NONE);
}

#[test]
fn recovery_replay_hands_prefix_ownership_back_without_rearming_old_press() {
    let mut e = Engine::new(vec![wheel(1, vec![SIDE], WheelDirection::Up)]).unwrap();
    e.pointer_moved((0, 0), 0);
    input(&mut e, InputKind::Down(SIDE), 0);
    assert_eq!(e.wheel_session_snapshot(), MOUSE);
    assert_eq!(
        e.pointer_moved((9, 0), 1).replay,
        vec![InputKind::Down(SIDE)]
    );
    assert_eq!(e.wheel_session_snapshot(), NONE);
    assert!(!input(&mut e, InputKind::Down(SIDE), 2).consume);
    assert_eq!(e.wheel_session_snapshot(), NONE);
    input(&mut e, InputKind::Up(SIDE), 3);
    input(&mut e, InputKind::Down(SIDE), 4);
    assert_eq!(e.wheel_session_snapshot(), MOUSE);
}

#[test]
fn cancel_pause_process_screen_and_rebinding_clear_sessions_and_keep_draining_releases_owned() {
    for mode in 0..5 {
        let binding = wheel(1, vec![SIDE], WheelDirection::Up);
        let mut e = Engine::new(vec![binding.clone()]).unwrap();
        input(&mut e, InputKind::Down(SIDE), 0);
        scroll(&mut e, 1, 1);
        assert_eq!(e.wheel_session_snapshot(), MOUSE);
        match mode {
            0 => {
                e.cancel();
            }
            1 => {
                e.set_enabled(false);
                e.set_enabled(true);
            }
            2 => {
                e.set_foreground_process(Some("editor.exe")).unwrap();
            }
            3 => {
                e.set_screens(vec![ScreenRect {
                    left: 0,
                    top: 0,
                    right: 100,
                    bottom: 100,
                }])
                .unwrap();
            }
            _ => {
                e.replace_bindings(vec![binding]).unwrap();
            }
        }
        assert_eq!(e.wheel_session_snapshot(), NONE);
        input(&mut e, InputKind::Down(SIDE), 2);
        assert_eq!(e.wheel_session_snapshot(), NONE);
        assert!(input(&mut e, InputKind::Up(SIDE), 3).consume);
        input(&mut e, InputKind::Down(SIDE), 4);
        assert_eq!(e.wheel_session_snapshot(), MOUSE);
    }
}

#[test]
fn context_loss_cannot_transfer_an_old_session_to_a_different_scope_rule() {
    let mut a = wheel(1, vec![SIDE], WheelDirection::Up);
    a.pointer_scope.allow = vec!["editor".into()];
    let mut b = wheel(2, vec![SIDE], WheelDirection::Up);
    b.pointer_scope.allow = vec!["reader".into()];
    let mut e = Engine::new(vec![a, b]).unwrap();
    e.set_pointer_context(Some("editor"), WindowRole::Application)
        .unwrap();
    input(&mut e, InputKind::Down(SIDE), 0);
    scroll(&mut e, 1, 1);
    e.set_pointer_context(Some("reader"), WindowRole::Application)
        .unwrap();
    assert_eq!(e.wheel_session_snapshot(), NONE);
    e.set_pointer_context(Some("editor"), WindowRole::Application)
        .unwrap();
    input(&mut e, InputKind::Down(SIDE), 2);
    assert_eq!(e.wheel_session_snapshot(), NONE);
    input(&mut e, InputKind::Up(SIDE), 3);
    input(&mut e, InputKind::Down(SIDE), 4);
    assert_eq!(e.wheel_session_snapshot(), MOUSE);
}

#[test]
fn region_revocation_is_per_original_owner_and_never_recreates_lost_owners() {
    let mut a = wheel(1, vec![SIDE], WheelDirection::Up);
    a.conditions = vec![TriggerCondition::PointerRegion(PointerRegion::Rectangle(
        ScreenRect {
            left: 0,
            top: 0,
            right: 10,
            bottom: 10,
        },
    ))];
    for keep_other in [false, true] {
        let mut bindings = vec![a.clone()];
        if keep_other {
            bindings.push(wheel(2, vec![SIDE], WheelDirection::Down));
        }
        let mut e = Engine::new(bindings).unwrap();
        e.pointer_moved((5, 5), 0);
        input(&mut e, InputKind::Down(SIDE), 0);
        scroll(&mut e, 1, 1);
        e.pointer_moved((20, 5), 2);
        assert_eq!(
            e.wheel_session_snapshot(),
            if keep_other { MOUSE } else { NONE }
        );
        e.pointer_moved((5, 5), 3);
        assert_eq!(
            e.wheel_session_snapshot(),
            if keep_other { MOUSE } else { NONE }
        );
    }
}

#[test]
fn every_captured_mouse_slot_is_independent_until_last_release() {
    let buttons = vec![
        RIGHT,
        Button::Mouse(MouseButton::Middle),
        SIDE,
        Button::Mouse(MouseButton::X2),
    ];
    let mut e = Engine::new(vec![wheel(1, buttons.clone(), WheelDirection::Up)]).unwrap();
    for (at, button) in buttons.iter().copied().enumerate() {
        input(&mut e, InputKind::Down(button), at as u64);
        assert_eq!(e.wheel_session_snapshot(), MOUSE);
    }
    assert_eq!(scroll(&mut e, 1, 5).triggered, vec![1]);
    for (at, button) in buttons.into_iter().enumerate() {
        input(&mut e, InputKind::Up(button), 6 + at as u64);
        assert_eq!(
            e.wheel_session_snapshot(),
            if at == 3 { NONE } else { MOUSE }
        );
    }
}

#[test]
fn owner_bitmap_covers_the_last_registered_rule_without_hook_allocations() {
    let bindings = (0..MAX_BINDINGS)
        .map(|index| {
            let mut row = wheel(index as u64 + 1, vec![SIDE], WheelDirection::Up);
            row.process_scope.allow = vec![format!("app{index}")];
            row
        })
        .collect();
    let mut e = Engine::new(bindings).unwrap();
    e.set_foreground_process(Some(&format!("app{}", MAX_BINDINGS - 1)))
        .unwrap();
    input(&mut e, InputKind::Down(SIDE), 0);
    assert_eq!(e.wheel_session_snapshot(), MOUSE);
    assert_eq!(scroll(&mut e, 1, 1).triggered, vec![MAX_BINDINGS as u64]);
    input(&mut e, InputKind::Up(SIDE), 2);
    assert_eq!(e.wheel_session_snapshot(), NONE);
}

#[test]
fn rejected_replacement_and_injected_edges_do_not_change_established_ownership() {
    let binding = wheel(1, vec![SIDE], WheelDirection::Up);
    let mut e = Engine::new(vec![binding.clone()]).unwrap();
    e.process_at(
        InputEvent {
            kind: InputKind::Down(SIDE),
            injected: true,
        },
        0,
    );
    assert_eq!(e.wheel_session_snapshot(), NONE);
    input(&mut e, InputKind::Down(SIDE), 1);
    let invalid = TriggerSpec {
        held: vec![SIDE, SIDE],
        ..binding
    };
    assert!(e.replace_bindings(vec![invalid]).is_err());
    e.process_at(
        InputEvent {
            kind: InputKind::Up(SIDE),
            injected: true,
        },
        999,
    );
    assert_eq!(e.wheel_session_snapshot(), MOUSE);
    input(&mut e, InputKind::Up(SIDE), 2);
    assert_eq!(e.wheel_session_snapshot(), NONE);
}

#[test]
fn retained_owner_completion_clears_mouse_prefixes_it_turns_into_draining() {
    let mut e = Engine::new(vec![
        wheel(1, vec![SIDE], WheelDirection::Up),
        retained_owner(),
    ])
    .unwrap();
    input(&mut e, InputKind::Down(SIDE), 0);
    scroll(&mut e, 1, 1);
    assert_eq!(e.wheel_session_snapshot(), MOUSE);
    input(&mut e, InputKind::Down(SPACE), 100);
    input(&mut e, InputKind::Up(SPACE), 200);
    assert_eq!(e.wheel_session_snapshot(), NONE);
    assert!(input(&mut e, InputKind::Up(SIDE), 201).consume);
}

#[test]
fn stale_drag_cancel_does_not_clear_live_session_but_real_cancel_does() {
    let mut e = Engine::new(vec![drag(vec![], RIGHT)]).unwrap();
    e.pointer_moved((0, 0), 0);
    input(&mut e, InputKind::Down(RIGHT), 0);
    e.pointer_moved((4, 0), 1);
    let session = e.active_drag().unwrap().session;
    e.cancel_drag(session + 1);
    assert_eq!(e.wheel_session_snapshot(), MOUSE);
    e.cancel_drag(session);
    assert_eq!(e.wheel_session_snapshot(), NONE);
    assert!(input(&mut e, InputKind::Up(RIGHT), 2).consume);
}

#[test]
fn dedicated_layer_release_clears_a_mouse_prefix_that_becomes_draining() {
    let mut binding = wheel(1, vec![SPACE, SIDE], WheelDirection::Up);
    binding.keyboard.dedicated_prefix = Some(SPACE);
    let mut e = Engine::new(vec![binding]).unwrap();
    input(&mut e, InputKind::Down(SPACE), 0);
    assert_eq!(e.wheel_session_snapshot(), NONE);
    input(&mut e, InputKind::Down(SIDE), 1);
    assert_eq!(e.wheel_session_snapshot(), MOUSE);
    scroll(&mut e, 1, 2);
    input(&mut e, InputKind::Up(SPACE), 3);
    assert_eq!(e.wheel_session_snapshot(), NONE);
    assert!(input(&mut e, InputKind::Up(SIDE), 4).consume);
}
