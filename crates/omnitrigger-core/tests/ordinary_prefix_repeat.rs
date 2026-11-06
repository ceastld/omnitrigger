use omnitrigger_core::{Activation, Button, Decision, Engine, InputEvent, InputKind, TriggerSpec};

fn key(value: u16) -> Button {
    Button::Key(value)
}

fn spec(id: u64, held: &[u16], terminal: u16) -> TriggerSpec {
    TriggerSpec {
        id,
        held: held.iter().copied().map(key).collect(),
        activation: Activation::Press(key(terminal)),
        conditions: Vec::new(),
        process_scope: Default::default(),
        pointer_scope: Default::default(),
        wheel: Default::default(),
        keyboard: Default::default(),
    }
}

fn down(value: u16) -> InputKind {
    InputKind::Down(key(value))
}

fn up(value: u16) -> InputKind {
    InputKind::Up(key(value))
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

#[test]
fn repeat_of_either_unmatched_prefix_key_delivers_pending_input_once() {
    for repeated in [20, 69] {
        let mut engine = Engine::new(vec![spec(1, &[20, 69], 191)]).unwrap();
        assert!(input(&mut engine, down(20), 1).consume);
        assert!(input(&mut engine, down(69), 2).consume);
        let repeat = input(&mut engine, down(repeated), 3);
        assert_eq!(repeat.replay, [down(20), down(69)]);
        assert!(!repeat.consume);
        assert!(repeat.triggered.is_empty());
        assert_eq!(input(&mut engine, down(repeated), 4), Decision::default());
        assert_eq!(input(&mut engine, down(191), 5), Decision::default());
        assert_eq!(input(&mut engine, up(191), 6), Decision::default());
        assert_eq!(input(&mut engine, up(69), 7), Decision::default());
        assert_eq!(input(&mut engine, up(20), 8), Decision::default());
        assert!(input(&mut engine, down(20), 9).consume);
        assert!(input(&mut engine, down(69), 10).consume);
        assert_eq!(input(&mut engine, down(191), 11).triggered, [1]);
    }
}

#[test]
fn secondary_prefix_repeat_preserves_an_already_consumed_leader() {
    let mut engine = Engine::new(vec![spec(1, &[20], 87), spec(2, &[20, 69], 191)]).unwrap();
    input(&mut engine, down(20), 1);
    assert_eq!(input(&mut engine, down(87), 2).triggered, [1]);
    assert!(input(&mut engine, up(87), 3).consume);
    assert!(input(&mut engine, down(69), 4).consume);
    let repeat = input(&mut engine, down(69), 5);
    assert_eq!(repeat.replay, [down(69)]);
    assert!(!repeat.consume);
    assert_eq!(input(&mut engine, up(69), 6), Decision::default());
    assert_eq!(input(&mut engine, down(87), 7).triggered, [1]);
    assert!(input(&mut engine, up(87), 8).consume);
    assert!(input(&mut engine, up(20), 9).consume);
}

#[test]
fn single_pending_prefix_keeps_its_existing_repeat_and_tap_policy() {
    let mut engine = Engine::new(vec![spec(1, &[20, 69], 191)]).unwrap();
    assert!(input(&mut engine, down(20), 1).consume);
    let repeat = input(&mut engine, down(20), 2);
    assert!(repeat.consume);
    assert!(repeat.replay.is_empty());
    assert!(repeat.triggered.is_empty());
    let release = input(&mut engine, up(20), 3);
    assert!(release.consume);
    assert_eq!(release.replay, [down(20), up(20)]);
}

#[test]
fn matched_terminal_without_repeat_remains_consumed_until_its_release() {
    let mut engine = Engine::new(vec![spec(1, &[20, 69], 191)]).unwrap();
    input(&mut engine, down(20), 1);
    input(&mut engine, down(69), 2);
    assert_eq!(input(&mut engine, down(191), 3).triggered, [1]);
    let repeat = input(&mut engine, down(191), 4);
    assert!(repeat.consume);
    assert!(repeat.replay.is_empty());
    assert!(repeat.triggered.is_empty());
    assert!(input(&mut engine, up(20), 5).consume);
    assert!(input(&mut engine, up(69), 6).consume);
    assert!(input(&mut engine, up(191), 7).consume);
}
