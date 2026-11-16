//! 只用既有fake运行时、显式身份和鼠标事件；不安装Hook或调用原生输入。
use super::super::{Ack, Work};
use super::fake_runtime;
use crate::platform::*;
use omnitrigger_core::{Activation, ProcessIdentity, ProcessScope, WheelDirection, WindowRole};

const POINT: (i32, i32) = (20, 30);
const NEXT: (i32, i32) = (21, 30);
const X1: Button = Button::Mouse(MouseButton::X1);
const RIGHT: Button = Button::Mouse(MouseButton::Right);

// 同一前缀的全局滚轮绑定不能替受范围限制的按键绑定取得新资格。
fn bindings() -> Vec<TriggerSpec> {
    let make = |id, activation| TriggerSpec {
        id,
        held: vec![X1],
        activation,
        conditions: vec![],
        keyboard: Default::default(),
        pointer_scope: Default::default(),
        process_scope: Default::default(),
        wheel: Default::default(),
    };
    let mut scoped = make(3, Activation::Press(RIGHT));
    scoped.pointer_scope = ProcessScope {
        identity: ProcessIdentity::ShellContext,
        allow: vec![],
        deny: vec!["blocked-app".into()],
    };
    vec![
        make(1, Activation::Wheel(WheelDirection::Up)),
        make(2, Activation::Wheel(WheelDirection::Down)),
        scoped,
    ]
}

struct Fixture {
    jobs: Receiver<Work>,
    _acks: SyncSender<Ack>,
    events: Receiver<BackendEvent>,
}

impl Fixture {
    fn new() -> Self {
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
        state.command(BackendCommand::SetBindings(bindings()));
        state.apply_process_context(
            identity,
            state.foreground_epoch,
            Some("notepad"),
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
        let result = Self {
            jobs,
            _acks: acks,
            events,
        };
        result.sample(POINT, "notepad");
        result
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

    fn sample(&self, point: (i32, i32), name: &str) {
        self.state(|state| {
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
            )
        });
    }

    fn mouse(&self, message: u32, data: u32, point: (i32, i32), at: u64) -> bool {
        self.state(|state| {
            if let Some(sample) = state.pointer_context.as_mut() {
                // 仅续夹具时钟；不改旧样本的坐标或身份。
                sample.observed = Instant::now();
            }
        });
        process_mouse_packet_at(
            MousePacket {
                kind: mouse_input(message, data),
                movement: message == WM_MOUSEMOVE,
                pointer: point,
                injected: false,
                tag: 0,
            },
            at as i64,
            Some(at),
        )
    }

    fn begin_consumed_press(&self) {
        assert!(self.mouse(WM_XBUTTONDOWN, 1 << 16, POINT, 1));
        assert!(self.mouse(WM_RBUTTONDOWN, 0, POINT, 2));
        assert!(self.mouse(WM_RBUTTONUP, 0, POINT, 3));
    }

    fn hits(&self) -> Vec<u64> {
        self.events
            .try_iter()
            .filter_map(|event| match event {
                BackendEvent::Triggered { id, .. } => Some(id),
                BackendEvent::Error(error) => panic!("fake不应错误：{error}"),
                _ => None,
            })
            .collect()
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
        let _ = WHEEL_INPUT.with(|slot| slot.borrow_mut().take());
        let _ = STATE.with(|slot| slot.borrow_mut().take());
    }
}

#[test]
fn scoped_one_pixel_motion_then_same_scope_sample_keeps_the_original_press() {
    let fixture = Fixture::new();
    fixture.begin_consumed_press();
    assert!(!fixture.mouse(WM_MOUSEMOVE, 0, NEXT, 4));
    fixture.state(|state| {
        assert!(!state
            .pointer_context
            .as_ref()
            .unwrap()
            .valid_at(Some(NEXT), Instant::now()));
        assert_eq!(state.engine.pointer_process(), Some("notepad"));
    });
    fixture.sample(NEXT, "notepad");
    assert!(fixture.mouse(WM_RBUTTONDOWN, 0, NEXT, 5));
    assert!(fixture.mouse(WM_RBUTTONUP, 0, NEXT, 6));
    assert!(fixture.mouse(WM_XBUTTONUP, 1 << 16, NEXT, 7));
    assert_eq!(fixture.hits(), [3, 3]);
    fixture.clean();
}

#[test]
fn scoped_missing_or_denied_new_sample_cannot_borrow_the_retained_global_x1_owners() {
    for denied_sample in [false, true] {
        let fixture = Fixture::new();
        fixture.begin_consumed_press();
        assert!(!fixture.mouse(WM_MOUSEMOVE, 0, NEXT, 4));
        if denied_sample {
            fixture.sample(NEXT, "blocked-app");
        }
        assert!(!fixture.mouse(WM_RBUTTONDOWN, 0, NEXT, 5));
        assert!(!fixture.mouse(WM_RBUTTONUP, 0, NEXT, 6));
        fixture.sample(NEXT, "notepad");
        assert!(!fixture.mouse(WM_RBUTTONDOWN, 0, NEXT, 7));
        assert!(!fixture.mouse(WM_RBUTTONUP, 0, NEXT, 8));
        assert!(fixture.mouse(WM_XBUTTONUP, 1 << 16, NEXT, 9));
        assert_eq!(fixture.hits(), [3]);
        fixture.clean();
    }
}
