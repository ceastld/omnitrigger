//! 通知有一个待创建/可见名额；工作线程只等创建回执，生命周期由应用 UI 持有。
use crate::theme;
use gpui::{
    div, prelude::*, px, size, App, Bounds, Context, FontWeight, Window,
    WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind, WindowOptions,
};
use omnitrigger_actions::{ActionError, CancellationToken};
use omnitrigger_windows::{NotificationPlacement, SystemHudPlacement};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
        Arc, Condvar, Mutex, MutexGuard, OnceLock,
    },
    time::{Duration, Instant},
};

const POLL: Duration = Duration::from_millis(10);
const SUBMIT_LIMIT: Duration = Duration::from_millis(1000);
const WIDTH: f32 = 460.;
static BROKER: OnceLock<Mutex<Option<Broker>>> = OnceLock::new();
static DIAGNOSTICS: Mutex<Diagnostics> = Mutex::new(Diagnostics {
    window_id: None,
    presented_count: 0,
    closed_count: 0,
});

struct Diagnostics {
    window_id: Option<isize>,
    presented_count: u64,
    closed_count: u64,
}

/// 只供本机验收使用；不包含正文、标题、目标或脚本信息。
#[allow(dead_code)]
pub(crate) fn diagnostics() -> Value {
    let busy = BROKER
        .get()
        .and_then(|broker| {
            broker
                .lock()
                .ok()?
                .as_ref()
                .map(|broker| broker.busy.load(Ordering::Acquire))
        })
        .unwrap_or(false);
    let state = DIAGNOSTICS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    json!({"busy":busy,"windowId":state.window_id,"presentedCount":state.presented_count,"closedCount":state.closed_count})
}

fn fault(code: &str, message: impl Into<String>, effect: &str) -> ActionError {
    let mut error = ActionError::new(code, message);
    error.method = Some("ui.notify".into());
    error.effect_state = Some(effect.into());
    error
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum Level {
    #[default]
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum Placement {
    #[default]
    TopRight,
    BottomRight,
    Center,
}

impl Placement {
    fn native(self) -> NotificationPlacement {
        match self {
            Self::TopRight => NotificationPlacement::TopRight,
            Self::BottomRight => NotificationPlacement::BottomRight,
            Self::Center => NotificationPlacement::Center,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Options {
    message: String,
    #[serde(default = "default_title")]
    title: String,
    #[serde(default)]
    level: Level,
    #[serde(default)]
    placement: Placement,
    #[serde(default = "default_duration")]
    duration_ms: u64,
}
fn default_title() -> String {
    "OmniTrigger".into()
}
fn default_duration() -> u64 {
    3000
}

impl Options {
    fn parse(params: Value) -> Result<Self, ActionError> {
        let invalid = || {
            fault("invalid_input", "通知参数无效：需要 1–1024 UTF-16 单元的非空文本、有效标题、级别、位置和 1000–30000 毫秒整数时长。", "not_started")
        };
        let options: Self = serde_json::from_value(params).map_err(|_| invalid())?;
        if options.message.trim().is_empty()
            || options.message.encode_utf16().count() > 1024
            || options
                .message
                .chars()
                .any(|ch| ch.is_control() && !matches!(ch, '\r' | '\n' | '\t'))
            || options.title.trim().is_empty()
            || options.title.encode_utf16().count() > 128
            || options.title.chars().any(char::is_control)
            || options.title.contains(['\u{2028}', '\u{2029}'])
            || !(1000..=30_000).contains(&options.duration_ms)
        {
            return Err(invalid());
        }
        Ok(options)
    }

    fn height(&self) -> f32 {
        // 保守预留换行高度，保留原文；实际尺寸仍受主显示器工作区约束。
        let mut lines = 1usize;
        let mut column = 0usize;
        let mut previous_cr = false;
        for ch in self.message.chars() {
            if ch == '\r' || ch == '\n' || ch == '\u{2028}' || ch == '\u{2029}' {
                if !(ch == '\n' && previous_cr) {
                    lines += 1;
                }
                column = 0;
            } else {
                column += if ch == '\t' { 4 } else { ch.len_utf16() };
                if column > 25 {
                    lines += 1;
                    column = 1;
                }
            }
            previous_cr = ch == '\r';
        }
        let title_lines = self.title.encode_utf16().count().div_ceil(22).max(1);
        54. + (lines + title_lines) as f32 * 22.
    }
}

#[derive(Clone, Debug)]
enum Phase {
    Pending,
    Creating,
    Created,
    Submitted,
    Failed(ActionError),
}

struct Submission {
    phase: Mutex<Phase>,
    changed: Condvar,
}
impl Submission {
    fn new() -> Self {
        Self {
            phase: Mutex::new(Phase::Pending),
            changed: Condvar::new(),
        }
    }
    fn lock(&self) -> MutexGuard<'_, Phase> {
        self.phase.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn interrupt(phase: &mut Phase, code: &str, message: &str) -> ActionError {
        if let Phase::Failed(error) = phase {
            return error.clone();
        }
        let error = fault(
            code,
            message,
            if matches!(phase, Phase::Pending) {
                "not_started"
            } else {
                "unknown"
            },
        );
        if matches!(phase, Phase::Pending) {
            *phase = Phase::Failed(error.clone());
        }
        error
    }

    fn begin(&self, cancellation: &CancellationToken, stopped: bool, deadline: Instant) -> bool {
        let mut phase = self.lock();
        if !matches!(*phase, Phase::Pending) {
            return false;
        }
        if cancellation.is_cancelled() {
            Self::interrupt(&mut phase, "cancelled", "动作已取消，未创建通知。");
        } else if stopped {
            Self::interrupt(&mut phase, "ui_unavailable", "通知界面已退出。");
        } else if Instant::now() >= deadline {
            Self::interrupt(&mut phase, "timeout", "通知提交超过等待时限，未创建通知。");
        } else {
            *phase = Phase::Creating;
            return true;
        }
        self.changed.notify_all();
        false
    }

    fn created(&self) {
        *self.lock() = Phase::Created;
        self.changed.notify_all();
    }
    fn submitted(&self) {
        *self.lock() = Phase::Submitted;
        self.changed.notify_all();
    }
    fn fail(&self, message: impl Into<String>) {
        let mut phase = self.lock();
        if matches!(*phase, Phase::Submitted | Phase::Failed(_)) {
            return;
        }
        let effect = if matches!(*phase, Phase::Pending) {
            "not_started"
        } else {
            "unknown"
        };
        *phase = Phase::Failed(fault("ui_unavailable", message, effect));
        self.changed.notify_all();
    }

    fn wait(
        &self,
        cancellation: &CancellationToken,
        stopped: &AtomicBool,
        deadline: Instant,
    ) -> Result<Value, ActionError> {
        let mut phase = self.lock();
        loop {
            if stopped.load(Ordering::Acquire) {
                return Err(Self::interrupt(
                    &mut phase,
                    "ui_unavailable",
                    "通知界面已退出。",
                ));
            }
            if cancellation.is_cancelled() {
                return Err(Self::interrupt(
                    &mut phase,
                    "cancelled",
                    "动作已取消；已经创建的通知不会自动重试。",
                ));
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(Self::interrupt(
                    &mut phase,
                    "timeout",
                    "通知提交超过等待时限；不会自动重试。",
                ));
            }
            match &*phase {
                Phase::Submitted => return Ok(json!({"submitted":true})),
                Phase::Failed(error) => return Err(error.clone()),
                _ => {}
            }
            let (next, _) = self
                .changed
                .wait_timeout(phase, POLL.min(deadline - now))
                .unwrap_or_else(|error| error.into_inner());
            phase = next;
        }
    }
}

struct Slot(Arc<AtomicBool>);
impl Slot {
    fn acquire(busy: Arc<AtomicBool>) -> Result<Self, ActionError> {
        busy.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| fault("ui_busy", "已有一个待创建或可见通知。", "not_started"))?;
        Ok(Self(busy))
    }
}
impl Drop for Slot {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

struct Request {
    options: Options,
    submission: Arc<Submission>,
    cancellation: CancellationToken,
    deadline: Instant,
    _slot: Slot,
}
impl Drop for Request {
    fn drop(&mut self) {
        self.submission.fail("通知界面请求已结束。");
    }
}

#[derive(Clone)]
struct Broker {
    sender: SyncSender<Request>,
    busy: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
}

struct Running(Arc<AtomicBool>);
impl Drop for Running {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

pub fn notify(
    params: Value,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<Value, ActionError> {
    let started = Instant::now();
    let options = Options::parse(params)?;
    let deadline = deadline.min(started + SUBMIT_LIMIT);
    if cancellation.is_cancelled() {
        return Err(fault(
            "cancelled",
            "动作已取消，未创建通知。",
            "not_started",
        ));
    }
    if Instant::now() >= deadline {
        return Err(fault(
            "timeout",
            "通知提交超过等待时限，未创建通知。",
            "not_started",
        ));
    }
    let unavailable = || {
        fault(
            "ui_unavailable",
            "通知需要正在运行的 OmniTrigger 界面。",
            "not_started",
        )
    };
    let broker = BROKER.get().and_then(|broker| broker.lock().ok()?.clone());
    submit(
        options,
        cancellation,
        deadline,
        broker.ok_or_else(unavailable)?,
    )
}

fn submit(
    options: Options,
    cancellation: &CancellationToken,
    deadline: Instant,
    broker: Broker,
) -> Result<Value, ActionError> {
    if cancellation.is_cancelled() {
        return Err(fault(
            "cancelled",
            "动作已取消，未创建通知。",
            "not_started",
        ));
    }
    if Instant::now() >= deadline {
        return Err(fault(
            "timeout",
            "通知提交超过等待时限，未创建通知。",
            "not_started",
        ));
    }
    if broker.stopped.load(Ordering::Acquire) {
        return Err(fault("ui_unavailable", "通知界面已退出。", "not_started"));
    }
    let slot = Slot::acquire(broker.busy)?;
    let submission = Arc::new(Submission::new());
    let request = Request {
        options,
        submission: submission.clone(),
        cancellation: cancellation.clone(),
        deadline,
        _slot: slot,
    };
    broker
        .sender
        .try_send(request)
        .map_err(|error| match error {
            mpsc::TrySendError::Full(_) => {
                fault("ui_busy", "已有一个待创建或可见通知。", "not_started")
            }
            mpsc::TrySendError::Disconnected(_) => {
                fault("ui_unavailable", "通知界面已退出。", "not_started")
            }
        })?;
    submission.wait(cancellation, &broker.stopped, deadline)
}

struct Active {
    window: WindowHandle<Notification>,
    until: Instant,
    _request: Request,
}
impl Active {
    fn close(self, cx: &mut App) {
        let _ = self
            .window
            .update(cx, |_, window, _| window.remove_window());
        let mut state = DIAGNOSTICS
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.window_id = None;
        state.closed_count = state.closed_count.saturating_add(1);
        // 窗口已移除后才释放 slot；不依赖脚本连接是否仍存在。
    }
}

pub fn init(cx: &mut App) {
    let (sender, receiver) = mpsc::sync_channel(1);
    let stopped = Arc::new(AtomicBool::new(false));
    let broker = Broker {
        sender,
        busy: Arc::new(AtomicBool::new(false)),
        stopped: stopped.clone(),
    };
    *BROKER
        .get_or_init(|| Mutex::new(None))
        .lock()
        .expect("通知界面通道") = Some(broker);
    let receiver = Rc::new(RefCell::new(receiver));
    let active: Rc<RefCell<Option<Active>>> = Rc::new(RefCell::new(None));
    let quit_receiver = receiver.clone();
    let quit_active = active.clone();
    let quit_stopped = stopped.clone();
    cx.on_app_quit(move |cx| {
        quit_stopped.store(true, Ordering::Release);
        if let Some(active) = quit_active.borrow_mut().take() {
            active.close(cx);
        }
        while let Ok(request) = quit_receiver.borrow_mut().try_recv() {
            drop(request);
        }
        async {}
    })
    .detach();
    cx.spawn(async move |cx| {
        let _running = Running(stopped.clone());
        loop {
            cx.background_executor().timer(POLL).await;
            if stopped.load(Ordering::Acquire) {
                break;
            }
            if cx
                .update(|cx| {
                    let expired = active
                        .borrow()
                        .as_ref()
                        .is_some_and(|active| Instant::now() >= active.until);
                    if expired {
                        if let Some(active) = active.borrow_mut().take() {
                            active.close(cx);
                        }
                    }
                    if active.borrow().is_some() {
                        return;
                    }
                    let Ok(request) = receiver.borrow_mut().try_recv() else {
                        return;
                    };
                    let placement = match omnitrigger_windows::primary_notification_placement(
                        request.options.placement.native(),
                        WIDTH,
                        request.options.height(),
                    ) {
                        Ok(placement) => placement,
                        Err(message) => {
                            request.submission.fail(message);
                            return;
                        }
                    };
                    if !request.submission.begin(
                        &request.cancellation,
                        stopped.load(Ordering::Acquire),
                        request.deadline,
                    ) {
                        return;
                    }
                    match create_window(&request, &placement, cx) {
                        Ok((window, hwnd)) => {
                            let until =
                                Instant::now() + Duration::from_millis(request.options.duration_ms);
                            let mut state = DIAGNOSTICS
                                .lock()
                                .unwrap_or_else(|error| error.into_inner());
                            state.window_id = Some(hwnd);
                            state.presented_count = state.presented_count.saturating_add(1);
                            drop(state);
                            request.submission.submitted();
                            *active.borrow_mut() = Some(Active {
                                window,
                                until,
                                _request: request,
                            });
                        }
                        Err(message) => request.submission.fail(message),
                    }
                })
                .is_err()
            {
                break;
            }
        }
        stopped.store(true, Ordering::Release);
    })
    .detach();
}

fn create_window(
    request: &Request,
    placement: &SystemHudPlacement,
    cx: &mut App,
) -> Result<(WindowHandle<Notification>, isize), String> {
    let bounds = Bounds::centered(None, size(px(WIDTH), px(100.)), cx);
    let mut prepared = Ok(());
    let window = cx
        .open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: None,
                kind: WindowKind::PopUp,
                focus: false,
                show: false,
                is_resizable: false,
                is_minimizable: false,
                is_movable: false,
                window_background: WindowBackgroundAppearance::Transparent,
                app_id: Some("OmniTrigger.ScriptNotification".into()),
                ..Default::default()
            },
            |window, cx| {
                request.submission.created();
                prepared = raw_handle(window).and_then(|hwnd| {
                    omnitrigger_windows::prepare_notification_window(hwnd, placement)
                });
                cx.new(|_| Notification {
                    options: request.options.clone(),
                })
            },
        )
        .map_err(|error| format!("无法创建通知窗口：{error}"))?;
    // open_window 已绘制首帧；原生无激活显示成功后才确认 submitted。
    let result = prepared.and_then(|()| {
        window
            .update(cx, |_, window, _| {
                let hwnd = raw_handle(window)?;
                omnitrigger_windows::show_system_hud_window(
                    hwnd,
                    placement,
                    &request.options.title,
                )?;
                Ok(hwnd)
            })
            .map_err(|error| format!("通知窗口已不可用：{error}"))?
    });
    match result {
        Ok(hwnd) => Ok((window, hwnd)),
        Err(error) => {
            let _ = window.update(cx, |_, window, _| window.remove_window());
            Err(error)
        }
    }
}

fn raw_handle(window: &Window) -> Result<isize, String> {
    match HasWindowHandle::window_handle(window).map(|handle| handle.as_raw()) {
        Ok(RawWindowHandle::Win32(handle)) => Ok(handle.hwnd.get()),
        _ => Err("无法取得通知窗口句柄。".into()),
    }
}

struct Notification {
    options: Options,
}
impl Render for Notification {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let (label, color) = match self.options.level {
            Level::Info => ("信息", theme::ACCENT_2),
            Level::Success => ("完成", theme::ACCENT),
            Level::Warning => ("提醒", theme::RECORD),
            Level::Error => ("错误", theme::DANGER),
        };
        div()
            .size_full()
            .overflow_hidden()
            .font_family(theme::UI_FONT)
            .bg(theme::rgb(theme::BACKGROUND))
            .text_color(theme::rgb(theme::TEXT))
            .border_1()
            .border_color(theme::rgb(color))
            .rounded(px(10.))
            .p(px(18.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(theme::rgb(color))
                            .child(label),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.options.title.clone()),
                    ),
            )
            .child(
                div()
                    .text_size(px(14.))
                    .line_height(px(22.))
                    .child(self.options.message.clone()),
            )
    }
}

#[cfg(test)]
#[path = "../../../tests/script_ui_notify.rs"]
mod tests;
