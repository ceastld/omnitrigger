//! 脚本选择：工作线程有界等待，GPUI 线程创建、更新和关闭窗口。
use crate::{message_input::MessageInput, theme};
use gpui::{
    div, prelude::*, px, size, App, Bounds, Context, Entity, Focusable, KeyDownEvent, ScrollHandle,
    TitlebarOptions, Window, WindowBounds, WindowHandle, WindowOptions,
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
    sender: SyncSender<Request>,
    busy: Arc<AtomicBool>,
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
            fault("invalid_input", "选择参数无效：需要 1–200 个唯一 ID 的候选项，文本总量最多 32 KiB，时限 1–60000 毫秒。")
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
        if method == "fs.stat" {
            crate::script_fs::stat(params, cancellation, deadline)
        } else if method == "ui.notify" {
            crate::script_notify::notify(params, cancellation, deadline)
        } else if method == "ui.choose" {
            choose(params, cancellation, deadline, self.target)
        } else {
            self.inner
                .call_with_deadline(method, params, cancellation, deadline)
        }
    }
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
        .try_send(Request {
            options,
            response,
            abandoned: waiting.abandoned.clone(),
            target,
        })
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
                return Err(fault("ui_unavailable", "选择窗口宿主已退出。"))
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
    };
    *BROKER
        .get_or_init(|| Mutex::new(None))
        .lock()
        .expect("script UI broker") = Some(broker);
    cx.spawn(async move |cx| {
        let mut active: Option<(WindowHandle<Chooser>, Arc<AtomicBool>)> = None;
        loop {
            cx.background_executor().timer(POLL).await;
            if cx
                .update(|cx| {
                    if active
                        .as_ref()
                        .is_some_and(|(_, abandoned)| abandoned.load(Ordering::Acquire))
                    {
                        if let Some((handle, _)) = active.take() {
                            let _ = handle.update(cx, |this, window, _| this.close(window));
                        }
                    }
                    if active.is_some() {
                        return;
                    }
                    let Ok(request) = receiver.try_recv() else {
                        return;
                    };
                    if request.abandoned.load(Ordering::Acquire) {
                        return;
                    }
                    let abandoned = request.abandoned.clone();
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
                        |window, cx| cx.new(|cx| Chooser::new(request, window, cx)),
                    ) {
                        active = Some((handle, abandoned));
                    }
                })
                .is_err()
            {
                break;
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

#[cfg(test)]
#[path = "../../../tests/script_ui_choose.rs"]
mod tests;
