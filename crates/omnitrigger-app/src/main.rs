#![windows_subsystem = "windows"]

mod assets;
mod message_input;
mod settings;
mod theme;

use assets::{Assets, Icon};
use gpui::{
    div, point, prelude::*, px, rgb, size, AnyElement, App, Application, Bounds, Context, Div,
    Entity, FontWeight, SharedString, TitlebarOptions, Window, WindowBackgroundAppearance,
    WindowBounds, WindowControlArea, WindowHandle, WindowKind, WindowOptions,
};
use message_input::MessageInput;
use omnitrigger_core::{Button, TriggerSpec};
use omnitrigger_windows::{
    format_button, local_clock, Backend, BackendCommand, BackendEvent, ListenerDiagnostics,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use settings::{Binding, Conflict};
use std::{
    collections::{HashMap, VecDeque},
    sync::mpsc::{Receiver, TryRecvError},
    time::{Duration, Instant},
};
use theme::{
    button, caps_for, caps_for_buttons, combo, dot, ghost_cap, hsla, icon, icon_button, keycap,
    logo, plus, small, switch, Cap, CapSize, CapTone, Tone,
};

const TOAST_SECONDS: f32 = 4.;
const TOAST_WIDTH: f32 = 380.;
const TOAST_HEIGHT: f32 = 108.;
/// GPUI 无边框窗口会自行收窄客户区，右下留出透明余量，避免浮层被裁切。
const TOAST_MARGIN: f32 = 24.;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Triggers,
    History,
    Diagnostics,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HistoryFilter {
    All,
    Real,
    Preview,
}

/// 侧栏与状态栏共用的监听状态，由后端状态推导，不单独保存。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Listener {
    Starting,
    Rebuilding,
    Offline,
    Recording,
    Paused,
    Listening,
}

struct Hit {
    time: String,
    caps: Vec<Cap>,
    message: String,
    preview: bool,
}

#[derive(Clone, Default)]
struct Stats {
    hits: u64,
    last: Option<String>,
}

fn clock() -> String {
    let (hour, minute, second) = local_clock();
    format!("{hour:02}:{minute:02}:{second:02}")
}

fn thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn heading(title: &'static str, subtitle: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(
            div()
                .text_size(px(20.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(rgb(theme::TEXT_DIM))
                .child(subtitle.into()),
        )
}

fn caption(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(11.))
        .text_color(rgb(theme::TEXT_FAINT))
        .child(text.into())
}

fn card() -> Div {
    div()
        .rounded(px(10.))
        .bg(rgb(theme::SURFACE))
        .border_1()
        .border_color(rgb(theme::BORDER))
}

fn stat(label: &'static str, value: impl Into<SharedString>, note: Option<(String, u32)>) -> Div {
    card()
        .flex_1()
        .min_w_0()
        .px(px(16.))
        .py(px(14.))
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(
            div()
                .text_size(px(12.))
                .text_color(rgb(theme::TEXT_DIM))
                .child(label),
        )
        .child(
            div()
                .text_size(px(22.))
                .font_weight(FontWeight::SEMIBOLD)
                .font_family(theme::MONO_FONT)
                .child(value.into()),
        )
        .when_some(note, |el, (note, color)| {
            el.child(div().text_size(px(11.)).text_color(rgb(color)).child(note))
        })
}

struct Workspace {
    backend: Option<Backend>,
    events: Receiver<BackendEvent>,
    ready: bool,
    paused: bool,
    recording: bool,
    recording_held: Vec<Button>,
    pending: Option<TriggerSpec>,
    conflict: Option<Conflict>,
    selected: Option<u64>,
    confirm_delete: Option<u64>,
    page: Page,
    history_filter: HistoryFilter,
    message: Entity<MessageInput>,
    bindings: Vec<Binding>,
    stats: HashMap<u64, Stats>,
    error: Option<String>,
    status: String,
    hits: VecDeque<Hit>,
    trigger_count: u64,
    toast: Option<WindowHandle<Toast>>,
    toast_time: Option<Instant>,
    ui_smoke: bool,
    started: Instant,
    restarting: bool,
    diagnostics: ListenerDiagnostics,
    last_diagnostics_request: Instant,
    listener_generation: u64,
    /// 已延伸到标题栏的主窗口句柄；未延伸时保留系统标题栏。
    title_bar: Option<isize>,
    caption_color: Option<u32>,
}

impl Workspace {
    fn new(cx: &mut Context<Self>, ui_smoke: bool, title_bar: Result<isize, String>) -> Self {
        let (bindings, load_error) = match settings::load() {
            Ok(bindings) => (bindings, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        let (backend, events, startup_error) = match Backend::start() {
            Ok((backend, events)) => (Some(backend), events, None),
            Err(error) => (None, std::sync::mpsc::channel().1, Some(error)),
        };
        let selected = bindings.first().map(|binding| binding.spec.id);
        let initial = bindings
            .first()
            .map(|binding| binding.message.clone())
            .unwrap_or_default();
        let message = cx.new(|cx| {
            let mut input = MessageInput::new(cx);
            input.set_text(initial, cx);
            input
        });
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(30))
                .await;
            if this.update(cx, |this, cx| this.poll(cx)).is_err() {
                break;
            }
        })
        .detach();
        Self {
            backend,
            events,
            ready: false,
            paused: false,
            recording: false,
            recording_held: Vec::new(),
            pending: None,
            conflict: None,
            selected,
            confirm_delete: None,
            page: Page::Triggers,
            history_filter: HistoryFilter::All,
            message,
            bindings,
            stats: HashMap::new(),
            error: startup_error.or(load_error).or(title_bar.clone().err()),
            status: "正在连接键盘和鼠标…".into(),
            hits: VecDeque::new(),
            trigger_count: 0,
            toast: None,
            toast_time: None,
            ui_smoke,
            started: Instant::now(),
            restarting: false,
            diagnostics: ListenerDiagnostics::default(),
            last_diagnostics_request: Instant::now(),
            listener_generation: 1,
            title_bar: title_bar.ok(),
            caption_color: None,
        }
    }

    fn listener(&self) -> Listener {
        if self.restarting {
            Listener::Rebuilding
        } else if self.backend.is_none() {
            Listener::Offline
        } else if self.recording {
            Listener::Recording
        } else if !self.ready {
            Listener::Starting
        } else if self.paused {
            Listener::Paused
        } else {
            Listener::Listening
        }
    }

    fn rebuild_backend(&mut self, cx: &mut Context<Self>) {
        if self.restarting {
            return;
        }
        if self.recording {
            self.status = "请先松开按键并结束录制，再重建监听。".into();
            cx.notify();
            return;
        }
        self.restarting = true;
        self.ready = false;
        self.error = None;
        self.status = "正在重建监听，请松开其他按键…".into();
        let previous = self.backend.take();
        // 保留界面、未保存的录制及用户配置；旧按压先按既有退出协议收尾。
        // 不在 UI 线程等待两秒 drain，也不定时重挂与其他软件争抢链首。
        self.events = std::sync::mpsc::channel().1;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    drop(previous);
                    Backend::start()
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.restarting = false;
                this.diagnostics = ListenerDiagnostics::default();
                match result {
                    Ok((backend, events)) => {
                        this.backend = Some(backend);
                        this.events = events;
                        this.listener_generation += 1;
                        this.status = "新监听已安装，正在恢复触发器配置…".into();
                    }
                    Err(error) => {
                        this.paused = true;
                        this.error = Some(error);
                        this.status = "监听重建失败，可以再次点击“重建监听”。".into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn toggle_pause(&mut self, cx: &mut Context<Self>) {
        if self.recording || !self.ready {
            return;
        }
        let paused = !self.paused;
        if self.send(BackendCommand::SetEnabled(!paused)) {
            self.paused = paused;
            self.status = if paused {
                "监听已暂停，输入照常交给应用。"
            } else {
                "监听已恢复，可以切换到其他窗口测试。"
            }
            .into();
        }
        cx.notify();
    }

    fn send(&mut self, command: BackendCommand) -> bool {
        let result = self
            .backend
            .as_ref()
            .ok_or_else(|| "输入引擎尚未就绪。".to_string())
            .and_then(|backend| backend.send(command));
        if let Err(error) = result {
            self.error = Some(error);
            false
        } else {
            true
        }
    }

    fn sync_bindings(&mut self) {
        self.send(BackendCommand::SetBindings(
            self.bindings
                .iter()
                .filter(|b| b.enabled)
                .map(|b| b.spec.clone())
                .collect(),
        ));
    }

    fn poll(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        for _ in 0..64 {
            let event = match self.events.try_recv() {
                Ok(event) => event,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if self.backend.is_some() && !self.restarting {
                        self.ready = false;
                        self.paused = true;
                        self.recording = false;
                        self.recording_held.clear();
                        self.error = Some(
                            "输入监听线程已结束，组合不会再触发。松开其他按键后重建监听，消息和配置都会保留。"
                                .into(),
                        );
                        self.status = "监听已断开。".into();
                        self.backend.take();
                        changed = true;
                    }
                    break;
                }
            };
            changed = true;
            match event {
                BackendEvent::Ready => {
                    self.ready = true;
                    self.status = if self.paused {
                        "监听已就绪，已保留暂停状态；恢复监听后即可测试。"
                    } else {
                        "监听已就绪，可以切换到其他窗口测试。"
                    }
                    .into();
                    // 初始化/重建时先暂停再恢复配置，避免暂停状态短暂失效。
                    self.send(BackendCommand::SetEnabled(false));
                    self.sync_bindings();
                    self.send(BackendCommand::SetEnabled(!self.paused));
                }
                BackendEvent::RecordingProgress(held) => self.recording_held = held,
                BackendEvent::Recorded(spec) => {
                    self.recording = false;
                    self.recording_held.clear();
                    self.conflict = settings::check_candidate(&self.bindings, &spec).err();
                    self.pending = Some(spec);
                    self.status = if self.conflict.is_some() {
                        "录制的组合无法添加，请重新录制。"
                    } else {
                        "已录制，填写提示消息后点击“添加触发器”。"
                    }
                    .into();
                    self.message.update(cx, |input, cx| input.set_text("", cx));
                }
                BackendEvent::Triggered(id) => self.fire(id, false, cx),
                BackendEvent::Error(error) => {
                    self.error = Some(error);
                    self.recording = false;
                    self.recording_held.clear();
                    self.paused = true;
                    self.status = "监听已暂停，请处理错误后恢复监听。".into();
                    self.send(BackendCommand::SetEnabled(false));
                }
                BackendEvent::RecordingCancelled => {
                    self.recording = false;
                    self.recording_held.clear();
                    self.status = "已取消录制。".into();
                    self.load_selected_message(cx);
                }
                BackendEvent::Diagnostics(snapshot) => self.diagnostics = snapshot,
            }
        }
        if self.ready && self.last_diagnostics_request.elapsed() >= Duration::from_secs(1) {
            self.send(BackendCommand::GetDiagnostics);
            self.last_diagnostics_request = Instant::now();
        }
        if let Some(shown) = self.toast_time {
            if shown.elapsed() >= Duration::from_secs_f32(TOAST_SECONDS) {
                if let Some(toast) = self.toast.take() {
                    let _ = toast.update(cx, |_, window, _| window.remove_window());
                }
                self.toast_time = None;
            } else if let Some(toast) = self.toast {
                // 倒计时条随轮询刷新。
                let _ = toast.update(cx, |_, _, cx| cx.notify());
            }
        }
        if self.ui_smoke && self.started.elapsed() > Duration::from_millis(700) {
            self.ui_smoke = false;
            self.show_message(
                Vec::new(),
                "提示消息显示正常。现在可以试用真实组合输入。".into(),
                true,
                cx,
            );
        }
        if changed {
            cx.notify();
        }
    }

    fn binding(&self, id: u64) -> Option<&Binding> {
        self.bindings.iter().find(|binding| binding.spec.id == id)
    }

    fn fire(&mut self, id: u64, preview: bool, cx: &mut Context<Self>) {
        if let Some(binding) = self.binding(id) {
            let caps = caps_for(&binding.spec);
            let message = binding.message.clone();
            if !preview {
                let stats = self.stats.entry(id).or_default();
                stats.hits += 1;
                stats.last = Some(clock());
            }
            self.show_message(caps, message, preview, cx);
        }
    }

    fn preview_draft(&mut self, cx: &mut Context<Self>) {
        let text = self.message.read(cx).text();
        if text.trim().is_empty() {
            self.error = Some("请先填写提示消息。".into());
            cx.notify();
            return;
        }
        let caps = self
            .pending
            .as_ref()
            .or_else(|| {
                self.selected
                    .and_then(|id| self.binding(id))
                    .map(|b| &b.spec)
            })
            .map(caps_for)
            .unwrap_or_default();
        self.show_message(caps, text, true, cx);
    }

    fn show_message(
        &mut self,
        caps: Vec<Cap>,
        message: String,
        preview: bool,
        cx: &mut Context<Self>,
    ) {
        if !preview {
            self.trigger_count += 1;
        }
        self.hits.push_front(Hit {
            time: clock(),
            caps: caps.clone(),
            message: message.clone(),
            preview,
        });
        self.hits.truncate(30);
        self.toast_time = Some(Instant::now());
        let shown = Instant::now();
        if let Some(handle) = self.toast {
            if handle
                .update(cx, |toast, _, cx| {
                    toast.caps = caps.clone();
                    toast.message = message.clone();
                    toast.preview = preview;
                    toast.shown = shown;
                    cx.notify();
                })
                .is_ok()
            {
                cx.notify();
                return;
            }
        }
        let bounds = cx
            .primary_display()
            .map(|display| display.bounds())
            .unwrap_or(Bounds::new(
                point(px(0.), px(0.)),
                size(px(1400.), px(900.)),
            ));
        let origin = point(
            bounds.right() - px(TOAST_WIDTH + TOAST_MARGIN),
            bounds.bottom() - px(TOAST_HEIGHT + 64.),
        );
        let mut popup_error = None;
        match cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    origin,
                    size(
                        px(TOAST_WIDTH + TOAST_MARGIN),
                        px(TOAST_HEIGHT + TOAST_MARGIN),
                    ),
                ))),
                kind: WindowKind::PopUp,
                titlebar: None,
                focus: false,
                show: false,
                is_resizable: false,
                is_minimizable: false,
                is_movable: false,
                window_background: WindowBackgroundAppearance::Transparent,
                app_id: Some("OmniTrigger.Notification".into()),
                ..Default::default()
            },
            |window, cx| {
                let scale = window.scale_factor();
                match window.window_handle().map(|handle| handle.as_raw()) {
                    Ok(RawWindowHandle::Win32(handle)) => {
                        popup_error = omnitrigger_windows::show_notification_window(
                            handle.hwnd.get(),
                            (f32::from(origin.x) * scale) as i32,
                            (f32::from(origin.y) * scale) as i32,
                            ((TOAST_WIDTH + TOAST_MARGIN) * scale) as i32,
                            ((TOAST_HEIGHT + TOAST_MARGIN) * scale) as i32,
                        )
                        .err();
                    }
                    _ => popup_error = Some("无法取得提示窗口句柄。".into()),
                }
                cx.new(|_| Toast {
                    caps,
                    message,
                    preview,
                    shown,
                })
            },
        ) {
            Ok(handle) => self.toast = Some(handle),
            Err(error) => self.error = Some(format!("打开提示浮层失败：{error}")),
        }
        if let Some(error) = popup_error {
            self.error = Some(error);
        }
        cx.notify();
    }

    fn load_selected_message(&mut self, cx: &mut Context<Self>) {
        let text = self
            .selected
            .and_then(|id| self.binding(id))
            .map(|binding| binding.message.clone())
            .unwrap_or_default();
        self.message
            .update(cx, |input, cx| input.set_text(text, cx));
    }

    fn start_recording(&mut self, cx: &mut Context<Self>) {
        if self.listener() != Listener::Listening {
            return;
        }
        self.error = None;
        if self.send(BackendCommand::StartRecording) {
            self.recording = true;
            self.recording_held.clear();
            self.pending = None;
            self.conflict = None;
            self.confirm_delete = None;
            self.page = Page::Triggers;
            self.status = "请按下组合并全部松开；按 Esc 取消。".into();
        }
        cx.notify();
    }

    fn discard_pending(&mut self, cx: &mut Context<Self>) {
        self.pending = None;
        self.conflict = None;
        self.status = "已放弃录制结果。".into();
        self.load_selected_message(cx);
        cx.notify();
    }

    fn select(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.recording {
            return;
        }
        self.selected = Some(id);
        self.pending = None;
        self.conflict = None;
        self.confirm_delete = None;
        self.page = Page::Triggers;
        self.load_selected_message(cx);
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if self.recording {
            return;
        }
        let text = self.message.read(cx).text();
        let mut next = self.bindings.clone();
        let mut added = None;
        if let Some(spec) = self.pending.clone() {
            if self.conflict.is_some() {
                return;
            }
            let id = settings::next_id(&next);
            next.push(Binding {
                spec: TriggerSpec { id, ..spec },
                message: text,
                enabled: true,
            });
            added = Some(id);
        } else if let Some(id) = self.selected {
            if let Some(binding) = next.iter_mut().find(|binding| binding.spec.id == id) {
                binding.message = text;
            }
        } else {
            self.error = Some("请先录制一个组合。".into());
            cx.notify();
            return;
        }
        if self.apply(next) {
            if let Some(id) = added {
                self.pending = None;
                self.selected = Some(id);
                self.status = "触发器已添加，试着在任意窗口触发它。".into();
            } else {
                self.status = "提示消息已保存。".into();
            }
        }
        cx.notify();
    }

    fn apply(&mut self, bindings: Vec<Binding>) -> bool {
        if let Err(error) = settings::validate(&bindings).and_then(|_| settings::save(&bindings)) {
            self.error = Some(error);
            return false;
        }
        self.bindings = bindings;
        self.error = None;
        self.sync_bindings();
        true
    }

    fn toggle_rule(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.recording {
            return;
        }
        let mut bindings = self.bindings.clone();
        if let Some(binding) = bindings.iter_mut().find(|binding| binding.spec.id == id) {
            binding.enabled = !binding.enabled;
        }
        self.apply(bindings);
        cx.notify();
    }

    /// 删除需二次确认：第一次点击只切换按钮文字。
    fn remove_rule(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.recording {
            return;
        }
        if self.confirm_delete != Some(id) {
            self.confirm_delete = Some(id);
            cx.notify();
            return;
        }
        let index = self.bindings.iter().position(|b| b.spec.id == id);
        let bindings = self
            .bindings
            .iter()
            .filter(|binding| binding.spec.id != id)
            .cloned()
            .collect();
        if self.apply(bindings) {
            self.stats.remove(&id);
            self.confirm_delete = None;
            self.selected = index.and_then(|index| {
                self.bindings
                    .get(index)
                    .or_else(|| self.bindings.last())
                    .map(|binding| binding.spec.id)
            });
            self.status = "触发器已删除。".into();
            self.load_selected_message(cx);
        }
        cx.notify();
    }

    // ───────────────────────────── 侧栏 ─────────────────────────────

    fn render_sidebar(&self, header: f32, cx: &mut Context<Self>) -> Div {
        let listener = self.listener();
        let enabled = self.bindings.iter().filter(|b| b.enabled).count();
        let (label, sub, color, label_color, card_bg, card_line) = match listener {
            Listener::Listening => (
                "正在监听",
                format!("监听 #{} · {} 条生效", self.listener_generation, enabled),
                theme::ACCENT,
                theme::TEXT,
                theme::ACCENT_BG,
                0x21362c,
            ),
            Listener::Recording => (
                "正在录制",
                "输入暂被拦截 · Esc 取消".into(),
                theme::RECORD,
                theme::RECORD_TEXT,
                0x1a150f,
                0x3e3121,
            ),
            Listener::Paused => (
                "已暂停",
                "触发器不会响应".into(),
                theme::TEXT_FAINT,
                theme::TEXT_SOFT,
                theme::ROW,
                0x2a303a,
            ),
            Listener::Offline => (
                "未监听",
                "输入线程已结束".into(),
                theme::DANGER,
                theme::DANGER_TEXT,
                theme::DANGER_BG,
                0x4a2429,
            ),
            Listener::Starting => (
                "正在连接",
                "正在安装输入监听".into(),
                theme::TEXT_FAINT,
                theme::TEXT_SOFT,
                theme::ROW,
                0x2a303a,
            ),
            Listener::Rebuilding => (
                "正在重建",
                "请松开其他按键".into(),
                theme::TEXT_FAINT,
                theme::TEXT_SOFT,
                theme::ROW,
                0x2a303a,
            ),
        };
        let action = match listener {
            Listener::Listening => div()
                .id("listener-action")
                .h(px(30.))
                .rounded(px(7.))
                .border_1()
                .border_color(rgb(theme::ACCENT_LINE))
                .text_color(rgb(0xbfebd5))
                .cursor_pointer()
                .hover(|s| s.bg(rgb(0x16241e)))
                .child(icon(Icon::Pause, 13., 0xbfebd5))
                .child("暂停监听")
                .on_click(cx.listener(|this, _, _, cx| this.toggle_pause(cx))),
            Listener::Paused => small(icon_button(
                "listener-action",
                Icon::Play,
                "恢复监听",
                Tone::Primary,
                true,
            ))
            .h(px(30.))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_pause(cx))),
            Listener::Offline => small(button(
                "listener-action",
                "重建监听",
                Tone::DangerSolid,
                true,
            ))
            .h(px(30.))
            .on_click(cx.listener(|this, _, _, cx| this.rebuild_backend(cx))),
            Listener::Recording | Listener::Starting | Listener::Rebuilding => div()
                .id("listener-action")
                .h(px(30.))
                .rounded(px(7.))
                .border_1()
                .border_color(rgb(card_line))
                .text_color(rgb(theme::TEXT_FAINT))
                .opacity(0.7)
                .child(match listener {
                    Listener::Recording => "录制中",
                    Listener::Rebuilding => "正在重建…",
                    _ => "正在连接…",
                }),
        }
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD);

        let nav = |id: &'static str,
                   page: Page,
                   glyph: Icon,
                   label: &'static str,
                   count: Option<usize>| {
            let active = self.page == page;
            div()
                .id(id)
                .h(px(34.))
                .px(px(10.))
                .rounded(px(8.))
                .flex()
                .items_center()
                .gap(px(10.))
                .text_size(px(13.))
                .cursor_pointer()
                .text_color(rgb(if active {
                    theme::TEXT
                } else {
                    theme::TEXT_MUTED
                }))
                .when(active, |el| el.bg(rgb(theme::SURFACE_HI)))
                .when(!active, |el| el.hover(|s| s.bg(rgb(0x14171c))))
                .child(icon(
                    glyph,
                    16.,
                    if active {
                        theme::TEXT
                    } else {
                        theme::TEXT_MUTED
                    },
                ))
                .child(div().flex_1().child(label))
                .when_some(count, |el, count| {
                    el.child(
                        div()
                            .text_size(px(12.))
                            .font_family(theme::MONO_FONT)
                            .text_color(rgb(theme::TEXT_DIM))
                            .child(count.to_string()),
                    )
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.recording {
                        this.page = page;
                        cx.notify();
                    }
                }))
        };

        div()
            .w(px(208.))
            .flex_none()
            .h_full()
            .bg(rgb(theme::SIDEBAR))
            .border_r_1()
            .border_color(rgb(theme::DIVIDER))
            .pb(px(14.))
            .px(px(12.))
            .flex()
            .flex_col()
            .gap(px(18.))
            .child(
                // 应用名占据标题栏左侧，可拖动窗口。
                div()
                    .h(px(header.max(36.)))
                    .flex_none()
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .window_control_area(WindowControlArea::Drag)
                    .child(logo(22.))
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("OmniTrigger"),
                    ),
            )
            .child(
                div()
                    .rounded(px(10.))
                    .p(px(12.))
                    .bg(rgb(card_bg))
                    .border_1()
                    .border_color(rgb(card_line))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .size(px(14.))
                                    .rounded(px(7.))
                                    .bg(hsla(color, 0.18))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(dot(color, 8.)),
                            )
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(label_color))
                                    .child(label),
                            ),
                    )
                    .child(
                        div()
                            .mt(px(-4.))
                            .text_size(px(12.))
                            .text_color(rgb(theme::TEXT_DIM))
                            .child(sub),
                    )
                    .child(action),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(nav(
                        "nav-triggers",
                        Page::Triggers,
                        Icon::Keyboard,
                        "触发器",
                        Some(self.bindings.len()),
                    ))
                    .child(nav(
                        "nav-history",
                        Page::History,
                        Icon::Clock,
                        "触发记录",
                        Some(self.hits.len()),
                    ))
                    .child(nav(
                        "nav-diagnostics",
                        Page::Diagnostics,
                        Icon::Pulse,
                        "输入诊断",
                        None,
                    )),
            )
            .child(div().flex_1())
            .child(
                div()
                    .px(px(6.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(
                        div()
                            .id("sidebar-rebuild")
                            .h(px(30.))
                            .rounded(px(7.))
                            .border_1()
                            .border_color(rgb(0x262c36))
                            .text_color(rgb(theme::TEXT_MUTED))
                            .text_size(px(12.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap(px(6.))
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(0x14171c)))
                            .child(icon(Icon::Refresh, 13., theme::TEXT_MUTED))
                            .child(if self.restarting {
                                "正在重建…"
                            } else {
                                "重建监听"
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.rebuild_backend(cx))),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(theme::TEXT_FAINT))
                            .flex()
                            .justify_center()
                            .child("配置自动保存在本机"),
                    ),
            )
    }

    // ───────────────────────────── 触发器页 ─────────────────────────────

    fn render_banner(&self, cx: &mut Context<Self>) -> Option<Div> {
        if let Some(error) = &self.error {
            let offline = self.backend.is_none() && !self.restarting;
            return Some(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(12.))
                    .py(px(10.))
                    .rounded(px(9.))
                    .bg(rgb(0x2a1619))
                    .border_1()
                    .border_color(rgb(theme::DANGER_LINE))
                    .child(icon(Icon::Alert, 16., theme::DANGER))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.))
                            .text_color(rgb(theme::DANGER_TEXT))
                            .child(error.clone()),
                    )
                    .child(if offline {
                        small(button(
                            "banner-rebuild",
                            "重建监听",
                            Tone::DangerSolid,
                            true,
                        ))
                        .on_click(cx.listener(|this, _, _, cx| this.rebuild_backend(cx)))
                    } else {
                        small(button("banner-dismiss", "知道了", Tone::Secondary, true)).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.error = None;
                                cx.notify();
                            }),
                        )
                    }),
            );
        }
        (self.listener() == Listener::Paused).then(|| {
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .px(px(12.))
                .py(px(10.))
                .rounded(px(9.))
                .bg(rgb(0x171a20))
                .border_1()
                .border_color(rgb(theme::BORDER_HI))
                .child(icon(Icon::Pause, 16., theme::TEXT_MUTED))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(12.))
                        .text_color(rgb(theme::TEXT_SOFT))
                        .child("监听已暂停。触发器不会响应，按键和滚轮照常交给应用。"),
                )
                .child(
                    small(button("banner-resume", "恢复监听", Tone::Secondary, true))
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_pause(cx))),
                )
        })
    }

    fn render_list(&self, cx: &mut Context<Self>) -> Div {
        let listening = self.listener() == Listener::Listening;
        let dim = matches!(self.listener(), Listener::Paused | Listener::Offline);
        let mut rows = div()
            .id("trigger-rows")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(8.))
            .when(dim, |el| el.opacity(0.6));
        for binding in &self.bindings {
            let id = binding.spec.id;
            let selected = self.pending.is_none() && !self.recording && self.selected == Some(id);
            let stats = self.stats.get(&id).cloned().unwrap_or_default();
            let (hits_label, hits_color) = if stats.hits > 0 {
                (format!("{} 次", stats.hits), theme::TEXT_SOFT)
            } else {
                ("未触发".to_string(), theme::TEXT_FAINT)
            };
            let last = if !binding.enabled {
                "已停用".to_string()
            } else {
                stats
                    .last
                    .map(|last| format!("最近 {}", &last[..5]))
                    .unwrap_or_default()
            };
            rows = rows.child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .px(px(14.))
                    .py(px(12.))
                    .rounded(px(10.))
                    .bg(rgb(if selected {
                        theme::SELECTED
                    } else {
                        theme::ROW
                    }))
                    .border_1()
                    .border_color(rgb(if selected {
                        theme::KEYCAP_EDGE
                    } else {
                        theme::BORDER
                    }))
                    .child(
                        div()
                            .id(("row", id))
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .cursor_pointer()
                            .when(!binding.enabled, |el| el.opacity(0.5))
                            .child(combo(&caps_for(&binding.spec), CapSize::Medium))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(rgb(theme::TEXT_MUTED))
                                    .truncate()
                                    .child(binding.message.clone()),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| this.select(id, cx))),
                    )
                    .child(
                        div()
                            .flex_none()
                            .min_w(px(64.))
                            .flex()
                            .flex_col()
                            .items_end()
                            .gap(px(2.))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_family(theme::MONO_FONT)
                                    .text_color(rgb(hits_color))
                                    .child(hits_label),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(theme::TEXT_FAINT))
                                    .child(last),
                            ),
                    )
                    .child(
                        switch(("switch", id), binding.enabled)
                            .on_click(cx.listener(move |this, _, _, cx| this.toggle_rule(id, cx))),
                    ),
            );
        }
        if self.bindings.is_empty() {
            rows = rows.child(
                div()
                    .py(px(48.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .text_size(px(14.))
                            .text_color(rgb(theme::TEXT_SOFT))
                            .child("还没有触发器"),
                    )
                    .child(caption("点击右上角“录制组合”，按下一个组合即可添加。")),
            );
        }
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .pt(px(12.))
            .pb(px(16.))
            .pl(px(24.))
            .pr(px(20.))
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(
                div()
                    .flex()
                    .items_end()
                    .justify_between()
                    .gap(px(12.))
                    .child(heading(
                        "触发器",
                        "按住前缀，再按目标键或滚动滚轮；在任意应用中生效",
                    ))
                    .child(
                        icon_button("record", Icon::Record, "录制组合", Tone::Primary, listening)
                            .on_click(cx.listener(|this, _, _, cx| this.start_recording(cx))),
                    ),
            )
            .children(self.render_banner(cx))
            .child(rows)
            .child(caption(
                "最多 64 条触发器，每条最多 8 个前缀。单独按下未用于组合的前缀时，原输入会补发给应用。",
            ))
    }

    fn inspector_header(title: &'static str, right: impl IntoElement) -> Div {
        div()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .text_size(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::TEXT_MUTED))
                    .child(title),
            )
            .child(right)
    }

    fn message_field(&self, label: &'static str, hint: Option<&'static str>) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::TEXT_SOFT))
                    .child(label),
            )
            .child(self.message.clone())
            .when_some(hint, |el, hint| el.child(caption(hint)))
    }

    fn render_recording(&self) -> Vec<AnyElement> {
        let held = caps_for_buttons(&self.recording_held);
        let mut live = div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_center()
            .gap(px(8.));
        for (index, cap) in held.iter().enumerate() {
            if index > 0 {
                live = live.child(plus(CapSize::Large, theme::RECORD_DIM));
            }
            live = live.child(keycap(cap, CapSize::Large, CapTone::Held));
        }
        if held.len() <= 1 {
            if !held.is_empty() {
                live = live.child(plus(CapSize::Large, theme::RECORD_DIM));
            }
            live = live.child(ghost_cap());
        }
        let hint = match held.len() {
            0 => "按住前缀，再按目标键或滚动滚轮",
            1 => "已按住前缀，再按目标键或滚动滚轮",
            _ => "全部松开后完成录制",
        };
        let bullet = |text: &'static str| {
            div()
                .flex()
                .gap(px(8.))
                .text_size(px(12.))
                .text_color(rgb(theme::TEXT_MUTED))
                .child(div().mt(px(7.)).child(dot(theme::TEXT_FAINT, 4.)))
                .child(div().flex_1().child(text))
        };
        vec![
            Self::inspector_header(
                "录制新组合",
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(12.))
                    .text_color(rgb(theme::RECORD))
                    .child(dot(theme::RECORD, 7.))
                    .child("录制中"),
            )
            .into_any_element(),
            div()
                .h(px(236.))
                .rounded(px(12.))
                .border_1()
                .border_dashed()
                .border_color(rgb(theme::RECORD_LINE))
                .bg(rgb(theme::RECORD_BG))
                .p(px(16.))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(16.))
                .child(
                    div()
                        .size(px(44.))
                        .rounded(px(22.))
                        .border_2()
                        .border_color(rgb(0x3e3121))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon(Icon::Record, 30., theme::RECORD)),
                )
                .child(live)
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(rgb(0xe9d6bc))
                        .child(hint),
                )
                .into_any_element(),
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(bullet("录制期间输入暂被拦截，不会传给其他应用。"))
                .child(bullet("其他软件注入的模拟输入不参与录制。"))
                .child(bullet(
                    "鼠标点击也会被录制；单独的左键、右键不能作为触发器。",
                ))
                .into_any_element(),
            div().flex_1().into_any_element(),
            div()
                .h(px(34.))
                .rounded(px(8.))
                .border_1()
                .border_color(rgb(theme::BORDER_MID))
                .flex()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .text_size(px(13.))
                .text_color(rgb(theme::TEXT_SOFT))
                .child("按")
                .child(keycap(
                    &Cap {
                        label: "Esc".into(),
                        mouse: false,
                        arrow: None,
                    },
                    CapSize::Small,
                    CapTone::Normal,
                ))
                .child("取消录制")
                .into_any_element(),
        ]
    }

    fn render_captured(&self, spec: &TriggerSpec, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let conflict = self.conflict.clone();
        let ok = conflict.is_none();
        let status = match &conflict {
            None => div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(12.))
                .text_color(rgb(theme::ACCENT))
                .child(icon(Icon::Check, 14., theme::ACCENT))
                .child("没有冲突，可以添加"),
            Some(conflict) => div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(12.))
                .text_color(rgb(theme::DANGER))
                .child(icon(Icon::Alert, 14., theme::DANGER))
                .child(conflict.title),
        };
        let mut items = vec![
            Self::inspector_header(
                "录制新组合",
                small(icon_button(
                    "rerecord-top",
                    Icon::Refresh,
                    "重新录制",
                    Tone::Ghost,
                    true,
                ))
                .on_click(cx.listener(|this, _, _, cx| this.start_recording(cx))),
            )
            .into_any_element(),
            div()
                .rounded(px(12.))
                .border_1()
                .border_color(rgb(if ok {
                    theme::ACCENT_LINE
                } else {
                    theme::DANGER_LINE
                }))
                .bg(rgb(if ok {
                    theme::ACCENT_BG
                } else {
                    theme::DANGER_BG
                }))
                .px(px(16.))
                .py(px(18.))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(12.))
                .child(combo(&caps_for(spec), CapSize::Large).justify_center())
                .child(status)
                .into_any_element(),
        ];
        match conflict {
            None => {
                items.push(self.message_field("提示消息", None).into_any_element());
                items.push(div().flex_1().into_any_element());
                items.push(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(
                            button("add", "添加触发器", Tone::Primary, true)
                                .flex_1()
                                .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                        )
                        .child(
                            button("preview-draft", "预览提示", Tone::Secondary, true)
                                .px(px(12.))
                                .on_click(cx.listener(|this, _, _, cx| this.preview_draft(cx))),
                        )
                        .child(
                            button("discard", "放弃", Tone::Ghost, true)
                                .px(px(12.))
                                .on_click(cx.listener(|this, _, _, cx| this.discard_pending(cx))),
                        )
                        .into_any_element(),
                );
            }
            Some(conflict) => {
                let mut detail = div().flex().flex_col().gap(px(8.)).child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(theme::TEXT_MUTED))
                        .child(conflict.detail.clone()),
                );
                if let Some(other) = conflict.other.and_then(|id| self.binding(id)) {
                    let other_id = other.spec.id;
                    detail =
                        detail.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(10.))
                                .px(px(12.))
                                .py(px(10.))
                                .rounded(px(9.))
                                .bg(rgb(theme::SURFACE))
                                .border_1()
                                .border_color(rgb(0x262c36))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .gap(px(6.))
                                        .child(combo(&caps_for(&other.spec), CapSize::Small))
                                        .child(
                                            div()
                                                .text_size(px(12.))
                                                .text_color(rgb(theme::TEXT_MUTED))
                                                .truncate()
                                                .child(other.message.clone()),
                                        ),
                                )
                                .child(
                                    small(button("view-conflict", "查看", Tone::Secondary, true))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.select(other_id, cx)
                                        })),
                                ),
                        );
                }
                items.push(detail.into_any_element());
                items.push(div().flex_1().into_any_element());
                items.push(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(
                            button("rerecord", "重新录制", Tone::Primary, true)
                                .flex_1()
                                .on_click(cx.listener(|this, _, _, cx| this.start_recording(cx))),
                        )
                        .child(
                            button("discard", "取消", Tone::Secondary, true)
                                .on_click(cx.listener(|this, _, _, cx| this.discard_pending(cx))),
                        )
                        .into_any_element(),
                );
            }
        }
        items
    }

    fn render_edit(&self, binding: &Binding, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let id = binding.spec.id;
        let stats = self.stats.get(&id).cloned().unwrap_or_default();
        let draft = self.message.read(cx).text();
        let changed = draft != binding.message && !draft.trim().is_empty();
        let confirming = self.confirm_delete == Some(id);
        let mini = |label: &'static str, value: String| {
            div()
                .flex_1()
                .px(px(12.))
                .py(px(10.))
                .rounded(px(9.))
                .bg(rgb(theme::SURFACE))
                .border_1()
                .border_color(rgb(0x22272f))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(theme::TEXT_DIM))
                        .child(label),
                )
                .child(
                    div()
                        .text_size(px(18.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .font_family(theme::MONO_FONT)
                        .child(value),
                )
        };
        vec![
            Self::inspector_header(
                "触发器详情",
                div()
                    .text_size(px(12.))
                    .text_color(rgb(if binding.enabled {
                        theme::ACCENT
                    } else {
                        theme::TEXT_DIM
                    }))
                    .child(if binding.enabled {
                        "已启用"
                    } else {
                        "已停用"
                    }),
            )
            .into_any_element(),
            div()
                .min_h(px(44.))
                .child(combo(&caps_for(&binding.spec), CapSize::Large))
                .into_any_element(),
            div()
                .flex()
                .gap(px(8.))
                .child(mini("真实触发", stats.hits.to_string()))
                .child(mini("最近一次", stats.last.unwrap_or_else(|| "—".into())))
                .into_any_element(),
            self.message_field(
                "提示消息",
                Some("触发时在屏幕右下角显示 4 秒，不抢占当前窗口焦点。"),
            )
            .into_any_element(),
            div()
                .flex()
                .gap(px(8.))
                .child(
                    button("save", "保存消息", Tone::Primary, changed)
                        .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                )
                .child(
                    icon_button("preview", Icon::Play, "测试提示", Tone::Secondary, true)
                        .px(px(12.))
                        .on_click(cx.listener(|this, _, _, cx| this.preview_draft(cx))),
                )
                .into_any_element(),
            div().flex_1().into_any_element(),
            div()
                .pt(px(12.))
                .border_t_1()
                .border_color(rgb(theme::DIVIDER))
                .flex()
                .items_center()
                .justify_between()
                .child(caption("测试提示不计入触发次数"))
                .child(
                    small(if confirming {
                        button("delete", "再次点击确认删除", Tone::DangerSolid, true)
                    } else {
                        icon_button("delete", Icon::Trash, "删除触发器", Tone::Danger, true)
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.remove_rule(id, cx))),
                )
                .into_any_element(),
        ]
    }

    fn render_inspector(&self, cx: &mut Context<Self>) -> Div {
        let children = if self.recording {
            self.render_recording()
        } else if let Some(spec) = self.pending.clone() {
            self.render_captured(&spec, cx)
        } else if let Some(binding) = self.selected.and_then(|id| self.binding(id)).cloned() {
            self.render_edit(&binding, cx)
        } else {
            vec![
                Self::inspector_header("触发器详情", div()).into_any_element(),
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(rgb(theme::TEXT_MUTED))
                            .child("选择一个触发器查看详情，或录制新组合。"),
                    )
                    .child(
                        icon_button(
                            "record-empty",
                            Icon::Record,
                            "录制组合",
                            Tone::Primary,
                            self.listener() == Listener::Listening,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.start_recording(cx))),
                    )
                    .into_any_element(),
            ]
        };
        div()
            .w(px(340.))
            .flex_none()
            .h_full()
            .bg(rgb(theme::INSPECTOR))
            .border_l_1()
            .border_color(rgb(theme::DIVIDER))
            .pt(px(12.))
            .pb(px(18.))
            .px(px(20.))
            .flex()
            .flex_col()
            .gap(px(16.))
            .children(children)
    }

    // ───────────────────────────── 记录与诊断 ─────────────────────────────

    fn render_history(&self, cx: &mut Context<Self>) -> Div {
        let filter = self.history_filter;
        let segment = |id: &'static str, label: &'static str, value: HistoryFilter| {
            let active = filter == value;
            div()
                .id(id)
                .h(px(26.))
                .px(px(12.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .text_size(px(12.))
                .cursor_pointer()
                .text_color(rgb(if active {
                    theme::TEXT
                } else {
                    theme::TEXT_MUTED
                }))
                .when(active, |el| el.bg(rgb(0x262c36)))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.history_filter = value;
                    cx.notify();
                }))
        };
        let mut list = div()
            .id("history-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .rounded(px(10.))
            .border_1()
            .border_color(rgb(theme::BORDER))
            .bg(rgb(theme::INSPECTOR));
        let mut shown = 0;
        for hit in self.hits.iter().filter(|hit| match filter {
            HistoryFilter::All => true,
            HistoryFilter::Real => !hit.preview,
            HistoryFilter::Preview => hit.preview,
        }) {
            shown += 1;
            let (tag, tag_bg, tag_fg) = if hit.preview {
                ("测试", theme::SURFACE_HI, theme::TEXT_MUTED)
            } else {
                ("触发", theme::ACCENT_DEEP, theme::ACCENT)
            };
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .px(px(16.))
                    .py(px(11.))
                    .border_b_1()
                    .border_color(rgb(0x1b1f26))
                    .child(
                        div()
                            .w(px(64.))
                            .flex_none()
                            .font_family(theme::MONO_FONT)
                            .text_size(px(12.))
                            .text_color(rgb(theme::TEXT_DIM))
                            .child(hit.time.clone()),
                    )
                    .child(
                        div()
                            .w(px(40.))
                            .flex_none()
                            .py(px(2.))
                            .rounded(px(5.))
                            .bg(rgb(tag_bg))
                            .text_color(rgb(tag_fg))
                            .text_size(px(11.))
                            .flex()
                            .justify_center()
                            .child(tag),
                    )
                    .child(div().w(px(250.)).flex_none().overflow_hidden().child(
                        if hit.caps.is_empty() {
                            caption("界面测试")
                        } else {
                            combo(&hit.caps, CapSize::Small).flex_nowrap()
                        },
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(13.))
                            .text_color(rgb(theme::TEXT_SOFT))
                            .child(hit.message.clone()),
                    ),
            );
        }
        if shown == 0 {
            list = list.child(
                div()
                    .py(px(48.))
                    .flex()
                    .justify_center()
                    .text_size(px(13.))
                    .text_color(rgb(theme::TEXT_DIM))
                    .child("按下一个组合，这里会出现触发记录。"),
            );
        }
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .pt(px(12.))
            .pb(px(16.))
            .px(px(24.))
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(
                div()
                    .flex()
                    .items_end()
                    .justify_between()
                    .gap(px(12.))
                    .child(heading(
                        "触发记录",
                        format!(
                            "本次运行真实触发 {} 次，测试提示单独标记，不计入次数",
                            self.trigger_count
                        ),
                    ))
                    .child(
                        div()
                            .flex()
                            .p(px(3.))
                            .rounded(px(8.))
                            .bg(rgb(theme::SURFACE))
                            .border_1()
                            .border_color(rgb(0x22272f))
                            .child(segment("filter-all", "全部", HistoryFilter::All))
                            .child(segment("filter-real", "触发", HistoryFilter::Real))
                            .child(segment("filter-preview", "测试", HistoryFilter::Preview)),
                    ),
            )
            .child(list)
            .child(caption("保留最近 30 条，仅在本次运行期间。"))
    }

    fn render_diagnostics(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let d = &self.diagnostics;
        let age = |ms: Option<u64>| {
            ms.map(|ms| format!("最近 {:.1} 秒前", ms as f64 / 1000.))
                .unwrap_or_else(|| "尚无事件".into())
        };
        let held = caps_for_buttons(&d.held);
        let info = |title: &'static str| {
            div()
                .flex_1()
                .min_w_0()
                .p(px(16.))
                .rounded(px(10.))
                .bg(rgb(theme::INSPECTOR))
                .border_1()
                .border_color(rgb(theme::BORDER))
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
        };
        let para = |text: &'static str| {
            div()
                .text_size(px(12.))
                .text_color(rgb(theme::TEXT_MUTED))
                .child(text)
        };
        div()
            .id("diagnostics")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .pt(px(12.))
            .pb(px(16.))
            .px(px(24.))
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(heading(
                "输入诊断",
                "每秒更新。用于判断监听是否还在收到键盘和鼠标事件。",
            ))
            .child(
                div()
                    .flex()
                    .gap(px(10.))
                    .child(stat(
                        "当前监听",
                        format!("#{}", self.listener_generation),
                        Some(("重建后加一".into(), theme::TEXT_FAINT)),
                    ))
                    .child(stat(
                        "键盘回调",
                        thousands(d.keyboard_events),
                        Some((
                            age(d.last_keyboard_ms),
                            if d.last_keyboard_ms.is_some() {
                                theme::ACCENT
                            } else {
                                theme::TEXT_FAINT
                            },
                        )),
                    ))
                    .child(stat(
                        "鼠标回调",
                        thousands(d.mouse_events),
                        Some((age(d.last_mouse_ms), theme::TEXT_FAINT)),
                    )),
            )
            .child(
                div()
                    .flex()
                    .gap(px(10.))
                    .child(stat(
                        "外部模拟输入",
                        thousands(d.external_injected),
                        Some(("不触发、不参与录制".into(), theme::TEXT_FAINT)),
                    ))
                    .child(
                        card()
                            .flex_grow()
                            .flex_basis(px(0.))
                            .min_w_0()
                            .px(px(16.))
                            .py(px(14.))
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .flex_1()
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(rgb(theme::TEXT_DIM))
                                    .child("当前按住"),
                            )
                            .child(if held.is_empty() {
                                div()
                                    .min_h(px(30.))
                                    .flex()
                                    .items_center()
                                    .text_size(px(13.))
                                    .text_color(rgb(theme::TEXT_SOFT))
                                    .child("无")
                            } else {
                                combo(&held, CapSize::Medium).min_h(px(30.))
                            })
                            .child(caption(format!(
                                "自身补发 {} · 切换窗口取消 {} · 补发失败 {}",
                                thousands(d.self_replayed),
                                thousands(d.foreground_cancellations),
                                thousands(d.replay_failures),
                            ))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap(px(10.))
                    .child(
                        info("偶发失效时")
                            .child(para("1. 松开所有按键和鼠标按钮。"))
                            .child(para(
                                "2. 点击“重建监听”，旧监听会先收尾已消费的按压。",
                            ))
                            .child(para("3. 触发器、暂停状态和未保存的录制结果都会保留。"))
                            .child(
                                div().flex().child(
                                    icon_button(
                                        "diag-rebuild",
                                        Icon::Refresh,
                                        "重建监听",
                                        Tone::Secondary,
                                        true,
                                    )
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.rebuild_backend(cx)),
                                        ),
                                ),
                            ),
                    )
                    .child(
                        info("与其他输入工具并用")
                            .child(para(
                                "Windows 按“后安装先处理”的顺序调用低级 Hook。其他软件若在上游拦截 CapsLock、侧键或滚轮，这里可能收不到原输入。",
                            ))
                            .child(para(
                                "程序不会定时重装 Hook 抢占链首；仅凭漏触发无法判断具体来源。",
                            )),
                    ),
            )
    }

    /// 标题栏高度与系统按钮宽度（逻辑像素），并同步按钮下方的底色。
    fn caption_area(&mut self, window: &Window) -> (f32, f32) {
        let Some(hwnd) = self.title_bar else {
            return (0., 0.);
        };
        let color = if self.page == Page::Triggers {
            theme::INSPECTOR
        } else {
            theme::BACKGROUND
        };
        if self.caption_color != Some(color) {
            omnitrigger_windows::set_caption_color(hwnd, color);
            self.caption_color = Some(color);
        }
        let scale = window.scale_factor();
        omnitrigger_windows::caption_buttons(hwnd)
            .map(|buttons| (buttons.height as f32 / scale, buttons.width as f32 / scale))
            .unwrap_or((32., 138.))
    }

    /// 页面各栏的底色延伸进标题栏，右上角留空给系统按钮。
    fn render_title_bar(&self, height: f32, buttons_width: f32) -> Div {
        let reserve = div().flex_none().w(px(buttons_width)).h_full();
        let band = div()
            .flex_none()
            .h(px(height))
            .flex()
            .window_control_area(WindowControlArea::Drag)
            .child(div().flex_1().h_full().bg(rgb(theme::BACKGROUND)));
        if self.page == Page::Triggers {
            band.child(
                div()
                    .w(px(340.))
                    .flex_none()
                    .h_full()
                    .border_l_1()
                    .border_color(rgb(theme::DIVIDER))
                    .flex()
                    .child(div().flex_1().h_full().bg(rgb(theme::INSPECTOR)))
                    .child(reserve),
            )
        } else {
            band.child(reserve)
        }
    }

    fn render_status_bar(&self) -> Div {
        let color = match self.listener() {
            Listener::Listening => theme::ACCENT,
            Listener::Recording => theme::RECORD,
            Listener::Offline => theme::DANGER,
            _ => theme::TEXT_FAINT,
        };
        let held = if self.diagnostics.held.is_empty() {
            "无".to_string()
        } else {
            self.diagnostics
                .held
                .iter()
                .take(4)
                .map(format_button)
                .collect::<Vec<_>>()
                .join(" + ")
        };
        div()
            .h(px(30.))
            .flex_none()
            .px(px(16.))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(16.))
            .bg(rgb(theme::SIDEBAR))
            .border_t_1()
            .border_color(rgb(theme::DIVIDER))
            .text_size(px(12.))
            .text_color(rgb(theme::TEXT_DIM))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .min_w_0()
                    .child(dot(color, 6.))
                    .child(div().truncate().child(self.status.clone())),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .gap(px(14.))
                    .text_size(px(11.))
                    .text_color(rgb(theme::TEXT_FAINT))
                    .child(format!(
                        "键盘 {}",
                        thousands(self.diagnostics.keyboard_events)
                    ))
                    .child(format!("鼠标 {}", thousands(self.diagnostics.mouse_events)))
                    .child(format!(
                        "模拟 {}",
                        thousands(self.diagnostics.external_injected)
                    ))
                    .child(format!("按住 {held}")),
            )
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (title_height, buttons_width) = self.caption_area(window);
        let main = match self.page {
            Page::Triggers => div()
                .flex_1()
                .min_w_0()
                .flex()
                .child(self.render_list(cx))
                .child(self.render_inspector(cx)),
            Page::History => div()
                .flex_1()
                .min_w_0()
                .flex()
                .child(self.render_history(cx)),
            Page::Diagnostics => div()
                .flex_1()
                .min_w_0()
                .flex()
                .child(self.render_diagnostics(cx)),
        };
        // 根节点不铺底色：系统标题按钮绘制在界面下方，需透出。
        div()
            .size_full()
            .text_color(rgb(theme::TEXT))
            .font_family(theme::UI_FONT)
            .text_size(px(13.))
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(self.render_sidebar(title_height, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .when(title_height > 0., |el| {
                                el.child(self.render_title_bar(title_height, buttons_width))
                            })
                            .child(main.min_h_0().bg(rgb(theme::BACKGROUND))),
                    ),
            )
            .child(self.render_status_bar())
    }
}

struct Toast {
    caps: Vec<Cap>,
    message: String,
    preview: bool,
    shown: Instant,
}

impl Render for Toast {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let progress = (1. - self.shown.elapsed().as_secs_f32() / TOAST_SECONDS).clamp(0., 1.);
        let header = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(logo(16.))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(theme::TEXT_MUTED))
                    .child("OmniTrigger"),
            )
            .when(self.preview, |el| {
                el.child(
                    div()
                        .px(px(6.))
                        .rounded(px(4.))
                        .bg(rgb(theme::SURFACE_HI))
                        .border_1()
                        .border_color(rgb(theme::BORDER_MID))
                        .text_size(px(11.))
                        .text_color(rgb(theme::TEXT_MUTED))
                        .child("测试"),
                )
            })
            .child(div().flex_1())
            .when(!self.caps.is_empty(), |el| {
                el.child(combo(&self.caps, CapSize::Small).flex_none().flex_nowrap())
            });
        div()
            .w(px(TOAST_WIDTH))
            .h(px(TOAST_HEIGHT))
            .font_family(theme::UI_FONT)
            .text_color(rgb(theme::TEXT))
            .rounded(px(12.))
            .bg(rgb(0x15181e))
            .border_1()
            .border_color(rgb(if self.preview {
                theme::BORDER_MID
            } else {
                0x2f4a3d
            }))
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .px(px(16.))
                    .pt(px(14.))
                    .pb(px(12.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(header)
                    .child(
                        div()
                            .text_size(px(15.))
                            .line_height(px(22.))
                            .line_clamp(2)
                            .child(self.message.clone()),
                    ),
            )
            .child(
                div()
                    .h(px(2.))
                    .flex_none()
                    .bg(rgb(if self.preview {
                        theme::DIVIDER
                    } else {
                        theme::ACCENT_DEEP
                    }))
                    .child(
                        div()
                            .h_full()
                            .w(gpui::relative(progress))
                            .bg(rgb(if self.preview {
                                theme::PLUS
                            } else {
                                theme::ACCENT
                            })),
                    ),
            )
    }
}

fn smoke_test() -> Result<(), String> {
    let bindings = settings::defaults();
    settings::validate(&bindings)?;
    let (backend, events) = Backend::start()?;
    match events
        .recv_timeout(Duration::from_secs(5))
        .map_err(|e| e.to_string())?
    {
        BackendEvent::Ready => {}
        event => return Err(format!("输入后端未就绪：{event:?}")),
    }
    backend.send(BackendCommand::SetBindings(
        bindings.into_iter().map(|b| b.spec).collect(),
    ))?;
    backend.send(BackendCommand::SetEnabled(false))?;
    drop(backend);
    println!("OmniTrigger smoke test: keyboard/mouse hooks installed and cleanly removed; default registrations valid.");
    Ok(())
}

fn main() {
    let arguments: Vec<_> = std::env::args().collect();
    if arguments.iter().any(|argument| argument == "--smoke-test") {
        if let Err(error) = smoke_test() {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    let ui_smoke = arguments.iter().any(|argument| argument == "--ui-smoke");
    Application::new()
        .with_assets(Assets)
        .run(move |cx: &mut App| {
            message_input::init(cx);
            let bounds = Bounds::centered(None, size(px(1060.), px(780.)), cx);
            let result = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(920.), px(620.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("OmniTrigger".into()),
                        ..Default::default()
                    }),
                    app_id: Some("OmniTrigger".into()),
                    ..Default::default()
                },
                |window, cx| {
                    let title_bar = match window.window_handle().map(|handle| handle.as_raw()) {
                        Ok(RawWindowHandle::Win32(handle)) => {
                            let hwnd = handle.hwnd.get();
                            omnitrigger_windows::extend_into_title_bar(hwnd)
                                .map(|()| hwnd)
                                .map_err(|error| format!("{error}，已改用系统标题栏。"))
                        }
                        _ => Err("无法取得主窗口句柄，已改用系统标题栏。".into()),
                    };
                    let view = cx.new(|cx| Workspace::new(cx, ui_smoke, title_bar));
                    let closing_view = view.downgrade();
                    window.on_window_should_close(cx, move |_, cx| {
                        let _ = closing_view.update(cx, |view, _| {
                            view.backend.take();
                        });
                        cx.quit();
                        true
                    });
                    view
                },
            );
            if let Err(error) = result {
                eprintln!("无法启动 OmniTrigger 界面：{error}");
                cx.quit();
            }
            cx.activate(true);
        });
}

#[cfg(test)]
mod tests {
    use super::thousands;

    #[test]
    fn thousands_groups_digits_from_the_right() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1284), "1,284");
        assert_eq!(thousands(1234567), "1,234,567");
    }
}
