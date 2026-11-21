mod actions;
mod calculator;
mod calendar;
mod conversion;
mod duration;
mod examples;
mod operators;
mod percentage;
mod presets;
mod radix;
mod recent;
mod scientific;
mod scope;
mod settings_sections;
mod timestamp;
mod triggers;
mod wheel_slots;
mod wheels;

use gpui::{
    div, prelude::*, px, size, App, Bounds, ClipboardItem, Context, DisplayId, Div, Entity,
    Focusable, FontWeight, KeyDownEvent, Stateful, Subscription, TitlebarOptions, WeakEntity,
    Window, WindowBounds, WindowHandle, WindowKind, WindowOptions,
};

use crate::appearance_ui::SettingsSection;
use crate::{
    appearance,
    assets::Icon,
    message_input::MessageInput,
    theme::{self, button, icon, rgb, Tone},
    wheel_scenes::WheelSlotKey,
    Page, Workspace,
};
use actions::{matching_actions, SearchAction};
use calculator::{calculate, looks_like_calculation, CalculationError, QUERY_LIMIT};
use calendar::{calculate_date, DateError};
use conversion::{convert, ConversionError};
use duration::{convert_duration, DurationError};
use examples::{matching_examples, EXAMPLES};
use omnitrigger_windows::ShortcutTarget;
use presets::matching_presets;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
pub(super) use recent::RecentDestinations;
use scope::{scoped_query, Scope};
use settings_sections::matching_settings;
use timestamp::{convert_timestamp, TimestampError};
use triggers::{matching_triggers, SearchTrigger};
use wheel_slots::{index_slots, matching_slots, SearchWheelSlot};
use wheels::{matching_wheels, SearchWheel};

const WIDTH: f32 = 500.;
const PAGE_ROW_HEIGHT: f32 = 38.;
const PAGE_RESULT_LIMIT: usize = 4;
const EXAMPLE_ROW_HEIGHT: f32 = 30.;
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
    Help(Vec<usize>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Destination {
    Page(Page),
    Trigger(u64),
    Wheel(String),
    WheelSlot {
        key: WheelSlotKey,
        label: String,
        detail: String,
    },
    Preset(&'static str),
    SettingsSection(SettingsSection),
    TextHistoryQuery(String),
    ActionLibraryQuery(String),
    ActionLibraryEntry {
        title: String,
        provider: String,
        filter: String,
    },
}

pub(super) struct SearchIndex {
    triggers: Vec<SearchTrigger>,
    wheels: Vec<SearchWheel>,
    wheel_slots: Option<Vec<SearchWheelSlot>>,
    actions: Vec<SearchAction>,
    settings: Vec<SettingsSection>,
    recent: RecentDestinations,
}

impl SearchIndex {
    pub(super) fn from_workspace(workspace: &Workspace) -> Self {
        let mut actions: Vec<_> = workspace
            .action_service
            .catalog()
            .actions
            .iter()
            .map(SearchAction::from_package)
            .collect();
        actions.extend(
            workspace
                .plugins
                .catalogue()
                .unwrap_or_default()
                .iter()
                .map(SearchAction::from_plugin),
        );
        let wheels = if workspace.wheels_error.is_none() {
            workspace
                .wheels
                .wheels
                .iter()
                .map(SearchWheel::from_definition)
                .collect()
        } else {
            Vec::new()
        };
        let standard_config = Workspace::settings_standard_config();
        Self {
            triggers: workspace
                .triggers
                .bindings()
                .iter()
                .map(SearchTrigger::from_binding)
                .collect(),
            wheels,
            wheel_slots: None,
            actions,
            settings: SettingsSection::ALL
                .into_iter()
                .filter(|section| section.available(standard_config, workspace.test_data))
                .collect(),
            recent: workspace.quick_search_recent.clone(),
        }
    }

    fn results(&self, query: &str) -> Results {
        Results::for_query(
            query,
            &self.triggers,
            &self.wheels,
            self.wheel_slots.as_deref().unwrap_or_default(),
            &self.actions,
            &self.settings,
            &self.recent,
        )
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
    Duration(DurationError),
    Timestamp(TimestampError),
}

impl ValueError {
    fn message(self) -> &'static str {
        match self {
            Self::Calculation(error) => error.message(),
            Self::Conversion(error) => error.message(),
            Self::Date(error) => error.message(),
            Self::Duration(error) => error.message(),
            Self::Timestamp(error) => error.message(),
        }
    }

    fn is_incomplete(self) -> bool {
        matches!(
            self,
            Self::Calculation(CalculationError::Incomplete)
                | Self::Conversion(ConversionError::Incomplete)
                | Self::Date(DateError::Incomplete)
                | Self::Duration(DurationError::Incomplete)
                | Self::Timestamp(TimestampError::Incomplete)
        )
    }
}

impl Results {
    fn for_query(
        query: &str,
        triggers: &[SearchTrigger],
        wheels: &[SearchWheel],
        wheel_slots: &[SearchWheelSlot],
        actions: &[SearchAction],
        settings: &[SettingsSection],
        recent: &RecentDestinations,
    ) -> Self {
        let query = query.trim();
        if query.is_empty() {
            return Self::Destinations(recent.suggestions(triggers));
        }
        if let Some(terms) = query.strip_prefix(['?', '？']) {
            return Self::Help(matching_examples(terms));
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
                Scope::WheelSlots if term.is_empty() => {
                    Self::Destinations(vec![Destination::Page(Page::Wheels)])
                }
                Scope::WheelSlots => {
                    Self::Destinations(matching_slots(wheel_slots, term, PAGE_RESULT_LIMIT))
                }
                Scope::Presets if term.is_empty() => {
                    Self::Destinations(vec![Destination::Page(Page::Presets)])
                }
                Scope::Presets => Self::Destinations(
                    matching_presets(term, PAGE_RESULT_LIMIT)
                        .into_iter()
                        .map(|preset| Destination::Preset(preset.key))
                        .collect(),
                ),
                Scope::Settings if term.is_empty() => {
                    Self::Destinations(vec![Destination::Page(Page::Settings)])
                }
                Scope::Settings => Self::Destinations(
                    matching_settings(settings, term, PAGE_RESULT_LIMIT)
                        .into_iter()
                        .map(Destination::SettingsSection)
                        .collect(),
                ),
                Scope::TextHistory if term.is_empty() => {
                    Self::Destinations(vec![Destination::Page(Page::TextHistory)])
                }
                Scope::TextHistory => {
                    Self::Destinations(vec![Destination::TextHistoryQuery(term.to_owned())])
                }
                Scope::Actions => {
                    let mut entries = matching_actions(actions, term, PAGE_RESULT_LIMIT - 1);
                    entries.push(Destination::ActionLibraryQuery(term.to_owned()));
                    Self::Destinations(entries)
                }
                Scope::Date => Self::Value(
                    calculate_date(term, chrono::Local::now().date_naive())
                        .map_err(ValueError::Date),
                ),
                Scope::Duration => {
                    Self::Value(convert_duration(term).map_err(ValueError::Duration))
                }
                Scope::Timestamp => Self::Value(
                    convert_timestamp(term, chrono::Utc::now()).map_err(ValueError::Timestamp),
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
        destinations.extend(
            matching_presets(&query, PAGE_RESULT_LIMIT - destinations.len())
                .into_iter()
                .map(|preset| Destination::Preset(preset.key)),
        );
        destinations.extend(matching_actions(
            actions,
            &query,
            PAGE_RESULT_LIMIT - destinations.len(),
        ));
        destinations.extend(
            matching_settings(settings, &query, PAGE_RESULT_LIMIT - destinations.len())
                .into_iter()
                .map(Destination::SettingsSection),
        );
        Self::Destinations(destinations)
    }

    fn height(&self) -> f32 {
        match self {
            Self::Destinations(items) if items.len() > 1 => {
                132. + items.len() as f32 * PAGE_ROW_HEIGHT
            }
            Self::Value(Ok(value)) => 158. + value_extra_height(value),
            Self::Help(items) if !items.is_empty() => {
                132. + items.len() as f32 * EXAMPLE_ROW_HEIGHT
            }
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
        index: SearchIndex,
        cx: &mut App,
    ) -> Result<WindowHandle<Self>, String> {
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
            let query = input.read(cx).text();
            if this.index.wheel_slots.is_none()
                && matches!(scoped_query(query.trim()), Some((Scope::WheelSlots, term)) if !term.is_empty())
            {
                this.index.wheel_slots = Some(this.workspace.upgrade().map_or_else(Vec::new, |workspace| {
                    let workspace = workspace.read(cx);
                    if workspace.wheels_error.is_none() {
                        index_slots(&workspace.wheels.wheels)
                    } else {
                        Vec::new()
                    }
                }));
            }
            let next = this.index.results(&query);
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
            Results::Help(items) => {
                if let Some(&index) = items.get(self.selected) {
                    self.fill_example(EXAMPLES[index].query, window, cx);
                }
            }
            Results::Value(Err(_)) => {}
        }
    }

    fn fill_example(&self, query: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| {
            input.set_text(query, cx);
            input.move_cursor_to_end(cx);
        });
        window.focus(&self.input.focus_handle(cx));
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.modifiers.number_of_modifiers() != 0
            || self.input.read(cx).is_composing()
        {
            return;
        }
        match event.keystroke.key.as_str() {
            "enter" => {
                if !event.is_held {
                    self.confirm(window, cx);
                }
            }
            "escape" => window.remove_window(),
            key @ ("up" | "down") => {
                let count = match &self.results {
                    Results::Destinations(items) => items.len(),
                    Results::Help(items) => items.len(),
                    Results::Value(_) => 0,
                };
                if count == 0 {
                    return;
                }
                self.selected = if key == "up" {
                    self.selected.saturating_sub(1)
                } else {
                    (self.selected + 1).min(count - 1)
                };
                cx.notify();
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    fn suggestion_row(&self, index: usize, height: f32) -> Stateful<Div> {
        let selected = index == self.selected;
        div()
            .id(("quick-search-result", index))
            .h(px(height))
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
    }

    fn selection_hint(label: &'static str) -> Div {
        div()
            .mt(px(5.))
            .px(px(10.))
            .text_color(rgb(theme::TEXT_MUTED))
            .child(label)
    }

    fn render_examples(&self, items: &[usize], cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .children(items.iter().enumerate().map(|(index, &example)| {
                let example = &EXAMPLES[example];
                let query = example.query;
                self.suggestion_row(index, EXAMPLE_ROW_HEIGHT)
                    .child(icon(example.icon, 16., theme::ACCENT))
                    .child(div().w(px(130.)).truncate().child(example.title))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(theme::MONO_FONT)
                            .text_size(px(12.))
                            .text_color(rgb(theme::TEXT_MUTED))
                            .child(query.trim_end()),
                    )
                    .when(index == self.selected, |row| {
                        row.child(div().text_color(rgb(theme::TEXT_MUTED)).child("Enter 填入"))
                    })
                    .on_click(
                        cx.listener(move |this, _, window, cx| {
                            this.fill_example(query, window, cx)
                        }),
                    )
            }))
            .when(items.len() > 1, |list| {
                let hint = if matches!(self.input.read(cx).text().trim(), "?" | "？") {
                    "↑ ↓ 选择  ·  ? 后可加关键词"
                } else {
                    "↑ ↓ 选择"
                };
                list.child(Self::selection_hint(hint))
            })
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
            Results::Help(items) if items.is_empty() => row
                .text_color(rgb(theme::TEXT_MUTED))
                .child("未找到用法示例"),
            Results::Help(items) => self.render_examples(items, cx),
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
                        Destination::WheelSlot { label, detail, .. } => {
                            (Page::Wheels.icon(), label.clone(), Some(detail.clone()))
                        }
                        Destination::Preset(key) => {
                            let preset = crate::presets::PRESETS
                                .iter()
                                .find(|preset| preset.key == *key)?;
                            (
                                Page::Presets.icon(),
                                preset.title.to_owned(),
                                Some(preset.group.to_owned()),
                            )
                        }
                        Destination::SettingsSection(section) => (
                            Page::Settings.icon(),
                            section.title().to_owned(),
                            Some(Page::Settings.title().to_owned()),
                        ),
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
                        Destination::ActionLibraryEntry {
                            title, provider, ..
                        } => (Page::Actions.icon(), title.clone(), Some(provider.clone())),
                    };
                    let destination = destination.clone();
                    let selected = index == self.selected;
                    Some(
                        self.suggestion_row(index, PAGE_ROW_HEIGHT)
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
                        "↑ ↓ 选择  ·  输入 ? 查看用法"
                    } else {
                        "↑ ↓ 选择"
                    };
                    list.child(Self::selection_hint(hint))
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
        Results::for_query(
            query,
            triggers,
            &[],
            &[],
            &[],
            &[],
            &RecentDestinations::default(),
        )
    }

    #[test]
    fn quick_search_help_shows_only_supported_examples() {
        for query in ["?", " ？ "] {
            assert!(matches!(for_query(query, &[]), Results::Help(items) if items.len() == 8));
        }
        for (query, expected) in [
            ("？ 速率", &["8Mbps to MB/s"][..]),
            ("? UNIX", &["时间戳: 1700000000123ms", "unix: now"][..]),
            ("? 时长", &["时长: 5400秒", "时长: 01:30:00 - 25分钟"][..]),
            ("? 无对应示例", &[][..]),
        ] {
            let Results::Help(items) = for_query(query, &[]) else {
                panic!("未打开用法示例：{query}");
            };
            let displayed: Vec<_> = items
                .into_iter()
                .map(|index| EXAMPLES[index].query)
                .collect();
            assert_eq!(displayed, expected, "{query}");
        }
        let index = SearchIndex {
            triggers: crate::settings::defaults()
                .iter()
                .map(SearchTrigger::from_binding)
                .collect(),
            wheels: crate::wheels::Wheels::default()
                .wheels
                .iter()
                .map(SearchWheel::from_definition)
                .collect(),
            wheel_slots: None,
            actions: Vec::new(),
            settings: SettingsSection::ALL.to_vec(),
            recent: RecentDestinations::default(),
        };
        for example in EXAMPLES {
            match index.results(example.query) {
                Results::Value(Ok(_)) => {}
                Results::Destinations(items) if !items.is_empty() => {}
                _ => panic!("用法示例没有可用结果：{}", example.query),
            }
        }
    }

    #[test]
    fn quick_search_routes_wheel_names_and_scope_to_wheel_entries() {
        let settings = crate::wheels::Wheels::default();
        let wheels = [SearchWheel::from_definition(&settings.wheels[0])];
        for query in ["编辑轨道", "wheel: orbit-edit", "轮盘：", "轨道轮盘：编辑"] {
            let result = Results::for_query(
                query,
                &[],
                &wheels,
                &[],
                &[],
                &[],
                &RecentDestinations::default(),
            );
            assert!(
                matches!(result, Results::Destinations(items) if items == [Destination::Wheel("orbit-edit".into())]),
                "{query}"
            );
        }
    }

    #[test]
    fn quick_search_routes_preset_names_and_scope_to_the_catalog() {
        for query in ["顶部边缘", "预设：顶部 边缘", "preset: edge-top"] {
            assert!(
                matches!(for_query(query, &[]), Results::Destinations(items) if items == [Destination::Preset("edge-top")]),
                "{query}"
            );
        }
        assert!(matches!(
            for_query("触发预设：", &[]),
            Results::Destinations(items) if items == [Destination::Page(Page::Presets)]
        ));
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
        assert!(matches!(
            for_query("时间戳：-1ms", &[]),
            Results::Value(Ok(result)) if result == "1969-12-31T23:59:59.999Z"
        ));
        assert!(matches!(
            for_query("时长：01:30:00", &[]),
            Results::Value(Ok(result)) if result == "5400 秒"
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
