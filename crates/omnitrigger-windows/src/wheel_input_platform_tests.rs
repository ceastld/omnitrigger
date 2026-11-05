//! 与 mouse_hook 共用 process_mouse_packet_at；不安装 Hook，不启动回放 worker。
use super::*;
use omnitrigger_core::{
    Activation, KeyboardPolicy, PointerRegion, ProcessScope, ScreenEdge, TriggerCondition,
    WheelDirection, WheelPolicy, WindowRole,
};

const SIDE: Button = Button::Mouse(MouseButton::X1);
const RIGHT: Button = Button::Mouse(MouseButton::Right);
const POINT: (i32, i32) = (20, 30);

fn wheel(delta: i32) -> InputKind {
    InputKind::Wheel {
        axis: Axis::Vertical,
        delta,
    }
}

fn binding(id: u64, direction: WheelDirection, held: Vec<Button>) -> TriggerSpec {
    TriggerSpec {
        id,
        held,
        activation: Activation::Wheel(direction),
        wheel: WheelPolicy::Packet,
        keyboard: Default::default(),
        conditions: vec![],
        process_scope: Default::default(),
        pointer_scope: Default::default(),
    }
}

fn plain_bindings() -> Vec<TriggerSpec> {
    vec![
        binding(1, WheelDirection::Up, vec![]),
        binding(2, WheelDirection::Down, vec![]),
    ]
}

struct Fixture {
    received: Receiver<BackendEvent>,
}

impl Fixture {
    fn new(policy: WheelInputPolicy, bindings: Vec<TriggerSpec>) -> Self {
        assert!(STATE.with(|slot| slot.borrow().is_none()));
        assert!(WHEEL_INPUT.with(|slot| slot.borrow().is_none()));
        let (events, received) = mpsc::sync_channel(EVENT_CAPACITY);
        let mut state = ThreadState::new_with_pointer_policy(
            events,
            Arc::new(PointerSnapshot::default()),
            policy,
        );
        state.screen_context_ready = true;
        state.pointer.store(POINT);
        state.command(BackendCommand::SetBindings(bindings));
        match received.try_recv() {
            Err(mpsc::TryRecvError::Empty) => {}
            Ok(BackendEvent::Error(message)) => panic!("测试绑定注册失败：{message}"),
            _ => panic!("测试绑定初始化产生非预期事件或断开"),
        }
        install_input_state(state);
        let fixture = Self { received };
        fixture.assert_snapshot();
        fixture
    }

    fn inspect<T>(&self, operation: impl FnOnce(&ThreadState) -> T) -> T {
        STATE.with(|slot| operation(slot.borrow().as_ref().unwrap()))
    }

    fn change<T>(&self, operation: impl FnOnce(&mut ThreadState) -> T) -> T {
        let result = STATE.with(|slot| operation(slot.borrow_mut().as_mut().unwrap()));
        self.assert_snapshot();
        result
    }

    fn assert_snapshot(&self) {
        self.inspect(|state| {
            assert_eq!(
                state.wheel_session.get(),
                state.engine.wheel_session_snapshot()
            )
        });
    }

    fn send(
        &self,
        kind: Option<InputKind>,
        point: (i32, i32),
        absolute_ms: i64,
        elapsed_ms: u64,
    ) -> bool {
        let packet = MousePacket {
            kind,
            movement: kind.is_none(),
            pointer: point,
            injected: false,
            tag: 0,
        };
        let consume = process_mouse_packet_at(packet, absolute_ms, Some(elapsed_ms));
        self.assert_snapshot();
        consume
    }

    fn scroll(&self, delta: i32, absolute_ms: i64, elapsed_ms: u64) -> bool {
        self.send(Some(wheel(delta)), POINT, absolute_ms, elapsed_ms)
    }

    fn injected(&self, delta: i32, absolute_ms: i64, injected: bool, tag: usize) -> bool {
        process_mouse_packet_at(
            MousePacket {
                kind: Some(wheel(delta)),
                movement: false,
                pointer: (99, 99),
                injected,
                tag,
            },
            absolute_ms,
            Some(99_999),
        )
    }

    fn key(&self, kind: InputKind, elapsed_ms: u64) -> bool {
        self.change(|state| {
            let outcome = state.input_at(Some(kind), None, None, elapsed_ms);
            state.reschedule_hold_timer();
            state.queue_outcome(outcome, Some(kind))
        })
    }

    fn hits(&self) -> Vec<u64> {
        self.received
            .try_iter()
            .filter_map(|event| match event {
                BackendEvent::Triggered { id, .. } => Some(id),
                _ => None,
            })
            .collect()
    }

    fn diagnostics(&self) -> ListenerDiagnostics {
        self.inspect(ThreadState::diagnostics)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        clear_input_state();
    }
}

#[test]
fn wheel_live_default_unfiltered_preserves_packet_matching_at_close_timestamps() {
    let fixture = Fixture::new(WheelInputPolicy::Unfiltered, plain_bindings());
    assert!(fixture.scroll(1, 1, 0));
    assert!(fixture.scroll(1, 2, 1));
    assert_eq!(fixture.hits(), [1, 1]);
    let diagnostics = fixture.diagnostics();
    assert_eq!(diagnostics.wheel_input_policy, WheelInputPolicy::Unfiltered);
    assert_eq!(diagnostics.wheel_input_observed, 2);
    assert_eq!(diagnostics.wheel_input_passthrough, 0);
}

#[test]
fn wheel_live_absolute_boot_clock_is_independent_of_listener_elapsed_time() {
    assert_eq!(absolute_wheel_time_ms(u64::MAX), i64::MAX);
    assert_eq!(absolute_wheel_time_ms(150), 150);
    let fixture = Fixture::new(WheelInputPolicy::SmallVertical150ms, plain_bindings());
    assert!(fixture.scroll(1, absolute_wheel_time_ms(8_000_000), 0));
    assert!(!fixture.scroll(-1, 8_000_149, 50_000));
    assert!(fixture.scroll(-1, 8_000_150, 1));
    assert_eq!(fixture.hits(), [1, 2]);
    assert_eq!(fixture.diagnostics().wheel_input_passthrough, 1);
}

#[test]
fn wheel_live_initial_clock_boundary_and_dropped_packets_do_not_slide_history() {
    let fixture = Fixture::new(WheelInputPolicy::SmallVertical150ms, plain_bindings());
    for at in [0, 100, 149] {
        assert!(!fixture.scroll(1, at, at as u64));
    }
    assert!(fixture.scroll(1, 150, 0));
    for at in [151, 200, 299] {
        assert!(!fixture.scroll(-1, at, 50_000));
    }
    assert!(fixture.scroll(-1, 300, 1));
    assert_eq!(fixture.hits(), [1, 2]);
    assert_eq!(fixture.diagnostics().wheel_input_passthrough, 6);
}

#[test]
fn wheel_live_gate_does_not_update_pointer_expire_context_or_queue_replay() {
    let mut spec = binding(1, WheelDirection::Up, vec![]);
    spec.pointer_scope = ProcessScope {
        allow: vec!["reader".into()],
        ..Default::default()
    };
    let fixture = Fixture::new(WheelInputPolicy::SmallVertical150ms, vec![spec]);
    fixture.change(|state| {
        state.apply_pointer_context(
            pointer_context::Snapshot {
                point: POINT,
                identity: foreground_context::Identity {
                    hwnd: 123,
                    pid: 456,
                },
                name: Some("reader".into()),
                role: WindowRole::Application,
                observed: Instant::now(),
            },
            state.foreground_epoch,
        )
    });
    assert!(fixture.scroll(1, 1000, 0));
    assert_eq!(fixture.hits(), [1]);
    fixture.change(|state| {
        state.pointer_context.as_mut().unwrap().observed = Instant::now() - Duration::from_secs(2)
    });
    assert!(!fixture.send(Some(wheel(1)), (90, 91), 1001, 99_999));
    fixture.inspect(|state| {
        assert_eq!(state.pointer.load(), Some(POINT));
        assert_eq!(state.engine.pointer_process(), Some("reader"));
        assert!(state.pointer_context.is_some());
        assert!(!state.replay.busy());
    });
    assert!(!fixture.send(Some(wheel(1)), (90, 91), 1150, 1));
    fixture.inspect(|state| {
        assert_eq!(state.pointer.load(), Some((90, 91)));
        assert!(state.pointer_context.is_none());
        assert_eq!(state.engine.pointer_process(), None);
    });
    assert!(fixture.hits().is_empty());
}

#[test]
fn wheel_live_unmatched_foreground_context_still_shares_direction_history() {
    let mut specs = plain_bindings();
    for spec in &mut specs {
        spec.process_scope.allow = vec!["reader".into()];
    }
    let fixture = Fixture::new(WheelInputPolicy::SmallVertical150ms, specs);
    assert!(!fixture.scroll(1, 1000, 0));
    fixture.change(|state| {
        state.apply_process_context(
            foreground_context::Identity {
                hwnd: state.foreground,
                pid: state.foreground_pid,
            },
            state.foreground_epoch,
            Some("reader"),
            WindowRole::Application,
        )
    });
    assert!(!fixture.scroll(-1, 1001, 50_000));
    assert!(fixture.scroll(-1, 1150, 1));
    assert_eq!(fixture.hits(), [2]);
}

#[test]
fn wheel_live_pause_and_rebind_keep_history_and_paused_forward_observations() {
    let fixture = Fixture::new(WheelInputPolicy::SmallVertical150ms, plain_bindings());
    assert!(fixture.scroll(1, 1000, 0));
    assert_eq!(fixture.hits(), [1]);
    fixture.change(|state| state.command(BackendCommand::SetEnabled(false)));
    assert!(!fixture.scroll(1, 1100, 100));
    assert!(!fixture.scroll(1, 1150, 150));
    fixture.change(|state| {
        state.command(BackendCommand::SetBindings(vec![binding(
            7,
            WheelDirection::Up,
            vec![],
        )]))
    });
    fixture.change(|state| state.command(BackendCommand::SetEnabled(true)));
    assert!(!fixture.scroll(1, 1200, 200));
    assert!(fixture.scroll(1, 1300, 300));
    assert_eq!(fixture.hits(), [7]);
}

#[test]
fn wheel_live_recording_receives_raw_packet_even_when_gate_votes_passthrough() {
    let fixture = Fixture::new(WheelInputPolicy::SmallVertical150ms, plain_bindings());
    assert!(fixture.scroll(1, 1000, 0));
    fixture.hits();
    fixture.change(|state| state.command(BackendCommand::StartRecording));
    assert!(fixture.scroll(1, 1001, 1));
    let recorded = fixture
        .received
        .try_iter()
        .find_map(|event| match event {
            BackendEvent::Recorded(spec) => Some(*spec),
            _ => None,
        })
        .expect("原始小滚轮包应完成录制");
    let mut recorder = Recorder::new();
    let expected = recorder
        .process(InputEvent {
            kind: wheel(1),
            injected: false,
        })
        .unwrap();
    assert_eq!(recorded, expected);
    assert_eq!(fixture.diagnostics().wheel_input_passthrough, 1);
    assert!(fixture.scroll(1, 1150, 2));
    assert_eq!(fixture.hits(), [1]);
}

#[test]
fn wheel_live_recording_mouse_prefix_keeps_press_and_release_ownership() {
    let fixture = Fixture::new(WheelInputPolicy::SmallVertical150ms, vec![]);
    assert!(!fixture.injected(1, 1000, true, REPLAY_TAG));
    fixture.change(|state| state.command(BackendCommand::StartRecording));
    assert!(fixture.send(Some(InputKind::Down(SIDE)), POINT, 1001, 0));
    assert!(fixture.scroll(1, 1002, 1));
    assert!(fixture.send(Some(InputKind::Up(SIDE)), POINT, 1003, 2));
    let recorded = fixture
        .received
        .try_iter()
        .find_map(|event| match event {
            BackendEvent::Recorded(spec) => Some(*spec),
            _ => None,
        })
        .unwrap();
    assert_eq!(recorded.held, [SIDE]);
    assert_eq!(recorded.activation, Activation::Wheel(WheelDirection::Up));
    fixture.inspect(|state| {
        assert!(state.recording_consumed.is_empty());
        assert!(state.owned_press.is_empty());
        assert!(state.physical_held.is_empty());
    });
}

#[test]
fn wheel_live_all_injection_and_marker_paths_observe_before_ignoring() {
    for (injected, tag) in [
        (true, 0),
        (true, REPLAY_TAG),
        (true, SHORTCUT_TAG),
        (true, PRIORITY_PROBE_TAG),
        (false, REPLAY_TAG),
        (false, SHORTCUT_TAG),
        (false, PRIORITY_PROBE_TAG),
    ] {
        let fixture = Fixture::new(WheelInputPolicy::SmallVertical150ms, plain_bindings());
        assert!(!fixture.injected(1, 1000, injected, tag));
        assert_eq!(fixture.inspect(|state| state.pointer.load()), Some(POINT));
        assert!(fixture.hits().is_empty());
        assert!(!fixture.scroll(1, 1001, 0));
        assert!(fixture.scroll(1, 1150, 1));
        assert_eq!(fixture.hits(), [1]);
        assert_eq!(fixture.diagnostics().wheel_input_observed, 3);
        assert_eq!(fixture.diagnostics().wheel_input_unavailable, 0);
    }
}

#[test]
fn wheel_live_injected_reentry_observes_history_without_borrowing_state() {
    let fixture = Fixture::new(WheelInputPolicy::SmallVertical150ms, plain_bindings());
    STATE.with(|slot| {
        let _borrowed = slot.borrow_mut();
        assert!(!fixture.injected(1, 1000, true, REPLAY_TAG));
    });
    assert!(!fixture.scroll(1, 1001, 0));
    assert!(fixture.scroll(1, 1150, 1));
    assert_eq!(fixture.hits(), [1]);
    assert_eq!(fixture.diagnostics().wheel_input_unavailable, 0);
}

#[test]
fn wheel_live_reentry_reads_captured_mouse_snapshot_and_release_ends_bypass() {
    let fixture = Fixture::new(
        WheelInputPolicy::SmallVertical150ms,
        vec![binding(1, WheelDirection::Up, vec![SIDE])],
    );
    assert!(!fixture.injected(1, 1000, true, REPLAY_TAG));
    assert!(fixture.send(Some(InputKind::Down(SIDE)), POINT, 1001, 0));
    assert!(fixture.inspect(|state| state.engine.wheel_session_snapshot().captured_mouse_buttons));
    STATE.with(|slot| {
        let _borrowed = slot.borrow_mut();
        assert!(!fixture.injected(1, 1002, true, REPLAY_TAG));
    });
    assert_eq!(
        fixture.diagnostics().wheel_input_passthrough,
        0,
        "不能在STATE借用失败时猜false"
    );
    assert!(fixture.scroll(1, 1003, 1));
    assert_eq!(fixture.hits(), [1]);
    assert!(fixture.send(Some(InputKind::Up(SIDE)), POINT, 1004, 2));
    assert!(!fixture.inspect(|state| state.engine.wheel_session_snapshot().captured_mouse_buttons));
    assert!(!fixture.scroll(1, 1005, 3));
    fixture.inspect(|state| assert!(!state.owned_press.contains(&SIDE)));
}

#[test]
fn wheel_live_keyboard_prefix_does_not_claim_pure_key_movement_session() {
    let key = Button::Key(20);
    let mut spec = binding(1, WheelDirection::Up, vec![key]);
    spec.keyboard = KeyboardPolicy {
        dedicated_prefix: Some(key),
        ..Default::default()
    };
    let fixture = Fixture::new(WheelInputPolicy::SmallVertical150ms, vec![spec]);
    assert!(!fixture.injected(1, 1000, true, REPLAY_TAG));
    assert!(fixture.key(InputKind::Down(key), 0));
    let snapshot = fixture.inspect(|state| state.engine.wheel_session_snapshot());
    assert!(!snapshot.captured_mouse_buttons && !snapshot.holding_move_key);
    assert!(!fixture.scroll(1, 1001, 1));
    assert!(fixture.scroll(1, 1150, 2));
    assert_eq!(fixture.hits(), [1]);
    assert!(fixture.key(InputKind::Up(key), 3));
}

#[test]
fn wheel_live_gate_never_advances_hold_time_or_confuses_owned_terminal_with_session() {
    let key = Button::Key(65);
    let mut specs = plain_bindings();
    let mut hold = binding(3, WheelDirection::Up, vec![]);
    hold.wheel = WheelPolicy::Notch;
    hold.activation = Activation::Hold {
        button: key,
        duration_ms: 300,
    };
    specs.push(hold);
    let fixture = Fixture::new(WheelInputPolicy::SmallVertical150ms, specs);
    assert!(!fixture.injected(1, 1000, true, REPLAY_TAG));
    assert!(fixture.key(InputKind::Down(key), 0));
    fixture.inspect(|state| {
        assert!(state.owned_press.contains(&key));
        assert_eq!(state.engine.next_hold_deadline_ms(), Some(300));
        assert!(!state.engine.wheel_session_snapshot().is_active());
    });
    assert!(!fixture.send(Some(wheel(1)), (999, 999), 1001, 10_000));
    fixture.inspect(|state| {
        assert_eq!(state.pointer.load(), Some(POINT));
        assert_eq!(state.engine.next_hold_deadline_ms(), Some(300));
        assert!(!state.replay.busy());
    });
    assert!(fixture.hits().is_empty());
    assert!(fixture.key(InputKind::Up(key), 1));
    assert!(fixture.hits().is_empty());
}

#[test]
fn wheel_live_mouse_drag_and_failure_draining_publish_actual_session_changes() {
    let mut drag = binding(1, WheelDirection::Up, vec![]);
    drag.wheel = WheelPolicy::Notch;
    drag.activation = Activation::Drag {
        button: RIGHT,
        min_distance_px: 18,
        max_start_ms: 180,
    };
    let fixture = Fixture::new(WheelInputPolicy::SmallVertical150ms, vec![drag]);
    assert!(!fixture.injected(1, 1000, true, REPLAY_TAG));
    assert!(fixture.send(Some(InputKind::Down(RIGHT)), POINT, 1001, 0));
    assert!(fixture.inspect(|state| state.engine.wheel_session_snapshot().captured_mouse_buttons));
    fixture.send(None, (POINT.0 + 18, POINT.1), 1002, 20);
    assert!(fixture.inspect(|state| state.engine.active_drag().is_some()));
    fixture.change(|state| state.replay_failed(&[InputKind::Down(RIGHT)]));
    fixture.inspect(|state| {
        assert!(state.failure_draining.contains(&RIGHT));
        assert!(state.owned_press.contains(&RIGHT));
        assert!(!state.engine.wheel_session_snapshot().is_active());
    });
    assert!(!fixture.scroll(1, 1003, 99_999));
    assert!(fixture.send(Some(InputKind::Up(RIGHT)), POINT, 1004, 21));
    fixture.inspect(|state| {
        assert!(!state.failure_draining.contains(&RIGHT));
        assert!(!state.owned_press.contains(&RIGHT));
        assert!(!state.physical_held.contains(&RIGHT));
    });
}

#[test]
fn wheel_live_gate_does_not_enqueue_behind_an_existing_replay_barrier() {
    let fixture = Fixture::new(
        WheelInputPolicy::SmallVertical150ms,
        vec![binding(1, WheelDirection::Up, vec![SIDE])],
    );
    assert!(!fixture.injected(1, 1000, true, REPLAY_TAG));
    assert!(fixture.send(Some(InputKind::Down(SIDE)), POINT, 1001, 0));
    assert!(fixture.send(Some(InputKind::Up(SIDE)), POINT, 1002, 1));
    fixture.inspect(|state| {
        assert!(state.replay.busy());
        assert!(!state.engine.wheel_session_snapshot().is_active());
    });
    assert!(
        !fixture.scroll(1, 1003, 2),
        "门禁透传不得被queue_outcome改成消费补发"
    );
    assert_eq!(fixture.diagnostics().wheel_input_passthrough, 1);
}

#[test]
fn wheel_live_screen_and_foreground_changes_cancel_session_without_resetting_history() {
    for context in [0, 1] {
        let fixture = Fixture::new(
            WheelInputPolicy::SmallVertical150ms,
            vec![binding(1, WheelDirection::Up, vec![SIDE])],
        );
        assert!(!fixture.injected(1, 1000, true, REPLAY_TAG));
        fixture.send(Some(InputKind::Down(SIDE)), POINT, 1001, 0);
        fixture.change(|state| {
            if context == 0 {
                state
                    .apply_screen_context(
                        vec![ScreenRect {
                            left: 0,
                            top: 0,
                            right: 1920,
                            bottom: 1080,
                        }],
                        POINT,
                        1,
                    )
                    .unwrap();
            } else {
                state.update_foreground_identity(foreground_context::Identity {
                    hwnd: state.foreground.wrapping_add(1),
                    pid: state.foreground_pid,
                });
            }
        });
        assert!(!fixture.inspect(|state| state.engine.wheel_session_snapshot().is_active()));
        assert!(!fixture.scroll(1, 1002, 99_999));
        assert_eq!(fixture.diagnostics().wheel_input_passthrough, 1);
    }
}

#[test]
fn wheel_live_large_and_horizontal_packets_forward_without_changing_small_history() {
    let fixture = Fixture::new(WheelInputPolicy::SmallVertical150ms, plain_bindings());
    assert!(!fixture.injected(1, 1000, true, REPLAY_TAG));
    assert!(fixture.scroll(10, 1100, 0));
    fixture.send(
        Some(InputKind::Wheel {
            axis: Axis::Horizontal,
            delta: 1,
        }),
        POINT,
        1149,
        1,
    );
    assert!(fixture.scroll(-1, 1150, 2));
    assert_eq!(fixture.hits(), [1, 2]);
}

#[test]
fn wheel_live_full_stream_restart_creates_new_history() {
    {
        let fixture = Fixture::new(WheelInputPolicy::SmallVertical150ms, plain_bindings());
        assert!(fixture.scroll(1, 1000, 0));
        assert!(!fixture.scroll(1, 1001, 1));
    }
    let fixture = Fixture::new(WheelInputPolicy::SmallVertical150ms, plain_bindings());
    assert!(fixture.scroll(1, 1001, 0));
    assert_eq!(fixture.diagnostics().wheel_input_observed, 1);
}

#[test]
fn wheel_live_clock_rollback_does_not_replace_the_previous_absolute_observation() {
    let fixture = Fixture::new(WheelInputPolicy::SmallVertical150ms, plain_bindings());
    assert!(fixture.scroll(1, 1000, 0));
    assert!(!fixture.scroll(-1, 900, 99_999));
    assert!(!fixture.scroll(-1, 1100, 99_999));
    assert!(fixture.scroll(-1, 1150, 1));
    assert_eq!(fixture.hits(), [1, 2]);
}

#[test]
fn wheel_live_unavailable_stream_is_diagnosed_instead_of_guessing_a_false_snapshot() {
    let fixture = Fixture::new(WheelInputPolicy::SmallVertical150ms, plain_bindings());
    WHEEL_INPUT.with(|slot| slot.borrow_mut().take());
    assert!(!fixture.scroll(1, 1000, 99_999));
    assert!(fixture.hits().is_empty());
    assert_eq!(fixture.inspect(|state| state.pointer.load()), Some(POINT));
    assert_eq!(fixture.diagnostics().wheel_input_unavailable, 1);
}

#[test]
fn wheel_live_physical_reentry_keeps_observation_but_does_not_recurse_into_engine() {
    let fixture = Fixture::new(WheelInputPolicy::SmallVertical150ms, plain_bindings());
    STATE.with(|slot| {
        let _borrowed = slot.borrow_mut();
        assert!(!process_mouse_packet_at(
            MousePacket {
                kind: Some(wheel(1)),
                movement: false,
                pointer: POINT,
                injected: false,
                tag: 0,
            },
            1000,
            Some(99_999)
        ));
    });
    fixture.assert_snapshot();
    assert!(fixture.hits().is_empty());
    assert_eq!(fixture.diagnostics().wheel_input_state_busy, 1);
    assert!(!fixture.scroll(1, 1001, 0));
    assert!(fixture.scroll(1, 1150, 1));
    assert_eq!(fixture.hits(), [1]);
}

#[test]
fn wheel_live_keyboard_prefixed_mouse_drag_is_not_reported_as_pure_key_movement() {
    let key = Button::Key(65);
    let mut drag = binding(3, WheelDirection::Up, vec![key]);
    drag.wheel = WheelPolicy::Notch;
    drag.activation = Activation::Drag {
        button: RIGHT,
        min_distance_px: 18,
        max_start_ms: 180,
    };
    let fixture = Fixture::new(WheelInputPolicy::SmallVertical150ms, vec![drag]);
    assert!(!fixture.injected(1, 1000, true, REPLAY_TAG));
    fixture.key(InputKind::Down(key), 0);
    assert!(!fixture.inspect(|state| state.engine.wheel_session_snapshot().is_active()));
    assert!(fixture.send(Some(InputKind::Down(RIGHT)), POINT, 1001, 1));
    let snapshot = fixture.inspect(|state| state.engine.wheel_session_snapshot());
    assert!(snapshot.captured_mouse_buttons);
    assert!(!snapshot.holding_move_key);
    fixture.send(None, (POINT.0 + 18, POINT.1), 1002, 20);
    let session = fixture.inspect(|state| state.engine.active_drag().unwrap().session);
    fixture.change(|state| state.command(BackendCommand::CancelDrag { session }));
    assert!(!fixture.inspect(|state| state.engine.wheel_session_snapshot().is_active()));
    assert!(!fixture.scroll(1, 1003, 99_999));
    fixture.send(Some(InputKind::Up(RIGHT)), POINT, 1004, 21);
    fixture.key(InputKind::Up(key), 22);
}

#[test]
fn wheel_live_different_screen_regions_share_one_stream_history() {
    let mut left = binding(1, WheelDirection::Up, vec![]);
    left.conditions
        .push(TriggerCondition::PointerRegion(PointerRegion::ScreenEdge {
            edge: ScreenEdge::Left,
            thickness_px: 4,
        }));
    let mut right = binding(2, WheelDirection::Down, vec![]);
    right
        .conditions
        .push(TriggerCondition::PointerRegion(PointerRegion::ScreenEdge {
            edge: ScreenEdge::Right,
            thickness_px: 4,
        }));
    let fixture = Fixture::new(WheelInputPolicy::SmallVertical150ms, vec![left, right]);
    fixture.change(|state| {
        state
            .apply_screen_context(
                vec![ScreenRect {
                    left: 0,
                    top: 0,
                    right: 1920,
                    bottom: 1080,
                }],
                POINT,
                0,
            )
            .unwrap()
    });
    assert!(fixture.send(Some(wheel(1)), (0, 100), 1000, 0));
    assert!(!fixture.send(Some(wheel(-1)), (1919, 100), 1001, 99_999));
    assert_eq!(
        fixture.inspect(|state| state.pointer.load()),
        Some((0, 100))
    );
    assert!(fixture.send(Some(wheel(-1)), (1919, 100), 1150, 1));
    assert_eq!(fixture.hits(), [1, 2]);
}
