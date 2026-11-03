//! 标准化输入、统一触发描述与确定性决策。

use crate::{ProcessScope, TriggerCondition};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    X1,
    X2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Button {
    Key(u16),
    Mouse(MouseButton),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Axis {
    Vertical,
    Horizontal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WheelDirection {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputKind {
    Down(Button),
    Up(Button),
    Wheel { axis: Axis, delta: i32 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InputEvent {
    pub kind: InputKind,
    pub injected: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Activation {
    Press(Button),
    Hold {
        button: Button,
        duration_ms: u32,
    },
    /// 从按下起严格小于 max_start_ms 的窗口内，物理鼠标位移达到阈值时触发。
    Drag {
        button: Button,
        min_distance_px: u32,
        max_start_ms: u32,
    },
    Wheel(WheelDirection),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DragPhase {
    Started,
    Ended,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DragEvent {
    pub id: u64,
    pub session: u64,
    pub phase: DragPhase,
    pub origin: (i32, i32),
    pub position: (i32, i32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActiveDrag {
    pub id: u64,
    pub session: u64,
    pub button: Button,
    pub origin: (i32, i32),
    pub position: (i32, i32),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TriggerSpec {
    pub keyboard: KeyboardPolicy,
    pub process_scope: ProcessScope,
    pub id: u64,
    pub held: Vec<Button>,
    pub activation: Activation,
    pub conditions: Vec<TriggerCondition>,
}

/// 只影响物理键盘输入；缺省保留既有回退和单次触发行为。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct KeyboardPolicy {
    /// 默认精确按下集合；捕获模式要求显式键盘层。
    pub matching: KeyboardMatching,
    /// 专用层的键必须在 held 中。层内未匹配的键盘按压不回放，鼠标输入仍按各自规则处理。
    pub dedicated_prefix: Option<Button>,
    /// 保留原功能的捕获层；与专用层互斥，时间由事件调用方显式提供。
    pub retained_prefix: Option<Box<RetainedPrefix>>,
    /// 仅普通键盘 Press 可用；精确模式重复原动作，捕获模式按当前捕获集合重新选择。
    pub repeat: bool,
    /// 独立按下且没有其他键盘输入或匹配动作时，松开后补发自身的一次点击。
    pub restore_on_tap: bool,
    /// 单按恢复的等待上限；None 不限时，不安装额外计时器。
    pub tap_timeout_ms: Option<u32>,
    /// 单按恢复的额外应用范围，不改变整个层的适用范围。
    pub tap_scope: ProcessScope,
}

impl KeyboardPolicy {
    pub(crate) fn prefix(&self) -> Option<Button> {
        self.dedicated_prefix
            .or(self.retained_prefix.as_ref().map(|policy| policy.key))
    }

    pub(crate) fn same_layer(&self, other: &Self) -> bool {
        self.dedicated_prefix == other.dedicated_prefix
            && self.retained_prefix == other.retained_prefix
            && self.matching.is_captured() == other.matching.is_captured()
            && self.restore_on_tap == other.restore_on_tap
            && self.tap_timeout_ms == other.tap_timeout_ms
            && self.tap_scope == other.tap_scope
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetainedPrefix {
    pub key: Button,
    /// 与动作应用范围独立：没有可用动作时仍可捕获前缀并执行回退。
    pub capture_scope: ProcessScope,
    /// 距最近普通按下不足此时长时，引导键直接交给应用。
    pub quiet_ms: u32,
    /// 捕获后此时窗内，附加键暂存，其他键使引导键交还应用。
    pub fast_ms: u32,
    /// 鼠标主键已按住时不捕获此引导键。
    pub block_with_primary_mouse: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum KeyboardMatching {
    #[default]
    Exact,
    /// 按已捕获附加键匹配；同应用优先级内，数值越小越先选。
    Captured { priority: u16 },
}

impl KeyboardMatching {
    pub fn is_captured(self) -> bool {
        matches!(self, Self::Captured { .. })
    }

    pub(crate) fn priority(self) -> u16 {
        match self {
            Self::Exact => 0,
            Self::Captured { priority } => priority,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Decision {
    pub consume: bool,
    pub replay: Vec<InputKind>,
    pub triggered: Vec<u64>,
    pub drags: Vec<DragEvent>,
}

pub(super) fn direction(axis: Axis, delta: i32) -> WheelDirection {
    match (axis, delta > 0) {
        (Axis::Vertical, true) => WheelDirection::Up,
        (Axis::Vertical, false) => WheelDirection::Down,
        (Axis::Horizontal, true) => WheelDirection::Right,
        (Axis::Horizontal, false) => WheelDirection::Left,
    }
}
