//! 仅合成上下文和已有fake worker；不启动Hook、线程、计时器或原生输入。
use super::super::{Ack, Work};
use super::{acknowledge, fake_runtime, kinds, next_job};
use crate::platform::*;
use omnitrigger_core::{Activation, ProcessScope, WheelDirection, WheelPolicy, WindowRole};

const POINT: (i32, i32) = (20, 30);
const CAPS: Button = Button::Key(20);
const W: Button = Button::Key(87);
const X1: Button = Button::Mouse(MouseButton::X1);
const X2: Button = Button::Mouse(MouseButton::X2);

struct Fixture {
    jobs: Receiver<Work>,
    acks: SyncSender<Ack>,
    events: Receiver<BackendEvent>,
}

impl Fixture {
    fn new(bindings: Vec<TriggerSpec>) -> Self {
        let (sender, events) = mpsc::sync_channel(256);
        let identity = foreground_context::Identity {
            hwnd: 0x55,
            pid: 0x66,
        };
        let mut state = ThreadState::from_context(
            sender,
            Arc::new(PointerSnapshot::default()),
            WheelInputPolicy::Unfiltered,
            identity,
            None,
        );
        state.test_foreground = Some(identity);
        state.screen_context_ready = true;
        state.pointer.store(POINT);
        let (runtime, jobs, acks) = fake_runtime();
        state.replay = runtime;
        state.command(BackendCommand::SetBindings(bindings));
        state.apply_process_context(
            identity,
            state.foreground_epoch,
            Some("receiver"),
            WindowRole::Application,
        );
        let stream =
            WheelInputStream::new(WheelInputPolicy::Unfiltered, state.wheel_session.clone());
        STATE.with(|slot| {
            assert!(slot.borrow().is_none());
            *slot.borrow_mut() = Some(state);
        });
        WHEEL_INPUT.with(|slot| {
            assert!(slot.borrow().is_none());
            *slot.borrow_mut() = Some(stream);
        });
        Self { jobs, acks, events }
    }

    fn state<T>(&self, action: impl FnOnce(&mut ThreadState) -> T) -> T {
        STATE.with(|slot| {
            let mut slot = slot.borrow_mut();
            let state = slot.as_mut().unwrap();
            assert!(state.test_foreground.is_some());
            assert!(state.waitable_timer.is_none());
            assert_eq!(state.hold_timer, 0);
            assert_eq!(state.engine.next_deadline_ms(), None);
            action(state)
        })
    }

    fn scope(&self, name: &str) {
        self.state(|state| {
            state.apply_pointer_context(
                pointer_context::Snapshot {
                    point: POINT,
                    identity: foreground_context::Identity {
                        hwnd: 0x77,
                        pid: 0x88,
                    },
                    name: Some(name.into()),
                    role: WindowRole::Application,
                    observed: Instant::now(),
                },
                state.foreground_epoch,
            )
        });
    }

    fn key(&self, message: u32, key: u32, injected: bool, tag: usize, now: u64) -> bool {
        // Caps/W不经过Shift的MapVirtualKey分支；补发保留这里显式提供的扫描码。
        assert!(matches!(key, 20 | 87));
        let mut native = KBDLLHOOKSTRUCT::default();
        native.vkCode = key;
        native.scanCode = if key == 20 { 0x3a } else { 0x11 };
        native.flags = if injected {
            LLKHF_INJECTED
        } else {
            Default::default()
        };
        native.dwExtraInfo = tag;
        let Some((kind, metadata)) = keyboard_input(message, &native) else {
            return false;
        };
        self.state(|state| {
            if let Some(sample) = state.pointer_context.as_mut() {
                sample.observed = Instant::now();
            }
            let outcome = state.input_at(Some(kind), Some(metadata), None, now);
            state.queue_outcome(outcome, Some(kind))
        })
    }

    fn mouse(&self, message: u32, data: u32, injected: bool, tag: usize, now: u64) -> bool {
        self.state(|state| {
            // 本组只测所有权交接；指针样本存续时间由夹具明确刷新，过期另有既有测试。
            if let Some(sample) = state.pointer_context.as_mut() {
                sample.observed = Instant::now();
            }
        });
        process_mouse_packet_at(
            MousePacket {
                kind: mouse_input(message, data),
                movement: message == WM_MOUSEMOVE,
                pointer: POINT,
                injected,
                tag,
            },
            now as i64,
            Some(now),
        )
    }

    fn down(&self, button: Button, now: u64) -> bool {
        match button {
            CAPS => self.key(WM_KEYDOWN, 20, false, 0, now),
            W => self.key(WM_KEYDOWN, 87, false, 0, now),
            X1 => self.mouse(WM_XBUTTONDOWN, 1 << 16, false, 0, now),
            X2 => self.mouse(WM_XBUTTONDOWN, 2 << 16, false, 0, now),
            _ => unreachable!(),
        }
    }

    fn up(&self, button: Button, now: u64) -> bool {
        match button {
            CAPS => self.key(WM_KEYUP, 20, false, 0, now),
            W => self.key(WM_KEYUP, 87, false, 0, now),
            X1 => self.mouse(WM_XBUTTONUP, 1 << 16, false, 0, now),
            X2 => self.mouse(WM_XBUTTONUP, 2 << 16, false, 0, now),
            _ => unreachable!(),
        }
    }

    fn wheel(&self, delta: i16, now: u64) -> bool {
        self.mouse(WM_MOUSEWHEEL, u32::from(delta as u16) << 16, false, 0, now)
    }

    fn hits(&self) -> Vec<u64> {
        self.events
            .try_iter()
            .filter_map(|event| match event {
                BackendEvent::Triggered { id, .. } => Some(id),
                BackendEvent::Error(error) => panic!("假后端不应错误：{error}"),
                _ => None,
            })
            .collect()
    }

    fn pump(&self) {
        self.state(|state| {
            let effects = state.replay.pump(state.foreground);
            state.apply_replay_effects(effects);
        });
    }

    fn delivered(&self, expected: &[InputKind]) {
        self.pump();
        let job = next_job(&self.jobs);
        assert!(!job.cleanup);
        assert_eq!(kinds(&job), expected);
        for (native, tag) in job.inputs.iter().zip(&job.tags) {
            if matches!(tag.kind, InputKind::Down(CAPS) | InputKind::Up(CAPS)) {
                // 读取INPUT普通内存，不调用Native；原始扫描码和自身标记均须保留。
                let keyboard = unsafe { native.Anonymous.ki };
                assert_eq!(keyboard.wScan, 0x3a);
                assert_eq!(keyboard.wVk.0, 0);
                assert_eq!(keyboard.dwExtraInfo, REPLAY_TAG);
                assert!(keyboard.dwFlags.contains(KEYEVENTF_SCANCODE));
                assert_eq!(
                    keyboard.dwFlags.contains(KEYEVENTF_KEYUP),
                    matches!(tag.kind, InputKind::Up(_))
                );
            }
        }
        acknowledge(&self.acks, &job, job.tags.len());
        self.pump();
    }

    fn clean(&self) {
        self.state(|state| {
            assert!(state.physical_held.is_empty());
            assert!(state.owned_press.is_empty());
            assert!(state.failure_draining.is_empty());
            assert!(!state.replay.busy());
        });
        assert!(self.jobs.try_recv().is_err());
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        WHEEL_INPUT.with(|slot| {
            slot.borrow_mut().take();
        });
        // fake_runtime没有线程和handle；其Drop只向假通道发送Stop，不会SendInput。
        STATE.with(|slot| {
            slot.borrow_mut().take();
        });
    }
}

fn binding(id: u64, prefix: Button, activation: Activation, scope: Option<&str>) -> TriggerSpec {
    TriggerSpec {
        id,
        held: vec![prefix],
        activation,
        conditions: vec![],
        process_scope: Default::default(),
        pointer_scope: ProcessScope {
            allow: scope.into_iter().map(str::to_owned).collect(),
            ..Default::default()
        },
        wheel: if matches!(activation, Activation::Wheel(_)) {
            WheelPolicy::Packet
        } else {
            WheelPolicy::Notch
        },
        keyboard: Default::default(),
    }
}

fn switched(prefix: Button) -> Vec<TriggerSpec> {
    vec![
        binding(
            201,
            prefix,
            Activation::Wheel(WheelDirection::Up),
            Some("first"),
        ),
        binding(
            202,
            prefix,
            Activation::Wheel(WheelDirection::Down),
            Some("second"),
        ),
        binding(203, prefix, Activation::Press(X2), Some("second")),
    ]
}

#[test]
fn caps_pending_scope_change_replays_original_scan_down_and_physical_up_once() {
    let fixture = Fixture::new(switched(CAPS));
    fixture.scope("first");
    assert!(fixture.down(CAPS, 1));
    assert!(fixture.down(CAPS, 2));
    fixture.scope("second");
    assert!(fixture.up(CAPS, 3)); // 回执前先到Up，由原屏障排在Down之后。
    fixture.delivered(&[InputKind::Down(CAPS), InputKind::Up(CAPS)]);
    assert!(fixture.hits().is_empty());
    fixture.scope("second");
    assert!(fixture.down(CAPS, 4));
    assert!(fixture.wheel(-1, 5));
    assert!(fixture.up(CAPS, 6));
    assert_eq!(fixture.hits(), [202]);
    fixture.clean();
}

#[test]
fn caps_committed_scope_revocation_keeps_keyboard_owner_but_never_rearms_mouse_owner() {
    let fixture = Fixture::new(vec![
        binding(
            198,
            CAPS,
            Activation::Wheel(WheelDirection::Up),
            Some("first"),
        ),
        binding(9, CAPS, Activation::Press(W), None),
    ]);
    fixture.scope("first");
    assert!(fixture.down(CAPS, 1));
    assert!(fixture.wheel(1, 2));
    fixture.scope("second");
    fixture.scope("first");
    assert!(fixture.down(CAPS, 3));
    assert!(!fixture.wheel(240, 4));
    assert!(fixture.down(W, 5));
    assert!(fixture.up(W, 6));
    assert!(fixture.up(CAPS, 7));
    assert_eq!(fixture.hits(), [198, 9]);
    fixture.clean();
}

#[test]
fn x1_pending_handoff_queues_normalized_wheel_and_up_behind_original_down() {
    let fixture = Fixture::new(switched(X1));
    fixture.scope("first");
    assert!(fixture.down(X1, 1));
    assert!(fixture.down(X1, 2));
    fixture.scope("second");
    assert!(fixture.wheel(-15, 3)); // 是交付屏障消费，不是匹配成功。
    assert!(fixture.up(X1, 4));
    fixture.delivered(&[
        InputKind::Down(X1),
        InputKind::Wheel {
            axis: Axis::Vertical,
            delta: -15,
        },
        InputKind::Up(X1),
    ]);
    assert!(fixture.hits().is_empty());
    fixture.scope("second");
    assert!(fixture.down(X1, 5));
    assert!(fixture.wheel(-240, 6));
    assert!(fixture.up(X1, 7));
    assert_eq!(fixture.hits(), [202]);
    fixture.clean();
}

#[test]
fn x1_committed_handoff_drains_old_press_without_replay_then_fresh_terminal_pairs_release() {
    for release in [[X1, X2], [X2, X1]] {
        let fixture = Fixture::new(switched(X1));
        fixture.scope("first");
        assert!(fixture.down(X1, 1));
        assert!(fixture.wheel(1, 2));
        fixture.scope("second");
        assert!(fixture.down(X1, 3));
        assert!(!fixture.wheel(-1, 4));
        assert!(fixture.up(X1, 5));
        fixture.scope("second");
        assert!(fixture.down(X1, 6));
        assert!(fixture.down(X2, 7));
        assert!(fixture.down(X2, 8));
        for (index, button) in release.into_iter().enumerate() {
            assert!(fixture.up(button, 9 + index as u64));
        }
        assert_eq!(fixture.hits(), [201, 203]);
        fixture.clean();
    }
}

#[test]
fn own_tags_and_external_injection_cannot_release_or_rebuild_caps_or_x1() {
    for prefix in [CAPS, X1] {
        let fixture = Fixture::new(switched(prefix));
        fixture.scope("first");
        assert!(fixture.down(prefix, 1));
        for (injected, tag) in [
            (true, 0),
            (false, REPLAY_TAG),
            (false, SHORTCUT_TAG),
            (false, PRIORITY_PROBE_TAG),
        ] {
            if prefix == CAPS {
                assert!(!fixture.key(WM_KEYUP, 20, injected, tag, 2));
                assert!(!fixture.key(WM_KEYDOWN, 20, injected, tag, 2));
            } else {
                assert!(!fixture.mouse(WM_XBUTTONUP, 1 << 16, injected, tag, 2));
                assert!(!fixture.mouse(WM_XBUTTONDOWN, 1 << 16, injected, tag, 2));
            }
            assert!(!fixture.mouse(WM_MOUSEWHEEL, 120 << 16, injected, tag, 2));
        }
        fixture.state(|state| {
            assert!(state.physical_held.contains(&prefix));
            assert!(state.owned_press.contains(&prefix));
        });
        fixture.scope("first");
        assert!(fixture.wheel(1, 3));
        assert!(fixture.up(prefix, 4));
        assert_eq!(fixture.hits(), [201]);
        fixture.clean();
    }
}

#[test]
fn foreground_process_pause_and_replacement_cancel_without_leaking_consumed_releases() {
    for prefix in [CAPS, X1] {
        for boundary in 0..4 {
            for release in [[prefix, X2], [X2, prefix]] {
                let fixture = Fixture::new(vec![binding(
                    500,
                    prefix,
                    Activation::Press(X2),
                    Some("first"),
                )]);
                fixture.scope("first");
                assert!(fixture.down(prefix, 1));
                assert!(fixture.down(X2, 2));
                fixture.state(|state| match boundary {
                    0 => {
                        state.test_foreground.as_mut().unwrap().hwnd += 1;
                    }
                    1 => {
                        let identity = state.test_foreground.unwrap();
                        state.apply_process_context(
                            identity,
                            state.foreground_epoch,
                            Some("after"),
                            WindowRole::Application,
                        );
                    }
                    2 => {
                        state.command(BackendCommand::SetEnabled(false));
                        state.command(BackendCommand::SetEnabled(true));
                    }
                    _ => {
                        state.command(BackendCommand::SetBindings(vec![binding(
                            500,
                            prefix,
                            Activation::Press(X2),
                            Some("first"),
                        )]));
                    }
                });
                assert!(fixture.down(prefix, 3));
                assert!(fixture.down(X2, 4));
                for (index, button) in release.into_iter().enumerate() {
                    assert!(fixture.up(button, 5 + index as u64));
                }
                assert_eq!(fixture.hits(), [500]);
                fixture.clean();
            }
        }
    }
}

fn scope_at(fixture: &Fixture, point: (i32, i32), name: &str) {
    fixture.state(|state| {
        state.apply_pointer_context(
            pointer_context::Snapshot {
                point,
                identity: foreground_context::Identity {
                    hwnd: 0x77,
                    pid: 0x88,
                },
                name: Some(name.into()),
                role: WindowRole::Application,
                observed: Instant::now(),
            },
            state.foreground_epoch,
        );
    });
}

fn mouse_at(message: u32, data: u32, point: (i32, i32), now: u64) -> bool {
    process_mouse_packet_at(
        MousePacket {
            kind: mouse_input(message, data),
            movement: message == WM_MOUSEMOVE,
            pointer: point,
            injected: false,
            tag: 0,
        },
        now as i64,
        Some(now),
    )
}

fn same_scope_rules(prefix: Button) -> Vec<TriggerSpec> {
    vec![
        binding(
            201,
            prefix,
            Activation::Wheel(WheelDirection::Up),
            Some("first"),
        ),
        binding(
            202,
            prefix,
            Activation::Wheel(WheelDirection::Down),
            Some("first"),
        ),
    ]
}

fn release_at(fixture: &Fixture, prefix: Button, point: (i32, i32), now: u64) -> bool {
    if prefix == CAPS {
        fixture.up(CAPS, now)
    } else {
        mouse_at(WM_XBUTTONUP, 1 << 16, point, now)
    }
}

#[test]
fn pointer_motion_same_scope_probe_keeps_original_consumed_caps_and_x1_owners() {
    let next = (POINT.0 + 1, POINT.1);
    for prefix in [CAPS, X1] {
        let fixture = Fixture::new(same_scope_rules(prefix));
        fixture.scope("first");
        assert!(fixture.down(prefix, 1));
        assert!(fixture.wheel(1, 2));
        assert!(!mouse_at(WM_MOUSEMOVE, 0, next, 3));
        fixture.state(|state| {
            // 暂存的是旧采样，绝不伪装成已经验证新坐标。
            assert!(!state
                .pointer_context
                .as_ref()
                .unwrap()
                .valid_at(Some(next), Instant::now()));
            assert_eq!(state.engine.pointer_process(), Some("first"));
        });
        scope_at(&fixture, next, "first"); // 模拟既有Hook外采样确认同一范围。
        assert!(mouse_at(
            WM_MOUSEWHEEL,
            u32::from((-1i16) as u16) << 16,
            next,
            4
        ));
        assert!(release_at(&fixture, prefix, next, 5));
        assert_eq!(fixture.hits(), [201, 202]);
        fixture.clean();
    }
}

#[test]
fn pointer_motion_without_new_sample_rejects_discrete_input_and_cannot_rearm_old_press() {
    let next = (POINT.0 + 1, POINT.1);
    for prefix in [CAPS, X1] {
        let fixture = Fixture::new(same_scope_rules(prefix));
        fixture.scope("first");
        assert!(fixture.down(prefix, 1));
        assert!(fixture.wheel(1, 2));
        assert!(!mouse_at(WM_MOUSEMOVE, 0, next, 3));
        // 未到达新点采样时，下一条离散输入必须立即写Unknown并撤销owner。
        assert!(!mouse_at(
            WM_MOUSEWHEEL,
            u32::from((-1i16) as u16) << 16,
            next,
            4
        ));
        fixture.state(|state| assert_eq!(state.engine.pointer_process(), None));
        scope_at(&fixture, next, "first");
        assert!(!mouse_at(
            WM_MOUSEWHEEL,
            u32::from((-1i16) as u16) << 16,
            next,
            5
        ));
        assert!(release_at(&fixture, prefix, next, 6));
        assert_eq!(fixture.hits(), [201]);
        fixture.clean();
    }
}

#[test]
fn pointer_motion_changed_scope_probe_still_revokes_before_matching_and_never_borrows_back() {
    let next = (POINT.0 + 1, POINT.1);
    for prefix in [CAPS, X1] {
        let fixture = Fixture::new(same_scope_rules(prefix));
        fixture.scope("first");
        assert!(fixture.down(prefix, 1));
        assert!(fixture.wheel(1, 2));
        assert!(!mouse_at(WM_MOUSEMOVE, 0, next, 3));
        scope_at(&fixture, next, "second");
        assert!(!mouse_at(
            WM_MOUSEWHEEL,
            u32::from((-1i16) as u16) << 16,
            next,
            4
        ));
        scope_at(&fixture, next, "first");
        assert!(!mouse_at(
            WM_MOUSEWHEEL,
            u32::from((-1i16) as u16) << 16,
            next,
            5
        ));
        assert!(release_at(&fixture, prefix, next, 6));
        assert_eq!(fixture.hits(), [201]);
        fixture.clean();
    }
}

#[test]
fn pointer_motion_pending_x1_still_falls_back_at_drag_distance() {
    let next = (POINT.0 + 20, POINT.1);
    let fixture = Fixture::new(same_scope_rules(X1));
    fixture.scope("first");
    assert!(fixture.down(X1, 1));
    assert!(!mouse_at(WM_MOUSEMOVE, 0, next, 2));
    fixture.delivered(&[InputKind::Down(X1)]);
    scope_at(&fixture, next, "first");
    assert!(!mouse_at(WM_MOUSEWHEEL, 1 << 16, next, 3));
    assert!(!release_at(&fixture, X1, next, 4));
    assert!(fixture.hits().is_empty());
    fixture.clean();
}

#[test]
fn pointer_motion_pending_hold_still_invalidates_scope_without_arming_a_native_timer() {
    let right = Button::Mouse(MouseButton::Right);
    let mut hold = binding(600, X1, Activation::Press(X2), Some("first"));
    hold.held.clear();
    hold.activation = Activation::Hold {
        button: right,
        duration_ms: 300,
    };
    let fixture = Fixture::new(vec![hold]);
    fixture.scope("first");
    fixture.state(|state| {
        // 仅走相同input_at/queue_outcome；不调用外层reschedule_hold_timer，
        // 因此可验证计时分支的范围撤销而完全不创建/设置原生计时器。
        let kind = mouse_input(WM_RBUTTONDOWN, 0).unwrap();
        let outcome = state.input_at(Some(kind), None, Some(POINT), 1);
        assert!(state.queue_outcome(outcome, Some(kind)));
        assert_eq!(state.engine.next_deadline_ms(), Some(301));
    });
    STATE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let state = slot.as_mut().unwrap();
        let outcome = state.input_at(None, None, Some((POINT.0 + 1, POINT.1)), 2);
        assert!(!state.queue_outcome(outcome, None));
        assert_eq!(state.engine.pointer_process(), None);
        assert_eq!(state.engine.next_deadline_ms(), None);
        assert!(state.waitable_timer.is_none());
        assert_eq!(state.hold_timer, 0);
    });
    fixture.delivered(&[InputKind::Down(right)]);
    assert!(!mouse_at(WM_RBUTTONUP, 0, (POINT.0 + 1, POINT.1), 3));
    assert!(fixture.hits().is_empty());
    fixture.clean();
}
