//! Windows 输入适配。Hook 只负责同步决策，触发通知交由外部 UI 处理。

use omnitrigger_core::{Activation, Button, MouseButton, TriggerSpec, WheelDirection};

mod automation_basics;
#[cfg(windows)]
pub use automation_basics::{
    copy_selection_text, open_target, read_clipboard_text, send_text_to, write_clipboard_text,
};
pub use automation_basics::{validate_automation_text, validate_open_target};

mod system_adjustment;
#[cfg(windows)]
pub use system_adjustment::{
    adjust_system_at, capture_adjustment_point, query_system_adjustment_capabilities_at,
};
pub use system_adjustment::{
    validate_system_adjustment, AdjustmentCapability, AdjustmentKind, AdjustmentResult,
    BrightnessMethod, SystemAdjustment, SystemAdjustmentCapabilities, SystemAdjustmentError,
    SystemAdjustmentErrorCode,
};

mod window_action;
pub use window_action::WindowOperation;
#[cfg(windows)]
pub use window_action::{
    perform_window_action, pointer_scale_at_point, shortcut_target_context,
    shortcut_target_process, shortcut_target_title,
};

mod shortcut;
#[cfg(windows)]
pub use shortcut::{
    capture_shortcut_target, ensure_shortcut_target, restore_shortcut_target, send_key_sequence_to,
    send_key_tap_to, send_shortcut, send_shortcut_to, ShortcutTarget,
};
pub use shortcut::{validate_key_sequence, validate_key_tap, validate_shortcut};

/// 专用于诊断 Hook 链顺序的注入标记；探针只计数，不进入触发器状态机。
pub const PRIORITY_PROBE_TAG: usize = 0x4f4d_5052;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldOutcome {
    Matched,
    ShortRelease,
    Movement,
    FocusChanged,
    OtherButton,
    Wheel,
    Configuration,
    Paused,
    Recording,
    Recovery,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoldAttempt {
    pub outcome: HoldOutcome,
    pub elapsed_ms: u64,
    pub threshold_ms: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldStartRejection {
    Paused,
    Recording,
    ExtraHeld,
    ExistingPress,
    Unmatched,
}

/// 监听线程的观测快照。计数表示收到的回调，不代表系统全部输入。
#[derive(Debug, Clone, Default)]
pub struct ListenerDiagnostics {
    pub keyboard_events: u64,
    pub mouse_events: u64,
    pub external_injected: u64,
    pub self_replayed: u64,
    /// 本应用快捷键动作的注入事件，与原输入补发分别计数。
    pub self_shortcuts: u64,
    pub probe_events: u64,
    pub foreground_cancellations: u64,
    pub replay_failures: u64,
    /// code>=0 回调至消费返回或调用下一 Hook 前的最大墙钟耗时，包含补发。
    pub max_hook_work_us: u64,
    /// 仅 CallNextHookEx 调用的最大墙钟耗时，不含本项目处理。
    pub max_hook_chain_us: u64,
    /// 本项目重放及失败恢复 SendInput 调用的最大墙钟耗时。
    pub max_replay_us: u64,
    /// 对应区段严格超过 50 毫秒的次数，包含线程被抢占造成的等待。
    pub slow_hook_work_count: u64,
    pub slow_hook_chain_count: u64,
    pub slow_replay_count: u64,
    /// 长按实际使用的计时后端；降级不会自动暂停监听。
    pub hold_timer_backend: &'static str,
    pub held: Vec<Button>,
    pub owned: Vec<Button>,
    /// 距最近键盘回调的毫秒数，None 表示尚未收到。
    pub last_keyboard_ms: Option<u64>,
    /// 距最近鼠标回调的毫秒数，None 表示尚未收到。
    pub last_mouse_ms: Option<u64>,
    /// 最近一次结束的长按（匹配或取消）的经过时间与配置阈值。
    pub last_hold_elapsed_ms: Option<u64>,
    pub last_hold_threshold_ms: Option<u32>,
    /// 仅最近一次长按成功匹配时有值；取消不计为迟到。
    pub last_hold_lateness_ms: Option<u64>,
    pub hold_triggered: u64,
    pub hold_cancelled_movement: u64,
    pub hold_cancelled_focus: u64,
    /// 包括短按释放、其他输入、暂停、录制、替换和错误恢复。
    pub hold_cancelled_other: u64,
    /// 最近八次已经开始计时的长按结果，按旧到新排序，不保留按键或坐标历史。
    pub recent_hold_attempts: Vec<HoldAttempt>,
    pub hold_started: u64,
    /// 已注册长按终端的新物理按下/释放边沿，排除重复事件。
    pub hold_terminal_downs: u64,
    pub hold_terminal_ups: u64,
    /// 鼠标终端已有按压时再次收到 Down；键盘自动重复不计入。
    pub hold_terminal_duplicate_downs: u64,
    /// 新按下边沿未能开始计时的次数，重复鼠标 Down 由上一字段单独记录。
    pub hold_not_started: u64,
    pub last_hold_start_rejection: Option<HoldStartRejection>,
    /// 完整显示器矩形是否已成功缓存；失败时暂停监听，不猜测虚拟桌面边界。
    pub screen_context_ready: bool,
    pub screen_count: usize,
}

#[derive(Debug)]
pub enum BackendCommand {
    SetBindings(Vec<TriggerSpec>),
    StartRecording,
    CancelRecording,
    SetEnabled(bool),
    /// 仅取消指定的活动拖拽会话；过期会话不影响新的拖拽。
    CancelDrag {
        session: u64,
    },
    GetDiagnostics,
    /// 登记可见面板的窗口，`None` 表示已收起。面板可见且未获前台时，Hook 接管单独按下的导航键。
    SetPanelWindow(Option<isize>),
    Shutdown,
}

/// 面板未获前台时由 Hook 接管的导航键；按下与对应释放都被消费，不交给前台窗口。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelKey {
    Left,
    Right,
    Up,
    Down,
    /// 方向键选择之后才接管，未选择时回车仍交给前台窗口。
    Enter,
    Escape,
    /// 主键盘或小键盘的 1–9。
    Digit(u8),
}

#[derive(Debug)]
pub enum BackendEvent {
    Ready,
    /// 录制期间当前按住的按键（不含录制前已按住的键），按下集合变化时发送。
    RecordingProgress(Vec<Button>),
    Recorded(TriggerSpec),
    /// 物理鼠标按下的位置，仅用于当前界面的窗外点击判断，不改变输入决策。
    PointerPressed {
        x: i32,
        y: i32,
    },
    /// 物理 Esc 的新按下边沿，仅通知界面收起，不消费原按键；被面板导航接管时不发送。
    EscapePressed,
    /// 面板导航消费的按键。方向键随自动重复发送，其余只在新按下时发送；
    /// Enter 与数字排在更早的输入交付之后。
    PanelKey(PanelKey),
    /// 在对应 Triggered 之前发送，时间戳仅供本进程测量匹配后的界面延迟。
    HoldTiming {
        id: u64,
        elapsed_ms: u64,
        threshold_ms: u32,
        matched_at: std::time::Instant,
    },
    /// 拖拽生命周期保留原始前台身份；不为每次鼠标移动排队。
    Drag {
        event: omnitrigger_core::DragEvent,
        foreground: isize,
    },
    /// 匹配时冻结物理指针上下文；经过回放/界面队列后仍保留当时位置。
    Triggered {
        id: u64,
        pointer: Option<(i32, i32)>,
    },
    Error(String),
    RecordingCancelled,
    Diagnostics(Box<ListenerDiagnostics>),
}

#[cfg(windows)]
mod platform;
#[cfg(windows)]
mod title_bar;
#[cfg(windows)]
mod wheel_window;
#[cfg(windows)]
pub use platform::Backend;

/// 显示由本应用持有的隐藏提示窗口，保留输入焦点和原有扩展样式。
#[cfg(windows)]
pub fn show_notification_window(
    hwnd: isize,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> Result<(), String> {
    platform::show_notification_window(hwnd, x, y, width, height)
}

/// 为面板设置窗口名称并同步隐藏，供界面准备好内容后再显示。
#[cfg(windows)]
pub fn prepare_panel_window(hwnd: isize) -> Result<(), String> {
    platform::prepare_panel_window(hwnd)
}

/// 为独立轮盘设置窗口名称并同步隐藏；复用浮动面板的不激活与收尾逻辑。
#[cfg(windows)]
pub fn prepare_wheel_window(hwnd: isize) -> Result<(), String> {
    platform::prepare_wheel_window(hwnd)
}

/// 将本应用的面板以鼠标位置为中心显示，限制在所在屏幕的工作区内；不移动指针或改变输入焦点。
/// 面板以不激活方式显示，并记录当前前台窗口，供输入筛选后收起时交还。
#[cfg(windows)]
pub fn show_panel_at_cursor(hwnd: isize) -> Result<(), String> {
    platform::show_panel_at_cursor(hwnd, None)
}

/// 与 `show_panel_at_cursor` 相同，并按目标显示器 DPI 把客户区调整为给定的逻辑尺寸。
#[cfg(windows)]
pub fn show_panel_at_cursor_with_size(hwnd: isize, width: f32, height: f32) -> Result<(), String> {
    platform::show_panel_at_cursor(hwnd, Some((width, height)))
}

/// 在鼠标位置显示独立轮盘，按目标显示器 DPI 调整客户区并限制在工作区内，不抢占前台。
#[cfg(windows)]
pub fn show_wheel_at_cursor_with_size(hwnd: isize, width: f32, height: f32) -> Result<(), String> {
    platform::show_wheel_at_cursor_with_size(hwnd, width, height)
}

/// 以显式屏幕坐标为中心显示轮盘；不移动鼠标，按目标显示器 DPI 换算并限制在工作区内。
#[cfg(windows)]
pub fn show_wheel_at_point_with_size(
    hwnd: isize,
    x: i32,
    y: i32,
    width: f32,
    height: f32,
) -> Result<(), String> {
    platform::show_wheel_at_point_with_size(hwnd, x, y, width, height)
}

/// 兼容原有调用，与 `show_panel_at_cursor` 相同。
#[cfg(windows)]
pub fn position_panel_near_cursor(hwnd: isize) -> Result<(), String> {
    show_panel_at_cursor(hwnd)
}

/// 同步隐藏本应用的面板，供截图动作启动前移除面板画面。
/// 面板曾为输入筛选取得前台时，隐藏后把前台交还给呼出前的窗口。
#[cfg(windows)]
pub fn hide_panel_window(hwnd: isize) {
    platform::hide_panel_window(hwnd)
}

/// 按原生前台窗口核验当前状态，避免异步激活通知携带过期状态。
#[cfg(windows)]
pub fn is_foreground_window(hwnd: isize) -> bool {
    platform::is_foreground_window(hwnd)
}

/// 在拥有窗口的界面线程尝试取得前台与键盘焦点，不注入输入。
/// 系统拒绝前台切换时返回 `Ok(false)`；调用方应在 GPUI 状态借用之外调用。
#[cfg(windows)]
pub fn activate_panel_window(hwnd: isize) -> Result<bool, String> {
    platform::activate_panel_window(hwnd)
}

/// 用户点击筛选框时，暂时取消不激活样式并请求前台，使面板可以接收文字输入。
/// 下次显示时恢复不激活样式；调用方应在 GPUI 状态借用之外调用。
#[cfg(windows)]
pub fn activate_panel_for_input(hwnd: isize) -> Result<bool, String> {
    platform::activate_panel_for_input(hwnd)
}

/// 判断物理屏幕坐标是否位于面板外接矩形内；句柄或矩形无效时返回 false。
#[cfg(windows)]
pub fn panel_contains_point(hwnd: isize, x: i32, y: i32) -> bool {
    platform::panel_contains_point(hwnd, x, y)
}

#[cfg(windows)]
pub use title_bar::CaptionButtons;

/// 主窗口内容延伸到标题栏，保留系统绘制的最小化、最大化、关闭按钮。
#[cfg(windows)]
pub fn extend_into_title_bar(hwnd: isize) -> Result<(), String> {
    title_bar::extend_into_title_bar(hwnd)
}

/// 系统标题按钮在客户区右上角占据的物理像素尺寸。
#[cfg(windows)]
pub fn caption_buttons(hwnd: isize) -> Option<CaptionButtons> {
    title_bar::caption_buttons(hwnd)
}

/// 设置标题按钮下方的底色，`rgb` 为 0xRRGGBB；系统不支持时保持默认。
#[cfg(windows)]
pub fn set_caption_color(hwnd: isize, rgb: u32) {
    title_bar::set_caption_color(hwnd, rgb)
}

/// 本地时间的时、分、秒，用于界面记录；不参与触发判定。
#[cfg(windows)]
pub fn local_clock() -> (u16, u16, u16) {
    platform::local_clock()
}

pub fn format_button(button: &Button) -> String {
    match button {
        Button::Mouse(mouse) => match mouse {
            MouseButton::Left => "鼠标左键".into(),
            MouseButton::Right => "鼠标右键".into(),
            MouseButton::Middle => "鼠标中键".into(),
            MouseButton::X1 => "鼠标侧键 1".into(),
            MouseButton::X2 => "鼠标侧键 2".into(),
        },
        Button::Key(key) => format_key(*key),
    }
}

fn format_key(key: u16) -> String {
    match key {
        0x08 => "Backspace".into(),
        0x09 => "Tab".into(),
        0x0d => "Enter".into(),
        0x14 => "CapsLock".into(),
        0x1b => "Esc".into(),
        0x20 => "Space".into(),
        0x21 => "PageUp".into(),
        0x22 => "PageDown".into(),
        0x23 => "End".into(),
        0x24 => "Home".into(),
        0x25 => "←".into(),
        0x26 => "↑".into(),
        0x27 => "→".into(),
        0x28 => "↓".into(),
        0x2d => "Insert".into(),
        0x2e => "Delete".into(),
        0x30..=0x39 | 0x41..=0x5a => char::from_u32(key.into()).unwrap().into(),
        0x5b => "左 Win".into(),
        0x5c => "右 Win".into(),
        0x70..=0x87 => format!("F{}", key - 0x6f),
        0xa0 => "左 Shift".into(),
        0xa1 => "右 Shift".into(),
        0xa2 => "左 Ctrl".into(),
        0xa3 => "右 Ctrl".into(),
        0xa4 => "左 Alt".into(),
        0xa5 => "右 Alt".into(),
        _ => {
            #[cfg(windows)]
            if let Some(name) = platform::key_name(key) {
                return name;
            }
            format!("VK {key:02X}")
        }
    }
}

pub fn format_trigger(spec: &TriggerSpec) -> String {
    let mut parts: Vec<String> = spec.held.iter().map(format_button).collect();
    parts.push(match &spec.activation {
        Activation::Press(button) => format_button(button),
        Activation::Hold {
            button,
            duration_ms,
        } => format!("{}（长按 {duration_ms}ms）", format_button(button)),
        Activation::Drag {
            button,
            min_distance_px,
            max_start_ms,
        } => format!(
            "{}（快速拖拽 ≥{min_distance_px}px / {max_start_ms}ms 内）",
            format_button(button)
        ),
        Activation::Wheel(direction) => match direction {
            WheelDirection::Up => "滚轮上滚".into(),
            WheelDirection::Down => "滚轮下滚".into(),
            WheelDirection::Left => "滚轮左滚".into(),
            WheelDirection::Right => "滚轮右滚".into(),
        },
    });
    parts.join(" + ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hold_description_keeps_the_prefix_button_and_duration() {
        let spec = TriggerSpec {
            conditions: Vec::new(),
            id: 5,
            held: vec![Button::Key(0xa2)],
            activation: Activation::Hold {
                button: Button::Mouse(MouseButton::Right),
                duration_ms: 300,
            },
        };
        assert_eq!(format_trigger(&spec), "左 Ctrl + 鼠标右键（长按 300ms）");
    }
}
