#![windows_subsystem = "windows"]

mod message_input;
mod settings;

use gpui::{
    div, point, prelude::*, px, rgb, size, App, Application, Bounds, Context, Div, Entity,
    Focusable, FontWeight, SharedString, Stateful, TitlebarOptions, Window, WindowBounds,
    WindowHandle, WindowKind, WindowOptions,
};
use message_input::MessageInput;
use omnitrig_core::TriggerSpec;
use omnitrig_windows::{
    format_button, format_trigger, Backend, BackendCommand, BackendEvent, ListenerDiagnostics,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use settings::Binding;
use std::{
    collections::VecDeque,
    sync::mpsc::{Receiver, TryRecvError},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const BACKGROUND: u32 = 0x101318;
const PANEL: u32 = 0x191e26;
const BORDER: u32 = 0x2b3441;
const MUTED: u32 = 0x94a3b8;
const ACCENT: u32 = 0x83e8b7;

fn button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    accent: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .px_3()
        .py_2()
        .rounded_md()
        .cursor_pointer()
        .border_1()
        .border_color(rgb(if accent { ACCENT } else { BORDER }))
        .bg(rgb(if accent { 0x244537 } else { 0x222a35 }))
        .text_color(rgb(if accent { ACCENT } else { 0xd5dce7 }))
        .text_sm()
        .hover(|s| s.bg(rgb(if accent { 0x315e48 } else { 0x343f50 })))
        .child(label.into())
}

struct Hit {
    time: String,
    trigger: String,
    message: String,
    preview: bool,
}

struct Workspace {
    backend: Option<Backend>,
    events: Receiver<BackendEvent>,
    ready: bool,
    paused: bool,
    recording: bool,
    pending: Option<TriggerSpec>,
    editing: Option<u64>,
    message: Entity<MessageInput>,
    bindings: Vec<Binding>,
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
}

impl Workspace {
    fn new(cx: &mut Context<Self>, ui_smoke: bool) -> Self {
        let (bindings, load_error) = match settings::load() {
            Ok(bindings) => (bindings, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        let (backend, events, startup_error) = match Backend::start() {
            Ok((backend, events)) => (Some(backend), events, None),
            Err(error) => (None, std::sync::mpsc::channel().1, Some(error)),
        };
        let message = cx.new(MessageInput::new);
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
            pending: None,
            editing: None,
            message,
            bindings,
            error: startup_error.or(load_error),
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
                        self.error = Some("输入监听线程已结束，请点击“重建监听”。".into());
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
                        "监听已就绪，已保留暂停状态；点击“恢复监听”后测试。"
                    } else {
                        "监听已就绪，可以切换到其他窗口测试。"
                    }
                    .into();
                    // 初始化/重建时先暂停再恢复配置，避免暂停状态短暂失效。
                    self.send(BackendCommand::SetEnabled(false));
                    self.sync_bindings();
                    self.send(BackendCommand::SetEnabled(!self.paused));
                }
                BackendEvent::Recorded(spec) => {
                    self.recording = false;
                    self.pending = Some(spec);
                    self.editing = None;
                    self.status = "已录制，填写提示消息后点击“添加触发器”。".into();
                }
                BackendEvent::Triggered(id) => self.fire(id, false, cx),
                BackendEvent::Error(error) => {
                    self.error = Some(error);
                    self.recording = false;
                    self.paused = true;
                    self.status = "监听已暂停，请处理错误后点击“恢复监听”。".into();
                    self.send(BackendCommand::SetEnabled(false));
                }
                BackendEvent::RecordingCancelled => {
                    self.recording = false;
                    self.status = "已取消录制。".into();
                }
                BackendEvent::Diagnostics(snapshot) => self.diagnostics = snapshot,
            }
        }
        if self.ready && self.last_diagnostics_request.elapsed() >= Duration::from_secs(1) {
            self.send(BackendCommand::GetDiagnostics);
            self.last_diagnostics_request = Instant::now();
        }
        if self
            .toast_time
            .is_some_and(|time| time.elapsed() >= Duration::from_secs(4))
        {
            if let Some(toast) = self.toast.take() {
                let _ = toast.update(cx, |_, window, _| window.remove_window());
            }
            self.toast_time = None;
        }
        if self.ui_smoke && self.started.elapsed() > Duration::from_millis(700) {
            self.ui_smoke = false;
            self.show_message(
                "界面测试".into(),
                "提示消息显示正常。现在可以试用真实组合输入。".into(),
                true,
                cx,
            );
        }
        if changed {
            cx.notify();
        }
    }

    fn diagnostics_summary(&self) -> String {
        let held = if self.diagnostics.held.is_empty() {
            "无".to_string()
        } else {
            let mut names = self
                .diagnostics
                .held
                .iter()
                .take(8)
                .map(format_button)
                .collect::<Vec<_>>()
                .join(" + ");
            if self.diagnostics.held.len() > 8 {
                names.push_str(" …");
            }
            names
        };
        let keyboard_age = self
            .diagnostics
            .last_keyboard_ms
            .map(|value| format!("{:.1} 秒前", value as f64 / 1000.))
            .unwrap_or_else(|| "尚无事件".into());
        format!(
            "监听 #{} · 键盘 {} / 鼠标 {} · 模拟输入 {} · 最近键盘：{} · 按住：{}",
            self.listener_generation,
            self.diagnostics.keyboard_events,
            self.diagnostics.mouse_events,
            self.diagnostics.external_injected,
            keyboard_age,
            held,
        )
    }

    fn fire(&mut self, id: u64, preview: bool, cx: &mut Context<Self>) {
        if let Some(binding) = self.bindings.iter().find(|binding| binding.spec.id == id) {
            let trigger = format_trigger(&binding.spec);
            let message = binding.message.clone();
            self.show_message(trigger, message, preview, cx);
        }
    }

    fn show_message(
        &mut self,
        trigger: String,
        message: String,
        preview: bool,
        cx: &mut Context<Self>,
    ) {
        if !preview {
            self.trigger_count += 1;
        }
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.hits.push_front(Hit {
            time: format!("{:02}:{:02}", seconds / 60 % 60, seconds % 60),
            trigger: trigger.clone(),
            message: message.clone(),
            preview,
        });
        self.hits.truncate(30);
        self.toast_time = Some(Instant::now());
        if let Some(handle) = self.toast {
            if handle
                .update(cx, |toast, _, cx| {
                    toast.trigger = trigger.clone();
                    toast.message = message.clone();
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
        let origin = point(bounds.right() - px(444.), bounds.bottom() - px(252.));
        let mut popup_error = None;
        match cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    origin,
                    size(px(420.), px(186.)),
                ))),
                kind: WindowKind::PopUp,
                titlebar: None,
                focus: false,
                show: false,
                is_resizable: false,
                is_minimizable: false,
                is_movable: false,
                app_id: Some("OmniTrig.Notification".into()),
                ..Default::default()
            },
            |window, cx| {
                let scale = window.scale_factor();
                match window.window_handle().map(|handle| handle.as_raw()) {
                    Ok(RawWindowHandle::Win32(handle)) => {
                        popup_error = omnitrig_windows::show_notification_window(
                            handle.hwnd.get(),
                            (f32::from(origin.x) * scale) as i32,
                            (f32::from(origin.y) * scale) as i32,
                            (420. * scale) as i32,
                            (186. * scale) as i32,
                        )
                        .err();
                    }
                    _ => popup_error = Some("无法取得提示窗口句柄。".into()),
                }
                cx.new(|_| Toast { trigger, message })
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

    fn start_recording(&mut self, cx: &mut Context<Self>) {
        if !self.ready || self.recording {
            return;
        }
        self.error = None;
        if self.send(BackendCommand::StartRecording) {
            self.recording = true;
            self.pending = None;
            self.editing = None;
            self.status = "请按下组合并全部松开；按 Esc 取消。".into();
        }
        cx.notify();
    }

    fn save_edit(&mut self, cx: &mut Context<Self>) {
        if self.recording {
            return;
        }
        let text = self.message.read(cx).text();
        let mut next = self.bindings.clone();
        if let Some(id) = self.editing {
            if let Some(binding) = next.iter_mut().find(|binding| binding.spec.id == id) {
                binding.message = text;
            }
        } else if let Some(mut spec) = self.pending.clone() {
            spec.id = next
                .iter()
                .map(|binding| binding.spec.id)
                .max()
                .unwrap_or(0)
                .saturating_add(1);
            next.push(Binding {
                spec,
                message: text,
                enabled: true,
            });
        } else {
            self.error = Some("请先录制一个组合。".into());
            cx.notify();
            return;
        }
        if self.apply(next) {
            self.pending = None;
            self.editing = None;
            self.status = "触发器已保存，试着在任意窗口触发它。".into();
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

    fn edit(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self.recording {
            return;
        }
        if let Some(binding) = self.bindings.iter().find(|binding| binding.spec.id == id) {
            let text = binding.message.clone();
            self.message
                .update(cx, |input, cx| input.set_text(text, cx));
            window.focus(&self.message.focus_handle(cx));
            self.editing = Some(id);
            self.pending = None;
            self.status = "修改提示文字后点击“保存消息”。".into();
            cx.notify();
        }
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

    fn remove_rule(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.recording {
            return;
        }
        let bindings = self
            .bindings
            .iter()
            .filter(|binding| binding.spec.id != id)
            .cloned()
            .collect();
        if self.apply(bindings) && self.editing == Some(id) {
            self.editing = None;
        }
        cx.notify();
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.ready && !self.paused;
        let recording = self.recording;
        let editing = self.editing;
        let has_pending = self.pending.is_some();
        let selection = self.pending.as_ref().map(format_trigger).or_else(|| {
            editing.and_then(|id| {
                self.bindings
                    .iter()
                    .find(|b| b.spec.id == id)
                    .map(|b| format_trigger(&b.spec))
            })
        });
        let mut rules = div().flex().flex_col().gap_3();
        for binding in &self.bindings {
            let id = binding.spec.id;
            rules = rules.child(
                div()
                    .p_4()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(BORDER))
                    .bg(rgb(PANEL))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(format_trigger(&binding.spec)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(if binding.enabled { ACCENT } else { MUTED }))
                                    .child(if binding.enabled {
                                        "已启用"
                                    } else {
                                        "已停用"
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(MUTED))
                            .line_clamp(2)
                            .child(binding.message.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(button(("preview", id), "测试提示", false).on_click(
                                cx.listener(move |this, _, _, cx| this.fire(id, true, cx)),
                            ))
                            .child(button(("edit", id), "编辑消息", false).on_click(
                                cx.listener(move |this, _, window, cx| this.edit(id, window, cx)),
                            ))
                            .child(
                                button(
                                    ("toggle", id),
                                    if binding.enabled { "停用" } else { "启用" },
                                    false,
                                )
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.toggle_rule(id, cx)),
                                ),
                            )
                            .child(button(("delete", id), "删除", false).on_click(
                                cx.listener(move |this, _, _, cx| this.remove_rule(id, cx)),
                            )),
                    ),
            );
        }
        if self.bindings.is_empty() {
            rules = rules.child(
                div()
                    .p_6()
                    .text_color(rgb(MUTED))
                    .child("还没有触发器。点击右侧“开始录制”添加一个。"),
            );
        }
        let mut history = div().flex().flex_col().gap_2();
        if self.hits.is_empty() {
            history = history.child(
                div()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child("按下一个组合，这里会出现触发记录。"),
            );
        }
        for hit in self.hits.iter().take(8) {
            history = history.child(
                div()
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .py_2()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(if hit.preview { MUTED } else { ACCENT }))
                            .child(format!(
                                "{}  {}{}",
                                hit.time,
                                if hit.preview {
                                    "预览 · "
                                } else {
                                    "触发 · "
                                },
                                hit.trigger
                            )),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0xc7d1df))
                            .line_clamp(2)
                            .child(hit.message.clone()),
                    ),
            );
        }
        div().size_full().bg(rgb(BACKGROUND)).text_color(rgb(0xe8edf5)).font_family("Microsoft YaHei UI").flex().flex_col()
            .child(div().px_7().py_5().border_b_1().border_color(rgb(BORDER)).flex().items_center().justify_between()
                .child(div().flex().items_center().gap_3()
                    .child(div().size_9().rounded_lg().bg(rgb(0x244537)).text_color(rgb(ACCENT)).flex().items_center().justify_center().text_xl().child("O"))
                    .child(div().flex().flex_col().child(div().text_xl().font_weight(FontWeight::BOLD).child("OmniTrig"))
                        .child(div().text_xs().text_color(rgb(MUTED)).child("组合输入 · 提示消息"))))
                .child(div().flex().items_center().gap_3()
                    .child(div().text_sm().text_color(rgb(if active { ACCENT } else { MUTED })).child(if self.restarting { "○ 正在重建" } else if self.recording { "● 正在录制" } else if active { "● 正在监听" } else if !self.ready { "○ 监听未就绪" } else { "○ 监听已暂停" }))
                    .child(button("rebuild", if self.restarting { "正在重建…" } else { "重建监听" }, false).on_click(cx.listener(|this, _, _, cx| this.rebuild_backend(cx))))
                    .child(button("pause", if self.paused { "恢复监听" } else { "暂停监听" }, false).on_click(cx.listener(|this, _, _, cx| {
                        if this.recording || !this.ready { return; }
                        let paused = !this.paused;
                        if this.send(BackendCommand::SetEnabled(!paused)) { this.paused = paused; }
                        cx.notify();
                    })))))
            .child(div().id("content").flex_1().min_h_0().overflow_y_scroll().p_7().flex().flex_col().gap_5()
                .child(div().flex().flex_col().gap_2()
                    .child(div().text_2xl().font_weight(FontWeight::BOLD).child("录下一个组合，触发一条消息。"))
                    .child(div().text_sm().text_color(rgb(MUTED)).child("按住前面的键，再按下后面的键或滚动滚轮。切换到其他应用也能触发。"))
                    .child(div().text_xs().text_color(rgb(MUTED)).child("输入偶发失效时，先松开其他按键，再点击右上角“重建监听”；底部可查看输入诊断。")))
                .when_some(self.error.clone(), |element, error| element.child(div().p_3().rounded_md().bg(rgb(0x452a2f)).text_color(rgb(0xffb6ba)).text_sm().child(error)))
                .child(div().flex().gap_5()
                    .child(div().flex_1().min_w_0().flex().flex_col().gap_3()
                        .child(div().text_sm().text_color(rgb(MUTED)).child(format!("我的触发器  {}", self.bindings.len())))
                        .child(rules))
                    .child(div().w(px(350.)).flex_shrink_0().flex().flex_col().gap_4()
                        .child(div().p_5().rounded_lg().bg(rgb(PANEL)).border_1().border_color(rgb(BORDER)).flex().flex_col().gap_4()
                            .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child(if editing.is_some() { "编辑提示消息" } else { "添加触发器" }))
                            .child(div().p_4().rounded_md().border_1().border_color(rgb(if recording { ACCENT } else { BORDER })).bg(rgb(BACKGROUND)).text_color(rgb(if selection.is_some() { ACCENT } else { MUTED }))
                                .child(selection.unwrap_or_else(|| if recording { "等待输入… 松开所有键后完成".into() } else { "例如：CapsLock + W".into() })))
                            .child(button("record", if recording { "录制中 · Esc 取消" } else { "开始录制" }, true)
                                .on_click(cx.listener(|this, _, _, cx| this.start_recording(cx))))
                            .child(div().text_xs().text_color(rgb(MUTED)).child("录制时输入会被暂时拦截。按下组合后全部松开；Esc 取消。"))
                            .child(div().text_sm().child("提示消息"))
                            .child(self.message.clone())
                            .child(div().flex().gap_2()
                                .child(button("save", if editing.is_some() { "保存消息" } else { "添加触发器" }, has_pending || editing.is_some()).on_click(cx.listener(|this, _, _, cx| this.save_edit(cx))))
                                .child(button("preview-message", "预览", false).on_click(cx.listener(|this, _, _, cx| {
                                    let text = this.message.read(cx).text();
                                    this.show_message("提示消息预览".into(), text, true, cx);
                                }))))
                            .child(div().text_xs().text_color(rgb(MUTED)).child("消息会在屏幕右下角显示 4 秒，配置自动保存在本机。")))
                        .child(div().p_5().rounded_lg().bg(rgb(PANEL)).border_1().border_color(rgb(BORDER)).flex().flex_col().gap_3()
                            .child(div().flex().items_center().justify_between().child(div().font_weight(FontWeight::SEMIBOLD).child("最近触发"))
                                .child(div().text_xs().text_color(rgb(ACCENT)).child(format!("{} 次", self.trigger_count))))
                            .child(history)))))
            .child(div().px_7().py_3().border_t_1().border_color(rgb(BORDER)).text_xs().text_color(rgb(MUTED)).flex().flex_col().gap_1()
                .child(self.status.clone())
                .child(self.diagnostics_summary()))
    }
}

struct Toast {
    trigger: String,
    message: String,
}
impl Render for Toast {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .p_5()
            .bg(rgb(PANEL))
            .border_1()
            .border_color(rgb(0x477b64))
            .rounded_lg()
            .font_family("Microsoft YaHei UI")
            .text_color(rgb(0xe8edf5))
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(ACCENT))
                    .child("●  OMNITRIG · 提示消息"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .truncate()
                    .child(self.trigger.clone()),
            )
            .child(div().text_base().line_clamp(3).child(self.message.clone()))
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
    println!("OmniTrig smoke test: keyboard/mouse hooks installed and cleanly removed; default registrations valid.");
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
    Application::new().run(move |cx: &mut App| {
        message_input::init(cx);
        let bounds = Bounds::centered(None, size(px(1060.), px(780.)), cx);
        let result = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(920.), px(620.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("OmniTrig · 组合输入与提示消息".into()),
                    ..Default::default()
                }),
                app_id: Some("OmniTrig".into()),
                ..Default::default()
            },
            |window, cx| {
                let view = cx.new(|cx| Workspace::new(cx, ui_smoke));
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
            eprintln!("无法启动 OmniTrig 界面：{error}");
            cx.quit();
        }
        cx.activate(true);
    });
}
