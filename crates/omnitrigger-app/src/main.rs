#![windows_subsystem = "windows"]

mod action_library;
mod action_packages;
mod action_target_context;
mod adjustment_guard;
mod app_sets;
mod appearance;
mod appearance_ui;
mod assets;
mod brightness_step_host;
mod browser_actions;
mod browser_actions_ui;
mod browser_connector;
mod browser_connector_ui;
mod clipboard_history_background;
mod clipboard_history_controller;
mod clipboard_history_filter;
mod clipboard_history_host;
#[cfg(test)]
mod clipboard_history_review;
mod clipboard_history_send;
#[cfg(test)]
mod clipboard_history_send_candidate;
mod clipboard_history_store;
mod clipboard_history_ui;
mod clipboard_history_ui_state;
#[cfg(all(windows, test))]
mod clipboard_restore;
#[cfg(all(windows, test))]
mod clipboard_smoke_protection;
mod definitions;
mod desktop_host;
#[cfg(test)]
mod desktop_key_smoke;
#[cfg(test)]
mod desktop_text_smoke;
mod event_bridge;
mod execution;
mod explorer_context_helper;
mod external_action;
mod gesture_choice;
mod gesture_editor;
mod legacy_load_gate;
mod legacy_local_source;
mod legacy_plugin_ui;
mod legacy_plugin_work;
mod legacy_prepare_bridge;
mod legacy_upgrade;
mod local_action;
mod local_editor;
mod local_executor;
mod local_files;
#[cfg(test)]
mod local_text_smoke;
mod message_input;
mod mouse_shortcut_dispatch;
mod package_feedback;
mod plugin_install;
mod plugins;
mod presets;
#[cfg(all(windows, test))]
#[path = "../../omnitrigger-windows/tests/support/private_clipboard.rs"]
mod private_window_station;
mod process_capture_host;
mod profiles;
mod radial_wheel;
#[cfg(windows)]
mod read_once_helper;
mod scenes;
mod script_browser;
mod script_fs;
mod script_notify;
#[cfg(debug_assertions)]
mod script_notify_smoke;
mod script_open;
mod script_projects;
mod script_runtime;
mod script_ui;
mod selected_text_helper;
mod service;
mod settings;
mod shortcut;
mod smooth_scroll;
mod system_action;
mod system_executor;
mod system_hud;
mod theme;
mod trigger_browse;
mod trigger_conditions;
mod trigger_input_map;
mod trigger_panel;
mod trigger_preview;
mod update_ui;
mod updater;
mod volume_adjust_host;
mod wheel_editor;
mod wheel_input_cli;
mod wheel_scenes;
mod wheel_workspace;
mod wheels;
mod window_action;
mod workspace_mode;

use assets::{Assets, Icon};
use event_bridge::{EventBridge, UiWake};
use execution::{CommandAction, CommandExecutor, ExecutionOutcome};
use gpui::Focusable as _;
use gpui::{
    div, point, prelude::*, px, size, AnyElement, AnyWindowHandle, App, Application, Bounds,
    Context, Div, Entity, FontWeight, SharedString, Stateful, TitlebarOptions, Window,
    WindowBackgroundAppearance, WindowBounds, WindowControlArea, WindowHandle, WindowKind,
    WindowOptions,
};
use message_input::MessageInput;
use omnitrigger_core::{Activation, Button, MouseButton, TriggerSpec, WheelInputPolicy};
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
    collections::{HashMap, HashSet, VecDeque},
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
use workspace_mode::WorkspaceMode;

#[derive(Clone, Copy, PartialEq, Eq)]
enum TriggerMode {
    Press,
    Hold,
    Drag,
    Gesture,
}

impl TriggerMode {
    fn of(activation: Activation) -> Self {
        match activation {
            Activation::Hold { .. } => Self::Hold,
            Activation::Drag { .. } => Self::Drag,
            Activation::Gesture { .. } => Self::Gesture,
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
    TextHistory,
    Diagnostics,
    Settings,
}

fn should_refresh_workspace_for_backend_event(event: &BackendEvent, page: Page) -> bool {
    match event {
        BackendEvent::Ready
        | BackendEvent::RecordingProgress(_)
        | BackendEvent::Recorded(_)
        | BackendEvent::Triggered { .. }
        | BackendEvent::GestureChoice { .. }
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

/// 触发器页工具栏中展开的下拉。
#[derive(Clone, Copy, PartialEq, Eq)]
enum TriggerMenu {
    Facet(trigger_browse::Facet),
    Group,
}

/// 触发器页的浏览方式。
#[derive(Clone, Copy, PartialEq, Eq)]
enum TriggerView {
    List,
    Keyboard,
    Mouse,
}

/// 下拉中的一个选项。
#[derive(Clone, Copy)]
enum TriggerChoice {
    Facet(trigger_browse::Facet, Option<trigger_browse::FacetValue>),
    Group(trigger_browse::GroupBy),
}

/// 组合中目标键之前的部分：指针区域条件与前缀键，用作前缀层名称。
fn prefix_caps(spec: &TriggerSpec) -> Vec<Cap> {
    let mut caps = caps_for(spec);
    caps.truncate(spec.conditions.len() + spec.held.len());
    caps
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
    /// 测试配置档默认不安装 Hook。
    Suspended,
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
    *plugins = plugins
        .save_raishot_install(&settings::path(), found.cli.to_string_lossy().into_owned())
        .ok()?;
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
/// 触发器列表与浏览视图共用的摘要；`ready` 为动作当前是否可用。
fn browse_row(binding: &settings::Binding, ready: bool) -> trigger_browse::Row {
    let spec = &binding.spec;
    let caps = caps_for(spec);
    let action = trigger_browse::action_kind(&binding.action);
    let mut text = format!(
        "{} {} {}",
        binding.message,
        action_detail(&binding.action),
        action.label()
    );
    for cap in &caps {
        text.push(' ');
        text.push_str(&cap.label);
    }
    // 键帽只显示位置名称，尺寸与缩进仍可搜索。
    for condition in &spec.conditions {
        text.push(' ');
        text.push_str(&trigger_conditions::condition_label(condition));
    }
    // 场景名、集合名与展开后的名称都可搜索。
    let (scene, own) = binding.context.authored(spec);
    if let Some(name) = scene {
        text.push_str(" 场景 ");
        text.push_str(name);
    }
    let sources = &binding.context.scopes;
    for (scope, own, source) in [
        (
            &spec.process_scope,
            &own.foreground,
            sources.process.as_ref(),
        ),
        (&spec.pointer_scope, &own.pointer, sources.pointer.as_ref()),
    ] {
        let authored = (!own.is_global()).then(|| settings::process_scope_label(own, source));
        if let Some(label) = &authored {
            text.push(' ');
            text.push_str(label);
        }
        let actual = (!scope.is_global()).then(|| settings::process_scope_label(scope, None));
        if let Some(label) = actual.filter(|label| authored.as_ref() != Some(label)) {
            text.push(' ');
            text.push_str(&label);
        }
    }
    let (held_keys, held_mouse) = trigger_browse::split_held(&spec.held);
    let prefix_keys: Vec<String> = prefix_caps(spec)
        .iter()
        .map(|cap| cap.label.to_string())
        .collect();
    trigger_browse::Row {
        id: spec.id,
        text: text.to_lowercase(),
        prefix: prefix_keys.join(" + "),
        prefix_keys,
        input: trigger_browse::input_kind(spec),
        action,
        scope: trigger_browse::scope_kind(spec),
        status: trigger_browse::status_kind(binding, ready),
        held_keys,
        held_mouse,
        terminal: trigger_browse::Terminal::of(spec),
        has_region: !spec.conditions.is_empty(),
    }
}

/// 列表行第二行的说明文字与颜色；动作不可用时显示原因。
fn row_detail(binding: &settings::Binding, unavailable: Option<String>) -> (String, u32) {
    let (detail, detail_color) = match unavailable {
        Some(reason) => (reason, theme::RECORD),
        None => (action_detail(&binding.action), theme::TEXT_DIM),
    };
    // 沿用场景时显示场景名与绑定自身的附加条件，完整范围见详情。
    let (scene, own) = binding.context.authored(&binding.spec);
    let sources = &binding.context.scopes;
    let detail = if own.foreground.is_global() {
        detail
    } else {
        format!(
            "{} · {}",
            settings::process_scope_label(&own.foreground, sources.process.as_ref()),
            detail
        )
    };
    let detail = if own.pointer.is_global() {
        detail
    } else {
        format!(
            "指针窗口：{} · {}",
            settings::process_scope_label(&own.pointer, sources.pointer.as_ref()),
            detail
        )
    };
    let detail = match scene {
        Some(name) => format!("场景 {name} · {detail}"),
        None => detail,
    };
    let detail = if binding.spec.keyboard == Default::default() {
        detail
    } else {
        format!(
            "{}{} · {}",
            if binding.spec.keyboard.dedicated_prefix.is_some() {
                "专用层"
            } else if binding.spec.keyboard.retained_prefix.is_some() {
                "保留原功能层"
            } else if binding.spec.keyboard.matching.is_mouse_unmodified() {
                "鼠标无 Ctrl/Shift/Alt 观察"
            } else if binding.spec.keyboard.matching.is_mouse_exact() {
                "鼠标修饰键观察"
            } else if binding.spec.keyboard.matching.is_mouse_captured_leader() {
                "鼠标捕获会话引用"
            } else {
                ""
            },
            if binding.spec.keyboard.repeat {
                " 按住重复"
            } else {
                ""
            },
            detail
        )
    };
    (detail, detail_color)
}

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
        Activation::Gesture { .. } => "画出相似轨迹后松开时触发".into(),
        Activation::Wheel(_) => match spec.wheel {
            omnitrigger_core::WheelPolicy::Notch => "滚轮一格即触发".into(),
            omnitrigger_core::WheelPolicy::Packet => "每个滚轮事件触发一次".into(),
        },
    }
}

struct StartupOptions {
    mouse_shortcuts: Option<mouse_shortcut_dispatch::Plan>,
    wheel_input_policy: WheelInputPolicy,
    ui_smoke: bool,
    browser_connector_settings: bool,
    appearance_settings: bool,
    text_history_page: bool,
    text_history_target: Option<clipboard_history_send::HistoryLaunchTarget>,
    text_history_ui_window: Option<isize>,
    panel: bool,
    wheel: Option<String>,
    wheel_target: Option<omnitrigger_windows::ShortcutTarget>,
    wheel_editor: bool,
    /// 是否安装输入 Hook；测试配置档默认不安装。
    listen: bool,
    /// 测试配置档显示名，用于醒目标识。
    test_profile: Option<String>,
    test_data: bool,
}

struct Workspace {
    workspace_instance: Option<omnitrigger_windows::WorkspaceInstance>,
    mode: WorkspaceMode,
    listen: bool,
    test_profile: Option<String>,
    test_data: bool,
    backend: Option<Backend>,
    wheel_input_policy: WheelInputPolicy,
    events: Option<EventBridge<BackendEvent>>,
    ui_wake: UiWake,
    text_history: clipboard_history_ui::ClipboardHistoryUi,
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
    opening_test_copy: bool,
    test_copy_error: Option<String>,
    update_ui: update_ui::UpdateUi,
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
    gesture_draft: gesture_editor::GestureDraft,
    /// 等待用户点选的手势候选弹窗及其匹配上下文。
    gesture_choice: Option<gesture_choice::PendingGestureChoice>,
    gesture_choice_generation: u64,
    draft_action: ActionSpec,
    recording_for_panel: bool,
    recording_preserve_action: bool,
    recording_edit_id: Option<u64>,
    program: Entity<MessageInput>,
    arguments: Entity<MessageInput>,
    working_directory: Entity<MessageInput>,
    shortcut_keys: Entity<MessageInput>,
    action_search: Entity<MessageInput>,
    /// 触发器页的搜索框、筛选、分组与折叠状态，只在本次运行内保留。
    trigger_search: Entity<MessageInput>,
    trigger_filter: trigger_browse::Filter,
    trigger_group: trigger_browse::GroupBy,
    trigger_collapsed: HashSet<String>,
    trigger_menu: Option<TriggerMenu>,
    trigger_view: TriggerView,
    /// 键盘视图选中的前缀层；`None` 为全部层。
    trigger_layer: Option<String>,
    /// 键盘视图选中的键位。
    trigger_key: Option<u16>,
    trigger_mouse: Option<trigger_input_map::MousePart>,
    /// 键盘视图可用宽度（逻辑像素），由上一帧布局测得，用于让键盘随窗口缩放。
    trigger_board_width: std::rc::Rc<std::cell::Cell<f32>>,
    /// 列表视图的滚动位置，分组导航据此跳转并标出当前分组。
    trigger_list_scroll: gpui::ScrollHandle,
    /// 刚在导航中点选的分组；靠近列表底部时滚不到顶，滚动前一直标为当前。
    trigger_nav_pick: Option<String>,
    package_input: Entity<MessageInput>,
    package_picker_for_draft: bool,
    action_service: omnitrigger_actions::ActionService,
    package_feedback: package_feedback::PackageFeedback,
    script_projects: script_projects::ScriptProjects,
    plugins: Plugins,
    plugin_settings_error: Option<String>,
    browser_connector_ui: browser_connector_ui::BrowserConnectorUi,
    browser_executor: browser_actions::BrowserExecutor,
    browser_editor: browser_actions_ui::BrowserEditor,
    wheels: Wheels,
    wheels_error: Option<String>,
    /// bindings.json 中的场景，供轮盘槽位按目标窗口筛选；读取失败时为空，原因见 `wheel_scenes_error`。
    wheel_scenes: scenes::CompiledScenes,
    wheel_scenes_error: Option<String>,
    wheel_selected: Option<String>,
    wheel_name: Entity<MessageInput>,
    wheel_slot_label: Entity<MessageInput>,
    wheel_slot_shortcut: Entity<MessageInput>,
    wheel_slot_selected: Option<usize>,
    /// 轮盘编辑器中选中的格子（圈, 方向）。
    wheel_cell: Option<(u8, u8)>,
    wheel_preview: Option<wheel_workspace::WheelPreview>,
    wheel_moving: bool,
    wheel_replacing: bool,
    /// 正在编辑的应用专用布局（进程名）；`None` 为通用布局。
    wheel_scope: Option<wheel_workspace::WheelScope>,
    /// 应用范围下拉是否展开。
    wheel_scope_open: bool,
    /// 鼠标悬停的格子，只用于提亮显示。
    wheel_hover: Option<(u8, u8)>,
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
    raishot_install_origin: Option<Plugins>,
    install_status: Option<String>,
    legacy_gate: legacy_load_gate::LegacyLoadGate,
    legacy_source: Option<std::sync::Arc<dyn legacy_plugin_work::LegacyArtifactSource>>,
    legacy_job: Option<legacy_plugin_ui::LegacyUiJob>,
    legacy_installing: bool, // 现有单事件槽的作业类型，非持久化启用或授权状态。
    legacy_stop_after_job: bool,
    legacy_status: Option<String>,
    legacy_session_used: bool,
    legacy_last_stop: Option<bool>,
    legacy_close_requested: bool,
    executor: CommandExecutor,
    shortcut_executor: CommandExecutor,
    mouse_shortcut_invocations: HashMap<u64, u64>,
    mouse_shortcut_dispatch: Option<mouse_shortcut_dispatch::MouseShortcutDispatch>,
    mouse_shortcut_dispatch_error: Option<mouse_shortcut_dispatch::Error>,
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
    binding_reload_notice: Option<String>,
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
            mouse_shortcuts: mouse_shortcuts_plan,
            wheel_input_policy,
            ui_smoke,
            browser_connector_settings,
            appearance_settings,
            text_history_page,
            text_history_target,
            text_history_ui_window,
            panel: open_panel,
            wheel: initial_wheel,
            wheel_target: wheel_launch_target,
            wheel_editor,
            listen,
            test_profile,
            test_data,
        } = startup;
        let mode = WorkspaceMode::from_text_history_flag(text_history_page);
        let (theme_mode, theme_error) = match appearance::load(&settings::path()) {
            Ok(mode) => (mode, None),
            Err(error) => (appearance::Mode::default(), Some(error)),
        };
        theme::apply(theme_mode, cx.window_appearance());
        let (bindings, load_error) = if mode.owns_input_engine() {
            match settings::load() {
                Ok(bindings) => (bindings, None),
                Err(error) => (Vec::new(), Some(error)),
            }
        } else {
            (Vec::new(), None)
        };
        let (mut backend, events, mut startup_error) = if mode.owns_input_engine() && listen {
            match Backend::start_with_wheel_input_policy(wheel_input_policy) {
                Ok((backend, events)) => (Some(backend), Some(events), None),
                Err(error) => (None, Some(std::sync::mpsc::channel().1), Some(error)),
            }
        } else {
            (None, None, None)
        };
        let events = events.and_then(|events| match EventBridge::new(events) {
            Ok(events) => Some(events),
            Err(error) => {
                backend.take();
                startup_error = Some(error);
                None
            }
        });
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
        let (wheels, wheels_error) = if mode.owns_input_engine() {
            match Wheels::load(&settings::path()) {
                Ok(wheels) => (wheels, None),
                Err(error) => (Wheels { wheels: Vec::new() }, Some(error)),
            }
        } else {
            (Wheels { wheels: Vec::new() }, None)
        };
        let (wheel_scenes, wheel_scenes_error) = match settings::load_scenes() {
            Ok(scenes) => (scenes, None),
            Err(error) => (Default::default(), Some(error)),
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
        let (mut plugins, plugin_settings_error) = if mode.owns_input_engine() {
            match Plugins::load(&settings::path()) {
                Ok(plugins) => (plugins, None),
                Err(error) => (Plugins::default(), Some(error.to_string())),
            }
        } else {
            (Plugins::default(), None)
        };
        let relocated = (mode.owns_input_engine() && plugin_settings_error.is_none())
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
        let trigger_search = cx.new(|cx| {
            let mut input = MessageInput::new(cx).embedded();
            input.set_placeholder("搜索名称、动作、组合或应用");
            input
        });
        cx.observe(&trigger_search, |_, _, cx| cx.notify()).detach();
        let script_projects = script_projects::ScriptProjects::new(cx);
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
        let mut gesture_draft = gesture_editor::GestureDraft::default();
        if let Some(Activation::Gesture { shape, .. }) = initial_activation {
            gesture_draft.load(Some(shape));
        }
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
        let history_store = std::path::absolute(settings::path())
            .ok()
            .and_then(|path| {
                clipboard_history_store::ClipboardHistoryStore::new(
                    path.with_file_name(clipboard_history_store::FILE_NAME),
                )
                .ok()
            })
            .map(std::sync::Arc::new);
        let text_history = clipboard_history_ui::ClipboardHistoryUi::new(
            history_store,
            ui_wake.clone(),
            text_history_target,
            text_history_ui_window,
            cx,
        );
        let configured = mouse_shortcuts_plan
            .map(|plan| {
                if plan.bindings.iter().any(|id| {
                    !bindings.iter().any(|binding| {
                        binding.spec.id == *id && mouse_shortcut_dispatch::supports_binding(binding)
                    })
                }) {
                    return Err(mouse_shortcut_dispatch::Error::InvalidBindings);
                }
                mouse_shortcut_dispatch::MouseShortcutDispatch::new(plan)
            })
            .transpose();
        let (mouse_shortcut_dispatch, mouse_shortcut_dispatch_error) = match configured {
            Ok(queue) => (queue, None),
            Err(error) => (None, Some(error)),
        };
        let (legacy_source, legacy_location_error) =
            match legacy_local_source::locate(&settings::path(), &plugins) {
                Ok(source) => (source, None),
                Err(error) => (None, Some(error.to_string())),
            };
        let mut workspace = Self {
            workspace_instance: None,
            mode,
            listen,
            test_profile,
            test_data,
            backend,
            wheel_input_policy,
            events,
            ui_wake,
            text_history,
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
            opening_test_copy: false,
            test_copy_error: None,
            update_ui: update_ui::UpdateUi::new(),
            page: if text_history_page {
                Page::TextHistory
            } else if appearance_settings {
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
            gesture_draft,
            gesture_choice: None,
            gesture_choice_generation: 0,
            draft_action,
            recording_for_panel: false,
            recording_preserve_action: false,
            recording_edit_id: None,
            program,
            arguments,
            working_directory,
            shortcut_keys,
            action_search,
            trigger_search,
            trigger_filter: Default::default(),
            trigger_group: Default::default(),
            trigger_collapsed: HashSet::new(),
            trigger_menu: None,
            trigger_view: TriggerView::List,
            trigger_layer: None,
            trigger_key: None,
            trigger_mouse: None,
            trigger_board_width: Default::default(),
            trigger_list_scroll: gpui::ScrollHandle::new(),
            trigger_nav_pick: None,
            plugins,
            plugin_settings_error,
            browser_connector_ui,
            browser_executor: browser_actions::BrowserExecutor::new(settings::path()),
            browser_editor,
            wheels,
            wheels_error,
            wheel_scenes,
            wheel_scenes_error,
            wheel_selected,
            wheel_name,
            wheel_slot_label,
            wheel_slot_shortcut,
            wheel_slot_selected: None,
            wheel_cell: None,
            wheel_preview: None,
            wheel_moving: false,
            wheel_replacing: false,
            wheel_scope: None,
            wheel_scope_open: false,
            wheel_hover: None,
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
            raishot_install_origin: None,
            install_status: relocated,
            legacy_gate: Default::default(),
            legacy_source,
            legacy_job: None,
            legacy_installing: false,
            legacy_stop_after_job: false,
            legacy_status: legacy_location_error,
            legacy_session_used: false,
            legacy_last_stop: None,
            legacy_close_requested: false,
            executor: CommandExecutor::new(),
            package_input,
            package_picker_for_draft: false,
            action_service: if mode.owns_input_engine() {
                action_packages::service(&settings::path())
            } else {
                omnitrigger_actions::ActionService::new(
                    omnitrigger_actions::PackageCatalog::default(),
                    omnitrigger_actions::RuntimeRegistry::new(),
                )
            },
            package_feedback: Default::default(),
            script_projects,
            shortcut_executor: CommandExecutor::shortcuts(),
            mouse_shortcut_invocations: HashMap::new(),
            mouse_shortcut_dispatch,
            mouse_shortcut_dispatch_error,
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
            error: startup_error
                .or(load_error)
                .or(title_bar.clone().err())
                .or(mouse_shortcut_dispatch_error.map(|error| error.message().to_owned())),
            action_error: None,
            binding_reload_notice: None,
            status: if mode.owns_input_engine() && !listen {
                "测试数据，未安装输入监听；需要验证真实输入时加 --listen 启动。".into()
            } else {
                mode.initial_status().into()
            },
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
        };
        if let Some(source) = workspace.legacy_source.as_ref().cloned() {
            if let Err(error) = workspace.accept_legacy_artifact_source(source, cx) {
                workspace.legacy_status = Some(error);
            }
        }
        workspace
    }

    fn listener(&self) -> Listener {
        if self.restarting {
            Listener::Rebuilding
        } else if self.backend.is_none() && !self.listen && self.mode.owns_input_engine() {
            Listener::Suspended
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
        if self.mode.history_only() {
            return;
        }
        if self.recording {
            return;
        }
        self.wheel_preview = None;
        if page != Page::TextHistory {
            self.text_history.cancel_send();
        }
        self.smooth_scroll.cancel_all();
        self.page = page;
        if page == Page::TextHistory {
            self.text_history.activate();
        }
        self.package_picker_for_draft = false;
        self.preset_edit_id = None;
        cx.notify();
    }

    fn rebuild_backend(&mut self, cx: &mut Context<Self>) {
        if self.legacy_close_requested {
            return;
        }
        if !self.mode.owns_input_engine() {
            self.status = self.mode.initial_status().into();
            cx.notify();
            return;
        }
        if !self.listen {
            self.status = "测试配置档未安装输入监听；需要验证真实输入时加 --listen 启动。".into();
            cx.notify();
            return;
        }
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
        let wheel_input_policy = self.wheel_input_policy;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    drop(previous);
                    let (backend, events) =
                        Backend::start_with_wheel_input_policy(wheel_input_policy)?;
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
        if self.legacy_close_requested {
            return;
        }
        if !self.mode.owns_input_engine() {
            return;
        }
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
            BackendCommand::SetBindings(_)
                | BackendCommand::SetEnabled(false)
                | BackendCommand::StartRecording
                | BackendCommand::Shutdown
        ) {
            self.cancel_owned_mouse_shortcuts();
        }
        if !self.mode.owns_input_engine() {
            return false;
        }
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
        if !self.mode.owns_input_engine() || !self.listen {
            self.bindings_need_sync = false;
            return;
        }
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
        if self
            .workspace_instance
            .as_ref()
            .is_some_and(|instance| instance.take_open_request())
        {
            let _ = self.main_window.update(cx, |_, window, _| {
                if let Ok(RawWindowHandle::Win32(handle)) =
                    HasWindowHandle::window_handle(&*window).map(|handle| handle.as_raw())
                {
                    omnitrigger_windows::show_workspace_window(handle.hwnd.get());
                }
                window.activate_window();
            });
        }
        if self.mode.history_only() {
            if self.text_history.poll() {
                cx.notify();
            }
            return;
        }
        if !self.panel_prepared {
            self.panel_prepared = true;
            if let Err(error) = self.prepare_panel(cx) {
                self.error = Some(error);
            }
        }
        let mut changed = self.poll_wheel(cx);
        changed |= self.text_history.poll();
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
                        let result = if self.executor.is_plugin_running("raishot") {
                            Err("请等待 RaiShot 动作结束后再更改插件设置。".into())
                        } else {
                            self.raishot_install_origin
                                .take()
                                .ok_or_else(|| {
                                    "找不到本次 RaiShot 安装的初始设置，请重新启用。".to_string()
                                })
                                .and_then(|origin| {
                                    origin
                                        .save_raishot_install(
                                            &settings::path(),
                                            cli_path.to_string_lossy().into_owned(),
                                        )
                                        .map_err(|error| error.to_string())
                                })
                        };
                        let saved = self.apply_raishot_settings(result, true, cx);
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
                    Ok(plugin_install::InstallEvent::LegacyInstallProgress(message)) => {
                        if !self.legacy_close_requested {
                            self.legacy_status = Some(if self.plugins.legacy_enabled() {
                                message
                            } else {
                                "插件已停用，安装会完成但不会启动宿主。".into()
                            });
                        }
                    }
                    Ok(plugin_install::InstallEvent::LegacyInstalled(result)) => {
                        // 先结束安装槽，再由既有连接点按当前enabled决定是否prepare。
                        finished = true;
                        self.finish_legacy_install(result, cx);
                    }
                    Ok(plugin_install::InstallEvent::LegacyFinished(event)) => {
                        self.finish_legacy_job(*event, cx);
                        finished = true;
                    }
                    Ok(plugin_install::InstallEvent::Failed(error)) => {
                        self.install_status = Some(format!("安装失败：{error}"));
                        finished = true;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        if !finished {
                            if self.legacy_installing {
                                self.finish_legacy_install(
                                    Err(omnitrigger_actions::ActionError::new(
                                        "legacy_install_worker_disconnected",
                                        "安装线程已结束，未确认制品完成。",
                                    )),
                                    cx,
                                );
                            } else if self.legacy_job.is_some() {
                                self.legacy_worker_disconnected(cx);
                            } else {
                                self.install_status =
                                    Some("安装线程已结束，未确认安装完成。".into());
                            }
                        }
                        finished = true;
                        break;
                    }
                }
                changed = true;
            }
            if !finished {
                self.install_events = Some(events);
            } else {
                self.raishot_install_origin = None;
            }
        }
        changed |= self.poll_browser_connector(cx);
        changed |= self.poll_update(cx);
        changed |= self.poll_packages();
        changed |= self.poll_script_editor();
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
                    self.text_history.cancel_send();
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
        while let Ok((from_shortcut_executor, event)) = self
            .executor
            .try_recv()
            .map(|event| (false, event))
            .or_else(|_| self.shortcut_executor.try_recv().map(|event| (true, event)))
        {
            changed = true;
            self.observe_mouse_shortcut_queue(from_shortcut_executor, &event);
            consume_owned_shortcut_receipt(
                &mut self.mouse_shortcut_invocations,
                from_shortcut_executor,
                &event,
            );
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
                BackendEvent::Recorded(spec) => {
                    let mut spec = *spec;
                    spec.conditions = self.recording_conditions.clone();
                    self.recording = false;
                    self.recording_held.clear();
                    let editing = self.recording_edit_id.take();
                    if let Some(base) = editing
                        .and_then(|id| self.binding(id).map(|binding| &binding.spec))
                        .or_else(|| {
                            self.recording_preserve_action
                                .then_some(self.pending.as_ref())
                                .flatten()
                        })
                    {
                        spec.process_scope = base.process_scope.clone();
                        spec.pointer_scope = base.pointer_scope.clone();
                        spec.keyboard = base.keyboard.clone();
                        if matches!(spec.activation, Activation::Wheel(_)) {
                            spec.wheel = base.wheel;
                        }
                    }
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
                BackendEvent::Triggered {
                    id,
                    pointer,
                    foreground,
                } => match omnitrigger_windows::capture_matching_shortcut_target(foreground) {
                    Ok(target) => self.fire_at(id, HitSource::Input, pointer, Some(target), cx),
                    Err(error) => {
                        if let Some((caps, message)) = self
                            .binding(id)
                            .filter(|binding| binding.enabled && !self.recording)
                            .map(|binding| (caps_for(&binding.spec), binding.message.clone()))
                        {
                            self.triggers.record_hit(id, clock());
                            self.record_hit(caps, message, HitSource::Input);
                            self.action_error = Some((id, error.clone()));
                            self.record_action_status(id, error);
                        }
                    }
                },
                BackendEvent::Drag { event, foreground } => {
                    self.handle_drag_event(event, foreground, cx)
                }
                BackendEvent::GestureChoice {
                    candidates,
                    pointer,
                    foreground,
                } => self.offer_gesture_choice(candidates, pointer, foreground, cx),
                BackendEvent::PointerPressed { x, y } => {
                    self.dismiss_gesture_choice_outside(x, y, cx);
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
                    self.close_gesture_choice(cx);
                    self.cancel_owned_mouse_shortcuts();
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
        changed |= self.pump_mouse_shortcuts();
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
            self.preview_gesture_choice(cx);
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
        if self.legacy_close_requested {
            return false;
        }
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
        self.action_running(id)
            || match action {
                ActionSpec::Plugin(reference) => self
                    .plugins
                    .concurrency_group(reference)
                    .is_some_and(|group| self.executor.is_group_running(&group)),
                _ => false,
            }
    }

    fn action_running(&self, id: u64) -> bool {
        self.executor.is_running(id)
            || self.shortcut_executor.is_running(id)
            || self.system_executor.is_running(id)
            || self.local_executor.is_running(id)
            || self.action_service.is_running(id)
            || self.browser_executor.is_running(id)
    }

    fn action_unavailable_reason(&self, action: &ActionSpec) -> Option<String> {
        match action {
            ActionSpec::Browser(action) => action.validate().err().or_else(|| {
                (!self.browser_connector_ui.installed())
                    .then(|| "请先在扩展页安装浏览器连接器。".into())
            }),
            ActionSpec::Invoke(call) => self.package_unavailable_reason(call),
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
        match spec.activation {
            Activation::Gesture { shape, .. } => self.gesture_draft.load(Some(shape)),
            _ => self.gesture_draft.load(None),
        }
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
            | Activation::Drag { button, .. }
            | Activation::Gesture { button, .. } => button,
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
        } else if self.trigger_mode == TriggerMode::Gesture {
            if !matches!(button, Button::Mouse(_)) {
                return Err("手势仅适用于鼠标按键，请选择鼠标预设或重新录制。".into());
            }
            Activation::Gesture {
                button,
                shape: self
                    .gesture_draft
                    .shape
                    .ok_or_else(|| "请先在面板中画出手势。".to_string())?,
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
        // 尚未绘制手势时只切换编辑面板；保存按钮在画出形状前不可用。
        let awaiting_gesture = mode == TriggerMode::Gesture && self.gesture_draft.shape.is_none();
        if let Some(base) = self.pending.clone().filter(|_| !awaiting_gesture) {
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
        self.text_history.cancel_send();
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
        self.fire_at(id, source, None, None, cx);
    }

    fn fire_at(
        &mut self,
        id: u64,
        source: HitSource,
        pointer: Option<(i32, i32)>,
        target: Option<omnitrigger_windows::ShortcutTarget>,
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
            if source == HitSource::Input && self.queue_owns_mouse_rule(id) {
                self.record_hit(caps, message, source);
                self.enqueue_mouse_shortcut(id, action, target, cx);
            } else if is_drag_wheel {
                // 同一次匹配的 Started/Ended 生命周期负责轮盘交互；这里仅记录命中。
                self.record_hit(caps, message, source);
            } else if matches!(action, ActionSpec::ShowPanel) {
                self.record_hit(caps, message, source);
                self.open_panel(panel_behavior, timing, cx);
            } else if !matches!(action, ActionSpec::Notification) {
                // 截图等动作可能立即采集屏幕，执行前不弹出自己的提示浮层。
                self.record_hit(caps, message, source);
                self.dispatch_action_context(id, action, target, pointer, cx);
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

    fn mouse_shortcut_frame(&self) -> (u64, omnitrigger_core::DispatchEnablement) {
        let now = self
            .started
            .elapsed()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX);
        let state = if !self.ready {
            omnitrigger_core::DispatchEnablement::Unknown
        } else if self.paused || self.recording {
            omnitrigger_core::DispatchEnablement::Disabled
        } else {
            omnitrigger_core::DispatchEnablement::Enabled
        };
        (now, state)
    }
    fn queue_owns_mouse_rule(&self, id: u64) -> bool {
        match &self.mouse_shortcut_dispatch {
            Some(queue) => queue.owns_binding(id),
            None => {
                self.mouse_shortcut_dispatch_error.is_some() && self.is_mouse_shortcut_binding(id)
            }
        }
    }
    fn mouse_queue_result(
        &mut self,
        result: Result<mouse_shortcut_dispatch::Report, mouse_shortcut_dispatch::Error>,
        admission_binding: Option<u64>,
    ) -> bool {
        let report = match result {
            Ok(report) => report,
            Err(error) => {
                self.mouse_shortcut_dispatch_error = Some(error);
                self.error = Some(error.message().into());
                return true;
            }
        };
        let changed = report.changed();
        if let (Some(id), Some(admission)) = (admission_binding, report.admission) {
            match admission {
                omnitrigger_core::MouseDispatchAdmission::Started(_) => {
                    self.record_action_status(id, "等待发送快捷键".into())
                }
                omnitrigger_core::MouseDispatchAdmission::Delayed { .. } => {
                    self.record_action_status(id, "等待前一项快捷键结束".into())
                }
                omnitrigger_core::MouseDispatchAdmission::Dropped(reason) => {
                    self.mouse_queue_drop(id, reason)
                }
            }
        }
        for dropped in report.dropped {
            self.mouse_queue_drop(dropped.request.binding_id, dropped.reason);
        }
        if let Some(lease) = report.submitted {
            self.mouse_shortcut_invocations
                .insert(lease.invocation, lease.binding);
            if self
                .action_error
                .as_ref()
                .is_some_and(|(id, _)| *id == lease.binding)
            {
                self.action_error = None;
            }
            self.record_action_status(lease.binding, "等待发送快捷键".into());
        }
        if let Some((lease, result)) = report.cancellation {
            match result {
                Ok(
                    execution::ShortcutCancelState::Requested
                    | execution::ShortcutCancelState::AlreadyRequested,
                ) => {
                    self.record_action_status(
                        lease.binding,
                        "已请求停止，等待快捷键发送收尾".into(),
                    );
                }
                Ok(_) => {
                    self.action_error =
                        Some((lease.binding, "取消尚未确认，仍等待原调用终态。".into()))
                }
                Err(message) => self.action_error = Some((lease.binding, message)),
            }
        }
        if let Some((id, message)) = report.rejected_before_start {
            self.action_error = Some((id, message.clone()));
            self.record_action_status(id, message);
        }
        changed
    }
    fn mouse_queue_drop(&mut self, id: u64, reason: omnitrigger_core::MouseDispatchDropReason) {
        use omnitrigger_core::MouseDispatchDropReason as R;
        let message = match reason {
            R::QueueFull => "鼠标快捷键队列已满，本次未提交。",
            R::Busy => "前一次鼠标快捷键尚未结束，本次未提交。",
            R::Expired => "鼠标快捷键等待已过期，本次未提交。",
            R::Disabled | R::EnablementUnknown => "鼠标快捷键当前未启用或状态未知，本次未提交。",
            R::Cancelled | R::ContextInvalidated => "本次鼠标快捷键等待项已取消，未提交。",
        }
        .to_owned();
        self.action_error = Some((id, message.clone()));
        self.record_action_status(id, message);
    }
    fn enqueue_mouse_shortcut(
        &mut self,
        id: u64,
        action: ActionSpec,
        target: Option<omnitrigger_windows::ShortcutTarget>,
        cx: &mut Context<Self>,
    ) {
        let prepared = (|| {
            if let Some(error) = self.mouse_shortcut_dispatch_error {
                return Err(error.message().to_owned());
            }
            if !self
                .binding(id)
                .is_some_and(mouse_shortcut_dispatch::supports_binding)
            {
                return Err("该规则已不属于已配置的鼠标快捷键范围，未改用其他投递路径。".into());
            }
            let ActionSpec::SendShortcut(shortcut) = action else {
                return Err("此调度只接收快捷键。".into());
            };
            let target =
                target.ok_or_else(|| "缺少本次输入的原目标，未改选当前前台。".to_owned())?;
            execution::PreparedShortcut::capture(shortcut, target)
        })();
        match prepared {
            Ok(prepared) => {
                let (now, state) = self.mouse_shortcut_frame();
                let executor = &self.shortcut_executor;
                let result = self
                    .mouse_shortcut_dispatch
                    .as_mut()
                    .ok_or(mouse_shortcut_dispatch::Error::InvalidBindings)
                    .and_then(|queue| queue.offer(id, prepared, executor, now, state));
                self.mouse_queue_result(result, Some(id));
                self.pump_mouse_shortcuts();
            }
            Err(message) => {
                self.action_error = Some((id, message.clone()));
                self.record_action_status(id, message);
            }
        }
        cx.notify();
    }
    fn pump_mouse_shortcuts(&mut self) -> bool {
        if self.mouse_shortcut_dispatch_error.is_some() {
            return false;
        }
        let (now, state) = self.mouse_shortcut_frame();
        let Some(queue) = &mut self.mouse_shortcut_dispatch else {
            return false;
        };
        let result = queue.pump_one(&self.shortcut_executor, now, state);
        self.mouse_queue_result(result, None)
    }
    fn observe_mouse_shortcut_queue(
        &mut self,
        from_shortcut: bool,
        event: &execution::ExecutionEvent,
    ) {
        let (now, state) = self.mouse_shortcut_frame();
        if let Some(queue) = &mut self.mouse_shortcut_dispatch {
            let result = queue.observe(from_shortcut, event, &self.shortcut_executor, now, state);
            self.mouse_queue_result(result, None);
        }
    }
    fn cancel_mouse_shortcut_rule(&mut self, id: u64) {
        let (now, state) = self.mouse_shortcut_frame();
        if let Some(queue) = &mut self.mouse_shortcut_dispatch {
            let result = queue.cancel_rule(id, &self.shortcut_executor, now, state);
            self.mouse_queue_result(result, None);
        }
        for (&invocation, &binding) in &self.mouse_shortcut_invocations {
            if binding == id
                && !self
                    .mouse_shortcut_dispatch
                    .as_ref()
                    .is_some_and(|queue| queue.owns_invocation(invocation))
            {
                if let Err(error) = self.shortcut_executor.cancel_shortcut(invocation) {
                    self.error = Some(error);
                }
            }
        }
    }

    fn cancel_owned_mouse_shortcuts(&mut self) {
        let (now, state) = self.mouse_shortcut_frame();
        if let Some(queue) = &mut self.mouse_shortcut_dispatch {
            let result = queue.cancel_all(&self.shortcut_executor, now, state);
            self.mouse_queue_result(result, None);
        }
        // 每个配额对应一个准确回执；同绑定允许多次调用，不能用binding作为取消键。
        for &invocation in self.mouse_shortcut_invocations.keys() {
            if self
                .mouse_shortcut_dispatch
                .as_ref()
                .is_some_and(|queue| queue.owns_invocation(invocation))
            {
                continue;
            }
            if let Err(error) = self.shortcut_executor.cancel_shortcut(invocation) {
                self.error = Some(error);
            }
        }
        // 取消不是终态；只在同一executor的准确终态被消费后移除回执。
    }
    fn is_mouse_shortcut_binding(&self, id: u64) -> bool {
        self.binding(id).is_some_and(|binding| {
            matches!(
                binding.spec.activation,
                Activation::Wheel(_)
                    | Activation::Press(omnitrigger_core::Button::Mouse(_))
                    | Activation::Hold {
                        button: omnitrigger_core::Button::Mouse(_),
                        ..
                    }
                    | Activation::Drag {
                        button: omnitrigger_core::Button::Mouse(_),
                        ..
                    }
                    | Activation::Gesture {
                        button: omnitrigger_core::Button::Mouse(_),
                        ..
                    }
            )
        })
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
        if self.legacy_close_requested {
            return;
        }
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
                let route = action_packages::clipboard_route(
                    self.test_data,
                    &call,
                    self.action_service.catalog(),
                );
                if matches!(route, Ok(action_packages::ClipboardRoute::WorkspaceHistory)) {
                    let snapshot = target.and_then(|target| {
                        desktop_host::DesktopHost::capture(Some(target)).choice_target()
                    });
                    script_ui::toggle_workspace_history(id, snapshot, cx);
                    return;
                }
                let submission = if let Err(error) = route {
                    Err(error)
                } else if let Some(reason) = self.legacy_unavailable_reason(&call) {
                    let mut error =
                        omnitrigger_actions::ActionError::new("legacy_unavailable", reason);
                    error.effect_state = Some("not_started".into());
                    Err(error)
                } else {
                    action_packages::submit(&self.action_service, id, call.clone(), target)
                };
                match submission {
                    Ok(invocation) => {
                        self.package_feedback.accepted(id, invocation, call);
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
                let result = if self.is_mouse_shortcut_binding(id) {
                    match target {
                        Some(target) => self
                            .shortcut_executor
                            .submit_shortcut_to_tracked(id, shortcut, target),
                        None => self.shortcut_executor.submit_shortcut_tracked(id, shortcut),
                    }
                    .map(Some)
                } else {
                    match target {
                        Some(target) => self
                            .shortcut_executor
                            .submit_shortcut_to(id, shortcut, target),
                        None => self.shortcut_executor.submit_shortcut(id, shortcut),
                    }
                    .map(|()| None)
                };
                match result {
                    Ok(invocation) => {
                        if let Some(invocation) = invocation {
                            self.mouse_shortcut_invocations.insert(invocation, id);
                        }
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
        if let ActionSpec::SendShortcut(existing) = &self.draft_action {
            return existing
                .edit_keys(&self.shortcut_keys.read(cx).text())
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
        if !self.mode.owns_input_engine() {
            return;
        }
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
            self.text_history.cancel_send();
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
        self.text_history.cancel_send();
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
                    context: Default::default(),
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
                self.binding_save_error(error);
            }
        }
        cx.notify();
    }

    /// 服务层已提交绑定变更后，清除旧错误并同步输入后端。
    fn applied(&mut self) {
        self.error = None;
        self.binding_reload_notice = None;
        self.sync_bindings();
    }

    fn binding_save_error(&mut self, error: TriggerError) {
        match error {
            TriggerError::ReloadRequired(message) => self.binding_reload_notice = Some(message),
            error => self.error = Some(error.message()),
        }
    }

    fn reload_bindings(&mut self, cx: &mut Context<Self>) {
        if self.recording {
            return;
        }
        let loaded = settings::reload_bindings_from(&settings::path())
            .and_then(|(bindings, scenes)| self.apply_disk_bindings(bindings, scenes, None, cx));
        match loaded {
            Ok(()) => self.status = "已载入磁盘上的热键配置。".into(),
            Err(error) => self.binding_reload_notice = Some(error),
        }
        cx.notify();
    }

    fn reload_wheel_and_binding_snapshot(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        let path = settings::path();
        // 首次启动时数据文件可以尚未生成。已有自定义绑定的窗口须等文件恢复。
        let allow_missing = self.triggers.bindings() == settings::defaults()
            && self.wheel_scenes == scenes::CompiledScenes::default();
        let snapshot = self.wheels.reload_with_bindings(&path, allow_missing)?;
        self.apply_disk_bindings(
            snapshot.bindings,
            snapshot.scenes,
            Some(snapshot.wheels),
            cx,
        )
    }

    fn apply_disk_bindings(
        &mut self,
        bindings: Vec<Binding>,
        scenes: scenes::CompiledScenes,
        wheels: Option<Wheels>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        for wheel in &wheels.as_ref().unwrap_or(&self.wheels).wheels {
            wheel_scenes::check_wheel(wheel, &scenes)
                .map_err(|error| format!("轮盘“{}”：{error}", wheel.name))?;
        }
        let from_wheels = wheels.is_some();
        let by_id: HashMap<_, _> = bindings
            .iter()
            .map(|binding| (binding.spec.id, binding))
            .collect();
        let mut retired: HashSet<_> = self
            .triggers
            .bindings()
            .iter()
            .filter(|old| by_id.get(&old.spec.id).copied() != Some(*old))
            .map(|old| old.spec.id)
            .collect();
        // 外部新增的绑定可能刚好用了本进程留给轮盘槽位或录制草稿的编号。
        retired.extend(
            self.wheel_runtime_ids
                .values()
                .filter(|id| by_id.contains_key(id))
                .copied(),
        );
        let selected_changed = self.selected.is_some_and(|id| retired.contains(&id));
        if from_wheels {
            let pending_collision = self
                .pending
                .as_ref()
                .is_some_and(|spec| by_id.contains_key(&spec.id));
            let edited_externally = self.pending.is_none()
                && selected_changed
                && self
                    .selected
                    .and_then(|id| self.binding(id))
                    .is_some_and(|binding| self.has_binding_edits(binding, cx));
            if pending_collision || edited_externally {
                let reason =
                    "外部修改与未保存的热键编辑冲突。请先返回触发器页处理草稿，再重新载入轮盘。";
                self.binding_reload_notice = Some(reason.into());
                return Err(reason.into());
            }
        } else {
            retired.extend(self.pending.as_ref().map(|spec| spec.id));
        }
        if retired.iter().any(|id| {
            self.action_running(*id)
                || self
                    .mouse_shortcut_invocations
                    .values()
                    .any(|binding| binding == id)
        }) {
            return Err("待替换的动作仍在运行，请在动作结束后重新载入。".to_owned());
        }
        if !from_wheels {
            self.hide_wheel(cx);
        }
        self.close_gesture_choice(cx);
        self.wheel_runtime_ids
            .retain(|_, id| !by_id.contains_key(id));
        self.wheel_source_binding = self
            .wheel_source_binding
            .filter(|id| by_id.contains_key(id));
        let selected = self
            .selected
            .filter(|id| by_id.contains_key(id))
            .or_else(|| bindings.first().map(|binding| binding.spec.id));
        let refresh_editor = !from_wheels
            || (self.pending.is_none() && (selected_changed || selected != self.selected));
        self.selected = selected;
        self.triggers.replace_bindings(bindings);
        self.wheel_scenes = scenes;
        self.wheel_scenes_error = None;
        if !from_wheels {
            self.clear_pending();
        }
        if refresh_editor {
            self.confirm_delete = None;
            self.conflict = None;
            self.preset_edit_id = None;
            self.load_selected_message(cx);
        }
        self.binding_reload_notice = None;
        for id in retired {
            self.cancel_mouse_shortcut_rule(id);
            self.action_status.remove(&id);
            if self
                .action_error
                .as_ref()
                .is_some_and(|(failed, _)| *failed == id)
            {
                self.action_error = None;
            }
        }
        if let Some(wheels) = wheels {
            self.replace_wheels(wheels, cx);
        }
        self.sync_bindings();
        self.refresh_panel(cx);
        Ok(())
    }

    fn has_binding_edits(&self, binding: &Binding, cx: &App) -> bool {
        self.message.read(cx).text() != binding.message
            || self.draft_spec(&binding.spec, cx).as_ref() != Ok(&binding.spec)
            || self.draft_action_spec(cx).as_ref() != Ok(&binding.action)
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
            self.text_history.cancel_send();
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
                self.text_history.cancel_send();
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
        if self.executor.is_plugin_running("raishot") {
            self.error = Some("请等待 RaiShot 动作结束后再更改插件设置。".into());
            cx.notify();
            return false;
        }
        let result = if let Some(error) = &self.plugin_settings_error {
            Err(error.clone())
        } else {
            Plugins::set_raishot_enabled(&settings::path(), enabled)
                .map_err(|error| error.to_string())
        };
        self.apply_raishot_settings(result, enabled, cx)
    }

    fn apply_raishot_settings(
        &mut self,
        result: Result<Plugins, String>,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        match result {
            Ok(latest) => self.plugins = latest,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return false;
            }
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
        let previous_legacy = self.legacy_source.as_ref().map(|source| source.key());
        self.connect_installed_legacy(cx);
        if self.legacy_source.as_ref().map(|source| source.key()) == previous_legacy {
            self.refresh_script_catalog();
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
                    self.raishot_install_origin = Some(self.plugins.clone());
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
            Err(error) => self.binding_save_error(error),
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
                self.cancel_mouse_shortcut_rule(id);
                self.applied();
                self.local_executor.cancel_id(id);
                self.action_service.cancel_id(id);
                self.action_status.remove(&id);
                self.confirm_delete = None;
                self.selected = next;
                self.status = "触发器已删除。".into();
                self.load_selected_message(cx);
            }
            Err(error) => self.binding_save_error(error),
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
            Listener::Suspended => (
                "未监听",
                "测试数据 · 未安装输入监听".into(),
                theme::TEXT_DIM,
            ),
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
            Listener::Suspended => small(button(
                "listener-action",
                "加 --listen 启动可监听",
                Tone::Secondary,
                false,
            )),
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
            .child(action)
            .when_some(self.test_profile.clone(), |card, name| {
                card.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .px(px(8.))
                        .py(px(5.))
                        .rounded(px(6.))
                        .bg(rgb(theme::RECORD_CAP))
                        .border_1()
                        .border_color(rgb(theme::RECORD_LINE))
                        .text_size(px(11.5))
                        .text_color(rgb(theme::RECORD_TEXT))
                        .child(dot(theme::RECORD, 6.))
                        .child(div().truncate().child(format!("测试数据：{name}"))),
                )
            });

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
                            .child(nav(
                                "nav-text-history",
                                Page::TextHistory,
                                Icon::Clock,
                                "文本历史",
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
            .binding_reload_notice
            .as_ref()
            .or(self.error.as_ref())
            .or_else(|| self.action_error.as_ref().map(|(_, error)| error))
        {
            let offline = self.listener() == Listener::Offline;
            return Some(
                banner(theme::DANGER, Icon::Alert)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.5))
                            .text_color(rgb(theme::DANGER_TEXT))
                            .child(if self.binding_reload_notice.is_some() {
                                format!("{error} 重新载入会放弃未保存的热键编辑。")
                            } else {
                                error.clone()
                            }),
                    )
                    .child(if self.binding_reload_notice.is_some() {
                        small(button(
                            "banner-reload-bindings",
                            "重新载入",
                            Tone::Secondary,
                            !self.recording,
                        ))
                        .on_click(cx.listener(|this, _, _, cx| this.reload_bindings(cx)))
                    } else if offline {
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

    /// 触发器页参与搜索与筛选的摘要，与绑定同序。
    fn trigger_rows(&self) -> Vec<trigger_browse::Row> {
        self.triggers
            .bindings()
            .iter()
            .map(|binding| {
                let ready = self.action_unavailable_reason(&binding.action).is_none();
                browse_row(binding, ready)
            })
            .collect()
    }

    fn apply_trigger_choice(&mut self, choice: TriggerChoice, cx: &mut Context<Self>) {
        match choice {
            TriggerChoice::Facet(facet, value) => self.trigger_filter.set(facet, value),
            TriggerChoice::Group(group) => self.trigger_group = group,
        }
        self.trigger_menu = None;
        cx.notify();
    }

    fn clear_trigger_filters(&mut self, cx: &mut Context<Self>) {
        self.trigger_filter = Default::default();
        self.trigger_menu = None;
        self.trigger_search
            .update(cx, |input, cx| input.set_text(String::new(), cx));
        cx.notify();
    }

    /// 工具栏中的一个下拉：按钮展开选项，点击选项后收起。
    fn trigger_dropdown(
        &self,
        menu: TriggerMenu,
        text: String,
        active: bool,
        align_right: bool,
        options: Vec<(String, Option<usize>, bool, TriggerChoice)>,
        cx: &mut Context<Self>,
    ) -> Div {
        let open = self.trigger_menu == Some(menu);
        let element = match menu {
            TriggerMenu::Facet(facet) => facet as u64,
            TriggerMenu::Group => 99,
        };
        let mut list = div()
            .absolute()
            .top(px(32.))
            .when(align_right, |el| el.right(px(0.)))
            .when(!align_right, |el| el.left(px(0.)))
            .min_w(px(176.))
            .p(px(4.))
            .rounded(px(8.))
            .border_1()
            .border_color(rgb(theme::BORDER_MID))
            .bg(rgb(theme::SURFACE))
            .shadow(theme::lift(0.45))
            .occlude()
            .flex()
            .flex_col()
            .gap(px(2.));
        for (index, (label, count, selected, choice)) in options.into_iter().enumerate() {
            list = list.child(
                div()
                    .id(("trigger-option", element * 100 + index as u64))
                    .h(px(28.))
                    .px(px(8.))
                    .rounded(px(5.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(12.))
                    .cursor_pointer()
                    .when(selected, |el| {
                        el.bg(rgb(theme::SURFACE_HI)).text_color(rgb(theme::ACCENT))
                    })
                    .hover(|style| style.bg(rgb(theme::SURFACE_HI)))
                    .child(div().flex_1().whitespace_nowrap().child(label))
                    .when_some(count, |el, count| {
                        el.child(
                            div()
                                .font_family(theme::MONO_FONT)
                                .text_size(px(11.))
                                .text_color(rgb(theme::TEXT_FAINT))
                                .child(count.to_string()),
                        )
                    })
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.apply_trigger_choice(choice, cx)),
                    ),
            );
        }
        div()
            .relative()
            .flex_none()
            .child(
                div()
                    .id(("trigger-menu", element))
                    .h(px(28.))
                    .px(px(10.))
                    .rounded(px(14.))
                    .border_1()
                    .border_color(if active || open {
                        rgb(theme::ACCENT_LINE)
                    } else {
                        rgb(theme::BORDER_MID)
                    })
                    .when(active, |el| el.bg(rgb(theme::ACCENT_BG)))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(12.))
                    .text_color(rgb(if active {
                        theme::ACCENT
                    } else {
                        theme::TEXT_MUTED
                    }))
                    .whitespace_nowrap()
                    .cursor_pointer()
                    .hover(|style| style.border_color(rgb(theme::BORDER_HI)))
                    .child(text)
                    .child(
                        div()
                            .text_size(px(9.))
                            .text_color(rgb(theme::TEXT_FAINT))
                            .child(if open { "▲" } else { "▼" }),
                    )
                    // 按下即切换并阻止冒泡，避免页面的“点击空白处收起”立刻把它关掉。
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.trigger_menu = if this.trigger_menu == Some(menu) {
                                None
                            } else {
                                Some(menu)
                            };
                            cx.notify();
                        }),
                    ),
            )
            .when(open, |el| el.child(gpui::deferred(list).with_priority(1)))
    }

    fn render_trigger_toolbar(
        &self,
        rows: &[trigger_browse::Row],
        tokens: &[String],
        shown: usize,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let focused = self.trigger_search.focus_handle(cx).is_focused(window);
        let search = div()
            .h(px(34.))
            .px(px(12.))
            .rounded(px(6.))
            .bg(rgb(theme::BACKGROUND))
            .border_1()
            .border_color(if focused {
                theme::hsla(theme::ACCENT, 0.7)
            } else {
                rgb(theme::BORDER_MID).into()
            })
            .flex()
            .items_center()
            .gap(px(8.))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    window.focus(&this.trigger_search.focus_handle(cx));
                }),
            )
            .child(icon(
                Icon::Search,
                14.,
                if focused {
                    theme::ACCENT
                } else {
                    theme::TEXT_FAINT
                },
            ))
            .child(div().flex_1().min_w_0().child(self.trigger_search.clone()));
        let mut facets = div().flex().flex_wrap().items_center().gap(px(6.));
        for facet in trigger_browse::Facet::ALL {
            let current = self.trigger_filter.get(facet);
            let mut options = vec![(
                "全部".to_string(),
                None,
                current.is_none(),
                TriggerChoice::Facet(facet, None),
            )];
            for value in facet.options() {
                let count = trigger_browse::facet_count(rows, tokens, &self.trigger_filter, value);
                // 没有命中的选项不列出，避免点出空列表；已选中的始终保留以便取消。
                if count > 0 || current == Some(value) {
                    options.push((
                        value.label().to_string(),
                        Some(count),
                        current == Some(value),
                        TriggerChoice::Facet(facet, Some(value)),
                    ));
                }
            }
            facets = facets.child(self.trigger_dropdown(
                TriggerMenu::Facet(facet),
                format!(
                    "{}：{}",
                    facet.label(),
                    current.map_or("全部", |value| value.label())
                ),
                current.is_some(),
                false,
                options,
                cx,
            ));
        }
        if !tokens.is_empty() || !self.trigger_filter.is_empty() {
            facets = facets.child(
                div()
                    .id("trigger-clear-filters")
                    .px(px(6.))
                    .text_size(px(12.))
                    .text_color(rgb(theme::TEXT_DIM))
                    .cursor_pointer()
                    .hover(|style| style.text_color(rgb(theme::TEXT)))
                    .child("清除筛选")
                    .on_click(cx.listener(|this, _, _, cx| this.clear_trigger_filters(cx))),
            );
        }
        let enabled = rows
            .iter()
            .filter(|row| row.status != trigger_browse::StatusKind::Disabled)
            .count();
        let group_options = trigger_browse::GroupBy::ALL
            .into_iter()
            .map(|group| {
                (
                    group.label().to_string(),
                    None,
                    group == self.trigger_group,
                    TriggerChoice::Group(group),
                )
            })
            .collect();
        facets = facets.child(div().flex_1()).child(
            div()
                .flex_none()
                .font_family(theme::MONO_FONT)
                .text_size(px(11.5))
                .text_color(rgb(theme::TEXT_FAINT))
                .child(format!(
                    "{shown} / {} 条 · {enabled} 启用 · 本次 {}",
                    rows.len(),
                    thousands(self.trigger_count)
                )),
        );
        if self.trigger_view == TriggerView::List {
            facets = facets.child(self.trigger_dropdown(
                TriggerMenu::Group,
                self.trigger_group.label().to_string(),
                false,
                true,
                group_options,
                cx,
            ));
        }
        let view = self.trigger_view;
        let tab = |id: &'static str, label: &'static str, value: TriggerView| {
            segment(id, label, view == value).on_click(cx.listener(move |this, _, _, cx| {
                this.trigger_view = value;
                this.trigger_menu = None;
                cx.notify();
            }))
        };
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(search.flex_1().min_w_0())
                    .child(
                        theme::segments()
                            .flex_none()
                            .child(tab("trigger-view-list", "列表", TriggerView::List))
                            .child(tab("trigger-view-keyboard", "键盘", TriggerView::Keyboard))
                            .child(tab("trigger-view-mouse", "鼠标", TriggerView::Mouse)),
                    ),
            )
            .child(facets)
    }

    fn render_trigger_row(
        &self,
        binding: &settings::Binding,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let id = binding.spec.id;
        let selected = self.pending.is_none() && !self.recording && self.selected == Some(id);
        let stats = self.triggers.stats(id);
        let (glyph, tint) = action_look(&binding.action);
        let (detail, detail_color) =
            row_detail(binding, self.action_unavailable_reason(&binding.action));
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
                                    .when_some(badge, |el, badge| el.child(badge.flex_none())),
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
            )
    }

    fn render_group_header(
        &self,
        index: usize,
        group: &trigger_browse::Group,
        collapse_key: String,
        collapsed: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let prefix = (self.trigger_group == trigger_browse::GroupBy::Prefix
            && !group.key.is_empty())
        .then(|| {
            group
                .ids
                .first()
                .and_then(|id| self.triggers.bindings().iter().find(|b| b.spec.id == *id))
                .map(|binding| prefix_caps(&binding.spec))
        })
        .flatten();
        let count = if group.disabled > 0 {
            format!("{} · {} 停用", group.ids.len(), group.disabled)
        } else {
            group.ids.len().to_string()
        };
        div()
            .id(("trigger-group", index as u64))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(4.))
            .pt(px(if index == 0 { 2. } else { 12. }))
            .pb(px(4.))
            .cursor_pointer()
            .child(icon(
                if collapsed {
                    Icon::ArrowRight
                } else {
                    Icon::ArrowDown
                },
                10.,
                theme::TEXT_FAINT,
            ))
            .child(match prefix {
                Some(caps) => combo(&caps, CapSize::Small).flex_none().flex_nowrap(),
                None => div()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::TEXT_MUTED))
                    .child(group.label.clone()),
            })
            .child(
                div()
                    .font_family(theme::MONO_FONT)
                    .text_size(px(11.))
                    .text_color(rgb(theme::TEXT_FAINT))
                    .child(count),
            )
            .child(div().flex_1().h(px(1.)).bg(rgb(theme::DIVIDER)))
            .on_click(cx.listener(move |this, _, _, cx| {
                if !this.trigger_collapsed.remove(&collapse_key) {
                    this.trigger_collapsed.insert(collapse_key.clone());
                }
                cx.notify();
            }))
    }

    /// 列表左侧的分组导航，顺序与列表一致。
    fn render_group_nav(
        &self,
        groups: &[trigger_browse::Group],
        headers: &[usize],
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let keys: Vec<String> = groups
            .iter()
            .map(|group| format!("{:?}:{}", self.trigger_group, group.key))
            .collect();
        let current = self
            .trigger_nav_pick
            .as_ref()
            .and_then(|pick| keys.iter().position(|key| key == pick))
            .unwrap_or_else(|| {
                let top = self.trigger_list_scroll.top_item();
                headers
                    .iter()
                    .rposition(|&header| header <= top)
                    .unwrap_or(0)
            });
        let mut nav = div()
            .id("trigger-nav")
            .flex_none()
            .w(px(168.))
            .h_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(2.))
            .pr(px(10.))
            .border_r_1()
            .border_color(rgb(theme::DIVIDER))
            .child(
                div()
                    .px(px(10.))
                    .pt(px(2.))
                    .pb(px(6.))
                    .text_size(px(11.))
                    .text_color(rgb(theme::TEXT_FAINT))
                    .child("分组"),
            );
        for (index, group) in groups.iter().enumerate() {
            // 紧跟上层前缀组时，共有的键淡色显示，突出新增的键。
            let (lead, rest) = if group.inherited > 0 {
                let mut keys = group.label.splitn(group.inherited + 1, " + ");
                let lead: String = keys
                    .by_ref()
                    .take(group.inherited)
                    .map(|key| format!("{key} + "))
                    .collect();
                (lead, keys.next().unwrap_or_default().to_string())
            } else {
                (String::new(), group.label.clone())
            };
            let active = index == current;
            let all_disabled = group.disabled == group.ids.len();
            let key = keys[index].clone();
            let header = headers.get(index).copied().unwrap_or(0);
            nav = nav.child(
                div()
                    .id(("trigger-nav-item", index))
                    .flex_none()
                    .relative()
                    .h(px(30.))
                    .px(px(10.))
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(12.5))
                    .cursor_pointer()
                    .text_color(rgb(if active {
                        theme::TEXT
                    } else if all_disabled {
                        theme::TEXT_FAINT
                    } else {
                        theme::TEXT_DIM
                    }))
                    .when(active, |el| {
                        el.bg(rgb(theme::SURFACE_HI))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(
                                div()
                                    .absolute()
                                    .left(px(0.))
                                    .top(px(8.))
                                    .bottom(px(8.))
                                    .w(px(2.))
                                    .rounded(px(2.))
                                    .bg(rgb(theme::ACCENT)),
                            )
                    })
                    .when(!active, |el| el.hover(|s| s.bg(rgb(theme::ROW))))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .when(!lead.is_empty(), |el| {
                                el.child(
                                    div()
                                        .flex_none()
                                        .font_weight(FontWeight::NORMAL)
                                        .text_color(rgb(theme::TEXT_FAINT))
                                        .whitespace_nowrap()
                                        .child(lead),
                                )
                            })
                            .child(div().flex_1().min_w_0().truncate().child(rest)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .font_family(theme::MONO_FONT)
                            .text_size(px(11.))
                            .text_color(rgb(theme::TEXT_FAINT))
                            .child(group.ids.len().to_string()),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.trigger_collapsed.remove(&key);
                        this.smooth_scroll.cancel("trigger-rows");
                        this.trigger_list_scroll.scroll_to_top_of_item(header);
                        this.trigger_nav_pick = Some(key.clone());
                        cx.notify();
                    })),
            );
        }
        nav
    }

    /// 按 id 取绑定并画成列表行。
    fn trigger_rows_for(&self, ids: &[u64], cx: &mut Context<Self>) -> Vec<AnyElement> {
        ids.iter()
            .filter_map(|id| self.triggers.bindings().iter().find(|b| b.spec.id == *id))
            .map(|binding| self.render_trigger_row(binding, cx).into_any_element())
            .collect()
    }

    fn input_view_heading(title: String, count: usize) -> Div {
        div()
            .flex_none()
            .pt(px(14.))
            .pb(px(4.))
            .px(px(4.))
            .flex()
            .items_center()
            .gap(px(8.))
            .child(
                div()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::TEXT_MUTED))
                    .child(title),
            )
            .child(
                div()
                    .font_family(theme::MONO_FONT)
                    .text_size(px(11.))
                    .text_color(rgb(theme::TEXT_FAINT))
                    .child(count.to_string()),
            )
            .child(div().flex_1().h(px(1.)).bg(rgb(theme::DIVIDER)))
    }

    fn input_view_panel() -> Div {
        div()
            .flex_none()
            .p(px(14.))
            .rounded(px(10.))
            .bg(rgb(theme::SIDEBAR))
            .border_1()
            .border_color(rgb(theme::DIVIDER))
    }

    /// 键盘视图：选择前缀层，键盘只标出按键，点击按键后在下方列出触发器。
    fn render_keyboard_view(
        &self,
        visible: &[&trigger_browse::Row],
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        // 键宽按面板可用宽度计算；首帧尚未测得时用默认值。
        let measured = self.trigger_board_width.get();
        let unit = if measured > 0. {
            (measured / trigger_input_map::KEYBOARD_WIDTH).clamp(22., 40.)
        } else {
            30.
        };
        let width = self.trigger_board_width.clone();
        let probe = gpui::canvas(
            move |bounds, window, _| {
                let available = f32::from(bounds.size.width);
                if (available - width.get()).abs() > 1. {
                    width.set(available);
                    window.refresh();
                }
            },
            |_, _, _, _| {},
        )
        .w_full()
        .h(px(0.));
        let layers = trigger_input_map::key_layers(visible);
        let layer = self
            .trigger_layer
            .as_deref()
            .filter(|layer| layers.iter().any(|(name, _)| name == layer));
        let view = trigger_input_map::keyboard_view(visible, layer, self.trigger_key);
        let total: usize = layers.iter().map(|(_, count)| count).sum();

        let chip = |id: u64, text: String, count: usize, active: bool| {
            div()
                .id(("trigger-layer", id))
                .h(px(28.))
                .px(px(10.))
                .rounded(px(6.))
                .border_1()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(12.))
                .whitespace_nowrap()
                .cursor_pointer()
                .when(active, |el| {
                    el.bg(rgb(theme::RECORD_CAP))
                        .border_color(rgb(theme::RECORD_LINE))
                        .text_color(rgb(theme::RECORD))
                })
                .when(!active, |el| {
                    el.bg(rgb(theme::SURFACE))
                        .border_color(rgb(theme::BORDER))
                        .text_color(rgb(theme::TEXT_MUTED))
                        .hover(|style| style.border_color(rgb(theme::BORDER_HI)))
                })
                .child(text)
                .child(
                    div()
                        .font_family(theme::MONO_FONT)
                        .text_size(px(11.))
                        .opacity(0.75)
                        .child(count.to_string()),
                )
        };
        let mut bar = div()
            .flex_none()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(6.))
            .pb(px(10.))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(theme::TEXT_FAINT))
                    .child("按住"),
            )
            .child(
                chip(0, "全部层".into(), total, layer.is_none()).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.trigger_layer = None;
                        this.trigger_key = None;
                        cx.notify();
                    },
                )),
            );
        for (index, (name, count)) in layers.iter().enumerate() {
            let name = name.to_string();
            let text = if name.is_empty() {
                "无前缀".to_string()
            } else {
                name.clone()
            };
            bar = bar.child(
                chip(index as u64 + 1, text, *count, layer == Some(name.as_str())).on_click(
                    cx.listener(move |this, _, _, cx| {
                        this.trigger_layer = Some(name.clone());
                        this.trigger_key = None;
                        cx.notify();
                    }),
                ),
            );
        }

        let mut board = div()
            .relative()
            .flex_none()
            .w(px(unit * trigger_input_map::KEYBOARD_WIDTH))
            .h(px(unit * trigger_input_map::KEYBOARD_HEIGHT));
        for slot in trigger_input_map::KEYBOARD {
            let vk = slot.vk;
            let held = view.held.contains(&vk);
            let bound = view.bound.contains(&vk);
            let disabled_only = view.bound_disabled.contains(&vk);
            let picked = self.trigger_key == Some(vk);
            let (bg, border, text) = if held {
                (
                    rgb(theme::RECORD_CAP).into(),
                    rgb(theme::RECORD_LINE).into(),
                    theme::RECORD,
                )
            } else if bound && !disabled_only {
                (
                    hsla(theme::ACCENT, 0.3),
                    hsla(theme::ACCENT, 0.6),
                    theme::TEXT,
                )
            } else if bound {
                (
                    rgb(theme::SURFACE_HI).into(),
                    rgb(theme::BORDER_HI).into(),
                    theme::TEXT_MUTED,
                )
            } else {
                (
                    rgb(theme::KEYCAP).into(),
                    rgb(theme::KEYCAP_EDGE).into(),
                    theme::TEXT_FAINT,
                )
            };
            board = board.child(
                div()
                    .id(("trigger-key", vk as u64))
                    .absolute()
                    .left(px(slot.x * unit))
                    .top(px(slot.y * unit))
                    .w(px(slot.w * unit - 3.))
                    .h(px(unit - 3.))
                    .rounded(px(5.))
                    .bg(bg)
                    .border_1()
                    .border_b_2()
                    .border_color(if picked {
                        rgb(theme::ACCENT).into()
                    } else {
                        border
                    })
                    .when(picked, |el| el.border_2())
                    .when(layer.is_some() && !held && !bound, |el| el.opacity(0.45))
                    .flex()
                    .items_center()
                    .justify_center()
                    .overflow_hidden()
                    .font_family(theme::MONO_FONT)
                    .text_size(px(if slot.label.len() > 3 {
                        (unit * 0.3).min(11.)
                    } else {
                        (unit * 0.36).min(13.)
                    }))
                    .text_color(rgb(text))
                    .when(held, |el| el.font_weight(FontWeight::SEMIBOLD))
                    .child(slot.label)
                    .when(bound, |el| {
                        el.cursor_pointer()
                            .hover(|style| style.border_color(rgb(theme::ACCENT)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.trigger_key = if this.trigger_key == Some(vk) {
                                    None
                                } else {
                                    Some(vk)
                                };
                                cx.notify();
                            }))
                    }),
            );
        }
        let legend = |color: gpui::Hsla, border: gpui::Hsla, text: &'static str| {
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .child(
                    div()
                        .size(px(11.))
                        .rounded(px(3.))
                        .bg(color)
                        .border_1()
                        .border_color(border),
                )
                .child(text)
        };
        let panel = Self::input_view_panel()
            .id("trigger-keyboard")
            .overflow_x_scroll()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(probe)
            .child(board)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(14.))
                    .text_size(px(11.))
                    .text_color(rgb(theme::TEXT_FAINT))
                    .child(legend(
                        rgb(theme::RECORD_CAP).into(),
                        rgb(theme::RECORD_LINE).into(),
                        "按住的前缀",
                    ))
                    .child(legend(
                        hsla(theme::ACCENT, 0.3),
                        hsla(theme::ACCENT, 0.6),
                        "已绑定",
                    ))
                    .child(legend(
                        rgb(theme::SURFACE_HI).into(),
                        rgb(theme::BORDER_HI).into(),
                        "仅有停用绑定",
                    ))
                    .child("点击已绑定的键查看触发器"),
            );

        let picked_label = self.trigger_key.and_then(|vk| {
            trigger_input_map::KEYBOARD
                .iter()
                .find(|slot| slot.vk == vk)
                .map(|slot| slot.label)
        });
        let title = match (picked_label, layer) {
            (Some(key), _) => format!("目标键 {key}"),
            (None, None) => "所有以键盘按键结尾的触发器".into(),
            (None, Some("")) => "无前缀".into(),
            (None, Some(layer)) => format!("按住 {layer} 时"),
        };
        let mut elements = vec![
            bar.into_any_element(),
            panel.into_any_element(),
            Self::input_view_heading(title, view.ids.len()).into_any_element(),
        ];
        if view.off_layout > 0 && self.trigger_key.is_none() {
            elements.push(
                caption(format!(
                    "其中 {} 条的目标键不在上方布局中，例如小键盘或媒体键。",
                    view.off_layout
                ))
                .px(px(4.))
                .pb(px(4.))
                .into_any_element(),
            );
        }
        elements.extend(self.trigger_rows_for(&view.ids, cx));
        elements
    }

    /// 鼠标视图：标出各部位用作目标或前缀的条数，点击部位后在下方列出触发器。
    fn render_mouse_view(
        &self,
        visible: &[&trigger_browse::Row],
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        use trigger_input_map::MousePart;
        // 机身左上角与尺寸；其余坐标都相对整幅图。
        const MX: f32 = 115.;
        const MW: f32 = 170.;
        let picked = self.trigger_mouse;
        let count = |part| trigger_input_map::part_count(visible, part);
        let select = |part: MousePart| {
            cx.listener(move |this, _, _, cx| {
                this.trigger_mouse = if this.trigger_mouse == Some(part) {
                    None
                } else {
                    Some(part)
                };
                cx.notify();
            })
        };
        let fill = |part: MousePart| -> gpui::Hsla {
            match count(part).total() {
                0 => rgb(theme::SURFACE_HI).into(),
                n => hsla(theme::ACCENT, (0.16 + n as f32 * 0.06).min(0.5)),
            }
        };
        let zone = |part: MousePart, id: u64| {
            div()
                .id(("mouse-part", id))
                .absolute()
                .bg(fill(part))
                .border_1()
                .border_color(if picked == Some(part) {
                    rgb(theme::ACCENT).into()
                } else {
                    hsla(0xffffff, 0.06)
                })
                .when(picked == Some(part), |el| el.border_2())
                .occlude()
                .cursor_pointer()
                .hover(|style| style.border_color(hsla(theme::ACCENT, 0.7)))
                .on_click(select(part))
        };
        let wheel_fill = |part: MousePart| -> gpui::Hsla {
            if count(part).total() > 0 {
                hsla(theme::ACCENT, 0.55)
            } else {
                rgb(theme::BORDER_MID).into()
            }
        };
        let mut drawing = div()
            .relative()
            .flex_none()
            .w(px(410.))
            .h(px(266.))
            // 机身
            .child(
                div()
                    .absolute()
                    .left(px(MX))
                    .top(px(0.))
                    .w(px(MW))
                    .h(px(262.))
                    .bg(rgb(theme::KEYCAP_BASE))
                    .border_1()
                    .border_color(rgb(theme::BORDER_MID))
                    .rounded_tl(px(84.))
                    .rounded_tr(px(84.))
                    .rounded_bl(px(76.))
                    .rounded_br(px(76.))
                    .shadow(theme::lift(0.35)),
            )
            .child(
                zone(MousePart::Left, 0)
                    .left(px(MX + 2.))
                    .top(px(2.))
                    .w(px(MW / 2. - 4.))
                    .h(px(110.))
                    .rounded_tl(px(82.))
                    .rounded_tr(px(3.))
                    .rounded_bl(px(3.))
                    .rounded_br(px(3.)),
            )
            .child(
                zone(MousePart::Right, 1)
                    .left(px(MX + MW / 2. + 2.))
                    .top(px(2.))
                    .w(px(MW / 2. - 4.))
                    .h(px(110.))
                    .rounded_tr(px(82.))
                    .rounded_tl(px(3.))
                    .rounded_bl(px(3.))
                    .rounded_br(px(3.)),
            )
            .child(
                zone(MousePart::Middle, 2)
                    .left(px(MX + MW / 2. - 14.))
                    .top(px(22.))
                    .w(px(28.))
                    .h(px(66.))
                    .rounded(px(14.))
                    .when(count(MousePart::Middle).total() == 0, |el| {
                        el.bg(rgb(theme::SIDEBAR))
                    }),
            )
            .child(
                zone(MousePart::WheelUp, 3)
                    .left(px(MX + MW / 2. - 9.))
                    .top(px(27.))
                    .w(px(18.))
                    .h(px(27.))
                    .rounded_t(px(9.))
                    .bg(wheel_fill(MousePart::WheelUp)),
            )
            .child(
                zone(MousePart::WheelDown, 4)
                    .left(px(MX + MW / 2. - 9.))
                    .top(px(56.))
                    .w(px(18.))
                    .h(px(27.))
                    .rounded_b(px(9.))
                    .bg(wheel_fill(MousePart::WheelDown)),
            )
            .child(
                zone(MousePart::X2, 5)
                    .left(px(MX - 6.))
                    .top(px(128.))
                    .w(px(10.))
                    .h(px(34.))
                    .rounded(px(4.)),
            )
            .child(
                zone(MousePart::X1, 6)
                    .left(px(MX - 6.))
                    .top(px(168.))
                    .w(px(10.))
                    .h(px(34.))
                    .rounded(px(4.)),
            );
        // 引线标注：部位、标注所在 y、锚点 (x, y)、是否在左侧。
        let callouts = [
            (MousePart::Left, 56., (MX + 40., 56.), true),
            (MousePart::X2, 145., (MX - 6., 145.), true),
            (MousePart::X1, 185., (MX - 6., 185.), true),
            (MousePart::Middle, 12., (MX + MW / 2., 22.), false),
            (MousePart::WheelUp, 40., (MX + MW / 2. + 9., 40.), false),
            (MousePart::WheelDown, 70., (MX + MW / 2. + 9., 70.), false),
            (MousePart::Right, 100., (MX + 130., 100.), false),
        ];
        for (index, (part, y, (ax, ay), left)) in callouts.into_iter().enumerate() {
            let n = count(part).total();
            let active = picked == Some(part);
            let line = if active {
                rgb(theme::ACCENT).into()
            } else if n > 0 {
                hsla(theme::ACCENT, 0.45)
            } else {
                rgb(theme::BORDER_MID).into()
            };
            let (from, to) = if left { (96., ax) } else { (ax, 312.) };
            drawing = drawing
                .child(
                    div()
                        .absolute()
                        .left(px(from))
                        .top(px(y))
                        .w(px(to - from))
                        .h(px(1.))
                        .bg(line),
                )
                .when(ay != y, |el| {
                    el.child(
                        div()
                            .absolute()
                            .left(px(ax))
                            .top(px(y.min(ay)))
                            .w(px(1.))
                            .h(px((ay - y).abs()))
                            .bg(line),
                    )
                })
                .child(
                    div()
                        .absolute()
                        .left(px(ax - 2.5))
                        .top(px(ay - 2.5))
                        .size(px(5.))
                        .rounded(px(3.))
                        .bg(line),
                )
                .child(
                    div()
                        .id(("mouse-callout", index as u64))
                        .absolute()
                        .top(px(y - 10.))
                        .h(px(20.))
                        .when(left, |el| el.left(px(0.)).w(px(92.)).justify_end())
                        .when(!left, |el| el.left(px(316.)).w(px(94.)))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_size(px(12.))
                        .whitespace_nowrap()
                        .cursor_pointer()
                        .text_color(rgb(if active {
                            theme::ACCENT
                        } else if n > 0 {
                            theme::TEXT_SOFT
                        } else {
                            theme::TEXT_FAINT
                        }))
                        .child(part.label())
                        .when(n > 0, |el| {
                            el.child(
                                div()
                                    .font_family(theme::MONO_FONT)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(theme::ACCENT))
                                    .child(n.to_string()),
                            )
                        })
                        .on_click(select(part)),
                );
        }
        // 图上没有位置的部位，有绑定时以按钮列出。
        let mut extras = div().flex().flex_wrap().gap(px(6.));
        for (index, part) in [MousePart::WheelSide, MousePart::Region]
            .into_iter()
            .enumerate()
        {
            let n = count(part).total();
            if n == 0 && picked != Some(part) {
                continue;
            }
            extras = extras.child(
                segment(
                    ("mouse-extra", index as u64),
                    format!("{} {n}", part.label()),
                    picked == Some(part),
                )
                .on_click(select(part)),
            );
        }
        let panel = Self::input_view_panel()
            .id("trigger-mouse")
            .overflow_x_scroll()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(12.))
            .child(drawing)
            .child(extras);
        let ids = trigger_input_map::mouse_ids(visible, picked);
        let title = match picked {
            Some(part) => {
                let count = count(part);
                format!(
                    "{}：作目标 {} · 作前缀 {}",
                    part.label(),
                    count.terminal,
                    count.prefix
                )
            }
            None => "所有用到鼠标、滚轮或屏幕区域的触发器".into(),
        };
        let mut elements = vec![
            panel.into_any_element(),
            Self::input_view_heading(title, ids.len()).into_any_element(),
        ];
        elements.extend(self.trigger_rows_for(&ids, cx));
        elements
    }

    fn render_list(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let listening = self.listener() == Listener::Listening;
        let dim = matches!(self.listener(), Listener::Paused | Listener::Offline);
        let summary = self.trigger_rows();
        let tokens = trigger_browse::tokens(&self.trigger_search.read(cx).text());
        let groups =
            trigger_browse::browse(&summary, &tokens, &self.trigger_filter, self.trigger_group);
        let shown: usize = groups.iter().map(|group| group.ids.len()).sum();
        let grouped = self.trigger_group != trigger_browse::GroupBy::None;
        let mut rows = div()
            .id("trigger-rows")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(2.))
            .when(dim, |el| el.opacity(0.6));
        let mut nav = None;
        match self.trigger_view {
            TriggerView::List => {
                // 各组标题在滚动容器中的子元素下标，供导航跳转。
                let mut headers = Vec::with_capacity(groups.len());
                let mut child = 0;
                for (index, group) in groups.iter().enumerate() {
                    let collapse_key = format!("{:?}:{}", self.trigger_group, group.key);
                    let collapsed = grouped && self.trigger_collapsed.contains(&collapse_key);
                    if grouped {
                        headers.push(child);
                        child += 1;
                        rows = rows.child(self.render_group_header(
                            index,
                            group,
                            collapse_key,
                            collapsed,
                            cx,
                        ));
                    }
                    if collapsed {
                        continue;
                    }
                    for id in &group.ids {
                        if let Some(binding) =
                            self.triggers.bindings().iter().find(|b| b.spec.id == *id)
                        {
                            child += 1;
                            rows = rows.child(self.render_trigger_row(binding, cx));
                        }
                    }
                }
                if grouped && groups.len() > 1 {
                    nav = Some(self.render_group_nav(&groups, &headers, cx));
                }
            }
            TriggerView::Keyboard if shown > 0 => {
                let visible = trigger_browse::visible(&summary, &tokens, &self.trigger_filter);
                rows = rows.children(self.render_keyboard_view(&visible, cx));
            }
            TriggerView::Mouse if shown > 0 => {
                let visible = trigger_browse::visible(&summary, &tokens, &self.trigger_filter);
                rows = rows.children(self.render_mouse_view(&visible, cx));
            }
            _ => {}
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
        } else if shown == 0 {
            rows = rows.child(
                div()
                    .py(px(56.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(12.))
                    .child(icon_tile(Icon::Search, theme::ACCENT_2, 48.))
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(theme::TEXT_SOFT))
                            .child("没有符合条件的触发器"),
                    )
                    .child(
                        icon_button("clear-filters", Icon::Close, "清除筛选", Tone::Ghost, true)
                            .on_click(cx.listener(|this, _, _, cx| this.clear_trigger_filters(cx))),
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
            .pr(px(18.))
            .flex()
            .flex_col()
            .gap(px(14.))
            // 点击页面其他位置收起展开的下拉；下拉本身遮挡下层，不会触发这里。
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.trigger_menu.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .child(
                div()
                    .flex()
                    .items_end()
                    .justify_between()
                    .gap(px(12.))
                    .child(heading(
                        "触发器",
                        "按住前缀再按目标键或滚轮，支持应用范围",
                    ))
                    .child(
                        icon_button("record", Icon::Record, "录制组合", Tone::Primary, listening)
                            .on_click(cx.listener(|this, _, _, cx| this.start_recording(cx))),
                    ),
            )
            .child(self.render_trigger_toolbar(&summary, &tokens, shown, window, cx))
            .children(self.render_banner(cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .gap(px(14.))
                    .children(nav)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .on_scroll_wheel(cx.listener(|this, _, _, cx| {
                                if this.trigger_nav_pick.take().is_some() {
                                    cx.notify();
                                }
                            }))
                            .child(self.smooth_scroll.wrap_with_handle(
                                "trigger-rows",
                                rows,
                                &self.trigger_list_scroll,
                            )),
                    ),
            )
            .child(caption(format!(
                "最多 {} 条触发器，每条最多 8 个前缀。单独按下未用于组合的前缀时，原输入会补发给应用。",
                omnitrigger_core::MAX_BINDINGS,
            )))
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
                    this.text_history.cancel_send();
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
        // 草稿沿用已保存绑定的场景与集合引用；范围被改写时标签自动回到实际名称。
        let context = self
            .binding(spec.id)
            .map(|binding| binding.context.clone())
            .unwrap_or_default();
        let scene = context.authored(spec).0.map(str::to_string);
        let sources = context.scopes;
        let fields = div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(section_title("触发方式"))
            .when_some(scene, |element, name| {
                element.child(caption(format!(
                    "场景：{name}，生效范围如下（配置文件中设置）"
                )))
            })
            .when(!spec.pointer_scope.is_global(), |element| {
                element.child(caption(format!(
                    "指针窗口范围：{}（配置文件中设置）",
                    settings::process_scope_detail(&spec.pointer_scope, sources.pointer.as_ref())
                )))
            })
            .when(!spec.process_scope.is_global(), |element| {
                element.child(caption(format!(
                    "应用范围：{}（配置文件中设置）",
                    settings::process_scope_detail(&spec.process_scope, sources.process.as_ref())
                )))
            })
            .when(spec.keyboard.matching.is_mouse_exact(), |element| {
                element.child(caption(if spec.held.is_empty() {
                    "鼠标精确观察：有任意按键或鼠标键按住时不触发。"
                } else {
                    "鼠标精确观察：修饰键照常交给应用；泛型 Ctrl/Shift/Alt 接受左右任一侧或双侧，其他按住键会排除匹配。"
                }))
            })
            .when(spec.keyboard.matching.is_mouse_unmodified(), |element| {
                element.child(caption("鼠标无控制键观察：已透传的普通键、Win 键或额外鼠标键可以继续按住；任一侧 Ctrl/Shift/Alt 或任何仍被规则持有的按压都会排除匹配。此规则不捕获额外键，不改变其按下、重复或释放。"))
            })
            .when(spec.keyboard.matching.is_mouse_sampled(), |element| {
                element.child(caption("逐事件修饰快照：仅按本次事件提供的修饰状态匹配；没有有效快照时不触发，额外按压须已透传。"))
            })
            .when_some(match spec.keyboard.matching {
                omnitrigger_core::KeyboardMatching::MouseExact { priority, pointer_scope_priority }
                | omnitrigger_core::KeyboardMatching::MouseUnmodified { priority, pointer_scope_priority }
                | omnitrigger_core::KeyboardMatching::MouseSampled { priority, pointer_scope_priority, .. }
                | omnitrigger_core::KeyboardMatching::MouseCapturedLeader { priority, pointer_scope_priority } => Some((priority, pointer_scope_priority)),
                _ => None,
            }, |element, (priority, pointer_scope_priority)| {
                element
                    .child(caption(format!("鼠标匹配顺序：{priority}（同应用优先级内，数值越小越先；配置文件中设置）。")))
                    .child(caption(if pointer_scope_priority {
                        "指针窗口范围参与匹配优先级。"
                    } else {
                        "指针窗口范围仅作过滤，不提升其匹配优先级。"
                    }))
            })
            .when(spec.keyboard.matching.is_mouse_captured_leader(), |element| {
                element.child(caption("仅引用已由键盘层捕获的引导会话；此鼠标规则不会自行捕获按键，透传或已取消的旧按压不能触发。"))
            })
            .when_some(spec.keyboard.dedicated_prefix, |element, button| {
                let name = theme::caps_for_buttons(&[button])[0].label.clone();
                element.child(caption(format!("{name} 专用层：未绑定的键盘输入不回放。")))
            })
            .when_some(spec.keyboard.retained_prefix.as_ref(), |element, policy| {
                let name = theme::caps_for_buttons(&[policy.key])[0].label.clone();
                element
                    .child(caption(format!(
                        "{name} 保留原功能层：前键间隔 {}ms，快速输入时窗 {}ms。",
                        policy.quiet_ms, policy.fast_ms
                    )))
                    .child(caption(format!(
                        "前缀捕获范围：{}。",
                        settings::process_scope_detail(
                            &policy.capture_scope,
                            sources.capture.as_ref()
                        )
                    )))
            })
            .when(spec.keyboard.restore_on_tap, |element| {
                element.child(caption(spec.keyboard.tap_timeout_ms.map_or_else(
                    || "独立单按恢复原键（不限时）。".to_string(),
                    |ms| {
                        if spec.keyboard.retained_prefix.is_some() {
                            format!("独立单按恢复原键（不超过 {ms}ms）。")
                        } else {
                            format!("独立单按恢复原键（严格小于 {ms}ms）。")
                        }
                    },
                )))
            })
            .when(!spec.keyboard.tap_scope.is_global(), |element| {
                element.child(caption(format!(
                    "单按恢复范围：{}。",
                    settings::process_scope_detail(&spec.keyboard.tap_scope, sources.tap.as_ref())
                )))
            })
            .when(spec.keyboard.repeat, |element| {
                element.child(caption("按住重复执行（仅支持按下触发，配置文件中设置）。"))
            });
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
                .child(caption(match spec.wheel {
                    omnitrigger_core::WheelPolicy::Notch => {
                        "滚轮累计一格时触发；位置条件和按住组合独立配置。"
                    }
                    omnitrigger_core::WheelPolicy::Packet => {
                        "每个非零滚轮事件触发一次；位置条件和按住组合独立配置。"
                    }
                }))
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
                        .min_w_0()
                        .px(px(6.))
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
                        .min_w_0()
                        .px(px(6.))
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
                        .min_w_0()
                        .px(px(6.))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_trigger_mode(TriggerMode::Drag, cx)
                        })),
                    )
                    .child(
                        segment(
                            "trigger-gesture",
                            "手势",
                            self.trigger_mode == TriggerMode::Gesture,
                        )
                        .flex_1()
                        .min_w_0()
                        .px(px(6.))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_trigger_mode(TriggerMode::Gesture, cx)
                        })),
                    ),
            )
            .when(self.trigger_mode == TriggerMode::Gesture, |el| {
                el.child(self.gesture_fields(cx))
            })
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
        if let ActionSpec::Invoke(call) = &self.draft_action {
            let call = call.clone();
            let is_script = self
                .action_service
                .catalog()
                .resolve(&call)
                .is_ok_and(|entry| entry.definition.runtime == "script");
            return div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(div().flex_1().child(section_title("动作包")))
                        .when(is_script, |element| {
                            element.child(
                                small(button(
                                    "edit-current-script",
                                    "编辑代码",
                                    Tone::Secondary,
                                    !self.recording,
                                ))
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.edit_script_project(call.clone(), cx)
                                    },
                                )),
                            )
                        })
                        .child(
                            small(button(
                                "change-package-action-kind",
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
                .child(self.package_action_fields(cx))
                .child(self.message_field("动作名称", hint));
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
                                ActionSpec::SendShortcut(ShortcutAction::new(vec![0x08]));
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
                                    this.text_history.cancel_send();
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
                    this.text_history.cancel_send();
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
                let result = if let ActionSpec::SendShortcut(existing) = &self.draft_action {
                    existing.edit_keys(&self.shortcut_keys.read(cx).text())
                } else {
                    ShortcutAction::parse(&self.shortcut_keys.read(cx).text())
                };
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
                    title: None,
                    gesture: None,
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
        // 刚切到手势、尚未绘制时不是错误：不提示冲突，只是暂不能添加。
        let awaiting = self.awaiting_gesture();
        let conflict = match &draft {
            Ok(spec) => self.triggers.check_candidate(spec, None).err(),
            Err(_) if awaiting => None,
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
                None if awaiting => "绘制手势后即可添加",
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
                if matches!(
                    self.draft_action,
                    ActionSpec::Browser(_) | ActionSpec::Invoke(_)
                ) {
                    body.swap(3, 4);
                }
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        icon_button("add", Icon::Check, "添加触发器", Tone::Primary, !awaiting)
                            .flex_1()
                            .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                    )
                    .child(if matches!(self.draft_action, ActionSpec::Invoke(_)) {
                        button(
                            "run-package-draft-footer",
                            "运行草稿",
                            Tone::Secondary,
                            !self.action_service.is_running(spec.id),
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.test_action(cx)))
                    } else {
                        button("preview-draft", "预览", Tone::Secondary, true)
                            .on_click(cx.listener(|this, _, _, cx| this.preview_draft(cx)))
                    })
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
        // 刚切到手势、尚未绘制时不是错误：不提示冲突，只是暂不能保存。
        let awaiting = self.awaiting_gesture();
        let conflict = match self.draft_spec(&binding.spec, cx) {
            Ok(spec) => self.check_draft_trigger(&spec, Some(id)).err(),
            Err(_) if awaiting => None,
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
        let changed = !draft.trim().is_empty() && self.has_binding_edits(binding, cx);
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
        if matches!(
            self.draft_action,
            ActionSpec::Browser(_) | ActionSpec::Invoke(_)
        ) {
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
                    changed && conflict.is_none() && draft_available && !awaiting,
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
        let package_result = self.package_result(cx);
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
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .when_some(package_result, |element, result| element.child(result))
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
        if self.mode.history_only() {
            return div()
                .h(px(26.))
                .flex_none()
                .px(px(14.))
                .flex()
                .items_center()
                .gap(px(8.))
                .bg(rgb(theme::SIDEBAR))
                .border_t_1()
                .border_color(rgb(theme::DIVIDER))
                .text_size(px(11.5))
                .text_color(rgb(theme::TEXT_MUTED))
                .child(dot(theme::TEXT_MUTED, 6.))
                .child(self.mode.initial_status());
        }
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
        let history_window = self.mode.history_only();
        let visible_page = if history_window {
            Page::TextHistory
        } else {
            self.page
        };
        let page = match visible_page {
            Page::Actions => self.render_actions(window, cx).into_any_element(),
            Page::Wheels => self
                .render_wheels(
                    (
                        f32::from(window.viewport_size().width),
                        f32::from(window.viewport_size().height),
                    ),
                    cx,
                )
                .into_any_element(),
            Page::Presets => self.render_presets(cx).into_any_element(),
            Page::Extensions => self.render_extensions(cx).into_any_element(),
            Page::Triggers => div()
                .flex_1()
                .min_w_0()
                .flex()
                .child(self.render_list(window, cx))
                .child(self.render_inspector(cx))
                .into_any_element(),
            Page::History => self.render_history(cx).into_any_element(),
            Page::TextHistory => self.render_text_history(cx).into_any_element(),
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
                    .when(!history_window, |element| {
                        element.child(self.render_sidebar(title_height, cx))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .when(title_height > 0., |el| {
                                el.child(self.render_title_bar(title_height, buttons_width))
                            })
                            .when(history_window, |element| {
                                element.child(
                                    div()
                                        .px(px(20.))
                                        .py(px(8.))
                                        .window_control_area(WindowControlArea::Drag)
                                        .child(pill("独立历史窗口", theme::TEXT_MUTED)),
                                )
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

fn smoke_test(wheel_input_policy: WheelInputPolicy) -> Result<(), String> {
    let mut bindings = settings::defaults();
    let mut packet = bindings[0].clone();
    packet.spec.id = settings::next_id(&bindings);
    packet.spec.held = vec![Button::Mouse(omnitrigger_core::MouseButton::X2)];
    packet.spec.activation = Activation::Wheel(omnitrigger_core::WheelDirection::Left);
    packet.spec.wheel = omnitrigger_core::WheelPolicy::Packet;
    packet.spec.pointer_scope.allow = vec!["omnitrigger-smoke".into()];
    bindings.push(packet);
    let mut scoped = bindings[0].clone();
    scoped.spec.id = settings::next_id(&bindings);
    scoped.spec.process_scope.allow = vec!["omnitrigger-smoke".into()];
    bindings.push(scoped);
    for name in ["explorer", "desktop", "taskbar"] {
        let mut shell = bindings[0].clone();
        shell.spec.id = settings::next_id(&bindings);
        shell.spec.held = vec![Button::Key(162)];
        shell.spec.activation = Activation::Press(Button::Key(131));
        shell.spec.process_scope = omnitrigger_core::ProcessScope {
            identity: omnitrigger_core::ProcessIdentity::ShellContext,
            allow: vec![name.into()],
            deny: vec![],
        };
        bindings.push(shell);
    }
    // 使用 S，避开默认 CapsLock+Space 呼出面板的真实前缀冲突。
    for (held, terminal) in [([20, 83], 69), ([20, 69], 80)] {
        let mut contextual = bindings[0].clone();
        contextual.spec.id = settings::next_id(&bindings);
        contextual.spec.held = held.into_iter().map(Button::Key).collect();
        contextual.spec.activation = Activation::Press(Button::Key(terminal));
        bindings.push(contextual);
    }
    for (priority, held) in [
        vec![Button::Key(135), Button::Key(69)],
        vec![Button::Key(135)],
    ]
    .into_iter()
    .enumerate()
    {
        let mut captured = bindings[0].clone();
        captured.spec.id = settings::next_id(&bindings);
        captured.spec.held = held;
        captured.spec.activation = Activation::Press(Button::Key(80));
        captured.spec.keyboard = omnitrigger_core::KeyboardPolicy {
            dedicated_prefix: Some(Button::Key(135)),
            matching: omnitrigger_core::KeyboardMatching::Captured {
                priority: priority as u16,
            },
            repeat: true,
            ..Default::default()
        };
        bindings.push(captured);
    }
    let mut retained = bindings[0].clone();
    retained.spec.id = settings::next_id(&bindings);
    retained.spec.held = vec![Button::Key(134)];
    retained.spec.activation = Activation::Press(Button::Key(75));
    retained.spec.keyboard = omnitrigger_core::KeyboardPolicy {
        retained_prefix: Some(Box::new(omnitrigger_core::RetainedPrefix {
            key: Button::Key(134),
            capture_scope: Default::default(),
            quiet_ms: 50,
            fast_ms: 50,
            block_with_primary_mouse: false,
        })),
        matching: omnitrigger_core::KeyboardMatching::Captured { priority: 0 },
        repeat: true,
        restore_on_tap: true,
        tap_timeout_ms: Some(1500),
        ..Default::default()
    };
    bindings.push(retained);
    for leader in [134, 135] {
        let mut observer = bindings[0].clone();
        observer.spec.id = settings::next_id(&bindings);
        observer.spec.held = vec![Button::Key(leader)];
        observer.spec.activation = Activation::Wheel(omnitrigger_core::WheelDirection::Right);
        observer.spec.wheel = omnitrigger_core::WheelPolicy::Packet;
        observer.spec.keyboard = omnitrigger_core::KeyboardPolicy {
            matching: omnitrigger_core::KeyboardMatching::MouseCapturedLeader {
                priority: 0,
                pointer_scope_priority: false,
            },
            ..Default::default()
        };
        bindings.push(observer);
    }
    // 只校验注册与监听启动，不注入输入；同时覆盖鼠标按下和滚轮两种合法入口。
    for (activation, pointer_scope_priority) in [
        (Activation::Press(Button::Mouse(MouseButton::Middle)), true),
        (
            Activation::Wheel(omnitrigger_core::WheelDirection::Left),
            false,
        ),
    ] {
        let mut unmodified = bindings[0].clone();
        unmodified.spec.id = settings::next_id(&bindings);
        unmodified.spec.held.clear();
        unmodified.spec.activation = activation;
        unmodified.spec.process_scope.allow = vec!["omnitrigger-smoke".into()];
        unmodified.spec.pointer_scope.allow = vec!["omnitrigger-smoke".into()];
        unmodified.spec.keyboard = omnitrigger_core::KeyboardPolicy {
            matching: omnitrigger_core::KeyboardMatching::MouseUnmodified {
                priority: 1,
                pointer_scope_priority,
            },
            ..Default::default()
        };
        bindings.push(unmodified);
    }
    for activation in [
        Activation::Press(Button::Mouse(MouseButton::Middle)),
        Activation::Wheel(omnitrigger_core::WheelDirection::Left),
    ] {
        let mut sampled = bindings[0].clone();
        sampled.spec.id = settings::next_id(&bindings);
        sampled.spec.held.clear();
        sampled.spec.activation = activation;
        sampled.spec.process_scope.allow = vec!["omnitrigger-snapshot-smoke".into()];
        sampled.spec.pointer_scope.allow = vec!["omnitrigger-snapshot-smoke".into()];
        sampled.spec.keyboard = omnitrigger_core::KeyboardPolicy {
            matching: omnitrigger_core::KeyboardMatching::MouseSampled {
                policy: omnitrigger_core::MouseModifierPolicy {
                    required: omnitrigger_core::ModifierRequirement::Control,
                    allow_other_modifiers: false,
                    allow_extra_passed_keys: true,
                    allow_extra_passed_mouse: false,
                },
                priority: 0,
                pointer_scope_priority: true,
            },
            ..Default::default()
        };
        bindings.push(sampled);
    }
    for binding in &mut bindings {
        if binding.spec.held.contains(&Button::Key(20)) {
            binding.spec.keyboard = omnitrigger_core::KeyboardPolicy {
                matching: Default::default(),
                dedicated_prefix: Some(Button::Key(20)),
                retained_prefix: None,
                repeat: true,
                restore_on_tap: true,
                tap_timeout_ms: Some(20_000),
                tap_scope: Default::default(),
            };
        }
    }
    settings::validate(&bindings)?;
    let (backend, events) = Backend::start_with_wheel_input_policy(wheel_input_policy)?;
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
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        backend.send(BackendCommand::GetDiagnostics)?;
        match events.recv_timeout(Duration::from_millis(100)) {
            Ok(BackendEvent::Diagnostics(snapshot))
                if snapshot.wheel_input_policy != wheel_input_policy =>
            {
                return Err("实际监听的滚轮策略与启动参数不一致。".into());
            }
            Ok(BackendEvent::Diagnostics(snapshot))
                if snapshot.process_context_required
                    && snapshot.pointer_context_required
                    && snapshot.pointer_process.is_some()
                    && snapshot.pointer_window_role != omnitrigger_core::WindowRole::Unknown
                    && snapshot.foreground_process.is_some()
                    && snapshot.foreground_window_role != omnitrigger_core::WindowRole::Unknown =>
            {
                break
            }
            Ok(BackendEvent::Error(error)) => return Err(error),
            _ => {}
        }
        if std::time::Instant::now() >= deadline {
            return Err("前台或指针窗口上下文未就绪；带窗口范围的真实 Hook 验证未完成。".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    backend.send(BackendCommand::SetEnabled(false))?;
    drop(backend);
    println!("OmniTrigger 开发检查：键盘/鼠标 Hook 已安装并正常卸载；应用/指针窗口范围、Shell 上下文、专用/保留层、重复、上下文终止键、捕获匹配、鼠标引导引用、无控制键鼠标及逐事件修饰快照注册有效；前台与指针进程/窗口角色已在 Hook 外解析。本检查不注入输入，未验证物理触发效果。");
    Ok(())
}

/// 仅由既有UI broker在GPUI线程调用；该窗关闭不退出主应用。
fn open_script_history_window(
    target: clipboard_history_send::HistoryLaunchTarget,
    test_data: bool,
    cx: &mut App,
) -> Result<WindowHandle<Workspace>, String> {
    let bounds = Bounds::centered(None, size(px(1060.), px(780.)), cx);
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(920.), px(620.))),
            titlebar: Some(TitlebarOptions {
                title: Some("OmniTrigger · 文本历史".into()),
                ..Default::default()
            }),
            focus: true,
            app_id: Some("OmniTrigger.TextHistory".into()),
            ..Default::default()
        },
        |window, cx| {
            let hwnd =
                HasWindowHandle::window_handle(&*window)
                    .ok()
                    .and_then(|handle| match handle.as_raw() {
                        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
                        _ => None,
                    });
            let title_bar = hwnd
                .ok_or_else(|| "无法取得历史窗口句柄。".to_string())
                .and_then(|hwnd| {
                    omnitrigger_windows::extend_into_title_bar(hwnd)
                        .map(|()| hwnd)
                        .map_err(|_| "无法扩展历史标题栏。".to_string())
                });
            let own_window = gpui::Window::window_handle(&*window);
            let view = cx.new(|cx| {
                Workspace::new(
                    cx,
                    StartupOptions {
                        mouse_shortcuts: None,
                        wheel_input_policy: WheelInputPolicy::Unfiltered,
                        ui_smoke: false,
                        browser_connector_settings: false,
                        appearance_settings: false,
                        text_history_page: true,
                        text_history_target: Some(target),
                        text_history_ui_window: hwnd,
                        panel: false,
                        wheel: None,
                        wheel_target: None,
                        wheel_editor: false,
                        listen: false,
                        test_profile: None,
                        test_data,
                    },
                    own_window,
                    title_bar,
                )
            });
            view.update(cx, |view, cx| {
                view.text_history.activate();
                cx.notify();
            });
            let closing = view.downgrade();
            window.on_window_should_close(cx, move |_, cx| {
                let _ = closing.update(cx, |view, cx| {
                    view.text_history.request_popup_close();
                    cx.notify();
                });
                // 既有broker轮询只在本窗采集明确Off后移除此窗。
                false
            });
            view
        },
    )
    .map_err(|_| "无法创建历史窗口。".to_string())
}

fn check_config() -> Result<String, String> {
    let bindings = settings::load()?;
    settings::validate(&bindings)?;
    let wheels = Wheels::load(&settings::path())?;
    let scenes = settings::load_scenes()?;
    for wheel in &wheels.wheels {
        wheel_scenes::check_wheel(wheel, &scenes)
            .map_err(|error| format!("轮盘“{}”：{error}", wheel.name))?;
    }
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
    Ok(format!(
        "配置有效，共 {} 条绑定：{}",
        bindings.len(),
        settings::path().display()
    ))
}

fn main() {
    let mut arguments: Vec<_> = std::env::args().collect();
    if let Some(code) = explorer_context_helper::cli(&arguments) {
        std::process::exit(code);
    }
    if let Some(code) = selected_text_helper::cli(&arguments) {
        std::process::exit(code);
    }
    // 配置档参数先于其他入口取出：确定数据根、处理打包重定向，配置档命令在此退出。
    let profile_startup = profiles::startup(&mut arguments);
    if let Some(result) = legacy_local_source::cli(&arguments) {
        match result {
            Ok(message) => println!("{message}"),
            Err(error) => {
                eprintln!("{}", serde_json::json!({"status":"rejected","error":error}));
                std::process::exit(1);
            }
        }
        return;
    }
    // 先拒绝混用模式，尚未读配置、启动 worker、安装 Hook 或执行动作。
    let mouse_shortcuts_path = mouse_shortcut_dispatch::parse_plan_path(&arguments)
        .unwrap_or_else(|error| mouse_shortcut_dispatch::reject_cli(error));
    #[cfg(windows)]
    if arguments.get(1).map(String::as_str) == Some("--omni-clipboard-history-worker") {
        let code = if arguments.len() == 4 {
            omnitrigger_windows::run_clipboard_history_worker_helper(&arguments[2], &arguments[3])
        } else {
            2
        };
        std::process::exit(code);
    }
    #[cfg(windows)]
    if arguments.get(1).map(String::as_str) == Some("--omni-clipboard-history-capture-gated") {
        let previous = if arguments.len() == 5 {
            arguments[4].parse::<u32>().ok()
        } else {
            None
        };
        let code = match previous {
            Some(previous) => omnitrigger_windows::run_clipboard_history_gated_capture_helper(
                &arguments[2],
                &arguments[3],
                previous,
            ),
            None => 2,
        };
        std::process::exit(code);
    }
    #[cfg(windows)]
    if arguments.get(1).map(String::as_str) == Some("--omni-clipboard-history-capture") {
        let previous = if arguments.len() == 5 {
            arguments[4].parse::<u32>().ok()
        } else {
            None
        };
        let code = match previous {
            Some(previous) => omnitrigger_windows::run_clipboard_history_capture_helper(
                &arguments[2],
                &arguments[3],
                previous,
            ),
            None => 2,
        };
        std::process::exit(code);
    }
    let startup_mode = match WorkspaceMode::from_arguments(&arguments) {
        Ok(mode) => mode,
        Err(message) => {
            eprintln!(
                "{}",
                serde_json::json!({"status":"rejected","error":{
                    "code":"invalid_cli_arguments","message":message,"effect_state":"not_started"
                }})
            );
            std::process::exit(1);
        }
    };
    let text_history_page = startup_mode.history_only();
    let wheel_input_policy = match wheel_input_cli::parse(&arguments) {
        Ok(policy) => policy,
        Err(error) => {
            eprintln!("{}", serde_json::json!({"status":"rejected","error":error}));
            std::process::exit(1);
        }
    };
    #[cfg(debug_assertions)]
    if script_notify_smoke::maybe_run(&arguments) {
        return;
    }
    // 新建工程、安装宿主不与系统操作或其他命令混用；在任何 CLI 副作用前拒绝组合。
    if arguments.iter().any(|argument| {
        matches!(
            argument.as_str(),
            "--create-script-action" | "--install-script-runtime"
        )
    }) {
        if let Err(message) = action_target_context::validate_cli(&arguments) {
            eprintln!(
                "{}",
                serde_json::json!({"status":"rejected","error":{"code":"invalid_cli_arguments","message":message,"effect_state":"not_started"}})
            );
            std::process::exit(1);
        }
    }
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
    let mouse_shortcuts_plan = mouse_shortcuts_path.map(|path| {
        let plan = mouse_shortcut_dispatch::read_plan(&path)
            .unwrap_or_else(|error| mouse_shortcut_dispatch::reject_cli(error));
        let bindings = settings::load().unwrap_or_else(|message| {
            mouse_shortcut_dispatch::reject_cli(mouse_shortcut_dispatch::CliError {
                code: "mouse_queue_config_read_failed",
                message,
            })
        });
        settings::validate(&bindings).unwrap_or_else(|message| {
            mouse_shortcut_dispatch::reject_cli(mouse_shortcut_dispatch::CliError {
                code: "mouse_queue_config_invalid",
                message,
            })
        });
        mouse_shortcut_dispatch::validate_plan_bindings(&plan, &bindings)
            .unwrap_or_else(|error| mouse_shortcut_dispatch::reject_cli(error));
        plan
    });
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
    if let Some(result) = action_packages::cli(&arguments, profile_startup.test_data) {
        match result {
            Ok(text) => println!("{text}"),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    if let Some(result) = trigger_preview::cli(&arguments) {
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
        match check_config() {
            Ok(message) => println!("{message}"),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    if arguments.iter().any(|argument| argument == "--smoke-test") {
        if let Err(error) = smoke_test(wheel_input_policy) {
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
    let launch_target = if preserve_foreground || text_history_page {
        match omnitrigger_windows::capture_shortcut_target() {
            Ok(target) => Some(target),
            Err(error) if preserve_foreground => {
                eprintln!("{error}");
                std::process::exit(1);
            }
            Err(_) => None,
        }
    } else {
        None
    };
    let wheel_launch_target = if preserve_foreground {
        launch_target
    } else {
        None
    };
    let text_history_target = if text_history_page {
        launch_target
            .and_then(|target| clipboard_history_send::HistoryLaunchTarget::capture(target).ok())
    } else {
        None
    };
    // 更新器的成功和回滚入口都用原配置打开设置页。标准配置仍要占用其日常窗口身份。
    let updater_settings = matches!(&arguments[1..], [config, _, page] if config == "--config" && page == "--settings")
        && (std::env::var_os("OMNITRIGGER_UPDATE_HEALTH").is_some()
            || std::env::var_os("OMNITRIGGER_UPDATE_ERROR").is_some());
    let config_path = settings::path();
    let _profile_in_use = match profile_startup.hold_window_profile(&config_path) {
        Ok(file) => file,
        Err(error) => {
            omnitrigger_windows::show_error_dialog("OmniTrigger 无法打开配置窗口", &error);
            return;
        }
    };
    let workspace_instance = if arguments.len() == 1
        || (updater_settings
            && config_path
                .file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("bindings.json")))
    {
        match profile_startup
            .workspace_key(&config_path)
            .and_then(|key| omnitrigger_windows::WorkspaceInstance::open(&key))
        {
            Ok(Some(instance)) => {
                // 只有新主窗口能在载入配置前导入；重复启动只唤回原窗口。
                #[cfg(windows)]
                profile_startup.offer_external_copy();
                Some(instance)
            }
            Ok(None) => return,
            Err(message) => {
                omnitrigger_windows::show_error_dialog("OmniTrigger 无法打开配置窗口", &message);
                return;
            }
        }
    } else {
        None
    };
    Application::new()
        .with_assets(Assets)
        .run(move |cx: &mut App| {
            message_input::init(cx);
            script_ui::init(profile_startup.test_data, cx);
            let bounds = Bounds::centered(None, size(px(1060.), px(780.)), cx);
            let result = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(920.), px(620.))),
                    focus: !preserve_foreground,
                    titlebar: Some(TitlebarOptions {
                        title: Some(
                            profile_startup
                                .window_title(startup_mode.window_title())
                                .into(),
                        ),
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
                    let text_history_ui_window = HasWindowHandle::window_handle(&*window)
                        .ok()
                        .and_then(|handle| match handle.as_raw() {
                            RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
                            _ => None,
                        });
                    let main_window = gpui::Window::window_handle(&*window);
                    let tray_window = text_history_ui_window.filter(|_| {
                        profile_startup.test_name.is_none()
                            && (arguments.iter().skip(1).all(|arg| arg == "--settings")
                                || updater_settings)
                    });
                    let view = cx.new(|cx| {
                        let mut workspace = Workspace::new(
                            cx,
                            StartupOptions {
                                mouse_shortcuts: mouse_shortcuts_plan,
                                wheel_input_policy,
                                ui_smoke,
                                browser_connector_settings,
                                appearance_settings,
                                text_history_page,
                                text_history_target,
                                text_history_ui_window,
                                panel: open_panel && initial_wheel.is_none(),
                                wheel: initial_wheel,
                                wheel_target: wheel_launch_target,
                                wheel_editor,
                                listen: profile_startup.listen,
                                test_profile: profile_startup.test_name.clone(),
                                test_data: profile_startup.test_data,
                            },
                            main_window,
                            title_bar,
                        );
                        workspace.workspace_instance = workspace_instance;
                        workspace
                    });
                    if text_history_page {
                        view.update(cx, |view, cx| {
                            view.text_history.activate();
                            cx.notify();
                        });
                    }
                    let closing_view = view.downgrade();
                    if let Some(error) =
                        tray_window.and_then(|hwnd| omnitrigger_windows::install_tray(hwnd).err())
                    {
                        view.update(cx, |view, cx| {
                            view.status = error;
                            cx.notify();
                        });
                    }
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
                        match closing_view.update(cx, |view, cx| {
                            if !view.legacy_close_requested
                                && tray_window.is_some_and(omnitrigger_windows::hide_to_tray)
                            {
                                if view.recording {
                                    view.send(BackendCommand::CancelRecording);
                                }
                                return false;
                            }
                            view.request_legacy_close(cx)
                        }) {
                            Ok(allow_close) => allow_close,
                            Err(_) => {
                                cx.quit();
                                true
                            }
                        }
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

// 编号只在所属executor内唯一；仅消费本路径实际可能产生的终态。
fn consume_owned_shortcut_receipt(
    owners: &mut HashMap<u64, u64>,
    from_shortcut_executor: bool,
    event: &execution::ExecutionEvent,
) -> bool {
    if !from_shortcut_executor
        || !matches!(
            &event.outcome,
            ExecutionOutcome::ShortcutSent | ExecutionOutcome::Failed { .. }
        )
        || owners.get(&event.invocation_id) != Some(&event.id)
    {
        return false;
    }
    owners.remove(&event.invocation_id);
    true
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
            Page::TextHistory,
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
            BackendEvent::Recorded(Box::new(super::PRESETS[0].spec(1))),
            BackendEvent::RecordingCancelled,
            BackendEvent::Triggered {
                id: 1,
                pointer: None,
                foreground: 0,
            },
            BackendEvent::Error("监听已断开".into()),
        ];
        for page in [
            Page::Triggers,
            Page::Actions,
            Page::Wheels,
            Page::Presets,
            Page::History,
            Page::TextHistory,
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
    #[test]
    fn tracked_shortcut_owner_ignores_other_executor_nonterminal_and_stale_receipts() {
        use super::consume_owned_shortcut_receipt;
        use super::execution::{ExecutionEvent, ExecutionOutcome};
        use std::collections::HashMap;

        let mut owners = HashMap::from([(1, 7)]);
        let mut event = ExecutionEvent {
            id: 7,
            invocation_id: 1,
            outcome: ExecutionOutcome::ShortcutSent,
        };
        // command executor也可能编号1、binding7，来源不匹配时不能回收。
        assert!(!consume_owned_shortcut_receipt(&mut owners, false, &event));
        assert_eq!(owners.get(&1), Some(&7));
        for outcome in [
            ExecutionOutcome::Started { pid: 123 },
            ExecutionOutcome::Unknown {
                message: "未知".into(),
            },
            ExecutionOutcome::WindowApplied,
            ExecutionOutcome::Finished {
                code: Some(0),
                success: true,
            },
        ] {
            event.outcome = outcome;
            assert!(!consume_owned_shortcut_receipt(&mut owners, true, &event));
            assert_eq!(owners.get(&1), Some(&7));
        }
        event.outcome = ExecutionOutcome::ShortcutSent;
        event.id = 8;
        assert!(!consume_owned_shortcut_receipt(&mut owners, true, &event));
        assert_eq!(owners.get(&1), Some(&7));
        event.id = 7;
        assert!(consume_owned_shortcut_receipt(&mut owners, true, &event));
        assert!(owners.is_empty());

        owners.insert(2, 7);
        assert!(!consume_owned_shortcut_receipt(&mut owners, true, &event));
        assert_eq!(owners.get(&2), Some(&7));
        event.invocation_id = 2;
        event.outcome = ExecutionOutcome::Failed {
            message: "取消后返回".into(),
        };
        assert!(consume_owned_shortcut_receipt(&mut owners, true, &event));
        assert!(owners.is_empty());
        assert!(!consume_owned_shortcut_receipt(&mut owners, true, &event));
    }
}
