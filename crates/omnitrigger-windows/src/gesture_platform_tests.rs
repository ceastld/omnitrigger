//! 手势的平台事件序列测试，不安装 Hook，也不向桌面发送输入。
use super::*;
use omnitrigger_core::{Activation, GestureDirection, GesturePath};

const RIGHT: Button = Button::Mouse(MouseButton::Right);

fn setup() -> (ThreadState, Receiver<BackendEvent>) {
    let (sender, receiver) = mpsc::sync_channel(EVENT_CAPACITY);
    let mut state = ThreadState::new(sender);
    state.command(BackendCommand::SetBindings(vec![TriggerSpec {
        pointer_scope: Default::default(),
        wheel: Default::default(),
        keyboard: Default::default(),
        process_scope: Default::default(),
        conditions: Vec::new(),
        id: 7,
        held: vec![],
        activation: Activation::Gesture {
            button: RIGHT,
            path: GesturePath::new(&[GestureDirection::Down, GestureDirection::Right]).unwrap(),
        },
    }]));
    (state, receiver)
}

fn triggered(events: &Receiver<BackendEvent>) -> Vec<u64> {
    events
        .try_iter()
        .filter_map(|event| match event {
            BackendEvent::Triggered { id, .. } => Some(id),
            _ => None,
        })
        .collect()
}

#[test]
fn drawn_gesture_triggers_on_release_with_both_edges_consumed() {
    let (mut state, events) = setup();
    assert!(
        state
            .input_at(Some(InputKind::Down(RIGHT)), None, Some((100, 100)), 0)
            .consume
    );
    let mut now = 0;
    for y in (104..=220).step_by(4) {
        now += 2;
        let moved = state.input_at(None, None, Some((100, y)), now);
        assert!(!moved.consume && moved.replay_kinds.is_empty());
    }
    for x in (104..=220).step_by(4) {
        now += 2;
        state.input_at(None, None, Some((x, 220)), now);
    }
    assert!(triggered(&events).is_empty());
    let release = state.input_at(Some(InputKind::Up(RIGHT)), None, Some((220, 220)), now + 5);
    assert!(release.consume && release.replay_kinds.is_empty());
    assert_eq!(triggered(&events), [7]);
    assert!(state.owned_press.is_empty());
    assert_eq!(state.engine.gesture_session(), None);
}

#[test]
fn click_and_still_hold_return_the_original_press() {
    let (mut state, events) = setup();
    state.input_at(Some(InputKind::Down(RIGHT)), None, Some((100, 100)), 0);
    let click = state.input_at(Some(InputKind::Up(RIGHT)), None, Some((102, 100)), 30);
    assert!(click.consume);
    assert_eq!(
        click.replay_kinds,
        vec![InputKind::Down(RIGHT), InputKind::Up(RIGHT)]
    );

    state.input_at(Some(InputKind::Down(RIGHT)), None, Some((100, 100)), 100);
    assert_eq!(
        state.engine.next_deadline_ms(),
        Some(100 + u64::from(omnitrigger_core::GESTURE_START_MS))
    );
    assert!(state.advance_time_at(599).replay_kinds.is_empty());
    assert_eq!(
        state.advance_time_at(600).replay_kinds,
        vec![InputKind::Down(RIGHT)]
    );
    let drag = state.input_at(None, None, Some((300, 100)), 610);
    assert!(!drag.consume && drag.replay_kinds.is_empty());
    let release = state.input_at(Some(InputKind::Up(RIGHT)), None, Some((300, 100)), 620);
    assert!(!release.consume && release.replay_kinds.is_empty());
    assert!(triggered(&events).is_empty());
}
