mod actions;
mod applications;
mod aspect_ratio;
mod batch;
mod bin_plot;
mod box_plot;
mod calculator;
mod calendar;
mod cidr;
mod circle;
mod color;
mod complex;
mod conversion;
mod correlation_plot;
mod csv_tools;
mod data_pipeline;
mod decibel;
mod duration;
mod equation;
mod examples;
mod file_mode;
mod folders;
mod fraction;
mod identifiers;
mod intervals;
mod json_tools;
mod linear_fit;
mod local_files;
mod matrix;
mod open_windows;
mod operators;
mod paper;
mod path_tools;
mod percentage;
mod pitch;
mod plane;
mod polygon;
mod presets;
mod radix;
mod ratio_allocation;
mod recent;
mod rectangle;
mod renminbi;
mod scatter_plot;
mod scientific;
mod scope;
mod settings_sections;
mod shortcut_help;
mod solid;
mod statistics;
mod table_preview;
mod tempo;
mod text_tools;
mod timecode;
mod timestamp;
mod toml_tools;
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

use std::{cell::RefCell, io::Read, ops::Range, path::Path, rc::Rc, sync::Arc};

use gpui::{
    div, prelude::*, px, size, App, Bounds, ClipboardItem, Context, DisplayId, Div, Entity,
    EntityInputHandler, Focusable, FontWeight, KeyDownEvent, Stateful, Subscription,
    TitlebarOptions, WeakEntity, Window, WindowBounds, WindowHandle, WindowKind, WindowOptions,
};

use crate::appearance_ui::SettingsSection;
use crate::{
    appearance,
    assets::Icon,
    message_input::{input_preview, MessageInput, MessageSelection, OpenLineJump},
    theme::{self, button, icon, rgb, Tone},
    wheel_scenes::WheelSlotKey,
    Page, Workspace,
};
use actions::{matching_actions, SearchAction};
use applications::{ApplicationCache, ApplicationIndex, RecentApplications};
use aspect_ratio::{calculate_aspect_ratio, AspectRatioError};
use batch::{
    batch_arguments, batch_calculation_input, batch_calculation_report, batch_line_range,
    map_batch_csv, map_batch_lines, map_tool_batch, BatchCalculationKind, BatchError,
    BatchToolOutput,
};
use calculator::{calculate, calculate_with_answers, looks_like_calculation, CalculationError};
use calendar::{
    calculate_date, is_date_report, looks_like_date_query, parse_complete_date, DateError,
};
use cidr::{calculate_cidr, CidrError};
use circle::{calculate_circle, CircleError};
use color::{
    calculate_contrast, convert_color, convert_color_to_hex, hex_for_swatch, make_color_palette,
    ColorError, ColorMixSpace,
};
use complex::{calculate_complex, ComplexError};
use conversion::{
    calculate_circuit, calculate_fuel_economy, calculate_pace, calculate_pixel_density,
    calculate_unit_price, conversion_target_input, convert, convert_batch, convert_batch_csv,
    convert_exact_data, convert_map_scale, convert_period, convert_wave, estimate_transfer,
    exact_data_csv_report, suggested_targets, ConversionError,
};
use csv_tools::{transform_csv, CsvError};
use data_pipeline::{transform_data_pipeline, DataPipelineError, PipelinePreview};
use decibel::{convert_decibels, DecibelError};
use duration::{
    convert_duration, convert_exact_duration, is_clock_duration, is_duration_report, DurationError,
};
use equation::{solve_equation_output, solve_linear_system, EquationError, EquationOutput};
use examples::{
    example_help_query, fraction_function_examples, function_examples, matching_examples,
    neighboring_function_argument, query_argument_range, tool_index_examples,
    tool_operation_examples, Example, EXAMPLES,
};
use file_mode::{calculate_file_mode, FileModeError};
use folders::{matching_folders, SearchFolder};
use fraction::{calculate_fraction, FractionError};
use identifiers::{
    transform_gtin, transform_isbn, transform_semver, transform_ulid, transform_uuid,
    IdentifierError,
};
use intervals::{calculate_intervals, IntervalError};
use json_tools::{transform_json, JsonError};
use linear_fit::{calculate_linear_fit, LinearFitError};
use local_files::{
    directory_query, DirectoryLoad, LocalFilesIndex, RecentFilePreviews, FOLDER_DEBOUNCE,
};
use matrix::{calculate_matrix, MatrixError};
use omnitrigger_windows::{OpenWindow, ShellApp, ShortcutTarget};
use open_windows::OpenWindowsIndex;
use paper::{calculate_paper, PaperError};
use path_tools::{path_component, path_csv_report, PathError};
use percentage::{compare_percentages, PercentageError};
use pitch::{convert_pitch, PitchError};
use polygon::{calculate_polygon, PolygonError};
use presets::matching_presets;
use radix::{convert_arbitrary_radix, radix_csv_report, RadixError};
use ratio_allocation::{allocate_ratio, RatioAllocationError};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use recent::{QueryBookmarks, RecentDestinations, RecentExamples};
use rectangle::{calculate_rectangle, RectangleError};
use renminbi::{uppercase_renminbi, RenminbiError};
use scope::{scoped_query, tool_example_keyword, Scope};
use settings_sections::matching_settings;
use solid::{calculate_solid, SolidError};
use statistics::{calculate_statistics, StatisticsError};
use tempo::{calculate_tempo, TempoError};
use text_tools::{count_text, transform_text, TextError};
use timecode::{convert_timecode, TimecodeError};
use timestamp::{convert_timestamp, is_offset_timestamp, is_timestamp_report, TimestampError};
use toml_tools::{transform_toml, TomlError};
use triangle::{calculate_triangle, TriangleError};
use triggers::{matching_triggers, search_triggers, SearchTrigger, TriggerSearchError};
use unicode_segmentation::UnicodeSegmentation;
use url_encoding::{convert_url_component, UrlEncodingError};
use value_history::RecentValues;
use vector::{calculate_vector, VectorError};
use video_frames::{convert_video_frames, VideoFrameError};
use wheel_slots::{index_slots, matching_slots, SearchWheelSlot};
use wheels::{matching_wheels, SearchWheel};

const WIDTH: f32 = 500.;
const INPUT_CHARACTER_LIMIT: usize = 4096;
const EXACT_MODE_PREFIX: &str = "分数: 计算 ";
const PAGE_ROW_HEIGHT: f32 = 38.;
const DESTINATION_PAGE_SIZE: usize = 4;
const CONVERSION_PAGE_SIZE: usize = 4;
const EXAMPLE_ROW_HEIGHT: f32 = 30.;
const EXAMPLE_SHORTCUT_ROW_HEIGHT: f32 = 24.;
const EXAMPLE_PAGE_SIZE: usize = 8;
const MAX_VALUE_LINES: usize = 6;
const EXPANDED_VALUE_LINES: usize = 12;
const VALUE_LINE_HEIGHT: f32 = 18.;
const TABLE_PAGER_HEIGHT: f32 = 20.;
const TABLE_FOOTER_ROW_GAP: f32 = 4.;

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
    RefilledInput,
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
    Queries(Vec<InputSuggestion>),
    ConversionTargets(Vec<InputSuggestion>),
    FileOptions(Vec<InputSuggestion>),
    FilterOptions(Vec<InputSuggestion>),
    Bookmarks(Vec<InputSuggestion>),
    BookmarksUnavailable(String),
    BookmarkNameInput(Option<String>),
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

struct InputTableReport {
    csv: String,
    input_errors: Vec<Range<usize>>,
}

impl InputTableReport {
    fn offset_input_errors(mut self, offset: usize) -> Self {
        for range in &mut self.input_errors {
            range.start += offset;
            range.end += offset;
        }
        self
    }
}

struct InputValidationReport {
    text: String,
    error_span: Option<Range<usize>>,
}

impl InputValidationReport {
    fn offset_error_span(mut self, offset: usize) -> Self {
        if let Some(range) = &mut self.error_span {
            range.start += offset;
            range.end += offset;
        }
        self
    }
}

fn is_validation_operation(operation: &str) -> bool {
    operation == "校验" || operation.eq_ignore_ascii_case("validate")
}

fn input_line_column(prefix: &str) -> (usize, usize) {
    let line = prefix.bytes().filter(|&byte| byte == b'\n').count() + 1;
    let column = prefix
        .rsplit('\n')
        .next()
        .unwrap_or_default()
        .chars()
        .count()
        + 1;
    (line, column)
}

impl ResultValue {
    fn from_validation(report: InputValidationReport, scope: Scope) -> Self {
        Self {
            text: report.text,
            kind: report.error_span.map_or(ValueKind::Text, |range| {
                ValueKind::InputValidationError { scope, range }
            }),
        }
    }

    fn suggested_file_name(&self, query: &str) -> &'static str {
        use data_pipeline::DataPipelineMode;
        use json_tools::JsonResultFormat;

        if matches!(
            &self.kind,
            ValueKind::Table(_)
                | ValueKind::Scatter(_)
                | ValueKind::BinPlot(_)
                | ValueKind::CorrelationPlot(_)
        ) {
            return "搜索结果.csv";
        }
        let format = match scoped_query(query.trim_start()) {
            Some((Scope::Json, term)) => json_tools::result_format(term, &self.text),
            Some((Scope::Toml, term)) if toml_tools::produces_toml(term) => {
                return "搜索结果.toml";
            }
            Some((Scope::Toml, term)) if toml_tools::produces_json(term) => {
                Some(JsonResultFormat::Json)
            }
            Some((Scope::DataPipeline, term)) => {
                match data_pipeline::split_pipeline_input(term).0 {
                    DataPipelineMode::JsonLines => Some(JsonResultFormat::JsonLines),
                    DataPipelineMode::JsonArray | DataPipelineMode::JsonObject => {
                        Some(JsonResultFormat::Json)
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        match format {
            Some(JsonResultFormat::Csv) => "搜索结果.csv",
            Some(JsonResultFormat::Json) => "搜索结果.json",
            Some(JsonResultFormat::JsonLines) => "搜索结果.jsonl",
            None => "搜索结果.txt",
        }
    }

    fn visual_preview_height(&self) -> Option<f32> {
        match &self.kind {
            ValueKind::Table(table) => {
                let extra = if table.spans_both_axes() {
                    TABLE_PAGER_HEIGHT + TABLE_FOOTER_ROW_GAP
                } else {
                    0.
                };
                Some(table.preview_height() + extra)
            }
            ValueKind::CorrelationPlot(plot) => Some(plot.preview_height()),
            ValueKind::Scatter(_) | ValueKind::BoxPlot(_) | ValueKind::BinPlot(_) => {
                Some(scatter_plot::PREVIEW_HEIGHT)
            }
            _ => None,
        }
    }

    fn continuation_hint(&self) -> Option<&'static str> {
        match &self.kind {
            ValueKind::Calculation
            | ValueKind::ExactCalculation {
                can_continue: true, ..
            } => Some("Tab 继续计算"),
            ValueKind::Conversion(Some(_)) => Some("Tab 换目标"),
            ValueKind::CalendarDate => Some("Tab 继续日期计算"),
            ValueKind::ClockDuration => Some("Tab 继续时长计算"),
            ValueKind::TimestampInstant => Some("Tab 继续换时区"),
            ValueKind::Color(_) => Some("Tab 继续处理颜色"),
            _ => None,
        }
    }

    fn exact_decimal_preview(&self) -> Option<&fraction::DecimalPreview> {
        match &self.kind {
            ValueKind::ExactCalculation {
                decimal_preview, ..
            } => decimal_preview.as_ref(),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
enum CalculationMode {
    Decimal,
    Exact,
}

#[derive(PartialEq, Eq)]
enum ValueKind {
    Text,
    InputValidationError {
        scope: Scope,
        range: Range<usize>,
    },
    CalendarDate,
    ClockDuration,
    TimestampInstant,
    Calculation,
    ExactCalculation {
        can_continue: bool,
        decimal_preview: Option<fraction::DecimalPreview>,
    },
    Conversion(Option<String>),
    Color(u32),
    ColorContrast([u32; 2]),
    Scatter(scatter_plot::ScatterPlot),
    BoxPlot(box_plot::BoxPlot),
    BinPlot(bin_plot::BinPlot),
    CorrelationPlot(correlation_plot::CorrelationPlot),
    Table(table_preview::TablePreview),
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
struct RememberedSearchInput {
    query: String,
    selection: Option<MessageSelection>,
}

#[derive(Clone, Default)]
pub(super) struct RecentSearch {
    pub(super) destinations: RecentDestinations,
    pub(super) launched_apps: RecentApplications,
    file_previews: RecentFilePreviews,
    calculations: RecentValues,
    conversions: RecentValues,
    queries: RecentValues,
    examples: RecentExamples,
    bookmarks: QueryBookmarks,
    applications: ApplicationCache,
    last_input: Rc<RefCell<RememberedSearchInput>>,
}

impl RecentSearch {
    pub(super) fn from_bindings_path(bindings: &Path) -> Self {
        Self {
            bookmarks: QueryBookmarks::from_bindings_path(bindings),
            ..Self::default()
        }
    }
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
    TriggerSearch(TriggerSearchError),
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
    BatchColor(BatchError<ColorError>),
    BatchRadix(BatchError<RadixError>),
    BatchTool(BatchError<&'static str>),
    Radix(RadixError),
    RatioAllocation(RatioAllocationError),
    Intervals(IntervalError),
    AspectRatio(AspectRatioError),
    Paper(PaperError),
    Pitch(PitchError),
    Decibel(DecibelError),
    Solid(SolidError),
    Tempo(TempoError),
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
    Toml(TomlError),
    Csv(CsvError),
    DataPipeline(DataPipelineError),
    Fraction(FractionError),
    Equation(EquationError),
    Percentage(PercentageError),
    UrlEncoding(UrlEncodingError),
    Vector(VectorError),
    Matrix(MatrixError),
    Circle(CircleError),
    Polygon(PolygonError),
    Complex(ComplexError),
    Path(PathError),
}

impl ValueError {
    fn message(self) -> gpui::SharedString {
        let message = match self {
            Self::InputTooLong => "输入最多支持 4096 个字符",
            Self::TriggerSearch(error) => error.message(),
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
            Self::BatchColor(error) => {
                return batch_error_message(
                    error,
                    "逐行输入 HEX、RGB、HSL、HWB 或 CSS 颜色名",
                    ColorError::message,
                );
            }
            Self::BatchTool(error) => {
                return batch_error_message(error, "逐行填写要计算的输入", std::convert::identity);
            }
            Self::BatchRadix(error) => {
                return batch_error_message(
                    error,
                    "逐行输入进制换算或字节序操作，例如 ff 从 16 到 10",
                    RadixError::message,
                );
            }
            Self::Radix(error) => error.message(),
            Self::RatioAllocation(error) => error.message(),
            Self::Intervals(error) => error.message(),
            Self::AspectRatio(error) => error.message(),
            Self::Paper(error) => error.message(),
            Self::Pitch(error) => error.message(),
            Self::Decibel(error) => error.message(),
            Self::Solid(error) => error.message(),
            Self::Tempo(error) => error.message(),
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
            Self::Json(JsonError::PatchStep { index, cause }) => {
                return format!("第 {index} 步补丁失败 · {cause}").into();
            }
            Self::Csv(CsvError::CellEdit { index, cause }) => {
                return format!("第 {index} 项单元格修改失败 · {cause}").into();
            }
            Self::Json(JsonError::PipelineStep { index, cause })
            | Self::Csv(CsvError::PipelineStep { index, cause })
            | Self::Text(TextError::PipelineStep { index, cause })
            | Self::DataPipeline(DataPipelineError::Step { index, cause }) => {
                return format!("第 {index} 步失败 · {cause}").into();
            }
            Self::DataPipeline(DataPipelineError::Arguments) => {
                "填写 1 至 8 条单行命令，再粘贴要处理的数据"
            }
            Self::DataPipeline(DataPipelineError::CsvColumnArguments) => {
                "填写 [\"新列名\",源列,[\"步骤一\",\"步骤二\"]]，再粘贴含表头的 CSV"
            }
            Self::DataPipeline(DataPipelineError::Csv(error)) => error.message(),
            Self::DataPipeline(DataPipelineError::Json(error)) => error.message(),
            Self::DataPipeline(DataPipelineError::JsonLine { line, cause }) => {
                return format!("第 {line} 行 · {}", cause.message()).into();
            }
            Self::DataPipeline(DataPipelineError::CsvRowStep { row, index, cause }) => {
                return format!("第 {row} 个数据行的第 {index} 步失败 · {cause}").into();
            }
            Self::DataPipeline(DataPipelineError::CsvTableStep { index, row, cause }) => {
                return format!("第 {index} 步失败 · 第 {row} 个数据行 · {cause}").into();
            }
            Self::DataPipeline(DataPipelineError::JsonItemStep { item, index, cause }) => {
                return format!("第 {item} 个数组元素的第 {index} 步失败 · {cause}").into();
            }
            Self::DataPipeline(DataPipelineError::JsonArrayTooLarge { item }) => {
                return format!("第 {item} 个数组元素使结果超过 1 MiB，请缩小数据范围").into();
            }
            Self::DataPipeline(DataPipelineError::JsonFieldStep {
                field,
                index,
                cause,
            }) => {
                return format!("第 {field} 个对象字段的第 {index} 步失败 · {cause}").into();
            }
            Self::DataPipeline(DataPipelineError::JsonObjectTooLarge { field }) => {
                return format!("第 {field} 个对象字段使结果超过 1 MiB，请缩小数据范围").into();
            }
            Self::DataPipeline(DataPipelineError::RowStep { line, index, cause }) => {
                return format!("第 {line} 行的第 {index} 步失败 · {cause}").into();
            }
            Self::DataPipeline(DataPipelineError::RowOutputTooLarge { line }) => {
                return format!("第 {line} 行使逐行结果超过 1 MiB，请缩小数据范围").into();
            }
            Self::Text(error) => error.message(),
            Self::Csv(CsvError::DataRowValue { row, cause }) => {
                return format!("第 {row} 个数据行 · {cause}").into();
            }
            Self::Json(error) => error.message(),
            Self::Toml(TomlError::Invalid { line, column, .. }) => {
                return toml_tools::syntax_error_message(line, column).into();
            }
            Self::Toml(error) => error.message(),
            Self::Csv(error) => error.message(),
            Self::Fraction(error) => error.message(),
            Self::Equation(error) => error.message(),
            Self::Percentage(error) => error.message(),
            Self::UrlEncoding(error @ UrlEncodingError::InvalidWebAddressRow { row }) => {
                return format!("第 {row} 行 · {}", error.message()).into();
            }
            Self::UrlEncoding(error) => error.message(),
            Self::Vector(error) => error.message(),
            Self::Matrix(error) => error.message(),
            Self::Circle(error) => error.message(),
            Self::Polygon(error) => error.message(),
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
                | Self::BatchColor(BatchError::Empty)
                | Self::BatchRadix(BatchError::Empty)
                | Self::BatchTool(BatchError::Empty)
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
                | Self::Intervals(IntervalError::Incomplete)
                | Self::Triangle(
                    TriangleError::Dimensions | TriangleError::RightTriangleDimensions
                )
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
                | Self::Polygon(PolygonError::Incomplete)
                | Self::Complex(ComplexError::Incomplete)
                | Self::Path(PathError::Incomplete)
                | Self::Json(JsonError::Incomplete)
                | Self::Toml(TomlError::Incomplete)
                | Self::Csv(CsvError::MissingInput | CsvError::UnclosedQuote)
                | Self::DataPipeline(DataPipelineError::Arguments)
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
                Scope::SearchShortcuts => {
                    Self::table_value(Ok(shortcut_help::shortcut_table(term)))
                }
                Scope::ExampleHistory => Self::Help(recent.examples.matching(term)),
                Scope::Bookmarks => recent.bookmarks.load_error().map_or_else(
                    || Self::Bookmarks(recent.bookmarks.matching(term)),
                    Self::BookmarksUnavailable,
                ),
                Scope::Triggers(filter) => search_triggers(triggers, query, term, filter),
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
                    let (kind, input) = batch_calculation_input(term);
                    let exact =
                        |query: &str| calculate_fraction(query).map(|value| value.to_string());
                    match kind {
                        BatchCalculationKind::Standard => Self::text_value(
                            map_batch_lines(input, calculate).map_err(ValueError::BatchCalculation),
                        ),
                        BatchCalculationKind::StandardCsv => Self::table_value(
                            map_batch_csv(input, "算式", calculate)
                                .map_err(ValueError::BatchCalculation),
                        ),
                        BatchCalculationKind::Exact => Self::text_value(
                            map_batch_lines(input, exact).map_err(ValueError::BatchFraction),
                        ),
                        BatchCalculationKind::ExactCsv => Self::table_value(
                            map_batch_csv(input, "算式", exact).map_err(ValueError::BatchFraction),
                        ),
                        kind @ (BatchCalculationKind::StandardReport
                        | BatchCalculationKind::ExactReport) => {
                            let (report, missing_input) =
                                if matches!(kind, BatchCalculationKind::ExactReport) {
                                    (
                                        batch_calculation_report(input, |line| {
                                            exact(line).map_err(FractionError::message)
                                        }),
                                        ValueError::BatchFraction(BatchError::Empty),
                                    )
                                } else {
                                    (
                                        batch_calculation_report(input, |line| {
                                            calculate(line).map_err(CalculationError::message)
                                        }),
                                        ValueError::BatchCalculation(BatchError::Empty),
                                    )
                                };
                            Self::input_table_value(
                                report
                                    .map_err(ValueError::Csv)
                                    .and_then(|report| report.ok_or(missing_input)),
                                raw_query.trim_end().len() - input.len(),
                            )
                        }
                    }
                }
                Scope::ConversionHistory => Self::Conversions(recent.conversions.matching(term)),
                Scope::QueryHistory => Self::Queries(recent.queries.matching(term)),
                Scope::BatchConversion => {
                    if let Some(arguments) = batch_arguments(term, "对照", "csv") {
                        Self::table_value(
                            convert_batch_csv(arguments).map_err(ValueError::BatchConversion),
                        )
                    } else if let Some(arguments) =
                        batch_arguments(term, "时长对照", "duration-csv")
                    {
                        Self::table_value(
                            map_batch_csv(arguments, "输入", convert_duration)
                                .map_err(ValueError::BatchDuration),
                        )
                    } else if let Some(arguments) = batch_arguments(term, "时长", "duration") {
                        Self::text_value(
                            map_batch_lines(arguments, convert_duration)
                                .map_err(ValueError::BatchDuration),
                        )
                    } else if let Some(arguments) =
                        batch_arguments(term, "精确时长对照", "exact-duration-csv")
                    {
                        Self::table_value(
                            map_batch_csv(arguments, "输入", convert_exact_duration)
                                .map_err(ValueError::BatchDuration),
                        )
                    } else if let Some(arguments) =
                        batch_arguments(term, "精确时长", "exact-duration")
                    {
                        Self::text_value(
                            map_batch_lines(arguments, convert_exact_duration)
                                .map_err(ValueError::BatchDuration),
                        )
                    } else if let Some(arguments) =
                        batch_arguments(term, "时间戳对照", "timestamp-csv")
                    {
                        let now = chrono::Utc::now();
                        Self::table_value(
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
                        Self::table_value(
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
                        Self::table_value(
                            map_batch_csv(arguments, "输入", convert_url_component)
                                .map_err(ValueError::BatchUrl),
                        )
                    } else if let Some(arguments) = batch_arguments(term, "网址", "url") {
                        Self::text_value(
                            map_batch_lines(arguments, convert_url_component)
                                .map_err(ValueError::BatchUrl),
                        )
                    } else if let Some(arguments) = batch_arguments(term, "颜色对照", "color-csv")
                    {
                        Self::table_value(
                            map_batch_csv(arguments, "输入", convert_color_to_hex)
                                .map_err(ValueError::BatchColor),
                        )
                    } else if let Some(arguments) = batch_arguments(term, "颜色", "color") {
                        Self::text_value(
                            map_batch_lines(arguments, convert_color_to_hex)
                                .map_err(ValueError::BatchColor),
                        )
                    } else if let Some(arguments) = batch_arguments(term, "日期对照", "date-csv")
                    {
                        let today = chrono::Local::now().date_naive();
                        Self::table_value(
                            map_batch_csv(arguments, "输入", |line| calculate_date(line, today))
                                .map_err(ValueError::BatchDate),
                        )
                    } else if let Some(arguments) = batch_arguments(term, "日期", "date") {
                        let today = chrono::Local::now().date_naive();
                        Self::text_value(
                            map_batch_lines(arguments, |line| calculate_date(line, today))
                                .map_err(ValueError::BatchDate),
                        )
                    } else if let Some(arguments) = batch_arguments(term, "进制对照", "radix-csv")
                    {
                        Self::table_value(
                            map_batch_csv(arguments, "输入", convert_arbitrary_radix)
                                .map_err(ValueError::BatchRadix),
                        )
                    } else if let Some(result) = map_tool_batch(term) {
                        match result {
                            Ok(BatchToolOutput::Lines(text)) => Self::text_value(Ok(text)),
                            Ok(BatchToolOutput::Csv(text)) => Self::table_value(Ok(text)),
                            Err(error) => Self::Value(Err(ValueError::BatchTool(error))),
                        }
                    } else {
                        Self::text_value(convert_batch(term).map_err(ValueError::BatchConversion))
                    }
                }
                Scope::Radix => match radix_csv_report(term) {
                    Some(report) => Self::table_value(report.map_err(ValueError::Radix)),
                    None => Self::Value(
                        convert_arbitrary_radix(term)
                            .map(|text| ResultValue {
                                text,
                                kind: ValueKind::Conversion(None),
                            })
                            .map_err(ValueError::Radix),
                    ),
                },
                Scope::Applications => applications.results(term),
                Scope::ApplicationHistory => recent.launched_apps.results(term),
                Scope::Folders => Self::Destinations(matching_folders(term)),
                Scope::WindowsSettings => {
                    Self::Destinations(windows_settings::matching_windows_settings(term))
                }
                Scope::LocalFiles => local_files.results(term),
                Scope::RecentFilePreviews => recent.file_previews.results(term),
                Scope::OpenWindows => open_windows.results(term),
                Scope::Actions => {
                    let mut entries = matching_actions(actions, term);
                    entries.insert(
                        entries.len().min(DESTINATION_PAGE_SIZE - 1),
                        Destination::ActionLibraryQuery(term.to_owned()),
                    );
                    Self::Destinations(entries)
                }
                Scope::Date => Self::date_value(term),
                Scope::Duration => Self::duration_value(term),
                Scope::VideoFrames => {
                    Self::text_value(convert_video_frames(term).map_err(ValueError::VideoFrames))
                }
                Scope::Timecode => {
                    Self::text_value(convert_timecode(term).map_err(ValueError::Timecode))
                }
                Scope::TransferTime => Self::conversion_value(estimate_transfer(term), None),
                Scope::ExactData => exact_data_csv_report(term).map_or_else(
                    || Self::conversion_value(convert_exact_data(term), None),
                    |report| Self::table_value(report.map_err(ValueError::Conversion)),
                ),
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
                Scope::Wave => Self::text_value(convert_wave(term).map_err(ValueError::Conversion)),
                Scope::AspectRatio => {
                    Self::text_value(calculate_aspect_ratio(term).map_err(ValueError::AspectRatio))
                }
                Scope::MapScale => {
                    Self::text_value(convert_map_scale(term).map_err(ValueError::Conversion))
                }
                Scope::PixelDensity => {
                    Self::text_value(calculate_pixel_density(term).map_err(ValueError::Conversion))
                }
                Scope::Paper => Self::text_value(calculate_paper(term).map_err(ValueError::Paper)),
                Scope::Pitch => Self::text_value(convert_pitch(term).map_err(ValueError::Pitch)),
                Scope::Solid => Self::text_value(calculate_solid(term).map_err(ValueError::Solid)),
                Scope::Tempo => Self::text_value(calculate_tempo(term).map_err(ValueError::Tempo)),
                Scope::Decibel => {
                    Self::text_value(convert_decibels(term).map_err(ValueError::Decibel))
                }
                Scope::Rectangle => {
                    Self::text_value(calculate_rectangle(term).map_err(ValueError::Rectangle))
                }
                Scope::Cidr => match cidr::cidr_csv_report(term) {
                    Some(report) => Self::table_value(report.map_err(ValueError::Cidr)),
                    None => Self::text_value(calculate_cidr(term).map_err(ValueError::Cidr)),
                },
                Scope::FileMode => {
                    Self::text_value(calculate_file_mode(term).map_err(ValueError::FileMode))
                }
                Scope::Timestamp => Self::Value(
                    convert_timestamp(term, chrono::Utc::now())
                        .map_err(ValueError::Timestamp)
                        .and_then(|text| {
                            let kind = if is_timestamp_report(term) {
                                ValueKind::Table(
                                    csv_tools::parse_table_preview(&text)
                                        .map_err(ValueError::Csv)?,
                                )
                            } else if is_offset_timestamp(&text) {
                                ValueKind::TimestampInstant
                            } else {
                                ValueKind::Text
                            };
                            Ok(ResultValue { text, kind })
                        }),
                ),
                Scope::Uuid => {
                    Self::text_value(transform_uuid(term).map_err(ValueError::Identifier))
                }
                Scope::Ulid => {
                    Self::text_value(transform_ulid(term).map_err(ValueError::Identifier))
                }
                Scope::Isbn => {
                    Self::text_value(transform_isbn(term).map_err(ValueError::Identifier))
                }
                Scope::Gtin => {
                    Self::text_value(transform_gtin(term).map_err(ValueError::Identifier))
                }
                Scope::Semver => {
                    Self::text_value(transform_semver(term).map_err(ValueError::Identifier))
                }
                Scope::Color => {
                    let palette = batch_arguments(term, "色阶", "palette")
                        .map(|input| (input, ColorMixSpace::Srgb))
                        .or_else(|| {
                            batch_arguments(term, "线性色阶", "linear-palette")
                                .map(|input| (input, ColorMixSpace::LinearSrgb))
                        })
                        .or_else(|| {
                            batch_arguments(term, "OKLab色阶", "oklab-palette")
                                .map(|input| (input, ColorMixSpace::Oklab))
                        });
                    if let Some((input, space)) = palette {
                        Self::text_value(
                            make_color_palette(input, space).map_err(ValueError::Color),
                        )
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
                Scope::TextTools => match text_tools::text_csv_report(term) {
                    Some(report) => Self::table_value(report.map_err(ValueError::Text)),
                    None => Self::text_value(transform_text(term).map_err(ValueError::Text)),
                },
                Scope::Json => {
                    if let Some(report) = json_tools::json_validation_report(term) {
                        Self::Value(
                            report
                                .map(|report| ResultValue::from_validation(report, scope))
                                .map_err(ValueError::Json),
                        )
                    } else if let Some(report) = json_tools::json_validation_table(term) {
                        Self::input_table_value(
                            report.map_err(ValueError::Json),
                            raw_query.trim_end().len() - term.len(),
                        )
                    } else if let Some(report) = json_tools::json_csv_report(term) {
                        Self::table_value(report.map_err(ValueError::Json))
                    } else {
                        Self::text_value(transform_json(term).map_err(ValueError::Json))
                    }
                }
                Scope::Toml => {
                    if let Some(report) = toml_tools::toml_validation_report(term) {
                        Self::Value(
                            report
                                .map(|report| ResultValue::from_validation(report, scope))
                                .map_err(ValueError::Toml),
                        )
                    } else if let Some(report) = toml_tools::toml_csv_report(term) {
                        Self::table_value(report.map_err(ValueError::Toml))
                    } else {
                        Self::text_value(transform_toml(term).map_err(ValueError::Toml))
                    }
                }
                Scope::Csv => {
                    if let Some(report) = csv_tools::csv_validation_report(term) {
                        Self::Value(
                            report
                                .map(|report| ResultValue::from_validation(report, scope))
                                .map_err(ValueError::Csv),
                        )
                    } else if let Some(result) = csv_tools::csv_table_preview(term) {
                        Self::Value(
                            result
                                .map(|(text, table)| ResultValue {
                                    text,
                                    kind: ValueKind::Table(table),
                                })
                                .map_err(ValueError::Csv),
                        )
                    } else if let Some(arguments) = csv_tools::scatter_arguments(term) {
                        Self::plot_value(
                            csv_tools::build_scatter_plot(arguments).map_err(ValueError::Csv),
                        )
                    } else if let Some(result) = csv_tools::coordinate_line_chart(term) {
                        Self::plot_value(result.map_err(ValueError::Csv))
                    } else if let Some(arguments) =
                        csv_tools::column_cumulative_plot_arguments(term)
                    {
                        Self::plot_value(
                            csv_tools::build_column_cumulative_plot(arguments)
                                .map_err(ValueError::Csv),
                        )
                    } else if let Some(arguments) = csv_tools::column_box_plot_arguments(term) {
                        Self::Value(
                            csv_tools::build_column_box_plot(arguments)
                                .map(|(text, plot)| ResultValue {
                                    text,
                                    kind: ValueKind::BoxPlot(plot),
                                })
                                .map_err(ValueError::Csv),
                        )
                    } else if let Some(result) = csv_tools::csv_count_chart(term) {
                        Self::Value(
                            result
                                .map(|(text, plot)| ResultValue {
                                    text,
                                    kind: ValueKind::BinPlot(plot),
                                })
                                .map_err(ValueError::Csv),
                        )
                    } else if let Some((arguments, rank)) =
                        csv_tools::correlation_plot_arguments(term)
                    {
                        Self::Value(
                            csv_tools::build_correlation_plot(arguments, rank)
                                .map(|(text, plot)| ResultValue {
                                    text,
                                    kind: ValueKind::CorrelationPlot(plot),
                                })
                                .map_err(ValueError::Csv),
                        )
                    } else {
                        Self::text_value(transform_csv(term).map_err(ValueError::Csv))
                    }
                }
                Scope::DataPipeline => Self::Value(
                    transform_data_pipeline(term, chrono::Local::now().fixed_offset())
                        .map_err(ValueError::DataPipeline)
                        .map(|(text, preview)| ResultValue {
                            text,
                            kind: match preview {
                                Some(PipelinePreview::Scatter(plot)) => ValueKind::Scatter(plot),
                                Some(PipelinePreview::BoxPlot(plot)) => ValueKind::BoxPlot(plot),
                                Some(PipelinePreview::Bins(plot)) => ValueKind::BinPlot(plot),
                                Some(PipelinePreview::Correlation(plot)) => {
                                    ValueKind::CorrelationPlot(plot)
                                }
                                Some(PipelinePreview::Table(table)) => ValueKind::Table(table),
                                None => ValueKind::Text,
                            },
                        }),
                ),
                Scope::Fraction => {
                    if let Some(examples) = fraction_function_examples(term) {
                        Self::Help(examples)
                    } else if let Some(report) = fraction::fraction_csv_report(term) {
                        Self::table_value(report.map_err(ValueError::Fraction))
                    } else {
                        Self::Value(
                            fraction::simplify_fraction_with_answers(term, &|index| {
                                recent.calculations.exact_number_at(index)
                            })
                            .map(|text| {
                                let exact = fraction::parse_confirmed_fraction(&text).ok();
                                ResultValue {
                                    kind: ValueKind::ExactCalculation {
                                        can_continue: exact.is_some(),
                                        decimal_preview: exact
                                            .as_ref()
                                            .filter(|_| text.contains('/'))
                                            .and_then(fraction::decimal_preview),
                                    },
                                    text,
                                }
                            })
                            .map_err(ValueError::Fraction),
                        )
                    }
                }
                Scope::Equation => {
                    if let Some(result) = equation::equation_chart(term) {
                        Self::plot_value(result.map_err(ValueError::Equation))
                    } else {
                        Self::Value(
                            solve_equation_output(term)
                                .map_err(ValueError::Equation)
                                .and_then(|output| match output {
                                    EquationOutput::Text(text) => Ok(ResultValue {
                                        text,
                                        kind: ValueKind::Text,
                                    }),
                                    EquationOutput::Table(text) => {
                                        let table = csv_tools::parse_table_preview(&text)
                                            .map_err(ValueError::Csv)?;
                                        Ok(ResultValue {
                                            text,
                                            kind: ValueKind::Table(table),
                                        })
                                    }
                                }),
                        )
                    }
                }
                Scope::LinearSystem => {
                    Self::text_value(solve_linear_system(term).map_err(ValueError::Equation))
                }
                Scope::LinearFit => {
                    if let Some(plot) = linear_fit::plot_request(term) {
                        Self::plot_value(
                            linear_fit::calculate_plot(plot).map_err(ValueError::LinearFit),
                        )
                    } else {
                        Self::text_value(calculate_linear_fit(term).map_err(ValueError::LinearFit))
                    }
                }
                Scope::Statistics => {
                    if let Some(result) = statistics::statistics_csv_report(term) {
                        Self::table_value(result.map_err(ValueError::Statistics))
                    } else if let Some(values) = statistics::cumulative_plot_arguments(term) {
                        Self::plot_value(
                            statistics::plot_cumulative_distribution(values)
                                .map_err(ValueError::Statistics),
                        )
                    } else if let Some(values) = statistics::box_plot_arguments(term) {
                        Self::Value(
                            statistics::calculate_box_plot(values)
                                .map(|(text, plot)| ResultValue {
                                    text,
                                    kind: ValueKind::BoxPlot(plot),
                                })
                                .map_err(ValueError::Statistics),
                        )
                    } else if let Some(values) = statistics::bin_plot_arguments(term) {
                        Self::Value(
                            statistics::calculate_bin_plot(values)
                                .map(|(text, plot)| ResultValue {
                                    text,
                                    kind: ValueKind::BinPlot(plot),
                                })
                                .map_err(ValueError::Statistics),
                        )
                    } else {
                        Self::text_value(calculate_statistics(term).map_err(ValueError::Statistics))
                    }
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
                Scope::Intervals => {
                    Self::text_value(calculate_intervals(term).map_err(ValueError::Intervals))
                }
                Scope::UrlEncoding => match url_encoding::url_csv_report(term) {
                    Some(report) => Self::table_value(report.map_err(ValueError::UrlEncoding)),
                    None => Self::text_value(
                        convert_url_component(term).map_err(ValueError::UrlEncoding),
                    ),
                },
                Scope::Vector => {
                    Self::text_value(calculate_vector(term).map_err(ValueError::Vector))
                }
                Scope::Matrix => {
                    Self::text_value(calculate_matrix(term).map_err(ValueError::Matrix))
                }
                Scope::Path => match path_csv_report(term) {
                    Some(report) => Self::table_value(report.map_err(ValueError::Path)),
                    None => Self::text_value(path_component(term).map_err(ValueError::Path)),
                },
                Scope::Circle => {
                    Self::text_value(calculate_circle(term).map_err(ValueError::Circle))
                }
                Scope::Polygon => {
                    Self::text_value(calculate_polygon(term).map_err(ValueError::Polygon))
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
            let result =
                calculate_with_answers(query, &|index| recent.calculations.number_at(index));
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
            matching_triggers(triggers, &query, &Default::default(), None)
                .into_iter()
                .map(|trigger| Destination::Trigger(trigger.id())),
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
                return Self::date_value(&query);
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

    fn plot_value(result: Result<(String, scatter_plot::ScatterPlot), ValueError>) -> Self {
        Self::Value(result.map(|(text, plot)| ResultValue {
            text,
            kind: ValueKind::Scatter(plot),
        }))
    }

    fn input_table_value(result: Result<InputTableReport, ValueError>, offset: usize) -> Self {
        Self::Value(result.and_then(|report| {
            let report = report.offset_input_errors(offset);
            let table = csv_tools::parse_table_preview(&report.csv)
                .map_err(ValueError::Csv)?
                .with_input_errors(report.input_errors);
            Ok(ResultValue {
                text: report.csv,
                kind: ValueKind::Table(table),
            })
        }))
    }

    fn table_value(result: Result<String, ValueError>) -> Self {
        Self::table_value_with_columns(result, table_preview::MAX_VISIBLE_COLUMNS)
    }

    fn table_value_with_columns(result: Result<String, ValueError>, columns: usize) -> Self {
        Self::Value(result.and_then(|text| {
            let table = csv_tools::parse_table_preview(&text)
                .map_err(ValueError::Csv)?
                .with_columns_per_page(columns);
            Ok(ResultValue {
                text,
                kind: ValueKind::Table(table),
            })
        }))
    }

    fn date_value(query: &str) -> Self {
        let result =
            calculate_date(query, chrono::Local::now().date_naive()).map_err(ValueError::Date);
        Self::Value(result.and_then(|text| {
            let date = text.trim();
            let kind = if is_date_report(query) {
                ValueKind::Table(csv_tools::parse_table_preview(&text).map_err(ValueError::Csv)?)
            } else if date.chars().nth(11).is_none() && parse_complete_date(date).is_ok() {
                ValueKind::CalendarDate
            } else {
                ValueKind::Text
            };
            Ok(ResultValue { text, kind })
        }))
    }

    fn duration_value(query: &str) -> Self {
        if is_duration_report(query) {
            return Self::table_value(convert_duration(query).map_err(ValueError::Duration));
        }
        Self::Value(
            convert_duration(query)
                .map_err(ValueError::Duration)
                .map(|text| {
                    let kind = if is_clock_duration(&text) {
                        ValueKind::ClockDuration
                    } else {
                        ValueKind::Text
                    };
                    ResultValue { text, kind }
                }),
        )
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

    fn selectable_count(&self) -> usize {
        match self {
            Self::Destinations(items) => items.len(),
            Self::Help(items) => items.len(),
            Self::Calculations(items)
            | Self::Conversions(items)
            | Self::Queries(items)
            | Self::ConversionTargets(items)
            | Self::FileOptions(items)
            | Self::FilterOptions(items)
            | Self::Bookmarks(items) => items.len(),
            Self::Value(_)
            | Self::RefilledInput
            | Self::ApplicationsLoading
            | Self::ApplicationsUnavailable(_)
            | Self::OpenWindowsLoading
            | Self::OpenWindowsUnavailable(_)
            | Self::LocalFilesPrompt
            | Self::DirectoryInput(_)
            | Self::LocalFilesLoading
            | Self::LocalFilesUnavailable(_)
            | Self::BookmarksUnavailable(_)
            | Self::BookmarkNameInput(_) => 0,
        }
    }

    fn table_preview(&self) -> Option<&table_preview::TablePreview> {
        match self {
            Self::Value(Ok(ResultValue {
                kind: ValueKind::Table(table),
                ..
            })) => Some(table),
            _ => None,
        }
    }

    fn suggested_input(&self, selected: usize) -> Option<&str> {
        match self {
            Self::Help(items) => Some(EXAMPLES[*items.get(selected)?].query),
            Self::DirectoryInput(item) if selected == 0 => Some(&item.expression),
            Self::Calculations(items)
            | Self::Conversions(items)
            | Self::Queries(items)
            | Self::ConversionTargets(items)
            | Self::FileOptions(items)
            | Self::FilterOptions(items)
            | Self::Bookmarks(items) => Some(&items.get(selected)?.expression),
            _ => None,
        }
    }

    fn bookmark_uses_selection(&self) -> bool {
        matches!(
            self,
            Self::Help(_)
                | Self::Bookmarks(_)
                | Self::Queries(_)
                | Self::Calculations(_)
                | Self::Conversions(_)
        )
    }

    fn bookmark_query<'a>(&'a self, query: &'a str, selected: usize) -> Option<&'a str> {
        if self.bookmark_uses_selection() {
            return self.suggested_input(selected);
        }
        let trimmed = query.trim();
        (!trimmed.is_empty()
            && !trimmed.starts_with(['?', '？'])
            && !matches!(
                self,
                Self::FileOptions(_)
                    | Self::FilterOptions(_)
                    | Self::BookmarksUnavailable(_)
                    | Self::Value(Err(ValueError::InputTooLong))
            ))
        .then_some(query)
    }

    fn continuation_query(&self) -> Option<String> {
        let Self::Value(Ok(ResultValue { kind, text })) = self else {
            return None;
        };
        match kind {
            ValueKind::Calculation if text.starts_with('-') => Some(format!("({text})")),
            ValueKind::Calculation => Some(text.clone()),
            // 为括号、运算符和至少一位操作数留出空间。
            ValueKind::ExactCalculation {
                can_continue: true, ..
            } if text.len() > calculator::QUERY_LIMIT - 4 => Some("分数: 计算 (ans)".to_owned()),
            ValueKind::ExactCalculation {
                can_continue: true, ..
            } => Some(format!("分数: 计算 ({text})")),
            ValueKind::Conversion(Some(query)) => Some(query.clone()),
            ValueKind::CalendarDate => Some(format!("日期: {}", text.trim())),
            ValueKind::ClockDuration => Some(format!("时长: {text}")),
            ValueKind::TimestampInstant => Some(format!("时间戳: {text}")),
            ValueKind::Color(swatch) => Some(format!("颜色: {}", hex_for_swatch(*swatch))),
            _ => None,
        }
    }

    fn refillable_text(&self) -> Option<&str> {
        let Self::Value(Ok(value)) = self else {
            return None;
        };
        let text = &value.text;
        (!text.is_empty()
            && text.chars().take(INPUT_CHARACTER_LIMIT + 1).count() <= INPUT_CHARACTER_LIMIT)
            .then_some(text.as_str())
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

    fn local_file_index(&self, path: &str) -> Option<usize> {
        let Self::Destinations(items) = self else {
            return None;
        };
        items.iter().position(
            |item| matches!(item, Destination::LocalFile { path: current, .. } if current == path),
        )
    }

    fn selected_text_file(&self, selected: usize) -> Option<&str> {
        let Self::Destinations(items) = self else {
            return None;
        };
        let Destination::LocalFile {
            path,
            is_directory: false,
            ..
        } = items.get(selected)?
        else {
            return None;
        };
        matches!(
            input_file_extension(Path::new(path)).as_deref(),
            Some("txt" | "md" | "log" | "csv" | "tsv" | "json" | "jsonl" | "ndjson" | "toml")
        )
        .then_some(path.as_str())
    }

    fn value_preview_expandable(&self) -> bool {
        matches!(
            self,
            Self::Value(Ok(value)) if value.visual_preview_height().is_some()
                || value_preview::lines(&value.text).nth(MAX_VALUE_LINES).is_some()
        )
    }

    fn height(&self, value_line_limit: usize) -> f32 {
        match self {
            Self::Destinations(items) if items.len() > 1 => {
                132. + items.len().min(DESTINATION_PAGE_SIZE) as f32 * PAGE_ROW_HEIGHT
            }
            Self::Value(Ok(value)) => {
                if let Some(preview) = value
                    .visual_preview_height()
                    .filter(|_| value_line_limit == MAX_VALUE_LINES)
                {
                    return 158. + preview + 52.;
                }
                let hints = usize::from(value.continuation_hint().is_some())
                    + usize::from(value.exact_decimal_preview().is_some())
                    + usize::from(matches!(
                        &value.kind,
                        ValueKind::InputValidationError { .. }
                    ))
                    + usize::from(self.value_preview_expandable());
                158. + value_extra_height(&value.text, value_line_limit) + hints as f32 * 24.
            }
            Self::Help(items) if !items.is_empty() => {
                132. + items.len().min(EXAMPLE_PAGE_SIZE) as f32 * EXAMPLE_ROW_HEIGHT
                    + EXAMPLE_SHORTCUT_ROW_HEIGHT
            }
            Self::DirectoryInput(_) => 132. + EXAMPLE_ROW_HEIGHT,
            Self::ConversionTargets(items) if !items.is_empty() => {
                132. + items.len().min(CONVERSION_PAGE_SIZE) as f32 * EXAMPLE_ROW_HEIGHT
            }
            Self::FileOptions(items)
            | Self::FilterOptions(items)
            | Self::Bookmarks(items)
            | Self::Calculations(items)
            | Self::Conversions(items)
            | Self::Queries(items)
                if !items.is_empty() =>
            {
                132. + items.len().min(EXAMPLE_PAGE_SIZE) as f32 * EXAMPLE_ROW_HEIGHT
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

#[derive(Clone, Copy)]
enum PageChange {
    Previous,
    Next,
    First,
    Last,
}

impl PageChange {
    fn target(self, page: usize, last_page: usize) -> usize {
        match self {
            Self::Previous => page.saturating_sub(1),
            Self::Next => (page + 1).min(last_page),
            Self::First => 0,
            Self::Last => last_page,
        }
    }
}

#[derive(Clone, Copy)]
enum TableAxis {
    Rows,
    Columns,
}

#[derive(Clone, Copy)]
enum TableCopy {
    CurrentPage,
    CurrentPageTsv,
    Tsv,
}

#[derive(Clone, Copy, Default)]
struct TablePages {
    rows: usize,
    columns: usize,
}

impl TablePages {
    fn clamped_to(self, table: &table_preview::TablePreview) -> Self {
        Self {
            rows: self.rows.min(table.row_pages().saturating_sub(1)),
            columns: self.columns.min(table.column_pages().saturating_sub(1)),
        }
    }
}

struct ResultSearchDraft {
    original: QueryDraft,
    command: &'static str,
}

struct QueryDraft {
    text: String,
    selection: MessageSelection,
    selected: usize,
    suggestion: Option<String>,
    table_pages: Option<TablePages>,
    expanded_value: bool,
}

impl QueryDraft {
    fn text(&self) -> &str {
        &self.text
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ApplicationRefresh {
    NotRequested,
    Loading,
    Finished,
}

const SEARCH_PLACEHOLDER: &str = "搜索页面、触发器，或输入算式";
const INPUT_PROMPT_HEIGHT: f32 = 148.;
const INPUT_REPLACE_HEIGHT: f32 = 200.;
const TABLE_SEARCH_COMMAND: &str = "CSV: 搜索行 ";
const TEXT_SEARCH_COMMAND: &str = "文本: 搜索行 ";
const TABLE_CELL_COMMAND: &str = "CSV: 取单元格 ";

struct BookmarkNameEdit {
    expression: String,
    filter: String,
}

#[derive(Clone, Copy)]
enum LineJumpTarget {
    Input,
    Table(TableAxis),
}

impl LineJumpTarget {
    fn number_label(self) -> &'static str {
        match self {
            Self::Input | Self::Table(TableAxis::Rows) => "行号",
            Self::Table(TableAxis::Columns) => "列号",
        }
    }
}

struct LineJump {
    input: Entity<MessageInput>,
    total: usize,
    target: LineJumpTarget,
    _changes: Subscription,
}

impl LineJump {
    fn number(&self, cx: &App) -> Option<usize> {
        let normalized = calculator::normalize(self.input.read(cx).text_ref());
        normalized
            .trim()
            .parse()
            .ok()
            .filter(|number| (1..=self.total).contains(number))
    }
}

struct ResultRefill {
    original: String,
    inserted: String,
    kind: RefillKind,
}

struct InputFind {
    input: Entity<MessageInput>,
    replacement: Option<InputReplacement>,
    _changes: Subscription,
}

struct InputReplacement {
    input: Entity<MessageInput>,
    _changes: Subscription,
}

fn input_replacement_fits(text: &str, needle: &str, replacement: &str, count: usize) -> bool {
    let unchanged = text.chars().count() - count * needle.chars().count();
    unchanged + count * replacement.chars().count() <= INPUT_CHARACTER_LIMIT
}

fn find_input_range(
    text: &str,
    needle: &str,
    selection: Range<usize>,
    backwards: bool,
) -> Option<Range<usize>> {
    if needle.is_empty() {
        return None;
    }
    let matches = || {
        text.match_indices(needle)
            .map(|(start, matched)| start..start + matched.len())
    };
    if backwards {
        matches()
            .take_while(|range| range.end <= selection.start)
            .last()
            .or_else(|| matches().last())
    } else {
        matches()
            .find(|range| range.start >= selection.end)
            .or_else(|| matches().next())
    }
}

enum RefillKind {
    Result { evaluated: bool },
    File { path: String },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FilePreviewAction {
    Load,
    Refresh,
}

struct FileSearchReturn {
    query: String,
    path: String,
}

enum ResultSaveState {
    Ready,
    InProgress,
    Finished {
        query: String,
        result: Result<(), String>,
    },
}

enum InputFileLoadState {
    Ready,
    InProgress,
    Failed(String),
}

fn accepts_input_file(query: &str) -> bool {
    query.trim().is_empty()
        || matches!(
            scoped_query(query.trim_start()),
            Some((
                Scope::Csv | Scope::Json | Scope::Toml | Scope::TextTools | Scope::DataPipeline,
                _
            ))
        )
}

fn input_file_extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(std::ffi::OsStr::to_str)
        .map(str::to_ascii_lowercase)
}

fn input_file_query_prefix(path: &Path) -> &'static str {
    match input_file_extension(path).as_deref() {
        Some("csv") => "CSV: 表格预览 ",
        Some("tsv") => "CSV: 从TSV ",
        Some("json") => "JSON: 格式化 ",
        Some("jsonl" | "ndjson") => "JSON: 合并行 ",
        Some("toml") => "TOML: 校验 ",
        _ => "文本: 原文 ",
    }
}

fn read_input_file(path: &Path, character_limit: usize) -> Result<String, String> {
    let read_error = |error: std::io::Error| format!("无法读取输入文件：{error}");
    if !path.metadata().map_err(read_error)?.is_file() {
        return Err("请选择一个普通文本文件".to_owned());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(read_error)?
        .take((input_file_byte_limit(character_limit) + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(read_error)?;
    decode_input_file(&bytes, character_limit)
}

fn input_file_byte_limit(character_limit: usize) -> usize {
    character_limit * 4 + "\u{feff}".len()
}

fn decode_input_file(bytes: &[u8], character_limit: usize) -> Result<String, String> {
    let too_long = || format!("文件内容超过可插入的 {character_limit} 字符");
    if bytes.len() > input_file_byte_limit(character_limit) {
        return Err(too_long());
    }
    let encoding_error = || "文件须为 UTF-8 文本或带 BOM 的 UTF-16 文本".to_owned();
    let text = match bytes {
        [0xff, 0xfe, payload @ ..] | [0xfe, 0xff, payload @ ..] => {
            let units = payload.chunks_exact(2);
            if !units.remainder().is_empty() {
                return Err(encoding_error());
            }
            let little_endian = bytes[0] == 0xff;
            let units = units.map(|unit| {
                let encoded = [unit[0], unit[1]];
                if little_endian {
                    u16::from_le_bytes(encoded)
                } else {
                    u16::from_be_bytes(encoded)
                }
            });
            char::decode_utf16(units)
                .collect::<Result<String, _>>()
                .map_err(|_| encoding_error())?
        }
        _ => {
            let payload = bytes.strip_prefix("\u{feff}".as_bytes()).unwrap_or(bytes);
            std::str::from_utf8(payload)
                .map_err(|_| encoding_error())?
                .to_owned()
        }
    };
    if text.contains('\0') {
        return Err(encoding_error());
    }
    if text.chars().count() > character_limit {
        return Err(too_long());
    }
    Ok(text)
}

fn read_selected_text_query(path: &str) -> Result<String, String> {
    let prefix = input_file_query_prefix(Path::new(path));
    let character_limit = INPUT_CHARACTER_LIMIT - prefix.chars().count();
    let bytes = omnitrigger_windows::read_local_preview_file(
        path,
        input_file_byte_limit(character_limit) + 1,
        || Ok(()),
    )
    .map_err(|error| error.message)?;
    let text = decode_input_file(&bytes, character_limit)?;
    Ok(format!("{prefix}{text}"))
}

pub(super) struct QuickSearch {
    theme_mode: appearance::Mode,
    input: Entity<MessageInput>,
    input_revision: u64,
    workspace: WeakEntity<Workspace>,
    index: SearchIndex,
    was_active: bool,
    results: Results,
    expanded_value_pages: Option<Vec<usize>>,
    table_pages: TablePages,
    selected: usize,
    application_refresh: ApplicationRefresh,
    input_before_help: Option<QueryDraft>,
    input_before_calculation_history: Option<QueryDraft>,
    input_before_query_history: Option<QueryDraft>,
    input_before_bookmarks: Option<QueryDraft>,
    query_draft_return: Option<QueryDraft>,
    input_before_result_search: Option<ResultSearchDraft>,
    result_refill: Option<ResultRefill>,
    file_search_return: Option<FileSearchReturn>,
    result_save: ResultSaveState,
    result_copied: bool,
    input_file_load: InputFileLoadState,
    previous_input: RememberedSearchInput,
    bookmark_limit_reached: bool,
    bookmark_save_error: Option<String>,
    bookmark_name_edit: Option<BookmarkNameEdit>,
    line_jump: Option<LineJump>,
    input_find: Option<InputFind>,
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
                .with_line_position()
                .with_undo_history()
                .with_character_limit(INPUT_CHARACTER_LIMIT + 1);
            input.set_placeholder(SEARCH_PLACEHOLDER);
            input
        });
        let input_changes = cx.observe_in(&input, window, |this, input, window, cx| {
            let input = input.read(cx);
            let revision = input.edit_revision();
            let selection = input.selection_position();
            if this.input_revision == revision {
                if !this.has_temporary_query() && this.bookmark_name_edit.is_none() {
                    let mut last = this.index.recent.last_input.borrow_mut();
                    if !last.query.is_empty() && last.query == input.text_ref() {
                        last.selection = Some(selection);
                    }
                }
                return;
            }
            this.input_revision = revision;
            if this.bookmark_name_edit.is_some() {
                this.results = Results::BookmarkNameInput(None);
                cx.notify();
                return;
            }
            let query = input.text();
            this.result_copied = false;
            if matches!(&this.result_save, ResultSaveState::Finished { query: saved, .. } if saved != &query) {
                this.result_save = ResultSaveState::Ready;
            }
            if matches!(this.input_file_load, InputFileLoadState::Failed(_)) {
                this.input_file_load = InputFileLoadState::Ready;
            }
            this.bookmark_limit_reached = false;
            this.bookmark_save_error = None;
            this.refresh_results(&query, window, cx);
            if !query.trim().is_empty() && !this.has_temporary_query() {
                *this.index.recent.last_input.borrow_mut() = RememberedSearchInput {
                    query,
                    selection: Some(selection),
                };
            }
        });
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.was_active = true;
                this.activate_input(window, cx);
            } else if this.was_active && !this.file_operation_in_progress() {
                window.remove_window();
            }
        });
        window.focus(&input.focus_handle(cx));
        let previous_input = index.recent.last_input.borrow().clone();
        Self {
            theme_mode,
            input,
            input_revision: 0,
            workspace,
            index,
            was_active: window.is_window_active(),
            results,
            expanded_value_pages: None,
            table_pages: TablePages::default(),
            selected: 0,
            application_refresh: ApplicationRefresh::NotRequested,
            input_before_help: None,
            input_before_calculation_history: None,
            input_before_query_history: None,
            input_before_bookmarks: None,
            query_draft_return: None,
            input_before_result_search: None,
            result_refill: None,
            file_search_return: None,
            result_save: ResultSaveState::Ready,
            result_copied: false,
            input_file_load: InputFileLoadState::Ready,
            previous_input,
            bookmark_limit_reached: false,
            bookmark_save_error: None,
            bookmark_name_edit: None,
            line_jump: None,
            input_find: None,
            _input_changes: input_changes,
            _activation: activation,
        }
    }

    pub(super) fn activate(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.activate_window();
        self.activate_input(window, cx);
    }

    fn activate_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        let input = if let Some(find) = &self.input_find {
            &find.input
        } else {
            self.line_jump
                .as_ref()
                .map_or(&self.input, |jump| &jump.input)
        };
        window.focus(&input.focus_handle(cx));
        cx.notify();
    }

    fn open_line_jump(
        &mut self,
        target: LineJumpTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.line_jump.is_some()
            || self.input_find.is_some()
            || self.bookmark_name_edit.is_some()
            || self.file_operation_in_progress()
            || self.input.read(cx).is_composing()
        {
            return;
        }
        let position = match target {
            LineJumpTarget::Input => self.input.read(cx).line_position(),
            LineJumpTarget::Table(axis) if self.expanded_value_pages.is_none() => {
                self.results.table_preview().and_then(|table| match axis {
                    TableAxis::Rows => table.page_row_position(self.table_pages.rows),
                    TableAxis::Columns => table.page_column_position(self.table_pages.columns),
                })
            }
            LineJumpTarget::Table(_) => None,
        };
        let Some((number, total)) = position else {
            return;
        };
        let input = cx.new(|cx| {
            let mut input = MessageInput::new(cx).embedded().with_character_limit(8);
            let number = number.to_string();
            let len = number.len();
            input.set_placeholder(target.number_label());
            input.set_text(number, cx);
            input.select_range(0..len, cx);
            input
        });
        let changes = cx.observe_in(&input, window, |_, _, _, cx| cx.notify());
        self.line_jump = Some(LineJump {
            input,
            total,
            target,
            _changes: changes,
        });
        window.resize(size(px(WIDTH), px(INPUT_PROMPT_HEIGHT)));
        self.activate_input(window, cx);
    }

    fn finish_line_jump(&mut self, confirm: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(jump) = &self.line_jump else {
            return;
        };
        if confirm {
            let Some(number) = jump.number(cx) else {
                return;
            };
            match jump.target {
                LineJumpTarget::Input => {
                    if !self
                        .input
                        .update(cx, |input, cx| input.go_to_line(number, cx))
                    {
                        return;
                    }
                }
                LineJumpTarget::Table(axis) => {
                    let Some(page) = self.results.table_preview().and_then(|table| match axis {
                        TableAxis::Rows => table.page_for_data_row(number),
                        TableAxis::Columns => table.page_for_column(number),
                    }) else {
                        return;
                    };
                    match axis {
                        TableAxis::Rows => self.table_pages.rows = page,
                        TableAxis::Columns => self.table_pages.columns = page,
                    }
                }
            }
        }
        self.line_jump = None;
        window.resize(size(
            px(WIDTH),
            px(self.results.height(self.value_line_limit())),
        ));
        self.activate_input(window, cx);
    }

    fn open_input_find(&mut self, replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.input_find.is_some()
            || self.line_jump.is_some()
            || self.bookmark_name_edit.is_some()
            || self.file_operation_in_progress()
        {
            return;
        }
        let query = self.input.read(cx);
        if query.text_ref().is_empty() || query.is_composing() {
            return;
        }
        let selected = &query.text_ref()[query.selection()];
        let initial = if selected.contains(['\r', '\n', '\u{2028}', '\u{2029}']) {
            String::new()
        } else {
            selected.to_owned()
        };
        let input = cx.new(|cx| {
            let mut input = MessageInput::new(cx)
                .embedded()
                .with_character_limit(INPUT_CHARACTER_LIMIT);
            input.set_placeholder("输入要查找的文字");
            let len = initial.len();
            input.set_text(initial, cx);
            input.select_range(0..len, cx);
            input
        });
        let changes = cx.observe_in(&input, window, |_, _, _, cx| cx.notify());
        self.input_find = Some(InputFind {
            input,
            replacement: replace.then(|| {
                let input = cx.new(|cx| {
                    let mut input = MessageInput::new(cx)
                        .embedded()
                        .with_character_limit(INPUT_CHARACTER_LIMIT);
                    input.set_placeholder("输入替换文字，留空则删除");
                    input
                });
                let changes = cx.observe_in(&input, window, |_, _, _, cx| cx.notify());
                InputReplacement {
                    input,
                    _changes: changes,
                }
            }),
            _changes: changes,
        });
        let height = if replace {
            INPUT_REPLACE_HEIGHT
        } else {
            INPUT_PROMPT_HEIGHT
        };
        window.resize(size(px(WIDTH), px(height)));
        self.activate_input(window, cx);
    }

    fn replace_input_matches(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(find) = &self.input_find else {
            return;
        };
        let Some(replacement) = &find.replacement else {
            return;
        };
        let needle = find.input.read(cx);
        let replacement = replacement.input.read(cx);
        if needle.is_composing() || replacement.is_composing() || needle.text_ref().is_empty() {
            return;
        }
        let (needle, replacement) = (needle.text_ref(), replacement.text_ref());
        let text = self.input.read(cx).text_ref();
        let count = text.matches(needle).count();
        if count == 0 || !input_replacement_fits(text, needle, replacement, count) {
            return;
        }
        let len = text.encode_utf16().count();
        let edited = text.replace(needle, replacement);
        self.input.update(cx, |input, cx| {
            input.replace_text_in_range(Some(0..len), &edited, window, cx);
        });
        self.finish_input_find(None, window, cx);
    }

    fn finish_input_find(
        &mut self,
        backwards: Option<bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(find) = &self.input_find else {
            return;
        };
        if let Some(backwards) = backwards {
            let query = self.input.read(cx);
            let needle = find.input.read(cx).text_ref();
            let Some(range) =
                find_input_range(query.text_ref(), needle, query.selection(), backwards)
            else {
                return;
            };
            self.input
                .update(cx, |input, cx| input.select_range(range, cx));
        }
        self.input_find = None;
        window.resize(size(
            px(WIDTH),
            px(self.results.height(self.value_line_limit())),
        ));
        self.activate_input(window, cx);
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
        if self.bookmark_name_edit.is_some() {
            return;
        }
        if self
            .result_refill
            .as_ref()
            .is_some_and(|refill| refill.inserted != query)
        {
            self.result_refill = None;
        }
        let refilled = self.has_refilled_input(query);
        if !refilled
            && self.index.wheel_slots.is_none()
            && matches!(scoped_query(query.trim()), Some((Scope::WheelSlots, term)) if !term.is_empty())
        {
            self.index.wheel_slots =
                Some(self.workspace.upgrade().map_or_else(Vec::new, |workspace| {
                    let workspace = workspace.read(cx);
                    if workspace.wheels_error.is_none() {
                        index_slots(&workspace.wheels.wheels)
                    } else {
                        Vec::new()
                    }
                }));
        }
        let next = if refilled {
            Results::RefilledInput
        } else {
            self.index.results(query)
        };
        if !Self::shows_help(query, &next) {
            self.input_before_help = None;
        }
        if !matches!(&next, Results::Calculations(_))
            && !self
                .input_before_help
                .as_ref()
                .map(QueryDraft::text)
                .is_some_and(|input| {
                    matches!(
                        scoped_query(input.trim()),
                        Some((Scope::CalculationHistory, _))
                    )
                })
        {
            self.input_before_calculation_history = None;
        }
        let before_help = self.input_before_help.as_ref().map(QueryDraft::text);
        let history_visible =
            Self::is_query_history(query) || before_help.is_some_and(Self::is_query_history);
        let bookmarks_visible =
            Self::is_bookmark_query(query) || before_help.is_some_and(Self::is_bookmark_query);
        let bookmarks_from_history = self
            .input_before_bookmarks
            .as_ref()
            .map(QueryDraft::text)
            .is_some_and(Self::is_query_history);
        let history_from_bookmarks = self
            .input_before_query_history
            .as_ref()
            .map(QueryDraft::text)
            .is_some_and(Self::is_bookmark_query);
        if !(history_visible || (bookmarks_visible && bookmarks_from_history)) {
            self.input_before_query_history = None;
        }
        if !(bookmarks_visible || (history_visible && history_from_bookmarks)) {
            self.input_before_bookmarks = None;
        }
        if self
            .input_before_result_search
            .as_ref()
            .is_some_and(|draft| {
                !query.starts_with(draft.command)
                    && !before_help.is_some_and(|previous| previous.starts_with(draft.command))
            })
        {
            self.input_before_result_search = None;
        }
        if self.results != next {
            self.result_copied = false;
            let previous_height = self.results.height(self.value_line_limit());
            let next_height = next.height(MAX_VALUE_LINES);
            self.results = next;
            self.expanded_value_pages = None;
            self.table_pages = TablePages::default();
            if let Results::Value(Ok(ResultValue {
                kind: ValueKind::Table(table),
                ..
            })) = &self.results
            {
                self.table_pages.columns = table.first_match_column_page().unwrap_or_default();
            }
            self.selected = 0;
            if previous_height != next_height
                && self.line_jump.is_none()
                && self.input_find.is_none()
            {
                window.resize(size(px(WIDTH), px(next_height)));
            }
        }
        self.restore_query_result_position(query, window, cx);
        if let Some(origin) = self
            .file_search_return
            .take()
            .filter(|origin| origin.query == query)
        {
            if matches!(self.results, Results::LocalFilesLoading) {
                self.file_search_return = Some(origin);
            } else if let Some(index) = self.results.local_file_index(&origin.path) {
                self.selected = index;
            }
        }
        let query = if refilled { "" } else { query.trim() };
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
                        let previous = this
                            .results
                            .local_file_path(this.selected)
                            .map(str::to_owned);
                        let query = this.input.read(cx).text();
                        this.refresh_results(&query, window, cx);
                        if let Some(index) = previous
                            .as_deref()
                            .and_then(|path| this.results.local_file_index(path))
                        {
                            this.selected = index;
                        }
                    }
                });
            });
        })
        .detach();
    }

    fn open_destination(
        &mut self,
        destination: Destination,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let (Some((Scope::RecentFilePreviews, _)), Destination::LocalFile { path, .. }) =
            (scoped_query(self.input.read(cx).text_ref()), &destination)
        {
            if let Some(index) = self.results.local_file_index(path) {
                self.selected = index;
                self.load_selected_text_file(window, cx);
            }
            return;
        }
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

    fn selected_known_folder(&self) -> Option<SearchFolder> {
        let Results::Destinations(items) = &self.results else {
            return None;
        };
        match items.get(self.selected) {
            Some(Destination::KnownFolder(folder)) => Some(*folder),
            _ => None,
        }
    }

    fn use_known_folder_for_file_search(
        &self,
        folder: SearchFolder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let original = self.input.read(cx).text();
        cx.spawn_in(window, async move |this, cx| {
            let path = cx
                .background_executor()
                .spawn(async move { folder.kind.filesystem_path() })
                .await;
            let _ = cx.update(|window, cx| {
                let _ = this.update(cx, |this, cx| {
                    if this.input.read(cx).text() != original
                        || this.selected_known_folder() != Some(folder)
                    {
                        return;
                    }
                    match path {
                        Ok(path) => this.fill_query(format!("文件: {path}"), window, cx),
                        Err(error) => {
                            this.results = Results::LocalFilesUnavailable(error);
                            this.selected = 0;
                            window
                                .resize(size(px(WIDTH), px(this.results.height(MAX_VALUE_LINES))));
                            cx.notify();
                        }
                    }
                });
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

    fn prepare_value_copy(&self, value: &ResultValue, cx: &Context<Self>) -> ClipboardItem {
        self.remember_value(cx);
        self.index
            .recent
            .queries
            .record_exact(&self.input.read(cx).text(), "");
        ClipboardItem::new_string(value.text.clone())
    }

    fn prepare_in_place_copy(&mut self, cx: &mut Context<Self>) -> Option<ClipboardItem> {
        let Results::Value(Ok(value)) = &self.results else {
            return None;
        };
        let item = self.prepare_value_copy(value, cx);
        self.result_copied = true;
        cx.notify();
        Some(item)
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match &self.results {
            Results::Value(Ok(result)) => {
                let item = self.prepare_value_copy(result, cx);
                cx.write_to_clipboard(item);
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

    fn copy_table(
        &self,
        format: TableCopy,
        is_held: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(table) = self.results.table_preview() else {
            return false;
        };
        if matches!(format, TableCopy::CurrentPage | TableCopy::CurrentPageTsv)
            && (self.expanded_value_pages.is_some() || !table.spans_pages())
        {
            return false;
        }
        if !is_held {
            let text = match format {
                TableCopy::CurrentPage => {
                    table.page_csv(self.table_pages.rows, self.table_pages.columns)
                }
                TableCopy::CurrentPageTsv => {
                    table.page_tsv(self.table_pages.rows, self.table_pages.columns)
                }
                TableCopy::Tsv => table.tsv(),
            };
            self.index
                .recent
                .queries
                .record_exact(&self.input.read(cx).text(), "");
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            window.remove_window();
        }
        true
    }

    fn copy_expanded_value_page(
        &self,
        is_held: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(pages) = self
            .expanded_value_pages
            .as_ref()
            .filter(|pages| pages.len() > 1)
        else {
            return false;
        };
        let Results::Value(Ok(output)) = &self.results else {
            return false;
        };
        let Some(text) = value_preview::page_text(&output.text, pages, self.selected) else {
            return false;
        };
        if !is_held {
            self.index
                .recent
                .queries
                .record_exact(&self.input.read(cx).text(), "");
            cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
            window.remove_window();
        }
        true
    }

    fn file_operation_in_progress(&self) -> bool {
        matches!(self.result_save, ResultSaveState::InProgress)
            || matches!(self.input_file_load, InputFileLoadState::InProgress)
    }

    fn load_input_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.input.read(cx);
        let query = input.text();
        if self.file_operation_in_progress()
            || self.bookmark_name_edit.is_some()
            || input.is_composing()
            || !accepts_input_file(&query)
        {
            return;
        }
        let selection = input.selection();
        let choose_tool = query.trim().is_empty();
        let character_limit = if choose_tool {
            INPUT_CHARACTER_LIMIT
        } else {
            let retained =
                query[..selection.start].chars().count() + query[selection.end..].chars().count();
            INPUT_CHARACTER_LIMIT.saturating_sub(retained)
        };
        self.input_file_load = InputFileLoadState::InProgress;
        let picker = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("载入文本到快速搜索".into()),
        });
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let loaded = async {
                let paths = picker
                    .await
                    .map_err(|error| error.to_string())?
                    .map_err(|error| error.to_string())?;
                let Some(path) = paths.and_then(|mut paths| paths.pop()) else {
                    return Ok(None);
                };
                cx.background_executor()
                    .spawn(async move {
                        let prefix = if choose_tool {
                            input_file_query_prefix(&path)
                        } else {
                            ""
                        };
                        let max_text = character_limit.saturating_sub(prefix.chars().count());
                        let mut text = read_input_file(&path, max_text)?;
                        text.insert_str(0, prefix);
                        Ok(text)
                    })
                    .await
                    .map(Some)
            }
            .await;
            let _ = cx.update(|window, cx| {
                let _ = this.update(cx, |this, cx| {
                    let input = this.input.read(cx);
                    this.input_file_load = if this.bookmark_name_edit.is_some()
                        || input.is_composing()
                        || input.text() != query
                        || input.selection() != selection
                    {
                        InputFileLoadState::Ready
                    } else {
                        match loaded {
                            Ok(Some(text)) if choose_tool => {
                                this.fill_query(text, window, cx);
                                InputFileLoadState::Ready
                            }
                            Ok(Some(text)) => {
                                this.input.update(cx, |input, cx| {
                                    input.replace_text_in_range(None, &text, window, cx);
                                });
                                InputFileLoadState::Ready
                            }
                            Ok(None) => InputFileLoadState::Ready,
                            Err(error) => InputFileLoadState::Failed(error),
                        }
                    };
                    cx.notify();
                });
            });
        })
        .detach();
    }

    fn load_selected_text_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self
            .results
            .selected_text_file(self.selected)
            .map(str::to_owned)
        else {
            return;
        };
        self.read_text_file_preview(path, FilePreviewAction::Load, window, cx);
    }

    fn file_preview_path(&self) -> Option<&str> {
        match &self.result_refill.as_ref()?.kind {
            RefillKind::File { path } => Some(path),
            RefillKind::Result { .. } => None,
        }
    }

    fn read_text_file_preview(
        &mut self,
        path: String,
        action: FilePreviewAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = self.input.read(cx);
        if self.file_operation_in_progress()
            || self.bookmark_name_edit.is_some()
            || input.is_composing()
        {
            return;
        }
        let query = input.text();
        let revision = input.edit_revision();
        self.input_file_load = InputFileLoadState::InProgress;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let selected_path = path.clone();
            let loaded = cx
                .background_executor()
                .spawn(async move { read_selected_text_query(&path) })
                .await;
            let _ = cx.update(|window, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.input_file_load = InputFileLoadState::Ready;
                    let input = this.input.read(cx);
                    if !input.is_composing()
                        && this.bookmark_name_edit.is_none()
                        && this.line_jump.is_none()
                        && this.input_find.is_none()
                        && input.edit_revision() == revision
                    {
                        let original = match action {
                            FilePreviewAction::Load => this
                                .results
                                .selected_text_file(this.selected)
                                .filter(|path| *path == selected_path)
                                .map(|_| query.clone()),
                            FilePreviewAction::Refresh => this
                                .result_refill
                                .as_ref()
                                .filter(|refill| {
                                    refill.inserted == query
                                        && this.file_preview_path() == Some(selected_path.as_str())
                                })
                                .map(|refill| refill.original.clone()),
                        };
                        if let Some(original) = original {
                            match loaded {
                                Ok(inserted)
                                    if action == FilePreviewAction::Load || inserted != query =>
                                {
                                    let selection = input.selection_position();
                                    if action == FilePreviewAction::Load {
                                        this.index.recent.file_previews.record(&selected_path);
                                    }
                                    this.result_refill = Some(ResultRefill {
                                        original,
                                        inserted: inserted.clone(),
                                        kind: RefillKind::File {
                                            path: selected_path,
                                        },
                                    });
                                    this.fill_query(inserted, window, cx);
                                    if action == FilePreviewAction::Refresh {
                                        this.input.update(cx, |input, cx| {
                                            input.restore_selection(selection, cx);
                                        });
                                    }
                                }
                                Ok(_) => {}
                                Err(error) => {
                                    this.input_file_load = InputFileLoadState::Failed(error)
                                }
                            }
                        }
                    }
                    cx.notify();
                });
            });
        })
        .detach();
    }

    fn save_result(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_operation_in_progress() {
            return;
        }
        let Results::Value(Ok(result)) = &self.results else {
            return;
        };
        let original_query = self.input.read(cx).text();
        let name = result.suggested_file_name(&original_query);
        let text = result.text.clone();
        self.result_save = ResultSaveState::InProgress;
        let receiver = cx.prompt_for_new_path(Path::new(""), Some(name));
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let saved = async {
                let path = receiver
                    .await
                    .map_err(|error| error.to_string())?
                    .map_err(|error| error.to_string())?;
                let Some(path) = path else {
                    return Ok(false);
                };
                cx.background_executor()
                    .spawn(async move {
                        crate::settings::atomic_write_labeled(&path, text.as_bytes(), "搜索结果")
                    })
                    .await?;
                Ok::<_, String>(true)
            }
            .await;
            let _ = cx.update(|_, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.result_save = if this.input.read(cx).text() == original_query {
                        match saved {
                            Ok(true) => ResultSaveState::Finished {
                                query: original_query,
                                result: Ok(()),
                            },
                            Ok(false) => ResultSaveState::Ready,
                            Err(error) => ResultSaveState::Finished {
                                query: original_query,
                                result: Err(error),
                            },
                        }
                    } else {
                        ResultSaveState::Ready
                    };
                    cx.notify();
                });
            });
        })
        .detach();
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

    fn refill_result(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(refill) = self.result_refill.take() {
            if let RefillKind::File { path } = refill.kind {
                self.file_search_return = Some(FileSearchReturn {
                    query: refill.original.clone(),
                    path,
                });
            }
            self.fill_query(refill.original, window, cx);
        } else if let Some(text) = self.results.refillable_text() {
            self.refill_result_text(text.to_owned(), window, cx);
        }
    }

    fn refill_result_text(
        &mut self,
        inserted: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let original = self.input.read(cx).text();
        self.index.recent.queries.record_exact(&original, "");
        self.result_refill = Some(ResultRefill {
            original,
            inserted: inserted.clone(),
            kind: RefillKind::Result { evaluated: false },
        });
        self.input
            .update(cx, |input, cx| input.set_text(inserted, cx));
        window.focus(&self.input.focus_handle(cx));
    }

    fn next_calculation_mode(&self, cx: &App) -> Option<CalculationMode> {
        if self.input.read(cx).text().starts_with(EXACT_MODE_PREFIX) {
            Some(CalculationMode::Decimal)
        } else if matches!(
            &self.results,
            Results::Value(Ok(ResultValue {
                kind: ValueKind::Calculation,
                ..
            })) | Results::Value(Err(ValueError::Calculation(
                CalculationError::OutOfRange | CalculationError::FunctionDomain
            )))
        ) {
            Some(CalculationMode::Exact)
        } else {
            None
        }
    }

    fn switch_calculation_mode(
        &self,
        mode: CalculationMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (prefix, replacement) = match mode {
            CalculationMode::Decimal => (EXACT_MODE_PREFIX, ""),
            CalculationMode::Exact => ("", EXACT_MODE_PREFIX),
        };
        self.input.update(cx, |input, cx| {
            input.replace_leading_text(prefix, replacement, window, cx);
        });
        window.focus(&self.input.focus_handle(cx));
    }

    fn refill_decimal_preview(
        &mut self,
        is_held: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Results::Value(Ok(value)) = &self.results else {
            return false;
        };
        let Some(decimal) = value.exact_decimal_preview() else {
            return false;
        };
        if !is_held {
            self.refill_result_text(decimal.digits.clone(), window, cx);
        }
        true
    }

    fn searchable_result_text(&self) -> Option<(&'static str, &str)> {
        let Results::Value(Ok(ResultValue { text, kind })) = &self.results else {
            return None;
        };
        let (command, extra) = match kind {
            ValueKind::Table(_) => (TABLE_SEARCH_COMMAND, 3),
            ValueKind::Text if text_tools::logical_newline_pattern().is_match(text) => {
                (TEXT_SEARCH_COMMAND, 5)
            }
            _ => return None,
        };
        let available = INPUT_CHARACTER_LIMIT - command.chars().count() - extra;
        (text.chars().take(available + 1).count() <= available).then_some((command, text.as_str()))
    }

    fn toggle_result_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.input.read(cx).is_composing() {
            return;
        }
        if let Some(draft) = self.input_before_result_search.take() {
            self.restore_query_draft(draft.original, window, cx);
        } else if let Some((command, text)) = self.searchable_result_text() {
            let query = if command == TABLE_SEARCH_COMMAND {
                format!("{command}\"\" {text}")
            } else {
                format!("{command}\"\" \"{text}\"")
            };
            let caret = command.len() + 1;
            self.open_result_tool(command, query, caret..caret, window, cx);
        }
    }

    fn open_result_tool(
        &mut self,
        command: &'static str,
        query: String,
        selection: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.input_before_result_search = Some(ResultSearchDraft {
            original: self.query_draft(cx),
            command,
        });
        self.fill_query(query, window, cx);
        self.input
            .update(cx, |input, cx| input.select_range(selection, cx));
    }

    fn is_table_cell_lookup(&self) -> bool {
        self.input_before_result_search
            .as_ref()
            .is_some_and(|draft| draft.command == TABLE_CELL_COMMAND)
    }

    fn table_cell_source(&self) -> Option<(&str, String)> {
        if self.input_before_result_search.is_some() || self.expanded_value_pages.is_some() {
            return None;
        }
        let Results::Value(Ok(ResultValue {
            text,
            kind: ValueKind::Table(table),
        })) = &self.results
        else {
            return None;
        };
        let (row, column) =
            table.page_first_data_cell(self.table_pages.rows, self.table_pages.columns)?;
        let coordinates = format!("{row},{column}");
        let available =
            INPUT_CHARACTER_LIMIT - TABLE_CELL_COMMAND.chars().count() - coordinates.len() - 3;
        (text.chars().take(available + 1).count() <= available)
            .then_some((text.as_str(), coordinates))
    }

    fn toggle_table_cell(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.input.read(cx).is_composing() || self.file_operation_in_progress() {
            return;
        }
        if self.is_table_cell_lookup() {
            self.toggle_result_search(window, cx);
        } else if let Some((text, coordinates)) = self.table_cell_source() {
            let query = format!("{TABLE_CELL_COMMAND}[{coordinates}] {text}");
            let start = TABLE_CELL_COMMAND.len() + 1;
            let selection = start..start + coordinates.len();
            self.open_result_tool(TABLE_CELL_COMMAND, query, selection, window, cx);
        }
    }

    fn has_refilled_input(&self, query: &str) -> bool {
        self.result_refill.as_ref().is_some_and(|refill| {
            matches!(refill.kind, RefillKind::Result { evaluated: false })
                && refill.inserted == query
        })
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

    fn has_temporary_query(&self) -> bool {
        self.input_before_help.is_some()
            || self.input_before_calculation_history.is_some()
            || self.input_before_query_history.is_some()
            || self.input_before_bookmarks.is_some()
            || self.input_before_result_search.is_some()
    }

    fn query_draft(&self, cx: &Context<Self>) -> QueryDraft {
        let input = self.input.read(cx);
        QueryDraft {
            text: input.text(),
            selection: input.selection_position(),
            selected: self.selected,
            suggestion: self
                .results
                .suggested_input(self.selected)
                .map(str::to_owned),
            table_pages: self.results.table_preview().map(|_| self.table_pages),
            expanded_value: self.expanded_value_pages.is_some(),
        }
    }

    fn restore_query_draft(
        &mut self,
        draft: QueryDraft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = draft.text.clone();
        let selection = draft.selection;
        self.query_draft_return = Some(draft);
        self.input.update(cx, |input, cx| {
            input.set_text(text, cx);
            input.restore_selection(selection, cx);
        });
        window.focus(&self.input.focus_handle(cx));
    }

    fn restore_query_result_position(
        &mut self,
        query: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(draft) = self
            .query_draft_return
            .take()
            .filter(|draft| draft.text == query)
        else {
            return;
        };
        if draft.expanded_value != self.expanded_value_pages.is_some() {
            self.toggle_value_preview(window, cx);
        }
        let count = self
            .expanded_value_pages
            .as_ref()
            .map_or_else(|| self.results.selectable_count(), Vec::len);
        self.selected = draft
            .suggestion
            .as_deref()
            .and_then(|suggestion| {
                (0..count).find(|&index| self.results.suggested_input(index) == Some(suggestion))
            })
            .unwrap_or_else(|| draft.selected.min(count.saturating_sub(1)));
        if let (Some(pages), Some(table)) = (draft.table_pages, self.results.table_preview()) {
            self.table_pages = pages.clamped_to(table);
        }
    }

    fn toggle_calculation_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = self.query_draft(cx);
        let query = draft.text();
        const PREFIX: &str = "算式历史: ";
        match scoped_query(query.trim()) {
            Some((Scope::CalculationHistory, term)) => {
                if let Some(previous) = self.input_before_calculation_history.take() {
                    self.restore_query_draft(previous, window, cx);
                } else {
                    let filter = query.strip_prefix(PREFIX).unwrap_or(term);
                    self.fill_query(filter.to_owned(), window, cx);
                }
            }
            _ => {
                let filter: String = query
                    .chars()
                    .take(INPUT_CHARACTER_LIMIT - PREFIX.chars().count())
                    .collect();
                self.input_before_calculation_history = Some(draft);
                self.fill_query(format!("{PREFIX}{filter}"), window, cx);
            }
        }
    }

    fn is_query_history(query: &str) -> bool {
        matches!(scoped_query(query), Some((Scope::QueryHistory, _)))
    }

    fn toggle_query_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = self.query_draft(cx);
        let query = draft.text();
        if Self::is_bookmark_query(query) {
            if let Some(previous) = self
                .input_before_bookmarks
                .take_if(|input| Self::is_query_history(input.text()))
            {
                self.restore_query_draft(previous, window, cx);
                return;
            }
        }
        if Self::is_query_history(query) {
            if let Some(previous) = self.input_before_query_history.take() {
                self.restore_query_draft(previous, window, cx);
            }
        } else if Self::shows_help(query, &self.results)
            && self
                .input_before_help
                .as_ref()
                .map(QueryDraft::text)
                .is_some_and(Self::is_query_history)
        {
            self.toggle_help(window, cx);
        } else {
            self.input_before_query_history = Some(draft);
            self.fill_query("查询历史: ", window, cx);
        }
    }

    fn remove_selected_query_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Results::Queries(items) = &self.results else {
            return;
        };
        let selected = self.selected;
        let Some(item) = items.get(selected) else {
            return;
        };
        self.index.recent.queries.remove_exact(&item.expression);
        let query = self.input.read(cx).text();
        self.refresh_results(&query, window, cx);
        if let Results::Queries(items) = &self.results {
            self.selected = selected.min(items.len().saturating_sub(1));
        }
        cx.notify();
    }

    fn is_bookmark_query(query: &str) -> bool {
        matches!(scoped_query(query), Some((Scope::Bookmarks, _)))
    }

    fn toggle_bookmark_browser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = self.query_draft(cx);
        let query = draft.text();
        if Self::is_query_history(query) {
            if let Some(previous) = self
                .input_before_query_history
                .take_if(|input| Self::is_bookmark_query(input.text()))
            {
                self.restore_query_draft(previous, window, cx);
                return;
            }
        }
        if Self::is_bookmark_query(query) {
            if let Some(previous) = self.input_before_bookmarks.take() {
                self.restore_query_draft(previous, window, cx);
            }
        } else if Self::shows_help(query, &self.results)
            && self
                .input_before_help
                .as_ref()
                .map(QueryDraft::text)
                .is_some_and(Self::is_bookmark_query)
        {
            self.toggle_help(window, cx);
        } else {
            self.input_before_bookmarks = Some(draft);
            self.fill_query("收藏: ", window, cx);
        }
    }

    fn toggle_bookmark(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.input.read(cx).text();
        if query.trim().is_empty() {
            self.toggle_bookmark_browser(window, cx);
            return;
        }
        let Some(target) = self.results.bookmark_query(&query, self.selected) else {
            return;
        };
        let saved = self.index.recent.bookmarks.toggle(target);
        self.bookmark_limit_reached = matches!(&saved, Ok(false));
        self.bookmark_save_error = saved.err();
        if matches!(self.results, Results::Bookmarks(_)) {
            let selected = self.selected;
            self.refresh_results(&query, window, cx);
            if let Results::Bookmarks(items) = &self.results {
                self.selected = selected.min(items.len().saturating_sub(1));
            }
        }
        cx.notify();
    }

    fn move_selected_bookmark(&mut self, down: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Results::Bookmarks(items) = &self.results else {
            return;
        };
        let selected = self.selected;
        let neighbor = if down {
            selected.checked_add(1)
        } else {
            selected.checked_sub(1)
        };
        let (Some(current), Some(neighbor)) = (
            items.get(selected),
            neighbor.and_then(|index| items.get(index)),
        ) else {
            return;
        };
        let expression = current.expression.clone();
        let (first, second) = if down {
            (&expression, &neighbor.expression)
        } else {
            (&neighbor.expression, &expression)
        };
        self.bookmark_save_error = self
            .index
            .recent
            .bookmarks
            .swap_visible_items(first, second)
            .err();
        let query = self.input.read(cx).text();
        self.refresh_results(&query, window, cx);
        if let Results::Bookmarks(items) = &self.results {
            self.selected = items
                .iter()
                .position(|item| item.expression == expression)
                .unwrap_or_else(|| selected.min(items.len().saturating_sub(1)));
        }
        cx.notify();
    }

    fn start_bookmark_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Results::Bookmarks(items) = &self.results else {
            return;
        };
        let Some(item) = items.get(self.selected) else {
            return;
        };
        let name = item.result.clone();
        self.bookmark_name_edit = Some(BookmarkNameEdit {
            expression: item.expression.clone(),
            filter: self.input.read(cx).text(),
        });
        self.results = Results::BookmarkNameInput(None);
        self.input
            .update(cx, |input, _| input.set_placeholder("输入收藏名"));
        self.fill_query(name, window, cx);
        window.resize(size(px(WIDTH), px(self.results.height(MAX_VALUE_LINES))));
        cx.notify();
    }

    fn finish_bookmark_name(&mut self, save: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.bookmark_name_edit.take() else {
            return;
        };
        if save {
            let name = self.input.read(cx).text();
            if let Err(error) = self.index.recent.bookmarks.rename(&edit.expression, &name) {
                self.bookmark_name_edit = Some(edit);
                self.results = Results::BookmarkNameInput(Some(error));
                cx.notify();
                return;
            }
        }
        self.input
            .update(cx, |input, _| input.set_placeholder(SEARCH_PLACEHOLDER));
        self.fill_query(edit.filter.clone(), window, cx);
        self.refresh_results(&edit.filter, window, cx);
    }

    fn shows_help(query: &str, results: &Results) -> bool {
        query.trim_start().starts_with(['?', '？']) || matches!(results, Results::FileOptions(_))
    }

    fn toggle_help(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.input.read(cx).is_composing() {
            return;
        }
        let draft = self.query_draft(cx);
        let query = draft.text();
        if Self::shows_help(query, &self.results) {
            if let Some(previous) = self.input_before_help.take() {
                self.restore_query_draft(previous, window, cx);
            }
            return;
        }
        let selected_query = self.results.suggested_input(self.selected);
        let help = match scoped_query(query.trim()) {
            Some((Scope::LocalFiles, term)) => local_files::option_help_query(term),
            Some((Scope::ToolIndex, _)) => selected_query
                .and_then(|query| query.split_once([':', '：']))
                .and_then(|(prefix, _)| tool_example_keyword(prefix))
                .map(|keyword| format!("? {keyword}")),
            Some((Scope::ExampleHistory | Scope::QueryHistory | Scope::Bookmarks, _)) => {
                selected_query.and_then(example_help_query)
            }
            _ => None,
        }
        .or_else(|| example_help_query(query));
        self.input_before_help = Some(draft);
        self.fill_query(help.unwrap_or_else(|| "?".to_owned()), window, cx);
    }

    fn select_example_range(
        &self,
        selection_range: fn(&Example) -> Option<Range<usize>>,
        is_held: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Results::Help(items) = &self.results else {
            return false;
        };
        let Some(&index) = items.get(self.selected) else {
            return false;
        };
        let example = &EXAMPLES[index];
        let Some(range) = selection_range(example) else {
            return false;
        };
        if !is_held {
            self.index.recent.examples.record(index);
            self.fill_query(example.query, window, cx);
            self.input
                .update(cx, |input, cx| input.select_range(range, cx));
        }
        true
    }

    fn batch_error_range(&self, cx: &App) -> Option<Range<usize>> {
        let line = match &self.results {
            Results::Value(Err(ValueError::BatchCalculation(BatchError::Row { line, .. })))
            | Results::Value(Err(ValueError::BatchFraction(BatchError::Row { line, .. }))) => *line,
            _ => return None,
        };
        let query = self.input.read(cx).text_ref();
        if !matches!(scoped_query(query), Some((Scope::BatchCalculation, _))) {
            return None;
        }
        let (_, term) = query.split_once([':', '：'])?;
        let (_, input) = batch_calculation_input(term.trim_start());
        let start = query.len() - input.len();
        batch_line_range(input, line).map(|range| start + range.start..start + range.end)
    }

    fn table_input_error_range(&self, cx: &App) -> Option<Range<usize>> {
        let errors = self.results.table_preview()?.input_errors();
        let first = errors.first()?;
        let selection = self.input.read(cx).selection();
        Some(
            errors
                .iter()
                .skip_while(|range| *range != &selection)
                .nth(1)
                .unwrap_or(first)
                .clone(),
        )
    }

    fn validation_error_range(&self, cx: &App) -> Option<Range<usize>> {
        let Results::Value(Ok(ResultValue {
            kind: ValueKind::InputValidationError { scope, range },
            ..
        })) = &self.results
        else {
            return None;
        };
        let query = self.input.read(cx).text_ref();
        let (query_scope, _) = scoped_query(query)?;
        if query_scope != *scope {
            return None;
        }
        let (_, term) = query.split_once([':', '：'])?;
        let offset = query.len() - term.trim_start().len();
        Some(offset + range.start..offset + range.end)
    }

    fn select_input_error(
        &self,
        is_held: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(range) = self
            .batch_error_range(cx)
            .or_else(|| self.table_input_error_range(cx))
            .or_else(|| self.validation_error_range(cx))
        else {
            return false;
        };
        if !is_held {
            self.input
                .update(cx, |input, cx| input.select_range(range, cx));
            window.focus(&self.input.focus_handle(cx));
        }
        true
    }

    fn select_query_arguments(
        &self,
        is_held: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(range) = query_argument_range(self.input.read(cx).text_ref()) else {
            return false;
        };
        if !is_held {
            self.input
                .update(cx, |input, cx| input.select_range(range, cx));
            window.focus(&self.input.focus_handle(cx));
        }
        true
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(find) = &self.input_find {
            let key = &event.keystroke;
            let modifiers = key.modifiers.number_of_modifiers();
            let replacement = find.replacement.as_ref().map(|entry| &entry.input);
            if find.input.read(cx).is_composing()
                || replacement.is_some_and(|entry| entry.read(cx).is_composing())
            {
                return;
            }
            if let Some(replacement) = replacement.filter(|_| {
                key.key == "tab" && (modifiers == 0 || (modifiers == 1 && key.modifiers.shift))
            }) {
                if !event.is_held {
                    let needle = find.input.focus_handle(cx);
                    let next = if needle.is_focused(window) {
                        replacement.focus_handle(cx)
                    } else {
                        needle
                    };
                    window.focus(&next);
                    cx.notify();
                }
                cx.stop_propagation();
                return;
            }
            let cancels = key.key == "escape" && modifiers == 0;
            let confirms = key.key == "enter"
                && (modifiers == 0
                    || (replacement.is_none() && modifiers == 1 && key.modifiers.shift));
            if cancels || confirms {
                if !event.is_held {
                    if confirms && replacement.is_some() {
                        self.replace_input_matches(window, cx);
                    } else {
                        self.finish_input_find(confirms.then_some(key.modifiers.shift), window, cx);
                    }
                }
                cx.stop_propagation();
            }
            return;
        }
        if let Some(jump) = &self.line_jump {
            let key = &event.keystroke;
            if !jump.input.read(cx).is_composing()
                && key.modifiers.number_of_modifiers() == 0
                && matches!(key.key.as_str(), "enter" | "escape")
            {
                if !event.is_held {
                    self.finish_line_jump(key.key == "enter", window, cx);
                }
                cx.stop_propagation();
            }
            return;
        }
        if self.input.read(cx).is_composing() {
            return;
        }
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers.number_of_modifiers();
        if keystroke.modifiers.control {
            let redo = match (keystroke.key.as_str(), modifiers, keystroke.modifiers.shift) {
                ("z", 1, false) => Some(false),
                ("y", 1, false) | ("z", 2, true) => Some(true),
                _ => None,
            };
            if let Some(redo) = redo {
                self.input
                    .update(cx, |input, cx| input.step_edit_history(redo, cx));
                cx.stop_propagation();
                return;
            }
        }
        if self.bookmark_name_edit.is_some() {
            if modifiers == 0 && matches!(keystroke.key.as_str(), "enter" | "escape") {
                if !event.is_held {
                    self.finish_bookmark_name(keystroke.key == "enter", window, cx);
                }
                cx.stop_propagation();
            }
            return;
        }
        if modifiers != 0 {
            if modifiers == 2
                && keystroke.modifiers.control
                && keystroke.modifiers.shift
                && keystroke.key == "r"
            {
                if !event.is_held && !self.previous_input.query.is_empty() {
                    self.fill_query(self.previous_input.query.clone(), window, cx);
                    if let Some(selection) = self.previous_input.selection {
                        self.input
                            .update(cx, |input, cx| input.restore_selection(selection, cx));
                    }
                }
                cx.stop_propagation();
            } else if modifiers == 2
                && keystroke.modifiers.control
                && keystroke.modifiers.shift
                && match keystroke.key.as_str() {
                    "c" => {
                        self.copy_table(TableCopy::CurrentPage, event.is_held, window, cx)
                            || self.copy_expanded_value_page(event.is_held, window, cx)
                    }
                    "t" => self.copy_table(TableCopy::Tsv, event.is_held, window, cx),
                    "enter" => {
                        self.copy_table(TableCopy::CurrentPageTsv, event.is_held, window, cx)
                    }
                    _ => false,
                }
            {
                cx.stop_propagation();
            } else if modifiers == 1 && keystroke.modifiers.control && keystroke.key == "enter" {
                if event.is_held {
                    if matches!(self.results, Results::Value(Ok(_))) {
                        cx.stop_propagation();
                    }
                } else if let Some(item) = self.prepare_in_place_copy(cx) {
                    cx.write_to_clipboard(item);
                    cx.stop_propagation();
                }
            } else if modifiers == 1 && keystroke.modifiers.shift && keystroke.key == "enter" {
                if !event.is_held {
                    self.input.update(cx, |input, cx| {
                        input.replace_text_in_range(None, "\n", window, cx);
                    });
                }
                cx.stop_propagation();
            } else if modifiers == 1 && keystroke.modifiers.shift && keystroke.key == "tab" {
                if self.select_example_range(Example::argument_range, event.is_held, window, cx) {
                    cx.stop_propagation();
                }
            } else if modifiers == 2
                && keystroke.modifiers.control
                && keystroke.modifiers.shift
                && keystroke.key == "k"
            {
                if !event.is_held {
                    self.input
                        .update(cx, |input, cx| input.delete_logical_line(cx));
                }
                cx.stop_propagation();
            } else if modifiers == 2
                && keystroke.modifiers.control
                && keystroke.modifiers.shift
                && keystroke.key == "d"
            {
                if !event.is_held {
                    self.input
                        .update(cx, |input, cx| input.duplicate_logical_line(cx));
                }
                cx.stop_propagation();
            } else if modifiers == 2
                && keystroke.modifiers.control
                && keystroke.modifiers.shift
                && keystroke.key == "a"
                && self.select_query_arguments(event.is_held, window, cx)
            {
                cx.stop_propagation();
            } else if modifiers == 2
                && keystroke.modifiers.alt
                && keystroke.modifiers.shift
                && matches!(keystroke.key.as_str(), "left" | "right")
            {
                let input = self.input.read(cx);
                if let Some(range) = neighboring_function_argument(
                    input.text_ref(),
                    input.selection(),
                    keystroke.key == "left",
                ) {
                    if !event.is_held {
                        self.input
                            .update(cx, |input, cx| input.select_range(range, cx));
                    }
                    cx.stop_propagation();
                }
            } else if keystroke.modifiers.control
                && ((modifiers == 2 && keystroke.modifiers.shift && keystroke.key == "f")
                    || (modifiers == 1 && keystroke.key == "h"))
            {
                if !event.is_held {
                    self.open_input_find(keystroke.key == "h", window, cx);
                }
                if self.input_find.is_some() {
                    cx.stop_propagation();
                }
            } else if modifiers == 1 && keystroke.modifiers.control && keystroke.key == "l" {
                if !event.is_held {
                    self.input
                        .update(cx, |input, cx| input.select_current_line(cx));
                }
                cx.stop_propagation();
            } else if keystroke.modifiers.control
                && (modifiers == 1
                    || (modifiers == 2 && (keystroke.modifiers.shift || keystroke.modifiers.alt)))
                && keystroke.key == "g"
            {
                if !event.is_held {
                    let target = if keystroke.modifiers.shift {
                        LineJumpTarget::Table(TableAxis::Rows)
                    } else if keystroke.modifiers.alt {
                        LineJumpTarget::Table(TableAxis::Columns)
                    } else {
                        LineJumpTarget::Input
                    };
                    self.open_line_jump(target, window, cx);
                }
                if self.line_jump.is_some() {
                    cx.stop_propagation();
                }
            } else if keystroke.modifiers.control
                && (modifiers == 1 || (modifiers == 2 && keystroke.modifiers.shift))
                && matches!(keystroke.key.as_str(), "up" | "down")
            {
                self.input.update(cx, |input, cx| {
                    input.jump_logical_line(keystroke.key == "down", keystroke.modifiers.shift, cx);
                });
                cx.stop_propagation();
            } else if modifiers == 1 && keystroke.modifiers.alt && keystroke.key == "f2" {
                if let Some(mode) = self.next_calculation_mode(cx) {
                    if !event.is_held {
                        self.switch_calculation_mode(mode, window, cx);
                    }
                    cx.stop_propagation();
                }
            } else if keystroke.modifiers.alt
                && (modifiers == 1 || (modifiers == 2 && keystroke.modifiers.shift))
                && matches!(keystroke.key.as_str(), "home" | "end")
            {
                self.input.update(cx, |input, cx| {
                    input.move_to_line_edge(keystroke.key == "end", keystroke.modifiers.shift, cx);
                });
                cx.stop_propagation();
            } else if modifiers == 1
                && keystroke.modifiers.shift
                && keystroke.key == "f3"
                && self.refill_decimal_preview(event.is_held, window, cx)
            {
                cx.stop_propagation();
            } else if modifiers == 1
                && keystroke.modifiers.control
                && keystroke.key == "f"
                && (self.input_before_result_search.is_some()
                    || self.searchable_result_text().is_some())
            {
                if !event.is_held {
                    self.toggle_result_search(window, cx);
                }
                cx.stop_propagation();
            } else if modifiers == 2
                && keystroke.modifiers.control
                && keystroke.modifiers.shift
                && keystroke.key == "j"
                && (self.is_table_cell_lookup() || self.table_cell_source().is_some())
            {
                if !event.is_held {
                    self.toggle_table_cell(window, cx);
                }
                cx.stop_propagation();
            } else if modifiers == 1 && keystroke.modifiers.control && keystroke.key == "d" {
                if !event.is_held {
                    self.toggle_bookmark(window, cx);
                }
                cx.stop_propagation();
            } else if modifiers == 1 && keystroke.modifiers.control && keystroke.key == "r" {
                if !event.is_held {
                    self.toggle_calculation_history(window, cx);
                }
                cx.stop_propagation();
            } else if modifiers == 1
                && keystroke.modifiers.control
                && keystroke.key == "o"
                && accepts_input_file(&self.input.read(cx).text())
            {
                if !event.is_held {
                    self.load_input_file(window, cx);
                }
                cx.stop_propagation();
            } else if modifiers == 1
                && keystroke.modifiers.control
                && keystroke.key == "s"
                && matches!(&self.results, Results::Value(Ok(_)))
            {
                if !event.is_held {
                    self.save_result(window, cx);
                }
                cx.stop_propagation();
            } else if keystroke.modifiers.shift
                && (modifiers == 1 || (modifiers == 2 && keystroke.modifiers.control))
                && matches!(keystroke.key.as_str(), "pageup" | "pagedown")
            {
                let page = match (keystroke.key == "pageup", keystroke.modifiers.control) {
                    (true, false) => PageChange::Previous,
                    (false, false) => PageChange::Next,
                    (true, true) => PageChange::First,
                    (false, true) => PageChange::Last,
                };
                if self.change_table_page(TableAxis::Columns, page, cx) {
                    cx.stop_propagation();
                }
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
                if self.select_example_range(
                    |example| example.data_start().map(|start| start..example.query.len()),
                    event.is_held,
                    window,
                    cx,
                ) || self.select_input_error(event.is_held, window, cx)
                    || (!event.is_held && self.reveal_selected_file(window, cx))
                {
                    cx.stop_propagation();
                }
            } else if modifiers == 1
                && keystroke.modifiers.alt
                && keystroke.key == "delete"
                && matches!(self.results, Results::Queries(_))
            {
                if !event.is_held {
                    self.remove_selected_query_history(window, cx);
                }
                cx.stop_propagation();
            } else if modifiers == 1
                && keystroke.modifiers.alt
                && matches!(keystroke.key.as_str(), "up" | "down")
                && matches!(self.results, Results::Bookmarks(_))
            {
                if !event.is_held {
                    self.move_selected_bookmark(keystroke.key == "down", window, cx);
                }
                cx.stop_propagation();
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
                    self.toggle_help(window, cx);
                }
            }
            "f2" if matches!(&self.results, Results::Bookmarks(items) if !items.is_empty()) => {
                if !event.is_held {
                    self.start_bookmark_name(window, cx);
                }
            }
            "f2" if self.results.value_preview_expandable() => {
                if !event.is_held {
                    self.toggle_value_preview(window, cx);
                }
            }
            "f3" if self.result_refill.is_some() || self.results.refillable_text().is_some() => {
                if !event.is_held {
                    self.refill_result(window, cx);
                }
            }
            "f3" if self.results.selected_text_file(self.selected).is_some() => {
                if !event.is_held {
                    self.load_selected_text_file(window, cx);
                }
            }
            "f4" => {
                if !event.is_held {
                    self.toggle_bookmark_browser(window, cx);
                }
            }
            "f5" if matches!(self.results, Results::RefilledInput) => {
                if !event.is_held && !self.file_operation_in_progress() {
                    if let Some(ResultRefill {
                        kind: RefillKind::Result { evaluated },
                        ..
                    }) = &mut self.result_refill
                    {
                        *evaluated = true;
                        let query = self.input.read(cx).text();
                        self.result_save = ResultSaveState::Ready;
                        self.refresh_results(&query, window, cx);
                    }
                }
            }
            "f5" => {
                if let Some(path) = self.file_preview_path() {
                    if !event.is_held {
                        self.read_text_file_preview(
                            path.to_owned(),
                            FilePreviewAction::Refresh,
                            window,
                            cx,
                        );
                    }
                    cx.stop_propagation();
                    return;
                }
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
                    _ if matches!(self.results, Results::Value(_)) => {
                        if !event.is_held && !self.file_operation_in_progress() {
                            self.result_save = ResultSaveState::Ready;
                            self.refresh_results(&query, window, cx);
                        }
                    }
                    _ => return,
                }
            }
            "f6" => {
                if !event.is_held {
                    self.toggle_query_history(window, cx);
                }
            }
            "enter" => {
                if !event.is_held {
                    self.confirm(window, cx);
                }
            }
            "escape" => {
                if !event.is_held {
                    window.remove_window();
                }
            }
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
                if let Some(folder) = self.selected_known_folder() {
                    if !event.is_held {
                        self.use_known_folder_for_file_search(folder, window, cx);
                    }
                    cx.stop_propagation();
                    return;
                }
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
                let count = self.results.selectable_count();
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
        if self.change_table_page(TableAxis::Rows, movement, cx) {
            return true;
        }
        let (count, page_size) = match &self.results {
            Results::Destinations(items) => (items.len(), DESTINATION_PAGE_SIZE),
            Results::ConversionTargets(items) => (items.len(), CONVERSION_PAGE_SIZE),
            Results::FileOptions(items)
            | Results::FilterOptions(items)
            | Results::Bookmarks(items)
            | Results::Calculations(items)
            | Results::Conversions(items)
            | Results::Queries(items) => (items.len(), EXAMPLE_PAGE_SIZE),
            Results::Help(items) => (items.len(), EXAMPLE_PAGE_SIZE),
            Results::Value(Ok(_)) => (self.expanded_value_pages.as_ref().map_or(0, Vec::len), 1),
            _ => return false,
        };
        if count <= page_size {
            return false;
        }
        let page = self.selected / page_size;
        let last_page = (count - 1) / page_size;
        let next = movement.target(page, last_page);
        if next != page {
            self.selected = next * page_size;
            cx.notify();
        }
        true
    }

    fn change_table_page(
        &mut self,
        axis: TableAxis,
        movement: PageChange,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.expanded_value_pages.is_some() {
            return false;
        }
        let Results::Value(Ok(ResultValue {
            kind: ValueKind::Table(table),
            ..
        })) = &self.results
        else {
            return false;
        };
        let (page, count) = match axis {
            TableAxis::Rows => (&mut self.table_pages.rows, table.row_pages()),
            TableAxis::Columns => (&mut self.table_pages.columns, table.column_pages()),
        };
        if count < 2 {
            return false;
        }
        let next = movement.target(*page, count - 1);
        if next != *page {
            *page = next;
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
        let input = self.input.read(cx).text();
        let tool_index = matches!(scoped_query(input.trim()), Some((Scope::ToolIndex, _)));
        let shortcut = items.get(self.selected).map(|&index| {
            let example = &EXAMPLES[index];
            if tool_index {
                "F1 用法 · Tab 填入"
            } else if example.data_start().is_some() {
                "Tab 填入 · Shift+Tab 改参数 · Alt+Enter 换数据"
            } else if example.argument_range().is_some() {
                "Tab 填入 · Shift+Tab 改参数"
            } else {
                "Tab 填入"
            }
        });
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
                            .child(
                                div()
                                    .id(("quick-search-example-title", index))
                                    .w(px(184.))
                                    .truncate()
                                    .child(example.title)
                                    .tooltip(theme::text_tooltip(example.title.into())),
                            )
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
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.index.recent.examples.record(example_index);
                                this.fill_query(query, window, cx);
                            }))
                    }),
            )
            .when_some(shortcut, |list, hint| {
                list.child(Self::selection_hint(hint))
            })
            .when(items.len() > EXAMPLE_PAGE_SIZE, |list| {
                list.child(self.render_pagination(
                    items.len(),
                    EXAMPLE_PAGE_SIZE,
                    if tool_index {
                        "↑ ↓ 选择  ·  输入关键词筛选  ·  PgUp / PgDn"
                    } else {
                        "↑ ↓ 选择  ·  ? 后筛选  ·  PgUp / PgDn"
                    },
                    cx,
                ))
            })
            .when((2..=EXAMPLE_PAGE_SIZE).contains(&items.len()), |list| {
                let hint = if matches!(input.trim(), "?" | "？") {
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
        let bookmarks = matches!(self.results, Results::Bookmarks(_));
        let query_history = matches!(self.results, Results::Queries(_));
        div()
            .flex()
            .flex_col()
            .children(visible.map(|(index, item)| {
                let expression = item.expression.clone();
                let (label, detail) = match item.label {
                    Some(label) => (
                        label.into(),
                        (!item.result.is_empty()).then(|| item.result.clone()),
                    ),
                    None if bookmarks && !item.result.is_empty() => (
                        item.result.clone().into(),
                        Some(input_preview(expression.clone()).to_string()),
                    ),
                    None if bookmarks || query_history => (input_preview(expression.clone()), None),
                    None => (
                        expression.clone().into(),
                        Some(format!("= {}", item.result)),
                    ),
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
                    .when_some(detail, |row, detail| {
                        row.child(
                            div()
                                .max_w(px(200.))
                                .truncate()
                                .font_family(theme::MONO_FONT)
                                .text_color(rgb(theme::ACCENT))
                                .child(detail),
                        )
                    })
                    .when(index == self.selected, |row| {
                        row.child(
                            div()
                                .text_color(rgb(theme::TEXT_MUTED))
                                .child(if bookmarks {
                                    "F2 命名 · Tab 填入"
                                } else {
                                    "Tab 填入"
                                }),
                        )
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.fill_query(expression.clone(), window, cx);
                    }))
            }))
            .when(items.len() > page_size, |list| {
                list.child(self.render_pagination(
                    items.len(),
                    page_size,
                    if bookmarks {
                        "↑ ↓ 选择  ·  Alt+↑/↓ 排序  ·  PgUp / PgDn"
                    } else {
                        "↑ ↓ 选择  ·  PgUp / PgDn"
                    },
                    cx,
                ))
            })
            .when((2..=page_size).contains(&items.len()), |list| {
                list.child(Self::selection_hint(if bookmarks {
                    "↑ ↓ 选择  ·  Alt+↑/↓ 排序"
                } else {
                    "↑ ↓ 选择"
                }))
            })
    }

    fn copy_button_label(&self, label: &'static str) -> &'static str {
        if self.result_copied {
            "已复制"
        } else {
            label
        }
    }

    fn render_visual_preview(
        &self,
        legend: Div,
        preview: Div,
        copy_label: &'static str,
        footer: Div,
        cx: &mut Context<Self>,
    ) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .h(px(38.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(legend)
                    .child(
                        button(
                            "copy-visual-result",
                            self.copy_button_label(copy_label),
                            Tone::Primary,
                            true,
                        )
                        .flex_none()
                        .on_click(cx.listener(|this, _, window, cx| this.confirm(window, cx))),
                    ),
            )
            .child(preview)
            .child(footer)
    }

    fn visual_preview_footer(view_label: &'static str, cx: &mut Context<Self>) -> Div {
        Self::result_footer().child(
            div()
                .id("visual-result-data")
                .child(view_label)
                .cursor_pointer()
                .hover(|style| style.text_color(rgb(theme::ACCENT)))
                .on_click(cx.listener(|this, _, window, cx| this.toggle_value_preview(window, cx))),
        )
    }

    fn table_page_controls(&self, axis: TableAxis, count: usize, cx: &mut Context<Self>) -> Div {
        let (page, dimension, prev_id, prev_label, prev_key, next_id, next_label, next_key) =
            match axis {
                TableAxis::Rows => (
                    self.table_pages.rows,
                    "行",
                    "table-prev-rows",
                    "上页",
                    "PageUp 向上翻页 · Ctrl+PageUp 首页",
                    "table-next-rows",
                    "下页",
                    "PageDown 向下翻页 · Ctrl+PageDown 末页",
                ),
                TableAxis::Columns => (
                    self.table_pages.columns,
                    "列",
                    "table-prev-columns",
                    "左页",
                    "Shift+PageUp 左页 · Ctrl+Shift+PageUp 第一组列",
                    "table-next-columns",
                    "右页",
                    "Shift+PageDown 右页 · Ctrl+Shift+PageDown 最后一组列",
                ),
            };
        let counter = format!("{dimension} {}/{count}", page + 1);
        let (jump_id, jump_hint) = match axis {
            TableAxis::Rows => ("jump-table-row", "Ctrl+Shift+G 跳到包含指定数据行的页"),
            TableAxis::Columns => ("jump-table-column", "Ctrl+Alt+G 跳到包含指定列的页"),
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .child(
                button(prev_id, prev_label, Tone::Ghost, page > 0)
                    .h(px(TABLE_PAGER_HEIGHT))
                    .px(px(7.))
                    .tooltip(theme::text_tooltip(prev_key.into()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.change_table_page(axis, PageChange::Previous, cx);
                    })),
            )
            .child(
                button(jump_id, counter, Tone::Ghost, true)
                    .h(px(TABLE_PAGER_HEIGHT))
                    .px(px(3.))
                    .tooltip(theme::text_tooltip(jump_hint.into()))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_line_jump(LineJumpTarget::Table(axis), window, cx);
                    })),
            )
            .child(
                button(next_id, next_label, Tone::Ghost, page + 1 < count)
                    .h(px(TABLE_PAGER_HEIGHT))
                    .px(px(7.))
                    .tooltip(theme::text_tooltip(next_key.into()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.change_table_page(axis, PageChange::Next, cx);
                    })),
            )
    }

    fn table_footer(&self, table: &table_preview::TablePreview, cx: &mut Context<Self>) -> Div {
        let actions = Self::visual_preview_footer("F2 查看原文", cx)
            .flex()
            .items_center()
            .gap(px(10.))
            .when(
                self.input_before_result_search.is_some()
                    || self.searchable_result_text().is_some(),
                |footer| {
                    let (label, hint) = if self.input_before_result_search.is_some() {
                        ("返回", "Ctrl+F 恢复原查询")
                    } else {
                        ("查找", "Ctrl+F 在当前表格查找")
                    };
                    footer.child(
                        button("search-current-table", label, Tone::Ghost, true)
                            .h(px(TABLE_PAGER_HEIGHT))
                            .px(px(8.))
                            .tooltip(theme::text_tooltip(hint.into()))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_result_search(window, cx);
                            })),
                    )
                },
            )
            .when(self.table_cell_source().is_some(), |footer| {
                footer.child(
                    button("get-table-cell", "取单元格", Tone::Ghost, true)
                        .h(px(TABLE_PAGER_HEIGHT))
                        .px(px(8.))
                        .tooltip(theme::text_tooltip(
                            "Ctrl+Shift+J 输入数据行号和列号".into(),
                        ))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.toggle_table_cell(window, cx);
                        })),
                )
            })
            .when(!table.input_errors().is_empty(), |footer| {
                footer.child(
                    button("locate-input-table-error", "定位错误", Tone::Ghost, true)
                        .h(px(TABLE_PAGER_HEIGHT))
                        .px(px(8.))
                        .tooltip(theme::text_tooltip("Alt+Enter 定位下一处错误".into()))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.select_input_error(false, window, cx);
                        })),
                )
            })
            .when(table.spans_pages(), |footer| {
                footer.child(
                    button("copy-table-page", "复制本页", Tone::Ghost, true)
                        .h(px(TABLE_PAGER_HEIGHT))
                        .px(px(8.))
                        .tooltip(theme::text_tooltip("Ctrl+Shift+C 复制本页 CSV".into()))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.copy_table(TableCopy::CurrentPage, false, window, cx);
                        })),
                )
            })
            .child(
                button("copy-table-tsv", "复制 TSV", Tone::Ghost, true)
                    .h(px(TABLE_PAGER_HEIGHT))
                    .px(px(8.))
                    .tooltip(theme::text_tooltip(
                        if table.spans_pages() {
                            "Ctrl+Shift+T 复制完整表格。Ctrl+Shift+Enter 复制本页 TSV"
                        } else {
                            "Ctrl+Shift+T 复制完整表格"
                        }
                        .into(),
                    ))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.copy_table(TableCopy::Tsv, false, window, cx);
                    })),
            );
        let pagination = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .when(table.row_pages() > 1, |pager| {
                pager.child(self.table_page_controls(TableAxis::Rows, table.row_pages(), cx))
            })
            .when(table.column_pages() > 1, |pager| {
                pager.child(self.table_page_controls(TableAxis::Columns, table.column_pages(), cx))
            });
        if table.spans_both_axes() {
            div()
                .flex()
                .flex_col()
                .gap(px(TABLE_FOOTER_ROW_GAP))
                .child(actions)
                .child(pagination.justify_end().px(px(10.)))
        } else {
            actions
                .child(div().flex_1())
                .when(table.spans_pages(), |footer| footer.child(pagination))
        }
    }

    fn render_result(&self, cx: &mut Context<Self>) -> Div {
        let row = div().h(px(38.)).flex().items_center().gap(px(12.));
        let scope = scoped_query(self.input.read(cx).text_ref());
        let recent_previews = matches!(scope, Some((Scope::RecentFilePreviews, _)));
        match &self.results {
            Results::Value(Ok(ResultValue {
                kind: ValueKind::Table(table),
                ..
            })) if self.expanded_value_pages.is_none() => self.render_visual_preview(
                table.legend(),
                table.render(self.table_pages.rows, self.table_pages.columns),
                "复制 CSV  Enter",
                self.table_footer(table, cx),
                cx,
            ),
            Results::Value(Ok(ResultValue {
                kind: ValueKind::Scatter(plot),
                ..
            })) if self.expanded_value_pages.is_none() => self.render_visual_preview(
                plot.legend(),
                plot.render(),
                "复制 CSV  Enter",
                Self::visual_preview_footer("F2 查看数据", cx),
                cx,
            ),
            Results::Value(Ok(ResultValue {
                kind: ValueKind::BoxPlot(plot),
                ..
            })) if self.expanded_value_pages.is_none() => self.render_visual_preview(
                plot.legend(),
                plot.render(),
                "复制摘要  Enter",
                Self::visual_preview_footer("F2 查看摘要", cx),
                cx,
            ),
            Results::Value(Ok(ResultValue {
                kind: ValueKind::BinPlot(plot),
                ..
            })) if self.expanded_value_pages.is_none() => self.render_visual_preview(
                plot.legend(),
                plot.render(),
                "复制 CSV  Enter",
                Self::visual_preview_footer("F2 查看数据", cx),
                cx,
            ),
            Results::Value(Ok(ResultValue {
                kind: ValueKind::CorrelationPlot(plot),
                ..
            })) if self.expanded_value_pages.is_none() => self.render_visual_preview(
                plot.legend(),
                plot.render(),
                "复制 CSV  Enter",
                Self::visual_preview_footer("F2 查看数据", cx),
                cx,
            ),
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
                let compact = text_tools::logical_newline_pattern().is_match(result)
                    || value_preview::columns(result.graphemes(true).take(large_text_columns + 1))
                        > large_text_columns;
                let value = if result.is_empty() {
                    value
                        .text_size(px(14.))
                        .text_color(rgb(theme::TEXT_MUTED))
                        .child("空文本")
                } else if compact {
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
                        button(
                            "copy-calculation",
                            self.copy_button_label("复制  Enter"),
                            Tone::Primary,
                            true,
                        )
                        .flex_none()
                        .on_click(cx.listener(|this, _, window, cx| this.confirm(window, cx))),
                    );
                div()
                    .flex()
                    .flex_col()
                    .child(row)
                    .when(
                        matches!(&output.kind, ValueKind::InputValidationError { .. }),
                        |result| {
                            result.child(
                                Self::result_footer().child(
                                    button(
                                        "locate-validation-error",
                                        "定位错误 Alt+Enter",
                                        Tone::Ghost,
                                        true,
                                    )
                                    .h(px(18.))
                                    .on_click(cx.listener(
                                        |this, _, window, cx| {
                                            this.select_input_error(false, window, cx);
                                        },
                                    )),
                                ),
                            )
                        },
                    )
                    .when_some(output.exact_decimal_preview(), |result, decimal| {
                        result.child(
                            Self::result_footer()
                                .flex()
                                .items_center()
                                .gap(px(10.))
                                .child(div().font_family(theme::MONO_FONT).child(decimal.label()))
                                .child(
                                    button(
                                        "refill-quick-search-decimal",
                                        "填回小数 Shift+F3",
                                        Tone::Ghost,
                                        true,
                                    )
                                    .h(px(18.))
                                    .on_click(cx.listener(
                                        |this, _, window, cx| {
                                            this.refill_decimal_preview(false, window, cx);
                                        },
                                    )),
                                ),
                        )
                    })
                    .when_some(output.continuation_hint(), |result, hint| {
                        result.child(
                            Self::result_footer()
                                .flex()
                                .items_center()
                                .gap(px(10.))
                                .child(
                                    div()
                                        .id("continue-value")
                                        .child(hint)
                                        .cursor_pointer()
                                        .hover(|style| style.text_color(rgb(theme::ACCENT)))
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.continue_value(window, cx);
                                        })),
                                )
                                .when_some(self.next_calculation_mode(cx), |row, mode| {
                                    let label = match mode {
                                        CalculationMode::Decimal => "普通计算 Alt+F2",
                                        CalculationMode::Exact => "精确计算 Alt+F2",
                                    };
                                    row.child(
                                        button("switch-calculation-mode", label, Tone::Ghost, true)
                                            .h(px(18.))
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.switch_calculation_mode(mode, window, cx);
                                            })),
                                    )
                                }),
                        )
                    })
                    .when(self.results.value_preview_expandable(), |result| {
                        let hint = match &output.kind {
                            ValueKind::Table(_) => "F2 返回表格",
                            _ if output.visual_preview_height().is_some() => "F2 返回图形",
                            _ if self.expanded_value_pages.is_some() => "F2 收起预览",
                            _ => "F2 展开预览",
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
                                        .child(
                                            button(
                                                "copy-expanded-value-page",
                                                "复制本页",
                                                Tone::Ghost,
                                                true,
                                            )
                                            .h(px(23.))
                                            .px(px(6.))
                                            .tooltip(theme::text_tooltip(
                                                "Ctrl+Shift+C 复制本页对应的原文".into(),
                                            ))
                                            .on_click(
                                                cx.listener(|this, _, window, cx| {
                                                    cx.stop_propagation();
                                                    this.copy_expanded_value_page(
                                                        false, window, cx,
                                                    );
                                                }),
                                            ),
                                        )
                                        .child(self.page_controls(page_count, 1, cx))
                                }),
                        )
                    })
            }
            Results::Value(Err(error)) => {
                let message = error.message();
                row.text_color(rgb(if error.is_incomplete() {
                    theme::TEXT_MUTED
                } else {
                    theme::DANGER_TEXT
                }))
                .child(
                    div()
                        .id("quick-search-error")
                        .flex_1()
                        .min_w_0()
                        .line_clamp(2)
                        .text_ellipsis()
                        .child(message.clone())
                        .tooltip(theme::text_tooltip(message)),
                )
                .when(self.batch_error_range(cx).is_some(), |row| {
                    row.child(
                        button(
                            "locate-batch-error",
                            "定位错误行 Alt+Enter",
                            Tone::Ghost,
                            true,
                        )
                        .flex_none()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.select_input_error(false, window, cx);
                        })),
                    )
                })
            }
            Results::RefilledInput => row
                .text_color(rgb(theme::TEXT_MUTED))
                .child("可在结果前添加下一条命令，也可按 F5 执行当前输入"),
            Results::Help(items) if items.is_empty() => row
                .text_color(rgb(theme::TEXT_MUTED))
                .child("未找到用法示例"),
            Results::Help(items) => self.render_examples(items, cx),
            Results::Calculations(items) if items.is_empty() => row
                .text_color(rgb(theme::TEXT_MUTED))
                .child("未找到用过的算式"),
            Results::Calculations(items) => {
                self.render_input_suggestions(items, Icon::Terminal, EXAMPLE_PAGE_SIZE, cx)
            }
            Results::Queries(items) if items.is_empty() => row
                .text_color(rgb(theme::TEXT_MUTED))
                .child("未找到已使用的查询"),
            Results::Queries(items) => {
                self.render_input_suggestions(items, Icon::Clock, EXAMPLE_PAGE_SIZE, cx)
            }
            Results::Conversions(items) if items.is_empty() => row
                .text_color(rgb(theme::TEXT_MUTED))
                .child("未找到用过的换算"),
            Results::Conversions(items) => {
                self.render_input_suggestions(items, Icon::Layers, EXAMPLE_PAGE_SIZE, cx)
            }
            Results::ConversionTargets(items) => {
                self.render_input_suggestions(items, Icon::Layers, CONVERSION_PAGE_SIZE, cx)
            }
            Results::FileOptions(items) if items.is_empty() => row
                .text_color(rgb(theme::TEXT_MUTED))
                .child("没有可追加的选项"),
            Results::FileOptions(items) | Results::FilterOptions(items) => {
                self.render_input_suggestions(items, Icon::Layers, EXAMPLE_PAGE_SIZE, cx)
            }
            Results::Bookmarks(items) if items.is_empty() => {
                row.text_color(rgb(theme::TEXT_MUTED)).child(
                    if matches!(
                        scoped_query(&self.input.read(cx).text()),
                        Some((Scope::Bookmarks, ""))
                    ) {
                        "用 Ctrl+D 收藏当前查询"
                    } else {
                        "未找到收藏的查询"
                    },
                )
            }
            Results::Bookmarks(items) => {
                self.render_input_suggestions(items, Icon::Layers, EXAMPLE_PAGE_SIZE, cx)
            }
            Results::BookmarkNameInput(Some(error)) => {
                row.text_color(rgb(theme::DANGER_TEXT)).child(error.clone())
            }
            Results::BookmarkNameInput(None) => row
                .text_color(rgb(theme::TEXT_MUTED))
                .child("Enter 保存 · 留空可移除名称"),
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
            | Results::BookmarksUnavailable(error)
            | Results::LocalFilesUnavailable(error) => row
                .text_color(rgb(theme::DANGER_TEXT))
                .child(div().truncate().child(error.clone())),
            Results::Destinations(items) if items.is_empty() => {
                row.text_color(rgb(theme::TEXT_MUTED)).child(match scope {
                    Some((Scope::ApplicationHistory, "")) => "本次运行还没有从快速搜索打开的应用",
                    Some((Scope::RecentFilePreviews, "")) => "本次运行还没有预览过的文本文件",
                    _ => "未找到匹配项",
                })
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
                                    path,
                                    is_directory,
                                    size_bytes,
                                    ..
                                } => (
                                    Icon::Layers,
                                    name.clone(),
                                    if recent_previews {
                                        Some(path.clone())
                                    } else {
                                        size_bytes
                                            .map(|bytes| format!("{bytes} 字节"))
                                            .or_else(|| is_directory.then(|| "文件夹".to_owned()))
                                    },
                                ),
                                Destination::OpenWindow(window) => {
                                    let detail = if window.minimized {
                                        Some(window.process.as_deref().map_or_else(
                                            || "已最小化".to_owned(),
                                            |process| format!("已最小化 · {process}"),
                                        ))
                                    } else {
                                        window.process.clone()
                                    };
                                    (Icon::Grid, window.title.clone(), detail)
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
                                            Destination::KnownFolder(_) => "Tab 查找 · Enter 打开",
                                            Destination::OpenWindow(_) => "Enter 切换",
                                            Destination::LocalFile {
                                                is_directory: true, ..
                                            } => "Tab 进入 · Enter 复制路径",
                                            Destination::LocalFile { .. } if recent_previews => {
                                                "Enter 预览"
                                            }
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

    fn frame(cx: &mut Context<Self>) -> Stateful<Div> {
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
            .on_action(cx.listener(|this, _: &OpenLineJump, window, cx| {
                this.open_line_jump(LineJumpTarget::Input, window, cx);
            }))
    }

    fn render_prompt_field(input: &Entity<MessageInput>, window: &Window, cx: &App) -> Div {
        let focused = input.focus_handle(cx).is_focused(window);
        div()
            .h(px(44.))
            .px(px(12.))
            .flex()
            .items_center()
            .rounded(px(6.))
            .bg(rgb(theme::ROW))
            .border_1()
            .border_color(rgb(if focused {
                theme::ACCENT
            } else {
                theme::BORDER_HI
            }))
            .child(input.clone())
    }

    fn render_input_find(
        &self,
        find: &InputFind,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let needle = find.input.read(cx).text_ref();
        let text = self.input.read(cx).text_ref();
        let count = if needle.is_empty() {
            0
        } else {
            text.matches(needle).count()
        };
        let replacement = find.replacement.as_ref();
        let fits = replacement.is_none_or(|replacement| {
            input_replacement_fits(text, needle, replacement.input.read(cx).text_ref(), count)
        });
        let status = if needle.is_empty() {
            "输入关键词后查找".to_owned()
        } else if !fits {
            format!("替换后将超过 {INPUT_CHARACTER_LIMIT} 字符")
        } else {
            format!("{count} 处匹配")
        };
        let title = if replacement.is_some() {
            "替换输入"
        } else {
            "查找输入"
        };
        Self::frame(cx)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                    .child(
                        button("cancel-input-find", "Esc 返回", Tone::Ghost, true)
                            .h(px(24.))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.finish_input_find(None, window, cx);
                            })),
                    ),
            )
            .child(Self::render_prompt_field(&find.input, window, cx))
            .when_some(replacement, |frame, replacement| {
                frame.child(Self::render_prompt_field(&replacement.input, window, cx))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_color(rgb(theme::TEXT_MUTED)).child(status))
                    .when(replacement.is_some(), |row| {
                        row.child(
                            button(
                                "replace-input-matches",
                                "全部替换  Enter",
                                Tone::Primary,
                                count > 0 && fits,
                            )
                            .h(px(28.))
                            .on_click(cx.listener(
                                |this, _, window, cx| {
                                    this.replace_input_matches(window, cx);
                                },
                            )),
                        )
                    })
                    .when(replacement.is_none(), |row| {
                        row.child(
                            div()
                                .flex()
                                .gap(px(8.))
                                .child(
                                    button(
                                        "previous-input-match",
                                        "上一个  Shift+Enter",
                                        Tone::Ghost,
                                        count > 0,
                                    )
                                    .h(px(28.))
                                    .on_click(cx.listener(
                                        |this, _, window, cx| {
                                            this.finish_input_find(Some(true), window, cx);
                                        },
                                    )),
                                )
                                .child(
                                    button(
                                        "next-input-match",
                                        "下一个  Enter",
                                        Tone::Primary,
                                        count > 0,
                                    )
                                    .h(px(28.))
                                    .on_click(cx.listener(
                                        |this, _, window, cx| {
                                            this.finish_input_find(Some(false), window, cx);
                                        },
                                    )),
                                ),
                        )
                    }),
            )
    }

    fn render_line_jump(jump: &LineJump, window: &Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let valid = jump.number(cx).is_some();
        let title = match jump.target {
            LineJumpTarget::Input => "跳到输入行",
            LineJumpTarget::Table(TableAxis::Rows) => "跳到表格数据行所在页",
            LineJumpTarget::Table(TableAxis::Columns) => "跳到表格列所在页",
        };
        Self::frame(cx)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                    .child(
                        button("cancel-line-jump", "Esc 返回", Tone::Ghost, true)
                            .h(px(24.))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.finish_line_jump(false, window, cx);
                            })),
                    ),
            )
            .child(Self::render_prompt_field(&jump.input, window, cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_color(rgb(theme::TEXT_MUTED)).child(format!(
                        "输入 1 至 {} 的{}",
                        jump.total,
                        jump.target.number_label()
                    )))
                    .child(
                        button("confirm-line-jump", "跳转  Enter", Tone::Primary, valid)
                            .h(px(28.))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.finish_line_jump(true, window, cx);
                            })),
                    ),
            )
    }
}

impl Render for QuickSearch {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        theme::apply(self.theme_mode, window.appearance());
        if let Some(find) = &self.input_find {
            return self.render_input_find(find, window, cx);
        }
        if let Some(jump) = &self.line_jump {
            return Self::render_line_jump(jump, window, cx);
        }
        let focused = self.input.focus_handle(cx).is_focused(window);
        let query = self.input.read(cx).text();
        let naming_bookmark = self.bookmark_name_edit.is_some();
        let selected_text_file = self.results.selected_text_file(self.selected).is_some();
        let refill_action = if naming_bookmark {
            None
        } else if let Some(refill) = &self.result_refill {
            Some(match &refill.kind {
                RefillKind::Result { .. } => "恢复输入 F3",
                RefillKind::File { .. } => "回到文件 F3",
            })
        } else {
            self.results.refillable_text().map(|_| "填回结果 F3")
        };
        let help_action = if naming_bookmark || query.trim().is_empty() {
            None
        } else if Self::shows_help(&query, &self.results) {
            self.input_before_help.is_some().then_some("返回查询 F1")
        } else {
            Some("用法 F1")
        };
        let query_history_action = if naming_bookmark {
            None
        } else if query.trim().is_empty() {
            Some("查询历史 F6")
        } else if self.input_before_query_history.is_some() && Self::is_query_history(&query) {
            Some("返回 F6")
        } else {
            None
        };
        let bookmark_action = if naming_bookmark {
            None
        } else if query.trim().is_empty() {
            Some(("查看收藏 F4", "F4 或 Ctrl+D 查看收藏".to_owned()))
        } else {
            self.results
                .bookmark_query(&query, self.selected)
                .map(|target| {
                    let (label, hint) = if let Some(error) = &self.bookmark_save_error {
                        ("收藏保存失败", error.as_str())
                    } else if matches!(self.results, Results::Bookmarks(_)) {
                        ("移除所选", "Ctrl+D 移除所选查询")
                    } else if self.index.recent.bookmarks.contains(target) {
                        ("已收藏", "Ctrl+D 取消收藏")
                    } else if self.bookmark_limit_reached {
                        ("最多收藏 12 项", "在收藏列表移除一项后再添加")
                    } else if self.results.bookmark_uses_selection() {
                        ("收藏所选", "Ctrl+D 收藏所选查询")
                    } else {
                        ("收藏", "Ctrl+D 收藏当前查询")
                    };
                    (label, hint.to_owned())
                })
        };
        Self::frame(cx)
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(if naming_bookmark {
                                "命名收藏"
                            } else {
                                "快速搜索"
                            }),
                    )
                    .child(div().flex_1())
                    .when(!naming_bookmark && query.trim().is_empty(), |header| {
                        header.child(
                            button("open-quick-search-tools", "工具", Tone::Ghost, true)
                                .h(px(24.))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.fill_query("工具: ", window, cx);
                                })),
                        )
                    })
                    .when_some(query_history_action, |header, label| {
                        header.child(
                            button(
                                "toggle-quick-search-query-history",
                                label,
                                Tone::Ghost,
                                true,
                            )
                            .h(px(24.))
                            .px(px(10.))
                            .on_click(cx.listener(
                                |this, _, window, cx| {
                                    this.toggle_query_history(window, cx);
                                },
                            )),
                        )
                    })
                    .when_some(help_action, |header, label| {
                        header.child(
                            button("toggle-quick-search-help", label, Tone::Ghost, true)
                                .h(px(24.))
                                .px(px(10.))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.toggle_help(window, cx);
                                })),
                        )
                    })
                    .when(
                        !naming_bookmark
                            && (selected_text_file
                                || self.file_preview_path().is_some()
                                || accepts_input_file(&query)),
                        |header| {
                            let (label, hint) = match &self.input_file_load {
                                InputFileLoadState::Ready if selected_text_file => {
                                    ("载入所选 F3", "F3 在搜索框中载入所选本地文本")
                                }
                                InputFileLoadState::Ready if self.file_preview_path().is_some() => {
                                    ("重读 F5", "F5 重新读取当前预览的本地文件")
                                }
                                InputFileLoadState::Ready => {
                                    ("载入", "Ctrl+O 在光标处插入 UTF-8 或 UTF-16 文本")
                                }
                                InputFileLoadState::InProgress => {
                                    ("正在载入", "正在选择或读取输入文件")
                                }
                                InputFileLoadState::Failed(error) => ("载入失败", error.as_str()),
                            };
                            header.child(
                                button(
                                    label,
                                    label,
                                    Tone::Ghost,
                                    !self.file_operation_in_progress(),
                                )
                                .h(px(24.))
                                .px(px(10.))
                                .tooltip(theme::text_tooltip(hint.to_owned().into()))
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        if selected_text_file {
                                            this.load_selected_text_file(window, cx);
                                        } else if let Some(path) = this.file_preview_path() {
                                            this.read_text_file_preview(
                                                path.to_owned(),
                                                FilePreviewAction::Refresh,
                                                window,
                                                cx,
                                            );
                                        } else {
                                            this.load_input_file(window, cx);
                                        }
                                    },
                                )),
                            )
                        },
                    )
                    .when(!naming_bookmark && self.is_table_cell_lookup(), |header| {
                        header.child(
                            button("return-to-cell-table", "返回表格", Tone::Ghost, true)
                                .h(px(24.))
                                .px(px(10.))
                                .tooltip(theme::text_tooltip("Ctrl+Shift+J 恢复表格查询".into()))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.toggle_table_cell(window, cx);
                                })),
                        )
                    })
                    .when_some(
                        refill_action.filter(|_| !self.is_table_cell_lookup()),
                        |header, label| {
                            header.child(
                                button("refill-quick-search-result", label, Tone::Ghost, true)
                                    .h(px(24.))
                                    .px(px(10.))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.refill_result(window, cx);
                                    })),
                            )
                        },
                    )
                    .when_some(bookmark_action, |header, (label, hint)| {
                        header.child(
                            // 操作改变时使用新的按钮标识，结束上一次的悬停提示。
                            button(label, label, Tone::Ghost, true)
                                .h(px(24.))
                                .px(px(10.))
                                .tooltip(theme::text_tooltip(hint.into()))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.toggle_bookmark(window, cx);
                                })),
                        )
                    })
                    .when(
                        matches!(&self.results, Results::Queries(items) if !items.is_empty()),
                        |header| {
                            header.child(
                                button(
                                    "remove-quick-search-query-history",
                                    "移除记录",
                                    Tone::Ghost,
                                    true,
                                )
                                .h(px(24.))
                                .px(px(10.))
                                .tooltip(theme::text_tooltip(
                                    "Alt+Delete 移除选中的查询记录".into(),
                                ))
                                .on_click(cx.listener(
                                    |this, _, window, cx| {
                                        this.remove_selected_query_history(window, cx);
                                    },
                                )),
                            )
                        },
                    )
                    .when(matches!(&self.results, Results::Value(Ok(_))), |header| {
                        let (label, hint, enabled) = match &self.result_save {
                            ResultSaveState::Ready => ("保存 Ctrl+S", "选择结果文件".into(), true),
                            ResultSaveState::InProgress => {
                                ("正在保存", "正在选择位置或写入文件".into(), false)
                            }
                            ResultSaveState::Finished { result: Ok(()), .. } => {
                                ("已保存", "Ctrl+S 再次保存".into(), true)
                            }
                            ResultSaveState::Finished {
                                result: Err(error), ..
                            } => ("保存失败", error.clone().into(), true),
                        };
                        header.child(
                            button(
                                label,
                                label,
                                Tone::Ghost,
                                enabled && !self.file_operation_in_progress(),
                            )
                            .h(px(24.))
                            .px(px(10.))
                            .tooltip(theme::text_tooltip(hint))
                            .on_click(cx.listener(
                                |this, _, window, cx| {
                                    this.save_result(window, cx);
                                },
                            )),
                        )
                    })
                    .child(
                        div()
                            .id("close-quick-search")
                            .px(px(6.))
                            .py(px(3.))
                            .rounded(px(4.))
                            .cursor_pointer()
                            .text_color(rgb(theme::TEXT_MUTED))
                            .hover(|style| style.bg(rgb(theme::ROW)))
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this.bookmark_name_edit.is_some() {
                                    this.finish_bookmark_name(false, window, cx);
                                } else {
                                    window.remove_window();
                                }
                            }))
                            .child(if naming_bookmark {
                                "Esc 返回"
                            } else {
                                "Esc 关闭"
                            }),
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
    use crate::quick_search::fraction::simplify_fraction;

    fn for_query(query: &str, triggers: &[SearchTrigger]) -> Results {
        SearchIndex {
            triggers: triggers.to_vec(),
            ..SearchIndex::default()
        }
        .results(query)
    }

    #[test]
    fn shows_every_batch_calculation_result_and_error_with_the_original_line_number() {
        for (query, expected) in [
            (
                "批量计算: 明细 2**8\r\n\r\n1/0\r\navg(1,3)",
                "行号,算式,结果,错误\r\n1,2**8,256,\"\"\r\n3,1/0,\"\",不能除以零\r\n4,\"avg(1,3)\",2,\"\"",
            ),
            (
                "batch calculation: exact-report 1/3+1/6\n2/0\n9007199254740993",
                "行号,算式,结果,错误\r\n1,1/3+1/6,1/2,\"\"\r\n2,2/0,\"\",分母或除数不能为零\r\n3,9007199254740993,9007199254740993,\"\"",
            ),
        ] {
            assert!(matches!(for_query(query, &[]),
                Results::Value(Ok(ResultValue { text, kind: ValueKind::Table(_) })) if text == expected),
                "{query}");
        }
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
            for_query("batch calculation: 2^10\r\n\r3^2\u{2028}1/0", &[]),
            Results::Value(Err(error)) if error.message().as_ref() == "第 4 行：不能除以零"
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
    fn batch_comparisons_keep_verbatim_csv_and_preview_it_in_columns() {
        for (query, expected) in [
            (
                "批量计算: 对照 avg(1,3)\n2**10",
                "算式,结果\r\n\"avg(1,3)\",2\r\n2**10,1024",
            ),
            (
                "批量换算: 进制对照 字节序 16 @ 258",
                "输入,结果\r\n字节序 16 @ 258,\"大端 01 02\n小端 02 01\"",
            ),
            (
                "批量换算: 节拍对照 时长 120 BPM @ 3/2",
                "输入,结果\r\n时长 120 BPM @ 3/2,750 ms",
            ),
        ] {
            let result = for_query(query, &[]);
            assert!(
                matches!(
                    &result,
                    Results::Value(Ok(value)) if matches!(&value.kind, ValueKind::Table(_))
                ),
                "{query}"
            );
            assert!(result.value_preview_expandable());
            assert_eq!(result.refillable_text(), Some(expected), "{query}");
        }
        assert!(matches!(
            for_query("批量换算: 节拍 时长 120 BPM @ 3/2", &[]),
            Results::Value(Ok(ResultValue {
                kind: ValueKind::Text,
                ..
            }))
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
    fn runs_tool_operations_on_each_batch_input() {
        for (query, expected) in [
            (
                "批量换算: 节拍 时长 120 BPM @ 3/2\n速度 1.5秒 @ 3",
                "750 ms\n120 BPM",
            ),
            (
                "批量换算: DECIBEL-CSV 功率比 100\n转幅度比 20 dB",
                "输入,结果\r\n功率比 100,20 dB\r\n转幅度比 20 dB,10",
            ),
            (
                "批量换算: 立体对照 圆柱 3,5\n球 1/2",
                "输入,结果\r\n\"圆柱 3,5\",表面积 48π · 体积 45π\r\n球 1/2,表面积 π · 体积 π/6",
            ),
            (
                "批量换算: 波长对照 1 MHz\n34.3 cm @ 343 m/s to kHz",
                "输入,结果\r\n1 MHz,299.792458 m\r\n34.3 cm @ 343 m/s to kHz,1 kHz",
            ),
            (
                "批量换算: 网段 取地址 192.0.2.137/26,2\n取子网 192.0.2.137/26,/28,3",
                "192.0.2.129\n192.0.2.160/28",
            ),
            (
                "批量换算: CIDR-CSV 子网 192.0.2.0/30 到 /31\n偏移 2001:db8::3/126,1",
                "输入,结果\r\n子网 192.0.2.0/30 到 /31,\"192.0.2.0/31\n192.0.2.2/31\"\r\n\"偏移 2001:db8::3/126,1\",2001:db8::4/126",
            ),
        ] {
            assert!(
                matches!(for_query(query, &[]), Results::Value(Ok(value)) if value.text == expected),
                "{query}"
            );
        }
        assert!(matches!(
            for_query("批量换算: 立体 球 1/2\n\n球 0", &[]),
            Results::Value(Err(ValueError::BatchTool(BatchError::Row { line: 3, .. })))
        ));
        assert!(matches!(
            for_query("批量换算: wave 500 nm\n\n1 km @ 0 m/s", &[]),
            Results::Value(Err(ValueError::BatchTool(BatchError::Row { line: 3, .. })))
        ));
        assert!(matches!(
            for_query(
                "批量换算: 网段对照 取地址 192.0.2.0/26,1\n\n偏移 ::/0,-1",
                &[]
            ),
            Results::Value(Err(ValueError::BatchTool(BatchError::Row { line: 3, .. })))
        ));
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
    fn converts_each_batch_duration_to_an_exact_fraction_and_preserves_input_rows() {
        assert!(matches!(
            for_query("批量换算: 精确时长 1秒 to min\nPT1M30S TO H\n-0.25秒 to min", &[]),
            Results::Value(Ok(value)) if value.text == "1/60 min\n1/40 h\n-1/240 min"
        ));
        let report = "批量换算: 精确时长对照 1秒 to min\n23:45 到 01:15 to d\n1ns to s";
        assert!(matches!(
            for_query(report, &[]),
            Results::Value(Ok(value)) if value.text == concat!(
                "输入,结果\r\n",
                "1秒 to min,1/60 min\r\n",
                "23:45 到 01:15 to d,1/16 d\r\n",
                "1ns to s,1/1000000000 s"
            )
        ));
        assert!(matches!(
            for_query("批量换算: exact-duration-csv 1秒 to min\n\n1秒 to ISO", &[]),
            Results::Value(Err(ValueError::BatchDuration(BatchError::Row {
                line: 3,
                cause: DurationError::ExactUnitArguments,
            })))
        ));
        assert_eq!(
            for_query("批量换算: 精确时长对", &[]).suggested_input(0),
            Some(report)
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
    fn batches_radix_and_byte_orders_in_csv_and_reports_the_original_line() {
        assert!(matches!(
            for_query(
                "批量换算: 进制对照 ff 从 16 到 10\n字节序 16 @ 258\n读字节序 小端 @ 02 01",
                &[],
            ),
            Results::Value(Ok(value)) if value.text == concat!(
                "输入,结果\r\n",
                "ff 从 16 到 10,255\r\n",
                "字节序 16 @ 258,\"大端 01 02\n小端 02 01\"\r\n",
                "读字节序 小端 @ 02 01,\"十六进制 0x0102\n十进制 258\"",
            )
        ));
        assert!(matches!(
            for_query(
                "batch conversion: radix-csv ff from 16 to 10\n\n读字节序 大端 @ G0",
                &[]
            ),
            Results::Value(Err(ValueError::BatchRadix(BatchError::Row {
                line: 3,
                cause: RadixError::ReadByteOrderBytes,
            })))
        ));
    }

    #[test]
    fn batches_colors_as_hex_and_preserves_alpha_and_csv_input_quoting() {
        assert!(matches!(
            for_query(
                "批量换算: 颜色对照 rebeccapurple\nrgb(0, 255, 0)\nrgba(255,0,0,0.5)",
                &[],
            ),
            Results::Value(Ok(value)) if value.text == concat!(
                "输入,结果\r\n",
                "rebeccapurple,#663399\r\n",
                "\"rgb(0, 255, 0)\",#00FF00\r\n",
                "\"rgba(255,0,0,0.5)\",#FF000080",
            )
        ));
        assert!(matches!(
            for_query("批量换算: 颜色 ＃ａｂｃ\ntransparent", &[]),
            Results::Value(Ok(value)) if value.text == "#AABBCC\n#00000000"
        ));
        assert!(matches!(
            for_query("批量换算: 颜色对照 #abc\n\nrgb(300,0,0)", &[]),
            Results::Value(Err(ValueError::BatchColor(BatchError::Row { line: 3, .. })))
        ));
    }

    #[test]
    fn reuses_a_single_color_as_exact_hex_including_transparency() {
        for (query, expected) in [
            ("颜色: #FF800080 to hsl", Some("颜色: #FF800080")),
            (
                "颜色: color-mix(in srgb-linear, transparent, blue)",
                Some("颜色: #0000FF80"),
            ),
            ("颜色: red to rgb", Some("颜色: #FF0000")),
            ("颜色: 色阶 #000 到 #fff @ 3", None),
            ("颜色: 线性色阶 #000 到 #fff @ 3", None),
            ("颜色: OKLab色阶 #000 到 #fff @ 3", None),
            ("对比度: #fff vs #000", None),
        ] {
            assert_eq!(
                for_query(query, &[]).continuation_query().as_deref(),
                expected,
                "{query}"
            );
        }
        assert!(matches!(
            for_query("颜色: #0000FF80 叠加 #fff", &[]),
            Results::Value(Ok(value)) if value.text == "#7F7FFF"
        ));
    }

    #[test]
    fn renders_calendar_csv_reports_as_tables_for_search_and_pipelines() {
        for term in [
            "逐日ＣＳＶ 2024-02-29 至 2024-03-03",
            "按月拆分CSV 2024-01-31 至 2024-03-02",
            "SPLIT-WEEKS-CSV 2026-12-30 至 2027-01-05",
            "按季度拆分CSV 2024-03-30 至 2024-04-02",
            "按半年拆分CSV 2024-06-29 至 2024-07-02",
            "按年拆分CSV 2023-12-30 至 2024-01-02",
            "星期统计 2024-02-29 至 2024-03-09",
            "区间空档ＣＳＶ 2024-02-28 至 2024-02-29; 2024-03-04 至 2024-03-05",
        ] {
            let now = chrono::DateTime::parse_from_rfc3339("2024-02-29T12:00:00+08:00").unwrap();
            let pipeline = format!("[\"日期:\"] {term}");
            let (text, preview) = transform_data_pipeline(&pipeline, now).unwrap();
            assert_eq!(text, calculate_date(term, now.date_naive()).unwrap());
            assert!(matches!(preview, Some(PipelinePreview::Table(_))), "{term}");
            assert!(
                matches!(for_query(&format!("日期: {term}"), &[]),
                    Results::Value(Ok(ResultValue { text: direct, kind: ValueKind::Table(_) })) if direct == text),
                "{term}"
            );
        }
        assert!(matches!(
            for_query("日期: 按月拆分 2024-01-31 至 2024-03-02", &[]),
            Results::Value(Ok(ResultValue {
                kind: ValueKind::Text,
                ..
            }))
        ));
    }

    #[test]
    fn continues_absolute_timestamps_without_reusing_tables_or_elapsed_durations() {
        for (query, expected) in [
            ("时间戳: 0", Some("时间戳: 1970-01-01T00:00:00Z")),
            (
                "时间戳: 1970-01-01T00:00:00.123Z to +08:00",
                Some("时间戳: 1970-01-01T08:00:00.123+08:00"),
            ),
            ("时间戳: 0 to UTC, Asia/Shanghai", None),
            ("时间戳: 时区CSV 0 to UTC", None),
            ("时间戳: 间隔 0 到 3", None),
            ("时间戳: 1970-01-01T00:00:00Z", None),
        ] {
            assert_eq!(
                for_query(query, &[]).continuation_query().as_deref(),
                expected,
                "{query}"
            );
        }
    }

    #[test]
    fn shows_decimal_previews_alongside_exact_fraction_results() {
        for (query, text, decimal) in [
            ("分数: 计算 1/3", "1/3", Some("≈ 0.333333333333")),
            ("分数: 计算 -1/4", "-1/4", Some("= -0.25")),
            (
                "分数: 计算 1e-20",
                "1/100000000000000000000",
                Some("= 1.00000000000e-20"),
            ),
            ("分数: 计算 6/3", "2", None),
            ("分数: 小数 1/3 to 8", "0.33333333", None),
            ("分数: 循环小数 1/3", "0.(3)", None),
        ] {
            let Results::Value(Ok(value)) = for_query(query, &[]) else {
                panic!("应得到精确计算结果：{query}");
            };
            assert_eq!(value.text, text);
            assert_eq!(
                value
                    .exact_decimal_preview()
                    .map(|preview| preview.label())
                    .as_deref(),
                decimal
            );
        }
    }

    #[test]
    fn previews_exact_duration_unit_tables_directly_and_as_the_final_pipeline_step() {
        let expected = "单位,数值\r\nms,1000\r\ns,1\r\nmin,1/60";
        for query in [
            r#"时长: 单位表 ["ms","s","min"] 1秒"#,
            r#"处理: ["时长: 单位表 [\"ms\",\"s\",\"min\"]"] 1秒"#,
        ] {
            let result = for_query(query, &[]);
            assert!(result.table_preview().is_some(), "{query}");
            assert_eq!(result.refillable_text(), Some(expected), "{query}");
        }
    }

    #[test]
    fn previews_exact_data_units_directly_and_in_pipelines() {
        assert!(matches!(
            for_query("数据换算: 单位", &[]),
            Results::Help(items) if items.iter().any(|&index| EXAMPLES[index].query.starts_with("数据换算: 单位表 "))
        ));
        for (query, expected) in [
            (
                "数据换算: 单位表 [B,b,MiB] 1/3 KiB",
                "单位,数值\r\nB,1024/3\r\nb,8192/3\r\nMiB,1/3072",
            ),
            (
                r#"处理: ["数据换算: UNITS-CSV [MB/s,KiB/s]"] 1/3 MiB/s"#,
                "单位,数值\r\nMB/s,16384/46875\r\nKiB/s,1024/3",
            ),
            (
                r#"处理: ["数据换算: 单位表 [B,KiB]","CSV: 选列 2"] 9007199254740993 B"#,
                "数值\r\n9007199254740993\r\n9007199254740993/1024",
            ),
        ] {
            let results = for_query(query, &[]);
            assert!(
                matches!(
                    &results,
                    Results::Value(Ok(ResultValue {
                        kind: ValueKind::Table(_),
                        ..
                    }))
                ),
                "{query}"
            );
            assert_eq!(results.refillable_text(), Some(expected), "{query}");
        }
        assert!(matches!(
            for_query("数据换算: 单位表 [B,] 1 KiB", &[]),
            Results::Value(Err(ValueError::Conversion(ConversionError::InvalidTargets)))
        ));
        assert!(matches!(
            for_query("数据换算: 单位表 [B/s] 1 KiB", &[]),
            Results::Value(Err(ValueError::Conversion(
                ConversionError::DifferentDimensions
            )))
        ));
    }

    #[test]
    fn previews_duration_quantiles_as_tables_in_search_and_pipelines() {
        let expected = "分位数,时长\r\n25%,00:00:01.75\r\n0.5,00:00:02.5\r\n3/4,00:00:03.25";
        for query in [
            r#"时长: 分位表 ["25%",0.5,"3/4"] 3s,1s,2s,4s"#,
            r#"处理: ["时长: 分位表 [\"25%\",0.5,\"3/4\"]"] 3s,1s,2s,4s"#,
        ] {
            let result = for_query(query, &[]);
            assert!(result.table_preview().is_some(), "{query}");
            assert_eq!(result.refillable_text(), Some(expected), "{query}");
        }
    }

    #[test]
    fn previews_timestamp_reports_as_tables_without_changing_their_csv() {
        for term in [
            "时区ＣＳＶ 1ns to UTC, +08:00",
            "offset-changes 2026 UTC",
            "日区间 2026-03-08 America/New_York",
            "每日同刻 2026-03-07 02:30 America/New_York @ 3",
        ] {
            let csv = convert_timestamp(term, chrono::DateTime::UNIX_EPOCH).unwrap();
            assert!(
                matches!(for_query(&format!("时间戳: {term}"), &[]),
                    Results::Value(Ok(ResultValue { text, kind: ValueKind::Table(_) })) if text == csv),
                "{term}"
            );
        }
        assert!(matches!(
            for_query("时间戳: 0 to UTC, +08:00", &[]),
            Results::Value(Ok(ResultValue {
                kind: ValueKind::Text,
                ..
            }))
        ));
    }

    #[test]
    fn previews_path_components_in_input_order_for_direct_queries_and_pipelines() {
        let expected = concat!(
            "路径,上一级,文件名,主名,扩展名",
            "\r\n\"docs/report,final.csv\",docs,\"report,final.csv\",\"report,final\",csv",
            "\r\nREADME,\"\",README,README,\"\"",
            "\r\n.,\"\",\"\",\"\",\"\"",
        );
        for query in [
            r#"路径: 部件CSV ["docs/report,final.csv","README","."]"#,
            r#"处理: ["路径: 部件CSV"] ["docs/report,final.csv","README","."]"#,
            "路径: 逐行部件CSV \"docs/report,final.csv\"\r\n\rREADME\u{2028}.",
            "处理: [\"路径: 逐行部件CSV\"] \"docs/report,final.csv\"\nREADME\n.",
        ] {
            let result = for_query(query, &[]);
            assert_eq!(result.refillable_text(), Some(expected), "{query}");
            assert!(result.table_preview().is_some(), "{query}");
        }
        assert_eq!(
            path_component("部件CSV []"),
            Err(PathError::ReportArguments)
        );
        assert_eq!(
            path_component(r#"部件CSV ["a\tb"]"#),
            Err(PathError::InvalidPath)
        );
        assert_eq!(
            path_component("逐行部件CSV \"a.txt\nREADME"),
            Err(PathError::InvalidPath)
        );
        assert_eq!(
            path_component(&format!("逐行部件CSV {}", "README\n".repeat(129))),
            Err(PathError::LineReportArguments)
        );
    }

    #[test]
    fn previews_regex_capture_reports_as_tables_and_preserves_their_csv() {
        for (query, expected) in [
            (
                r#"文本: 正则取表 "(?<id>[0-9]+):([^;]+)" 001:茶,咖啡;002:梨"#,
                "id,2\r\n001,\"茶,咖啡\"\r\n002,梨",
            ),
            (r#"文本: regex-to-csv "([0-9]+)?X" X"#, "1\r\n\"\""),
            (
                r#"文本: 正则定位取表 "([0-9]+)" 无数字"#,
                "原文行号,原文列号,1",
            ),
            (
                r#"处理: ["文本: 正则定位取表 \"(ID=[0-9]+)\""] 前缀 ID=1"#,
                "原文行号,原文列号,1\r\n1,4,ID=1",
            ),
        ] {
            let result = for_query(query, &[]);
            assert_eq!(result.refillable_text(), Some(expected), "{query}");
            let table = result.table_preview().expect("正则取表结果可预览");
            assert_eq!(table.page_csv(0, 0), expected, "{query}");
        }
    }

    #[test]
    fn continues_single_clock_durations_without_reusing_ratios_or_lists() {
        for (query, expected) in [
            ("时长: 缩放 01:30:00 @ 2/3", Some("时长: 01:00:00")),
            ("时长: 舍入 01:07:30 @ 15分钟", Some("时长: 01:15:00")),
            ("时长: 1ns - 2ns", Some("时长: -00:00:00.000000001")),
            ("时长: 01:30:00 to min", None),
            ("时长: 比率 1ns @ 3ns", None),
            ("时长: 分摊 01:30:00 @ 1:2", None),
            ("时长: 排序 02:00,01:00", None),
        ] {
            assert_eq!(
                for_query(query, &[]).continuation_query().as_deref(),
                expected,
                "{query}"
            );
        }
        assert!(matches!(
            for_query("时长: -00:00:00.000000001 + 2ns", &[]),
            Results::Value(Ok(value)) if value.text == "00:00:00.000000001"
        ));
    }

    #[test]
    fn continues_an_absolute_date_without_changing_other_calendar_results() {
        for (query, expected) in [
            ("日期: 2024-02-28 + 2天", Some("日期: 2024-03-01")),
            ("日期: 2024-03-01 周日期", Some("日期: 2024-W09-5")),
            (
                r#"日期: 格式化 ["2024-03-01","          ２０２４年０３月０１日          "]"#,
                Some("日期: ２０２４年０３月０１日"),
            ),
            ("日期: 2024-02", None),
            ("日期: 2024-03-01 第几周", None),
            ("批量换算: 日期 2024-02-28 + 2天", None),
        ] {
            assert_eq!(
                for_query(query, &[]).continuation_query().as_deref(),
                expected,
                "{query}"
            );
        }
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
    fn query_history_filters_confirmed_queries_and_restores_the_exact_input() {
        let index = SearchIndex::default();
        let original = "文本: 原文   甲\r\n乙\t ";
        let current = index.results("查询历史:");
        assert!(matches!(current, Results::Queries(items) if items.is_empty()));
        index.recent.calculations.record("2**10", "1024");
        index.recent.queries.record_exact(original, "");
        index
            .recent
            .queries
            .record_exact("JSON: 紧凑 {\"字段\":2}", "");
        index.recent.queries.record_exact(original, "");
        let list = index.results("查询历史:");
        assert_eq!(list.suggested_input(0), Some(original));
        assert_eq!(list.suggested_input(1), Some("JSON: 紧凑 {\"字段\":2}"));
        assert_eq!(list.suggested_input(2), None);
        assert_eq!(
            index.results("query history: 原文 乙").suggested_input(0),
            Some(original)
        );
        assert_eq!(index.results("查询历史: 1024").suggested_input(0), None);
        let trimmed = original.trim();
        index.recent.queries.record_exact(trimmed, "");
        index.recent.queries.remove_exact(original);
        let list = index.results("查询历史:");
        assert_eq!(list.suggested_input(0), Some(trimmed));
        assert_eq!(list.suggested_input(1), Some("JSON: 紧凑 {\"字段\":2}"));
        assert_eq!(list.suggested_input(2), None);
        assert_eq!(index.results("算式历史:").suggested_input(0), Some("2**10"));
    }

    #[test]
    fn refills_exact_result_text_only_when_it_fits_the_search_input() {
        let result = |text: String| {
            Results::Value(Ok(ResultValue {
                text,
                kind: ValueKind::Text,
            }))
        };
        assert_eq!(
            result("甲\r\n乙\t ".to_owned()).refillable_text(),
            Some("甲\r\n乙\t ")
        );
        assert_eq!(
            result("甲".repeat(INPUT_CHARACTER_LIMIT))
                .refillable_text()
                .map(|text| text.chars().count()),
            Some(INPUT_CHARACTER_LIMIT)
        );
        assert!(result("甲".repeat(INPUT_CHARACTER_LIMIT + 1))
            .refillable_text()
            .is_none());
        assert!(result(String::new()).refillable_text().is_none());
    }

    #[test]
    fn correlation_charts_keep_precise_csv_and_render_only_for_the_final_step() {
        let data = "x,y,z,固定\n0,0,2,5\n1,1.0,1,5\n2,1/1,0,5\n-2,,8,6";
        let expected = transform_csv(&format!("秩相关矩阵 [1,2,3,4] {data}")).unwrap();
        for command in [
            "CSV: 秩相关图 [1,2,3,4]",
            r#"处理: ["CSV: 数值筛选 1 >= 0","CSV: 秩相关图 [1,2,3,4]"]"#,
        ] {
            let results = for_query(&format!("{command} {data}"), &[]);
            assert!(matches!(
                &results,
                Results::Value(Ok(ResultValue {
                    kind: ValueKind::CorrelationPlot(_),
                    ..
                }))
            ));
            assert_eq!(results.refillable_text(), Some(expected.as_str()));
            assert!(results.value_preview_expandable());
        }
        let selected_column = for_query(
            &format!(r#"处理: ["CSV: 秩相关图 [1,2,3,4]","CSV: 选列 2"] {data}"#),
            &[],
        );
        let expected_column = transform_csv(&format!("选列 2 {expected}")).unwrap();
        assert_eq!(
            selected_column.refillable_text(),
            Some(expected_column.as_str())
        );
        assert!(matches!(
            &selected_column,
            Results::Value(Ok(ResultValue {
                kind: ValueKind::Table(_),
                ..
            }))
        ));
    }

    #[test]
    fn previews_statistics_csv_reports_directly_and_as_pipeline_outputs() {
        for (operation, values, expected) in [
            (
                "频数CSV",
                "3,1/2,0.5,3,5",
                "值,次数,频率\r\n1/2,2,2/5\r\n3,2,2/5\r\n5,1,1/5",
            ),
            (
                "FREQUENCY-CSV",
                "频数 1@2,3@1",
                "值,次数,频率\r\n1,2,2/3\r\n3,1,1/3",
            ),
            (
                "分位表 [25%,50%,75%]",
                "4,1,3,2",
                "概率,分位数\r\n1/4,7/4\r\n1/2,5/2\r\n3/4,13/4",
            ),
            (
                "QUANTILES-CSV [75%,50%,25%,50%]",
                "频数 1@2,5@2",
                "概率,分位数\r\n3/4,5\r\n1/2,3\r\n1/4,1\r\n1/2,3",
            ),
            (
                "累计分布",
                "1,1,3",
                "值,累计次数,累计频率\r\n1,2,2/3\r\n3,3,1",
            ),
            (
                "分箱",
                "0,2 @ -1,0,2",
                "下界,上界,次数,频率\r\n-∞,0,1,1/3\r\n0,2,1,1/3\r\n2,+∞,1,1/3",
            ),
        ] {
            for query in [
                format!("统计: {operation} {values}"),
                format!(r#"处理: ["统计: {operation}"] {values}"#),
            ] {
                let results = for_query(&query, &[]);
                assert!(
                    matches!(
                        &results,
                        Results::Value(Ok(ResultValue {
                            kind: ValueKind::Table(_),
                            ..
                        }))
                    ),
                    "{query}"
                );
                assert_eq!(results.refillable_text(), Some(expected), "{query}");
            }
        }
    }

    #[test]
    fn rejects_invalid_statistics_quantile_tables() {
        for query in [
            "分位表 [] 1,2",
            "分位表 [50%,] 1,2",
            "分位表 [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0] 1,2",
        ] {
            assert_eq!(
                calculate_statistics(query),
                Err(StatisticsError::QuantileList)
            );
        }
        assert_eq!(
            calculate_statistics("分位表 [101%] 1,2"),
            Err(StatisticsError::InvalidProbability)
        );
    }

    #[test]
    fn previews_exact_fraction_samples_in_both_directions_and_in_pipelines() {
        for (query, expected) in [
            (
                "分数: 精确采样 [-1,1,5] x**2+1/3",
                "x,y\r\n-1,4/3\r\n-1/2,7/12\r\n0,1/3\r\n1/2,7/12\r\n1,4/3",
            ),
            (
                r#"处理: ["分数: sample-csv [gcd(2,3),-1,3]"] x"#,
                "x,y\r\n1,1\r\n0,0\r\n-1,-1",
            ),
        ] {
            let results = for_query(query, &[]);
            assert!(
                matches!(
                    &results,
                    Results::Value(Ok(ResultValue {
                        kind: ValueKind::Table(_),
                        ..
                    }))
                ),
                "{query}"
            );
            assert_eq!(results.refillable_text(), Some(expected), "{query}");
        }
        assert_eq!(
            simplify_fraction("精确采样 [-1,1,129] x"),
            Err(FractionError::SampleArguments)
        );
        assert_eq!(
            simplify_fraction("精确采样 [-1,1,3] 1/x"),
            Err(FractionError::ZeroDenominator)
        );
    }

    #[test]
    fn previews_explicit_fraction_points_in_input_order_with_duplicates() {
        let expected = "x,y\r\n3/2,31/12\r\n-2,13/3\r\n3/2,31/12\r\n0,1/3";
        for query in [
            "分数: 取值CSV [3/2,-2,1.50,0] x**2+1/3",
            r#"处理: ["分数: TABLE-CSV [3/2,-2,1.50,0]"] x**2+1/3"#,
        ] {
            let results = for_query(query, &[]);
            assert!(
                matches!(
                    &results,
                    Results::Value(Ok(ResultValue {
                        kind: ValueKind::Table(_),
                        ..
                    }))
                ),
                "{query}"
            );
            assert_eq!(results.refillable_text(), Some(expected), "{query}");
        }
        assert_eq!(
            simplify_fraction("取值CSV [1,,2] x"),
            Err(FractionError::TableInputs)
        );
    }

    #[test]
    fn previews_csv_rule_reports_and_filtered_records_without_rewriting_csv() {
        let input = "编号,日期,备注\r\nA,2024-02-29,\"保留,逗号\"\r\nB,2023-02-29,\"上\r\n下\"";
        for (operation, expected) in [
            (
                r#"检查表头 ["编号","日期","备注"]"#,
                "列号,问题,预期列名,实际列名",
            ),
            (
                "检查日期列 [2]",
                "数据行,列号,列名,内容\r\n2,2,日期,2023-02-29",
            ),
            (
                r#"检查规则 [["日期","日期"]]"#,
                "数据行,列号,列名,规则号,规则,内容\r\n2,2,日期,1,日期,2023-02-29",
            ),
            (
                r#"规则统计 [["日期","日期"]]"#,
                "规则号,列号,列名,规则,通过数,失败数\r\n1,2,日期,日期,1,1",
            ),
            (
                r#"规则保留行 [["日期","日期"]]"#,
                "编号,日期,备注\r\nA,2024-02-29,\"保留,逗号\"",
            ),
            (
                r#"KEEP-INVALID-ROWS [["日期","日期"]]"#,
                "编号,日期,备注\r\nB,2023-02-29,\"上\r\n下\"",
            ),
        ] {
            let step = serde_json::to_string(&[format!("CSV: {operation}")]).unwrap();
            for query in [
                format!("CSV: {operation} {input}"),
                format!("处理: {step} {input}"),
            ] {
                let results = for_query(&query, &[]);
                assert!(
                    matches!(
                        &results,
                        Results::Value(Ok(ResultValue {
                            kind: ValueKind::Table(_),
                            ..
                        }))
                    ),
                    "{query}"
                );
                assert_eq!(results.refillable_text(), Some(expected), "{query}");
            }
        }
    }

    #[test]
    fn reports_individual_exact_weight_shares_for_matching_values() {
        let expected = "值,权重,比例\r\n1/2,1,1/3\r\n2,2,2/3";
        for query in [
            "统计: 加权频数CSV 2@2,0.5@1/4,-1@0,1/2@3/4",
            "统计: WEIGHTED-FREQUENCY-CSV 2\t2\n0.5\t1/4\n-1\t0\n1/2\t3/4",
            r#"处理: ["统计: 加权频数CSV"] 2@2,0.5@1/4,-1@0,1/2@3/4"#,
        ] {
            let results = for_query(query, &[]);
            assert!(
                matches!(
                    &results,
                    Results::Value(Ok(ResultValue {
                        kind: ValueKind::Table(_),
                        ..
                    }))
                ),
                "{query}"
            );
            assert_eq!(results.refillable_text(), Some(expected), "{query}");
        }
        assert_eq!(
            calculate_statistics("加权频数CSV 5@0,2@0"),
            Err(StatisticsError::ZeroTotalWeight)
        );
    }

    #[test]
    fn previews_multiple_weighted_quantiles_in_query_order_including_exact_boundaries() {
        let output = "概率,分位数\r\n3/4,4\r\n0,1\r\n1/4,1\r\n1,4\r\n1/4,1";
        for query in [
            "统计: 加权分位表 [75% , 0,1/4, 100% ,0.25] 1@1,2@1,4@2,-1@0",
            r#"处理: ["统计: WEIGHTED-QUANTILES-CSV [75%,0,1/4,1,0.25]"] 1@1,2@1,4@2,-1@0"#,
        ] {
            let results = for_query(query, &[]);
            assert!(
                matches!(
                    &results,
                    Results::Value(Ok(ResultValue {
                        kind: ValueKind::Table(_),
                        ..
                    }))
                ),
                "{query}"
            );
            assert_eq!(results.refillable_text(), Some(output), "{query}");
        }
        assert_eq!(
            calculate_statistics("加权分位表 [] 1@1"),
            Err(StatisticsError::QuantileList)
        );
        assert_eq!(
            calculate_statistics("加权分位表 [110%] 1@1"),
            Err(StatisticsError::InvalidProbability)
        );
    }

    #[test]
    fn named_csv_box_plot_ignores_missing_values_and_keeps_an_exact_summary() {
        let data = "编号,金额\nA,1\nB,2\nC,\nD,3\nE,4\nF,8";
        let results = for_query(&format!(r#"CSV: 列箱线图 "金额" {data}"#), &[]);
        let expected = calculate_statistics("箱线摘要 1,2,3,4,8").unwrap();
        assert!(matches!(
            &results,
            Results::Value(Ok(ResultValue {
                kind: ValueKind::BoxPlot(_),
                ..
            }))
        ));
        assert_eq!(results.refillable_text(), Some(expected.as_str()));
        assert_eq!(transform_csv(&format!("列箱线摘要 2 {data}")), Ok(expected));
        assert!(matches!(
            transform_csv("列箱线摘要 2 编号,金额\nA,\nB,待定"),
            Err(CsvError::DataRowValue { row: 2, .. })
        ));
    }

    #[test]
    fn suggests_the_saved_extension_of_json_results_without_confusing_csv_or_text() {
        for (query, expected) in [
            (r#"JSON: 格式化 {"编号":"001"}"#, "搜索结果.json"),
            ("JSON: TO-JSONL [1.50]", "搜索结果.jsonl"),
            ("JSON: 展开数组 []", "搜索结果.jsonl"),
            ("JSON: 转CSV [[1]]", "搜索结果.csv"),
            ("JSON: 对比CSV [1] [2]", "搜索结果.csv"),
            ("JSON: 校验行 1", "搜索结果.txt"),
            ("TOML: 取值 /port port = 8000", "搜索结果.json"),
            ("TOML: 转JSON port = 8000", "搜索结果.json"),
            (r#"TOML: 从JSON {"port":8000}"#, "搜索结果.toml"),
            ("TOML: 校验 port = 8000", "搜索结果.txt"),
            ("处理: JSON行 [\"分数:\"]\n0.50\n1.25", "搜索结果.jsonl"),
            (r#"处理: JSON对象 ["分数:"] {"茶":1.50}"#, "搜索结果.json"),
            (r#"处理: ["文本: 原文"] 1"#, "搜索结果.txt"),
            ("2**10", "搜索结果.txt"),
        ] {
            let Results::Value(Ok(result)) = for_query(query, &[]) else {
                panic!("搜索框没有生成可保存的结果：{query}");
            };
            assert_eq!(result.suggested_file_name(query), expected, "{query}");
        }
    }

    #[test]
    fn saves_original_csv_bytes_and_replaces_an_existing_result_atomically() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("结果.csv");
        std::fs::write(&target, "旧结果").unwrap();
        let csv = "编号,备注\r\n001,\"茶,咖啡\"\r\n";
        let result = for_query(&format!("CSV: 表格预览 {csv}"), &[]);
        crate::settings::atomic_write_labeled(
            &target,
            result.refillable_text().unwrap().as_bytes(),
            "搜索结果",
        )
        .unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), csv.as_bytes());
        let blocked = directory.path().join("不可替换");
        std::fs::create_dir(&blocked).unwrap();
        let existing = blocked.join("已存在.txt");
        std::fs::write(&existing, "原始数据").unwrap();
        let error =
            crate::settings::atomic_write_labeled(&blocked, b"new", "搜索结果").unwrap_err();
        assert!(error.contains("保存搜索结果失败"));
        assert_eq!(std::fs::read_to_string(existing).unwrap(), "原始数据");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2);
    }

    #[test]
    fn previews_spreadsheet_paste_as_csv_with_the_same_quoted_cell_values() {
        let data = "编号\t备注\r\n001\t\"茶,咖啡\"\r\n002\t\"带\r\n换行\"";
        let expected = "编号,备注\r\n001,\"茶,咖啡\"\r\n002,\"带\r\n换行\"";
        for query in [
            format!("CSV: 从TSV {data}"),
            format!("CSV: FROM-TSV {data}"),
            format!("处理: [\"CSV: 从TSV\"] {data}"),
        ] {
            let result = for_query(&query, &[]);
            assert!(
                matches!(
                    &result,
                    Results::Value(Ok(value)) if matches!(&value.kind, ValueKind::Table(_))
                ),
                "{query}"
            );
            assert_eq!(result.refillable_text(), Some(expected), "{query}");
        }
        let query = format!("处理: [\"CSV: 从TSV\",\"CSV: 选列 1\"] {data}");
        assert_eq!(
            for_query(&query, &[]).refillable_text(),
            Some("编号\r\n001\r\n002")
        );
    }

    #[test]
    fn previews_toml_leaf_types_and_paths_as_a_csv_table() {
        let source = concat!(
            "flags = [true, false]\nserial = 9_007_199_254_740_993\n",
            "[\"a/b\"]\ncreated = 2024-02-29\nnote = '茶,咖啡'\nratio = nan"
        );
        let expected = concat!(
            "路径,类型,值\r\n/a~1b/created,datetime,2024-02-29\r\n",
            "/a~1b/note,string,\"茶,咖啡\"\r\n/a~1b/ratio,float,nan\r\n",
            "/flags/0,boolean,true\r\n/flags/1,boolean,false\r\n/serial,integer,9007199254740993"
        );
        let query = format!("TOML: 叶子CSV {source}");
        let result = for_query(&query, &[]);
        assert!(matches!(
            &result,
            Results::Value(Ok(ResultValue {
                kind: ValueKind::Table(_),
                ..
            }))
        ));
        assert_eq!(result.refillable_text(), Some(expected));
        let pipeline = format!("[\"TOML: LEAVES-CSV\"] {source}");
        let (text, preview) =
            transform_data_pipeline(&pipeline, chrono::DateTime::UNIX_EPOCH.fixed_offset())
                .unwrap();
        assert_eq!(text, expected);
        assert!(matches!(preview, Some(PipelinePreview::Table(table)) if table.row_pages() == 1));
        assert_eq!(
            transform_toml("叶子CSV list = []\n[empty]").as_deref(),
            Ok("路径,类型,值\r\n/empty,table,{}\r\n/list,array,[]")
        );
    }

    #[test]
    fn previews_json_csv_reports_with_headers_and_preserves_their_exact_text() {
        for term in [
            r#"转CSV [{"编号":"001","数量":1.20},{"编号":"002","备注":"甲,乙"}]"#,
            r#"行数组转CSV ["名称","金额"] [["茶,咖啡",1.50],["梨",null]]"#,
            r#"路径转CSV ["/订单/金额"] [{"订单":{"金额":1.50}},{"订单":{}}]"#,
            r#"LEAVES-CSV {"金额":[1.20e+2,null],"备注":"甲,乙"}"#,
            r#"找键CSV "金额" {"金额":1.50,"a/b":{"金额":null},"订单":[{"金额":2}]}"#,
            r#"FIND-VALUE-CSV 1.5 {"甲":1.50,"乙":1.5e0,"丙":"1.5"}"#,
        ] {
            let expected = transform_json(term).unwrap();
            let direct = for_query(&format!("JSON: {term}"), &[]);
            assert!(matches!(
                &direct,
                Results::Value(Ok(ResultValue {
                    kind: ValueKind::Table(_),
                    ..
                }))
            ));
            assert_eq!(direct.refillable_text(), Some(expected.as_str()));
            let (operation, input) = term.split_once(' ').unwrap();
            let pipeline = format!(r#"["JSON: {operation}"] {input}"#);
            let (text, preview) =
                transform_data_pipeline(&pipeline, chrono::DateTime::UNIX_EPOCH.fixed_offset())
                    .unwrap();
            assert_eq!(text, expected);
            assert!(matches!(preview, Some(PipelinePreview::Table(_))), "{term}");
        }
        assert!(matches!(
            for_query("JSON: 转CSV [[1,2],[3,4]]", &[]),
            Results::Value(Ok(ResultValue { text, kind: ValueKind::Text }))
                if text == "1,2\r\n3,4"
        ));
    }

    #[test]
    fn previews_json_lines_as_csv_and_bounds_expanded_object_tables() {
        let lines = "{\"编号\":\"001\",\"销售\":1.50}\r\n\r\n{\"编号\":\"002\",\"备注\":\"甲,乙\"}";
        let query = format!("JSON: 行转CSV {lines}");
        let result = for_query(&query, &[]);
        assert!(matches!(
            &result,
            Results::Value(Ok(ResultValue {
                kind: ValueKind::Table(_),
                ..
            }))
        ));
        assert_eq!(
            result.refillable_text(),
            Some("编号,销售,备注\r\n001,1.50,\"\"\r\n002,\"\",\"甲,乙\"")
        );
        assert_eq!(
            transform_json("行转CSV {\"编号\":1}\n{"),
            Err(JsonError::InvalidLine)
        );
        assert_eq!(
            transform_json("行转CSV {\"编号\":1}\n42"),
            Err(JsonError::CsvLineObjects)
        );
        let sparse = (0..640)
            .map(|index| format!(r#"{{"列{index}":1}}"#))
            .collect::<Vec<_>>();
        assert_eq!(
            transform_json(&format!("jsonl-to-csv {}", sparse.join("\n"))),
            Err(JsonError::CsvOutputTooLarge)
        );
        assert_eq!(
            transform_json(&format!("转CSV [{}]", sparse.join(","))),
            Err(JsonError::CsvOutputTooLarge)
        );
    }

    #[test]
    fn exports_matching_json_paths_with_exact_raw_values() {
        assert_eq!(
            transform_json(r#"找值CSV 1.5 {"甲":1.50,"乙":1.5e0,"丙":"1.5"}"#).as_deref(),
            Ok("指针,类型,原值\r\n/甲,number,1.50\r\n/乙,number,1.5e0")
        );
        assert_eq!(
            transform_json(r#"find-key-csv "金额" {"金额":1.50,"a/b":{"金额":null},"订单":[{"金额":2}]}"#)
                .as_deref(),
            Ok("指针,类型,原值\r\n/金额,number,1.50\r\n/a~1b/金额,null,null\r\n/订单/0/金额,number,2")
        );
        assert_eq!(
            transform_json(r#"找键CSV "缺失" {"金额":1}"#).as_deref(),
            Ok("指针,类型,原值")
        );
        assert_eq!(
            transform_json(r#"找键CSV "\u0000" {"\u0000":1}"#),
            Err(JsonError::CsvNullCharacter)
        );
    }

    #[test]
    fn converts_json_rows_with_explicit_headers_and_validates_their_width() {
        assert_eq!(
            transform_json(r#"rows-to-csv ["名称,备注","金额"] [["茶,咖啡",1.50],["梨",null]]"#)
                .as_deref(),
            Ok("\"名称,备注\",金额\r\n\"茶,咖啡\",1.50\r\n梨,\"\"")
        );
        assert_eq!(
            transform_json(r#"行数组转CSV ["金额"] []"#).as_deref(),
            Ok("金额")
        );
        assert_eq!(
            transform_json(r#"行数组转CSV ["金额","金额"] [[1,2]]"#),
            Err(JsonError::CsvRowHeaders)
        );
        assert_eq!(
            transform_json(r#"行数组转CSV ["品名","金额"] [["茶"]]"#),
            Err(JsonError::CsvRows)
        );
    }

    #[test]
    fn reserves_a_second_footer_row_only_when_table_paginates_on_both_axes() {
        for (data_rows, extra) in [(2, 0.), (8, TABLE_PAGER_HEIGHT + TABLE_FOOTER_ROW_GAP)] {
            let csv = format!("A,B,C,D,E,F,G,H{}", "\n1,2,3,4,5,6,7,8".repeat(data_rows));
            let result = Results::table_value(Ok(csv));
            let table = result.table_preview().expect("应返回表格");
            assert_eq!(
                result.height(MAX_VALUE_LINES),
                158. + (table.preview_height() + extra) + 52.
            );
        }
    }

    #[test]
    fn reserves_footer_space_for_input_validation_repair() {
        for (query, location) in [
            ("CSV: 校验 a,b\n1,\"未闭合", "第 2 条记录 · 第 2 列"),
            ("JSON: 校验 [\n  1,\n]", "第 3 行 · 第 1 列"),
            ("TOML: 校验 [server", "第 1 行第 8 列"),
        ] {
            let result = for_query(query, &[]);
            let report = result.refillable_text().unwrap().to_owned();
            assert!(report.contains(location));
            assert_eq!(
                result.height(MAX_VALUE_LINES),
                Results::text_value(Ok(report)).height(MAX_VALUE_LINES) + 24.
            );
        }
        assert!(matches!(
            for_query("JSON: 校验 []", &[]),
            Results::Value(Ok(ResultValue { text, kind: ValueKind::Text })) if text == "JSON 有效"
        ));
    }

    #[test]
    fn csv_table_preview_keeps_original_csv_for_copy_and_later_pipeline_steps() {
        let input = "编号,金额,备注\r\n001,3.50,\"甲,乙\"\r\n002,2,\"带\r\n换行\"";
        let direct = for_query(&format!("CSV: 表格预览 {input}"), &[]);
        assert!(matches!(
            &direct,
            Results::Value(Ok(ResultValue {
                kind: ValueKind::Table(_),
                ..
            }))
        ));
        assert!(direct.value_preview_expandable());
        assert_eq!(direct.refillable_text(), Some(input));
        let pipeline = for_query(
            r#"处理: ["文本: 拆分成行 \";\"","CSV: 表格预览"] 货品,数量;茶,2;米,3"#,
            &[],
        );
        assert!(matches!(
            &pipeline,
            Results::Value(Ok(ResultValue {
                kind: ValueKind::Table(_),
                ..
            }))
        ));
        assert_eq!(pipeline.refillable_text(), Some("货品,数量\n茶,2\n米,3"));
        let continued = for_query(
            &format!(r#"处理: ["CSV: 表格预览","CSV: 选列 2"] {input}"#),
            &[],
        );
        assert_eq!(continued.refillable_text(), Some("金额\r\n3.50\r\n2"));
        assert_eq!(
            transform_csv("表格预览 A,B\n1"),
            Err(CsvError::DifferentColumnCount)
        );
    }

    #[test]
    fn equation_sampling_previews_columns_and_keeps_the_original_csv() {
        let query = r#"方程: 参数采样 ["sqrt(t)","t²"] @ -1..1; 3"#;
        let result = for_query(query, &[]);
        assert!(matches!(
            &result,
            Results::Value(Ok(ResultValue {
                kind: ValueKind::Table(_),
                ..
            }))
        ));
        assert!(result.value_preview_expandable());
        assert_eq!(
            result.refillable_text(),
            Some("t,x,y\r\n-1,\"\",1\r\n0,0,0\r\n1,1,1")
        );
        let scalar = for_query("方程: 数值弧长 x @ 0..1; 8", &[]);
        assert!(matches!(
            scalar,
            Results::Value(Ok(ResultValue {
                kind: ValueKind::Text,
                ..
            }))
        ));
    }

    #[test]
    fn csv_column_bin_chart_uses_a_named_column_and_retains_exact_bin_data() {
        let input = r#"["金额","0,1/2,2"] 编号,金额
001,-1
002,0
003,0.50
004,1
005,2"#;
        let results = for_query(&format!("CSV: 列分箱图 {input}"), &[]);
        assert!(matches!(
            &results,
            Results::Value(Ok(ResultValue { kind: ValueKind::BinPlot(plot), .. }))
                if plot.bin_count() == 4
        ));
        let expected = transform_csv(&format!("列分箱 {input}")).unwrap();
        assert_eq!(results.refillable_text(), Some(expected.as_str()));
    }

    #[test]
    fn csv_function_bin_chart_uses_finite_values_and_includes_the_lower_edge() {
        let input = "[[\"数值\"],\"sqrt(x)\",\"0.3,2\"] 数值\n0\n0.09\n1\n4\n-1\n\"\"";
        let expected = "下界,上界,次数,频率\r\n-∞,0.3,1,1/4\r\n0.3,2,2,1/2\r\n2,+∞,1,1/4";
        let results = for_query(&format!("CSV: 函数分箱图 {input}"), &[]);
        assert!(matches!(
            &results,
            Results::Value(Ok(ResultValue { kind: ValueKind::BinPlot(plot), .. }))
                if plot.bin_count() == 3
        ));
        assert_eq!(results.refillable_text(), Some(expected));
        assert_eq!(
            transform_csv(&format!("function-bins {input}")).as_deref(),
            Ok(expected)
        );
        let pipeline = for_query(
            r#"处理: ["文本: 拆分成行 \";\"","CSV: 函数分箱图 [[1,2],\"x*y\",\"0,2\"]"] X,Y;1,2;-1,3;2,1"#,
            &[],
        );
        assert!(matches!(
            &pipeline,
            Results::Value(Ok(ResultValue { kind: ValueKind::BinPlot(plot), .. }))
                if plot.bin_count() == 3
        ));
        assert_eq!(
            pipeline.refillable_text(),
            Some("下界,上界,次数,频率\r\n-∞,0,1,1/3\r\n0,2,0,0\r\n2,+∞,2,2/3")
        );
        assert_eq!(
            transform_csv("函数分箱 [[1],\"x\",\"1,1.00000000000000001\"] X\n1"),
            Err(CsvError::FunctionBinEdges)
        );
        assert_eq!(
            transform_csv("函数分箱 [[1],\"sqrt(x)\",\"0\"] X\n-1"),
            Err(CsvError::FunctionBinEmpty)
        );
    }

    #[test]
    fn cumulative_csv_column_chart_preserves_fractions_and_skips_blank_values() {
        let data = "编号,金额\nA,1/2\nB,0.5\nC,\nD,3\nE,5";
        let results = for_query(&format!(r#"CSV: 列累计分布图 "金额" {data}"#), &[]);
        assert!(matches!(
            &results,
            Results::Value(Ok(ResultValue { kind: ValueKind::Scatter(plot), .. }))
                if plot.point_count() == 3 && plot.has_curve()
        ));
        let expected = "值,累计次数,累计频率\r\n1/2,2,1/2\r\n3,3,3/4\r\n5,4,1";
        assert_eq!(results.refillable_text(), Some(expected));
        assert_eq!(
            transform_csv(&format!("列累计分布 2 {data}")).as_deref(),
            Ok(expected)
        );
    }

    #[test]
    fn quadratic_function_plot_keeps_exact_endpoints_and_an_off_grid_vertex() {
        let input = "(x-1)**2 - 1 @ 4 到 -1";
        let chart = for_query(&format!("方程: 函数图 {input}"), &[]);
        assert!(matches!(
            &chart,
            Results::Value(Ok(ResultValue { kind: ValueKind::Scatter(plot), .. }))
                if plot.has_curve()
        ));
        let csv = chart.refillable_text().unwrap();
        assert!(csv.starts_with("x,y\r\n-1,3\r\n"));
        assert!(csv.contains("\r\n1,-1\r\n"));
        assert!(csv.ends_with("\r\n4,8"));
        assert_eq!(csv.lines().count(), 35);
        let pipeline = for_query(
            r#"处理: [{"模板":"方程: 函数图 {结果} @ 4 到 -1"}] (x-1)**2 - 1"#,
            &[],
        );
        assert!(matches!(
            &pipeline,
            Results::Value(Ok(ResultValue { kind: ValueKind::Scatter(plot), .. }))
                if plot.has_curve()
        ));
        assert_eq!(pipeline.refillable_text(), Some(csv));
        assert_eq!(
            equation::solve_equation("函数图 x² @ 2 到 2"),
            Err(EquationError::FunctionPlotArguments)
        );
    }

    #[test]
    fn previews_exact_weighted_cumulative_distributions_including_ties() {
        let term = "加权累积分布 4@25%,1/2@1/8,4@1/8,-1@0,2@1/4";
        let expected = "值,累计权重,累计比例\r\n1/2,1/8,1/6\r\n2,3/8,1/2\r\n4,3/4,1";
        assert_eq!(calculate_statistics(term).as_deref(), Ok(expected));
        assert!(matches!(
            for_query(&format!("统计: {term}"), &[]),
            Results::Value(Ok(ResultValue { text, kind: ValueKind::Table(_) })) if text == expected
        ));
        let now = chrono::DateTime::UNIX_EPOCH.fixed_offset();
        let pipeline = r#"["统计: 加权累积分布"] 4@25%,1/2@1/8,4@1/8,-1@0,2@1/4"#;
        let (text, preview) = transform_data_pipeline(pipeline, now).unwrap();
        assert_eq!(text, expected);
        assert!(matches!(preview, Some(PipelinePreview::Table(table)) if table.row_pages() == 1));
    }

    #[test]
    fn cumulative_chart_uses_unique_values_and_preserves_large_exact_counts() {
        let arguments = "频数 -1@1,0.50@2,1/2@18446744073709551612";
        let result = for_query(&format!("统计: 累计分布图 {arguments}"), &[]);
        assert!(matches!(
            &result,
            Results::Value(Ok(ResultValue { kind: ValueKind::Scatter(plot), .. }))
                if plot.point_count() == 2 && plot.has_curve()
        ));
        assert_eq!(
            result.refillable_text(),
            Some(
                "值,累计次数,累计频率\r\n-1,1,1/18446744073709551615\r\n1/2,18446744073709551615,1"
            )
        );
    }

    #[test]
    fn binned_chart_preserves_weighted_counts_and_the_exact_csv() {
        let arguments = "0,1 @ 频数 -1@1,0@18446744073709551614";
        let plot = for_query(&format!("统计: 分箱图 {arguments}"), &[]);
        assert!(matches!(
            &plot,
            Results::Value(Ok(ResultValue { kind: ValueKind::BinPlot(plot), .. }))
                if plot.bin_count() == 3
        ));
        let csv = calculate_statistics(&format!("分箱 {arguments}")).unwrap();
        assert_eq!(plot.refillable_text(), Some(csv.as_str()));
        assert!(plot.value_preview_expandable());
    }

    #[test]
    fn pipeline_previews_its_final_chart_and_keeps_the_original_trace_format() {
        let source = r#"["CSV: 数值筛选 2 >= 0","CSV: 散点图 [2,3]"] 编号,时间,温度
001,-1,2
002,0,1/2
003,1,5/2"#;
        let plot = for_query(&format!("处理: {source}"), &[]);
        assert!(matches!(
            &plot,
            Results::Value(Ok(ResultValue { kind: ValueKind::Scatter(plot), .. }))
                if plot.point_count() == 2
        ));
        assert_eq!(plot.refillable_text(), Some("时间,温度\r\n0,1/2\r\n1,5/2"));
        let trace = for_query(&format!("处理: 步骤CSV {source}"), &[]);
        assert!(matches!(
            &trace,
            Results::Value(Ok(ResultValue {
                kind: ValueKind::Table(_),
                ..
            }))
        ));
        assert_eq!(
            trace.refillable_text(),
            Some(concat!(
                "步骤,命令,结果\r\n",
                "1,CSV: 数值筛选 2 >= 0,\"编号,时间,温度\r\n002,0,1/2\r\n003,1,5/2\"\r\n",
                "2,\"CSV: 散点图 [2,3]\",\"时间,温度\r\n0,1/2\r\n1,5/2\""
            ))
        );

        let box_plot = for_query(r#"处理: ["分数: 计算","统计: 箱线图"] 2**10"#, &[]);
        assert!(matches!(
            box_plot,
            Results::Value(Ok(ResultValue {
                kind: ValueKind::BoxPlot(_),
                ..
            }))
        ));
    }

    #[test]
    fn box_plot_copies_the_same_exact_summary_as_the_text_command() {
        let values = "频数 1@1,2@1,3@1,4@1,8@1";
        let plot = for_query(&format!("统计: 箱线图 {values}"), &[]);
        let expected = calculate_statistics(&format!("箱线摘要 {values}")).unwrap();
        assert!(matches!(
            &plot,
            Results::Value(Ok(ResultValue {
                kind: ValueKind::BoxPlot(_),
                ..
            }))
        ));
        assert_eq!(plot.refillable_text(), Some(expected.as_str()));
        assert_eq!(
            calculate_statistics(&format!("box-plot {values}")).as_deref(),
            Ok(expected.as_str())
        );
    }

    #[test]
    fn csv_scatter_renders_the_selected_columns_and_refills_the_same_schema() {
        let results = for_query(
            r#"CSV: 散点图 ["时间","温度"] 编号,时间,温度
001,0,2
002,1,5/2"#,
            &[],
        );
        assert!(matches!(
            &results,
            Results::Value(Ok(ResultValue { kind: ValueKind::Scatter(plot), .. }))
                if plot.point_count() == 2
        ));
        assert_eq!(results.refillable_text(), Some("时间,温度\r\n0,2\r\n1,5/2"));
    }

    #[test]
    fn polynomial_plot_preserves_the_full_exact_residual_table() {
        let source = "2 @ -1,2; 0,0; 1,2; 0,2";
        let plot = for_query(&format!("拟合: 曲线图 {source}"), &[]);
        let expected = calculate_linear_fit(&format!("多项式残差 {source}")).unwrap();
        assert!(matches!(
            &plot,
            Results::Value(Ok(ResultValue { kind: ValueKind::Scatter(plot), .. }))
                if plot.has_curve() && plot.point_count() == 4
        ));
        assert_eq!(plot.refillable_text(), Some(expected.as_str()));
        let residual_plot = for_query(&format!("拟合: 残差图 {source}"), &[]);
        assert!(matches!(
            &residual_plot,
            Results::Value(Ok(ResultValue { kind: ValueKind::Scatter(plot), .. }))
                if plot.has_curve() && plot.point_count() == 4
        ));
        assert_eq!(residual_plot.refillable_text(), Some(expected.as_str()));
        assert_eq!(
            calculate_linear_fit(&format!("fit-plot {source}")).as_deref(),
            Ok(expected.as_str())
        );
    }

    #[test]
    fn scatter_preview_preserves_exact_csv_for_copy_and_refill() {
        let query = "拟合: 散点图 100000000000000000000,1/2; 100000000000000000001,1/2";
        let results = for_query(query, &[]);
        assert!(results.value_preview_expandable());
        assert!(results.height(MAX_VALUE_LINES) > results.height(EXPANDED_VALUE_LINES));
        let expected = "x,y\r\n100000000000000000000,1/2\r\n100000000000000000001,1/2";
        assert_eq!(results.refillable_text(), Some(expected));
        let Results::Value(Ok(ResultValue {
            kind: ValueKind::Scatter(plot),
            text,
        })) = results
        else {
            panic!("散点图没有返回图形预览");
        };
        assert_eq!(plot.point_count(), 2);
        assert_eq!(text, expected);
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
            ("? JSON:", "JSON:"),
            ("CSV", "CSV:"),
            ("ｔｅｘｔ　ｔｏｏｌｓ", "文本:"),
            ("文本工具", "文本:"),
            ("处理", "处理:"),
        ] {
            let Results::Help(items) = for_query(query, &[]) else {
                panic!("未显示对应工具的示例：{query}");
            };
            assert!(!items.is_empty(), "{query}");
            assert!(
                items
                    .iter()
                    .all(|&index| EXAMPLES[index].query.starts_with(prefix)),
                "{query}"
            );
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
    fn finds_the_replaceable_arguments_for_structured_example_commands() {
        for (query, prefix, arguments) in [
            (
                "JSON: 取值 /价格 {\"价格\":1}",
                "JSON: 取值 ",
                "/价格 {\"价格\":1}",
            ),
            (
                "ＣＳＶ： 压缩\n标题,值\r\n甲,1",
                "ＣＳＶ： 压缩\n",
                "标题,值\r\n甲,1",
            ),
            ("文本: 大写 👩‍💻 x", "文本: 大写 ", "👩‍💻 x"),
            ("URL: 解码 %2B", "URL: 解码 ", "%2B"),
            ("音高: MIDI 69", "音高: MIDI ", "69"),
            ("分贝: 转幅度比 20 dB", "分贝: 转幅度比 ", "20 dB"),
            ("节拍: 时长 120 BPM @ 3/2", "节拍: 时长 ", "120 BPM @ 3/2"),
            ("立体: 圆柱 3,5", "立体: 圆柱 ", "3,5"),
            (
                "日期: 星期统计 2024-02-29 至 2024-03-09",
                "日期: 星期统计 ",
                "2024-02-29 至 2024-03-09",
            ),
            ("时长: 分摊 01:30:00 @ 1:2", "时长: 分摊 ", "01:30:00 @ 1:2"),
            ("进制: 罗马数字 CDXCIX", "进制: 罗马数字 ", "CDXCIX"),
            (
                "路径: 文件名 C:\\工作\\报告.txt",
                "路径: 文件名 ",
                "C:\\工作\\报告.txt",
            ),
            (
                "UUID: v5 DNS www.example.com",
                "UUID: v5 ",
                "DNS www.example.com",
            ),
            (
                "ULID: 时间 01ARZ3NDEKTSV4RRFFQ69G5FAV",
                "ULID: 时间 ",
                "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            ),
        ] {
            let example = examples::Example {
                icon: Icon::Terminal,
                title: "",
                query,
            };
            let start = example.argument_start().expect("has example arguments");
            assert_eq!(query.split_at(start), (prefix, arguments));
        }
        for query in [
            "2**10",
            "日期: 2024-01-01 + 1d",
            "进制: ff 从 16 到 10",
            "时长: 23:45 到 01:15",
            "音高: 442 Hz",
            "节拍: 120 BPM",
            "JSON: 取值",
            "CSV: 转置  ",
            "UUID: v4",
            "ULID: 01ARZ3NDEKTSV4RRFFQ69G5FAV",
        ] {
            let example = examples::Example {
                icon: Icon::Terminal,
                title: "",
                query,
            };
            assert_eq!(example.argument_start(), None);
        }
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
        assert!(examples_for("JSON: 校验")[0].starts_with("JSON: 校验\n"));
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
    fn verify_input_error_selection(
        search: &mut QuickSearch,
        window: &mut Window,
        cx: &mut Context<QuickSearch>,
    ) {
        for (query, invalid) in [
            ("批量计算: 1+2\r\n\r\n  1/0  \r\n4+5", "1/0"),
            ("批量计算: 1+2\r\u{2028}  1/0  \u{2029}4+5", "1/0"),
            ("批量计算： 分数对照 1/2\r\n  2//0  \r\n3/4", "2//0"),
            (
                " CSV：校验 \u{feff}编号,备注\r\n001,\"茶\n梨\"\r\n002,\"未闭合 ",
                "\"未闭合 ",
            ),
            ("CSV: VALIDATE 编号,备注\n001,\"甲\"丁\n002,乙", "丁"),
            ("CSV: 校验 a,b\n001,带\"引号", "\""),
            (
                "CSV: 校验TSV 编号\t备注\r\n001\t茶\r\n002\t梨\t额外",
                "002\t梨\t额外",
            ),
            ("  JSON： 校验\r\n{\r\n  \"参数\": \"茶\"坏\r\n}   ", "坏"),
            ("JSON: VALIDATE [\"😀\",好]", "好"),
            ("JSON: 校验 [1,\r\n  ", ""),
            ("JSON: 校验行\r\nnull\r\n\r\n \"茶\"坏 \r\n[", " \"茶\"坏 "),
            ("TOML： 校验\r\nname = '茶'\r\nname = '咖啡' ", "name"),
            ("TOML: 校验 list = [1,2", ""),
            (
                "CSV: 校验 a,b\r\n01,\"甲\n乙\",多余\r\n02",
                "01,\"甲\n乙\",多余",
            ),
        ] {
            search.fill_query(query, window, cx);
            search.refresh_results(query, window, cx);
            let mut event = KeyDownEvent {
                keystroke: gpui::Keystroke::parse("alt-enter").unwrap(),
                is_held: true,
            };
            search.key_down(&event, window, cx);
            assert_eq!(search.input.read(cx).selection(), query.len()..query.len());
            event.is_held = false;
            search.key_down(&event, window, cx);
            let input = search.input.read(cx);
            if invalid.is_empty() {
                assert_eq!(input.selection(), query.len()..query.len());
            }
            assert_eq!(&query[input.selection()], invalid);
            assert_eq!(input.text(), query);
        }
        for (query, invalid) in [
            (
                "  批量计算： 明细 2**8\r\n\r1/0\u{2028}avg(1,3)\nsqrt(-1)  ",
                ["1/0", "sqrt(-1)", "1/0"],
            ),
            (
                "batch calculation: exact-report 2**80\n1/0\r\n\r7/6\nsqrt(-1)",
                ["1/0", "sqrt(-1)", "1/0"],
            ),
            (
                " JSON： 校验行CSV\r\nnull\r\n\r\n {坏}  \r\n[1,2]\r\n  [",
                [" {坏}  ", "  [", " {坏}  "],
            ),
        ] {
            search.fill_query(query, window, cx);
            search.refresh_results(query, window, cx);
            let mut event = KeyDownEvent {
                keystroke: gpui::Keystroke::parse("alt-enter").unwrap(),
                is_held: false,
            };
            for expected in invalid {
                search.key_down(&event, window, cx);
                assert_eq!(&query[search.input.read(cx).selection()], expected);
                event.is_held = true;
                search.key_down(&event, window, cx);
                assert_eq!(&query[search.input.read(cx).selection()], expected);
                event.is_held = false;
            }
            assert_eq!(search.input.read(cx).text(), query);
        }
    }

    #[cfg(target_os = "windows")]
    fn verify_query_argument_selection(
        search: &mut QuickSearch,
        window: &mut Window,
        cx: &mut Context<QuickSearch>,
    ) {
        for (query, expected) in [
            ("sqrt(81+19)", "81+19"),
            ("分数：计算 1/3+1/6", "1/3+1/6"),
            ("文本: 大写 北京", "北京"),
            (
                "JSON: 校验行CSV\r\nnull\r\n{\"编号\":1}",
                "null\r\n{\"编号\":1}",
            ),
        ] {
            search.fill_query(query, window, cx);
            let mut event = KeyDownEvent {
                keystroke: gpui::Keystroke::parse("ctrl-shift-a").unwrap(),
                is_held: true,
            };
            search.key_down(&event, window, cx);
            assert_eq!(search.input.read(cx).selection(), query.len()..query.len());
            event.is_held = false;
            search.key_down(&event, window, cx);
            assert_eq!(&query[search.input.read(cx).selection()], expected);
            assert_eq!(search.input.read(cx).text(), query);
        }
    }

    #[cfg(target_os = "windows")]
    fn verify_go_to_input_line(
        search: &mut QuickSearch,
        window: &mut Window,
        cx: &mut Context<QuickSearch>,
    ) {
        let initial = "批量计算: 明细 2**8\r\n\r1/0\u{2028}avg(1,3)";
        search.fill_query(initial, window, cx);
        search.input.update(cx, |input, cx| {
            input.replace_text_in_range(None, "\nsqrt(-1)", window, cx);
        });
        let query = format!("{initial}\nsqrt(-1)");
        search.refresh_results(&query, window, cx);
        let original = query.find("2**8").unwrap();
        let selection = original..original + "2**8".len();
        search
            .input
            .update(cx, |input, cx| input.select_range(selection.clone(), cx));
        let key = |name, is_held| KeyDownEvent {
            keystroke: gpui::Keystroke::parse(name).unwrap(),
            is_held,
        };
        search.key_down(&key("ctrl-g", true), window, cx);
        assert!(search.line_jump.is_none());
        search.key_down(&key("ctrl-g", false), window, cx);
        let jump = search.line_jump.as_ref().unwrap();
        assert_eq!(jump.number(cx), Some(1));
        let entry = jump.input.clone();
        assert_eq!(entry.read(cx).selection(), 0..1);
        entry.update(cx, |input, cx| input.set_text("0", cx));
        search.key_down(&key("enter", false), window, cx);
        assert!(search.line_jump.is_some());
        search.key_down(&key("escape", false), window, cx);
        assert!(search.line_jump.is_none());
        assert_eq!(search.input.read(cx).selection(), selection);

        search.key_down(&key("ctrl-g", false), window, cx);
        let entry = search.line_jump.as_ref().unwrap().input.clone();
        entry.update(cx, |input, cx| input.set_text("３", cx));
        search.key_down(&key("enter", true), window, cx);
        assert!(search.line_jump.is_some());
        search.key_down(&key("enter", false), window, cx);
        assert!(search.line_jump.is_none());
        let expected = query.find("1/0").unwrap();
        assert_eq!(search.input.read(cx).selection(), expected..expected);
        assert_eq!(search.input.read(cx).text(), query);
        search.key_down(&key("ctrl-z", false), window, cx);
        assert_eq!(search.input.read(cx).text(), initial);
    }

    #[cfg(target_os = "windows")]
    fn verify_input_find(
        search: &mut QuickSearch,
        window: &mut Window,
        cx: &mut Context<QuickSearch>,
    ) {
        let query = "文本: 原文 北区-A\r\n南区\n北区-B";
        search.fill_query(query, window, cx);
        let first = query.find("北区").unwrap();
        let last = query.rfind("北区").unwrap();
        let width = "北区".len();
        search
            .input
            .update(cx, |input, cx| input.select_range(first..first + width, cx));
        let key = |key, held| KeyDownEvent {
            keystroke: gpui::Keystroke::parse(key).unwrap(),
            is_held: held,
        };
        search.key_down(&key("ctrl-shift-f", true), window, cx);
        assert!(search.input_find.is_none());
        search.key_down(&key("ctrl-shift-f", false), window, cx);
        let entry = search.input_find.as_ref().unwrap().input.clone();
        assert_eq!(entry.read(cx).text(), "北区");
        search.key_down(&key("enter", true), window, cx);
        assert!(search.input_find.is_some());
        search.key_down(&key("enter", false), window, cx);
        assert_eq!(search.input.read(cx).selection(), last..last + width);
        search.key_down(&key("ctrl-shift-f", false), window, cx);
        search.key_down(&key("enter", false), window, cx);
        assert_eq!(search.input.read(cx).selection(), first..first + width);
        search.key_down(&key("ctrl-shift-f", false), window, cx);
        search.key_down(&key("shift-enter", false), window, cx);
        assert_eq!(search.input.read(cx).selection(), last..last + width);
        search.key_down(&key("ctrl-shift-f", false), window, cx);
        let entry = search.input_find.as_ref().unwrap().input.clone();
        entry.update(cx, |input, cx| input.set_text("未找到", cx));
        search.key_down(&key("enter", false), window, cx);
        assert!(search.input_find.is_some());
        search.key_down(&key("escape", false), window, cx);
        assert!(search.input_find.is_none());
        assert_eq!(search.input.read(cx).selection(), last..last + width);
        assert_eq!(search.input.read(cx).text(), query);
    }

    #[cfg(target_os = "windows")]
    fn verify_input_replace(
        search: &mut QuickSearch,
        window: &mut Window,
        cx: &mut Context<QuickSearch>,
    ) {
        let key = |key, is_held| KeyDownEvent {
            keystroke: gpui::Keystroke::parse(key).unwrap(),
            is_held,
        };
        let query = "文本: 原文 😀A+\r\nA+\u{2028}a+";
        let start = query.find("A+").unwrap();
        let selection = start..start + 2;
        search.fill_query(query, window, cx);
        search
            .input
            .update(cx, |input, cx| input.select_range(selection.clone(), cx));
        search.key_down(&key("ctrl-h", false), window, cx);
        let find = search.input_find.as_ref().unwrap();
        assert_eq!(find.input.read(cx).text_ref(), "A+");
        let replacement = find.replacement.as_ref().unwrap().input.clone();
        search.key_down(&key("tab", false), window, cx);
        assert!(replacement.focus_handle(cx).is_focused(window));
        replacement.update(cx, |input, cx| {
            input.set_text("北".repeat(INPUT_CHARACTER_LIMIT), cx)
        });
        search.key_down(&key("enter", false), window, cx);
        assert!(search.input_find.is_some());
        assert_eq!(search.input.read(cx).text_ref(), query);
        replacement.update(cx, |input, cx| input.set_text("北", cx));
        search.key_down(&key("enter", true), window, cx);
        assert!(search.input_find.is_some());
        search.key_down(&key("enter", false), window, cx);
        assert!(search.input_find.is_none());
        assert_eq!(
            search.input.read(cx).text_ref(),
            "文本: 原文 😀北\r\n北\u{2028}a+"
        );
        search.key_down(&key("ctrl-z", false), window, cx);
        assert_eq!(search.input.read(cx).text_ref(), query);
        assert_eq!(search.input.read(cx).selection(), selection);
        search.key_down(&key("ctrl-h", false), window, cx);
        search.key_down(&key("escape", false), window, cx);
        assert_eq!(search.input.read(cx).text_ref(), query);
        search.key_down(&key("ctrl-h", false), window, cx);
        search.key_down(&key("enter", false), window, cx);
        assert_eq!(
            search.input.read(cx).text_ref(),
            "文本: 原文 😀\r\n\u{2028}a+"
        );
    }

    #[cfg(target_os = "windows")]
    fn verify_result_copy_in_place(
        search: &mut QuickSearch,
        window: &mut Window,
        cx: &mut Context<QuickSearch>,
    ) {
        let query = "CSV: 表格预览 编号,备注\r\n001,\"茶\n梨\"";
        search.fill_query(query, window, cx);
        search.refresh_results(query, window, cx);
        let start = query.find("CSV").unwrap();
        search
            .input
            .update(cx, |input, cx| input.select_range(start..start + 3, cx));
        let held = KeyDownEvent {
            keystroke: gpui::Keystroke::parse("ctrl-enter").unwrap(),
            is_held: true,
        };
        search.key_down(&held, window, cx);
        assert!(!search.result_copied);
        let item = search.prepare_in_place_copy(cx).unwrap();
        assert_eq!(item.text().as_deref(), Some("编号,备注\r\n001,\"茶\n梨\""));
        assert_eq!(search.copy_button_label("复制"), "已复制");
        let input = search.input.read(cx);
        assert_eq!(input.text(), query);
        assert_eq!(input.selection(), start..start + 3);
        search.fill_query("2**10", window, cx);
        search.refresh_results("2**10", window, cx);
        assert_eq!(search.copy_button_label("复制"), "复制");
    }

    #[cfg(target_os = "windows")]
    fn verify_table_paging(
        search: &mut QuickSearch,
        window: &mut Window,
        cx: &mut Context<QuickSearch>,
    ) {
        let table = "a,b,c,d,e,f,g,h,i,j,k,l,m\n1,2,3,4,5,6,7,8,9,10,11,12,13";
        let data = format!(
            "{table}{}",
            "\n11,12,13,14,15,16,17,18,19,20,21,22,23".repeat(14)
        );
        let query = format!("CSV: 表格预览 {data}");
        search.fill_query(&query, window, cx);
        search.refresh_results(&query, window, cx);
        let pages = |search: &QuickSearch| (search.table_pages.rows, search.table_pages.columns);
        let press =
            |search: &mut QuickSearch, key, window: &mut Window, cx: &mut Context<QuickSearch>| {
                let event = KeyDownEvent {
                    keystroke: gpui::Keystroke::parse(key).unwrap(),
                    is_held: false,
                };
                search.key_down(&event, window, cx);
            };
        assert_eq!(pages(search), (0, 0));
        press(search, "pagedown", window, cx);
        assert_eq!(pages(search), (1, 0));
        press(search, "shift-pagedown", window, cx);
        assert_eq!(pages(search), (1, 1));
        press(search, "ctrl-shift-pageup", window, cx);
        assert_eq!(pages(search), (1, 0));
        press(search, "ctrl-shift-pagedown", window, cx);
        assert_eq!(pages(search), (1, 2));
        press(search, "shift-pageup", window, cx);
        assert_eq!(pages(search), (1, 1));
        press(search, "ctrl-pagedown", window, cx);
        assert_eq!(pages(search), (2, 1));
        assert!(search.copy_table(TableCopy::CurrentPageTsv, true, window, cx));
        press(search, "f2", window, cx);
        assert!(search.expanded_value_pages.is_some());
        assert!(!search.copy_table(TableCopy::CurrentPageTsv, true, window, cx));
        press(search, "ctrl-shift-g", window, cx);
        assert!(search.line_jump.is_none());
        press(search, "pageup", window, cx);
        press(search, "ctrl-shift-pagedown", window, cx);
        assert_eq!(pages(search), (2, 1));
        press(search, "f2", window, cx);
        assert_eq!(pages(search), (2, 1));
        assert!(search.expanded_value_pages.is_none());
        assert_eq!(search.results.refillable_text(), Some(data.as_str()));
        let selection = search.input.read(cx).selection();
        search.key_down(
            &KeyDownEvent {
                keystroke: gpui::Keystroke::parse("ctrl-shift-g").unwrap(),
                is_held: true,
            },
            window,
            cx,
        );
        assert!(search.line_jump.is_none());
        press(search, "ctrl-shift-g", window, cx);
        let jump = search.line_jump.as_ref().unwrap();
        assert_eq!(jump.total, 15);
        assert_eq!(jump.number(cx), Some(13));
        let entry = jump.input.clone();
        entry.update(cx, |input, cx| input.set_text("16", cx));
        press(search, "enter", window, cx);
        assert!(search.line_jump.is_some());
        press(search, "escape", window, cx);
        assert!(search.line_jump.is_none());
        assert_eq!(pages(search), (2, 1));
        press(search, "ctrl-alt-g", window, cx);
        let jump = search.line_jump.as_ref().unwrap();
        assert_eq!(jump.total, 13);
        assert_eq!(jump.input.read(cx).text_ref(), "7");
        let column = jump.input.clone();
        column.update(cx, |input, cx| input.set_text("14", cx));
        press(search, "enter", window, cx);
        assert!(search.line_jump.is_some());
        assert_eq!(pages(search), (2, 1));
        column.update(cx, |input, cx| input.set_text("１", cx));
        press(search, "enter", window, cx);
        assert!(search.line_jump.is_none());
        assert_eq!(pages(search), (2, 0));
        press(search, "ctrl-alt-g", window, cx);
        let column = search.line_jump.as_ref().unwrap().input.clone();
        column.update(cx, |input, cx| input.set_text("8", cx));
        press(search, "enter", window, cx);
        assert_eq!(pages(search), (2, 1));
        press(search, "ctrl-shift-g", window, cx);
        let entry = search.line_jump.as_ref().unwrap().input.clone();
        entry.update(cx, |input, cx| input.set_text("７", cx));
        press(search, "enter", window, cx);
        assert!(search.line_jump.is_none());
        assert_eq!(pages(search), (1, 1));
        assert_eq!(search.input.read(cx).text(), query);
        assert_eq!(search.input.read(cx).selection(), selection);
        search.fill_query("CSV: 表格预览 x,y\n1,2", window, cx);
        search.refresh_results("CSV: 表格预览 x,y\n1,2", window, cx);
        assert_eq!(pages(search), (0, 0));
        press(search, "ctrl-shift-g", window, cx);
        assert!(search.line_jump.is_none());
        press(search, "ctrl-alt-g", window, cx);
        assert!(search.line_jump.is_none());
    }

    #[cfg(target_os = "windows")]
    fn verify_table_cell_lookup(
        search: &mut QuickSearch,
        window: &mut Window,
        cx: &mut Context<QuickSearch>,
    ) {
        let mut query = "CSV: 表格预览 A,B,C,D,E,F,G,H".to_owned();
        for row in 1..=8 {
            query.push_str(&format!("\n{row},b,c,d,e,f,00{row},末列{row}"));
        }
        search.fill_query(query.clone(), window, cx);
        search.refresh_results(&query, window, cx);
        search.table_pages = TablePages {
            rows: 1,
            columns: 1,
        };
        search
            .input
            .update(cx, |input, cx| input.select_range(0..3, cx));
        let mut key = KeyDownEvent {
            keystroke: gpui::Keystroke::parse("ctrl-shift-j").unwrap(),
            is_held: true,
        };
        search.key_down(&key, window, cx);
        assert_eq!(search.input.read(cx).text(), query);
        key.is_held = false;
        search.key_down(&key, window, cx);
        let input = search.input.read(cx);
        assert!(input.text_ref().starts_with("CSV: 取单元格 [7,7] "));
        assert_eq!(&input.text_ref()[input.selection()], "7,7");
        let opened = search.input.read(cx).text();
        search.refresh_results(&opened, window, cx);
        assert_eq!(search.results.refillable_text(), Some("007"));
        search.input.update(cx, |input, cx| {
            input.replace_text_in_range(None, "-1,8", window, cx);
        });
        let edited = search.input.read(cx).text();
        search.refresh_results(&edited, window, cx);
        assert_eq!(search.results.refillable_text(), Some("末列8"));
        search.key_down(&key, window, cx);
        assert_eq!(search.input.read(cx).text(), query);
        assert_eq!(search.input.read(cx).selection(), 0..3);
        search.refresh_results(&query, window, cx);
        assert_eq!(
            (search.table_pages.rows, search.table_pages.columns),
            (1, 1)
        );
        assert!(!search.is_table_cell_lookup());
    }

    #[cfg(target_os = "windows")]
    fn verify_current_table_search(
        search: &mut QuickSearch,
        window: &mut Window,
        cx: &mut Context<QuickSearch>,
    ) {
        const QUERY: &str = "CSV: 表格预览 商品,数量\n绿茶,1\n米,2";
        let event = |key, is_held| KeyDownEvent {
            keystroke: gpui::Keystroke::parse(key).unwrap(),
            is_held,
        };
        search.fill_query(QUERY, window, cx);
        search.refresh_results(QUERY, window, cx);
        let selection = QUERY.find("绿茶").unwrap()..QUERY.find(",1").unwrap();
        search
            .input
            .update(cx, |input, cx| input.select_range(selection.clone(), cx));
        search.key_down(&event("ctrl-f", false), window, cx);
        let started = search.input.read(cx).text();
        assert_eq!(started, "CSV: 搜索行 \"\" 商品,数量\n绿茶,1\n米,2");
        let position = TABLE_SEARCH_COMMAND.len() + 1;
        assert_eq!(search.input.read(cx).selection(), position..position);
        search.key_down(&event("ctrl-f", true), window, cx);
        assert_eq!(search.input.read(cx).text(), started);
        search.input.update(cx, |input, cx| {
            input.replace_text_in_range(None, "茶", window, cx);
        });
        let filtered = search.input.read(cx).text();
        search.refresh_results(&filtered, window, cx);
        assert_eq!(
            search.results.refillable_text(),
            Some("商品,数量\r\n绿茶,1")
        );
        search.key_down(&event("f1", false), window, cx);
        let help = search.input.read(cx).text();
        search.refresh_results(&help, window, cx);
        assert_eq!(help, "? CSV: 搜索行");
        search.key_down(&event("ctrl-f", false), window, cx);
        assert_eq!(search.input.read(cx).text(), QUERY);
        assert_eq!(search.input.read(cx).selection(), selection);
        assert!(search.input_before_result_search.is_none());

        let mut query = "CSV: 表格预览 A,B,C,D,E,F,G,H".to_owned();
        for row in 1..=8 {
            query.push_str(&format!("\n{row},b,c,d,e,f,g,末列"));
        }
        search.fill_query(query.clone(), window, cx);
        search.refresh_results(&query, window, cx);
        search.key_down(&event("pagedown", false), window, cx);
        search.key_down(&event("shift-pagedown", false), window, cx);
        assert_eq!(
            (search.table_pages.rows, search.table_pages.columns),
            (1, 1)
        );
        search.key_down(&event("ctrl-f", false), window, cx);
        let filtering = search.input.read(cx).text();
        search.refresh_results(&filtering, window, cx);
        assert_eq!(
            (search.table_pages.rows, search.table_pages.columns),
            (0, 0)
        );
        search.key_down(&event("ctrl-f", false), window, cx);
        assert_eq!(search.input.read(cx).text(), query);
        search.refresh_results(&query, window, cx);
        assert_eq!(
            (search.table_pages.rows, search.table_pages.columns),
            (1, 1)
        );
        search.key_down(&event("f1", false), window, cx);
        let help = search.input.read(cx).text();
        search.refresh_results(&help, window, cx);
        search.key_down(&event("f1", false), window, cx);
        assert_eq!(search.input.read(cx).text(), query);
        search.refresh_results(&query, window, cx);
        assert_eq!(
            (search.table_pages.rows, search.table_pages.columns),
            (1, 1)
        );
    }

    #[cfg(target_os = "windows")]
    fn verify_expanded_result_restore(
        search: &mut QuickSearch,
        window: &mut Window,
        cx: &mut Context<QuickSearch>,
    ) {
        const QUERY: &str = "日期: 月末序列 2024-01,30";
        let key = |name| KeyDownEvent {
            keystroke: gpui::Keystroke::parse(name).unwrap(),
            is_held: false,
        };
        search.fill_query(QUERY, window, cx);
        search.refresh_results(QUERY, window, cx);
        search.key_down(&key("f2"), window, cx);
        search.key_down(&key("pagedown"), window, cx);
        let pages = search.expanded_value_pages.clone().unwrap();
        assert_eq!(pages.len(), 3);
        assert_eq!(search.selected, 1);
        search.key_down(&key("f1"), window, cx);
        let help = search.input.read(cx).text();
        search.refresh_results(&help, window, cx);
        assert!(search.expanded_value_pages.is_none());
        search.key_down(&key("f1"), window, cx);
        assert_eq!(search.input.read(cx).text(), QUERY);
        search.refresh_results(QUERY, window, cx);
        assert_eq!(search.expanded_value_pages, Some(pages));
        assert_eq!(search.selected, 1);
    }

    #[cfg(target_os = "windows")]
    fn verify_text_result_search(
        search: &mut QuickSearch,
        window: &mut Window,
        cx: &mut Context<QuickSearch>,
    ) {
        const QUERY: &str = "文本: 原文 北区: OK\r\n南区: OK\r北站待定";
        let key = |name, is_held| KeyDownEvent {
            keystroke: gpui::Keystroke::parse(name).unwrap(),
            is_held,
        };
        search.fill_query(QUERY, window, cx);
        search.refresh_results(QUERY, window, cx);
        let selection = QUERY.find("北区").unwrap()..QUERY.find("\r\n").unwrap();
        search
            .input
            .update(cx, |input, cx| input.select_range(selection.clone(), cx));
        search.key_down(&key("ctrl-f", false), window, cx);
        let started = search.input.read(cx).text();
        assert_eq!(
            started,
            "文本: 搜索行 \"\" \"北区: OK\r\n南区: OK\r北站待定\""
        );
        let caret = TEXT_SEARCH_COMMAND.len() + 1;
        assert_eq!(search.input.read(cx).selection(), caret..caret);
        search.refresh_results(&started, window, cx);
        assert_eq!(
            search.results.refillable_text(),
            Some("北区: OK\r\n南区: OK\r北站待定")
        );
        search.key_down(&key("ctrl-f", true), window, cx);
        assert_eq!(search.input.read(cx).text(), started);
        search.input.update(cx, |input, cx| {
            input.replace_text_in_range(None, "ok 北", window, cx);
        });
        let filtered = search.input.read(cx).text();
        search.refresh_results(&filtered, window, cx);
        assert_eq!(search.results.refillable_text(), Some("北区: OK"));
        search.key_down(&key("f1", false), window, cx);
        let help = search.input.read(cx).text();
        search.refresh_results(&help, window, cx);
        assert_eq!(help, "? 文本: 搜索行");
        search.key_down(&key("ctrl-f", false), window, cx);
        assert_eq!(search.input.read(cx).text(), QUERY);
        assert_eq!(search.input.read(cx).selection(), selection);
        assert!(search.input_before_result_search.is_none());
        const UNICODE_QUERY: &str = "文本: 原文 北区: OK\u{2028}南区: OK\u{2029}北站待定";
        search.fill_query(UNICODE_QUERY, window, cx);
        search.refresh_results(UNICODE_QUERY, window, cx);
        search.key_down(&key("ctrl-f", false), window, cx);
        let started = search.input.read(cx).text();
        assert_eq!(
            started,
            "文本: 搜索行 \"\" \"北区: OK\u{2028}南区: OK\u{2029}北站待定\""
        );
        search.input.update(cx, |input, cx| {
            input.replace_text_in_range(None, "南区", window, cx);
        });
        let filtered = search.input.read(cx).text();
        search.refresh_results(&filtered, window, cx);
        assert_eq!(search.results.refillable_text(), Some("南区: OK"));
        search.key_down(&key("ctrl-f", false), window, cx);
        assert_eq!(search.input.read(cx).text(), UNICODE_QUERY);
    }

    #[cfg(target_os = "windows")]
    fn verify_example_data_selection(
        search: &mut QuickSearch,
        window: &mut Window,
        cx: &mut Context<QuickSearch>,
    ) {
        let shortcut = KeyDownEvent {
            keystroke: gpui::Keystroke::parse("alt-enter").unwrap(),
            is_held: false,
        };
        for (command, prefix, replacement, expected) in [
            (
                "CSV: 搜索行",
                "CSV: 搜索行 {\"词\":[\"茶\"],\"列\":[\"备注\"]} ",
                "区域,备注,货品\r\n西区,茶叶,纸\r\n北区,水,茶",
                "区域,备注,货品\r\n西区,茶叶,纸",
            ),
            (
                "JSON: 路径转CSV",
                "JSON: 路径转CSV [\"/订单/商品\",\"/订单/金额\"] ",
                r#"[{"订单":{"商品":"甲,乙","金额":0.50}}]"#,
                "/订单/商品,/订单/金额\r\n\"甲,乙\",0.50",
            ),
            (
                "文本: 定宽转CSV",
                "文本: 定宽转CSV [3,2]\n",
                "009南京已核对\r\n012",
                "列1,列2,余下\r\n009,南京,已核对\r\n012,\"\",\"\"",
            ),
        ] {
            search.fill_query(command, window, cx);
            search.refresh_results(command, window, cx);
            let Results::Help(examples) = &search.results else {
                panic!("应展示 {command} 示例");
            };
            search.selected = examples
                .iter()
                .position(|&index| EXAMPLES[index].query.starts_with(prefix))
                .expect("应提供可替换数据的示例");
            search.key_down(&shortcut, window, cx);
            let input = search.input.read(cx);
            let example = input.text();
            assert!(example.starts_with(prefix));
            assert_eq!(input.selection(), prefix.len()..example.len());
            search.input.update(cx, |input, cx| {
                input.replace_text_in_range(None, replacement, window, cx);
            });
            let query = search.input.read(cx).text();
            assert_eq!(query, format!("{prefix}{replacement}"));
            search.refresh_results(&query, window, cx);
            assert_eq!(search.results.refillable_text(), Some(expected));
        }
        for (command, prefix, sample, suffix, replacement, expected) in [
            (
                "分数: 计算 root(",
                "分数: 计算 root(",
                "-125/343,3",
                ")",
                "1/8,3",
                "1/2",
            ),
            (
                "? 分数: 计算 2**80",
                "分数: 计算 ",
                "2**80",
                "",
                "2**64+1",
                "18446744073709551617",
            ),
        ] {
            search.fill_query(command, window, cx);
            search.refresh_results(command, window, cx);
            search.key_down(
                &KeyDownEvent {
                    keystroke: gpui::Keystroke::parse("shift-tab").unwrap(),
                    is_held: false,
                },
                window,
                cx,
            );
            let input = search.input.read(cx);
            assert_eq!(input.text(), format!("{prefix}{sample}{suffix}"));
            assert_eq!(input.selection(), prefix.len()..prefix.len() + sample.len());
            search.input.update(cx, |input, cx| {
                input.replace_text_in_range(None, replacement, window, cx);
            });
            let query = search.input.read(cx).text();
            assert_eq!(query, format!("{prefix}{replacement}{suffix}"));
            search.refresh_results(&query, window, cx);
            assert_eq!(search.results.refillable_text(), Some(expected));
        }
    }

    #[cfg(target_os = "windows")]
    fn verify_result_refill(
        search: &mut QuickSearch,
        window: &mut Window,
        cx: &mut Context<QuickSearch>,
    ) {
        let original = "文本: 大写 hello\nworld";
        let result = "HELLO\nWORLD";
        let application_refresh = search.application_refresh;
        let key = |is_held| KeyDownEvent {
            keystroke: gpui::Keystroke::parse("f3").unwrap(),
            is_held,
        };
        search.fill_query(original, window, cx);
        search.refresh_results(original, window, cx);
        search.key_down(&key(true), window, cx);
        assert_eq!(search.input.read(cx).text(), original);
        search.key_down(&key(false), window, cx);
        assert_eq!(search.input.read(cx).text(), result);
        search.refresh_results(result, window, cx);
        assert!(matches!(search.results, Results::RefilledInput));
        assert!(search.application_refresh == application_refresh);
        search.key_down(&key(true), window, cx);
        assert_eq!(search.input.read(cx).text(), result);
        search.key_down(&key(false), window, cx);
        assert_eq!(search.input.read(cx).text(), original);
        search.refresh_results(original, window, cx);
        search.key_down(&key(false), window, cx);
        search.input.update(cx, |input, cx| {
            input.replace_text_in_range(None, "文本: 小写 ", window, cx);
        });
        let next = search.input.read(cx).text();
        assert_eq!(next, "文本: 小写 HELLO\nWORLD");
        search.refresh_results(&next, window, cx);
        assert!(search.result_refill.is_none());
        assert_eq!(search.results.refillable_text(), Some("hello\nworld"));

        let original = "分数: 计算 1/3";
        let decimal = "0.333333333333";
        search.fill_query(original, window, cx);
        search.refresh_results(original, window, cx);
        let mut shortcut = KeyDownEvent {
            keystroke: gpui::Keystroke::parse("shift-f3").unwrap(),
            is_held: true,
        };
        search.key_down(&shortcut, window, cx);
        assert_eq!(search.input.read(cx).text(), original);
        shortcut.is_held = false;
        search.key_down(&shortcut, window, cx);
        assert_eq!(search.input.read(cx).text(), decimal);
        assert_eq!(search.input.read(cx).selection(), 0..0);
        search.refresh_results(decimal, window, cx);
        assert!(matches!(search.results, Results::RefilledInput));
        search.key_down(&key(false), window, cx);
        assert_eq!(search.input.read(cx).text(), original);

        let original = "进制: 浮点分解 0.375";
        let expression = "ldexp(0.75,-1)";
        search.fill_query(original, window, cx);
        search.refresh_results(original, window, cx);
        search.key_down(&key(false), window, cx);
        assert_eq!(search.input.read(cx).text(), expression);
        search.refresh_results(expression, window, cx);
        search
            .input
            .update(cx, |input, cx| input.move_cursor_to(6, cx));
        let mut refresh = KeyDownEvent {
            keystroke: gpui::Keystroke::parse("f5").unwrap(),
            is_held: true,
        };
        search.key_down(&refresh, window, cx);
        assert!(matches!(search.results, Results::RefilledInput));
        refresh.is_held = false;
        search.key_down(&refresh, window, cx);
        assert_eq!(search.results.refillable_text(), Some("0.375"));
        assert_eq!(search.input.read(cx).selection(), 6..6);
        search.refresh_results(expression, window, cx);
        assert_eq!(search.results.refillable_text(), Some("0.375"));
        search.key_down(&key(false), window, cx);
        assert_eq!(search.input.read(cx).text(), original);
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
                crate::message_input::init(cx);
                let index = SearchIndex::default();
                index.recent.last_input.borrow_mut().query = "2**10".into();
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
                            ("f6", true),
                            ("f6", false),
                            ("f6", true),
                            ("f6", false),
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
                            let mut bookmark = KeyDownEvent {
                                keystroke: gpui::Keystroke::parse("ctrl-d").unwrap(),
                                is_held: false,
                            };
                            search.key_down(&bookmark, window, cx);
                            bookmark.is_held = true;
                            search.key_down(&bookmark, window, cx);
                        })
                        .unwrap();
                    window
                        .update(cx, |search, _, cx| {
                            search.input.update(cx, |input, cx| input.select_range(1..3, cx));
                        })
                        .unwrap();
                    for (key, filter) in [
                        ("f4", Some("收藏: ")),
                        ("f1", Some("? 收藏:")),
                        ("escape", None),
                    ] {
                        window
                            .update(cx, |search, window, cx| {
                                let event = KeyDownEvent {
                                    keystroke: gpui::Keystroke::parse(key).unwrap(),
                                    is_held: false,
                                };
                                search.key_down(&event, window, cx);
                                if let Some(filter) = filter {
                                    assert_eq!(search.input.read(cx).text(), filter);
                                }
                            })
                            .unwrap();
                    }
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
                            assert_eq!(search.input.read(cx).selection(), 1..3);
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
                    reopened
                        .update(cx, |search, window, cx| {
                            let prefix = "CSV: 截取列";
                            search.fill_query(prefix, window, cx);
                            search.refresh_results(prefix, window, cx);
                            let mut invoke = |is_held| {
                                search.key_down(
                                    &KeyDownEvent {
                                        keystroke: gpui::Keystroke::parse("alt-enter").unwrap(),
                                        is_held,
                                    },
                                    window,
                                    cx,
                                );
                                search.input.read(cx).text()
                            };
                            assert_eq!(invoke(true), prefix);
                            assert!(invoke(false).starts_with(
                                "CSV: 截取列 [\"摘要\",\"文本\",\"2-3\"]\n编号,文本"
                            ));
                            let replacement = "编号,文本\n001,北方茶";
                            search.input.update(cx, |input, cx| {
                                input.replace_text_in_range(None, replacement, window, cx);
                            });
                            assert_eq!(
                                search.input.read(cx).text(),
                                "CSV: 截取列 [\"摘要\",\"文本\",\"2-3\"]\n编号,文本\n001,北方茶"
                            );
                        })
                        .unwrap();
                    let replace_example_arguments = reopened
                        .update(cx, |search, window, cx| {
                            let prefix = "CSV: 截取列";
                            search.fill_query(prefix, window, cx);
                            search.refresh_results(prefix, window, cx);
                            let mut invoke = |is_held| {
                                search.key_down(
                                    &KeyDownEvent {
                                        keystroke: gpui::Keystroke::parse("shift-tab").unwrap(),
                                        is_held,
                                    },
                                    window,
                                    cx,
                                );
                                search.input.read(cx).text()
                            };
                            let held = invoke(true);
                            let filled = invoke(false);
                            let replacement = "[\"示例\",1,\"1\"]\n标题\n茶";
                            search.input.update(cx, |input, cx| {
                                input.replace_text_in_range(None, replacement, window, cx);
                            });
                            let edited = search.input.read(cx).text();
                            [held, filled, edited]
                        })
                        .unwrap();
                    reopened
                        .update(cx, |search, window, cx| {
                            for (prefix, replacement, expected) in [
                                ("路径: 主名", "C:\\工作\\茶.txt", "路径: 主名 C:\\工作\\茶.txt"),
                                ("UUID: v5", "URL https://example.invalid/a", "UUID: v5 URL https://example.invalid/a"),
                                ("ULID: 格式", "01ARZ3NDEKTSV4RRFFQ69G5FAW", "ULID: 格式 01ARZ3NDEKTSV4RRFFQ69G5FAW"),
                            ] {
                                search.fill_query(prefix, window, cx);
                                search.refresh_results(prefix, window, cx);
                                search.key_down(
                                    &KeyDownEvent {
                                        keystroke: gpui::Keystroke::parse("shift-tab").unwrap(),
                                        is_held: false,
                                    },
                                    window,
                                    cx,
                                );
                                search.input.update(cx, |input, cx| {
                                    input.replace_text_in_range(None, replacement, window, cx);
                                });
                                assert_eq!(search.input.read(cx).text(), expected);
                            }
                        })
                        .unwrap();
                    reopened
                        .update(cx, |search, window, cx| {
                            for (prefix, replacement, expected) in [
                                ("日期: 星期统计", "2024-01-01 至 2024-01-07", "日期: 星期统计 2024-01-01 至 2024-01-07"),
                                ("时长: 分摊", "03:00:00 @ 1:2", "时长: 分摊 03:00:00 @ 1:2"),
                                ("进制: 罗马数字", "3999", "进制: 罗马数字 3999"),
                            ] {
                                search.fill_query(prefix, window, cx);
                                search.refresh_results(prefix, window, cx);
                                search.key_down(
                                    &KeyDownEvent {
                                        keystroke: gpui::Keystroke::parse("shift-tab").unwrap(),
                                        is_held: false,
                                    },
                                    window,
                                    cx,
                                );
                                search.input.update(cx, |input, cx| {
                                    input.replace_text_in_range(None, replacement, window, cx);
                                });
                                assert_eq!(search.input.read(cx).text(), expected);
                            }
                        })
                        .unwrap();
                    reopened
                        .update(cx, |search, window, cx| {
                            for (prefix, replacement, expected) in [
                                ("音高: midi", "81", "音高: MIDI 81"),
                                ("分贝: 功率比", "10000", "分贝: 功率比 10000"),
                                ("节拍: 速度", "1秒 @ 2", "节拍: 速度 1秒 @ 2"),
                                ("立体: 圆柱", "1/2,3", "立体: 圆柱 1/2,3"),
                                ("电路:", "6 V,3 Ω", "电路: 6 V,3 Ω"),
                                ("电路: 负载", "9 V @ 1 kΩ,1 kΩ,1 kΩ", "电路: 负载分压 9 V @ 1 kΩ,1 kΩ,1 kΩ"),
                                ("方程: 切", "0,1,0 @ 2", "方程: 切线 0,1,0 @ 2"),
                                ("方程:", "x² = 4", "方程: x² = 4"),
                                ("联立: 通", "x+y=3; 2x+2y=6", "联立: 通解 x+y=3; 2x+2y=6"),
                                ("圆: 直线交点", "0,0 @ 2; 0,0; 1,0", "圆: 直线交点 0,0 @ 2; 0,0; 1,0"),
                                ("复数: 极坐标", "1-i", "复数: 极坐标 1-i"),
                                ("向量: 直线距离", "1,1 @ 0,0; 1,0", "向量: 直线距离 1,1 @ 0,0; 1,0"),
                                ("矩阵: 乘方", "1,0; 0,1 @ 4", "矩阵: 乘方 1,0; 0,1 @ 4"),
                            ] {
                                search.fill_query(prefix, window, cx);
                                search.refresh_results(prefix, window, cx);
                                search.key_down(
                                    &KeyDownEvent {
                                        keystroke: gpui::Keystroke::parse("shift-tab").unwrap(),
                                        is_held: false,
                                    },
                                    window,
                                    cx,
                                );
                                search.input.update(cx, |input, cx| {
                                    input.replace_text_in_range(None, replacement, window, cx);
                                });
                                assert_eq!(search.input.read(cx).text(), expected);
                            }
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
                    let scoped_help = reopened
                        .update(cx, |search, window, cx| {
                            const QUERY: &str = "ＪＳＯＮ：取值 /城市 {\"城市\":\"北京\"}";
                            search.fill_query(QUERY, window, cx);
                            let event = KeyDownEvent {
                                keystroke: gpui::Keystroke::parse("f1").unwrap(),
                                is_held: false,
                            };
                            search.key_down(&event, window, cx);
                            let help = search.input.read(cx).text();
                            search.refresh_results(&help, window, cx);
                            let examples_match = matches!(&search.results, Results::Help(items)
                                if items.first().is_some_and(|&index| EXAMPLES[index].query.starts_with("JSON: 取值 "))
                                    && items.iter().all(|&index| EXAMPLES[index].query.starts_with("JSON:")));
                            search.key_down(&event, window, cx);
                            (help, examples_match, search.input.read(cx).text())
                        })
                        .unwrap();
                    let directory_help = reopened
                        .update(cx, |search, window, cx| {
                            const DIRECTORY: &str = "工具:";
                            search.fill_query(DIRECTORY, window, cx);
                            search.refresh_results(DIRECTORY, window, cx);
                            let select_duration = match &search.results {
                                Results::Help(items) => items.iter().position(|&index| {
                                    EXAMPLES[index].query.starts_with("时长:")
                                }).unwrap(),
                                _ => unreachable!(),
                            };
                            search.selected = select_duration;
                            let event = KeyDownEvent {
                                keystroke: gpui::Keystroke::parse("f1").unwrap(),
                                is_held: false,
                            };
                            search.key_down(&event, window, cx);
                            let help = search.input.read(cx).text();
                            search.refresh_results(&help, window, cx);
                            let examples_match = matches!(&search.results, Results::Help(items)
                                if items.len() > 1
                                    && items.iter().all(|&index| EXAMPLES[index].query.starts_with("时长:")));
                            search.key_down(&event, window, cx);
                            (select_duration, help, examples_match, search.input.read(cx).text())
                        })
                        .unwrap();
                    let bookmark_states = reopened
                        .update(cx, |search, window, cx| {
                            search.fill_query("", window, cx);
                            search.refresh_results("", window, cx);
                            let bookmark = KeyDownEvent {
                                keystroke: gpui::Keystroke::parse("ctrl-d").unwrap(),
                                is_held: false,
                            };
                            search.key_down(&bookmark, window, cx);
                            let menu = search.input.read(cx).text();
                            search.refresh_results(&menu, window, cx);
                            let key = |name| KeyDownEvent {
                                keystroke: gpui::Keystroke::parse(name).unwrap(),
                                is_held: false,
                            };
                            search.key_down(&key("f2"), window, cx);
                            let name_entry = matches!(search.results, Results::BookmarkNameInput(None));
                            search.fill_query("这个名字将取消", window, cx);
                            search.key_down(&key("escape"), window, cx);
                            let canceled = search.input.read(cx).text() == menu
                                && search.index.recent.bookmarks.matching("这个名字将取消").is_empty();
                            search.key_down(&key("f2"), window, cx);
                            search.fill_query("每周预算", window, cx);
                            search.key_down(&key("enter"), window, cx);
                            let saved_to_list = search.input.read(cx).text() == menu;
                            search.fill_query("收藏: 每周", window, cx);
                            search.refresh_results("收藏: 每周", window, cx);
                            let named = saved_to_list && matches!(&search.results, Results::Bookmarks(items)
                                if items.len() == 1 && items[0].result == "每周预算");
                            search.key_down(
                                &KeyDownEvent {
                                    keystroke: gpui::Keystroke::parse("tab").unwrap(),
                                    is_held: false,
                                },
                                window,
                                cx,
                            );
                            let restored = search.input.read(cx).text();
                            search.fill_query("收藏: 3**4", window, cx);
                            search.refresh_results("收藏: 3**4", window, cx);
                            search.key_down(&bookmark, window, cx);
                            let removed = matches!(&search.results, Results::Bookmarks(items) if items.is_empty());
                            let historical = "文本: 原文   甲\r\n乙 ";
                            search.index.recent.queries.record_exact(historical, "");
                            search.fill_query("查询历史:", window, cx);
                            search.refresh_results("查询历史:", window, cx);
                            search.key_down(&bookmark, window, cx);
                            assert!(search.index.recent.bookmarks.contains(historical));
                            assert!(!search.index.recent.bookmarks.contains("查询历史:"));
                            search.key_down(&bookmark, window, cx);
                            assert!(!search.index.recent.bookmarks.contains(historical));
                            for query in ["11 + 11", "2**3", "11 + 22"] {
                                search.index.recent.bookmarks.toggle(query).unwrap();
                            }
                            let filter = "收藏: １１";
                            search.fill_query(filter, window, cx);
                            search.refresh_results(filter, window, cx);
                            search.selected = 1;
                            search.key_down(&key("alt-up"), window, cx);
                            let moved_up = search.selected == 0 && matches!(&search.results, Results::Bookmarks(items)
                                if items.len() == 2 && items[0].expression == "11 + 11");
                            search.key_down(&key("alt-down"), window, cx);
                            let moved_down = search.selected == 1 && matches!(&search.results, Results::Bookmarks(items)
                                if items.len() == 2 && items[1].expression == "11 + 11");
                            (menu, restored, removed, name_entry, canceled, named, moved_up && moved_down)
                        })
                        .unwrap();
                    let bookmark_browser = reopened
                        .update(cx, |search, window, cx| {
                            let key = |name, is_held| KeyDownEvent {
                                keystroke: gpui::Keystroke::parse(name).unwrap(),
                                is_held,
                            };
                            let original = " 2**10 ";
                            search.fill_query(original, window, cx);
                            search.input.update(cx, |input, cx| input.move_cursor_to(2, cx));
                            search.refresh_results(original, window, cx);
                            search.key_down(&key("f4", true), window, cx);
                            let after_repeat = search.input.read(cx).text();
                            search.key_down(&key("f4", false), window, cx);
                            let opened = search.input.read(cx).text();
                            let filter = "收藏: 每周";
                            search.fill_query(filter, window, cx);
                            search.input.update(cx, |input, cx| {
                                input.select_range("收藏: ".len()..filter.len(), cx);
                            });
                            search.refresh_results(filter, window, cx);
                            search.key_down(&key("f1", false), window, cx);
                            let help = search.input.read(cx).text();
                            search.refresh_results(&help, window, cx);
                            search.key_down(&key("f1", false), window, cx);
                            let restored_filter = search.input.read(cx).text();
                            assert_eq!(search.input.read(cx).selection(), "收藏: ".len()..filter.len());
                            search.refresh_results(&restored_filter, window, cx);
                            search.key_down(&key("f1", false), window, cx);
                            let reopened_help = search.input.read(cx).text();
                            assert!(reopened_help.starts_with('?'));
                            search.refresh_results(&reopened_help, window, cx);
                            search.key_down(&key("f4", false), window, cx);
                            assert_eq!(search.input.read(cx).text(), filter);
                            assert_eq!(search.input.read(cx).selection(), "收藏: ".len()..filter.len());
                            search.refresh_results(filter, window, cx);
                            search.key_down(&key("f4", false), window, cx);
                            let restored_query = search.input.read(cx).text();
                            assert_eq!(search.input.read(cx).selection(), 2..2);
                            search.fill_query(filter, window, cx);
                            search.refresh_results(filter, window, cx);
                            search.key_down(&key("f4", false), window, cx);
                            let manual_filter = search.input.read(cx).text();
                            search.fill_query("", window, cx);
                            search.refresh_results("", window, cx);
                            search.toggle_bookmark(window, cx);
                            assert_eq!(search.input.read(cx).text(), "收藏: ");
                            search.key_down(&key("f4", false), window, cx);
                            assert_eq!(search.input.read(cx).text(), "");
                            [after_repeat, opened, restored_filter, restored_query, manual_filter]
                        })
                        .unwrap();
                    let history_toggle = reopened
                        .update(cx, |search, window, cx| {
                            let key = |name| KeyDownEvent {
                                keystroke: gpui::Keystroke::parse(name).unwrap(),
                                is_held: false,
                            };
                            let history = key("ctrl-r");
                            search.fill_query(" 2**10 ", window, cx);
                            search.input.update(cx, |input, cx| input.select_range(2.." 2**10 ".len(), cx));
                            search.key_down(&history, window, cx);
                            let opened = search.input.read(cx).text();
                            let filter = "算式历史: １０２４";
                            search.fill_query(filter, window, cx);
                            search.refresh_results(filter, window, cx);
                            search.key_down(&key("f1"), window, cx);
                            let help = search.input.read(cx).text();
                            search.refresh_results(&help, window, cx);
                            search.key_down(&key("f1"), window, cx);
                            search.refresh_results(filter, window, cx);
                            search.key_down(&history, window, cx);
                            let restored = search.input.read(cx).text();
                            assert_eq!(search.input.read(cx).selection(), 2..restored.len());
                            search.fill_query("算式历史： 3**4", window, cx);
                            search.key_down(&history, window, cx);
                            let manual = search.input.read(cx).text();
                            let long = "1".repeat(INPUT_CHARACTER_LIMIT + 1);
                            search.fill_query(long.clone(), window, cx);
                            search.key_down(&history, window, cx);
                            let limited = search.input.read(cx).text();
                            search.refresh_results(&limited, window, cx);
                            let matched = matches!(search.results, Results::Calculations(_));
                            search.key_down(&history, window, cx);
                            (
                                opened,
                                restored,
                                manual,
                                limited.chars().count(),
                                matched,
                                search.input.read(cx).text() == long,
                            )
                        })
                        .unwrap();
                    reopened.update(cx, verify_result_refill).unwrap();
                    reopened
                        .update(cx, |search, window, cx| {
                            let filter = "查询历史: 移除验证";
                            let first = "文本: 原文 移除验证甲";
                            let second = "文本: 原文 移除验证乙";
                            search.index.recent.queries.record_exact(second, "");
                            search.index.recent.queries.record_exact(first, "");
                            search.fill_query(filter, window, cx);
                            search.refresh_results(filter, window, cx);
                            search.selected = 1;
                            let mut event = KeyDownEvent {
                                keystroke: gpui::Keystroke::parse("alt-delete").unwrap(),
                                is_held: true,
                            };
                            search.key_down(&event, window, cx);
                            assert_eq!(search.results.suggested_input(search.selected), Some(second));
                            event.is_held = false;
                            search.key_down(&event, window, cx);
                            assert_eq!(search.input.read(cx).text(), filter);
                            assert_eq!(search.selected, 0);
                            assert_eq!(search.results.suggested_input(0), Some(first));
                            assert_eq!(search.results.suggested_input(1), None);
                        })
                        .unwrap();
                    reopened
                        .update(cx, |search, window, cx| {
                            let draft = "2**10\n7*6 ";
                            for (first, intermediate, second, innermost) in [
                                ("f6", "查询历史: ", "f4", "收藏: "),
                                ("f4", "收藏: ", "f6", "查询历史: "),
                            ] {
                                for back in [first, second] {
                                    search.fill_query(draft, window, cx);
                                    search.input.update(cx, |input, cx| input.move_cursor_to(3, cx));
                                    search.refresh_results(draft, window, cx);
                                    let press = |search: &mut QuickSearch,
                                                 key: &str,
                                                 window: &mut Window,
                                                 cx: &mut Context<QuickSearch>| {
                                        let event = KeyDownEvent {
                                            keystroke: gpui::Keystroke::parse(key).unwrap(),
                                            is_held: false,
                                        };
                                        search.key_down(&event, window, cx);
                                        let query = search.input.read(cx).text();
                                        search.refresh_results(&query, window, cx);
                                        query
                                    };
                                    assert_eq!(press(search, first, window, cx), intermediate);
                                    let filter = format!("{intermediate}待测试");
                                    search.fill_query(&filter, window, cx);
                                    search.input.update(cx, |input, cx| {
                                        input.select_range(intermediate.len()..filter.len(), cx);
                                    });
                                    search.refresh_results(&filter, window, cx);
                                    assert_eq!(press(search, second, window, cx), innermost);
                                    assert_eq!(press(search, back, window, cx), filter);
                                    assert_eq!(
                                        search.input.read(cx).selection(),
                                        intermediate.len()..filter.len()
                                    );
                                    assert_eq!(press(search, first, window, cx), draft);
                                    assert_eq!(search.input.read(cx).selection(), 3..3);
                                }
                            }
                        })
                        .unwrap();
                    reopened.update(cx, verify_table_paging).unwrap();
                    reopened.update(cx, verify_result_copy_in_place).unwrap();
                    reopened.update(cx, verify_input_error_selection).unwrap();
                    reopened.update(cx, verify_go_to_input_line).unwrap();
                    reopened.update(cx, verify_input_find).unwrap();
                    reopened.update(cx, verify_input_replace).unwrap();
                    reopened.update(cx, verify_query_argument_selection).unwrap();
                    reopened.update(cx, verify_current_table_search).unwrap();
                    reopened.update(cx, verify_table_cell_lookup).unwrap();
                    reopened.update(cx, verify_expanded_result_restore).unwrap();
                    reopened.update(cx, verify_example_data_selection).unwrap();
                    reopened.update(cx, verify_text_result_search).unwrap();
                    reopened
                        .update(cx, |search, window, cx| {
                            search.fill_query("1 + 24 + 300", window, cx);
                        })
                        .unwrap();
                    let window_handle = gpui::AnyWindowHandle::from(reopened);
                    window_handle
                        .update(cx, |_, window, cx| window.draw(cx).clear())
                        .unwrap();
                    for (key, text, selection) in [
                        ("ctrl-left", "1 + 24 + 300", 9..9),
                        ("ctrl-shift-left", "1 + 24 + 300", 4..9),
                        ("ctrl-delete", "1 + 300", 4..4),
                        ("ctrl-left", "1 + 300", 0..0),
                        ("ctrl-shift-right", "1 + 300", 0..4),
                        ("ctrl-backspace", "300", 0..0),
                        ("ctrl-right", "300", 3..3),
                        ("ctrl-backspace", "", 0..0),
                        ("ctrl-z", "300", 3..3),
                        ("ctrl-z", "1 + 300", 0..4),
                        ("ctrl-y", "300", 3..3),
                        ("ctrl-shift-z", "", 0..0),
                        ("ctrl-z", "300", 3..3),
                        ("home", "300", 0..0),
                        ("delete", "00", 0..0),
                        ("ctrl-y", "00", 0..0),
                    ] {
                        window_handle
                            .update(cx, |_, window, cx| {
                                window.dispatch_keystroke(gpui::Keystroke::parse(key).unwrap(), cx);
                            })
                            .unwrap();
                        reopened
                            .update(cx, |search, _, cx| {
                                let input = search.input.read(cx);
                                assert_eq!(input.text(), text, "{key}");
                                assert_eq!(input.selection(), selection, "{key}");
                            })
                            .unwrap();
                    }
                    reopened
                        .update(cx, |search, window, cx| search.fill_query("sqrt(81+19)", window, cx))
                        .unwrap();
                    window_handle
                        .update(cx, |_, window, cx| {
                            window.dispatch_keystroke(gpui::Keystroke::parse("ctrl-shift-a").unwrap(), cx);
                        })
                        .unwrap();
                    reopened
                        .update(cx, |search, _, cx| {
                            let input = search.input.read(cx);
                            assert_eq!(&input.text_ref()[input.selection()], "81+19");
                        })
                        .unwrap();
                    let query = "分数: 计算 lerp(root(1/8,3), 4/5, 0.5)";
                    reopened
                        .update(cx, |search, window, cx| search.fill_query(query, window, cx))
                        .unwrap();
                    for (key, expected) in [
                        ("alt-shift-right", "root(1/8,3)"),
                        ("alt-shift-right", "4/5"),
                        ("alt-shift-left", "root(1/8,3)"),
                        ("alt-shift-left", "0.5"),
                    ] {
                        window_handle
                            .update(cx, |_, window, cx| {
                                window.dispatch_keystroke(gpui::Keystroke::parse(key).unwrap(), cx);
                            })
                            .unwrap();
                        reopened
                            .update(cx, |search, _, cx| {
                                let input = search.input.read(cx);
                                assert_eq!(&input.text_ref()[input.selection()], expected, "{key}");
                                assert_eq!(input.text_ref(), query);
                            })
                            .unwrap();
                    }
                    let multiline = "CSV: 表格预览 名称,数量\r\n绿茶,1\n米,2";
                    let line_start = multiline.find("绿茶").unwrap();
                    let line_end = line_start + "绿茶,1".len();
                    let last_line_start = multiline.find("米").unwrap();
                    let revision = reopened
                        .update(cx, |search, window, cx| {
                            search.fill_query(multiline, window, cx);
                            search.input.update(cx, |input, cx| {
                                input.move_cursor_to(line_start + "绿茶".len(), cx);
                                input.edit_revision()
                            })
                        })
                        .unwrap();
                    for (key, expected) in [
                        ("alt-home", line_start..line_start),
                        ("alt-shift-end", line_start..line_end),
                        ("alt-end", line_end..line_end),
                        ("alt-shift-home", line_start..line_end),
                        ("alt-end", line_end..line_end),
                        ("ctrl-l", line_start..line_end),
                        ("home", 0..0),
                        ("end", multiline.len()..multiline.len()),
                        ("ctrl-up", last_line_start..last_line_start),
                        ("ctrl-up", line_start..line_start),
                        ("ctrl-shift-down", line_start..last_line_start),
                        ("ctrl-up", line_start..line_start),
                        ("ctrl-shift-up", 0..line_start),
                        ("ctrl-down", line_start..line_start),
                        ("ctrl-down", last_line_start..last_line_start),
                        ("ctrl-down", multiline.len()..multiline.len()),
                    ] {
                        window_handle
                            .update(cx, |_, window, cx| {
                                window.dispatch_keystroke(gpui::Keystroke::parse(key).unwrap(), cx);
                            })
                            .unwrap();
                        reopened
                            .update(cx, |search, _, cx| {
                                let input = search.input.read(cx);
                                assert_eq!(input.selection(), expected, "{key}");
                                assert_eq!(input.text(), multiline);
                                assert_eq!(input.edit_revision(), revision);
                            })
                            .unwrap();
                    }
                    let after_delete = &multiline[..line_end];
                    for (key, text, selection) in [
                        ("ctrl-shift-k", after_delete, line_end..line_end),
                        ("ctrl-z", multiline, multiline.len()..multiline.len()),
                        ("ctrl-y", after_delete, line_end..line_end),
                    ] {
                        window_handle
                            .update(cx, |_, window, cx| {
                                window.dispatch_keystroke(gpui::Keystroke::parse(key).unwrap(), cx);
                            })
                            .unwrap();
                        reopened
                            .update(cx, |search, _, cx| {
                                let input = search.input.read(cx);
                                assert_eq!(input.text(), text, "{key}");
                                assert_eq!(input.selection(), selection, "{key}");
                            })
                            .unwrap();
                    }
                    let original = "文本: 计数 甲🍵\r\n乙";
                    let insert_at = original.find('\r').unwrap();
                    let caret = insert_at - "🍵".len();
                    let duplicate = "文本: 计数 甲🍵\r\n文本: 计数 甲🍵\r\n乙";
                    let duplicate_caret = insert_at + "\r\n".len() + caret;
                    reopened
                        .update(cx, |search, window, cx| {
                            search.fill_query(original, window, cx);
                            search.input.update(cx, |input, cx| input.select_range(caret..caret, cx));
                            search.key_down(
                                &KeyDownEvent {
                                    keystroke: gpui::Keystroke::parse("ctrl-shift-d").unwrap(),
                                    is_held: true,
                                },
                                window,
                                cx,
                            );
                            assert_eq!(search.input.read(cx).text(), original);
                        })
                        .unwrap();
                    for (key, text, caret) in [
                        ("ctrl-shift-d", duplicate, duplicate_caret),
                        ("ctrl-z", original, caret),
                        ("ctrl-y", duplicate, duplicate_caret),
                    ] {
                        window_handle
                            .update(cx, |_, window, cx| {
                                window.dispatch_keystroke(gpui::Keystroke::parse(key).unwrap(), cx);
                            })
                            .unwrap();
                        reopened
                            .update(cx, |search, _, cx| {
                                let input = search.input.read(cx);
                                assert_eq!(input.text(), text, "{key}");
                                assert_eq!(input.selection(), caret..caret, "{key}");
                            })
                            .unwrap();
                    }
                    let last_line = "文本: 计数 甲\u{2028}🍵";
                    let full_input = "甲".repeat(INPUT_CHARACTER_LIMIT);
                    for (original, expected) in [
                        (last_line, "文本: 计数 甲\u{2028}🍵\u{2028}🍵"),
                        ("甲\r\n", "甲\r\n\r\n"),
                        ("甲", "甲\n甲"),
                        (full_input.as_str(), full_input.as_str()),
                    ] {
                        reopened
                            .update(cx, |search, window, cx| {
                                search.fill_query(original.to_owned(), window, cx)
                            })
                            .unwrap();
                        window_handle
                            .update(cx, |_, window, cx| {
                                window.dispatch_keystroke(gpui::Keystroke::parse("ctrl-shift-d").unwrap(), cx);
                            })
                            .unwrap();
                        reopened
                            .update(cx, |search, _, cx| {
                                let input = search.input.read(cx);
                                assert_eq!(input.text(), expected);
                                assert_eq!(input.selection(), expected.len()..expected.len());
                            })
                            .unwrap();
                    }
                    let ime_prefix = "文本: 大写 ";
                    reopened
                        .update(cx, |search, window, cx| {
                            search.fill_query(ime_prefix, window, cx);
                            search.input.update(cx, |input, cx| {
                                input.replace_and_mark_text_in_range(None, "n", Some(1..1), window, cx);
                                input.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx);
                                input.replace_text_in_range(None, "你", window, cx);
                            });
                        })
                        .unwrap();
                    for (key, text, selection) in [
                        ("ctrl-z", ime_prefix, ime_prefix.len()..ime_prefix.len()),
                        ("ctrl-y", "文本: 大写 你", "文本: 大写 你".len().."文本: 大写 你".len()),
                    ] {
                        window_handle
                            .update(cx, |_, window, cx| {
                                window.dispatch_keystroke(gpui::Keystroke::parse(key).unwrap(), cx);
                            })
                            .unwrap();
                        reopened
                            .update(cx, |search, _, cx| {
                                let input = search.input.read(cx);
                                assert_eq!(input.text(), text, "{key}");
                                assert_eq!(input.selection(), selection, "{key}");
                            })
                            .unwrap();
                    }
                    let query = "  =1/3 ";
                    let selection = 3..6;
                    reopened
                        .update(cx, |search, window, cx| {
                            search.fill_query(query, window, cx);
                            search.input.update(cx, |input, cx| input.select_range(selection.clone(), cx));
                        })
                        .unwrap();
                    let decimal = calculate(query).unwrap();
                    let exact_query = format!("{EXACT_MODE_PREFIX}{query}");
                    let exact_selection = (EXACT_MODE_PREFIX.len() + selection.start)
                        ..(EXACT_MODE_PREFIX.len() + selection.end);
                    for (key, text, range, result) in [
                        ("alt-f2", exact_query.as_str(), exact_selection.clone(), "1/3"),
                        ("alt-f2", query, selection, decimal.as_str()),
                        ("ctrl-z", exact_query.as_str(), exact_selection, "1/3"),
                    ] {
                        window_handle
                            .update(cx, |_, window, cx| {
                                window.dispatch_keystroke(gpui::Keystroke::parse(key).unwrap(), cx);
                            })
                            .unwrap();
                        reopened
                            .update(cx, |search, _, cx| {
                                let input = search.input.read(cx);
                                assert_eq!(input.text(), text, "{key}");
                                assert_eq!(input.selection(), range, "{key}");
                                assert_eq!(search.results.refillable_text(), Some(result), "{key}");
                            })
                            .unwrap();
                    }
                    let query = "分数: 计算 ans+1";
                    reopened
                        .update(cx, |search, window, cx| {
                            search.index.recent.calculations.record("4+5", "9");
                            search.fill_query(query, window, cx);
                        })
                        .unwrap();
                    reopened
                        .update(cx, |search, _, cx| {
                            assert_eq!(search.results.refillable_text(), Some("10"));
                            search.index.recent.calculations.record("8+9", "17");
                            search.input.update(cx, |input, cx| input.move_cursor_to(0, cx));
                        })
                        .unwrap();
                    reopened
                        .update(cx, |search, window, cx| {
                            assert_eq!(search.input.read(cx).selection(), 0..0);
                            assert_eq!(search.results.refillable_text(), Some("10"));
                            search.result_save = ResultSaveState::Finished {
                                query: query.to_owned(),
                                result: Ok(()),
                            };
                            let mut refresh = KeyDownEvent {
                                keystroke: gpui::Keystroke::parse("f5").unwrap(),
                                is_held: true,
                            };
                            search.key_down(&refresh, window, cx);
                            assert_eq!(search.results.refillable_text(), Some("10"));
                            assert!(matches!(search.result_save, ResultSaveState::Finished { .. }));
                            refresh.is_held = false;
                            search.key_down(&refresh, window, cx);
                            assert_eq!(search.input.read(cx).selection(), 0..0);
                            assert_eq!(search.results.refillable_text(), Some("18"));
                            assert!(matches!(search.result_save, ResultSaveState::Ready));
                            search.index.recent.calculations.record("99+1", "100");
                            search.fill_query(query, window, cx);
                        })
                        .unwrap();
                    reopened
                        .update(cx, |search, _, _| {
                            assert_eq!(search.results.refillable_text(), Some("101"));
                        })
                        .unwrap();
                    reopened
                        .update(cx, |search, window, cx| {
                            let index = EXAMPLES.iter()
                                .position(|example| example.query.starts_with("JSON: 取值 "))
                                .expect("应提供 JSON 取值示例");
                            let query = EXAMPLES[index].query;
                            search.index.recent.examples.record(index);
                            search.index.recent.queries.record_exact(query, "");
                            assert!(search.index.recent.bookmarks.toggle(query).unwrap());
                            for filter in ["用法历史: 取值", "查询历史: 取值", "收藏: 取值"] {
                                search.fill_query(filter, window, cx);
                                search.refresh_results(filter, window, cx);
                                assert_eq!(search.results.suggested_input(search.selected), Some(query));
                                search.toggle_help(window, cx);
                                let help = search.input.read(cx).text();
                                assert_eq!(help, "? JSON: 取值");
                                search.refresh_results(&help, window, cx);
                                search.toggle_help(window, cx);
                                assert_eq!(search.input.read(cx).text(), filter);
                            }
                            let older = r#"JSON: 取值 /a {"a":1}"#;
                            let newer = r#"JSON: 取值 /b {"b":2}"#;
                            search.index.recent.queries.record_exact(older, "1");
                            search.index.recent.queries.record_exact(newer, "2");
                            let filter = "查询历史: 取值";
                            search.fill_query(filter, window, cx);
                            search.refresh_results(filter, window, cx);
                            search.selected = 1;
                            assert_eq!(search.results.suggested_input(1), Some(older));
                            search.toggle_help(window, cx);
                            let help = search.input.read(cx).text();
                            search.refresh_results(&help, window, cx);
                            search.toggle_help(window, cx);
                            search.refresh_results(filter, window, cx);
                            assert_eq!(search.results.suggested_input(search.selected), Some(older));
                        })
                        .unwrap();
                    reopened
                        .update(cx, |search, window, cx| {
                            let prefix = "处理:";
                            search.fill_query(prefix, window, cx);
                            search.refresh_results(prefix, window, cx);
                            search.key_down(
                                &KeyDownEvent {
                                    keystroke: gpui::Keystroke::parse("shift-tab").unwrap(),
                                    is_held: false,
                                },
                                window,
                                cx,
                            );
                            search.input.update(cx, |input, cx| {
                                input.replace_text_in_range(
                                    None,
                                    "编号,金额\n003,3\n004,4.50",
                                    window,
                                    cx,
                                );
                            });
                            let query = search.input.read(cx).text();
                            search.refresh_results(&query, window, cx);
                            assert_eq!(search.results.refillable_text(), Some("4.50"));
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
                        replace_example_arguments,
                        scoped_help,
                        directory_help,
                        bookmark_states,
                        bookmark_browser,
                        history_toggle,
                    ))
                    .unwrap();
                    cx.update(|cx| cx.quit()).unwrap();
                })
                .detach();
            });
        let (
            states,
            reopened_states,
            [help_pages, output_pages, calculation_pages, conversion_pages],
            window_refresh,
            app_refresh,
            file_navigation,
            file_help,
            [after_held, after_fill, after_edit],
            (help_input, matching_json_examples, restored_input),
            (directory_selection, directory_help, all_duration_examples, restored_directory),
            (
                bookmark_menu,
                bookmark_restored,
                bookmark_removed,
                name_entry,
                canceled,
                named,
                reordered,
            ),
            [after_repeat, opened, restored_filter, restored_query, manual_filter],
            (
                history_opened,
                history_restored,
                manual_history,
                history_filter_length,
                matched,
                long_restored,
            ),
        ) = receive.recv().unwrap();
        assert_eq!(history_opened, "算式历史:  2**10 ");
        assert_eq!(history_restored, " 2**10 ");
        assert_eq!(manual_history, "3**4");
        assert_eq!(history_filter_length, INPUT_CHARACTER_LIMIT);
        assert!(matched && long_restored);
        assert_eq!(after_repeat, " 2**10 ");
        assert_eq!(opened, "收藏: ");
        assert_eq!(restored_filter, "收藏: 每周");
        assert_eq!(restored_query, " 2**10 ");
        assert_eq!(manual_filter, "收藏: 每周");
        assert_eq!(after_held, "CSV: 截取列");
        assert!(after_fill.starts_with("CSV: 截取列 [\"摘要\",\"文本\",\"2-3\"]"));
        assert_eq!(after_edit, "CSV: 截取列 [\"示例\",1,\"1\"]\n标题\n茶");
        assert_eq!(help_input, "? JSON: 取值");
        assert!(matching_json_examples);
        assert!(directory_selection > 0);
        assert_eq!(directory_help, "? 时长:");
        assert!(all_duration_examples);
        assert_eq!(restored_directory, "工具:");
        assert_eq!(restored_input, "ＪＳＯＮ：取值 /城市 {\"城市\":\"北京\"}");
        assert_eq!(reopened_states, ["", "3**4"]);
        assert_eq!(bookmark_menu, "收藏: ");
        assert_eq!(bookmark_restored, "3**4");
        assert!(bookmark_removed);
        assert!(name_entry);
        assert!(canceled);
        assert!(named);
        assert!(reordered);
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
        assert_eq!(calculation_pages, [EXAMPLE_PAGE_SIZE, 0, 0]);
        assert_eq!(conversion_pages, [EXAMPLE_PAGE_SIZE, 0, 0]);
        assert_eq!(
            states.unwrap(),
            [
                "换行验证甲",
                "换行验证甲",
                "换行验证甲\n",
                "换行验证甲\n",
                "换行验证甲\n",
                "查询历史: ",
                "查询历史: ",
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
    ) -> [[usize; 3]; 4] {
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
        handle
            .update(cx, |search, _, _| {
                for number in 0..12 {
                    search
                        .index
                        .recent
                        .calculations
                        .record(&format!("{number}+1"), &(number + 1).to_string());
                    search.index.recent.conversions.record(
                        &format!("{number} km to m"),
                        &format!("{} m", number * 1000),
                    );
                }
            })
            .unwrap();
        let [calculations, conversions] = ["算式历史:", "换算历史:"].map(|query| {
            handle
                .update(cx, |search, window, cx| {
                    search.fill_query(query, window, cx)
                })
                .unwrap();
            handle.update(cx, visit_pages).unwrap()
        });
        [help, output, calculations, conversions]
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
    fn offers_templates_for_exact_calculator_functions_inside_fraction_queries() {
        for query in [
            "分数: 计算 roo",
            "分数: 计算 root(",
            "分数： ＣＡＬＣ ＲＯＯＴ（",
        ] {
            assert_eq!(
                for_query(query, &[]).suggested_input(0),
                Some("分数: 计算 root(-125/343,3)"),
                "{query}"
            );
        }
        assert!(matches!(
            for_query("分数: 计算 root(-125/343,3)", &[]),
            Results::Value(Ok(value)) if value.text == "-5/7"
        ));
        assert!(for_query("分数: 计算 roo(", &[])
            .suggested_input(0)
            .is_none());
        assert_eq!(
            for_query("root(", &[]).suggested_input(0),
            Some("root(-32,5)")
        );
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
    fn reads_bounded_utf8_files_without_changing_line_breaks_or_truncating() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("输入.csv");
        std::fs::write(&file, "\u{feff}甲,😀\r\n乙,3").unwrap();
        assert_eq!(read_input_file(&file, 8).as_deref(), Ok("甲,😀\r\n乙,3"));
        assert!(read_input_file(&file, 7).is_err());
        std::fs::write(&file, [0xffu8]).unwrap();
        assert!(read_input_file(&file, 8).is_err());
        std::fs::write(&file, b"a\0b").unwrap();
        assert!(read_input_file(&file, 8).is_err());
        assert!(read_input_file(dir.path(), 8).is_err());
    }

    #[test]
    fn loads_both_utf16_byte_orders_without_replacing_invalid_code_units() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("电子表格.tsv");
        let text = "甲\t😀\r\n乙\t3";
        for (bom, encode) in [
            ([0xff, 0xfe], u16::to_le_bytes as fn(u16) -> [u8; 2]),
            ([0xfe, 0xff], u16::to_be_bytes),
        ] {
            let bytes: Vec<_> = bom
                .into_iter()
                .chain(text.encode_utf16().flat_map(encode))
                .collect();
            std::fs::write(&file, bytes).unwrap();
            assert_eq!(read_input_file(&file, 8).as_deref(), Ok(text));
            assert!(read_input_file(&file, 7).unwrap_err().contains("7 字符"));
        }
        for invalid in [
            &[0xff, 0xfe, 0x00][..],
            &[0xff, 0xfe, 0x00, 0xd8][..],
            &[0xfe, 0xff, 0x00, 0x00][..],
        ] {
            std::fs::write(&file, invalid).unwrap();
            assert!(read_input_file(&file, 8).unwrap_err().contains("UTF-16"));
        }
    }

    #[test]
    fn selected_file_opens_in_its_tool_with_original_text_and_size_limit() {
        // 使用检出的本地路径，避免系统临时目录的配置影响句柄名称核验。
        let dir = tempfile::Builder::new()
            .prefix("qs-local-file-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap();
        let path = dir.path().join("记录.JSONL");
        let payload = "{\"编号\":\"001\",\"金额\":1.50}\r\n{\"编号\":\"002\",\"备注\":\"茶,咖啡\"}";
        std::fs::write(&path, payload).unwrap();
        let result = read_selected_text_query(path.to_str().unwrap()).unwrap();
        assert_eq!(result, format!("JSON: 合并行 {payload}"));
        assert_eq!(
            for_query(&result, &[]).refillable_text(),
            Some("[{\"编号\":\"001\",\"金额\":1.50},{\"编号\":\"002\",\"备注\":\"茶,咖啡\"}]")
        );
        std::fs::write(&path, "甲".repeat(INPUT_CHARACTER_LIMIT)).unwrap();
        assert!(read_selected_text_query(path.to_str().unwrap())
            .unwrap_err()
            .contains("字符"));
    }

    #[test]
    fn opening_input_files_in_empty_search_uses_a_tool_and_preserves_other_text() {
        for (filename, expected) in [
            ("销售.CSV", "CSV: 表格预览 "),
            ("名单.tsv", "CSV: 从TSV "),
            ("package.JSON", "JSON: 格式化 "),
            ("交易.JSONL", "JSON: 合并行 "),
            ("日志.ndjson", "JSON: 合并行 "),
            ("配置.toml", "TOML: 校验 "),
            ("草稿.txt", "文本: 原文 "),
            ("说明", "文本: 原文 "),
        ] {
            assert_eq!(input_file_query_prefix(Path::new(filename)), expected);
        }
        assert!(accepts_input_file(""));
        assert!(accepts_input_file(" \t\r\n"));
        let payload = "{\"编号\":\"001\",\"金额\":1.50}\r\n{\"编号\":\"002\",\"备注\":\"茶,咖啡\"}";
        let json_lines = format!(
            "{}{payload}",
            input_file_query_prefix(Path::new("数据.jsonl"))
        );
        assert_eq!(
            for_query(&json_lines, &[]).refillable_text(),
            Some("[{\"编号\":\"001\",\"金额\":1.50},{\"编号\":\"002\",\"备注\":\"茶,咖啡\"}]")
        );
        assert!(!accepts_input_file("2**10"));
        let text = "  开头\r\n中间\t末尾 \r\n";
        let query = format!("{}{text}", input_file_query_prefix(Path::new("说明.txt")));
        assert_eq!(for_query(&query, &[]).refillable_text(), Some(text));
        assert_eq!(
            for_query("text tools: RAW \t ", &[]).refillable_text(),
            Some("\t ")
        );
        let data = "编号,金额\r\n001,3.50";
        let query = format!("{}{data}", input_file_query_prefix(Path::new("销售.CSV")));
        let result = for_query(&query, &[]);
        assert!(
            matches!(&result, Results::Value(Ok(value)) if matches!(value.kind, ValueKind::Table(_)))
        );
        assert_eq!(result.refillable_text(), Some(data));
    }

    #[test]
    fn quick_search_help_shows_only_supported_examples() {
        for query in ["?", " ？ "] {
            let results = for_query(query, &[]);
            assert_eq!(
                results.height(MAX_VALUE_LINES),
                132. + EXAMPLE_PAGE_SIZE as f32 * EXAMPLE_ROW_HEIGHT + EXAMPLE_SHORTCUT_ROW_HEIGHT
            );
            assert!(matches!(results, Results::Help(items) if items.len() == EXAMPLES.len()));
        }
        let unix_examples = [
            "unix: now",
            "时间戳: 1700000000123ms",
            "时间戳: 间隔 -1ns 到 1ns to ns",
        ];
        for (query, expected) in [
            (
                "？ 速率",
                &[
                    "数据换算: 1 MiB/s to MB/s",
                    "下载: 600 MB 用时 2分钟",
                    "8Mbps to MB/s",
                ][..],
            ),
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
                    "时长: 精确换算 1秒 to min",
                    "时长: P1DT2.5S",
                    "时长: 1天2.5秒 to iso",
                    "时长: 23:45 到 01:15",
                    "时长: 扣除休息 23:45 到 01:15 | 20分钟 + 10分钟",
                    "时长: 比率 45分钟 @ 2小时",
                    "时长: 重叠 23:00 到 01:00 | 00:30 到 02:00",
                    "时长: 缩放 01:30:00 @ 2/3",
                    "时长: 分摊 01:30:00 @ 1:2",
                    "时长: 舍入 01:07:30 @ 15分钟",
                    "时长: 向下取整 01:07:30 @ 15分钟",
                    "时长: 向上取整 01:01:00 @ 15分钟",
                    "时长: 合计 01:25,02:40,00:55",
                    "时长: 平均 01:25,02:40,00:55",
                    "时长: 频数平均 01:00@3,02:00@1",
                    "时长: 频数中位数 01:00@2,02:00@2",
                    "时长: 中位数 01:25,02:40,00:55",
                    "时长: 分位数 75% 3s,1s,2s,4s",
                    "时长: 分位表 [\"25%\",0.5,\"3/4\"] 3s,1s,2s,4s",
                    "时长: 单位表 [\"ms\",\"s\",\"min\"] 1秒",
                    "时长: 最短 01:25,02:40,00:55",
                    "时长: 最长 01:25,02:40,00:55",
                    "时长: 极差 01:25,02:40,00:55",
                    "时长: 排序 02:40,PT2M,00:55,02:00",
                    "时长: 降序 02:40,PT2M,00:55,02:00",
                    "批量换算: 时长 01:30 to s\nPT1M30S to min\n1天2.5秒 to iso",
                    "批量换算: 时长对照 扣除休息 23:45 到 01:15 | 30分钟\n08:00 到 10:15 to min",
                    "批量换算: 精确时长 1秒 to min\nPT1M30S to h\n-0.25秒 to min",
                    "批量换算: 精确时长对照 1秒 to min\n23:45 到 01:15 to d\n1ns to s",
                r#"处理: ["节拍: 时长","时长:"] 120 BPM @ 3/2"#,
                    "节拍: 时长 120 BPM @ 3/2",
                    r#"时间戳: 覆盖时长 [["0s","10s"],["5s","15s"],["20s","26s"]]"#,
                    "时间戳: 逐日时长 2026-03-07T14:00:00-05:00 到 2026-03-09T10:00:00-04:00 to America/New_York",
                    "CSV: 时刻差列 [\"经过时长\",\"开始\",\"结束\"]\n开始,结束\n23:45,01:15\n09:00:00.25,09:00:01",
                    "CSV: 时间戳差列 [\"时长\",\"开始\",\"结束\"] 开始,结束\n2026-03-08T01:30:00-05:00,2026-03-08T03:30:00-04:00\n1ns,-1ns",
                    "CSV: 分组时长合计 [\"人员\"] \"时长\" 人员,时长\n北,01:30\n南,PT45S\n北,30.25秒\n空,\n南,-15秒\n负,-1ns",
                    "CSV: 时长数值列 [\"分钟\",\"时长\",\"min\"] 编号,时长\n001,1秒\n002,PT30S\n003,-0.25秒",
                    "帧数: 125 帧 @ 25 fps",
                    "帧数: 10秒 @ 30000/1001 fps",
                    "节拍: 120 BPM",
                    "节拍: 速度 1.5秒 @ 3",
                    "节拍: 拍数 90秒 @ 120 BPM",
                    "时间戳: 现在 + 90分钟 to UTC+8",
                    "时间戳: 向下对齐 15分钟 @ 1970-01-01T00:17:30Z",
                    "时间戳: 向上对齐 15分钟 @ 1970-01-01T00:17:30Z",
                    "CSV: 组内时间戳偏移列 [\"偏移\",\"时刻\",[\"设备\"]] 设备,时刻\nA,3s\nB,10ns\nA,8s\nA,\nB,5ns\nA,1s",
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
                Results::Value(Ok(_))
                | Results::Calculations(_)
                | Results::Conversions(_)
                | Results::Queries(_)
                | Results::Bookmarks(_) => {}
                Results::ApplicationsLoading
                | Results::OpenWindowsLoading
                | Results::LocalFilesPrompt => {}
                Results::ConversionTargets(items) if !items.is_empty() => {}
                Results::Destinations(items)
                    if !items.is_empty()
                        || matches!(
                            scoped_query(example.query),
                            Some((Scope::ApplicationHistory | Scope::RecentFilePreviews, ""))
                        ) => {}
                _ => panic!("用法示例没有可用结果：{}", example.query),
            }
        }
    }

    #[test]
    fn reuses_recent_file_previews_across_searches_and_filters_their_paths() {
        let index = SearchIndex::default();
        let recent = index.recent.file_previews.clone();
        for number in 1..=13 {
            recent.record(&format!(r"C:\Reports\Batch-{number}.csv"));
        }
        recent.record(r"C:\Reports\Batch-5.csv");
        let matches = index.results("最近预览: REPORTS");
        let Results::Destinations(items) = &matches else {
            panic!("最近预览应显示文件");
        };
        assert_eq!(items.len(), 12);
        assert_eq!(
            matches.selected_text_file(0),
            Some(r"C:\Reports\Batch-5.csv")
        );
        assert_eq!(
            matches.selected_text_file(1),
            Some(r"C:\Reports\Batch-13.csv")
        );
        assert!(matches
            .local_file_index(r"C:\Reports\Batch-1.csv")
            .is_none());
        assert_eq!(
            index
                .results("recent previews: Batch-8.csv")
                .selected_text_file(0),
            Some(r"C:\Reports\Batch-8.csv")
        );
        assert_eq!(
            SearchIndex::default()
                .results("最近预览:")
                .selectable_count(),
            0
        );
    }

    #[test]
    fn bookmarks_the_selected_example_or_history_entry() {
        let index = SearchIndex::default();
        let filter = "? 分数: 等差求和";
        let help = index.results(filter);
        let query = help.bookmark_query(filter, 0).expect("用法应包含所选示例");
        assert_eq!(query, "分数: 等差求和 1/3,1/6,5");
        assert_eq!(help.bookmark_query(filter, usize::MAX), None);
        assert!(index.recent.bookmarks.toggle(query).unwrap());
        assert_eq!(
            index.results("收藏: 等差求和").suggested_input(0),
            Some(query)
        );
        assert!(!index.recent.bookmarks.contains(filter));

        index.recent.calculations.record("2**10", "1024");
        index.recent.conversions.record("1 GiB to MiB", "1024 MiB");
        for (filter, query) in [
            ("算式历史: 1024", "2**10"),
            ("换算历史: 1024", "1 GiB to MiB"),
        ] {
            assert_eq!(index.results(filter).bookmark_query(filter, 0), Some(query));
        }
    }

    #[test]
    fn quick_search_preserves_bookmarked_queries_when_the_index_is_reopened() {
        let index = SearchIndex::default();
        let query = "CSV: 选择 1\n编号,金额\n001,20  ";
        assert!(index.recent.bookmarks.toggle(query).unwrap());
        let resumed = SearchIndex {
            recent: index.recent.clone(),
            ..SearchIndex::default()
        };
        let list = resumed.results("收藏：ＣＳＶ");
        assert!(matches!(&list, Results::Bookmarks(items) if items.len() == 1));
        assert_eq!(list.suggested_input(0), Some(query));
        assert_eq!(list.suggested_input(1), None);
        assert!(
            matches!(resumed.results("bookmarks"), Results::Bookmarks(items) if items.len() == 1)
        );
        resumed
            .recent
            .bookmarks
            .rename(query, "月结: 原始")
            .unwrap();
        for filter in ["@", "@ＣＳＶ", " ＠ 月结: 原始 ", "@月结： 原始"] {
            let list = resumed.results(filter);
            assert!(matches!(&list, Results::Bookmarks(items) if items.len() == 1));
            assert_eq!(list.suggested_input(0), Some(query), "{filter}");
        }
        assert!(matches!(resumed.results("@未知"), Results::Bookmarks(items) if items.is_empty()));
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
        for query in ["应用历史:", "recent apps: calculator"] {
            assert!(
                matches!(index.results(query), Results::Destinations(items) if items.is_empty())
            );
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
    fn searches_shortcut_help_by_keys_and_context() {
        let results = for_query("搜索快捷键: 表格 ctrl+shift", &[]);
        assert!(matches!(
            &results,
            Results::Value(Ok(ResultValue {
                kind: ValueKind::Table(_),
                ..
            }))
        ));
        let text = results.refillable_text().unwrap();
        assert!(text.contains("表格,Ctrl+Shift+PageUp,跳到第一组列"));
        assert!(text.contains("表格,Ctrl+Shift+PageDown,跳到最后一组列"));
        assert!(text.contains("Ctrl+Shift+C"));
        assert!(!text.contains("Ctrl+Shift+R"));
        let shortcut = for_query("SEARCH SHORTCUTS: Ａｌｔ＋Ｆ２", &[]);
        assert_eq!(
            shortcut.refillable_text(),
            Some("场景,按键,操作\r\n计算,Alt+F2,切换普通计算与精确分数计算")
        );
        assert!(matches!(
            for_query("快捷键", &[]),
            Results::Destinations(items) if items.contains(&Destination::Page(Page::Triggers))
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
    fn substitutes_confirmed_numeric_answers_in_calculator_queries_by_recency() {
        let index = SearchIndex::default();
        assert!(matches!(
            index.results("ans+1"),
            Results::Value(Err(ValueError::Calculation(
                CalculationError::MissingAnswer
            )))
        ));
        index.recent.calculations.record("2**10", "1024");
        let preview = index.results("ＡＮＳ / 2");
        assert_eq!(preview.refillable_text(), Some("512"));
        assert_eq!(index.results("ans ** 2").refillable_text(), Some("1048576"));
        index.recent.calculations.record("分数: 计算 1/3", "1/3");
        index
            .recent
            .calculations
            .record("精确结果", "多行输出\n根一");
        assert_eq!(index.results("max(0, ans*3)").refillable_text(), Some("1"));
        assert_eq!(index.results("ans2+ans1*3").refillable_text(), Some("1025"));
        assert_eq!(
            index.results("ＡＮＳ２ - 24").refillable_text(),
            Some("1000")
        );
        assert!(matches!(
            index.results("ans3 + 1"),
            Results::Value(Err(ValueError::Calculation(
                CalculationError::MissingAnswer
            )))
        ));
        assert_eq!(for_query("12+3", &[]).refillable_text(), Some("15"));
    }

    #[test]
    fn continues_long_exact_results_using_the_confirmed_answer() {
        let index = SearchIndex::default();
        for (query, continuation, expected) in [
            ("分数: 计算 2**1024", "分数: 计算 ans/2**1023", "2"),
            (
                "分数: 计算 shl(1,8191)/(shl(1,8190)+1)",
                "分数: 计算 ans*(shl(1,8190)+1)/shl(1,8190)",
                "2",
            ),
        ] {
            let result = index.results(query);
            assert_eq!(
                result.continuation_query().as_deref(),
                Some("分数: 计算 (ans)")
            );
            let Results::Value(Ok(result)) = result else {
                panic!("应返回精确算式结果");
            };
            assert!(result.text.len() > calculator::QUERY_LIMIT);
            index.recent.calculations.record(query, &result.text);
            assert_eq!(
                index.results(continuation).refillable_text(),
                Some(expected)
            );
        }
    }

    #[test]
    fn reuses_confirmed_answers_as_exact_fractions_without_float_rounding() {
        let index = SearchIndex::default();
        assert!(matches!(
            index.results("分数: 计算 ans+1"),
            Results::Value(Err(ValueError::Fraction(FractionError::MissingAnswer)))
        ));
        index
            .recent
            .calculations
            .record("高精度数值", "9007199254740993");
        index.recent.calculations.record("精确比例", "1/3");
        index.recent.calculations.record("多行结果", "3\n5");
        assert_eq!(
            index.results("分数: 计算 ans*3").refillable_text(),
            Some("1")
        );
        assert_eq!(
            index
                .results("分数: ＣＡＬＣ ＡＮＳ２ + ans1*3")
                .refillable_text(),
            Some("9007199254740994")
        );
        assert!(matches!(
            index.results("分数: 计算 ans3+1"),
            Results::Value(Err(ValueError::Fraction(FractionError::MissingAnswer)))
        ));
        for (query, expected) in [
            ("分数: 小数 ans to 8", "0.33333333"),
            ("分数: 循环小数 ans", "0.(3)"),
            ("分数: 百分比 ans1*3", "100%"),
            ("分数: 有效数字 ans1 to 4", "3.333e-1"),
            ("分数: 工程计数法 ans1 to 4", "333.3e-3"),
            ("分数: 限分母 ans1 to 2", "1/2"),
            ("分数: 带分数 ans2+1/4", "9007199254740993 1/4"),
        ] {
            assert_eq!(
                index.results(query).refillable_text(),
                Some(expected),
                "{query}"
            );
        }
        assert!(matches!(
            index.results("分数: 小数 ans3 to 8"),
            Results::Value(Err(ValueError::Fraction(FractionError::MissingAnswer)))
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
            "日期: 今天星期几",
            "颜色: #12",
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
                    && value.continuation_hint().is_some()
        ));
        assert_eq!(
            large.continuation_query().as_deref(),
            Some("分数: 计算 (ans)")
        );
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
            ("全局触发器: 发送正文", vec![global.clone()]),
            ("指定应用触发器: editor", vec![app.clone()]),
            ("已启用触发器: editor", vec![]),
            ("触发器: 状态＝已启用 范围=全局 发送正文", vec![global]),
            ("已停用触发器: scope=app editor", vec![app]),
            ("触发器: 状态=已停用 范围=所有应用 发送正文", vec![]),
        ] {
            let Results::Destinations(items) = for_query(query, &triggers) else {
                panic!("没有按触发器筛选：{query}");
            };
            assert_eq!(items, expected, "{query}");
        }
        for query in [
            "触发器: 状态=未就绪",
            "触发器: 输入=未知",
            "已启用触发器: 状态=已停用",
            "指定应用触发器: 范围=全局",
        ] {
            assert!(
                matches!(
                    for_query(query, &triggers),
                    Results::Value(Err(ValueError::TriggerSearch(_)))
                ),
                "{query}"
            );
        }
    }

    #[test]
    fn completes_trigger_filters_without_changing_previous_conditions() {
        let Results::FilterOptions(options) = for_query("触发器: 状态=", &[]) else {
            panic!("缺少状态选项");
        };
        assert_eq!(options.len(), 2);
        assert_eq!(options[0].label, Some("已启用"));
        assert_eq!(options[1].label, Some("已停用"));

        for (query, filled) in [
            (
                r#"已启用触发器： 输出=CSV 名称正则="^Open Ａ" INPUT＝ＫＥ"#,
                r#"已启用触发器： 输出=CSV 名称正则="^Open Ａ" INPUT＝keyboard"#,
            ),
            ("触发器: 范围=指", "触发器: 范围=指定应用"),
            ("触发器: 动作=快速搜", "触发器: 动作=快速搜索"),
        ] {
            let results = for_query(query, &[]);
            assert_eq!(results.suggested_input(0), Some(filled), "{query}");
            assert!(matches!(
                for_query(filled, &[]),
                Results::Destinations(_) | Results::Value(Ok(_))
            ));
        }
        assert!(matches!(
            for_query("已启用触发器: 状态=", &[]),
            Results::Value(Err(ValueError::TriggerSearch(
                TriggerSearchError::RepeatedFacet
            )))
        ));
    }

    #[test]
    fn exports_filtered_triggers_with_the_same_fields_and_order_as_the_list() {
        let mut bindings = crate::settings::defaults();
        let bindings = &mut bindings[..2];
        bindings[0].message = "甲,乙".into();
        bindings[0].enabled = true;
        bindings[1].message = "停用项".into();
        bindings[1].enabled = false;
        let triggers: Vec<_> = bindings.iter().map(SearchTrigger::from_binding).collect();
        let header = "名称,组合与条件,输入,动作,范围,启用状态";
        let Results::Value(Ok(result)) = for_query("触发器: 输出=CSV 状态=已启用", &triggers)
        else {
            panic!("未生成触发器 CSV");
        };
        let mut rows = result.text.lines();
        assert_eq!(rows.next(), Some(header));
        let row = rows.next().unwrap();
        assert!(row.starts_with("\"甲,乙\","));
        assert!(row.ends_with(",所有应用,已启用"));
        assert_eq!(rows.next(), None);
        assert!(matches!(
            for_query("已停用触发器: output=csv 没有该触发器", &triggers),
            Results::Value(Ok(result)) if result.text == header
        ));
    }

    #[test]
    fn exports_trigger_json_with_boolean_status_and_escaped_names() {
        let mut bindings = crate::settings::defaults();
        let bindings = &mut bindings[..2];
        bindings[0].message = "发送\"正文\"".into();
        bindings[0].enabled = true;
        bindings[1].message = "发送已停用".into();
        bindings[1].enabled = false;
        let triggers: Vec<_> = bindings.iter().map(SearchTrigger::from_binding).collect();
        let Results::Value(Ok(result)) =
            for_query("触发器: 输出＝ＪＳＯＮ 状态=已启用 发送", &triggers)
        else {
            panic!("未生成触发器 JSON");
        };
        let rows: Vec<serde_json::Value> = serde_json::from_str(&result.text).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["name"], "发送\"正文\"");
        assert_eq!(
            rows[0]["detail"].as_str(),
            Some(triggers[0].detail.as_str())
        );
        assert_eq!(rows[0]["enabled"].as_bool(), Some(true));
        for (query, expected) in [
            ("已停用触发器: output=json 不存在", "[]"),
            ("触发器: 输出=JSON 名称正则=^不存在$", "[]"),
        ] {
            assert!(matches!(
                for_query(query, &triggers),
                Results::Value(Ok(value)) if value.text == expected
            ));
        }
        let Results::Value(Ok(result)) = for_query("已停用触发器: output=json", &triggers)
        else {
            panic!("未生成已停用触发器 JSON");
        };
        let rows: Vec<serde_json::Value> = serde_json::from_str(&result.text).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["enabled"].as_bool(), Some(false));
    }

    #[test]
    fn completes_trigger_export_formats_with_the_original_scope_prefix() {
        let Results::FilterOptions(options) = for_query("触发器: 输出=", &[]) else {
            panic!("缺少导出格式");
        };
        assert_eq!(options.len(), 3);
        assert_eq!(options[0].label, Some("CSV"));
        assert_eq!(options[1].label, Some("JSON"));
        assert_eq!(options[2].label, Some("动作统计"));

        let query = "已启用触发器： OUTPUT＝ｊ";
        let result = for_query(query, &[]);
        let filled = result.suggested_input(0).unwrap();
        assert_eq!(filled, "已启用触发器： OUTPUT＝JSON");
        assert!(matches!(
            for_query(filled, &[]),
            Results::Value(Ok(value)) if value.text == "[]"
        ));
        for invalid in ["触发器: 输出=XML", "触发器: 输出=j 状态=已启用"] {
            assert!(matches!(
                for_query(invalid, &[]),
                Results::Value(Err(ValueError::TriggerSearch(
                    TriggerSearchError::InvalidExport
                )))
            ));
        }
    }

    #[test]
    fn sorts_triggers_by_name_and_reuses_the_order_in_both_exports() {
        let mut bindings = crate::settings::defaults();
        let bindings = &mut bindings[..2];
        bindings[0].message = "项目10".into();
        bindings[1].message = "项目２".into();
        bindings[0].enabled = true;
        bindings[1].enabled = true;
        bindings[0].action = crate::settings::ActionSpec::ShowSearch;
        bindings[1].action = crate::settings::ActionSpec::ShowPanel;
        let triggers: Vec<_> = bindings.iter().map(SearchTrigger::from_binding).collect();
        let first = Destination::Trigger(bindings[0].spec.id);
        let second = Destination::Trigger(bindings[1].spec.id);
        for (query, expected) in [
            ("触发器: 项目", vec![first.clone(), second.clone()]),
            (
                "触发器: 排序=动作 项目",
                vec![first.clone(), second.clone()],
            ),
            ("触发器: 状态=已启用 排序=名称 项目", vec![second, first]),
        ] {
            let Results::Destinations(items) = for_query(query, &triggers) else {
                panic!("缺少触发器列表：{query}");
            };
            assert_eq!(items, expected, "{query}");
        }
        let Results::Value(Ok(csv)) =
            for_query(r"触发器: 输出=CSV 排序=名称 名称正则=^项目\d+$", &triggers)
        else {
            panic!("缺少触发器 CSV");
        };
        assert!(csv.text.lines().nth(1).unwrap().starts_with("项目２,"));
        let Results::Value(Ok(json)) = for_query("触发器: 输出=JSON SORT=NAME 项目", &triggers)
        else {
            panic!("缺少触发器 JSON");
        };
        let rows: Vec<serde_json::Value> = serde_json::from_str(&json.text).unwrap();
        assert_eq!(rows[0]["name"], "项目２");
        assert_eq!(rows[1]["name"], "项目10");
        for invalid in ["触发器: 排序=未知", "触发器: 排序=名称 排序=动作"] {
            assert!(matches!(
                for_query(invalid, &triggers),
                Results::Value(Err(ValueError::TriggerSearch(_)))
            ));
        }
    }

    #[test]
    fn completes_trigger_sort_order_after_existing_export_and_filters() {
        let Results::FilterOptions(options) = for_query("触发器: 排序=", &[]) else {
            panic!("缺少排序选项");
        };
        assert_eq!(options.len(), 2);
        assert_eq!(options[0].label, Some("名称"));
        assert_eq!(options[1].label, Some("动作"));
        let query = r#"全局触发器： 输出=JSON 名称正则="^Open Ａ" SORT＝ｎ"#;
        let result = for_query(query, &[]);
        let filled = result.suggested_input(0).unwrap();
        assert_eq!(
            filled,
            r#"全局触发器： 输出=JSON 名称正则="^Open Ａ" SORT＝name"#
        );
        assert!(matches!(
            for_query(filled, &[]),
            Results::Value(Ok(value)) if value.text == "[]"
        ));
    }

    #[test]
    fn counts_action_categories_in_the_filtered_trigger_set() {
        let mut bindings = crate::settings::defaults();
        let bindings = &mut bindings[..3];
        bindings[0].action = crate::settings::ActionSpec::ShowPanel;
        for binding in &mut bindings[1..] {
            binding.action = crate::settings::ActionSpec::ShowSearch;
        }
        for binding in bindings.iter_mut() {
            binding.enabled = true;
            binding.message = "示例".into();
        }
        let triggers: Vec<_> = bindings.iter().map(SearchTrigger::from_binding).collect();
        for (query, expected) in [
            (
                "触发器: 输出=动作统计 状态=已启用 示例",
                "动作,数量\r\n快速搜索,2\r\n触发面板,1",
            ),
            (
                "已启用触发器: output=action-counts 动作=触发面板",
                "动作,数量\r\n触发面板,1",
            ),
            ("触发器: 输出=动作统计 状态=已停用", "动作,数量"),
        ] {
            assert!(
                matches!(
                    for_query(query, &triggers),
                    Results::Value(Ok(value)) if value.text == expected
                ),
                "{query}"
            );
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
