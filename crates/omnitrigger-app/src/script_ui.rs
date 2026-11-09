//! 脚本选择：工作线程有界等待，GPUI 线程创建、更新和关闭窗口。
use crate::{message_input::MessageInput, theme};
use gpui::{
    div, prelude::*, px, size, App, Bounds, Context, Entity, Focusable, KeyDownEvent, ScrollHandle,
    TitlebarOptions, Window, WindowBounds, WindowHandle, WindowOptions,
};
use omnitrigger_actions::action_hint::{
    fault as hint_fault, ScopedActionHint, METHOD as ACTION_HINT_METHOD,
    SCOPED_METHOD as SCOPED_ACTION_HINT_METHOD,
};
use omnitrigger_actions::{ActionError, CancellationToken, NativeHost};
use omnitrigger_windows::ScriptTargetSnapshot;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

const POLL: Duration = Duration::from_millis(20);
static BROKER: OnceLock<Mutex<Option<Broker>>> = OnceLock::new();

#[derive(Clone)]
struct Broker {
    sender: SyncSender<UiRequest>,
    busy: Arc<AtomicBool>,
    workspace: Arc<Mutex<Option<WindowHandle<crate::Workspace>>>>,
}

fn fault(code: &str, message: &str) -> ActionError {
    let mut error = ActionError::new(code, message);
    error.method = Some("ui.choose".into());
    error.effect_state = Some("not_started".into());
    error
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Item {
    id: String,
    label: String,
    #[serde(default)]
    description: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Options {
    items: Vec<Item>,
    #[serde(default = "default_title")]
    title: String,
    #[serde(default = "default_placeholder")]
    placeholder: String,
    #[serde(default = "default_timeout")]
    timeout_ms: u64,
}

fn default_title() -> String {
    "选择一项".into()
}
fn default_placeholder() -> String {
    "搜索".into()
}
fn default_timeout() -> u64 {
    30_000
}

fn text_valid(text: &str, max: usize) -> bool {
    !text.trim().is_empty()
        && text.encode_utf16().count() <= max
        && !text.chars().any(char::is_control)
        && !text.contains(['\u{2028}', '\u{2029}'])
}

impl Options {
    fn parse(params: Value) -> Result<Self, ActionError> {
        let invalid = || {
            fault(
                "invalid_input",
                "选择参数无效：需要 1–200 个唯一 ID 的候选项，文本总量最多 32 KiB，时限 1–60000 毫秒。",
            )
        };
        let size = serde_json::to_vec(&params).map_err(|_| invalid())?.len();
        let options: Self = serde_json::from_value(params).map_err(|_| invalid())?;
        let mut ids = HashSet::new();
        if size > 32 * 1024
            || !(1..=200).contains(&options.items.len())
            || !(1..=60_000).contains(&options.timeout_ms)
            || !text_valid(&options.title, 128)
            || !text_valid(&options.placeholder, 128)
            || options.items.iter().any(|item| {
                !text_valid(&item.id, 128)
                    || !text_valid(&item.label, 256)
                    || (!item.description.is_empty() && !text_valid(&item.description, 512))
                    || !ids.insert(&item.id)
            })
        {
            return Err(invalid());
        }
        Ok(options)
    }
}

struct Request {
    options: Options,
    response: SyncSender<Value>,
    abandoned: Arc<AtomicBool>,
    target: Option<ScriptTargetSnapshot>,
}

struct Waiting {
    abandoned: Arc<AtomicBool>,
    busy: Arc<AtomicBool>,
}
impl Drop for Waiting {
    fn drop(&mut self) {
        self.abandoned.store(true, Ordering::Release);
        self.busy.store(false, Ordering::Release);
    }
}

/// 包住既有桌面/浏览器宿主，不重新捕获目标，不让选择逻辑进入 Hook。
pub struct Host<H> {
    inner: H,
    target: Option<ScriptTargetSnapshot>,
}
impl<H> Host<H> {
    pub fn new(inner: H, target: Option<ScriptTargetSnapshot>) -> Self {
        Self { inner, target }
    }
}
impl<H: NativeHost> NativeHost for Host<H> {
    fn create_owned_left_input(
        &self,
        owner: omnitrigger_actions::owned_mouse::OwnedInputOwner,
    ) -> Result<std::sync::Arc<omnitrigger_actions::owned_mouse::OwnedLeftSession>, ActionError>
    {
        self.inner.create_owned_left_input(owner)
    }

    fn call(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
    ) -> Result<Value, ActionError> {
        self.call_with_deadline(
            method,
            params,
            cancellation,
            Instant::now() + Duration::from_secs(60),
        )
    }
    fn call_with_deadline(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<Value, ActionError> {
        if method == "system.open" {
            crate::script_open::open(params, cancellation, deadline)
        } else if method == "system.open_file_in_parent" {
            crate::script_open::open_file_in_parent(params, cancellation, deadline)
        } else if method == "fs.stat" {
            crate::script_fs::stat(params, cancellation, deadline)
        } else if method == "fs.sort_logical_paths" {
            crate::script_fs::sort_logical_paths(params, cancellation, deadline)
        } else if method == "fs.read_directory" {
            crate::script_fs::read_directory(params, cancellation, deadline)
        } else if method == "ui.notify" {
            crate::script_notify::notify(params, cancellation, deadline)
        } else if method == "ui.open_history" {
            open_history(params, cancellation, deadline, self.target)
        } else if method == "ui.toggle_history" {
            toggle_history(params, cancellation, deadline, self.target)
        } else if method == SCOPED_ACTION_HINT_METHOD {
            set_action_hint(params, cancellation, deadline)
        } else if method == ACTION_HINT_METHOD {
            Err(hint_fault("ui_hint_scope_expired", "not_started"))
        } else if method == "ui.choose" {
            choose(params, cancellation, deadline, self.target)
        } else {
            self.inner
                .call_with_deadline(method, params, cancellation, deadline)
        }
    }
}

const ACTION_HINT_LIMIT: Duration = Duration::from_millis(1000);

pub fn attach_workspace(handle: WindowHandle<crate::Workspace>) {
    if let Some(broker) = BROKER.get().and_then(|value| value.lock().ok()?.clone()) {
        *broker
            .workspace
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(handle);
    }
}

#[derive(Clone)]
enum ActionHintPhase {
    Pending,
    Applied,
    Failed(ActionError),
}
struct ActionHintSubmission {
    phase: Mutex<ActionHintPhase>,
}
impl ActionHintSubmission {
    fn new() -> Self {
        Self {
            phase: Mutex::new(ActionHintPhase::Pending),
        }
    }
    fn result(
        &self,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Option<Result<Value, ActionError>> {
        let mut phase = self.phase.lock().unwrap_or_else(|error| error.into_inner());
        if let ActionHintPhase::Failed(error) = &*phase {
            return Some(Err(error.clone()));
        }
        let interrupted = if cancellation.is_cancelled() {
            Some("cancelled")
        } else if Instant::now() >= deadline {
            Some("timeout")
        } else {
            None
        };
        if let Some(code) = interrupted {
            let error = hint_fault(
                code,
                if matches!(*phase, ActionHintPhase::Pending) {
                    "not_started"
                } else {
                    "unknown"
                },
            );
            if matches!(*phase, ActionHintPhase::Pending) {
                *phase = ActionHintPhase::Failed(error.clone());
            }
            return Some(Err(error));
        }
        if matches!(*phase, ActionHintPhase::Applied) {
            Some(Ok(json!({"updated":true})))
        } else {
            None
        }
    }
    fn apply(
        &self,
        cancellation: &CancellationToken,
        deadline: Instant,
        update: impl FnOnce() -> Result<(), ActionError>,
    ) {
        let mut phase = self.phase.lock().unwrap_or_else(|error| error.into_inner());
        if !matches!(*phase, ActionHintPhase::Pending) {
            return;
        }
        let interrupted = if cancellation.is_cancelled() {
            Some("cancelled")
        } else if Instant::now() >= deadline {
            Some("timeout")
        } else {
            None
        };
        if let Some(code) = interrupted {
            *phase = ActionHintPhase::Failed(hint_fault(code, "not_started"));
            return;
        }
        *phase = match update() {
            Ok(()) => ActionHintPhase::Applied,
            Err(error) => ActionHintPhase::Failed(error),
        };
    }
}
struct ActionHintRequest {
    update: ScopedActionHint,
    owner: WindowHandle<crate::Workspace>,
    submission: Arc<ActionHintSubmission>,
    cancellation: CancellationToken,
    deadline: Instant,
}

fn set_action_hint(
    params: Value,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<Value, ActionError> {
    let update = ScopedActionHint::parse(params)?;
    let deadline = deadline.min(Instant::now() + ACTION_HINT_LIMIT);
    let submission = Arc::new(ActionHintSubmission::new());
    if let Some(result) = submission.result(cancellation, deadline) {
        return result;
    }
    let broker = BROKER
        .get()
        .and_then(|value| value.lock().ok()?.clone())
        .ok_or_else(|| hint_fault("ui_unavailable", "not_started"))?;
    let owner = (*broker
        .workspace
        .lock()
        .unwrap_or_else(|error| error.into_inner()))
    .ok_or_else(|| hint_fault("ui_unavailable", "not_started"))?;
    broker
        .sender
        .try_send(UiRequest::ActionHint(ActionHintRequest {
            update,
            owner,
            submission: submission.clone(),
            cancellation: cancellation.clone(),
            deadline,
        }))
        .map_err(|error| {
            hint_fault(
                if matches!(error, mpsc::TrySendError::Full(_)) {
                    "ui_busy"
                } else {
                    "ui_unavailable"
                },
                "not_started",
            )
        })?;
    loop {
        if let Some(result) = submission.result(cancellation, deadline) {
            return result;
        }
        std::thread::sleep(POLL);
    }
}

fn dispatch_action_hint(request: ActionHintRequest, cx: &mut App) {
    let ActionHintRequest {
        update,
        owner,
        submission,
        cancellation,
        deadline,
    } = request;
    submission.apply(&cancellation, deadline, || {
        owner
            .update(cx, |workspace, _, cx| -> Result<(), ActionError> {
                workspace.package_feedback.set_action_hint(&update)?;
                cx.notify();
                Ok(())
            })
            .map_err(|_| hint_fault("ui_unavailable", "not_started"))?
    });
}

fn choose(
    params: Value,
    cancellation: &CancellationToken,
    deadline: Instant,
    target: Option<ScriptTargetSnapshot>,
) -> Result<Value, ActionError> {
    let options = Options::parse(params)?;
    if cancellation.is_cancelled() {
        return Err(fault("cancelled", "动作已取消。"));
    }
    if Instant::now() >= deadline {
        return Err(fault("timeout", "动作超过运行时限。"));
    }
    let broker = BROKER
        .get()
        .and_then(|broker| broker.lock().ok()?.clone())
        .ok_or_else(|| {
            fault(
                "ui_unavailable",
                "选择窗口需要正在运行的 OmniTrigger 界面。",
            )
        })?;
    if broker
        .busy
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(fault("ui_busy", "已有一个选择窗口，请先完成或取消。"));
    }
    let waiting = Waiting {
        abandoned: Arc::new(AtomicBool::new(false)),
        busy: broker.busy,
    };
    let choice_deadline = Instant::now() + Duration::from_millis(options.timeout_ms);
    let (response, receiver) = mpsc::sync_channel(1);
    broker
        .sender
        .try_send(UiRequest::Choose(Request {
            options,
            response,
            abandoned: waiting.abandoned.clone(),
            target,
        }))
        .map_err(|_| fault("ui_unavailable", "选择窗口连接已关闭。"))?;
    wait_for_choice(receiver, cancellation, deadline, choice_deadline)
}

fn wait_for_choice(
    receiver: Receiver<Value>,
    cancellation: &CancellationToken,
    deadline: Instant,
    choice_deadline: Instant,
) -> Result<Value, ActionError> {
    loop {
        if cancellation.is_cancelled() {
            return Err(fault("cancelled", "动作已取消，选择窗口已请求关闭。"));
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(fault("timeout", "动作超过运行时限，选择窗口已请求关闭。"));
        }
        if now >= choice_deadline {
            return Ok(json!({"status":"cancelled","reason":"timeout"}));
        }
        match receiver.recv_timeout(POLL.min(deadline - now).min(choice_deadline - now)) {
            Ok(value) => {
                // 完成与取消竞争时，以取消/时限为准，避免动作继续提交输入。
                if cancellation.is_cancelled() {
                    return Err(fault("cancelled", "动作已取消。"));
                }
                if Instant::now() >= deadline {
                    return Err(fault("timeout", "动作超过运行时限。"));
                }
                if Instant::now() >= choice_deadline {
                    return Ok(json!({"status":"cancelled","reason":"timeout"}));
                }
                return Ok(value);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(fault("ui_unavailable", "选择窗口宿主已退出。"));
            }
        }
    }
}

/// 仅在 GPUI 线程初始化。接收器随 App 任务销毁，等待中的动作会立即得知宿主退出。
pub fn init(cx: &mut App) {
    crate::script_notify::init(cx);
    let (sender, receiver) = mpsc::sync_channel(1);
    let broker = Broker {
        sender,
        busy: Arc::new(AtomicBool::new(false)),
        workspace: Arc::new(Mutex::new(None)),
    };
    *BROKER
        .get_or_init(|| Mutex::new(None))
        .lock()
        .expect("script UI broker") = Some(broker);
    cx.spawn(async move |cx| {
        let mut active: Option<(WindowHandle<Chooser>, Arc<AtomicBool>)> = None;
        let mut history_active: Option<(WindowHandle<crate::Workspace>, HistorySlot)> = None;
        let mut history_toggle: Option<HistoryToggleRequest> = None;
        let mut deferred: Option<UiRequest> = None;
        loop {
            cx.background_executor().timer(POLL).await;
            let mut chooser_activation = None;
            if cx
                .update(|cx| {
                    if let Some((handle, _)) = &history_active {
                        let closed = handle.update(cx, |view, window, _| {
                            if view.text_history.popup_close_ready() {
                                window.remove_window();
                                true
                            } else { false }
                        });
                        match closed {
                            Ok(true) => {
                                // 先移除本窗并释放其原有槽，之后才确认关闭。
                                history_active.take();
                                if let Some(request) = history_toggle.take() {
                                    finish_history_toggle(&request, HistoryTogglePhase::Closed);
                                }
                            }
                            Ok(false) => {}
                            Err(_) => {
                                history_active.take();
                                if let Some(request) = history_toggle.take() {
                                    finish_history_toggle(&request, HistoryTogglePhase::Failed(
                                        history_toggle_fault("ui_unavailable", "unknown"),
                                    ));
                                }
                            }
                        }
                    }
                    if active
                        .as_ref()
                        .is_some_and(|(_, abandoned)| abandoned.load(Ordering::Acquire))
                    {
                        if let Some((handle, _)) = active.take() {
                            let _ = handle.update(cx, |this, window, _| this.close(window));
                        }
                    }
                    if active.is_some() {
                        // 切换必须能在chooser占用期间明确拒绝，不能等它关闭后再开历史窗。
                        // 既有打开/选择请求仍延迟到原chooser收尾，不改变其契约。
                        if deferred.is_none() {
                            if let Ok(request) = receiver.try_recv() {
                                match request {
                                    UiRequest::ActionHint(request) => dispatch_action_hint(request, cx),
                                    UiRequest::ToggleHistory(request) => finish_history_toggle(
                                        &request,
                                        HistoryTogglePhase::Failed(history_toggle_fault("ui_busy", "not_started")),
                                    ),
                                    request => deferred = Some(request),
                                }
                            }
                        }
                        return;
                    }
                    let Some(request) = deferred.take().or_else(|| receiver.try_recv().ok()) else {
                        return;
                    };
                    let request = match request {
                        UiRequest::ActionHint(request) => { dispatch_action_hint(request, cx); return; }
                        UiRequest::Choose(request) => request,
                        UiRequest::ToggleHistory(request) => {
                            dispatch_history_toggle(request, &mut history_active, &mut history_toggle, cx);
                            return;
                        }
                        UiRequest::History(request) => {
                            let target = match crate::clipboard_history_send::HistoryLaunchTarget::from_host_snapshot(request.target, request.accepted) {
                                Ok(target) => target,
                                Err(_) => {
                                    let mut phase = request.phase.lock().unwrap_or_else(|error| error.into_inner());
                                    if matches!(*phase, HistoryOpenPhase::Pending) {
                                        *phase = HistoryOpenPhase::Failed(history_fault("target_changed", "not_started"));
                                    }
                                    let _ = request.changed.try_send(());
                                    return;
                                }
                            };
                            if !history_begin(&request.phase, &request.cancellation, request.deadline) {
                                let _ = request.changed.try_send(());
                                return;
                            }
                            let result = crate::open_script_history_window(target, cx)
                                .map_err(|_| history_fault("ui_unavailable", "unknown"));
                            let mut phase = request.phase.lock().unwrap_or_else(|error| error.into_inner());
                            match result {
                                Ok(handle) => {
                                    history_active = Some((handle, request.slot));
                                    *phase = HistoryOpenPhase::Opened;
                                }
                                Err(error) => *phase = HistoryOpenPhase::Failed(error),
                            }
                            drop(phase);
                            let _ = request.changed.try_send(());
                            return;
                        }
                    };
                    if request.abandoned.load(Ordering::Acquire) {
                        return;
                    }
                    let abandoned = request.abandoned.clone();
                    let target = request.target;
                    let mut chooser_hwnd = None;
                    let bounds = Bounds::centered(None, size(px(520.), px(480.)), cx);
                    if let Ok(handle) = cx.open_window(
                        WindowOptions {
                            window_bounds: Some(WindowBounds::Windowed(bounds)),
                            titlebar: Some(TitlebarOptions {
                                title: Some(request.options.title.clone().into()),
                                ..Default::default()
                            }),
                            is_resizable: false,
                            is_minimizable: false,
                            focus: true,
                            app_id: Some("OmniTrigger.ScriptChoice".into()),
                            ..Default::default()
                        },
                        |window, cx| {
                            chooser_hwnd = HasWindowHandle::window_handle(&*window)
                                .ok()
                                .and_then(|handle| match handle.as_raw() {
                                    RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
                                    _ => None,
                                });
                            cx.new(|cx| Chooser::new(request, window, cx))
                        },
                    ) {
                        chooser_activation = chooser_hwnd.map(|hwnd| (hwnd, target, abandoned.clone()));
                        active = Some((handle, abandoned));
                    }
                })
                .is_err()
            {
                break;
            }
            // GPUI 0.2.2 的 Windows 创建路径未使用 focus 参数；显示窗口不保证取得前台。
            // 原生激活会同步发送窗口消息，必须在上述 GPUI 状态借用结束后执行。
            if let Some((hwnd, Some(snapshot), abandoned)) = chooser_activation {
                if !abandoned.load(Ordering::Acquire)
                    && (omnitrigger_windows::is_foreground_window(hwnd)
                        || omnitrigger_windows::ScriptTarget::from_snapshot(snapshot)
                            .is_ok_and(|target| target.ensure_foreground().is_ok()))
                {
                    // 只请求一次；用户已切到第三窗口时跳过，不通过模拟按键争夺前台。
                    let _ = omnitrigger_windows::activate_panel_window(hwnd);
                }
            }
        }
    })
    .detach();
}

fn filtered(items: &[Item], query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            (query.is_empty()
                || format!("{} {} {}", item.id, item.label, item.description)
                    .to_lowercase()
                    .contains(&query))
            .then_some(index)
        })
        .collect()
}

struct Chooser {
    request: Request,
    search: Entity<MessageInput>,
    visible: Vec<usize>,
    selected: usize,
    scroll: ScrollHandle,
    _search_changed: gpui::Subscription,
}
impl Chooser {
    fn new(request: Request, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| {
            let mut input = MessageInput::new(cx).with_character_limit(256);
            input.set_placeholder(request.options.placeholder.clone());
            input
        });
        let changed = cx.observe(&search, |this, search, cx| {
            this.visible = filtered(&this.request.options.items, &search.read(cx).text());
            this.selected = 0;
            this.scroll.scroll_to_item(0);
            cx.notify();
        });
        search.read(cx).focus_handle(cx).focus(window);
        let closing = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            let _ = closing.update(cx, |this, _| {
                this.complete(json!({"status":"cancelled","reason":"dismissed"}), window)
            });
            false
        });
        Self {
            visible: (0..request.options.items.len()).collect(),
            request,
            search,
            selected: 0,
            scroll: ScrollHandle::new(),
            _search_changed: changed,
        }
    }

    fn close(&self, window: &mut Window) {
        // 用户已切到其他应用时不抢回焦点。恢复前重新核验原窗口和进程创建时间。
        let handle = HasWindowHandle::window_handle(&*window).map(|handle| handle.as_raw());
        if let (Ok(RawWindowHandle::Win32(handle)), Some(snapshot)) = (handle, self.request.target)
        {
            if let Ok(target) = omnitrigger_windows::ScriptTarget::from_snapshot(snapshot) {
                let _ = target.restore_foreground_if(handle.hwnd.get());
            }
        }
        window.remove_window();
    }

    fn complete(&self, result: Value, window: &mut Window) {
        // try_send 永不阻塞 UI；接收方超时/退出时结果直接丢弃。
        if !self.request.abandoned.swap(true, Ordering::AcqRel) {
            self.close(window);
            let _ = self.request.response.try_send(result);
        }
    }

    fn select(&self, index: usize, window: &mut Window) {
        if let Some(item) = self.request.options.items.get(index) {
            self.complete(json!({"status":"selected","id":item.id}), window);
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.modifiers.control
            || event.keystroke.modifiers.alt
            || event.keystroke.modifiers.platform
            || event.keystroke.modifiers.function
        {
            return;
        }
        match event.keystroke.key.as_str() {
            "escape" => self.complete(json!({"status":"cancelled","reason":"dismissed"}), window),
            "enter" => {
                if let Some(index) = self.visible.get(self.selected) {
                    self.select(*index, window);
                }
            }
            "up" | "down" => {
                if self.visible.is_empty() {
                    return;
                }
                let len = self.visible.len();
                self.selected = (self.selected
                    + if event.keystroke.key == "up" {
                        len - 1
                    } else {
                        1
                    })
                    % len;
                self.scroll.scroll_to_item(self.selected);
                cx.notify();
            }
            _ => return,
        }
        cx.stop_propagation();
    }
}
impl Render for Chooser {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(theme::rgb(theme::BACKGROUND))
            .text_color(theme::rgb(theme::TEXT))
            .flex()
            .flex_col()
            .gap(px(12.))
            .p(px(18.))
            .on_key_down(cx.listener(Self::key_down))
            .child(
                div()
                    .text_size(px(18.))
                    .child(self.request.options.title.clone()),
            )
            .child(self.search.clone())
            .child(
                div()
                    .id("script-choice-items")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .children(self.visible.iter().enumerate().map(|(slot, index)| {
                        let index = *index;
                        let item = &self.request.options.items[index];
                        div()
                            .id(("script-choice-item", index))
                            .p(px(10.))
                            .rounded(px(6.))
                            .cursor_pointer()
                            .when(slot == self.selected, |el| {
                                el.bg(theme::rgb(theme::SELECTED))
                            })
                            .hover(|style| style.bg(theme::rgb(theme::SURFACE_HI)))
                            .child(div().text_size(px(14.)).child(item.label.clone()))
                            .when(!item.description.is_empty(), |el| {
                                el.child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(theme::rgb(theme::TEXT_MUTED))
                                        .child(item.description.clone()),
                                )
                            })
                            .on_click(
                                cx.listener(move |this, _, window, _| this.select(index, window)),
                            )
                    }))
                    .when(self.visible.is_empty(), |el| {
                        el.child(div().p(px(10.)).child("没有匹配项"))
                    }),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(theme::rgb(theme::TEXT_MUTED))
                    .child("↑↓ 选择 · Enter 确定 · Esc 取消"),
            )
    }
}

enum UiRequest {
    ActionHint(ActionHintRequest),
    Choose(Request),
    History(HistoryRequest),
    ToggleHistory(HistoryToggleRequest),
}

struct HistorySlot(Arc<AtomicBool>);
impl Drop for HistorySlot {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

enum HistoryOpenPhase {
    Pending,
    Creating,
    Opened,
    Failed(ActionError),
}

struct HistoryRequest {
    target: ScriptTargetSnapshot,
    accepted: Instant,
    cancellation: CancellationToken,
    deadline: Instant,
    phase: Arc<Mutex<HistoryOpenPhase>>,
    changed: SyncSender<()>,
    slot: HistorySlot,
}

fn history_fault(code: &str, effect: &str) -> ActionError {
    let mut error = ActionError::new(
        code,
        match code {
            "invalid_input" => "打开历史窗口不接受参数。",
            "ui_busy" => "历史窗口或选择窗口尚未关闭，请先完成当前操作。",
            "target_unavailable" | "target_changed" => "本次原目标不可用，未改选当前前台。",
            "cancelled" => "打开历史窗口已取消。",
            "timeout" => "历史窗口创建未在等待时限内确认，不会自动重试。",
            _ => "历史窗口需要正在运行的OmniTrigger界面。",
        },
    );
    error.method = Some("ui.open_history".into());
    error.effect_state = Some(effect.into());
    error
}

fn history_begin(
    phase: &Mutex<HistoryOpenPhase>,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> bool {
    let mut phase = phase.lock().unwrap_or_else(|error| error.into_inner());
    if !matches!(*phase, HistoryOpenPhase::Pending) {
        return false;
    }
    let error = if cancellation.is_cancelled() {
        Some(history_fault("cancelled", "not_started"))
    } else if Instant::now() >= deadline {
        Some(history_fault("timeout", "not_started"))
    } else {
        None
    };
    if let Some(error) = error {
        *phase = HistoryOpenPhase::Failed(error);
        false
    } else {
        *phase = HistoryOpenPhase::Creating;
        true
    }
}

fn open_history(
    params: Value,
    cancellation: &CancellationToken,
    deadline: Instant,
    target: Option<ScriptTargetSnapshot>,
) -> Result<Value, ActionError> {
    if params != json!({}) {
        return Err(history_fault("invalid_input", "not_started"));
    }
    let accepted = Instant::now();
    let deadline = deadline.min(accepted + Duration::from_secs(5));
    if cancellation.is_cancelled() {
        return Err(history_fault("cancelled", "not_started"));
    }
    if accepted >= deadline {
        return Err(history_fault("timeout", "not_started"));
    }
    // 无UI优先明确拒绝；CLI不能据此捕获一个新目标或启动主程序。
    let broker = BROKER
        .get()
        .and_then(|value| value.lock().ok()?.clone())
        .ok_or_else(|| history_fault("ui_unavailable", "not_started"))?;
    let target = target.ok_or_else(|| history_fault("target_unavailable", "not_started"))?;
    if broker
        .busy
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(history_fault("ui_busy", "not_started"));
    }
    let phase = Arc::new(Mutex::new(HistoryOpenPhase::Pending));
    let (changed, receiver) = mpsc::sync_channel(1);
    let request = HistoryRequest {
        target,
        accepted,
        cancellation: cancellation.clone(),
        deadline,
        phase: phase.clone(),
        changed,
        slot: HistorySlot(broker.busy),
    };
    broker
        .sender
        .try_send(UiRequest::History(request))
        .map_err(|error| {
            history_fault(
                if matches!(error, mpsc::TrySendError::Full(_)) {
                    "ui_busy"
                } else {
                    "ui_unavailable"
                },
                "not_started",
            )
        })?;
    wait_history_open(&phase, receiver, cancellation, deadline)
}

fn wait_history_open(
    phase: &Mutex<HistoryOpenPhase>,
    receiver: Receiver<()>,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<Value, ActionError> {
    loop {
        {
            let mut phase = phase.lock().unwrap_or_else(|error| error.into_inner());
            if let HistoryOpenPhase::Failed(error) = &*phase {
                return Err(error.clone());
            }
            let interrupt = if cancellation.is_cancelled() {
                Some("cancelled")
            } else if Instant::now() >= deadline {
                Some("timeout")
            } else {
                None
            };
            if let Some(code) = interrupt {
                let effect = if matches!(*phase, HistoryOpenPhase::Pending) {
                    "not_started"
                } else {
                    "unknown"
                };
                let error = history_fault(code, effect);
                if matches!(*phase, HistoryOpenPhase::Pending) {
                    *phase = HistoryOpenPhase::Failed(error.clone());
                }
                return Err(error);
            }
            if matches!(*phase, HistoryOpenPhase::Opened) {
                return Ok(json!({"opened":true}));
            }
        }
        match receiver.recv_timeout(POLL.min(deadline.saturating_duration_since(Instant::now()))) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let phase = phase.lock().unwrap_or_else(|error| error.into_inner());
                if matches!(
                    *phase,
                    HistoryOpenPhase::Opened | HistoryOpenPhase::Failed(_)
                ) {
                    continue;
                }
                return Err(history_fault(
                    "ui_unavailable",
                    if matches!(
                        *phase,
                        HistoryOpenPhase::Pending | HistoryOpenPhase::Failed(_)
                    ) {
                        "not_started"
                    } else {
                        "unknown"
                    },
                ));
            }
        }
    }
}

// toggle只追加显式切换，不改变openHistory的创建契约。
enum HistoryTogglePhase {
    Pending,
    Creating,
    Closing,
    Opened,
    Closed,
    Failed(ActionError),
}

struct HistoryToggleRequest {
    target: Option<ScriptTargetSnapshot>,
    accepted: Instant,
    cancellation: CancellationToken,
    deadline: Instant,
    phase: Arc<Mutex<HistoryTogglePhase>>,
    changed: SyncSender<()>,
    busy: Arc<AtomicBool>,
}

fn history_toggle_fault(code: &str, effect: &str) -> ActionError {
    let mut error = history_fault(code, effect);
    if let Some(message) = match code {
        "invalid_input" => Some("切换历史窗口不接受参数。"),
        "cancelled" => Some("切换历史窗口已取消。"),
        "timeout" => Some("历史窗口创建或关闭尚未确认，不会自动重试。"),
        _ => None,
    } {
        error.message = message.into();
    }
    error.method = Some("ui.toggle_history".into());
    error
}

fn finish_history_toggle(request: &HistoryToggleRequest, result: HistoryTogglePhase) {
    *request
        .phase
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = result;
    let _ = request.changed.try_send(());
}

fn begin_history_toggle(request: &HistoryToggleRequest, closing: bool) -> bool {
    let mut phase = request
        .phase
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if !matches!(*phase, HistoryTogglePhase::Pending) {
        return false;
    }
    let failure = if request.cancellation.is_cancelled() {
        Some("cancelled")
    } else if Instant::now() >= request.deadline {
        Some("timeout")
    } else {
        None
    };
    if let Some(code) = failure {
        *phase = HistoryTogglePhase::Failed(history_toggle_fault(code, "not_started"));
        let _ = request.changed.try_send(());
        false
    } else {
        *phase = if closing {
            HistoryTogglePhase::Closing
        } else {
            HistoryTogglePhase::Creating
        };
        true
    }
}

fn begin_history_toggle_close(request: &HistoryToggleRequest, already_requested: bool) -> bool {
    if already_requested {
        // 不能把第二次toggle变成RetryCleanup；由原窗口显式重试按钮负责。
        finish_history_toggle(
            request,
            HistoryTogglePhase::Failed(history_toggle_fault("ui_busy", "not_started")),
        );
        return false;
    }
    // 关闭只操作本窗，不读取或验证本次外部目标。
    begin_history_toggle(request, true)
}

fn prepare_history_toggle_open(
    request: &HistoryToggleRequest,
) -> Option<(
    crate::clipboard_history_send::HistoryLaunchTarget,
    HistorySlot,
)> {
    // 与openHistory/choose共用原有CAS，窗口继续持有同一个槽。
    if request
        .busy
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        finish_history_toggle(
            request,
            HistoryTogglePhase::Failed(history_toggle_fault("ui_busy", "not_started")),
        );
        return None;
    }
    let slot = HistorySlot(request.busy.clone());
    let Some(snapshot) = request.target else {
        finish_history_toggle(
            request,
            HistoryTogglePhase::Failed(history_toggle_fault("target_unavailable", "not_started")),
        );
        return None;
    };
    let target = match crate::clipboard_history_send::HistoryLaunchTarget::from_host_snapshot(
        snapshot,
        request.accepted,
    ) {
        Ok(target) => target,
        Err(_) => {
            finish_history_toggle(
                request,
                HistoryTogglePhase::Failed(history_toggle_fault("target_changed", "not_started")),
            );
            return None;
        }
    };
    begin_history_toggle(request, false).then_some((target, slot))
}

fn dispatch_history_toggle(
    request: HistoryToggleRequest,
    history: &mut Option<(WindowHandle<crate::Workspace>, HistorySlot)>,
    closing: &mut Option<HistoryToggleRequest>,
    cx: &mut App,
) {
    if closing.is_some() {
        finish_history_toggle(
            &request,
            HistoryTogglePhase::Failed(history_toggle_fault("ui_busy", "not_started")),
        );
        return;
    }
    if let Some((handle, _)) = history.as_ref() {
        let started = handle.update(cx, |view, _, cx| {
            if !begin_history_toggle_close(&request, view.text_history.popup_close_requested()) {
                return false;
            }
            view.text_history.request_popup_close();
            cx.notify();
            true
        });
        match started {
            Ok(true) => *closing = Some(request),
            Ok(false) => {}
            Err(_) => finish_history_toggle(
                &request,
                HistoryTogglePhase::Failed(history_toggle_fault("ui_unavailable", "unknown")),
            ),
        }
        return;
    }
    if let Some((target, slot)) = prepare_history_toggle_open(&request) {
        match crate::open_script_history_window(target, cx) {
            Ok(handle) => {
                *history = Some((handle, slot));
                finish_history_toggle(&request, HistoryTogglePhase::Opened);
            }
            Err(_) => finish_history_toggle(
                &request,
                HistoryTogglePhase::Failed(history_toggle_fault("ui_unavailable", "unknown")),
            ),
        }
    }
}

fn toggle_history(
    params: Value,
    cancellation: &CancellationToken,
    deadline: Instant,
    target: Option<ScriptTargetSnapshot>,
) -> Result<Value, ActionError> {
    if params != json!({}) {
        return Err(history_toggle_fault("invalid_input", "not_started"));
    }
    let accepted = Instant::now();
    let deadline = deadline.min(accepted + Duration::from_secs(5));
    if cancellation.is_cancelled() {
        return Err(history_toggle_fault("cancelled", "not_started"));
    }
    if accepted >= deadline {
        return Err(history_toggle_fault("timeout", "not_started"));
    }
    let broker = BROKER
        .get()
        .and_then(|value| value.lock().ok()?.clone())
        .ok_or_else(|| history_toggle_fault("ui_unavailable", "not_started"))?;
    let phase = Arc::new(Mutex::new(HistoryTogglePhase::Pending));
    let (changed, receiver) = mpsc::sync_channel(1);
    let request = HistoryToggleRequest {
        target,
        accepted,
        cancellation: cancellation.clone(),
        deadline,
        phase: phase.clone(),
        changed,
        busy: broker.busy,
    };
    // GPUI线程决定打开还是关闭；调用线程不能用过期的窗口存在性快照决定。
    broker
        .sender
        .try_send(UiRequest::ToggleHistory(request))
        .map_err(|error| {
            history_toggle_fault(
                if matches!(error, mpsc::TrySendError::Full(_)) {
                    "ui_busy"
                } else {
                    "ui_unavailable"
                },
                "not_started",
            )
        })?;
    wait_history_toggle(&phase, receiver, cancellation, deadline)
}

fn wait_history_toggle(
    phase: &Mutex<HistoryTogglePhase>,
    receiver: Receiver<()>,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<Value, ActionError> {
    loop {
        {
            let mut phase = phase.lock().unwrap_or_else(|error| error.into_inner());
            if let HistoryTogglePhase::Failed(error) = &*phase {
                return Err(error.clone());
            }
            let interrupt = if cancellation.is_cancelled() {
                Some("cancelled")
            } else if Instant::now() >= deadline {
                Some("timeout")
            } else {
                None
            };
            if let Some(code) = interrupt {
                let pending = matches!(*phase, HistoryTogglePhase::Pending);
                let error =
                    history_toggle_fault(code, if pending { "not_started" } else { "unknown" });
                if pending {
                    *phase = HistoryTogglePhase::Failed(error.clone());
                }
                // 已经开始的创建/关闭仍由UI拥有；不撤销、不重放、不释放窗口槽。
                return Err(error);
            }
            match *phase {
                HistoryTogglePhase::Opened => return Ok(json!({"opened":true})),
                HistoryTogglePhase::Closed => return Ok(json!({"closed":true})),
                _ => {}
            }
        }
        match receiver.recv_timeout(POLL.min(deadline.saturating_duration_since(Instant::now()))) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let phase = phase.lock().unwrap_or_else(|error| error.into_inner());
                if matches!(
                    *phase,
                    HistoryTogglePhase::Opened
                        | HistoryTogglePhase::Closed
                        | HistoryTogglePhase::Failed(_)
                ) {
                    continue;
                }
                return Err(history_toggle_fault(
                    "ui_unavailable",
                    if matches!(*phase, HistoryTogglePhase::Pending) {
                        "not_started"
                    } else {
                        "unknown"
                    },
                ));
            }
        }
    }
}

#[cfg(test)]
mod history_toggle_lifecycle_tests {
    use super::*;

    fn request() -> (HistoryToggleRequest, Receiver<()>) {
        let (changed, receiver) = mpsc::sync_channel(1);
        (
            HistoryToggleRequest {
                target: None,
                accepted: Instant::now(),
                cancellation: CancellationToken::new(),
                deadline: Instant::now() + Duration::from_secs(5),
                phase: Arc::new(Mutex::new(HistoryTogglePhase::Pending)),
                changed,
                busy: Arc::new(AtomicBool::new(false)),
            },
            receiver,
        )
    }

    #[test]
    fn headless_toggle_rejects_without_forwarding_or_launching() {
        struct Inner;
        impl NativeHost for Inner {
            fn call(&self, _: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
                panic!("切换历史窗不得回退到其他能力");
            }
        }
        let error = Host::new(Inner, None)
            .call("ui.toggle_history", json!({}), &CancellationToken::new())
            .unwrap_err();
        assert_eq!(error.code, "ui_unavailable");
        assert_eq!(error.method.as_deref(), Some("ui.toggle_history"));
        assert_eq!(error.effect_state.as_deref(), Some("not_started"));
    }

    #[test]
    fn chooser_slot_is_busy_before_missing_target_and_is_not_released() {
        let (request, receiver) = request();
        request.busy.store(true, Ordering::Release);
        assert!(prepare_history_toggle_open(&request).is_none());
        let error = wait_history_toggle(
            &request.phase,
            receiver,
            &request.cancellation,
            request.deadline,
        )
        .unwrap_err();
        assert_eq!(error.code, "ui_busy");
        assert_eq!(error.effect_state.as_deref(), Some("not_started"));
        assert!(request.busy.load(Ordering::Acquire));
    }

    #[test]
    fn opening_without_host_target_releases_only_its_new_reservation() {
        let (request, receiver) = request();
        assert!(prepare_history_toggle_open(&request).is_none());
        let error = wait_history_toggle(
            &request.phase,
            receiver,
            &request.cancellation,
            request.deadline,
        )
        .unwrap_err();
        assert_eq!(error.code, "target_unavailable");
        assert!(!request.busy.load(Ordering::Acquire));
    }

    #[test]
    fn close_needs_no_external_target_and_duplicate_close_does_not_retry_cleanup() {
        let (request, _) = request();
        assert!(begin_history_toggle_close(&request, false));
        assert!(matches!(
            *request.phase.lock().unwrap(),
            HistoryTogglePhase::Closing
        ));
        let (duplicate, receiver) = self::request();
        assert!(!begin_history_toggle_close(&duplicate, true));
        let error = wait_history_toggle(
            &duplicate.phase,
            receiver,
            &duplicate.cancellation,
            duplicate.deadline,
        )
        .unwrap_err();
        assert_eq!(error.code, "ui_busy");
        assert_eq!(error.effect_state.as_deref(), Some("not_started"));
        assert!(matches!(
            *request.phase.lock().unwrap(),
            HistoryTogglePhase::Closing
        ));
    }

    #[test]
    fn cancellation_before_start_prevents_both_branches_after_start_stays_unknown() {
        for closing in [false, true] {
            let (request, receiver) = request();
            request.cancellation.cancel();
            assert!(!begin_history_toggle(&request, closing));
            let error = wait_history_toggle(
                &request.phase,
                receiver,
                &request.cancellation,
                request.deadline,
            )
            .unwrap_err();
            assert_eq!(error.effect_state.as_deref(), Some("not_started"));
            let (started, receiver) = self::request();
            assert!(begin_history_toggle(&started, closing));
            started.cancellation.cancel();
            let error = wait_history_toggle(
                &started.phase,
                receiver,
                &started.cancellation,
                started.deadline,
            )
            .unwrap_err();
            assert_eq!(error.effect_state.as_deref(), Some("unknown"));
            assert!(matches!(
                *started.phase.lock().unwrap(),
                HistoryTogglePhase::Creating | HistoryTogglePhase::Closing
            ));
        }
    }

    #[test]
    fn timed_out_close_keeps_ui_slot_until_owner_removes_window_then_closed_receipt_is_distinct() {
        let (mut request, receiver) = request();
        request.busy.store(true, Ordering::Release);
        let slot = HistorySlot(request.busy.clone());
        let mut main = crate::clipboard_history_ui_state::HistoryUiState::new(true);
        let (main_start, _) = main.primary().unwrap();
        main.acknowledge(
            main_start,
            crate::clipboard_history_controller::Status::Running {
                key: crate::clipboard_history_controller::SessionKey {
                    generation: 1,
                    listener_session: 2,
                },
            },
            None,
        );
        let mut popup = crate::clipboard_history_ui_state::HistoryUiState::new(true);
        let (start, _) = popup.primary().unwrap();
        popup.acknowledge(
            start,
            crate::clipboard_history_controller::Status::Running {
                key: crate::clipboard_history_controller::SessionKey {
                    generation: 1,
                    listener_session: 1,
                },
            },
            None,
        );
        assert!(begin_history_toggle_close(
            &request,
            popup.popup_close_requested()
        ));
        let (stop, command) = popup.request_popup_close().unwrap();
        assert_eq!(command, crate::clipboard_history_ui_state::Control::Disable);
        assert!(!popup.popup_close_ready());
        request.deadline = Instant::now();
        let error = wait_history_toggle(
            &request.phase,
            receiver,
            &request.cancellation,
            request.deadline,
        )
        .unwrap_err();
        assert_eq!(error.code, "timeout");
        assert_eq!(error.effect_state.as_deref(), Some("unknown"));
        assert!(request.busy.load(Ordering::Acquire));
        assert!(matches!(
            *request.phase.lock().unwrap(),
            HistoryTogglePhase::Closing
        ));
        // 旧确认不能使本窗Off；主窗也继续保持自己的On。
        popup.acknowledge(
            start,
            crate::clipboard_history_controller::Status::Stopped,
            None,
        );
        assert!(!popup.popup_close_ready());
        assert!(request.busy.load(Ordering::Acquire));
        assert_eq!(main.phase, crate::clipboard_history_ui_state::Phase::On);
        popup.acknowledge(
            stop,
            crate::clipboard_history_controller::Status::Stopped,
            None,
        );
        assert!(popup.popup_close_ready());
        assert_eq!(main.phase, crate::clipboard_history_ui_state::Phase::On);
        // 与UI相同：本窗确认Off后移除窗口/释放所有者槽，之后才发布Closed。
        drop(slot);
        finish_history_toggle(&request, HistoryTogglePhase::Closed);
        assert!(!request.busy.load(Ordering::Acquire));
        let (_signal, receiver) = mpsc::sync_channel(1);
        assert_eq!(
            wait_history_toggle(
                &request.phase,
                receiver,
                &CancellationToken::new(),
                Instant::now() + Duration::from_secs(1)
            )
            .unwrap(),
            json!({"closed":true})
        );
    }
}
#[cfg(test)]
mod history_open_lifecycle_tests {
    use super::*;

    #[test]
    fn headless_ui_host_rejects_without_forwarding_or_capturing_a_target() {
        struct Inner;
        impl NativeHost for Inner {
            fn call(&self, _: &str, _: Value, _: &CancellationToken) -> Result<Value, ActionError> {
                panic!("历史打开不得回退到其他宿主能力");
            }
        }
        let error = Host::new(Inner, None)
            .call("ui.open_history", json!({}), &CancellationToken::new())
            .unwrap_err();
        assert_eq!(error.code, "ui_unavailable");
        assert_eq!(error.effect_state.as_deref(), Some("not_started"));
    }

    #[test]
    fn cancellation_before_creation_prevents_creation_but_after_begin_is_unknown() {
        let deadline = Instant::now() + Duration::from_secs(5);
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let pending = Mutex::new(HistoryOpenPhase::Pending);
        assert!(!history_begin(&pending, &cancellation, deadline));
        let (signal, receiver) = mpsc::sync_channel(1);
        let failure = wait_history_open(&pending, receiver, &cancellation, deadline).unwrap_err();
        assert_eq!(failure.effect_state.as_deref(), Some("not_started"));
        drop(signal);
        let active = Mutex::new(HistoryOpenPhase::Pending);
        let token = CancellationToken::new();
        assert!(history_begin(&active, &token, deadline));
        token.cancel();
        let (_signal, receiver) = mpsc::sync_channel(1);
        let failure = wait_history_open(&active, receiver, &token, deadline).unwrap_err();
        assert_eq!(failure.effect_state.as_deref(), Some("unknown"));
        assert!(matches!(
            *active.lock().unwrap(),
            HistoryOpenPhase::Creating
        ));
    }

    #[test]
    fn acknowledged_window_slot_outlives_action_and_is_released_only_by_ui_owner() {
        let busy = Arc::new(AtomicBool::new(true));
        let ui_slot = HistorySlot(busy.clone());
        let phase = Arc::new(Mutex::new(HistoryOpenPhase::Opened));
        let (_signal, receiver) = mpsc::sync_channel(1);
        assert_eq!(
            wait_history_open(
                &phase,
                receiver,
                &CancellationToken::new(),
                Instant::now() + Duration::from_secs(1)
            )
            .unwrap(),
            json!({"opened":true})
        );
        drop(phase);
        assert!(busy.load(Ordering::Acquire));
        // choose和下一次openHistory都使用这个原有CAS，动作结束不能让它们进入。
        assert!(busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err());
        drop(ui_slot);
        assert!(busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok());
    }
}

#[cfg(test)]
#[path = "../../../tests/script_ui_choose.rs"]
mod tests;

#[cfg(test)]
#[path = "../../../tests/script_ui_fs_read_directory.rs"]
mod fs_directory_dispatch_tests;

#[cfg(test)]
#[path = "../../../tests/script_ui_action_hint.rs"]
mod action_hint_tests;
