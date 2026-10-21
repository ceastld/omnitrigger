//! 界面配色与可复用组件：键帽、按钮、开关和图标。

use crate::assets::Icon;
use gpui::{
    div, prelude::*, px, rgb, svg, Div, ElementId, FontWeight, Hsla, SharedString, Stateful, Svg,
};
use omnitrigger_core::{Activation, Button, MouseButton, TriggerSpec, WheelDirection};
use omnitrigger_windows::format_button;

pub const SIDEBAR: u32 = 0x0b0d10;
pub const BACKGROUND: u32 = 0x0f1115;
pub const INSPECTOR: u32 = 0x12151a;
pub const ROW: u32 = 0x14171c;
pub const SURFACE: u32 = 0x161a20;
pub const SURFACE_HI: u32 = 0x1c2129;
pub const SELECTED: u32 = 0x1a1f27;
pub const DIVIDER: u32 = 0x1d2129;
pub const BORDER: u32 = 0x1f242c;
pub const BORDER_MID: u32 = 0x2c333e;
pub const BORDER_HI: u32 = 0x343c48;
pub const KEYCAP: u32 = 0x232933;
pub const KEYCAP_EDGE: u32 = 0x39414e;
pub const KEYCAP_BASE: u32 = 0x0a0c10;
pub const TEXT: u32 = 0xe7eaf0;
pub const TEXT_SOFT: u32 = 0xc3cad5;
pub const TEXT_MUTED: u32 = 0xa3abb9;
pub const TEXT_DIM: u32 = 0x8c95a4;
pub const TEXT_FAINT: u32 = 0x7d8696;
pub const PLUS: u32 = 0x5f6876;
pub const ICON: u32 = 0x9aa3b2;

pub const ACCENT: u32 = 0x7ddfb0;
pub const ON_ACCENT: u32 = 0x0b1f16;
pub const ACCENT_BG: u32 = 0x111a16;
pub const ACCENT_LINE: u32 = 0x2a4237;
pub const ACCENT_DEEP: u32 = 0x15241d;

pub const RECORD: u32 = 0xf2b366;
pub const RECORD_TEXT: u32 = 0xffe3bf;
pub const RECORD_BG: u32 = 0x18140f;
pub const RECORD_LINE: u32 = 0x6b5233;
pub const RECORD_CAP: u32 = 0x2e2416;
pub const RECORD_BASE: u32 = 0x7a5520;
pub const RECORD_DIM: u32 = 0x8a6b45;

pub const DANGER: u32 = 0xf2777a;
pub const DANGER_TEXT: u32 = 0xffc9cb;
pub const DANGER_BG: u32 = 0x1d1214;
pub const DANGER_LINE: u32 = 0x5c2b31;
pub const ON_DANGER: u32 = 0x2a0f11;

pub const UI_FONT: &str = "Microsoft YaHei UI";
pub const MONO_FONT: &str = "Cascadia Mono";

pub fn icon(icon: Icon, size: f32, color: u32) -> Svg {
    svg()
        .path(icon.path())
        .size(px(size))
        .flex_none()
        .text_color(rgb(color))
}

/// 键帽上显示的一个按键；鼠标按键带图标，滚轮带方向箭头。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cap {
    pub label: SharedString,
    pub mouse: bool,
    pub arrow: Option<Icon>,
}

fn button_cap(button: &Button) -> Cap {
    match button {
        Button::Mouse(mouse) => Cap {
            label: match mouse {
                MouseButton::Left => "左键",
                MouseButton::Right => "右键",
                MouseButton::Middle => "中键",
                MouseButton::X1 => "侧键 1",
                MouseButton::X2 => "侧键 2",
            }
            .into(),
            mouse: true,
            arrow: None,
        },
        Button::Key(_) => Cap {
            label: format_button(button).into(),
            mouse: false,
            arrow: None,
        },
    }
}

pub fn caps_for_buttons(buttons: &[Button]) -> Vec<Cap> {
    buttons.iter().map(button_cap).collect()
}

pub fn caps_for(spec: &TriggerSpec) -> Vec<Cap> {
    let mut caps = caps_for_buttons(&spec.held);
    caps.push(match &spec.activation {
        Activation::Press(button) => button_cap(button),
        Activation::Wheel(direction) => Cap {
            label: "滚轮".into(),
            mouse: true,
            arrow: Some(match direction {
                WheelDirection::Up => Icon::ArrowUp,
                WheelDirection::Down => Icon::ArrowDown,
                WheelDirection::Left => Icon::ArrowLeft,
                WheelDirection::Right => Icon::ArrowRight,
            }),
        },
    });
    caps
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CapSize {
    Small,
    Medium,
    Large,
}

impl CapSize {
    /// (面高, 底边, 圆角, 水平内边距, 字号, 图标)
    fn metrics(self) -> (f32, f32, f32, f32, f32, f32) {
        match self {
            Self::Small => (20., 2., 5., 7., 11., 11.),
            Self::Medium => (22., 2., 6., 8., 12., 12.),
            Self::Large => (37., 3., 9., 14., 15., 16.),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CapTone {
    Normal,
    Held,
}

pub fn keycap(cap: &Cap, size: CapSize, tone: CapTone) -> Div {
    let (height, edge, radius, padding, font, icon_size) = size.metrics();
    let (face, line, base, text) = match tone {
        CapTone::Normal => (KEYCAP, KEYCAP_EDGE, KEYCAP_BASE, TEXT),
        CapTone::Held => (RECORD_CAP, RECORD, RECORD_BASE, RECORD_TEXT),
    };
    div()
        .flex_none()
        .rounded(px(radius))
        .bg(rgb(base))
        .pb(px(edge))
        .child(
            div()
                .h(px(height))
                .px(px(padding))
                .rounded(px(radius))
                .bg(rgb(face))
                .border_1()
                .border_color(rgb(line))
                .flex()
                .items_center()
                .gap(px(icon_size * 0.4))
                .text_size(px(font))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(text))
                .whitespace_nowrap()
                .when(cap.mouse, |el| el.child(icon(Icon::Mouse, icon_size, ICON)))
                .child(cap.label.clone())
                .when_some(cap.arrow, |el, arrow| {
                    el.child(icon(arrow, icon_size, ACCENT))
                }),
        )
}

/// 录制时尚未确定的目标键占位。
pub fn ghost_cap() -> Div {
    div()
        .flex_none()
        .h(px(40.))
        .min_w(px(44.))
        .px(px(12.))
        .rounded(px(9.))
        .border_1()
        .border_dashed()
        .border_color(rgb(RECORD_LINE))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(15.))
        .text_color(rgb(RECORD_DIM))
        .child("?")
}

pub fn plus(size: CapSize, color: u32) -> Div {
    div()
        .flex_none()
        .text_size(px(match size {
            CapSize::Small => 11.,
            CapSize::Medium => 12.,
            CapSize::Large => 16.,
        }))
        .text_color(rgb(color))
        .child("+")
}

pub fn combo(caps: &[Cap], size: CapSize) -> Div {
    let gap = if size == CapSize::Large { 8. } else { 5. };
    let mut row = div().flex().flex_wrap().items_center().gap(px(gap));
    for (index, cap) in caps.iter().enumerate() {
        if index > 0 {
            row = row.child(plus(size, PLUS));
        }
        row = row.child(keycap(cap, size, CapTone::Normal));
    }
    row
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Primary,
    Secondary,
    Ghost,
    Danger,
    DangerSolid,
}

/// 34px 高的标准按钮；`enabled` 为 false 时降低不透明度且不响应悬停。
pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    tone: Tone,
    enabled: bool,
) -> Stateful<Div> {
    button_base(id, None, tone, enabled).child(label.into())
}

/// 图标在文字前的按钮，图标颜色跟随按钮文字。
pub fn icon_button(
    id: impl Into<ElementId>,
    glyph: Icon,
    label: impl Into<SharedString>,
    tone: Tone,
    enabled: bool,
) -> Stateful<Div> {
    button_base(id, Some(glyph), tone, enabled).child(label.into())
}

fn button_base(
    id: impl Into<ElementId>,
    glyph: Option<Icon>,
    tone: Tone,
    enabled: bool,
) -> Stateful<Div> {
    let (bg, fg, line, hover): (u32, u32, Option<u32>, u32) = match tone {
        Tone::Primary => (ACCENT, ON_ACCENT, None, 0x96e8c1),
        Tone::Secondary => (SURFACE_HI, TEXT, Some(BORDER_MID), 0x262c36),
        Tone::Ghost => (BACKGROUND, TEXT_MUTED, None, SURFACE_HI),
        Tone::Danger => (BACKGROUND, DANGER, None, DANGER_BG),
        Tone::DangerSolid => (DANGER, ON_DANGER, None, 0xf58d90),
    };
    let transparent = matches!(tone, Tone::Ghost | Tone::Danger);
    div()
        .id(id)
        .flex_none()
        .h(px(34.))
        .px(px(14.))
        .rounded(px(8.))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .text_size(px(13.))
        .text_color(rgb(fg))
        .when(matches!(tone, Tone::Primary | Tone::DangerSolid), |el| {
            el.font_weight(FontWeight::SEMIBOLD)
        })
        .when(!transparent, |el| el.bg(rgb(bg)))
        .when_some(line, |el, line| el.border_1().border_color(rgb(line)))
        .when(enabled, |el| {
            el.cursor_pointer().hover(move |style| style.bg(rgb(hover)))
        })
        .when(!enabled, |el| el.opacity(0.45))
        .when_some(glyph, |el, glyph| el.child(icon(glyph, 13., fg)))
}

pub fn small(el: Stateful<Div>) -> Stateful<Div> {
    el.h(px(28.)).px(px(10.)).text_size(px(12.)).rounded(px(7.))
}

pub fn switch(id: impl Into<ElementId>, on: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .w(px(36.))
        .h(px(20.))
        .rounded(px(10.))
        .cursor_pointer()
        .bg(rgb(if on { ACCENT } else { 0x2a303a }))
        .relative()
        .child(
            div()
                .absolute()
                .top(px(2.))
                .left(px(if on { 18. } else { 2. }))
                .size(px(16.))
                .rounded(px(8.))
                .bg(rgb(if on { ON_ACCENT } else { TEXT_FAINT })),
        )
}

pub fn dot(color: u32, size: f32) -> Div {
    div()
        .flex_none()
        .size(px(size))
        .rounded(px(size / 2.))
        .bg(rgb(color))
}

pub fn hsla(color: u32, alpha: f32) -> Hsla {
    let mut value: Hsla = rgb(color).into();
    value.a = alpha;
    value
}

pub fn logo(size: f32) -> Div {
    div()
        .flex_none()
        .size(px(size))
        .rounded(px(size * 0.27))
        .bg(rgb(ACCENT_DEEP))
        .border_1()
        .border_color(rgb(ACCENT_LINE))
        .flex()
        .items_center()
        .justify_center()
        .child(icon(Icon::Logo, size * 0.72, ACCENT))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wheel_and_mouse_caps_share_one_description_with_trigger_spec() {
        let spec = TriggerSpec {
            id: 1,
            held: vec![Button::Mouse(MouseButton::X1), Button::Key(0x14)],
            activation: Activation::Wheel(WheelDirection::Down),
        };
        let caps = caps_for(&spec);
        assert_eq!(caps.len(), 3);
        assert_eq!(caps[0].label.as_ref(), "侧键 1");
        assert!(caps[0].mouse);
        assert_eq!(caps[1].label.as_ref(), "CapsLock");
        assert!(!caps[1].mouse);
        assert_eq!(caps[2].arrow, Some(Icon::ArrowDown));
    }
}
