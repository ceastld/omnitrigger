use super::*;

const CAPS: Button = Button::Key(0x14);
const SHIFT: Button = Button::Key(0xA0);
const W: Button = Button::Key(0x57);
const Q: Button = Button::Key(0x51);
const SIDE: Button = Button::Mouse(MouseButton::X1);
const RIGHT: Button = Button::Mouse(MouseButton::Right);

fn event(kind: InputKind) -> InputEvent {
    InputEvent {
        kind,
        injected: false,
    }
}
fn press(id: u64, held: &[Button], terminal: Button) -> TriggerSpec {
    TriggerSpec {
        conditions: vec![],
        id,
        held: held.to_vec(),
        activation: Activation::Press(terminal),
    }
}
fn wheel(id: u64, held: &[Button], direction: WheelDirection) -> TriggerSpec {
    TriggerSpec {
        conditions: vec![],
        id,
        held: held.to_vec(),
        activation: Activation::Wheel(direction),
    }
}
fn hold(id: u64, held: &[Button], button: Button, duration_ms: u32) -> TriggerSpec {
    TriggerSpec {
        conditions: vec![],
        id,
        held: held.to_vec(),
        activation: Activation::Hold {
            button,
            duration_ms,
        },
    }
}
fn scroll(delta: i32) -> InputEvent {
    event(InputKind::Wheel {
        axis: Axis::Vertical,
        delta,
    })
}

#[test]
fn caps_chord_consumes_every_press_edge_and_ignores_repeat() {
    let mut engine = Engine::new(vec![press(1, &[CAPS], W)]).unwrap();
    assert!(engine.process(event(InputKind::Down(CAPS))).consume);
    assert!(engine.process(event(InputKind::Down(CAPS))).consume);
    assert_eq!(engine.process(event(InputKind::Down(W))).triggered, vec![1]);
    let repeat = engine.process(event(InputKind::Down(W)));
    assert!(repeat.consume && repeat.triggered.is_empty());
    assert!(engine.process(event(InputKind::Up(CAPS))).consume);
    assert!(engine.process(event(InputKind::Up(W))).consume);
    assert!(!engine.process(event(InputKind::Up(W))).consume);
}

#[test]
fn standalone_prefix_replays_exactly_one_down_and_up() {
    let mut engine = Engine::new(vec![press(1, &[CAPS], W)]).unwrap();
    engine.process(event(InputKind::Down(CAPS)));
    engine.process(event(InputKind::Down(CAPS)));
    assert_eq!(
        engine.process(event(InputKind::Up(CAPS))),
        Decision {
            consume: true,
            replay: vec![InputKind::Down(CAPS), InputKind::Up(CAPS)],
            triggered: vec![],
            drags: vec![],
        }
    );
}

#[test]
fn unmatched_successor_flushes_before_passthrough_and_disarms_prefix() {
    let mut engine = Engine::new(vec![press(1, &[CAPS], W)]).unwrap();
    engine.process(event(InputKind::Down(CAPS)));
    let fallback = engine.process(event(InputKind::Down(Q)));
    assert!(!fallback.consume);
    assert_eq!(fallback.replay, vec![InputKind::Down(CAPS)]);
    assert!(!engine.process(event(InputKind::Up(Q))).consume);
    assert_eq!(
        engine.process(event(InputKind::Down(W))),
        Decision::default()
    );
    assert!(!engine.process(event(InputKind::Up(W))).consume);
    assert!(!engine.process(event(InputKind::Up(CAPS))).consume);
}

#[test]
fn multiple_prefixes_accept_either_order_and_release_fallback_stays_paired() {
    let mut engine = Engine::new(vec![press(1, &[CAPS, SHIFT], W)]).unwrap();
    engine.process(event(InputKind::Down(SHIFT)));
    engine.process(event(InputKind::Down(CAPS)));
    let fallback = engine.process(event(InputKind::Up(SHIFT)));
    assert!(fallback.consume);
    assert_eq!(
        fallback.replay,
        vec![
            InputKind::Down(SHIFT),
            InputKind::Down(CAPS),
            InputKind::Up(SHIFT)
        ]
    );
    assert!(!engine.process(event(InputKind::Up(CAPS))).consume);
    engine.process(event(InputKind::Down(CAPS)));
    engine.process(event(InputKind::Down(SHIFT)));
    assert_eq!(engine.process(event(InputKind::Down(W))).triggered, vec![1]);
}

#[test]
fn matching_uses_exact_pressed_set() {
    let mut engine = Engine::new(vec![press(1, &[CAPS], W), press(2, &[CAPS, SHIFT], W)]).unwrap();
    engine.process(event(InputKind::Down(CAPS)));
    engine.process(event(InputKind::Down(SHIFT)));
    assert_eq!(engine.process(event(InputKind::Down(W))).triggered, vec![2]);
}

#[test]
fn side_button_wheel_accumulates_and_consumes_prefix_release() {
    let mut engine = Engine::new(vec![wheel(9, &[SIDE], WheelDirection::Up)]).unwrap();
    engine.process(event(InputKind::Down(SIDE)));
    let partial = engine.process(scroll(60));
    assert!(partial.consume && partial.triggered.is_empty());
    assert_eq!(engine.process(scroll(60)).triggered, vec![9]);
    assert_eq!(engine.process(scroll(240)).triggered, vec![9, 9]);
    let release = engine.process(event(InputKind::Up(SIDE)));
    assert!(release.consume && release.replay.is_empty());
}

#[test]
fn partial_wheel_is_replayed_on_unused_prefix_release() {
    let mut engine = Engine::new(vec![wheel(9, &[SIDE], WheelDirection::Up)]).unwrap();
    engine.process(event(InputKind::Down(SIDE)));
    engine.process(scroll(60));
    assert_eq!(
        engine.process(event(InputKind::Up(SIDE))).replay,
        vec![
            InputKind::Down(SIDE),
            InputKind::Wheel {
                axis: Axis::Vertical,
                delta: 60
            },
            InputKind::Up(SIDE),
        ]
    );
}

#[test]
fn opposite_directions_have_independent_accumulators() {
    let mut engine = Engine::new(vec![
        wheel(9, &[SIDE], WheelDirection::Up),
        wheel(10, &[SIDE], WheelDirection::Down),
    ])
    .unwrap();
    engine.process(event(InputKind::Down(SIDE)));
    assert!(engine.process(scroll(60)).triggered.is_empty());
    assert!(engine.process(scroll(-60)).triggered.is_empty());
    assert_eq!(engine.process(scroll(60)).triggered, vec![9]);
    assert_eq!(engine.process(scroll(-60)).triggered, vec![10]);
}

#[test]
fn unmatched_wheel_replays_partial_before_current_input() {
    let mut engine = Engine::new(vec![wheel(9, &[SIDE], WheelDirection::Up)]).unwrap();
    engine.process(event(InputKind::Down(SIDE)));
    engine.process(scroll(60));
    let decision = engine.process(scroll(-120));
    assert!(!decision.consume);
    assert_eq!(
        decision.replay,
        vec![
            InputKind::Down(SIDE),
            InputKind::Wheel {
                axis: Axis::Vertical,
                delta: 60
            }
        ]
    );
    assert!(!engine.process(event(InputKind::Up(SIDE))).consume);
}

#[test]
fn adding_another_prefix_does_not_borrow_wheel_delta_from_previous_chord() {
    let mut engine = Engine::new(vec![
        wheel(9, &[SIDE], WheelDirection::Up),
        wheel(10, &[SIDE, SHIFT], WheelDirection::Up),
    ])
    .unwrap();
    engine.process(event(InputKind::Down(SIDE)));
    engine.process(scroll(60));
    engine.process(event(InputKind::Down(SHIFT)));
    assert!(engine.process(scroll(60)).triggered.is_empty());
    assert_eq!(engine.process(scroll(60)).triggered, vec![10]);
}

#[test]
fn horizontal_wheel_and_extreme_delta_are_bounded() {
    let mut engine = Engine::new(vec![wheel(1, &[], WheelDirection::Left)]).unwrap();
    assert_eq!(
        engine
            .process(event(InputKind::Wheel {
                axis: Axis::Horizontal,
                delta: -120
            }))
            .triggered,
        vec![1]
    );
    assert_eq!(
        engine
            .process(event(InputKind::Wheel {
                axis: Axis::Horizontal,
                delta: i32::MIN
            }))
            .triggered
            .len(),
        64
    );
    engine.cancel();
    assert!(
        engine
            .process(event(InputKind::Wheel {
                axis: Axis::Horizontal,
                delta: -1
            }))
            .triggered
            .is_empty()
    );
}

#[test]
fn injected_events_are_passthrough_and_cannot_mutate_physical_state() {
    let mut engine = Engine::new(vec![press(1, &[CAPS], W)]).unwrap();
    engine.process(event(InputKind::Down(CAPS)));
    let injected = InputEvent {
        kind: InputKind::Up(CAPS),
        injected: true,
    };
    assert_eq!(engine.process(injected), Decision::default());
    assert_eq!(engine.process(event(InputKind::Down(W))).triggered, vec![1]);
}

#[test]
fn disable_drains_suppressed_up_but_preserves_passed_up() {
    let mut engine = Engine::new(vec![press(1, &[CAPS], W)]).unwrap();
    engine.process(event(InputKind::Down(CAPS)));
    engine.set_enabled(false);
    assert!(engine.process(event(InputKind::Down(CAPS))).consume);
    assert!(engine.process(event(InputKind::Up(CAPS))).consume);
    assert!(!engine.process(event(InputKind::Down(W))).consume);
    engine.set_enabled(true);
    assert!(!engine.process(event(InputKind::Up(W))).consume);
}

#[test]
fn cancel_and_replace_cannot_reuse_a_draining_press() {
    let mut engine = Engine::new(vec![press(1, &[CAPS], W)]).unwrap();
    engine.process(event(InputKind::Down(CAPS)));
    engine.replace_bindings(vec![press(2, &[CAPS], Q)]).unwrap();
    assert!(
        engine
            .process(event(InputKind::Down(Q)))
            .triggered
            .is_empty()
    );
    assert!(engine.process(event(InputKind::Up(CAPS))).consume);
    engine.process(event(InputKind::Up(Q)));
    engine.process(event(InputKind::Down(CAPS)));
    assert_eq!(engine.process(event(InputKind::Down(Q))).triggered, vec![2]);
}

#[test]
fn invalid_replacement_is_transactional() {
    let mut engine = Engine::new(vec![press(1, &[CAPS], W)]).unwrap();
    engine.process(event(InputKind::Down(CAPS)));
    assert!(
        engine
            .replace_bindings(vec![press(2, &[CAPS, CAPS], Q)])
            .is_err()
    );
    assert_eq!(engine.process(event(InputKind::Down(W))).triggered, vec![1]);
}

#[test]
fn registration_rejects_ambiguous_invalid_and_excessive_inputs() {
    assert!(matches!(
        Engine::new(vec![press(1, &[], CAPS), press(2, &[CAPS], W)]),
        Err(RegistrationError::TerminalIsPrefix { .. })
    ));
    assert!(matches!(
        Engine::new(vec![press(1, &[CAPS], W), press(2, &[CAPS], W)]),
        Err(RegistrationError::DuplicateTrigger { .. })
    ));
    assert!(matches!(
        Engine::new(vec![press(1, &[], W), press(1, &[], Q)]),
        Err(RegistrationError::DuplicateId { .. })
    ));
    assert!(matches!(
        Engine::new(vec![press(1, &[CAPS], CAPS)]),
        Err(RegistrationError::TerminalInHeld { .. })
    ));
    assert!(matches!(
        Engine::new(vec![press(1, &[], Button::Key(0))]),
        Err(RegistrationError::InvalidButton { .. })
    ));
    assert!(matches!(
        Engine::new(vec![press(1, &[], W); 65]),
        Err(RegistrationError::TooManyBindings)
    ));
    let many_held = (1..=9).map(Button::Key).collect::<Vec<_>>();
    assert!(matches!(
        Engine::new(vec![press(1, &many_held, W)]),
        Err(RegistrationError::TooManyHeld { .. })
    ));
}

#[test]
fn recording_chord_waits_for_all_releases_and_ignores_mouse_up_from_start_button() {
    let mut recorder = Recorder::new();
    assert!(
        recorder
            .process(event(InputKind::Up(Button::Mouse(MouseButton::Left))))
            .is_none()
    );
    assert!(recorder.process(event(InputKind::Down(CAPS))).is_none());
    assert!(recorder.process(event(InputKind::Down(W))).is_none());
    assert!(recorder.process(event(InputKind::Down(W))).is_none());
    assert!(recorder.process(event(InputKind::Up(CAPS))).is_none());
    assert_eq!(
        recorder.process(event(InputKind::Up(W))),
        Some(press(0, &[CAPS], W))
    );
    assert!(recorder.process(event(InputKind::Down(Q))).is_none());
    assert!(recorder.process(event(InputKind::Up(Q))).is_none());
}

#[test]
fn recorder_wheel_uses_first_nonzero_direction_and_reset_starts_new_recording() {
    let mut recorder = Recorder::new();
    assert!(
        recorder
            .process(InputEvent {
                kind: InputKind::Down(CAPS),
                injected: true
            })
            .is_none()
    );
    assert!(recorder.process(scroll(0)).is_none());
    recorder.process(event(InputKind::Down(SIDE)));
    assert_eq!(
        recorder.process(scroll(-1)),
        Some(wheel(0, &[SIDE], WheelDirection::Down))
    );
    assert!(recorder.process(scroll(120)).is_none());
    recorder.reset();
    recorder.process(event(InputKind::Down(Q)));
    assert_eq!(
        recorder.process(event(InputKind::Up(Q))),
        Some(press(0, &[], Q))
    );
}

#[test]
fn hold_timer_fires_once_without_more_input_and_consumes_its_release() {
    let mut engine = Engine::new(vec![hold(1, &[], RIGHT, 300)]).unwrap();
    assert!(
        engine
            .process_at(event(InputKind::Down(RIGHT)), 1000)
            .consume
    );
    assert_eq!(engine.next_deadline_ms(), Some(1300));
    assert_eq!(engine.advance_time(1299), Decision::default());
    assert_eq!(
        engine.advance_time(1300),
        Decision {
            triggered: vec![1],
            ..Decision::default()
        }
    );
    assert_eq!(engine.next_deadline_ms(), None);
    assert_eq!(engine.advance_time(5000), Decision::default());
    let repeat = engine.process_at(event(InputKind::Down(RIGHT)), 5000);
    assert!(repeat.consume && repeat.triggered.is_empty());
    let release = engine.process_at(event(InputKind::Up(RIGHT)), 5001);
    assert!(release.consume && release.replay.is_empty());
    assert!(!engine.process_at(event(InputKind::Up(RIGHT)), 5002).consume);
}

#[test]
fn hold_release_before_deadline_replays_both_edges_but_exact_deadline_fires() {
    for (release_at, fired) in [(299, false), (300, true), (1000, true)] {
        let mut engine = Engine::new(vec![hold(1, &[], RIGHT, 300)]).unwrap();
        engine.process_at(event(InputKind::Down(RIGHT)), 0);
        let release = engine.process_at(event(InputKind::Up(RIGHT)), release_at);
        assert!(release.consume);
        if fired {
            assert_eq!(release.triggered, vec![1]);
            assert!(release.replay.is_empty());
        } else {
            assert!(release.triggered.is_empty());
            assert_eq!(
                release.replay,
                vec![InputKind::Down(RIGHT), InputKind::Up(RIGHT)]
            );
        }
        assert_eq!(engine.next_deadline_ms(), None);
        assert_eq!(engine.advance_time(2000), Decision::default());
    }
}

#[test]
fn keyboard_hold_repeat_and_backwards_time_do_not_restart_the_deadline() {
    let mut engine = Engine::new(vec![hold(1, &[], W, 300)]).unwrap();
    engine.advance_time(1000);
    assert!(engine.process(event(InputKind::Down(W))).consume);
    assert_eq!(engine.next_deadline_ms(), Some(1300));
    assert!(engine.process_at(event(InputKind::Down(W)), 1200).consume);
    assert!(engine.process_at(event(InputKind::Down(W)), 1100).consume);
    assert_eq!(engine.next_deadline_ms(), Some(1300));
    assert_eq!(engine.advance_time(1299), Decision::default());
    assert_eq!(engine.advance_time(1300).triggered, vec![1]);
    assert!(engine.process(event(InputKind::Up(W))).consume);
}

#[test]
fn injected_input_cannot_advance_cancel_or_release_a_hold() {
    let mut engine = Engine::new(vec![hold(1, &[], W, 300)]).unwrap();
    engine.process_at(event(InputKind::Down(W)), 1000);
    for kind in [InputKind::Up(W), InputKind::Down(Q)] {
        assert_eq!(
            engine.process_at(
                InputEvent {
                    kind,
                    injected: true,
                },
                5000,
            ),
            Decision::default()
        );
    }
    assert_eq!(engine.next_deadline_ms(), Some(1300));
    assert_eq!(engine.advance_time(1299), Decision::default());
    assert_eq!(engine.advance_time(1300).triggered, vec![1]);
    assert!(engine.process_at(event(InputKind::Up(W)), 1301).consume);
}

#[test]
fn mouse_hold_has_an_eight_pixel_radius_including_negative_coordinates() {
    let mut engine = Engine::new(vec![hold(1, &[], RIGHT, 300)]).unwrap();
    engine.pointer_moved((-1920, -100), 0);
    engine.process_at(event(InputKind::Down(RIGHT)), 0);
    assert_eq!(
        engine.pointer_moved((-1912, -100), 100),
        Decision::default()
    );
    assert_eq!(
        engine.pointer_moved((-1920, -108), 200),
        Decision::default()
    );
    let moved = engine.pointer_moved((-1914, -94), 299);
    assert!(!moved.consume);
    assert_eq!(moved.replay, vec![InputKind::Down(RIGHT)]);
    assert_eq!(engine.next_deadline_ms(), None);
    assert_eq!(engine.advance_time(300), Decision::default());
    assert!(!engine.process_at(event(InputKind::Up(RIGHT)), 301).consume);
}

#[test]
fn extreme_pointer_coordinates_do_not_overflow_and_only_mouse_holds_cancel() {
    for button in [RIGHT, W] {
        let mut engine = Engine::new(vec![hold(1, &[], button, 300)]).unwrap();
        engine.pointer_moved((i32::MIN, i32::MIN), 0);
        engine.process_at(event(InputKind::Down(button)), 0);
        let moved = engine.pointer_moved((i32::MAX, i32::MAX), 299);
        if button == RIGHT {
            assert_eq!(moved.replay, vec![InputKind::Down(RIGHT)]);
            assert_eq!(engine.advance_time(300), Decision::default());
            assert!(!engine.process_at(event(InputKind::Up(RIGHT)), 301).consume);
        } else {
            assert_eq!(moved, Decision::default());
            assert_eq!(engine.advance_time(300).triggered, vec![1]);
            assert!(engine.process_at(event(InputKind::Up(W)), 301).consume);
        }
    }
}

#[test]
fn pointer_motion_at_the_deadline_fires_before_processing_the_motion() {
    let mut engine = Engine::new(vec![hold(1, &[], RIGHT, 300)]).unwrap();
    engine.pointer_moved((0, 0), 0);
    engine.process_at(event(InputKind::Down(RIGHT)), 0);
    let moved = engine.pointer_moved((1000, 1000), 300);
    assert_eq!(moved.triggered, vec![1]);
    assert!(moved.replay.is_empty());
    assert!(engine.process_at(event(InputKind::Up(RIGHT)), 301).consume);
}

#[test]
fn another_button_or_nonzero_wheel_cancels_and_hands_the_hold_back() {
    let mut engine = Engine::new(vec![hold(1, &[], RIGHT, 300)]).unwrap();
    engine.process_at(event(InputKind::Down(RIGHT)), 0);
    let interrupted = engine.process_at(event(InputKind::Down(Q)), 100);
    assert!(!interrupted.consume);
    assert_eq!(interrupted.replay, vec![InputKind::Down(RIGHT)]);
    assert!(!engine.process_at(event(InputKind::Up(Q)), 101).consume);
    assert!(!engine.process_at(event(InputKind::Up(RIGHT)), 102).consume);
    assert_eq!(engine.advance_time(300), Decision::default());
    engine.process_at(event(InputKind::Down(RIGHT)), 400);
    assert_eq!(engine.process_at(scroll(0), 499), Decision::default());
    assert_eq!(engine.next_deadline_ms(), Some(700));
    let wheel = engine.process_at(scroll(120), 500);
    assert!(!wheel.consume);
    assert_eq!(wheel.replay, vec![InputKind::Down(RIGHT)]);
    assert!(!engine.process_at(event(InputKind::Up(RIGHT)), 501).consume);
    assert_eq!(engine.advance_time(700), Decision::default());
    engine.process_at(event(InputKind::Down(RIGHT)), 800);
    assert_eq!(engine.advance_time(1100).triggered, vec![1]);
}

#[test]
fn hold_prefixes_use_the_exact_set_and_release_fallback_stays_paired() {
    let mut engine = Engine::new(vec![hold(1, &[CAPS], W, 300)]).unwrap();
    engine.process_at(event(InputKind::Down(SHIFT)), 0);
    engine.process_at(event(InputKind::Down(CAPS)), 1);
    engine.process_at(event(InputKind::Down(W)), 2);
    assert_eq!(engine.next_deadline_ms(), None);
    for button in [SHIFT, CAPS, W] {
        engine.process_at(event(InputKind::Up(button)), 3);
    }
    engine.process_at(event(InputKind::Down(CAPS)), 100);
    engine.process_at(event(InputKind::Down(W)), 200);
    assert_eq!(engine.next_deadline_ms(), Some(500));
    // 已按住前缀的重复事件同样不是新的按压。
    engine.process_at(event(InputKind::Down(CAPS)), 250);
    assert_eq!(engine.next_deadline_ms(), Some(500));
    let release = engine.process_at(event(InputKind::Up(CAPS)), 300);
    assert!(release.consume);
    assert_eq!(
        release.replay,
        vec![
            InputKind::Down(CAPS),
            InputKind::Down(W),
            InputKind::Up(CAPS)
        ]
    );
    assert_eq!(engine.next_deadline_ms(), None);
    assert!(!engine.process_at(event(InputKind::Up(W)), 301).consume);
    engine.process_at(event(InputKind::Down(CAPS)), 600);
    engine.process_at(event(InputKind::Down(W)), 700);
    assert_eq!(engine.advance_time(1000).triggered, vec![1]);
    assert!(engine.process_at(event(InputKind::Up(CAPS)), 1001).consume);
    assert!(engine.process_at(event(InputKind::Up(W)), 1002).consume);
}

#[test]
fn cancel_disable_and_replace_drop_hold_buffers_and_drain_releases() {
    let mut engine = Engine::new(vec![hold(1, &[], RIGHT, 300)]).unwrap();
    for mode in 0..3 {
        let start = mode * 1000;
        engine.process_at(event(InputKind::Down(RIGHT)), start);
        match mode {
            0 => engine.cancel(),
            1 => engine.set_enabled(false),
            _ => engine
                .replace_bindings(vec![hold(2, &[], RIGHT, 300)])
                .unwrap(),
        };
        assert_eq!(engine.next_deadline_ms(), None);
        assert_eq!(engine.advance_time(start + 500), Decision::default());
        let release = engine.process_at(event(InputKind::Up(RIGHT)), start + 501);
        assert!(release.consume && release.replay.is_empty());
        engine.set_enabled(true);
    }
    engine.process_at(event(InputKind::Down(RIGHT)), 3000);
    assert_eq!(engine.advance_time(3300).triggered, vec![2]);
}

#[test]
fn invalid_hold_replacement_preserves_the_original_pending_deadline() {
    let mut engine = Engine::new(vec![hold(1, &[], RIGHT, 300)]).unwrap();
    engine.process_at(event(InputKind::Down(RIGHT)), 1000);
    assert_eq!(
        engine.replace_bindings(vec![hold(2, &[], RIGHT, 49)]),
        Err(RegistrationError::InvalidHoldDuration {
            id: 2,
            duration_ms: 49,
        })
    );
    assert_eq!(engine.next_deadline_ms(), Some(1300));
    assert_eq!(engine.advance_time(1300).triggered, vec![1]);
}

#[test]
fn hold_registration_rejects_bad_durations_and_ambiguous_bindings() {
    for duration_ms in [0, 49, 10001, u32::MAX] {
        assert!(matches!(
            Engine::new(vec![hold(1, &[], RIGHT, duration_ms)]),
            Err(RegistrationError::InvalidHoldDuration {
                id: 1,
                duration_ms: value,
            }) if value == duration_ms
        ));
    }
    for duration_ms in [50, 10000] {
        assert!(Engine::new(vec![hold(1, &[], RIGHT, duration_ms)]).is_ok());
    }
    for other in [press(2, &[], RIGHT), hold(2, &[], RIGHT, 500)] {
        assert!(matches!(
            Engine::new(vec![hold(1, &[], RIGHT, 300), other]),
            Err(RegistrationError::DuplicateTrigger {
                first: 1,
                second: 2,
            })
        ));
    }
    assert!(matches!(
        Engine::new(vec![
            hold(1, &[], RIGHT, 300),
            wheel(2, &[RIGHT], WheelDirection::Up)
        ]),
        Err(RegistrationError::TerminalIsPrefix {
            terminal_binding: 1,
            prefix_binding: 2,
        })
    ));
    assert!(matches!(
        Engine::new(vec![hold(1, &[W], W, 300)]),
        Err(RegistrationError::TerminalInHeld { id: 1 })
    ));
    assert!(matches!(
        Engine::new(vec![hold(1, &[], Button::Key(0), 300)]),
        Err(RegistrationError::InvalidButton { id: 1, .. })
    ));
}
