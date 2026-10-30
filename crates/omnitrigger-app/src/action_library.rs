//! 插件动作目录与安装设置，和输入匹配保持独立。
use crate::theme::rgb;
use crate::{
    button, caption, card, combo, heading, icon, icon_button, icon_tile, section_title, small,
    status_pill, theme, ActionSpec, CapSize, Context, Div, Icon, Page, Tone, Workspace,
};
use gpui::{div, prelude::*, px, Focusable, FontWeight, MouseButton, Window};

/// 琥珀色提示条，用于插件未就绪等需要用户处理的状态。
fn notice(text: impl Into<gpui::SharedString>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(12.))
        .py(px(9.))
        .rounded(px(10.))
        .bg(theme::tint_fill(theme::RECORD, 0.12, 0.04))
        .border_1()
        .border_color(theme::hsla(theme::RECORD, 0.3))
        .text_size(px(12.))
        .text_color(rgb(theme::RECORD_TEXT))
        .child(icon(Icon::Alert, 14., theme::RECORD))
        .child(div().flex_1().min_w_0().child(text.into()))
}

/// 扩展页的统一卡片：图标、标题与说明在左，状态或主操作在右。
pub(super) fn extension_card(
    glyph: Icon,
    tint: u32,
    title: &'static str,
    description: impl Into<gpui::SharedString>,
    trailing: impl IntoElement,
) -> Div {
    card().p(px(16.)).flex().flex_col().gap(px(12.)).child(
        div()
            .flex()
            .items_center()
            .gap(px(14.))
            .child(icon_tile(glyph, tint, 40.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(caption(description)),
            )
            .child(trailing),
    )
}

impl Workspace {
    pub(super) fn render_actions(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let query = self.action_search.read(cx).text().trim().to_lowercase();
        let catalogue = self.plugins.catalogue();
        let catalogue_error = catalogue.as_ref().err().map(ToString::to_string);
        let actions: Vec<_> = catalogue
            .unwrap_or_default()
            .into_iter()
            .filter(|action| {
                format!(
                    "{} {} {} {}",
                    action.plugin_name,
                    action.title,
                    action.description,
                    action.reference.action_id
                )
                .to_lowercase()
                .contains(&query)
            })
            .collect();
        let focused = self.action_search.focus_handle(cx).is_focused(window);
        let search = div()
            .h(px(42.))
            .px(px(14.))
            .rounded(px(6.))
            .bg(rgb(theme::BACKGROUND))
            .border_1()
            .border_color(if focused {
                theme::hsla(theme::ACCENT, 0.7)
            } else {
                rgb(theme::BORDER_MID).into()
            })
            .flex()
            .items_center()
            .gap(px(10.))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    window.focus(&this.action_search.focus_handle(cx));
                }),
            )
            .child(icon(
                Icon::Search,
                15.,
                if focused {
                    theme::ACCENT
                } else {
                    theme::TEXT_FAINT
                },
            ))
            .child(div().flex_1().min_w_0().child(self.action_search.clone()));
        let local_matches = "本地步骤动作 快捷键 组合键 输入文本 剪贴板 网址 文件 等待 变量 local"
            .contains(query.as_str());
        let local = card()
            .p(px(16.))
            .flex()
            .items_center()
            .gap(px(14.))
            .child(icon_tile(Icon::Keyboard, theme::ACCENT, 40.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("本地步骤动作"),
                    )
                    .child(caption(
                        "快捷键、连续组合键、输入文本、剪贴板、打开网址或文件、等待与变量，按顺序运行。",
                    )),
            )
            .child(
                icon_button(
                    "create-local-action",
                    Icon::Bolt,
                    "新建",
                    Tone::Primary,
                    !self.recording,
                )
                .on_click(cx.listener(|this, _, _, cx| this.new_local_action(cx))),
            );
        let packages = self.package_catalog(&query, cx);
        let browser = self.browser_catalog(&query, cx);
        let action_cards: Vec<_> = actions.into_iter().enumerate().map(|(index, action)| {
        let reference = action.reference;
        let reason = self.action_unavailable_reason(&ActionSpec::Plugin(reference.clone()));
        let ready = reason.is_none();
        let count = self.triggers.bindings().iter().filter(|binding| matches!(&binding.action,
            ActionSpec::Plugin(existing) if existing.plugin_id == reference.plugin_id && existing.action_id == reference.action_id)).count();
        let default_trigger = self.plugins.default_trigger(&reference, 1).ok();
        card()
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .child(icon_tile(Icon::Capture, theme::PINK, 40.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(
                                div()
                                    .text_size(px(15.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(action.title.clone()),
                            )
                            .child(caption(format!("{} 插件 · 已有 {count} 条绑定", action.plugin_name))),
                    )
                    .child(if ready {
                        status_pill("已就绪", theme::ACCENT)
                    } else {
                        status_pill("未就绪", theme::RECORD)
                    }),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(rgb(theme::TEXT_MUTED))
                    .child(action.description),
            )
            .when_some(reason, |el, reason| {
                el.child(
                    notice(reason).child(
                        small(button(
                            ("goto-extensions", index),
                            "前往扩展",
                            Tone::Secondary,
                            !self.recording,
                        ))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.open_page(Page::Extensions, cx)
                        })),
                    ),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .when_some(default_trigger, |el, spec| {
                        el.child(caption("默认"))
                            .child(combo(&crate::caps_for(&spec), CapSize::Small))
                    })
                    .child(div().flex_1())
                    .child(
                        icon_button(
                            ("bind-plugin-action", index),
                            Icon::Bolt,
                            if self.pending.is_some()
                                && self.draft_action
                                    == ActionSpec::Plugin(reference.clone())
                            {
                                "继续设置触发"
                            } else {
                                "设置触发"
                            },
                            Tone::Primary,
                            !self.recording,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| this.choose_plugin_action(reference.clone(), action.title.clone(), cx))),
                    ),
            )
        }).collect();
        let has_plugin_actions = !action_cards.is_empty();
        let empty = !local_matches
            && packages.is_none()
            && browser.is_none()
            && !has_plugin_actions
            && catalogue_error.is_none();
        let page = div()
            .pt(px(6.))
            .pb(px(24.))
            .pl(px(20.))
            .pr(px(24.))
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(heading(
                "动作库",
                "选择动作并设置触发；需要更多动作时前往“扩展”",
            ))
            .children(self.render_banner(cx))
            .child(search)
            .when(local_matches, |el| {
                el.child(section_title("内置")).child(local)
            })
            .when_some(packages, |el, packages| {
                el.child(section_title("动作包")).child(packages)
            })
            .when_some(browser, |el, browser| {
                el.child(section_title("浏览器")).child(browser)
            })
            .when(has_plugin_actions || catalogue_error.is_some(), |el| {
                el.child(section_title("插件"))
                    .when_some(catalogue_error.clone(), |el, error| el.child(notice(error)))
                    .children(action_cards)
            })
            .when(empty, |el| {
                el.child(
                    div()
                        .py(px(28.))
                        .flex()
                        .justify_center()
                        .text_size(px(13.))
                        .text_color(rgb(theme::TEXT_DIM))
                        .child("没有匹配的动作。"),
                )
            });
        // 外层负责滚动、内层负责纵向排列，避免内容在可滚动的弹性容器中被压缩。
        div()
            .id("action-library")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .child(page)
            .map(|el| self.smooth_scroll.wrap("action-library", el))
    }

    /// 扩展页：浏览器连接器、插件与本地动作包的安装和状态集中在此。
    pub(super) fn render_extensions(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let installing = self.install_events.is_some();
        let busy = self.executor.is_plugin_running("raishot");
        let catalogue_error = self.plugins.catalogue().err().map(|e| e.to_string());
        let ready = self.plugin_settings_error.is_none()
            && self.plugins.raishot_enabled()
            && self.plugins.catalogue().is_ok_and(|actions| {
                actions
                    .iter()
                    .any(|action| self.plugins.availability(&action.reference).is_ok())
            });
        let raishot = extension_card(
            Icon::Capture,
            theme::PINK,
            "RaiShot 截图",
            if ready {
                "插件已就绪，截图动作可在动作库中设置触发。"
            } else {
                "自动查找本机兼容安装；未找到时下载并安装。"
            },
            if ready {
                status_pill("已启用", theme::ACCENT).into_any_element()
            } else {
                icon_button(
                    "install-raishot",
                    Icon::ArrowDown,
                    if installing {
                        "正在准备…"
                    } else {
                        "启用"
                    },
                    Tone::Primary,
                    !installing && !busy && catalogue_error.is_none(),
                )
                .on_click(cx.listener(|this, _, _, cx| this.install_raishot(cx)))
                .into_any_element()
            },
        )
        .when_some(self.install_status.clone(), |el, status| {
            el.child(caption(status))
        })
        .when_some(self.plugin_settings_error.clone(), |el, error| {
            el.child(notice(error))
        })
        .when_some(catalogue_error, |el, error| el.child(notice(error)))
        .child(
            div()
                .flex()
                .gap(px(8.))
                .child(
                    small(icon_button(
                        "refresh-plugins",
                        Icon::Refresh,
                        "刷新状态",
                        Tone::Ghost,
                        !installing && !busy,
                    ))
                    .on_click(cx.listener(|this, _, _, cx| this.refresh_plugins(cx))),
                )
                .when(ready, |el| {
                    el.child(
                        small(button(
                            "toggle-raishot",
                            "停用",
                            Tone::Ghost,
                            !installing && !busy,
                        ))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.save_plugin_settings(false, cx);
                        })),
                    )
                }),
        );
        let catalog = self.action_service.catalog();
        let count = catalog.actions.len();
        let packages = extension_card(
            Icon::Layers,
            theme::ACCENT_2,
            "本地动作包",
            if count == 0 {
                "放入包含 action.json 与 JavaScript 入口的动作包，重启后出现在动作库。".to_string()
            } else {
                format!("已加载 {count} 个动作；新增或修改清单后重启加载。")
            },
            status_pill(
                format!("{count} 个动作"),
                if count == 0 {
                    theme::TEXT_DIM
                } else {
                    theme::ACCENT
                },
            ),
        )
        .child(caption(format!(
            "动作目录：{}",
            crate::action_packages::root(&crate::settings::path()).display()
        )))
        .children(catalog.diagnostics.iter().cloned().map(notice));
        let page = div()
            .pt(px(6.))
            .pb(px(24.))
            .pl(px(20.))
            .pr(px(24.))
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(heading(
                "扩展",
                "连接浏览器、启用插件；新增的动作会出现在动作库",
            ))
            .children(self.render_banner(cx))
            .child(self.render_browser_connector(cx))
            .child(raishot)
            .child(packages);
        div()
            .id("extensions")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .child(page)
            .map(|el| self.smooth_scroll.wrap("extensions", el))
    }

    pub(super) fn plugin_action_fields(&self, cx: &mut Context<Self>) -> Div {
        let action = self.draft_action.clone();
        let reason = self.action_unavailable_reason(&action);
        let alternatives = self.plugins.catalogue().unwrap_or_default();
        let id = self
            .pending
            .as_ref()
            .map(|spec| spec.id)
            .or(self.selected)
            .unwrap_or(0);
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .p(px(12.))
                    .rounded(px(12.))
                    .bg(rgb(theme::BACKGROUND))
                    .border_1()
                    .border_color(rgb(theme::BORDER))
                    .child(icon_tile(Icon::Capture, theme::PINK, 34.))
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
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(match &action {
                                        ActionSpec::Plugin(reference) => self
                                            .plugins
                                            .action_title(reference)
                                            .unwrap_or_else(|| {
                                                format!(
                                                    "{} / {}",
                                                    reference.plugin_id, reference.action_id
                                                )
                                            }),
                                        _ => String::new(),
                                    }),
                            )
                            .child(caption("运行依赖由插件统一管理，保存绑定不执行动作。")),
                    ),
            )
            .when(alternatives.len() > 1, |el| {
                el.child(
                    div().flex().flex_wrap().gap(px(6.)).children(
                        alternatives
                            .into_iter()
                            .enumerate()
                            .map(|(index, alternative)| {
                                let selected =
                                    action == ActionSpec::Plugin(alternative.reference.clone());
                                button(
                                    ("choose-plugin-draft", index),
                                    alternative.title.clone(),
                                    if selected {
                                        Tone::Secondary
                                    } else {
                                        Tone::Ghost
                                    },
                                    !self.recording,
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        if this.recording
                                            || this.draft_action
                                                == ActionSpec::Plugin(alternative.reference.clone())
                                        {
                                            return;
                                        }
                                        let previous_title =
                                            this.plugins.catalogue().ok().and_then(|actions| {
                                                actions
                                                    .into_iter()
                                                    .find(|action| {
                                                        this.draft_action
                                                            == ActionSpec::Plugin(
                                                                action.reference.clone(),
                                                            )
                                                    })
                                                    .map(|action| action.title)
                                            });
                                        let name = this.message.read(cx).text();
                                        let use_default_name = name.trim().is_empty()
                                            || previous_title.as_deref() == Some(name.as_str());
                                        this.load_action(
                                            ActionSpec::Plugin(alternative.reference.clone()),
                                            cx,
                                        );
                                        if use_default_name {
                                            this.message.update(cx, |input, cx| {
                                                input.set_text(alternative.title.clone(), cx)
                                            });
                                        }
                                        cx.notify();
                                    },
                                ))
                            }),
                    ),
                )
            })
            .when_some(reason.clone(), |el, error| el.child(notice(error)))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        small(icon_button(
                            "configure-plugin",
                            Icon::Sliders,
                            "管理插件",
                            Tone::Secondary,
                            !self.recording,
                        ))
                        .flex_1()
                        .on_click(
                            cx.listener(|this, _, _, cx| this.open_page(Page::Extensions, cx)),
                        ),
                    )
                    .child(
                        small(icon_button(
                            "test-plugin-action",
                            Icon::Play,
                            "测试动作",
                            Tone::Secondary,
                            reason.is_none() && !self.recording && !self.action_busy(id, &action),
                        ))
                        .flex_1()
                        .on_click(cx.listener(|this, _, _, cx| this.test_action(cx))),
                    ),
            )
            .child(caption("测试会执行当前动作，不保存草稿，不计组合命中。"))
    }
}
