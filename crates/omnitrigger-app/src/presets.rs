//! 预设只生成 TriggerSpec 草稿，注册与执行继续共享核心语义。
use omnitrigger_core::{Activation, Button, MouseButton, TriggerSpec, WheelDirection};

pub const DEFAULT_DRAG_DISTANCE_PX: u32 = 18;
pub const DEFAULT_DRAG_START_MS: u32 = 180;

pub struct TriggerPreset {
    pub key: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub group: &'static str,
    pub held: &'static [Button],
    pub activation: Activation,
    pub region: Option<omnitrigger_core::PointerRegion>,
}

impl TriggerPreset {
    pub fn spec(&self, id: u64) -> TriggerSpec {
        TriggerSpec {
            conditions: self
                .region
                .map(omnitrigger_core::TriggerCondition::PointerRegion)
                .into_iter()
                .collect(),
            id,
            held: self.held.to_vec(),
            activation: self.activation,
        }
    }
}

const fn hold(
    key: &'static str,
    title: &'static str,
    button: Button,
    duration_ms: u32,
) -> TriggerPreset {
    TriggerPreset {
        key,
        title,
        group: "长按",
        description: "达到时间后触发一次；提前松开会回放原按压。",
        held: &[],
        region: None,
        activation: Activation::Hold {
            button,
            duration_ms,
        },
    }
}

pub const PRESETS: &[TriggerPreset] = &[
    region(
        "edge-left",
        "左边缘 + 滚轮",
        omnitrigger_core::PointerRegion::ScreenEdge {
            edge: omnitrigger_core::ScreenEdge::Left,
            thickness_px: 8,
        },
    ),
    region(
        "edge-right",
        "右边缘 + 滚轮",
        omnitrigger_core::PointerRegion::ScreenEdge {
            edge: omnitrigger_core::ScreenEdge::Right,
            thickness_px: 8,
        },
    ),
    region(
        "edge-top",
        "顶部边缘 + 滚轮",
        omnitrigger_core::PointerRegion::ScreenEdge {
            edge: omnitrigger_core::ScreenEdge::Top,
            thickness_px: 8,
        },
    ),
    region(
        "edge-bottom",
        "底部边缘 + 滚轮",
        omnitrigger_core::PointerRegion::ScreenEdge {
            edge: omnitrigger_core::ScreenEdge::Bottom,
            thickness_px: 8,
        },
    ),
    region(
        "corner-top-left",
        "左上角 + 滚轮",
        omnitrigger_core::PointerRegion::ScreenCorner {
            corner: omnitrigger_core::ScreenCorner::TopLeft,
            size_px: 32,
        },
    ),
    region(
        "corner-top-right",
        "右上角 + 滚轮",
        omnitrigger_core::PointerRegion::ScreenCorner {
            corner: omnitrigger_core::ScreenCorner::TopRight,
            size_px: 32,
        },
    ),
    region(
        "corner-bottom-left",
        "左下角 + 滚轮",
        omnitrigger_core::PointerRegion::ScreenCorner {
            corner: omnitrigger_core::ScreenCorner::BottomLeft,
            size_px: 32,
        },
    ),
    region(
        "corner-bottom-right",
        "右下角 + 滚轮",
        omnitrigger_core::PointerRegion::ScreenCorner {
            corner: omnitrigger_core::ScreenCorner::BottomRight,
            size_px: 32,
        },
    ),
    TriggerPreset {
        key: "right-quick-drag",
        title: "右键快速拖拽",
        group: "快速拖拽",
        description: "按下右键后，在 180ms 之前移动达到 18px；可与右键长按同时配置。",
        held: &[],
        region: None,
        activation: Activation::Drag {
            button: Button::Mouse(MouseButton::Right),
            min_distance_px: DEFAULT_DRAG_DISTANCE_PX,
            max_start_ms: DEFAULT_DRAG_START_MS,
        },
    },
    hold(
        "right-300",
        "右键长按 300ms",
        Button::Mouse(MouseButton::Right),
        300,
    ),
    hold(
        "right-500",
        "右键长按 500ms",
        Button::Mouse(MouseButton::Right),
        500,
    ),
    hold(
        "right-800",
        "右键长按 800ms",
        Button::Mouse(MouseButton::Right),
        800,
    ),
    hold(
        "middle-300",
        "中键长按 300ms",
        Button::Mouse(MouseButton::Middle),
        300,
    ),
    hold(
        "side-1-300",
        "侧键 1 长按 300ms",
        Button::Mouse(MouseButton::X1),
        300,
    ),
    hold(
        "side-2-300",
        "侧键 2 长按 300ms",
        Button::Mouse(MouseButton::X2),
        300,
    ),
    hold("f8-500", "F8 长按 500ms", Button::Key(0x77), 500),
    TriggerPreset {
        key: "caps-space",
        title: "CapsLock + Space",
        group: "键盘组合",
        description: "按住 CapsLock 再按空格；适合作为面板入口。",
        held: &[Button::Key(0x14)],
        region: None,
        activation: Activation::Press(Button::Key(0x20)),
    },
    TriggerPreset {
        key: "ctrl-alt-space",
        title: "左 Ctrl + 左 Alt + Space",
        group: "键盘组合",
        description: "同时按住左侧 Ctrl、Alt，再按空格。",
        held: &[Button::Key(0xa2), Button::Key(0xa4)],
        region: None,
        activation: Activation::Press(Button::Key(0x20)),
    },
    TriggerPreset {
        key: "f8",
        title: "单按 F8",
        group: "单键与鼠标组合",
        description: "按下立即触发，会消费 F8 原本的输入。",
        held: &[],
        region: None,
        activation: Activation::Press(Button::Key(0x77)),
    },
    TriggerPreset {
        key: "side-2",
        title: "单按鼠标侧键 2",
        group: "单键与鼠标组合",
        description: "按下侧键即触发，会消费该键原本的输入。",
        held: &[],
        region: None,
        activation: Activation::Press(Button::Mouse(MouseButton::X2)),
    },
    TriggerPreset {
        key: "side-wheel-up",
        title: "侧键 1 + 滚轮上滚",
        group: "单键与鼠标组合",
        description: "按住侧键 1，再向上滚动一格。",
        held: &[Button::Mouse(MouseButton::X1)],
        region: None,
        activation: Activation::Wheel(WheelDirection::Up),
    },
    TriggerPreset {
        key: "side-wheel-down",
        title: "侧键 1 + 滚轮下滚",
        group: "单键与鼠标组合",
        description: "按住侧键 1，再向下滚动一格。",
        held: &[Button::Mouse(MouseButton::X1)],
        region: None,
        activation: Activation::Wheel(WheelDirection::Down),
    },
    TriggerPreset {
        key: "shift-right",
        title: "左 Shift + 右键",
        group: "单键与鼠标组合",
        description: "按住左 Shift 再按右键，按下时触发。",
        held: &[Button::Key(0xa0)],
        region: None,
        activation: Activation::Press(Button::Mouse(MouseButton::Right)),
    },
];

const fn region(
    key: &'static str,
    title: &'static str,
    region: omnitrigger_core::PointerRegion,
) -> TriggerPreset {
    TriggerPreset {
        key,
        title,
        group: "屏幕区域",
        description: "仅在屏幕外露边角触发，排除多屏连接处；方向可改为下滚，也可重新录制按键。",
        held: &[],
        activation: Activation::Wheel(WheelDirection::Up),
        region: Some(region),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omnitrigger_core::Engine;

    #[test]
    fn quick_drag_preset_preserves_parameters_and_registers_with_stationary_hold() {
        let drag = PRESETS
            .iter()
            .find(|preset| preset.key == "right-quick-drag")
            .unwrap();
        let hold = PRESETS
            .iter()
            .find(|preset| preset.key == "right-300")
            .unwrap();
        let expected = TriggerSpec {
            conditions: Vec::new(),
            id: 42,
            held: vec![],
            activation: Activation::Drag {
                button: Button::Mouse(MouseButton::Right),
                min_distance_px: 18,
                max_start_ms: 180,
            },
        };
        let spec = drag.spec(42);
        assert_eq!(spec, expected);
        assert_eq!(drag.group, "快速拖拽");
        Engine::new(vec![spec.clone(), hold.spec(43)]).unwrap();
        assert_eq!(spec, expected);
    }

    #[test]
    fn every_preset_registers_without_rewriting_its_definition() {
        for preset in PRESETS {
            let spec = preset.spec(42);
            Engine::new(vec![spec.clone()])
                .unwrap_or_else(|error| panic!("{}: {error}", preset.title));
            assert_eq!(spec.activation, preset.activation);
        }
    }
}
