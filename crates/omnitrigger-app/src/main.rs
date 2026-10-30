#![windows_subsystem = "windows"]

mod action_library;
mod action_packages;
mod appearance;
mod appearance_ui;
mod assets;
mod browser_actions;
mod browser_actions_ui;
mod browser_connector;
mod browser_connector_ui;
mod event_bridge;
mod execution;
mod external_action;
mod local_action;
mod local_editor;
mod local_executor;
mod message_input;
mod plugin_install;
mod plugins;
mod presets;
mod radial_wheel;
mod service;
mod settings;
mod shortcut;
mod smooth_scroll;
mod system_action;
mod system_executor;
mod system_hud;
mod theme;
mod trigger_conditions;
mod trigger_panel;
mod wheel_editor;
mod wheel_workspace;
mod wheels;
mod window_action;

use assets::{Assets, Icon};
use event_bridge::{EventBridge, UiWake};
use execution::{CommandAction, CommandExecutor, ExecutionOutcome};
use gpui::{
    div, point, prelude::*, px, size, AnyElement, AnyWindowHandle, App, Application, Bounds,
    Context, Div, Entity, FontWeight, SharedString, Stateful, TitlebarOptions, Window,
    WindowBackgroundAppearance, WindowBounds, WindowControlArea, WindowHandle, WindowKind,
    WindowOptions,
};
use message_input::MessageInput;
use omnitrigger_core::{Activation, Button, MouseButton, TriggerSpec};
use omnitrigger_windows::{
    format_button, local_clock, Backend, BackendCommand, BackendEvent, HoldOutcome,
    HoldStartRejection, ListenerDiagnostics,
};
use plugins::{PluginActionRef, Plugins};
use presets::{DEFAULT_DRAG_DISTANCE_PX, DEFAULT_DRAG_START_MS, PRESETS};
use radial_wheel::{RadialWheel, WheelRequest, WHEEL_SIZE};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use service::TriggerError;
use settings::{ActionSpec, Binding, Conflict};
use shortcut::ShortcutAction;
use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError},
    time::{Duration, Instant},
};
use theme::{
    button, caps_for, caps_for_buttons, combo, combo_toned, dot, ghost_cap, glyph_button, hsla,
    icon, icon_button, icon_tile, keycap, logo, pill, plus, rgb, segment, small, status_pill,
    switch, Cap, CapSize, CapTone, Tone,
};
use trigger_panel::{
    panel_size, PanelContext, PanelDiagnostics, PanelItem, PanelKind, PanelRequest, PanelState,
    TriggerPanel,
};
use wheel_workspace::DragWheelSession;
use wheels::{WheelAccent, WheelActionRef, Wheels};

#[derive(Clone, Copy, PartialEq, Eq)]
enum TriggerMode {
    Press,
    Hold,
    Drag,
}

impl TriggerMode {
    fn of(activation: Activation) -> Self {
        match activation {
            Activation::Hold { .. } => Self::Hold,
            Activation::Drag { .. } => Self::Drag,
            _ => Self::Press,
        }
    }
}

const TOAST_SECONDS: f32 = 4.;
const TOAST_WIDTH: f32 = 380.;
const TOAST_HEIGHT: f32 = 108.;
/// GPUI 无边框窗口会自行收窄客户区，右下留出透明余量，避免浮层被裁切。
const TOAST_MARGIN: f32 = 24.;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Actions,
    Wheels,
    Triggers,
    Presets,
    Extensions,
    History,
    Diagnostics,
    Settings,
}

fn should_refresh_workspace_for_backend_event(event: &BackendEvent, page: Page) -> bool {
    match event {
        BackendEvent::Ready
        | BackendEvent::RecordingProgress(_)
        | BackendEvent::Recorded(_)
        | BackendEvent::Triggered { .. }
        | BackendEvent::Error(_)
        | BackendEvent::RecordingCancelled => true,
        BackendEvent::Diagnostics(_) | BackendEvent::HoldTiming { .. } => page == Page::Diagnostics,
        // 这些事件由各浮窗直接处理，不改变主窗口的可见内容。
        BackendEvent::PointerPressed { .. }
        | BackendEvent::EscapePressed
        | BackendEvent::PanelKey(_)
        | BackendEvent::Drag { .. } => false,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HistoryFilter {
    All,
    Real,
    Panel,
    Preview,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HitSource {
    Input,
    Panel,
    Wheel,
    Preview,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PanelBehavior {
    Toggle,
    Reposition,
}

impl PanelBehavior {
    fn for_activation(activation: Activation) -> Self {
        match activation {
            Activation::Hold {
                button: Button::Mouse(MouseButton::Right),
                ..
            } => Self::Reposition,
            _ => Self::Toggle,
        }
    }
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
    source: HitSource,
}

struct HoldTrace {
    id: u64,
    elapsed_ms: u64,
    threshold_ms: u32,
    matched_at: Instant,
    queue_ms: u64,
}

#[derive(Default)]
struct PanelShowStats {
    requested: u64,
    submitted: u64,
    skipped: u64,
    failed: u64,
    last: &'static str,
}

struct PanelLatency {
    threshold_ms: u32,
    held_ms: u64,
    queue_ms: u64,
    display_ms: u64,
    total_ms: u64,
}

fn elapsed_ms(since: Instant) -> u64 {
    since.elapsed().as_millis().min(u64::MAX as u128) as u64
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

/// 已启用 RaiShot 时按本机现状重新定位：原路径失效、或有正在运行 / 更新的兼容安装时
/// 更新设置。只读清单与文件大小，耗时为毫秒级；未找到时保留原设置。
fn relocate_raishot(plugins: &mut Plugins) -> Option<String> {
    if !plugins.raishot_enabled() {
        return None;
    }
    let configured =
        (!plugins.raishot_path().is_empty()).then(|| PathBuf::from(plugins.raishot_path()));
    let found = plugin_install::locate(&settings::path(), configured.as_deref())?;
    if configured.as_deref() == Some(found.cli.as_path()) {
        return None;
    }
    let mut candidate = plugins.clone();
    candidate
        .configure_raishot(found.cli.to_string_lossy().into_owned(), true)
        .ok()?;
    candidate.save(&settings::path()).ok()?;
    *plugins = candidate;
    Some(if configured.is_some() {
        format!("已改用本机的 RaiShot {}。", found.version)
    } else {
        format!("已接入本机的 RaiShot {}，无需下载。", found.version)
    })
}

fn heading(title: &'static str, subtitle: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .min_w_0()
        .child(
            div()
                .text_size(px(20.))
                .line_height(px(28.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(
            div()
                .text_size(px(12.5))
                .text_color(rgb(theme::TEXT_DIM))
                .child(subtitle.into()),
        )
}

fn caption(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(11.5))
        .text_color(rgb(theme::TEXT_FAINT))
        .child(text.into())
}

/// 简洁的小节标题。
fn section_title(title: &'static str) -> Div {
    div().flex().items_center().gap(px(8.)).child(
        div()
            .text_size(px(12.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(theme::TEXT_SOFT))
            .child(title),
    )
}

fn card() -> Div {
    div()
        .rounded(px(8.))
        .bg(rgb(theme::ROW))
        .border_1()
        .border_color(rgb(theme::BORDER))
}

fn stat(
    label: &'static str,
    value: impl Into<SharedString>,
    note: Option<(String, u32)>,
    glyph: Icon,
    tint: u32,
) -> Div {
    card()
        .flex_1()
        .min_w_0()
        .p(px(16.))
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(icon_tile(glyph, tint, 28.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(12.))
                        .text_color(rgb(theme::TEXT_DIM))
                        .child(label),
                ),
        )
        .child(
            div()
                .text_size(px(24.))
                .font_weight(FontWeight::SEMIBOLD)
                .font_family(theme::MONO_FONT)
                .child(value.into()),
        )
        .when_some(note, |el, (note, color)| {
            el.child(div().text_size(px(11.)).text_color(rgb(color)).child(note))
        })
}

/// 列表页眉下的轻量统计。
fn chip(glyph: Icon, tint: u32, value: impl Into<SharedString>, label: &'static str) -> Div {
    div()
        .flex_none()
        .h(px(26.))
        .pr(px(12.))
        .flex()
        .items_center()
        .gap(px(8.))
        .child(
            div()
                .size(px(20.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .justify_center()
                .child(icon(glyph, 12., tint)),
        )
        .child(
            div()
                .font_family(theme::MONO_FONT)
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(value.into()),
        )
        .child(
            div()
                .text_size(px(11.5))
                .text_color(rgb(theme::TEXT_DIM))
                .child(label),
        )
}

/// 动作的一句话说明；列表与触发面板卡片共用同一措辞。
fn action_detail(action: &ActionSpec) -> String {
    match action {
        ActionSpec::Notification => "仅提示".into(),
        ActionSpec::ShowPanel => "打开 / 收起触发面板".into(),
        ActionSpec::ShowWheel(reference) => format!("轨道轮盘 · {}", reference.wheel_id),
        ActionSpec::SendShortcut(shortcut) => format!("发送快捷键 · {}", shortcut.display()),
        ActionSpec::Window(action) => format!("窗口 · {}", action.display()),
        ActionSpec::AdjustSystem(action) => action.display(),
        ActionSpec::Local(action) => action.summary(),
        ActionSpec::Invoke(call) => format!("动作包 · {} / {}", call.package_id, call.action_id),
        ActionSpec::Browser(action) => format!("浏览器 · {}", action.label()),
        ActionSpec::External(action) => format!("外部动作 · 依赖 {}", action.provider),
        ActionSpec::Command(command) => {
            let program = Path::new(&command.program)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(&command.program);
            format!("运行程序 · {program}")
        }
        ActionSpec::Plugin(reference) if reference.plugin_id == "raishot" => {
            "RaiShot · 插件动作".into()
        }
        ActionSpec::Plugin(reference) => format!(
            "{} / {} · 插件动作",
            reference.plugin_id, reference.action_id
        ),
    }
}

/// 动作类型的图标与色调。
fn action_look(action: &ActionSpec) -> (Icon, u32) {
    match action {
        ActionSpec::Notification => (Icon::Bell, theme::VIOLET),
        ActionSpec::ShowPanel => (Icon::Panel, theme::ACCENT),
        ActionSpec::ShowWheel(_) => (Icon::Pulse, theme::VIOLET),
        ActionSpec::SendShortcut(_) => (Icon::Keyboard, theme::ACCENT_2),
        ActionSpec::Window(_) => (Icon::Panel, theme::ACCENT_2),
        ActionSpec::AdjustSystem(_) => (Icon::Pulse, theme::ACCENT_2),
        ActionSpec::Local(_) => (Icon::Layers, theme::ACCENT_2),
        ActionSpec::Invoke(_) => (Icon::Terminal, theme::ACCENT_2),
        ActionSpec::Browser(_) => (Icon::Browser, theme::ACCENT_2),
        ActionSpec::External(_) => (Icon::Terminal, theme::RECORD),
        ActionSpec::Command(_) => (Icon::Terminal, theme::RECORD),
        ActionSpec::Plugin(_) => (Icon::Capture, theme::PINK),
    }
}

fn activation_label(spec: &TriggerSpec) -> String {
    match spec.activation {
        Activation::Press(_) => "按下即触发".into(),
        Activation::Hold { duration_ms, .. } => format!("长按 {duration_ms}ms 触发"),
        Activation::Drag {
            min_distance_px,
            max_start_ms,
            ..
        } => format!("快速拖拽 ≥{min_distance_px}px · <{max_start_ms}ms"),
        Activation::Wheel(_) => "滚轮一格即触发".into(),
    }
}

struct StartupOptions {
    ui_smoke: bool,
    browser_connector_settings: bool,
    appearance_settings: bool,
    panel: bool,
    wheel: Option<String>,
    wheel_target: Option<omnitrigger_windows::ShortcutTarget>,
    wheel_editor: bool,
}

struct Workspace {
    backend: Option<Backend>,
    events: Option<EventBridge<BackendEvent>>,
    ui_wake: UiWake,
    ready: bool,
    bindings_need_sync: bool,
    paused: bool,
    recording: bool,
    recording_held: Vec<Button>,
    pending: Option<TriggerSpec>,
    edited_spec: Option<TriggerSpec>,
    preset_edit_id: Option<u64>,
    conflict: Option<Conflict>,
    selected: Option<u64>,
    confirm_delete: Option<u64>,
    page: Page,
    theme_mode: appearance::Mode,
    theme_error: Option<String>,
    history_filter: HistoryFilter,
    smooth_scroll: smooth_scroll::ScrollRegions,
    message: Entity<MessageInput>,
    trigger_mode: TriggerMode,
    condition_editor: trigger_conditions::ConditionEditor,
    recording_conditions: Vec<omnitrigger_core::TriggerCondition>,
    adjustment_amount: Entity<MessageInput>,
    hold_duration: Entity<MessageInput>,
    drag_distance: Entity<MessageInput>,
    drag_window: Entity<MessageInput>,
    draft_action: ActionSpec,
    recording_for_panel: bool,
    recording_preserve_action: bool,
    recording_edit_id: Option<u64>,
    program: Entity<MessageInput>,
    arguments: Entity<MessageInput>,
    working_directory: Entity<MessageInput>,
    shortcut_keys: Entity<MessageInput>,
    action_search: Entity<MessageInput>,
    package_input: Entity<MessageInput>,
    package_picker_for_draft: bool,
    action_service: omnitrigger_actions::ActionService,
    plugins: Plugins,
    plugin_settings_error: Option<String>,
    browser_connector_ui: browser_connector_ui::BrowserConnectorUi,
    browser_executor: browser_actions::BrowserExecutor,
    browser_editor: browser_actions_ui::BrowserEditor,
    wheels: Wheels,
    wheels_error: Option<String>,
    wheel_selected: Option<String>,
    wheel_name: Entity<MessageInput>,
    wheel_slot_label: Entity<MessageInput>,
    wheel_slot_shortcut: Entity<MessageInput>,
    wheel_slot_selected: Option<usize>,
    wheel_source_binding: Option<u64>,
    wheel_window: Option<WindowHandle<RadialWheel>>,
    wheel_registered: Option<isize>,
    wheel_generation: u64,
    wheel_active: Option<String>,
    wheel_active_definition: Option<wheels::WheelDefinition>,
    wheel_target: Option<omnitrigger_windows::ShortcutTarget>,
    wheel_runtime_ids: HashMap<(String, usize), u64>,
    wheel_requests: Receiver<WheelRequest>,
    wheel_sender: SyncSender<WheelRequest>,
    open_wheel_on_ready: Option<String>,
    wheel_launch_target: Option<omnitrigger_windows::ShortcutTarget>,
    drag_wheel_session: Option<DragWheelSession>,
    install_events: Option<Receiver<plugin_install::InstallEvent>>,
    install_status: Option<String>,
    executor: CommandExecutor,
    shortcut_executor: CommandExecutor,
    system_executor: system_executor::SystemAdjustmentExecutor,
    system_hud: system_hud::SystemHud,
    brightness_hud_prepared: bool,
    local_executor: local_executor::LocalExecutor,
    local_invocations: HashMap<u64, u64>,
    local_editor: local_editor::LocalEditor,
    wheel_local_editor: local_editor::LocalEditor,
    action_status: HashMap<u64, String>,
    action_log: VecDeque<String>,
    triggers: service::TriggerService,
    error: Option<String>,
    action_error: Option<(u64, String)>,
    status: String,
    hits: VecDeque<Hit>,
    trigger_count: u64,
    toast: Option<WindowHandle<Toast>>,
    toast_time: Option<Instant>,
    panel: Option<WindowHandle<TriggerPanel>>,
    panel_prepared: bool,
    /// 已向输入采集登记的面板窗口；面板收起后撤销。
    panel_registered: Option<isize>,
    panel_show_generation: u64,
    panel_show_stats: PanelShowStats,
    panel_diagnostics: PanelDiagnostics,
    last_hold_timing: Option<HoldTrace>,
    last_panel_latency: Option<PanelLatency>,
    max_panel_total_ms: u64,
    max_hold_lateness_ms: u64,
    panel_requests: Receiver<PanelRequest>,
    panel_sender: SyncSender<PanelRequest>,
    main_window: AnyWindowHandle,
    open_panel_on_ready: bool,
    panel_click_count: u64,
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
    fn watch_ui(wake: UiWake, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| loop {
            wake.wait().await;
            if this.update(cx, |this, cx| this.poll(cx)).is_err() {
                break;
            }
        })
        .detach();
    }

    fn new(
        cx: &mut Context<Self>,
        startup: StartupOptions,
        main_window: AnyWindowHandle,
        title_bar: Result<isize, String>,
    ) -> Self {
        let StartupOptions {
            ui_smoke,
            browser_connector_settings,
            appearance_settings,
            panel: open_panel,
            wheel: initial_wheel,
            wheel_target: wheel_launch_target,
            wheel_editor,
        } = startup;
        let (theme_mode, theme_error) = match appearance::load(&settings::path()) {
            Ok(mode) => (mode, None),
            Err(error) => (appearance::Mode::default(), Some(error)),
        };
        theme::apply(theme_mode, cx.window_appearance());
        let (bindings, load_error) = match settings::load() {
            Ok(bindings) => (bindings, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        let (mut backend, events, mut startup_error) = match Backend::start() {
            Ok((backend, events)) => (Some(backend), events, None),
            Err(error) => (None, std::sync::mpsc::channel().1, Some(error)),
        };
        let events = match EventBridge::new(events) {
            Ok(events) => Some(events),
            Err(error) => {
                backend.take();
                startup_error = Some(error);
                None
            }
        };
        let ui_wake = events.as_ref().map(EventBridge::wake).unwrap_or_default();
        Self::watch_ui(ui_wake.clone(), cx);
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
        let draft_action = bindings
            .first()
            .map(|b| b.action.clone())
            .unwrap_or(ActionSpec::Notification);
        let (wheels, wheels_error) = match Wheels::load(&settings::path()) {
            Ok(wheels) => (wheels, None),
            Err(error) => (Wheels { wheels: Vec::new() }, Some(error)),
        };
        let initial_definition = bindings
            .iter()
            .find_map(|binding| {
                if binding.enabled && matches!(binding.spec.activation, Activation::Drag { .. }) {
                    if let ActionSpec::ShowWheel(reference) = &binding.action {
                        return wheels
                            .wheels
                            .iter()
                            .find(|wheel| wheel.id == reference.wheel_id);
                    }
                }
                None
            })
            .or_else(|| wheels.wheels.first());
        let wheel_selected = initial_definition.map(|wheel| wheel.id.clone());
        let wheel_name = cx.new(|cx| {
            let mut input = MessageInput::new(cx).with_character_limit(30);
            input.set_text(
                initial_definition
                    .map(|wheel| wheel.name.clone())
                    .unwrap_or_default(),
                cx,
            );
            input
        });
        let wheel_slot_label = cx.new(|cx| {
            let mut input = MessageInput::new(cx).with_character_limit(30);
            input.set_placeholder("动作名称，例如：复制");
            input
        });
        let wheel_slot_shortcut = cx.new(|cx| {
            let mut input = MessageInput::new(cx).with_character_limit(128);
            input.set_placeholder("例如 Ctrl+C、Backspace");
            input.set_text("Ctrl+C", cx);
            input
        });
        for input in [&wheel_name, &wheel_slot_label, &wheel_slot_shortcut] {
            cx.observe(input, |_, _, cx| cx.notify()).detach();
        }
        let command = match &draft_action {
            ActionSpec::Command(command) => Some(command),
            _ => None,
        };
        let shortcut_keys = cx.new(|cx| {
            let mut input = MessageInput::new(cx).with_character_limit(128);
            input.set_placeholder("例如：Backspace、Left、Ctrl+C");
            input.set_text(
                match &draft_action {
                    ActionSpec::SendShortcut(shortcut) => shortcut.display(),
                    _ => "Backspace".into(),
                },
                cx,
            );
            input
        });
        cx.observe(&shortcut_keys, |_, _, cx| cx.notify()).detach();
        let (mut plugins, plugin_settings_error) = match Plugins::load(&settings::path()) {
            Ok(plugins) => (plugins, None),
            Err(error) => (Plugins::default(), Some(error.to_string())),
        };
        let relocated = plugin_settings_error
            .is_none()
            .then(|| relocate_raishot(&mut plugins))
            .flatten();
        let action_search = cx.new(|cx| {
            let mut input = MessageInput::new(cx).embedded();
            input.set_placeholder("搜索动作，例如：截图");
            input
        });
        let browser_connector_ui = browser_connector_ui::BrowserConnectorUi::new(cx);
        let mut browser_editor = browser_actions_ui::BrowserEditor::new(cx);
        if let ActionSpec::Browser(action) = &draft_action {
            browser_editor.load(action, cx);
        }
        cx.observe(&action_search, |_, _, cx| cx.notify()).detach();
        let package_input = cx.new(|cx| {
            let mut input = MessageInput::new(cx)
                .with_json_text()
                .with_character_limit(128 * 1024);
            input.set_placeholder(r#"例如：{"name":"示例"}"#);
            input.set_text(
                match &draft_action {
                    ActionSpec::Invoke(call) => call.input.to_string(),
                    _ => "{}".into(),
                },
                cx,
            );
            input
        });
        cx.observe(&package_input, |_, _, cx| cx.notify()).detach();
        let initial_activation = bindings.first().map(|binding| binding.spec.activation);
        let trigger_mode = initial_activation.map_or(TriggerMode::Press, TriggerMode::of);
        let mut local_editor = local_editor::LocalEditor::new(cx);
        if let ActionSpec::Local(action) = &draft_action {
            local_editor.load(action, cx);
        }
        let wheel_local_editor = local_editor::LocalEditor::new(cx);
        let condition_editor = trigger_conditions::ConditionEditor::new(
            cx,
            bindings.first().map(|binding| &binding.spec),
        );
        let initial_adjustment_amount = if let ActionSpec::AdjustSystem(action) = draft_action {
            action.delta_percent.unsigned_abs().to_string()
        } else {
            "5".into()
        };
        let adjustment_amount = cx.new(|cx| {
            let mut input = MessageInput::new(cx).with_character_limit(3);
            input.set_text(initial_adjustment_amount, cx);
            input
        });
        cx.observe(&adjustment_amount, |_, _, cx| cx.notify())
            .detach();
        let hold_duration = cx.new(|cx| {
            let mut input = MessageInput::new(cx).with_character_limit(10);
            input.set_placeholder("50–10000 毫秒");
            input.set_text(
                match initial_activation {
                    Some(Activation::Hold { duration_ms, .. }) => duration_ms.to_string(),
                    _ => "300".into(),
                },
                cx,
            );
            input
        });
        let (distance, window_ms) = match initial_activation {
            Some(Activation::Drag {
                min_distance_px,
                max_start_ms,
                ..
            }) => (min_distance_px, max_start_ms),
            _ => (DEFAULT_DRAG_DISTANCE_PX, DEFAULT_DRAG_START_MS),
        };
        let drag_distance = cx.new(|cx| {
            let mut input = MessageInput::new(cx).with_character_limit(10);
            input.set_text(distance.to_string(), cx);
            input
        });
        let drag_window = cx.new(|cx| {
            let mut input = MessageInput::new(cx).with_character_limit(10);
            input.set_text(window_ms.to_string(), cx);
            input
        });
        for input in [&hold_duration, &drag_distance, &drag_window] {
            cx.observe(input, |_, _, cx| cx.notify()).detach();
        }
        let program = cx.new(|cx| {
            let mut input = MessageInput::new(cx).with_character_limit(512 * 1024);
            input.set_placeholder("可执行文件的完整路径");
            input.set_text(command.map(|c| c.program.clone()).unwrap_or_default(), cx);
            input
        });
        let arguments = cx.new(|cx| {
            let mut input = MessageInput::new(cx).with_character_limit(512 * 1024);
            input.set_placeholder(r#"例如：["--capture"]"#);
            input.set_text(
                serde_json::to_string(&command.map(|c| c.args.clone()).unwrap_or_default())
                    .unwrap_or_else(|_| "[]".into()),
                cx,
            );
            input
        });
        let working_directory = cx.new(|cx| {
            let mut input = MessageInput::new(cx).with_character_limit(512 * 1024);
            input.set_placeholder("留空使用 OmniTrigger 的工作目录");
            input.set_text(
                command
                    .and_then(|c| c.working_directory.clone())
                    .unwrap_or_default(),
                cx,
            );
            input
        });
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(50))
                .await;
            if this.update(cx, |this, cx| this.poll(cx)).is_err() {
                break;
            }
        })
        .detach();
        let (panel_sender, panel_requests) = mpsc::sync_channel(8);
        let (wheel_sender, wheel_requests) = mpsc::sync_channel(8);
        let system_executor = system_executor::SystemAdjustmentExecutor::with_wake(ui_wake.clone());
        Self {
            backend,
            events,
            ui_wake,
            ready: false,
            bindings_need_sync: false,
            paused: false,
            recording: false,
            recording_held: Vec::new(),
            pending: None,
            edited_spec: None,
            preset_edit_id: None,
            conflict: None,
            selected,
            confirm_delete: None,
            theme_mode,
            theme_error,
            page: if appearance_settings {
                Page::Settings
            } else if browser_connector_settings {
                Page::Extensions
            } else if wheel_editor {
                Page::Wheels
            } else {
                Page::Triggers
            },
            history_filter: HistoryFilter::All,
            smooth_scroll: smooth_scroll::ScrollRegions::default(),
            message,
            trigger_mode,
            condition_editor,
            recording_conditions: Vec::new(),
            adjustment_amount,
            hold_duration,
            drag_distance,
            drag_window,
            draft_action,
            recording_for_panel: false,
            recording_preserve_action: false,
            recording_edit_id: None,
            program,
            arguments,
            working_directory,
            shortcut_keys,
            action_search,
            plugins,
            plugin_settings_error,
            browser_connector_ui,
            browser_executor: browser_actions::BrowserExecutor::new(settings::path()),
            browser_editor,
            wheels,
            wheels_error,
            wheel_selected,
            wheel_name,
            wheel_slot_label,
            wheel_slot_shortcut,
            wheel_slot_selected: None,
            wheel_source_binding: None,
            wheel_window: None,
            wheel_registered: None,
            wheel_generation: 0,
            wheel_active: None,
            wheel_active_definition: None,
            wheel_target: None,
            wheel_runtime_ids: HashMap::new(),
            wheel_requests,
            wheel_sender,
            open_wheel_on_ready: initial_wheel,
            wheel_launch_target,
            drag_wheel_session: None,
            install_events: None,
            install_status: relocated,
            executor: CommandExecutor::new(),
            package_input,
            package_picker_for_draft: false,
            action_service: action_packages::service(&settings::path()),
            shortcut_executor: CommandExecutor::shortcuts(),
            system_executor,
            system_hud: system_hud::SystemHud::default(),
            brightness_hud_prepared: false,
            local_executor: local_executor::LocalExecutor::new(),
            local_invocations: HashMap::new(),
            local_editor,
            wheel_local_editor,
            action_status: HashMap::new(),
            action_log: VecDeque::new(),
            triggers: service::TriggerService::new(bindings, service::SettingsFile),
            error: startup_error.or(load_error).or(title_bar.clone().err()),
            action_error: None,
            status: "正在连接键盘和鼠标…".into(),
            hits: VecDeque::new(),
            trigger_count: 0,
            toast: None,
            toast_time: None,
            panel: None,
            panel_prepared: false,
            panel_registered: None,
            panel_show_generation: 0,
            panel_show_stats: PanelShowStats::default(),
            panel_diagnostics: PanelDiagnostics::default(),
            last_hold_timing: None,
            last_panel_latency: None,
            max_panel_total_ms: 0,
            max_hold_lateness_ms: 0,
            panel_requests,
            panel_sender,
            main_window,
            open_panel_on_ready: open_panel,
            panel_click_count: 0,
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

    /// 切换主页面；录制期间保持当前页，避免打断输入拦截。
    fn open_page(&mut self, page: Page, cx: &mut Context<Self>) {
        if self.recording {
            return;
        }
        self.smooth_scroll.cancel_all();
        self.page = page;
        self.package_picker_for_draft = false;
        self.preset_edit_id = None;
        cx.notify();
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
        self.hide_wheel(cx);
        self.ready = false;
        self.error = None;
        self.status = "正在重建监听，请松开其他按键…".into();
        let previous = self.backend.take();
        // 保留界面、未保存的录制及用户配置；旧按压先按既有退出协议收尾。
        // 不在 UI 线程等待两秒 drain，也不定时重挂与其他软件争抢链首。
        self.events = None;
        self.listener_generation += 1;
        let ui_wake = self.ui_wake.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    drop(previous);
                    let (backend, events) = Backend::start()?;
                    let events = EventBridge::with_wake(events, ui_wake)?;
                    Ok::<_, String>((backend, events))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.restarting = false;
                this.diagnostics = ListenerDiagnostics::default();
                match result {
                    Ok((backend, events)) => {
                        this.backend = Some(backend);
                        this.events = Some(events);
                        // 桥可能在安装前已发出 Ready 唤醒；安装后重新通知，避免等维护 tick。
                        this.ui_wake.notify();
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
            if paused {
                self.hide_wheel(cx);
            }
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
        if matches!(
            command,
            BackendCommand::SetEnabled(false)
                | BackendCommand::StartRecording
                | BackendCommand::Shutdown
        ) {
            self.local_executor.cancel_all();
            self.action_service.cancel_all();
        }
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
        if self.recording {
            self.bindings_need_sync = true;
            return;
        }
        self.bindings_need_sync = false;
        let specs = self
            .triggers
            .active_specs(|action| self.action_available(action));
        self.send(BackendCommand::SetBindings(specs));
    }

    fn poll(&mut self, cx: &mut Context<Self>) {
        if !self.panel_prepared {
            self.panel_prepared = true;
            if let Err(error) = self.prepare_panel(cx) {
                self.error = Some(error);
            }
        }
        let mut changed = self.poll_wheel(cx);
        if !self.brightness_hud_prepared
            && self.ready
            && self.triggers.bindings().iter().any(|binding| {
                binding.enabled
                    && matches!(&binding.action, ActionSpec::AdjustSystem(action)
                        if action.target == system_action::AdjustmentTarget::Brightness)
            })
        {
            self.brightness_hud_prepared = true;
            if let Err(error) = self.system_hud.prewarm() {
                self.error = Some(error);
                changed = true;
            }
        }
        if let Some(error) = self.system_hud.take_error() {
            self.error = Some(error);
            changed = true;
        }
        if let Some(events) = self.install_events.take() {
            let mut finished = false;
            loop {
                match events.try_recv() {
                    Ok(plugin_install::InstallEvent::Progress(message)) => {
                        self.install_status = Some(message)
                    }
                    Ok(plugin_install::InstallEvent::Finished {
                        cli_path,
                        version,
                        downloaded,
                    }) => {
                        let saved = self.save_plugin_configuration(
                            cli_path.to_string_lossy().into_owned(),
                            true,
                            cx,
                        );
                        self.install_status = Some(if !saved {
                            "运行文件已就绪，插件设置保存失败；处理错误后重试启用。".into()
                        } else if downloaded {
                            format!(
                                "RaiShot {version} 已安装。首次截图时 Windows 会检查新程序，可能需要约 1 分钟，之后会快很多。"
                            )
                        } else {
                            format!("已接入本机的 RaiShot {version}，无需下载。")
                        });
                        finished = true;
                    }
                    Ok(plugin_install::InstallEvent::Failed(error)) => {
                        self.install_status = Some(format!("安装失败：{error}"));
                        finished = true;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        if !finished {
                            self.install_status = Some("安装线程已结束，未确认安装完成。".into());
                        }
                        finished = true;
                        break;
                    }
                }
                changed = true;
            }
            if !finished {
                self.install_events = Some(events);
            }
        }
        changed |= self.poll_browser_connector(cx);
        changed |= self.poll_packages();
        changed |= self.poll_browser_actions();
        for _ in 0..64 {
            let Ok(event) = self.local_executor.try_recv() else {
                break;
            };
            if event.outcome.is_terminal()
                && self.local_invocations.get(&event.id) == Some(&event.invocation_id)
            {
                self.local_invocations.remove(&event.id);
            }
            changed = true;
            use local_executor::LocalExecutionOutcome;
            let (message, failed) = match event.outcome {
                LocalExecutionOutcome::StepProgress {
                    step_index,
                    description,
                } => (
                    format!(
                        "运行 {} · 第 {} 步：{}",
                        event.invocation_id,
                        step_index + 1,
                        description
                    ),
                    false,
                ),
                LocalExecutionOutcome::Finished {
                    completed_steps,
                    summary,
                } => (
                    format!(
                        "运行 {} 已完成 {completed_steps} 步 · {summary}",
                        event.invocation_id
                    ),
                    false,
                ),
                LocalExecutionOutcome::Failed {
                    completed_steps,
                    failed_step,
                    message,
                } => (
                    format!(
                        "运行 {} 在{}失败，已完成 {completed_steps} 步：{message}",
                        event.invocation_id,
                        failed_step.map_or_else(
                            || "准备阶段".into(),
                            |index| format!("第 {} 步", index + 1)
                        )
                    ),
                    true,
                ),
                LocalExecutionOutcome::Cancelled {
                    completed_steps, ..
                } => (
                    format!(
                        "运行 {} 已取消，已完成 {completed_steps} 步",
                        event.invocation_id
                    ),
                    false,
                ),
            };
            if failed {
                self.action_error = Some((event.id, message.clone()));
            } else if self
                .action_error
                .as_ref()
                .is_some_and(|(id, _)| *id == event.id)
            {
                self.action_error = None;
            }
            self.record_action_status(event.id, message);
        }
        for _ in 0..8 {
            match self.panel_requests.try_recv() {
                Ok(PanelRequest::Run(id)) => {
                    // 请求提交后若面板已被再次呼出，执行截图前也先将其收起。
                    if let Some(panel) = self.panel {
                        let _ = panel.update(cx, |view, window, _| view.hide(window));
                    }
                    self.fire(id, HitSource::Panel, cx);
                    changed = true;
                }
                Ok(PanelRequest::Configure) => {
                    self.page = Page::Triggers;
                    let _ = self
                        .main_window
                        .update(cx, |_, window, _| window.activate_window());
                    changed = true;
                }
                Ok(PanelRequest::Close) => changed = true,
                Err(_) => break,
            }
        }
        let mut panel_visible = false;
        if let Some(panel) = self.panel {
            if let Ok((snapshot, visible)) =
                panel.update(cx, |view, _, _| (view.diagnostics(), view.is_visible()))
            {
                panel_visible = visible;
                if snapshot != self.panel_diagnostics {
                    self.panel_diagnostics = snapshot;
                    changed = true;
                }
            }
        }
        // 面板收起后撤销导航登记；输入采集另外核验窗口可见性，登记滞后也不会吞键。
        if !panel_visible && self.panel_registered.take().is_some() {
            self.send(BackendCommand::SetPanelWindow(None));
        }
        for _ in 0..8 {
            let Ok(event) = self.system_executor.try_recv() else {
                break;
            };
            changed = true;
            match &event.result {
                Ok(result) => {
                    if let Err(error) = self.system_hud.complete(event.token, result, event.point) {
                        self.error = Some(error);
                    }
                }
                Err(_) => self.system_hud.fail(event.token),
            }
            let (text, failed) = match event.result {
                Ok(result) => (
                    format!(
                        "{} 当前 {}% · {}",
                        match result.kind {
                            omnitrigger_windows::AdjustmentKind::Volume => "音量",
                            omnitrigger_windows::AdjustmentKind::Brightness => "亮度",
                        },
                        result.current_percent,
                        result.target_label
                    ),
                    false,
                ),
                Err(error) => (error, true),
            };
            if failed {
                self.action_error = Some((event.id, text.clone()));
            } else if self
                .action_error
                .as_ref()
                .is_some_and(|(id, _)| *id == event.id)
            {
                self.action_error = None;
            }
            self.record_action_status(event.id, text);
        }
        while let Ok(event) = self
            .executor
            .try_recv()
            .or_else(|_| self.shortcut_executor.try_recv())
        {
            changed = true;
            let (text, failed) = match event.outcome {
                ExecutionOutcome::ShortcutSent => ("已发送快捷键".into(), false),
                ExecutionOutcome::WindowApplied => ("已提交窗口操作".into(), false),
                ExecutionOutcome::Started { pid } => (format!("已启动进程（PID {pid}）"), false),
                ExecutionOutcome::Finished { code, success } => (
                    format!(
                        "进程{}退出（{}）",
                        if success { "正常" } else { "异常" },
                        code.map(|c| c.to_string())
                            .unwrap_or_else(|| "无退出码".into())
                    ),
                    !success,
                ),
                ExecutionOutcome::Failed { message } => (message, true),
                ExecutionOutcome::Unknown { message } => (format!("进程状态未知：{message}"), true),
            };
            if failed {
                self.action_error = Some((event.id, format!("动作 #{}：{text}", event.id)));
            } else if self
                .action_error
                .as_ref()
                .is_some_and(|(id, _)| *id == event.id)
            {
                self.action_error = None;
            }
            self.record_action_status(event.id, text);
        }
        let mut processed_backend = 0;
        for _ in 0..64 {
            let event = match self
                .events
                .as_ref()
                .map_or(Err(TryRecvError::Empty), EventBridge::try_recv)
            {
                Ok(event) => event,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if self.backend.is_some() && !self.restarting {
                        self.hide_wheel(cx);
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
            processed_backend += 1;
            changed |= should_refresh_workspace_for_backend_event(&event, self.page);
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
                BackendEvent::Recorded(mut spec) => {
                    spec.conditions = self.recording_conditions.clone();
                    self.recording = false;
                    self.recording_held.clear();
                    let editing = self.recording_edit_id.take();
                    let reusable = editing.or_else(|| {
                        if self.recording_preserve_action {
                            self.pending.as_ref().map(|s| s.id)
                        } else {
                            None
                        }
                    });
                    let Some(id) = self.draft_id(reusable) else {
                        continue;
                    };
                    spec.id = id;
                    self.conflict = self.check_draft_trigger(&spec, editing).err();
                    self.load_trigger_fields(&spec, cx);
                    if editing.is_some() {
                        self.edited_spec = Some(spec);
                        self.pending = None;
                    } else {
                        self.clear_pending();
                        self.pending = Some(spec);
                        self.edited_spec = None;
                    }
                    self.status = if self.conflict.is_some() {
                        "录制的组合无法添加，请重新录制。"
                    } else {
                        "已录制，填写提示消息后点击“添加触发器”。"
                    }
                    .into();
                    if !self.recording_preserve_action {
                        self.message.update(cx, |input, cx| input.set_text("", cx));
                        self.load_action(ActionSpec::Notification, cx);
                    }
                    let for_panel = self.recording_for_panel;
                    self.recording_for_panel = false;
                    if for_panel {
                        self.draft_action = ActionSpec::ShowPanel;
                        self.message
                            .update(cx, |input, cx| input.set_text("打开触发面板", cx));
                    }
                }
                BackendEvent::HoldTiming {
                    id,
                    elapsed_ms: held_ms,
                    threshold_ms,
                    matched_at,
                } => {
                    self.max_hold_lateness_ms = self
                        .max_hold_lateness_ms
                        .max(held_ms.saturating_sub(u64::from(threshold_ms)));
                    self.last_hold_timing = Some(HoldTrace {
                        id,
                        elapsed_ms: held_ms,
                        threshold_ms,
                        matched_at,
                        queue_ms: elapsed_ms(matched_at),
                    });
                }
                BackendEvent::Triggered { id, pointer } => {
                    self.fire_at(id, HitSource::Input, pointer, cx)
                }
                BackendEvent::Drag { event, foreground } => {
                    self.handle_drag_event(event, foreground, cx)
                }
                BackendEvent::PointerPressed { x, y } => {
                    if let Some(wheel) = self.wheel_window {
                        let _ = wheel.update(cx, |view, window, _| {
                            if view.is_visible() {
                                if let Ok(RawWindowHandle::Win32(handle)) =
                                    HasWindowHandle::window_handle(&*window)
                                        .map(|handle| handle.as_raw())
                                {
                                    if !omnitrigger_windows::panel_contains_point(
                                        handle.hwnd.get(),
                                        x,
                                        y,
                                    ) {
                                        view.dismiss_outside(window);
                                    }
                                }
                            }
                        });
                    }
                    if let Some(panel) = self.panel {
                        let _ = panel.update(cx, |view, window, _| {
                            if !view.is_visible() {
                                return;
                            }
                            if let Ok(RawWindowHandle::Win32(handle)) =
                                HasWindowHandle::window_handle(&*window).map(|h| h.as_raw())
                            {
                                // 前台面板靠失焦收起；这里只补足未获前台的面板。
                                // 否则右键按下立即隐藏会切换前台，取消尚未到期的长按。
                                if !omnitrigger_windows::is_foreground_window(handle.hwnd.get())
                                    && !omnitrigger_windows::panel_contains_point(
                                        handle.hwnd.get(),
                                        x,
                                        y,
                                    )
                                {
                                    view.dismiss_outside(window);
                                }
                            }
                        });
                    }
                }
                BackendEvent::EscapePressed => {
                    self.local_executor.cancel_all();
                    self.action_service.cancel_all();
                    if let Some(wheel) = self.wheel_window {
                        let _ = wheel.update(cx, |view, window, _| view.dismiss_escape(window));
                    }
                    if let Some(panel) = self.panel {
                        let _ = panel.update(cx, |view, window, _| view.dismiss_escape(window));
                    }
                }
                BackendEvent::PanelKey(key) => {
                    if let Some(wheel) = self.wheel_window {
                        let _ = wheel
                            .update(cx, |view, window, cx| view.navigation_key(key, window, cx));
                    }
                    if let Some(panel) = self.panel {
                        let _ = panel
                            .update(cx, |view, window, cx| view.navigation_key(key, window, cx));
                    }
                }
                BackendEvent::Error(error) => {
                    self.hide_wheel(cx);
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
                    self.recording_edit_id = None;
                    if !self.recording_preserve_action && self.pending.is_none() {
                        self.load_selected_message(cx);
                    }
                }
                BackendEvent::Diagnostics(snapshot) => self.diagnostics = *snapshot,
            }
        }
        // 一次只处理有界批次；仍有积压时立即再调度，避免等维护轮询。
        if processed_backend == 64 {
            self.ui_wake.notify();
        }
        if self.bindings_need_sync && !self.recording {
            self.sync_bindings();
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
                HitSource::Preview,
                cx,
            );
        }
        if changed {
            self.refresh_panel(cx);
            self.refresh_wheel(cx);
            cx.notify();
        }
        if self.open_panel_on_ready && (self.ready || self.backend.is_none()) {
            self.open_panel_on_ready = false;
            self.toggle_panel(cx);
        }
        if self.open_wheel_on_ready.is_some() && (self.ready || self.backend.is_none()) {
            if let Some(id) = self.open_wheel_on_ready.take() {
                if let Some(target) = self.wheel_launch_target.take() {
                    self.open_wheel_after_launch(id, target, cx);
                } else {
                    self.open_wheel(&id, cx);
                }
            }
        }
    }

    /// 优先复用草稿或正在编辑的编号，否则分配新编号；失败时写入错误提示。
    fn draft_id(&mut self, reuse: Option<u64>) -> Option<u64> {
        if reuse.is_some() {
            return reuse;
        }
        match self.triggers.allocate_id() {
            Ok(id) => Some(id),
            Err(error) => {
                self.error = Some(error.message());
                None
            }
        }
    }

    fn binding(&self, id: u64) -> Option<&Binding> {
        self.triggers.get(id)
    }

    fn action_available(&self, action: &ActionSpec) -> bool {
        match action {
            ActionSpec::ShowWheel(_)
            | ActionSpec::External(_)
            | ActionSpec::Invoke(_)
            | ActionSpec::Browser(_) => self.action_unavailable_reason(action).is_none(),
            ActionSpec::Plugin(reference) => {
                self.plugin_settings_error.is_none() && self.plugins.availability(reference).is_ok()
            }
            _ => true,
        }
    }

    fn action_busy(&self, id: u64, action: &ActionSpec) -> bool {
        // 每次按压/拖拽都是独立调用；快捷键可在有界 FIFO 中排队，不因未读结果互斥。
        if matches!(
            action,
            ActionSpec::SendShortcut(_) | ActionSpec::Window(_) | ActionSpec::AdjustSystem(_)
        ) {
            return false;
        }
        self.executor.is_running(id)
            || self.shortcut_executor.is_running(id)
            || self.system_executor.is_running(id)
            || self.local_executor.is_running(id)
            || self.action_service.is_running(id)
            || self.browser_executor.is_running(id)
            || match action {
                ActionSpec::Plugin(reference) => self
                    .plugins
                    .concurrency_group(reference)
                    .is_some_and(|group| self.executor.is_group_running(&group)),
                _ => false,
            }
    }

    fn action_unavailable_reason(&self, action: &ActionSpec) -> Option<String> {
        match action {
            ActionSpec::Browser(action) => action.validate().err().or_else(|| {
                (!self.browser_connector_ui.installed())
                    .then(|| "请先在扩展页安装浏览器连接器。".into())
            }),
            ActionSpec::Invoke(call) => self
                .action_service
                .validate(call)
                .err()
                .map(|error| error.to_string()),
            ActionSpec::External(action) => action.unavailable_reason(),
            ActionSpec::ShowWheel(reference) => self.wheels_error.clone().or_else(|| {
                (!self
                    .wheels
                    .wheels
                    .iter()
                    .any(|wheel| wheel.id == reference.wheel_id))
                .then(|| format!("轮盘“{}”不存在，请重新选择轮盘。", reference.wheel_id))
            }),
            ActionSpec::Plugin(reference) => self.plugin_settings_error.clone().or_else(|| {
                self.plugins
                    .availability(reference)
                    .err()
                    .map(|error| error.to_string())
            }),
            _ => None,
        }
    }

    fn check_draft_trigger(
        &self,
        spec: &TriggerSpec,
        editing: Option<u64>,
    ) -> Result<(), Conflict> {
        self.triggers.check_candidate(spec, editing)
    }

    fn load_trigger_fields(&mut self, spec: &TriggerSpec, cx: &mut Context<Self>) {
        self.condition_editor.load(spec, cx);
        self.trigger_mode = TriggerMode::of(spec.activation);
        let duration = match spec.activation {
            Activation::Hold { duration_ms, .. } => duration_ms,
            _ => 300,
        };
        self.hold_duration
            .update(cx, |input, cx| input.set_text(duration.to_string(), cx));
        let (distance, window_ms) = match spec.activation {
            Activation::Drag {
                min_distance_px,
                max_start_ms,
                ..
            } => (min_distance_px, max_start_ms),
            _ => (DEFAULT_DRAG_DISTANCE_PX, DEFAULT_DRAG_START_MS),
        };
        self.drag_distance
            .update(cx, |input, cx| input.set_text(distance.to_string(), cx));
        self.drag_window
            .update(cx, |input, cx| input.set_text(window_ms.to_string(), cx));
    }

    fn draft_spec(&self, base: &TriggerSpec, cx: &App) -> Result<TriggerSpec, String> {
        let base = self
            .edited_spec
            .as_ref()
            .filter(|spec| spec.id == base.id)
            .unwrap_or(base);
        let button = match base.activation {
            Activation::Press(button)
            | Activation::Hold { button, .. }
            | Activation::Drag { button, .. } => button,
            Activation::Wheel(_) => {
                return Ok(TriggerSpec {
                    conditions: self.condition_editor.conditions(cx)?,
                    ..base.clone()
                })
            }
        };
        let activation = if self.trigger_mode == TriggerMode::Hold {
            let duration_ms: u32 = self
                .hold_duration
                .read(cx)
                .text()
                .trim()
                .parse()
                .map_err(|_| "长按时间请输入 50 至 10000 的整数毫秒。".to_string())?;
            if !(50..=10000).contains(&duration_ms) {
                return Err("长按时间应为 50 至 10000 毫秒，不会自动调整输入值。".into());
            }
            Activation::Hold {
                button,
                duration_ms,
            }
        } else if self.trigger_mode == TriggerMode::Drag {
            if !matches!(button, Button::Mouse(_)) {
                return Err("快速拖拽仅适用于鼠标按键，请选择鼠标预设或重新录制。".into());
            }
            let min_distance_px = self
                .drag_distance
                .read(cx)
                .text()
                .trim()
                .parse::<u32>()
                .map_err(|_| "拖拽距离请输入 4–500 的整数像素。".to_string())?;
            let max_start_ms = self
                .drag_window
                .read(cx)
                .text()
                .trim()
                .parse::<u32>()
                .map_err(|_| "启动时窗请输入 20–2000 的整数毫秒。".to_string())?;
            if !(4..=500).contains(&min_distance_px) || !(20..=2000).contains(&max_start_ms) {
                return Err("快速拖拽距离应为 4–500px，启动时窗应为 20–2000ms。".into());
            }
            Activation::Drag {
                button,
                min_distance_px,
                max_start_ms,
            }
        } else {
            Activation::Press(button)
        };
        Ok(TriggerSpec {
            activation,
            conditions: self.condition_editor.conditions(cx)?,
            ..base.clone()
        })
    }

    fn set_trigger_mode(&mut self, mode: TriggerMode, cx: &mut Context<Self>) {
        self.trigger_mode = mode;
        if let Some(base) = self.pending.clone() {
            match self.draft_spec(&base, cx) {
                Ok(spec) => {
                    self.conflict = self.triggers.check_candidate(&spec, None).err();
                    self.pending = Some(spec);
                }
                Err(error) => self.error = Some(error),
            }
        }
        cx.notify();
    }

    fn choose_preset(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.recording {
            return;
        }
        let Some(preset) = PRESETS.get(index) else {
            return;
        };
        let editing = self
            .preset_edit_id
            .take()
            .filter(|id| self.binding(*id).is_some());
        let Some(id) = self.draft_id(self.pending.as_ref().map(|spec| spec.id).or(editing)) else {
            cx.notify();
            return;
        };
        self.clear_pending();
        self.edited_spec = None;
        let spec = preset.spec(id);
        self.load_trigger_fields(&spec, cx);
        self.conflict = self.check_draft_trigger(&spec, editing).err();
        if editing.is_some() {
            self.edited_spec = Some(spec);
        } else {
            self.pending = Some(spec);
        }
        if self.message.read(cx).text().trim().is_empty() {
            self.message
                .update(cx, |input, cx| input.set_text("新动作", cx));
        }
        self.page = Page::Triggers;
        self.confirm_delete = None;
        self.error = None;
        self.status = format!("已选择“{}”，保留当前动作，检查参数后保存。", preset.title);
        cx.notify();
    }

    fn panel_items(&self) -> Vec<PanelItem> {
        self.triggers
            .bindings()
            .iter()
            .filter(|b| !matches!(b.action, ActionSpec::ShowPanel))
            .map(|binding| {
                let id = binding.spec.id;
                let state = if !binding.enabled {
                    PanelState::Disabled
                } else if !self.action_available(&binding.action) {
                    PanelState::Unavailable
                } else if self.action_busy(id, &binding.action) {
                    PanelState::Running
                } else {
                    PanelState::Ready
                };
                let kind = match &binding.action {
                    ActionSpec::SendShortcut(_) => PanelKind::Shortcut,
                    ActionSpec::Command(_) => PanelKind::Command,
                    ActionSpec::Plugin(_) => PanelKind::Plugin,
                    _ => PanelKind::Notice,
                };
                PanelItem {
                    id,
                    title: binding.message.clone(),
                    caps: caps_for(&binding.spec),
                    kind,
                    detail: action_detail(&binding.action),
                    state,
                    status: self
                        .action_unavailable_reason(&binding.action)
                        .or_else(|| self.action_status.get(&id).cloned()),
                }
            })
            .collect()
    }

    fn panel_context(&self) -> PanelContext {
        PanelContext {
            paused: self.paused,
            shortcuts: self
                .triggers
                .bindings()
                .iter()
                .filter(|b| matches!(b.action, ActionSpec::ShowPanel) && b.enabled)
                .map(|b| caps_for(&b.spec))
                .collect(),
        }
    }

    fn refresh_panel(&mut self, cx: &mut Context<Self>) {
        if let Some(panel) = self.panel {
            match panel.update(cx, |view, _, _| view.is_visible()) {
                Ok(true) => {}
                Ok(false) => return,
                Err(_) => {
                    self.panel = None;
                    return;
                }
            }
            // 隐藏期间不重建卡片；open_panel 在 present 前总会提供最新快照。
            let items = self.panel_items();
            let context = self.panel_context();
            let error = self
                .action_error
                .as_ref()
                .map(|(_, message)| message.clone());
            if panel
                .update(cx, |view, _, cx| {
                    view.update_items(items, context, error, cx)
                })
                .is_err()
            {
                self.panel = None;
            }
        }
    }

    fn prepare_panel(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Result<WindowHandle<TriggerPanel>, String> {
        if let Some(panel) = self.panel {
            if panel.update(cx, |_, _, _| ()).is_ok() {
                return Ok(panel);
            }
            self.panel = None;
        }
        let items = self.panel_items();
        let context = self.panel_context();
        let requests = self.panel_sender.clone();
        let ui_wake = self.ui_wake.clone();
        let (width, height) = panel_size(items.len());
        let bounds = Bounds::centered(None, size(px(width), px(height)), cx);
        let mut preparation_error = None;
        let panel = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: None,
                    kind: WindowKind::PopUp,
                    is_resizable: false,
                    is_minimizable: false,
                    is_movable: false,
                    focus: false,
                    // 避免 GPUI 缓存待激活的位置；原生窗口在同一创建回调中立即隐藏。
                    show: true,
                    app_id: Some("OmniTrigger.Panel".into()),
                    ..Default::default()
                },
                |window, cx| {
                    if let Ok(RawWindowHandle::Win32(handle)) =
                        HasWindowHandle::window_handle(&*window).map(|h| h.as_raw())
                    {
                        preparation_error =
                            omnitrigger_windows::prepare_panel_window(handle.hwnd.get()).err();
                    } else {
                        preparation_error = Some("无法取得触发面板窗口句柄。".into());
                    }
                    cx.new(|cx| TriggerPanel::new(items, context, requests, ui_wake, window, cx))
                },
            )
            .map_err(|error| format!("准备触发面板失败：{error}"))?;
        if let Some(error) = preparation_error {
            let _ = panel.update(cx, |_, window, _| window.remove_window());
            return Err(error);
        }
        self.panel = Some(panel);
        Ok(panel)
    }

    fn toggle_panel(&mut self, cx: &mut Context<Self>) {
        self.open_panel(PanelBehavior::Toggle, None, cx);
    }

    fn open_panel(
        &mut self,
        behavior: PanelBehavior,
        timing: Option<HoldTrace>,
        cx: &mut Context<Self>,
    ) {
        if self.recording {
            self.status = "请先结束录制，再打开触发面板。".into();
            cx.notify();
            return;
        }
        self.hide_wheel(cx);
        let panel = match self.prepare_panel(cx) {
            Ok(panel) => panel,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        let items = self.panel_items();
        let context = self.panel_context();
        // 显示时按动作行数调整窗口高度；筛选不改变尺寸。
        let (panel_width, panel_height) = panel_size(items.len());
        let error = self.action_error.as_ref().map(|(_, text)| text.clone());
        self.panel_show_generation = self.panel_show_generation.wrapping_add(1);
        let generation = self.panel_show_generation;
        let result = panel.update(cx, |view, window, cx| {
            if behavior == PanelBehavior::Toggle && view.is_visible() {
                view.hide(window);
                return false;
            }
            view.update_items(items, context, error, cx);
            view.present(window, cx);
            true
        });
        match result {
            Ok(false) => {}
            Ok(true) => {
                self.panel_show_stats.requested = self.panel_show_stats.requested.saturating_add(1);
                self.panel_show_stats.last = "等待显示提交";
                cx.spawn(async move |this, cx| {
                    let handle = this
                        .update(cx, |this, cx| {
                            if this.panel_show_generation != generation {
                                this.panel_show_stats.skipped =
                                    this.panel_show_stats.skipped.saturating_add(1);
                                this.panel_show_stats.last = "旧请求被新呼出替代";
                                return None;
                            }
                            if this.recording {
                                let _ = panel.update(cx, |view, window, _| view.hide(window));
                                this.panel_show_stats.skipped =
                                    this.panel_show_stats.skipped.saturating_add(1);
                                this.panel_show_stats.last = "录制中取消显示";
                                return None;
                            }
                            let handle = panel
                                .update(cx, |view, window, _| {
                                    if !view.is_visible() {
                                        return None;
                                    }
                                    HasWindowHandle::window_handle(&*window).ok().and_then(
                                        |handle| match handle.as_raw() {
                                            RawWindowHandle::Win32(handle) => {
                                                Some(handle.hwnd.get())
                                            }
                                            _ => None,
                                        },
                                    )
                                })
                                .ok()
                                .flatten();
                            if handle.is_none() {
                                this.panel_show_stats.skipped =
                                    this.panel_show_stats.skipped.saturating_add(1);
                                this.panel_show_stats.last = "显示前已收起或窗口不可用";
                            }
                            handle
                        })
                        .ok()
                        .flatten();
                    let Some(handle) = handle else {
                        return;
                    };
                    // 此处不持有 GPUI App/Window 的可变借用，WM_SHOWWINDOW 可立即绘制。
                    let result = omnitrigger_windows::show_panel_at_cursor_with_size(
                        handle,
                        panel_width,
                        panel_height,
                    );
                    // 面板参照剪贴板面板以不激活方式显示，前台窗口保持不变；
                    // 只有点击筛选框时才请求前台。
                    let delay = timing.as_ref().map(|trace| elapsed_ms(trace.matched_at));
                    let should_hide = this
                        .update(cx, |this, cx| {
                            if result.is_ok() {
                                this.panel_show_stats.submitted =
                                    this.panel_show_stats.submitted.saturating_add(1);
                                this.panel_show_stats.last = "原生显示已提交（不激活）";
                            } else {
                                this.panel_show_stats.failed =
                                    this.panel_show_stats.failed.saturating_add(1);
                                this.panel_show_stats.last = "原生显示失败";
                            }
                            if this.panel_show_generation != generation {
                                return panel
                                    .update(cx, |view, _, _| !view.is_visible())
                                    .unwrap_or(true);
                            }
                            if let Err(error) = result {
                                this.error = Some(error);
                                let _ = panel.update(cx, |view, window, _| view.hide(window));
                                cx.notify();
                                return true;
                            }
                            if let (Some(trace), Some(delay)) = (timing, delay) {
                                this.max_panel_total_ms = this
                                    .max_panel_total_ms
                                    .max(trace.elapsed_ms.saturating_add(delay));
                                this.last_panel_latency = Some(PanelLatency {
                                    threshold_ms: trace.threshold_ms,
                                    held_ms: trace.elapsed_ms,
                                    queue_ms: trace.queue_ms,
                                    display_ms: delay.saturating_sub(trace.queue_ms),
                                    total_ms: trace.elapsed_ms.saturating_add(delay),
                                });
                            }
                            let visible = panel
                                .update(cx, |view, _, _| view.is_visible())
                                .unwrap_or(false);
                            if visible {
                                // 每次显示都重新登记，输入采集据此在面板未获前台时接管导航键。
                                this.send(BackendCommand::SetPanelWindow(Some(handle)));
                                this.panel_registered = Some(handle);
                            } else {
                                this.panel_show_stats.last = "提交时面板已收起";
                            }
                            cx.notify();
                            !visible
                        })
                        .unwrap_or(true);
                    if should_hide {
                        omnitrigger_windows::hide_panel_window(handle);
                    }
                })
                .detach();
            }
            Err(error) => {
                self.panel = None;
                self.error = Some(format!("显示触发面板失败：{error}"));
            }
        }
        cx.notify();
    }

    fn record_panel_shortcut(&mut self, cx: &mut Context<Self>) {
        self.start_recording(cx);
        if self.recording {
            self.recording_for_panel = true;
            self.status = "录制用于呼出面板的键盘或鼠标组合，全部松开后保存。".into();
        }
    }

    fn fire(&mut self, id: u64, source: HitSource, cx: &mut Context<Self>) {
        self.fire_at(id, source, None, cx);
    }

    fn fire_at(
        &mut self,
        id: u64,
        source: HitSource,
        pointer: Option<(i32, i32)>,
        cx: &mut Context<Self>,
    ) {
        if let Some(binding) = self.binding(id) {
            if !binding.enabled || self.recording {
                return;
            }
            if source == HitSource::Panel
                && (matches!(binding.action, ActionSpec::ShowPanel)
                    || self.action_busy(id, &binding.action))
            {
                return;
            }
            let caps = caps_for(&binding.spec);
            let message = binding.message.clone();
            let action = binding.action.clone();
            let panel_behavior = PanelBehavior::for_activation(binding.spec.activation);
            let is_hold = matches!(binding.spec.activation, Activation::Hold { .. });
            let is_drag_wheel = source == HitSource::Input
                && matches!(binding.spec.activation, Activation::Drag { .. })
                && matches!(action, ActionSpec::ShowWheel(_));
            let timing = if source == HitSource::Input && is_hold {
                self.last_hold_timing.take().filter(|trace| trace.id == id)
            } else {
                None
            };
            if source == HitSource::Input {
                self.triggers.record_hit(id, clock());
            }
            if is_drag_wheel {
                // 同一次匹配的 Started/Ended 生命周期负责轮盘交互；这里仅记录命中。
                self.record_hit(caps, message, source);
            } else if matches!(action, ActionSpec::ShowPanel) {
                self.record_hit(caps, message, source);
                self.open_panel(panel_behavior, timing, cx);
            } else if !matches!(action, ActionSpec::Notification) {
                // 截图等动作可能立即采集屏幕，执行前不弹出自己的提示浮层。
                self.record_hit(caps, message, source);
                self.dispatch_action_context(id, action, None, pointer, cx);
            } else {
                self.show_message(caps, message, source, cx);
            }
        }
    }

    fn record_action_status(&mut self, id: u64, text: String) {
        self.status = format!("动作 #{id}：{text}");
        self.action_log
            .push_front(format!("{} · {}", clock(), self.status));
        self.action_log.truncate(30);
        // 已删除的绑定仍可能收到进程退出通知，不在详情缓存中积累。
        if self.binding(id).is_some() || self.pending.as_ref().is_some_and(|spec| spec.id == id) {
            self.action_status.insert(id, text);
        }
    }

    fn submit_action(&mut self, id: u64, command: CommandAction, cx: &mut Context<Self>) {
        let text = match self.executor.submit(id, command) {
            Ok(()) => {
                if self
                    .action_error
                    .as_ref()
                    .is_some_and(|(failed_id, _)| *failed_id == id)
                {
                    self.action_error = None;
                }
                "等待启动".into()
            }
            Err(error) => {
                self.action_error = Some((id, format!("动作 #{id}：{error}")));
                error
            }
        };
        self.record_action_status(id, text);
        cx.notify();
    }

    fn dispatch_action(&mut self, id: u64, action: ActionSpec, cx: &mut Context<Self>) {
        self.dispatch_action_to(id, action, None, cx);
    }

    fn dispatch_action_to(
        &mut self,
        id: u64,
        action: ActionSpec,
        target: Option<omnitrigger_windows::ShortcutTarget>,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_action_context(id, action, target, None, cx);
    }

    fn dispatch_action_context(
        &mut self,
        id: u64,
        action: ActionSpec,
        target: Option<omnitrigger_windows::ShortcutTarget>,
        pointer: Option<(i32, i32)>,
        cx: &mut Context<Self>,
    ) {
        match action {
            ActionSpec::Browser(action) => {
                match self.browser_executor.submit(id, action) {
                    Ok(()) => {
                        self.action_error = None;
                        self.record_action_status(id, "等待执行浏览器动作".into());
                    }
                    Err(error) => {
                        self.action_error = Some((id, error.clone()));
                        self.record_action_status(id, error);
                    }
                }
                cx.notify();
            }
            ActionSpec::Invoke(call) => {
                match self.action_service.submit(id, call) {
                    Ok(invocation) => {
                        self.action_error = None;
                        self.record_action_status(id, format!("动作包已接纳 · 运行 {invocation}"));
                    }
                    Err(error) => {
                        let message = error.to_string();
                        self.action_error = Some((id, message.clone()));
                        self.record_action_status(id, message);
                    }
                }
                cx.notify();
            }
            ActionSpec::Local(action) => {
                let target = target
                    .map(Ok)
                    .unwrap_or_else(omnitrigger_windows::capture_shortcut_target);
                let result =
                    target.and_then(|target| self.local_executor.submit(id, action, target));
                match result {
                    Ok(invocation) => {
                        self.local_invocations.insert(id, invocation);
                        self.action_error = None;
                        self.record_action_status(
                            id,
                            format!("本地动作已排队 · 运行 {invocation}"),
                        );
                    }
                    Err(error) => {
                        self.action_error = Some((id, error.clone()));
                        self.record_action_status(id, error);
                    }
                }
                cx.notify();
            }
            ActionSpec::AdjustSystem(action) => {
                let point = pointer.or_else(|| {
                    self.backend
                        .as_ref()
                        .and_then(|backend| backend.pointer_position())
                });
                let result = point
                    .ok_or_else(|| "尚未获取鼠标位置，请恢复监听后重试。".to_string())
                    .and_then(|point| {
                        self.system_executor
                            .submit(id, action, point)
                            .map(|token| (token, point))
                    });
                match result {
                    Ok((token, point)) => {
                        self.action_error = None;
                        if action.target == system_action::AdjustmentTarget::Brightness {
                            if let Err(error) =
                                self.system_hud.begin(token, point, action.delta_percent)
                            {
                                self.error = Some(error);
                            }
                        }
                        self.record_action_status(id, format!("等待{}", action.display()));
                    }
                    Err(error) => {
                        self.action_error = Some((id, error.clone()));
                        self.record_action_status(id, error);
                    }
                }
                cx.notify();
            }
            ActionSpec::ShowWheel(reference) => self.open_wheel(&reference.wheel_id, cx),
            ActionSpec::SendShortcut(shortcut) => {
                let result = match target {
                    Some(target) => self
                        .shortcut_executor
                        .submit_shortcut_to(id, shortcut, target),
                    None => self.shortcut_executor.submit_shortcut(id, shortcut),
                };
                match result {
                    Ok(()) => {
                        self.action_error = None;
                        self.record_action_status(id, "等待发送快捷键".into());
                    }
                    Err(error) => {
                        self.action_error = Some((id, error.clone()));
                        self.record_action_status(id, error);
                    }
                }
                cx.notify();
            }
            ActionSpec::Command(command) => self.submit_action(id, command, cx),
            ActionSpec::External(action) => {
                let provider = action.provider.clone();
                let result = action
                    .validate()
                    .and_then(|()| action.unavailable_reason().map_or(Ok(()), Err))
                    .and_then(|()| match target {
                        Some(target) => self.executor.submit_external_to(id, action, target),
                        None => self.executor.submit_external(id, action),
                    });
                match result {
                    Ok(()) => {
                        self.action_error = None;
                        self.record_action_status(id, format!("等待调用 {provider} 外部动作"));
                    }
                    Err(error) => {
                        self.action_error = Some((id, error.clone()));
                        self.record_action_status(id, error);
                    }
                }
                cx.notify();
            }
            ActionSpec::Window(action) => {
                let result = match target {
                    Some(target) => self.shortcut_executor.submit_window_to(id, action, target),
                    None => self.shortcut_executor.submit_window(id, action),
                };
                match result {
                    Ok(()) => {
                        self.action_error = None;
                        self.record_action_status(id, "等待窗口操作".into());
                    }
                    Err(error) => {
                        self.action_error = Some((id, error.clone()));
                        self.record_action_status(id, error);
                    }
                }
                cx.notify();
            }
            ActionSpec::Plugin(reference) => {
                let result = if let Some(error) = &self.plugin_settings_error {
                    Err(error.clone())
                } else {
                    self.plugins
                        .resolve(&reference)
                        .map_err(|error| format!("{}（{}）", error, error.code()))
                        .and_then(|resolved| {
                            self.executor
                                .submit_group(id, resolved.command, Some(resolved.group))
                                .map(|()| {
                                    format!(
                                        "{} / {} · 适配 {} · 等待启动",
                                        reference.plugin_id,
                                        reference.action_id,
                                        resolved.plugin_version
                                    )
                                })
                        })
                };
                match result {
                    Ok(text) => {
                        self.action_error = None;
                        self.record_action_status(id, text);
                    }
                    Err(error) => {
                        self.action_error = Some((id, error.clone()));
                        self.record_action_status(id, error);
                        self.plugins.refresh_availability();
                        self.sync_bindings();
                    }
                }
                cx.notify();
            }
            _ => {}
        }
    }

    fn draft_command(&self, cx: &App) -> Result<Option<CommandAction>, String> {
        if !matches!(self.draft_action, ActionSpec::Command(_)) {
            return Ok(None);
        }
        let text = self.arguments.read(cx).text();
        let args = if text.trim().is_empty() {
            Vec::new()
        } else {
            serde_json::from_str::<Vec<String>>(&text).map_err(|_| {
                r#"参数格式有误，请填写字符串数组，例如 ["--capture"]。"#.to_string()
            })?
        };
        let directory = self.working_directory.read(cx).text();
        let command = CommandAction {
            program: self.program.read(cx).text(),
            args,
            working_directory: (!directory.is_empty()).then_some(directory),
        };
        command.validate()?;
        Ok(Some(command))
    }

    fn draft_action_spec(&self, cx: &App) -> Result<ActionSpec, String> {
        if let ActionSpec::Browser(action) = &self.draft_action {
            return self
                .browser_editor
                .snapshot(action, cx)
                .map(ActionSpec::Browser);
        }
        if let ActionSpec::Invoke(call) = &self.draft_action {
            let mut call = call.clone();
            call.input = serde_json::from_str(&self.package_input.read(cx).text())
                .map_err(|error| format!("动作参数需要 JSON 对象：{error}"))?;
            call.validate().map_err(|error| error.to_string())?;
            return Ok(ActionSpec::Invoke(call));
        }
        if matches!(self.draft_action, ActionSpec::Local(_)) {
            return self.local_editor.snapshot(cx).map(ActionSpec::Local);
        }
        if matches!(self.draft_action, ActionSpec::AdjustSystem(_)) {
            return self.draft_adjustment(cx).map(ActionSpec::AdjustSystem);
        }
        if matches!(self.draft_action, ActionSpec::SendShortcut(_)) {
            return ShortcutAction::parse(&self.shortcut_keys.read(cx).text())
                .map(ActionSpec::SendShortcut);
        }
        if let Some(command) = self.draft_command(cx)? {
            return Ok(ActionSpec::Command(command));
        }
        Ok(self.draft_action.clone())
    }

    fn test_action(&mut self, cx: &mut Context<Self>) {
        let id = if let Some(spec) = &self.pending {
            spec.id
        } else if let Some(id) = self.selected {
            id
        } else {
            return;
        };
        match self.draft_action_spec(cx) {
            Ok(action) => {
                self.record_action_status(id, "测试动作（未保存草稿）".into());
                self.dispatch_action(id, action, cx);
            }
            Err(error) => {
                self.action_error = Some((id, error));
                cx.notify();
            }
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
            .map(|spec| caps_for(&self.draft_spec(spec, cx).unwrap_or_else(|_| spec.clone())))
            .unwrap_or_default();
        self.show_message(caps, text, HitSource::Preview, cx);
    }

    fn record_hit(&mut self, caps: Vec<Cap>, message: String, source: HitSource) {
        match source {
            HitSource::Input => self.trigger_count += 1,
            HitSource::Panel | HitSource::Wheel => self.panel_click_count += 1,
            HitSource::Preview => {}
        }
        self.hits.push_front(Hit {
            time: clock(),
            caps,
            message,
            source,
        });
        self.hits.truncate(30);
    }

    fn show_message(
        &mut self,
        caps: Vec<Cap>,
        message: String,
        source: HitSource,
        cx: &mut Context<Self>,
    ) {
        self.record_hit(caps.clone(), message.clone(), source);
        self.toast_time = Some(Instant::now());
        let shown = Instant::now();
        if let Some(handle) = self.toast {
            if handle
                .update(cx, |toast, _, cx| {
                    toast.caps = caps.clone();
                    toast.message = message.clone();
                    toast.source = source;
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
                    source,
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
        self.edited_spec = None;
        if let Some(spec) = self
            .selected
            .and_then(|id| self.binding(id))
            .map(|b| b.spec.clone())
        {
            self.load_trigger_fields(&spec, cx);
        }
        let action = self
            .selected
            .and_then(|id| self.binding(id))
            .map(|b| b.action.clone())
            .unwrap_or(ActionSpec::Notification);
        let text = self
            .selected
            .and_then(|id| self.binding(id))
            .map(|binding| binding.message.clone())
            .unwrap_or_default();
        self.message
            .update(cx, |input, cx| input.set_text(text, cx));
        self.load_action(action, cx);
    }

    fn load_action(&mut self, action: ActionSpec, cx: &mut Context<Self>) {
        if let ActionSpec::Browser(action) = &action {
            self.browser_editor.load(action, cx);
            self.refresh_browser_profiles();
        }
        if let ActionSpec::Invoke(call) = &action {
            self.package_input
                .update(cx, |input, cx| input.set_text(call.input.to_string(), cx));
        }
        if let ActionSpec::Local(local) = &action {
            self.local_editor.load(local, cx);
        }
        if let ActionSpec::AdjustSystem(adjustment) = action {
            self.adjustment_amount.update(cx, |input, cx| {
                input.set_text(adjustment.delta_percent.unsigned_abs().to_string(), cx)
            });
        }
        let shortcut_text = match &action {
            ActionSpec::SendShortcut(shortcut) => shortcut.display(),
            _ => "Backspace".into(),
        };
        self.shortcut_keys
            .update(cx, |input, cx| input.set_text(shortcut_text, cx));
        let command = if let ActionSpec::Command(command) = &action {
            Some(command.clone())
        } else {
            None
        };
        self.draft_action = action;
        let command = command.unwrap_or(CommandAction {
            program: String::new(),
            args: Vec::new(),
            working_directory: None,
        });
        self.program
            .update(cx, |input, cx| input.set_text(command.program, cx));
        self.arguments.update(cx, |input, cx| {
            input.set_text(
                serde_json::to_string(&command.args).unwrap_or_else(|_| "[]".into()),
                cx,
            )
        });
        self.working_directory.update(cx, |input, cx| {
            input.set_text(command.working_directory.unwrap_or_default(), cx)
        });
    }

    fn start_recording(&mut self, cx: &mut Context<Self>) {
        self.begin_recording(false, None, cx);
    }

    fn record_trigger(&mut self, cx: &mut Context<Self>) {
        let editing = if self.pending.is_none() {
            self.selected
        } else {
            None
        };
        self.begin_recording(true, editing, cx);
    }

    fn begin_recording(
        &mut self,
        preserve_action: bool,
        editing: Option<u64>,
        cx: &mut Context<Self>,
    ) {
        if self.listener() != Listener::Listening {
            return;
        }
        self.recording_conditions = match self.condition_editor.conditions(cx) {
            Ok(conditions) => conditions,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        self.hide_wheel(cx);
        self.error = None;
        self.recording_for_panel = false;
        self.recording_preserve_action = preserve_action;
        self.recording_edit_id = editing;
        if self.send(BackendCommand::StartRecording) {
            self.smooth_scroll.cancel_all();
            self.recording = true;
            self.recording_held.clear();
            self.conflict = None;
            self.confirm_delete = None;
            self.page = Page::Triggers;
            self.status = "请按下组合并全部松开；按 Esc 取消。".into();
        }
        cx.notify();
    }

    fn discard_pending(&mut self, cx: &mut Context<Self>) {
        self.clear_pending();
        self.conflict = None;
        self.status = "已放弃录制结果。".into();
        self.load_selected_message(cx);
        cx.notify();
    }

    fn select(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.recording {
            return;
        }
        self.smooth_scroll.cancel("inspector");
        self.selected = Some(id);
        self.clear_pending();
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
        let base = self.pending.as_ref().or_else(|| {
            self.selected
                .and_then(|id| self.binding(id))
                .map(|b| &b.spec)
        });
        let spec = match base
            .ok_or_else(|| "请先录制组合或选择预设。".to_string())
            .and_then(|base| self.draft_spec(base, cx))
        {
            Ok(spec) => spec,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        let action = match self.draft_action_spec(cx) {
            Ok(action) => action,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        if let Some(error) = self.action_unavailable_reason(&action) {
            self.error = Some(format!(
                "动作尚不可用：{error}。请先在“扩展”页安装或启用插件。"
            ));
            cx.notify();
            return;
        }
        let adding = self.pending.is_some();
        let result = if adding {
            self.triggers
                .add(Binding {
                    spec,
                    message: text,
                    enabled: true,
                    action,
                })
                .map(Some)
        } else if let Some(id) = self.selected {
            self.triggers.update(id, spec, text, action).map(|_| None)
        } else {
            self.error = Some("请先录制一个组合。".into());
            cx.notify();
            return;
        };
        match result {
            Ok(added) => {
                self.applied();
                if let Some(id) = added {
                    self.pending = None;
                    self.selected = Some(id);
                    self.status = "触发器已添加，试着在任意窗口触发它。".into();
                } else {
                    self.status = "触发器动作已保存。".into();
                }
                self.edited_spec = None;
                self.load_selected_message(cx);
            }
            // 新录制的冲突在录制区原位提示；修改已有绑定时同时给出错误横幅。
            Err(TriggerError::Conflict(conflict)) if adding => self.conflict = Some(conflict),
            Err(error) => {
                if let TriggerError::Conflict(conflict) = &error {
                    self.conflict = Some(conflict.clone());
                }
                self.error = Some(error.message());
            }
        }
        cx.notify();
    }

    /// 服务层已提交绑定变更后，清除旧错误并同步输入后端。
    fn applied(&mut self) {
        self.error = None;
        self.sync_bindings();
    }

    fn clear_pending(&mut self) {
        if let Some(spec) = self.pending.take() {
            self.action_status.remove(&spec.id);
        }
    }

    fn choose_plugin_action(
        &mut self,
        reference: PluginActionRef,
        title: String,
        cx: &mut Context<Self>,
    ) {
        if self.recording {
            return;
        }
        if self.pending.is_some() && self.draft_action == ActionSpec::Plugin(reference.clone()) {
            self.page = Page::Triggers;
            cx.notify();
            return;
        }
        let Some(id) = self.draft_id(None) else {
            cx.notify();
            return;
        };
        match self.plugins.default_trigger(&reference, id) {
            Ok(spec) => {
                self.clear_pending();
                self.edited_spec = None;
                self.load_trigger_fields(&spec, cx);
                self.conflict = self.triggers.check_candidate(&spec, None).err();
                self.pending = Some(spec);
                self.load_action(ActionSpec::Plugin(reference), cx);
                self.message
                    .update(cx, |input, cx| input.set_text(title.clone(), cx));
                self.page = Page::Triggers;
                self.error = None;
                self.status = format!("已选择{title}，默认快捷键已填入草稿，可修改后保存。");
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        cx.notify();
    }

    fn restore_default_trigger(&mut self, cx: &mut Context<Self>) {
        let ActionSpec::Plugin(reference) = &self.draft_action else {
            return;
        };
        let id = self.pending.as_ref().map(|s| s.id).or(self.selected);
        let Some(id) = id else {
            return;
        };
        match self.plugins.default_trigger(reference, id) {
            Ok(spec) => {
                let editing = if self.pending.is_none() {
                    Some(id)
                } else {
                    None
                };
                self.conflict = self.check_draft_trigger(&spec, editing).err();
                self.load_trigger_fields(&spec, cx);
                if editing.is_some() {
                    self.edited_spec = Some(spec);
                } else {
                    self.pending = Some(spec);
                }
                self.status = "默认快捷键已恢复到草稿，检查冲突后保存才会生效。".into();
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        cx.notify();
    }

    fn save_plugin_settings(&mut self, enabled: bool, cx: &mut Context<Self>) -> bool {
        self.save_plugin_configuration(self.plugins.raishot_path().to_string(), enabled, cx)
    }

    fn save_plugin_configuration(
        &mut self,
        path: String,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.executor.is_plugin_running("raishot") {
            self.error = Some("请等待 RaiShot 动作结束后再更改插件设置。".into());
            cx.notify();
            return false;
        }
        let result = if let Some(error) = &self.plugin_settings_error {
            Err(error.clone())
        } else {
            let mut candidate = self.plugins.clone();
            candidate
                .configure_raishot(path, enabled)
                .and_then(|()| candidate.save(&settings::path()))
                .map(|()| candidate)
                .map_err(|error| error.to_string())
                .map(|candidate| {
                    self.plugins = candidate;
                })
        };
        if let Err(error) = result {
            self.error = Some(error);
            cx.notify();
            return false;
        }
        self.error = None;
        self.sync_bindings();
        self.refresh_panel(cx);
        self.status = if enabled {
            "RaiShot 插件已启用，所有插件动作已就绪。"
        } else {
            "RaiShot 插件已停用，绑定配置已保留。"
        }
        .into();
        cx.notify();
        true
    }

    fn refresh_plugins(&mut self, cx: &mut Context<Self>) {
        if self.install_events.is_some() || self.executor.is_plugin_running("raishot") {
            self.error = Some("请等待 RaiShot 当前任务结束后再刷新插件。".into());
            cx.notify();
            return;
        }
        match Plugins::load(&settings::path()) {
            Ok(plugins) => {
                self.plugins = plugins;
                self.plugin_settings_error = None;
            }
            Err(error) => self.plugin_settings_error = Some(error.to_string()),
        }
        self.plugins.refresh();
        if let Some(message) = relocate_raishot(&mut self.plugins) {
            self.install_status = Some(message);
        }
        self.sync_bindings();
        self.refresh_panel(cx);
        self.status = "已刷新插件状态。".into();
        cx.notify();
    }

    fn install_raishot(&mut self, cx: &mut Context<Self>) {
        if self.install_events.is_some() || self.executor.is_plugin_running("raishot") {
            return;
        }
        if self.plugin_settings_error.is_some() {
            self.error = Some("请先修复插件设置文件，再刷新状态，原文件不会被覆盖。".into());
        } else {
            let configured = (!self.plugins.raishot_path().is_empty())
                .then(|| PathBuf::from(self.plugins.raishot_path()));
            match plugin_install::start_prepare(settings::path(), configured) {
                Ok(events) => {
                    self.install_events = Some(events);
                    self.install_status = Some("正在查找本机已有的 RaiShot…".into());
                }
                Err(error) => self.install_status = Some(format!("无法开始安装：{error}")),
            }
        }
        cx.notify();
    }

    fn toggle_rule(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.recording {
            return;
        }
        match self.triggers.toggle(id) {
            Ok(_) => self.applied(),
            Err(error) => self.error = Some(error.message()),
        }
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
        match self.triggers.remove(id) {
            Ok(next) => {
                self.applied();
                self.local_executor.cancel_id(id);
                self.action_service.cancel_id(id);
                self.action_status.remove(&id);
                self.confirm_delete = None;
                self.selected = next;
                self.status = "触发器已删除。".into();
                self.load_selected_message(cx);
            }
            Err(error) => self.error = Some(error.message()),
        }
        cx.notify();
    }

    // ───────────────────────────── 侧栏 ─────────────────────────────

    fn render_sidebar(&self, header: f32, cx: &mut Context<Self>) -> Div {
        let listener = self.listener();
        let enabled = self
            .triggers
            .bindings()
            .iter()
            .filter(|b| b.enabled && self.action_available(&b.action))
            .count();
        let (label, sub, color) = match listener {
            Listener::Listening => (
                "正在监听",
                format!("{enabled} 条触发器生效中"),
                theme::ACCENT,
            ),
            Listener::Recording => ("正在录制", "输入暂被拦截 · Esc 取消".into(), theme::RECORD),
            Listener::Paused => ("已暂停", "触发器不会响应".into(), theme::TEXT_DIM),
            Listener::Offline => ("未监听", "输入线程已结束".into(), theme::DANGER),
            Listener::Starting => ("正在连接", "正在安装输入监听".into(), theme::TEXT_DIM),
            Listener::Rebuilding => ("正在重建", "请松开其他按键".into(), theme::TEXT_DIM),
        };
        let action = match listener {
            Listener::Listening => small(icon_button(
                "listener-action",
                Icon::Pause,
                "暂停监听",
                Tone::Secondary,
                true,
            ))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_pause(cx))),
            Listener::Paused => small(icon_button(
                "listener-action",
                Icon::Play,
                "恢复监听",
                Tone::Primary,
                true,
            ))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_pause(cx))),
            Listener::Offline => small(icon_button(
                "listener-action",
                Icon::Refresh,
                "重建监听",
                Tone::DangerSolid,
                true,
            ))
            .on_click(cx.listener(|this, _, _, cx| this.rebuild_backend(cx))),
            Listener::Recording | Listener::Starting | Listener::Rebuilding => small(button(
                "listener-action",
                match listener {
                    Listener::Recording => "录制中",
                    Listener::Rebuilding => "正在重建…",
                    _ => "正在连接…",
                },
                Tone::Secondary,
                false,
            )),
        }
        .w_full();
        let status = div()
            .rounded(px(8.))
            .p(px(12.))
            .bg(rgb(theme::ROW))
            .border_1()
            .border_color(rgb(theme::BORDER))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(theme::beacon(color, 7.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(label),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(11.5))
                                    .text_color(rgb(theme::TEXT_DIM))
                                    .child(sub),
                            ),
                    ),
            )
            .child(action);

        let panel_combo = self
            .triggers
            .bindings()
            .iter()
            .find(|b| matches!(b.action, ActionSpec::ShowPanel) && b.enabled)
            .map(|b| caps_for(&b.spec));
        let launcher = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                icon_button(
                    "open-panel",
                    Icon::Bolt,
                    "打开触发面板",
                    Tone::Secondary,
                    !self.recording,
                )
                .h(px(40.))
                .w_full()
                .on_click(cx.listener(|this, _, _, cx| this.toggle_panel(cx))),
            )
            .child(
                div()
                    .id("record-panel-shortcut")
                    .px(px(10.))
                    .py(px(8.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(rgb(theme::BORDER))
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .when(listener == Listener::Listening, |el| {
                        el.cursor_pointer()
                            .hover(|s| s.bg(rgb(theme::ROW)))
                            .on_click(cx.listener(|this, _, _, cx| this.record_panel_shortcut(cx)))
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .child(div().flex_1().child(caption("呼出组合")))
                            .child(icon(Icon::Record, 12., theme::TEXT_DIM))
                            .child(div().ml(px(4.)).child(caption("录制"))),
                    )
                    .child(match panel_combo {
                        Some(caps) => combo(&caps, CapSize::Small).into_any_element(),
                        None => caption("未设置").into_any_element(),
                    }),
            );

        let nav = |id: &'static str,
                   page: Page,
                   glyph: Icon,
                   label: &'static str,
                   count: Option<usize>| {
            let active = self.page == page;
            div()
                .id(id)
                .relative()
                .h(px(34.))
                .px(px(10.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(11.))
                .text_size(px(13.))
                .cursor_pointer()
                .text_color(rgb(if active {
                    theme::TEXT
                } else {
                    theme::TEXT_MUTED
                }))
                .when(active, |el| {
                    el.bg(rgb(theme::SELECTED))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(
                            div()
                                .absolute()
                                .left(px(0.))
                                .top(px(9.))
                                .w(px(2.))
                                .h(px(16.))
                                .rounded(px(2.))
                                .bg(rgb(theme::ACCENT)),
                        )
                })
                .when(!active, |el| {
                    el.hover(|s| s.bg(rgb(theme::ROW)).text_color(rgb(theme::TEXT_SOFT)))
                })
                .child(icon(
                    glyph,
                    16.,
                    if active {
                        theme::ACCENT
                    } else {
                        theme::TEXT_DIM
                    },
                ))
                .child(div().flex_1().child(label))
                .when_some(count, |el, count| {
                    el.child(
                        div()
                            .min_w(px(22.))
                            .h(px(18.))
                            .px(px(6.))
                            .rounded(px(9.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .font_family(theme::MONO_FONT)
                            .text_size(px(11.))
                            .bg(if active {
                                hsla(theme::ACCENT, 0.14)
                            } else {
                                hsla(0x161923, 1.)
                            })
                            .text_color(rgb(if active {
                                theme::ACCENT
                            } else {
                                theme::TEXT_DIM
                            }))
                            .child(count.to_string()),
                    )
                })
                .on_click(cx.listener(move |this, _, _, cx| this.open_page(page, cx)))
        };
        let group = |label: &'static str| {
            div()
                .px(px(12.))
                .pt(px(6.))
                .pb(px(4.))
                .text_size(px(11.))
                .text_color(rgb(theme::TEXT_FAINT))
                .child(label)
        };

        div()
            .w(px(192.))
            .flex_none()
            .h_full()
            .relative()
            .overflow_hidden()
            .bg(rgb(theme::SIDEBAR))
            .border_r_1()
            .border_color(rgb(theme::DIVIDER))
            .child(
                div()
                    .size_full()
                    .px(px(14.))
                    .pb(px(14.))
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .child(
                        // 应用名占据标题栏左侧，可拖动窗口。
                        div()
                            .h(px(header.max(36.) + 12.))
                            .flex_none()
                            .px(px(4.))
                            .flex()
                            .items_center()
                            .gap(px(11.))
                            .window_control_area(WindowControlArea::Drag)
                            .child(logo(30.))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .text_size(px(15.))
                                            .font_weight(FontWeight::BOLD)
                                            .child("OmniTrigger"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(10.5))
                                            .text_color(rgb(theme::TEXT_FAINT))
                                            .child("组合输入触发引擎"),
                                    ),
                            ),
                    )
                    .child(status)
                    .child(launcher)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(group("工作区"))
                            .child(nav(
                                "nav-triggers",
                                Page::Triggers,
                                Icon::Keyboard,
                                "触发器",
                                Some(self.triggers.bindings().len()),
                            ))
                            .child(nav(
                                "nav-actions",
                                Page::Actions,
                                Icon::Grid,
                                "动作库",
                                None,
                            ))
                            .child(nav(
                                "nav-wheels",
                                Page::Wheels,
                                Icon::Wheel,
                                "轨道轮盘",
                                Some(self.wheels.wheels.len()),
                            ))
                            .child(nav(
                                "nav-presets",
                                Page::Presets,
                                Icon::Layers,
                                "触发预设",
                                None,
                            ))
                            .child(nav(
                                "nav-extensions",
                                Page::Extensions,
                                Icon::Plug,
                                "扩展",
                                None,
                            ))
                            .child(div().h(px(10.)))
                            .child(group("监控"))
                            .child(nav(
                                "nav-history",
                                Page::History,
                                Icon::Clock,
                                "触发记录",
                                (!self.hits.is_empty()).then_some(self.hits.len()),
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
                    .child(nav(
                        "nav-settings",
                        Page::Settings,
                        Icon::Sliders,
                        "设置",
                        None,
                    )),
            )
    }

    // ───────────────────────────── 触发器页 ─────────────────────────────

    fn render_banner(&self, cx: &mut Context<Self>) -> Option<Div> {
        let banner = |color: u32, glyph: Icon| {
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .px(px(12.))
                .py(px(10.))
                .rounded(px(12.))
                .bg(theme::tint_fill(color, 0.14, 0.04))
                .border_1()
                .border_color(hsla(color, 0.32))
                .child(icon_tile(glyph, color, 30.))
        };
        if let Some(error) = self
            .error
            .as_ref()
            .or_else(|| self.action_error.as_ref().map(|(_, error)| error))
        {
            let offline = self.backend.is_none() && !self.restarting;
            return Some(
                banner(theme::DANGER, Icon::Alert)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.5))
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
                                this.action_error = None;
                                cx.notify();
                            }),
                        )
                    }),
            );
        }
        (self.listener() == Listener::Paused).then(|| {
            banner(theme::TEXT_MUTED, Icon::Pause)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(12.5))
                        .text_color(rgb(theme::TEXT_SOFT))
                        .child("监听已暂停。触发器不会响应，按键和滚轮照常交给应用。"),
                )
                .child(
                    small(icon_button(
                        "banner-resume",
                        Icon::Play,
                        "恢复监听",
                        Tone::Secondary,
                        true,
                    ))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_pause(cx))),
                )
        })
    }

    fn render_list(&self, cx: &mut Context<Self>) -> Div {
        let listening = self.listener() == Listener::Listening;
        let dim = matches!(self.listener(), Listener::Paused | Listener::Offline);
        let enabled = self
            .triggers
            .bindings()
            .iter()
            .filter(|b| b.enabled)
            .count();
        let mut rows = div()
            .id("trigger-rows")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(2.))
            .when(dim, |el| el.opacity(0.6));
        for binding in self.triggers.bindings() {
            let id = binding.spec.id;
            let selected = self.pending.is_none() && !self.recording && self.selected == Some(id);
            let stats = self.triggers.stats(id);
            let (glyph, tint) = action_look(&binding.action);
            let (detail, detail_color) = match self.action_unavailable_reason(&binding.action) {
                Some(reason) => (reason, theme::RECORD),
                None => (action_detail(&binding.action), theme::TEXT_DIM),
            };
            // 标题行右侧：停用标记，或本次运行的命中次数与最近时间。
            let badge = if !binding.enabled {
                Some(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(theme::TEXT_FAINT))
                        .child("已停用"),
                )
            } else {
                (stats.hits > 0).then(|| {
                    div()
                        .h(px(20.))
                        .px(px(7.))
                        .rounded(px(10.))
                        .bg(hsla(theme::VIOLET, 0.12))
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .font_family(theme::MONO_FONT)
                        .text_size(px(11.))
                        .text_color(rgb(theme::VIOLET))
                        .child(icon(Icon::Bolt, 10., theme::VIOLET))
                        .child(match &stats.last {
                            Some(last) => format!("{} · {}", thousands(stats.hits), &last[..5]),
                            None => thousands(stats.hits),
                        })
                })
            };
            rows = rows.child(
                div()
                    .id(("row-card", id))
                    .flex_none()
                    .relative()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .pl(px(12.))
                    .pr(px(12.))
                    .py(px(10.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(hsla(theme::BORDER, 0.))
                    .when(selected, |el| {
                        el.bg(rgb(theme::SELECTED)).child(
                            div()
                                .absolute()
                                .left(px(-1.))
                                .top(px(18.))
                                .bottom(px(18.))
                                .w(px(2.))
                                .rounded(px(2.))
                                .bg(rgb(theme::ACCENT)),
                        )
                    })
                    .when(!selected, |el| el.hover(|s| s.bg(rgb(theme::ROW))))
                    .child(
                        div()
                            .id(("row", id))
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .cursor_pointer()
                            .when(!binding.enabled, |el| el.opacity(0.45))
                            .child(icon_tile(glyph, tint, 28.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap(px(5.))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(8.))
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .truncate()
                                                    .text_size(px(13.))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .child(binding.message.clone()),
                                            )
                                            .when_some(badge, |el, badge| {
                                                el.child(badge.flex_none())
                                            }),
                                    )
                                    .child(
                                        div()
                                            .min_w_0()
                                            .flex()
                                            .items_center()
                                            .gap(px(10.))
                                            .child(
                                                combo(&caps_for(&binding.spec), CapSize::Small)
                                                    .flex_none()
                                                    .flex_nowrap(),
                                            )
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .truncate()
                                                    .text_size(px(12.))
                                                    .text_color(rgb(detail_color))
                                                    .child(detail),
                                            ),
                                    ),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| this.select(id, cx))),
                    )
                    .child(
                        switch(("switch", id), binding.enabled)
                            .on_click(cx.listener(move |this, _, _, cx| this.toggle_rule(id, cx))),
                    ),
            );
        }
        if self.triggers.bindings().is_empty() {
            rows = rows.child(
                div()
                    .py(px(56.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(12.))
                    .child(icon_tile(Icon::Keyboard, theme::ACCENT, 48.))
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
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
            .pt(px(6.))
            .pb(px(16.))
            .pl(px(20.))
            .pr(px(18.))
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(
                div()
                    .flex()
                    .items_end()
                    .justify_between()
                    .gap(px(12.))
                    .child(heading(
                        "触发器",
                        "按住前缀再按目标键或滚轮，全局生效",
                    ))
                    .child(
                        icon_button("record", Icon::Record, "录制组合", Tone::Primary, listening)
                            .on_click(cx.listener(|this, _, _, cx| this.start_recording(cx))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(chip(
                        Icon::Keyboard,
                        theme::ACCENT_2,
                        self.triggers.bindings().len().to_string(),
                        "触发器",
                    ))
                    .child(chip(Icon::Check, theme::ACCENT, enabled.to_string(), "已启用"))
                    .child(chip(
                        Icon::Bolt,
                        theme::VIOLET,
                        thousands(self.trigger_count),
                        "本次触发",
                    )),
            )
            .children(self.render_banner(cx))
            .child(self.smooth_scroll.wrap("trigger-rows", rows))
            .child(caption(
                "最多 64 条触发器，每条最多 8 个前缀。单独按下未用于组合的前缀时，原输入会补发给应用。",
            ))
    }

    fn inspector_header(title: &'static str, right: impl IntoElement) -> Div {
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::TEXT_MUTED))
                    .child(title),
            )
            .child(right)
    }

    /// 详情顶部的组合与状态摘要。
    fn stage(color: u32) -> Div {
        div()
            .rounded(px(6.))
            .px(px(12.))
            .py(px(14.))
            .bg(hsla(color, 0.035))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(10.))
    }

    /// 舞台底部的两项指标。
    fn stage_metrics(color: u32, metrics: [(&'static str, String); 2]) -> Div {
        let [left, right] = metrics;
        let metric = |(label, value): (&'static str, String)| {
            div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(2.))
                .child(
                    div()
                        .font_family(theme::MONO_FONT)
                        .text_size(px(16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(value),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(theme::TEXT_DIM))
                        .child(label),
                )
        };
        div()
            .w_full()
            .pt(px(10.))
            .border_t_1()
            .border_color(hsla(color, 0.16))
            .flex()
            .items_center()
            .child(metric(left))
            .child(div().w(px(1.)).h(px(28.)).bg(hsla(color, 0.16)))
            .child(metric(right))
    }

    fn message_field(&self, label: &'static str, hint: Option<&'static str>) -> Div {
        Self::input_field(label, &self.message, hint)
    }

    fn input_field(
        label: &'static str,
        input: &Entity<MessageInput>,
        hint: Option<&'static str>,
    ) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(px(7.))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(theme::TEXT_MUTED))
                    .child(label),
            )
            .child(input.clone())
            .when_some(hint, |el, hint| el.child(caption(hint)))
    }

    /// 舞台下方的触发组合工具：重新录制、预设和插件默认值。
    fn trigger_tools(&self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_wrap()
            .justify_center()
            .gap(px(6.))
            .child(
                small(icon_button(
                    "customize-trigger",
                    Icon::Record,
                    "重新录制",
                    Tone::Secondary,
                    self.listener() == Listener::Listening,
                ))
                .on_click(cx.listener(|this, _, _, cx| this.record_trigger(cx))),
            )
            .child(
                small(icon_button(
                    "select-trigger-preset",
                    Icon::Layers,
                    "选择预设",
                    Tone::Secondary,
                    !self.recording,
                ))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.preset_edit_id = if this.pending.is_none() {
                        this.selected
                    } else {
                        None
                    };
                    this.page = Page::Presets;
                    cx.notify();
                })),
            )
            .when(matches!(self.draft_action, ActionSpec::Plugin(_)), |el| {
                el.child(
                    small(icon_button(
                        "restore-default-trigger",
                        Icon::Refresh,
                        "恢复默认",
                        Tone::Ghost,
                        !self.recording,
                    ))
                    .on_click(cx.listener(|this, _, _, cx| this.restore_default_trigger(cx))),
                )
            })
    }

    fn trigger_fields(&self, spec: &TriggerSpec, cx: &mut Context<Self>) -> Div {
        let spec = self
            .edited_spec
            .as_ref()
            .filter(|draft| draft.id == spec.id)
            .unwrap_or(spec);
        let fields = div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(section_title("触发方式"));
        if matches!(spec.activation, Activation::Wheel(_)) {
            let mut directions = theme::segments();
            for (key, label, direction) in [
                ("wheel-up", "上滚", omnitrigger_core::WheelDirection::Up),
                ("wheel-down", "下滚", omnitrigger_core::WheelDirection::Down),
                ("wheel-left", "左滚", omnitrigger_core::WheelDirection::Left),
                (
                    "wheel-right",
                    "右滚",
                    omnitrigger_core::WheelDirection::Right,
                ),
            ] {
                directions = directions.child(
                    segment(key, label, spec.activation == Activation::Wheel(direction))
                        .flex_1()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(base) = this
                                .pending
                                .as_ref()
                                .or_else(|| {
                                    this.selected
                                        .and_then(|id| this.binding(id))
                                        .map(|binding| &binding.spec)
                                })
                                .cloned()
                            {
                                this.edited_spec = Some(TriggerSpec {
                                    activation: Activation::Wheel(direction),
                                    ..base
                                });
                                cx.notify();
                            }
                        })),
                );
            }
            return fields
                .child(directions)
                .child(caption("滚轮累计一格时触发；位置条件和按住组合独立配置。"))
                .child(self.condition_fields(cx));
        }
        fields
            .child(
                theme::segments()
                    .child(
                        segment(
                            "trigger-press",
                            "按下即触发",
                            self.trigger_mode == TriggerMode::Press,
                        )
                        .flex_1()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_trigger_mode(TriggerMode::Press, cx)
                        })),
                    )
                    .child(
                        segment(
                            "trigger-hold",
                            "长按触发",
                            self.trigger_mode == TriggerMode::Hold,
                        )
                        .flex_1()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_trigger_mode(TriggerMode::Hold, cx)
                        })),
                    )
                    .child(
                        segment(
                            "trigger-drag",
                            "快速拖拽",
                            self.trigger_mode == TriggerMode::Drag,
                        )
                        .flex_1()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_trigger_mode(TriggerMode::Drag, cx)
                        })),
                    ),
            )
            .when(self.trigger_mode == TriggerMode::Drag, |el| {
                el.child(Self::input_field(
                    "启动距离（物理像素）",
                    &self.drag_distance,
                    Some("4–500；在时窗内达到位移阈值后认领本次按压。"),
                ))
                .child(Self::input_field(
                    "启动时窗（毫秒）",
                    &self.drag_window,
                    Some("20–2000；须早于同组合的长按阈值。默认 18px / 180ms。"),
                ))
                .child(caption(
                    "轮盘动作在认领后持续按住选择，松开执行；其他动作在认领时执行。",
                ))
            })
            .when(self.trigger_mode == TriggerMode::Hold, |el| {
                el.child(Self::input_field(
                    "长按时间（毫秒）",
                    &self.hold_duration,
                    Some("50–10000；达到时间触发一次，提前松开回放原按压。"),
                ))
                .child(caption(
                    "额外按键或滚轮取消等待；鼠标位移超过 8px 也会取消。",
                ))
            })
            .child(self.condition_fields(cx))
    }

    fn render_presets(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut content = div()
            .id("preset-catalog")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(22.));
        for (group, glyph, tint) in [
            ("快速拖拽", Icon::Mouse, theme::ACCENT),
            ("屏幕区域", Icon::Mouse, theme::ACCENT_2),
            ("长按", Icon::Clock, theme::VIOLET),
            ("键盘组合", Icon::Keyboard, theme::ACCENT_2),
            ("单键与鼠标组合", Icon::Mouse, theme::PINK),
        ] {
            let mut cards = div().grid().grid_cols(2).gap(px(12.));
            let mut count = 0;
            for (index, preset) in PRESETS
                .iter()
                .enumerate()
                .filter(|(_, preset)| preset.group == group)
            {
                count += 1;
                let hover_group = SharedString::from(format!("preset-{}", preset.key));
                cards =
                    cards.child(
                        div()
                            .id(preset.key)
                            .group(hover_group.clone())
                            .min_w_0()
                            .p(px(16.))
                            .rounded(px(14.))
                            .bg(theme::gradient(
                                180.,
                                hsla(theme::SURFACE, 1.),
                                hsla(theme::ROW, 1.),
                            ))
                            .border_1()
                            .border_color(rgb(theme::BORDER))
                            .flex()
                            .flex_col()
                            .gap(px(12.))
                            .when(!self.recording, |el| {
                                el.cursor_pointer()
                                    .hover(|s| s.border_color(hsla(tint, 0.5)))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.choose_preset(index, cx)
                                    }))
                            })
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(10.))
                                    .child(icon_tile(glyph, tint, 32.))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .text_size(px(14.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(preset.title),
                                    )
                                    .child(
                                        icon(Icon::ArrowRight, 14., theme::TEXT_FAINT)
                                            .group_hover(hover_group, |s| s.text_color(rgb(tint))),
                                    ),
                            )
                            .child(combo(&caps_for(&preset.spec(0)), CapSize::Small))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(rgb(theme::TEXT_DIM))
                                    .line_clamp(2)
                                    .child(preset.description),
                            ),
                    );
            }
            content = content.child(
                div()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(section_title(group))
                            .child(
                                div()
                                    .font_family(theme::MONO_FONT)
                                    .text_size(px(11.))
                                    .text_color(rgb(theme::TEXT_FAINT))
                                    .child(count.to_string()),
                            ),
                    )
                    .child(cards),
            );
        }
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .pl(px(20.))
            .pr(px(24.))
            .pt(px(6.))
            .pb(px(18.))
            .flex()
            .flex_col()
            .gap(px(18.))
            .child(heading(
                "触发预设",
                "选择触发方式并调整参数；保留已选动作，保存后生效。",
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .p(px(14.))
                    .rounded(px(14.))
                    .bg(theme::tint_fill(theme::ACCENT, 0.13, 0.03))
                    .border_1()
                    .border_color(hsla(theme::ACCENT, 0.26))
                    .child(icon_tile(Icon::Bolt, theme::ACCENT, 36.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("右键快拖与静止长按可以共存"),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(rgb(theme::TEXT_MUTED))
                                    .child(
                                        "快拖时窗必须早于长按阈值；命中后由同一次按压互斥认领。",
                                    ),
                            ),
                    ),
            )
            .child(self.smooth_scroll.wrap("preset-catalog", content))
            .child(caption(
                "预设只填入草稿，保存后才生效。已有组合和前缀冲突会明确报告。",
            ))
    }

    /// 动作类型选择格。
    fn kind_tile(
        id: &'static str,
        glyph: Icon,
        tint: u32,
        label: &'static str,
        active: bool,
    ) -> Stateful<Div> {
        div()
            .id(id)
            .min_w_0()
            .h(px(64.))
            .rounded(px(12.))
            .border_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(7.))
            .text_size(px(12.))
            .whitespace_nowrap()
            .cursor_pointer()
            .when(active, |el| {
                el.bg(theme::tint_fill(tint, 0.18, 0.04))
                    .border_color(hsla(tint, 0.6))
                    .text_color(rgb(theme::TEXT))
                    .font_weight(FontWeight::SEMIBOLD)
            })
            .when(!active, |el| {
                el.bg(rgb(theme::BACKGROUND))
                    .border_color(rgb(theme::BORDER))
                    .text_color(rgb(theme::TEXT_MUTED))
                    .hover(|s| {
                        s.border_color(rgb(theme::BORDER_HI))
                            .text_color(rgb(theme::TEXT_SOFT))
                    })
            })
            .child(icon(
                glyph,
                18.,
                if active { tint } else { theme::TEXT_DIM },
            ))
            .child(label)
    }

    fn action_fields(&self, hint: Option<&'static str>, cx: &mut Context<Self>) -> Div {
        if matches!(self.draft_action, ActionSpec::Browser(_)) {
            return div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(section_title("浏览器操作"))
                        .child(
                            small(button(
                                "change-browser-action-kind",
                                "更换动作",
                                Tone::Ghost,
                                !self.recording,
                            ))
                            .on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.draft_action = ActionSpec::Notification;
                                    cx.notify();
                                },
                            )),
                        ),
                )
                .child(self.browser_action_fields(cx))
                .child(self.message_field("动作名称", None))
                .child(
                    small(icon_button(
                        "test-browser-action",
                        Icon::Play,
                        "测试动作",
                        Tone::Secondary,
                        !self.recording,
                    ))
                    .on_click(cx.listener(|this, _, _, cx| this.test_action(cx))),
                );
        }
        let notification = matches!(self.draft_action, ActionSpec::Notification);
        let command = matches!(self.draft_action, ActionSpec::Command(_));
        let panel = matches!(self.draft_action, ActionSpec::ShowPanel);
        let plugin = matches!(self.draft_action, ActionSpec::Plugin(_));
        let shortcut = matches!(self.draft_action, ActionSpec::SendShortcut(_));
        let wheel = matches!(self.draft_action, ActionSpec::ShowWheel(_));
        let system = matches!(self.draft_action, ActionSpec::AdjustSystem(_));
        let local = matches!(self.draft_action, ActionSpec::Local(_));
        let package = matches!(self.draft_action, ActionSpec::Invoke(_));
        let browser = matches!(self.draft_action, ActionSpec::Browser(_));
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(section_title("触发后执行"))
            .child(
                div()
                    .grid()
                    .grid_cols(3)
                    .gap(px(8.))
                    .child(
                        Self::kind_tile(
                            "notification-only",
                            Icon::Bell,
                            theme::VIOLET,
                            "仅提示",
                            notification,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.draft_action = ActionSpec::Notification;
                            cx.notify();
                        })),
                    )
                    .child(
                        Self::kind_tile(
                            "send-shortcut-action",
                            Icon::Keyboard,
                            theme::ACCENT_2,
                            "发送快捷键",
                            shortcut,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.draft_action =
                                ActionSpec::SendShortcut(ShortcutAction { keys: vec![0x08] });
                            cx.notify();
                        })),
                    )
                    .child(
                        Self::kind_tile(
                            "command-action",
                            Icon::Terminal,
                            theme::RECORD,
                            "运行程序",
                            command,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.draft_action = ActionSpec::Command(CommandAction {
                                program: String::new(),
                                args: Vec::new(),
                                working_directory: None,
                            });
                            cx.notify();
                        })),
                    )
                    .child(
                        Self::kind_tile(
                            "plugin-action",
                            Icon::Capture,
                            theme::PINK,
                            "插件动作",
                            plugin,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            if matches!(this.draft_action, ActionSpec::Plugin(_)) {
                                return;
                            }
                            match this.plugins.catalogue() {
                                Ok(actions) => if let Some(action) = actions.into_iter().next() {
                                    this.load_action(ActionSpec::Plugin(action.reference), cx);
                                    if this.message.read(cx).text().trim().is_empty() {
                                        this.message.update(cx, |input, cx| input.set_text(action.title, cx));
                                    }
                                },
                                Err(error) => this.error = Some(error.to_string()),
                            }
                            cx.notify();
                        })),
                    )
                    .child(
                        Self::kind_tile(
                            "show-panel-action",
                            Icon::Panel,
                            theme::ACCENT,
                            "触发面板",
                            panel,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.draft_action = ActionSpec::ShowPanel;
                            cx.notify();
                        })),
                    )
                    .child(
                        Self::kind_tile("show-wheel-action", Icon::Pulse, theme::VIOLET, "轨道轮盘", wheel)
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(wheel) = this.wheels.wheels.first() {
                                    this.choose_draft_wheel(wheel.id.clone(), cx);
                                } else {
                                    this.page = Page::Wheels;
                                    cx.notify();
                                }
                            })),
                    ),
            )
            .when(wheel, |el| el.child(self.draft_wheel_fields(cx)))
            .child(Self::kind_tile("browser-action", Icon::Browser, theme::ACCENT_2, "浏览器操作", browser)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.load_action(ActionSpec::Browser(browser_actions::BrowserAction {
                        command: browser_actions::BrowserCommand::ReloadTab, profile: None,
                    }), cx);
                    this.refresh_browser_profiles();
                    cx.notify();
                })))
            .when(browser, |el| el.child(self.browser_action_fields(cx)))
            .child(Self::kind_tile("package-action", Icon::Terminal, theme::ACCENT_2, "动作包 / JavaScript", package)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.package_picker_for_draft = true;
                    this.page = Page::Actions;
                    cx.notify();
                })))
            .when(package, |el| el.child(self.package_action_fields(cx)))
            .child(Self::kind_tile("system-adjustment",Icon::Pulse,theme::ACCENT_2,"音量 / 屏幕亮度",system).on_click(cx.listener(|this,_,_,cx| {
                this.draft_action=ActionSpec::AdjustSystem(system_action::SystemAdjustmentAction {target:system_action::AdjustmentTarget::Volume,delta_percent:5});
                this.adjustment_amount.update(cx,|input,cx|input.set_text("5",cx));
                cx.notify();
            })))
            .when(system,|el|el.child(self.system_action_fields(cx)))
            .child(Self::kind_tile("local-action",Icon::Layers,theme::ACCENT_2,"本地步骤动作",local).on_click(cx.listener(|this,_,_,cx|this.choose_local_action(cx))))
            .when(local,|el|el.child(self.local_action_fields(false,cx))
                .child(div().flex().flex_wrap().gap(px(6.))
                    .child(small(button("test-local-action","运行当前草稿",Tone::Secondary,!self.recording)).on_click(cx.listener(|this,_,_,cx|this.test_action(cx))))
                    .child(small(button("cancel-local-action","停止此动作",Tone::Danger,true)).on_click(cx.listener(|this,_,_,cx|this.stop_local_action(false,cx)))))
                .child(caption("输入步骤作用于运行时的前台窗口。正式试用请保存并切换到目标软件，再用组合触发。")))
            .child(self.message_field("动作名称 / 提示消息", hint))
            .when(panel, |el| {
                el.child(caption(
                    "打开或收起触发面板。该组合用于控制面板，不作为面板中的动作卡片；右键长按每次在当前鼠标位置显示。",
                ))
            })
            .when(plugin, |el| el.child(self.plugin_action_fields(cx)))
            .when(shortcut, |el| {
                let result = ShortcutAction::parse(&self.shortcut_keys.read(cx).text());
                let (preview, ok) = match result {
                    Ok(shortcut) => (format!("将发送：{}", shortcut.display()), true),
                    Err(error) => (error, false),
                };
                el.child(Self::input_field(
                    "要发送的按键 / 快捷键",
                    &self.shortcut_keys,
                    Some("单键如 Backspace、Left；组合如 Ctrl+C、Ctrl+Shift+Left。"),
                ))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_size(px(12.))
                        .text_color(rgb(if ok { theme::ACCENT } else { theme::DANGER }))
                        .child(icon(
                            if ok { Icon::Check } else { Icon::Alert },
                            13.,
                            if ok { theme::ACCENT } else { theme::DANGER },
                        ))
                        .child(preview),
                )
                .child(caption("保存后用触发组合测试。普通按键发送完整按下与释放；独立音量键使用 Windows 原生调节与浮层，不叠加本软件提示。"))
            })
            .when(command, |el| {
                el.child(Self::input_field(
                    "程序",
                    &self.program,
                    Some("路径不加引号；脚本请填写解释器程序。"),
                ))
                .child(Self::input_field(
                    "参数",
                    &self.arguments,
                    Some(r#"每项单独传递，例如 ["--capture"]。无参数填 []。"#),
                ))
                .child(Self::input_field(
                    "工作目录（可选）",
                    &self.working_directory,
                    None,
                ))
                .child(
                    small(icon_button(
                        "test-action",
                        Icon::Play,
                        "测试动作",
                        Tone::Secondary,
                        true,
                    ))
                    .on_click(cx.listener(|this, _, _, cx| this.test_action(cx))),
                )
                .child(caption(
                    "测试会立即运行当前草稿。仅报告进程状态，实际结果由程序确认。",
                ))
            })
    }

    fn render_recording(&self) -> (Vec<AnyElement>, Option<AnyElement>) {
        let held = caps_for_buttons(&self.recording_held);
        let mut live = div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_center()
            .gap(px(9.));
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
                .gap(px(10.))
                .text_size(px(12.))
                .text_color(rgb(theme::TEXT_MUTED))
                .child(div().mt(px(6.)).child(dot(theme::RECORD_DIM, 5.)))
                .child(div().flex_1().child(text))
        };
        let body = vec![
            Self::inspector_header("新增触发器", status_pill("录制中", theme::RECORD))
                .into_any_element(),
            div()
                .h(px(250.))
                .rounded(px(16.))
                .border_1()
                .border_dashed()
                .border_color(hsla(theme::RECORD, 0.45))
                .bg(theme::tint_fill(theme::RECORD, 0.13, 0.02))
                .p(px(16.))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(18.))
                .child(
                    div()
                        .size(px(56.))
                        .rounded(px(28.))
                        .border_2()
                        .border_color(hsla(theme::RECORD, 0.45))
                        .bg(hsla(theme::RECORD, 0.08))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon(Icon::Record, 26., theme::RECORD)),
                )
                .child(live)
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(rgb(theme::RECORD_TEXT))
                        .child(hint),
                )
                .into_any_element(),
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(bullet("录制期间输入暂被拦截，不会传给其他应用。"))
                .child(bullet("其他软件注入的模拟输入不参与录制。"))
                .child(bullet(
                    "鼠标点击也会被录制；单独的左键、右键不能作为触发器。",
                ))
                .into_any_element(),
        ];
        let footer = div()
            .h(px(36.))
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
            .into_any_element();
        (body, Some(footer))
    }

    fn render_captured(
        &self,
        spec: &TriggerSpec,
        cx: &mut Context<Self>,
    ) -> (Vec<AnyElement>, Option<AnyElement>) {
        let draft = self.draft_spec(spec, cx);
        let conflict = match &draft {
            Ok(spec) => self.triggers.check_candidate(spec, None).err(),
            Err(error) => Some(Conflict {
                title: "触发参数无效",
                detail: error.clone(),
                other: None,
            }),
        };
        let display_spec = draft.as_ref().unwrap_or(spec);
        let ok = conflict.is_none();
        let color = if ok { theme::ACCENT } else { theme::DANGER };
        let status = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .text_size(px(12.5))
            .text_color(rgb(color))
            .child(icon(if ok { Icon::Check } else { Icon::Alert }, 14., color))
            .child(match &conflict {
                None => "没有冲突，可以添加",
                Some(conflict) => conflict.title,
            });
        let mut body = vec![
            Self::inspector_header(
                "新增触发器",
                status_pill(if ok { "待保存" } else { "存在冲突" }, color),
            )
            .into_any_element(),
            Self::stage(color)
                .child(
                    combo_toned(
                        &caps_for(display_spec),
                        CapSize::Large,
                        if ok { CapTone::Lit } else { CapTone::Normal },
                    )
                    .justify_center(),
                )
                .child(status)
                .into_any_element(),
            self.trigger_tools(cx).into_any_element(),
            self.trigger_fields(spec, cx).into_any_element(),
        ];
        let footer = match conflict {
            None => {
                body.push(self.action_fields(None, cx).into_any_element());
                if matches!(self.draft_action, ActionSpec::Browser(_)) {
                    body.swap(3, 4);
                }
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        icon_button("add", Icon::Check, "添加触发器", Tone::Primary, true)
                            .flex_1()
                            .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                    )
                    .child(
                        button("preview-draft", "预览", Tone::Secondary, true)
                            .on_click(cx.listener(|this, _, _, cx| this.preview_draft(cx))),
                    )
                    .child(
                        button("discard", "放弃", Tone::Ghost, true)
                            .on_click(cx.listener(|this, _, _, cx| this.discard_pending(cx))),
                    )
            }
            Some(conflict) => {
                let mut detail = div().flex().flex_col().gap(px(10.)).child(
                    div()
                        .text_size(px(12.5))
                        .text_color(rgb(theme::DANGER_TEXT))
                        .child(conflict.detail.clone()),
                );
                if let Some(other) = conflict.other.and_then(|id| self.binding(id)) {
                    let other_id = other.spec.id;
                    let (glyph, tint) = action_look(&other.action);
                    detail =
                        detail.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(12.))
                                .p(px(12.))
                                .rounded(px(12.))
                                .bg(rgb(theme::ROW))
                                .border_1()
                                .border_color(rgb(theme::BORDER_MID))
                                .child(icon_tile(glyph, tint, 34.))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .gap(px(6.))
                                        .child(
                                            div()
                                                .truncate()
                                                .text_size(px(13.))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .child(other.message.clone()),
                                        )
                                        .child(combo(&caps_for(&other.spec), CapSize::Small)),
                                )
                                .child(
                                    small(button("view-conflict", "查看", Tone::Secondary, true))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.select(other_id, cx)
                                        })),
                                ),
                        );
                }
                body.push(detail.into_any_element());
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        icon_button("rerecord", Icon::Record, "重新录制", Tone::Primary, true)
                            .flex_1()
                            .on_click(cx.listener(|this, _, _, cx| this.record_trigger(cx))),
                    )
                    .child(
                        button("discard", "取消", Tone::Secondary, true)
                            .on_click(cx.listener(|this, _, _, cx| this.discard_pending(cx))),
                    )
            }
        };
        (body, Some(footer.into_any_element()))
    }

    fn render_edit(
        &self,
        binding: &Binding,
        cx: &mut Context<Self>,
    ) -> (Vec<AnyElement>, Option<AnyElement>) {
        let id = binding.spec.id;
        let conflict = match self.draft_spec(&binding.spec, cx) {
            Ok(spec) => self.check_draft_trigger(&spec, Some(id)).err(),
            Err(detail) => Some(Conflict {
                title: "触发参数无效",
                detail,
                other: None,
            }),
        };
        let available = self.action_available(&binding.action);
        let draft_available = self
            .draft_action_spec(cx)
            .is_ok_and(|action| self.action_available(&action));
        let stats = self.triggers.stats(id);
        let draft = self.message.read(cx).text();
        let changed = !draft.trim().is_empty()
            && (draft != binding.message
                || self
                    .draft_spec(&binding.spec, cx)
                    .as_ref()
                    .map(|spec| spec != &binding.spec)
                    .unwrap_or(true)
                || self
                    .draft_action_spec(cx)
                    .as_ref()
                    .map(|action| action != &binding.action)
                    .unwrap_or(true));
        let confirming = self.confirm_delete == Some(id);
        let display = self
            .draft_spec(&binding.spec, cx)
            .unwrap_or_else(|_| binding.spec.clone());
        let (state, state_color) = if binding.enabled && !available {
            ("动作不可用", theme::RECORD)
        } else if binding.enabled {
            ("已启用", theme::ACCENT)
        } else {
            ("已停用", theme::TEXT_DIM)
        };
        let action_status = self
            .action_status
            .get(&id)
            .cloned()
            .unwrap_or_else(|| "本次运行尚未执行动作".into());
        let mut body = vec![
            Self::inspector_header("触发器详情", status_pill(state, state_color))
                .into_any_element(),
            Self::stage(theme::ACCENT)
                .child(
                    combo_toned(&caps_for(&display), CapSize::Large, CapTone::Lit).justify_center(),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(theme::TEXT_MUTED))
                        .child(activation_label(&display)),
                )
                .child(Self::stage_metrics(
                    theme::ACCENT,
                    [
                        ("真实触发", thousands(stats.hits)),
                        ("最近一次", stats.last.unwrap_or_else(|| "—".into())),
                    ],
                ))
                .into_any_element(),
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(self.trigger_tools(cx))
                .when_some(conflict.clone(), |el, conflict| {
                    el.child(
                        div()
                            .flex()
                            .gap(px(10.))
                            .p(px(12.))
                            .rounded(px(12.))
                            .bg(theme::tint_fill(theme::DANGER, 0.14, 0.04))
                            .border_1()
                            .border_color(hsla(theme::DANGER, 0.32))
                            .child(icon(Icon::Alert, 15., theme::DANGER))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(12.))
                                    .text_color(rgb(theme::DANGER_TEXT))
                                    .child(format!(
                                        "{}：{}{}",
                                        conflict.title,
                                        conflict.detail,
                                        conflict
                                            .other
                                            .and_then(|other| self.binding(other))
                                            .map(|other| format!(
                                                "（{}，#{}）",
                                                other.message, other.spec.id
                                            ))
                                            .unwrap_or_default()
                                    )),
                            ),
                    )
                })
                .into_any_element(),
            self.trigger_fields(&binding.spec, cx).into_any_element(),
            self.action_fields(Some("作为面板卡片名称；仅提示动作会显示 4 秒浮层。"), cx)
                .into_any_element(),
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(12.))
                .py(px(10.))
                .rounded(px(10.))
                .bg(rgb(theme::BACKGROUND))
                .border_1()
                .border_color(rgb(theme::BORDER))
                .child(icon(Icon::Pulse, 14., theme::TEXT_DIM))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(12.))
                        .text_color(rgb(theme::TEXT_MUTED))
                        .child(action_status),
                )
                .into_any_element(),
            caption("测试提示不计入触发次数。").into_any_element(),
        ];
        if matches!(self.draft_action, ActionSpec::Browser(_)) {
            body.swap(3, 4);
        }
        let footer = div()
            .flex()
            .gap(px(8.))
            .child(
                icon_button(
                    "save",
                    Icon::Check,
                    "保存动作",
                    Tone::Primary,
                    changed && conflict.is_none() && draft_available,
                )
                .flex_1()
                .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
            )
            .child(
                icon_button("preview", Icon::Play, "测试提示", Tone::Secondary, true)
                    .on_click(cx.listener(|this, _, _, cx| this.preview_draft(cx))),
            )
            .child(
                if confirming {
                    button("delete", "确认删除", Tone::DangerSolid, true)
                } else {
                    glyph_button("delete", Icon::Trash, Tone::Danger, true)
                        .border_1()
                        .border_color(hsla(theme::DANGER, 0.3))
                }
                .on_click(cx.listener(move |this, _, _, cx| this.remove_rule(id, cx))),
            )
            .into_any_element();
        (body, Some(footer))
    }

    fn render_inspector(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (body, footer) = if self.recording {
            self.render_recording()
        } else if let Some(spec) = self.pending.clone() {
            self.render_captured(&spec, cx)
        } else if let Some(binding) = self.selected.and_then(|id| self.binding(id)).cloned() {
            self.render_edit(&binding, cx)
        } else {
            (
                vec![
                    Self::inspector_header("触发器详情", div()).into_any_element(),
                    div()
                        .pt(px(120.))
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(px(14.))
                        .child(icon_tile(Icon::Keyboard, theme::ACCENT, 48.))
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
                ],
                None,
            )
        };
        div()
            .w(px(320.))
            .flex_none()
            .mt(px(2.))
            .mr(px(14.))
            .mb(px(14.))
            .rounded(px(8.))
            .bg(rgb(theme::INSPECTOR))
            .border_1()
            .border_color(rgb(theme::BORDER))
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("inspector")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(px(16.))
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .children(body.into_iter().map(|part| div().flex_none().child(part)))
                    .map(|el| self.smooth_scroll.wrap("inspector", el)),
            )
            .when_some(footer, |el, footer| {
                el.child(
                    div()
                        .flex_none()
                        .px(px(16.))
                        .py(px(14.))
                        .rounded_b(px(7.))
                        .bg(rgb(theme::INSPECTOR))
                        .border_t_1()
                        .border_color(rgb(theme::DIVIDER))
                        .child(footer),
                )
            })
    }

    // ───────────────────────────── 记录与诊断 ─────────────────────────────

    fn render_history(&self, cx: &mut Context<Self>) -> Div {
        let filter = self.history_filter;
        let tab = |id: &'static str, label: &'static str, value: HistoryFilter| {
            segment(id, label, filter == value).on_click(cx.listener(move |this, _, _, cx| {
                this.history_filter = value;
                cx.notify();
            }))
        };
        let mut list = div()
            .id("history-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .rounded(px(14.))
            .border_1()
            .border_color(rgb(theme::BORDER))
            .bg(rgb(theme::ROW));
        let mut shown = 0;
        for hit in self.hits.iter().filter(|hit| match filter {
            HistoryFilter::All => true,
            HistoryFilter::Real => hit.source == HitSource::Input,
            HistoryFilter::Panel => matches!(hit.source, HitSource::Panel | HitSource::Wheel),
            HistoryFilter::Preview => hit.source == HitSource::Preview,
        }) {
            shown += 1;
            let (tag, color) = match hit.source {
                HitSource::Preview => ("测试", theme::TEXT_MUTED),
                HitSource::Panel => ("面板", theme::RECORD),
                HitSource::Wheel => ("轮盘", theme::VIOLET),
                HitSource::Input => ("组合", theme::ACCENT),
            };
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .px(px(16.))
                    .py(px(11.))
                    .border_b_1()
                    .border_color(rgb(theme::DIVIDER))
                    .hover(|s| s.bg(rgb(0x131620)))
                    .child(
                        div()
                            .w(px(64.))
                            .flex_none()
                            .font_family(theme::MONO_FONT)
                            .text_size(px(12.))
                            .text_color(rgb(theme::TEXT_DIM))
                            .child(hit.time.clone()),
                    )
                    .child(div().w(px(48.)).flex_none().child(pill(tag, color)))
                    .child(div().w(px(230.)).flex_none().overflow_hidden().child(
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
                    .py(px(56.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(12.))
                    .child(icon_tile(Icon::Clock, theme::VIOLET, 48.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(rgb(theme::TEXT_DIM))
                            .child("按下一个组合，这里会出现触发记录。"),
                    ),
            );
        }
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .pt(px(6.))
            .pb(px(16.))
            .pl(px(20.))
            .pr(px(24.))
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(
                div()
                    .flex()
                    .items_end()
                    .justify_between()
                    .gap(px(12.))
                    .child(heading("触发记录", "仅保留本次运行期间的最近 30 条"))
                    .child(
                        theme::segments()
                            .child(tab("filter-all", "全部", HistoryFilter::All))
                            .child(tab("filter-real", "组合", HistoryFilter::Real))
                            .child(tab("filter-panel", "面板 / 轮盘", HistoryFilter::Panel))
                            .child(tab("filter-preview", "测试", HistoryFilter::Preview)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(chip(
                        Icon::Keyboard,
                        theme::ACCENT,
                        thousands(self.trigger_count),
                        "组合触发",
                    ))
                    .child(chip(
                        Icon::Panel,
                        theme::RECORD,
                        thousands(self.panel_click_count),
                        "面板 / 轮盘",
                    ))
                    .child(chip(
                        Icon::Play,
                        theme::TEXT_MUTED,
                        self.hits
                            .iter()
                            .filter(|hit| hit.source == HitSource::Preview)
                            .count()
                            .to_string(),
                        "测试提示",
                    )),
            )
            .child(self.smooth_scroll.wrap("history-list", list))
            .when(!self.action_log.is_empty(), |el| {
                el.child(
                    card()
                        .p(px(14.))
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .child(section_title("动作执行记录（含测试动作，最近 30 条）"))
                        .child(
                            div()
                                .id("action-log")
                                .max_h(px(160.))
                                .overflow_y_scroll()
                                .flex()
                                .flex_col()
                                .gap(px(6.))
                                .children(self.action_log.iter().map(|line| {
                                    div()
                                        .font_family(theme::MONO_FONT)
                                        .text_size(px(11.5))
                                        .text_color(rgb(theme::TEXT_DIM))
                                        .child(line.clone())
                                }))
                                .map(|el| self.smooth_scroll.wrap("action-log", el)),
                        ),
                )
            })
    }

    fn render_diagnostics(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let d = &self.diagnostics;
        let hold_outcome = |outcome: HoldOutcome| match outcome {
            HoldOutcome::Matched => "已命中",
            HoldOutcome::ShortRelease => "未满阈值松开",
            HoldOutcome::Movement => "移动取消",
            HoldOutcome::FocusChanged => "切窗取消",
            HoldOutcome::OtherButton => "其他按键打断",
            HoldOutcome::Wheel => "滚轮打断",
            HoldOutcome::Configuration => "配置变更取消",
            HoldOutcome::Paused => "暂停取消",
            HoldOutcome::Recording => "录制取消",
            HoldOutcome::Recovery => "恢复或错误取消",
        };
        let rejected = match d.last_hold_start_rejection {
            Some(HoldStartRejection::Paused) => "监听暂停",
            Some(HoldStartRejection::Recording) => "正在录制",
            Some(HoldStartRejection::ExtraHeld) => "其他按键仍按住，组合未匹配",
            Some(HoldStartRejection::ExistingPress) => "已有该键按压，尚未收到释放",
            Some(HoldStartRejection::Unmatched) => "组合未匹配",
            None => "无",
        };
        let age = |ms: Option<u64>| {
            ms.map(|ms| format!("最近 {:.1} 秒前", ms as f64 / 1000.))
                .unwrap_or_else(|| "尚无事件".into())
        };
        let held = caps_for_buttons(&d.held);
        let info = |glyph: Icon, tint: u32, title: &'static str| {
            card()
                .flex_1()
                .min_w_0()
                .p(px(16.))
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(icon_tile(glyph, tint, 28.))
                        .child(
                            div()
                                .text_size(px(14.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(title),
                        ),
                )
        };
        let para = |text: &'static str| {
            div()
                .text_size(px(12.))
                .text_color(rgb(theme::TEXT_MUTED))
                .child(text)
        };
        let latency = self.last_panel_latency.as_ref();
        let page = div()
            .pt(px(6.))
            .pb(px(18.))
            .pl(px(20.))
            .pr(px(24.))
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
                        Icon::Refresh,
                        theme::ACCENT,
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
                        Icon::Keyboard,
                        theme::ACCENT_2,
                    ))
                    .child(stat(
                        "鼠标回调",
                        thousands(d.mouse_events),
                        Some((age(d.last_mouse_ms), theme::TEXT_FAINT)),
                        Icon::Mouse,
                        theme::VIOLET,
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
                        Icon::Terminal,
                        theme::RECORD,
                    ))
                    .child(
                        card()
                            .flex_grow()
                            .flex_basis(px(0.))
                            .min_w_0()
                            .p(px(16.))
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .flex_1()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(10.))
                                    .child(icon_tile(Icon::Bolt, theme::PINK, 28.))
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(rgb(theme::TEXT_DIM))
                                            .child("当前按住"),
                                    ),
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
            .child(section_title("长按与面板耗时"))
            .child(
                div()
                    .flex()
                    .gap(px(10.))
                    .child(stat(
                        "最近长按判定",
                        d.last_hold_elapsed_ms
                            .map(|ms| format!("{ms} ms"))
                            .unwrap_or_else(|| "—".into()),
                        Some((
                            match (d.last_hold_threshold_ms, d.last_hold_lateness_ms) {
                                (Some(threshold), Some(late)) => format!(
                                    "阈值 {threshold}ms · 超出 {late}ms · 最多超出 {}ms",
                                    self.max_hold_lateness_ms
                                ),
                                (Some(threshold), None) => {
                                    format!("阈值 {threshold}ms · 提前松开或取消")
                                }
                                _ => "尚无长按记录".into(),
                            },
                            theme::TEXT_DIM,
                        )),
                        Icon::Clock,
                        theme::VIOLET,
                    ))
                    .child(stat(
                        "上次长按至面板提交",
                        latency
                            .map(|latency| format!("{} ms", latency.total_ms))
                            .unwrap_or_else(|| "—".into()),
                        Some((
                            latency
                                .map(|latency| {
                                    format!(
                                        "判定 {}ms · 阈值 {}ms · 最长 {}ms",
                                        latency.held_ms,
                                        latency.threshold_ms,
                                        self.max_panel_total_ms
                                    )
                                })
                                .unwrap_or_else(|| "尚无长按打开面板".into()),
                            theme::TEXT_DIM,
                        )),
                        Icon::Panel,
                        theme::ACCENT,
                    ))
                    .child(stat(
                        "判定后的界面耗时",
                        latency
                            .map(|latency| {
                                format!("{} ms", latency.queue_ms.saturating_add(latency.display_ms))
                            })
                            .unwrap_or_else(|| "—".into()),
                        Some((
                            latency
                                .map(|latency| {
                                    format!(
                                        "通知 {}ms · 显示 {}ms",
                                        latency.queue_ms, latency.display_ms
                                    )
                                })
                                .unwrap_or_else(|| "不含显示器扫描输出".into()),
                            theme::TEXT_DIM,
                        )),
                        Icon::Pulse,
                        theme::ACCENT_2,
                    )),
            )
            .child(caption(format!(
                "计时方式：{}。长按命中 {} · 移动取消 {} · 切窗取消 {} · 短按 / 其他取消 {}；仅记录耗时与次数，不保存原始输入。",
                if d.hold_timer_backend.is_empty() {
                    "等待监听就绪"
                } else {
                    d.hold_timer_backend
                },
                d.hold_triggered,
                d.hold_cancelled_movement,
                d.hold_cancelled_focus,
                d.hold_cancelled_other
            )))
            .child(caption(format!(
                "Hook 墙钟耗时最大值：本体 {:.2}ms（慢 {} 次）· 下游链 {:.2}ms（慢 {} 次）· 后台补发 {:.2}ms（慢 {} 次）；慢指超过 50ms，补发独立执行。",
                d.max_hook_work_us as f64 / 1000.,
                d.slow_hook_work_count,
                d.max_hook_chain_us as f64 / 1000.,
                d.slow_hook_chain_count,
                d.max_replay_us as f64 / 1000.,
                d.slow_replay_count
            )))
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(10.))
                    .child(
                        info(Icon::Clock, theme::VIOLET, "最近 8 次长按结果")
                            .child(caption(format!(
                                "按下 {} · 释放 {} · 开始计时 {} · 未开始 {} · 重复鼠标按下 {}",
                                d.hold_terminal_downs,
                                d.hold_terminal_ups,
                                d.hold_started,
                                d.hold_not_started,
                                d.hold_terminal_duplicate_downs
                            )))
                            .child(caption(format!("最近未开始原因：{rejected}")))
                            .when(d.recent_hold_attempts.is_empty(), |el| {
                                el.child(caption("尚无长按结果"))
                            })
                            .children(d.recent_hold_attempts.iter().rev().map(|attempt| {
                                let matched = attempt.outcome == HoldOutcome::Matched;
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .gap(px(8.))
                                    .px(px(10.))
                                    .py(px(5.))
                                    .rounded(px(8.))
                                    .bg(rgb(theme::BACKGROUND))
                                    .text_size(px(12.))
                                    .text_color(rgb(if matched {
                                        theme::ACCENT
                                    } else {
                                        theme::TEXT_MUTED
                                    }))
                                    .child(hold_outcome(attempt.outcome))
                                    .child(
                                        div()
                                            .font_family(theme::MONO_FONT)
                                            .text_size(px(11.5))
                                            .child(format!(
                                                "{} / {} ms",
                                                attempt.elapsed_ms, attempt.threshold_ms
                                            )),
                                    )
                            })),
                    )
                    .child(
                        info(Icon::Panel, theme::ACCENT, "面板显示与收起")
                            .child(caption(format!(
                                "呼出请求 {} · 原生提交 {} · 跳过 {} · 失败 {}",
                                self.panel_show_stats.requested,
                                self.panel_show_stats.submitted,
                                self.panel_show_stats.skipped,
                                self.panel_show_stats.failed
                            )))
                            .child(caption(format!(
                                "呈现请求 {} · 收起 {} · 其中失焦收起 {}",
                                self.panel_diagnostics.present_count,
                                self.panel_diagnostics.hide_count,
                                self.panel_diagnostics.focus_dismiss_count
                            )))
                            .child(caption(format!(
                                "最近显示：{}",
                                if self.panel_show_stats.last.is_empty() {
                                    "尚未呼出"
                                } else {
                                    self.panel_show_stats.last
                                }
                            )))
                            .child(caption(format!(
                                "最近面板事件：{}",
                                self.panel_diagnostics.last_event
                            )))
                            .child(caption("原生提交表示显示调用成功返回，不代表屏幕像素已亮起。")),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap(px(10.))
                    .child(
                        info(Icon::Refresh, theme::ACCENT_2, "偶发失效时")
                            .child(para("1. 松开所有按键和鼠标按钮。"))
                            .child(para(
                                "2. 点击“重建监听”，旧监听会先收尾已消费的按压。",
                            ))
                            .child(para("3. 触发器、暂停状态和未保存的录制结果都会保留。"))
                            .child(
                                div().flex().child(
                                    small(icon_button(
                                        "diag-rebuild",
                                        Icon::Refresh,
                                        "重建监听",
                                        Tone::Secondary,
                                        true,
                                    ))
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.rebuild_backend(cx)),
                                    ),
                                ),
                            ),
                    )
                    .child(
                        info(Icon::Layers, theme::RECORD, "与其他输入工具并用")
                            .child(para(
                                "Windows 按“后安装先处理”的顺序调用低级 Hook。其他软件若在上游拦截 CapsLock、侧键或滚轮，这里可能收不到原输入。",
                            ))
                            .child(para(
                                "程序不会定时重装 Hook 抢占链首；仅凭漏触发无法判断具体来源。",
                            )),
                    ),
            );
        div()
            .id("diagnostics")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .child(page)
            .map(|el| self.smooth_scroll.wrap("diagnostics", el))
    }

    /// 标题栏高度与系统按钮宽度（逻辑像素），并同步按钮下方的底色。
    fn caption_area(&mut self, window: &Window) -> (f32, f32) {
        let Some(hwnd) = self.title_bar else {
            return (0., 0.);
        };
        let background = theme::resolve_color(theme::BACKGROUND);
        if self.caption_color != Some(background) {
            omnitrigger_windows::set_caption_color(hwnd, background);
            self.caption_color = Some(background);
        }
        let scale = window.scale_factor();
        omnitrigger_windows::caption_buttons(hwnd)
            .map(|buttons| (buttons.height as f32 / scale, buttons.width as f32 / scale))
            .unwrap_or((32., 138.))
    }

    /// 主区底色延伸进标题栏，右上角留空给系统按钮。
    fn render_title_bar(&self, height: f32, buttons_width: f32) -> Div {
        div()
            .flex_none()
            .h(px(height))
            .flex()
            .window_control_area(WindowControlArea::Drag)
            .child(div().flex_1().h_full().bg(rgb(theme::BACKGROUND)))
            .child(div().flex_none().w(px(buttons_width)).h_full())
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
        let metric = |glyph: Icon, value: String| {
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .child(icon(glyph, 12., theme::TEXT_FAINT))
                .child(
                    div()
                        .font_family(theme::MONO_FONT)
                        .text_color(rgb(theme::TEXT_MUTED))
                        .child(value),
                )
        };
        div()
            .h(px(26.))
            .flex_none()
            .px(px(14.))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(16.))
            .bg(rgb(theme::SIDEBAR))
            .border_t_1()
            .border_color(rgb(theme::DIVIDER))
            .text_size(px(11.5))
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
            .when(self.page == Page::Diagnostics, |el| {
                el.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(14.))
                        .child(metric(
                            Icon::Keyboard,
                            thousands(self.diagnostics.keyboard_events),
                        ))
                        .child(metric(
                            Icon::Mouse,
                            thousands(self.diagnostics.mouse_events),
                        ))
                        .child(metric(
                            Icon::Terminal,
                            thousands(self.diagnostics.external_injected),
                        ))
                        .child(div().w(px(1.)).h(px(12.)).bg(rgb(theme::BORDER_MID)))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(5.))
                                .child("按住")
                                .child(div().text_color(rgb(theme::TEXT_SOFT)).child(held)),
                        ),
                )
            })
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        theme::apply(self.theme_mode, window.appearance());
        let (title_height, buttons_width) = self.caption_area(window);
        let page = match self.page {
            Page::Actions => self.render_actions(window, cx).into_any_element(),
            Page::Wheels => self.render_wheels(cx).into_any_element(),
            Page::Presets => self.render_presets(cx).into_any_element(),
            Page::Extensions => self.render_extensions(cx).into_any_element(),
            Page::Triggers => div()
                .flex_1()
                .min_w_0()
                .flex()
                .child(self.render_list(cx))
                .child(self.render_inspector(cx))
                .into_any_element(),
            Page::History => self.render_history(cx).into_any_element(),
            Page::Diagnostics => self.render_diagnostics(cx).into_any_element(),
            Page::Settings => self.render_appearance(window, cx).into_any_element(),
        };
        // 纯色工作区避免大半径模糊随交互反复绘制。
        let main = div()
            .flex_1()
            .min_h_0()
            .relative()
            .overflow_hidden()
            .bg(rgb(theme::BACKGROUND))
            .flex()
            .child(div().flex_1().min_w_0().flex().child(page));
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
                            .child(main),
                    ),
            )
            .child(self.render_status_bar())
    }
}

struct Toast {
    caps: Vec<Cap>,
    message: String,
    source: HitSource,
    shown: Instant,
}

impl Render for Toast {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let progress = (1. - self.shown.elapsed().as_secs_f32() / TOAST_SECONDS).clamp(0., 1.);
        let preview = self.source == HitSource::Preview;
        let header = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(logo(18.))
            .child(
                div()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::TEXT_MUTED))
                    .child("OmniTrigger"),
            )
            .when(self.source != HitSource::Input, |el| {
                el.child(pill(
                    match self.source {
                        HitSource::Panel => "面板",
                        HitSource::Wheel => "轮盘",
                        _ => "测试",
                    },
                    if self.source == HitSource::Panel {
                        theme::RECORD
                    } else {
                        theme::TEXT_MUTED
                    },
                ))
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
            .rounded(px(14.))
            .bg(theme::gradient(
                160.,
                hsla(0x171b28, 1.),
                hsla(0x0c0e15, 1.),
            ))
            .border_1()
            .border_color(if preview {
                hsla(theme::BORDER_MID, 1.)
            } else {
                hsla(theme::ACCENT, 0.42)
            })
            .px(px(16.))
            .pt(px(13.))
            .pb(px(12.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .text_size(px(15.))
                    .line_height(px(22.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .line_clamp(2)
                    .child(self.message.clone()),
            )
            .child(
                div()
                    .h(px(3.))
                    .flex_none()
                    .rounded(px(2.))
                    .bg(hsla(
                        if preview { theme::PLUS } else { theme::ACCENT },
                        0.14,
                    ))
                    .child(
                        div()
                            .h_full()
                            .w(gpui::relative(progress))
                            .rounded(px(2.))
                            .when(preview, |el| el.bg(rgb(theme::PLUS)))
                            .when(!preview, |el| {
                                el.bg(theme::accent_fill(90.)).shadow(theme::glow(
                                    theme::ACCENT,
                                    0.6,
                                    6.,
                                ))
                            }),
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
    if let Some(preview) = system_hud::Preview::parse(&arguments) {
        match preview {
            Ok(preview) => preview.run(),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    if let Some(result) = system_action::cli(&arguments) {
        match result {
            Ok(text) => println!("{text}"),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    if let Some(index) = arguments.iter().position(|argument| argument == "--config") {
        let result = arguments
            .get(index + 1)
            .filter(|path| !path.starts_with("--"))
            .ok_or_else(|| "--config 后需要配置文件路径。".to_string())
            .and_then(|path| settings::use_path(path.into()));
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
    if let Some(result) = browser_actions_ui::cli(&arguments) {
        match result {
            Ok(text) => println!("{text}"),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    if let Some(result) = browser_connector_ui::cli(&arguments) {
        match result {
            Ok(text) => println!("{text}"),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    if let Some(result) = action_packages::cli(&arguments) {
        match result {
            Ok(text) => println!("{text}"),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    if arguments
        .iter()
        .any(|argument| argument == "--check-config")
    {
        match settings::load().and_then(|bindings| {
            settings::validate(&bindings)?;
            let wheels = Wheels::load(&settings::path())?;
            for binding in &bindings {
                if let ActionSpec::ShowWheel(reference) = &binding.action {
                    if !wheels
                        .wheels
                        .iter()
                        .any(|wheel| wheel.id == reference.wheel_id)
                    {
                        return Err(format!(
                            "触发器 #{} 引用的轮盘“{}”不存在。",
                            binding.spec.id, reference.wheel_id
                        ));
                    }
                }
            }
            Ok(bindings.len())
        }) {
            Ok(count) => println!(
                "配置有效，共 {count} 条绑定：{}",
                settings::path().display()
            ),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    if arguments.iter().any(|argument| argument == "--smoke-test") {
        if let Err(error) = smoke_test() {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    let ui_smoke = arguments.iter().any(|argument| argument == "--ui-smoke");
    let appearance_settings = arguments.iter().any(|argument| argument == "--settings");
    let browser_connector_settings = arguments
        .iter()
        .any(|argument| argument == "--browser-connector-settings");
    let open_panel = arguments.iter().any(|argument| argument == "--panel");
    let initial_wheel = arguments
        .iter()
        .position(|argument| argument == "--wheel")
        .map(|index| {
            arguments
                .get(index + 1)
                .filter(|value| !value.starts_with("--"))
                .cloned()
                .ok_or_else(|| "--wheel 后需要轮盘标识。".to_string())
        })
        .transpose()
        .and_then(|id| {
            if let Some(id) = &id {
                WheelActionRef {
                    wheel_id: id.clone(),
                }
                .validate()?;
            }
            Ok(id)
        });
    let initial_wheel = match initial_wheel {
        Ok(id) => id,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };
    let wheel_editor = arguments
        .iter()
        .any(|argument| argument == "--wheel-editor");
    let preserve_foreground = initial_wheel.is_some() && !wheel_editor;
    let wheel_launch_target = if preserve_foreground {
        match omnitrigger_windows::capture_shortcut_target() {
            Ok(target) => Some(target),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
    } else {
        None
    };
    Application::new()
        .with_assets(Assets)
        .run(move |cx: &mut App| {
            message_input::init(cx);
            let bounds = Bounds::centered(None, size(px(1060.), px(780.)), cx);
            let result = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(920.), px(620.))),
                    focus: !preserve_foreground,
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
                    let main_window = gpui::Window::window_handle(&*window);
                    let view = cx.new(|cx| {
                        Workspace::new(
                            cx,
                            StartupOptions {
                                ui_smoke,
                                browser_connector_settings,
                                appearance_settings,
                                panel: open_panel && initial_wheel.is_none(),
                                wheel: initial_wheel,
                                wheel_target: wheel_launch_target,
                                wheel_editor,
                            },
                            main_window,
                            title_bar,
                        )
                    });
                    let closing_view = view.downgrade();
                    let appearance_view = view.downgrade();
                    window
                        .observe_window_appearance(move |window, cx| {
                            let _ = appearance_view.update(cx, |view, cx| {
                                if theme::apply(view.theme_mode, window.appearance()) {
                                    cx.refresh_windows();
                                    cx.notify();
                                }
                            });
                        })
                        .detach();
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
            if !preserve_foreground {
                cx.activate(true);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::{should_refresh_workspace_for_backend_event, thousands, Page};
    use omnitrigger_windows::{BackendEvent, PanelKey};

    #[test]
    fn background_input_and_diagnostics_do_not_redraw_unrelated_pages() {
        let events = [
            BackendEvent::PointerPressed { x: 100, y: 200 },
            BackendEvent::EscapePressed,
            BackendEvent::PanelKey(PanelKey::Down),
            BackendEvent::Diagnostics(Box::default()),
            BackendEvent::HoldTiming {
                id: 1,
                elapsed_ms: 300,
                threshold_ms: 300,
                matched_at: std::time::Instant::now(),
            },
        ];
        for page in [
            Page::Triggers,
            Page::Actions,
            Page::Wheels,
            Page::Presets,
            Page::History,
        ] {
            assert!(events
                .iter()
                .all(|event| !should_refresh_workspace_for_backend_event(event, page)));
        }
        assert!(should_refresh_workspace_for_backend_event(
            &BackendEvent::Diagnostics(Box::default()),
            Page::Diagnostics,
        ));
        assert!(!should_refresh_workspace_for_backend_event(
            &events[0],
            Page::Diagnostics,
        ));
    }

    #[test]
    fn recording_and_action_changes_still_refresh_every_page() {
        let events = [
            BackendEvent::Ready,
            BackendEvent::RecordingProgress(Vec::new()),
            BackendEvent::Recorded(super::PRESETS[0].spec(1)),
            BackendEvent::RecordingCancelled,
            BackendEvent::Triggered {
                id: 1,
                pointer: None,
            },
            BackendEvent::Error("监听已断开".into()),
        ];
        for page in [
            Page::Triggers,
            Page::Actions,
            Page::Wheels,
            Page::Presets,
            Page::History,
            Page::Diagnostics,
        ] {
            assert!(events
                .iter()
                .all(|event| should_refresh_workspace_for_backend_event(event, page)));
        }
    }

    #[test]
    fn thousands_groups_digits_from_the_right() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1284), "1,284");
        assert_eq!(thousands(1234567), "1,234,567");
    }
}
