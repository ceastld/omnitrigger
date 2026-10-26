//! 界面配色与可复用组件：键帽、按钮、开关、图标、分段选择和光晕。

use crate::assets::Icon;
use gpui::{
    div, linear_color_stop, linear_gradient, point, prelude::*, px, rgb, svg, Background,
    BoxShadow, Div, ElementId, FontWeight, Hsla, SharedString, Stateful, Svg,
};
use omnitrigger_core::{Activation, Button, MouseButton, TriggerSpec, WheelDirection};
use omnitrigger_windows::format_button;

pub const SIDEBAR: u32 = 0x0b0c12;
pub const BACKGROUND: u32 = 0x08090d;
pub const INSPECTOR: u32 = 0x0f1118;
pub const ROW: u32 = 0x10121a;
pub const SURFACE: u32 = 0x141722;
pub const SURFACE_HI: u32 = 0x1a1e2b;
pub const SELECTED: u32 = 0x131a22;
pub const DIVIDER: u32 = 0x181b25;
pub const BORDER: u32 = 0x1d2130;
pub const BORDER_MID: u32 = 0x282d3e;
pub const BORDER_HI: u32 = 0x363c51;
pub const KEYCAP_TOP: u32 = 0x323849;
pub const KEYCAP: u32 = 0x222735;
pub const KEYCAP_EDGE: u32 = 0x444b61;
pub const KEYCAP_BASE: u32 = 0x05060a;
pub const TEXT: u32 = 0xeef1f8;
pub const TEXT_SOFT: u32 = 0xc6cbd9;
pub const TEXT_MUTED: u32 = 0x9ca2b6;
pub const TEXT_DIM: u32 = 0x858ba0;
pub const TEXT_FAINT: u32 = 0x6e748a;
pub const PLUS: u32 = 0x545a6e;
pub const ICON: u32 = 0x9aa1b7;

pub const ACCENT: u32 = 0x4ef2b0;
pub const ACCENT_2: u32 = 0x3cc8ff;
pub const ON_ACCENT: u32 = 0x03140d;
pub const ACCENT_BG: u32 = 0x0c1915;
pub const ACCENT_LINE: u32 = 0x1d4a3b;
pub const ACCENT_DEEP: u32 = 0x0f241d;
pub const VIOLET: u32 = 0xa08fff;
pub const PINK: u32 = 0xff7ac6;

pub const RECORD: u32 = 0xffb54a;
pub const RECORD_TEXT: u32 = 0xffe5bf;
pub const RECORD_BG: u32 = 0x18120a;
pub const RECORD_LINE: u32 = 0x6b4d1f;
pub const RECORD_CAP: u32 = 0x2d2210;
pub const RECORD_BASE: u32 = 0x6e4f17;
pub const RECORD_DIM: u32 = 0x9c7c4c;

pub const DANGER: u32 = 0xff5d7a;
pub const DANGER_TEXT: u32 = 0xffc8d3;
pub const DANGER_BG: u32 = 0x1e0d13;
pub const DANGER_LINE: u32 = 0x5c2233;
pub const ON_DANGER: u32 = 0x2b0710;

pub const UI_FONT: &str = "Microsoft YaHei UI";
pub const MONO_FONT: &str = "Cascadia Mono";

pub fn icon(icon: Icon, size: f32, color: u32) -> Svg {
    svg()
        .path(icon.path())
        .size(px(size))
        .flex_none()
        .text_color(rgb(color))
}

pub fn hsla(color: u32, alpha: f32) -> Hsla {
    let mut value: Hsla = rgb(color).into();
    value.a = alpha;
    value
}

/// 两色线性渐变；`angle` 与 CSS 一致，0 指向上方、顺时针增加。
pub fn gradient(angle: f32, from: Hsla, to: Hsla) -> Background {
    linear_gradient(
        angle,
        linear_color_stop(from, 0.),
        linear_color_stop(to, 1.),
    )
}

/// 主强调色：薄荷到天青。
pub fn accent_fill(angle: f32) -> Background {
    gradient(angle, hsla(ACCENT, 1.), hsla(ACCENT_2, 1.))
}

/// 同色系从浓到淡的底色，用于选中、舞台区等高亮区域。
pub fn tint_fill(color: u32, from: f32, to: f32) -> Background {
    gradient(160., hsla(color, from), hsla(color, to))
}

pub fn glow(color: u32, alpha: f32, blur: f32) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: hsla(color, alpha),
        offset: point(px(0.), px(0.)),
        blur_radius: px(blur),
        spread_radius: px(0.),
    }]
}

/// 悬浮卡片的投影。
pub fn lift(alpha: f32) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: hsla(0x000000, alpha),
        offset: point(px(0.), px(12.)),
        blur_radius: px(32.),
        spread_radius: px(-6.),
    }]
}

/// 绝对定位的柔光团，作为背景氛围；不参与布局和命中。
pub fn aurora(color: u32, alpha: f32, radius: f32) -> Div {
    div().absolute().size(px(1.)).shadow(vec![BoxShadow {
        color: hsla(color, alpha),
        offset: point(px(0.), px(0.)),
        blur_radius: px(radius),
        spread_radius: px(radius * 0.45),
    }])
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
        Activation::Hold { button, .. } => button_cap(button),
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
    if let Activation::Hold { duration_ms, .. } = spec.activation {
        caps.push(Cap {
            label: format!("长按 {duration_ms}ms").into(),
            mouse: false,
            arrow: None,
        });
    }
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
            Self::Medium => (24., 2., 6., 9., 12., 12.),
            Self::Large => (40., 3., 10., 15., 16., 17.),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CapTone {
    Normal,
    /// 录制中实时按住的键。
    Held,
    /// 详情舞台上的当前组合。
    Lit,
}

pub fn keycap(cap: &Cap, size: CapSize, tone: CapTone) -> Div {
    let (height, edge, radius, padding, font, icon_size) = size.metrics();
    let (top, face, line, base, text, halo) = match tone {
        CapTone::Normal => (KEYCAP_TOP, KEYCAP, KEYCAP_EDGE, KEYCAP_BASE, TEXT, None),
        CapTone::Held => (
            0x4a3713,
            RECORD_CAP,
            RECORD,
            RECORD_BASE,
            RECORD_TEXT,
            Some(RECORD),
        ),
        CapTone::Lit => (
            0x1d4638,
            0x112a22,
            0x3a9878,
            0x04120d,
            0xdcfff0,
            Some(ACCENT),
        ),
    };
    div()
        .flex_none()
        .rounded(px(radius))
        .bg(rgb(base))
        .pb(px(edge))
        .when_some(halo, |el, color| {
            el.shadow(glow(
                color,
                0.22,
                if size == CapSize::Large { 18. } else { 10. },
            ))
        })
        .child(
            div()
                .h(px(height))
                .px(px(padding))
                .rounded(px(radius))
                .bg(gradient(180., hsla(top, 1.), hsla(face, 1.)))
                .border_1()
                .border_color(rgb(line))
                .flex()
                .items_center()
                .gap(px(icon_size * 0.4))
                .text_size(px(font))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(text))
                .whitespace_nowrap()
                .when(cap.mouse, |el| {
                    el.child(icon(
                        Icon::Mouse,
                        icon_size,
                        if tone == CapTone::Normal { ICON } else { text },
                    ))
                })
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
        .h(px(43.))
        .min_w(px(46.))
        .px(px(12.))
        .rounded(px(10.))
        .border_1()
        .border_dashed()
        .border_color(rgb(RECORD_LINE))
        .bg(hsla(RECORD, 0.04))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(16.))
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
    combo_toned(caps, size, CapTone::Normal)
}

pub fn combo_toned(caps: &[Cap], size: CapSize, tone: CapTone) -> Div {
    let gap = if size == CapSize::Large { 9. } else { 5. };
    let mut row = div().flex().flex_wrap().items_center().gap(px(gap));
    for (index, cap) in caps.iter().enumerate() {
        if index > 0 {
            row = row.child(plus(size, PLUS));
        }
        row = row.child(keycap(cap, size, tone));
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

/// 36px 高的标准按钮；`enabled` 为 false 时降低不透明度且不响应悬停。
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

/// 只有图标的方形按钮。
pub fn glyph_button(
    id: impl Into<ElementId>,
    glyph: Icon,
    tone: Tone,
    enabled: bool,
) -> Stateful<Div> {
    button_base(id, Some(glyph), tone, enabled)
        .w(px(36.))
        .px(px(0.))
}

fn button_base(
    id: impl Into<ElementId>,
    glyph: Option<Icon>,
    tone: Tone,
    enabled: bool,
) -> Stateful<Div> {
    // 禁用的实心按钮退为次要样式，避免半透明渐变发浑。
    let tone = match tone {
        Tone::Primary | Tone::DangerSolid if !enabled => Tone::Secondary,
        tone => tone,
    };
    let fg = match tone {
        Tone::Primary => ON_ACCENT,
        Tone::Secondary => TEXT,
        Tone::Ghost => TEXT_MUTED,
        Tone::Danger => DANGER,
        Tone::DangerSolid => ON_DANGER,
    };
    let el = div()
        .id(id)
        .flex_none()
        .h(px(36.))
        .px(px(15.))
        .rounded(px(10.))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(7.))
        .text_size(px(13.))
        .text_color(rgb(fg))
        .whitespace_nowrap()
        .when(matches!(tone, Tone::Primary | Tone::DangerSolid), |el| {
            el.font_weight(FontWeight::SEMIBOLD)
        })
        .when(!enabled, |el| el.opacity(0.4))
        .when(enabled, |el| el.cursor_pointer())
        .when_some(glyph, |el, glyph| el.child(icon(glyph, 14., fg)));
    match tone {
        Tone::Primary => el.bg(accent_fill(100.)).when(enabled, |el| {
            el.shadow(glow(ACCENT, 0.28, 16.))
                .hover(|style| style.bg(gradient(100., hsla(0x7cf7c9, 1.), hsla(0x6ad8ff, 1.))))
        }),
        Tone::Secondary => el
            .bg(rgb(SURFACE_HI))
            .border_1()
            .border_color(rgb(BORDER_MID))
            .when(enabled, |el| {
                el.hover(|style| style.bg(rgb(0x232838)).border_color(rgb(BORDER_HI)))
            }),
        Tone::Ghost => el.when(enabled, |el| {
            el.hover(|style| style.bg(rgb(SURFACE_HI)).text_color(rgb(TEXT)))
        }),
        Tone::Danger => el.when(enabled, |el| el.hover(|style| style.bg(rgb(DANGER_BG)))),
        Tone::DangerSolid => el.bg(rgb(DANGER)).when(enabled, |el| {
            el.shadow(glow(DANGER, 0.3, 16.))
                .hover(|style| style.bg(rgb(0xff7890)))
        }),
    }
}

pub fn small(el: Stateful<Div>) -> Stateful<Div> {
    el.h(px(30.)).px(px(11.)).text_size(px(12.)).rounded(px(8.))
}

pub fn switch(id: impl Into<ElementId>, on: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .w(px(40.))
        .h(px(22.))
        .rounded(px(11.))
        .cursor_pointer()
        .relative()
        .when(on, |el| {
            el.bg(accent_fill(90.)).shadow(glow(ACCENT, 0.35, 12.))
        })
        .when(!on, |el| {
            el.bg(rgb(0x1f2330))
                .border_1()
                .border_color(rgb(BORDER_MID))
        })
        .child(
            div()
                .absolute()
                .top(px(if on { 3. } else { 2. }))
                .left(px(if on { 21. } else { 2. }))
                .size(px(16.))
                .rounded(px(8.))
                .bg(rgb(if on { 0xffffff } else { 0x6b7287 }))
                .when(on, |el| el.shadow(lift(0.5))),
        )
}

pub fn dot(color: u32, size: f32) -> Div {
    div()
        .flex_none()
        .size(px(size))
        .rounded(px(size / 2.))
        .bg(rgb(color))
}

/// 带光环的状态灯。
pub fn beacon(color: u32, size: f32) -> Div {
    div()
        .flex_none()
        .size(px(size * 2.))
        .rounded(px(size))
        .bg(hsla(color, 0.16))
        .flex()
        .items_center()
        .justify_center()
        .child(dot(color, size).shadow(glow(color, 0.8, size * 1.5)))
}

/// 着色方块图标，用于动作类型、统计和空状态。
pub fn icon_tile(glyph: Icon, color: u32, size: f32) -> Div {
    div()
        .flex_none()
        .size(px(size))
        .rounded(px(size * 0.3))
        .bg(tint_fill(color, 0.2, 0.06))
        .border_1()
        .border_color(hsla(color, 0.28))
        .flex()
        .items_center()
        .justify_center()
        .child(icon(glyph, size * 0.48, color))
}

/// 小号胶囊标签。
pub fn pill(label: impl Into<SharedString>, color: u32) -> Div {
    pill_base(color).child(label.into())
}

/// 状态胶囊：带状态灯。
pub fn status_pill(label: impl Into<SharedString>, color: u32) -> Div {
    pill_base(color)
        .child(dot(color, 6.).shadow(glow(color, 0.8, 6.)))
        .child(label.into())
}

fn pill_base(color: u32) -> Div {
    div()
        .flex_none()
        .h(px(22.))
        .px(px(9.))
        .rounded(px(11.))
        .bg(hsla(color, 0.1))
        .border_1()
        .border_color(hsla(color, 0.28))
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(color))
        .whitespace_nowrap()
}

/// 分段选择的外框；子项用 [`segment`]。
pub fn segments() -> Div {
    div()
        .flex()
        .p(px(3.))
        .gap(px(3.))
        .rounded(px(11.))
        .bg(rgb(0x0a0b11))
        .border_1()
        .border_color(rgb(BORDER))
}

pub fn segment(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    active: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(30.))
        .px(px(12.))
        .rounded(px(8.))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(12.))
        .whitespace_nowrap()
        .cursor_pointer()
        .when(active, |el| {
            el.bg(gradient(180., hsla(0x262b3b, 1.), hsla(SURFACE_HI, 1.)))
                .border_1()
                .border_color(rgb(BORDER_HI))
                .text_color(rgb(TEXT))
                .font_weight(FontWeight::SEMIBOLD)
                .shadow(lift(0.5))
        })
        .when(!active, |el| {
            el.text_color(rgb(TEXT_DIM))
                .hover(|style| style.text_color(rgb(TEXT_SOFT)).bg(rgb(0x12141c)))
        })
        .child(label.into())
}

pub fn logo(size: f32) -> Div {
    div()
        .flex_none()
        .size(px(size))
        .rounded(px(size * 0.3))
        .bg(gradient(160., hsla(0x184536, 1.), hsla(0x08130f, 1.)))
        .border_1()
        .border_color(hsla(ACCENT, 0.35))
        .shadow(glow(ACCENT, 0.25, size * 0.6))
        .flex()
        .items_center()
        .justify_center()
        .child(icon(Icon::Logo, size * 0.66, ACCENT))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hold_caps_preserve_the_button_and_configured_duration() {
        let spec = TriggerSpec {
            id: 5,
            held: vec![],
            activation: Activation::Hold {
                button: Button::Mouse(MouseButton::Right),
                duration_ms: 300,
            },
        };
        let caps = caps_for(&spec);
        assert_eq!(caps.len(), 2);
        assert_eq!(caps[0].label.as_ref(), "右键");
        assert!(caps[0].mouse);
        assert_eq!(caps[1].label.as_ref(), "长按 300ms");
        assert!(!caps[1].mouse);
    }

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
