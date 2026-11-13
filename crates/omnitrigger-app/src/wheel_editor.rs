//! 自定义轮盘的管理页：轮盘本身就是编辑器，点格子添加或修改动作；保存与验证由应用层统一处理。
use crate::theme::rgb;
use crate::wheel_workspace::{
    ring_geometry, slot_at, view_geometry, Preset, WheelScope, RING_NAMES, WHEEL_PRESETS,
};
use crate::{
    button, caption, icon, small, theme,
    wheels::{WheelAccent, WheelDefinition},
    ActionSpec, Context, Div, Icon, Page, Tone, Workspace,
};
use gpui::{
    deferred, div, prelude::*, px, FontWeight, MouseButton, MouseDownEvent, MouseMoveEvent,
    Stateful,
};
use std::{cell::Cell, rc::Rc};

/// 编辑页轮盘舞台的边长范围（逻辑像素），按窗口高度在其间取值。
const STAGE_RANGE: (f32, f32) = (340., 540.);
/// 右侧面板宽度；面板文字保持单行短句，避免滚动列中自动换行被量高。
const PANEL_WIDTH: f32 = 360.;
/// 编辑页的分隔线、说明文字与色块，与轮盘舞台的平面配色一致。
const LINE: u32 = 0x22262e;
const MUTED: u32 = 0x8b93a1;
const FAINT: u32 = 0x6f7785;
const TILE: u32 = 0x1b1f26;
const TILE_EDGE: u32 = 0x2f3540;

fn accent_color(accent: WheelAccent) -> u32 {
    match accent {
        WheelAccent::Mint => theme::ACCENT,
        WheelAccent::Violet => theme::VIOLET,
        WheelAccent::Amber => theme::RECORD,
    }
}

fn action_text(action: &ActionSpec) -> String {
    match action {
        ActionSpec::Notification => "显示提示".into(),
        ActionSpec::SendShortcut(shortcut) => format!("发送 {}", shortcut.display()),
        ActionSpec::Window(action) => format!("窗口 · {}", action.display()),
        ActionSpec::AdjustSystem(action) => action.display(),
        ActionSpec::Local(action) => action.summary(),
        ActionSpec::Invoke(call) => format!("动作包 · {} / {}", call.package_id, call.action_id),
        ActionSpec::Browser(action) => format!("浏览器 · {}", action.label()),
        ActionSpec::External(action) => format!("外部动作 · 依赖 {}", action.provider),
        ActionSpec::Command(_) => "运行程序 / 脚本".into(),
        ActionSpec::Plugin(_) => "插件动作".into(),
        ActionSpec::ShowPanel => "打开触发面板".into(),
        ActionSpec::ShowWheel(_) => "打开轮盘".into(),
    }
}

/// 格子的大致方向，0 格在正上方顺时针排列；只用于提示，不显示内部编号。
fn direction_name(sector: u8, (sectors, half_step): (u8, bool)) -> &'static str {
    const NAMES: [&str; 8] = [
        "正上", "右上", "正右", "右下", "正下", "左下", "正左", "左上",
    ];
    let turn = (f32::from(sector) + if half_step { 0.5 } else { 0. }) / f32::from(sectors);
    NAMES[(turn * 8.).round() as usize % 8]
}

/// 下划线文字按钮，用于次要操作。
fn link(id: impl Into<gpui::ElementId>, label: impl Into<gpui::SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .text_size(px(12.))
        .text_color(rgb(theme::TEXT_DIM))
        .underline()
        .cursor_pointer()
        .hover(|style| style.text_color(rgb(theme::TEXT_SOFT)))
        .child(label.into())
}

fn panel_title(text: impl Into<gpui::SharedString>) -> Div {
    div()
        .text_size(px(20.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(text.into())
}

fn label(text: impl Into<gpui::SharedString>) -> Div {
    div()
        .text_size(px(13.))
        .text_color(rgb(MUTED))
        .child(text.into())
}

fn divider() -> Div {
    div().h(px(1.)).bg(rgb(LINE))
}

/// 分段切换中的一项：选中项为浅底亮字，其余透明。
fn segment(
    id: impl Into<gpui::ElementId>,
    text: impl Into<gpui::SharedString>,
    active: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .px(px(14.))
        .py(px(8.))
        .rounded(px(10.))
        .text_size(px(14.))
        .cursor_pointer()
        .when(active, |el| {
            el.bg(rgb(0x343945)).text_color(rgb(theme::TEXT))
        })
        .when(!active, |el| {
            el.text_color(rgb(MUTED))
                .hover(|style| style.text_color(rgb(theme::TEXT_SOFT)))
        })
        .child(text.into())
}

/// 面板中的浅色块：常用动作、执行内容等。
fn tile() -> Div {
    div()
        .rounded(px(10.))
        .border_1()
        .border_color(rgb(TILE_EDGE))
        .bg(rgb(TILE))
}

fn alert(text: impl Into<gpui::SharedString>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .p(px(12.))
        .rounded(px(10.))
        .bg(theme::tint_fill(theme::RECORD, 0.13, 0.04))
        .border_1()
        .border_color(theme::hsla(theme::RECORD, 0.3))
        .text_size(px(12.))
        .text_color(rgb(theme::RECORD_TEXT))
        .child(icon(Icon::Alert, 15., theme::RECORD))
        .child(div().flex_1().min_w_0().child(text.into()))
}

impl Workspace {
    /// 顶部切换轮盘与呼出方式，左侧轮盘即编辑器，右侧面板随选中的格子变化。
    pub(super) fn render_wheels(&self, (width, height): (f32, f32), cx: &mut Context<Self>) -> Div {
        let can_edit = self.wheels_error.is_none() && !self.recording;
        let wheel = self
            .wheel_selected
            .as_deref()
            .and_then(|id| self.wheels.wheels.iter().find(|wheel| wheel.id == id));
        let banner = self.render_banner(cx);
        // 舞台以外的固定高度：页边距、顶栏、移动提示与状态栏，另加横幅、错误和应用范围行；
        // 宽度扣除侧栏、页边距与右侧面板。
        let overhead = 200.
            + if banner.is_some() { 66. } else { 0. }
            + if self.wheels_error.is_some() { 60. } else { 0. }
            + if wheel.is_some_and(|wheel| !self.wheel_scope_choices(wheel).is_empty()) {
                46.
            } else {
                0.
            };
        let stage = (height - overhead)
            .min(width - PANEL_WIDTH - 262.)
            .clamp(STAGE_RANGE.0, STAGE_RANGE.1);
        let mut tabs = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(4.))
            .p(px(4.))
            .rounded(px(12.))
            .bg(rgb(0x24272f));
        for (index, item) in self.wheels.wheels.iter().enumerate() {
            let id = item.id.clone();
            let active = wheel.is_some_and(|wheel| wheel.id == item.id);
            tabs = tabs.child(
                segment(("wheel-tab", index as u64), item.name.clone(), active)
                    .on_click(cx.listener(move |this, _, _, cx| this.switch_wheel(id.clone(), cx))),
            );
        }
        tabs = tabs.child(
            segment("create-wheel", "＋ 新建", false).on_click(cx.listener(
                move |this, _, _, cx| {
                    if can_edit {
                        this.create_wheel(cx);
                    }
                },
            )),
        );
        let header = div()
            .flex()
            .items_center()
            .gap(px(16.))
            .pb(px(14.))
            .border_b_1()
            .border_color(rgb(LINE))
            .child(div().flex_1().min_w_0().flex().child(tabs))
            .when_some(wheel, |el, wheel| {
                el.child(self.wheel_trigger_bar(wheel, cx))
            });
        let content = match wheel {
            Some(wheel) => self.wheel_editor_content(wheel, stage, cx),
            None => div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(14.))
                .child(theme::icon_tile(Icon::Layers, theme::ACCENT, 56.))
                .child(panel_title(if self.wheels_error.is_some() {
                    "轮盘配置无法载入"
                } else {
                    "还没有轮盘"
                }))
                .when(self.wheels_error.is_none(), |el| {
                    el.child(
                        button("empty-create-wheel", "新建轮盘", Tone::Primary, can_edit).when(
                            can_edit,
                            |button| {
                                button.on_click(cx.listener(|this, _, _, cx| this.create_wheel(cx)))
                            },
                        ),
                    )
                })
                .child(self.wheel_file_actions(cx)),
        };
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .min_h_0()
            .pl(px(20.))
            .pr(px(24.))
            .pt(px(6.))
            .pb(px(14.))
            .flex()
            .flex_col()
            .gap(px(14.))
            .when(!self.wheels.wheels.is_empty(), |el| el.child(header))
            .children(banner)
            .when_some(self.wheels_error.clone(), |el, error| {
                el.child(alert(error))
            })
            .child(content)
    }

    fn wheel_file_actions(&self, cx: &mut Context<Self>) -> Div {
        let actions = div().flex().items_center().gap(px(14.));
        if self.recording {
            return actions
                .opacity(0.45)
                .child(caption("高级：编辑 JSON"))
                .child(caption("重新载入"));
        }
        actions
            .child(
                link("open-wheels-file", "高级：编辑 JSON")
                    .on_click(cx.listener(|this, _, _, cx| this.open_wheels_file(cx))),
            )
            .child(
                link("reload-wheels", "重新载入")
                    .on_click(cx.listener(|this, _, _, cx| this.reload_wheels(cx))),
            )
    }

    /// 呼出方式与试用：已有绑定时显示组合并可跳转修改，没有时引导设置。
    fn wheel_trigger_bar(&self, wheel: &WheelDefinition, cx: &mut Context<Self>) -> Div {
        let can_edit = self.wheels_error.is_none() && !self.recording;
        let trigger = self
            .triggers
            .bindings()
            .iter()
            .find(|binding| matches!(&binding.action, ActionSpec::ShowWheel(reference) if reference.wheel_id == wheel.id))
            .map(|binding| crate::activation_label(&binding.spec));
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(rgb(MUTED))
                    .child("呼出方式："),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(rgb(theme::TEXT))
                    .child(trigger.clone().unwrap_or_else(|| "未设置".into())),
            )
            .child(
                link(
                    "bind-wheel",
                    if trigger.is_some() {
                        "更改"
                    } else {
                        "设置"
                    },
                )
                .text_size(px(13.))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if can_edit {
                        this.bind_wheel(cx);
                    }
                })),
            )
            .child(
                small(button("preview-wheel", "试一试", Tone::Secondary, can_edit)).on_click(
                    cx.listener(move |this, _, _, cx| {
                        if can_edit {
                            this.preview_wheel(cx);
                        }
                    }),
                ),
            )
    }

    fn wheel_editor_content(
        &self,
        wheel: &WheelDefinition,
        stage: f32,
        cx: &mut Context<Self>,
    ) -> Div {
        let can_edit = self.wheels_error.is_none() && !self.recording;
        let tint = accent_color(wheel.accent);
        let view = self.wheel_view().unwrap_or_else(|| wheel.clone());
        let geometry = view_geometry(&view);
        let scope = self.wheel_scope_for(wheel);
        let scope_choices = self.wheel_scope_choices(wheel);
        let mut stage_column = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .min_h_0()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(14.));
        if !scope_choices.is_empty() {
            // 应用与场景范围用下拉选择，再多也只占一行。
            let current = scope
                .as_ref()
                .map_or_else(|| "通用".into(), WheelScope::label);
            let open = self.wheel_scope_open;
            let mut options = div()
                .absolute()
                .top(px(40.))
                .left(px(0.))
                .w(px(200.))
                .p(px(4.))
                .rounded(px(10.))
                .border_1()
                .border_color(rgb(0x333944))
                .bg(rgb(TILE))
                .shadow(theme::lift(0.45))
                .occlude()
                .flex()
                .flex_col()
                .gap(px(2.));
            let choices = std::iter::once(None).chain(scope_choices.into_iter().map(Some));
            for (index, choice) in choices.enumerate() {
                let active = choice == scope;
                let name = choice
                    .as_ref()
                    .map_or_else(|| "通用".into(), WheelScope::label);
                options = options.child(
                    div()
                        .id(("wheel-scope", index as u64))
                        .px(px(10.))
                        .py(px(7.))
                        .rounded(px(7.))
                        .text_size(px(13.))
                        .truncate()
                        .cursor_pointer()
                        .when(active, |el| el.bg(rgb(0x2a2f38)))
                        .hover(|style| style.bg(rgb(0x262b34)))
                        .child(name)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_wheel_scope(choice.clone(), cx)
                        })),
                );
            }
            stage_column = stage_column.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(label("用于"))
                    .child(
                        div()
                            .relative()
                            .child(
                                div()
                                    .id("wheel-scope-toggle")
                                    .w(px(200.))
                                    .px(px(10.))
                                    .py(px(7.))
                                    .rounded(px(10.))
                                    .border_1()
                                    .border_color(if open {
                                        theme::hsla(tint, 0.8)
                                    } else {
                                        rgb(0x333944).into()
                                    })
                                    .bg(rgb(0x101216))
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .gap(px(8.))
                                    .text_size(px(13.))
                                    .cursor_pointer()
                                    .child(div().min_w_0().truncate().child(current))
                                    .child(
                                        div()
                                            .text_size(px(10.))
                                            .text_color(rgb(MUTED))
                                            .child(if open { "▲" } else { "▼" }),
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.wheel_scope_open = !this.wheel_scope_open;
                                        cx.notify();
                                    })),
                            )
                            .when(open, |el| el.child(deferred(options).with_priority(1))),
                    )
                    .when(matches!(scope, Some(WheelScope::Scene(_))), |el| {
                        el.child(
                            div()
                                .text_size(px(12.))
                                .text_color(rgb(MUTED))
                                .child("改动只在此场景生效，其余格子沿用通用动作"),
                        )
                    }),
            );
        }
        let moving_label = self
            .wheel_moving
            .then(|| {
                self.wheel_slot_selected
                    .and_then(|slot| view.slots.get(slot))
            })
            .flatten()
            .map(|slot| slot.label.clone());
        // 提示条占固定高度，出现与消失时轮盘位置不变。
        let hint = div()
            .h(px(38.))
            .flex()
            .items_center()
            .when_some(moving_label, |el, name| {
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .px(px(14.))
                        .py(px(8.))
                        .rounded(px(10.))
                        .bg(rgb(theme::mix(tint, 0x14161b, 0.12)))
                        .border_1()
                        .border_color(theme::hsla(tint, 0.45))
                        .text_color(rgb(theme::mix(tint, 0xffffff, 0.55)))
                        .text_size(px(13.))
                        .child(format!("点选新位置，“{name}”会移过去（有动作就互换）"))
                        .child(link("cancel-wheel-move", "取消").on_click(
                            cx.listener(|this, _, _, cx| this.set_wheel_moving(false, cx)),
                        )),
                )
            });
        stage_column = stage_column
            .child(hint)
            .child(self.wheel_cell_stage(&view, &geometry, stage, can_edit, cx));

        let panel = match self.wheel_cell {
            Some(cell) if self.wheel_slot_selected.is_some() && !self.wheel_replacing => {
                self.wheel_edit_panel(&view, cell, &geometry, can_edit, cx)
            }
            Some(cell) => self.wheel_add_panel(&view, cell, &geometry, can_edit, cx),
            None => self.wheel_home_panel(wheel, &view, can_edit, cx),
        };
        let footer = div()
            .flex_none()
            .pl(px(28.))
            .pt(px(14.))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(FAINT))
                    .child("改动自动保存"),
            )
            .child(self.wheel_file_actions(cx));
        let scroll = div()
            .id("wheel-editor-content")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .pl(px(28.))
            .pt(px(10.))
            .pb(px(8.))
            .flex()
            .flex_col()
            .child(panel);
        let aside = div()
            .w(px(PANEL_WIDTH))
            .flex_none()
            .h_full()
            .min_h_0()
            .border_l_1()
            .border_color(rgb(LINE))
            .flex()
            .flex_col()
            .child(self.smooth_scroll.wrap("wheel-editor-content", scroll))
            .child(footer);
        div()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .child(stage_column)
            .child(aside)
    }

    /// 三圈全部格子都可点击；空格显示“＋”，移动中点目标格完成移动。
    fn wheel_cell_stage(
        &self,
        view: &WheelDefinition,
        geometry: &crate::radial_wheel::RingGeometry,
        size: f32,
        can_edit: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let bounds = Rc::new(Cell::new(None));
        let stage = crate::radial_wheel::editor_stage(
            view,
            geometry,
            self.wheel_cell,
            self.wheel_hover,
            self.wheel_moving,
            |index| {
                view.slots
                    .get(index)
                    .is_some_and(|slot| self.action_unavailable_reason(&slot.action).is_none())
            },
            size,
            bounds.clone(),
        );
        let geometry = *geometry;
        let hover_bounds = bounds.clone();
        div()
            .id("wheel-cell-stage")
            .flex_none()
            .cursor_pointer()
            .child(stage)
            .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                let cell = hover_bounds.get().and_then(|stage| {
                    let point = (
                        f32::from(event.position.x - stage.origin.x),
                        f32::from(event.position.y - stage.origin.y),
                    );
                    crate::radial_wheel::editor_hit(&geometry, size, point)
                });
                if this.wheel_hover != cell {
                    this.wheel_hover = cell;
                    cx.notify();
                }
            }))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if !*hovered && this.wheel_hover.take().is_some() {
                    cx.notify();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    let Some(stage) = bounds.get() else {
                        return;
                    };
                    let point = (
                        f32::from(event.position.x - stage.origin.x),
                        f32::from(event.position.y - stage.origin.y),
                    );
                    this.wheel_scope_open = false;
                    if !can_edit {
                        cx.notify();
                        return;
                    }
                    if let Some(cell) = crate::radial_wheel::editor_hit(&geometry, size, point) {
                        this.select_wheel_cell(cell, cx);
                    }
                }),
            )
    }

    /// 未选格子时：用法提示、每圈格数，以及轮盘名称与颜色。
    fn wheel_home_panel(
        &self,
        wheel: &WheelDefinition,
        view: &WheelDefinition,
        can_edit: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let tip = |key: &'static str, text: &'static str| {
            div()
                .flex()
                .gap(px(6.))
                .text_size(px(14.))
                .child(div().text_color(rgb(theme::TEXT)).child(key))
                .child(div().text_color(rgb(0xa7aeba)).child(text))
        };
        let mut rings = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(label("每圈格数"));
        for (ring, name) in RING_NAMES.iter().enumerate() {
            let ring = ring as u8;
            let Some((sectors, _)) = ring_geometry(view, ring) else {
                continue;
            };
            let used = view
                .slots
                .iter()
                .filter(|slot| slot.position.is_some_and(|position| position.ring == ring))
                .count();
            let (can_less, can_more) = (can_edit && sectors > 2, can_edit && sectors < 16);
            rings = rings.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(div().w(px(44.)).text_size(px(14.)).child(*name))
                    .child(
                        small(button(
                            ("wheel-ring-less", u64::from(ring)),
                            "−",
                            Tone::Secondary,
                            can_less,
                        ))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if can_less {
                                this.resize_wheel_ring(ring, -1, cx);
                            }
                        })),
                    )
                    .child(
                        div()
                            .w(px(48.))
                            .flex()
                            .justify_center()
                            .text_size(px(14.))
                            .child(format!("{sectors} 格")),
                    )
                    .child(
                        small(button(
                            ("wheel-ring-more", u64::from(ring)),
                            "+",
                            Tone::Secondary,
                            can_more,
                        ))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if can_more {
                                this.resize_wheel_ring(ring, 1, cx);
                            }
                        })),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(MUTED))
                            .child(format!("已放 {used} 个")),
                    ),
            );
        }
        let mut colors = div().flex().items_center().gap(px(8.));
        for (index, accent) in [WheelAccent::Mint, WheelAccent::Violet, WheelAccent::Amber]
            .into_iter()
            .enumerate()
        {
            let color = accent_color(accent);
            let active = wheel.accent == accent;
            colors = colors.child(
                div()
                    .id(("wheel-accent", index as u64))
                    .size(px(26.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_2()
                    .border_color(theme::hsla(color, if active { 0.9 } else { 0. }))
                    .when(can_edit, |el| el.cursor_pointer())
                    .child(theme::dot(color, 16.))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if can_edit {
                            this.set_wheel_accent(accent, cx);
                        }
                    })),
            );
        }
        let references = self.triggers.bindings().iter().filter(|binding|
            matches!(&binding.action, ActionSpec::ShowWheel(reference) if reference.wheel_id == wheel.id)
        ).count();
        let can_remove = can_edit && references == 0;
        div()
            .flex()
            .flex_col()
            .gap(px(22.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(panel_title("点轮盘上的格子"))
                    .child(tip("空格子", "→ 添加动作"))
                    .child(tip("已有动作", "→ 改名、移动或移除"))
                    .child(tip("使用时", "→ 按住呼出键往方向拖，松开执行")),
            )
            .child(divider())
            .child(rings)
            .child(divider())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(label("轮盘名称"))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(div().flex_1().min_w_0().child(self.wheel_name.clone()))
                            .child(
                                small(button("save-wheel-name", "保存", Tone::Secondary, can_edit))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if can_edit {
                                            this.save_wheel_name(cx);
                                        }
                                    })),
                            ),
                    )
                    .child(colors),
            )
            .child(
                link(
                    "remove-wheel",
                    if references == 0 {
                        "删除这个轮盘".to_string()
                    } else {
                        format!("有 {references} 条呼出绑定在用，不能删除")
                    },
                )
                .text_color(rgb(if can_remove { 0xf0a0a0 } else { FAINT }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if can_remove {
                        this.remove_wheel(cx);
                    }
                })),
            )
    }

    /// 空格子或更换动作：常用动作一键放入，也可填写快捷键、本地动作或复制已有触发器。
    fn wheel_add_panel(
        &self,
        view: &WheelDefinition,
        (ring, sector): (u8, u8),
        geometry: &crate::radial_wheel::RingGeometry,
        can_edit: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let tint = accent_color(view.accent);
        let place = format!(
            "{} · {}",
            RING_NAMES[usize::from(ring)],
            direction_name(sector, geometry[usize::from(ring)])
        );
        let mut presets = div().grid().grid_cols(2).gap(px(8.));
        for (index, (name, preset)) in WHEEL_PRESETS.iter().enumerate() {
            let hint = match preset {
                Preset::Keys(keys) => *keys,
                Preset::Window(_) => "窗口",
            };
            presets = presets.child(
                tile()
                    .id(("wheel-preset", index as u64))
                    .min_w_0()
                    .px(px(12.))
                    .py(px(10.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.))
                    .text_size(px(13.))
                    .when(can_edit, |el| {
                        el.cursor_pointer().hover(|style| {
                            style
                                .border_color(theme::hsla(tint, 0.9))
                                .bg(rgb(theme::mix(tint, TILE, 0.08)))
                        })
                    })
                    .child(div().min_w_0().truncate().child(*name))
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(12.))
                            .text_color(rgb(FAINT))
                            .child(hint),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if can_edit {
                            this.put_wheel_preset(index, cx);
                        }
                    })),
            );
        }
        let candidates: Vec<_> = self
            .triggers
            .bindings()
            .iter()
            .filter(|binding| {
                !matches!(
                    binding.action,
                    ActionSpec::ShowPanel | ActionSpec::ShowWheel(_)
                )
            })
            .collect();
        let mut sources = div()
            .id("wheel-source-list")
            .max_h(px(180.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(4.));
        for binding in &candidates {
            let id = binding.spec.id;
            sources = sources.child(
                div()
                    .id(("wheel-source-binding", id))
                    .px(px(10.))
                    .py(px(7.))
                    .rounded(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(12.))
                    .when(can_edit, |el| {
                        el.cursor_pointer()
                            .hover(|style| style.bg(rgb(theme::SURFACE)))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(binding.message.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .max_w(px(120.))
                            .truncate()
                            .text_color(rgb(theme::TEXT_FAINT))
                            .child(action_text(&binding.action)),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if can_edit {
                            this.put_wheel_binding(id, cx);
                        }
                    })),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(18.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(panel_title(if self.wheel_replacing {
                        "更换动作"
                    } else {
                        "放一个动作"
                    }))
                    .child(label(place)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(label("常用，点一下就放好"))
                    .child(presets),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(label("或者，填写自己的快捷键"))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(self.wheel_slot_shortcut.clone()),
                            )
                            .child(
                                small(button(
                                    "put-wheel-shortcut",
                                    "放这里",
                                    Tone::Primary,
                                    can_edit,
                                ))
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        if can_edit {
                                            this.put_wheel_shortcut(cx);
                                        }
                                    },
                                )),
                            ),
                    ),
            )
            .child(
                link("put-wheel-local", "改用本地动作（打开网址、运行程序等）").on_click(
                    cx.listener(move |this, _, _, cx| {
                        if can_edit {
                            this.put_wheel_local_action(cx);
                        }
                    }),
                ),
            )
            .when(!candidates.is_empty(), |el| {
                el.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(label("或复制已有触发器的动作"))
                        .child(self.smooth_scroll.wrap("wheel-source-list", sources)),
                )
            })
            .child(if self.wheel_replacing {
                link("cancel-wheel-replace", "不换了")
                    .on_click(cx.listener(|this, _, _, cx| this.set_wheel_replacing(false, cx)))
            } else {
                link("deselect-wheel-cell", "取消")
                    .on_click(cx.listener(|this, _, _, cx| this.deselect_wheel_cell(cx)))
            })
    }

    /// 已有动作的格子：改名（离开时自动保存）、更换、移动或移除。
    fn wheel_edit_panel(
        &self,
        view: &WheelDefinition,
        (ring, sector): (u8, u8),
        geometry: &crate::radial_wheel::RingGeometry,
        can_edit: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let Some(slot) = slot_at(view, ring, sector).map(|index| &view.slots[index]) else {
            return div();
        };
        let place = format!(
            "{} · {}",
            RING_NAMES[usize::from(ring)],
            direction_name(sector, geometry[usize::from(ring)])
        );
        let local = matches!(slot.action, ActionSpec::Local(_));
        let can_remove = can_edit && view.slots.len() > 1;
        div()
            .flex()
            .flex_col()
            .gap(px(18.))
            .child(label(place))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(label("名称"))
                    .child(self.wheel_slot_label.clone()),
            )
            .child(
                tile()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .p(px(14.))
                    .rounded(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(label("执行").text_size(px(12.)))
                            .child(
                                div()
                                    .text_size(px(15.))
                                    .truncate()
                                    .child(action_text(&slot.action)),
                            )
                            .when_some(
                                self.action_unavailable_reason(&slot.action),
                                |el, error| {
                                    el.child(
                                        div()
                                            .text_size(px(11.))
                                            .truncate()
                                            .text_color(rgb(theme::RECORD))
                                            .child(error),
                                    )
                                },
                            ),
                    )
                    .child(
                        small(button(
                            "replace-wheel-action",
                            "更换",
                            Tone::Secondary,
                            can_edit,
                        ))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if can_edit {
                                this.set_wheel_replacing(true, cx);
                            }
                        })),
                    ),
            )
            .when(local, |el| {
                el.child(self.local_action_fields(true, cx)).child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(
                            small(button(
                                "save-wheel-local",
                                "保存本地动作",
                                Tone::Primary,
                                can_edit,
                            ))
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if can_edit {
                                        this.save_wheel_local(cx);
                                    }
                                },
                            )),
                        )
                        .child(
                            small(button("stop-wheel-local", "停止运行", Tone::Danger, true))
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.stop_local_action(true, cx)),
                                ),
                        ),
                )
            })
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        button("move-wheel-cell", "移动", Tone::Secondary, can_edit)
                            .flex_1()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if can_edit {
                                    this.set_wheel_moving(true, cx);
                                }
                            })),
                    )
                    .child(
                        button("remove-wheel-cell", "移除", Tone::Danger, can_remove)
                            .flex_1()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if can_remove {
                                    this.remove_wheel_cell(cx);
                                }
                            })),
                    ),
            )
            .child(
                link("deselect-wheel-cell", "完成")
                    .on_click(cx.listener(|this, _, _, cx| this.deselect_wheel_cell(cx))),
            )
    }

    /// 触发器动作编辑区中的轮盘选择器，保留缺失引用以便明确修复。
    pub(super) fn draft_wheel_fields(&self, cx: &mut Context<Self>) -> Div {
        let selected = match &self.draft_action {
            ActionSpec::ShowWheel(reference) => Some(reference.wheel_id.as_str()),
            _ => None,
        };
        let missing =
            selected.is_some_and(|id| !self.wheels.wheels.iter().any(|wheel| wheel.id == id));
        let mut fields = div()
            .flex()
            .flex_col()
            .gap(px(9.))
            .child(caption("选择触发后要打开的轮盘"));
        for (index, wheel) in self.wheels.wheels.iter().enumerate() {
            let id = wheel.id.clone();
            let active = selected == Some(wheel.id.as_str());
            let tint = accent_color(wheel.accent);
            fields = fields.child(
                div()
                    .id(("draft-wheel-choice", index as u64))
                    .p(px(11.))
                    .rounded(px(10.))
                    .border_1()
                    .border_color(if active {
                        theme::hsla(tint, 0.55)
                    } else {
                        rgb(theme::BORDER).into()
                    })
                    .bg(if active {
                        theme::tint_fill(tint, 0.17, 0.04)
                    } else {
                        rgb(theme::BACKGROUND).into()
                    })
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .cursor_pointer()
                    .child(theme::dot(tint, 7.))
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
                                    .truncate()
                                    .child(wheel.name.clone()),
                            )
                            .child(caption(format!("{} 个动作槽位", wheel.slots.len()))),
                    )
                    .when(active, |el| el.child(icon(Icon::Check, 14., tint)))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.choose_draft_wheel(id.clone(), cx)),
                    ),
            );
        }
        fields
            .when(missing, |el| {
                el.child(alert(
                    "引用的轮盘已不存在。请选择一个可用轮盘后保存此绑定。",
                ))
            })
            .when(self.wheels.wheels.is_empty(), |el| {
                el.child(caption("还没有可用轮盘，请先进入轮盘管理创建。"))
            })
            .when_some(self.wheels_error.clone(), |el, error| {
                el.child(alert(error))
            })
            .child(
                button(
                    "manage-draft-wheel",
                    "管理轮盘",
                    Tone::Secondary,
                    !self.recording,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    if !this.recording {
                        this.page = Page::Wheels;
                        cx.notify();
                    }
                })),
            )
            .child(caption(
                "轮盘内容由轮盘管理统一保存；呼出组合由当前触发绑定管理。",
            ))
    }
}
