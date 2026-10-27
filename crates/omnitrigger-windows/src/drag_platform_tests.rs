//! 平台事件序列测试，不安装 Hook，也不向桌面发送输入。
use super::*;
use omnitrigger_core::Activation;

const RIGHT: Button = Button::Mouse(MouseButton::Right);

fn bindings(with_hold: bool) -> Vec<TriggerSpec> {
    let mut bindings = vec![TriggerSpec {
        conditions: Vec::new(),
        id: 2,
        held: vec![],
        activation: Activation::Drag {
            button: RIGHT,
            min_distance_px: 18,
            max_start_ms: 180,
        },
    }];
    if with_hold {
        bindings.push(TriggerSpec {
            conditions: Vec::new(),
            id: 1,
            held: vec![],
            activation: Activation::Hold {
                button: RIGHT,
                duration_ms: 300,
            },
        });
    }
    bindings
}

fn setup(with_hold: bool) -> (ThreadState, Receiver<BackendEvent>) {
    let (sender, receiver) = mpsc::sync_channel(EVENT_CAPACITY);
    let mut state = ThreadState::new(sender);
    state.command(BackendCommand::SetBindings(bindings(with_hold)));
    (state, receiver)
}

fn down(state: &mut ThreadState, now: u64) {
    assert!(
        state
            .input_at(Some(InputKind::Down(RIGHT)), None, Some((100, 100)), now)
            .consume
    );
}

fn start(state: &mut ThreadState, now: u64) -> u64 {
    let outcome = state.input_at(None, None, Some((118, 100)), now);
    assert!(!outcome.consume && outcome.replay_kinds.is_empty());
    state
        .engine
        .active_drag()
        .expect("快速移动应认领拖拽")
        .session
}

#[test]
fn pointer_snapshot_keeps_signed_pairs_and_ignores_keyboard_events() {
    let (mut state, _) = setup(false);
    assert_eq!(state.pointer.load(), None);
    state.input_at(None, None, Some((i32::MIN, i32::MAX)), 0);
    assert_eq!(state.pointer.load(), Some((i32::MIN, i32::MAX)));
    state.input_at(Some(InputKind::Down(Button::Key(0x41))), None, None, 1);
    assert_eq!(state.pointer.load(), Some((i32::MIN, i32::MAX)));
    state.input_at(None, None, Some((-1920, -1080)), 2);
    assert_eq!(state.pointer.load(), Some((-1920, -1080)));
}

#[test]
fn quick_drag_starts_before_trigger_then_ends_once_and_consumes_release() {
    let (mut state, events) = setup(true);
    let foreground = state.foreground as isize;
    down(&mut state, 0);
    assert_eq!(state.engine.next_deadline_ms(), Some(180));
    assert_eq!(state.engine.next_hold_deadline_ms(), Some(300));
    assert_eq!(state.diagnostics().hold_started, 1);
    assert_eq!(state.active_hold.unwrap().threshold_ms, 300);
    let session = start(&mut state, 100);
    let lifecycle: Vec<_> = events
        .try_iter()
        .filter_map(|event| match event {
            BackendEvent::Drag {
                event,
                foreground: actual,
            } => {
                assert_eq!(foreground, actual);
                assert_eq!(event.session, session);
                assert_eq!(event.origin, (100, 100));
                Some("start")
            }
            BackendEvent::Triggered { id: 2, .. } => Some("trigger"),
            BackendEvent::HoldTiming { .. } => panic!("快拖不能被记录成长按命中"),
            _ => None,
        })
        .collect();
    assert_eq!(lifecycle, vec!["start", "trigger"]);
    assert_eq!(state.diagnostics().hold_triggered, 0);
    let release = state.input_at(Some(InputKind::Up(RIGHT)), None, Some((150, 100)), 120);
    assert!(release.consume && release.replay_kinds.is_empty());
    let terminal: Vec<_> = events
        .try_iter()
        .filter_map(|event| match event {
            BackendEvent::Drag { event, .. } => Some(event),
            _ => None,
        })
        .collect();
    assert_eq!(terminal.len(), 1);
    assert_eq!(terminal[0].phase, DragPhase::Ended);
    assert_eq!(terminal[0].session, session);
    assert_eq!(terminal[0].position, (150, 100));
    assert!(state.owned_press.is_empty());
    assert!(state.engine.active_drag().is_none());
}

#[test]
fn drag_deadline_is_not_a_hold_and_movement_fallback_waits_until_it() {
    let (mut only_drag, _) = setup(false);
    down(&mut only_drag, 0);
    assert_eq!(only_drag.engine.next_deadline_ms(), Some(180));
    assert!(only_drag.active_hold.is_none());
    assert_eq!(only_drag.diagnostics().hold_started, 0);
    let (mut state, events) = setup(true);
    down(&mut state, 0);
    let movement = state.input_at(None, None, Some((109, 100)), 50);
    assert!(movement.replay_kinds.is_empty());
    assert_eq!(state.engine.next_hold_deadline_ms(), None);
    assert_eq!(state.engine.next_deadline_ms(), Some(180));
    assert_eq!(state.diagnostics().hold_cancelled_movement, 1);
    assert!(state.advance_time_at(179).replay_kinds.is_empty());
    assert_eq!(
        state.advance_time_at(180).replay_kinds,
        vec![InputKind::Down(RIGHT)]
    );
    assert!(state.engine.next_deadline_ms().is_none());
    assert!(!events.try_iter().any(|event| matches!(
        event,
        BackendEvent::Triggered { .. } | BackendEvent::Drag { .. }
    )));
}

#[test]
fn stationary_hold_wins_and_late_motion_cannot_start_drag() {
    let (mut state, events) = setup(true);
    down(&mut state, 0);
    assert!(state.advance_time_at(180).replay_kinds.is_empty());
    assert_eq!(state.engine.next_deadline_ms(), Some(300));
    state.advance_time_at(300);
    state.input_at(None, None, Some((150, 100)), 301);
    assert!(events
        .try_iter()
        .any(|event| matches!(event, BackendEvent::Triggered { id: 1, .. })));
    assert!(state.engine.active_drag().is_none());
    assert_eq!(state.diagnostics().hold_triggered, 1);
    assert!(
        state
            .input_at(Some(InputKind::Up(RIGHT)), None, Some((150, 100)), 302)
            .consume
    );
    let (mut late, events) = setup(true);
    down(&mut late, 0);
    let motion = late.input_at(None, None, Some((118, 100)), 180);
    assert_eq!(motion.replay_kinds, vec![InputKind::Down(RIGHT)]);
    assert!(!events
        .try_iter()
        .any(|event| matches!(event, BackendEvent::Drag { .. })));
}

#[test]
fn pause_replacement_recording_focus_and_recovery_send_cancelled_and_drain_release() {
    for mode in 0..6 {
        let (mut state, events) = setup(true);
        down(&mut state, 0);
        let session = start(&mut state, 50);
        events.try_iter().for_each(drop);
        match mode {
            0 => state.command(BackendCommand::SetEnabled(false)),
            1 => state.command(BackendCommand::SetBindings(vec![])),
            2 => state.command(BackendCommand::StartRecording),
            3 => state.update_foreground(state.foreground.wrapping_add(1)),
            4 => state.replay_failed(&[]),
            _ => state.command(BackendCommand::CancelDrag { session }),
        }
        let cancelled: Vec<_> = events
            .try_iter()
            .filter_map(|event| match event {
                BackendEvent::Drag { event, .. } => Some(event),
                _ => None,
            })
            .collect();
        assert_eq!(cancelled.len(), 1, "mode={mode}");
        assert_eq!(cancelled[0].session, session);
        assert_eq!(cancelled[0].phase, DragPhase::Cancelled);
        assert!(state.engine.active_drag().is_none());
        assert!(
            state
                .input_at(Some(InputKind::Up(RIGHT)), None, Some((118, 100)), 60)
                .consume
        );
        assert!(state.owned_press.is_empty(), "mode={mode}");
    }
}

#[test]
fn stale_ui_cancel_does_not_end_the_next_drag_session() {
    let (mut state, events) = setup(false);
    down(&mut state, 0);
    let old = start(&mut state, 50);
    state.input_at(Some(InputKind::Up(RIGHT)), None, Some((118, 100)), 60);
    down(&mut state, 100);
    let current = start(&mut state, 150);
    assert_ne!(old, current);
    events.try_iter().for_each(drop);
    state.command(BackendCommand::CancelDrag { session: old });
    assert_eq!(state.engine.active_drag().unwrap().session, current);
    assert_eq!(events.try_iter().count(), 0);
    state.command(BackendCommand::CancelDrag { session: current });
    assert!(
        matches!(events.try_recv().unwrap(), BackendEvent::Drag { event, .. } if event.session == current && event.phase == DragPhase::Cancelled)
    );
}

#[test]
fn full_lifecycle_queue_pauses_and_retries_a_terminal_cancel() {
    let (sender, events) = mpsc::sync_channel(1);
    let mut state = ThreadState::new(sender);
    state.command(BackendCommand::SetBindings(bindings(false)));
    down(&mut state, 0); // PointerPressed 填满队列。
    state.input_at(None, None, Some((118, 100)), 50);
    assert!(!state.enabled);
    assert!(state.engine.active_drag().is_none());
    assert_eq!(state.pending_drag_cancels.len(), 1);
    assert!(matches!(
        events.try_recv().unwrap(),
        BackendEvent::PointerPressed { .. }
    ));
    state.report_overflow();
    assert!(
        matches!(events.try_recv().unwrap(), BackendEvent::Drag { event, .. } if event.phase == DragPhase::Cancelled)
    );
    assert!(state.pending_drag_cancels.is_empty());
    state.report_overflow();
    assert!(
        matches!(events.try_recv().unwrap(), BackendEvent::Error(message) if message.contains("暂停"))
    );
    assert!(
        state
            .input_at(Some(InputKind::Up(RIGHT)), None, Some((118, 100)), 60)
            .consume
    );
}

#[test]
fn failed_end_delivery_retains_cancel_after_core_has_finished_the_session() {
    let (sender, events) = mpsc::sync_channel(1);
    let mut state = ThreadState::new(sender);
    state.command(BackendCommand::SetBindings(bindings(false)));
    down(&mut state, 0);
    events.try_iter().for_each(drop);
    let session = start(&mut state, 50); // Started 填满队列，尚未读走。
    assert!(
        state
            .input_at(Some(InputKind::Up(RIGHT)), None, Some((118, 100)), 60)
            .consume
    );
    assert!(state.engine.active_drag().is_none());
    assert!(!state.enabled);
    assert!(
        matches!(events.try_recv().unwrap(), BackendEvent::Drag { event, .. } if event.phase == DragPhase::Started)
    );
    state.report_overflow();
    assert!(
        matches!(events.try_recv().unwrap(), BackendEvent::Drag { event, .. } if event.session == session && event.phase == DragPhase::Cancelled)
    );
}
