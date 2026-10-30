//! 鼠标键作为组合前缀时的扣留与交还：左键不扣留，其他鼠标前缀移动后交还。
use crate::*;

const LEFT: Button = Button::Mouse(MouseButton::Left);
const RIGHT: Button = Button::Mouse(MouseButton::Right);
const MIDDLE: Button = Button::Mouse(MouseButton::Middle);
const SIDE: Button = Button::Mouse(MouseButton::X1);
const CAPS: Button = Button::Key(0x14);
const W: Button = Button::Key(0x57);

fn press(id: u64, held: &[Button], terminal: Button) -> TriggerSpec {
    TriggerSpec {
        conditions: vec![],
        id,
        held: held.to_vec(),
        activation: Activation::Press(terminal),
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

fn wheel_up() -> InputKind {
    InputKind::Wheel {
        axis: Axis::Vertical,
        delta: 120,
    }
}

#[test]
fn left_prefix_passes_through_and_right_click_still_triggers() {
    let mut engine = Engine::new(vec![press(1, &[LEFT], RIGHT)]).unwrap();
    engine.pointer_moved((100, 100), 0);
    assert_eq!(
        input(&mut engine, InputKind::Down(LEFT), 0),
        Decision::default()
    );
    let combo = input(&mut engine, InputKind::Down(RIGHT), 10);
    assert!(combo.consume);
    assert_eq!(combo.triggered, vec![1]);
    assert!(combo.replay.is_empty());
    let release = input(&mut engine, InputKind::Up(RIGHT), 20);
    assert!(release.consume && release.replay.is_empty());
    // 左键的按下已交给应用，释放也必须交给应用，保持按下与释放成对。
    assert_eq!(
        input(&mut engine, InputKind::Up(LEFT), 30),
        Decision::default()
    );
}

#[test]
fn dragging_with_a_left_prefix_binding_never_holds_input() {
    let mut engine = Engine::new(vec![press(1, &[LEFT], RIGHT)]).unwrap();
    engine.pointer_moved((100, 100), 0);
    assert_eq!(
        input(&mut engine, InputKind::Down(LEFT), 0),
        Decision::default()
    );
    for step in 1..=20 {
        assert_eq!(
            engine.pointer_moved((100 + step * 10, 100), step as u64 * 10),
            Decision::default()
        );
    }
    assert_eq!(
        input(&mut engine, InputKind::Down(LEFT), 220),
        Decision::default(),
        "自动重复的左键按下同样交给应用"
    );
    assert_eq!(
        input(&mut engine, InputKind::Up(LEFT), 230),
        Decision::default()
    );
}

#[test]
fn left_prefix_with_wheel_consumes_only_the_wheel() {
    let mut engine = Engine::new(vec![TriggerSpec {
        conditions: vec![],
        id: 1,
        held: vec![LEFT],
        activation: Activation::Wheel(WheelDirection::Up),
    }])
    .unwrap();
    assert_eq!(
        input(&mut engine, InputKind::Down(LEFT), 0),
        Decision::default()
    );
    let wheel = input(&mut engine, wheel_up(), 10);
    assert!(wheel.consume);
    assert_eq!(wheel.triggered, vec![1]);
    assert_eq!(
        input(&mut engine, InputKind::Up(LEFT), 20),
        Decision::default()
    );
}

#[test]
fn cancelling_does_not_swallow_a_passed_left_release() {
    let mut engine = Engine::new(vec![press(1, &[LEFT], RIGHT)]).unwrap();
    assert_eq!(
        input(&mut engine, InputKind::Down(LEFT), 0),
        Decision::default()
    );
    engine.cancel();
    assert_eq!(
        input(&mut engine, InputKind::Up(LEFT), 10),
        Decision::default()
    );
}

#[test]
fn moving_a_held_mouse_prefix_hands_the_press_back() {
    for prefix in [RIGHT, MIDDLE, SIDE] {
        let mut engine = Engine::new(vec![TriggerSpec {
            conditions: vec![],
            id: 1,
            held: vec![prefix],
            activation: Activation::Wheel(WheelDirection::Up),
        }])
        .unwrap();
        engine.pointer_moved((50, 50), 0);
        assert!(input(&mut engine, InputKind::Down(prefix), 0).consume);
        // 8 像素以内视为静止，继续等待组合。
        assert_eq!(engine.pointer_moved((58, 50), 5), Decision::default());
        let moved = engine.pointer_moved((59, 50), 10);
        assert_eq!(moved.replay, vec![InputKind::Down(prefix)]);
        assert!(!moved.consume);
        // 交还后不再匹配组合，滚轮与释放都原样交给应用。
        assert_eq!(input(&mut engine, wheel_up(), 20), Decision::default());
        assert_eq!(
            input(&mut engine, InputKind::Up(prefix), 30),
            Decision::default()
        );
    }
}

#[test]
fn small_jitter_keeps_a_mouse_prefix_waiting_for_its_combo() {
    let mut engine = Engine::new(vec![TriggerSpec {
        conditions: vec![],
        id: 1,
        held: vec![SIDE],
        activation: Activation::Wheel(WheelDirection::Up),
    }])
    .unwrap();
    engine.pointer_moved((0, 0), 0);
    assert!(input(&mut engine, InputKind::Down(SIDE), 0).consume);
    assert_eq!(engine.pointer_moved((5, -5), 5), Decision::default());
    let wheel = input(&mut engine, wheel_up(), 10);
    assert_eq!(wheel.triggered, vec![1]);
    assert!(input(&mut engine, InputKind::Up(SIDE), 20).consume);
}

#[test]
fn prefix_origin_resets_between_presses() {
    let mut engine = Engine::new(vec![TriggerSpec {
        conditions: vec![],
        id: 1,
        held: vec![SIDE],
        activation: Activation::Wheel(WheelDirection::Up),
    }])
    .unwrap();
    engine.pointer_moved((0, 0), 0);
    assert!(input(&mut engine, InputKind::Down(SIDE), 0).consume);
    assert!(!engine.pointer_moved((40, 0), 5).replay.is_empty());
    assert_eq!(
        input(&mut engine, InputKind::Up(SIDE), 10),
        Decision::default()
    );
    // 新一次按下以新的位置为起点，不沿用上次的偏移。
    assert!(input(&mut engine, InputKind::Down(SIDE), 20).consume);
    assert_eq!(engine.pointer_moved((45, 0), 25), Decision::default());
    assert_eq!(input(&mut engine, wheel_up(), 30).triggered, vec![1]);
}

#[test]
fn keyboard_prefix_ignores_pointer_movement() {
    let mut engine = Engine::new(vec![press(1, &[CAPS], W)]).unwrap();
    engine.pointer_moved((0, 0), 0);
    assert!(input(&mut engine, InputKind::Down(CAPS), 0).consume);
    assert_eq!(engine.pointer_moved((300, 300), 5), Decision::default());
    assert_eq!(
        input(&mut engine, InputKind::Down(W), 10).triggered,
        vec![1]
    );
}
