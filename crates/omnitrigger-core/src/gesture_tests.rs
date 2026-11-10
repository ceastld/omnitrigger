use crate::*;

use GestureDirection::{Down, DownLeft, DownRight, Left, Right, Up};

const RIGHT: Button = Button::Mouse(MouseButton::Right);
const MIDDLE: Button = Button::Mouse(MouseButton::Middle);
const CAPS: Button = Button::Key(0x14);

fn path(steps: &[GestureDirection]) -> GesturePath {
    GesturePath::new(steps).unwrap()
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
            path: path(steps),
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

fn track(points: &[(i32, i32)]) -> GestureReading {
    let mut tracker = GestureTracker::new(points[0]);
    for pair in points.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        let steps = ((to.0 - from.0).abs().max((to.1 - from.1).abs()) / 3).max(1);
        for step in 1..=steps {
            tracker.push((
                from.0 + (to.0 - from.0) * step / steps,
                from.1 + (to.1 - from.1) * step / steps,
            ));
        }
    }
    tracker.reading()
}

#[test]
fn path_rejects_empty_repeated_and_overlong_sequences_without_rewriting() {
    assert_eq!(GesturePath::new(&[]), Err(GesturePathError::Empty));
    assert_eq!(
        GesturePath::new(&[Right, Right]),
        Err(GesturePathError::RepeatedStep { index: 1 })
    );
    assert_eq!(
        GesturePath::new(&[Up, Down, Up, Down, Up, Down, Up, Down, Up]),
        Err(GesturePathError::TooLong { len: 9 })
    );
    let valid = path(&[Right, Down, DownRight]);
    assert_eq!(valid.steps(), [Right, Down, DownRight]);
    assert!(valid.has_diagonal());
    assert_eq!(valid.to_string(), "→↓↘");
    for direction in GestureDirection::ALL {
        assert_eq!(
            GestureDirection::from_token(direction.token()),
            Some(direction)
        );
    }
}

#[test]
fn tracker_reads_straight_corner_and_diagonal_strokes() {
    let straight = track(&[(0, 0), (120, 6)]);
    assert_eq!(straight.cardinal, Some(path(&[Right])));
    assert_eq!(straight.octal, Some(path(&[Right])));

    // 圆角转弯在八向读数中可能留下短斜笔；四向读数仍为“右、下”。
    let corner = track(&[(0, 0), (100, 0), (120, 20), (120, 120)]);
    assert_eq!(corner.cardinal, Some(path(&[Right, Down])));
    assert!(corner.matches(&path(&[Right, Down])));

    let diagonal = track(&[(0, 0), (90, 90)]);
    assert_eq!(diagonal.octal, Some(path(&[DownRight])));
    assert!(diagonal.matches(&path(&[DownRight])));
    assert_eq!(diagonal.preferred(), Some(path(&[DownRight])));
}

#[test]
fn tracker_ignores_perpendicular_jitter_and_short_tails() {
    let mut points = vec![(0, 0)];
    for x in (10..=200).step_by(10) {
        points.push((x, if x % 20 == 0 { 3 } else { -3 }));
    }
    // 松开前的小幅回拉不足一笔。
    points.push((190, -3));
    let reading = track(&points);
    assert_eq!(reading.cardinal, Some(path(&[Right])));
    assert_eq!(reading.octal, Some(path(&[Right])));
}

#[test]
fn tracker_keeps_one_stroke_along_a_sector_boundary() {
    // 贴着八向扇区边界（22.5°）的长直线：采样段在“右”与“右下”之间摇摆，读数仍是一笔。
    let mut points = vec![(0, 0)];
    for x in (12..=360).step_by(12) {
        let wobble = if (x / 12) % 2 == 0 { 3 } else { -3 };
        points.push((x, x * 4_142 / 10_000 + wobble));
    }
    let reading = track(&points);
    assert_eq!(reading.cardinal, Some(path(&[Right])));
    let octal = reading.octal.expect("边界上的直线仍应读出一笔");
    assert!(octal == path(&[Right]) || octal == path(&[DownRight]));

    // 贴着四向边界（45°）的直线同理，四向读数不会因交替而丢失。
    let mut points = vec![(0, 0)];
    for step in 1..=30 {
        let wobble = if step % 2 == 0 { 4 } else { -4 };
        points.push((step * 12, step * 12 + wobble));
    }
    let reading = track(&points);
    let cardinal = reading.cardinal.expect("边界上的直线仍应读出一笔");
    assert!(cardinal == path(&[Right]) || cardinal == path(&[Down]));
    assert_eq!(reading.octal, Some(path(&[DownRight])));
}

#[test]
fn stroke_length_is_euclidean_so_diagonals_are_not_undercounted() {
    // 斜向 34×34（约 48px）与水平 48px 同样成笔；按较大轴计只有 34px 的旧算法会漏掉前者。
    assert_eq!(track(&[(0, 0), (34, 34)]).octal, Some(path(&[DownRight])));
    assert_eq!(track(&[(0, 0), (48, 0)]).octal, Some(path(&[Right])));
    assert_eq!(track(&[(0, 0), (20, 20)]).octal, None);
}

#[test]
fn short_strokes_scale_with_the_longest_stroke() {
    // 大手势松开时的甩尾（50px）超过固定下限，但不足最长一笔的 1/8。
    let hooked = track(&[(0, 0), (480, 0), (455, -44)]);
    assert_eq!(hooked.cardinal, Some(path(&[Right])));
    assert_eq!(hooked.octal, Some(path(&[Right])));
    // 同样 50px 的一笔在小手势里是有意的笔画。
    assert_eq!(
        track(&[(0, 0), (120, 0), (120, -50)]).cardinal,
        Some(path(&[Right, Up]))
    );
    // 含斜向的路径不受大圆角影响：圆角处的短斜笔随比例下限删除。
    let rounded = track(&[
        (0, 0),
        (400, 0),
        (425, 8),
        (442, 25),
        (450, 50),
        (450, 450),
        (250, 650),
    ]);
    assert_eq!(rounded.octal, Some(path(&[Right, Down, DownLeft])));
    // 长短腿 1:5 仍是两笔。
    assert_eq!(
        track(&[(0, 0), (0, 80), (400, 80)]).cardinal,
        Some(path(&[Down, Right]))
    );
}

#[test]
fn extreme_coordinates_do_not_overflow() {
    let mut tracker = GestureTracker::new((i32::MIN, i32::MIN));
    tracker.push((i32::MAX, i32::MAX));
    tracker.push((i32::MIN, i32::MAX));
    assert!(tracker.started());
    assert_eq!(tracker.reading().octal, Some(path(&[DownRight, Left])));
}

#[test]
fn tracker_needs_start_distance_and_rejects_overly_complex_traces() {
    let mut tracker = GestureTracker::new((0, 0));
    tracker.push((11, 0));
    assert!(!tracker.started());
    assert_eq!(tracker.reading().cardinal, None);
    tracker.push((12, 0));
    assert!(tracker.started());

    // 九个完整笔画超过上限；更多往返写满原始缓冲后也不会误判为短路径。
    let mut stairs = vec![(0, 0)];
    for index in 1..=9 {
        let (x, y) = stairs[index - 1];
        stairs.push(if index % 2 == 1 {
            (x + 60, y)
        } else {
            (x, y + 60)
        });
    }
    assert_eq!(
        track(&stairs[..9]).cardinal.map(|path| path.steps().len()),
        Some(8)
    );
    assert_eq!(track(&stairs).cardinal, None);
    let mut long = vec![(0, 0)];
    for index in 1..=80 {
        long.push((0, if index % 2 == 0 { 0 } else { 80 }));
    }
    assert_eq!(track(&long).cardinal, None);
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
fn diagonal_paths_are_compared_before_cardinal_paths() {
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
fn same_path_prefers_the_most_specific_scope_while_other_scopes_still_join() {
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
fn registration_allows_distinct_paths_and_reports_structured_conflicts() {
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
                path: path(&[Right]),
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
