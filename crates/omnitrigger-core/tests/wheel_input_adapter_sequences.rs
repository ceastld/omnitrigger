//! 公共事件序列：真实 Engine 会话快照 → 显式输入门禁 → Engine 决策。
//! 这里只重放合成输入，不代替平台 Hook、物理设备或前台效果验收。
use omnitrigger_core::*;

const PREFIX: Button = Button::Mouse(MouseButton::X2);
const RIGHT: Button = Button::Mouse(MouseButton::Right);
const LEFT: Button = Button::Mouse(MouseButton::Left);
const LEADER: Button = Button::Key(0x70);
const TERMINAL: Button = Button::Key(0x71);
const CONTROL: Button = Button::Key(0x11);
const LEFT_CONTROL: Button = Button::Key(0xa2);

fn event(kind: InputKind) -> InputEvent {
    InputEvent {
        kind,
        injected: false,
    }
}
fn vertical(delta: i32) -> InputKind {
    InputKind::Wheel {
        axis: Axis::Vertical,
        delta,
    }
}

fn binding(id: u64, held: &[Button], activation: Activation) -> TriggerSpec {
    TriggerSpec {
        id,
        held: held.to_vec(),
        activation,
        wheel: WheelPolicy::Notch,
        keyboard: Default::default(),
        conditions: vec![],
        process_scope: Default::default(),
        pointer_scope: Default::default(),
    }
}

fn wheel(id: u64, held: &[Button], direction: WheelDirection, policy: WheelPolicy) -> TriggerSpec {
    let mut row = binding(id, held, Activation::Wheel(direction));
    row.wheel = policy;
    row
}

fn observer(id: u64, held: &[Button], direction: WheelDirection) -> TriggerSpec {
    let mut row = wheel(id, held, direction, WheelPolicy::Packet);
    row.keyboard.matching = KeyboardMatching::MouseExact {
        priority: 0,
        pointer_scope_priority: false,
    };
    row
}

fn owner(id: u64, retained: bool) -> TriggerSpec {
    let mut row = binding(id, &[LEADER], Activation::Press(TERMINAL));
    row.keyboard.matching = KeyboardMatching::Captured { priority: 0 };
    if retained {
        row.keyboard.retained_prefix = Some(Box::new(RetainedPrefix {
            key: LEADER,
            capture_scope: Default::default(),
            quiet_ms: 50,
            fast_ms: 50,
            block_with_primary_mouse: false,
        }));
        row.keyboard.restore_on_tap = true;
    } else {
        row.keyboard.dedicated_prefix = Some(LEADER);
    }
    row
}

fn watcher(id: u64) -> TriggerSpec {
    let mut row = wheel(id, &[LEADER], WheelDirection::Up, WheelPolicy::Packet);
    row.keyboard.matching = KeyboardMatching::MouseCapturedLeader {
        priority: 1,
        pointer_scope_priority: false,
    };
    row
}

fn drag(id: u64, held: &[Button]) -> TriggerSpec {
    binding(
        id,
        held,
        Activation::Drag {
            button: RIGHT,
            min_distance_px: 18,
            max_start_ms: 180,
        },
    )
}

#[derive(Debug)]
struct Step {
    snapshot: WheelSessionSnapshot,
    admission: WheelInputAdmission,
    decision: Decision,
}

struct Stream {
    engine: Engine,
    normalizer: WheelInputNormalizer,
}
impl Stream {
    fn new(bindings: Vec<TriggerSpec>) -> Self {
        Self {
            engine: Engine::new(bindings).unwrap(),
            normalizer: WheelInputNormalizer::new(WheelInputPolicy::SmallVertical150ms),
        }
    }
    fn input(&mut self, kind: InputKind, absolute_ms: i64, engine_ms: u64) -> Step {
        self.route(event(kind), absolute_ms, engine_ms, |_| Decision::default())
    }
    fn route(
        &mut self,
        raw: InputEvent,
        absolute_ms: i64,
        engine_ms: u64,
        update: impl FnOnce(&mut Engine) -> Decision,
    ) -> Step {
        // 快照每次从实际状态机获取，不从 pressed/owned 数量或测试常量猜测。
        let snapshot = self.engine.wheel_session_snapshot();
        let admission = self.normalizer.admit_at(raw, absolute_ms, snapshot);
        let mut decision = Decision::default();
        if admission == WheelInputAdmission::Forward {
            if !raw.injected {
                decision = update(&mut self.engine);
            }
            let next = self.engine.process_at(raw, engine_ms);
            decision.consume |= next.consume;
            decision.replay.extend(next.replay);
            decision.triggered.extend(next.triggered);
            decision.drags.extend(next.drags);
        }
        Step {
            snapshot,
            admission,
            decision,
        }
    }
    fn snapshot(&self, mouse: bool, moving_key: bool) {
        assert_eq!(
            self.engine.wheel_session_snapshot(),
            WheelSessionSnapshot {
                captured_mouse_buttons: mouse,
                holding_move_key: moving_key
            }
        );
    }
}

fn filtered(step: Step) {
    assert!(!step.snapshot.is_active());
    assert_eq!(step.admission, WheelInputAdmission::PassThrough);
    assert_eq!(step.decision, Decision::default());
}

#[test]
fn default_adapter_keeps_direct_engine_packet_and_notch_decisions() {
    assert_eq!(WheelInputPolicy::default(), WheelInputPolicy::Unfiltered);
    assert_eq!(WheelPolicy::default(), WheelPolicy::Notch);
    for policy in [WheelPolicy::Packet, WheelPolicy::Notch] {
        for captured in [false, true] {
            let held = if captured { vec![PREFIX] } else { vec![] };
            let rows: Vec<_> = [
                WheelDirection::Up,
                WheelDirection::Down,
                WheelDirection::Left,
                WheelDirection::Right,
            ]
            .into_iter()
            .enumerate()
            .map(|(index, direction)| wheel(100 + index as u64, &held, direction, policy))
            .collect();
            let mut direct = Engine::new(rows.clone()).unwrap();
            let mut stream = Stream::new(rows);
            stream.normalizer = WheelInputNormalizer::default();
            let mut sequence = vec![
                vertical(60),
                vertical(60),
                vertical(1),
                vertical(-1),
                vertical(240),
                vertical(0),
                InputKind::Wheel {
                    axis: Axis::Horizontal,
                    delta: 120,
                },
            ];
            if captured {
                sequence.insert(0, InputKind::Down(PREFIX));
                sequence.push(InputKind::Up(PREFIX));
            }
            for (time, kind) in sequence.into_iter().enumerate() {
                let expected = direct.process_at(event(kind), time as u64);
                let step = stream.input(kind, time as i64, time as u64);
                assert_eq!(step.admission, WheelInputAdmission::Forward);
                assert_eq!(step.decision, expected);
            }
            stream.snapshot(false, false);
        }
    }
}

#[test]
fn modifier_observation_is_not_a_mouse_session_and_does_not_bypass() {
    for (direction, delta) in [(WheelDirection::Up, 1), (WheelDirection::Down, -1)] {
        let mut stream = Stream::new(vec![observer(201, &[CONTROL], direction)]);
        assert_eq!(
            stream
                .input(InputKind::Down(LEFT_CONTROL), 1000, 0)
                .decision,
            Decision::default()
        );
        stream.snapshot(false, false);
        assert_eq!(
            stream.input(vertical(delta), 1000, 1).decision.triggered,
            vec![201]
        );
        filtered(stream.input(vertical(delta), 1149, 2));
        assert_eq!(
            stream.input(vertical(delta), 1150, 3).decision.triggered,
            vec![201]
        );
        assert_eq!(
            stream.input(InputKind::Up(LEFT_CONTROL), 1151, 4).decision,
            Decision::default()
        );
    }
}

#[test]
fn ordinary_captured_keyboard_layer_and_its_consumed_terminal_do_not_bypass() {
    for (direction, delta) in [(WheelDirection::Up, 1), (WheelDirection::Down, -1)] {
        let mut watch = watcher(302);
        watch.activation = Activation::Wheel(direction);
        let mut stream = Stream::new(vec![owner(301, false), watch]);
        assert!(
            stream
                .input(InputKind::Down(LEADER), 1000, 0)
                .decision
                .consume
        );
        stream.snapshot(false, false);
        assert_eq!(
            stream.input(vertical(delta), 1000, 1).decision.triggered,
            vec![302]
        );
        filtered(stream.input(vertical(delta), 1001, 2));
        assert_eq!(
            stream
                .input(InputKind::Down(TERMINAL), 1002, 3)
                .decision
                .triggered,
            vec![301]
        );
        stream.snapshot(false, false);
        filtered(stream.input(vertical(delta), 1003, 4));
        assert!(
            stream
                .input(InputKind::Up(TERMINAL), 1004, 5)
                .decision
                .consume
        );
        assert!(
            stream
                .input(InputKind::Up(LEADER), 1005, 6)
                .decision
                .consume
        );
    }
}

#[test]
fn unowned_physical_mouse_and_consumed_mouse_terminal_are_not_prefix_sessions() {
    for terminal in [false, true] {
        let mut rows = vec![observer(401, &[], WheelDirection::Up)];
        if terminal {
            rows.push(binding(402, &[], Activation::Press(RIGHT)));
        }
        let mut stream = Stream::new(rows);
        let down = stream.input(InputKind::Down(RIGHT), 1000, 0).decision;
        assert_eq!(down.consume, terminal);
        stream.snapshot(false, false);
        assert_eq!(
            stream.input(vertical(1), 1000, 1).admission,
            WheelInputAdmission::Forward
        );
        filtered(stream.input(vertical(1), 1001, 2));
        assert_eq!(
            stream.input(InputKind::Up(RIGHT), 1002, 3).decision.consume,
            terminal
        );
        stream.snapshot(false, false);
    }
}

#[test]
fn captured_mouse_prefix_survives_commits_and_bypasses_without_sliding_history() {
    let mut stream = Stream::new(vec![
        wheel(501, &[PREFIX], WheelDirection::Up, WheelPolicy::Packet),
        wheel(502, &[PREFIX], WheelDirection::Down, WheelPolicy::Packet),
    ]);
    assert_eq!(
        stream.input(vertical(1), 1000, 0).decision,
        Decision::default()
    );
    assert!(
        stream
            .input(InputKind::Down(PREFIX), 1001, 1)
            .decision
            .consume
    );
    stream.snapshot(true, false);
    for (time, delta, id) in [(1002, 1, 501), (1003, -1, 502), (1004, 1, 501)] {
        let step = stream.input(vertical(delta), time, (time - 1000) as u64);
        assert!(step.snapshot.captured_mouse_buttons);
        assert_eq!(step.admission, WheelInputAdmission::Forward);
        assert_eq!(step.decision.triggered, vec![id]);
        stream.snapshot(true, false);
    }
    let up = stream.input(InputKind::Up(PREFIX), 1005, 5).decision;
    assert!(up.consume && up.replay.is_empty());
    stream.snapshot(false, false);
    filtered(stream.input(vertical(1), 1149, 6));
    assert_eq!(
        stream.input(vertical(1), 1150, 7).admission,
        WheelInputAdmission::Forward
    );
}

#[test]
fn unmatched_mouse_prefix_fallback_ends_the_bypass_before_release() {
    let mut stream = Stream::new(vec![wheel(
        511,
        &[PREFIX],
        WheelDirection::Up,
        WheelPolicy::Packet,
    )]);
    stream.input(vertical(1), 1000, 0);
    stream.input(InputKind::Down(PREFIX), 1001, 1);
    stream.snapshot(true, false);
    let fallback = stream.input(vertical(-1), 1002, 2);
    assert_eq!(fallback.admission, WheelInputAdmission::Forward);
    assert_eq!(fallback.decision.replay, vec![InputKind::Down(PREFIX)]);
    stream.snapshot(false, false);
    filtered(stream.input(vertical(1), 1003, 3));
    assert_eq!(
        stream.input(InputKind::Up(PREFIX), 1004, 4).decision,
        Decision::default()
    );
}

#[test]
fn pointer_recovery_clears_mouse_prefix_but_not_the_shared_clock() {
    let mut stream = Stream::new(vec![wheel(
        521,
        &[PREFIX],
        WheelDirection::Up,
        WheelPolicy::Packet,
    )]);
    stream.engine.pointer_moved((0, 0), 0);
    stream.input(vertical(1), 1000, 0);
    stream.input(InputKind::Down(PREFIX), 1001, 1);
    stream.snapshot(true, false);
    assert_eq!(
        stream.engine.pointer_moved((9, 0), 2).replay,
        vec![InputKind::Down(PREFIX)]
    );
    stream.snapshot(false, false);
    filtered(stream.input(vertical(1), 1002, 3));
    assert_eq!(
        stream.input(InputKind::Up(PREFIX), 1003, 4).decision,
        Decision::default()
    );
}

#[test]
fn pending_and_active_mouse_drag_bypass_then_cancellation_drains_without_bypass() {
    let mut stream = Stream::new(vec![drag(601, &[])]);
    stream.engine.pointer_moved((0, 0), 0);
    stream.input(vertical(1), 1000, 0);
    assert!(
        stream
            .input(InputKind::Down(RIGHT), 1001, 1)
            .decision
            .consume
    );
    stream.snapshot(true, false);
    let zero = stream.input(vertical(0), 1002, 2);
    assert_eq!(zero.admission, WheelInputAdmission::Forward);
    assert_eq!(stream.engine.pending_drag_id(), Some(601));
    let started = stream.engine.pointer_moved((18, 0), 3);
    assert_eq!(started.drags[0].phase, DragPhase::Started);
    stream.snapshot(true, false);
    let cancelled = stream.input(vertical(1), 1003, 4);
    assert_eq!(cancelled.admission, WheelInputAdmission::Forward);
    assert_eq!(cancelled.decision.drags[0].phase, DragPhase::Cancelled);
    stream.snapshot(false, false);
    filtered(stream.input(vertical(1), 1004, 5));
    let up = stream.input(InputKind::Up(RIGHT), 1005, 6).decision;
    assert!(up.consume && up.replay.is_empty() && up.drags.is_empty());
}

#[test]
fn expired_drag_restores_down_and_does_not_lend_the_passed_press_as_a_session() {
    let mut stream = Stream::new(vec![drag(611, &[])]);
    stream.engine.pointer_moved((0, 0), 0);
    stream.input(vertical(1), 1000, 0);
    stream.input(InputKind::Down(RIGHT), 1001, 1);
    stream.snapshot(true, false);
    assert_eq!(
        stream.engine.advance_time(181).replay,
        vec![InputKind::Down(RIGHT)]
    );
    stream.snapshot(false, false);
    filtered(stream.input(vertical(1), 1002, 182));
    assert_eq!(
        stream.input(InputKind::Up(RIGHT), 1003, 183).decision,
        Decision::default()
    );
}

#[test]
fn cancel_pause_context_and_rebind_exclude_draining_prefix_without_clearing_history() {
    for mode in 0..4 {
        let row = wheel(701, &[PREFIX], WheelDirection::Up, WheelPolicy::Packet);
        let mut stream = Stream::new(vec![row.clone()]);
        stream.input(vertical(1), 1000, 0);
        stream.input(InputKind::Down(PREFIX), 1001, 1);
        stream.snapshot(true, false);
        let cleanup = match mode {
            0 => stream.engine.cancel(),
            1 => stream.engine.set_enabled(false),
            2 => stream
                .engine
                .set_foreground_process(Some("synthetic-editor.exe"))
                .unwrap(),
            _ => stream.engine.replace_bindings(vec![row]).unwrap(),
        };
        assert!(cleanup.replay.is_empty());
        stream.snapshot(false, false);
        filtered(stream.input(vertical(1), 1002, 2));
        assert!(
            stream
                .input(InputKind::Down(PREFIX), 1003, 3)
                .decision
                .consume
        );
        stream.snapshot(false, false);
        assert!(
            stream
                .input(InputKind::Up(PREFIX), 1004, 4)
                .decision
                .consume
        );
        stream.engine.set_enabled(true);
        filtered(stream.input(vertical(1), 1149, 5));
        assert_eq!(
            stream.input(vertical(1), 1150, 6).admission,
            WheelInputAdmission::Forward
        );
    }
}

#[test]
fn restored_keyboard_layer_remains_a_keyboard_identity_not_a_mouse_session() {
    let mut stream = Stream::new(vec![owner(801, true), watcher(802)]);
    stream.input(vertical(1), 1000, 0);
    assert!(
        stream
            .input(InputKind::Down(LEADER), 1001, 1000)
            .decision
            .consume
    );
    let restored = stream.input(InputKind::Down(LEFT), 1002, 1060).decision;
    assert_eq!(restored.replay, vec![InputKind::Down(LEADER)]);
    stream.input(InputKind::Up(LEFT), 1003, 1061);
    stream.snapshot(false, false);
    filtered(stream.input(vertical(1), 1004, 1062));
    assert_eq!(
        stream.input(vertical(1), 1150, 1063).decision.triggered,
        vec![802]
    );
    assert_eq!(
        stream.input(InputKind::Up(LEADER), 1151, 1064).decision,
        Decision::default()
    );
}

#[test]
fn directions_regions_and_replaced_rules_share_one_pre_match_timestamp() {
    let mut left = observer(901, &[], WheelDirection::Up);
    left.conditions = vec![TriggerCondition::PointerRegion(
        PointerRegion::ScreenBoundary {
            edge: ScreenEdge::Left,
        },
    )];
    let mut top = observer(902, &[], WheelDirection::Down);
    top.conditions = vec![TriggerCondition::PointerRegion(
        PointerRegion::ScreenBoundary {
            edge: ScreenEdge::Top,
        },
    )];
    let mut stream = Stream::new(vec![left, top.clone()]);
    stream
        .engine
        .set_screens(vec![ScreenRect {
            left: 0,
            top: 0,
            right: 1000,
            bottom: 800,
        }])
        .unwrap();
    stream.engine.pointer_moved((500, 400), 0);
    assert_eq!(
        stream.input(vertical(1), 1000, 0).decision,
        Decision::default()
    );
    stream.engine.pointer_moved((0, 400), 1);
    filtered(stream.input(vertical(1), 1010, 2));
    assert_eq!(
        stream.input(vertical(1), 1150, 3).decision.triggered,
        vec![901]
    );
    stream.engine.pointer_moved((500, 0), 4);
    filtered(stream.input(vertical(-1), 1151, 5));
    assert_eq!(
        stream.input(vertical(-1), 1300, 6).decision.triggered,
        vec![902]
    );
    top.id = 903;
    stream.engine.replace_bindings(vec![top]).unwrap();
    filtered(stream.input(vertical(-1), 1301, 7));
    assert_eq!(
        stream.input(vertical(-1), 1450, 8).decision.triggered,
        vec![903]
    );
}

#[test]
fn excluded_packet_does_not_advance_hold_deadline_or_update_context_or_replay() {
    let hold = binding(
        1001,
        &[],
        Activation::Hold {
            button: TERMINAL,
            duration_ms: 100,
        },
    );
    let mut stream = Stream::new(vec![hold]);
    stream.input(vertical(1), 1000, 0);
    assert!(
        stream
            .input(InputKind::Down(TERMINAL), 1001, 0)
            .decision
            .consume
    );
    stream.snapshot(false, false);
    filtered(stream.route(event(vertical(1)), 1010, 100, |_| {
        panic!("透传不能更新指针/前台上下文")
    }));
    assert_eq!(stream.engine.next_deadline_ms(), Some(100));
    assert_eq!(stream.engine.advance_time(99), Decision::default());
    assert_eq!(stream.engine.advance_time(100).triggered, vec![1001]);
    stream.snapshot(false, false);
    assert!(
        stream
            .input(InputKind::Up(TERMINAL), 1150, 101)
            .decision
            .consume
    );
}

#[test]
fn injected_and_paused_observations_share_history_without_mutating_engine_ownership() {
    let mut stream = Stream::new(vec![observer(1101, &[], WheelDirection::Up)]);
    let mut raw = event(vertical(1));
    raw.injected = true;
    assert_eq!(
        stream
            .route(raw, 1000, 9000, |_| Decision::default())
            .decision,
        Decision::default()
    );
    stream.snapshot(false, false);
    stream.engine.set_enabled(false);
    filtered(stream.input(vertical(1), 1010, 1));
    assert_eq!(
        stream.input(vertical(1), 1150, 2).decision,
        Decision::default()
    );
    stream.engine.set_enabled(true);
    filtered(stream.input(vertical(1), 1151, 3));
    assert_eq!(
        stream.input(vertical(1), 1300, 4).decision.triggered,
        vec![1101]
    );
}

#[test]
fn recorder_keeps_raw_modifier_wheel_even_when_runtime_admission_passes_it_through() {
    let mut stream = Stream::new(vec![observer(1201, &[CONTROL], WheelDirection::Up)]);
    let mut recorder = Recorder::new();
    recorder.process(event(InputKind::Down(LEFT_CONTROL)));
    stream.input(InputKind::Down(LEFT_CONTROL), 1000, 0);
    stream.input(vertical(1), 1000, 1);
    let raw = event(vertical(1));
    let recorded = recorder.process(raw).unwrap();
    assert_eq!(recorded.held, vec![LEFT_CONTROL]);
    assert_eq!(recorded.activation, Activation::Wheel(WheelDirection::Up));
    assert_eq!(recorded.wheel, WheelPolicy::Notch);
    filtered(stream.route(raw, 1010, 2, |_| Decision::default()));
}

#[test]
fn keyboard_prefix_with_mouse_drag_is_a_mouse_session_not_a_keyboard_move_session() {
    let mut stream = Stream::new(vec![drag(1301, &[LEADER])]);
    stream.engine.pointer_moved((0, 0), 0);
    stream.input(vertical(1), 1000, 0);
    assert!(
        stream
            .input(InputKind::Down(LEADER), 1001, 1)
            .decision
            .consume
    );
    stream.snapshot(false, false);
    assert!(
        stream
            .input(InputKind::Down(RIGHT), 1002, 2)
            .decision
            .consume
    );
    stream.snapshot(true, false);
    assert_eq!(
        stream.input(vertical(0), 1003, 3).admission,
        WheelInputAdmission::Forward
    );
    let start = stream.engine.pointer_moved((18, 0), 4);
    assert_eq!(start.drags[0].phase, DragPhase::Started);
    stream.snapshot(true, false);
    let end = stream.input(InputKind::Up(RIGHT), 1004, 5).decision;
    assert_eq!(end.drags[0].phase, DragPhase::Ended);
    stream.snapshot(false, false);
    filtered(stream.input(vertical(1), 1005, 6));
    assert!(
        stream
            .input(InputKind::Up(LEADER), 1006, 7)
            .decision
            .consume
    );
}

#[test]
fn observed_primary_prefix_does_not_create_a_captured_mouse_session() {
    let mut stream = Stream::new(vec![wheel(
        1401,
        &[LEFT],
        WheelDirection::Up,
        WheelPolicy::Packet,
    )]);
    assert_eq!(
        stream.input(InputKind::Down(LEFT), 1000, 0).decision,
        Decision::default()
    );
    stream.snapshot(false, false);
    assert_eq!(
        stream.input(vertical(1), 1000, 1).decision.triggered,
        vec![1401]
    );
    stream.snapshot(false, false);
    filtered(stream.input(vertical(1), 1001, 2));
    assert_eq!(
        stream.input(InputKind::Up(LEFT), 1002, 3).decision,
        Decision::default()
    );
}

#[test]
fn mouse_hold_terminal_is_not_misclassified_as_a_prefix_or_drag() {
    let mut stream = Stream::new(vec![binding(
        1501,
        &[],
        Activation::Hold {
            button: RIGHT,
            duration_ms: 100,
        },
    )]);
    stream.engine.pointer_moved((0, 0), 0);
    stream.input(vertical(1), 1000, 0);
    assert!(
        stream
            .input(InputKind::Down(RIGHT), 1001, 0)
            .decision
            .consume
    );
    stream.snapshot(false, false);
    filtered(stream.input(vertical(1), 1002, 100));
    assert_eq!(stream.engine.advance_time(100).triggered, vec![1501]);
    stream.snapshot(false, false);
    assert!(
        stream
            .input(InputKind::Up(RIGHT), 1003, 101)
            .decision
            .consume
    );
}

#[test]
fn losing_region_after_a_committed_mouse_prefix_ends_bypass_but_keeps_release_owned() {
    let mut row = wheel(1601, &[PREFIX], WheelDirection::Up, WheelPolicy::Packet);
    row.conditions = vec![TriggerCondition::PointerRegion(
        PointerRegion::ScreenBoundary {
            edge: ScreenEdge::Left,
        },
    )];
    let mut stream = Stream::new(vec![row]);
    stream
        .engine
        .set_screens(vec![ScreenRect {
            left: 0,
            top: 0,
            right: 1000,
            bottom: 800,
        }])
        .unwrap();
    stream.engine.pointer_moved((500, 400), 0);
    stream.input(vertical(1), 1000, 0);
    stream.engine.pointer_moved((0, 400), 1);
    assert!(
        stream
            .input(InputKind::Down(PREFIX), 1001, 2)
            .decision
            .consume
    );
    stream.snapshot(true, false);
    assert_eq!(
        stream.input(vertical(1), 1002, 3).decision.triggered,
        vec![1601]
    );
    stream.snapshot(true, false);
    stream.engine.pointer_moved((500, 400), 4);
    stream.snapshot(false, false);
    filtered(stream.input(vertical(1), 1003, 5));
    let up = stream.input(InputKind::Up(PREFIX), 1004, 6).decision;
    assert!(up.consume && up.replay.is_empty());
}
