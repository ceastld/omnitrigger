use crate::*;

use GestureDirection::{Down, DownRight, Left, Right, Up};

const RIGHT: Button = Button::Mouse(MouseButton::Right);
const MIDDLE: Button = Button::Mouse(MouseButton::Middle);
const CAPS: Button = Button::Key(0x14);

fn shape(steps: &[GestureDirection]) -> GestureShape {
    GestureShape::from_directions(steps).unwrap()
}

fn spec(id: u64, held: &[Button], activation: Activation) -> TriggerSpec {
    TriggerSpec {
        pointer_scope: Default::default(),
        wheel: Default::default(),
        keyboard: Default::default(),
        process_scope: Default::default(),
        conditions: vec![],
        id,
        held: held.to_vec(),
        activation,
    }
}

fn gesture(id: u64, steps: &[GestureDirection]) -> TriggerSpec {
    spec(
        id,
        &[],
        Activation::Gesture {
            button: RIGHT,
            shape: shape(steps),
        },
    )
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

/// 从 from 到 to 以 4px 步长移动，模拟连续的物理鼠标事件。
fn stroke(engine: &mut Engine, from: (i32, i32), to: (i32, i32), now: u64) -> Decision {
    let mut decision = Decision::default();
    let steps = ((to.0 - from.0).abs().max((to.1 - from.1).abs()) / 4).max(1);
    for step in 1..=steps {
        let point = (
            from.0 + (to.0 - from.0) * step / steps,
            from.1 + (to.1 - from.1) * step / steps,
        );
        let moved = engine.pointer_moved(point, now);
        decision.consume |= moved.consume;
        decision.replay.extend(moved.replay);
        decision.triggered.extend(moved.triggered);
    }
    decision
}

/// 沿折线以 3px 步长记录轨迹。
fn trace(points: &[(i32, i32)]) -> GestureTrace {
    let mut trace = GestureTrace::new(points[0]);
    for pair in points.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        let steps = ((to.0 - from.0).abs().max((to.1 - from.1).abs()) / 3).max(1);
        for step in 1..=steps {
            trace.push((
                from.0 + (to.0 - from.0) * step / steps,
                from.1 + (to.1 - from.1) * step / steps,
            ));
        }
    }
    trace
}

fn drawn(points: &[(i32, i32)]) -> GestureShape {
    trace(points).shape().unwrap()
}

#[test]
fn shapes_are_normalized_and_survive_a_save_and_load_round_trip() {
    let corner = GestureShape::from_polyline(&[(40, 40), (240, 40), (240, 140)]).unwrap();
    let points = corner.points();
    assert_eq!(points.len(), GESTURE_SHAPE_POINTS);
    // 较长一边占满 ±1000，较短一边按比例居中；起点与终点保留。
    assert_eq!(points[0], (-1000, -500));
    assert_eq!(points[GESTURE_SHAPE_POINTS - 1], (1000, 500));
    // 位置与大小不影响形状。
    let moved = GestureShape::from_polyline(&[(-900, 7), (-500, 7), (-500, 207)]).unwrap();
    assert!(corner.similarity(&moved) >= 995);
    // 已归一化的采样点原样载入，反复保存不漂移。
    let saved: Vec<(i32, i32)> = points
        .iter()
        .map(|&(x, y)| (i32::from(x), i32::from(y)))
        .collect();
    assert_eq!(GestureShape::from_points(&saved), Ok(corner));
    // 点数不同的折线会重采样。
    assert!(
        GestureShape::from_points(&[(0, 0), (2, 0), (2, 1)])
            .unwrap()
            .similarity(&corner)
            >= 995
    );

    assert_eq!(
        GestureShape::from_polyline(&[]),
        Err(GestureShapeError::Empty)
    );
    assert_eq!(
        GestureShape::from_polyline(&[(5, 5), (5, 5)]),
        Err(GestureShapeError::Empty)
    );
    assert_eq!(
        GestureShape::from_directions(&[Right, Right]),
        Err(GestureShapeError::RepeatedStep { index: 1 })
    );
    assert_eq!(
        GestureShape::from_directions(&[]),
        Err(GestureShapeError::Empty)
    );
    for direction in GestureDirection::ALL {
        assert_eq!(
            GestureDirection::from_token(direction.token()),
            Some(direction)
        );
    }
}

#[test]
fn similarity_falls_with_tilt_and_separates_different_shapes() {
    let right = shape(&[Right]);
    assert_eq!(right.similarity(&right), 1000);
    // 倾斜越大越不像：10° 仍很接近，45° 已不匹配，反向几乎为零。
    let tilted = |degrees: f64| {
        let angle = degrees.to_radians();
        GestureShape::from_polyline(&[
            (0, 0),
            ((1000. * angle.cos()) as i32, (1000. * angle.sin()) as i32),
        ])
        .unwrap()
    };
    let scores: Vec<u16> = [10., 20., 30., 45.]
        .iter()
        .map(|&degrees| right.similarity(&tilted(degrees)))
        .collect();
    assert!(
        scores.windows(2).all(|pair| pair[0] > pair[1]),
        "{scores:?}"
    );
    assert!(
        scores[0] >= 900 && scores[1] >= GESTURE_ACCEPT,
        "{scores:?}"
    );
    assert!(scores[3] < GESTURE_ACCEPT, "{scores:?}");
    assert!(right.similarity(&shape(&[Left])) < 400);
    assert!(right.similarity(&shape(&[Down])) < GESTURE_ACCEPT);

    // 折线、斜线与直线互不相似；同一形状画圆角或带抖动仍然匹配。
    let corner = shape(&[Right, Down]);
    assert!(corner.similarity(&right) < 400);
    assert!(corner.similarity(&shape(&[DownRight])) < GESTURE_ACCEPT);
    assert!(corner.similarity(&shape(&[Down, Right])) < 400);
    let rounded = drawn(&[
        (0, 0),
        (150, 4),
        (185, 14),
        (204, 40),
        (212, 80),
        (208, 220),
    ]);
    assert!(rounded.similarity(&corner) >= 850);
    assert!(rounded.similarity(&right) < GESTURE_ACCEPT);
    // 任意形状同样可比：圆弧不是折线，也不是直线。
    let arc: Vec<(i32, i32)> = (0..=18)
        .map(|step| {
            let angle = f64::from(step) * 10f64.to_radians();
            (
                (200. - 200. * angle.cos()) as i32,
                (-200. * angle.sin()) as i32,
            )
        })
        .collect();
    let arc = GestureShape::from_polyline(&arc).unwrap();
    assert!(arc.similarity(&arc) == 1000 && arc.similarity(&right) < GESTURE_ACCEPT);
    assert!(arc.similarity(&shape(&[Up, Right, Down])) < GESTURE_DUPLICATE);
}

#[test]
fn trace_is_bounded_keeps_the_shape_and_needs_a_minimum_length() {
    let mut short = GestureTrace::new((0, 0));
    short.push((11, 0));
    assert!(!short.started());
    assert_eq!(short.shape(), Err(GestureShapeError::TooShort));
    short.push((12, 0));
    assert!(short.started());
    assert_eq!(short.shape(), Err(GestureShapeError::TooShort));
    short.push((40, 0));
    assert!(short.shape().unwrap().similarity(&shape(&[Right])) >= 990);

    // 远超缓冲容量的长轨迹被抽稀后形状不变。
    let long = drawn(&[(0, 0), (6000, 0), (6000, 6000), (0, 6000)]);
    assert!(long.similarity(&shape(&[Right, Down, Left])) >= 950);
    // 垂直抖动与松开前的小幅回拉不改变匹配。
    let mut points = vec![(0, 0)];
    for x in (10..=300).step_by(10) {
        points.push((x, if x % 20 == 0 { 4 } else { -4 }));
    }
    points.push((285, -10));
    assert!(drawn(&points).similarity(&shape(&[Right])) >= 900);

    // 极端坐标不溢出。
    let mut extreme = GestureTrace::new((i32::MIN, i32::MIN));
    extreme.push((i32::MAX, i32::MAX));
    extreme.push((i32::MIN, i32::MAX));
    assert!(extreme.started() && extreme.shape().is_ok());
}

#[test]
fn close_scores_ask_the_user_instead_of_guessing() {
    let mut engine = Engine::new(vec![gesture(1, &[Right]), gesture(2, &[DownRight])]).unwrap();
    engine.pointer_moved((0, 0), 0);
    // 约 26° 的斜线与“右”“右下”几乎同样相似：不触发，留下按分数排序的候选。
    input(&mut engine, InputKind::Down(RIGHT), 0);
    stroke(&mut engine, (0, 0), (200, 96), 20);
    let up = input(&mut engine, InputKind::Up(RIGHT), 30);
    assert!(up.consume && up.triggered.is_empty() && up.replay.is_empty());
    let choice = engine.take_gesture_choice().expect("应留下待选候选");
    let candidates: Vec<(u64, u16)> = choice.candidates().collect();
    assert_eq!(candidates.len(), 2);
    assert!(candidates[0].1 >= candidates[1].1 && candidates[1].1 >= GESTURE_ACCEPT);
    assert!(candidates[0].1 - candidates[1].1 <= GESTURE_CLOSE_MARGIN);
    let mut ids: Vec<u64> = candidates.iter().map(|&(id, _)| id).collect();
    ids.sort_unstable();
    assert_eq!(ids, [1, 2]);
    assert_eq!(engine.take_gesture_choice(), None);

    // 明显更像其中一个时直接触发，不留候选。
    input(&mut engine, InputKind::Down(RIGHT), 40);
    stroke(&mut engine, (200, 96), (400, 110), 60);
    assert_eq!(input(&mut engine, InputKind::Up(RIGHT), 70).triggered, [1]);
    assert_eq!(engine.take_gesture_choice(), None);
}

#[test]
fn click_without_movement_replays_original_press_and_release() {
    let mut engine = Engine::new(vec![gesture(1, &[Right])]).unwrap();
    engine.pointer_moved((0, 0), 0);
    assert!(input(&mut engine, InputKind::Down(RIGHT), 0).consume);
    assert_eq!(engine.gesture_session(), Some((RIGHT, false)));
    assert_eq!(engine.next_deadline_ms(), Some(u64::from(GESTURE_START_MS)));
    assert_eq!(engine.pointer_moved((5, 5), 10), Decision::default());
    let up = input(&mut engine, InputKind::Up(RIGHT), 20);
    assert!(up.consume);
    assert_eq!(up.replay, [InputKind::Down(RIGHT), InputKind::Up(RIGHT)]);
    assert!(up.triggered.is_empty());
    assert_eq!(engine.gesture_session(), None);
}

#[test]
fn matching_trace_triggers_once_on_release_and_consumes_both_edges() {
    let mut engine = Engine::new(vec![
        gesture(1, &[Right]),
        gesture(2, &[Left]),
        gesture(3, &[Down, Right]),
    ])
    .unwrap();
    engine.pointer_moved((500, 500), 0);
    assert!(input(&mut engine, InputKind::Down(RIGHT), 0).consume);
    let moved = stroke(&mut engine, (500, 500), (500, 600), 50);
    assert_eq!(moved, Decision::default());
    assert_eq!(engine.gesture_session(), Some((RIGHT, true)));
    assert_eq!(engine.next_deadline_ms(), None);
    assert_eq!(
        stroke(&mut engine, (500, 600), (620, 600), 900),
        Decision::default()
    );
    let up = input(&mut engine, InputKind::Up(RIGHT), 950);
    assert!(up.consume);
    assert!(up.replay.is_empty());
    assert_eq!(up.triggered, [3]);
    assert_eq!(engine.gesture_session(), None);
}

#[test]
fn unmatched_trace_is_consumed_without_trigger_or_replay() {
    let mut engine = Engine::new(vec![gesture(1, &[Right])]).unwrap();
    engine.pointer_moved((0, 0), 0);
    input(&mut engine, InputKind::Down(RIGHT), 0);
    stroke(&mut engine, (0, 0), (0, 150), 30);
    let up = input(&mut engine, InputKind::Up(RIGHT), 60);
    assert!(up.consume);
    assert!(up.replay.is_empty() && up.triggered.is_empty());
}

#[test]
fn staying_still_until_the_start_window_returns_the_press_to_the_application() {
    let mut engine = Engine::new(vec![gesture(1, &[Right])]).unwrap();
    engine.pointer_moved((0, 0), 0);
    input(&mut engine, InputKind::Down(RIGHT), 0);
    // 严格早于时窗的位移仍可开始手势。
    let mut early = Engine::new(vec![gesture(1, &[Right])]).unwrap();
    early.pointer_moved((0, 0), 0);
    input(&mut early, InputKind::Down(RIGHT), 0);
    early.pointer_moved((12, 0), u64::from(GESTURE_START_MS) - 1);
    assert_eq!(early.gesture_session(), Some((RIGHT, true)));

    let expired = engine.advance_time(u64::from(GESTURE_START_MS));
    assert_eq!(expired.replay, [InputKind::Down(RIGHT)]);
    assert_eq!(engine.gesture_session(), None);
    assert_eq!(
        stroke(&mut engine, (0, 0), (200, 0), 600),
        Decision::default()
    );
    let up = input(&mut engine, InputKind::Up(RIGHT), 700);
    assert!(!up.consume && up.replay.is_empty() && up.triggered.is_empty());
}

#[test]
fn other_input_before_start_replays_press_first_and_after_start_drains_release() {
    let mut engine = Engine::new(vec![gesture(1, &[Right])]).unwrap();
    engine.pointer_moved((0, 0), 0);
    input(&mut engine, InputKind::Down(RIGHT), 0);
    let middle = input(&mut engine, InputKind::Down(MIDDLE), 10);
    assert_eq!(middle.replay, [InputKind::Down(RIGHT)]);
    assert!(!middle.consume);
    assert!(!input(&mut engine, InputKind::Up(MIDDLE), 20).consume);
    assert!(!input(&mut engine, InputKind::Up(RIGHT), 30).consume);

    input(&mut engine, InputKind::Down(RIGHT), 100);
    stroke(&mut engine, (0, 0), (100, 0), 120);
    assert_eq!(engine.gesture_session(), Some((RIGHT, true)));
    let wheel = input(
        &mut engine,
        InputKind::Wheel {
            axis: Axis::Vertical,
            delta: 120,
        },
        130,
    );
    assert!(wheel.replay.is_empty() && wheel.triggered.is_empty());
    assert_eq!(engine.gesture_session(), None);
    let up = input(&mut engine, InputKind::Up(RIGHT), 140);
    assert!(up.consume);
    assert!(up.replay.is_empty() && up.triggered.is_empty());
}

#[test]
fn cancel_and_injected_events_never_finish_a_gesture() {
    let mut engine = Engine::new(vec![gesture(1, &[Right])]).unwrap();
    engine.pointer_moved((0, 0), 0);
    input(&mut engine, InputKind::Down(RIGHT), 0);
    let injected = engine.process_at(
        InputEvent {
            kind: InputKind::Up(RIGHT),
            injected: true,
        },
        5,
    );
    assert_eq!(injected, Decision::default());
    stroke(&mut engine, (0, 0), (100, 0), 10);
    engine.cancel();
    assert_eq!(engine.gesture_session(), None);
    let up = input(&mut engine, InputKind::Up(RIGHT), 20);
    assert!(up.consume && up.triggered.is_empty() && up.replay.is_empty());
}

#[test]
fn hold_fires_when_still_and_gesture_wins_after_movement() {
    let hold = spec(
        9,
        &[],
        Activation::Hold {
            button: RIGHT,
            duration_ms: 300,
        },
    );
    let mut engine = Engine::new(vec![hold, gesture(1, &[Right])]).unwrap();
    engine.pointer_moved((0, 0), 0);
    input(&mut engine, InputKind::Down(RIGHT), 0);
    let fired = engine.advance_time(300);
    assert_eq!(fired.triggered, [9]);
    assert_eq!(engine.gesture_session(), None);
    stroke(&mut engine, (0, 0), (100, 0), 310);
    let up = input(&mut engine, InputKind::Up(RIGHT), 320);
    assert!(up.consume && up.triggered.is_empty());

    engine.pointer_moved((0, 0), 400);
    input(&mut engine, InputKind::Down(RIGHT), 400);
    stroke(&mut engine, (0, 0), (100, 0), 420);
    assert_eq!(engine.next_hold_deadline_ms(), None);
    assert_eq!(engine.advance_time(800), Decision::default());
    assert_eq!(input(&mut engine, InputKind::Up(RIGHT), 810).triggered, [1]);
}

#[test]
fn the_most_similar_gesture_wins() {
    let mut engine = Engine::new(vec![gesture(1, &[Down]), gesture(2, &[DownRight])]).unwrap();
    engine.pointer_moved((0, 0), 0);
    input(&mut engine, InputKind::Down(RIGHT), 0);
    stroke(&mut engine, (0, 0), (100, 100), 20);
    assert_eq!(input(&mut engine, InputKind::Up(RIGHT), 30).triggered, [2]);

    input(&mut engine, InputKind::Down(RIGHT), 40);
    stroke(&mut engine, (100, 100), (110, 220), 60);
    assert_eq!(input(&mut engine, InputKind::Up(RIGHT), 70).triggered, [1]);
}

#[test]
fn same_shape_prefers_the_most_specific_scope_while_other_scopes_still_join() {
    let mut scoped = gesture(1, &[Right]);
    scoped.process_scope.allow = vec!["chrome.exe".into()];
    let mut engine = Engine::new(vec![gesture(2, &[Right]), scoped, gesture(3, &[Left])]).unwrap();
    engine.set_foreground_process(Some("chrome.exe")).unwrap();
    engine.pointer_moved((0, 0), 0);
    input(&mut engine, InputKind::Down(RIGHT), 0);
    stroke(&mut engine, (0, 0), (100, 0), 20);
    assert_eq!(input(&mut engine, InputKind::Up(RIGHT), 30).triggered, [1]);
    input(&mut engine, InputKind::Down(RIGHT), 40);
    stroke(&mut engine, (100, 0), (0, 0), 60);
    assert_eq!(input(&mut engine, InputKind::Up(RIGHT), 70).triggered, [3]);

    engine.set_foreground_process(Some("notepad.exe")).unwrap();
    input(&mut engine, InputKind::Down(RIGHT), 80);
    stroke(&mut engine, (0, 0), (100, 0), 100);
    assert_eq!(input(&mut engine, InputKind::Up(RIGHT), 110).triggered, [2]);
}

#[test]
fn held_prefix_gesture_requires_the_prefix_and_keeps_it_consumed() {
    let mut caps_gesture = gesture(1, &[Up]);
    caps_gesture.held = vec![CAPS];
    let mut engine = Engine::new(vec![caps_gesture]).unwrap();
    engine.pointer_moved((0, 0), 0);
    // 没有前缀时右键直接交给应用。
    assert!(!input(&mut engine, InputKind::Down(RIGHT), 0).consume);
    assert!(!input(&mut engine, InputKind::Up(RIGHT), 5).consume);

    assert!(input(&mut engine, InputKind::Down(CAPS), 10).consume);
    assert!(input(&mut engine, InputKind::Down(RIGHT), 20).consume);
    stroke(&mut engine, (0, 0), (0, -100), 40);
    assert_eq!(input(&mut engine, InputKind::Up(RIGHT), 50).triggered, [1]);
    let caps_up = input(&mut engine, InputKind::Up(CAPS), 60);
    assert!(caps_up.consume && caps_up.replay.is_empty());
}

#[test]
fn registration_allows_distinct_shapes_and_reports_structured_conflicts() {
    assert!(Engine::new(vec![gesture(1, &[Right]), gesture(2, &[Right, Down])]).is_ok());
    assert!(matches!(
        Engine::new(vec![gesture(1, &[Right]), gesture(2, &[Right])]),
        Err(RegistrationError::DuplicateTrigger {
            first: 1,
            second: 2
        })
    ));
    let drag = spec(
        2,
        &[],
        Activation::Drag {
            button: RIGHT,
            min_distance_px: 18,
            max_start_ms: 180,
        },
    );
    assert!(matches!(
        Engine::new(vec![gesture(1, &[Right]), drag]),
        Err(RegistrationError::GestureDragConflict {
            gesture_binding: 1,
            drag_binding: 2,
        })
    ));
    assert!(matches!(
        Engine::new(vec![spec(
            1,
            &[],
            Activation::Gesture {
                button: CAPS,
                shape: shape(&[Right]),
            },
        )]),
        Err(RegistrationError::InvalidGestureButton { id: 1, .. })
    ));
    assert!(matches!(
        Engine::new(vec![
            gesture(1, &[Right]),
            spec(2, &[], Activation::Press(RIGHT)),
        ]),
        Err(RegistrationError::DuplicateTrigger { .. })
    ));
    assert!(matches!(
        Engine::new(vec![
            gesture(1, &[Right]),
            spec(2, &[RIGHT], Activation::Wheel(WheelDirection::Up)),
        ]),
        Err(RegistrationError::TerminalIsPrefix { .. })
    ));
}
