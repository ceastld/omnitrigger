//! 界面配色与可复用组件；主界面采用低饱和纯色，阴影留给浮窗。

use std::cell::Cell;

use crate::{appearance::Mode, assets::Icon};
use gpui::{
    div, linear_color_stop, linear_gradient, point, prelude::*, px, svg, Background, BoxShadow,
    Div, ElementId, FontWeight, Hsla, Rgba, SharedString, Stateful, Svg, WindowAppearance,
};
use omnitrigger_core::{Activation, Button, MouseButton, TriggerSpec, WheelDirection};
use omnitrigger_windows::format_button;

pub const SIDEBAR: u32 = 0x17191f;
pub const BACKGROUND: u32 = 0x1c1e25;
pub const INSPECTOR: u32 = 0x21242d;
pub const ROW: u32 = 0x22252e;
pub const SURFACE: u32 = 0x252832;
pub const SURFACE_HI: u32 = 0x2c303b;
pub const SELECTED: u32 = 0x2b3b3b;
pub const DIVIDER: u32 = 0x2a2e38;
pub const BORDER: u32 = 0x303540;
pub const BORDER_MID: u32 = 0x3a414e;
pub const BORDER_HI: u32 = 0x4b5566;
pub const KEYCAP: u32 = 0x2d333f;
pub const KEYCAP_EDGE: u32 = 0x464e5e;
pub const KEYCAP_BASE: u32 = 0x20242c;
pub const TEXT: u32 = 0xe8eaf0;
pub const TEXT_SOFT: u32 = 0xcdd1da;
pub const TEXT_MUTED: u32 = 0xa5abba;
pub const TEXT_DIM: u32 = 0x9198a6;
pub const TEXT_FAINT: u32 = 0x747d8e;
pub const PLUS: u32 = 0x707888;
pub const ICON: u32 = 0xa0a8b7;

pub const ACCENT: u32 = 0x83c8b5;
pub const ACCENT_2: u32 = 0x96b8da;
pub const ON_ACCENT: u32 = 0x122922;
pub const ACCENT_BG: u32 = 0x263c35;
pub const ACCENT_LINE: u32 = 0x426257;
pub const ACCENT_DEEP: u32 = 0x23372f;
pub const VIOLET: u32 = 0xb0a6d2;
pub const PINK: u32 = 0xd1a3bd;

pub const RECORD: u32 = 0xe3b977;
pub const RECORD_TEXT: u32 = 0xffe5bf;
pub const RECORD_BG: u32 = 0x18120a;
pub const RECORD_LINE: u32 = 0x6b4d1f;
pub const RECORD_CAP: u32 = 0x2d2210;
pub const RECORD_DIM: u32 = 0x9c7c4c;

pub const DANGER: u32 = 0xdc8994;
pub const DANGER_TEXT: u32 = 0xffc8d3;
pub const DANGER_BG: u32 = 0x1e0d13;
pub const DANGER_LINE: u32 = 0x5c2233;
pub const ON_DANGER: u32 = 0x2b0710;

pub const UI_FONT: &str = "Microsoft YaHei UI";
pub const MONO_FONT: &str = "Cascadia Mono";

thread_local! {
    static DARK: Cell<bool> = const { Cell::new(true) };
}

/// 在 UI 线程按用户偏好和当前窗口的系统外观更新调色板。
pub fn apply(mode: Mode, appearance: WindowAppearance) -> bool {
    let dark = mode.is_dark(appearance);
    DARK.with(|current| current.replace(dark) != dark)
}

pub fn is_dark() -> bool {
    DARK.with(Cell::get)
}

/// 原有深色语义 token 在浅色主题中解析为配套颜色；普通自定义颜色原样保留。
pub fn rgb(color: u32) -> Rgba {
    gpui::rgb(resolve_color(color))
}

/// 返回实际 RGB 值，供原生标题栏等不接受 GPUI 颜色的接口使用。
pub fn resolve_color(color: u32) -> u32 {
    if is_dark() {
        color
    } else {
        light_color(color)
    }
}

fn light_color(color: u32) -> u32 {
    match color {
        SIDEBAR => 0xf0f2f5,
        BACKGROUND => 0xf7f8fa,
        INSPECTOR | ROW => 0xffffff,
        SURFACE => 0xf2f4f7,
        SURFACE_HI => 0xe8edf2,
        SELECTED => 0xe3f1eb,
        DIVIDER => 0xe0e5ea,
        BORDER => 0xdbe1e8,
        BORDER_MID => 0xc7d0db,
        BORDER_HI => 0xaab7c7,
        KEYCAP => 0xffffff,
        KEYCAP_EDGE => 0xc5cdd7,
        KEYCAP_BASE => 0xdfe4eb,
        TEXT => 0x202632,
        TEXT_SOFT => 0x354152,
        TEXT_MUTED => 0x556274,
        TEXT_DIM => 0x657285,
        TEXT_FAINT => 0x758293,
        PLUS => 0x7b8796,
        ICON => 0x586779,
        ACCENT => 0x0d755c,
        ACCENT_2 => 0x2f649a,
        ON_ACCENT | ON_DANGER => 0xffffff,
        ACCENT_BG => 0xe2f1ea,
        ACCENT_LINE => 0x9fc6b7,
        ACCENT_DEEP => 0xd7eadf,
        VIOLET => 0x7054a8,
        PINK => 0x9c4f78,
        RECORD => 0xa46916,
        RECORD_TEXT => 0x7e4e0e,
        RECORD_BG => 0xfff4df,
        RECORD_LINE => 0xdab77b,
        RECORD_CAP => 0xffedc7,
        RECORD_DIM => 0x946821,
        DANGER => 0xb54355,
        DANGER_TEXT => 0x9c2d40,
        DANGER_BG => 0xfff0f2,
        DANGER_LINE => 0xe4b1bb,
        // 存量浮窗、编辑器和按钮使用的固定界面色，避免浅色界面残留深色块。
        0x06080d => 0xe5eaf0,
        0x080b12 => 0xedf1f6,
        0x0b0f17 => 0xf2f5f9,
        0x0c0e15 => 0xf1f4f8,
        0x0d121c => 0xf7f9fb,
        0x12141c => 0xe5eaf1,
        0x121824 | 0x121826 => 0xedf1f6,
        0x131620 => 0xe8edf3,
        0x161923 => 0xecf0f5,
        0x171b28 | 0x1a2232 | 0x1a2233 => 0xffffff,
        0x1c2434 => 0xf6f8fb,
        0x353b48 => 0xdce3eb,
        0x6b7287 => 0x8a96a6,
        0x8a93a8 => 0x748194,
        0x8fa3c4 | 0x9fb2d6 => 0x61748f,
        0x9bd6c5 => 0x0a654f,
        0xe9a1aa => 0x9e3546,
        0xb5a0ff => 0x7959bc,
        0xffc36a => 0xa6690f,
        0x5df3bd => 0x148263,
        // 黑色仍用于真实投影，白色仍用于开关和实心按钮上的前景。
        _ => color,
    }
}

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

/// 主要操作使用统一纯色。
pub fn accent_fill(_: f32) -> Background {
    rgb(ACCENT).into()
}

/// 低饱和纯色底，保持相同组件的层级一致。
pub fn tint_fill(color: u32, from: f32, to: f32) -> Background {
    hsla(color, (from + to) * 0.5).into()
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
    let mut caps: Vec<_> = spec
        .conditions
        .iter()
        .map(|condition| Cap {
            label: crate::trigger_conditions::condition_label(condition).into(),
            mouse: true,
            arrow: None,
        })
        .collect();
    caps.extend(caps_for_buttons(&spec.held));
    caps.push(match &spec.activation {
        Activation::Press(button) => button_cap(button),
        Activation::Hold { button, .. } | Activation::Drag { button, .. } => button_cap(button),
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
    if let Activation::Drag {
        min_distance_px,
        max_start_ms,
        ..
    } = spec.activation
    {
        caps.push(Cap {
            label: "快速拖拽".into(),
            mouse: false,
            arrow: None,
        });
        caps.push(Cap {
            label: format!("≥{min_distance_px}px · <{max_start_ms}ms").into(),
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
            Self::Small => (18., 0., 4., 6., 10.5, 10.),
            Self::Medium => (24., 0., 5., 8., 12., 12.),
            Self::Large => (34., 0., 6., 12., 14., 15.),
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
    let (face, line, text) = match tone {
        CapTone::Normal => (KEYCAP, KEYCAP_EDGE, TEXT_SOFT),
        CapTone::Held => (RECORD_CAP, RECORD_LINE, RECORD_TEXT),
        CapTone::Lit => (ACCENT_BG, ACCENT_LINE, TEXT),
    };
    div()
        .flex_none()
        .rounded(px(radius))
        .bg(rgb(face))
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
        .rounded(px(6.))
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
        .rounded(px(6.))
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
        Tone::Primary => el
            .bg(rgb(ACCENT))
            .when(enabled, |el| el.hover(|style| style.bg(rgb(0x9bd6c5)))),
        Tone::Secondary => el
            .bg(rgb(SURFACE_HI))
            .border_1()
            .border_color(rgb(BORDER_MID))
            .when(enabled, |el| {
                el.hover(|style| style.bg(rgb(0x353b48)).border_color(rgb(BORDER_HI)))
            }),
        Tone::Ghost => el.when(enabled, |el| {
            el.hover(|style| style.bg(rgb(SURFACE_HI)).text_color(rgb(TEXT)))
        }),
        Tone::Danger => el.when(enabled, |el| el.hover(|style| style.bg(rgb(DANGER_BG)))),
        Tone::DangerSolid => el
            .bg(rgb(DANGER))
            .when(enabled, |el| el.hover(|style| style.bg(rgb(0xe9a1aa)))),
    }
}

pub fn small(el: Stateful<Div>) -> Stateful<Div> {
    el.h(px(30.)).px(px(11.)).text_size(px(12.)).rounded(px(6.))
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
        .when(on, |el| el.bg(rgb(ACCENT)))
        .when(!on, |el| {
            el.bg(rgb(SURFACE_HI))
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
                .bg(rgb(if on { 0xffffff } else { 0x6b7287 })),
        )
}

pub fn dot(color: u32, size: f32) -> Div {
    div()
        .flex_none()
        .size(px(size))
        .rounded(px(size / 2.))
        .bg(rgb(color))
}

/// 简单状态灯。
pub fn beacon(color: u32, size: f32) -> Div {
    div()
        .flex_none()
        .size(px(size * 2.))
        .rounded(px(size))
        .bg(hsla(color, 0.16))
        .flex()
        .items_center()
        .justify_center()
        .child(dot(color, size))
}

/// 着色方块图标，用于动作类型、统计和空状态。
pub fn icon_tile(glyph: Icon, color: u32, size: f32) -> Div {
    div()
        .flex_none()
        .size(px(size))
        .rounded(px(size * 0.2))
        .bg(hsla(color, 0.1))
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
    pill_base(color).child(dot(color, 6.)).child(label.into())
}

fn pill_base(color: u32) -> Div {
    div()
        .flex_none()
        .h(px(22.))
        .px(px(9.))
        .rounded(px(6.))
        .bg(hsla(color, 0.1))
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
        .bg(rgb(SIDEBAR))
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
            el.bg(rgb(SURFACE_HI))
                .border_1()
                .border_color(rgb(BORDER_HI))
                .text_color(rgb(TEXT))
                .font_weight(FontWeight::SEMIBOLD)
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
        .bg(rgb(ACCENT_BG))
        .flex()
        .items_center()
        .justify_center()
        .child(icon(Icon::Logo, size * 0.66, ACCENT))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_mode_resolves_both_gpui_and_native_colors() {
        let previous = is_dark();
        apply(Mode::Light, WindowAppearance::Dark);
        assert_eq!(resolve_color(BACKGROUND), 0xf7f8fa);
        assert_eq!(rgb(BACKGROUND), gpui::rgb(0xf7f8fa));
        let translucent = hsla(BACKGROUND, 0.4);
        assert_eq!(translucent.a, 0.4);
        apply(Mode::Dark, WindowAppearance::Light);
        assert_eq!(resolve_color(BACKGROUND), BACKGROUND);
        apply(Mode::System, WindowAppearance::VibrantLight);
        assert!(!is_dark());
        apply(Mode::System, WindowAppearance::VibrantDark);
        assert!(is_dark());
        DARK.with(|current| current.set(previous));
    }

    fn contrast(a: u32, b: u32) -> f32 {
        fn luminance(color: u32) -> f32 {
            let linear = |shift| {
                let value = ((color >> shift) & 0xff_u32) as f32 / 255.;
                if value <= 0.04045 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(16) + 0.7152 * linear(8) + 0.0722 * linear(0)
        }
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn light_theme_keeps_text_buttons_and_recording_keycaps_readable() {
        for (foreground, background) in [
            (TEXT, BACKGROUND),
            (TEXT_SOFT, KEYCAP),
            (TEXT_MUTED, INSPECTOR),
            (ON_ACCENT, ACCENT),
            (ON_DANGER, DANGER),
            (DANGER_TEXT, DANGER_BG),
            (RECORD_TEXT, RECORD_CAP),
        ] {
            assert!(
                contrast(light_color(foreground), light_color(background)) >= 4.5,
                "浅色主题颜色对比度不足：{foreground:06x}/{background:06x}"
            );
        }
    }

    #[test]
    fn light_theme_converts_old_surfaces_but_preserves_custom_colors_and_real_shadows() {
        for background in [
            SIDEBAR, BACKGROUND, ROW, KEYCAP, 0x0d121c, 0x171b28, 0x080b12,
        ] {
            let color = light_color(background);
            assert!(((color >> 16) & 0xff) >= 0xe0);
            assert!(((color >> 8) & 0xff) >= 0xe0);
            assert!((color & 0xff) >= 0xe0);
        }
        assert_eq!(light_color(0x000000), 0x000000);
        assert_eq!(light_color(0x123456), 0x123456);
        assert_eq!(light_color(0xffffff), 0xffffff);
    }

    #[test]
    fn drag_caps_preserve_mouse_button_distance_and_strict_start_window() {
        let spec = TriggerSpec {
            conditions: Vec::new(),
            id: 6,
            held: vec![Button::Key(0x14)],
            activation: Activation::Drag {
                button: Button::Mouse(MouseButton::Right),
                min_distance_px: 18,
                max_start_ms: 180,
            },
        };
        let caps = caps_for(&spec);
        assert_eq!(caps.len(), 4);
        assert_eq!(caps[0].label.as_ref(), "CapsLock");
        assert_eq!(caps[1].label.as_ref(), "右键");
        assert!(caps[1].mouse);
        assert_eq!(caps[2].label.as_ref(), "快速拖拽");
        assert_eq!(caps[3].label.as_ref(), "≥18px · <180ms");
        assert!(!caps[2].mouse && !caps[3].mouse);
    }

    #[test]
    fn hold_caps_preserve_the_button_and_configured_duration() {
        let spec = TriggerSpec {
            conditions: Vec::new(),
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
            conditions: Vec::new(),
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
