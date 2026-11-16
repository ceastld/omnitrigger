//! 设置页提供外观选择和应用更新。
use crate::{appearance::Mode, caption, card, heading, icon, profiles, theme, Workspace};
use gpui::{div, prelude::*, px, Context, Div, FontWeight, Window};

fn preview(dark: bool) -> Div {
    // 缩略图显示选项本身的颜色，不随当前主题反转。
    let background = gpui::rgb(if dark { 0x1c1e25 } else { 0xf4f5f7 });
    let panel = gpui::rgb(if dark { 0x2c303b } else { 0xe0e4ea });
    let line = gpui::rgb(if dark { 0x778192 } else { 0xa0a8b5 });
    div()
        .h(px(80.))
        .w_full()
        .rounded(px(6.))
        .overflow_hidden()
        .bg(background)
        .flex()
        .child(div().w(px(28.)).h_full().bg(panel))
        .child(
            div()
                .flex_1()
                .p(px(12.))
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(div().h(px(5.)).w(px(46.)).rounded(px(2.)).bg(line))
                .child(div().h(px(8.)).w_full().rounded(px(2.)).bg(panel))
                .child(div().h(px(8.)).w_full().rounded(px(2.)).bg(panel)),
        )
}

enum TestProfileAction {
    List,
    DailyCopy,
    Open(profiles::SavedTestProfile),
}

impl Workspace {
    pub(super) fn open_edit_history(&mut self, cx: &mut Context<Self>) {
        let config = crate::settings::path();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let folder = crate::settings::edit_history_folder_to_open(&config)?;
                    omnitrigger_windows::open_target(&folder)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.backup_notice = result.err();
                cx.notify();
            });
        })
        .detach();
    }

    fn select_test_profile(&mut self, action: TestProfileAction, cx: &mut Context<Self>) {
        if self.test_profiles_busy || self.test_data {
            return;
        }
        self.test_profiles_error = None;
        if matches!(action, TestProfileAction::List) && self.saved_test_profiles.take().is_some() {
            cx.notify();
            return;
        }
        let config = crate::settings::path();
        let Some(root) = config.parent() else { return };
        let root = if root.as_os_str().is_empty() {
            std::path::Path::new(".")
        } else {
            root
        }
        .to_owned();
        let list_visible = self.saved_test_profiles.is_some();
        self.test_profiles_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match action {
                        TestProfileAction::List => profiles::saved_test_profiles(&root).map(Some),
                        TestProfileAction::DailyCopy => {
                            profiles::open_daily_test_copy(&root)?;
                            if list_visible {
                                profiles::saved_test_profiles(&root).map(Some)
                            } else {
                                Ok(None)
                            }
                        }
                        TestProfileAction::Open(selected) => selected.open().map(|()| None),
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.test_profiles_busy = false;
                match result {
                    Ok(Some(items)) => this.saved_test_profiles = Some(items),
                    Ok(None) => {}
                    Err(error) => this.test_profiles_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_test_profiles(&self, can_copy: bool, cx: &mut Context<Self>) -> Div {
        let enabled = !self.test_profiles_busy;
        let list_label = if self.saved_test_profiles.is_some() {
            "收起测试档"
        } else {
            "查看已有测试档"
        };
        card()
            .p(px(18.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(16.))
                    .child(
                        div()
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("测试配置"),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(8.))
                            .child(
                                theme::button(
                                    "list-test-profiles",
                                    list_label,
                                    theme::Tone::Secondary,
                                    enabled,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.select_test_profile(TestProfileAction::List, cx)
                                    },
                                )),
                            )
                            .when(can_copy, |el| {
                                el.child(
                                    theme::button(
                                        "open-test-copy",
                                        "打开测试副本",
                                        theme::Tone::Primary,
                                        enabled,
                                    )
                                    .on_click(cx.listener(
                                        |this, _, _, cx| {
                                            this.select_test_profile(
                                                TestProfileAction::DailyCopy,
                                                cx,
                                            )
                                        },
                                    )),
                                )
                            }),
                    ),
            )
            .when(can_copy, |el| {
                el.child(caption(
                    "首次复制已保存的热键、轮盘和动作。之后打开同一副本，保留测试修改。",
                ))
            })
            .when_some(self.saved_test_profiles.as_ref(), |el, items| {
                el.when(items.is_empty(), |el| {
                    el.child(caption("当前数据目录还没有测试档。"))
                })
                .children(items.iter().enumerate().map(|(i, profile)| {
                    let selected = profile.clone();
                    let (name, alias) = profile.labels();
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(12.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(div().truncate().child(name.to_owned()))
                                .when(name != alias, |el| el.child(caption(alias.to_owned()))),
                        )
                        .child(
                            theme::button(
                                ("open-test-profile", i),
                                "打开",
                                theme::Tone::Secondary,
                                enabled,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.select_test_profile(
                                        TestProfileAction::Open(selected.clone()),
                                        cx,
                                    )
                                },
                            )),
                        )
                }))
            })
            .when(self.test_profiles_busy, |el| {
                el.child(caption("正在读取配置档…"))
            })
            .when_some(self.test_profiles_error.clone(), |el, error| {
                el.child(caption(error).text_color(theme::rgb(theme::DANGER)))
            })
    }

    fn choose_theme(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        match crate::appearance::save(&crate::settings::path(), mode) {
            Ok(()) => {
                self.theme_mode = mode;
                self.theme_error = None;
                self.smooth_scroll.cancel_all();
                theme::apply(mode, window.appearance());
                cx.refresh_windows();
            }
            Err(error) => self.theme_error = Some(error),
        }
        cx.notify();
    }

    pub(super) fn render_appearance(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let test_profiles = (!self.test_data).then(|| {
            let can_copy = crate::settings::path()
                .file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("bindings.json"));
            self.render_test_profiles(can_copy, cx)
        });
        let choices = Mode::ALL.into_iter().enumerate().map(|(index, mode)| {
            let selected = self.theme_mode == mode;
            card()
                .id(("appearance-theme", index as u64))
                .flex_1()
                .min_w(px(140.))
                .max_w(px(220.))
                .p(px(12.))
                .cursor_pointer()
                .border_color(if selected {
                    theme::hsla(theme::ACCENT, 0.85)
                } else {
                    theme::rgb(theme::BORDER).into()
                })
                .hover(|style| style.border_color(theme::rgb(theme::BORDER_HI)))
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(preview(mode.is_dark(window.appearance())))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(mode.label()),
                        )
                        .when(selected, |el| {
                            el.child(icon(crate::Icon::Check, 15., theme::ACCENT))
                        }),
                )
                .on_click(
                    cx.listener(move |this, _, window, cx| this.choose_theme(mode, window, cx)),
                )
        });
        let quit = theme::button(
            "quit-omnitrigger",
            "退出 OmniTrigger",
            theme::Tone::Secondary,
            !self.legacy_close_requested,
        )
        .on_click(cx.listener(|this, _, _, cx| {
            this.request_legacy_close(cx);
        }));
        div().id("appearance-settings").flex_1().min_w_0().h_full().overflow_y_scroll()
            .child(div().pt(px(6.)).pb(px(24.)).pl(px(20.)).pr(px(24.)).flex().flex_col().gap(px(24.))
                .child(heading("设置", format!("{} · Windows x64 预览版", crate::BUILD_LABEL)))
                .children(test_profiles)
                .child(self.render_backup_settings(cx))
                .child(div().flex().flex_col().gap(px(14.))
                    .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child("颜色主题"))
                    .child(caption("选择后立即生效，自动保存。跟随系统会与 Windows 的浅色或深色模式保持一致。"))
                    .child(div().flex().flex_wrap().gap(px(12.)).children(choices)))
                .when_some(self.theme_error.clone(), |el, error| el.child(caption(error).text_color(theme::rgb(theme::DANGER))))
                .when(!self.test_data, |el| el.child(self.render_update(cx)).child(div().flex().child(quit))))
            .map(|el| self.smooth_scroll.wrap("appearance-settings", el))
    }
}
