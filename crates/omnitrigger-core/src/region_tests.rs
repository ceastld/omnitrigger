use super::*;

const CAPS: Button = Button::Key(0x14);
const W: Button = Button::Key(0x57);
const RIGHT: Button = Button::Mouse(MouseButton::Right);

fn rect(left: i32, top: i32, right: i32, bottom: i32) -> ScreenRect {
    ScreenRect {
        left,
        top,
        right,
        bottom,
    }
}
fn edge(edge: ScreenEdge) -> PointerRegion {
    PointerRegion::ScreenEdge {
        edge,
        thickness_px: 8,
    }
}
fn corner(corner: ScreenCorner) -> PointerRegion {
    PointerRegion::ScreenCorner {
        corner,
        size_px: 32,
    }
}
fn binding(id: u64, held: &[Button], activation: Activation, region: PointerRegion) -> TriggerSpec {
    TriggerSpec {
        id,
        held: held.to_vec(),
        activation,
        conditions: vec![TriggerCondition::PointerRegion(region)],
    }
}
fn event(kind: InputKind) -> InputEvent {
    InputEvent {
        kind,
        injected: false,
    }
}
fn scroll(delta: i32) -> InputEvent {
    event(InputKind::Wheel {
        axis: Axis::Vertical,
        delta,
    })
}
fn left_wheel(id: u64, held: &[Button], direction: WheelDirection) -> TriggerSpec {
    binding(
        id,
        held,
        Activation::Wheel(direction),
        edge(ScreenEdge::Left),
    )
}
fn engine(bindings: Vec<TriggerSpec>, point: (i32, i32)) -> Engine {
    let mut engine = Engine::new(bindings).unwrap();
    engine.set_screens(vec![rect(0, 0, 1000, 800)]).unwrap();
    engine.pointer_moved(point, 0);
    engine
}

#[test]
fn statically_disjoint_regions_never_overlap_in_sampled_multi_screen_topologies() {
    let regions = [
        edge(ScreenEdge::Top),
        edge(ScreenEdge::Right),
        edge(ScreenEdge::Bottom),
        edge(ScreenEdge::Left),
        corner(ScreenCorner::TopLeft),
        corner(ScreenCorner::TopRight),
        corner(ScreenCorner::BottomRight),
        corner(ScreenCorner::BottomLeft),
    ];
    let topologies = [
        vec![rect(0, 0, 10, 10), rect(10, 5, 20, 15)],
        vec![rect(0, 0, 10, 10), rect(5, 5, 15, 15)],
        vec![rect(0, 0, 10, 10); 2],
        vec![rect(-10, 0, 0, 10), rect(5, 0, 15, 10)],
        vec![rect(0, 0, 20, 20), rect(5, 5, 15, 15)],
        vec![rect(0, 0, 1, 1), rect(1, 1, 2, 2)],
    ];
    for screens in topologies {
        for &left in &regions {
            for &right in &regions {
                if super::regions::conditions_overlap(
                    &[TriggerCondition::PointerRegion(left)],
                    &[TriggerCondition::PointerRegion(right)],
                ) {
                    continue;
                }
                for y in -10..=20 {
                    for x in -10..=20 {
                        assert!(
                            !(left.matches((x, y), &screens) && right.matches((x, y), &screens)),
                            "{left:?} 与 {right:?} 在 {screens:?} 的 {x},{y} 同时命中"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn screen_regions_exclude_only_the_connected_segment_and_both_sides_of_it() {
    let screens = [rect(-100, -100, 0, 100), rect(0, 0, 100, 100)];
    let left = edge(ScreenEdge::Left);
    let right = edge(ScreenEdge::Right);
    assert!(right.matches((-1, -50), &screens));
    assert!(!right.matches((-1, 50), &screens));
    assert!(!left.matches((0, 50), &screens));
    assert!(left.matches((-100, 50), &screens));
    assert!(right.matches((99, 50), &screens));
    let vertical = [rect(0, -100, 100, 0), rect(25, 0, 100, 100)];
    assert!(edge(ScreenEdge::Bottom).matches((10, -1), &vertical));
    assert!(!edge(ScreenEdge::Bottom).matches((50, -1), &vertical));
    assert!(!edge(ScreenEdge::Top).matches((50, 0), &vertical));
}

#[test]
fn corner_requires_both_anchor_edges_and_local_segments_to_be_exposed() {
    let screens = [rect(0, 0, 100, 100), rect(100, 20, 200, 100)];
    assert!(corner(ScreenCorner::TopRight).matches((99, 0), &screens));
    assert!(corner(ScreenCorner::TopRight).matches((99, 19), &screens));
    assert!(!corner(ScreenCorner::TopRight).matches((99, 20), &screens));
    assert!(!corner(ScreenCorner::TopLeft).matches((100, 20), &screens));
    assert!(!corner(ScreenCorner::BottomRight).matches((99, 99), &screens));
    assert!(corner(ScreenCorner::BottomRight).matches((199, 99), &screens));
    assert!(corner(ScreenCorner::TopLeft).matches((0, 0), &screens));
    let diagonal = [rect(0, 0, 100, 100), rect(100, 100, 200, 200)];
    assert!(!corner(ScreenCorner::BottomRight).matches((99, 99), &diagonal));
    assert!(!corner(ScreenCorner::TopLeft).matches((100, 100), &diagonal));
    assert!(corner(ScreenCorner::TopRight).matches((99, 0), &diagonal));
}

#[test]
fn empty_context_half_open_rectangles_mirrors_and_coordinate_extremes_are_explicit() {
    let rectangle = PointerRegion::Rectangle(rect(-10, -20, 10, 20));
    assert!(rectangle.matches((-10, -20), &[]));
    assert!(!rectangle.matches((10, 0), &[]));
    assert!(!rectangle.matches((0, 20), &[]));
    assert!(!edge(ScreenEdge::Left).matches((0, 0), &[]));
    let mirrored = [rect(-100, 0, 0, 100); 2];
    assert!(edge(ScreenEdge::Left).matches((-100, 20), &mirrored));
    assert!(edge(ScreenEdge::Right).matches((-1, 20), &mirrored));
    let extreme = [rect(i32::MIN, i32::MIN, i32::MAX, i32::MAX)];
    assert!(corner(ScreenCorner::TopLeft).matches((i32::MIN, i32::MIN), &extreme));
    assert!(corner(ScreenCorner::BottomRight).matches((i32::MAX - 1, i32::MAX - 1), &extreme));
    assert!(!edge(ScreenEdge::Right).matches((i32::MAX, 0), &extreme));
}

#[test]
fn opposite_region_ownership_is_unique_on_narrow_screens() {
    let screen = [rect(0, 0, 5, 5)];
    for y in 0..5 {
        for x in 0..5 {
            let horizontal = [ScreenEdge::Left, ScreenEdge::Right]
                .map(|side| edge(side).matches((x, y), &screen));
            let vertical = [ScreenEdge::Top, ScreenEdge::Bottom]
                .map(|side| edge(side).matches((x, y), &screen));
            assert_eq!(horizontal.into_iter().filter(|&matches| matches).count(), 1);
            assert_eq!(vertical.into_iter().filter(|&matches| matches).count(), 1);
            assert_eq!(
                [
                    ScreenCorner::TopLeft,
                    ScreenCorner::TopRight,
                    ScreenCorner::BottomRight,
                    ScreenCorner::BottomLeft
                ]
                .into_iter()
                .filter(|&side| corner(side).matches((x, y), &screen))
                .count(),
                1
            );
        }
    }
    assert!(edge(ScreenEdge::Left).matches((2, 2), &screen));
    assert!(!edge(ScreenEdge::Right).matches((2, 2), &screen));
}

#[test]
fn wheel_conditions_keep_raw_partial_recovery_and_do_not_borrow_across_edges() {
    let right = binding(
        2,
        &[],
        Activation::Wheel(WheelDirection::Up),
        edge(ScreenEdge::Right),
    );
    let mut engine = engine(
        vec![left_wheel(1, &[], WheelDirection::Up), right],
        (0, 100),
    );
    assert!(engine.process(scroll(60)).consume);
    assert_eq!(
        engine.pointer_moved((999, 100), 1).replay,
        [scroll(60).kind]
    );
    assert!(engine.process(scroll(60)).triggered.is_empty());
    assert_eq!(engine.process(scroll(60)).triggered, [2]);
    assert!(engine.pointer_moved((0, 100), 2).replay.is_empty());
    assert!(engine.process(scroll(60)).triggered.is_empty());
    assert_eq!(engine.process(scroll(60)).triggered, [1]);
    assert_eq!(engine.process(scroll(180)).triggered, [1]);
    assert_eq!(
        engine.pointer_moved((500, 100), 3).replay,
        [scroll(60).kind]
    );
    assert!(!engine.process(scroll(120)).consume);
}

#[test]
fn opposite_regional_wheel_tails_survive_another_direction_firing() {
    let mut engine = engine(
        vec![
            left_wheel(1, &[], WheelDirection::Up),
            left_wheel(2, &[], WheelDirection::Down),
        ],
        (0, 100),
    );
    engine.process(scroll(60));
    engine.process(scroll(-60));
    assert_eq!(engine.process(scroll(60)).triggered, [1]);
    assert_eq!(
        engine.pointer_moved((100, 100), 1).replay,
        [scroll(-60).kind]
    );
}

#[test]
fn region_exit_flushes_prefix_and_does_not_rearm_a_passed_physical_press() {
    let mut engine = engine(vec![left_wheel(1, &[CAPS], WheelDirection::Up)], (0, 100));
    assert!(engine.process(event(InputKind::Down(CAPS))).consume);
    engine.process(scroll(60));
    assert_eq!(
        engine.pointer_moved((100, 100), 1).replay,
        [InputKind::Down(CAPS), scroll(60).kind]
    );
    engine.pointer_moved((0, 100), 2);
    assert!(!engine.process(scroll(120)).consume);
    assert!(!engine.process(event(InputKind::Up(CAPS))).consume);
    assert!(engine.process(event(InputKind::Down(CAPS))).consume);
    assert_eq!(engine.process(scroll(120)).triggered, [1]);
    engine.pointer_moved((500, 100), 3);
    assert!(engine.process(event(InputKind::Up(CAPS))).consume);
}

#[test]
fn conditional_press_release_is_owned_outside_and_outside_prefix_cannot_be_borrowed() {
    let spec = binding(1, &[CAPS], Activation::Press(W), edge(ScreenEdge::Left));
    let mut engine = engine(vec![spec], (100, 100));
    assert!(!engine.process(event(InputKind::Down(CAPS))).consume);
    engine.pointer_moved((0, 100), 1);
    assert!(!engine.process(event(InputKind::Down(W))).consume);
    engine.process(event(InputKind::Up(W)));
    engine.process(event(InputKind::Up(CAPS)));
    assert!(engine.process(event(InputKind::Down(CAPS))).consume);
    assert_eq!(engine.process(event(InputKind::Down(W))).triggered, [1]);
    engine.pointer_moved((100, 100), 2);
    assert!(engine.process(event(InputKind::Up(W))).consume);
    assert!(engine.process(event(InputKind::Up(CAPS))).consume);
}

#[test]
fn region_exit_before_time_evaluation_cancels_a_keyboard_hold_at_its_deadline() {
    let spec = binding(
        1,
        &[],
        Activation::Hold {
            button: W,
            duration_ms: 300,
        },
        edge(ScreenEdge::Left),
    );
    let mut engine = engine(vec![spec], (0, 100));
    engine.process_at(event(InputKind::Down(W)), 0);
    let moved = engine.pointer_moved((100, 100), 300);
    assert!(moved.triggered.is_empty());
    assert_eq!(moved.replay, [InputKind::Down(W)]);
    assert!(!engine.process_at(event(InputKind::Up(W)), 301).consume);
    engine.pointer_moved((0, 100), 400);
    engine.process_at(event(InputKind::Down(W)), 400);
    assert_eq!(engine.advance_time(700).triggered, [1]);
    engine.pointer_moved((100, 100), 701);
    assert!(engine.process_at(event(InputKind::Up(W)), 702).consume);
}

#[test]
fn drag_lifecycle_cancels_on_region_exit_and_drains_the_owned_release() {
    let spec = binding(
        1,
        &[],
        Activation::Drag {
            button: RIGHT,
            min_distance_px: 18,
            max_start_ms: 180,
        },
        edge(ScreenEdge::Left),
    );
    let mut engine = engine(vec![spec], (0, 100));
    engine.process_at(event(InputKind::Down(RIGHT)), 0);
    assert_eq!(
        engine.pointer_moved((0, 120), 10).drags[0].phase,
        DragPhase::Started
    );
    let cancel = engine.pointer_moved((100, 120), 20);
    assert_eq!(cancel.drags[0].phase, DragPhase::Cancelled);
    assert_eq!(cancel.drags[0].position, (100, 120));
    assert!(cancel.replay.is_empty());
    assert!(engine.process_at(event(InputKind::Up(RIGHT)), 21).consume);
    assert!(engine.active_drag().is_none());
}

#[test]
fn topology_changes_are_transactional_and_cancel_old_gestures_but_equal_snapshots_do_not() {
    let spec = binding(
        1,
        &[],
        Activation::Hold {
            button: W,
            duration_ms: 300,
        },
        edge(ScreenEdge::Left),
    );
    let mut engine = engine(vec![spec], (0, 100));
    engine.process_at(event(InputKind::Down(W)), 0);
    assert_eq!(
        engine.set_screens(vec![rect(0, 0, 1000, 800)]).unwrap(),
        Decision::default()
    );
    assert!(engine.set_screens(vec![rect(1, 0, 0, 1)]).is_err());
    assert!(engine.set_screens(vec![rect(0, 0, 1000, 800); 33]).is_err());
    assert_eq!(engine.next_deadline_ms(), Some(300));
    engine.set_screens(vec![rect(0, 0, 500, 800)]).unwrap();
    assert_eq!(engine.next_deadline_ms(), None);
    assert!(engine.process_at(event(InputKind::Up(W)), 301).consume);
    assert!(engine.advance_time(500).triggered.is_empty());
}

#[test]
fn injected_discrete_events_cannot_advance_time_or_flush_regional_pending_input() {
    let spec = binding(
        1,
        &[],
        Activation::Hold {
            button: W,
            duration_ms: 300,
        },
        edge(ScreenEdge::Left),
    );
    let mut engine = engine(vec![spec], (0, 100));
    engine.process_at(event(InputKind::Down(W)), 1000);
    assert_eq!(
        engine.process_at(
            InputEvent {
                kind: InputKind::Up(W),
                injected: true
            },
            5000
        ),
        Decision::default()
    );
    assert_eq!(engine.next_deadline_ms(), Some(1300));
    assert_eq!(engine.advance_time(1300).triggered, [1]);
}

#[test]
fn registration_rejects_intersections_without_giving_a_region_implicit_priority() {
    let base = left_wheel(1, &[], WheelDirection::Up);
    let mut unconditional = base.clone();
    unconditional.id = 2;
    unconditional.conditions.clear();
    assert!(matches!(
        Engine::new(vec![base.clone(), unconditional]),
        Err(RegistrationError::DuplicateTrigger { .. })
    ));
    let right = binding(
        2,
        &[],
        Activation::Wheel(WheelDirection::Up),
        edge(ScreenEdge::Right),
    );
    assert!(Engine::new(vec![base.clone(), right]).is_ok());
    let top_left = binding(
        2,
        &[],
        Activation::Wheel(WheelDirection::Up),
        corner(ScreenCorner::TopLeft),
    );
    assert!(matches!(
        Engine::new(vec![base.clone(), top_left]),
        Err(RegistrationError::DuplicateTrigger { .. })
    ));
    let top_right = binding(
        2,
        &[],
        Activation::Wheel(WheelDirection::Up),
        corner(ScreenCorner::TopRight),
    );
    assert!(Engine::new(vec![base.clone(), top_right]).is_ok());
    let absolute = binding(
        2,
        &[],
        Activation::Wheel(WheelDirection::Up),
        PointerRegion::Rectangle(rect(400, 400, 500, 500)),
    );
    assert!(matches!(
        Engine::new(vec![base, absolute]),
        Err(RegistrationError::DuplicateTrigger { .. })
    ));
    let absolute = |id, area| {
        binding(
            id,
            &[],
            Activation::Wheel(WheelDirection::Up),
            PointerRegion::Rectangle(area),
        )
    };
    assert!(
        Engine::new(vec![
            absolute(1, rect(0, 0, 10, 10)),
            absolute(2, rect(10, 0, 20, 10))
        ])
        .is_ok()
    );
    assert!(matches!(
        Engine::new(vec![
            absolute(1, rect(0, 0, 11, 10)),
            absolute(2, rect(10, 0, 20, 10))
        ]),
        Err(RegistrationError::DuplicateTrigger { .. })
    ));
}

#[test]
fn condition_validation_is_structured_and_invalid_replacement_preserves_pending_state() {
    let base = left_wheel(1, &[], WheelDirection::Up);
    let mut engine = engine(vec![base.clone()], (0, 100));
    engine.process(scroll(60));
    let mut bad = base.clone();
    bad.conditions.push(bad.conditions[0]);
    assert!(matches!(
        engine.replace_bindings(vec![bad]),
        Err(RegistrationError::TooManyConditions { id: 1 })
    ));
    let mut bad = base;
    bad.conditions = vec![TriggerCondition::PointerRegion(PointerRegion::ScreenEdge {
        edge: ScreenEdge::Left,
        thickness_px: 0,
    })];
    assert!(matches!(
        engine.replace_bindings(vec![bad]),
        Err(RegistrationError::InvalidPointerRegion {
            id: 1,
            reason: RegionError::InvalidThickness { thickness_px: 0 }
        })
    ));
    assert_eq!(engine.process(scroll(60)).triggered, [1]);
    assert!(
        PointerRegion::ScreenCorner {
            corner: ScreenCorner::TopLeft,
            size_px: 1025
        }
        .validate()
        .is_err()
    );
    assert!(
        PointerRegion::Rectangle(rect(0, 1, 1, 0))
            .validate()
            .is_err()
    );
}
