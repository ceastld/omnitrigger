//! 预设只生成 TriggerSpec 草稿，注册与执行继续共享核心语义。
use omnitrigger_core::{Activation, Button, MouseButton, TriggerSpec, WheelDirection};

pub struct TriggerPreset {
    pub key: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub group: &'static str,
    pub held: &'static [Button],
    pub activation: Activation,
}

impl TriggerPreset {
    pub fn spec(&self, id: u64) -> TriggerSpec {
        TriggerSpec {
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
        activation: Activation::Hold {
            button,
            duration_ms,
        },
    }
}

pub const PRESETS: &[TriggerPreset] = &[
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
        activation: Activation::Press(Button::Key(0x20)),
    },
    TriggerPreset {
        key: "ctrl-alt-space",
        title: "左 Ctrl + 左 Alt + Space",
        group: "键盘组合",
        description: "同时按住左侧 Ctrl、Alt，再按空格。",
        held: &[Button::Key(0xa2), Button::Key(0xa4)],
        activation: Activation::Press(Button::Key(0x20)),
    },
    TriggerPreset {
        key: "f8",
        title: "单按 F8",
        group: "单键与鼠标组合",
        description: "按下立即触发，会消费 F8 原本的输入。",
        held: &[],
        activation: Activation::Press(Button::Key(0x77)),
    },
    TriggerPreset {
        key: "side-2",
        title: "单按鼠标侧键 2",
        group: "单键与鼠标组合",
        description: "按下侧键即触发，会消费该键原本的输入。",
        held: &[],
        activation: Activation::Press(Button::Mouse(MouseButton::X2)),
    },
    TriggerPreset {
        key: "side-wheel-up",
        title: "侧键 1 + 滚轮上滚",
        group: "单键与鼠标组合",
        description: "按住侧键 1，再向上滚动一格。",
        held: &[Button::Mouse(MouseButton::X1)],
        activation: Activation::Wheel(WheelDirection::Up),
    },
    TriggerPreset {
        key: "side-wheel-down",
        title: "侧键 1 + 滚轮下滚",
        group: "单键与鼠标组合",
        description: "按住侧键 1，再向下滚动一格。",
        held: &[Button::Mouse(MouseButton::X1)],
        activation: Activation::Wheel(WheelDirection::Down),
    },
    TriggerPreset {
        key: "shift-right",
        title: "左 Shift + 右键",
        group: "单键与鼠标组合",
        description: "按住左 Shift 再按右键，按下时触发。",
        held: &[Button::Key(0xa0)],
        activation: Activation::Press(Button::Mouse(MouseButton::Right)),
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use omnitrigger_core::Engine;

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
