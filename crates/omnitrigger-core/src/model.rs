//! 标准化输入、统一触发描述与确定性决策。

use crate::TriggerCondition;

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
    pub id: u64,
    pub held: Vec<Button>,
    pub activation: Activation,
    pub conditions: Vec<TriggerCondition>,
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
