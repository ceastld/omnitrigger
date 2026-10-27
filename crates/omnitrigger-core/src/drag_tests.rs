use crate::*;

const RIGHT: Button = Button::Mouse(MouseButton::Right);
const CAPS: Button = Button::Key(0x14);
const Q: Button = Button::Key(0x51);

fn drag(id: u64, held: &[Button], button: Button, distance: u32, window: u32) -> TriggerSpec {
    TriggerSpec {
        conditions: vec![],
        id,
        held: held.to_vec(),
        activation: Activation::Drag {
            button,
            min_distance_px: distance,
            max_start_ms: window,
        },
    }
}

fn hold(id: u64, held: &[Button], duration: u32) -> TriggerSpec {
    TriggerSpec {
        conditions: vec![],
        id,
        held: held.to_vec(),
        activation: Activation::Hold {
            button: RIGHT,
            duration_ms: duration,
        },
    }
}

fn input(engine: &mut Engine, kind: InputKind, now: u64) -> Decision {
    engine.process_at(
        InputEvent {
            kind,
            injected: false,
        },
        now,
    )
}

fn down(engine: &mut Engine, held: bool) {
    engine.pointer_moved((0, 0), 0);
    if held {
        assert!(input(engine, InputKind::Down(CAPS), 0).consume);
    }
    assert!(input(engine, InputKind::Down(RIGHT), 0).consume);
}

#[test]
fn fast_drag_keeps_waiting_after_hold_movement_cancellation_then_starts_and_ends_once() {
    let mut engine = Engine::new(vec![hold(1, &[], 300), drag(2, &[], RIGHT, 18, 180)]).unwrap();
    down(&mut engine, false);
    assert_eq!(engine.next_deadline_ms(), Some(180));
    assert_eq!(engine.next_hold_deadline_ms(), Some(300));
    assert_eq!(engine.pointer_moved((9, 0), 20), Decision::default());
    assert_eq!(engine.next_hold_deadline_ms(), None);
    assert_eq!(engine.pending_drag_id(), Some(2));
    assert_eq!(engine.pointer_moved((17, 0), 178), Decision::default());
    let started = engine.pointer_moved((18, 0), 179);
    assert!(!started.consume && started.replay.is_empty());
    assert_eq!(started.triggered, [2]);
    assert_eq!(
        started.drags,
        [DragEvent {
            id: 2,
            session: 1,
            phase: DragPhase::Started,
            origin: (0, 0),
            position: (18, 0),
        }]
    );
    assert_eq!(engine.next_deadline_ms(), None);
    assert_eq!(engine.pointer_moved((100, 40), 250), Decision::default());
    assert_eq!(engine.advance_time(300), Decision::default());
    assert_eq!(engine.active_drag().unwrap().position, (100, 40));
    let ended = input(&mut engine, InputKind::Up(RIGHT), 301);
    assert!(ended.consume && ended.replay.is_empty() && ended.triggered.is_empty());
    assert_eq!(
        ended.drags,
        [DragEvent {
            id: 2,
            session: 1,
            phase: DragPhase::Ended,
            origin: (0, 0),
            position: (100, 40),
        }]
    );
    assert_eq!(engine.active_drag(), None);
    assert_eq!(
        input(&mut engine, InputKind::Up(RIGHT), 302),
        Decision::default()
    );
}

#[test]
fn stationary_drag_timeout_preserves_hold_and_hold_wins_after_its_threshold() {
    let mut engine = Engine::new(vec![drag(2, &[], RIGHT, 18, 180), hold(1, &[], 300)]).unwrap();
    down(&mut engine, false);
    assert_eq!(engine.advance_time(180), Decision::default());
    assert_eq!(engine.pending_drag_id(), None);
    assert_eq!(engine.next_deadline_ms(), Some(300));
    assert_eq!(engine.pointer_moved((8, 0), 299), Decision::default());
    let held = engine.pointer_moved((100, 100), 300);
    assert_eq!(held.triggered, [1]);
    assert!(held.drags.is_empty() && held.replay.is_empty());
    assert!(input(&mut engine, InputKind::Up(RIGHT), 301).consume);
}

#[test]
fn failed_fast_drag_replays_down_at_timeout_and_slow_drag_then_passes_through() {
    let mut engine = Engine::new(vec![hold(1, &[], 300), drag(2, &[], RIGHT, 18, 180)]).unwrap();
    down(&mut engine, false);
    assert_eq!(engine.pointer_moved((9, 0), 30), Decision::default());
    assert_eq!(engine.advance_time(179), Decision::default());
    let timed_out = engine.advance_time(180);
    assert_eq!(timed_out.replay, [InputKind::Down(RIGHT)]);
    assert!(timed_out.drags.is_empty() && timed_out.triggered.is_empty());
    assert_eq!(engine.pointer_moved((50, 0), 190), Decision::default());
    assert_eq!(engine.advance_time(300), Decision::default());
    assert_eq!(
        input(&mut engine, InputKind::Up(RIGHT), 301),
        Decision::default()
    );
}

#[test]
fn short_click_and_prefix_release_fallback_keep_all_edges_paired() {
    for release in [RIGHT, CAPS] {
        let mut engine = Engine::new(vec![
            hold(1, &[CAPS], 300),
            drag(2, &[CAPS], RIGHT, 18, 180),
        ])
        .unwrap();
        down(&mut engine, true);
        let short = input(&mut engine, InputKind::Up(release), 100);
        assert!(short.consume && short.drags.is_empty());
        assert_eq!(
            short.replay,
            [
                InputKind::Down(CAPS),
                InputKind::Down(RIGHT),
                InputKind::Up(release)
            ]
        );
        let other = if release == RIGHT { CAPS } else { RIGHT };
        assert!(!input(&mut engine, InputKind::Up(other), 101).consume);
        assert_eq!(engine.advance_time(1000), Decision::default());
    }
    let mut engine = Engine::new(vec![drag(2, &[], RIGHT, 18, 180)]).unwrap();
    down(&mut engine, false);
    let short = input(&mut engine, InputKind::Up(RIGHT), 179);
    assert_eq!(short.replay, [InputKind::Down(RIGHT), InputKind::Up(RIGHT)]);
    assert!(short.consume && short.drags.is_empty());
}

#[test]
fn movement_must_reach_distance_strictly_before_the_start_deadline() {
    for now in [179, 180, 181] {
        let mut engine =
            Engine::new(vec![hold(1, &[], 300), drag(2, &[], RIGHT, 18, 180)]).unwrap();
        down(&mut engine, false);
        let moved = engine.pointer_moved((18, 0), now);
        if now == 179 {
            assert_eq!(moved.triggered, [2]);
            assert_eq!(moved.drags[0].phase, DragPhase::Started);
        } else {
            assert!(moved.triggered.is_empty() && moved.drags.is_empty());
            assert_eq!(moved.replay, [InputKind::Down(RIGHT)]);
            assert_eq!(engine.advance_time(300), Decision::default());
        }
    }
    for (position, expected) in [
        ((17, 0), false),
        ((18, 0), true),
        ((12, 12), false),
        ((13, 13), true),
    ] {
        let mut engine = Engine::new(vec![drag(2, &[], RIGHT, 18, 180)]).unwrap();
        down(&mut engine, false);
        assert_eq!(
            !engine.pointer_moved(position, 100).drags.is_empty(),
            expected
        );
    }
}

#[test]
fn repeats_and_injected_events_cannot_restart_cancel_or_advance_drag_state() {
    let mut engine = Engine::new(vec![hold(1, &[], 300), drag(2, &[], RIGHT, 18, 180)]).unwrap();
    down(&mut engine, false);
    let repeat = input(&mut engine, InputKind::Down(RIGHT), 50);
    assert!(repeat.consume && repeat.drags.is_empty());
    for kind in [
        InputKind::Up(RIGHT),
        InputKind::Down(Q),
        InputKind::Wheel {
            axis: Axis::Vertical,
            delta: 120,
        },
    ] {
        assert_eq!(
            engine.process_at(
                InputEvent {
                    kind,
                    injected: true
                },
                10_000
            ),
            Decision::default()
        );
    }
    assert_eq!(engine.next_deadline_ms(), Some(180));
    let started = engine.pointer_moved((18, 0), 179);
    assert_eq!(started.triggered, [2]);
    for kind in [InputKind::Up(RIGHT), InputKind::Down(Q)] {
        assert_eq!(
            engine.process_at(
                InputEvent {
                    kind,
                    injected: true
                },
                20_000
            ),
            Decision::default()
        );
    }
    assert!(engine.active_drag().is_some());
    assert_eq!(engine.advance_time(180), Decision::default());
    let repeat = input(&mut engine, InputKind::Down(RIGHT), 181);
    assert!(repeat.consume && repeat.drags.is_empty() && repeat.triggered.is_empty());
    assert_eq!(
        input(&mut engine, InputKind::Up(RIGHT), 182).drags[0].phase,
        DragPhase::Ended
    );
}

#[test]
fn active_drag_cancellation_is_reported_once_and_old_releases_are_drained() {
    for mode in 0..6 {
        let mut engine = Engine::new(vec![drag(2, &[CAPS], RIGHT, 18, 180)]).unwrap();
        down(&mut engine, true);
        let started = engine.pointer_moved((18, 0), 10).drags[0];
        engine.pointer_moved((25, -5), 20);
        let cancelled = match mode {
            0 => engine.cancel(),
            1 => engine.set_enabled(false),
            2 => engine
                .replace_bindings(vec![drag(3, &[CAPS], RIGHT, 18, 180)])
                .unwrap(),
            3 => input(&mut engine, InputKind::Down(Q), 30),
            4 => input(
                &mut engine,
                InputKind::Wheel {
                    axis: Axis::Vertical,
                    delta: 120,
                },
                30,
            ),
            _ => input(&mut engine, InputKind::Up(CAPS), 30),
        };
        assert_eq!(
            cancelled.drags,
            [DragEvent {
                phase: DragPhase::Cancelled,
                position: (25, -5),
                ..started
            }]
        );
        assert!(cancelled.replay.is_empty());
        assert_eq!(engine.active_drag(), None);
        assert_eq!(engine.cancel().drags, []);
        let terminal_up = input(&mut engine, InputKind::Up(RIGHT), 40);
        assert!(
            terminal_up.consume && terminal_up.replay.is_empty() && terminal_up.drags.is_empty()
        );
        if mode != 5 {
            let prefix_up = input(&mut engine, InputKind::Up(CAPS), 41);
            assert!(prefix_up.consume && prefix_up.replay.is_empty());
        }
    }
}

#[test]
fn pending_drag_extra_input_falls_back_but_extra_held_keys_never_claim_it() {
    let mut engine = Engine::new(vec![drag(2, &[CAPS], RIGHT, 18, 180)]).unwrap();
    input(&mut engine, InputKind::Down(Q), 0);
    input(&mut engine, InputKind::Down(CAPS), 1);
    assert!(!input(&mut engine, InputKind::Down(RIGHT), 2).consume);
    assert_eq!(engine.pointer_moved((100, 0), 10), Decision::default());
    assert_eq!(engine.pending_drag_id(), None);
    for key in [Q, CAPS, RIGHT] {
        input(&mut engine, InputKind::Up(key), 11);
    }
    engine.pointer_moved((0, 0), 20);
    assert!(input(&mut engine, InputKind::Down(CAPS), 20).consume);
    assert!(input(&mut engine, InputKind::Down(RIGHT), 20).consume);
    let zero_wheel = input(
        &mut engine,
        InputKind::Wheel {
            axis: Axis::Vertical,
            delta: 0,
        },
        21,
    );
    assert_eq!(zero_wheel, Decision::default());
    let extra = input(&mut engine, InputKind::Down(Q), 22);
    assert_eq!(
        extra.replay,
        [InputKind::Down(CAPS), InputKind::Down(RIGHT)]
    );
    assert!(!extra.consume && extra.drags.is_empty());
    assert_eq!(engine.pending_drag_id(), None);
}

#[test]
fn late_session_cancellation_does_not_affect_a_new_session_and_replacement_does_not_reuse_ids() {
    let mut engine = Engine::new(vec![drag(2, &[], RIGHT, 18, 180)]).unwrap();
    down(&mut engine, false);
    let first = engine.pointer_moved((18, 0), 10).drags[0];
    assert_eq!(
        engine.cancel_drag(first.session).drags[0].phase,
        DragPhase::Cancelled
    );
    input(&mut engine, InputKind::Up(RIGHT), 11);
    engine
        .replace_bindings(vec![drag(2, &[], RIGHT, 18, 180)])
        .unwrap();
    engine.pointer_moved((0, 0), 20);
    input(&mut engine, InputKind::Down(RIGHT), 20);
    let second = engine.pointer_moved((18, 0), 30).drags[0];
    assert!(second.session > first.session);
    assert_eq!(engine.cancel_drag(first.session), Decision::default());
    assert_eq!(engine.active_drag().unwrap().session, second.session);
    assert_eq!(
        input(&mut engine, InputKind::Up(RIGHT), 40).drags[0].session,
        second.session
    );
}

#[test]
fn extreme_coordinates_are_bounded_and_exhausted_sessions_fall_back_without_reuse() {
    let mut engine = Engine::new(vec![drag(2, &[], RIGHT, 500, 180)]).unwrap();
    engine.pointer_moved((i32::MIN, i32::MIN), 0);
    input(&mut engine, InputKind::Down(RIGHT), 0);
    assert_eq!(
        engine.pointer_moved((i32::MAX, i32::MAX), 10).drags[0].phase,
        DragPhase::Started
    );
    input(&mut engine, InputKind::Up(RIGHT), 11);
    engine.next_drag_session = Some(u64::MAX);
    engine.pointer_moved((0, 0), 20);
    input(&mut engine, InputKind::Down(RIGHT), 20);
    assert_eq!(
        engine.pointer_moved((500, 0), 30).drags[0].session,
        u64::MAX
    );
    input(&mut engine, InputKind::Up(RIGHT), 31);
    engine.pointer_moved((0, 0), 40);
    input(&mut engine, InputKind::Down(RIGHT), 40);
    let exhausted = engine.pointer_moved((500, 0), 50);
    assert!(exhausted.drags.is_empty() && exhausted.triggered.is_empty());
    assert_eq!(exhausted.replay, [InputKind::Down(RIGHT)]);
    assert!(!input(&mut engine, InputKind::Up(RIGHT), 51).consume);
}

#[test]
fn registration_validates_drag_parameters_timing_and_terminal_prefix_ambiguity() {
    for distance in [0, 3, 501, u32::MAX] {
        assert!(
            matches!(Engine::new(vec![drag(1, &[], RIGHT, distance, 180)]), Err(RegistrationError::InvalidDragDistance { id: 1, min_distance_px }) if min_distance_px == distance)
        );
    }
    for window in [0, 19, 2001, u32::MAX] {
        assert!(
            matches!(Engine::new(vec![drag(1, &[], RIGHT, 18, window)]), Err(RegistrationError::InvalidDragWindow { id: 1, max_start_ms }) if max_start_ms == window)
        );
    }
    assert!(matches!(
        Engine::new(vec![drag(1, &[], Q, 18, 180)]),
        Err(RegistrationError::InvalidDragButton { id: 1, button: Q })
    ));
    for (distance, window) in [(4, 20), (500, 2000)] {
        assert!(Engine::new(vec![drag(1, &[], RIGHT, distance, window)]).is_ok());
    }
    for (window, duration) in [(300, 300), (301, 300)] {
        for reverse in [false, true] {
            let mut bindings = vec![hold(1, &[], duration), drag(2, &[], RIGHT, 18, window)];
            if reverse {
                bindings.reverse();
            }
            assert!(matches!(
                Engine::new(bindings),
                Err(RegistrationError::TimingConflict {
                    hold_binding: 1,
                    drag_binding: 2,
                    hold_duration_ms: 300,
                    ..
                })
            ));
        }
    }
    assert!(Engine::new(vec![hold(1, &[], 300), drag(2, &[], RIGHT, 18, 299)]).is_ok());
    for other in [
        drag(2, &[], RIGHT, 30, 200),
        TriggerSpec {
            conditions: vec![],
            id: 2,
            held: vec![],
            activation: Activation::Press(RIGHT),
        },
    ] {
        assert!(matches!(
            Engine::new(vec![drag(1, &[], RIGHT, 18, 180), other]),
            Err(RegistrationError::DuplicateTrigger {
                first: 1,
                second: 2
            })
        ));
    }
    assert!(matches!(
        Engine::new(vec![
            drag(1, &[], RIGHT, 18, 180),
            TriggerSpec {
                conditions: vec![],
                id: 2,
                held: vec![RIGHT],
                activation: Activation::Press(Q)
            }
        ]),
        Err(RegistrationError::TerminalIsPrefix {
            terminal_binding: 1,
            prefix_binding: 2
        })
    ));
}

#[test]
fn invalid_drag_replacement_keeps_the_original_pending_or_active_session() {
    let mut engine = Engine::new(vec![drag(1, &[], RIGHT, 18, 180)]).unwrap();
    down(&mut engine, false);
    assert!(
        engine
            .replace_bindings(vec![drag(2, &[], RIGHT, 3, 180)])
            .is_err()
    );
    assert_eq!(engine.next_deadline_ms(), Some(180));
    let started = engine.pointer_moved((18, 0), 100).drags[0];
    assert!(
        engine
            .replace_bindings(vec![drag(2, &[], RIGHT, 18, 19)])
            .is_err()
    );
    assert_eq!(engine.active_drag().unwrap().session, started.session);
    let ended = input(&mut engine, InputKind::Up(RIGHT), 101);
    assert_eq!(ended.drags[0].phase, DragPhase::Ended);
}
