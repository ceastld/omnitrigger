//! 自定义轮盘的管理页；动作与触发绑定仍由应用层统一保存和验证。
use crate::{
    button, caption, card, heading, icon, icon_button, section_title, small, status_pill, theme,
    wheels::{WheelAccent, WheelDefinition},
    ActionSpec, Context, Div, Icon, Page, Tone, Workspace,
};
use gpui::{div, prelude::*, px, rgb, FontWeight};

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
        ActionSpec::Command(_) => "运行程序 / 脚本".into(),
        ActionSpec::Plugin(_) => "插件动作".into(),
        ActionSpec::ShowPanel => "打开触发面板".into(),
        ActionSpec::ShowWheel(_) => "打开轮盘".into(),
    }
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
    /// 左侧轮盘目录，右侧提供预览、外观与槽位编辑。
    pub(super) fn render_wheels(&self, cx: &mut Context<Self>) -> Div {
        let can_edit = self.wheels_error.is_none() && !self.recording;
        let wheel = self
            .wheel_selected
            .as_deref()
            .and_then(|id| self.wheels.wheels.iter().find(|wheel| wheel.id == id));
        let mut directory = div()
            .id("wheel-directory")
            .w(px(210.))
            .flex_none()
            .h_full()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(section_title("我的轮盘"))
                    .child(caption(self.wheels.wheels.len().to_string())),
            )
            .child(
                icon_button(
                    "create-wheel",
                    Icon::Bolt,
                    "新建轮盘",
                    Tone::Primary,
                    can_edit,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    if can_edit {
                        this.create_wheel(cx);
                    }
                })),
            );
        let mut rows = div()
            .id("wheel-directory-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(8.));
        for (index, item) in self.wheels.wheels.iter().enumerate() {
            let selected = self.wheel_selected.as_deref() == Some(item.id.as_str());
            let tint = accent_color(item.accent);
            let id = item.id.clone();
            let references = self.bindings.iter().filter(|binding|
                matches!(&binding.action, ActionSpec::ShowWheel(reference) if reference.wheel_id == item.id)
            ).count();
            rows = rows.child(
                div()
                    .id(("wheel-directory-item", index as u64))
                    .p(px(13.))
                    .rounded(px(12.))
                    .border_1()
                    .border_color(if selected {
                        theme::hsla(tint, 0.5)
                    } else {
                        rgb(theme::BORDER).into()
                    })
                    .bg(if selected {
                        theme::tint_fill(tint, 0.14, 0.035)
                    } else {
                        rgb(theme::ROW).into()
                    })
                    .cursor_pointer()
                    .flex()
                    .flex_col()
                    .gap(px(9.))
                    .hover(|style| style.border_color(theme::hsla(tint, 0.65)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(theme::dot(tint, 7.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(item.name.clone()),
                            )
                            .when(selected, |el| el.child(icon(Icon::Check, 13., tint))),
                    )
                    .child(caption(format!(
                        "{} 个槽位 · {references} 条绑定",
                        item.slots.len()
                    )))
                    .on_click(cx.listener(move |this, _, _, cx| this.select_wheel(id.clone(), cx))),
            );
        }
        if self.wheels.wheels.is_empty() {
            rows = rows.child(caption("还没有轮盘。新建后，为它添加常用动作。"));
        }
        directory = directory
            .child(rows)
            .child(caption("每个轮盘有独立名称、颜色和动作顺序。"));
        let content = if let Some(wheel) = wheel {
            self.wheel_editor_content(wheel, cx)
        } else {
            card()
                .flex_1()
                .min_w_0()
                .p(px(28.))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(14.))
                .child(theme::icon_tile(Icon::Layers, theme::ACCENT, 56.))
                .child(
                    div()
                        .text_size(px(20.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(if self.wheel_selected.is_some() {
                            "这个轮盘已不存在"
                        } else {
                            "把常用动作放入一条轨道"
                        }),
                )
                .child(caption("在左侧选择轮盘，或新建一个轮盘开始配置。"))
                .child(
                    button("empty-create-wheel", "新建轮盘", Tone::Primary, can_edit).on_click(
                        cx.listener(move |this, _, _, cx| {
                            if can_edit {
                                this.create_wheel(cx);
                            }
                        }),
                    ),
                )
        };
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .min_h_0()
            .pl(px(28.))
            .pr(px(24.))
            .pt(px(6.))
            .pb(px(18.))
            .flex()
            .flex_col()
            .gap(px(18.))
            .child(heading(
                "自定义轮盘",
                "为不同场景建立动作轨道，再绑定自己的键盘或鼠标组合。",
            ))
            .children(self.render_banner(cx))
            .when_some(self.wheels_error.clone(), |el, error| {
                el.child(alert(error))
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .flex()
                    .gap(px(18.))
                    .child(directory)
                    .child(content),
            )
    }

    fn wheel_editor_content(&self, wheel: &WheelDefinition, cx: &mut Context<Self>) -> Div {
        let can_edit = self.wheels_error.is_none() && !self.recording;
        let tint = accent_color(wheel.accent);
        let selected = self
            .wheel_slot_selected
            .filter(|index| *index < wheel.slots.len());
        let references = self.bindings.iter().filter(|binding|
            matches!(&binding.action, ActionSpec::ShowWheel(reference) if reference.wheel_id == wheel.id)
        ).count();
        let can_preview = can_edit && !wheel.slots.is_empty();
        let can_remove = can_edit && references == 0;
        let mut appearance = card()
            .p(px(18.))
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(10.))
                    .child(section_title("轮盘设置"))
                    .child(status_pill(format!("{} / 8 槽位", wheel.slots.len()), tint)),
            )
            .child(Self::input_field("轮盘名称", &self.wheel_name, None))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        small(button(
                            "save-wheel-name",
                            "保存名称",
                            Tone::Secondary,
                            can_edit,
                        ))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if can_edit {
                                this.save_wheel_name(cx);
                            }
                        })),
                    )
                    .child(caption("名称修改后点击保存；槽位和颜色分别保存。")),
            );
        let mut colors = div().flex().flex_wrap().items_center().gap(px(7.));
        for (index, (accent, label)) in [
            (WheelAccent::Mint, "薄荷"),
            (WheelAccent::Violet, "紫罗兰"),
            (WheelAccent::Amber, "琥珀"),
        ]
        .into_iter()
        .enumerate()
        {
            let color = accent_color(accent);
            let active = wheel.accent == accent;
            colors = colors.child(
                div()
                    .id(("wheel-accent", index as u64))
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(px(9.))
                    .border_1()
                    .border_color(theme::hsla(color, if active { 0.7 } else { 0.22 }))
                    .bg(theme::tint_fill(
                        color,
                        if active { 0.22 } else { 0.07 },
                        0.03,
                    ))
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .text_size(px(12.))
                    .when(can_edit, |el| el.cursor_pointer())
                    .when(!can_edit, |el| el.opacity(0.45))
                    .child(theme::dot(color, 8.))
                    .child(label)
                    .when(active, |el| el.child(icon(Icon::Check, 12., color)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if can_edit {
                            this.set_wheel_accent(accent, cx);
                        }
                    })),
            );
        }
        appearance = appearance.child(colors);

        let preview = card()
            .p(px(18.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.))
                    .child(section_title("轨道预览"))
                    .child(caption("点击槽位以编辑")),
            )
            .child(
                div()
                    .flex()
                    .justify_center()
                    .child(self.wheel_orbit_preview(wheel, cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(
                        icon_button(
                            "preview-wheel",
                            Icon::Play,
                            "打开预览",
                            Tone::Secondary,
                            can_preview,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if can_preview {
                                this.preview_wheel(cx);
                            }
                        })),
                    )
                    .child(
                        icon_button(
                            "bind-wheel",
                            Icon::Keyboard,
                            "设置呼出组合",
                            Tone::Primary,
                            can_preview,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if can_preview {
                                this.bind_wheel(cx);
                            }
                        })),
                    ),
            )
            .child(caption(
                "预览使用已保存的槽位。设置呼出组合后，仍需在触发器页保存绑定。",
            ));

        let mut slots = card().p(px(18.)).flex().flex_col().gap(px(12.)).child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(section_title("动作顺序"))
                .child(caption("从顶部开始，顺时针排列")),
        );
        for (index, slot) in wheel.slots.iter().enumerate() {
            let active = selected == Some(index);
            let reason = self.action_unavailable_reason(&slot.action);
            slots = slots.child(
                div()
                    .id(("wheel-slot-row", index as u64))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .p(px(11.))
                    .rounded(px(10.))
                    .border_1()
                    .border_color(if active {
                        theme::hsla(tint, 0.5)
                    } else {
                        rgb(theme::BORDER).into()
                    })
                    .bg(if active {
                        theme::tint_fill(tint, 0.14, 0.04)
                    } else {
                        rgb(theme::BACKGROUND).into()
                    })
                    .cursor_pointer()
                    .hover(|style| style.border_color(theme::hsla(tint, 0.55)))
                    .child(
                        div()
                            .size(px(28.))
                            .rounded(px(9.))
                            .bg(theme::hsla(tint, 0.12))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(12.))
                            .font_family(theme::MONO_FONT)
                            .text_color(rgb(tint))
                            .child((index + 1).to_string()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .truncate()
                                    .child(slot.label.clone()),
                            )
                            .child(caption(action_text(&slot.action)))
                            .when_some(reason, |el, error| {
                                el.child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(rgb(theme::RECORD))
                                        .child(error),
                                )
                            }),
                    )
                    .when(active, |el| el.child(icon(Icon::Sliders, 14., tint)))
                    .on_click(cx.listener(move |this, _, _, cx| this.select_wheel_slot(index, cx))),
            );
        }
        if let Some(index) = selected {
            let can_move_up = can_edit && index > 0;
            let can_move_down = can_edit && index + 1 < wheel.slots.len();
            let can_remove_slot = can_edit && wheel.slots.len() > 1;
            slots = slots
                .child(div().h(px(1.)).bg(rgb(theme::BORDER)).my(px(3.)))
                .child(caption(format!(
                    "已选择第 {} 个槽位，在下方编辑动作内容。",
                    index + 1
                )))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(6.))
                        .child(
                            small(icon_button(
                                "wheel-slot-up",
                                Icon::ArrowUp,
                                "前移",
                                Tone::Secondary,
                                can_move_up,
                            ))
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if can_move_up {
                                        this.move_wheel_slot(-1, cx);
                                    }
                                },
                            )),
                        )
                        .child(
                            small(icon_button(
                                "wheel-slot-down",
                                Icon::ArrowDown,
                                "后移",
                                Tone::Secondary,
                                can_move_down,
                            ))
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if can_move_down {
                                        this.move_wheel_slot(1, cx);
                                    }
                                },
                            )),
                        )
                        .child(
                            small(icon_button(
                                "remove-wheel-slot",
                                Icon::Trash,
                                "删除槽位",
                                Tone::Danger,
                                can_remove_slot,
                            ))
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if can_remove_slot {
                                        this.remove_wheel_slot(cx);
                                    }
                                },
                            )),
                        ),
                )
                .when(wheel.slots.len() == 1, |el| {
                    el.child(caption("每个轮盘至少保留一个槽位。"))
                });
        } else {
            slots = slots.child(caption("在上方选择一个槽位，修改名称、快捷键或排列顺序。"));
        }

        let has_room = can_edit && wheel.slots.len() < 8;
        let can_save_slot = can_edit && selected.is_some();
        let selected_shortcut = selected
            .is_some_and(|index| matches!(wheel.slots[index].action, ActionSpec::SendShortcut(_)));
        let mut add = card()
            .p(px(18.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(section_title("编辑 / 添加动作"))
            .child(Self::input_field(
                "槽位名称",
                &self.wheel_slot_label,
                Some("填写简短名称，方便在轮盘上辨认。"),
            ))
            .child(Self::input_field(
                "快捷键",
                &self.wheel_slot_shortcut,
                Some("单个按键或组合，如 Enter、Ctrl+V。"),
            ))
            .when(selected.is_some(), |el| {
                el.child(
                    button(
                        "save-wheel-slot",
                        "保存选中槽位",
                        Tone::Primary,
                        can_save_slot,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if can_save_slot {
                            this.save_wheel_slot(cx);
                        }
                    })),
                )
            })
            .child(
                button(
                    "add-wheel-shortcut",
                    "添加为新快捷键槽位",
                    Tone::Secondary,
                    has_room,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    if has_room {
                        this.add_wheel_shortcut(cx);
                    }
                })),
            )
            .when(selected.is_some() && !selected_shortcut, |el| {
                el.child(caption(
                    "选中槽位使用程序、插件或提示动作，保存时仅更新名称；添加快捷键会创建新槽位。",
                ))
            })
            .when(!has_room && wheel.slots.len() >= 8, |el| {
                el.child(caption("已达到 8 个槽位上限，可先删除不常用的动作。"))
            })
            .child(div().h(px(1.)).bg(rgb(theme::BORDER)).my(px(3.)))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(theme::TEXT_MUTED))
                    .child("从现有触发器复制动作"),
            );
        let candidates: Vec<_> = self
            .bindings
            .iter()
            .filter(|binding| {
                !matches!(
                    binding.action,
                    ActionSpec::ShowPanel | ActionSpec::ShowWheel(_)
                )
            })
            .collect();
        let source_exists = candidates
            .iter()
            .any(|binding| Some(binding.spec.id) == self.wheel_source_binding);
        let mut source_rows = div()
            .id("wheel-source-list")
            .max_h(px(220.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(6.));
        for binding in candidates.iter() {
            let id = binding.spec.id;
            let active = self.wheel_source_binding == Some(id);
            source_rows = source_rows.child(
                div()
                    .id(("wheel-source-binding", id))
                    .p(px(10.))
                    .rounded(px(9.))
                    .border_1()
                    .border_color(if active {
                        theme::hsla(tint, 0.5)
                    } else {
                        rgb(theme::BORDER).into()
                    })
                    .bg(if active {
                        theme::tint_fill(tint, 0.13, 0.04)
                    } else {
                        rgb(theme::BACKGROUND).into()
                    })
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .cursor_pointer()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .truncate()
                                    .child(binding.message.clone()),
                            )
                            .child(caption(action_text(&binding.action))),
                    )
                    .when(active, |el| el.child(icon(Icon::Check, 14., tint)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.wheel_source_binding = Some(id);
                        cx.notify();
                    })),
            );
        }
        if candidates.is_empty() {
            source_rows = source_rows.child(caption(
                "还没有可复制的动作。先在触发器页创建程序、插件或快捷键动作。",
            ));
        }
        let can_add_binding = has_room && source_exists;
        add = add
            .child(source_rows)
            .when(
                self.wheel_source_binding.is_some() && !source_exists,
                |el| el.child(alert("原触发器已不存在，请重新选择要复制的动作。")),
            )
            .child(
                button(
                    "add-binding-to-wheel",
                    "复制为新槽位",
                    Tone::Secondary,
                    can_add_binding,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    if can_add_binding {
                        if let Some(id) = this.wheel_source_binding {
                            this.add_wheel_binding(id, cx);
                        }
                    }
                })),
            )
            .child(caption(
                "复制后独立保存。程序动作保留原参数，插件动作继续使用共享插件设置。",
            ));

        let footer = div().flex().flex_col().gap(px(10.))
            .child(alert("使用时移动指针选择槽位，再点击执行；Esc 取消。先用预览确认动作顺序，再设置呼出组合。"))
            .child(div().flex().flex_wrap().items_center().gap(px(8.))
                .child(icon_button("remove-wheel", Icon::Trash,
                    if references == 0 { "删除这个轮盘".to_string() } else { format!("{references} 条绑定使用中，无法删除") },
                    Tone::Danger, can_remove)
                    .on_click(cx.listener(move |this, _, _, cx| { if can_remove { this.remove_wheel(cx); } })))
                .when(references > 0, |el| el.child(caption("先修改或删除引用此轮盘的触发绑定。"))));

        div().flex_1().min_w_0().h_full().min_h_0().flex().child(
            div()
                .id("wheel-editor-content")
                .flex_1()
                .min_w_0()
                .h_full()
                .min_h_0()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap(px(16.))
                .child(appearance)
                .child(preview)
                .child(slots)
                .child(add)
                .child(footer),
        )
    }

    /// 圆形舞台只编辑选中项，不在配置页面执行槽位动作。
    fn wheel_orbit_preview(&self, wheel: &WheelDefinition, cx: &mut Context<Self>) -> Div {
        let tint = accent_color(wheel.accent);
        let mut orbit = div()
            .relative()
            .size(px(320.))
            .flex_none()
            .child(theme::aurora(tint, 0.12, 95.).left(px(160.)).top(px(160.)));
        for (index, diameter) in [282., 212., 126.].into_iter().enumerate() {
            orbit = orbit.child(
                div()
                    .absolute()
                    .left(px((320. - diameter) / 2.))
                    .top(px((320. - diameter) / 2.))
                    .size(px(diameter))
                    .rounded(px(diameter / 2.))
                    .border_1()
                    .border_color(theme::hsla(tint, if index == 0 { 0.2 } else { 0.1 })),
            );
        }
        orbit = orbit.child(
            div()
                .absolute()
                .left(px(112.))
                .top(px(112.))
                .size(px(96.))
                .rounded(px(48.))
                .border_1()
                .border_color(theme::hsla(tint, 0.45))
                .bg(theme::tint_fill(tint, 0.18, 0.035))
                .shadow(theme::glow(tint, 0.14, 20.))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(5.))
                .child(
                    div()
                        .font_family(theme::MONO_FONT)
                        .text_size(px(25.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(tint))
                        .child(format!("{:02}", wheel.slots.len())),
                )
                .child(caption("动作轨道")),
        );
        let count = wheel.slots.len().max(1) as f32;
        for (index, slot) in wheel.slots.iter().enumerate() {
            let angle = index as f32 * std::f32::consts::TAU / count - std::f32::consts::FRAC_PI_2;
            let x = 160. + 116. * angle.cos();
            let y = 160. + 116. * angle.sin();
            let active = self.wheel_slot_selected == Some(index);
            orbit = orbit.child(
                div()
                    .id(("wheel-orbit-slot", index as u64))
                    .absolute()
                    .left(px(x - 37.))
                    .top(px(y - 28.))
                    .w(px(74.))
                    .h(px(56.))
                    .rounded(px(15.))
                    .border_1()
                    .border_color(theme::hsla(tint, if active { 0.9 } else { 0.35 }))
                    .bg(theme::tint_fill(
                        tint,
                        if active { 0.3 } else { 0.15 },
                        0.06,
                    ))
                    .when(active, |el| el.shadow(theme::glow(tint, 0.28, 16.)))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(5.))
                    .px(px(5.))
                    .cursor_pointer()
                    .hover(|style| style.border_color(theme::hsla(tint, 0.9)))
                    .child(
                        div()
                            .font_family(theme::MONO_FONT)
                            .text_size(px(10.))
                            .text_color(rgb(tint))
                            .child(format!("{:02}", index + 1)),
                    )
                    .child(
                        div()
                            .max_w(px(64.))
                            .truncate()
                            .text_size(px(11.))
                            .text_color(rgb(theme::TEXT))
                            .child(slot.label.clone()),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.select_wheel_slot(index, cx))),
            );
        }
        orbit
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
