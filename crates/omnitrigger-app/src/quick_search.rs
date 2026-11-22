mod actions;
mod applications;
mod aspect_ratio;
mod batch;
mod calculator;
mod calendar;
mod cidr;
mod circle;
mod color;
mod complex;
mod conversion;
mod csv_tools;
mod duration;
mod equation;
mod examples;
mod file_mode;
mod folders;
mod fraction;
mod identifiers;
mod json_tools;
mod linear_fit;
mod local_files;
mod matrix;
mod open_windows;
mod operators;
mod paper;
mod path_tools;
mod percentage;
mod presets;
mod radix;
mod ratio_allocation;
mod recent;
mod rectangle;
mod renminbi;
mod scientific;
mod scope;
mod settings_sections;
mod statistics;
mod text_tools;
mod timecode;
mod timestamp;
mod triangle;
mod triggers;
mod url_encoding;
mod value_history;
mod value_preview;
mod vector;
mod video_frames;
mod wheel_slots;
mod wheels;
mod windows_settings;

use std::{cell::RefCell, path::Path, rc::Rc, sync::Arc};

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
use batch::{batch_arguments, map_batch_csv, map_batch_lines, BatchError};
use calculator::{calculate, looks_like_calculation, CalculationError};
use calendar::{calculate_date, looks_like_date_query, DateError};
use cidr::{calculate_cidr, CidrError};
use circle::{calculate_circle, CircleError};
use color::{calculate_contrast, convert_color, make_color_palette, ColorError};
use complex::{calculate_complex, ComplexError};
use conversion::{
    calculate_circuit, calculate_fuel_economy, calculate_pace, calculate_pixel_density,
    calculate_unit_price, conversion_target_input, convert, convert_batch, convert_batch_csv,
    convert_period, estimate_transfer, suggested_targets, ConversionError,
};
use csv_tools::{transform_csv, CsvError};
use duration::{convert_duration, DurationError};
use equation::{solve_equation, solve_linear_system, EquationError};
use examples::{
    function_examples, matching_examples, tool_index_examples, tool_operation_examples, EXAMPLES,
};
use file_mode::{calculate_file_mode, FileModeError};
use folders::{matching_folders, SearchFolder};
use fraction::{calculate_fraction, parse_fraction_literal, simplify_fraction, FractionError};
use identifiers::{transform_ulid, transform_uuid, IdentifierError};
use json_tools::{transform_json, JsonError};
use linear_fit::{calculate_linear_fit, LinearFitError};
use local_files::{directory_query, DirectoryLoad, LocalFilesIndex, FOLDER_DEBOUNCE};
use matrix::{calculate_matrix, MatrixError};
use omnitrigger_windows::{OpenWindow, ShellApp, ShortcutTarget};
use open_windows::OpenWindowsIndex;
use paper::{calculate_paper, PaperError};
use path_tools::{path_component, PathError};
use percentage::{compare_percentages, PercentageError};
use presets::matching_presets;
use radix::{convert_arbitrary_radix, RadixError};
use ratio_allocation::{allocate_ratio, RatioAllocationError};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use recent::{RecentDestinations, RecentExamples};
use rectangle::{calculate_rectangle, RectangleError};
use renminbi::{uppercase_renminbi, RenminbiError};
use scope::{scoped_query, tool_example_keyword, Scope};
use settings_sections::matching_settings;
use statistics::{calculate_statistics, StatisticsError};
use text_tools::{count_text, transform_text, TextError};
use timecode::{convert_timecode, TimecodeError};
use timestamp::{convert_timestamp, TimestampError};
use triangle::{calculate_triangle, TriangleError};
use triggers::{matching_triggers, SearchTrigger};
use unicode_segmentation::UnicodeSegmentation;
use url_encoding::{convert_url_component, UrlEncodingError};
use value_history::RecentValues;
use vector::{calculate_vector, VectorError};
use video_frames::{convert_video_frames, VideoFrameError};
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
const EXPANDED_VALUE_LINES: usize = 12;
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
    OpenWindowsLoading,
    OpenWindowsUnavailable(String),
    LocalFilesPrompt,
    DirectoryInput(InputSuggestion),
    LocalFilesLoading,
    LocalFilesUnavailable(String),
    Help(Vec<usize>),
    Calculations(Vec<InputSuggestion>),
    Conversions(Vec<InputSuggestion>),
    ConversionTargets(Vec<InputSuggestion>),
    FileOptions(Vec<InputSuggestion>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct InputSuggestion {
    expression: String,
    label: Option<&'static str>,
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
    KnownFolder(SearchFolder),
    WindowsSetting {
        title: &'static str,
        page: omnitrigger_windows::WindowsSettingsPage,
    },
    LocalFile {
        name: String,
        path: String,
        is_directory: bool,
        size_bytes: Option<u64>,
    },
    OpenWindow(OpenWindow),
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
    open_windows: OpenWindowsIndex,
    local_files: LocalFilesIndex,
}

#[derive(Clone, Default)]
pub(super) struct RecentSearch {
    pub(super) destinations: RecentDestinations,
    calculations: RecentValues,
    conversions: RecentValues,
    examples: RecentExamples,
    applications: ApplicationCache,
    last_input: Rc<RefCell<String>>,
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
            open_windows: OpenWindowsIndex::default(),
            local_files: LocalFilesIndex::default(),
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

fn is_application_name_query(query: &str) -> bool {
    !query.contains([':', '：', '\r', '\n'])
        && scoped_query(query).is_none()
        && query.chars().take(2).count() == 2
        && query.chars().any(char::is_alphabetic)
}

fn value_extra_height(value: &str, line_limit: usize) -> f32 {
    value_preview::lines(value)
        .take(line_limit)
        .count()
        .saturating_sub(2) as f32
        * VALUE_LINE_HEIGHT
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ValueError {
    InputTooLong,
    Calculation(CalculationError),
    BatchCalculation(BatchError<CalculationError>),
    BatchFraction(BatchError<FractionError>),
    Conversion(ConversionError),
    BatchConversion(BatchError<ConversionError>),
    BatchDuration(BatchError<DurationError>),
    BatchTimestamp(BatchError<TimestampError>),
    BatchPath(BatchError<PathError>),
    BatchUrl(BatchError<UrlEncodingError>),
    BatchDate(BatchError<DateError>),
    Radix(RadixError),
    RatioAllocation(RatioAllocationError),
    AspectRatio(AspectRatioError),
    Paper(PaperError),
    Rectangle(RectangleError),
    Cidr(CidrError),
    FileMode(FileModeError),
    Renminbi(RenminbiError),
    Date(DateError),
    LinearFit(LinearFitError),
    Statistics(StatisticsError),
    Triangle(TriangleError),
    Duration(DurationError),
    VideoFrames(VideoFrameError),
    Timecode(TimecodeError),
    Timestamp(TimestampError),
    Identifier(IdentifierError),
    Color(ColorError),
    Text(TextError),
    Json(JsonError),
    Csv(CsvError),
    Fraction(FractionError),
    Equation(EquationError),
    Percentage(PercentageError),
    UrlEncoding(UrlEncodingError),
    Vector(VectorError),
    Matrix(MatrixError),
    Circle(CircleError),
    Complex(ComplexError),
    Path(PathError),
}

impl ValueError {
    fn message(self) -> gpui::SharedString {
        let message = match self {
            Self::InputTooLong => "输入最多支持 4096 个字符",
            Self::Calculation(error) => error.message(),
            Self::BatchCalculation(error) => {
                return batch_error_message(
                    error,
                    "逐行输入算式，例如 2**8",
                    CalculationError::message,
                );
            }
            Self::BatchFraction(error) => {
                return batch_error_message(
                    error,
                    "逐行输入精确算式，例如 1/3 + 1/6",
                    FractionError::message,
                );
            }
            Self::Conversion(error) => error.message(),
            Self::BatchConversion(error) => {
                return batch_error_message(
                    error,
                    "逐行输入完整换算，例如 10 km to mi",
                    ConversionError::message,
                );
            }
            Self::BatchDuration(error) => {
                return batch_error_message(
                    error,
                    "逐行输入时长换算，例如 01:30 to s",
                    DurationError::message,
                );
            }
            Self::BatchTimestamp(error) => {
                return batch_error_message(
                    error,
                    "逐行输入时间戳换算，例如 1700000000 to Asia/Shanghai",
                    TimestampError::message,
                );
            }
            Self::BatchPath(error) => {
                return batch_error_message(
                    error,
                    "逐行输入路径操作，例如 文件名 C:\\docs\\report.txt",
                    PathError::message,
                );
            }
            Self::BatchUrl(error) => {
                return batch_error_message(
                    error,
                    "逐行输入网址操作，例如 去查询串 https://example.invalid/?ref=mail",
                    UrlEncodingError::message,
                );
            }
            Self::BatchDate(error) => {
                return batch_error_message(
                    error,
                    "逐行输入日期操作，例如 2026-11-20 + 1工作日",
                    DateError::message,
                );
            }
            Self::Radix(error) => error.message(),
            Self::RatioAllocation(error) => error.message(),
            Self::AspectRatio(error) => error.message(),
            Self::Paper(error) => error.message(),
            Self::Rectangle(error) => error.message(),
            Self::Cidr(error) => error.message(),
            Self::FileMode(error) => error.message(),
            Self::Renminbi(error) => error.message(),
            Self::Date(error) => error.message(),
            Self::LinearFit(error) => error.message(),
            Self::Statistics(error) => error.message(),
            Self::Triangle(error) => error.message(),
            Self::Duration(error) => error.message(),
            Self::VideoFrames(error) => error.message(),
            Self::Timecode(error) => error.message(),
            Self::Timestamp(error) => error.message(),
            Self::Identifier(error) => error.message(),
            Self::Color(error) => error.message(),
            Self::Text(error) => error.message(),
            Self::Json(JsonError::PatchStep { index, cause }) => {
                return format!("第 {index} 步补丁失败 · {cause}").into();
            }
            Self::Json(error) => error.message(),
            Self::Csv(CsvError::PipelineStep { index, cause }) => {
                return format!("第 {index} 步失败 · {cause}").into();
            }
            Self::Csv(error) => error.message(),
            Self::Fraction(error) => error.message(),
            Self::Equation(error) => error.message(),
            Self::Percentage(error) => error.message(),
            Self::UrlEncoding(error) => error.message(),
            Self::Vector(error) => error.message(),
            Self::Matrix(error) => error.message(),
            Self::Circle(error) => error.message(),
            Self::Complex(error) => error.message(),
            Self::Path(error) => error.message(),
        };
        message.into()
    }

    fn is_incomplete(self) -> bool {
        matches!(
            self,
            Self::Calculation(CalculationError::Incomplete)
                | Self::Conversion(ConversionError::Incomplete)
                | Self::BatchCalculation(BatchError::Empty)
                | Self::BatchFraction(BatchError::Empty)
                | Self::BatchConversion(BatchError::Empty)
                | Self::BatchDuration(BatchError::Empty)
                | Self::BatchTimestamp(BatchError::Empty)
                | Self::BatchPath(BatchError::Empty)
                | Self::BatchUrl(BatchError::Empty)
                | Self::BatchDate(BatchError::Empty)
                | Self::AspectRatio(AspectRatioError::Incomplete)
                | Self::Paper(PaperError::Incomplete)
                | Self::Rectangle(RectangleError::Incomplete)
                | Self::Cidr(CidrError::Incomplete)
                | Self::FileMode(FileModeError::Incomplete)
                | Self::Renminbi(RenminbiError::Incomplete)
                | Self::Date(DateError::Incomplete)
                | Self::LinearFit(LinearFitError::Incomplete)
                | Self::Statistics(
                    StatisticsError::Incomplete
                        | StatisticsError::Paired(LinearFitError::Incomplete)
                )
                | Self::Equation(EquationError::SystemRows)
                | Self::Radix(RadixError::Incomplete)
                | Self::RatioAllocation(RatioAllocationError::Incomplete)
                | Self::Triangle(TriangleError::Dimensions)
                | Self::Duration(DurationError::Incomplete)
                | Self::VideoFrames(VideoFrameError::Incomplete)
                | Self::Timecode(TimecodeError::Incomplete)
                | Self::Timestamp(
                    TimestampError::Incomplete
                        | TimestampError::IntervalOutput(DurationError::Incomplete)
                )
                | Self::Identifier(IdentifierError::Incomplete)
                | Self::Color(ColorError::Incomplete | ColorError::IncompleteContrast)
                | Self::Text(
                    TextError::MissingText
                        | TextError::UnclosedQuote
                        | TextError::IncompleteTransform
                        | TextError::IncompleteReplacement
                        | TextError::IncompleteTextArgument
                        | TextError::IncompleteLineComparison
                )
                | Self::Fraction(FractionError::Incomplete)
                | Self::Equation(
                    EquationError::Coefficients
                        | EquationError::Evaluation(FractionError::Incomplete)
                )
                | Self::Percentage(PercentageError::Incomplete)
                | Self::UrlEncoding(UrlEncodingError::Incomplete)
                | Self::Vector(VectorError::Incomplete)
                | Self::Matrix(MatrixError::Incomplete)
                | Self::Circle(CircleError::Incomplete)
                | Self::Complex(ComplexError::Incomplete)
                | Self::Path(PathError::Incomplete)
                | Self::Json(JsonError::Incomplete)
                | Self::Csv(CsvError::MissingInput | CsvError::UnclosedQuote)
        )
    }
}

fn batch_error_message<E>(
    error: BatchError<E>,
    hint: &'static str,
    message: impl FnOnce(E) -> &'static str,
) -> gpui::SharedString {
    match error {
        BatchError::Empty => hint.into(),
        BatchError::Row { line, cause } => format!("第 {line} 行：{}", message(cause)).into(),
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
            open_windows,
            local_files,
        } = index;
        if query.len() > INPUT_CHARACTER_LIMIT
            && query.chars().take(INPUT_CHARACTER_LIMIT + 1).count() > INPUT_CHARACTER_LIMIT
        {
            return Self::Value(Err(ValueError::InputTooLong));
        }
        let raw_query = query;
        let query = raw_query.trim();
        if query.is_empty() {
            return Self::Destinations(recent.destinations.suggestions(triggers));
        }
        if let Some(terms) = query.strip_prefix(['?', '？']) {
            return Self::Help(matching_examples(terms));
        }
        if let Some((scope, term)) = scoped_query(raw_query) {
            if let Some(examples) = tool_operation_examples(scope, term) {
                return Self::Help(examples);
            }
            return match scope {
                Scope::ToolIndex => Self::Help(tool_index_examples(term)),
                Scope::ExampleHistory => Self::Help(recent.examples.matching(term)),
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
                Scope::BatchCalculation => {
                    if let Some(arguments) = batch_arguments(term, "分数", "exact") {
                        Self::text_value(
                            map_batch_lines(arguments, |query| {
                                calculate_fraction(query).map(|value| value.to_string())
                            })
                            .map_err(ValueError::BatchFraction),
                        )
                    } else if let Some(arguments) = batch_arguments(term, "分数对照", "exact-csv")
                    {
                        Self::text_value(
                            map_batch_csv(arguments, "算式", |query| {
                                calculate_fraction(query).map(|value| value.to_string())
                            })
                            .map_err(ValueError::BatchFraction),
                        )
                    } else if let Some(arguments) = batch_arguments(term, "对照", "csv") {
                        Self::text_value(
                            map_batch_csv(arguments, "算式", calculate)
                                .map_err(ValueError::BatchCalculation),
                        )
                    } else {
                        Self::text_value(
                            map_batch_lines(term, calculate).map_err(ValueError::BatchCalculation),
                        )
                    }
                }
                Scope::ConversionHistory => Self::Conversions(recent.conversions.matching(term)),
                Scope::BatchConversion => {
                    if let Some(arguments) = batch_arguments(term, "对照", "csv") {
                        Self::text_value(
                            convert_batch_csv(arguments).map_err(ValueError::BatchConversion),
                        )
                    } else if let Some(arguments) =
                        batch_arguments(term, "时长对照", "duration-csv")
                    {
                        Self::text_value(
                            map_batch_csv(arguments, "输入", convert_duration)
                                .map_err(ValueError::BatchDuration),
                        )
                    } else if let Some(arguments) = batch_arguments(term, "时长", "duration") {
                        Self::text_value(
                            map_batch_lines(arguments, convert_duration)
                                .map_err(ValueError::BatchDuration),
                        )
                    } else if let Some(arguments) =
                        batch_arguments(term, "时间戳对照", "timestamp-csv")
                    {
                        let now = chrono::Utc::now();
                        Self::text_value(
                            map_batch_csv(arguments, "输入", |line| convert_timestamp(line, now))
                                .map_err(ValueError::BatchTimestamp),
                        )
                    } else if let Some(arguments) = batch_arguments(term, "时间戳", "timestamp")
                    {
                        let now = chrono::Utc::now();
                        Self::text_value(
                            map_batch_lines(arguments, |line| convert_timestamp(line, now))
                                .map_err(ValueError::BatchTimestamp),
                        )
                    } else if let Some(arguments) = batch_arguments(term, "路径对照", "path-csv")
                    {
                        Self::text_value(
                            map_batch_csv(arguments, "输入", path_component)
                                .map_err(ValueError::BatchPath),
                        )
                    } else if let Some(arguments) = batch_arguments(term, "路径", "path") {
                        Self::text_value(
                            map_batch_lines(arguments, path_component)
                                .map_err(ValueError::BatchPath),
                        )
                    } else if let Some(arguments) = batch_arguments(term, "网址对照", "url-csv")
                    {
                        Self::text_value(
                            map_batch_csv(arguments, "输入", convert_url_component)
                                .map_err(ValueError::BatchUrl),
                        )
                    } else if let Some(arguments) = batch_arguments(term, "网址", "url") {
                        Self::text_value(
                            map_batch_lines(arguments, convert_url_component)
                                .map_err(ValueError::BatchUrl),
                        )
                    } else if let Some(arguments) = batch_arguments(term, "日期对照", "date-csv")
                    {
                        let today = chrono::Local::now().date_naive();
                        Self::text_value(
                            map_batch_csv(arguments, "输入", |line| calculate_date(line, today))
                                .map_err(ValueError::BatchDate),
                        )
                    } else if let Some(arguments) = batch_arguments(term, "日期", "date") {
                        let today = chrono::Local::now().date_naive();
                        Self::text_value(
                            map_batch_lines(arguments, |line| calculate_date(line, today))
                                .map_err(ValueError::BatchDate),
                        )
                    } else {
                        Self::text_value(convert_batch(term).map_err(ValueError::BatchConversion))
                    }
                }
                Scope::Radix => Self::Value(
                    convert_arbitrary_radix(term)
                        .map(|text| ResultValue {
                            text,
                            kind: ValueKind::Conversion(None),
                        })
                        .map_err(ValueError::Radix),
                ),
                Scope::Applications => applications.results(term),
                Scope::Folders => Self::Destinations(matching_folders(term)),
                Scope::WindowsSettings => {
                    Self::Destinations(windows_settings::matching_windows_settings(term))
                }
                Scope::LocalFiles => local_files.results(term),
                Scope::OpenWindows => open_windows.results(term),
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
                Scope::VideoFrames => {
                    Self::text_value(convert_video_frames(term).map_err(ValueError::VideoFrames))
                }
                Scope::Timecode => {
                    Self::text_value(convert_timecode(term).map_err(ValueError::Timecode))
                }
                Scope::TransferTime => Self::conversion_value(estimate_transfer(term), None),
                Scope::Pace => {
                    Self::text_value(calculate_pace(term).map_err(ValueError::Conversion))
                }
                Scope::FuelEconomy => {
                    Self::text_value(calculate_fuel_economy(term).map_err(ValueError::Conversion))
                }
                Scope::UnitPrice => {
                    Self::text_value(calculate_unit_price(term).map_err(ValueError::Conversion))
                }
                Scope::Renminbi => {
                    Self::text_value(uppercase_renminbi(term).map_err(ValueError::Renminbi))
                }
                Scope::Circuit => {
                    Self::text_value(calculate_circuit(term).map_err(ValueError::Conversion))
                }
                Scope::Period => {
                    Self::text_value(convert_period(term).map_err(ValueError::Conversion))
                }
                Scope::AspectRatio => {
                    Self::text_value(calculate_aspect_ratio(term).map_err(ValueError::AspectRatio))
                }
                Scope::PixelDensity => {
                    Self::text_value(calculate_pixel_density(term).map_err(ValueError::Conversion))
                }
                Scope::Paper => Self::text_value(calculate_paper(term).map_err(ValueError::Paper)),
                Scope::Rectangle => {
                    Self::text_value(calculate_rectangle(term).map_err(ValueError::Rectangle))
                }
                Scope::Cidr => Self::text_value(calculate_cidr(term).map_err(ValueError::Cidr)),
                Scope::FileMode => {
                    Self::text_value(calculate_file_mode(term).map_err(ValueError::FileMode))
                }
                Scope::Timestamp => Self::text_value(
                    convert_timestamp(term, chrono::Utc::now()).map_err(ValueError::Timestamp),
                ),
                Scope::Uuid => {
                    Self::text_value(transform_uuid(term).map_err(ValueError::Identifier))
                }
                Scope::Ulid => {
                    Self::text_value(transform_ulid(term).map_err(ValueError::Identifier))
                }
                Scope::Color => {
                    if let Some(input) = batch_arguments(term, "色阶", "palette") {
                        Self::text_value(make_color_palette(input).map_err(ValueError::Color))
                    } else {
                        Self::Value(
                            convert_color(term)
                                .map(|color| ResultValue {
                                    text: color.output,
                                    kind: ValueKind::Color(color.swatch),
                                })
                                .map_err(ValueError::Color),
                        )
                    }
                }
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
                Scope::Csv => Self::text_value(transform_csv(term).map_err(ValueError::Csv)),
                Scope::Fraction => Self::Value(
                    simplify_fraction(term)
                        .map(|text| ResultValue {
                            kind: ValueKind::ExactCalculation {
                                // 为括号、运算符和至少一位操作数预留空间。
                                can_continue: text.len() <= calculator::QUERY_LIMIT - 4
                                    && parse_fraction_literal(&text).is_ok(),
                            },
                            text,
                        })
                        .map_err(ValueError::Fraction),
                ),
                Scope::Equation => {
                    Self::text_value(solve_equation(term).map_err(ValueError::Equation))
                }
                Scope::LinearSystem => {
                    Self::text_value(solve_linear_system(term).map_err(ValueError::Equation))
                }
                Scope::LinearFit => {
                    Self::text_value(calculate_linear_fit(term).map_err(ValueError::LinearFit))
                }
                Scope::Statistics => {
                    Self::text_value(calculate_statistics(term).map_err(ValueError::Statistics))
                }
                Scope::Triangle => {
                    Self::text_value(calculate_triangle(term).map_err(ValueError::Triangle))
                }
                Scope::Percentage => {
                    Self::text_value(compare_percentages(term).map_err(ValueError::Percentage))
                }
                Scope::RatioAllocation => {
                    Self::text_value(allocate_ratio(term).map_err(ValueError::RatioAllocation))
                }
                Scope::UrlEncoding => {
                    Self::text_value(convert_url_component(term).map_err(ValueError::UrlEncoding))
                }
                Scope::Vector => {
                    Self::text_value(calculate_vector(term).map_err(ValueError::Vector))
                }
                Scope::Matrix => {
                    Self::text_value(calculate_matrix(term).map_err(ValueError::Matrix))
                }
                Scope::Path => Self::text_value(path_component(term).map_err(ValueError::Path)),
                Scope::Circle => {
                    Self::text_value(calculate_circle(term).map_err(ValueError::Circle))
                }
                Scope::Complex => {
                    Self::text_value(calculate_complex(term).map_err(ValueError::Complex))
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
            let result = calculate(query);
            let examples = match result {
                Err(CalculationError::Incomplete | CalculationError::Invalid) => {
                    function_examples(query)
                }
                _ => None,
            };
            if let Some(examples) = examples {
                return Self::Help(examples);
            }
            return Self::Value(
                result
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
            if let Some(examples) = function_examples(&query) {
                return Self::Help(examples);
            }
            if let Some(keyword) = tool_example_keyword(&query) {
                return Self::Help(matching_examples(keyword));
            }
            if is_application_name_query(&query)
                && !matches!(applications, ApplicationIndex::Unavailable(_))
            {
                return applications.results(&query);
            }
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
            Self::DirectoryInput(item) if selected == 0 => Some(&item.expression),
            Self::Calculations(items)
            | Self::Conversions(items)
            | Self::ConversionTargets(items)
            | Self::FileOptions(items) => Some(&items.get(selected)?.expression),
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

    fn subdirectory_query(&self, selected: usize) -> Option<String> {
        let Self::Destinations(items) = self else {
            return None;
        };
        if let Destination::LocalFile {
            path,
            is_directory: true,
            ..
        } = items.get(selected)?
        {
            Some(directory_query(path))
        } else {
            None
        }
    }

    fn local_file_path(&self, selected: usize) -> Option<&str> {
        let Self::Destinations(items) = self else {
            return None;
        };
        match items.get(selected)? {
            Destination::LocalFile { path, .. } => Some(path),
            _ => None,
        }
    }

    fn value_preview_expandable(&self) -> bool {
        matches!(
            self,
            Self::Value(Ok(value)) if value_preview::lines(&value.text).nth(MAX_VALUE_LINES).is_some()
        )
    }

    fn height(&self, value_line_limit: usize) -> f32 {
        match self {
            Self::Destinations(items) if items.len() > 1 => {
                132. + items.len().min(DESTINATION_PAGE_SIZE) as f32 * PAGE_ROW_HEIGHT
            }
            Self::Value(Ok(value)) => {
                let hints = usize::from(value.continuation_hint().is_some())
                    + usize::from(self.value_preview_expandable());
                158. + value_extra_height(&value.text, value_line_limit) + hints as f32 * 24.
            }
            Self::Help(items) if !items.is_empty() => {
                132. + items.len().min(EXAMPLE_PAGE_SIZE) as f32 * EXAMPLE_ROW_HEIGHT
            }
            Self::DirectoryInput(_) => 132. + EXAMPLE_ROW_HEIGHT,
            Self::ConversionTargets(items) if !items.is_empty() => {
                132. + items.len().min(CONVERSION_PAGE_SIZE) as f32 * EXAMPLE_ROW_HEIGHT
            }
            Self::FileOptions(items) if !items.is_empty() => {
                132. + items.len().min(EXAMPLE_PAGE_SIZE) as f32 * EXAMPLE_ROW_HEIGHT
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

enum PageChange {
    Previous,
    Next,
    First,
    Last,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ApplicationRefresh {
    NotRequested,
    Loading,
    Finished,
}

pub(super) struct QuickSearch {
    theme_mode: appearance::Mode,
    input: Entity<MessageInput>,
    workspace: WeakEntity<Workspace>,
    index: SearchIndex,
    was_active: bool,
    results: Results,
    expanded_value_pages: Option<Vec<usize>>,
    selected: usize,
    application_refresh: ApplicationRefresh,
    input_before_help: Option<String>,
    previous_input: String,
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
        let bounds = Bounds::centered(
            display,
            size(px(WIDTH), px(results.height(MAX_VALUE_LINES))),
            cx,
        );
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
            if !query.trim().is_empty() {
                this.index.recent.last_input.borrow_mut().clone_from(&query);
            }
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
        let previous_input = index.recent.last_input.borrow().clone();
        Self {
            theme_mode,
            input,
            workspace,
            index,
            was_active: window.is_window_active(),
            results,
            expanded_value_pages: None,
            selected: 0,
            application_refresh: ApplicationRefresh::NotRequested,
            input_before_help: None,
            previous_input,
            _input_changes: input_changes,
            _activation: activation,
        }
    }

    pub(super) fn activate(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.activate_window();
        window.focus(&self.input.focus_handle(cx));
    }

    fn value_line_limit(&self) -> usize {
        if self.expanded_value_pages.is_some() {
            EXPANDED_VALUE_LINES
        } else {
            MAX_VALUE_LINES
        }
    }

    fn toggle_value_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Results::Value(Ok(value)) = &self.results else {
            return;
        };
        if !self.results.value_preview_expandable() {
            return;
        }
        self.expanded_value_pages = self
            .expanded_value_pages
            .is_none()
            .then(|| value_preview::page_starts(&value.text, EXPANDED_VALUE_LINES));
        self.selected = 0;
        window.resize(size(
            px(WIDTH),
            px(self.results.height(self.value_line_limit())),
        ));
        cx.notify();
    }

    fn refresh_results(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        let next = self.index.results(query);
        if !query.trim_start().starts_with(['?', '？']) && !matches!(&next, Results::FileOptions(_))
        {
            self.input_before_help = None;
        }
        if self.results != next {
            let previous_height = self.results.height(self.value_line_limit());
            let next_height = next.height(MAX_VALUE_LINES);
            self.results = next;
            self.expanded_value_pages = None;
            self.selected = 0;
            if previous_height != next_height {
                window.resize(size(px(WIDTH), px(next_height)));
            }
        }
        let query = query.trim();
        let local_files_query = match scoped_query(query) {
            Some((Scope::LocalFiles, term))
                if !matches!(self.results, Results::Value(Err(ValueError::InputTooLong))) =>
            {
                Some(term)
            }
            _ => None,
        };
        if let Some(load) = self.index.local_files.request(local_files_query) {
            self.load_local_files(load, window, cx);
        }
        if self.application_refresh == ApplicationRefresh::NotRequested
            && !matches!(self.results, Results::Value(Err(ValueError::InputTooLong)))
            && (matches!(scoped_query(query), Some((Scope::Applications, _)))
                || matches!(self.results, Results::ApplicationsLoading)
                || matches!(&self.results, Results::Destinations(items)
                    if is_application_name_query(query)
                        && items.iter().all(|item| matches!(item, Destination::InstalledApp(_)))))
            && self
                .index
                .applications
                .needs_refresh(std::time::Instant::now())
        {
            self.load_applications(window, cx);
        }
        if matches!(self.index.open_windows, OpenWindowsIndex::Unloaded)
            && matches!(self.results, Results::OpenWindowsLoading)
        {
            self.load_open_windows(window, cx);
        }
        cx.notify();
    }

    fn load_open_windows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.index.open_windows.begin_loading() {
            return;
        }
        cx.spawn_in(window, async move |this, cx| {
            let open_windows = cx
                .background_executor()
                .spawn(async { OpenWindowsIndex::load() })
                .await;
            let _ = cx.update(|window, cx| {
                let _ = this.update(cx, |this, cx| {
                    let previous = match &this.results {
                        Results::Destinations(items) => items.get(this.selected).cloned(),
                        _ => None,
                    };
                    this.index.open_windows = open_windows;
                    let query = this.input.read(cx).text();
                    this.refresh_results(&query, window, cx);
                    let Some(Destination::OpenWindow(previous)) = previous else {
                        return;
                    };
                    let Results::Destinations(items) = &this.results else {
                        return;
                    };
                    if let Some(index) = items.iter().position(|item| {
                        matches!(item, Destination::OpenWindow(target) if target.same_window(&previous))
                    }) {
                        this.selected = index;
                    }
                });
            });
        })
        .detach();
    }

    fn load_applications(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.application_refresh == ApplicationRefresh::Loading {
            return;
        }
        self.application_refresh = ApplicationRefresh::Loading;
        let cached = self.index.recent.applications.clone();
        cx.spawn_in(window, async move |this, cx| {
            let applications = cx
                .background_executor()
                .spawn(async { ApplicationIndex::load() })
                .await;
            cached.remember(&applications);
            let _ = cx.update(|window, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.application_refresh = ApplicationRefresh::Finished;
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

    fn load_local_files(
        &mut self,
        load: Arc<DirectoryLoad>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            let reader = cx.background_executor();
            reader.timer(FOLDER_DEBOUNCE).await;
            let task = load.clone();
            let entries = reader.spawn(async move { task.read() }).await;
            let _ = cx.update(|window, cx| {
                let _ = this.update(cx, |this, cx| {
                    if this.index.local_files.finish(&load, entries) {
                        let previous = this.results.local_file_path(this.selected).map(str::to_owned);
                        let query = this.input.read(cx).text();
                        this.refresh_results(&query, window, cx);
                        if let (Some(path), Results::Destinations(items)) = (previous, &this.results) {
                            if let Some(index) = items.iter().position(|item| {
                                matches!(item, Destination::LocalFile { path: current, .. } if current == &path)
                            }) {
                                this.selected = index;
                            }
                        }
                    }
                });
            });
        })
        .detach();
    }

    fn open_destination(
        &self,
        destination: Destination,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Destination::OpenWindow(target) = destination {
            let Some(hwnd) = window
                .window_handle()
                .ok()
                .and_then(|handle| match handle.as_raw() {
                    RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
                    _ => None,
                })
            else {
                return;
            };
            cx.spawn_in(window, async move |this, cx| {
                // 切换前台可能同步进入窗口回调，在 GPUI 可变借用之外执行。
                let result = if omnitrigger_windows::is_foreground_window(hwnd) {
                    target.activate()
                } else {
                    Err("搜索框已失去焦点，请重新打开后切换。".to_owned())
                };
                let _ = cx.update(|window, cx| match result {
                    Ok(()) => window.remove_window(),
                    Err(error) => {
                        let _ = this.update(cx, |this, cx| {
                            this.results = Results::OpenWindowsUnavailable(error);
                            this.selected = 0;
                            window
                                .resize(size(px(WIDTH), px(this.results.height(MAX_VALUE_LINES))));
                            cx.notify();
                        });
                    }
                });
            })
            .detach();
            return;
        }
        let workspace = self.workspace.clone();
        window.remove_window();
        cx.spawn(async move |_, cx| {
            let _ = workspace.update(cx, |workspace, cx| {
                workspace.open_quick_search_destination(destination, cx)
            });
        })
        .detach();
    }

    fn reveal_selected_file(&self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(path) = self.results.local_file_path(self.selected) else {
            return false;
        };
        cx.reveal_path(Path::new(path));
        window.remove_window();
        true
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
                    self.remember_selected_example();
                    self.fill_query(query.to_owned(), window, cx);
                }
            }
        }
    }

    fn remember_selected_example(&self) {
        let Results::Help(items) = &self.results else {
            return;
        };
        if let Some(&index) = items.get(self.selected) {
            self.index.recent.examples.record(index);
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
            if modifiers == 2
                && keystroke.modifiers.control
                && keystroke.modifiers.shift
                && keystroke.key == "r"
            {
                if !event.is_held && !self.previous_input.is_empty() {
                    self.fill_query(self.previous_input.clone(), window, cx);
                }
                cx.stop_propagation();
            } else if modifiers == 1 && keystroke.modifiers.shift && keystroke.key == "enter" {
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
            } else if modifiers == 1
                && keystroke.modifiers.control
                && matches!(keystroke.key.as_str(), "pageup" | "pagedown")
            {
                let page = if keystroke.key == "pageup" {
                    PageChange::First
                } else {
                    PageChange::Last
                };
                if self.change_page(page, cx) {
                    cx.stop_propagation();
                }
            } else if modifiers == 1 && keystroke.modifiers.alt && keystroke.key == "enter" {
                if !event.is_held && self.reveal_selected_file(window, cx) {
                    cx.stop_propagation();
                }
            } else if modifiers == 1 && keystroke.modifiers.alt && keystroke.key == "up" {
                let query = self.input.read(cx).text();
                let Some((Scope::LocalFiles, term)) = scoped_query(&query) else {
                    return;
                };
                if !event.is_held {
                    if let Some(parent) = self.index.local_files.parent_query(term) {
                        self.index.local_files.remember_navigation(query);
                        self.fill_query(parent, window, cx);
                    }
                }
                cx.stop_propagation();
            } else if modifiers == 1
                && keystroke.modifiers.alt
                && matches!(keystroke.key.as_str(), "left" | "right")
            {
                let query = self.input.read(cx).text();
                if !matches!(scoped_query(&query), Some((Scope::LocalFiles, _))) {
                    return;
                }
                if !event.is_held {
                    let forward = keystroke.key == "right";
                    if let Some(previous) = self.index.local_files.move_navigation(query, forward) {
                        self.fill_query(previous, window, cx);
                    }
                }
                cx.stop_propagation();
            }
            return;
        }
        match keystroke.key.as_str() {
            "f1" => {
                if !event.is_held {
                    let query = self.input.read(cx).text();
                    if query.trim_start().starts_with(['?', '？'])
                        || matches!(&self.results, Results::FileOptions(_))
                    {
                        if let Some(previous) = self.input_before_help.take() {
                            self.fill_query(previous, window, cx);
                        }
                    } else {
                        let help = match scoped_query(query.trim()) {
                            Some((Scope::LocalFiles, term)) => local_files::option_help_query(term),
                            _ => None,
                        };
                        self.input_before_help = Some(query);
                        self.fill_query(help.unwrap_or_else(|| "?".to_owned()), window, cx);
                    }
                }
            }
            "f2" if self.results.value_preview_expandable() => {
                if !event.is_held {
                    self.toggle_value_preview(window, cx);
                }
            }
            "f5" => {
                let query = self.input.read(cx).text();
                match scoped_query(query.trim()) {
                    Some((Scope::LocalFiles | Scope::OpenWindows | Scope::Applications, _))
                        if event.is_held => {}
                    Some((Scope::LocalFiles, term)) => {
                        if let Some(load) = self.index.local_files.reload(term) {
                            self.load_local_files(load, window, cx);
                        }
                    }
                    Some((Scope::OpenWindows, _)) => self.load_open_windows(window, cx),
                    Some((Scope::Applications, _)) => self.load_applications(window, cx),
                    _ => return,
                }
            }
            "enter" => {
                if !event.is_held {
                    self.confirm(window, cx);
                }
            }
            "escape" => window.remove_window(),
            key @ ("pageup" | "pagedown") => {
                let page = if key == "pageup" {
                    PageChange::Previous
                } else {
                    PageChange::Next
                };
                if !self.change_page(page, cx) {
                    return;
                }
            }
            "tab" => {
                let suggestion = self
                    .results
                    .suggested_input(self.selected)
                    .map(str::to_owned);
                let subdirectory = self.results.subdirectory_query(self.selected);
                let continues_value = matches!(
                    &self.results,
                    Results::Value(Ok(value)) if value.continuation_hint().is_some()
                );
                if suggestion.is_none() && subdirectory.is_none() && !continues_value {
                    return;
                }
                if !event.is_held {
                    if let Some(query) = suggestion {
                        self.remember_selected_example();
                        self.fill_query(query, window, cx);
                    } else if let Some(target) = subdirectory {
                        let query = self.input.read(cx).text();
                        self.index.local_files.remember_navigation(query);
                        self.fill_query(target, window, cx);
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
                    | Results::ConversionTargets(items)
                    | Results::FileOptions(items) => items.len(),
                    Results::Value(_)
                    | Results::ApplicationsLoading
                    | Results::ApplicationsUnavailable(_)
                    | Results::OpenWindowsLoading
                    | Results::OpenWindowsUnavailable(_)
                    | Results::LocalFilesPrompt
                    | Results::DirectoryInput(_)
                    | Results::LocalFilesLoading
                    | Results::LocalFilesUnavailable(_) => 0,
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

    fn change_page(&mut self, movement: PageChange, cx: &mut Context<Self>) -> bool {
        let (count, page_size) = match &self.results {
            Results::Destinations(items) => (items.len(), DESTINATION_PAGE_SIZE),
            Results::ConversionTargets(items) => (items.len(), CONVERSION_PAGE_SIZE),
            Results::FileOptions(items) => (items.len(), EXAMPLE_PAGE_SIZE),
            Results::Help(items) => (items.len(), EXAMPLE_PAGE_SIZE),
            Results::Value(Ok(_)) => (self.expanded_value_pages.as_ref().map_or(0, Vec::len), 1),
            _ => return false,
        };
        if count <= page_size {
            return false;
        }
        let page = self.selected / page_size;
        let last_page = (count - 1) / page_size;
        let next = match movement {
            PageChange::Previous => page.saturating_sub(1),
            PageChange::Next => (page + 1).min(last_page),
            PageChange::First => 0,
            PageChange::Last => last_page,
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
        Self::result_footer().child(label)
    }

    fn result_footer() -> Div {
        div()
            .mt(px(5.))
            .px(px(10.))
            .text_color(rgb(theme::TEXT_MUTED))
    }

    fn render_pagination(
        &self,
        count: usize,
        page_size: usize,
        hint: &'static str,
        cx: &mut Context<Self>,
    ) -> Div {
        Self::selection_hint(hint)
            .flex()
            .items_center()
            .gap(px(6.))
            .child(div().flex_1())
            .child(self.page_controls(count, page_size, cx))
    }

    fn page_controls(&self, count: usize, page_size: usize, cx: &mut Context<Self>) -> Div {
        let page = self.selected / page_size;
        let page_count = count.div_ceil(page_size);
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .child(
                button("result-prev", "上一页", Tone::Ghost, page > 0)
                    .h(px(20.))
                    .tooltip(theme::text_tooltip(
                        "PageUp 上一页 · Ctrl+PageUp 首页".into(),
                    ))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.change_page(PageChange::Previous, cx);
                    })),
            )
            .child(format!("{} / {page_count}", page + 1))
            .child(
                button("result-next", "下一页", Tone::Ghost, page + 1 < page_count)
                    .h(px(20.))
                    .tooltip(theme::text_tooltip(
                        "PageDown 下一页 · Ctrl+PageDown 末页".into(),
                    ))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.change_page(PageChange::Next, cx);
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
                    .map(|(index, &example_index)| {
                        let example = &EXAMPLES[example_index];
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
                                this.index.recent.examples.record(example_index);
                                this.fill_query(query, window, cx);
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
                let (label, detail) = match item.label {
                    Some(label) => (label.to_owned(), item.result.clone()),
                    None => (expression.clone(), format!("= {}", item.result)),
                };
                self.suggestion_row(index, EXAMPLE_ROW_HEIGHT)
                    .child(icon(glyph, 16., theme::ACCENT))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(theme::MONO_FONT)
                            .child(label),
                    )
                    .child(
                        div()
                            .max_w(px(200.))
                            .truncate()
                            .font_family(theme::MONO_FONT)
                            .text_color(rgb(theme::ACCENT))
                            .child(detail),
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
                let line_limit = self.value_line_limit();
                let extra_height = value_extra_height(result, line_limit);
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
                    || value_preview::columns(result.graphemes(true).take(large_text_columns + 1))
                        > large_text_columns;
                let value = if compact {
                    let start = self
                        .expanded_value_pages
                        .as_ref()
                        .map_or(0, |pages| pages[self.selected]);
                    let mut preview = value_preview::lines(&result[start..]);
                    let lines: Vec<_> = preview.by_ref().take(line_limit).collect();
                    let clipped = self.expanded_value_pages.is_none() && preview.next().is_some();
                    value
                        .flex()
                        .flex_col()
                        .when(self.expanded_value_pages.is_some(), |value| {
                            value.min_h(px(extra_height + 2. * VALUE_LINE_HEIGHT))
                        })
                        .text_size(px(14.))
                        .line_height(px(VALUE_LINE_HEIGHT))
                        .children(lines.into_iter().enumerate().map(|(index, text)| {
                            let prefix = if index == 0 { "= " } else { "  " };
                            let (text, suffix) = if clipped && index + 1 == line_limit {
                                let columns = value_preview::columns(text.graphemes(true));
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
                    .h(px(38. + extra_height))
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
                div()
                    .flex()
                    .flex_col()
                    .child(row)
                    .when_some(output.continuation_hint(), |result, hint| {
                        result.child(
                            Self::selection_hint(hint)
                                .id("continue-value")
                                .cursor_pointer()
                                .hover(|style| style.text_color(rgb(theme::ACCENT)))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.continue_value(window, cx);
                                })),
                        )
                    })
                    .when(self.results.value_preview_expandable(), |result| {
                        let hint = if self.expanded_value_pages.is_some() {
                            "F2 收起预览"
                        } else {
                            "F2 展开预览"
                        };
                        let page_count = self.expanded_value_pages.as_ref().map_or(0, Vec::len);
                        result.child(
                            Self::result_footer()
                                .flex()
                                .items_center()
                                .gap(px(10.))
                                .child(
                                    div()
                                        .id("toggle-value-preview")
                                        .child(hint)
                                        .cursor_pointer()
                                        .hover(|style| style.text_color(rgb(theme::ACCENT)))
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.toggle_value_preview(window, cx);
                                        })),
                                )
                                .when(page_count > 1, |footer| {
                                    footer
                                        .child("PgUp PgDn 翻页")
                                        .child(div().flex_1())
                                        .child(self.page_controls(page_count, 1, cx))
                                }),
                        )
                    })
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
            Results::FileOptions(items) if items.is_empty() => row
                .text_color(rgb(theme::TEXT_MUTED))
                .child("没有可追加的选项"),
            Results::FileOptions(items) => {
                self.render_input_suggestions(items, Icon::Layers, EXAMPLE_PAGE_SIZE, cx)
            }
            Results::ApplicationsLoading => row
                .text_color(rgb(theme::TEXT_MUTED))
                .child("正在读取 Windows 应用"),
            Results::OpenWindowsLoading => row
                .text_color(rgb(theme::TEXT_MUTED))
                .child("正在读取当前窗口"),
            Results::LocalFilesPrompt => row
                .text_color(rgb(theme::TEXT_MUTED))
                .child(r"输入目录和名称，例如 文件: C:\工作 | 报告 | 仅文件 最近修改"),
            Results::DirectoryInput(item) => {
                self.render_input_suggestions(std::slice::from_ref(item), Icon::Layers, 1, cx)
            }
            Results::LocalFilesLoading => {
                row.text_color(rgb(theme::TEXT_MUTED)).child("正在读取目录")
            }
            Results::ApplicationsUnavailable(error)
            | Results::OpenWindowsUnavailable(error)
            | Results::LocalFilesUnavailable(error) => row
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
                                Destination::KnownFolder(folder) => {
                                    (Icon::Layers, folder.title.to_owned(), None)
                                }
                                Destination::WindowsSetting { title, .. } => (
                                    Page::Settings.icon(),
                                    (*title).to_owned(),
                                    Some("Windows 设置".to_owned()),
                                ),
                                Destination::LocalFile {
                                    name,
                                    is_directory,
                                    size_bytes,
                                    ..
                                } => (
                                    Icon::Layers,
                                    name.clone(),
                                    size_bytes
                                        .map(|bytes| format!("{bytes} 字节"))
                                        .or_else(|| is_directory.then(|| "文件夹".to_owned())),
                                ),
                                Destination::OpenWindow(window) => {
                                    (Icon::Grid, window.title.clone(), window.process.clone())
                                }
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
                                            Destination::OpenWindow(_) => "Enter 切换",
                                            Destination::LocalFile {
                                                is_directory: true, ..
                                            } => "Tab 进入 · Enter 复制路径",
                                            Destination::LocalFile { .. } => "Enter 复制路径",
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
                                        .when(
                                            matches!(&destination, Destination::LocalFile { .. }),
                                            |row| {
                                                row.child(
                                                    button(
                                                        "search-reveal-file",
                                                        "定位",
                                                        Tone::Ghost,
                                                        true,
                                                    )
                                                    .h(px(25.))
                                                    .px(px(6.))
                                                    .flex_none()
                                                    .tooltip(theme::text_tooltip(
                                                        "Alt+Enter 在资源管理器中定位".into(),
                                                    ))
                                                    .on_click(cx.listener(|this, _, window, cx| {
                                                        cx.stop_propagation();
                                                        this.reveal_selected_file(window, cx);
                                                    })),
                                                )
                                            },
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
                        "↑ ↓ 选择  ·  F1 或 ? 查看用法"
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
    fn calculates_multiline_expressions_and_shows_the_failed_line() {
        assert!(matches!(
            for_query("批量计算: 2**8\n \n3^3\nsqrt(81)", &[]),
            Results::Value(Ok(value)) if value.text == "256\n27\n9"
        ));
        assert!(matches!(
            for_query("batch calculation: 2^10\n1/0", &[]),
            Results::Value(Err(error)) if error.message().as_ref() == "第 2 行：不能除以零"
        ));
        assert!(matches!(
            for_query("批量计算: 分数 1/3+1/6\n2**80", &[]),
            Results::Value(Ok(value)) if value.text == "1/2\n1208925819614629174706176"
        ));
        assert!(matches!(
            for_query("batch calculation: exact 1/2\n1/0", &[]),
            Results::Value(Err(ValueError::BatchFraction(BatchError::Row {
                line: 2,
                cause: FractionError::ZeroDenominator,
            })))
        ));
    }

    #[test]
    fn exports_batch_expressions_with_their_results_as_two_csv_columns() {
        assert!(matches!(
            for_query("批量计算: 对照 2**10\r\n \r\navg(1,3)", &[]),
            Results::Value(Ok(value)) if value.text == "算式,结果\r\n2**10,1024\r\n\"avg(1,3)\",2"
        ));
        assert!(matches!(
            for_query("batch calculation: exact-csv 1/3 + 1/6\n2**80", &[]),
            Results::Value(Ok(value))
                if value.text == "算式,结果\r\n1/3 + 1/6,1/2\r\n2**80,1208925819614629174706176"
        ));
        assert!(matches!(
            for_query("批量计算: 分数对照 1/2\n\n1/0", &[]),
            Results::Value(Err(ValueError::BatchFraction(BatchError::Row {
                line: 3,
                ..
            })))
        ));
    }

    #[test]
    fn exports_unit_and_radix_conversions_in_a_two_column_csv() {
        assert!(matches!(
            for_query("批量换算: 对照 3 m to cm\r\n0xff to dec\n1 GiB to MiB", &[]),
            Results::Value(Ok(value))
                if value.text == "输入,结果\r\n3 m to cm,300 cm\r\n0xff to dec,255\r\n1 GiB to MiB,1024 MiB"
        ));
        assert!(matches!(
            for_query("批量换算: csv 3 m to cm\n\n未知", &[]),
            Results::Value(Err(ValueError::BatchConversion(BatchError::Row {
                line: 3,
                cause: ConversionError::Incomplete
            })))
        ));
        assert_eq!(
            for_query("批量换算: 对", &[]).suggested_input(0),
            Some("批量换算: 对照 3 m to cm\n0xff to dec\n1 GiB to MiB")
        );
    }

    #[test]
    fn converts_duration_notations_line_by_line_and_identifies_failures() {
        assert!(matches!(
            for_query("批量换算: 时长 01:30 to s\r\nPT1M30S to min\n1天2.5秒 to iso", &[]),
            Results::Value(Ok(value)) if value.text == "90 s\n1.5 min\nP1DT2.5S"
        ));
        assert!(matches!(
            for_query("批量换算: duration 01:30 to s\n\n0.5ns", &[]),
            Results::Value(Err(ValueError::BatchDuration(BatchError::Row {
                line: 3,
                cause: DurationError::Precision,
            })))
        ));
        assert!(for_query("批量换算: 时", &[])
            .suggested_input(0)
            .is_some_and(|query| query.starts_with("批量换算: 时长 ")));
    }

    #[test]
    fn exports_duration_inputs_and_results_as_csv_with_original_error_line_numbers() {
        assert!(matches!(
            for_query(
                "批量换算: 时长对照 扣除休息 23:45 到 01:15 | 30分钟\r\n平均 1min,3min to min",
                &[]
            ),
            Results::Value(Ok(value))
                if value.text == concat!(
                    "输入,结果\r\n",
                    "扣除休息 23:45 到 01:15 | 30分钟,01:00:00\r\n",
                    "\"平均 1min,3min to min\",2 min"
                )
        ));
        assert!(matches!(
            for_query(
                "批量换算: duration-csv 1s to ns\n\n扣除休息 08:00 到 08:15 | 30min",
                &[]
            ),
            Results::Value(Err(ValueError::BatchDuration(BatchError::Row {
                line: 3,
                cause: DurationError::BreakOutOfRange
            })))
        ));
        assert_eq!(
            for_query("批量换算: 时长对", &[]).suggested_input(0),
            Some("批量换算: 时长对照 扣除休息 23:45 到 01:15 | 30分钟\n08:00 到 10:15 to min")
        );
    }

    #[test]
    fn exports_timestamp_queries_and_results_with_one_batch_reference_time() {
        assert!(matches!(
            for_query(
                "批量换算: 时间戳对照 0 to UTC+8\n1970-01-01T08:00:00+08:00",
                &[]
            ),
            Results::Value(Ok(value)) if value.text
                == "输入,结果\r\n0 to UTC+8,1970-01-01T08:00:00+08:00\r\n1970-01-01T08:00:00+08:00,0"
        ));
        let Results::Value(Ok(value)) = for_query("批量换算: timestamp-csv 现在 ns\nnow ns", &[])
        else {
            panic!("缺少时间戳对照结果");
        };
        let results: Vec<_> = value
            .text
            .lines()
            .skip(1)
            .map(|line| line.rsplit_once(',').unwrap().1)
            .collect();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0], results[1]);
    }

    #[test]
    fn converts_timestamps_line_by_line_with_a_shared_reference_time() {
        assert!(matches!(
            for_query("批量换算: 时间戳 1700000000 to Asia/Shanghai\r\n1970-01-01T08:00:00+08:00", &[]),
            Results::Value(Ok(value)) if value.text == "2023-11-15T06:13:20+08:00\n0"
        ));
        let Results::Value(Ok(now)) = for_query("批量换算: timestamp 现在 ns\nnow ns", &[])
        else {
            panic!("应返回两行参考时间");
        };
        let (first, second) = now.text.split_once('\n').unwrap();
        assert_eq!(first, second);
        assert!(matches!(
            for_query("批量换算: timestamp 0\n\n0 to Asia/Unknown", &[]),
            Results::Value(Err(ValueError::BatchTimestamp(BatchError::Row {
                line: 3,
                cause: TimestampError::InvalidOffset,
            })))
        ));
        assert!(for_query("批量换算: 时间", &[])
            .suggested_input(0)
            .is_some_and(|query| query.starts_with("批量换算: 时间戳 ")));
    }

    #[test]
    fn quick_search_batches_calendar_operations_and_reports_input_lines() {
        assert!(matches!(
            for_query(
                "批量换算: 日期 2026-11-20 + 1工作日\r\n2024-02-29 + 1年\n2027-01-01 第几周",
                &[],
            ),
            Results::Value(Ok(value)) if value.text == "2026-11-23\n2025-02-28\n2026-W53"
        ));
        assert!(matches!(
            for_query(
                "批量换算: date 2026-11-20 + 1工作日\n\n2026-02-30 第几周",
                &[]
            ),
            Results::Value(Err(ValueError::BatchDate(BatchError::Row { line: 3, .. })))
        ));
        assert!(for_query("批量换算: 日", &[])
            .suggested_input(0)
            .is_some_and(|query| query.starts_with("批量换算: 日期 ")));
    }

    #[test]
    fn exports_batch_date_queries_and_results_as_csv() {
        assert!(matches!(
            for_query(
                "批量换算: 日期对照 2024-02-29 + 1年\r\n2027-01-01 第几周",
                &[]
            ),
            Results::Value(Ok(value))
                if value.text == "输入,结果\r\n2024-02-29 + 1年,2025-02-28\r\n2027-01-01 第几周,2026-W53"
        ));
        assert!(matches!(
            for_query("批量换算: date-csv 2027-01-01 第几周\n\n2024-02-30", &[]),
            Results::Value(Err(ValueError::BatchDate(BatchError::Row { line: 3, .. })))
        ));
        assert_eq!(
            for_query("批量换算: 日期对", &[]).suggested_input(0),
            Some("批量换算: 日期对照 2024-02-29 + 1年\n2027-01-01 第几周")
        );
    }

    #[test]
    fn exports_url_transformations_and_original_inputs_as_csv() {
        assert!(matches!(
            for_query(
                "批量换算: 网址对照 去查询串 https://example.invalid/doc?q=a#part\n表单解码 a%2Bb+c",
                &[]
            ),
            Results::Value(Ok(value)) if value.text == concat!(
                "输入,结果\r\n",
                "去查询串 https://example.invalid/doc?q=a#part,https://example.invalid/doc#part\r\n",
                "表单解码 a%2Bb+c,a+b c"
            )
        ));
        assert_eq!(
            for_query("批量换算: 网址对", &[]).suggested_input(0),
            Some("批量换算: 网址对照 去查询串 https://example.invalid/doc?q=a#part\n表单解码 a%2Bb+c")
        );
    }

    #[test]
    fn quick_search_batches_different_url_operations_and_reports_the_input_line() {
        assert!(matches!(
            for_query(
                "批量换算: 网址 去查询串 https://example.invalid/doc?q=a#part\r\n表单解码 a%2Bb+c\n源 https://example.invalid:8443/doc",
                &[],
            ),
            Results::Value(Ok(value))
                if value.text == "https://example.invalid/doc#part\na+b c\nhttps://example.invalid:8443"
        ));
        assert!(matches!(
            for_query("批量换算: URL 表单编码 茶\n\n表单解码 %GG", &[]),
            Results::Value(Err(ValueError::BatchUrl(BatchError::Row {
                line: 3,
                cause: UrlEncodingError::InvalidEscape,
            })))
        ));
        assert!(for_query("批量换算: 网", &[])
            .suggested_input(0)
            .is_some_and(|query| query.starts_with("批量换算: 网址 ")));
    }

    #[test]
    #[cfg(windows)]
    fn exports_path_results_and_quotes_the_original_inputs_as_csv() {
        assert!(matches!(
            for_query(
                "批量换算: 路径对照 文件名 \"C:\\工作\\报告 集.txt\"\n转文件网址 C:\\docs\\report 2026.txt",
                &[]
            ),
            Results::Value(Ok(value)) if value.text == concat!(
                "输入,结果\r\n",
                "\"文件名 \"\"C:\\工作\\报告 集.txt\"\"\",报告 集.txt\r\n",
                "转文件网址 C:\\docs\\report 2026.txt,file:///C:/docs/report%202026.txt"
            )
        ));
    }

    #[test]
    #[cfg(windows)]
    fn transforms_multiple_path_queries_and_reports_invalid_input_with_a_line_number() {
        assert!(matches!(
            for_query(
                "批量换算: 路径 文件名 \"C:\\工作\\报告 集.txt\"\r\n转文件网址 C:\\docs\\report 2026.txt\n扩展名 C:\\资料\\图.png",
                &[],
            ),
            Results::Value(Ok(value)) if value.text == "报告 集.txt\nfile:///C:/docs/report%202026.txt\npng"
        ));
        assert!(matches!(
            for_query("批量换算: path 文件名 C:\\ok.txt\n\n转文件网址 对照", &[]),
            Results::Value(Err(ValueError::BatchPath(BatchError::Row {
                line: 3,
                cause: PathError::AbsolutePathRequired,
            })))
        ));
        assert!(for_query("批量换算: 路", &[])
            .suggested_input(0)
            .is_some_and(|query| query.starts_with("批量换算: 路径 ")));
    }

    #[test]
    fn csv_queries_preserve_significant_whitespace_and_suggest_operations() {
        assert!(matches!(
            for_query(" CSV： 转JSON  001,茶\n002,末尾 ", &[]),
            Results::Value(Ok(value)) if value.text == r#"[[" 001","茶"],["002","末尾 "]]"#
        ));
        assert!(matches!(
            for_query("CSV: 从TSV \t茶\t", &[]),
            Results::Value(Ok(value)) if value.text == r#""",茶,"""#
        ));
        let suggestions = for_query("CSV: 对", &[]);
        assert!(suggestions
            .suggested_input(0)
            .is_some_and(|query| query.starts_with("CSV: 对象 ")));
    }

    #[test]
    fn url_decoding_previews_each_line_and_preserves_the_copy_value() {
        let query = "URL: 解码 %E5%8C%97%E4%BA%AC%0D%0A%E4%B8%8A%E6%B5%B7%0A%E5%B9%BF%E5%B7%9E%0A%E6%B7%B1%E5%9C%B3";
        let results = for_query(query, &[]);
        assert_eq!(
            results.height(MAX_VALUE_LINES),
            158. + 2. * VALUE_LINE_HEIGHT
        );
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
    fn expands_long_value_previews_without_changing_the_copied_text() {
        let results = for_query(r#"文本: 拆分成行 "," 一,二,三,四,五,六,七,八,九"#, &[]);
        assert!(results.value_preview_expandable());
        assert_eq!(
            results.height(MAX_VALUE_LINES),
            158. + 4. * VALUE_LINE_HEIGHT + 24.
        );
        assert_eq!(
            results.height(EXPANDED_VALUE_LINES),
            158. + 7. * VALUE_LINE_HEIGHT + 24.
        );
        assert!(matches!(
            results,
            Results::Value(Ok(output)) if output.text == "一\n二\n三\n四\n五\n六\n七\n八\n九"
        ));
        assert!(!for_query("2**10", &[]).value_preview_expandable());
    }

    #[test]
    fn conversion_suggestions_support_symbol_prefixes_and_later_pages() {
        let results = for_query("10 km to", &[]);
        assert_eq!(
            results.height(MAX_VALUE_LINES),
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
            ("ＪＳＯＮ", "JSON:"),
            ("ｔｅｘｔ　ｔｏｏｌｓ", "文本:"),
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
    fn accepts_fullwidth_tool_prefixes_and_preserves_the_payload() {
        let Results::Value(Ok(value)) = for_query(
            r#"ＪＳＯＮ： 压缩 { "名称": "ＡＢＣ：１２３", "金额": 1.00 }"#,
            &[],
        ) else {
            panic!("未识别全角 JSON 工具前缀");
        };
        assert_eq!(value.text, r#"{"名称":"ＡＢＣ：１２３","金额":1.00}"#);
        assert!(matches!(
            for_query("ＣｓＶ： 选列", &[]),
            Results::Help(items) if items.iter().any(|&index| {
                EXAMPLES[index].query.starts_with("CSV: 选列 ")
            })
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
        assert_eq!(
            examples_for("JSON："),
            EXAMPLES
                .iter()
                .map(|example| example.query)
                .filter(|query| query.starts_with("JSON: "))
                .collect::<Vec<_>>()
        );
        assert!(examples_for("JSON: 展平")[0].starts_with("JSON: 展平 "));
        assert!(examples_for("JSON: 取")[0].starts_with("JSON: 取值 "));
        assert!(examples_for("JSON: 取列")[0].starts_with("JSON: 取列 "));
        assert!(examples_for("JSON: 取多")[0].starts_with("JSON: 取多路径 "));
        assert!(examples_for("JSON: 键")[0].starts_with("JSON: 键名 "));
        assert!(examples_for("JSON: 键排")[0].starts_with("JSON: 键排序 "));
        assert!(examples_for("JSON: 长")[0].starts_with("JSON: 长度 "));
        assert!(examples_for("JSON: 展")[0].starts_with("JSON: 展开数组 "));
        assert!(examples_for("JSON: 合")[0].starts_with("JSON: 合并行 "));
        assert!(examples_for("JSON: 校验")[0].starts_with("JSON: 校验行 "));
        assert!(examples_for("JSON: 合并对")[0].starts_with("JSON: 合并对象 "));
        assert!(examples_for("JSON: 合并子")[0].starts_with("JSON: 合并子数组 "));
        assert!(examples_for("JSON: 改")[0].starts_with("JSON: 改键 "));
        assert!(examples_for("JSON: 改值")[0].starts_with("JSON: 改值 "));
        assert!(examples_for("JSON: 加路")[0].starts_with("JSON: 加路径 "));
        assert!(examples_for("JSON: 复制路")[0].starts_with("JSON: 复制路径 "));
        assert!(examples_for("JSON: 移动路")[0].starts_with("JSON: 移动路径 "));
        assert!(examples_for("JSON: 应用补")[0].starts_with("JSON: 应用补丁 "));
        assert!(examples_for("JSON: 对比")[0].starts_with("JSON: 对比 "));
        assert!(examples_for("JSON: 删路")[0].starts_with("JSON: 删路径 "));
        assert!(examples_for("JSON: 去")[0].starts_with("JSON: 去键 "));
        assert!(examples_for("JSON: 去注")[0].starts_with("JSON: 去注释 "));
        assert!(examples_for("JSON: 转")[0].starts_with("JSON: 转CSV "));
        assert!(examples_for("JSON: 路径转")[0].starts_with("JSON: 路径转CSV "));
        assert!(examples_for("JSON: 选")[0].starts_with("JSON: 选键 "));
        assert!(examples_for("JSON: 生成")[0].starts_with("JSON: 生成指针 "));
        assert!(examples_for("JSON: 解析")[0].starts_with("JSON: 解析指针 "));
        assert!(examples_for("JSON: 列")[0].starts_with("JSON: 列指针 "));
        assert!(examples_for("JSON: 有")[0].starts_with("JSON: 有路径 "));
        assert!(examples_for("JSON: 缺")[0].starts_with("JSON: 缺路径 "));
        assert!(examples_for("JSON: 筛")[0].starts_with("JSON: 筛选 "));
        assert!(examples_for("JSON: 排")[0].starts_with("JSON: 排除 "));
        assert!(examples_for("JSON: 按")[0].starts_with("JSON: 按值去重 "));
        assert!(examples_for("JSON: 分组")[0].starts_with("JSON: 分组计数 "));
        assert!(examples_for("JSON: 分组明")[0].starts_with("JSON: 分组明细 "));
        assert!(examples_for("JSON: 分组统")[0].starts_with("JSON: 分组统计 "));
        assert!(examples_for("统计: 移动")[0].starts_with("统计: 移动平均 "));
        assert!(examples_for("分数: 循环")[0].starts_with("分数: 循环小数 "));
        assert!(examples_for("分数: 取值")[0].starts_with("分数: 取值表 "));
        assert!(examples_for("分数: 连")[0].starts_with("分数: 连分数 "));
        assert!(examples_for("分数: 比")[0].starts_with("分数: 比较 "));
        for (query, expected) in [
            ("分数: 比较 1/3 < 0.34", "成立"),
            ("分数: 比较 1/3 > 0.34", "不成立"),
        ] {
            let result = for_query(query, &[]);
            assert!(matches!(
                &result,
                Results::Value(Ok(value)) if value.text == expected && value.continuation_hint().is_none()
            ));
            assert!(result.continuation_query().is_none());
        }
        assert!(matches!(
            for_query("分数: 连分数 355/113", &[]),
            Results::Value(Ok(value)) if value.text == "[3,7,16]" && value.continuation_hint().is_none()
        ));
        assert!(matches!(
            for_query("分数: 取值表 [0,1] 1/3+x", &[]),
            Results::Value(Ok(value)) if value.text == "x\ty\n0\t1/3\n1\t4/3" && value.continuation_hint().is_none()
        ));
        assert!(matches!(
            for_query("分数: 循环小数 1/3", &[]),
            Results::Value(Ok(value)) if value.text == "0.(3)" && value.continuation_hint().is_none()
        ));
        assert!(examples_for("JSON: 数值筛")[0].starts_with("JSON: 数值筛选 "));
        assert!(examples_for("JSON: 多条件")[0].starts_with("JSON: 多条件筛选 "));
        assert!(examples_for("JSON: 多路径")[0].starts_with("JSON: 多路径排序 "));
        assert!(examples_for("JSON: 多路径去")[0].starts_with("JSON: 多路径去重 "));
        assert!(examples_for("JSON: 数值排")[0].starts_with("JSON: 数值排序 "));
        assert!(examples_for("JSON: 文本排")[0].starts_with("JSON: 文本排序 "));
        assert!(examples_for("CSV: 纵向")[0].starts_with("CSV: 纵向合并 "));
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

    #[cfg(target_os = "windows")]
    fn open_test_search(index: SearchIndex, cx: &mut App) -> WindowHandle<QuickSearch> {
        let results = index.results("");
        cx.open_window(
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
        .unwrap()
    }

    #[test]
    #[cfg(target_os = "windows")]
    #[ignore = "需要 Windows 桌面会话"]
    fn search_shortcuts_handle_text_and_page_navigation() {
        const PARENT: &str = r"文件: C:\__omnitrigger_quick_search_test__ | child | 仅文件夹";
        const CHILD: &str = r"文件: C:\__omnitrigger_quick_search_test__\child |";
        const FILE_QUERY: &str = r"文件: C:\__omnitrigger_quick_search_test__ | child | 大小";
        const FILE_HELP: &str = r"文件: C:\__omnitrigger_quick_search_test__ | child | 大小 ?";
        let (send, receive) = std::sync::mpsc::channel();
        gpui::Application::new()
            .with_assets(crate::Assets)
            .run(move |cx| {
                let index = SearchIndex::default();
                index.recent.last_input.borrow_mut().push_str("2**10");
                let recent = index.recent.clone();
                let window = open_test_search(index, cx);
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
                            ("ctrl-r", false),
                            ("ctrl-r", true),
                            ("ctrl-r", false),
                            ("f1", true),
                            ("shift-f1", false),
                            ("f1", false),
                            ("f1", true),
                            ("f1", false),
                            ("ctrl-shift-r", true),
                            ("ctrl-shift-r", false),
                            ("ctrl-shift-r", true),
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
                    window
                        .update(cx, |search, window, cx| {
                            search.fill_query("3**4", window, cx);
                        })
                        .unwrap();
                    window
                        .update(cx, |search, window, cx| {
                            let event = KeyDownEvent {
                                keystroke: gpui::Keystroke::parse("escape").unwrap(),
                                is_held: false,
                            };
                            search.key_down(&event, window, cx);
                        })
                        .unwrap();
                    let reopened = cx
                        .update(|cx| {
                            open_test_search(
                                SearchIndex {
                                    recent,
                                    ..Default::default()
                                },
                                cx,
                            )
                        })
                        .unwrap();
                    let reopened_states = reopened
                        .update(cx, |search, window, cx| {
                            let before = search.input.read(cx).text();
                            let event = KeyDownEvent {
                                keystroke: gpui::Keystroke::parse("ctrl-shift-r").unwrap(),
                                is_held: false,
                            };
                            search.key_down(&event, window, cx);
                            [before, search.input.read(cx).text()]
                        })
                        .unwrap();
                    let page_states = visit_search_pages(reopened, cx);
                    let window_refresh = reopened
                        .update(cx, |search, window, cx| {
                            search.index.open_windows = OpenWindowsIndex::Ready(Vec::new());
                            search.fill_query("窗口: 测试", window, cx);
                            search.refresh_results("窗口: 测试", window, cx);
                            let event = KeyDownEvent {
                                keystroke: gpui::Keystroke::parse("f5").unwrap(),
                                is_held: false,
                            };
                            let held = KeyDownEvent {
                                keystroke: event.keystroke.clone(),
                                is_held: true,
                            };
                            search.key_down(&held, window, cx);
                            let ready =
                                matches!(search.index.open_windows, OpenWindowsIndex::Ready(_));
                            search.key_down(&event, window, cx);
                            let loading = matches!(
                                search.index.open_windows,
                                OpenWindowsIndex::Refreshing(_)
                            ) && matches!(&search.results, Results::Destinations(items) if items.is_empty());
                            search.key_down(&event, window, cx);
                            (
                                ready,
                                loading,
                                matches!(search.index.open_windows, OpenWindowsIndex::Refreshing(_)),
                            )
                        })
                        .unwrap();
                    let app_refresh = reopened
                        .update(cx, |search, window, cx| {
                            search.index.applications = ApplicationIndex::Ready {
                                apps: Vec::new().into(),
                                loaded_at: std::time::Instant::now(),
                            };
                            search.application_refresh = ApplicationRefresh::Finished;
                            search.fill_query("应用: 测试", window, cx);
                            let mut event = KeyDownEvent {
                                keystroke: gpui::Keystroke::parse("f5").unwrap(),
                                is_held: true,
                            };
                            search.key_down(&event, window, cx);
                            let finished =
                                search.application_refresh == ApplicationRefresh::Finished;
                            event.is_held = false;
                            search.key_down(&event, window, cx);
                            let loading = search.application_refresh == ApplicationRefresh::Loading;
                            search.key_down(&event, window, cx);
                            (
                                finished,
                                loading,
                                search.application_refresh == ApplicationRefresh::Loading,
                                matches!(search.index.applications, ApplicationIndex::Ready { .. }),
                            )
                        })
                        .unwrap();
                    let file_navigation = reopened
                        .update(cx, |search, window, cx| {
                            search.fill_query(PARENT, window, cx);
                            search.results = Results::Destinations(vec![Destination::LocalFile {
                                name: "child".to_owned(),
                                path: r"C:\__omnitrigger_quick_search_test__\child".to_owned(),
                                is_directory: true,
                                size_bytes: None,
                            }]);
                            search.selected = 0;
                            [
                                ("tab", true),
                                ("tab", false),
                                ("alt-left", true),
                                ("alt-left", false),
                                ("alt-right", true),
                                ("alt-right", false),
                            ]
                            .map(|(key, is_held)| {
                                let event = KeyDownEvent {
                                    keystroke: gpui::Keystroke::parse(key).unwrap(),
                                    is_held,
                                };
                                search.key_down(&event, window, cx);
                                search.input.read(cx).text()
                            })
                        })
                        .unwrap();
                    let file_help = reopened
                        .update(cx, |search, window, cx| {
                            search.fill_query(FILE_QUERY, window, cx);
                            search.refresh_results(FILE_QUERY, window, cx);
                            [
                                ("f1", true),
                                ("f1", false),
                                ("f1", true),
                                ("f1", false),
                                ("f1", false),
                                ("down", false),
                                ("tab", false),
                            ]
                            .map(|(key, is_held)| {
                                let event = KeyDownEvent {
                                    keystroke: gpui::Keystroke::parse(key).unwrap(),
                                    is_held,
                                };
                                search.key_down(&event, window, cx);
                                let input = search.input.read(cx).text();
                                search.refresh_results(&input, window, cx);
                                input
                            })
                        })
                        .unwrap();
                    send.send((
                        states,
                        reopened_states,
                        page_states,
                        window_refresh,
                        app_refresh,
                        file_navigation,
                        file_help,
                    ))
                    .unwrap();
                    cx.update(|cx| cx.quit()).unwrap();
                })
                .detach();
            });
        let (
            states,
            reopened_states,
            (help_pages, output_pages),
            window_refresh,
            app_refresh,
            file_navigation,
            file_help,
        ) = receive.recv().unwrap();
        assert_eq!(reopened_states, ["", "3**4"]);
        assert_eq!(window_refresh, (true, true, true));
        assert_eq!(app_refresh, (true, true, true, true));
        assert_eq!(
            file_navigation,
            [PARENT, CHILD, CHILD, PARENT, PARENT, CHILD]
        );
        assert_eq!(
            file_help,
            [
                FILE_QUERY,
                FILE_HELP,
                FILE_HELP,
                FILE_QUERY,
                FILE_HELP,
                FILE_HELP,
                r"文件: C:\__omnitrigger_quick_search_test__ | child | 大小 仅文件夹",
            ]
        );
        let last_example_page = (EXAMPLES.len() - 1) / EXAMPLE_PAGE_SIZE * EXAMPLE_PAGE_SIZE;
        assert_eq!(
            help_pages,
            [last_example_page, last_example_page - EXAMPLE_PAGE_SIZE, 0]
        );
        assert_eq!(output_pages, [3, 2, 0]);
        assert_eq!(
            states.unwrap(),
            [
                "换行验证甲",
                "换行验证甲",
                "换行验证甲\n",
                "换行验证甲\n",
                "换行验证甲\n",
                "算式历史: 换行验证甲\n",
                "算式历史: 换行验证甲\n",
                "换行验证甲\n",
                "换行验证甲\n",
                "换行验证甲\n",
                "?",
                "?",
                "换行验证甲\n",
                "换行验证甲\n",
                "2**10",
                "2**10",
            ]
        );
    }

    #[cfg(target_os = "windows")]
    fn visit_search_pages(
        handle: WindowHandle<QuickSearch>,
        cx: &mut gpui::AsyncApp,
    ) -> ([usize; 3], [usize; 3]) {
        let visit_pages =
            |search: &mut QuickSearch, window: &mut Window, cx: &mut Context<QuickSearch>| {
                ["ctrl-pagedown", "pageup", "ctrl-pageup"].map(|key| {
                    search.key_down(
                        &KeyDownEvent {
                            keystroke: gpui::Keystroke::parse(key).unwrap(),
                            is_held: false,
                        },
                        window,
                        cx,
                    );
                    search.selected
                })
            };
        handle
            .update(cx, |search, window, cx| search.fill_query("?", window, cx))
            .unwrap();
        let help = handle.update(cx, visit_pages).unwrap();
        handle
            .update(cx, |search, window, cx| {
                let lines = vec!["样例"; 40].join("\n");
                search.fill_query(format!("文本: 按行编号 {lines}"), window, cx);
            })
            .unwrap();
        let output = handle
            .update(cx, |search, window, cx| {
                search.toggle_value_preview(window, cx);
                visit_pages(search, window, cx)
            })
            .unwrap();
        (help, output)
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
    fn offers_function_templates_for_partial_names_and_empty_calls() {
        for query in ["sqr", "sqrt", "sqrt(", "ＳＱＲＴ（"] {
            assert_eq!(
                for_query(query, &[]).suggested_input(0),
                Some("sqrt(2)"),
                "{query}"
            );
        }
        assert_eq!(for_query("sin", &[]).suggested_input(0), Some("sin(pi/2)"));
        assert!(
            matches!(for_query("sqrt(9)", &[]), Results::Value(Ok(value)) if value.text == "3")
        );
        assert!(for_query("sq(", &[]).suggested_input(0).is_none());
    }

    #[test]
    fn tool_index_shows_one_representative_example_per_matching_tool() {
        let Results::Help(items) = for_query("工具：", &[]) else {
            panic!("未打开工具目录");
        };
        let mut scopes = Vec::new();
        for index in items {
            let (scope, _) = scoped_query(EXAMPLES[index].query).expect("应有工具前缀");
            assert!(!scopes.contains(&scope));
            scopes.push(scope);
        }
        for scope in [Scope::Csv, Scope::Vector, Scope::Path, Scope::Date] {
            assert!(scopes.contains(&scope), "{scope:?}");
        }
        let result = for_query("tools: 叉积", &[]);
        assert_eq!(
            result.suggested_input(0),
            Some("向量: 叉积 1/2,0,0; 0,1/3,0")
        );
        assert_eq!(result.suggested_input(1), None);
        let alias = for_query("tools: ＶＥＣＴＯＲ", &[]);
        assert_eq!(alias.suggested_input(0), Some("向量: 点积 1,2,3; 4,5,6"));
        assert_eq!(alias.suggested_input(1), None);
    }

    #[test]
    fn quick_search_help_shows_only_supported_examples() {
        for query in ["?", " ？ "] {
            let results = for_query(query, &[]);
            assert_eq!(
                results.height(MAX_VALUE_LINES),
                132. + EXAMPLE_PAGE_SIZE as f32 * EXAMPLE_ROW_HEIGHT
            );
            assert!(matches!(results, Results::Help(items) if items.len() == EXAMPLES.len()));
        }
        let unix_examples = [
            "unix: now",
            "时间戳: 1700000000123ms",
            "时间戳: 间隔 -1ns 到 1ns to ns",
        ];
        for (query, expected) in [
            ("？ 速率", &["下载: 600 MB 用时 2分钟", "8Mbps to MB/s"][..]),
            ("? UNIX", &unix_examples[..]),
            ("？ ＵＮＩＸ", &unix_examples[..]),
            (
                "？ ２＊＊１０",
                &[
                    "2**10",
                    "(2**10)B to KiB",
                    "批量计算: 对照 2**10\navg(1,3)",
                    "分数: 计算 isqrt(2**100 - 1)",
                ][..],
            ),
            (
                "? 时长",
                &[
                    "时长: 5400秒",
                    "时长: 01:30:00 - 25分钟",
                    "时长: 00:00:01.25 + 500ms",
                    "时长: 45分钟 * 12",
                    "时长: 01:30:00 to min",
                    "时长: P1DT2.5S",
                    "时长: 1天2.5秒 to iso",
                    "时长: 23:45 到 01:15",
                    "时长: 扣除休息 23:45 到 01:15 | 20分钟 + 10分钟",
                    "时长: 合计 01:25,02:40,00:55",
                    "时长: 平均 01:25,02:40,00:55",
                    "时长: 频数平均 01:00@3,02:00@1",
                    "时长: 中位数 01:25,02:40,00:55",
                    "时长: 最短 01:25,02:40,00:55",
                    "时长: 最长 01:25,02:40,00:55",
                    "时长: 极差 01:25,02:40,00:55",
                    "时长: 排序 02:40,PT2M,00:55,02:00",
                    "时长: 降序 02:40,PT2M,00:55,02:00",
                    "批量换算: 时长 01:30 to s\nPT1M30S to min\n1天2.5秒 to iso",
                    "批量换算: 时长对照 扣除休息 23:45 到 01:15 | 30分钟\n08:00 到 10:15 to min",
                    "帧数: 125 帧 @ 25 fps",
                    "帧数: 10秒 @ 30000/1001 fps",
                    "时间戳: 现在 + 90分钟 to UTC+8",
                    "时间戳: 向下对齐 15分钟 @ 1970-01-01T00:17:30Z",
                    "时间戳: 向上对齐 15分钟 @ 1970-01-01T00:17:30Z",
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
                Results::ApplicationsLoading
                | Results::OpenWindowsLoading
                | Results::LocalFilesPrompt => {}
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
    fn installed_app_search_supports_a_scope_and_fallback_after_local_results() {
        let index = SearchIndex::default();
        for query in ["应用: 计算器", "apps: calculator", "程序：记事本", "记事本"] {
            assert!(matches!(index.results(query), Results::ApplicationsLoading));
        }
        assert!(matches!(
            index.results("动作: calculator"),
            Results::Destinations(_)
        ));
        assert!(matches!(index.results("calculator"), Results::Help(_)));
        assert_eq!(index.results("2**10").arithmetic_result(), Some("1024"));
        assert!(matches!(
            index.results("文本历史"),
            Results::Destinations(items) if items == [Destination::Page(Page::TextHistory)]
        ));
        for query in ["x", "无对应前缀: 记事本", "记事本\n第二行"] {
            assert!(
                !matches!(index.results(query), Results::ApplicationsLoading),
                "{query}",
            );
        }
        let ready = SearchIndex {
            applications: ApplicationIndex::Ready {
                apps: std::sync::Arc::from([]),
                loaded_at: std::time::Instant::now(),
            },
            ..SearchIndex::default()
        };
        assert!(matches!(
            ready.results("无对应应用"),
            Results::Destinations(items) if items.is_empty()
        ));
        let unavailable = SearchIndex {
            applications: ApplicationIndex::Unavailable("读取失败".into()),
            ..SearchIndex::default()
        };
        assert!(matches!(
            unavailable.results("记事本"),
            Results::Destinations(items) if items.is_empty()
        ));
        assert!(matches!(
            unavailable.results("应用: 记事本"),
            Results::ApplicationsUnavailable(_)
        ));
    }

    #[test]
    fn open_window_catalog_requires_an_explicit_scope_and_uses_the_loaded_snapshot() {
        let index = SearchIndex::default();
        assert!(matches!(
            index.results("窗口: 浏览器"),
            Results::OpenWindowsLoading
        ));
        assert!(matches!(
            index.results("windows："),
            Results::OpenWindowsLoading
        ));
        assert!(!matches!(
            index.results("窗口"),
            Results::OpenWindowsLoading
        ));
        let loaded = SearchIndex {
            open_windows: OpenWindowsIndex::Ready(Vec::new()),
            ..index
        };
        assert!(
            matches!(loaded.results("窗口: 浏览器"), Results::Destinations(items) if items.is_empty())
        );
    }

    #[test]
    fn lists_windows_known_folders_and_filters_localized_names_and_aliases() {
        use omnitrigger_windows::ShellFolder;

        let index = SearchIndex::default();
        let Results::Destinations(folders) = index.results("文件夹:") else {
            panic!("未列出 Windows 常用文件夹");
        };
        assert_eq!(folders.len(), 6);
        assert!(matches!(
            folders.first(),
            Some(Destination::KnownFolder(folder)) if folder.kind == ShellFolder::Desktop
        ));
        for (query, expected) in [
            ("文件夹: 下载", ShellFolder::Downloads),
            ("folder: ＤＯＷＮＬＯＡＤＳ", ShellFolder::Downloads),
            ("目录: 我的文档", ShellFolder::Documents),
            ("文件夹: 图片 photos", ShellFolder::Pictures),
        ] {
            assert!(
                matches!(
                    index.results(query),
                    Results::Destinations(items)
                        if matches!(items.as_slice(), [Destination::KnownFolder(folder)]
                            if folder.kind == expected)
                ),
                "{query}"
            );
        }
        assert!(matches!(
            index.results("文件夹: 无匹配"),
            Results::Destinations(items) if items.is_empty()
        ));
    }

    #[test]
    fn searches_windows_settings_pages_by_label_and_alias() {
        use omnitrigger_windows::WindowsSettingsPage;

        for (query, page) in [
            ("系统设置: 蓝牙", WindowsSettingsPage::Bluetooth),
            (
                "windows settings: ＳＣＲＥＥＮ 分辨率",
                WindowsSettingsPage::Display,
            ),
            ("系统设置: default apps", WindowsSettingsPage::DefaultApps),
        ] {
            assert!(matches!(
                for_query(query, &[]),
                Results::Destinations(items) if matches!(
                    items.as_slice(),
                    [Destination::WindowsSetting { page: matched, .. }] if *matched == page
                )
            ));
        }
        assert!(matches!(
            for_query("系统设置:", &[]),
            Results::Destinations(items) if items.len() == 8
        ));
        assert!(matches!(
            for_query("系统设置: 现有热键", &[]),
            Results::Destinations(items) if items.is_empty()
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
            results.height(MAX_VALUE_LINES),
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
