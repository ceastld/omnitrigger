mod calculator;
mod conversion;

use gpui::{
    div, prelude::*, px, size, App, Bounds, ClipboardItem, Context, DisplayId, Div, Entity,
    Focusable, FontWeight, KeyDownEvent, Subscription, TitlebarOptions, WeakEntity, Window,
    WindowBounds, WindowHandle, WindowKind, WindowOptions,
};

use crate::{
    appearance,
    assets::Icon,
    message_input::MessageInput,
    theme::{self, button, icon, rgb, Tone},
    Page, Workspace,
};
use calculator::{calculate, CalculationError, QUERY_LIMIT};
use conversion::{convert, ConversionError};
use omnitrigger_windows::ShortcutTarget;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

const WIDTH: f32 = 500.;
const PAGE_ROW_HEIGHT: f32 = 38.;
const PAGE_RESULT_LIMIT: usize = 4;

const PAGES: [(Page, &str); 9] = [
    (Page::Triggers, "热键 快捷键 组合 trigger shortcut hotkey"),
    (Page::Actions, "命令 action command library"),
    (Page::TextHistory, "剪贴板 复制 clipboard text history"),
    (Page::Settings, "偏好 配置 settings preferences"),
    (Page::Wheels, "径向 菜单 wheel radial menu"),
    (Page::Presets, "模板 preset template"),
    (Page::Extensions, "插件 浏览器 extension plugin browser"),
    (Page::History, "日志 命中 trigger history log"),
    (Page::Diagnostics, "监听 调试 输入 hook diagnostics"),
];

#[derive(PartialEq, Eq)]
enum Results {
    Value(Result<String, ValueError>),
    Pages(Vec<Page>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ValueError {
    Calculation(CalculationError),
    Conversion(ConversionError),
}

impl ValueError {
    fn message(self) -> &'static str {
        match self {
            Self::Calculation(error) => error.message(),
            Self::Conversion(error) => error.message(),
        }
    }

    fn is_incomplete(self) -> bool {
        matches!(
            self,
            Self::Calculation(CalculationError::Incomplete)
                | Self::Conversion(ConversionError::Incomplete)
        )
    }
}

impl Results {
    fn for_query(query: &str) -> Self {
        let query = query.trim();
        if query.starts_with(|ch| {
            matches!(ch, '0'..='9' | '０'..='９' | '=' | '＝' | '+' | '＋' | '-' | '－' | '−' | '.' | '．' | '(' | '（')
        }) {
            return Self::Value(match convert(query) {
                Some(result) => result.map_err(ValueError::Conversion),
                None => calculate(query).map_err(ValueError::Calculation),
            });
        }

        let query = query.to_lowercase();
        let mut pages: Vec<_> = PAGES
            .iter()
            .filter_map(|&(page, keywords)| {
                let title = page.title();
                if !query
                    .split_whitespace()
                    .all(|word| title.contains(word) || keywords.contains(word))
                {
                    return None;
                }
                let rank = if title == query {
                    0
                } else if title.starts_with(&query) {
                    1
                } else if title.contains(&query) {
                    2
                } else {
                    3
                };
                Some((rank, page))
            })
            .collect();
        pages.sort_by_key(|&(rank, _)| rank);
        Self::Pages(
            pages
                .into_iter()
                .take(PAGE_RESULT_LIMIT)
                .map(|(_, page)| page)
                .collect(),
        )
    }

    fn height(&self) -> f32 {
        match self {
            Self::Pages(pages) if !pages.is_empty() => 132. + pages.len() as f32 * PAGE_ROW_HEIGHT,
            _ => 158.,
        }
    }
}

pub(super) fn request_foreground(
    search: WindowHandle<QuickSearch>,
    target: Option<ShortcutTarget>,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        let hwnd = search
            .update(cx, |_, window, _| {
                match HasWindowHandle::window_handle(&*window).ok()?.as_raw() {
                    RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
                    _ => None,
                }
            })
            .ok()
            .flatten();
        if let Some(hwnd) = hwnd {
            // 原生激活可能同步进入窗口回调，等 GPUI 的可变借用结束后再请求。
            if omnitrigger_windows::is_foreground_window(hwnd)
                || target.is_some_and(|target| {
                    omnitrigger_windows::ensure_shortcut_target(target).is_ok()
                })
            {
                let _ = omnitrigger_windows::activate_panel_window(hwnd);
            }
        }
    })
    .detach();
}

pub(super) struct QuickSearch {
    theme_mode: appearance::Mode,
    input: Entity<MessageInput>,
    workspace: WeakEntity<Workspace>,
    was_active: bool,
    results: Results,
    selected: usize,
    _input_changes: Subscription,
    _activation: Subscription,
}

impl QuickSearch {
    pub(super) fn open(
        display: Option<DisplayId>,
        theme_mode: appearance::Mode,
        workspace: WeakEntity<Workspace>,
        cx: &mut App,
    ) -> Result<WindowHandle<Self>, String> {
        let results = Results::for_query("");
        let bounds = Bounds::centered(display, size(px(WIDTH), px(results.height())), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                display_id: display,
                titlebar: Some(TitlebarOptions {
                    title: Some("OmniTrigger · 快速搜索".into()),
                    appears_transparent: true,
                    ..Default::default()
                }),
                kind: WindowKind::PopUp,
                is_movable: false,
                is_resizable: false,
                is_minimizable: false,
                ..Default::default()
            },
            |window, cx| cx.new(|cx| Self::new(theme_mode, workspace, results, window, cx)),
        )
        .map_err(|error| format!("无法打开快速搜索：{error}"))
    }

    fn new(
        theme_mode: appearance::Mode,
        workspace: WeakEntity<Workspace>,
        results: Results,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            let mut input = MessageInput::new(cx)
                .embedded()
                .with_character_limit(QUERY_LIMIT + 1);
            input.set_placeholder("搜索页面，或输入 2**10、10 km to mi");
            input
        });
        let input_changes = cx.observe_in(&input, window, |this, input, window, cx| {
            let next = Results::for_query(&input.read(cx).text());
            if this.results != next {
                let height_changed = this.results.height() != next.height();
                this.results = next;
                this.selected = 0;
                if height_changed {
                    window.resize(size(px(WIDTH), px(this.results.height())));
                }
                cx.notify();
            }
        });
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.was_active = true;
                window.focus(&this.input.focus_handle(cx));
                cx.notify();
            } else if this.was_active {
                window.remove_window();
            }
        });
        window.focus(&input.focus_handle(cx));
        Self {
            theme_mode,
            input,
            workspace,
            was_active: window.is_window_active(),
            results,
            selected: 0,
            _input_changes: input_changes,
            _activation: activation,
        }
    }

    pub(super) fn activate(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.activate_window();
        window.focus(&self.input.focus_handle(cx));
    }

    fn run_page(&self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = self.workspace.clone();
        window.remove_window();
        cx.spawn(async move |_, cx| {
            let _ = workspace.update(cx, |workspace, cx| {
                workspace.open_quick_search_page(page, cx)
            });
        })
        .detach();
    }

    fn confirm(&self, window: &mut Window, cx: &mut Context<Self>) {
        match &self.results {
            Results::Value(Ok(result)) => {
                cx.write_to_clipboard(ClipboardItem::new_string(result.clone()));
                window.remove_window();
            }
            Results::Pages(pages) => {
                if let Some(&page) = pages.get(self.selected) {
                    self.run_page(page, window, cx);
                }
            }
            Results::Value(Err(_)) => {}
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.modifiers.number_of_modifiers() != 0
            || self.input.read(cx).is_composing()
        {
            return;
        }
        match event.keystroke.key.as_str() {
            "enter" => self.confirm(window, cx),
            "escape" => window.remove_window(),
            key @ ("up" | "down") => {
                let Results::Pages(pages) = &self.results else {
                    return;
                };
                if pages.is_empty() {
                    return;
                }
                self.selected = if key == "up" {
                    self.selected.saturating_sub(1)
                } else {
                    (self.selected + 1).min(pages.len() - 1)
                };
                cx.notify();
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    fn render_result(&self, cx: &mut Context<Self>) -> Div {
        let row = div().h(px(38.)).flex().items_center().gap(px(12.));
        match &self.results {
            Results::Value(Ok(result)) => row
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_family(theme::MONO_FONT)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(22.))
                        .text_color(rgb(theme::ACCENT))
                        .child(format!("= {result}")),
                )
                .child(
                    button("copy-calculation", "复制  Enter", Tone::Primary, true)
                        .flex_none()
                        .on_click(cx.listener(|this, _, window, cx| this.confirm(window, cx))),
                ),
            Results::Value(Err(error)) => row
                .text_color(rgb(if error.is_incomplete() {
                    theme::TEXT_MUTED
                } else {
                    theme::DANGER_TEXT
                }))
                .child(error.message()),
            Results::Pages(pages) if pages.is_empty() => row
                .text_color(rgb(theme::TEXT_MUTED))
                .child("未找到工作台页面"),
            Results::Pages(pages) => div()
                .flex()
                .flex_col()
                .children(pages.iter().enumerate().map(|(index, &page)| {
                    let selected = index == self.selected;
                    div()
                        .id(("quick-search-page", index))
                        .h(px(PAGE_ROW_HEIGHT))
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .px(px(10.))
                        .rounded(px(5.))
                        .cursor_pointer()
                        .when(selected, |row| row.bg(rgb(theme::SELECTED)))
                        .when(!selected, |row| {
                            row.hover(|style| style.bg(rgb(theme::ROW)))
                        })
                        .child(icon(page.icon(), 16., theme::ACCENT))
                        .child(div().flex_1().child(page.title()))
                        .when(selected, |row| {
                            row.child(div().text_color(rgb(theme::TEXT_MUTED)).child("Enter 打开"))
                        })
                        .on_click(
                            cx.listener(move |this, _, window, cx| this.run_page(page, window, cx)),
                        )
                }))
                .child(
                    div()
                        .mt(px(5.))
                        .px(px(10.))
                        .text_color(rgb(theme::TEXT_MUTED))
                        .child("↑ ↓ 选择"),
                ),
        }
    }
}

impl Render for QuickSearch {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        theme::apply(self.theme_mode, window.appearance());
        let focused = self.input.focus_handle(cx).is_focused(window);
        div()
            .id("quick-search")
            .size_full()
            .bg(rgb(theme::BACKGROUND))
            .border_1()
            .border_color(rgb(theme::BORDER_HI))
            .font_family(theme::UI_FONT)
            .text_color(rgb(theme::TEXT))
            .text_size(px(13.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .px(px(16.))
            .py(px(10.))
            .on_key_down(cx.listener(Self::key_down))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("快速搜索"))
                    .child(
                        div()
                            .id("close-quick-search")
                            .px(px(6.))
                            .py(px(3.))
                            .rounded(px(4.))
                            .cursor_pointer()
                            .text_color(rgb(theme::TEXT_MUTED))
                            .hover(|style| style.bg(rgb(theme::ROW)))
                            .on_click(|_, window, _| window.remove_window())
                            .child("Esc 关闭"),
                    ),
            )
            .child(
                div()
                    .h(px(44.))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .rounded(px(6.))
                    .bg(rgb(theme::ROW))
                    .border_1()
                    .border_color(rgb(if focused {
                        theme::ACCENT
                    } else {
                        theme::BORDER_HI
                    }))
                    .child(icon(Icon::Search, 16., theme::TEXT_DIM))
                    .child(div().flex_1().min_w_0().child(self.input.clone())),
            )
            .child(self.render_result(cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_search_matches_page_names_and_aliases() {
        for (query, expected) in [
            ("文本历史", vec![Page::TextHistory]),
            ("clipBOARD", vec![Page::TextHistory]),
            ("TRIGGER history", vec![Page::History]),
            ("触发", vec![Page::Triggers, Page::Presets, Page::History]),
            ("无对应页面", vec![]),
        ] {
            let Results::Pages(pages) = Results::for_query(query) else {
                panic!("页面查询被解析成了算式：{query}");
            };
            assert_eq!(pages, expected, "{query}");
        }
        assert!(
            matches!(Results::for_query(" "), Results::Pages(pages) if pages.len() == PAGE_RESULT_LIMIT)
        );
    }

    #[test]
    fn quick_search_keeps_ascii_and_fullwidth_calculations() {
        for query in ["2**10", "（２＾１０）", "＝２＾１０"] {
            assert!(
                matches!(Results::for_query(query), Results::Value(Ok(result)) if result == "1024")
            );
        }
        assert!(matches!(
            Results::for_query("2+"),
            Results::Value(Err(ValueError::Calculation(CalculationError::Incomplete)))
        ));
        assert!(
            matches!(Results::for_query("100厘米转米"), Results::Value(Ok(result)) if result == "1 m")
        );
    }
}
