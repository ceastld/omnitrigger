use crate::{
    shortcut::SHORTCUT_TAG, BackendCommand, BackendEvent, HoldAttempt, HoldOutcome,
    HoldStartRejection, ListenerDiagnostics, PanelKey, PRIORITY_PROBE_TAG,
};
use omnitrigger_core::{
    Axis, Button, Decision, DragEvent, DragPhase, Engine, InputEvent, InputKind, MouseButton,
    Recorder, ScreenRect, TriggerSpec,
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, VecDeque},
    mem::size_of,
    sync::{
        atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender, SyncSender},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::{
        CloseHandle, HANDLE, HINSTANCE, LPARAM, LRESULT, WAIT_FAILED, WAIT_OBJECT_0, WPARAM,
    },
    System::{
        LibraryLoader::GetModuleHandleW,
        Threading::{
            CancelWaitableTimer, CreateWaitableTimerExW, GetCurrentThreadId, SetWaitableTimerEx,
            CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, INFINITE, TIMER_ALL_ACCESS,
        },
    },
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, GetKeyNameTextW, MapVirtualKeyW, INPUT, INPUT_0, INPUT_KEYBOARD,
            INPUT_MOUSE, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP,
            KEYEVENTF_SCANCODE, MAPVK_VK_TO_VSC_EX, MAPVK_VSC_TO_VK_EX, MOUSEEVENTF_HWHEEL,
            MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
            MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN,
            MOUSEEVENTF_XUP, MOUSEINPUT, VIRTUAL_KEY,
        },
        WindowsAndMessaging::{
            CallNextHookEx, GetForegroundWindow, KillTimer, MsgWaitForMultipleObjectsEx,
            PeekMessageW, PostThreadMessageW, SetTimer, SetWindowsHookExW, UnhookWindowsHookEx,
            KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLKHF_INJECTED, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT,
            MWMO_INPUTAVAILABLE, PM_NOREMOVE, PM_REMOVE, QS_ALLINPUT, WH_KEYBOARD_LL, WH_MOUSE_LL,
            WM_APP, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN,
            WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_QUIT, WM_RBUTTONDOWN,
            WM_RBUTTONUP, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_TIMER, WM_XBUTTONDOWN, WM_XBUTTONUP,
        },
    },
};

const COMMAND_MESSAGE: u32 = WM_APP + 0x26;
const REPLAY_TAG: usize = 0x4f4d_4e49;
const EVENT_CAPACITY: usize = 256;
const HOLD_POLL_MS: u64 = 25;
const MESSAGE_BATCH: usize = 64;

#[path = "replay_runtime.rs"]
mod replay_runtime;
use replay_runtime::{Effect as ReplayEffect, ReplayRuntime};

#[path = "screen_context.rs"]
mod screen_context;

thread_local! {
    // Hook 始终在安装它的消息线程运行；不在 UI 线程共享可变引擎。
    static STATE: RefCell<Option<ThreadState>> = const { RefCell::new(None) };
}

pub struct Backend {
    commands: Sender<BackendCommand>,
    thread_id: u32,
    worker: Option<JoinHandle<()>>,
    pointer: Arc<PointerSnapshot>,
}

/// 同一原子保存两个带符号坐标，读者不会拼接到两次鼠标事件。
#[derive(Default)]
struct PointerSnapshot {
    packed: AtomicU64,
    known: AtomicBool,
}

impl PointerSnapshot {
    fn store(&self, (x, y): (i32, i32)) {
        self.packed.store(
            (u64::from(x as u32) << 32) | u64::from(y as u32),
            Ordering::Release,
        );
        self.known.store(true, Ordering::Release);
    }

    fn load(&self) -> Option<(i32, i32)> {
        if !self.known.load(Ordering::Acquire) {
            return None;
        }
        let packed = self.packed.load(Ordering::Acquire);
        Some(((packed >> 32) as u32 as i32, packed as u32 as i32))
    }
}

impl Backend {
    pub fn start() -> Result<(Self, Receiver<BackendEvent>), String> {
        let (commands_tx, commands_rx) = mpsc::channel();
        let (events_tx, events_rx) = mpsc::sync_channel(EVENT_CAPACITY);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let pointer = Arc::new(PointerSnapshot::default());
        let worker_pointer = pointer.clone();
        let worker = thread::Builder::new()
            .name("omnitrigger-input".into())
            .spawn(move || run_input_thread(commands_rx, events_tx, ready_tx, worker_pointer))
            .map_err(|error| format!("无法创建输入线程：{error}"))?;
        match ready_rx.recv() {
            Ok(Ok(thread_id)) => Ok((
                Self {
                    commands: commands_tx,
                    thread_id,
                    worker: Some(worker),
                    pointer,
                },
                events_rx,
            )),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(error)
            }
            Err(_) => {
                let _ = worker.join();
                Err("输入线程在初始化完成前结束".into())
            }
        }
    }

    pub fn send(&self, command: BackendCommand) -> Result<(), String> {
        self.commands
            .send(command)
            .map_err(|_| "输入线程已经结束".to_string())?;
        // 先入队再唤醒。消息循环已在 start 返回前创建。
        unsafe { PostThreadMessageW(self.thread_id, COMMAND_MESSAGE, WPARAM(0), LPARAM(0)) }
            .map_err(|error| format!("无法唤醒输入线程：{error}"))
    }

    /// 初始化位置或最近一次物理鼠标事件的屏幕坐标。不会访问 Hook 状态或取得 UI 锁。
    pub fn pointer_position(&self) -> Option<(i32, i32)> {
        self.pointer.load()
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        let _ = self.send(BackendCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run_input_thread(
    commands: Receiver<BackendCommand>,
    events: SyncSender<BackendEvent>,
    ready: SyncSender<Result<u32, String>>,
    pointer: Arc<PointerSnapshot>,
) {
    let mut message = MSG::default();
    unsafe {
        let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
    }
    let module = match unsafe { GetModuleHandleW(None) } {
        Ok(module) => module,
        Err(error) => {
            let _ = ready.send(Err(format!("无法定位应用模块：{error}")));
            return;
        }
    };
    // 广播 WM_DISPLAYCHANGE 不会送到纯线程消息队列或 message-only 窗口。
    // 专用隐藏顶层窗口仅标记刷新，枚举屏幕始终留在 Hook 之外。
    let display_events = match screen_context::DisplayEvents::new(HINSTANCE(module.0)) {
        Ok(window) => window,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    let mut state = ThreadState::new_with_pointer(events.clone(), pointer);
    let initial_context = screen_context::collect_screens()
        .and_then(|screens| screen_context::cursor_position().map(|pointer| (screens, pointer)));
    let context_result = initial_context
        .and_then(|(screens, pointer)| state.apply_screen_context(screens, pointer, 0));
    if let Err(error) = context_result {
        let _ = ready.send(Err(format!("无法初始化显示器与指针上下文：{error}")));
        return;
    }
    if let Err(error) = state.replay.start(unsafe { GetCurrentThreadId() }) {
        let _ = ready.send(Err(error));
        return;
    }
    state.snapshot_preexisting();
    STATE.with(|slot| *slot.borrow_mut() = Some(state));
    let keyboard = match unsafe {
        SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(keyboard_hook),
            Some(HINSTANCE(module.0)),
            0,
        )
    } {
        Ok(hook) => hook,
        Err(error) => {
            STATE.with(|slot| slot.borrow_mut().take());
            let _ = ready.send(Err(format!("无法安装键盘 Hook：{error}")));
            return;
        }
    };
    let mouse = match unsafe {
        SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), Some(HINSTANCE(module.0)), 0)
    } {
        Ok(hook) => hook,
        Err(error) => {
            unsafe {
                let _ = UnhookWindowsHookEx(keyboard);
            }
            STATE.with(|slot| slot.borrow_mut().take());
            let _ = ready.send(Err(format!("无法安装鼠标 Hook：{error}")));
            return;
        }
    };
    if ready.send(Ok(unsafe { GetCurrentThreadId() })).is_ok() {
        let _ = events.try_send(BackendEvent::Ready);
        let mut shutdown_deadline = None;
        'message_loop: loop {
            if screen_context::take_refresh() && shutdown_deadline.is_none() {
                refresh_screen_context();
            }
            // 命令队列先于到期匹配处理，暂停/关闭不会被积压的普通消息推迟。
            let mut more_commands = match drain_commands(&commands, &mut shutdown_deadline) {
                CommandBatch::Stop => break,
                CommandBatch::More => true,
                CommandBatch::Done => false,
            };
            pump_replay();
            let mut message_count = 0;
            for _ in 0..MESSAGE_BATCH {
                // PeekMessage 同时派发低级 Hook 的同步回调；不在任何 RefCell 借用内泵消息。
                if !unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
                    break;
                }
                message_count += 1;
                if message.message == WM_QUIT {
                    break 'message_loop;
                }
                display_events.dispatch(&message);
                if message.message == WM_TIMER {
                    STATE.with(|slot| {
                        if let Some(state) = slot.borrow_mut().as_mut() {
                            if state.hold_timer != 0 && state.hold_timer == message.wParam.0 {
                                state.timer_due_ms = None;
                            }
                        }
                    });
                }
            }
            if screen_context::take_refresh() && shutdown_deadline.is_none() {
                refresh_screen_context();
            }
            match drain_commands(&commands, &mut shutdown_deadline) {
                CommandBatch::Stop => break,
                CommandBatch::More => more_commands = true,
                CommandBatch::Done => {}
            }
            pump_replay();
            if let Some(deadline) = shutdown_deadline {
                let drained = STATE.with(|slot| {
                    slot.borrow()
                        .as_ref()
                        .is_none_or(|state| state.owned_press.is_empty() && !state.replay.busy())
                });
                if drained || Instant::now() >= deadline {
                    break;
                }
            } else {
                advance_pending_hold();
            }
            // 忙碌队列每批仍复查 deadline；不因无限 drain 而饿死长按到期。
            if more_commands || message_count == MESSAGE_BATCH {
                continue;
            }
            let (timer_handle, hold_active) = STATE.with(|slot| {
                let slot = slot.borrow();
                let state = slot.as_ref().expect("输入状态存在");
                (
                    state
                        .waitable_timer
                        .as_ref()
                        .filter(|timer| timer.usable)
                        .map(|timer| timer.handle),
                    state.engine.next_deadline_ms().is_some()
                        || state.engine.active_drag().is_some()
                        || state.replay.busy()
                        || !state.pending_drag_cancels.is_empty(),
                )
            });
            let handles = timer_handle.map(|handle| [handle]);
            let timeout = input_wait_timeout_ms(hold_active, shutdown_deadline);
            let result = unsafe {
                MsgWaitForMultipleObjectsEx(
                    handles.as_ref().map(|handles| handles.as_slice()),
                    timeout,
                    QS_ALLINPUT,
                    MWMO_INPUTAVAILABLE,
                )
            };
            if result == WAIT_FAILED {
                let _ = events.try_send(BackendEvent::Error(format!(
                    "输入等待失败，监听已结束：{}",
                    std::io::Error::last_os_error()
                )));
                break;
            }
            if timer_handle.is_some() && result == WAIT_OBJECT_0 {
                STATE.with(|slot| {
                    if let Some(timer) = slot
                        .borrow_mut()
                        .as_mut()
                        .and_then(|state| state.waitable_timer.as_mut())
                    {
                        // one-shot 信号已被消费，提前唤醒时需要按剩余时间重新 arm。
                        timer.armed_deadline_ms = None;
                    }
                });
            }
        }
    }
    unsafe {
        let _ = UnhookWindowsHookEx(mouse);
        let _ = UnhookWindowsHookEx(keyboard);
    }
    STATE.with(|slot| slot.borrow_mut().take());
}

enum CommandBatch {
    Stop,
    More,
    Done,
}

fn drain_commands(
    commands: &Receiver<BackendCommand>,
    shutdown_deadline: &mut Option<Instant>,
) -> CommandBatch {
    for _ in 0..MESSAGE_BATCH {
        let Ok(command) = commands.try_recv() else {
            return CommandBatch::Done;
        };
        if matches!(command, BackendCommand::SetEnabled(true))
            && STATE.with(|slot| {
                slot.borrow()
                    .as_ref()
                    .is_some_and(|state| !state.screen_context_ready)
            })
        {
            refresh_screen_context();
        }
        if matches!(command, BackendCommand::Shutdown) {
            let drained = STATE.with(|slot| {
                let mut slot = slot.borrow_mut();
                let state = slot.as_mut().expect("输入状态存在");
                state.command(BackendCommand::SetEnabled(false));
                state.reschedule_hold_timer();
                state.owned_press.is_empty() && !state.replay.busy()
            });
            if drained {
                return CommandBatch::Stop;
            }
            // 维持最多两秒的释放收尾；重复关闭请求不延长等待。
            shutdown_deadline.get_or_insert_with(|| Instant::now() + Duration::from_secs(2));
        } else if shutdown_deadline.is_none() {
            STATE.with(|slot| {
                if let Some(state) = slot.borrow_mut().as_mut() {
                    state.command(command);
                    state.reschedule_hold_timer();
                }
            });
        }
    }
    CommandBatch::More
}

fn advance_pending_hold() {
    STATE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(state) = slot.as_mut() else {
            return;
        };
        state.report_overflow();
        if state.engine.active_drag().is_some() {
            // 拖拽认领后已无计时 deadline，仍按有界间隔检查前台变化。
            state.check_foreground();
        }
        if state.engine.next_deadline_ms().is_none() {
            return;
        }
        let outcome = state.advance_time_at(state.now_ms());
        state.reschedule_hold_timer();
        state.queue_outcome(outcome, None);
    });
}

fn refresh_screen_context() {
    // 原生查询可能派发消息；不能在可变借用核心状态期间执行。
    let context = screen_context::collect_screens()
        .and_then(|screens| screen_context::cursor_position().map(|pointer| (screens, pointer)));
    STATE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(state) = slot.as_mut() else {
            return;
        };
        let result = context.and_then(|(screens, pointer)| {
            state.apply_screen_context(screens, pointer, state.now_ms())
        });
        if let Err(error) = result {
            state.screen_context_failed(error);
        }
    });
}

fn pump_replay() {
    STATE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(state) = slot.as_mut() else {
            return;
        };
        let effects = state
            .replay
            .pump(unsafe { GetForegroundWindow() }.0 as usize);
        state.apply_replay_effects(effects);
    });
}

fn input_wait_timeout_ms(hold_active: bool, shutdown_deadline: Option<Instant>) -> u32 {
    if let Some(deadline) = shutdown_deadline {
        deadline
            .saturating_duration_since(Instant::now())
            .as_millis()
            .clamp(1, u128::from(HOLD_POLL_MS)) as u32
    } else if hold_active {
        HOLD_POLL_MS as u32 // 仅用于前台检查，到期由 waitable timer 精确唤醒。
    } else {
        INFINITE
    }
}

#[derive(Clone, Copy, Debug)]
struct KeyMetadata {
    scan_code: u16,
    extended: bool,
}

struct RecordingSession {
    recorder: Recorder,
    preexisting: BTreeSet<Button>,
    candidate: Option<TriggerSpec>,
}

#[derive(Clone, Copy)]
struct ActiveHoldTiming {
    started_ms: u64,
    deadline_ms: u64,
    threshold_ms: u32,
}

struct HoldDeadlineTimer {
    handle: HANDLE,
    armed_deadline_ms: Option<u64>,
    usable: bool,
    backend: &'static str,
}

impl HoldDeadlineTimer {
    fn new() -> Option<Self> {
        for (flags, backend) in [
            (CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, "高精度 waitable"),
            (0, "普通 waitable"),
        ] {
            if let Ok(handle) = unsafe {
                CreateWaitableTimerExW(
                    None,
                    windows::core::PCWSTR::null(),
                    flags,
                    TIMER_ALL_ACCESS.0,
                )
            } {
                return Some(Self {
                    handle,
                    armed_deadline_ms: None,
                    usable: true,
                    backend,
                });
            }
        }
        None
    }

    fn arm(&mut self, deadline_ms: u64, elapsed: Duration) -> windows::core::Result<()> {
        if self.armed_deadline_ms == Some(deadline_ms) {
            return Ok(());
        }
        let due = relative_due_100ns(deadline_ms, elapsed);
        unsafe { SetWaitableTimerEx(self.handle, &due, 0, None, None, None, 0) }?;
        self.armed_deadline_ms = Some(deadline_ms);
        Ok(())
    }

    fn cancel(&mut self) {
        if self.armed_deadline_ms.take().is_some() {
            let _ = unsafe { CancelWaitableTimer(self.handle) };
        }
    }
}

impl Drop for HoldDeadlineTimer {
    fn drop(&mut self) {
        // 只在输入循环停止等待以后关闭；Hook 中降级时也保留该句柄到这里。
        let _ = unsafe { CancelWaitableTimer(self.handle) };
        let _ = unsafe { CloseHandle(self.handle) };
    }
}

fn relative_due_100ns(deadline_ms: u64, elapsed: Duration) -> i64 {
    let remaining_ns = (u128::from(deadline_ms) * 1_000_000).saturating_sub(elapsed.as_nanos());
    -(remaining_ns.div_ceil(100).clamp(1, i64::MAX as u128) as i64)
}

struct ThreadState {
    engine: Engine,
    events: SyncSender<BackendEvent>,
    recording: Option<RecordingSession>,
    recording_consumed: BTreeSet<Button>,
    failure_draining: BTreeSet<Button>,
    owned_press: BTreeSet<Button>,
    physical_held: BTreeSet<Button>,
    key_metadata: BTreeMap<Button, KeyMetadata>,
    foreground: usize,
    overflow_reported: bool,
    counters: ListenerDiagnostics,
    last_keyboard: Option<Instant>,
    last_mouse: Option<Instant>,
    clock_origin: Instant,
    hold_timer: usize,
    timer_due_ms: Option<u64>,
    active_hold: Option<ActiveHoldTiming>,
    waitable_timer: Option<HoldDeadlineTimer>,
    hold_specs: Vec<TriggerSpec>,
    enabled: bool,
    replay: ReplayRuntime,
    /// 已登记的可见面板窗口，0 表示没有。
    panel_window: usize,
    /// 面板导航消费的按下；对应释放同样消费，即使面板已经收起。
    panel_consumed: BTreeSet<Button>,
    /// 方向键选择之后才接管 Enter，未选择时不吞掉前台窗口的回车。
    panel_selecting: bool,
    pointer: Arc<PointerSnapshot>,
    /// 通知拥塞时保留终止事件，监听暂停后仍定时重试，避免 UI 留下活动拖拽。
    pending_drag_cancels: VecDeque<(DragEvent, isize)>,
    screens: Vec<ScreenRect>,
    screen_context_ready: bool,
}

impl ThreadState {
    #[cfg(test)]
    fn new(events: SyncSender<BackendEvent>) -> Self {
        let mut state = Self::new_with_pointer(events, Arc::new(PointerSnapshot::default()));
        // 事件序列测试由夹具提供上下文，保留无区域绑定的既有测试行为。
        state.screen_context_ready = true;
        state
    }

    fn new_with_pointer(events: SyncSender<BackendEvent>, pointer: Arc<PointerSnapshot>) -> Self {
        let waitable_timer = HoldDeadlineTimer::new();
        let counters = ListenerDiagnostics {
            hold_timer_backend: waitable_timer
                .as_ref()
                .map_or("SetTimer 回退", |timer| timer.backend),
            ..ListenerDiagnostics::default()
        };
        Self {
            engine: Engine::new(Vec::new()).expect("空绑定列表始终有效"),
            events,
            recording: None,
            recording_consumed: BTreeSet::new(),
            failure_draining: BTreeSet::new(),
            owned_press: BTreeSet::new(),
            physical_held: BTreeSet::new(),
            key_metadata: BTreeMap::new(),
            foreground: unsafe { GetForegroundWindow() }.0 as usize,
            overflow_reported: false,
            counters,
            last_keyboard: None,
            last_mouse: None,
            clock_origin: Instant::now(),
            hold_timer: 0,
            timer_due_ms: None,
            active_hold: None,
            waitable_timer,
            hold_specs: Vec::new(),
            enabled: true,
            replay: ReplayRuntime::new(),
            panel_window: 0,
            panel_consumed: BTreeSet::new(),
            panel_selecting: false,
            pointer,
            pending_drag_cancels: VecDeque::with_capacity(2),
            screens: Vec::new(),
            screen_context_ready: false,
        }
    }

    fn snapshot_preexisting(&mut self) {
        // 启动前已有的按压已交付给目标窗口，不能将它们的自动重复误当新前缀。
        // 此处运行在安装 Hook 之前，不能放进 Hook 回调（回调早于系统异步状态更新）。
        for key in 1..=254 {
            if matches!(key, 0x10..=0x12) || unsafe { GetAsyncKeyState(key) } >= 0 {
                continue;
            }
            let button = match key {
                0x01 => Button::Mouse(MouseButton::Left),
                0x02 => Button::Mouse(MouseButton::Right),
                0x04 => Button::Mouse(MouseButton::Middle),
                0x05 => Button::Mouse(MouseButton::X1),
                0x06 => Button::Mouse(MouseButton::X2),
                _ => Button::Key(key as u16),
            };
            self.physical_held.insert(button);
            self.replay.new_press(button);
            self.replay
                .passed(InputKind::Down(button), &self.key_metadata);
            self.engine.process_at(
                InputEvent {
                    kind: InputKind::Down(button),
                    injected: false,
                },
                self.now_ms(),
            );
        }
    }

    fn apply_screen_context(
        &mut self,
        screens: Vec<ScreenRect>,
        pointer: (i32, i32),
        now_ms: u64,
    ) -> Result<(), String> {
        let changed = self.screens != screens;
        let decision = self
            .engine
            .set_screens(screens.clone())
            .map_err(|error| error.to_string())?;
        if changed {
            self.cancel_queued_replay();
        }
        self.screens = screens;
        self.screen_context_ready = true;
        self.observe_hold(now_ms, &decision, HoldOutcome::Configuration);
        let outcome = self.finish_decision(decision, None);
        self.queue_outcome(outcome, None);
        self.pointer.store(pointer);
        let movement = self.engine.pointer_moved(pointer, now_ms);
        self.observe_hold(now_ms, &movement, HoldOutcome::Configuration);
        let outcome = self.finish_decision(movement, None);
        self.queue_outcome(outcome, None);
        self.reschedule_hold_timer();
        Ok(())
    }

    fn screen_context_failed(&mut self, error: String) {
        self.screen_context_ready = false;
        let cancelled = self.engine.set_enabled(false);
        self.enabled = false;
        self.cancel_queued_replay();
        self.forward_cancelled_drags(cancelled);
        self.cancel_recording();
        if let Ok(cancelled) = self.engine.set_screens(Vec::new()) {
            self.forward_cancelled_drags(cancelled);
        }
        self.screens.clear();
        self.observe_hold(
            self.now_ms(),
            &Decision::default(),
            HoldOutcome::Configuration,
        );
        self.reschedule_hold_timer();
        self.notify(BackendEvent::Error(format!(
            "显示器上下文更新失败，监听已暂停；请重新启用或重建监听：{error}"
        )));
    }

    fn notify(&mut self, event: BackendEvent) {
        if self.events.try_send(event).is_err() {
            self.overflow_reported = true;
        }
    }

    fn notify_action(&mut self, event: BackendEvent) {
        if !self.enabled
            && !matches!(&event, BackendEvent::Drag { event, .. } if event.phase == DragPhase::Cancelled)
        {
            return;
        }
        // Ended 产生时核心已移除会话，交付屏障拒绝入队时仍需通知界面取消。
        let ended = match &event {
            BackendEvent::Drag { event, foreground } if event.phase == DragPhase::Ended => {
                Some((*event, *foreground))
            }
            _ => None,
        };
        match self.replay.defer_action(event, self.foreground) {
            Ok(Some(event)) => self.deliver_action(event),
            Ok(None) => {}
            Err(()) => {
                let affected = self.replay.abandon(&[]);
                self.replay_failed_async(
                    affected,
                    "输入交付队列已满，待执行动作已取消，监听已暂停。".into(),
                );
                if let Some((mut event, foreground)) = ended {
                    event.phase = DragPhase::Cancelled;
                    self.deliver_action(BackendEvent::Drag { event, foreground });
                }
            }
        }
    }

    fn retain_drag_cancel(&mut self, mut event: DragEvent, foreground: isize) {
        event.phase = DragPhase::Cancelled;
        if let Some(pending) = self
            .pending_drag_cancels
            .iter_mut()
            .find(|(pending, _)| pending.session == event.session)
        {
            *pending = (event, foreground);
        } else {
            // 首次交付失败即暂停匹配；最多保留失败通知对应会话和当前核心活动会话。
            self.pending_drag_cancels.push_back((event, foreground));
        }
    }

    fn deliver_action(&mut self, event: BackendEvent) {
        if !self.enabled
            && !matches!(&event, BackendEvent::Drag { event, .. } if event.phase == DragPhase::Cancelled)
        {
            return;
        }
        if let Err(error) = self.events.try_send(event) {
            self.overflow_reported = true;
            let (mpsc::TrySendError::Full(event) | mpsc::TrySendError::Disconnected(event)) = error;
            if let BackendEvent::Drag { event, foreground } = event {
                self.retain_drag_cancel(event, foreground);
                let cancelled = self.engine.set_enabled(false);
                self.enabled = false;
                self.cancel_queued_replay();
                for event in cancelled.drags {
                    self.retain_drag_cancel(event, self.foreground as isize);
                }
                self.observe_hold(self.now_ms(), &Decision::default(), HoldOutcome::Recovery);
            }
        }
    }

    fn forward_cancelled_drags(&mut self, decision: Decision) {
        for event in decision.drags {
            self.deliver_action(BackendEvent::Drag {
                event,
                foreground: self.foreground as isize,
            });
        }
    }

    fn note_callback(&mut self, keyboard: bool, injected: bool, tag: usize) {
        let now = Instant::now();
        if keyboard {
            self.counters.keyboard_events = self.counters.keyboard_events.saturating_add(1);
            self.last_keyboard = Some(now);
        } else {
            self.counters.mouse_events = self.counters.mouse_events.saturating_add(1);
            self.last_mouse = Some(now);
        }
        if injected {
            match tag {
                REPLAY_TAG => {
                    self.counters.self_replayed = self.counters.self_replayed.saturating_add(1)
                }
                SHORTCUT_TAG => {
                    self.counters.self_shortcuts = self.counters.self_shortcuts.saturating_add(1)
                }
                PRIORITY_PROBE_TAG => {
                    self.counters.probe_events = self.counters.probe_events.saturating_add(1)
                }
                _ => {
                    self.counters.external_injected =
                        self.counters.external_injected.saturating_add(1);
                }
            }
        }
    }

    fn diagnostics(&self) -> ListenerDiagnostics {
        let mut snapshot = self.counters.clone();
        snapshot.held = self.physical_held.iter().copied().collect();
        snapshot.owned = self.owned_press.iter().copied().collect();
        snapshot.screen_context_ready = self.screen_context_ready;
        snapshot.screen_count = self.screens.len();
        let age_ms = |time: Instant| time.elapsed().as_millis().min(u64::MAX as u128) as u64;
        snapshot.last_keyboard_ms = self.last_keyboard.map(age_ms);
        snapshot.last_mouse_ms = self.last_mouse.map(age_ms);
        snapshot
    }

    fn command(&mut self, command: BackendCommand) {
        let cancelled_by = match &command {
            BackendCommand::SetBindings(_) => HoldOutcome::Configuration,
            BackendCommand::StartRecording | BackendCommand::CancelRecording => {
                HoldOutcome::Recording
            }
            BackendCommand::SetEnabled(_) | BackendCommand::Shutdown => HoldOutcome::Paused,
            BackendCommand::GetDiagnostics | BackendCommand::SetPanelWindow(_) => {
                HoldOutcome::Recovery
            }
            BackendCommand::CancelDrag { .. } => HoldOutcome::Recovery,
        };
        match command {
            BackendCommand::SetBindings(bindings) => {
                if self.recording.is_some() {
                    self.notify(BackendEvent::Error("请先结束录制，再更新触发器".into()));
                } else {
                    let hold_specs = bindings
                        .iter()
                        .filter(|binding| {
                            matches!(
                                binding.activation,
                                omnitrigger_core::Activation::Hold { .. }
                            )
                        })
                        .cloned()
                        .collect();
                    match self.engine.replace_bindings(bindings) {
                        Ok(cancelled) => {
                            self.hold_specs = hold_specs;
                            self.cancel_queued_replay();
                            self.forward_cancelled_drags(cancelled);
                        }
                        Err(error) => self.notify(BackendEvent::Error(error.to_string())),
                    }
                }
            }
            BackendCommand::StartRecording => {
                if !self.recording_consumed.is_empty() {
                    self.notify(BackendEvent::Error("请先松开上次录制中按住的按键".into()));
                    return;
                }
                let cancelled = self.engine.cancel();
                self.cancel_queued_replay();
                self.forward_cancelled_drags(cancelled);
                self.recording = Some(RecordingSession {
                    recorder: Recorder::new(),
                    preexisting: self.physical_held.clone(),
                    candidate: None,
                });
            }
            BackendCommand::CancelRecording => self.cancel_recording(),
            BackendCommand::SetEnabled(enabled) => {
                if enabled && !self.screen_context_ready {
                    self.notify(BackendEvent::Error(
                        "显示器上下文不可用，无法重新启用监听；请重试或重建监听。".into(),
                    ));
                    return;
                }
                self.cancel_recording();
                let cancelled = self.engine.set_enabled(enabled);
                self.enabled = enabled;
                if !enabled {
                    self.cancel_queued_replay();
                }
                self.forward_cancelled_drags(cancelled);
                if enabled {
                    self.replay.retry_cleanup();
                }
            }
            BackendCommand::GetDiagnostics => {
                self.notify(BackendEvent::Diagnostics(Box::new(self.diagnostics())));
            }
            BackendCommand::SetPanelWindow(window) => {
                self.panel_window = window.map_or(0, |hwnd| hwnd as usize);
                self.panel_selecting = false;
            }
            BackendCommand::CancelDrag { session } => {
                let cancelled = self.engine.cancel_drag(session);
                if !cancelled.drags.is_empty() {
                    self.cancel_queued_replay();
                    self.forward_cancelled_drags(cancelled);
                }
            }
            BackendCommand::Shutdown => {}
        }
        self.observe_hold(self.now_ms(), &Decision::default(), cancelled_by);
    }

    fn cancel_recording(&mut self) {
        if self.recording.take().is_some() {
            let cancelled = self.engine.cancel();
            self.forward_cancelled_drags(cancelled);
            self.observe_hold(self.now_ms(), &Decision::default(), HoldOutcome::Recording);
            self.notify(BackendEvent::RecordingCancelled);
        }
    }

    fn cancel_queued_replay(&mut self) {
        if self.replay.busy() {
            let affected = self.replay.abandon(&[]);
            for button in affected {
                if self.physical_held.contains(&button) {
                    self.failure_draining.insert(button);
                    self.owned_press.insert(button);
                }
            }
        }
    }

    fn observe_hold(&mut self, now_ms: u64, decision: &Decision, cancelled_by: HoldOutcome) {
        let deadline = self.engine.next_hold_deadline_ms();
        if let Some(active) = self.active_hold {
            if deadline != Some(active.deadline_ms) {
                self.active_hold = None;
                let elapsed_ms = now_ms.saturating_sub(active.started_ms);
                self.counters.last_hold_elapsed_ms = Some(elapsed_ms);
                self.counters.last_hold_threshold_ms = Some(active.threshold_ms);
                self.counters.last_hold_lateness_ms = None;
                if now_ms >= active.deadline_ms {
                    if let Some(&id) = decision.triggered.first() {
                        self.counters.hold_triggered =
                            self.counters.hold_triggered.saturating_add(1);
                        self.counters.last_hold_lateness_ms =
                            Some(elapsed_ms.saturating_sub(u64::from(active.threshold_ms)));
                        self.record_hold_attempt(
                            HoldOutcome::Matched,
                            elapsed_ms,
                            active.threshold_ms,
                        );
                        self.notify_action(BackendEvent::HoldTiming {
                            id,
                            elapsed_ms,
                            threshold_ms: active.threshold_ms,
                            matched_at: Instant::now(),
                        });
                        return;
                    }
                }
                let count = match cancelled_by {
                    HoldOutcome::Movement => &mut self.counters.hold_cancelled_movement,
                    HoldOutcome::FocusChanged => &mut self.counters.hold_cancelled_focus,
                    _ => &mut self.counters.hold_cancelled_other,
                };
                *count = count.saturating_add(1);
                self.record_hold_attempt(cancelled_by, elapsed_ms, active.threshold_ms);
            }
        }
        if self.active_hold.is_none() {
            if let Some(deadline_ms) = deadline {
                self.active_hold = Some(ActiveHoldTiming {
                    started_ms: now_ms,
                    deadline_ms,
                    threshold_ms: deadline_ms.saturating_sub(now_ms).min(u64::from(u32::MAX))
                        as u32,
                });
                self.counters.hold_started = self.counters.hold_started.saturating_add(1);
            }
        }
    }

    fn record_hold_attempt(&mut self, outcome: HoldOutcome, elapsed_ms: u64, threshold_ms: u32) {
        if self.counters.recent_hold_attempts.len() == 8 {
            self.counters.recent_hold_attempts.remove(0);
        }
        self.counters.recent_hold_attempts.push(HoldAttempt {
            outcome,
            elapsed_ms,
            threshold_ms,
        });
    }

    fn note_hold_edge(&mut self, kind: InputKind) -> Option<HoldStartRejection> {
        let button = match kind {
            InputKind::Down(button) | InputKind::Up(button) => button,
            InputKind::Wheel { .. } => return None,
        };
        let is_terminal = |spec: &TriggerSpec| {
            matches!(
                spec.activation,
                omnitrigger_core::Activation::Hold { button: terminal, .. } if terminal == button
            )
        };
        if !self.hold_specs.iter().any(is_terminal) {
            return None;
        }
        if matches!(kind, InputKind::Up(_)) {
            if self.physical_held.contains(&button) {
                self.counters.hold_terminal_ups = self.counters.hold_terminal_ups.saturating_add(1);
            }
            return None;
        }
        if self.physical_held.contains(&button) {
            if matches!(button, Button::Mouse(_)) {
                self.counters.hold_terminal_duplicate_downs = self
                    .counters
                    .hold_terminal_duplicate_downs
                    .saturating_add(1);
                self.counters.last_hold_start_rejection = Some(HoldStartRejection::ExistingPress);
            }
            return None;
        }
        self.counters.hold_terminal_downs = self.counters.hold_terminal_downs.saturating_add(1);
        Some(if !self.enabled {
            HoldStartRejection::Paused
        } else if self.recording.is_some() {
            HoldStartRejection::Recording
        } else if self
            .hold_specs
            .iter()
            .filter(|spec| is_terminal(spec))
            .all(|spec| {
                self.physical_held
                    .iter()
                    .any(|pressed| !spec.held.contains(pressed))
            })
        {
            HoldStartRejection::ExtraHeld
        } else {
            HoldStartRejection::Unmatched
        })
    }

    fn now_ms(&self) -> u64 {
        self.clock_origin
            .elapsed()
            .as_millis()
            .min(u64::MAX as u128) as u64
    }

    fn kill_hold_timer(&mut self) {
        if let Some(timer) = self.waitable_timer.as_mut() {
            timer.cancel();
        }
        if self.hold_timer != 0 {
            unsafe {
                let _ = KillTimer(None, self.hold_timer);
            }
            self.hold_timer = 0;
        }
        self.timer_due_ms = None;
    }

    fn reschedule_hold_timer(&mut self) {
        let Some(deadline) = self.engine.next_deadline_ms() else {
            self.kill_hold_timer();
            return;
        };
        if let Some(timer) = self.waitable_timer.as_mut().filter(|timer| timer.usable) {
            match timer.arm(deadline, self.clock_origin.elapsed()) {
                Ok(()) => return,
                Err(_) => {
                    // 句柄可能正被消息感知等待持有，只取消、不在 Hook 内关闭。
                    timer.cancel();
                    timer.usable = false;
                    self.counters.hold_timer_backend = "SetTimer 回退（waitable 设置失败）";
                }
            }
        }
        let now = self.now_ms();
        let delay = hold_timer_delay_ms(now, deadline);
        let due = now.saturating_add(u64::from(delay));
        // 已有更早的唤醒可以复用；避免鼠标移动不断重置计时器。
        if self.hold_timer != 0 && self.timer_due_ms.is_some_and(|existing| existing <= due) {
            return;
        }
        let timer = unsafe { SetTimer(None, self.hold_timer, delay, None) };
        if timer == 0 {
            let error = std::io::Error::last_os_error().to_string();
            self.hold_timer_failed(error);
            return;
        }
        self.hold_timer = timer;
        self.timer_due_ms = Some(due);
    }

    fn hold_timer_failed(&mut self, error: String) {
        self.kill_hold_timer();
        // 计时失效不能让待定按压永远悬挂，按原取消协议吞掉后续释放。
        let cancelled = self.engine.set_enabled(false);
        self.enabled = false;
        self.cancel_queued_replay();
        self.forward_cancelled_drags(cancelled);
        self.observe_hold(self.now_ms(), &Decision::default(), HoldOutcome::Recovery);
        self.notify(BackendEvent::Error(format!(
            "无法启动长按计时器，监听已暂停；请松开按键后重新启用：{error}"
        )));
    }

    fn check_foreground(&mut self) {
        let foreground = unsafe { GetForegroundWindow() }.0 as usize;
        self.update_foreground(foreground);
    }

    fn update_foreground(&mut self, foreground: usize) {
        if foreground != self.foreground {
            self.counters.foreground_cancellations =
                self.counters.foreground_cancellations.saturating_add(1);
            // 不把旧窗口的缓冲输入补发给新窗口。
            let cancelled = self.engine.cancel();
            self.cancel_queued_replay();
            self.forward_cancelled_drags(cancelled);
            self.observe_hold(
                self.now_ms(),
                &Decision::default(),
                HoldOutcome::FocusChanged,
            );
            self.cancel_recording();
            self.foreground = foreground;
        }
    }

    fn report_overflow(&mut self) {
        while let Some((event, foreground)) = self.pending_drag_cancels.pop_front() {
            if self
                .events
                .try_send(BackendEvent::Drag { event, foreground })
                .is_err()
            {
                self.pending_drag_cancels.push_front((event, foreground));
                break;
            }
        }
        if self.overflow_reported {
            self.overflow_reported = self
                .events
                .try_send(BackendEvent::Error(
                    if self.enabled {
                        "提示队列已满，部分触发通知被丢弃"
                    } else {
                        "提示队列已满，监听已暂停；拖拽已取消，请松开按键后重新启用。"
                    }
                    .into(),
                ))
                .is_err();
        }
    }

    fn advance_time_at(&mut self, now_ms: u64) -> HookOutcome {
        // 没有后续输入时也先确认前台，再作到期决策，不能向新窗口触发旧按压。
        self.check_foreground();
        self.report_overflow();
        let decision = if self.recording.is_none() {
            self.engine.advance_time(now_ms)
        } else {
            Decision::default()
        };
        self.observe_hold(now_ms, &decision, HoldOutcome::Recovery);
        self.finish_decision(decision, None)
    }

    #[cfg(test)]
    fn input(&mut self, kind: InputKind, metadata: Option<KeyMetadata>) -> HookOutcome {
        self.input_at(Some(kind), metadata, None, self.now_ms())
    }

    fn input_at(
        &mut self,
        kind: Option<InputKind>,
        metadata: Option<KeyMetadata>,
        pointer: Option<(i32, i32)>,
        now_ms: u64,
    ) -> HookOutcome {
        if let Some(position) = pointer {
            self.pointer.store(position);
        }
        self.check_foreground();
        self.report_overflow();
        let mut movement = pointer
            .map(|position| self.engine.pointer_moved(position, now_ms))
            .unwrap_or_default();
        self.observe_hold(now_ms, &movement, HoldOutcome::Movement);
        let Some(kind) = kind else {
            return self.finish_decision(movement, None);
        };
        // 在更新按住集合之前判定，确保“单独按下”只看此前已经按住的键。
        let panel_taken = self.panel_takes(kind);
        match kind {
            InputKind::Down(Button::Mouse(_)) => {
                if let Some((x, y)) = pointer {
                    self.notify(BackendEvent::PointerPressed { x, y });
                }
            }
            InputKind::Down(Button::Key(0x1b))
                if !panel_taken && !self.physical_held.contains(&Button::Key(0x1b)) =>
            {
                self.notify(BackendEvent::EscapePressed);
            }
            _ => {}
        }
        let start_rejection = self.note_hold_edge(kind);
        match kind {
            InputKind::Down(button) => {
                if !self.physical_held.contains(&button) {
                    self.replay.new_press(button);
                }
                self.physical_held.insert(button);
                if let Some(metadata) = metadata {
                    self.key_metadata.insert(button, metadata);
                }
            }
            InputKind::Up(button) => {
                self.physical_held.remove(&button);
            }
            InputKind::Wheel { .. } => {}
        }
        let event = InputEvent {
            kind,
            injected: false,
        };
        let failed_button = match kind {
            InputKind::Down(button) | InputKind::Up(button)
                if self.failure_draining.contains(&button) =>
            {
                Some(button)
            }
            _ => None,
        };
        let mut decision = if let Some(button) = failed_button {
            if matches!(kind, InputKind::Up(_)) {
                self.failure_draining.remove(&button);
                self.engine.process_at(event, now_ms);
            }
            Decision {
                consume: true,
                ..Decision::default()
            }
        } else if panel_taken {
            self.panel_input(kind)
        } else if self.recording.is_some() {
            self.record_input_at(event, now_ms)
        } else if let InputKind::Down(button) | InputKind::Up(button) = kind {
            if self.recording_consumed.contains(&button) {
                if matches!(kind, InputKind::Up(_)) {
                    self.recording_consumed.remove(&button);
                }
                Decision {
                    consume: true,
                    ..Decision::default()
                }
            } else {
                self.engine.process_at(event, now_ms)
            }
        } else {
            self.engine.process_at(event, now_ms)
        };
        let cancelled_by = match kind {
            InputKind::Down(_) => HoldOutcome::OtherButton,
            InputKind::Up(_) => HoldOutcome::ShortRelease,
            InputKind::Wheel { .. } => HoldOutcome::Wheel,
        };
        self.observe_hold(now_ms, &decision, cancelled_by);
        if let Some(reason) = start_rejection {
            if self.engine.next_hold_deadline_ms().is_none() {
                self.counters.hold_not_started = self.counters.hold_not_started.saturating_add(1);
                self.counters.last_hold_start_rejection = Some(reason);
            }
        }
        movement.replay.append(&mut decision.replay);
        decision.replay = movement.replay;
        movement.triggered.append(&mut decision.triggered);
        decision.triggered = movement.triggered;
        movement.drags.append(&mut decision.drags);
        decision.drags = movement.drags;
        self.finish_decision(decision, Some(kind))
    }

    fn finish_decision(
        &mut self,
        mut decision: Decision,
        current: Option<InputKind>,
    ) -> HookOutcome {
        for event in decision.drags.drain(..) {
            let cancelled = event.phase == DragPhase::Cancelled;
            let notification = BackendEvent::Drag {
                event,
                foreground: self.foreground as isize,
            };
            if cancelled {
                self.cancel_queued_replay();
                self.deliver_action(notification);
            } else {
                self.notify_action(notification);
            }
        }
        for id in decision.triggered.drain(..) {
            self.notify_action(BackendEvent::Triggered {
                id,
                pointer: self.pointer.load(),
            });
        }
        if !decision.consume && !decision.replay.is_empty() {
            // Hook 回调内不能假定补发已先于原事件交付；把当前事件也放进同一批
            // SendInput 并消费原事件，保证“前缀 → 后继输入”的顺序。
            if let Some(kind) = current {
                decision.replay.push(kind);
                decision.consume = true;
            }
        }
        match current {
            Some(InputKind::Down(button)) if decision.consume => {
                self.owned_press.insert(button);
            }
            Some(InputKind::Down(button) | InputKind::Up(button)) => {
                self.owned_press.remove(&button);
            }
            Some(InputKind::Wheel { .. }) | None => {}
        }
        let replay = decision
            .replay
            .iter()
            .map(|kind| replay_input(*kind, &self.key_metadata))
            .collect();
        HookOutcome {
            consume: decision.consume,
            replay,
            replay_kinds: decision.replay,
        }
    }

    /// 面板可见、未获前台且监听未暂停时才接管导航键；录制期间不接管。
    fn panel_navigation_active(&self) -> bool {
        self.enabled
            && self.recording.is_none()
            && self.panel_window != 0
            && self.panel_window != self.foreground
            && panel_window_visible(self.panel_window)
    }

    fn panel_takes(&self, kind: InputKind) -> bool {
        match kind {
            // 已接管的按下，其自动重复和释放都继续消费，目标窗口不会收到不成对的事件。
            InputKind::Down(button) | InputKind::Up(button)
                if self.panel_consumed.contains(&button) =>
            {
                true
            }
            // 只接管单独按下的键；带修饰键、前缀或按住鼠标时仍交给组合匹配。
            InputKind::Down(button) => {
                self.physical_held.is_empty()
                    && panel_key(button, self.panel_selecting).is_some()
                    && self.panel_navigation_active()
            }
            InputKind::Up(_) | InputKind::Wheel { .. } => false,
        }
    }

    fn panel_input(&mut self, kind: InputKind) -> Decision {
        match kind {
            InputKind::Down(button) => {
                let repeat = !self.panel_consumed.insert(button);
                match panel_key(button, true) {
                    Some(
                        key @ (PanelKey::Left | PanelKey::Right | PanelKey::Up | PanelKey::Down),
                    ) => {
                        self.panel_selecting = true;
                        self.notify(BackendEvent::PanelKey(key));
                    }
                    Some(_) if repeat => {}
                    Some(PanelKey::Escape) => {
                        self.panel_selecting = false;
                        self.notify(BackendEvent::PanelKey(PanelKey::Escape));
                    }
                    // 会运行动作的按键与组合命中一样，排在更早的输入交付之后。
                    Some(key) => self.notify_action(BackendEvent::PanelKey(key)),
                    None => {}
                }
            }
            InputKind::Up(button) => {
                self.panel_consumed.remove(&button);
            }
            InputKind::Wheel { .. } => {}
        }
        Decision {
            consume: true,
            ..Decision::default()
        }
    }

    #[cfg(test)]
    fn record_input(&mut self, event: InputEvent) -> Decision {
        self.record_input_at(event, self.now_ms())
    }

    fn record_input_at(&mut self, event: InputEvent, now_ms: u64) -> Decision {
        let session = self.recording.as_mut().expect("录制会话存在");
        if let InputKind::Down(button) | InputKind::Up(button) = event.kind {
            if session.preexisting.contains(&button) {
                if matches!(event.kind, InputKind::Up(_)) {
                    session.preexisting.remove(&button);
                }
                return self.engine.process_at(event, now_ms);
            }
        }
        if event.kind == InputKind::Down(Button::Key(0x1b)) {
            self.recording_consumed.insert(Button::Key(0x1b));
            self.cancel_recording();
            return Decision {
                consume: true,
                ..Decision::default()
            };
        }
        let (consume, changed) = match event.kind {
            InputKind::Down(button) => (true, self.recording_consumed.insert(button)),
            InputKind::Up(button) => {
                let removed = self.recording_consumed.remove(&button);
                (removed, removed)
            }
            InputKind::Wheel { .. } => (true, false),
        };
        if !consume {
            return self.engine.process_at(event, now_ms);
        }
        if session.candidate.is_none() {
            session.candidate = session.recorder.process(event);
        }
        if self.recording_consumed.is_empty() {
            if let Some(spec) = session.candidate.take() {
                self.recording = None;
                self.notify(BackendEvent::Recorded(spec));
            }
        }
        // 只在按下集合变化时通知界面，键盘 repeat 不产生事件。
        if changed && self.recording.is_some() {
            let held = self.recording_consumed.iter().copied().collect();
            self.notify(BackendEvent::RecordingProgress(held));
        }
        Decision {
            consume: true,
            ..Decision::default()
        }
    }

    fn replay_failed(&mut self, kinds: &[InputKind]) {
        self.counters.replay_failures = self.counters.replay_failures.saturating_add(1);
        let cancelled = self.engine.set_enabled(false);
        self.enabled = false;
        self.forward_cancelled_drags(cancelled);
        self.observe_hold(self.now_ms(), &Decision::default(), HoldOutcome::Recovery);
        // Engine 原本预期回放成功，故标为 Passed。实际未发送/已恢复释放的按压
        // 仍需吞掉物理 repeat/up，不能把没有对应 Down 的 Up 传给目标窗口。
        for kind in kinds {
            match kind {
                InputKind::Down(button) if self.physical_held.contains(button) => {
                    self.failure_draining.insert(*button);
                    self.owned_press.insert(*button);
                }
                _ => {}
            }
        }
    }

    fn replay_failed_async(&mut self, affected: BTreeSet<Button>, message: String) {
        let kinds: Vec<_> = affected.into_iter().map(InputKind::Down).collect();
        self.replay_failed(&kinds);
        self.reschedule_hold_timer();
        self.notify(BackendEvent::Error(message));
    }

    fn apply_replay_effects(&mut self, effects: Vec<ReplayEffect>) {
        for effect in effects {
            match effect {
                ReplayEffect::Action(event) => self.deliver_action(event),
                ReplayEffect::Timing(elapsed) => {
                    update_hook_timing(&mut self.counters, HookTiming::Replay, elapsed)
                }
                ReplayEffect::Delivered(tags) => {
                    for tag in tags {
                        if let InputKind::Down(button) | InputKind::Up(button) = tag.kind {
                            if self.replay.epoch(button) == tag.epoch
                                && !self.failure_draining.contains(&button)
                            {
                                self.owned_press.remove(&button);
                            }
                        }
                    }
                }
                ReplayEffect::Failed { affected, message } => {
                    self.replay_failed_async(affected, message)
                }
            }
        }
    }

    fn queue_outcome(&mut self, mut outcome: HookOutcome, current: Option<InputKind>) -> bool {
        if self.replay.is_fatal() {
            if let Some(InputKind::Up(button)) = current {
                if self.replay.can_pass_release(button) {
                    outcome.consume = false;
                    self.failure_draining.remove(&button);
                    self.owned_press.remove(&button);
                }
            }
            // 清理失败时保留暂停，但新普通输入不能被永久困在失效交付队列里。
            if !outcome.consume {
                if let Some(kind) = current {
                    self.replay.passed(kind, &self.key_metadata);
                }
            }
            return outcome.consume;
        }
        if self.replay.busy() && !outcome.consume {
            if let Some(kind) = current {
                outcome.replay.push(replay_input(kind, &self.key_metadata));
                outcome.replay_kinds.push(kind);
                outcome.consume = true;
                if let InputKind::Down(button) = kind {
                    self.owned_press.insert(button);
                }
            }
        }
        if !outcome.replay.is_empty() {
            if let Err(affected) = self.replay.enqueue(
                outcome.replay,
                outcome.replay_kinds,
                self.foreground,
                &self.key_metadata,
            ) {
                self.replay_failed_async(
                    affected,
                    "输入交付队列已满，监听已暂停；释放收尾将尽力完成。".into(),
                );
            }
        } else if !outcome.consume {
            if let Some(kind) = current {
                self.replay.passed(kind, &self.key_metadata);
            }
        }
        outcome.consume
    }

    #[cfg(test)]
    fn replay_succeeded(&mut self, kinds: &[InputKind]) {
        for kind in kinds {
            if let InputKind::Down(button) | InputKind::Up(button) = kind {
                self.owned_press.remove(button);
            }
        }
    }
}

impl Drop for ThreadState {
    fn drop(&mut self) {
        self.kill_hold_timer();
    }
}

fn hold_timer_delay_ms(now_ms: u64, deadline_ms: u64) -> u32 {
    deadline_ms.saturating_sub(now_ms).clamp(1, HOLD_POLL_MS) as u32
}

struct HookOutcome {
    consume: bool,
    replay: Vec<INPUT>,
    replay_kinds: Vec<InputKind>,
}

fn process_input(kind: InputKind, metadata: Option<KeyMetadata>) -> bool {
    process_normalized_input(Some(kind), metadata, None)
}

fn process_normalized_input(
    kind: Option<InputKind>,
    metadata: Option<KeyMetadata>,
    pointer: Option<(i32, i32)>,
) -> bool {
    STATE
        .with(|slot| {
            let Ok(mut slot) = slot.try_borrow_mut() else {
                return None;
            };
            slot.as_mut().map(|state| {
                let outcome = state.input_at(kind, metadata, pointer, state.now_ms());
                state.reschedule_hold_timer();
                state.queue_outcome(outcome, kind)
            })
        })
        .unwrap_or(false)
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }
    let started = Instant::now();
    {
        let native = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        note_callback(
            true,
            native.flags.contains(LLKHF_INJECTED),
            native.dwExtraInfo,
        );
        if !native.flags.contains(LLKHF_INJECTED)
            && native.dwExtraInfo != REPLAY_TAG
            && native.dwExtraInfo != SHORTCUT_TAG
            && native.dwExtraInfo != PRIORITY_PROBE_TAG
        {
            let down = match wparam.0 as u32 {
                WM_KEYDOWN | WM_SYSKEYDOWN => Some(true),
                WM_KEYUP | WM_SYSKEYUP => Some(false),
                _ => None,
            };
            if let Some(down) = down {
                let button = Button::Key(normalized_key(native));
                let kind = if down {
                    InputKind::Down(button)
                } else {
                    InputKind::Up(button)
                };
                let metadata = KeyMetadata {
                    scan_code: native.scanCode as u16,
                    extended: native.flags.contains(LLKHF_EXTENDED),
                };
                if process_input(kind, Some(metadata)) {
                    record_hook_timing(HookTiming::Work, started.elapsed());
                    return LRESULT(1);
                }
            }
        }
    }
    record_hook_timing(HookTiming::Work, started.elapsed());
    timed_call_next_hook(code, wparam, lparam)
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }
    let started = Instant::now();
    {
        let native = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        note_callback(
            false,
            native.flags & LLMHF_INJECTED != 0,
            native.dwExtraInfo,
        );
        if native.flags & LLMHF_INJECTED == 0
            && native.dwExtraInfo != REPLAY_TAG
            && native.dwExtraInfo != SHORTCUT_TAG
            && native.dwExtraInfo != PRIORITY_PROBE_TAG
        {
            let message = wparam.0 as u32;
            let kind = mouse_input(message, native.mouseData);
            if (kind.is_some() || message == WM_MOUSEMOVE)
                && process_normalized_input(kind, None, Some((native.pt.x, native.pt.y)))
            {
                record_hook_timing(HookTiming::Work, started.elapsed());
                return LRESULT(1);
            }
        }
    }
    record_hook_timing(HookTiming::Work, started.elapsed());
    timed_call_next_hook(code, wparam, lparam)
}

#[derive(Clone, Copy)]
enum HookTiming {
    Work,
    Chain,
    Replay,
}

fn update_hook_timing(counters: &mut ListenerDiagnostics, kind: HookTiming, elapsed: Duration) {
    let elapsed_us = elapsed.as_micros().min(u128::from(u64::MAX)) as u64;
    let (maximum, slow_count) = match kind {
        HookTiming::Work => (
            &mut counters.max_hook_work_us,
            &mut counters.slow_hook_work_count,
        ),
        HookTiming::Chain => (
            &mut counters.max_hook_chain_us,
            &mut counters.slow_hook_chain_count,
        ),
        HookTiming::Replay => (&mut counters.max_replay_us, &mut counters.slow_replay_count),
    };
    *maximum = (*maximum).max(elapsed_us);
    if elapsed > Duration::from_millis(50) {
        *slow_count = slow_count.saturating_add(1);
    }
}

fn record_hook_timing(kind: HookTiming, elapsed: Duration) {
    STATE.with(|slot| {
        if let Ok(mut slot) = slot.try_borrow_mut() {
            if let Some(state) = slot.as_mut() {
                update_hook_timing(&mut state.counters, kind, elapsed);
            }
        }
    });
}

fn timed_call_next_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let started = Instant::now();
    let result = unsafe { CallNextHookEx(None, code, wparam, lparam) };
    record_hook_timing(HookTiming::Chain, started.elapsed());
    result
}

fn note_callback(keyboard: bool, injected: bool, tag: usize) {
    STATE.with(|slot| {
        if let Ok(mut slot) = slot.try_borrow_mut() {
            if let Some(state) = slot.as_mut() {
                state.note_callback(keyboard, injected, tag);
            }
        }
    });
}

fn normalized_key(native: &KBDLLHOOKSTRUCT) -> u16 {
    match native.vkCode {
        0x10 => unsafe { MapVirtualKeyW(native.scanCode, MAPVK_VSC_TO_VK_EX) as u16 },
        0x11 => {
            if native.flags.contains(LLKHF_EXTENDED) {
                0xa3
            } else {
                0xa2
            }
        }
        0x12 => {
            if native.flags.contains(LLKHF_EXTENDED) {
                0xa5
            } else {
                0xa4
            }
        }
        key => key as u16,
    }
}

fn mouse_input(message: u32, data: u32) -> Option<InputKind> {
    let (mouse, down) = match message {
        WM_LBUTTONDOWN => (MouseButton::Left, true),
        WM_LBUTTONUP => (MouseButton::Left, false),
        WM_RBUTTONDOWN => (MouseButton::Right, true),
        WM_RBUTTONUP => (MouseButton::Right, false),
        WM_MBUTTONDOWN => (MouseButton::Middle, true),
        WM_MBUTTONUP => (MouseButton::Middle, false),
        WM_XBUTTONDOWN | WM_XBUTTONUP => {
            let mouse = match (data >> 16) as u16 {
                1 => MouseButton::X1,
                2 => MouseButton::X2,
                _ => return None,
            };
            (mouse, message == WM_XBUTTONDOWN)
        }
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            return Some(InputKind::Wheel {
                axis: if message == WM_MOUSEWHEEL {
                    Axis::Vertical
                } else {
                    Axis::Horizontal
                },
                delta: (data >> 16) as i16 as i32,
            })
        }
        _ => return None,
    };
    let button = Button::Mouse(mouse);
    Some(if down {
        InputKind::Down(button)
    } else {
        InputKind::Up(button)
    })
}

fn replay_input(kind: InputKind, metadata: &BTreeMap<Button, KeyMetadata>) -> INPUT {
    match kind {
        InputKind::Down(Button::Key(key)) | InputKind::Up(Button::Key(key)) => {
            let fallback_scan = unsafe { MapVirtualKeyW(key.into(), MAPVK_VK_TO_VSC_EX) };
            let metadata = metadata
                .get(&Button::Key(key))
                .copied()
                .unwrap_or(KeyMetadata {
                    scan_code: (fallback_scan & 0xff) as u16,
                    extended: fallback_scan & 0xff00 == 0xe000,
                });
            let mut flags = KEYBD_EVENT_FLAGS(0);
            if metadata.scan_code != 0 {
                flags |= KEYEVENTF_SCANCODE;
            }
            if metadata.extended {
                flags |= KEYEVENTF_EXTENDEDKEY;
            }
            if matches!(kind, InputKind::Up(_)) {
                flags |= KEYEVENTF_KEYUP;
            }
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(if metadata.scan_code == 0 { key } else { 0 }),
                        wScan: metadata.scan_code,
                        dwFlags: flags,
                        time: 0,
                        dwExtraInfo: REPLAY_TAG,
                    },
                },
            }
        }
        InputKind::Down(Button::Mouse(mouse)) | InputKind::Up(Button::Mouse(mouse)) => {
            let down = matches!(kind, InputKind::Down(_));
            let (flags, data) = match (mouse, down) {
                (MouseButton::Left, true) => (MOUSEEVENTF_LEFTDOWN, 0),
                (MouseButton::Left, false) => (MOUSEEVENTF_LEFTUP, 0),
                (MouseButton::Right, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
                (MouseButton::Right, false) => (MOUSEEVENTF_RIGHTUP, 0),
                (MouseButton::Middle, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
                (MouseButton::Middle, false) => (MOUSEEVENTF_MIDDLEUP, 0),
                (MouseButton::X1, true) => (MOUSEEVENTF_XDOWN, 1),
                (MouseButton::X1, false) => (MOUSEEVENTF_XUP, 1),
                (MouseButton::X2, true) => (MOUSEEVENTF_XDOWN, 2),
                (MouseButton::X2, false) => (MOUSEEVENTF_XUP, 2),
            };
            INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        mouseData: data,
                        dwFlags: flags,
                        dwExtraInfo: REPLAY_TAG,
                        ..MOUSEINPUT::default()
                    },
                },
            }
        }
        InputKind::Wheel { axis, delta } => INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    mouseData: delta as u32,
                    dwFlags: if axis == Axis::Vertical {
                        MOUSEEVENTF_WHEEL
                    } else {
                        MOUSEEVENTF_HWHEEL
                    },
                    dwExtraInfo: REPLAY_TAG,
                    ..MOUSEINPUT::default()
                },
            },
        },
    }
}

pub(crate) fn local_clock() -> (u16, u16, u16) {
    let time = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    (time.wHour, time.wMinute, time.wSecond)
}

pub(crate) fn key_name(key: u16) -> Option<String> {
    let scan = unsafe { MapVirtualKeyW(key.into(), MAPVK_VK_TO_VSC_EX) };
    let mut lparam = ((scan & 0xff) << 16) as i32;
    if scan & 0xff00 == 0xe000 {
        lparam |= 1 << 24;
    }
    let mut buffer = [0u16; 64];
    let count = unsafe { GetKeyNameTextW(lparam, &mut buffer) };
    (count > 0).then(|| String::from_utf16_lossy(&buffer[..count as usize]))
}

/// 面板导航键；`enter` 为 false 时不接管回车。
fn panel_key(button: Button, enter: bool) -> Option<PanelKey> {
    let Button::Key(vk) = button else {
        return None;
    };
    Some(match vk {
        0x25 => PanelKey::Left,
        0x26 => PanelKey::Up,
        0x27 => PanelKey::Right,
        0x28 => PanelKey::Down,
        0x0d if enter => PanelKey::Enter,
        0x1b => PanelKey::Escape,
        0x31..=0x39 => PanelKey::Digit((vk - 0x30) as u8),
        0x61..=0x69 => PanelKey::Digit((vk - 0x60) as u8),
        _ => return None,
    })
}

fn panel_axis_origin(cursor: i32, start: i32, end: i32, extent: i32) -> i32 {
    cursor
        .saturating_sub(extent / 2)
        .clamp(start, end.saturating_sub(extent).max(start))
}

/// 呼出面板前的前台窗口；面板为输入筛选取得前台后，收起时交还给它。
static PANEL_RESTORE_TARGET: AtomicIsize = AtomicIsize::new(0);

/// 设置或取消面板的不激活样式；点击与显示都不抢占前台。
fn set_panel_no_activate(hwnd: windows::Win32::Foundation::HWND, no_activate: bool) {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_NOACTIVATE,
    };
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
    let next = if no_activate {
        style | WS_EX_NOACTIVATE.0 as isize
    } else {
        style & !(WS_EX_NOACTIVATE.0 as isize)
    };
    if next != style {
        unsafe { SetWindowLongPtrW(hwnd, GWL_EXSTYLE, next) };
    }
}

/// Hook 接管导航键前核验登记的面板仍可见，避免界面状态滞后时吞掉按键。
#[cfg(not(test))]
fn panel_window_visible(hwnd: usize) -> bool {
    use windows::Win32::{Foundation::HWND, UI::WindowsAndMessaging::IsWindowVisible};
    unsafe { IsWindowVisible(HWND(hwnd as *mut std::ffi::c_void)) }.as_bool()
}

#[cfg(test)]
fn panel_window_visible(_: usize) -> bool {
    true
}

pub(crate) fn hide_panel_window(hwnd: isize) {
    use windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{IsWindow, SetForegroundWindow, ShowWindow, SW_HIDE},
    };
    let hwnd = HWND(hwnd as *mut std::ffi::c_void);
    let was_foreground = unsafe { GetForegroundWindow() } == hwnd;
    let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
    if was_foreground {
        // 此时本进程仍持有前台权限，交还给呼出前的窗口，不依赖系统挑选下一个窗口。
        let target = HWND(PANEL_RESTORE_TARGET.load(Ordering::Relaxed) as *mut std::ffi::c_void);
        if !target.is_invalid() && target != hwnd && unsafe { IsWindow(Some(target)) }.as_bool() {
            let _ = unsafe { SetForegroundWindow(target) };
        }
    }
}

pub(crate) fn activate_panel_for_input(hwnd: isize) -> Result<bool, String> {
    let handle = windows::Win32::Foundation::HWND(hwnd as *mut std::ffi::c_void);
    set_panel_no_activate(handle, false);
    activate_panel_window(hwnd)
}

pub(crate) fn is_foreground_window(hwnd: isize) -> bool {
    let hwnd = windows::Win32::Foundation::HWND(hwnd as *mut std::ffi::c_void);
    unsafe { GetForegroundWindow() == hwnd }
}

pub(crate) fn activate_panel_window(hwnd: isize) -> Result<bool, String> {
    use windows::Win32::{
        Foundation::HWND,
        UI::{
            Input::KeyboardAndMouse::{GetFocus, SetFocus},
            WindowsAndMessaging::{GetWindowThreadProcessId, IsWindow, SetForegroundWindow},
        },
    };
    let hwnd = HWND(hwnd as *mut std::ffi::c_void);
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return Err("触发面板窗口句柄无效。".into());
    }
    if unsafe { GetWindowThreadProcessId(hwnd, None) } != unsafe { GetCurrentThreadId() } {
        return Err("触发面板窗口不属于当前界面线程。".into());
    }
    if unsafe { GetForegroundWindow() } != hwnd {
        // 前台权限由系统决定；拒绝不等于 API 故障，也不通过模拟按键绕过。
        let _ = unsafe { SetForegroundWindow(hwnd) };
    }
    if unsafe { GetForegroundWindow() } != hwnd {
        return Ok(false);
    }
    // SetFocus 返回的是旧焦点，旧值为 NULL 时绑定也可能报告错误；核验实际结果。
    let focus_result = unsafe { SetFocus(Some(hwnd)) };
    if unsafe { GetFocus() } != hwnd {
        return Err(match focus_result {
            Ok(_) => "触发面板未能取得键盘焦点。".into(),
            Err(error) => format!("触发面板未能取得键盘焦点：{error}"),
        });
    }
    Ok(true)
}

pub(crate) fn panel_contains_point(hwnd: isize, x: i32, y: i32) -> bool {
    use windows::Win32::{
        Foundation::{HWND, RECT},
        UI::WindowsAndMessaging::{GetWindowRect, IsWindow},
    };
    let hwnd = HWND(hwnd as *mut std::ffi::c_void);
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return false;
    }
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect) }.is_ok()
        && x >= rect.left
        && x < rect.right
        && y >= rect.top
        && y < rect.bottom
}

pub(crate) fn prepare_panel_window(hwnd: isize) -> Result<(), String> {
    prepare_floating_window(hwnd, windows::core::w!("OmniTrigger 触发面板"), "触发面板")
}

pub(crate) fn prepare_wheel_window(hwnd: isize) -> Result<(), String> {
    prepare_floating_window(hwnd, windows::core::w!("OmniTrigger 轨道轮盘"), "轨道轮盘")?;
    crate::wheel_window::prepare_borderless(hwnd)
}

fn prepare_floating_window(
    hwnd: isize,
    title: windows::core::PCWSTR,
    name: &str,
) -> Result<(), String> {
    use windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{IsWindow, SetWindowTextW, ShowWindow, SW_HIDE},
    };
    let hwnd = HWND(hwnd as *mut std::ffi::c_void);
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return Err(format!("{name}窗口句柄无效。"));
    }
    unsafe { SetWindowTextW(hwnd, title) }
        .map_err(|error| format!("无法设置{name}名称：{error}"))?;
    set_panel_no_activate(hwnd, true);
    let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
    Ok(())
}

/// 逻辑像素按窗口 DPI 换算为设备像素；DPI 不可用时按 96 处理。
fn panel_device_extent(logical: f32, dpi: u32) -> i32 {
    let dpi = if dpi == 0 { 96 } else { dpi };
    (logical.max(1.) * dpi as f32 / 96.).round() as i32
}

/// `content` 为界面布局的逻辑尺寸；给定时按目标显示器 DPI 同步调整客户区。
pub(crate) fn show_panel_at_cursor(hwnd: isize, content: Option<(f32, f32)>) -> Result<(), String> {
    show_floating_at_cursor(
        hwnd,
        content,
        windows::core::w!("OmniTrigger 触发面板"),
        "触发面板",
        None,
    )
}

pub(crate) fn show_wheel_at_cursor_with_size(
    hwnd: isize,
    width: f32,
    height: f32,
) -> Result<(), String> {
    show_floating_at_cursor(
        hwnd,
        Some((width, height)),
        windows::core::w!("OmniTrigger 轨道轮盘"),
        "轨道轮盘",
        None,
    )
}

pub(crate) fn show_wheel_at_point_with_size(
    hwnd: isize,
    x: i32,
    y: i32,
    width: f32,
    height: f32,
) -> Result<(), String> {
    show_floating_at_cursor(
        hwnd,
        Some((width, height)),
        windows::core::w!("OmniTrigger 轨道轮盘"),
        "轨道轮盘",
        Some((x, y)),
    )
}

fn show_floating_at_cursor(
    hwnd: isize,
    content: Option<(f32, f32)>,
    title: windows::core::PCWSTR,
    name: &str,
    anchor: Option<(i32, i32)>,
) -> Result<(), String> {
    use windows::Win32::{
        Foundation::{HWND, POINT, RECT},
        Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST},
        UI::{
            HiDpi::GetDpiForWindow,
            WindowsAndMessaging::{
                GetClientRect, GetCursorPos, GetWindowRect, IsWindow, SetWindowPos, SetWindowTextW,
                HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOSIZE, SWP_SHOWWINDOW,
            },
        },
    };
    let hwnd = HWND(hwnd as *mut std::ffi::c_void);
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return Err(format!("{name}窗口句柄无效。"));
    }
    unsafe { SetWindowTextW(hwnd, title) }
        .map_err(|error| format!("无法设置{name}名称：{error}"))?;
    // 上次输入筛选可能取消了不激活样式；每次呼出都恢复为不抢占前台。
    set_panel_no_activate(hwnd, true);
    let foreground = unsafe { GetForegroundWindow() };
    if foreground != hwnd {
        PANEL_RESTORE_TARGET.store(foreground.0 as isize, Ordering::Relaxed);
    }
    let cursor = if let Some((x, y)) = anchor {
        POINT { x, y }
    } else {
        let mut cursor = POINT::default();
        unsafe { GetCursorPos(&mut cursor) }
            .map_err(|error| format!("无法读取鼠标位置：{error}"))?;
        cursor
    };
    let monitor = unsafe { MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST) };
    if monitor.is_invalid() {
        return Err("无法确定鼠标所在的显示器。".into());
    }
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..MONITORINFO::default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return Err(format!(
            "无法读取显示器工作区：{}",
            std::io::Error::last_os_error()
        ));
    }
    let work = info.rcWork;
    if work.right <= work.left || work.bottom <= work.top {
        return Err("显示器工作区尺寸无效。".into());
    }
    // 全程使用 Win32 屏幕坐标，保留负坐标；逻辑尺寸只在此按窗口 DPI 换算一次。
    // 跨显示器移动可同步触发 DPI 调整，再按调整后的 DPI 与实际尺寸校正一次。
    for _ in 0..2 {
        let mut rect = RECT::default();
        unsafe { GetWindowRect(hwnd, &mut rect) }
            .map_err(|error| format!("无法读取{name}尺寸：{error}"))?;
        let mut width = rect.right.saturating_sub(rect.left);
        let mut height = rect.bottom.saturating_sub(rect.top);
        let mut flags = SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW;
        if let Some((content_width, content_height)) = content {
            let mut client = RECT::default();
            unsafe { GetClientRect(hwnd, &mut client) }
                .map_err(|error| format!("无法读取{name}客户区：{error}"))?;
            // 保留窗框差值，使客户区等于界面布局尺寸。
            let frame_x = width.saturating_sub(client.right - client.left).max(0);
            let frame_y = height.saturating_sub(client.bottom - client.top).max(0);
            let dpi = unsafe { GetDpiForWindow(hwnd) };
            width = panel_device_extent(content_width, dpi).saturating_add(frame_x);
            height = panel_device_extent(content_height, dpi).saturating_add(frame_y);
            flags = SWP_NOACTIVATE | SWP_SHOWWINDOW;
        }
        if width <= 0 || height <= 0 {
            return Err(format!("{name}尺寸无效。"));
        }
        let x = panel_axis_origin(cursor.x, work.left, work.right, width);
        let y = panel_axis_origin(cursor.y, work.top, work.bottom, height);
        unsafe { SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, width, height, flags) }
            .map_err(|error| format!("无法显示或定位{name}：{error}"))?;
    }
    Ok(())
}

pub(crate) fn show_notification_window(
    hwnd: isize,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> Result<(), String> {
    use windows::Win32::{
        Foundation::{GetLastError, SetLastError, HWND, WIN32_ERROR},
        UI::WindowsAndMessaging::{
            GetWindowLongPtrW, IsWindow, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, GWL_STYLE,
            HWND_TOPMOST, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_SHOWWINDOW, WS_CAPTION,
            WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_THICKFRAME,
        },
    };
    let hwnd = HWND(hwnd as *mut std::ffi::c_void);
    if width <= 0 || height <= 0 || !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return Err("提示窗口句柄或尺寸无效".into());
    }
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetLastError(WIN32_ERROR(0));
        let previous = SetWindowLongPtrW(
            hwnd,
            GWL_EXSTYLE,
            style | WS_EX_NOACTIVATE.0 as isize | WS_EX_TOOLWINDOW.0 as isize,
        );
        let error = GetLastError();
        if previous == 0 && error.0 != 0 {
            return Err(format!("无法设置提示窗口样式：{error:?}"));
        }
        // 去掉窗框，使客户区与给定尺寸一致；否则界面按窗口尺寸布局，右下角会被裁掉。
        let frame = GetWindowLongPtrW(hwnd, GWL_STYLE);
        SetLastError(WIN32_ERROR(0));
        let previous = SetWindowLongPtrW(
            hwnd,
            GWL_STYLE,
            frame & !(WS_CAPTION.0 as isize | WS_THICKFRAME.0 as isize),
        );
        let error = GetLastError();
        if previous == 0 && error.0 != 0 {
            return Err(format!("无法设置提示窗口边框：{error:?}"));
        }
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_FRAMECHANGED,
        )
        .map_err(|error| format!("无法显示提示窗口：{error}"))
    }
}

#[cfg(test)]
#[path = "drag_platform_tests.rs"]
mod drag_tests;

#[cfg(test)]
#[path = "region_platform_tests.rs"]
mod region_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_timing_metrics_keep_separate_maxima_and_count_only_over_fifty_ms() {
        let mut counters = ListenerDiagnostics::default();
        update_hook_timing(&mut counters, HookTiming::Work, Duration::from_millis(50));
        assert_eq!(counters.max_hook_work_us, 50_000);
        assert_eq!(counters.slow_hook_work_count, 0);
        update_hook_timing(&mut counters, HookTiming::Work, Duration::from_millis(51));
        update_hook_timing(&mut counters, HookTiming::Work, Duration::from_millis(1));
        update_hook_timing(&mut counters, HookTiming::Chain, Duration::from_millis(60));
        update_hook_timing(&mut counters, HookTiming::Replay, Duration::from_millis(5));
        assert_eq!(counters.max_hook_work_us, 51_000);
        assert_eq!(counters.slow_hook_work_count, 1);
        assert_eq!(counters.max_hook_chain_us, 60_000);
        assert_eq!(counters.slow_hook_chain_count, 1);
        assert_eq!(counters.max_replay_us, 5_000);
        assert_eq!(counters.slow_replay_count, 0);
        counters.slow_replay_count = u64::MAX;
        update_hook_timing(
            &mut counters,
            HookTiming::Replay,
            Duration::from_secs(u64::MAX),
        );
        assert_eq!(counters.max_replay_us, u64::MAX);
        assert_eq!(counters.slow_replay_count, u64::MAX);
    }

    struct HoldEvents(Receiver<BackendEvent>);

    impl HoldEvents {
        // 既有长按状态测试只观察匹配结果；独立测试检查新增的界面通知。
        fn try_recv(&self) -> Result<BackendEvent, mpsc::TryRecvError> {
            loop {
                match self.0.try_recv()? {
                    BackendEvent::PointerPressed { .. } | BackendEvent::EscapePressed => {}
                    event => return Ok(event),
                }
            }
        }

        fn try_iter(&self) -> impl Iterator<Item = BackendEvent> + '_ {
            std::iter::from_fn(|| self.try_recv().ok())
        }
    }

    #[test]
    fn physical_ui_notifications_precede_trigger_and_escape_repeat_is_ignored() {
        let (tx, rx) = mpsc::sync_channel(EVENT_CAPACITY);
        let mut state = ThreadState::new(tx);
        let middle = Button::Mouse(MouseButton::Middle);
        state.command(BackendCommand::SetBindings(vec![TriggerSpec {
            conditions: Vec::new(),
            id: 1,
            held: vec![],
            activation: omnitrigger_core::Activation::Press(middle),
        }]));
        let down = state.input_at(Some(InputKind::Down(middle)), None, Some((-20, 30)), 0);
        assert!(down.consume);
        assert!(matches!(
            rx.try_recv(),
            Ok(BackendEvent::PointerPressed { x: -20, y: 30 })
        ));
        assert!(matches!(
            rx.try_recv(),
            Ok(BackendEvent::Triggered { id: 1, .. })
        ));
        let escape = Button::Key(0x1b);
        assert!(
            !state
                .input_at(Some(InputKind::Down(escape)), None, None, 1)
                .consume
        );
        assert!(matches!(rx.try_recv(), Ok(BackendEvent::EscapePressed)));
        assert!(
            !state
                .input_at(Some(InputKind::Down(escape)), None, None, 2)
                .consume
        );
        assert!(rx.try_recv().is_err());
        state.input_at(Some(InputKind::Up(escape)), None, None, 3);
        state.input_at(Some(InputKind::Down(escape)), None, None, 4);
        assert!(matches!(rx.try_recv(), Ok(BackendEvent::EscapePressed)));
    }

    fn panel_state() -> (ThreadState, Receiver<BackendEvent>) {
        let (tx, rx) = mpsc::sync_channel(EVENT_CAPACITY);
        let mut state = ThreadState::new(tx);
        // 测试中的前台窗口来自真实系统；面板句柄取一个不同的非零值。
        let panel = (unsafe { GetForegroundWindow() }.0 as usize).wrapping_add(16) | 1;
        state.command(BackendCommand::SetPanelWindow(Some(panel as isize)));
        (state, rx)
    }

    fn press(state: &mut ThreadState, button: Button, at: u64) -> bool {
        state
            .input_at(Some(InputKind::Down(button)), None, None, at)
            .consume
    }

    fn release(state: &mut ThreadState, button: Button, at: u64) -> bool {
        state
            .input_at(Some(InputKind::Up(button)), None, None, at)
            .consume
    }

    #[test]
    fn panel_navigation_consumes_bare_keys_with_their_releases() {
        let (mut state, rx) = panel_state();
        let up = Button::Key(0x26);
        assert!(press(&mut state, up, 0));
        assert!(press(&mut state, up, 1));
        assert!(release(&mut state, up, 2));
        assert!(matches!(
            rx.try_recv(),
            Ok(BackendEvent::PanelKey(PanelKey::Up))
        ));
        assert!(matches!(
            rx.try_recv(),
            Ok(BackendEvent::PanelKey(PanelKey::Up))
        ));
        let digit = Button::Key(0x33);
        assert!(press(&mut state, digit, 3));
        assert!(press(&mut state, digit, 4));
        assert!(release(&mut state, digit, 5));
        assert!(matches!(
            rx.try_recv(),
            Ok(BackendEvent::PanelKey(PanelKey::Digit(3)))
        ));
        assert!(press(&mut state, Button::Key(0x67), 6));
        assert!(release(&mut state, Button::Key(0x67), 7));
        assert!(matches!(
            rx.try_recv(),
            Ok(BackendEvent::PanelKey(PanelKey::Digit(7)))
        ));
        let escape = Button::Key(0x1b);
        assert!(press(&mut state, escape, 8));
        assert!(release(&mut state, escape, 9));
        // 被面板接管的 Esc 不再另发 EscapePressed。
        assert!(matches!(
            rx.try_recv(),
            Ok(BackendEvent::PanelKey(PanelKey::Escape))
        ));
        let letter = Button::Key(0x41);
        assert!(!press(&mut state, letter, 10));
        assert!(!release(&mut state, letter, 11));
        assert!(rx.try_recv().is_err());
        assert!(state.panel_consumed.is_empty());
    }

    #[test]
    fn panel_enter_waits_for_arrow_selection_and_escape_disarms_it() {
        let (mut state, rx) = panel_state();
        let enter = Button::Key(0x0d);
        assert!(!press(&mut state, enter, 0));
        assert!(!release(&mut state, enter, 1));
        assert!(rx.try_recv().is_err());
        assert!(press(&mut state, Button::Key(0x28), 2));
        assert!(release(&mut state, Button::Key(0x28), 3));
        assert!(press(&mut state, enter, 4));
        assert!(release(&mut state, enter, 5));
        assert!(matches!(
            rx.try_recv(),
            Ok(BackendEvent::PanelKey(PanelKey::Down))
        ));
        assert!(matches!(
            rx.try_recv(),
            Ok(BackendEvent::PanelKey(PanelKey::Enter))
        ));
        assert!(press(&mut state, Button::Key(0x1b), 6));
        assert!(release(&mut state, Button::Key(0x1b), 7));
        assert!(!press(&mut state, enter, 8));
        assert!(!release(&mut state, enter, 9));
    }

    #[test]
    fn panel_navigation_yields_to_held_keys_pause_recording_and_foreground() {
        let (mut state, rx) = panel_state();
        let ctrl = Button::Key(0xa2);
        let digit = Button::Key(0x31);
        assert!(!press(&mut state, ctrl, 0));
        assert!(!press(&mut state, digit, 1));
        assert!(!release(&mut state, digit, 2));
        assert!(!release(&mut state, ctrl, 3));
        let right = Button::Mouse(MouseButton::Right);
        state.input_at(Some(InputKind::Down(right)), None, Some((0, 0)), 4);
        assert!(!press(&mut state, Button::Key(0x26), 5));
        assert!(!release(&mut state, Button::Key(0x26), 6));
        state.input_at(Some(InputKind::Up(right)), None, Some((0, 0)), 7);
        state.command(BackendCommand::SetEnabled(false));
        assert!(!press(&mut state, digit, 8));
        assert!(!release(&mut state, digit, 9));
        state.command(BackendCommand::SetEnabled(true));
        state.command(BackendCommand::StartRecording);
        assert!(state.recording.is_some());
        state.command(BackendCommand::CancelRecording);
        let foreground = unsafe { GetForegroundWindow() }.0 as isize;
        state.command(BackendCommand::SetPanelWindow(Some(foreground)));
        assert!(!press(&mut state, digit, 10));
        assert!(!release(&mut state, digit, 11));
        state.command(BackendCommand::SetPanelWindow(None));
        assert!(!press(&mut state, digit, 12));
        assert!(!release(&mut state, digit, 13));
        assert!(!rx
            .try_iter()
            .any(|event| matches!(event, BackendEvent::PanelKey(_))));
    }

    #[test]
    fn panel_release_stays_consumed_after_the_panel_hides() {
        let (mut state, rx) = panel_state();
        let left = Button::Key(0x25);
        assert!(press(&mut state, left, 0));
        state.command(BackendCommand::SetPanelWindow(None));
        assert!(press(&mut state, left, 1));
        assert!(release(&mut state, left, 2));
        assert!(!press(&mut state, left, 3));
        assert!(!release(&mut state, left, 4));
        assert_eq!(
            rx.try_iter()
                .filter(|event| matches!(event, BackendEvent::PanelKey(PanelKey::Left)))
                .count(),
            2
        );
    }

    #[test]
    fn exact_due_time_uses_submillisecond_remaining_time_and_saturates_safely() {
        assert_eq!(relative_due_100ns(300, Duration::ZERO), -3_000_000);
        assert_eq!(
            relative_due_100ns(300, Duration::from_nanos(299_123_450)),
            -8766
        );
        assert_eq!(relative_due_100ns(300, Duration::from_millis(300)), -1);
        assert_eq!(relative_due_100ns(300, Duration::from_millis(400)), -1);
        assert_eq!(relative_due_100ns(u64::MAX, Duration::ZERO), -i64::MAX);
        assert_eq!(input_wait_timeout_ms(false, None), INFINITE);
        assert_eq!(input_wait_timeout_ms(true, None), 25);
        assert_eq!(
            input_wait_timeout_ms(false, Some(Instant::now() - Duration::from_millis(1))),
            1
        );
    }

    #[test]
    fn native_waitable_is_one_shot_and_same_deadline_does_not_rearm_it() {
        use windows::Win32::{Foundation::WAIT_TIMEOUT, System::Threading::WaitForSingleObject};
        let mut timer = HoldDeadlineTimer::new().expect("Windows 应可创建本进程的等待计时器");
        timer.arm(1, Duration::ZERO).unwrap();
        assert_eq!(
            unsafe { WaitForSingleObject(timer.handle, 1000) },
            WAIT_OBJECT_0
        );
        // 相同 deadline 的重新调度不能创建第二次唤醒。
        timer.arm(1, Duration::ZERO).unwrap();
        assert_eq!(
            unsafe { WaitForSingleObject(timer.handle, 25) },
            WAIT_TIMEOUT
        );
        // 模拟消息泵已消费提前唤醒，剩余时间必须能重新 arm。
        timer.armed_deadline_ms = None;
        timer.arm(1, Duration::ZERO).unwrap();
        assert_eq!(
            unsafe { WaitForSingleObject(timer.handle, 1000) },
            WAIT_OBJECT_0
        );
        timer.armed_deadline_ms = None;
        timer.arm(10000, Duration::ZERO).unwrap();
        timer.cancel();
        assert_eq!(timer.armed_deadline_ms, None);
        assert_eq!(
            unsafe { WaitForSingleObject(timer.handle, 0) },
            WAIT_TIMEOUT
        );
    }

    #[test]
    fn message_aware_wait_handles_queued_messages_and_native_timer_without_input_injection() {
        let mut message = MSG::default();
        for _ in 0..MESSAGE_BATCH {
            if !unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
                break;
            }
        }
        let mut timer = HoldDeadlineTimer::new().expect("Windows 应可创建本进程的等待计时器");
        let handles = [timer.handle];
        let thread_id = unsafe { GetCurrentThreadId() };
        for value in 1..=2 {
            unsafe { PostThreadMessageW(thread_id, COMMAND_MESSAGE, WPARAM(value), LPARAM(0)) }
                .unwrap();
        }
        for value in 1..=2 {
            let wake = unsafe {
                MsgWaitForMultipleObjectsEx(Some(&handles), 1000, QS_ALLINPUT, MWMO_INPUTAVAILABLE)
            };
            assert_eq!(wake.0, WAIT_OBJECT_0.0 + 1);
            assert!(unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool());
            assert_eq!(message.message, COMMAND_MESSAGE);
            assert_eq!(message.wParam.0, value);
        }
        timer.arm(1, Duration::ZERO).unwrap();
        assert_eq!(
            unsafe {
                MsgWaitForMultipleObjectsEx(Some(&handles), 1000, QS_ALLINPUT, MWMO_INPUTAVAILABLE)
            },
            WAIT_OBJECT_0
        );
    }

    #[test]
    fn mouse_moves_keep_the_exact_deadline_and_pause_cancels_the_native_timer() {
        let (mut state, _) = right_hold_state();
        let right = Button::Mouse(MouseButton::Right);
        state.input_at(Some(InputKind::Down(right)), None, Some((100, 100)), 0);
        state.reschedule_hold_timer();
        assert_eq!(
            state.waitable_timer.as_ref().unwrap().armed_deadline_ms,
            Some(300)
        );
        state.input_at(None, None, Some((102, 100)), 10);
        state.reschedule_hold_timer();
        assert_eq!(
            state.waitable_timer.as_ref().unwrap().armed_deadline_ms,
            Some(300)
        );
        assert_eq!(state.hold_timer, 0);
        state.command(BackendCommand::SetEnabled(false));
        state.reschedule_hold_timer();
        assert_eq!(
            state.waitable_timer.as_ref().unwrap().armed_deadline_ms,
            None
        );
        assert!(state.diagnostics().hold_timer_backend.contains("waitable"));
    }

    #[test]
    fn legacy_timer_fallback_can_arm_and_cancel_on_a_local_message_queue() {
        let mut message = MSG::default();
        let _ = unsafe { PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE) };
        let (mut state, _) = right_hold_state();
        state.waitable_timer = None;
        let right = Button::Mouse(MouseButton::Right);
        state.input_at(Some(InputKind::Down(right)), None, Some((0, 0)), 0);
        state.reschedule_hold_timer();
        assert_ne!(state.hold_timer, 0);
        state.command(BackendCommand::SetEnabled(false));
        state.reschedule_hold_timer();
        assert_eq!(state.hold_timer, 0);
    }

    fn right_hold_state() -> (ThreadState, HoldEvents) {
        let (tx, rx) = mpsc::sync_channel(EVENT_CAPACITY);
        let mut state = ThreadState::new(tx);
        state.command(BackendCommand::SetBindings(vec![TriggerSpec {
            conditions: Vec::new(),
            id: 7,
            held: vec![],
            activation: omnitrigger_core::Activation::Hold {
                button: Button::Mouse(MouseButton::Right),
                duration_ms: 300,
            },
        }]));
        (state, HoldEvents(rx))
    }

    #[test]
    fn repeated_holds_and_focus_changes_leave_no_press_state_and_keep_eight_results() {
        let (mut state, _) = right_hold_state();
        let right = Button::Mouse(MouseButton::Right);
        let mut expected = Vec::new();
        for round in 0..10 {
            let start = round * 1000;
            state.input_at(Some(InputKind::Down(right)), None, Some((100, 100)), start);
            assert_eq!(state.engine.next_deadline_ms(), Some(start + 300));
            let outcome = match round % 3 {
                0 => {
                    state.advance_time_at(start + 300);
                    // 已匹配后面板激活造成的前台变化只应收尾释放，不遗留 Draining。
                    state.update_foreground(state.foreground.wrapping_add(1));
                    HoldOutcome::Matched
                }
                1 => {
                    state.clock_origin = Instant::now() - Duration::from_millis(start + 100);
                    state.update_foreground(state.foreground.wrapping_add(1));
                    HoldOutcome::FocusChanged
                }
                _ => HoldOutcome::ShortRelease,
            };
            let release_at = if outcome == HoldOutcome::ShortRelease {
                299
            } else {
                350
            };
            let release = state.input_at(
                Some(InputKind::Up(right)),
                None,
                Some((100, 100)),
                start + release_at,
            );
            if !release.replay_kinds.is_empty() {
                state.replay_succeeded(&release.replay_kinds);
            }
            assert_eq!(state.engine.next_deadline_ms(), None);
            assert!(!state.physical_held.contains(&right));
            assert!(!state.owned_press.contains(&right));
            assert!(!state.failure_draining.contains(&right));
            expected.push(outcome);
        }
        let snapshot = state.diagnostics();
        assert_eq!(snapshot.hold_terminal_downs, 10);
        assert_eq!(snapshot.hold_terminal_ups, 10);
        assert_eq!(snapshot.hold_started, 10);
        assert_eq!(snapshot.hold_not_started, 0);
        assert_eq!(snapshot.hold_terminal_duplicate_downs, 0);
        assert_eq!(snapshot.hold_triggered, 4);
        assert_eq!(snapshot.hold_cancelled_focus, 3);
        assert_eq!(snapshot.hold_cancelled_other, 3);
        assert_eq!(snapshot.recent_hold_attempts.len(), 8);
        assert_eq!(
            snapshot
                .recent_hold_attempts
                .iter()
                .map(|attempt| attempt.outcome)
                .collect::<Vec<_>>(),
            expected[2..]
        );
        assert!(snapshot
            .recent_hold_attempts
            .iter()
            .all(|attempt| attempt.threshold_ms == 300));
    }

    #[test]
    fn diagnostic_distinguishes_extra_held_keys_and_missing_release_from_hold_cancellation() {
        let (mut state, _) = right_hold_state();
        let right = Button::Mouse(MouseButton::Right);
        let control = Button::Key(0xa3);
        state.input_at(Some(InputKind::Down(control)), None, None, 0);
        state.input_at(Some(InputKind::Down(right)), None, Some((0, 0)), 1);
        assert_eq!(state.engine.next_deadline_ms(), None);
        assert_eq!(state.diagnostics().hold_not_started, 1);
        assert_eq!(
            state.diagnostics().last_hold_start_rejection,
            Some(HoldStartRejection::ExtraHeld)
        );
        assert!(state.diagnostics().recent_hold_attempts.is_empty());
        state.input_at(Some(InputKind::Up(control)), None, None, 2);
        state.input_at(Some(InputKind::Up(right)), None, Some((0, 0)), 3);
        state.input_at(Some(InputKind::Down(right)), None, Some((0, 0)), 1000);
        state.advance_time_at(1300);
        // 模拟没收到前一次 Up 而再次收到右键 Down，不修改核心的 repeat 语义。
        state.input_at(Some(InputKind::Down(right)), None, Some((0, 0)), 1400);
        assert_eq!(state.diagnostics().hold_terminal_duplicate_downs, 1);
        assert_eq!(
            state.diagnostics().last_hold_start_rejection,
            Some(HoldStartRejection::ExistingPress)
        );
        assert_eq!(state.diagnostics().hold_started, 1);
        state.input_at(Some(InputKind::Up(right)), None, Some((0, 0)), 1401);
        state.input_at(Some(InputKind::Down(right)), None, Some((0, 0)), 2000);
        assert_eq!(state.engine.next_deadline_ms(), Some(2300));
        assert_eq!(state.diagnostics().hold_started, 2);
    }

    #[test]
    fn rejected_hold_start_reports_pause_recording_or_unmatched_context() {
        let right = Button::Mouse(MouseButton::Right);
        for (command, expected) in [
            (
                BackendCommand::SetEnabled(false),
                HoldStartRejection::Paused,
            ),
            (
                BackendCommand::StartRecording,
                HoldStartRejection::Recording,
            ),
        ] {
            let (mut state, _) = right_hold_state();
            state.command(command);
            state.input_at(Some(InputKind::Down(right)), None, Some((0, 0)), 0);
            assert_eq!(state.diagnostics().hold_not_started, 1);
            assert_eq!(
                state.diagnostics().last_hold_start_rejection,
                Some(expected)
            );
            assert!(state.diagnostics().recent_hold_attempts.is_empty());
        }
        let (mut state, _) = right_hold_state();
        state.command(BackendCommand::SetBindings(vec![TriggerSpec {
            conditions: Vec::new(),
            id: 9,
            held: vec![Button::Key(0x14)],
            activation: omnitrigger_core::Activation::Hold {
                button: right,
                duration_ms: 300,
            },
        }]));
        state.input_at(Some(InputKind::Down(right)), None, Some((0, 0)), 0);
        assert_eq!(
            state.diagnostics().last_hold_start_rejection,
            Some(HoldStartRejection::Unmatched)
        );
    }

    #[test]
    fn hold_attempt_history_separates_other_button_wheel_and_movement() {
        let right = Button::Mouse(MouseButton::Right);
        for (kind, pointer, expected) in [
            (
                Some(InputKind::Down(Button::Key(0x41))),
                None,
                HoldOutcome::OtherButton,
            ),
            (
                Some(InputKind::Wheel {
                    axis: Axis::Vertical,
                    delta: 120,
                }),
                None,
                HoldOutcome::Wheel,
            ),
            (None, Some((9, 0)), HoldOutcome::Movement),
        ] {
            let (mut state, _) = right_hold_state();
            state.input_at(Some(InputKind::Down(right)), None, Some((0, 0)), 0);
            state.input_at(kind, None, pointer, 100);
            assert_eq!(
                state.diagnostics().recent_hold_attempts,
                vec![HoldAttempt {
                    outcome: expected,
                    elapsed_ms: 100,
                    threshold_ms: 300,
                }]
            );
        }
    }

    #[test]
    fn hold_clock_fires_without_another_input_and_consumes_physical_release() {
        let (mut state, rx) = right_hold_state();
        let right = Button::Mouse(MouseButton::Right);
        assert!(
            state
                .input_at(Some(InputKind::Down(right)), None, Some((100, 100)), 0)
                .consume
        );
        assert_eq!(state.engine.next_deadline_ms(), Some(300));
        state.advance_time_at(299);
        assert!(rx.try_recv().is_err());
        state.advance_time_at(300);
        assert!(matches!(
            rx.try_recv(),
            Ok(BackendEvent::HoldTiming {
                id: 7,
                elapsed_ms: 300,
                threshold_ms: 300,
                ..
            })
        ));
        assert!(matches!(
            rx.try_recv(),
            Ok(BackendEvent::Triggered { id: 7, .. })
        ));
        let timing = state.diagnostics();
        assert_eq!(timing.last_hold_elapsed_ms, Some(300));
        assert_eq!(timing.last_hold_threshold_ms, Some(300));
        assert_eq!(timing.last_hold_lateness_ms, Some(0));
        assert_eq!(timing.hold_triggered, 1);
        state.advance_time_at(900);
        assert!(rx.try_recv().is_err());
        let release = state.input_at(Some(InputKind::Up(right)), None, Some((100, 100)), 950);
        assert!(release.consume);
        assert!(release.replay_kinds.is_empty());
        assert!(state.owned_press.is_empty());
    }

    #[test]
    fn delayed_hold_match_reports_elapsed_threshold_and_lateness_before_trigger() {
        let (mut state, rx) = right_hold_state();
        let right = Button::Mouse(MouseButton::Right);
        state.input_at(Some(InputKind::Down(right)), None, Some((100, 100)), 1000);
        let before_match = Instant::now();
        state.advance_time_at(1350);
        let matched_at = match rx.try_recv().unwrap() {
            BackendEvent::HoldTiming {
                id: 7,
                elapsed_ms: 350,
                threshold_ms: 300,
                matched_at,
            } => matched_at,
            other => panic!("意外长按计时通知：{other:?}"),
        };
        assert!(matched_at >= before_match && matched_at <= Instant::now());
        assert!(matches!(
            rx.try_recv(),
            Ok(BackendEvent::Triggered { id: 7, .. })
        ));
        let timing = state.diagnostics();
        assert_eq!(timing.last_hold_elapsed_ms, Some(350));
        assert_eq!(timing.last_hold_threshold_ms, Some(300));
        assert_eq!(timing.last_hold_lateness_ms, Some(50));
        assert_eq!(timing.hold_triggered, 1);
        assert_eq!(timing.hold_cancelled_movement, 0);
        assert_eq!(timing.hold_cancelled_focus, 0);
        assert_eq!(timing.hold_cancelled_other, 0);
        state.advance_time_at(2000);
        assert!(rx.try_recv().is_err());
        assert_eq!(state.diagnostics().hold_triggered, 1);
    }

    #[test]
    fn short_right_hold_replays_a_complete_click_without_triggering() {
        let (mut state, rx) = right_hold_state();
        let right = Button::Mouse(MouseButton::Right);
        state.input_at(Some(InputKind::Down(right)), None, Some((-20, 50)), 0);
        let release = state.input_at(Some(InputKind::Up(right)), None, Some((-20, 50)), 299);
        assert!(release.consume);
        assert_eq!(
            release.replay_kinds,
            vec![InputKind::Down(right), InputKind::Up(right)]
        );
        assert_eq!(state.engine.next_deadline_ms(), None);
        assert!(rx.try_recv().is_err());
        let timing = state.diagnostics();
        assert_eq!(timing.last_hold_elapsed_ms, Some(299));
        assert_eq!(timing.last_hold_threshold_ms, Some(300));
        assert_eq!(timing.last_hold_lateness_ms, None);
        assert_eq!(timing.hold_cancelled_other, 1);
        assert_eq!(timing.hold_triggered, 0);
        assert_eq!(
            timing.recent_hold_attempts[0].outcome,
            HoldOutcome::ShortRelease
        );
    }

    #[test]
    fn movement_and_release_in_one_packet_preserve_replay_order() {
        let (mut state, rx) = right_hold_state();
        let right = Button::Mouse(MouseButton::Right);
        state.input_at(Some(InputKind::Down(right)), None, Some((10, 10)), 0);
        let release = state.input_at(Some(InputKind::Up(right)), None, Some((19, 10)), 100);
        assert!(release.consume);
        assert_eq!(
            release.replay_kinds,
            vec![InputKind::Down(right), InputKind::Up(right)]
        );
        state.advance_time_at(400);
        assert!(rx.try_recv().is_err());
        assert_eq!(state.diagnostics().hold_cancelled_movement, 1);
        assert_eq!(state.diagnostics().hold_cancelled_other, 0);
    }

    #[test]
    fn drag_fallback_passes_mouse_move_and_then_the_physical_release() {
        let (mut state, rx) = right_hold_state();
        let right = Button::Mouse(MouseButton::Right);
        state.input_at(Some(InputKind::Down(right)), None, Some((100, 100)), 0);
        let movement = state.input_at(None, None, Some((109, 100)), 100);
        assert!(!movement.consume);
        assert_eq!(movement.replay_kinds, vec![InputKind::Down(right)]);
        state.replay_succeeded(&movement.replay_kinds);
        assert!(state.owned_press.is_empty());
        let release = state.input_at(Some(InputKind::Up(right)), None, Some((110, 100)), 200);
        assert!(!release.consume);
        assert!(release.replay_kinds.is_empty());
        state.advance_time_at(400);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn focus_change_cancels_hold_and_drains_without_replaying_to_the_new_window() {
        let (mut state, rx) = right_hold_state();
        let right = Button::Mouse(MouseButton::Right);
        state.input_at(Some(InputKind::Down(right)), None, Some((10, 10)), 0);
        state.update_foreground(state.foreground.wrapping_add(1));
        assert_eq!(state.engine.next_deadline_ms(), None);
        assert_eq!(state.diagnostics().hold_cancelled_focus, 1);
        assert_eq!(state.diagnostics().hold_cancelled_other, 0);
        state.advance_time_at(400);
        assert!(rx.try_recv().is_err());
        let release = state.input_at(Some(InputKind::Up(right)), None, Some((10, 10)), 450);
        assert!(release.consume);
        assert!(release.replay_kinds.is_empty());
        assert!(state.owned_press.is_empty());
    }

    #[test]
    fn pause_replacement_and_recording_invalidate_pending_hold() {
        for command in [
            BackendCommand::SetEnabled(false),
            BackendCommand::SetBindings(vec![]),
            BackendCommand::StartRecording,
        ] {
            let (mut state, rx) = right_hold_state();
            let right = Button::Mouse(MouseButton::Right);
            state.input_at(Some(InputKind::Down(right)), None, Some((10, 10)), 0);
            state.command(command);
            assert_eq!(state.engine.next_deadline_ms(), None);
            assert_eq!(state.diagnostics().hold_cancelled_other, 1);
            state.advance_time_at(400);
            let release = state.input_at(Some(InputKind::Up(right)), None, Some((10, 10)), 450);
            assert!(release.consume);
            assert!(release.replay_kinds.is_empty());
            assert!(!rx
                .try_iter()
                .any(|event| matches!(event, BackendEvent::Triggered { .. })));
        }
    }

    #[test]
    fn timer_failure_reports_pause_and_keeps_release_owned_until_drained() {
        let (mut state, rx) = right_hold_state();
        let right = Button::Mouse(MouseButton::Right);
        state.input_at(Some(InputKind::Down(right)), None, Some((10, 10)), 0);
        state.hold_timer_failed("模拟计时器失败".into());
        assert_eq!(state.diagnostics().hold_cancelled_other, 1);
        assert!(
            matches!(rx.try_recv(), Ok(BackendEvent::Error(message)) if message.contains("监听已暂停"))
        );
        assert_eq!(state.engine.next_deadline_ms(), None);
        state.advance_time_at(400);
        assert!(rx.try_recv().is_err());
        let release = state.input_at(Some(InputKind::Up(right)), None, Some((10, 10)), 450);
        assert!(release.consume);
        assert!(release.replay_kinds.is_empty());
        assert!(state.owned_press.is_empty());
        // 计时器失败已暂停，而不是只清掉本次候选。
        assert!(
            !state
                .input_at(Some(InputKind::Down(right)), None, Some((10, 10)), 500)
                .consume
        );
    }

    #[test]
    fn hold_timer_wakes_at_the_deadline_or_for_bounded_foreground_checks() {
        assert_eq!(hold_timer_delay_ms(0, 300), 25);
        assert_eq!(hold_timer_delay_ms(290, 300), 10);
        assert_eq!(hold_timer_delay_ms(300, 300), 1);
        assert_eq!(hold_timer_delay_ms(400, 300), 1);
        assert_eq!(hold_timer_delay_ms(u64::MAX - 3, u64::MAX), 3);
    }

    #[test]
    fn panel_position_centers_on_the_pointer_and_clamps_to_the_work_area() {
        assert_eq!(panel_axis_origin(960, 0, 1920, 640), 640);
        assert_eq!(panel_axis_origin(-960, -1920, 0, 640), -1280);
        assert_eq!(panel_axis_origin(500, 0, 1920, 301), 350);
        assert_eq!(panel_axis_origin(-1000, -1920, 0, 641), -1320);
        assert_eq!(panel_axis_origin(-1800, -1920, 0, 640), -1920);
        assert_eq!(panel_axis_origin(-50, -1920, 0, 640), -640);
        assert_eq!(panel_axis_origin(-2000, -1920, 0, 640), -1920);
        assert_eq!(panel_axis_origin(1900, 0, 1920, 640), 1280);
        assert_eq!(panel_axis_origin(100, 0, 800, 1000), 0);
        assert_eq!(panel_axis_origin(i32::MIN, 0, 800, i32::MAX), 0);
        assert_eq!(panel_axis_origin(i32::MAX, -1920, 0, i32::MAX), -1920);
    }

    #[test]
    fn panel_content_size_scales_with_window_dpi() {
        assert_eq!(panel_device_extent(552., 96), 552);
        assert_eq!(panel_device_extent(552., 144), 828);
        assert_eq!(panel_device_extent(229., 120), 286);
        assert_eq!(panel_device_extent(552., 0), 552);
        assert_eq!(panel_device_extent(0., 96), 1);
    }

    #[test]
    fn mouse_mapping_preserves_signed_high_precision_delta_and_axis() {
        assert_eq!(
            mouse_input(WM_MOUSEWHEEL, (-30i16 as u16 as u32) << 16),
            Some(InputKind::Wheel {
                axis: Axis::Vertical,
                delta: -30
            })
        );
        assert_eq!(
            mouse_input(WM_MOUSEHWHEEL, 120 << 16),
            Some(InputKind::Wheel {
                axis: Axis::Horizontal,
                delta: 120
            })
        );
        assert_eq!(
            mouse_input(WM_XBUTTONDOWN, 2 << 16),
            Some(InputKind::Down(Button::Mouse(MouseButton::X2)))
        );
        assert_eq!(mouse_input(WM_XBUTTONUP, 3 << 16), None);
    }
    #[test]
    fn right_control_and_alt_are_distinct_from_left() {
        let mut native = KBDLLHOOKSTRUCT {
            vkCode: 0x11,
            ..KBDLLHOOKSTRUCT::default()
        };
        assert_eq!(normalized_key(&native), 0xa2);
        native.flags = LLKHF_EXTENDED;
        assert_eq!(normalized_key(&native), 0xa3);
        native.vkCode = 0x12;
        assert_eq!(normalized_key(&native), 0xa5);
    }
    #[test]
    fn replay_uses_original_extended_scan_code_and_injection_tag() {
        let mut metadata = BTreeMap::new();
        metadata.insert(
            Button::Key(0xa3),
            KeyMetadata {
                scan_code: 0x1d,
                extended: true,
            },
        );
        let input = replay_input(InputKind::Up(Button::Key(0xa3)), &metadata);
        let keyboard = unsafe { input.Anonymous.ki };
        assert_eq!(keyboard.wScan, 0x1d);
        assert_eq!(keyboard.wVk.0, 0);
        assert!(keyboard
            .dwFlags
            .contains(KEYEVENTF_SCANCODE | KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP));
        assert_eq!(keyboard.dwExtraInfo, REPLAY_TAG);
    }
    #[test]
    fn recording_ignores_preexisting_mouse_release_and_consumes_complete_chord() {
        let (tx, rx) = mpsc::sync_channel(EVENT_CAPACITY);
        let mut state = ThreadState::new(tx);
        let mouse = Button::Mouse(MouseButton::Left);
        state.physical_held.insert(mouse);
        state.command(BackendCommand::StartRecording);
        assert!(
            !state
                .record_input(InputEvent {
                    kind: InputKind::Up(mouse),
                    injected: false
                })
                .consume
        );
        let caps = Button::Key(0x14);
        let w = Button::Key(0x57);
        for kind in [
            InputKind::Down(caps),
            InputKind::Down(caps),
            InputKind::Down(w),
            InputKind::Up(w),
            InputKind::Up(caps),
        ] {
            assert!(
                state
                    .record_input(InputEvent {
                        kind,
                        injected: false
                    })
                    .consume
            );
        }
        let events: Vec<_> = rx.try_iter().collect();
        let progress: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                BackendEvent::RecordingProgress(held) => Some(held.clone()),
                _ => None,
            })
            .collect();
        // repeat 不重复通知；完成录制的最后一次释放直接发送 Recorded。
        assert_eq!(progress, vec![vec![caps], vec![caps, w], vec![caps]]);
        assert!(
            matches!(events.last(), Some(BackendEvent::Recorded(spec)) if spec.held == vec![caps] && spec.activation == omnitrigger_core::Activation::Press(w))
        );
        assert!(state.recording.is_none());
    }
    #[test]
    fn escape_cancel_keeps_release_consumed() {
        let (tx, rx) = mpsc::sync_channel(EVENT_CAPACITY);
        let mut state = ThreadState::new(tx);
        state.command(BackendCommand::StartRecording);
        assert!(
            state
                .record_input(InputEvent {
                    kind: InputKind::Down(Button::Key(0x1b)),
                    injected: false
                })
                .consume
        );
        assert!(matches!(
            rx.try_recv(),
            Ok(BackendEvent::RecordingCancelled)
        ));
        assert!(state.recording.is_none());
        assert!(state.recording_consumed.contains(&Button::Key(0x1b)));
    }

    #[test]
    fn fallback_batches_current_down_and_failure_drains_its_release() {
        let (tx, _) = mpsc::sync_channel(EVENT_CAPACITY);
        let mut state = ThreadState::new(tx);
        let caps = Button::Key(0x14);
        let c = Button::Key(0x43);
        state
            .engine
            .replace_bindings(vec![TriggerSpec {
                conditions: Vec::new(),
                id: 1,
                held: vec![caps],
                activation: omnitrigger_core::Activation::Press(Button::Key(0x57)),
            }])
            .unwrap();
        assert!(state.input(InputKind::Down(caps), None).consume);
        let fallback = state.input(InputKind::Down(c), None);
        assert!(fallback.consume);
        assert_eq!(
            fallback.replay_kinds,
            vec![InputKind::Down(caps), InputKind::Down(c)]
        );
        state.replay_failed(&fallback.replay_kinds);
        assert!(state.failure_draining.contains(&caps));
        assert!(state.failure_draining.contains(&c));
        assert!(state.input(InputKind::Down(c), None).consume);
        assert!(state.input(InputKind::Up(caps), None).consume);
        assert!(state.input(InputKind::Up(c), None).consume);
        assert!(state.owned_press.is_empty());
        state.engine.set_enabled(true);
        assert!(!state.input(InputKind::Down(c), None).consume);
    }

    #[test]
    fn successful_fallback_hands_held_press_back_to_physical_release() {
        let (tx, _) = mpsc::sync_channel(EVENT_CAPACITY);
        let mut state = ThreadState::new(tx);
        let caps = Button::Key(0x14);
        state
            .engine
            .replace_bindings(vec![TriggerSpec {
                conditions: Vec::new(),
                id: 1,
                held: vec![caps],
                activation: omnitrigger_core::Activation::Press(Button::Key(0x57)),
            }])
            .unwrap();
        state.input(InputKind::Down(caps), None);
        assert!(state.owned_press.contains(&caps));
        let fallback = state.input(InputKind::Down(Button::Key(0x43)), None);
        state.replay_succeeded(&fallback.replay_kinds);
        assert!(state.owned_press.is_empty());
        assert!(!state.input(InputKind::Up(caps), None).consume);
    }

    #[test]
    fn wheel_recording_waits_for_side_button_release() {
        let (tx, rx) = mpsc::sync_channel(EVENT_CAPACITY);
        let mut state = ThreadState::new(tx);
        state.command(BackendCommand::StartRecording);
        let side = Button::Mouse(MouseButton::X1);
        assert!(
            state
                .record_input(InputEvent {
                    kind: InputKind::Down(side),
                    injected: false
                })
                .consume
        );
        assert!(matches!(
            rx.try_recv(),
            Ok(BackendEvent::RecordingProgress(held)) if held == vec![side]
        ));
        assert!(
            state
                .record_input(InputEvent {
                    kind: InputKind::Wheel {
                        axis: Axis::Vertical,
                        delta: 30
                    },
                    injected: false
                })
                .consume
        );
        assert!(rx.try_recv().is_err());
        assert!(
            state
                .record_input(InputEvent {
                    kind: InputKind::Up(side),
                    injected: false
                })
                .consume
        );
        assert!(
            matches!(rx.try_recv(), Ok(BackendEvent::Recorded(spec)) if spec.held == vec![side] && spec.activation == omnitrigger_core::Activation::Wheel(omnitrigger_core::WheelDirection::Up))
        );
    }

    #[test]
    fn startup_held_press_remains_passed_during_recording() {
        let (tx, _) = mpsc::sync_channel(EVENT_CAPACITY);
        let mut state = ThreadState::new(tx);
        let caps = Button::Key(0x14);
        state.physical_held.insert(caps);
        state.engine.process(InputEvent {
            kind: InputKind::Down(caps),
            injected: false,
        });
        state.command(BackendCommand::StartRecording);
        assert!(
            !state
                .record_input(InputEvent {
                    kind: InputKind::Down(caps),
                    injected: false
                })
                .consume
        );
        assert!(
            !state
                .record_input(InputEvent {
                    kind: InputKind::Up(caps),
                    injected: false
                })
                .consume
        );
        assert!(state.recording_consumed.is_empty());
    }

    #[test]
    fn callback_diagnostics_classify_sources_without_changing_match_state() {
        let (tx, _) = mpsc::sync_channel(EVENT_CAPACITY);
        let mut state = ThreadState::new(tx);
        let caps = Button::Key(0x14);
        let w = Button::Key(0x57);
        state
            .engine
            .replace_bindings(vec![TriggerSpec {
                conditions: Vec::new(),
                id: 9,
                held: vec![caps],
                activation: omnitrigger_core::Activation::Press(w),
            }])
            .unwrap();
        assert!(
            state
                .engine
                .process(InputEvent {
                    kind: InputKind::Down(caps),
                    injected: false
                })
                .consume
        );
        assert!(state.diagnostics().last_keyboard_ms.is_none());
        assert!(state.diagnostics().last_mouse_ms.is_none());

        state.note_callback(true, false, PRIORITY_PROBE_TAG);
        state.note_callback(true, true, REPLAY_TAG);
        state.note_callback(true, true, SHORTCUT_TAG);
        state.note_callback(true, true, 0x12345);
        state.note_callback(false, true, PRIORITY_PROBE_TAG);
        state.note_callback(false, false, 0);

        let snapshot = state.diagnostics();
        assert_eq!(snapshot.keyboard_events, 4);
        assert_eq!(snapshot.mouse_events, 2);
        assert_eq!(snapshot.external_injected, 1);
        assert_eq!(snapshot.self_replayed, 1);
        assert_eq!(snapshot.self_shortcuts, 1);
        assert_eq!(snapshot.probe_events, 1);
        assert!(snapshot.last_keyboard_ms.is_some());
        assert!(snapshot.last_mouse_ms.is_some());
        assert_eq!(
            state
                .engine
                .process(InputEvent {
                    kind: InputKind::Down(w),
                    injected: false
                })
                .triggered,
            vec![9]
        );
    }

    #[test]
    fn diagnostics_snapshot_copies_held_and_owned_without_resetting_ledger() {
        let (tx, rx) = mpsc::sync_channel(EVENT_CAPACITY);
        let mut state = ThreadState::new(tx);
        let caps = Button::Key(0x14);
        let side = Button::Mouse(MouseButton::X1);
        state.physical_held.extend([caps, side]);
        state.owned_press.insert(caps);
        state.command(BackendCommand::GetDiagnostics);
        let Ok(BackendEvent::Diagnostics(mut snapshot)) = rx.try_recv() else {
            panic!("应返回诊断快照");
        };
        assert_eq!(snapshot.held, vec![caps, side]);
        assert_eq!(snapshot.owned, vec![caps]);
        snapshot.held.clear();
        snapshot.owned.clear();
        let unchanged = state.diagnostics();
        assert_eq!(unchanged.held, vec![caps, side]);
        assert_eq!(unchanged.owned, vec![caps]);
    }
}
