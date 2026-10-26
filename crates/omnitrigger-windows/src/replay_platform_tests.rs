//! 手工回执的假 worker，不运行包括退出收尾在内的任何 SendInput。
use super::super::{BackendCommand, ThreadState};
use super::*;
use omnitrigger_core::{Activation, MouseButton, TriggerSpec};

fn setup() -> (
    ThreadState,
    Receiver<Work>,
    SyncSender<Ack>,
    Receiver<BackendEvent>,
) {
    let (events, receiver) = mpsc::sync_channel(256);
    let mut state = ThreadState::new(events);
    let (jobs, jobs_rx) = mpsc::sync_channel(1);
    let (acks, acks_rx) = mpsc::sync_channel(1);
    state.replay.worker = Some(Worker {
        jobs,
        acknowledgements: acks_rx,
        stop: Arc::new(Mutex::new(None)),
        handle: None,
    });
    (state, jobs_rx, acks, receiver)
}

fn input(state: &mut ThreadState, kind: InputKind, now: u64) -> bool {
    let outcome = state.input_at(Some(kind), None, Some((100, 100)), now);
    state.queue_outcome(outcome, Some(kind))
}

fn pump(state: &mut ThreadState) {
    let effects = state.replay.pump(state.foreground);
    state.apply_replay_effects(effects);
}

fn job(receiver: &Receiver<Work>) -> Job {
    match receiver.try_recv().unwrap() {
        Work::Submit(job) => job,
        Work::Stop => panic!("提前退出"),
    }
}

fn ack(sender: &SyncSender<Ack>, job: &Job) {
    sender
        .try_send(Ack {
            sequence: job.sequence,
            sent: job.tags.len(),
            elapsed: Duration::ZERO,
            skipped: false,
        })
        .unwrap();
}

#[test]
fn pending_replay_consumes_later_discrete_edges_but_never_mouse_move() {
    let (mut state, jobs, acks, _) = setup();
    let caps = Button::Key(0x14);
    let target = Button::Key(0x57);
    let other = Button::Key(0x51);
    state.command(BackendCommand::SetBindings(vec![TriggerSpec {
        id: 1,
        held: vec![caps],
        activation: Activation::Press(target),
    }]));
    assert!(input(&mut state, InputKind::Down(caps), 0));
    assert!(input(&mut state, InputKind::Down(other), 1));
    pump(&mut state);
    let first = job(&jobs); // 假 sender 在此挂起首批，输入线程仍可处理下一组物理边沿。
    assert!(input(&mut state, InputKind::Up(caps), 2));
    assert!(input(&mut state, InputKind::Up(other), 3));
    let motion = state.input_at(None, None, Some((200, 200)), 4);
    assert!(!state.queue_outcome(motion, None));
    assert_eq!(
        first.tags.iter().map(|tag| tag.kind).collect::<Vec<_>>(),
        vec![InputKind::Down(caps), InputKind::Down(other)]
    );
    ack(&acks, &first);
    pump(&mut state);
    let second = job(&jobs);
    assert_eq!(
        second.tags.iter().map(|tag| tag.kind).collect::<Vec<_>>(),
        vec![InputKind::Up(caps), InputKind::Up(other)]
    );
    ack(&acks, &second);
    pump(&mut state);
    assert!(!state.replay.busy());
}

#[test]
fn old_ack_cannot_clear_new_press_and_hold_action_waits_for_older_replay() {
    let (mut state, jobs, acks, events) = setup();
    let right = Button::Mouse(MouseButton::Right);
    state.command(BackendCommand::SetBindings(vec![TriggerSpec {
        id: 7,
        held: vec![],
        activation: Activation::Hold {
            button: right,
            duration_ms: 300,
        },
    }]));
    assert!(input(&mut state, InputKind::Down(right), 0));
    assert!(input(&mut state, InputKind::Up(right), 100));
    pump(&mut state);
    let old = job(&jobs);
    assert!(input(&mut state, InputKind::Down(right), 200));
    assert!(state.owned_press.contains(&right));
    state.advance_time_at(500);
    assert!(!events
        .try_iter()
        .any(|event| matches!(event, BackendEvent::Triggered(_))));
    ack(&acks, &old);
    pump(&mut state);
    assert!(state.owned_press.contains(&right));
    assert!(events
        .try_iter()
        .any(|event| matches!(event, BackendEvent::Triggered(7))));
    assert!(input(&mut state, InputKind::Up(right), 501));
    assert!(!state.owned_press.contains(&right));
}

#[test]
fn stale_foreground_markers_are_cancelled_without_running_actions() {
    let (mut state, jobs, acks, events) = setup();
    let right = Button::Mouse(MouseButton::Right);
    state.replay.new_press(right);
    let kinds = vec![InputKind::Down(right), InputKind::Up(right)];
    let inputs = kinds
        .iter()
        .map(|kind| replay_input(*kind, &BTreeMap::new()))
        .collect();
    state
        .replay
        .enqueue(inputs, kinds, state.foreground, &BTreeMap::new())
        .unwrap();
    pump(&mut state);
    let first = job(&jobs);
    state.notify_action(BackendEvent::Triggered(9));
    ack(&acks, &first);
    let effects = state.replay.pump(state.foreground.wrapping_add(1));
    state.apply_replay_effects(effects);
    assert!(!events
        .try_iter()
        .any(|event| matches!(event, BackendEvent::Triggered(_))));
}

#[test]
fn paused_press_stays_owned_after_replay_and_cleanup_ack_until_physical_release() {
    let (mut state, jobs, acks, _) = setup();
    let caps = Button::Key(0x14);
    let other = Button::Key(0x51);
    state.command(BackendCommand::SetBindings(vec![TriggerSpec {
        id: 1,
        held: vec![caps],
        activation: Activation::Press(Button::Key(0x57)),
    }]));
    assert!(input(&mut state, InputKind::Down(caps), 0));
    assert!(input(&mut state, InputKind::Down(other), 1));
    pump(&mut state);
    let original = job(&jobs);
    state.command(BackendCommand::SetEnabled(false));
    assert!(state.failure_draining.contains(&caps));
    assert!(state.failure_draining.contains(&other));
    ack(&acks, &original);
    pump(&mut state);
    assert!(state.owned_press.contains(&caps));
    assert!(state.owned_press.contains(&other));
    let cleanup = job(&jobs);
    assert!(cleanup.cleanup);
    ack(&acks, &cleanup);
    pump(&mut state);
    assert!(!state.replay.busy());
    assert!(state.owned_press.contains(&caps));
    assert!(state.owned_press.contains(&other));
    assert!(input(&mut state, InputKind::Up(caps), 2));
    assert!(input(&mut state, InputKind::Up(other), 3));
    assert!(state.owned_press.is_empty());
    assert!(state.failure_draining.is_empty());
}
