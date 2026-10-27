//! 鼠标位置条件的配置与编辑器；实际匹配、区域相交和校验共用核心模型。
use crate::{caption, section_title, small, theme, MessageInput, Workspace};
use gpui::{div, prelude::*, px, rgb, App, Context, Div, Entity};
use omnitrigger_core::{
    PointerRegion, ScreenCorner, ScreenEdge, ScreenRect, TriggerCondition, TriggerSpec,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConditionDto {
    PointerRegion { region: RegionDto },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RegionDto {
    ScreenEdge {
        edge: EdgeDto,
        thickness_px: u32,
    },
    ScreenCorner {
        corner: CornerDto,
        size_px: u32,
    },
    Rectangle {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeDto {
    Top,
    Right,
    Bottom,
    Left,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CornerDto {
    TopLeft,
    TopRight,
    BottomRight,
    BottomLeft,
}

impl From<&TriggerCondition> for ConditionDto {
    fn from(condition: &TriggerCondition) -> Self {
        let TriggerCondition::PointerRegion(region) = condition;
        Self::PointerRegion {
            region: match *region {
                PointerRegion::ScreenEdge { edge, thickness_px } => RegionDto::ScreenEdge {
                    edge: match edge {
                        ScreenEdge::Top => EdgeDto::Top,
                        ScreenEdge::Right => EdgeDto::Right,
                        ScreenEdge::Bottom => EdgeDto::Bottom,
                        ScreenEdge::Left => EdgeDto::Left,
                    },
                    thickness_px,
                },
                PointerRegion::ScreenCorner { corner, size_px } => RegionDto::ScreenCorner {
                    corner: match corner {
                        ScreenCorner::TopLeft => CornerDto::TopLeft,
                        ScreenCorner::TopRight => CornerDto::TopRight,
                        ScreenCorner::BottomRight => CornerDto::BottomRight,
                        ScreenCorner::BottomLeft => CornerDto::BottomLeft,
                    },
                    size_px,
                },
                PointerRegion::Rectangle(rect) => RegionDto::Rectangle {
                    left: rect.left,
                    top: rect.top,
                    right: rect.right,
                    bottom: rect.bottom,
                },
            },
        }
    }
}
impl From<ConditionDto> for TriggerCondition {
    fn from(condition: ConditionDto) -> Self {
        let ConditionDto::PointerRegion { region } = condition;
        Self::PointerRegion(match region {
            RegionDto::ScreenEdge { edge, thickness_px } => PointerRegion::ScreenEdge {
                edge: match edge {
                    EdgeDto::Top => ScreenEdge::Top,
                    EdgeDto::Right => ScreenEdge::Right,
                    EdgeDto::Bottom => ScreenEdge::Bottom,
                    EdgeDto::Left => ScreenEdge::Left,
                },
                thickness_px,
            },
            RegionDto::ScreenCorner { corner, size_px } => PointerRegion::ScreenCorner {
                corner: match corner {
                    CornerDto::TopLeft => ScreenCorner::TopLeft,
                    CornerDto::TopRight => ScreenCorner::TopRight,
                    CornerDto::BottomRight => ScreenCorner::BottomRight,
                    CornerDto::BottomLeft => ScreenCorner::BottomLeft,
                },
                size_px,
            },
            RegionDto::Rectangle {
                left,
                top,
                right,
                bottom,
            } => PointerRegion::Rectangle(ScreenRect {
                left,
                top,
                right,
                bottom,
            }),
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RegionChoice {
    Any,
    TopLeft,
    Top,
    TopRight,
    Left,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
    Rectangle,
}
impl RegionChoice {
    fn label(self) -> &'static str {
        match self {
            Self::Any => "不限位置",
            Self::TopLeft => "左上角",
            Self::Top => "顶部边缘",
            Self::TopRight => "右上角",
            Self::Left => "左侧边缘",
            Self::Right => "右侧边缘",
            Self::BottomLeft => "左下角",
            Self::Bottom => "底部边缘",
            Self::BottomRight => "右下角",
            Self::Rectangle => "自定义范围",
        }
    }
    fn edge(self) -> Option<ScreenEdge> {
        match self {
            Self::Top => Some(ScreenEdge::Top),
            Self::Right => Some(ScreenEdge::Right),
            Self::Bottom => Some(ScreenEdge::Bottom),
            Self::Left => Some(ScreenEdge::Left),
            _ => None,
        }
    }
    fn corner(self) -> Option<ScreenCorner> {
        match self {
            Self::TopLeft => Some(ScreenCorner::TopLeft),
            Self::TopRight => Some(ScreenCorner::TopRight),
            Self::BottomRight => Some(ScreenCorner::BottomRight),
            Self::BottomLeft => Some(ScreenCorner::BottomLeft),
            _ => None,
        }
    }
}

pub struct ConditionEditor {
    choice: RegionChoice,
    extent: Entity<MessageInput>,
    rectangle: Entity<MessageInput>,
}
impl ConditionEditor {
    pub fn new(cx: &mut Context<Workspace>, spec: Option<&TriggerSpec>) -> Self {
        let extent = cx.new(|cx| {
            let mut input = MessageInput::new(cx).with_character_limit(5);
            input.set_text("8", cx);
            input
        });
        let rectangle = cx.new(|cx| {
            let mut input = MessageInput::new(cx).with_character_limit(80);
            input.set_text("0, 0, 200, 200", cx);
            input
        });
        let mut editor = Self {
            choice: RegionChoice::Any,
            extent,
            rectangle,
        };
        for input in [&editor.extent, &editor.rectangle] {
            cx.observe(input, |_, _, cx| cx.notify()).detach();
        }
        if let Some(spec) = spec {
            editor.load(spec, cx);
        }
        editor
    }
    pub fn load(&mut self, spec: &TriggerSpec, cx: &mut Context<Workspace>) {
        self.choice = RegionChoice::Any;
        if let Some(TriggerCondition::PointerRegion(region)) = spec.conditions.first() {
            match *region {
                PointerRegion::ScreenEdge { edge, thickness_px } => {
                    self.choice = match edge {
                        ScreenEdge::Top => RegionChoice::Top,
                        ScreenEdge::Right => RegionChoice::Right,
                        ScreenEdge::Bottom => RegionChoice::Bottom,
                        ScreenEdge::Left => RegionChoice::Left,
                    };
                    self.extent
                        .update(cx, |input, cx| input.set_text(thickness_px.to_string(), cx));
                }
                PointerRegion::ScreenCorner { corner, size_px } => {
                    self.choice = match corner {
                        ScreenCorner::TopLeft => RegionChoice::TopLeft,
                        ScreenCorner::TopRight => RegionChoice::TopRight,
                        ScreenCorner::BottomRight => RegionChoice::BottomRight,
                        ScreenCorner::BottomLeft => RegionChoice::BottomLeft,
                    };
                    self.extent
                        .update(cx, |input, cx| input.set_text(size_px.to_string(), cx));
                }
                PointerRegion::Rectangle(rect) => {
                    self.choice = RegionChoice::Rectangle;
                    self.rectangle.update(cx, |input, cx| {
                        input.set_text(
                            format!(
                                "{}, {}, {}, {}",
                                rect.left, rect.top, rect.right, rect.bottom
                            ),
                            cx,
                        )
                    });
                }
            }
        }
    }
    pub fn conditions(&self, cx: &App) -> Result<Vec<TriggerCondition>, String> {
        if self.choice == RegionChoice::Any {
            return Ok(Vec::new());
        }
        let region = if self.choice == RegionChoice::Rectangle {
            let values = self
                .rectangle
                .read(cx)
                .text()
                .split(',')
                .map(|part| part.trim().parse::<i32>())
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| "范围应为四个整数：左, 上, 右, 下。")?;
            let [left, top, right, bottom] = values.as_slice() else {
                return Err("范围需要左、上、右、下四个坐标。".into());
            };
            PointerRegion::Rectangle(ScreenRect {
                left: *left,
                top: *top,
                right: *right,
                bottom: *bottom,
            })
        } else {
            let value = self
                .extent
                .read(cx)
                .text()
                .trim()
                .parse::<u32>()
                .map_err(|_| "区域范围应为正整数物理像素。")?;
            if let Some(edge) = self.choice.edge() {
                PointerRegion::ScreenEdge {
                    edge,
                    thickness_px: value,
                }
            } else {
                PointerRegion::ScreenCorner {
                    corner: self.choice.corner().expect("角落选项"),
                    size_px: value,
                }
            }
        };
        region.validate().map_err(|error| error.to_string())?;
        Ok(vec![TriggerCondition::PointerRegion(region)])
    }
}

pub fn condition_label(condition: &TriggerCondition) -> String {
    let TriggerCondition::PointerRegion(region) = condition;
    match *region {
        PointerRegion::ScreenEdge { edge, thickness_px } => format!(
            "{} {thickness_px}px",
            match edge {
                ScreenEdge::Top => "顶部边缘",
                ScreenEdge::Right => "右侧边缘",
                ScreenEdge::Bottom => "底部边缘",
                ScreenEdge::Left => "左侧边缘",
            }
        ),
        PointerRegion::ScreenCorner { corner, size_px } => format!(
            "{} {size_px}px",
            match corner {
                ScreenCorner::TopLeft => "左上角",
                ScreenCorner::TopRight => "右上角",
                ScreenCorner::BottomRight => "右下角",
                ScreenCorner::BottomLeft => "左下角",
            }
        ),
        PointerRegion::Rectangle(rect) => format!(
            "范围 ({}, {})–({}, {})",
            rect.left, rect.top, rect.right, rect.bottom
        ),
    }
}

impl Workspace {
    pub(super) fn condition_fields(&self, cx: &mut Context<Self>) -> Div {
        let mut grid = div().grid().grid_cols(3).gap(px(5.));
        for (index, choice) in [
            RegionChoice::TopLeft,
            RegionChoice::Top,
            RegionChoice::TopRight,
            RegionChoice::Left,
            RegionChoice::Any,
            RegionChoice::Right,
            RegionChoice::BottomLeft,
            RegionChoice::Bottom,
            RegionChoice::BottomRight,
        ]
        .into_iter()
        .enumerate()
        {
            let active = self.condition_editor.choice == choice;
            grid = grid.child(
                small(crate::button(
                    ("pointer-region", index as u64),
                    choice.label(),
                    if active {
                        crate::Tone::Primary
                    } else {
                        crate::Tone::Secondary
                    },
                    !self.recording,
                ))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.recording {
                        return;
                    }
                    this.condition_editor.choice = choice;
                    if choice != RegionChoice::Any {
                        this.condition_editor.extent.update(cx, |input, cx| {
                            input.set_text(if choice.edge().is_some() { "8" } else { "32" }, cx)
                        });
                    }
                    cx.notify();
                })),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(9.))
            .child(section_title("鼠标位置条件"))
            .child(grid)
            .child(
                small(crate::button(
                    "custom-pointer-region",
                    "自定义范围",
                    if self.condition_editor.choice == RegionChoice::Rectangle {
                        crate::Tone::Primary
                    } else {
                        crate::Tone::Secondary
                    },
                    !self.recording,
                ))
                .on_click(cx.listener(|this, _, _, cx| {
                    if this.recording {
                        return;
                    }
                    this.condition_editor.choice = RegionChoice::Rectangle;
                    cx.notify();
                })),
            )
            .when(self.condition_editor.choice != RegionChoice::Any, |el| {
                if self.condition_editor.choice == RegionChoice::Rectangle {
                    el.child(Self::input_field(
                        "物理坐标：左, 上, 右, 下",
                        &self.condition_editor.rectangle,
                        Some("左上包含，右下不包含；支持负坐标。"),
                    ))
                } else {
                    el.child(Self::input_field(
                        if self.condition_editor.choice.edge().is_some() {
                            "边缘厚度（物理像素）"
                        } else {
                            "角落边长（物理像素）"
                        },
                        &self.condition_editor.extent,
                        Some("边缘 1–256px，角落 1–1024px；范围不超过对应半屏。"),
                    ))
                }
            })
            .child(caption(
                "位置条件可与滚轮、按键、长按和快拖组合；屏幕边角会排除多屏相接的共享边段。",
            ))
            .when_some(self.condition_editor.conditions(cx).err(), |el, error| {
                el.child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(theme::DANGER))
                        .child(error),
                )
            })
    }
}
