mod calculator;
mod calendar;
mod conversion;
mod radix;
mod recent;
mod scientific;
mod scope;
mod triggers;
mod wheels;

use gpui::{
    div, prelude::*, px, size, App, Bounds, ClipboardItem, Context, DisplayId, Div, Entity,
    Focusable, FontWeight, KeyDownEvent, Subscription, TitlebarOptions, WeakEntity, Window,
    WindowBounds, WindowHandle, WindowKind, WindowOptions,
};

use crate::{
    appearance,
    assets::Icon,
    message_input::MessageInput,
    settings::Binding,
    theme::{self, button, icon, rgb, Tone},
    wheels::WheelDefinition,
    Page, Workspace,
};
use calculator::{calculate, looks_like_calculation, CalculationError, QUERY_LIMIT};
use calendar::{calculate_date, DateError};
use conversion::{convert, ConversionError};
use omnitrigger_windows::ShortcutTarget;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
pub(super) use recent::RecentDestinations;
use scope::{scoped_query, Scope};
use triggers::{matching_triggers, SearchTrigger};
use wheels::{matching_wheels, SearchWheel};

const WIDTH: f32 = 500.;
const PAGE_ROW_HEIGHT: f32 = 38.;
const PAGE_RESULT_LIMIT: usize = 4;
const VALUE_COLUMNS: usize = 40;
const VALUE_LINE_HEIGHT: f32 = 18.;

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
    Destinations(Vec<Destination>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Destination {
    Page(Page),
    Trigger(u64),
    Wheel(String),
    TextHistoryQuery(String),
    ActionLibraryQuery(String),
}

struct SearchIndex {
    triggers: Vec<SearchTrigger>,
    wheels: Vec<SearchWheel>,
    recent: RecentDestinations,
}

impl SearchIndex {
    fn results(&self, query: &str) -> Results {
        Results::for_query(query, &self.triggers, &self.wheels, &self.recent)
    }
}

fn title_rank(title: &str, query: &str) -> u8 {
    if title == query {
        0
    } else if title.starts_with(query) {
        1
    } else if title.contains(query) {
        2
    } else {
        3
    }
}

fn value_extra_height(value: &str) -> f32 {
    if value.is_ascii() {
        value.len().div_ceil(VALUE_COLUMNS).saturating_sub(2) as f32 * VALUE_LINE_HEIGHT
    } else {
        0.
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ValueError {
    Calculation(CalculationError),
    Conversion(ConversionError),
    Date(DateError),
}

impl ValueError {
    fn message(self) -> &'static str {
        match self {
            Self::Calculation(error) => error.message(),
            Self::Conversion(error) => error.message(),
            Self::Date(error) => error.message(),
        }
    }

    fn is_incomplete(self) -> bool {
        matches!(
            self,
            Self::Calculation(CalculationError::Incomplete)
                | Self::Conversion(ConversionError::Incomplete)
                | Self::Date(DateError::Incomplete)
        )
    }
}

impl Results {
    fn for_query(
        query: &str,
        triggers: &[SearchTrigger],
        wheels: &[SearchWheel],
        recent: &RecentDestinations,
    ) -> Self {
        let query = query.trim();
        if query.is_empty() {
            return Self::Destinations(recent.suggestions(triggers));
        }
        if let Some((scope, term)) = scoped_query(query) {
            return match scope {
                Scope::Triggers => Self::Destinations(
                    matching_triggers(triggers, term, PAGE_RESULT_LIMIT)
                        .into_iter()
                        .map(Destination::Trigger)
                        .collect(),
                ),
                Scope::Wheels => Self::Destinations(
                    matching_wheels(wheels, term, PAGE_RESULT_LIMIT)
                        .into_iter()
                        .map(|wheel| Destination::Wheel(wheel.id.clone()))
                        .collect(),
                ),
                Scope::TextHistory if term.is_empty() => {
                    Self::Destinations(vec![Destination::Page(Page::TextHistory)])
                }
                Scope::TextHistory => {
                    Self::Destinations(vec![Destination::TextHistoryQuery(term.to_owned())])
                }
                Scope::Actions => {
                    Self::Destinations(vec![Destination::ActionLibraryQuery(term.to_owned())])
                }
                Scope::Date => Self::Value(
                    calculate_date(term, chrono::Local::now().date_naive())
                        .map_err(ValueError::Date),
                ),
            };
        }
        if let Some(result) = convert(query) {
            return Self::Value(result.map_err(ValueError::Conversion));
        }
        if looks_like_calculation(query) {
            return Self::Value(calculate(query).map_err(ValueError::Calculation));
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
                Some((title_rank(title, &query), page))
            })
            .collect();
        pages.sort_by_key(|&(rank, _)| rank);
        let mut destinations: Vec<_> = pages
            .into_iter()
            .take(PAGE_RESULT_LIMIT)
            .map(|(_, page)| Destination::Page(page))
            .collect();
        destinations.extend(
            matching_triggers(triggers, &query, PAGE_RESULT_LIMIT - destinations.len())
                .into_iter()
                .map(Destination::Trigger),
        );
        destinations.extend(
            matching_wheels(wheels, &query, PAGE_RESULT_LIMIT - destinations.len())
                .into_iter()
                .map(|wheel| Destination::Wheel(wheel.id.clone())),
        );
        Self::Destinations(destinations)
    }

    fn height(&self) -> f32 {
        match self {
            Self::Destinations(items) if items.len() > 1 => {
                132. + items.len() as f32 * PAGE_ROW_HEIGHT
            }
            Self::Value(Ok(value)) => 158. + value_extra_height(value),
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
    index: SearchIndex,
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
        bindings: &[Binding],
        wheels: &[WheelDefinition],
        recent: &RecentDestinations,
        cx: &mut App,
    ) -> Result<WindowHandle<Self>, String> {
        let index = SearchIndex {
            triggers: bindings.iter().map(SearchTrigger::from_binding).collect(),
            wheels: wheels.iter().map(SearchWheel::from_definition).collect(),
            recent: recent.clone(),
        };
        let results = index.results("");
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
            |window, cx| cx.new(|cx| Self::new(theme_mode, workspace, index, results, window, cx)),
        )
        .map_err(|error| format!("无法打开快速搜索：{error}"))
    }

    fn new(
        theme_mode: appearance::Mode,
        workspace: WeakEntity<Workspace>,
        index: SearchIndex,
        results: Results,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            let mut input = MessageInput::new(cx)
                .embedded()
                .with_character_limit(QUERY_LIMIT + 1);
            input.set_placeholder("搜索页面、触发器，或输入算式");
            input
        });
        let input_changes = cx.observe_in(&input, window, |this, input, window, cx| {
            let next = this.index.results(&input.read(cx).text());
            if this.results != next {
                let height_changed = this.results.height() != next.height();
                this.results = next;
                this.selected = 0;
                if height_changed {
                    window.resize(size(px(WIDTH), px(this.results.height())));
                }
            }
            cx.notify();
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
            index,
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

    fn open_destination(
        &self,
        destination: Destination,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workspace = self.workspace.clone();
        window.remove_window();
        cx.spawn(async move |_, cx| {
            let _ = workspace.update(cx, |workspace, cx| {
                workspace.open_quick_search_destination(destination, cx)
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
            Results::Destinations(items) => {
                if let Some(destination) = items.get(self.selected).cloned() {
                    self.open_destination(destination, window, cx);
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
                let Results::Destinations(items) = &self.results else {
                    return;
                };
                if items.is_empty() {
                    return;
                }
                self.selected = if key == "up" {
                    self.selected.saturating_sub(1)
                } else {
                    (self.selected + 1).min(items.len() - 1)
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
            Results::Value(Ok(result)) => {
                let value = div()
                    .flex_1()
                    .min_w_0()
                    .font_family(theme::MONO_FONT)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::ACCENT));
                let value = if result.is_ascii() && result.len() > 28 {
                    value
                        .flex()
                        .flex_col()
                        .text_size(px(14.))
                        .line_height(px(VALUE_LINE_HEIGHT))
                        .children(result.as_bytes().chunks(VALUE_COLUMNS).enumerate().map(
                            |(index, digits)| {
                                let prefix = if index == 0 { "= " } else { "  " };
                                let digits =
                                    std::str::from_utf8(digits).expect("结果已核对为 ASCII");
                                div().child(format!("{prefix}{digits}"))
                            },
                        ))
                } else {
                    value
                        .truncate()
                        .text_size(px(22.))
                        .child(format!("= {result}"))
                };
                row.h(px(38. + value_extra_height(result)))
                    .child(value)
                    .child(
                        button("copy-calculation", "复制  Enter", Tone::Primary, true)
                            .flex_none()
                            .on_click(cx.listener(|this, _, window, cx| this.confirm(window, cx))),
                    )
            }
            Results::Value(Err(error)) => row
                .text_color(rgb(if error.is_incomplete() {
                    theme::TEXT_MUTED
                } else {
                    theme::DANGER_TEXT
                }))
                .child(error.message()),
            Results::Destinations(items) if items.is_empty() => {
                row.text_color(rgb(theme::TEXT_MUTED)).child("未找到匹配项")
            }
            Results::Destinations(items) => div()
                .flex()
                .flex_col()
                .children(items.iter().enumerate().filter_map(|(index, destination)| {
                    let (glyph, title, detail) = match destination {
                        Destination::Page(page) => (page.icon(), page.title().to_owned(), None),
                        Destination::Trigger(id) => {
                            let trigger = self
                                .index
                                .triggers
                                .iter()
                                .find(|trigger| trigger.id() == *id)?;
                            let detail = if trigger.enabled {
                                trigger.shortcut.clone()
                            } else {
                                format!("{} · 已停用", trigger.shortcut)
                            };
                            (trigger.icon, trigger.title.clone(), Some(detail))
                        }
                        Destination::Wheel(id) => {
                            let wheel = self.index.wheels.iter().find(|wheel| &wheel.id == id)?;
                            let duplicate = self
                                .index
                                .wheels
                                .iter()
                                .any(|other| other.id != *id && other.name == wheel.name);
                            let detail = if duplicate {
                                wheel.id.clone()
                            } else {
                                Page::Wheels.title().to_owned()
                            };
                            (Page::Wheels.icon(), wheel.name.clone(), Some(detail))
                        }
                        Destination::TextHistoryQuery(_) => {
                            (Icon::Clock, "搜索文本历史正文".to_owned(), None)
                        }
                        Destination::ActionLibraryQuery(term) => {
                            let title = if term.is_empty() {
                                "打开动作库"
                            } else {
                                "筛选动作库"
                            };
                            (Page::Actions.icon(), title.to_owned(), None)
                        }
                    };
                    let destination = destination.clone();
                    let selected = index == self.selected;
                    Some(
                        div()
                            .id(("quick-search-result", index))
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
                            .child(icon(glyph, 16., theme::ACCENT))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .child(div().truncate().child(title))
                                    .when_some(detail, |label, detail| {
                                        label.child(
                                            div()
                                                .truncate()
                                                .text_size(px(11.))
                                                .text_color(rgb(theme::TEXT_MUTED))
                                                .child(detail),
                                        )
                                    }),
                            )
                            .when(selected, |row| {
                                let hint = match &destination {
                                    Destination::TextHistoryQuery(_) => "Enter 查找",
                                    Destination::ActionLibraryQuery(term) if !term.is_empty() => {
                                        "Enter 筛选"
                                    }
                                    _ => "Enter 打开",
                                };
                                row.child(div().text_color(rgb(theme::TEXT_MUTED)).child(hint))
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_destination(destination.clone(), window, cx)
                            })),
                    )
                }))
                .when(items.len() > 1, |list| {
                    let hint = if self.input.read(cx).text().trim().is_empty() {
                        "↑ ↓ 选择  ·  例如 2**10、10 km to mi、剪贴板: 关键词、动作: 浏览器"
                    } else {
                        "↑ ↓ 选择"
                    };
                    list.child(
                        div()
                            .mt(px(5.))
                            .px(px(10.))
                            .text_color(rgb(theme::TEXT_MUTED))
                            .child(hint),
                    )
                }),
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

    fn for_query(query: &str, triggers: &[SearchTrigger]) -> Results {
        Results::for_query(query, triggers, &[], &RecentDestinations::default())
    }

    #[test]
    fn quick_search_routes_wheel_names_and_scope_to_wheel_entries() {
        let settings = crate::wheels::Wheels::default();
        let wheels = [SearchWheel::from_definition(&settings.wheels[0])];
        for query in ["编辑轨道", "wheel: orbit-edit", "轮盘：", "轨道轮盘：编辑"] {
            let result = Results::for_query(query, &[], &wheels, &RecentDestinations::default());
            assert!(
                matches!(result, Results::Destinations(items) if items == [Destination::Wheel("orbit-edit".into())]),
                "{query}"
            );
        }
    }

    #[test]
    fn quick_search_matches_page_names_and_aliases() {
        for (query, expected) in [
            ("文本历史", vec![Page::TextHistory]),
            ("clipBOARD", vec![Page::TextHistory]),
            ("TRIGGER history", vec![Page::History]),
            ("触发", vec![Page::Triggers, Page::Presets, Page::History]),
            ("无对应页面", vec![]),
        ] {
            let Results::Destinations(items) = for_query(query, &[]) else {
                panic!("页面查询被解析成了算式：{query}");
            };
            assert_eq!(
                items,
                expected
                    .into_iter()
                    .map(Destination::Page)
                    .collect::<Vec<_>>(),
                "{query}"
            );
        }
        assert!(
            matches!(for_query(" ", &[]), Results::Destinations(items) if items.len() == PAGE_RESULT_LIMIT)
        );
    }

    #[test]
    fn quick_search_keeps_ascii_and_fullwidth_calculations() {
        for query in ["2**10", "（２＾１０）", "＝２＾１０"] {
            assert!(
                matches!(for_query(query, &[]), Results::Value(Ok(result)) if result == "1024")
            );
        }
        assert!(matches!(
            for_query("2+", &[]),
            Results::Value(Err(ValueError::Calculation(CalculationError::Incomplete)))
        ));
        assert!(
            matches!(for_query("100厘米转米", &[]), Results::Value(Ok(result)) if result == "1 m")
        );
        assert!(matches!(
            for_query("日期：2024-02-28 + 1天", &[]),
            Results::Value(Ok(result)) if result == "2024-02-29"
        ));
    }

    #[test]
    fn quick_search_finds_configured_triggers_by_name_and_chord() {
        let mut bindings = crate::settings::defaults();
        bindings[0].message = "打开正文".into();
        bindings[1].message = "打开正文预览".into();
        let triggers: Vec<_> = bindings.iter().map(SearchTrigger::from_binding).collect();
        let first = Destination::Trigger(bindings[0].spec.id);
        let second = Destination::Trigger(bindings[1].spec.id);
        let panel = bindings
            .iter()
            .find(|binding| {
                binding.spec.held == [omnitrigger_core::Button::Key(20)]
                    && matches!(binding.action, crate::settings::ActionSpec::ShowPanel)
            })
            .unwrap();
        for (query, expected) in [
            ("打开正文", vec![first.clone(), second.clone()]),
            ("Caps ＋ W", vec![first.clone()]),
            ("触发器：打开正文", vec![first, second]),
            (
                "TRIGGER: Caps+Space",
                vec![Destination::Trigger(panel.spec.id)],
            ),
        ] {
            let Results::Destinations(items) = for_query(query, &triggers) else {
                panic!("触发器查询被解析成了算式：{query}");
            };
            assert_eq!(items, expected, "{query}");
        }
    }

    #[test]
    fn quick_search_routes_text_history_and_actions_queries_to_their_pages() {
        for (query, destination) in [
            ("剪贴板:", Destination::Page(Page::TextHistory)),
            (
                "剪贴板：Rust 工程",
                Destination::TextHistoryQuery("Rust 工程".into()),
            ),
            (
                "CLIPBOARD： 2**10",
                Destination::TextHistoryQuery("2**10".into()),
            ),
            ("ACTION:", Destination::ActionLibraryQuery(String::new())),
            (
                "动作库 ： 浏览器",
                Destination::ActionLibraryQuery("浏览器".into()),
            ),
            (
                "action: 2**10",
                Destination::ActionLibraryQuery("2**10".into()),
            ),
        ] {
            let Results::Destinations(items) = for_query(query, &[]) else {
                panic!("页面筛选词被解析成了算式：{query}");
            };
            assert_eq!(items, vec![destination], "{query}");
        }
    }
}
