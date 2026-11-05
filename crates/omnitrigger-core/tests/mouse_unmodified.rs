//! 无控制键鼠标模式的公共 API 契约测试：只重放标准化输入，不调用平台 API。
use omnitrigger_core::*;

#[test]
fn passed_inputs_do_not_create_mouse_sessions_or_bypass_the_shared_small_wheel_gate() {
    for button in [
        Button::Key(0x41),
        Button::Key(0x5b),
        Button::Mouse(MouseButton::X1),
    ] {
        let mut engine = Engine::new(vec![wheel_rule(1, 0)]).unwrap();
        let mut normalizer = WheelInputNormalizer::new(WheelInputPolicy::SmallVertical150ms);
        assert_eq!(step(&mut engine, InputKind::Down(button), 10), passed());
        for (time, expected) in [(1000, hit(1)), (1010, passed()), (1150, hit(1))] {
            // 使用真实核心快照，不能把透传按住量硬编码成“鼠标会话”。
            let session = engine.wheel_session_snapshot();
            assert!(!session.is_active());
            assert_eq!(
                normalizer.process_at(
                    &mut engine,
                    InputEvent {
                        kind: wheel(1),
                        injected: false
                    },
                    time,
                    time as u64,
                    session,
                ),
                expected,
            );
        }
        assert_eq!(step(&mut engine, InputKind::Up(button), 1200), passed());
    }
}

const A: Button = Button::Key(0x41);
const B: Button = Button::Key(0x42);
const LEADER: Button = Button::Key(0x70);
const WIN: Button = Button::Key(0x5b);
const CTRL: Button = Button::Key(0xa2);
const LEFT: Button = Button::Mouse(MouseButton::Left);
const RIGHT: Button = Button::Mouse(MouseButton::Right);
const X1: Button = Button::Mouse(MouseButton::X1);

fn plain(id: u64, held: &[Button], activation: Activation) -> TriggerSpec {
    TriggerSpec {
        id,
        held: held.to_vec(),
        activation,
        keyboard: Default::default(),
        wheel: Default::default(),
        conditions: vec![],
        process_scope: Default::default(),
        pointer_scope: Default::default(),
    }
}

fn mouse(id: u64, activation: Activation, priority: u16) -> TriggerSpec {
    let mut row = plain(id, &[], activation);
    row.keyboard.matching = KeyboardMatching::MouseUnmodified {
        priority,
        pointer_scope_priority: true,
    };
    if matches!(activation, Activation::Wheel(_)) {
        row.wheel = WheelPolicy::Packet;
    }
    row
}

fn wheel_rule(id: u64, priority: u16) -> TriggerSpec {
    mouse(id, Activation::Wheel(WheelDirection::Up), priority)
}

fn captured_owner() -> TriggerSpec {
    let mut row = plain(10, &[LEADER], Activation::Press(B));
    row.keyboard.matching = KeyboardMatching::Captured { priority: 0 };
    row.keyboard.dedicated_prefix = Some(LEADER);
    row
}

fn wheel(delta: i32) -> InputKind {
    InputKind::Wheel {
        axis: Axis::Vertical,
        delta,
    }
}

fn step(engine: &mut Engine, kind: InputKind, time: u64) -> Decision {
    engine.process_at(
        InputEvent {
            kind,
            injected: false,
        },
        time,
    )
}

fn sequence(engine: &mut Engine, steps: Vec<(InputKind, Decision)>) {
    for (index, (kind, expected)) in steps.into_iter().enumerate() {
        assert_eq!(
            step(engine, kind, index as u64),
            expected,
            "事件 {index}: {kind:?}"
        );
    }
}

fn passed() -> Decision {
    Decision::default()
}

fn consumed() -> Decision {
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

fn replay(events: Vec<InputKind>) -> Decision {
    Decision {
        replay: events,
        ..Default::default()
    }
}

fn rectangle(left: i32, right: i32) -> Vec<TriggerCondition> {
    vec![TriggerCondition::PointerRegion(PointerRegion::Rectangle(
        ScreenRect {
            left,
            top: 0,
            right,
            bottom: 100,
        },
    ))]
}

#[test]
fn public_classification_keeps_the_existing_exact_mode_distinct() {
    let matching = wheel_rule(1, 0).keyboard.matching;
    assert!(matching.is_mouse_unmodified());
    assert!(matching.is_mouse_rule());
    assert!(!matching.is_mouse_exact());
    assert!(!matching.is_captured());
    assert!(!matching.is_mouse_captured_leader());
    let strict = KeyboardMatching::MouseExact {
        priority: 0,
        pointer_scope_priority: true,
    };
    assert!(strict.is_mouse_exact());
    assert!(!strict.is_mouse_unmodified());
    assert!(!KeyboardMatching::Exact.is_mouse_unmodified());
}

#[test]
fn ordinary_and_both_win_keys_keep_down_repeat_and_up_passed() {
    for key in [0x08, 0x20, 0x41, 0x5a, 0x5b, 0x5c, 0x70, 0xfe] {
        let key = Button::Key(key);
        let mut engine = Engine::new(vec![wheel_rule(1, 0)]).unwrap();
        sequence(
            &mut engine,
            vec![
                (InputKind::Down(key), passed()),
                (InputKind::Down(key), passed()),
                (wheel(1), hit(1)),
                (InputKind::Down(key), passed()),
                (wheel(240), hit(1)),
                (InputKind::Up(key), passed()),
                (InputKind::Up(key), passed()),
                (wheel(120), hit(1)),
            ],
        );
    }
    let mut engine = Engine::new(vec![wheel_rule(1, 0)]).unwrap();
    sequence(
        &mut engine,
        vec![
            (InputKind::Down(A), passed()),
            (InputKind::Down(WIN), passed()),
            (InputKind::Down(Button::Key(0x5c)), passed()),
            (InputKind::Down(B), passed()),
            (wheel(1), hit(1)),
            (InputKind::Up(WIN), passed()),
            (wheel(1), hit(1)),
            (InputKind::Up(A), passed()),
            (InputKind::Up(B), passed()),
            (InputKind::Up(Button::Key(0x5c)), passed()),
        ],
    );
}

#[test]
fn every_generic_and_sided_control_shift_alt_blocks_until_physical_release() {
    for key in [0x10, 0x11, 0x12, 0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5] {
        let modifier = Button::Key(key);
        let mut engine = Engine::new(vec![wheel_rule(1, 0)]).unwrap();
        sequence(
            &mut engine,
            vec![
                (InputKind::Down(A), passed()),
                (InputKind::Down(WIN), passed()),
                (wheel(1), hit(1)),
                (InputKind::Down(modifier), passed()),
                (InputKind::Down(modifier), passed()),
                (wheel(120), passed()),
                (InputKind::Up(A), passed()),
                (wheel(1), passed()),
                (InputKind::Up(modifier), passed()),
                (wheel(1), hit(1)),
                (InputKind::Up(WIN), passed()),
            ],
        );
    }
    for (left, right) in [(0xa0, 0xa1), (0xa2, 0xa3), (0xa4, 0xa5)] {
        let mut engine = Engine::new(vec![wheel_rule(1, 0)]).unwrap();
        sequence(
            &mut engine,
            vec![
                (InputKind::Down(Button::Key(left)), passed()),
                (InputKind::Down(Button::Key(right)), passed()),
                (InputKind::Up(Button::Key(left)), passed()),
                (wheel(1), passed()),
                (InputKind::Up(Button::Key(right)), passed()),
                (wheel(1), hit(1)),
            ],
        );
    }
}

#[test]
fn extra_passed_mouse_buttons_remain_passed_around_wheel_actions() {
    let mut engine = Engine::new(vec![wheel_rule(1, 0)]).unwrap();
    for button in [
        MouseButton::Left,
        MouseButton::Right,
        MouseButton::Middle,
        MouseButton::X1,
        MouseButton::X2,
    ] {
        let button = Button::Mouse(button);
        sequence(
            &mut engine,
            vec![
                (InputKind::Down(button), passed()),
                (InputKind::Down(button), passed()),
                (wheel(1), hit(1)),
                (InputKind::Down(button), passed()),
                (InputKind::Up(button), passed()),
            ],
        );
    }
}

#[test]
fn each_mouse_press_claims_only_its_own_press_and_release() {
    for button in [
        MouseButton::Left,
        MouseButton::Right,
        MouseButton::Middle,
        MouseButton::X1,
        MouseButton::X2,
    ] {
        let button = Button::Mouse(button);
        let extra = if button == LEFT { RIGHT } else { LEFT };
        let mut engine = Engine::new(vec![mouse(1, Activation::Press(button), 0)]).unwrap();
        sequence(
            &mut engine,
            vec![
                (InputKind::Down(A), passed()),
                (InputKind::Down(WIN), passed()),
                (InputKind::Down(extra), passed()),
                (InputKind::Down(button), hit(1)),
                (InputKind::Down(A), passed()),
                (InputKind::Down(WIN), passed()),
                (InputKind::Down(button), consumed()),
                (InputKind::Down(CTRL), passed()),
                (InputKind::Up(extra), passed()),
                (InputKind::Up(button), consumed()),
                (InputKind::Down(button), passed()),
                (InputKind::Up(CTRL), passed()),
                (InputKind::Down(button), passed()),
                (InputKind::Up(button), passed()),
                (InputKind::Down(button), hit(1)),
                (InputKind::Up(button), consumed()),
                (InputKind::Up(A), passed()),
                (InputKind::Up(WIN), passed()),
            ],
        );
    }
}

#[test]
fn consumed_keyboard_and_mouse_terminals_cannot_be_borrowed_for_wheel() {
    for terminal in [A, RIGHT] {
        let owner = plain(2, &[], Activation::Press(terminal));
        let mut engine = Engine::new(vec![wheel_rule(1, 0), owner]).unwrap();
        sequence(
            &mut engine,
            vec![
                (InputKind::Down(terminal), hit(2)),
                (InputKind::Down(terminal), consumed()),
                (wheel(1), passed()),
                (InputKind::Up(terminal), consumed()),
                (wheel(1), hit(1)),
            ],
        );
    }
    let mut engine = Engine::new(vec![
        wheel_rule(1, 0),
        mouse(2, Activation::Press(RIGHT), 0),
    ])
    .unwrap();
    sequence(
        &mut engine,
        vec![
            (InputKind::Down(A), passed()),
            (InputKind::Down(RIGHT), hit(2)),
            (wheel(1), passed()),
            (InputKind::Up(RIGHT), consumed()),
            (wheel(1), hit(1)),
            (InputKind::Up(A), passed()),
        ],
    );
}

#[test]
fn pending_prefix_is_rejected_before_fallback_then_its_passed_state_is_usable() {
    for prefix in [A, X1] {
        let owner = plain(2, &[prefix], Activation::Press(B));
        let mut engine = Engine::new(vec![wheel_rule(1, 0), owner]).unwrap();
        sequence(
            &mut engine,
            vec![
                (InputKind::Down(prefix), consumed()),
                (InputKind::Down(prefix), consumed()),
                (wheel(1), replay(vec![InputKind::Down(prefix)])),
                (wheel(1), hit(1)),
                (InputKind::Down(prefix), passed()),
                (InputKind::Up(prefix), passed()),
            ],
        );
    }
}

#[test]
fn observed_primary_mouse_prefix_is_not_a_passed_press() {
    let mut engine = Engine::new(vec![
        wheel_rule(1, 0),
        plain(2, &[LEFT], Activation::Press(B)),
    ])
    .unwrap();
    sequence(
        &mut engine,
        vec![
            (InputKind::Down(LEFT), passed()),
            (InputKind::Down(LEFT), passed()),
            (wheel(1), passed()),
            (wheel(120), passed()),
            (InputKind::Up(LEFT), passed()),
            (wheel(1), hit(1)),
        ],
    );
}

#[test]
fn a_replayed_hold_terminal_is_still_owned_until_release() {
    let hold = plain(
        2,
        &[],
        Activation::Hold {
            button: A,
            duration_ms: 1000,
        },
    );
    let mut engine = Engine::new(vec![wheel_rule(1, 0), hold]).unwrap();
    sequence(
        &mut engine,
        vec![
            (InputKind::Down(A), consumed()),
            (InputKind::Down(B), replay(vec![InputKind::Down(A)])),
            (InputKind::Down(A), passed()),
            (wheel(1), passed()),
            (InputKind::Up(A), passed()),
            (wheel(1), hit(1)),
            (InputKind::Up(B), passed()),
        ],
    );
}

#[test]
fn suppressed_hold_and_draining_release_cannot_escape_a_dedicated_layer() {
    let mut owner = plain(
        2,
        &[LEADER],
        Activation::Hold {
            button: A,
            duration_ms: 1000,
        },
    );
    owner.keyboard.dedicated_prefix = Some(LEADER);
    let mut engine = Engine::new(vec![wheel_rule(1, 0), owner]).unwrap();
    sequence(
        &mut engine,
        vec![
            (InputKind::Down(LEADER), consumed()),
            (InputKind::Down(A), consumed()),
            (wheel(1), passed()),
            (InputKind::Down(A), consumed()),
            (InputKind::Up(LEADER), consumed()),
            (wheel(1), passed()),
            (InputKind::Up(A), consumed()),
            (wheel(1), hit(1)),
        ],
    );
}

#[test]
fn captured_mouse_watcher_keeps_runtime_precedence_despite_lower_numeric_priority() {
    let mut watcher = plain(11, &[LEADER], Activation::Wheel(WheelDirection::Up));
    watcher.wheel = WheelPolicy::Packet;
    watcher.keyboard.matching = KeyboardMatching::MouseCapturedLeader {
        priority: 511,
        pointer_scope_priority: true,
    };
    let mut engine = Engine::new(vec![wheel_rule(1, 0), watcher, captured_owner()]).unwrap();
    sequence(
        &mut engine,
        vec![
            (InputKind::Down(LEADER), consumed()),
            (wheel(1), hit(11)),
            (InputKind::Down(A), consumed()),
            (wheel(1), hit(11)),
            (InputKind::Up(LEADER), consumed()),
            (wheel(1), passed()),
            (InputKind::Down(A), consumed()),
            (InputKind::Up(A), consumed()),
            (wheel(1), hit(1)),
        ],
    );
}

#[test]
fn retained_leader_restored_as_observed_still_blocks_the_new_mode() {
    let mut owner = captured_owner();
    owner.keyboard.dedicated_prefix = None;
    owner.keyboard.retained_prefix = Some(Box::new(RetainedPrefix {
        key: LEADER,
        capture_scope: Default::default(),
        quiet_ms: 0,
        fast_ms: 50,
        block_with_primary_mouse: false,
    }));
    owner.keyboard.restore_on_tap = true;
    let mut engine = Engine::new(vec![wheel_rule(1, 0), owner]).unwrap();
    sequence(
        &mut engine,
        vec![
            (InputKind::Down(LEADER), consumed()),
            (wheel(1), passed()),
            (InputKind::Down(LEFT), replay(vec![InputKind::Down(LEADER)])),
            // 既有保留层已恢复引导 Down，后续重复透传；会话仍阻止借用为无控制键规则。
            (InputKind::Down(LEADER), passed()),
            (wheel(1), passed()),
            (InputKind::Up(LEFT), passed()),
            (InputKind::Up(LEADER), passed()),
            (wheel(1), hit(1)),
        ],
    );
}

#[test]
fn cancel_replace_disable_and_context_changes_keep_press_release_ownership() {
    for operation in ["cancel", "replace", "disable", "foreground", "pointer"] {
        let row = mouse(1, Activation::Press(RIGHT), 0);
        let mut engine = Engine::new(vec![row.clone()]).unwrap();
        assert_eq!(step(&mut engine, InputKind::Down(A), 0), passed());
        assert_eq!(step(&mut engine, InputKind::Down(WIN), 1), passed());
        assert_eq!(step(&mut engine, InputKind::Down(RIGHT), 2), hit(1));
        let decision = match operation {
            "cancel" => engine.cancel(),
            "replace" => engine.replace_bindings(vec![row]).unwrap(),
            "disable" => {
                assert_eq!(engine.set_enabled(false), passed());
                engine.set_enabled(true)
            }
            "foreground" => engine.set_foreground_process(Some("editor.exe")).unwrap(),
            "pointer" => engine
                .set_pointer_context(Some("editor.exe"), WindowRole::Application)
                .unwrap(),
            _ => unreachable!(),
        };
        assert_eq!(decision, passed(), "{operation}");
        sequence(
            &mut engine,
            vec![
                (InputKind::Down(RIGHT), consumed()),
                (InputKind::Down(A), passed()),
                (InputKind::Down(WIN), passed()),
                (InputKind::Up(A), passed()),
                (InputKind::Up(RIGHT), consumed()),
                (InputKind::Down(RIGHT), hit(1)),
                (InputKind::Up(RIGHT), consumed()),
                (InputKind::Up(WIN), passed()),
            ],
        );
    }
}

#[test]
fn enabling_or_replacing_rules_cannot_capture_an_already_passed_mouse_press() {
    let row = mouse(1, Activation::Press(RIGHT), 0);
    for disabled in [false, true] {
        let mut engine = Engine::new(vec![]).unwrap();
        if disabled {
            assert_eq!(engine.set_enabled(false), passed());
        }
        assert_eq!(step(&mut engine, InputKind::Down(RIGHT), 0), passed());
        assert_eq!(
            engine.replace_bindings(vec![row.clone()]).unwrap(),
            passed()
        );
        assert_eq!(engine.set_enabled(true), passed());
        sequence(
            &mut engine,
            vec![
                (InputKind::Down(RIGHT), passed()),
                (InputKind::Up(RIGHT), passed()),
                (InputKind::Down(RIGHT), hit(1)),
                (InputKind::Up(RIGHT), consumed()),
            ],
        );
    }
}

#[test]
fn injected_down_and_up_never_establish_or_release_physical_ownership() {
    let mut engine = Engine::new(vec![
        wheel_rule(1, 0),
        mouse(2, Activation::Press(RIGHT), 0),
    ])
    .unwrap();
    for kind in [InputKind::Down(CTRL), InputKind::Down(RIGHT), wheel(1)] {
        assert_eq!(
            engine.process_at(
                InputEvent {
                    kind,
                    injected: true
                },
                9999
            ),
            passed()
        );
    }
    assert_eq!(step(&mut engine, wheel(1), 0), hit(1));
    assert_eq!(step(&mut engine, InputKind::Down(CTRL), 1), passed());
    assert_eq!(
        engine.process_at(
            InputEvent {
                kind: InputKind::Up(CTRL),
                injected: true,
            },
            9999
        ),
        passed()
    );
    assert_eq!(step(&mut engine, wheel(1), 2), passed());
    assert_eq!(step(&mut engine, InputKind::Up(CTRL), 3), passed());
    assert_eq!(step(&mut engine, InputKind::Down(RIGHT), 4), hit(2));
    assert_eq!(
        engine.process_at(
            InputEvent {
                kind: InputKind::Up(RIGHT),
                injected: true,
            },
            9999
        ),
        passed()
    );
    sequence(
        &mut engine,
        vec![
            (InputKind::Down(RIGHT), consumed()),
            (wheel(1), passed()),
            (InputKind::Up(RIGHT), consumed()),
            (wheel(1), hit(1)),
        ],
    );
}

#[test]
fn packet_policy_uses_each_nonzero_event_once_on_all_four_directions() {
    for (direction, axis, sign) in [
        (WheelDirection::Up, Axis::Vertical, 1),
        (WheelDirection::Down, Axis::Vertical, -1),
        (WheelDirection::Right, Axis::Horizontal, 1),
        (WheelDirection::Left, Axis::Horizontal, -1),
    ] {
        let mut engine = Engine::new(vec![mouse(1, Activation::Wheel(direction), 0)]).unwrap();
        assert_eq!(step(&mut engine, InputKind::Down(WIN), 0), passed());
        for delta in [0, sign, 60 * sign, 120 * sign, 10000 * sign] {
            assert_eq!(
                step(&mut engine, InputKind::Wheel { axis, delta }, 1),
                if delta == 0 { passed() } else { hit(1) }
            );
        }
        assert_eq!(
            step(&mut engine, InputKind::Wheel { axis, delta: -sign }, 2),
            passed()
        );
        assert_eq!(step(&mut engine, InputKind::Up(WIN), 3), passed());
    }
}

#[test]
fn notch_threshold_and_per_event_work_are_bounded_on_each_direction() {
    for (direction, axis, sign) in [
        (WheelDirection::Up, Axis::Vertical, 1),
        (WheelDirection::Down, Axis::Vertical, -1),
        (WheelDirection::Right, Axis::Horizontal, 1),
        (WheelDirection::Left, Axis::Horizontal, -1),
    ] {
        let mut row = mouse(1, Activation::Wheel(direction), 0);
        row.wheel = WheelPolicy::Notch;
        let mut engine = Engine::new(vec![row]).unwrap();
        assert_eq!(step(&mut engine, InputKind::Down(A), 0), passed());
        assert_eq!(
            step(
                &mut engine,
                InputKind::Wheel {
                    axis,
                    delta: 119 * sign
                },
                1
            ),
            consumed()
        );
        assert_eq!(
            step(&mut engine, InputKind::Wheel { axis, delta: sign }, 2),
            hit(1)
        );
        let large = if sign < 0 { i32::MIN } else { i32::MAX };
        let bounded = Decision {
            consume: true,
            triggered: vec![1; 64],
            ..Default::default()
        };
        assert_eq!(
            step(&mut engine, InputKind::Wheel { axis, delta: large }, 3),
            bounded
        );
        assert_eq!(
            step(&mut engine, InputKind::Wheel { axis, delta: sign }, 4),
            bounded
        );
        assert_eq!(engine.cancel(), passed());
        assert_eq!(
            step(&mut engine, InputKind::Wheel { axis, delta: sign }, 5),
            consumed()
        );
    }
}

#[test]
fn existing_mouse_exact_still_rejects_extra_ordinary_and_win_keys() {
    for extra in [A, WIN, LEFT] {
        let mut strict = wheel_rule(1, 0);
        strict.keyboard.matching = KeyboardMatching::MouseExact {
            priority: 0,
            pointer_scope_priority: true,
        };
        let mut engine = Engine::new(vec![strict]).unwrap();
        sequence(
            &mut engine,
            vec![
                (InputKind::Down(extra), passed()),
                (InputKind::Down(extra), passed()),
                (wheel(1), passed()),
                (InputKind::Up(extra), passed()),
                (wheel(1), hit(1)),
            ],
        );
    }
}

#[test]
fn notch_tail_survives_passed_repeats_and_zero_but_is_returned_before_release() {
    let mut row = wheel_rule(1, 0);
    row.wheel = WheelPolicy::Notch;
    let mut engine = Engine::new(vec![row]).unwrap();
    sequence(
        &mut engine,
        vec![
            (InputKind::Down(A), passed()),
            (wheel(60), consumed()),
            (InputKind::Down(A), passed()),
            (wheel(0), passed()),
            (wheel(60), hit(1)),
            (wheel(40), consumed()),
            (InputKind::Up(A), replay(vec![wheel(40)])),
            (wheel(80), consumed()),
            (wheel(40), hit(1)),
        ],
    );
}

#[test]
fn notch_tail_is_returned_before_a_new_modifier_and_blocked_packets_do_not_accumulate() {
    let mut row = wheel_rule(1, 0);
    row.wheel = WheelPolicy::Notch;
    let mut engine = Engine::new(vec![row]).unwrap();
    sequence(
        &mut engine,
        vec![
            (wheel(60), consumed()),
            (InputKind::Down(CTRL), replay(vec![wheel(60)])),
            (wheel(120), passed()),
            (InputKind::Up(CTRL), passed()),
            (wheel(60), consumed()),
            (wheel(60), hit(1)),
        ],
    );
}

#[test]
fn notch_tail_returns_when_a_region_changes_the_winning_rule() {
    let mut fallback = wheel_rule(1, 7);
    fallback.wheel = WheelPolicy::Notch;
    let mut region = wheel_rule(2, 1);
    region.wheel = WheelPolicy::Notch;
    region.conditions = rectangle(0, 10);
    let mut engine = Engine::new(vec![fallback, region]).unwrap();
    assert_eq!(engine.pointer_moved((50, 0), 0), passed());
    assert_eq!(step(&mut engine, InputKind::Down(WIN), 1), passed());
    assert_eq!(step(&mut engine, wheel(60), 2), consumed());
    assert_eq!(engine.pointer_moved((0, 0), 3), replay(vec![wheel(60)]));
    assert_eq!(step(&mut engine, wheel(60), 4), consumed());
    assert_eq!(step(&mut engine, wheel(60), 5), hit(2));
    assert_eq!(engine.pointer_moved((50, 0), 6), passed());
    assert_eq!(step(&mut engine, wheel(60), 7), consumed());
    assert_eq!(step(&mut engine, wheel(60), 8), hit(1));
    assert_eq!(step(&mut engine, InputKind::Up(WIN), 9), passed());
}

#[test]
fn invalid_mode_combinations_return_structured_errors_without_silent_rewriting() {
    let base = wheel_rule(1, 0);
    let mut invalid = Vec::new();
    for held in [vec![A], vec![WIN], vec![CTRL], vec![LEFT], vec![A, WIN]] {
        let mut row = base.clone();
        row.held = held;
        invalid.push(row);
    }
    for activation in [
        Activation::Press(A),
        Activation::Hold {
            button: RIGHT,
            duration_ms: 100,
        },
        Activation::Drag {
            button: RIGHT,
            min_distance_px: 18,
            max_start_ms: 180,
        },
    ] {
        let mut row = base.clone();
        row.activation = activation;
        row.wheel = WheelPolicy::Notch;
        invalid.push(row);
    }
    let mut row = base.clone();
    row.keyboard.dedicated_prefix = Some(LEADER);
    invalid.push(row);
    let mut row = base.clone();
    row.keyboard.retained_prefix = Some(Box::new(RetainedPrefix {
        key: LEADER,
        capture_scope: Default::default(),
        quiet_ms: 0,
        fast_ms: 50,
        block_with_primary_mouse: false,
    }));
    invalid.push(row);
    let mut row = base.clone();
    row.keyboard.repeat = true;
    invalid.push(row);
    let mut row = base.clone();
    row.keyboard.restore_on_tap = true;
    invalid.push(row);
    let mut row = base.clone();
    row.keyboard.tap_timeout_ms = Some(100);
    invalid.push(row);
    let mut row = base;
    row.keyboard.tap_scope.allow = vec!["editor".into()];
    invalid.push(row);
    for row in invalid {
        assert_eq!(
            Engine::new(vec![row]).err(),
            Some(RegistrationError::InvalidMouseMatching { id: 1 })
        );
    }
    for priority in [0, 511] {
        assert!(Engine::new(vec![wheel_rule(1, priority)]).is_ok());
    }
    for priority in [512, u16::MAX] {
        assert_eq!(
            Engine::new(vec![wheel_rule(1, priority)]).err(),
            Some(RegistrationError::InvalidMousePriority { id: 1, priority })
        );
    }
    let mut press = mouse(1, Activation::Press(RIGHT), 0);
    press.wheel = WheelPolicy::Packet;
    assert_eq!(
        Engine::new(vec![press]).err(),
        Some(RegistrationError::InvalidWheelPolicy { id: 1 })
    );
}

#[test]
fn registration_detects_overlapping_states_instead_of_only_equal_held_arrays() {
    for activation in [
        Activation::Wheel(WheelDirection::Up),
        Activation::Press(RIGHT),
    ] {
        let mut candidates = vec![mouse(2, activation, 0)];
        let mut strict = mouse(2, activation, 0);
        strict.keyboard.matching = KeyboardMatching::MouseExact {
            priority: 0,
            pointer_scope_priority: true,
        };
        candidates.push(strict.clone());
        strict.held = vec![WIN];
        candidates.push(strict);
        candidates.push(plain(2, &[], activation));
        for candidate in candidates {
            let first = mouse(1, activation, 0);
            for rows in [
                vec![first.clone(), candidate.clone()],
                vec![candidate.clone(), first.clone()],
            ] {
                assert!(matches!(
                    Engine::new(rows).err(),
                    Some(
                        RegistrationError::DuplicateTrigger {
                            first: 1,
                            second: 2
                        } | RegistrationError::DuplicateTrigger {
                            first: 2,
                            second: 1
                        }
                    )
                ));
            }
        }
    }
}

#[test]
fn same_rank_nonempty_exact_uses_owned_prefixes_while_unmodified_uses_passed_inputs() {
    for activation in [
        Activation::Wheel(WheelDirection::Up),
        Activation::Press(RIGHT),
    ] {
        for held in [vec![A], vec![WIN], vec![LEFT], vec![X1], vec![A, WIN, LEFT]] {
            let exact = plain(2, &held, activation);
            let unmodified = mouse(1, activation, 0);
            for rows in [
                vec![unmodified.clone(), exact.clone()],
                vec![exact.clone(), unmodified.clone()],
            ] {
                let mut engine = Engine::new(rows).unwrap();
                let trigger = if matches!(activation, Activation::Wheel(_)) {
                    wheel(120)
                } else {
                    InputKind::Down(RIGHT)
                };
                let mut steps = vec![(trigger, hit(1))];
                if matches!(activation, Activation::Press(_)) {
                    steps.push((InputKind::Up(RIGHT), consumed()));
                }
                // 非空 Exact 建立等待/观察前缀；相同物理集合不代表相同所有权。
                for &prefix in &held {
                    let expected = if prefix == LEFT { passed() } else { consumed() };
                    steps.push((InputKind::Down(prefix), expected));
                }
                steps.push((trigger, hit(2)));
                if matches!(activation, Activation::Press(_)) {
                    steps.push((InputKind::Up(RIGHT), consumed()));
                }
                for &prefix in held.iter().rev() {
                    let expected = if prefix == LEFT { passed() } else { consumed() };
                    steps.push((InputKind::Down(prefix), expected.clone()));
                    steps.push((InputKind::Up(prefix), expected));
                }
                steps.push((trigger, hit(1)));
                if matches!(activation, Activation::Press(_)) {
                    steps.push((InputKind::Up(RIGHT), consumed()));
                }
                if !held.contains(&LEFT) {
                    // 回退后同一物理前缀成为 Passed；Exact 不能借用，新模式才可匹配。
                    for &prefix in &held {
                        steps.push((InputKind::Down(prefix), consumed()));
                    }
                    steps.push((
                        wheel(-120),
                        replay(held.iter().copied().map(InputKind::Down).collect()),
                    ));
                    steps.push((trigger, hit(1)));
                    if matches!(activation, Activation::Press(_)) {
                        steps.push((InputKind::Up(RIGHT), consumed()));
                    }
                    for &prefix in held.iter().rev() {
                        steps.push((InputKind::Down(prefix), passed()));
                        steps.push((InputKind::Up(prefix), passed()));
                    }
                }
                sequence(&mut engine, steps);
            }
        }
    }
}

#[test]
fn same_rank_captured_modes_own_their_sessions_without_conflicting_with_passed_mode() {
    for mode in ["captured", "mouse_captured_leader"] {
        for activation in [
            Activation::Wheel(WheelDirection::Up),
            Activation::Press(RIGHT),
        ] {
            let mut row = plain(2, &[LEADER], activation);
            let mut rows = vec![mouse(1, activation, 0)];
            if mode == "captured" {
                row.keyboard.matching = KeyboardMatching::Captured { priority: 0 };
                row.keyboard.retained_prefix = Some(Box::new(RetainedPrefix {
                    key: LEADER,
                    capture_scope: Default::default(),
                    quiet_ms: 0,
                    fast_ms: 50,
                    block_with_primary_mouse: false,
                }));
                row.keyboard.restore_on_tap = true;
            } else {
                row.keyboard.matching = KeyboardMatching::MouseCapturedLeader {
                    priority: 0,
                    pointer_scope_priority: true,
                };
                rows.push(captured_owner());
            }
            rows.push(row);
            let mut reversed = rows.clone();
            reversed.reverse();
            for rows in [rows, reversed] {
                let mut engine = Engine::new(rows).unwrap();
                let trigger = if matches!(activation, Activation::Wheel(_)) {
                    wheel(120)
                } else {
                    InputKind::Down(RIGHT)
                };
                let mut steps = vec![(trigger, hit(1))];
                if matches!(activation, Activation::Press(_)) {
                    steps.push((InputKind::Up(RIGHT), consumed()));
                }
                steps.push((InputKind::Down(LEADER), consumed()));
                steps.push((trigger, hit(2)));
                if matches!(activation, Activation::Press(_)) {
                    steps.push((InputKind::Up(RIGHT), consumed()));
                }
                steps.push((InputKind::Down(LEADER), consumed()));
                steps.push((InputKind::Up(LEADER), consumed()));
                steps.push((trigger, hit(1)));
                if matches!(activation, Activation::Press(_)) {
                    steps.push((InputKind::Up(RIGHT), consumed()));
                }
                sequence(&mut engine, steps);
            }
        }
    }
}

#[test]
fn rules_requiring_control_shift_alt_are_mutually_exclusive_with_unmodified_mouse() {
    for modifier in [0x10, 0x11, 0x12, 0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5] {
        for matching in [
            KeyboardMatching::Exact,
            KeyboardMatching::MouseExact {
                priority: 0,
                pointer_scope_priority: true,
            },
        ] {
            let mut other = plain(
                2,
                &[Button::Key(modifier)],
                Activation::Wheel(WheelDirection::Up),
            );
            other.keyboard.matching = matching;
            assert!(Engine::new(vec![wheel_rule(1, 0), other]).is_ok());
        }
        let mut other = plain(
            2,
            &[A, Button::Key(modifier)],
            Activation::Wheel(WheelDirection::Up),
        );
        other.keyboard.matching = KeyboardMatching::Exact;
        assert!(Engine::new(vec![wheel_rule(1, 0), other]).is_ok());
    }
}

#[test]
fn explicit_priorities_order_overlapping_modes_independently_of_registration_order() {
    for mode in ["exact", "mouse_exact", "mouse_unmodified"] {
        let mut other = wheel_rule(2, 0);
        other.keyboard.matching = match mode {
            "exact" => KeyboardMatching::Exact,
            "mouse_exact" => KeyboardMatching::MouseExact {
                priority: 0,
                pointer_scope_priority: true,
            },
            "mouse_unmodified" => KeyboardMatching::MouseUnmodified {
                priority: 0,
                pointer_scope_priority: true,
            },
            _ => unreachable!(),
        };
        for rows in [
            vec![wheel_rule(1, 7), other.clone()],
            vec![other, wheel_rule(1, 7)],
        ] {
            let mut engine = Engine::new(rows).unwrap();
            assert_eq!(step(&mut engine, wheel(120), 0), hit(2));
            sequence(
                &mut engine,
                vec![
                    (InputKind::Down(A), passed()),
                    (
                        wheel(120),
                        hit(if mode == "mouse_unmodified" { 2 } else { 1 }),
                    ),
                    (InputKind::Up(A), passed()),
                ],
            );
        }
    }
    for activation in [
        Activation::Wheel(WheelDirection::Up),
        Activation::Press(RIGHT),
    ] {
        let mut strict = mouse(2, activation, 0);
        strict.keyboard.matching = KeyboardMatching::MouseExact {
            priority: 7,
            pointer_scope_priority: true,
        };
        for rows in [
            vec![mouse(1, activation, 1), strict.clone()],
            vec![strict, mouse(1, activation, 1)],
        ] {
            let mut engine = Engine::new(rows).unwrap();
            let kind = if matches!(activation, Activation::Wheel(_)) {
                wheel(120)
            } else {
                InputKind::Down(RIGHT)
            };
            assert_eq!(step(&mut engine, kind, 0), hit(1));
        }
    }
}

#[test]
fn registration_respects_disjoint_process_pointer_and_region_scopes() {
    for scope in ["process", "pointer", "region"] {
        let mut first = wheel_rule(1, 0);
        let mut second = wheel_rule(2, 0);
        match scope {
            "process" => {
                first.process_scope.allow = vec!["editor".into()];
                second.process_scope.allow = vec!["reader".into()];
            }
            "pointer" => {
                first.pointer_scope.allow = vec!["editor".into()];
                second.pointer_scope.allow = vec!["reader".into()];
            }
            "region" => {
                first.conditions = rectangle(0, 10);
                second.conditions = rectangle(10, 20);
            }
            _ => unreachable!(),
        }
        assert!(
            Engine::new(vec![first.clone(), second.clone()]).is_ok(),
            "{scope}"
        );
        second.process_scope = first.process_scope.clone();
        second.pointer_scope = first.pointer_scope.clone();
        second.conditions = first.conditions.clone();
        assert!(
            matches!(
                Engine::new(vec![first, second]).err(),
                Some(RegistrationError::DuplicateTrigger { .. })
            ),
            "{scope}"
        );
    }
}

#[test]
fn ignored_pointer_rank_does_not_hide_a_same_rank_conflict() {
    for pointer_scope_priority in [false, true] {
        let first = wheel_rule(1, 0);
        let mut second = wheel_rule(2, 0);
        second.pointer_scope.allow = vec!["editor".into()];
        second.keyboard.matching = KeyboardMatching::MouseUnmodified {
            priority: 0,
            pointer_scope_priority,
        };
        let result = Engine::new(vec![first, second]);
        if pointer_scope_priority {
            assert!(result.is_ok());
        } else {
            assert!(matches!(
                result.err(),
                Some(RegistrationError::DuplicateTrigger { .. })
            ));
        }
    }
}

#[test]
fn foreground_pointer_and_region_filters_all_apply_with_passed_keys() {
    let mut row = wheel_rule(1, 0);
    row.process_scope.allow = vec!["editor".into()];
    row.pointer_scope.allow = vec!["reader".into()];
    row.conditions = rectangle(0, 10);
    let mut engine = Engine::new(vec![row]).unwrap();
    assert_eq!(engine.pointer_moved((0, 0), 0), passed());
    assert_eq!(step(&mut engine, InputKind::Down(WIN), 1), passed());
    assert_eq!(step(&mut engine, wheel(1), 2), passed());
    assert_eq!(
        engine.set_foreground_process(Some("editor.exe")).unwrap(),
        passed()
    );
    assert_eq!(step(&mut engine, wheel(1), 3), passed());
    assert_eq!(
        engine
            .set_pointer_context(Some("reader.exe"), WindowRole::Application)
            .unwrap(),
        passed()
    );
    assert_eq!(step(&mut engine, wheel(1), 4), hit(1));
    assert_eq!(engine.pointer_moved((10, 0), 5), passed());
    assert_eq!(step(&mut engine, wheel(1), 6), passed());
    assert_eq!(engine.pointer_moved((0, 0), 7), passed());
    assert_eq!(step(&mut engine, InputKind::Down(CTRL), 8), passed());
    assert_eq!(step(&mut engine, wheel(1), 9), passed());
    assert_eq!(step(&mut engine, InputKind::Up(CTRL), 10), passed());
    assert_eq!(step(&mut engine, wheel(1), 11), hit(1));
    assert_eq!(step(&mut engine, InputKind::Up(WIN), 12), passed());
}

#[test]
fn application_rank_and_optional_pointer_rank_precede_numeric_priority() {
    let global = wheel_rule(1, 0);
    let mut scoped = wheel_rule(2, 511);
    scoped.process_scope.allow = vec!["editor".into()];
    let mut engine = Engine::new(vec![global, scoped]).unwrap();
    assert_eq!(step(&mut engine, InputKind::Down(A), 0), passed());
    assert_eq!(
        engine.set_foreground_process(Some("editor.exe")).unwrap(),
        passed()
    );
    assert_eq!(step(&mut engine, wheel(1), 1), hit(2));
    assert_eq!(
        engine.set_foreground_process(Some("reader.exe")).unwrap(),
        passed()
    );
    assert_eq!(step(&mut engine, wheel(1), 2), hit(1));
    assert_eq!(step(&mut engine, InputKind::Up(A), 3), passed());

    for pointer_scope_priority in [false, true] {
        let global = wheel_rule(1, 0);
        let mut pointer = wheel_rule(2, 511);
        pointer.pointer_scope.allow = vec!["editor".into()];
        pointer.keyboard.matching = KeyboardMatching::MouseUnmodified {
            priority: 511,
            pointer_scope_priority,
        };
        let mut engine = Engine::new(vec![pointer, global]).unwrap();
        assert_eq!(
            engine
                .set_pointer_context(Some("editor.exe"), WindowRole::Application)
                .unwrap(),
            passed()
        );
        assert_eq!(
            step(&mut engine, wheel(1), 0),
            hit(if pointer_scope_priority { 2 } else { 1 })
        );
        assert_eq!(
            engine
                .set_pointer_context(Some("reader.exe"), WindowRole::Application)
                .unwrap(),
            passed()
        );
        assert_eq!(step(&mut engine, wheel(1), 1), hit(1));
    }
}

#[test]
fn rejected_replacement_keeps_existing_rules_and_release_state() {
    let row = mouse(1, Activation::Press(RIGHT), 0);
    let mut engine = Engine::new(vec![row.clone()]).unwrap();
    assert_eq!(step(&mut engine, InputKind::Down(WIN), 0), passed());
    assert_eq!(step(&mut engine, InputKind::Down(RIGHT), 1), hit(1));
    let mut duplicate = row.clone();
    duplicate.id = 2;
    assert!(matches!(
        engine.replace_bindings(vec![row, duplicate]),
        Err(RegistrationError::DuplicateTrigger { .. })
    ));
    sequence(
        &mut engine,
        vec![
            (InputKind::Down(RIGHT), consumed()),
            (InputKind::Up(RIGHT), consumed()),
            (InputKind::Down(RIGHT), hit(1)),
            (InputKind::Up(RIGHT), consumed()),
            (InputKind::Up(WIN), passed()),
        ],
    );
}
