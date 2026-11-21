mod actions;
mod applications;
mod aspect_ratio;
mod calculator;
mod calendar;
mod cidr;
mod color;
mod conversion;
mod duration;
mod examples;
mod fraction;
mod json_tools;
mod operators;
mod percentage;
mod presets;
mod radix;
mod recent;
mod scientific;
mod scope;
mod settings_sections;
mod text_tools;
mod timestamp;
mod triggers;
mod url_encoding;
mod value_history;
mod value_preview;
mod wheel_slots;
mod wheels;

use gpui::{
    div, prelude::*, px, size, App, Bounds, ClipboardItem, Context, DisplayId, Div, Entity,
    EntityInputHandler, Focusable, FontWeight, KeyDownEvent, Stateful, Subscription,
    TitlebarOptions, WeakEntity, Window, WindowBounds, WindowHandle, WindowKind, WindowOptions,
};

use crate::appearance_ui::SettingsSection;
use crate::{
    appearance,
    assets::Icon,
    message_input::{input_preview, MessageInput},
    theme::{self, button, icon, rgb, Tone},
    wheel_scenes::WheelSlotKey,
    Page, Workspace,
};
use actions::{matching_actions, SearchAction};
use applications::{ApplicationCache, ApplicationIndex};
use aspect_ratio::{calculate_aspect_ratio, AspectRatioError};
use calculator::{calculate, looks_like_calculation, CalculationError};
use calendar::{calculate_date, looks_like_date_query, DateError};
use cidr::{calculate_cidr, CidrError};
use color::{calculate_contrast, convert_color, ColorError};
use conversion::{
    conversion_target_input, convert, estimate_transfer, suggested_targets, ConversionError,
};
use duration::{convert_duration, DurationError};
use examples::{matching_examples, tool_operation_examples, EXAMPLES};
use fraction::{simplify_fraction, FractionError};
use json_tools::{transform_json, JsonError};
use omnitrigger_windows::{ShellApp, ShortcutTarget};
use percentage::{compare_percentages, PercentageError};
use presets::matching_presets;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use recent::RecentDestinations;
use scope::{scoped_query, tool_example_keyword, Scope};
use settings_sections::matching_settings;
use text_tools::{count_text, transform_text, TextError};
use timestamp::{convert_timestamp, TimestampError};
use triggers::{matching_triggers, SearchTrigger};
use unicode_segmentation::UnicodeSegmentation;
use url_encoding::{convert_url_component, UrlEncodingError};
use value_history::RecentValues;
use wheel_slots::{index_slots, matching_slots, SearchWheelSlot};
use wheels::{matching_wheels, SearchWheel};

const WIDTH: f32 = 500.;
const INPUT_CHARACTER_LIMIT: usize = 4096;
const PAGE_ROW_HEIGHT: f32 = 38.;
const DESTINATION_PAGE_SIZE: usize = 4;
const CONVERSION_PAGE_SIZE: usize = 4;
const EXAMPLE_ROW_HEIGHT: f32 = 30.;
const EXAMPLE_PAGE_SIZE: usize = 8;
const MAX_VALUE_LINES: usize = 6;
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
    Value(Result<ResultValue, ValueError>),
    Destinations(Vec<Destination>),
    ApplicationsLoading,
    ApplicationsUnavailable(String),
    Help(Vec<usize>),
    Calculations(Vec<InputSuggestion>),
    Conversions(Vec<InputSuggestion>),
    ConversionTargets(Vec<InputSuggestion>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct InputSuggestion {
    expression: String,
    result: String,
}

#[derive(PartialEq, Eq)]
struct ResultValue {
    text: String,
    kind: ValueKind,
}

impl ResultValue {
    fn continuation_hint(&self) -> Option<&'static str> {
        match &self.kind {
            ValueKind::Calculation | ValueKind::ExactCalculation { can_continue: true } => {
                Some("Tab 继续计算")
            }
            ValueKind::Conversion(Some(_)) => Some("Tab 换目标"),
            _ => None,
        }
    }
}

#[derive(PartialEq, Eq)]
enum ValueKind {
    Text,
    Calculation,
    ExactCalculation { can_continue: bool },
    Conversion(Option<String>),
    Color(u32),
    ColorContrast([u32; 2]),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Destination {
    Page(Page),
    InstalledApp(ShellApp),
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
    TriggerHistoryQuery(String),
    ActionLibraryQuery(String),
    ActionLibraryEntry {
        title: String,
        provider: String,
        filter: String,
    },
}

#[derive(Default)]
pub(super) struct SearchIndex {
    triggers: Vec<SearchTrigger>,
    wheels: Vec<SearchWheel>,
    wheel_slots: Option<Vec<SearchWheelSlot>>,
    actions: Vec<SearchAction>,
    settings: Vec<SettingsSection>,
    recent: RecentSearch,
    applications: ApplicationIndex,
}

#[derive(Clone, Default)]
pub(super) struct RecentSearch {
    pub(super) destinations: RecentDestinations,
    calculations: RecentValues,
    conversions: RecentValues,
    applications: ApplicationCache,
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
            applications: workspace.quick_search_recent.applications.snapshot(),
        }
    }

    fn results(&self, query: &str) -> Results {
        Results::for_query(query, self)
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
    value_preview::lines(value)
        .take(MAX_VALUE_LINES)
        .count()
        .saturating_sub(2) as f32
        * VALUE_LINE_HEIGHT
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ValueError {
    InputTooLong,
    Calculation(CalculationError),
    Conversion(ConversionError),
    AspectRatio(AspectRatioError),
    Cidr(CidrError),
    Date(DateError),
    Duration(DurationError),
    Timestamp(TimestampError),
    Color(ColorError),
    Text(TextError),
    Json(JsonError),
    Fraction(FractionError),
    Percentage(PercentageError),
    UrlEncoding(UrlEncodingError),
}

impl ValueError {
    fn message(self) -> &'static str {
        match self {
            Self::InputTooLong => "输入最多支持 4096 个字符",
            Self::Calculation(error) => error.message(),
            Self::Conversion(error) => error.message(),
            Self::AspectRatio(error) => error.message(),
            Self::Cidr(error) => error.message(),
            Self::Date(error) => error.message(),
            Self::Duration(error) => error.message(),
            Self::Timestamp(error) => error.message(),
            Self::Color(error) => error.message(),
            Self::Text(error) => error.message(),
            Self::Json(error) => error.message(),
            Self::Fraction(error) => error.message(),
            Self::Percentage(error) => error.message(),
            Self::UrlEncoding(error) => error.message(),
        }
    }

    fn is_incomplete(self) -> bool {
        matches!(
            self,
            Self::Calculation(CalculationError::Incomplete)
                | Self::Conversion(ConversionError::Incomplete)
                | Self::AspectRatio(AspectRatioError::Incomplete)
                | Self::Cidr(CidrError::Incomplete)
                | Self::Date(DateError::Incomplete)
                | Self::Duration(DurationError::Incomplete)
                | Self::Timestamp(TimestampError::Incomplete)
                | Self::Color(ColorError::Incomplete | ColorError::IncompleteContrast)
                | Self::Text(
                    TextError::MissingText
                        | TextError::UnclosedQuote
                        | TextError::IncompleteTransform
                        | TextError::IncompleteReplacement
                        | TextError::IncompleteTextArgument
                )
                | Self::Fraction(FractionError::Incomplete)
                | Self::Percentage(PercentageError::Incomplete)
                | Self::UrlEncoding(UrlEncodingError::Incomplete)
                | Self::Json(JsonError::Incomplete)
        )
    }
}

impl Results {
    fn for_query(query: &str, index: &SearchIndex) -> Self {
        let SearchIndex {
            triggers,
            wheels,
            wheel_slots,
            actions,
            settings,
            recent,
            applications,
        } = index;
        if query.len() > INPUT_CHARACTER_LIMIT
            && query.chars().take(INPUT_CHARACTER_LIMIT + 1).count() > INPUT_CHARACTER_LIMIT
        {
            return Self::Value(Err(ValueError::InputTooLong));
        }
        let query = query.trim();
        if query.is_empty() {
            return Self::Destinations(recent.destinations.suggestions(triggers));
        }
        if let Some(terms) = query.strip_prefix(['?', '？']) {
            return Self::Help(matching_examples(terms));
        }
        if let Some((scope, term)) = scoped_query(query) {
            if let Some(examples) = tool_operation_examples(scope, term) {
                return Self::Help(examples);
            }
            return match scope {
                Scope::Triggers(filter) => Self::Destinations(
                    matching_triggers(triggers, term, filter)
                        .into_iter()
                        .map(Destination::Trigger)
                        .collect(),
                ),
                Scope::Wheels => Self::Destinations(
                    matching_wheels(wheels, term)
                        .into_iter()
                        .map(|wheel| Destination::Wheel(wheel.id.clone()))
                        .collect(),
                ),
                Scope::WheelSlots if term.is_empty() => {
                    Self::Destinations(vec![Destination::Page(Page::Wheels)])
                }
                Scope::WheelSlots => Self::Destinations(matching_slots(
                    wheel_slots.as_deref().unwrap_or_default(),
                    term,
                )),
                Scope::Presets if term.is_empty() => {
                    Self::Destinations(vec![Destination::Page(Page::Presets)])
                }
                Scope::Presets => Self::Destinations(
                    matching_presets(term)
                        .into_iter()
                        .map(|preset| Destination::Preset(preset.key))
                        .collect(),
                ),
                Scope::Settings if term.is_empty() => {
                    Self::Destinations(vec![Destination::Page(Page::Settings)])
                }
                Scope::Settings => Self::Destinations(
                    matching_settings(settings, term)
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
                Scope::TriggerHistory => {
                    Self::Destinations(vec![Destination::TriggerHistoryQuery(term.to_owned())])
                }
                Scope::CalculationHistory => Self::Calculations(recent.calculations.matching(term)),
                Scope::ConversionHistory => Self::Conversions(recent.conversions.matching(term)),
                Scope::Applications => applications.results(term),
                Scope::Actions => {
                    let mut entries = matching_actions(actions, term);
                    entries.insert(
                        entries.len().min(DESTINATION_PAGE_SIZE - 1),
                        Destination::ActionLibraryQuery(term.to_owned()),
                    );
                    Self::Destinations(entries)
                }
                Scope::Date => Self::text_value(
                    calculate_date(term, chrono::Local::now().date_naive())
                        .map_err(ValueError::Date),
                ),
                Scope::Duration => {
                    Self::text_value(convert_duration(term).map_err(ValueError::Duration))
                }
                Scope::TransferTime => Self::conversion_value(estimate_transfer(term), None),
                Scope::AspectRatio => {
                    Self::text_value(calculate_aspect_ratio(term).map_err(ValueError::AspectRatio))
                }
                Scope::Cidr => Self::text_value(calculate_cidr(term).map_err(ValueError::Cidr)),
                Scope::Timestamp => Self::text_value(
                    convert_timestamp(term, chrono::Utc::now()).map_err(ValueError::Timestamp),
                ),
                Scope::Color => Self::Value(
                    convert_color(term)
                        .map(|color| ResultValue {
                            text: color.output,
                            kind: ValueKind::Color(color.swatch),
                        })
                        .map_err(ValueError::Color),
                ),
                Scope::ColorContrast => Self::Value(
                    calculate_contrast(term)
                        .map(|contrast| ResultValue {
                            text: contrast.output,
                            kind: ValueKind::ColorContrast(contrast.swatches),
                        })
                        .map_err(ValueError::Color),
                ),
                Scope::TextCount => Self::text_value(count_text(term).map_err(ValueError::Text)),
                Scope::TextTools => {
                    Self::text_value(transform_text(term).map_err(ValueError::Text))
                }
                Scope::Json => Self::text_value(transform_json(term).map_err(ValueError::Json)),
                Scope::Fraction => Self::Value(
                    simplify_fraction(term)
                        .map(|text| ResultValue {
                            kind: ValueKind::ExactCalculation {
                                // 为括号、运算符和至少一位操作数预留空间。
                                can_continue: text.len() <= calculator::QUERY_LIMIT - 4,
                            },
                            text,
                        })
                        .map_err(ValueError::Fraction),
                ),
                Scope::Percentage => {
                    Self::text_value(compare_percentages(term).map_err(ValueError::Percentage))
                }
                Scope::UrlEncoding => {
                    Self::text_value(convert_url_component(term).map_err(ValueError::UrlEncoding))
                }
            };
        }
        if let Some(result) = convert(query) {
            return match result {
                Err(error @ (ConversionError::Incomplete | ConversionError::UnknownUnit)) => {
                    suggested_targets(query).map_or_else(
                        || Self::Value(Err(ValueError::Conversion(error))),
                        Self::ConversionTargets,
                    )
                }
                result => Self::conversion_value(result, Some(query)),
            };
        }
        if let Some(targets) = suggested_targets(query) {
            return Self::ConversionTargets(targets);
        }
        let is_date = looks_like_date_query(query);
        if !is_date && looks_like_calculation(query) {
            return Self::Value(
                calculate(query)
                    .map(|text| ResultValue {
                        text,
                        kind: ValueKind::Calculation,
                    })
                    .map_err(ValueError::Calculation),
            );
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
            .map(|(_, page)| Destination::Page(page))
            .collect();
        destinations.extend(
            matching_triggers(triggers, &query, None)
                .into_iter()
                .map(Destination::Trigger),
        );
        destinations.extend(
            matching_wheels(wheels, &query)
                .into_iter()
                .map(|wheel| Destination::Wheel(wheel.id.clone())),
        );
        destinations.extend(
            matching_presets(&query)
                .into_iter()
                .map(|preset| Destination::Preset(preset.key)),
        );
        destinations.extend(matching_actions(actions, &query));
        destinations.extend(
            matching_settings(settings, &query)
                .into_iter()
                .map(Destination::SettingsSection),
        );
        if destinations.is_empty() {
            if is_date {
                return Self::text_value(
                    calculate_date(&query, chrono::Local::now().date_naive())
                        .map_err(ValueError::Date),
                );
            }
            return tool_example_keyword(&query).map_or_else(
                || Self::Destinations(destinations),
                |keyword| Self::Help(matching_examples(keyword)),
            );
        }
        Self::Destinations(destinations)
    }

    fn text_value(result: Result<String, ValueError>) -> Self {
        Self::Value(result.map(|text| ResultValue {
            text,
            kind: ValueKind::Text,
        }))
    }

    fn conversion_value(result: Result<String, ConversionError>, query: Option<&str>) -> Self {
        Self::Value(
            result
                .map(|text| ResultValue {
                    text,
                    kind: ValueKind::Conversion(query.and_then(conversion_target_input)),
                })
                .map_err(ValueError::Conversion),
        )
    }

    #[cfg(test)]
    fn arithmetic_result(&self) -> Option<&str> {
        let Self::Value(Ok(ResultValue {
            text,
            kind: ValueKind::Calculation,
        })) = self
        else {
            return None;
        };
        Some(text)
    }

    fn suggested_input(&self, selected: usize) -> Option<&str> {
        match self {
            Self::Help(items) => Some(EXAMPLES[*items.get(selected)?].query),
            Self::Calculations(items)
            | Self::Conversions(items)
            | Self::ConversionTargets(items) => Some(&items.get(selected)?.expression),
            _ => None,
        }
    }

    fn continuation_query(&self) -> Option<String> {
        let Self::Value(Ok(ResultValue { kind, text })) = self else {
            return None;
        };
        match kind {
            ValueKind::Calculation if text.starts_with('-') => Some(format!("({text})")),
            ValueKind::Calculation => Some(text.clone()),
            ValueKind::ExactCalculation { can_continue: true } => {
                Some(format!("分数: 计算 ({text})"))
            }
            ValueKind::Conversion(Some(query)) => Some(query.clone()),
            _ => None,
        }
    }

    fn height(&self) -> f32 {
        match self {
            Self::Destinations(items) if items.len() > 1 => {
                132. + items.len().min(DESTINATION_PAGE_SIZE) as f32 * PAGE_ROW_HEIGHT
            }
            Self::Value(Ok(value)) => {
                let hint_height = if value.continuation_hint().is_some() {
                    24.
                } else {
                    0.
                };
                158. + value_extra_height(&value.text) + hint_height
            }
            Self::Help(items) if !items.is_empty() => {
                132. + items.len().min(EXAMPLE_PAGE_SIZE) as f32 * EXAMPLE_ROW_HEIGHT
            }
            Self::ConversionTargets(items) if !items.is_empty() => {
                132. + items.len().min(CONVERSION_PAGE_SIZE) as f32 * EXAMPLE_ROW_HEIGHT
            }
            Self::Calculations(items) | Self::Conversions(items) if !items.is_empty() => {
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
    requested_applications: bool,
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
                .with_preserved_whitespace()
                .with_character_limit(INPUT_CHARACTER_LIMIT + 1);
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
            this.refresh_results(&query, window, cx);
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
            requested_applications: false,
            _input_changes: input_changes,
            _activation: activation,
        }
    }

    pub(super) fn activate(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.activate_window();
        window.focus(&self.input.focus_handle(cx));
    }

    fn refresh_results(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        let next = self.index.results(query);
        if self.results != next {
            let height_changed = self.results.height() != next.height();
            self.results = next;
            self.selected = 0;
            if height_changed {
                window.resize(size(px(WIDTH), px(self.results.height())));
            }
        }
        if !self.requested_applications
            && !matches!(self.results, Results::Value(Err(ValueError::InputTooLong)))
            && matches!(scoped_query(query.trim()), Some((Scope::Applications, _)))
            && self
                .index
                .applications
                .needs_refresh(std::time::Instant::now())
        {
            self.requested_applications = true;
            let cached = self.index.recent.applications.clone();
            cx.spawn_in(window, async move |this, cx| {
                let applications = cx
                    .background_executor()
                    .spawn(async { ApplicationIndex::load() })
                    .await;
                cached.remember(&applications);
                let _ = cx.update(|window, cx| {
                    let _ = this.update(cx, |this, cx| {
                        if matches!(applications, ApplicationIndex::Unavailable(_))
                            && matches!(this.index.applications, ApplicationIndex::Ready { .. })
                        {
                            return;
                        }
                        let previous = match &this.results {
                            Results::Destinations(items) => items.get(this.selected).cloned(),
                            _ => None,
                        };
                        this.index.applications = applications;
                        let query = this.input.read(cx).text();
                        this.refresh_results(&query, window, cx);
                        let Some(Destination::InstalledApp(previous)) = previous else {
                            return;
                        };
                        let Results::Destinations(items) = &this.results else {
                            return;
                        };
                        if let Some(index) = items.iter().position(|item| {
                            matches!(item, Destination::InstalledApp(app) if app == &previous
                                || (!app.identifier().is_empty()
                                    && app.title() == previous.title()
                                    && app.identifier() == previous.identifier()))
                        }) {
                            this.selected = index;
                        }
                    });
                });
            })
            .detach();
        }
        cx.notify();
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
                self.remember_value(cx);
                cx.write_to_clipboard(ClipboardItem::new_string(result.text.clone()));
                window.remove_window();
            }
            Results::Destinations(items) => {
                if let Some(destination) = items.get(self.selected).cloned() {
                    self.open_destination(destination, window, cx);
                }
            }
            _ => {
                if let Some(query) = self.results.suggested_input(self.selected) {
                    self.fill_query(query.to_owned(), window, cx);
                }
            }
        }
    }

    fn remember_value(&self, cx: &Context<Self>) {
        let Results::Value(Ok(result)) = &self.results else {
            return;
        };
        let history = match &result.kind {
            ValueKind::Calculation | ValueKind::ExactCalculation { .. } => {
                &self.index.recent.calculations
            }
            ValueKind::Conversion(_) => &self.index.recent.conversions,
            _ => return,
        };
        history.record(&self.input.read(cx).text(), &result.text);
    }

    fn continue_value(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(query) = self.results.continuation_query() {
            self.remember_value(cx);
            self.fill_query(query, window, cx);
        }
    }

    fn fill_query(
        &self,
        query: impl Into<gpui::SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.input.update(cx, |input, cx| {
            input.set_text(query, cx);
            input.move_cursor_to_end(cx);
        });
        window.focus(&self.input.focus_handle(cx));
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.input.read(cx).is_composing() {
            return;
        }
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers.number_of_modifiers();
        if modifiers != 0 {
            if modifiers == 1 && keystroke.modifiers.shift && keystroke.key == "enter" {
                if !event.is_held {
                    self.input.update(cx, |input, cx| {
                        input.replace_text_in_range(None, "\n", window, cx);
                    });
                }
                cx.stop_propagation();
            } else if modifiers == 1 && keystroke.modifiers.control && keystroke.key == "r" {
                if !event.is_held {
                    let query = self.input.read(cx).text();
                    const PREFIX: &str = "算式历史: ";
                    let filter = query.strip_prefix(PREFIX).or_else(|| {
                        let (scope, term) = scoped_query(&query)?;
                        (scope == Scope::CalculationHistory).then_some(term)
                    });
                    let next = filter.map_or_else(|| format!("{PREFIX}{query}"), str::to_owned);
                    self.fill_query(next, window, cx);
                }
                cx.stop_propagation();
            }
            return;
        }
        match keystroke.key.as_str() {
            "enter" => {
                if !event.is_held {
                    self.confirm(window, cx);
                }
            }
            "escape" => window.remove_window(),
            key @ ("pageup" | "pagedown") => {
                if !self.change_page(key == "pagedown", cx) {
                    return;
                }
            }
            "tab" => {
                let suggestion = self.results.suggested_input(self.selected);
                let continues_value = matches!(
                    &self.results,
                    Results::Value(Ok(value)) if value.continuation_hint().is_some()
                );
                if suggestion.is_none() && !continues_value {
                    return;
                }
                if !event.is_held {
                    if let Some(query) = suggestion {
                        self.fill_query(query.to_owned(), window, cx);
                    } else {
                        self.continue_value(window, cx);
                    }
                }
            }
            key @ ("up" | "down") => {
                let count = match &self.results {
                    Results::Destinations(items) => items.len(),
                    Results::Help(items) => items.len(),
                    Results::Calculations(items)
                    | Results::Conversions(items)
                    | Results::ConversionTargets(items) => items.len(),
                    Results::Value(_)
                    | Results::ApplicationsLoading
                    | Results::ApplicationsUnavailable(_) => 0,
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

    fn change_page(&mut self, forward: bool, cx: &mut Context<Self>) -> bool {
        let (count, page_size) = match &self.results {
            Results::Destinations(items) => (items.len(), DESTINATION_PAGE_SIZE),
            Results::ConversionTargets(items) => (items.len(), CONVERSION_PAGE_SIZE),
            Results::Help(items) => (items.len(), EXAMPLE_PAGE_SIZE),
            _ => return false,
        };
        if count <= page_size {
            return false;
        }
        let page = self.selected / page_size;
        let last_page = (count - 1) / page_size;
        let next = if forward {
            (page + 1).min(last_page)
        } else {
            page.saturating_sub(1)
        };
        if next != page {
            self.selected = next * page_size;
            cx.notify();
        }
        true
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

    fn render_pagination(
        &self,
        count: usize,
        page_size: usize,
        hint: &'static str,
        cx: &mut Context<Self>,
    ) -> Div {
        let page = self.selected / page_size;
        let page_count = count.div_ceil(page_size);
        Self::selection_hint(hint)
            .flex()
            .items_center()
            .gap(px(6.))
            .child(div().flex_1())
            .child(
                button("result-prev", "上一页", Tone::Ghost, page > 0)
                    .h(px(20.))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.change_page(false, cx);
                    })),
            )
            .child(format!("{} / {page_count}", page + 1))
            .child(
                button("result-next", "下一页", Tone::Ghost, page + 1 < page_count)
                    .h(px(20.))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.change_page(true, cx);
                    })),
            )
    }

    fn render_examples(&self, items: &[usize], cx: &mut Context<Self>) -> Div {
        let start = self.selected / EXAMPLE_PAGE_SIZE * EXAMPLE_PAGE_SIZE;
        div()
            .flex()
            .flex_col()
            .children(
                items
                    .iter()
                    .enumerate()
                    .skip(start)
                    .take(EXAMPLE_PAGE_SIZE)
                    .map(|(index, &example)| {
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
                                    .child(input_preview(query.trim_end())),
                            )
                            .when(index == self.selected, |row| {
                                row.child(
                                    div().text_color(rgb(theme::TEXT_MUTED)).child("Tab 填入"),
                                )
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.fill_query(query, window, cx)
                            }))
                    }),
            )
            .when(items.len() > EXAMPLE_PAGE_SIZE, |list| {
                list.child(self.render_pagination(
                    items.len(),
                    EXAMPLE_PAGE_SIZE,
                    "↑ ↓ 选择  ·  ? 后筛选  ·  PgUp / PgDn",
                    cx,
                ))
            })
            .when((2..=EXAMPLE_PAGE_SIZE).contains(&items.len()), |list| {
                let hint = if matches!(self.input.read(cx).text().trim(), "?" | "？") {
                    "↑ ↓ 选择  ·  ? 后可加关键词"
                } else {
                    "↑ ↓ 选择"
                };
                list.child(Self::selection_hint(hint))
            })
    }

    fn render_input_suggestions(
        &self,
        items: &[InputSuggestion],
        glyph: Icon,
        page_size: usize,
        cx: &mut Context<Self>,
    ) -> Div {
        let start = self.selected / page_size * page_size;
        let visible = items.iter().enumerate().skip(start).take(page_size);
        div()
            .flex()
            .flex_col()
            .children(visible.map(|(index, item)| {
                let expression = item.expression.clone();
                self.suggestion_row(index, EXAMPLE_ROW_HEIGHT)
                    .child(icon(glyph, 16., theme::ACCENT))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(theme::MONO_FONT)
                            .child(expression.clone()),
                    )
                    .child(
                        div()
                            .max_w(px(200.))
                            .truncate()
                            .font_family(theme::MONO_FONT)
                            .text_color(rgb(theme::ACCENT))
                            .child(format!("= {}", item.result)),
                    )
                    .when(index == self.selected, |row| {
                        row.child(div().text_color(rgb(theme::TEXT_MUTED)).child("Tab 填入"))
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.fill_query(expression.clone(), window, cx);
                    }))
            }))
            .when(items.len() > page_size, |list| {
                list.child(self.render_pagination(
                    items.len(),
                    page_size,
                    "↑ ↓ 选择  ·  PgUp / PgDn",
                    cx,
                ))
            })
            .when((2..=page_size).contains(&items.len()), |list| {
                list.child(Self::selection_hint("↑ ↓ 选择"))
            })
    }

    fn render_result(&self, cx: &mut Context<Self>) -> Div {
        let row = div().h(px(38.)).flex().items_center().gap(px(12.));
        match &self.results {
            Results::Value(Ok(output)) => {
                let result = &output.text;
                let swatches: &[u32] = match &output.kind {
                    ValueKind::Color(color) => std::slice::from_ref(color),
                    ValueKind::ColorContrast(colors) => colors,
                    _ => &[],
                };
                let value = div()
                    .flex_1()
                    .min_w_0()
                    .font_family(theme::MONO_FONT)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::ACCENT));
                let large_text_columns = match swatches.len() {
                    0 => 24,
                    1 => 20,
                    _ => 17,
                };
                let compact = result.contains('\n')
                    || result
                        .graphemes(true)
                        .take(large_text_columns + 1)
                        .map(value_preview::column_width)
                        .sum::<usize>()
                        > large_text_columns;
                let value = if compact {
                    let mut preview = value_preview::lines(result);
                    let lines: Vec<_> = preview.by_ref().take(MAX_VALUE_LINES).collect();
                    let clipped = preview.next().is_some();
                    value
                        .flex()
                        .flex_col()
                        .text_size(px(14.))
                        .line_height(px(VALUE_LINE_HEIGHT))
                        .children(lines.into_iter().enumerate().map(|(index, text)| {
                            let prefix = if index == 0 { "= " } else { "  " };
                            let (text, suffix) = if clipped && index + 1 == MAX_VALUE_LINES {
                                let columns = text
                                    .graphemes(true)
                                    .map(value_preview::column_width)
                                    .sum::<usize>();
                                let end = if columns >= value_preview::COLUMNS {
                                    text.grapheme_indices(true)
                                        .next_back()
                                        .map_or(0, |(index, _)| index)
                                } else {
                                    text.len()
                                };
                                (&text[..end], "…")
                            } else {
                                (text, "")
                            };
                            div().truncate().child(format!(
                                "{prefix}{}{suffix}",
                                value_preview::display_tabs(text)
                            ))
                        }))
                } else {
                    value
                        .truncate()
                        .text_size(px(22.))
                        .child(format!("= {}", value_preview::display_tabs(result)))
                };
                let row = row
                    .h(px(38. + value_extra_height(result)))
                    .children(swatches.iter().map(|&color| {
                        div()
                            .size(px(26.))
                            .flex_none()
                            .rounded(px(5.))
                            .border_1()
                            .border_color(rgb(theme::BORDER_HI))
                            .bg(gpui::rgba(color))
                    }))
                    .child(value)
                    .child(
                        button("copy-calculation", "复制  Enter", Tone::Primary, true)
                            .flex_none()
                            .on_click(cx.listener(|this, _, window, cx| this.confirm(window, cx))),
                    );
                div().flex().flex_col().child(row).when_some(
                    output.continuation_hint(),
                    |result, hint| {
                        result.child(
                            Self::selection_hint(hint)
                                .id("continue-value")
                                .cursor_pointer()
                                .hover(|style| style.text_color(rgb(theme::ACCENT)))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.continue_value(window, cx);
                                })),
                        )
                    },
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
            Results::Calculations(items) if items.is_empty() => row
                .text_color(rgb(theme::TEXT_MUTED))
                .child("未找到用过的算式"),
            Results::Calculations(items) => {
                self.render_input_suggestions(items, Icon::Terminal, items.len(), cx)
            }
            Results::Conversions(items) if items.is_empty() => row
                .text_color(rgb(theme::TEXT_MUTED))
                .child("未找到用过的换算"),
            Results::Conversions(items) => {
                self.render_input_suggestions(items, Icon::Layers, items.len(), cx)
            }
            Results::ConversionTargets(items) => {
                self.render_input_suggestions(items, Icon::Layers, CONVERSION_PAGE_SIZE, cx)
            }
            Results::ApplicationsLoading => row
                .text_color(rgb(theme::TEXT_MUTED))
                .child("正在读取 Windows 应用"),
            Results::ApplicationsUnavailable(error) => row
                .text_color(rgb(theme::DANGER_TEXT))
                .child(div().truncate().child(error.clone())),
            Results::Destinations(items) if items.is_empty() => {
                row.text_color(rgb(theme::TEXT_MUTED)).child("未找到匹配项")
            }
            Results::Destinations(items) => div()
                .flex()
                .flex_col()
                .children(
                    items
                        .iter()
                        .enumerate()
                        .skip(self.selected / DESTINATION_PAGE_SIZE * DESTINATION_PAGE_SIZE)
                        .take(DESTINATION_PAGE_SIZE)
                        .filter_map(|(index, destination)| {
                            let (glyph, title, detail) = match destination {
                                Destination::Page(page) => {
                                    (page.icon(), page.title().to_owned(), None)
                                }
                                Destination::InstalledApp(app) => (
                                    Icon::Grid,
                                    app.title().to_owned(),
                                    self.index
                                        .applications
                                        .has_namesake(app)
                                        .then(|| app.identifier().to_owned()),
                                ),
                                Destination::Trigger(id) => {
                                    let trigger = self
                                        .index
                                        .triggers
                                        .iter()
                                        .find(|trigger| trigger.id() == *id)?;
                                    (
                                        trigger.icon,
                                        trigger.title.clone(),
                                        Some(trigger.detail.clone()),
                                    )
                                }
                                Destination::Wheel(id) => {
                                    let wheel =
                                        self.index.wheels.iter().find(|wheel| &wheel.id == id)?;
                                    let duplicate =
                                        self.index.wheels.iter().any(|other| {
                                            other.id != *id && other.name == wheel.name
                                        });
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
                                Destination::TriggerHistoryQuery(term) => {
                                    let title = if term.is_empty() {
                                        "打开触发记录"
                                    } else {
                                        "筛选触发记录"
                                    };
                                    (Page::History.icon(), title.to_owned(), None)
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
                                            Destination::ActionLibraryQuery(term)
                                            | Destination::TriggerHistoryQuery(term)
                                                if !term.is_empty() =>
                                            {
                                                "Enter 筛选"
                                            }
                                            _ => "Enter 打开",
                                        };
                                        row.child(
                                            div().text_color(rgb(theme::TEXT_MUTED)).child(hint),
                                        )
                                    })
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_destination(destination.clone(), window, cx)
                                    })),
                            )
                        }),
                )
                .when(items.len() > DESTINATION_PAGE_SIZE, |list| {
                    list.child(self.render_pagination(
                        items.len(),
                        DESTINATION_PAGE_SIZE,
                        "↑ ↓ 选择  ·  PgUp / PgDn",
                        cx,
                    ))
                })
                .when((2..=DESTINATION_PAGE_SIZE).contains(&items.len()), |list| {
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
        SearchIndex {
            triggers: triggers.to_vec(),
            ..SearchIndex::default()
        }
        .results(query)
    }

    #[test]
    fn url_decoding_previews_each_line_and_preserves_the_copy_value() {
        let query = "URL: 解码 %E5%8C%97%E4%BA%AC%0D%0A%E4%B8%8A%E6%B5%B7%0A%E5%B9%BF%E5%B7%9E%0A%E6%B7%B1%E5%9C%B3";
        let results = for_query(query, &[]);
        assert_eq!(results.height(), 158. + 2. * VALUE_LINE_HEIGHT);
        assert!(matches!(
            results,
            Results::Value(Ok(output)) if output.text == "北京\r\n上海\n广州\n深圳"
        ));
    }

    #[test]
    fn permits_text_snippets_beyond_the_calculation_limit() {
        let value = "北京".repeat(512);
        let json = format!(r#"{{"name":"{value}"}}"#);
        let query = format!("JSON: 压缩 {json}");
        assert!(matches!(
            for_query(&query, &[]),
            Results::Value(Ok(result)) if result.text == json
        ));
        assert!(matches!(
            for_query(
                &format!("JSON: 压缩 {}", "1".repeat(INPUT_CHARACTER_LIMIT)),
                &[]
            ),
            Results::Value(Err(ValueError::InputTooLong))
        ));
        assert!(matches!(
            for_query(&"1".repeat(calculator::QUERY_LIMIT + 1), &[]),
            Results::Value(Err(ValueError::Calculation(CalculationError::TooLong)))
        ));
    }

    #[test]
    fn quick_search_finds_session_conversions_by_expression_and_result() {
        let index = SearchIndex::default();
        let recent = &index.recent;
        let Results::Value(Ok(value)) = for_query("8 Mbps to MB/s", &[]) else {
            panic!("换算没有生成结果");
        };
        assert!(matches!(value.kind, ValueKind::Conversion(_)));
        recent.conversions.record("8 Mbps to MB/s", &value.text);
        recent.conversions.record("1 GiB to MiB", "1024 MiB");
        recent.calculations.record("2**10", "1024");
        for (query, expected) in [
            ("换算历史: MIB 1024", "1 GiB to MiB"),
            ("conversion history: MBPS", "8 Mbps to MB/s"),
        ] {
            assert!(matches!(
                index.results(query),
                Results::Conversions(items) if items.len() == 1 && items[0].expression == expected
            ));
        }
        assert!(matches!(
            index.results("算式历史:"),
            Results::Calculations(items) if items.len() == 1 && items[0].expression == "2**10"
        ));
    }

    #[test]
    fn conversion_suggestions_support_symbol_prefixes_and_later_pages() {
        let results = for_query("10 km to", &[]);
        assert_eq!(
            results.height(),
            132. + CONVERSION_PAGE_SIZE as f32 * EXAMPLE_ROW_HEIGHT
        );
        assert!(matches!(
            results,
            Results::ConversionTargets(items) if items.len() > CONVERSION_PAGE_SIZE
        ));
        assert!(matches!(
            for_query("1 GiB to Ti", &[]),
            Results::ConversionTargets(items) if items.len() == 2
        ));
        assert!(matches!(
            for_query("0xff to b", &[]),
            Results::ConversionTargets(items) if items.len() == 1 && items[0].result == "0b11111111"
        ));
        assert_eq!(for_query("0xff", &[]).arithmetic_result(), Some("255"));
        assert!(matches!(
            for_query("10 km to nonexistent", &[]),
            Results::Value(Err(ValueError::Conversion(ConversionError::UnknownUnit)))
        ));
    }

    #[test]
    fn suggests_unit_targets_as_soon_as_the_quantity_is_complete() {
        let Results::ConversionTargets(items) = for_query("500 nm", &[]) else {
            panic!("没有显示可选换算单位");
        };
        assert_eq!(items[0].expression, "500 nm to μm");
        assert_eq!(items[0].result, "0.5 μm");
        assert_eq!(for_query("2 + 3", &[]).arithmetic_result(), Some("5"));
    }

    #[test]
    fn tool_names_offer_examples_when_there_are_no_page_matches() {
        for (query, prefix) in [
            ("字数", "字数:"),
            ("FrAcTiOn", "分数:"),
            ("date", "日期:"),
            ("COLOUR", "颜色:"),
            ("contrast", "对比度:"),
            ("URL", "URL:"),
            ("文本工具", "文本:"),
        ] {
            let Results::Help(items) = for_query(query, &[]) else {
                panic!("未显示对应工具的示例：{query}");
            };
            assert!(!items.is_empty(), "{query}");
            assert!(items
                .iter()
                .all(|&index| EXAMPLES[index].query.starts_with(prefix)));
        }
        let mut bindings = crate::settings::defaults();
        bindings[0].message = "分数".into();
        let triggers = [SearchTrigger::from_binding(&bindings[0])];
        assert!(matches!(
            for_query("计算器", &[]),
            Results::Help(items) if items.iter().any(|&index| EXAMPLES[index].query == "2**10")
        ));
        assert!(matches!(
            for_query("units", &[]),
            Results::Help(items) if items.iter().any(|&index| EXAMPLES[index].query == "10 km to mi")
        ));
        assert!(matches!(
            for_query("分数", &triggers),
            Results::Destinations(items) if items == [Destination::Trigger(bindings[0].spec.id)]
        ));
    }

    #[test]
    fn tools_offer_operations_until_the_user_enters_their_payload() {
        let examples_for = |query: &str| {
            let Results::Help(items) = for_query(query, &[]) else {
                panic!("未显示工具操作：{query}");
            };
            items
                .into_iter()
                .map(|index| EXAMPLES[index].query)
                .collect::<Vec<_>>()
        };
        assert_eq!(examples_for("JSON：").len(), 8);
        assert!(examples_for("JSON: 取")[0].starts_with("JSON: 取值 "));
        assert!(examples_for("JSON: 取列")[0].starts_with("JSON: 取列 "));
        assert!(examples_for("JSON: 键")[0].starts_with("JSON: 键名 "));
        assert!(examples_for("JSON: 长")[0].starts_with("JSON: 长度 "));
        assert!(examples_for("JSON: 展")[0].starts_with("JSON: 展开数组 "));
        assert!(examples_for("JSON: 合")[0].starts_with("JSON: 合并行 "));
        assert!(examples_for("URL: 去")[0].starts_with("URL: 去查询串 "));
        assert!(examples_for("URL: 去参")[0].starts_with("URL: 去参数 "));
        assert_eq!(
            examples_for("网址: 表单"),
            [
                "URL: 表单编码 北京 A+B",
                "URL: 表单解码 %E5%8C%97%E4%BA%AC+A%2BB",
            ]
        );
        assert_eq!(examples_for("文本: sha").len(), 2);
        assert!(matches!(
            for_query("JSON: 格式化 {", &[]),
            Results::Value(Err(ValueError::Json(JsonError::Incomplete)))
        ));
        assert!(matches!(
            for_query("文本: 大写 abc", &[]),
            Results::Value(Ok(value)) if value.text == "ABC"
        ));
    }

    #[test]
    #[cfg(target_os = "windows")]
    #[ignore = "需要 Windows 桌面会话"]
    fn search_shortcuts_insert_one_line_break_and_toggle_history() {
        let (send, receive) = std::sync::mpsc::channel();
        gpui::Application::new()
            .with_assets(crate::Assets)
            .run(move |cx| {
                let index = SearchIndex::default();
                let results = index.results("");
                let window = cx
                    .open_window(
                        WindowOptions {
                            show: false,
                            focus: false,
                            ..Default::default()
                        },
                        |window, cx| {
                            cx.new(|cx| {
                                QuickSearch::new(
                                    appearance::Mode::Dark,
                                    WeakEntity::new_invalid(),
                                    index,
                                    results,
                                    window,
                                    cx,
                                )
                            })
                        },
                    )
                    .unwrap();
                cx.spawn(async move |cx| {
                    let states = window.update(cx, |search, window, cx| {
                        search.input.update(cx, |input, cx| {
                            input.set_text("换行验证甲", cx);
                            input.move_cursor_to_end(cx);
                        });
                        [
                            ("shift-enter", true),
                            ("ctrl-shift-enter", false),
                            ("shift-enter", false),
                            ("shift-enter", true),
                            ("ctrl-r", true),
                            ("ctrl-shift-r", false),
                            ("ctrl-r", false),
                            ("ctrl-r", true),
                            ("ctrl-r", false),
                        ]
                        .map(|(key, is_held)| {
                            let event = KeyDownEvent {
                                keystroke: gpui::Keystroke::parse(key).unwrap(),
                                is_held,
                            };
                            search.key_down(&event, window, cx);
                            search.input.read(cx).text()
                        })
                    });
                    send.send(states).unwrap();
                    cx.update(|cx| cx.quit()).unwrap();
                })
                .detach();
            });
        assert_eq!(
            receive.recv().unwrap().unwrap(),
            [
                "换行验证甲",
                "换行验证甲",
                "换行验证甲\n",
                "换行验证甲\n",
                "换行验证甲\n",
                "换行验证甲\n",
                "算式历史: 换行验证甲\n",
                "算式历史: 换行验证甲\n",
                "换行验证甲\n",
            ]
        );
    }

    #[test]
    fn shared_input_suggestions_include_templates_and_compatible_targets() {
        let targets = for_query("0xff to", &[]);
        assert_eq!(targets.suggested_input(1), Some("0xff to bin"));
        assert_eq!(targets.suggested_input(4), None);
        assert_eq!(
            for_query("文本: 按行去", &[]).suggested_input(0),
            Some("文本: 按行去重 苹果\n梨\n苹果")
        );
        assert_eq!(for_query("2+3", &[]).suggested_input(0), None);
        assert_eq!(for_query("触发器", &[]).suggested_input(0), None);
    }

    #[test]
    fn quick_search_help_shows_only_supported_examples() {
        for query in ["?", " ？ "] {
            let results = for_query(query, &[]);
            assert_eq!(
                results.height(),
                132. + EXAMPLE_PAGE_SIZE as f32 * EXAMPLE_ROW_HEIGHT
            );
            assert!(matches!(results, Results::Help(items) if items.len() == EXAMPLES.len()));
        }
        for (query, expected) in [
            ("？ 速率", &["下载: 600 MB 用时 2分钟", "8Mbps to MB/s"][..]),
            ("? UNIX", &["unix: now", "时间戳: 1700000000123ms"][..]),
            ("？ ＵＮＩＸ", &["unix: now", "时间戳: 1700000000123ms"][..]),
            ("？ ２＊＊１０", &["2**10"][..]),
            (
                "? 时长",
                &[
                    "时长: 5400秒",
                    "时长: 01:30:00 - 25分钟",
                    "时长: 00:00:01.25 + 500ms",
                    "时长: 45分钟 * 12",
                    "时长: 01:30:00 to min",
                    "时间戳: 现在 + 90分钟 to UTC+8",
                ][..],
            ),
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
        let mut bindings = crate::settings::defaults();
        bindings[0].enabled = false;
        bindings[0].spec.process_scope.allow = vec!["editor.exe".into()];
        let index = SearchIndex {
            triggers: bindings.iter().map(SearchTrigger::from_binding).collect(),
            wheels: crate::wheels::Wheels::default()
                .wheels
                .iter()
                .map(SearchWheel::from_definition)
                .collect(),
            settings: SettingsSection::ALL.to_vec(),
            ..SearchIndex::default()
        };
        for example in EXAMPLES {
            match index.results(example.query) {
                Results::Value(Ok(_)) | Results::Calculations(_) | Results::Conversions(_) => {}
                Results::ApplicationsLoading => {}
                Results::ConversionTargets(items) if !items.is_empty() => {}
                Results::Destinations(items) if !items.is_empty() => {}
                _ => panic!("用法示例没有可用结果：{}", example.query),
            }
        }
    }

    #[test]
    fn quick_search_routes_wheel_names_and_scope_to_wheel_entries() {
        let settings = crate::wheels::Wheels::default();
        let index = SearchIndex {
            wheels: vec![SearchWheel::from_definition(&settings.wheels[0])],
            ..SearchIndex::default()
        };
        for query in ["编辑轨道", "wheel: orbit-edit", "轮盘：", "轨道轮盘：编辑"] {
            let result = index.results(query);
            assert!(
                matches!(result, Results::Destinations(items) if items == [Destination::Wheel("orbit-edit".into())]),
                "{query}"
            );
        }
    }

    #[test]
    fn installed_app_search_uses_an_explicit_scope() {
        let index = SearchIndex::default();
        for query in ["应用: 计算器", "apps: calculator", "程序：记事本"] {
            assert!(matches!(index.results(query), Results::ApplicationsLoading));
        }
        assert!(matches!(
            index.results("动作: calculator"),
            Results::Destinations(_)
        ));
        assert!(!matches!(
            index.results("calculator"),
            Results::ApplicationsLoading
        ));
    }

    #[test]
    fn routes_obvious_date_queries_and_keeps_numeric_subtraction() {
        for (query, expected) in [
            ("2026-11-21 星期几", "星期六"),
            ("2026-W47", "2026-11-16 至 2026-11-22"),
            ("2026-Q4", "2026-10-01 至 2026-12-31\n92 天 · 66 工作日"),
            ("2026-11-21", "1994"),
            ("2024-02", "2022"),
        ] {
            assert!(
                matches!(for_query(query, &[]), Results::Value(Ok(value)) if value.text == expected),
                "{query}"
            );
        }
        let mut binding = crate::settings::defaults().remove(0);
        binding.message = "明天".into();
        let trigger = SearchTrigger::from_binding(&binding);
        assert!(matches!(
            for_query("明天", &[trigger]),
            Results::Destinations(items) if items == [Destination::Trigger(binding.spec.id)]
        ));
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
            matches!(for_query(" ", &[]), Results::Destinations(items) if items.len() == DESTINATION_PAGE_SIZE)
        );
    }

    #[test]
    fn quick_search_keeps_ascii_and_fullwidth_calculations() {
        for query in ["2**10", "（２＾１０）", "＝２＾１０"] {
            assert!(
                matches!(for_query(query, &[]), Results::Value(Ok(result)) if result.text == "1024")
            );
        }
        assert!(matches!(
            for_query("2+", &[]),
            Results::Value(Err(ValueError::Calculation(CalculationError::Incomplete)))
        ));
        assert!(
            matches!(for_query("100厘米转米", &[]), Results::Value(Ok(result)) if result.text == "1 m")
        );
        assert!(matches!(
            for_query("日期：2024-02-28 + 1天", &[]),
            Results::Value(Ok(result)) if result.text == "2024-02-29"
        ));
        assert!(matches!(
            for_query("时间戳：-1ms", &[]),
            Results::Value(Ok(result)) if result.text == "1969-12-31T23:59:59.999Z"
        ));
        assert!(matches!(
            for_query("时长：01:30:00", &[]),
            Results::Value(Ok(result)) if result.text == "5400 秒"
        ));
        assert!(matches!(
            for_query("颜色：#3A86FF", &[]),
            Results::Value(Ok(ResultValue { text, kind: ValueKind::Color(0x3a86ffff) }))
                if text == "rgb(58, 134, 255)"
        ));
    }

    #[test]
    fn reuses_calculations_as_operands_without_changing_precedence() {
        for (query, expected) in [("2**10", "1024"), ("1-4", "(-3)")] {
            let operand = for_query(query, &[]).continuation_query().unwrap();
            assert_eq!(operand, expected);
        }
        for query in [
            "2+",
            "日期: 今天",
            "颜色: #3A86FF",
            "下载: 1 GB 按 100 Mbps",
        ] {
            assert!(
                for_query(query, &[]).continuation_query().is_none(),
                "{query}"
            );
        }
    }

    #[test]
    fn reuses_exact_results_as_grouped_operands() {
        for (query, expected_operand, expected_result) in [
            ("分数: 计算 1/2+1/3", "分数: 计算 (5/6)", "5"),
            ("fraction: -42/56", "分数: 计算 (-3/4)", "-9/2"),
        ] {
            let result = for_query(query, &[]);
            let operand = result.continuation_query().expect("分数结果可继续计算");
            assert_eq!(operand, expected_operand);
            assert!(matches!(
                for_query(&format!("{operand}*6"), &[]),
                Results::Value(Ok(ResultValue { text, .. })) if text == expected_result
            ));
        }
        let large = for_query("分数: 计算 250!", &[]);
        assert!(matches!(
            &large,
            Results::Value(Ok(value))
                if value.text.len() > calculator::QUERY_LIMIT
                    && value.continuation_hint().is_none()
        ));
        assert_eq!(large.continuation_query(), None);
    }

    #[test]
    fn reopens_conversion_targets_from_the_original_source() {
        for (query, expected) in [
            ("10 km to mi", "10 km to "),
            ("0xff to dec", "0xff to "),
            ("255 to bin", "255 to "),
            ("１２ km 转英里", "12 km to "),
        ] {
            let result = for_query(query, &[]);
            let next = result.continuation_query().expect("换算可继续选目标");
            assert_eq!(next, expected);
            assert!(
                matches!(for_query(&next, &[]), Results::ConversionTargets(items)
                if items.len() > 1)
            );
        }
        assert_eq!(for_query("255", &[]).arithmetic_result(), Some("255"));
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
    fn quick_search_filters_triggers_by_enabled_status_and_application_scope() {
        let mut bindings = crate::settings::defaults();
        let bindings = &mut bindings[..2];
        bindings[0].message = "发送正文".into();
        bindings[1].message = "发送正文".into();
        bindings[1].enabled = false;
        bindings[1].spec.pointer_scope.allow = vec!["editor.exe".into()];
        let triggers: Vec<_> = bindings.iter().map(SearchTrigger::from_binding).collect();
        let global = Destination::Trigger(bindings[0].spec.id);
        let app = Destination::Trigger(bindings[1].spec.id);
        assert!(triggers[1].detail.contains("已停用 · 指针窗口：仅 editor"));
        for (query, expected) in [
            ("触发器: 发送正文", vec![global.clone(), app.clone()]),
            ("已启用触发器: 发送正文", vec![global.clone()]),
            ("已停用触发器：", vec![app.clone()]),
            ("已停用触发器", vec![app.clone()]),
            ("DISABLED TRIGGERS: editor", vec![app.clone()]),
            ("全局触发器: 发送正文", vec![global]),
            ("指定应用触发器: editor", vec![app]),
            ("已启用触发器: editor", vec![]),
        ] {
            let Results::Destinations(items) = for_query(query, &triggers) else {
                panic!("没有按触发器筛选：{query}");
            };
            assert_eq!(items, expected, "{query}");
        }
    }

    #[test]
    fn trigger_search_keeps_matches_for_later_pages() {
        let bindings = crate::settings::defaults();
        let triggers: Vec<_> = bindings.iter().map(SearchTrigger::from_binding).collect();
        let results = for_query("trigger:", &triggers);
        assert_eq!(
            results.height(),
            132. + DESTINATION_PAGE_SIZE as f32 * PAGE_ROW_HEIGHT
        );
        let Results::Destinations(items) = results else {
            panic!("触发器搜索没有返回结果");
        };
        assert!(items.len() > DESTINATION_PAGE_SIZE);
        assert_eq!(
            items,
            bindings
                .iter()
                .map(|binding| Destination::Trigger(binding.spec.id))
                .collect::<Vec<_>>()
        );
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
                "触发记录：CapsLock+W",
                Destination::TriggerHistoryQuery("CapsLock+W".into()),
            ),
            ("hits:", Destination::TriggerHistoryQuery(String::new())),
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
