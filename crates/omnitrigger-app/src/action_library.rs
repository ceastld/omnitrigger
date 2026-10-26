//! 插件动作目录与安装设置，和输入匹配保持独立。
use crate::{
    button, caption, card, combo, heading, icon, icon_button, icon_tile, section_title, small,
    status_pill, theme, ActionSpec, CapSize, Context, Div, Icon, Page, PluginActionRef, Tone,
    Workspace,
};
use gpui::{div, prelude::*, px, rgb, Focusable, FontWeight, MouseButton, Window};

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

impl Workspace {
    pub(super) fn render_actions(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let reference = PluginActionRef::raishot_capture();
        let reason = self.action_unavailable_reason(&ActionSpec::Plugin(reference.clone()));
        let ready = reason.is_none();
        let query = self.action_search.read(cx).text().trim().to_lowercase();
        let matches = query.is_empty() || "raishot 截图 capture".contains(&query);
        let count = self.bindings.iter().filter(|binding| matches!(&binding.action,
            ActionSpec::Plugin(action) if action.plugin_id == "raishot" && action.action_id == "capture")).count();
        let default_trigger = self.plugins.default_trigger(&reference, 1).ok();
        let focused = self.action_search.focus_handle(cx).is_focused(window);
        let installing = self.install_events.is_some();
        let search = div()
            .h(px(42.))
            .px(px(14.))
            .rounded(px(12.))
            .bg(rgb(0x0a0b11))
            .border_1()
            .border_color(if focused {
                theme::hsla(theme::ACCENT, 0.7)
            } else {
                rgb(theme::BORDER_MID).into()
            })
            .when(focused, |el| {
                el.shadow(theme::glow(theme::ACCENT, 0.18, 10.))
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
        let capture = card()
            .p(px(20.))
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .child(icon_tile(Icon::Capture, theme::PINK, 48.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(
                                div()
                                    .text_size(px(18.))
                                    .font_weight(FontWeight::BOLD)
                                    .child("截图"),
                            )
                            .child(caption(format!("RaiShot 插件 · 已有 {count} 条绑定"))),
                    )
                    .child(if ready {
                        status_pill("已就绪", theme::ACCENT)
                    } else {
                        status_pill("待配置", theme::RECORD)
                    }),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(rgb(theme::TEXT_MUTED))
                    .child("打开 RaiShot 截图。无需填写命令参数，可绑定键盘、鼠标组合或长按。"),
            )
            .when_some(default_trigger, |el, spec| {
                el.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(10.))
                        .px(px(12.))
                        .py(px(10.))
                        .rounded(px(10.))
                        .bg(rgb(0x0c0e14))
                        .border_1()
                        .border_color(rgb(theme::BORDER))
                        .child(caption("默认快捷键，可自定义或恢复"))
                        .child(combo(&crate::caps_for(&spec), CapSize::Medium)),
                )
            })
            .when_some(reason, |el, reason| el.child(notice(reason)))
            .child(
                div().flex().justify_end().child(
                    icon_button(
                        "bind-raishot-capture",
                        Icon::Bolt,
                        if self.pending.is_some()
                            && self.draft_action
                                == ActionSpec::Plugin(PluginActionRef::raishot_capture())
                        {
                            "继续设置触发"
                        } else {
                            "设置触发"
                        },
                        Tone::Primary,
                        !self.recording,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.choose_capture(cx))),
                ),
            );
        let settings = card()
            .p(px(20.))
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(icon_tile(Icon::Sliders, theme::ACCENT_2, 36.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child("RaiShot 插件设置"))
                            .child(caption("安装会下载 RaiShot 及其运行依赖，校验后保存到当前配置目录。安装本身不会执行截图或注册快捷键。")),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(
                        icon_button(
                            "install-raishot",
                            Icon::ArrowDown,
                            if installing { "正在安装…" } else { "安装 RaiShot 插件" },
                            Tone::Primary,
                            !installing && !self.executor.is_group_running("raishot/capture"),
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.install_raishot(cx))),
                    )
                    .child(
                        icon_button("refresh-plugins", Icon::Refresh, "刷新状态", Tone::Secondary, !installing)
                            .on_click(cx.listener(|this, _, _, cx| this.refresh_plugins(cx))),
                    ),
            )
            .when_some(self.install_status.clone(), |el, status| {
                el.child(
                    div()
                        .px(px(12.))
                        .py(px(9.))
                        .rounded(px(10.))
                        .bg(rgb(0x0c0e14))
                        .border_1()
                        .border_color(rgb(theme::BORDER))
                        .text_size(px(12.))
                        .text_color(rgb(theme::TEXT_MUTED))
                        .child(status),
                )
            })
            .child(caption("当前发布包需仓库访问权限；公开下载不可用时使用本机已登录的 GitHub CLI。已有安装也可直接填写下方路径。"))
            .child(Self::input_field(
                "已有程序路径（可选）",
                &self.plugin_path,
                Some("填写 raishot-cli.exe 完整路径；保留原安装中的其他程序和运行库。"),
            ))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(
                        button("save-raishot-path", "保存并启用", Tone::Secondary, !installing).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.save_plugin_settings(true, cx);
                            }),
                        ),
                    )
                    .child(
                        button(
                            "toggle-raishot",
                            if self.plugins.raishot_enabled() { "停用插件" } else { "启用插件" },
                            Tone::Ghost,
                            !installing,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.save_plugin_settings(!this.plugins.raishot_enabled(), cx);
                        })),
                    ),
            )
            .when_some(self.plugin_settings_error.clone(), |el, error| el.child(notice(error)));
        let page = div()
            .pt(px(6.))
            .pb(px(24.))
            .pl(px(28.))
            .pr(px(24.))
            .flex()
            .flex_col()
            .gap(px(18.))
            .child(heading(
                "动作库",
                "选择预设动作，再设置触发；插件安装会下载所需程序与运行依赖",
            ))
            .children(self.render_banner(cx))
            .child(search)
            .child(section_title("插件动作"))
            .when(matches, |el| el.child(capture))
            .when(!matches, |el| {
                el.child(
                    div()
                        .py(px(28.))
                        .flex()
                        .justify_center()
                        .text_size(px(13.))
                        .text_color(rgb(theme::TEXT_DIM))
                        .child("没有匹配的动作。"),
                )
            })
            .child(section_title("插件设置"))
            .child(settings);
        // 外层负责滚动、内层负责纵向排列，避免内容在可滚动的弹性容器中被压缩。
        div()
            .id("action-library")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .child(page)
    }

    pub(super) fn plugin_action_fields(&self, cx: &mut Context<Self>) -> Div {
        let action = self.draft_action.clone();
        let reason = self.action_unavailable_reason(&action);
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
                    .bg(rgb(0x0c0e14))
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
                                        ActionSpec::Plugin(reference)
                                            if reference.plugin_id == "raishot"
                                                && reference.action_id == "capture" =>
                                        {
                                            "RaiShot / 截图".into()
                                        }
                                        ActionSpec::Plugin(reference) => {
                                            format!(
                                                "{} / {}",
                                                reference.plugin_id, reference.action_id
                                            )
                                        }
                                        _ => String::new(),
                                    }),
                            )
                            .child(caption("程序路径由插件统一管理，保存绑定不执行截图。")),
                    ),
            )
            .when_some(reason.clone(), |el, error| el.child(notice(error)))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        small(icon_button(
                            "configure-plugin",
                            Icon::Sliders,
                            "安装 / 配置",
                            Tone::Secondary,
                            !self.recording,
                        ))
                        .flex_1()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.page = Page::Actions;
                            cx.notify();
                        })),
                    )
                    .child(
                        small(icon_button(
                            "test-plugin-action",
                            Icon::Play,
                            "测试截图",
                            Tone::Secondary,
                            reason.is_none() && !self.recording && !self.action_busy(id, &action),
                        ))
                        .flex_1()
                        .on_click(cx.listener(|this, _, _, cx| this.test_action(cx))),
                    ),
            )
            .child(caption("测试会真实打开截图，不保存草稿，不计组合命中。"))
    }
}
