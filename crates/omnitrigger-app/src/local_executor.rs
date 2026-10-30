//! 本地动作在单个后台线程顺序执行；提交时冻结动作和原前台身份。
//! 取消只阻止后续步骤，不撤销已经执行的输入、剪贴板或打开操作。
use crate::local_action::{
    expand_template, extract_urls, search_url, strip_app_name, validate_text, LocalAction,
    LocalStep,
};
use omnitrigger_windows::ShortcutTarget;
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::TryRecvError,
        Arc, Condvar, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const CAPACITY: usize = 4;
const WAIT_SLICE: Duration = Duration::from_millis(25);

#[derive(Debug, PartialEq, Eq)]
pub struct LocalExecutionEvent {
    pub id: u64,
    pub invocation_id: u64,
    pub outcome: LocalExecutionOutcome,
}

#[derive(Debug, PartialEq, Eq)]
pub enum LocalExecutionOutcome {
    /// 步骤索引从 0 开始；此事件表示开始尝试，不代表已经成功。
    StepProgress {
        step_index: usize,
        description: String,
    },
    Finished {
        completed_steps: usize,
        summary: String,
    },
    Failed {
        completed_steps: usize,
        failed_step: Option<usize>,
        message: String,
    },
    Cancelled {
        completed_steps: usize,
        step_index: Option<usize>,
    },
}

impl LocalExecutionOutcome {
    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::StepProgress { .. })
    }
}

/// 仅拥有进程内队列；输入注入仍由 Windows 适配层统一串行化。
pub struct LocalExecutor {
    inner: Executor<ShortcutTarget>,
}

impl Default for LocalExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalExecutor {
    pub fn new() -> Self {
        Self {
            inner: Executor::with_host(NativeHost),
        }
    }

    pub fn submit(
        &self,
        id: u64,
        action: LocalAction,
        target: ShortcutTarget,
    ) -> Result<u64, String> {
        self.inner.submit(id, action, target)
    }

    pub fn try_recv(&self) -> Result<LocalExecutionEvent, TryRecvError> {
        self.inner.try_recv()
    }

    /// 包含未读取的终态；读取终态后才释放该绑定和执行配额。
    pub fn is_running(&self, id: u64) -> bool {
        self.inner.is_running(id)
    }

    pub fn cancel(&self, invocation_id: u64) -> bool {
        self.inner.cancel_matching(|id, _| id == invocation_id) > 0
    }

    pub fn cancel_id(&self, id: u64) -> usize {
        self.inner.cancel_matching(|_, binding_id| binding_id == id)
    }

    pub fn cancel_all(&self) -> usize {
        self.inner.cancel_matching(|_, _| true)
    }
}

/// 测试只使用替身，绝不读取真实剪贴板、发送输入或打开系统目标。
trait Host: Send + 'static {
    type Target: Copy + Send + 'static;
    fn ensure_target(&mut self, target: Self::Target) -> Result<(), String>;
    fn send_shortcut(&mut self, keys: &[u16], target: Self::Target) -> Result<(), String>;
    fn key_tap(&mut self, key: u16, target: Self::Target) -> Result<(), String>;
    fn key_sequence(
        &mut self,
        modifiers: &[u16],
        keys: &[u16],
        target: Self::Target,
    ) -> Result<(), String>;
    fn input_text(&mut self, text: &str, target: Self::Target) -> Result<(), String>;
    fn set_clipboard(&mut self, text: &str) -> Result<(), String>;
    fn read_clipboard(&mut self) -> Result<String, String>;
    fn open(&mut self, target: &str) -> Result<(), String>;
    fn copy_selection(&mut self, target: Self::Target) -> Result<String, String>;
    fn window_title(&mut self, target: Self::Target) -> Result<String, String>;
}

struct NativeHost;

impl Host for NativeHost {
    type Target = ShortcutTarget;

    fn ensure_target(&mut self, target: ShortcutTarget) -> Result<(), String> {
        omnitrigger_windows::ensure_shortcut_target(target)
    }

    fn send_shortcut(&mut self, keys: &[u16], target: ShortcutTarget) -> Result<(), String> {
        omnitrigger_windows::send_shortcut_to(keys, target)
    }

    fn key_tap(&mut self, key: u16, target: ShortcutTarget) -> Result<(), String> {
        omnitrigger_windows::send_key_tap_to(key, target)
    }

    fn key_sequence(
        &mut self,
        modifiers: &[u16],
        keys: &[u16],
        target: ShortcutTarget,
    ) -> Result<(), String> {
        omnitrigger_windows::send_key_sequence_to(modifiers, keys, target)
    }

    fn input_text(&mut self, text: &str, target: ShortcutTarget) -> Result<(), String> {
        omnitrigger_windows::send_text_to(text, target)
    }

    fn set_clipboard(&mut self, text: &str) -> Result<(), String> {
        omnitrigger_windows::write_clipboard_text(text)
    }

    fn read_clipboard(&mut self) -> Result<String, String> {
        omnitrigger_windows::read_clipboard_text()
    }

    fn open(&mut self, target: &str) -> Result<(), String> {
        omnitrigger_windows::open_target(target)
    }

    fn copy_selection(&mut self, target: ShortcutTarget) -> Result<String, String> {
        omnitrigger_windows::copy_selection_text(target)
    }

    fn window_title(&mut self, target: ShortcutTarget) -> Result<String, String> {
        omnitrigger_windows::shortcut_target_title(target)
    }
}

struct Request<T> {
    id: u64,
    invocation_id: u64,
    action: LocalAction,
    target: T,
    cancelled: Arc<AtomicBool>,
}

struct Reservation {
    id: u64,
    cancelled: Arc<AtomicBool>,
    terminal: bool,
}

struct State<T> {
    pending: VecDeque<Request<T>>,
    // 每次调用至多 MAX_STEPS 个进度和一个终态，且至多 CAPACITY 次未读终态。
    events: VecDeque<LocalExecutionEvent>,
    reservations: HashMap<u64, Reservation>,
    last_invocation_id: u64,
    alive: bool,
    stopping: bool,
}

struct Shared<T> {
    state: Mutex<State<T>>,
    ready: Condvar,
}

struct Executor<T> {
    shared: Arc<Shared<T>>,
    worker: Option<JoinHandle<()>>,
    startup_error: Option<String>,
}

impl<T: Copy + Send + 'static> Executor<T> {
    fn with_host<H: Host<Target = T>>(host: H) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                pending: VecDeque::with_capacity(CAPACITY),
                events: VecDeque::with_capacity(CAPACITY * (crate::local_action::MAX_STEPS + 1)),
                reservations: HashMap::with_capacity(CAPACITY),
                last_invocation_id: 0,
                alive: true,
                stopping: false,
            }),
            ready: Condvar::new(),
        });
        let worker_shared = shared.clone();
        let (worker, startup_error) = match thread::Builder::new()
            .name("omnitrigger-local-actions".into())
            .spawn(move || work(worker_shared, host))
        {
            Ok(worker) => (Some(worker), None),
            Err(error) => {
                if let Ok(mut state) = shared.state.lock() {
                    state.alive = false;
                }
                (None, Some(format!("无法启动本地动作线程：{error}")))
            }
        };
        Self {
            shared,
            worker,
            startup_error,
        }
    }

    fn submit(&self, id: u64, action: LocalAction, target: T) -> Result<u64, String> {
        action.validate()?;
        if let Some(error) = &self.startup_error {
            return Err(error.clone());
        }
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| "本地动作队列不可用。")?;
        if state.stopping || !state.alive {
            return Err("本地动作执行器已关闭。".into());
        }
        if state
            .reservations
            .values()
            .any(|reservation| reservation.id == id)
        {
            return Err("该绑定的上次本地动作尚未结束或结果尚未读取。".into());
        }
        if state.reservations.len() >= CAPACITY {
            return Err("本地动作队列已满，请稍后重试。".into());
        }
        let invocation_id = state
            .last_invocation_id
            .checked_add(1)
            .ok_or("本地动作调用编号已耗尽。")?;
        state.last_invocation_id = invocation_id;
        let cancelled = Arc::new(AtomicBool::new(false));
        state.reservations.insert(
            invocation_id,
            Reservation {
                id,
                cancelled: cancelled.clone(),
                terminal: false,
            },
        );
        state.pending.push_back(Request {
            id,
            invocation_id,
            action,
            target,
            cancelled,
        });
        drop(state);
        self.shared.ready.notify_one();
        Ok(invocation_id)
    }

    fn try_recv(&self) -> Result<LocalExecutionEvent, TryRecvError> {
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| TryRecvError::Disconnected)?;
        if let Some(event) = state.events.pop_front() {
            if event.outcome.is_terminal() {
                state.reservations.remove(&event.invocation_id);
            }
            return Ok(event);
        }
        if state.stopping || !state.alive {
            Err(TryRecvError::Disconnected)
        } else {
            Err(TryRecvError::Empty)
        }
    }

    fn is_running(&self, id: u64) -> bool {
        self.shared.state.lock().is_ok_and(|state| {
            state
                .reservations
                .values()
                .any(|reservation| reservation.id == id)
        })
    }

    fn cancel_matching(&self, matches: impl Fn(u64, u64) -> bool) -> usize {
        let Ok(state) = self.shared.state.lock() else {
            return 0;
        };
        state
            .reservations
            .iter()
            .filter(|(invocation_id, reservation)| {
                !reservation.terminal
                    && matches(**invocation_id, reservation.id)
                    && !reservation.cancelled.swap(true, Ordering::AcqRel)
            })
            .count()
    }
}

impl<T> Drop for Executor<T> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.shared.state.lock() {
            state.stopping = true;
            for reservation in state.reservations.values() {
                reservation.cancelled.store(true, Ordering::Release);
            }
            state.pending.clear();
            state.events.clear();
            state.reservations.clear();
        }
        self.shared.ready.notify_all();
        // 原生系统调用可能阻塞；不在界面销毁时等待它，也不强制终止线程。
        // 调用返回后，取消检查阻止后续步骤，工作线程自行退出。
        if let Some(worker) = self.worker.take().filter(JoinHandle::is_finished) {
            let _ = worker.join();
        }
    }
}

fn push_event<T>(shared: &Shared<T>, id: u64, invocation_id: u64, outcome: LocalExecutionOutcome) {
    let Ok(mut state) = shared.state.lock() else {
        return;
    };
    if state.stopping {
        return;
    }
    let Some(reservation) = state.reservations.get_mut(&invocation_id) else {
        return;
    };
    if reservation.terminal {
        return;
    }
    reservation.terminal = outcome.is_terminal();
    state.events.push_back(LocalExecutionEvent {
        id,
        invocation_id,
        outcome,
    });
}

fn work<H: Host>(shared: Arc<Shared<H::Target>>, mut host: H) {
    struct WorkerExit<T>(Arc<Shared<T>>);
    impl<T> Drop for WorkerExit<T> {
        fn drop(&mut self) {
            let Ok(mut state) = self.0.state.lock() else {
                return;
            };
            state.alive = false;
            state.pending.clear();
            if !state.stopping {
                let unfinished = state
                    .reservations
                    .iter_mut()
                    .filter_map(|(&invocation_id, reservation)| {
                        if reservation.terminal {
                            return None;
                        }
                        reservation.terminal = true;
                        Some((invocation_id, reservation.id))
                    })
                    .collect::<Vec<_>>();
                for (invocation_id, id) in unfinished {
                    state.events.push_back(LocalExecutionEvent {
                        id,
                        invocation_id,
                        outcome: LocalExecutionOutcome::Failed {
                            completed_steps: 0,
                            failed_step: None,
                            message: "本地动作工作线程已退出，未继续执行后续步骤。".into(),
                        },
                    });
                }
            }
        }
    }
    let _exit = WorkerExit(shared.clone());
    loop {
        let request = {
            let Ok(mut state) = shared.state.lock() else {
                return;
            };
            loop {
                if state.stopping {
                    return;
                }
                if let Some(request) = state.pending.pop_front() {
                    break request;
                }
                state = match shared.ready.wait(state) {
                    Ok(state) => state,
                    Err(_) => return,
                };
            }
        };
        let outcome = catch_unwind(AssertUnwindSafe(|| run(&shared, &request, &mut host)))
            .unwrap_or_else(|_| LocalExecutionOutcome::Failed {
                completed_steps: 0,
                failed_step: None,
                message: "本地动作执行异常，已停止后续步骤；已执行的操作不会撤销。".into(),
            });
        push_event(&shared, request.id, request.invocation_id, outcome);
    }
}

enum Stop {
    Cancelled,
    Failed(String),
}

fn guard<H: Host>(host: &mut H, request: &Request<H::Target>) -> Result<(), Stop> {
    if request.cancelled.load(Ordering::Acquire) {
        return Err(Stop::Cancelled);
    }
    host.ensure_target(request.target).map_err(Stop::Failed)?;
    // 目标检查期间也可能收到取消，执行前再确认一次。
    if request.cancelled.load(Ordering::Acquire) {
        Err(Stop::Cancelled)
    } else {
        Ok(())
    }
}

fn run<H: Host>(
    shared: &Shared<H::Target>,
    request: &Request<H::Target>,
    host: &mut H,
) -> LocalExecutionOutcome {
    let mut variables = BTreeMap::new();
    let mut completed_steps = 0;
    for (step_index, step) in request.action.steps.iter().enumerate() {
        let result = catch_unwind(AssertUnwindSafe(|| {
            guard(host, request).and_then(|()| {
                push_event(
                    shared,
                    request.id,
                    request.invocation_id,
                    LocalExecutionOutcome::StepProgress {
                        step_index,
                        description: step.description(),
                    },
                );
                execute_step(host, request, step, &mut variables)
            })
        }))
        .unwrap_or_else(|_| {
            Err(Stop::Failed(
                "本地动作执行异常，已停止后续步骤；已执行的操作不会撤销。".into(),
            ))
        });
        match result {
            Ok(()) => completed_steps += 1,
            Err(Stop::Cancelled) => {
                return LocalExecutionOutcome::Cancelled {
                    completed_steps,
                    step_index: Some(step_index),
                }
            }
            Err(Stop::Failed(message)) => {
                return LocalExecutionOutcome::Failed {
                    completed_steps,
                    failed_step: Some(step_index),
                    message,
                }
            }
        }
    }
    // 最后一步 Open 可以合法切换前台；完成后不尝试恢复原窗口。
    if request.cancelled.load(Ordering::Acquire) {
        LocalExecutionOutcome::Cancelled {
            completed_steps,
            step_index: None,
        }
    } else {
        LocalExecutionOutcome::Finished {
            completed_steps,
            summary: format!("已完成本地动作的 {completed_steps} 个步骤。"),
        }
    }
}

fn execute_step<H: Host>(
    host: &mut H,
    request: &Request<H::Target>,
    step: &LocalStep,
    variables: &mut BTreeMap<String, String>,
) -> Result<(), Stop> {
    // 进度事件入队也有同步边界；真正调用能力之前再次检查取消。
    guard(host, request)?;
    let result = match step {
        LocalStep::SendShortcut { keys } => host.send_shortcut(keys, request.target),
        LocalStep::KeyTap { key } => host.key_tap(*key, request.target),
        LocalStep::KeySequence { modifiers, keys } => {
            host.key_sequence(modifiers, keys, request.target)
        }
        LocalStep::InputText { text } => {
            let text = expand_template(text, variables).map_err(Stop::Failed)?;
            guard(host, request)?;
            host.input_text(&text, request.target)
        }
        LocalStep::SetClipboard { text } => {
            let text = expand_template(text, variables).map_err(Stop::Failed)?;
            guard(host, request)?;
            host.set_clipboard(&text)
        }
        LocalStep::ReadClipboard { variable } => host.read_clipboard().and_then(|value| {
            validate_text(&value)?;
            variables.insert(variable.clone(), value);
            Ok(())
        }),
        LocalStep::SetVariable { name, value } => expand_template(value, variables).map(|value| {
            variables.insert(name.clone(), value);
        }),
        LocalStep::Open { target } => {
            let target = expand_template(target, variables).map_err(Stop::Failed)?;
            omnitrigger_windows::validate_open_target(&target).map_err(Stop::Failed)?;
            guard(host, request)?;
            host.open(&target)
        }
        LocalStep::CopySelection { variable } => {
            host.copy_selection(request.target).and_then(|value| {
                validate_text(&value)?;
                variables.insert(variable.clone(), value);
                Ok(())
            })
        }
        LocalStep::ReadWindowTitle {
            variable,
            strip_app_name: strip,
        } => host.window_title(request.target).and_then(|title| {
            let title = if *strip {
                strip_app_name(&title)
            } else {
                &title
            };
            validate_text(title)?;
            variables.insert(variable.clone(), title.to_string());
            Ok(())
        }),
        LocalStep::OpenUrls { text } => {
            let text = expand_template(text, variables).map_err(Stop::Failed)?;
            let urls = extract_urls(&text);
            if urls.is_empty() {
                return Err(Stop::Failed("文本中没有找到网址。".into()));
            }
            // 首个网址打开后前台会切换，后续只检查取消，不再要求原窗口在前台。
            guard(host, request)?;
            for url in urls {
                if request.cancelled.load(Ordering::Acquire) {
                    return Err(Stop::Cancelled);
                }
                host.open(&url).map_err(Stop::Failed)?;
            }
            Ok(())
        }
        LocalStep::WebSearch { url, query } => {
            let query = expand_template(query, variables).map_err(Stop::Failed)?;
            let url = search_url(url, &query).map_err(Stop::Failed)?;
            guard(host, request)?;
            host.open(&url)
        }
        LocalStep::Delay { milliseconds } => {
            let started = Instant::now();
            let duration = Duration::from_millis(u64::from(*milliseconds));
            loop {
                guard(host, request)?;
                let remaining = duration.saturating_sub(started.elapsed());
                if remaining.is_zero() {
                    return Ok(());
                }
                thread::sleep(remaining.min(WAIT_SLICE));
            }
        }
    };
    result.map_err(Stop::Failed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{self, Receiver, Sender};

    const TIMEOUT: Duration = Duration::from_secs(3);

    #[derive(Default)]
    struct MockState {
        foreground: usize,
        clipboard: String,
        calls: Vec<String>,
        fail: Option<String>,
        switch_after: Option<String>,
        panic_on: Option<String>,
    }

    struct MockHost {
        state: Arc<Mutex<MockState>>,
        gate: Option<(Sender<()>, Receiver<()>)>,
        exited: Option<Sender<()>>,
    }

    impl Drop for MockHost {
        fn drop(&mut self) {
            if let Some(exited) = &self.exited {
                let _ = exited.send(());
            }
        }
    }

    impl MockHost {
        fn call(&mut self, operation: &str, value: &str) -> Result<(), String> {
            if let Some((entered, permit)) = self.gate.take() {
                entered.send(()).unwrap();
                permit.recv_timeout(TIMEOUT).unwrap();
            }
            let mut state = self.state.lock().unwrap();
            state.calls.push(format!("{operation}:{value}"));
            if state.panic_on.as_deref() == Some(operation) {
                state.panic_on = None;
                drop(state);
                panic!("模拟 Host 异常");
            }
            if state.fail.as_deref() == Some(operation) {
                return Err("模拟步骤失败".into());
            }
            if state.switch_after.as_deref() == Some(operation) {
                state.foreground = 2;
            }
            Ok(())
        }
    }

    impl Host for MockHost {
        type Target = usize;
        fn ensure_target(&mut self, target: usize) -> Result<(), String> {
            if self.state.lock().unwrap().foreground == target {
                Ok(())
            } else {
                Err("前台窗口已切换".into())
            }
        }
        fn send_shortcut(&mut self, _: &[u16], _: usize) -> Result<(), String> {
            self.call("shortcut", "")
        }
        fn key_tap(&mut self, _: u16, _: usize) -> Result<(), String> {
            self.call("tap", "")
        }
        fn key_sequence(&mut self, _: &[u16], _: &[u16], _: usize) -> Result<(), String> {
            self.call("sequence", "")
        }
        fn input_text(&mut self, text: &str, _: usize) -> Result<(), String> {
            self.call("text", text)
        }
        fn set_clipboard(&mut self, text: &str) -> Result<(), String> {
            self.call("write", text)?;
            self.state.lock().unwrap().clipboard = text.into();
            Ok(())
        }
        fn read_clipboard(&mut self) -> Result<String, String> {
            self.call("read", "")?;
            Ok(self.state.lock().unwrap().clipboard.clone())
        }
        fn open(&mut self, target: &str) -> Result<(), String> {
            self.call("open", target)
        }
        fn copy_selection(&mut self, _: usize) -> Result<String, String> {
            self.call("copy", "")?;
            Ok(self.state.lock().unwrap().clipboard.clone())
        }
        fn window_title(&mut self, _: usize) -> Result<String, String> {
            self.call("title", "")?;
            Ok("页面 - Google Chrome".into())
        }
    }

    fn mock() -> (MockHost, Arc<Mutex<MockState>>) {
        let state = Arc::new(Mutex::new(MockState {
            foreground: 1,
            ..Default::default()
        }));
        (
            MockHost {
                state: state.clone(),
                gate: None,
                exited: None,
            },
            state,
        )
    }

    fn action(steps: Vec<LocalStep>) -> LocalAction {
        LocalAction { version: 1, steps }
    }
    fn write(value: &str) -> LocalStep {
        LocalStep::SetClipboard { text: value.into() }
    }

    fn next(executor: &Executor<usize>) -> LocalExecutionEvent {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            match executor.try_recv() {
                Ok(event) => return event,
                Err(TryRecvError::Empty) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(1))
                }
                other => panic!("未收到动作事件：{other:?}"),
            }
        }
    }

    fn terminal(executor: &Executor<usize>) -> LocalExecutionEvent {
        loop {
            let event = next(executor);
            if event.outcome.is_terminal() {
                return event;
            }
        }
    }

    fn wait_terminals(executor: &Executor<usize>, count: usize) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if executor
                .shared
                .state
                .lock()
                .unwrap()
                .events
                .iter()
                .filter(|event| event.outcome.is_terminal())
                .count()
                == count
            {
                return;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn variables_flow_in_order_without_logging_clipboard_contents() {
        let (host, state) = mock();
        state.lock().unwrap().clipboard = "secret ${not_a_variable}".into();
        let executor = Executor::with_host(host);
        let invocation = executor
            .submit(
                7,
                action(vec![
                    LocalStep::ReadClipboard {
                        variable: "clip".into(),
                    },
                    LocalStep::SetVariable {
                        name: "message".into(),
                        value: "prefix ${clip} $$".into(),
                    },
                    LocalStep::InputText {
                        text: "${message}".into(),
                    },
                    write("${clip}"),
                ]),
                1,
            )
            .unwrap();
        loop {
            let event = next(&executor);
            assert_eq!(event.invocation_id, invocation);
            assert!(!format!("{event:?}").contains("secret"));
            if let LocalExecutionOutcome::Finished {
                completed_steps, ..
            } = event.outcome
            {
                assert_eq!(completed_steps, 4);
                break;
            }
        }
        assert_eq!(
            state.lock().unwrap().calls,
            [
                "read:",
                "text:prefix secret ${not_a_variable} $",
                "write:secret ${not_a_variable}"
            ]
        );
        assert!(!executor.is_running(7));
    }

    #[test]
    fn selection_and_title_feed_search_link_and_url_steps() {
        let (host, state) = mock();
        state.lock().unwrap().clipboard = "看 www.a.com 和 https://b.com/x。".into();
        let executor = Executor::with_host(host);
        executor
            .submit(
                3,
                action(vec![
                    LocalStep::CopySelection {
                        variable: "text".into(),
                    },
                    LocalStep::ReadWindowTitle {
                        variable: "title".into(),
                        strip_app_name: true,
                    },
                    LocalStep::WebSearch {
                        url: "https://s.com/?q=%s".into(),
                        query: "${title}".into(),
                    },
                    LocalStep::OpenUrls {
                        text: "${text}".into(),
                    },
                    write("[${title}](${text})"),
                ]),
                1,
            )
            .unwrap();
        let LocalExecutionOutcome::Finished {
            completed_steps, ..
        } = terminal(&executor).outcome
        else {
            panic!("动作应完成");
        };
        assert_eq!(completed_steps, 5);
        assert_eq!(
            state.lock().unwrap().calls,
            [
                "copy:",
                "title:",
                "open:https://s.com/?q=%E9%A1%B5%E9%9D%A2",
                "open:https://www.a.com",
                "open:https://b.com/x",
                "write:[页面](看 www.a.com 和 https://b.com/x。)"
            ]
        );
    }

    #[test]
    fn open_urls_without_any_url_fails_before_opening() {
        let (host, state) = mock();
        state.lock().unwrap().clipboard = "没有网址".into();
        let executor = Executor::with_host(host);
        executor
            .submit(
                4,
                action(vec![
                    LocalStep::CopySelection {
                        variable: "text".into(),
                    },
                    LocalStep::OpenUrls {
                        text: "${text}".into(),
                    },
                ]),
                1,
            )
            .unwrap();
        assert!(matches!(
            terminal(&executor).outcome,
            LocalExecutionOutcome::Failed {
                completed_steps: 1,
                failed_step: Some(1),
                ..
            }
        ));
        assert_eq!(state.lock().unwrap().calls, ["copy:"]);
    }

    #[test]
    fn step_failure_stops_later_operations_and_keeps_completed_count() {
        let (host, state) = mock();
        state.lock().unwrap().fail = Some("text".into());
        let executor = Executor::with_host(host);
        executor
            .submit(
                1,
                action(vec![
                    write("one"),
                    LocalStep::InputText { text: "two".into() },
                    write("three"),
                ]),
                1,
            )
            .unwrap();
        assert!(matches!(
            terminal(&executor).outcome,
            LocalExecutionOutcome::Failed {
                completed_steps: 1,
                failed_step: Some(1),
                ..
            }
        ));
        assert_eq!(state.lock().unwrap().calls, ["write:one", "text:two"]);
    }

    #[test]
    fn foreground_change_stops_following_steps_but_final_open_can_complete() {
        let (host, state) = mock();
        state.lock().unwrap().switch_after = Some("open".into());
        let executor = Executor::with_host(host);
        let open = LocalStep::Open {
            target: "https://example.com".into(),
        };
        executor
            .submit(1, action(vec![open.clone(), write("never")]), 1)
            .unwrap();
        assert!(matches!(
            terminal(&executor).outcome,
            LocalExecutionOutcome::Failed {
                completed_steps: 1,
                failed_step: Some(1),
                ..
            }
        ));
        state.lock().unwrap().foreground = 1;
        executor.submit(1, action(vec![open]), 1).unwrap();
        assert!(matches!(
            terminal(&executor).outcome,
            LocalExecutionOutcome::Finished {
                completed_steps: 1,
                ..
            }
        ));
        assert_eq!(state.lock().unwrap().calls.len(), 2);
    }

    #[test]
    fn cancel_interrupts_wait_and_pending_run_without_following_operations() {
        let (host, state) = mock();
        let executor = Executor::with_host(host);
        let first = executor
            .submit(
                1,
                action(vec![
                    LocalStep::Delay { milliseconds: 5000 },
                    write("never"),
                ]),
                1,
            )
            .unwrap();
        assert!(matches!(
            next(&executor).outcome,
            LocalExecutionOutcome::StepProgress { step_index: 0, .. }
        ));
        let second = executor
            .submit(2, action(vec![write("also never")]), 1)
            .unwrap();
        assert_eq!(
            executor.cancel_matching(|id, _| id == first || id == second),
            2
        );
        assert!(matches!(
            terminal(&executor).outcome,
            LocalExecutionOutcome::Cancelled {
                completed_steps: 0,
                step_index: Some(0)
            }
        ));
        assert!(matches!(
            terminal(&executor).outcome,
            LocalExecutionOutcome::Cancelled {
                completed_steps: 0,
                step_index: Some(0)
            }
        ));
        assert!(state.lock().unwrap().calls.is_empty());
    }

    #[test]
    fn foreground_change_during_wait_stops_before_next_operation() {
        let (host, state) = mock();
        let executor = Executor::with_host(host);
        executor
            .submit(
                1,
                action(vec![
                    LocalStep::Delay { milliseconds: 5000 },
                    write("never"),
                ]),
                1,
            )
            .unwrap();
        assert!(matches!(
            next(&executor).outcome,
            LocalExecutionOutcome::StepProgress { step_index: 0, .. }
        ));
        state.lock().unwrap().foreground = 2;
        assert!(matches!(
            terminal(&executor).outcome,
            LocalExecutionOutcome::Failed {
                completed_steps: 0,
                failed_step: Some(0),
                ..
            }
        ));
        assert!(state.lock().unwrap().calls.is_empty());
    }

    #[test]
    fn cancellation_after_inflight_operation_reports_partial_completion() {
        let (mut host, state) = mock();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (permit_tx, permit_rx) = mpsc::channel();
        host.gate = Some((entered_tx, permit_rx));
        let executor = Executor::with_host(host);
        let invocation_id = executor
            .submit(7, action(vec![write("started"), write("never")]), 1)
            .unwrap();
        entered_rx.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(
            executor.cancel_matching(|id, binding| id == invocation_id && binding == 7),
            1
        );
        assert_eq!(executor.cancel_matching(|_, _| true), 0);
        permit_tx.send(()).unwrap();
        assert!(matches!(
            terminal(&executor).outcome,
            LocalExecutionOutcome::Cancelled {
                completed_steps: 1,
                step_index: Some(1)
            }
        ));
        assert_eq!(state.lock().unwrap().calls, ["write:started"]);
    }

    #[test]
    fn all_key_forms_dispatch_in_saved_order() {
        let (host, state) = mock();
        let executor = Executor::with_host(host);
        executor
            .submit(
                1,
                action(vec![
                    LocalStep::SendShortcut {
                        keys: vec![0xa2, 0x41],
                    },
                    LocalStep::KeyTap { key: 0x12 },
                    LocalStep::KeySequence {
                        modifiers: vec![0xa2],
                        keys: vec![0x4b, 0x5a],
                    },
                ]),
                1,
            )
            .unwrap();
        assert!(matches!(
            terminal(&executor).outcome,
            LocalExecutionOutcome::Finished {
                completed_steps: 3,
                ..
            }
        ));
        assert_eq!(
            state.lock().unwrap().calls,
            ["shortcut:", "tap:", "sequence:"]
        );
    }

    #[test]
    fn dynamic_open_and_expansion_are_checked_before_host_dispatch() {
        let (host, state) = mock();
        state.lock().unwrap().clipboard = "relative.exe".into();
        let executor = Executor::with_host(host);
        executor
            .submit(
                1,
                action(vec![
                    LocalStep::ReadClipboard {
                        variable: "target".into(),
                    },
                    LocalStep::Open {
                        target: "${target}".into(),
                    },
                    write("never"),
                ]),
                1,
            )
            .unwrap();
        assert!(matches!(
            terminal(&executor).outcome,
            LocalExecutionOutcome::Failed {
                completed_steps: 1,
                failed_step: Some(1),
                ..
            }
        ));
        assert_eq!(state.lock().unwrap().calls, ["read:"]);
        state.lock().unwrap().clipboard = "a".repeat(crate::local_action::MAX_TEXT_UNITS);
        executor
            .submit(
                1,
                action(vec![
                    LocalStep::ReadClipboard {
                        variable: "value".into(),
                    },
                    LocalStep::InputText {
                        text: "${value}x".into(),
                    },
                ]),
                1,
            )
            .unwrap();
        assert!(matches!(
            terminal(&executor).outcome,
            LocalExecutionOutcome::Failed {
                completed_steps: 1,
                failed_step: Some(1),
                ..
            }
        ));
        assert_eq!(state.lock().unwrap().calls, ["read:", "read:"]);
    }

    #[test]
    fn oversized_clipboard_value_is_rejected_before_use() {
        let (host, state) = mock();
        state.lock().unwrap().clipboard = "😀".repeat(crate::local_action::MAX_TEXT_UNITS / 2 + 1);
        let executor = Executor::with_host(host);
        executor
            .submit(
                1,
                action(vec![
                    LocalStep::ReadClipboard {
                        variable: "clip".into(),
                    },
                    LocalStep::InputText {
                        text: "${clip}".into(),
                    },
                ]),
                1,
            )
            .unwrap();
        assert!(matches!(
            terminal(&executor).outcome,
            LocalExecutionOutcome::Failed {
                completed_steps: 0,
                failed_step: Some(0),
                ..
            }
        ));
        assert_eq!(state.lock().unwrap().calls, ["read:"]);
    }

    #[test]
    fn capacity_includes_running_pending_and_unread_terminal_results() {
        let (mut host, _) = mock();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (permit_tx, permit_rx) = mpsc::channel();
        host.gate = Some((entered_tx, permit_rx));
        let executor = Executor::with_host(host);
        let first = executor.submit(1, action(vec![write("one")]), 1).unwrap();
        entered_rx.recv_timeout(TIMEOUT).unwrap();
        for id in 2..=CAPACITY as u64 {
            executor.submit(id, action(vec![write("next")]), 1).unwrap();
        }
        assert!(executor.submit(99, action(vec![write("full")]), 1).is_err());
        assert!(executor
            .submit(1, action(vec![write("duplicate")]), 1)
            .is_err());
        permit_tx.send(()).unwrap();
        wait_terminals(&executor, CAPACITY);
        assert!(executor
            .submit(99, action(vec![write("still full")]), 1)
            .is_err());
        let progress = next(&executor);
        assert!(!progress.outcome.is_terminal());
        assert!(executor
            .submit(99, action(vec![write("still full")]), 1)
            .is_err());
        assert_eq!(next(&executor).invocation_id, first);
        let next_id = executor.submit(99, action(vec![write("room")]), 1).unwrap();
        assert!(next_id > first);
        for _ in 0..CAPACITY {
            terminal(&executor);
        }
        assert!(executor
            .shared
            .state
            .lock()
            .unwrap()
            .reservations
            .is_empty());
    }

    #[test]
    fn dropping_executor_cancels_later_steps_and_worker_exits_after_blocking_host() {
        let (mut host, state) = mock();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (permit_tx, permit_rx) = mpsc::channel();
        let (exited_tx, exited_rx) = mpsc::channel();
        host.gate = Some((entered_tx, permit_rx));
        host.exited = Some(exited_tx);
        let executor = Executor::with_host(host);
        executor
            .submit(1, action(vec![write("started"), write("never")]), 1)
            .unwrap();
        entered_rx.recv_timeout(TIMEOUT).unwrap();
        executor
            .submit(2, action(vec![write("pending")]), 1)
            .unwrap();
        drop(executor);
        permit_tx.send(()).unwrap();
        exited_rx.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(state.lock().unwrap().calls, ["write:started"]);
    }

    #[test]
    fn panicking_host_has_terminal_failure_and_worker_accepts_next_run() {
        let (host, state) = mock();
        state.lock().unwrap().panic_on = Some("write".into());
        let executor = Executor::with_host(host);
        executor
            .submit(1, action(vec![write("panic"), write("never")]), 1)
            .unwrap();
        assert!(matches!(
            terminal(&executor).outcome,
            LocalExecutionOutcome::Failed { .. }
        ));
        executor.submit(2, action(vec![write("next")]), 1).unwrap();
        assert!(matches!(
            terminal(&executor).outcome,
            LocalExecutionOutcome::Finished {
                completed_steps: 1,
                ..
            }
        ));
        assert_eq!(state.lock().unwrap().calls, ["write:panic", "write:next"]);
    }

    #[test]
    fn invalid_action_never_reserves_capacity_or_touches_host() {
        let (host, state) = mock();
        let executor = Executor::with_host(host);
        assert!(executor
            .submit(
                1,
                action(vec![LocalStep::InputText {
                    text: "${missing}".into()
                }]),
                1
            )
            .is_err());
        assert!(!executor.is_running(1));
        assert!(state.lock().unwrap().calls.is_empty());
    }
}
