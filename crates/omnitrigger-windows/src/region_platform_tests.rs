//! 使用固定拓扑和物理坐标的输入序列；不枚举真实屏幕，不注入桌面输入。
use super::*;
use omnitrigger_core::{
    Activation, PointerRegion, ScreenCorner, ScreenEdge, TriggerCondition, WheelDirection,
};

fn screen(left: i32, top: i32, right: i32, bottom: i32) -> ScreenRect {
    ScreenRect {
        left,
        top,
        right,
        bottom,
    }
}

fn setup(screens: Vec<ScreenRect>, pointer: (i32, i32)) -> (ThreadState, Receiver<BackendEvent>) {
    let (sender, receiver) = mpsc::sync_channel(EVENT_CAPACITY);
    let mut state = ThreadState::new(sender);
    state.apply_screen_context(screens, pointer, 0).unwrap();
    (state, receiver)
}

fn regional(activation: Activation, region: PointerRegion) -> TriggerSpec {
    TriggerSpec {
        id: 8,
        held: vec![],
        activation,
        conditions: vec![TriggerCondition::PointerRegion(region)],
    }
}

#[test]
fn keyboard_region_uses_explicit_initial_pointer_and_freezes_it_when_matching() {
    let (mut state, events) = setup(vec![screen(-1920, -1080, 0, 0)], (-1919, -1079));
    let f8 = Button::Key(0x77);
    state.command(BackendCommand::SetBindings(vec![regional(
        Activation::Press(f8),
        PointerRegion::ScreenCorner {
            corner: ScreenCorner::TopLeft,
            size_px: 8,
        },
    )]));
    let down = state.input_at(Some(InputKind::Down(f8)), None, None, 1);
    assert!(down.consume);
    state.input_at(None, None, Some((-1800, -900)), 2);
    assert!(matches!(
        events.try_recv().unwrap(),
        BackendEvent::Triggered {
            id: 8,
            pointer: Some((-1919, -1079))
        }
    ));
    assert!(
        state
            .input_at(Some(InputKind::Up(f8)), None, None, 3)
            .consume
    );
    assert!(
        !state
            .input_at(Some(InputKind::Down(f8)), None, None, 4)
            .consume
    );
    assert!(!events
        .try_iter()
        .any(|event| matches!(event, BackendEvent::Triggered { .. })));
    assert_eq!(state.diagnostics().screen_count, 1);
    assert!(state.diagnostics().screen_context_ready);
}

#[test]
fn wheel_uses_its_own_physical_point_and_full_monitor_bottom_instead_of_work_area() {
    let (mut state, events) = setup(vec![screen(0, 0, 1920, 1080)], (100, 500));
    state.command(BackendCommand::SetBindings(vec![regional(
        Activation::Wheel(WheelDirection::Up),
        PointerRegion::ScreenEdge {
            edge: ScreenEdge::Bottom,
            thickness_px: 4,
        },
    )]));
    let outcome = state.input_at(
        Some(InputKind::Wheel {
            axis: Axis::Vertical,
            delta: 120,
        }),
        None,
        Some((100, 1079)),
        1,
    );
    assert!(outcome.consume);
    assert!(matches!(
        events.try_recv().unwrap(),
        BackendEvent::Triggered {
            id: 8,
            pointer: Some((100, 1079))
        }
    ));
    assert_eq!(state.pointer.load(), Some((100, 1079)));
    assert!(
        !state
            .input_at(
                Some(InputKind::Wheel {
                    axis: Axis::Vertical,
                    delta: 120
                }),
                None,
                Some((100, 1040)),
                2
            )
            .consume
    );
}

#[test]
fn shared_monitor_edge_is_not_exposed_but_outer_edge_still_matches() {
    let (mut state, events) = setup(
        vec![screen(-100, 0, 0, 100), screen(0, 0, 100, 100)],
        (-1, 50),
    );
    state.command(BackendCommand::SetBindings(vec![regional(
        Activation::Wheel(WheelDirection::Down),
        PointerRegion::ScreenEdge {
            edge: ScreenEdge::Right,
            thickness_px: 1,
        },
    )]));
    let scroll = Some(InputKind::Wheel {
        axis: Axis::Vertical,
        delta: -120,
    });
    assert!(!state.input_at(scroll, None, Some((-1, 50)), 1).consume);
    assert!(events.try_iter().next().is_none());
    assert!(state.input_at(scroll, None, Some((99, 50)), 2).consume);
    assert!(matches!(
        events.try_recv().unwrap(),
        BackendEvent::Triggered {
            id: 8,
            pointer: Some((99, 50))
        }
    ));
}

#[test]
fn regional_hold_loses_eligibility_before_the_exact_due_time_without_eight_pixel_motion() {
    let (mut state, events) = setup(vec![screen(0, 0, 100, 100)], (19, 5));
    let right = Button::Mouse(MouseButton::Right);
    state.command(BackendCommand::SetBindings(vec![regional(
        Activation::Hold {
            button: right,
            duration_ms: 300,
        },
        PointerRegion::ScreenCorner {
            corner: ScreenCorner::TopLeft,
            size_px: 20,
        },
    )]));
    assert!(
        state
            .input_at(Some(InputKind::Down(right)), None, Some((19, 5)), 0)
            .consume
    );
    let loss = state.input_at(None, None, Some((20, 5)), 300);
    assert_eq!(loss.replay_kinds, vec![InputKind::Down(right)]);
    assert!(!events
        .try_iter()
        .any(|event| matches!(event, BackendEvent::Triggered { .. })));
    assert_eq!(state.engine.next_deadline_ms(), None);
}

#[test]
fn topology_change_cancels_active_drag_and_pending_hold_and_drains_releases() {
    let right = Button::Mouse(MouseButton::Right);
    for drag in [false, true] {
        let (mut state, events) = setup(vec![screen(0, 0, 1920, 1080)], (100, 100));
        let activation = if drag {
            Activation::Drag {
                button: right,
                min_distance_px: 18,
                max_start_ms: 180,
            }
        } else {
            Activation::Hold {
                button: right,
                duration_ms: 300,
            }
        };
        state.command(BackendCommand::SetBindings(vec![TriggerSpec {
            id: 8,
            held: vec![],
            activation,
            conditions: Vec::new(),
        }]));
        assert!(
            state
                .input_at(Some(InputKind::Down(right)), None, Some((100, 100)), 0)
                .consume
        );
        if drag {
            state.input_at(None, None, Some((118, 100)), 50);
        }
        events.try_iter().for_each(drop);
        state
            .apply_screen_context(vec![screen(-1920, 0, 1920, 1080)], (118, 100), 60)
            .unwrap();
        let notifications: Vec<_> = events.try_iter().collect();
        assert_eq!(notifications.iter().filter(|event| matches!(event, BackendEvent::Drag { event, .. } if event.phase == DragPhase::Cancelled)).count(), usize::from(drag));
        assert!(state.engine.active_drag().is_none());
        assert!(state.engine.next_deadline_ms().is_none());
        assert!(
            state
                .input_at(Some(InputKind::Up(right)), None, Some((118, 100)), 70)
                .consume
        );
        assert!(state.owned_press.is_empty());
    }
}

#[test]
fn failed_topology_pauses_clears_context_and_rejects_enable_until_a_valid_refresh() {
    let (mut state, events) = setup(vec![screen(0, 0, 1920, 1080)], (100, 100));
    state.screen_context_failed("枚举失败".into());
    assert!(!state.enabled);
    assert!(!state.screen_context_ready);
    assert_eq!(state.diagnostics().screen_count, 0);
    state.command(BackendCommand::SetEnabled(true));
    assert!(!state.enabled);
    assert!(events
        .try_iter()
        .any(|event| matches!(event, BackendEvent::Error(message) if message.contains("显示器"))));
    state
        .apply_screen_context(vec![screen(0, 0, 1920, 1080)], (100, 100), 1)
        .unwrap();
    state.command(BackendCommand::SetEnabled(true));
    assert!(state.enabled && state.screen_context_ready);
    assert_eq!(state.pointer.load(), Some((100, 100)));
}

#[test]
fn recording_keeps_conditions_empty_instead_of_capturing_hidden_screen_state() {
    let (mut state, events) = setup(vec![screen(0, 0, 1920, 1080)], (0, 0));
    let f8 = Button::Key(0x77);
    state.command(BackendCommand::StartRecording);
    state.input_at(Some(InputKind::Down(f8)), None, None, 0);
    state.input_at(Some(InputKind::Up(f8)), None, None, 1);
    let spec = events
        .try_iter()
        .find_map(|event| match event {
            BackendEvent::Recorded(spec) => Some(spec),
            _ => None,
        })
        .unwrap();
    assert!(spec.conditions.is_empty());
}
