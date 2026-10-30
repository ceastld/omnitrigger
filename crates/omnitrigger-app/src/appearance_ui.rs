//! 设置页仅提供常用外观选择，修改后立即生效并自动保存。
use crate::{appearance::Mode, caption, card, heading, icon, theme, Workspace};
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

impl Workspace {
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
        div().id("appearance-settings").flex_1().min_w_0().h_full().overflow_y_scroll()
            .child(div().pt(px(6.)).pb(px(24.)).pl(px(20.)).pr(px(24.)).flex().flex_col().gap(px(24.))
                .child(heading("设置", "让 OmniTrigger 更适合你的使用习惯"))
                .child(div().flex().flex_col().gap(px(14.))
                    .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child("颜色主题"))
                    .child(caption("选择后立即生效，自动保存。跟随系统会与 Windows 的浅色或深色模式保持一致。"))
                    .child(div().flex().flex_wrap().gap(px(12.)).children(choices)))
                .when_some(self.theme_error.clone(), |el, error| el.child(caption(error).text_color(theme::rgb(theme::DANGER)))))
            .map(|el| self.smooth_scroll.wrap("appearance-settings", el))
    }
}
