use super::*;
use omnitrigger_core::{Activation, ProcessScope, WheelDirection, WheelPolicy, WindowRole};

const SIDE: Button = Button::Mouse(MouseButton::X1);
const POINT: (i32, i32) = (20, 30);
fn setup() -> (ThreadState, Receiver<BackendEvent>) {
    let (tx, rx) = mpsc::sync_channel(EVENT_CAPACITY);
    let mut state = ThreadState::new(tx);
    state.pointer.store(POINT);
    state.command(BackendCommand::SetBindings(vec![TriggerSpec {
        id: 7,
        held: vec![SIDE],
        activation: Activation::Wheel(WheelDirection::Up),
        wheel: WheelPolicy::Packet,
        keyboard: Default::default(),
        conditions: vec![],
        process_scope: Default::default(),
        pointer_scope: ProcessScope {
            allow: vec!["reader".into()],
            ..Default::default()
        },
    }]));
    (state, rx)
}
fn snapshot(point: (i32, i32), age_ms: u64) -> pointer_context::Snapshot {
    pointer_context::Snapshot {
        point,
        identity: foreground_context::Identity {
            hwnd: 123,
            pid: 456,
        },
        name: Some("reader".into()),
        role: WindowRole::Application,
        observed: Instant::now() - Duration::from_millis(age_ms),
    }
}
fn sample(state: &mut ThreadState) {
    state.apply_pointer_context(snapshot(POINT, 0), state.foreground_epoch);
}

#[test]
fn pointer_context_backend_freezes_foreground_independently_of_the_matching_window() {
    let (mut state, events) = setup();
    sample(&mut state);
    let expected_foreground = state.foreground as isize;
    assert!(
        state
            .input_at(Some(InputKind::Down(SIDE)), None, Some(POINT), 1)
            .consume
    );
    assert!(
        state
            .input_at(
                Some(InputKind::Wheel {
                    axis: Axis::Vertical,
                    delta: 15
                }),
                None,
                Some(POINT),
                2
            )
            .consume
    );
    let hits: Vec<_> = events
        .try_iter()
        .filter_map(|event| match event {
            BackendEvent::Triggered {
                id,
                pointer,
                foreground,
            } => Some((id, pointer, foreground)),
            _ => None,
        })
        .collect();
    assert_eq!(hits, [(7, Some(POINT), expected_foreground)]);
    assert_eq!(state.diagnostics().pointer_window, Some(123));
    assert_eq!(state.foreground as isize, expected_foreground);
}

#[test]
fn pointer_context_backend_rejects_stale_epoch_point_and_expired_probe() {
    let (mut state, _) = setup();
    for (probe, epoch) in [
        (snapshot(POINT, 0), state.foreground_epoch.wrapping_add(1)),
        (snapshot((21, 30), 0), state.foreground_epoch),
        (snapshot(POINT, 250), state.foreground_epoch),
    ] {
        state.apply_pointer_context(probe, epoch);
        assert_eq!(state.engine.pointer_process(), None);
        assert!(state.pointer_context.is_none());
    }
    sample(&mut state);
    assert_eq!(state.engine.pointer_process(), Some("reader"));
}

#[test]
fn pointer_context_backend_invalidates_before_a_new_coordinate_can_trigger() {
    let (mut state, events) = setup();
    sample(&mut state);
    state.input_at(Some(InputKind::Down(SIDE)), None, Some(POINT), 1);
    let changed = state.input_at(
        Some(InputKind::Wheel {
            axis: Axis::Vertical,
            delta: 15,
        }),
        None,
        Some((21, 30)),
        2,
    );
    assert_eq!(state.engine.pointer_process(), None);
    assert_eq!(
        changed.replay_kinds,
        [
            InputKind::Down(SIDE),
            InputKind::Wheel {
                axis: Axis::Vertical,
                delta: 15
            }
        ]
    );
    assert!(!events
        .try_iter()
        .any(|event| matches!(event, BackendEvent::Triggered { .. })));
    assert!(state.diagnostics().pointer_process.is_none());
}

#[test]
fn pointer_context_backend_expiry_cancels_before_timed_matching() {
    let (mut state, events) = setup();
    state.command(BackendCommand::SetBindings(vec![TriggerSpec {
        id: 8,
        held: vec![],
        activation: Activation::Hold {
            button: SIDE,
            duration_ms: 300,
        },
        wheel: Default::default(),
        keyboard: Default::default(),
        conditions: vec![],
        process_scope: Default::default(),
        pointer_scope: ProcessScope {
            allow: vec!["reader".into()],
            ..Default::default()
        },
    }]));
    sample(&mut state);
    state.input_at(Some(InputKind::Down(SIDE)), None, Some(POINT), 1);
    assert_eq!(state.engine.next_hold_deadline_ms(), Some(301));
    state.pointer_context.as_mut().unwrap().observed = Instant::now() - pointer_context::MAX_AGE;
    state.advance_time_at(400);
    assert_eq!(state.engine.pointer_process(), None);
    assert!(state.pointer_context.is_none());
    assert!(state.engine.next_hold_deadline_ms().is_none());
    assert!(!events
        .try_iter()
        .any(|event| matches!(event, BackendEvent::Triggered { .. })));
}
