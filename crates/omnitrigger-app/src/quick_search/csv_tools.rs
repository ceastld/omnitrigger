use std::{
    borrow::Cow,
    cmp::Ordering,
    collections::{hash_map::Entry, BTreeMap, HashMap, HashSet, VecDeque},
    fmt::{Display, Write as _},
    hash::Hash,
    ops::Range,
};

use chrono::{
    format::{parse, Item, Parsed, StrftimeItems},
    DateTime, Datelike, NaiveDate, Utc,
};
use num_bigint::{BigInt, Sign};
use num_rational::BigRational;
use regex::Regex;
use serde::{
    de::DeserializeOwned,
    ser::{Serialize, SerializeMap, Serializer},
    Deserialize,
};
use serde_json::value::RawValue;

use super::scientific::{average, median};
use super::statistics::{
    interpolate_ordered_quantile, parse_bin_edges, parse_probability, parse_statistical_weight,
    parse_weighted_pair, plot_binned_float_values, plot_binned_unweighted,
    report_binned_unweighted, unweighted_box_fences, variance_unweighted, WeightedDistribution,
    WeightedMean, MAX_QUANTILE_PROBABILITIES,
};
use super::{
    bin_plot::BinPlot,
    box_plot::BoxPlot,
    calculator::{
        evaluate, format_result, narrow_ascii_graphic, normalize, sample_at_coordinates,
        QUERY_LIMIT,
    },
    calendar::{
        parse_complete_date, shift_calendar_days, shift_calendar_months, shift_calendar_workdays,
        workday_exceptions_from_dates, SpreadsheetDateSystem,
    },
    cidr::{CidrError, LongestPrefixLookup},
    conversion::{exact_data_converter, quantity_number_converter, ConversionError},
    correlation_plot::{CorrelationCell, CorrelationPlot},
    duration::{
        duration_nanoseconds, elapsed_clock_interval, exact_duration_converter,
        format_clock_duration, DurationError,
    },
    fraction::{
        calculate_fraction_with_variables, ensure_exact_size, format_engineering_notation,
        format_significant_digits, format_square_root, format_square_root_sum,
        fraction_alignment_converter, interpolate_fraction, parse_fraction_literal,
        rounded_decimal, FractionError, MAX_DECIMAL_PLACES,
    },
    json_tools::{
        json_array_cells_at_pointer, json_cell_at_pointer, json_pointer_tokens,
        json_wildcard_values, JsonCellContent, JsonError,
    },
    linear_fit::{doubled_midranks, LinearFitError, PairedSums, Point, MAX_POINTS},
    operators::{
        select_index_ranges, FieldSortOrder, IndexRangeError, NumberComparison, MAX_SORT_FIELDS,
    },
    radix::{convert_radix_fraction, convert_radix_integer},
    scatter_plot::{SampledSeries, ScatterPlot},
    statistics::{
        median_sorted_values, plot_box_unweighted, report_box_unweighted, summarize_unweighted,
        RunningTotals, StatisticsError, MAX_VALUES,
    },
    table_preview::TablePreview,
    text_tools::{
        append_padded_text, compare_natural, compile_regex, grapheme_common_substring,
        grapheme_edit_distance, newline_pattern, parse_template, regex_group_index,
        replace_literal_matches, replace_regex_matches, GraphemeRange, TemplatePart, TextError,
        TextLengthUnit, UnicodeForm, MAX_PADDING_WIDTH, MAX_TEXT_COMPARISON_WORK,
    },
    timestamp::{
        absolute_elapsed_nanoseconds, absolute_timestamp, format_absolute_elapsed,
        timestamp_alignment_converter, timestamp_clock_difference, timestamp_zone_converter,
    },
    url_encoding::url_parameter_values,
    vector::squared_euclidean_distance,
    InputValidationReport,
};

const MAX_PIVOT_BYTES: usize = 1024 * 1024;
const MAX_EXPANDED_CSV_BYTES: usize = 1024 * 1024;
const MAX_STACK_TABLES: usize = 16;
const MAX_VALUE_MAPPINGS: usize = 32;
const MAX_FORMULA_RECORDS: usize = 32;
const MAX_FUNCTION_RECORDS: usize = 128;
const MAX_ADDED_CELLS: usize = 16_384;
const MAX_REGEX_COLUMN_INPUT_BYTES: usize = 16 * 1024;
const MAX_REGEX_CAPTURE_COLUMNS: usize = 16;
const MAX_PIPELINE_STEPS: usize = 8;
const MAX_FILTER_CONDITIONS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CsvError {
    MissingInput,
    UnknownOperation,
    UnclosedQuote,
    InvalidSeparator,
    DelimiterArguments,
    UnknownDelimiter,
    AmbiguousDelimiter,
    DifferentColumnCount,
    TooManyAddedCells,
    InvalidHeader,
    MarkdownAlignments,
    MarkdownTable,
    InvalidColumns,
    InvalidColumnNames,
    NamedColumnArguments,
    NoColumnsRemain,
    InvalidRowRange,
    MissingDataRow,
    CellArguments,
    SetCellArguments,
    SetCellsArguments,
    CellEdit { index: usize, cause: &'static str },
    TableRegionArguments,
    GroupRowArguments,
    GroupRowsTooLarge,
    MissingColumnName,
    AmbiguousColumnName,
    RenameArguments,
    BulkRenameArguments,
    ReplaceValueArguments,
    MapValueArguments,
    MapValueTooLarge,
    NullReplacementValue,
    HeaderNameConflict,
    JoinArguments,
    JoinColumns,
    JoinTooLarge,
    HeaderComparisonArguments,
    ColumnComparisonArguments,
    AllColumnComparisonArguments,
    ComparisonHeaderMismatch,
    NoComparedColumns,
    DuplicateComparisonKey,
    ColumnComparisonTooLarge,
    ColumnPairDifferenceArguments,
    FillTooLarge,
    InterpolationColumns,
    ConstantColumnArguments,
    RowNumberArguments,
    RowNumberOverflow,
    ConcatenatedColumnArguments,
    TemplateColumnArguments,
    FirstNonemptyColumnArguments,
    RegexCaptureColumnArguments,
    RegexCaptureColumnsArguments,
    RegexCaptureColumnsInputTooLarge,
    SplitColumnArguments,
    SplitRowsArguments,
    SplitRowsTooLarge,
    UnitConversionColumnArguments,
    RadixColumnArguments,
    NumericComparisonColumnArguments,
    OutlierColumnArguments,
    DateDifferenceColumnArguments,
    ClockDifferenceColumnArguments,
    TimestampDifferenceColumnArguments,
    GroupTimestampDifferenceColumnArguments,
    GroupTimestampSessionArguments,
    GroupTimestampWindowArguments,
    GroupTimestampWindowValueArguments,
    GroupTimestampSummaryArguments,
    TimestampZoneColumnArguments,
    TimestampAlignmentColumnArguments,
    NumericAlignmentColumnArguments,
    DurationNumberColumnArguments,
    CidrColumnArguments,
    CidrLookup(CidrError),
    DateOffsetColumnArguments,
    WorkdayOffsetColumnArguments,
    WorkdayOffsetExceptions,
    DateFormatColumnArguments,
    DateFormatColumnPattern,
    SpreadsheetDateColumnArguments,
    TextLengthColumnArguments,
    TextSliceColumnArguments,
    TextPadColumnArguments,
    TextReplaceColumnArguments,
    RegexCountColumnArguments,
    RegexCountColumnInputTooLarge,
    RegexReplaceColumnArguments,
    RegexReplaceColumnInputTooLarge,
    JsonPointerColumnArguments,
    JsonWildcardColumnArguments,
    JsonPointerColumn(JsonError),
    UrlParameterColumnArguments,
    UnitConversionTarget(ConversionError),
    GroupedColumnArguments,
    GroupCountColumnArguments,
    ShiftedColumnArguments,
    MovingColumnArguments,
    UnicodeColumnArguments,
    EditDistanceColumnArguments,
    CommonSubstringColumnArguments,
    MovingQuantileColumnArguments,
    MovingDispersionColumnArguments,
    DataRowValue { row: usize, cause: &'static str },
    AddedColumnTooLarge,
    StackArguments,
    PipelineArguments,
    NestedPipeline,
    PipelineOutputTooLarge,
    PipelineStep { index: usize, cause: &'static str },
    ComputedColumnArguments,
    FormulaColumnArguments,
    FormulaColumnLimit,
    FormulaEvaluation(FractionError),
    FunctionColumnArguments,
    FunctionFilterArguments,
    FunctionNearFilterArguments,
    FunctionSortArguments,
    FunctionBinArguments,
    FunctionBinEdges,
    FunctionBinEmpty,
    FunctionRankArguments,
    FunctionSummaryArguments,
    FunctionGroupSummaryArguments,
    FunctionSummaryRange,
    FunctionColumnLimit,
    ComputedDivisionByZero,
    ComputedNumberRange,
    InvalidColumnValue,
    InvalidNumberFilter,
    FilterConditions,
    SearchRowKeywords,
    SearchRowColumns,
    Regex(TextError),
    ColumnOutOfRange,
    InvalidNumber,
    NumericIssuesTooLarge,
    DateIssuesTooLarge,
    DatePatternCheckArguments,
    LengthCheckArguments,
    HeaderCheckArguments,
    HeaderCheckTooLarge,
    ColumnDirectoryTooLarge,
    RegexCheckColumnsArguments,
    RegexCheckColumnsInputTooLarge,
    RegexIssuesTooLarge,
    NumericRangeCheckArguments,
    TimestampRangeCheckArguments,
    ValidationRulesArguments,
    RuleAnnotationArguments,
    ValidationRulesTooLarge,
    AllowedValuesCheckArguments,
    DecimalColumnsArguments,
    DecimalColumnsTooLarge,
    SignificantColumnsArguments,
    SignificantColumnsTooLarge,
    StatisticsColumn,
    ColumnBinsArguments,
    ObservationColumn,
    PairMatrixColumns,
    PairMatrixRows,
    DistanceMatrixArguments,
    DistanceMatrixRows,
    NearestPointArguments,
    NearestPointReference,
    StatisticsOutputTooLarge,
    AggregateColumn,
    GroupingColumns,
    GroupFitColumns,
    GroupFitTooLarge,
    FittedColumnsArguments,
    LinearFit(LinearFitError),
    ScatterColumns,
    ScatterPointCount,
    ScatterTooLarge,
    LinePlotPointCount,
    TrapezoidPointCount,
    TrapezoidDuplicateX,
    TrapezoidIntervalArguments,
    TrapezoidIntervalOrder,
    TrapezoidIntervalRange,
    CumulativeTrapezoidColumnArguments,
    SecantColumnArguments,
    SecantPointCount,
    SecantDuplicateX,
    CoordinateWindowArguments,
    CoordinateWindowPointCount,
    CoordinateInterpolationArguments,
    CoordinateInterpolationPointCount,
    CoordinateSlopePointCount,
    CoordinateSegmentPointCount,
    CoordinateDuplicateX,
    CoordinateInterpolationRange,
    CoordinateResampleArguments,
    CoordinateExtremaPointCount,
    CoordinateClipPointCount,
    CoordinateVariationPointCount,
    CoordinateLengthPointCount,
    CoordinateQuantileArguments,
    CoordinateQuantilePointCount,
    CoordinateDeviationPointCount,
    CoordinateCrossingArguments,
    CoordinateCrossingPointCount,
    CoordinateThresholdMeasurePointCount,
    CoordinateThresholdIntervalPointCount,
    CoordinateDifferenceArguments,
    CoordinateDifferencePointCount,
    CoordinateDifferenceNoOverlap,
    CoordinateCurveArguments,
    GroupSelectionArguments,
    SortFields,
    PivotColumns,
    PivotCountColumns,
    PivotValueColumns,
    UnpivotArguments,
    UnpivotColumns,
    PivotTooLarge,
    EmptyStatisticsColumn,
    WeightedMeanArguments,
    WeightedGroupArguments,
    WeightedFrequencyArguments,
    WeightedQuantileArguments,
    GroupQuantileArguments,
    Statistics(StatisticsError),
    Serialization,
    JsonNumericCells,
    JsonNumericObjectsTooLarge,
}

impl CsvError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::MissingInput => "在操作名后粘贴表格，可用换行开始数据",
            Self::UnknownOperation => "输入 ? CSV 查看可用操作",
            Self::UnclosedQuote => "字段的双引号尚未闭合",
            Self::InvalidSeparator => "检查引号和分隔符，引号内的双引号用两个双引号表示",
            Self::DelimiterArguments => "用 JSON 数组填写原分隔符和目标分隔符，例如 [\";\",\",\"]",
            Self::UnknownDelimiter => "需要至少两条等宽记录。也可用 分隔符转换 指定原分隔符",
            Self::AmbiguousDelimiter => "有多个可能的分隔符，请用 分隔符转换 指定一个",
            Self::DifferentColumnCount => "每行的列数须一致",
            Self::TooManyAddedCells => "每次最多补齐 16384 个空单元格，请缩小表格范围",
            Self::InvalidHeader => "CSV 表头的列名须非空且唯一",
            Self::MarkdownAlignments => "用 JSON 数组为每一列填写左、右、中或默认",
            Self::MarkdownTable => "粘贴一张 Markdown 表格，首行是表头，第二行用短横线分隔",
            Self::InvalidColumns => "用逗号隔开互不重复的列号，列号从 1 开始",
            Self::InvalidColumnNames => "用 JSON 字符串数组输入互不重复的列名",
            Self::NamedColumnArguments => "用 JSON 数组填写互不重复的列名或列号，例如 [\"地区\",2]",
            Self::NoColumnsRemain => "需要至少保留一列",
            Self::InvalidRowRange => "填写从 1 开始的数据行号或区间，例如 2,4-6",
            Self::MissingDataRow => "指定行号超出了表头之后的数据行数",
            Self::CellArguments => {
                "填写数据行号和列名或列号，例如 [2,\"备注\"]。首条数据行是 1，末条是 -1"
            }
            Self::SetCellArguments => {
                "填写数据行号、列名或列号和新的文本，例如 [2,\"备注\",\"已核对\"]"
            }
            Self::SetCellsArguments => "填写一组 [数据行号,列名或列号,新文本]，再粘贴 CSV",
            Self::CellEdit { cause, .. } => cause,
            Self::TableRegionArguments => {
                "填写数据行范围和一组不同的列，例如 [\"2-4\",[\"备注\",1]]"
            }
            Self::GroupRowArguments => {
                "填写分组列和组内非零行号，例如 [[\"部门\"],-1]。组内首条是 1，末条是 -1"
            }
            Self::GroupRowsTooLarge => "分组取行结果超过 1 MiB，请缩小表格范围",
            Self::MissingColumnName => "表头中找不到所选列名",
            Self::AmbiguousColumnName => "表头中存在多个同名列",
            Self::RenameArguments => "用 JSON 数组填写原列名和非空的新列名",
            Self::BulkRenameArguments => "填写一组 [原列名,新列名]，原列名不可重复，新列名须非空",
            Self::ReplaceValueArguments => "用 JSON 数组填写列名或列号、旧值和新值",
            Self::MapValueArguments => {
                "用 JSON 数组填写列名或列号，以及 1 至 32 个互不重复的 [旧值,新值]"
            }
            Self::MapValueTooLarge => "映射结果超过 1 MiB，请缩小表格范围",
            Self::NullReplacementValue => "新值不能包含空字符 U+0000",
            Self::HeaderNameConflict => "表头中已经有这个新列名",
            Self::JoinArguments => {
                "填写左右两侧的连接列，再填写包含两张 CSV 表格的 JSON 字符串数组"
            }
            Self::JoinColumns => "两侧连接列须具有相同列数，每一侧至少一列，列不可重复",
            Self::JoinTooLarge => "表格合并结果超过 1 MiB，请缩小表格范围",
            Self::HeaderComparisonArguments => "用 JSON 字符串数组填写原表和新表，两张表都需要表头",
            Self::ColumnComparisonArguments => {
                "填写键列与一个或多个对比列，例如 [[\"编号\"],[\"状态\",\"金额\"]]，再填写原表和新表"
            }
            Self::AllColumnComparisonArguments => "填写键列数组，例如 [\"编号\"]，再填写包含原表和新表的 JSON 字符串数组",
            Self::ComparisonHeaderMismatch => "两张表须包含相同的列名，列顺序可以不同",
            Self::NoComparedColumns => "请至少保留一个用来对比的非键列",
            Self::DuplicateComparisonKey => "对比列需要每张表中的键组合各出现一次",
            Self::ColumnComparisonTooLarge => "表格对比结果超过 1 MiB，请缩小表格范围",
            Self::ColumnPairDifferenceArguments => "填写两个不同的列名或列号，例如 [\"原备注\",\"新备注\"]",
            Self::FillTooLarge => "填充结果超过 1 MiB，请缩小表格范围",
            Self::InterpolationColumns => "填写数值列和分组列，例如 [\"金额\",[\"地区\"]]",
            Self::ConstantColumnArguments => {
                "用 JSON 数组填写新列名和固定文本，例如 [\"来源\",\"人工复核\"]"
            }
            Self::RowNumberArguments => "填写新列名和非负整数起点，例如 [\"序号\",1]",
            Self::RowNumberOverflow => "行号超过无符号 64 位整数范围",
            Self::ConcatenatedColumnArguments => {
                "填写新列名、至少两列和分隔符，例如 [\"地址\",[\"省\",\"市\"],\"-\"]"
            }
            Self::TemplateColumnArguments => {
                "填写新列名和模板，例如 [\"标签\",\"{编号}（{城市}）\"]。用 {{ 和 }} 表示花括号"
            }
            Self::FirstNonemptyColumnArguments => {
                "填写新列名和至少两个候选列，例如 [\"联系方式\",[\"手机\",\"座机\"]]"
            }
            Self::RegexCaptureColumnArguments => {
                "填写新列名、来源列、正则表达式和捕获组，捕获组用双引号写名称或编号"
            }
            Self::RegexCaptureColumnsArguments => {
                "填写来源列、正则表达式和 1 至 16 组 [新列名,捕获组]，新列名须非空且互不重复"
            }
            Self::RegexCaptureColumnsInputTooLarge => "来源列文本合计超过 16 KiB",
            Self::SplitColumnArguments => {
                "填写来源列、至少两个不同的新列名和非空分隔符，例如 [\"地址\",[\"省\",\"市\"],\"/\"]"
            }
            Self::SplitRowsArguments => "填写来源列和非空分隔符，例如 [\"标签\",\"/\"]",
            Self::SplitRowsTooLarge => "拆分后的表格超过 1 MiB，请缩小数据范围",
            Self::AddedColumnTooLarge => "追加列后的结果超过 1 MiB，请缩小表格范围",
            Self::UnitConversionColumnArguments => {
                "填写新列名、来源列和一个与源数值同类型的目标单位"
            }
            Self::RadixColumnArguments => {
                "填写新列名、来源列及 2 至 36 的两个进制，例如 [\"十进制\",\"编号\",16,10]"
            }
            Self::NumericComparisonColumnArguments => {
                "填写新列名、左列、右列和比较符，例如 [\"达标\",\"实测\",\"门槛\",\">=\"]"
            }
            Self::OutlierColumnArguments => "填写新列名和数值列，例如 [\"离群\",\"金额\"]",
            Self::DateDifferenceColumnArguments => {
                "填写新列名、开始日期列和结束日期列，例如 [\"天数\",\"开始\",\"结束\"]"
            }
            Self::ClockDifferenceColumnArguments => {
                "填写新列名、开始时刻列和结束时刻列，例如 [\"经过时长\",\"开始\",\"结束\"]"
            }
            Self::TimestampDifferenceColumnArguments => {
                "填写新列名、开始时间戳列和结束时间戳列，例如 [\"时长\",\"开始\",\"结束\"]"
            }
            Self::GroupTimestampDifferenceColumnArguments => {
                "填写新列名、时间戳列和分组列，例如 [\"间隔\",\"时刻\",[\"设备\"]]"
            }
            Self::GroupTimestampSessionArguments => {
                "填写新列名、时间戳列、分组列和非负间隔，例如 [\"会话\",\"时刻\",[\"设备\"],\"30分钟\"]"
            }
            Self::GroupTimestampWindowArguments => {
                "填写新列名、时间戳列、分组列和非负窗口时长，例如 [\"次数\",\"时刻\",[\"设备\"],\"30分钟\"]"
            }
            Self::GroupTimestampWindowValueArguments => {
                "填写新列名、时间戳列、数值列、分组列和非负窗口时长，例如 [\"合计\",\"时刻\",\"金额\",[\"设备\"],\"30分钟\"]"
            }
            Self::GroupTimestampSummaryArguments => {
                "填写分组列和时间戳列，例如 [[\"设备\"],\"时刻\"]"
            }
            Self::TimestampZoneColumnArguments => {
                "填写新列名、来源列和目标时区，例如 [\"标准时间\",\"时间\",\"UTC\"]"
            }
            Self::TimestampAlignmentColumnArguments => {
                "填写新列名、时间戳列、正时间步长和对齐方向，例如 [\"区间开始\",\"时刻\",\"15分钟\",\"向下\"]"
            }
            Self::NumericAlignmentColumnArguments => {
                "填写新列名、数值列、正步长和对齐方向，例如 [\"对齐值\",\"金额\",\"1/4\",\"就近\"]"
            }
            Self::DurationNumberColumnArguments => {
                "填写新列名、时长列和目标单位，例如 [\"分钟\",\"时长\",\"min\"]"
            }
            Self::CidrColumnArguments => "填写新列名、地址列和网段列表。网段名称列使用 [网段,名称] 列表",
            Self::CidrLookup(error) => error.message(),
            Self::DateOffsetColumnArguments => {
                "填写新列名、来源日期列和整数偏移量列，例如 [\"到期日期\",\"日期\",\"天数\"]"
            }
            Self::WorkdayOffsetColumnArguments => {
                "填写新列名、日期列、工作日数列，可追加包含休息日和补班日的数组"
            }
            Self::WorkdayOffsetExceptions => "例外日期须用 YYYY-MM-DD，每个日期只能出现一次",
            Self::DateFormatColumnArguments => {
                "填写新列名、日期源列和日期格式，例如 [\"标准日期\",\"日期\",\"%Y-%m-%d\"]"
            }
            Self::SpreadsheetDateColumnArguments => {
                "填写新列名、源列和日期系统，例如 [\"序号\",\"日期\",1900]，可选择 1900 或 1904"
            }
            Self::DateFormatColumnPattern => "填写有效的日期格式，例如 %Y-%m-%d 或 %d/%m/%Y",
            Self::TextLengthColumnArguments => {
                "填写新列名、来源列和单位，单位可用字素、码位、UTF-16 单元或 UTF-8 字节"
            }
            Self::TextSliceColumnArguments => {
                "填写新列名、来源列和字素位置，例如 [\"摘要\",\"文本\",\"2-3\"]"
            }
            Self::TextPadColumnArguments => {
                "填写新列名、来源列、0 至 4096 的宽度和一个填充字符，例如 [\"新编号\",\"编号\",4,\"0\"]"
            }
            Self::TextReplaceColumnArguments => {
                "填写新列名、来源列、要查找的非空文字和替换文字，例如 [\"新地址\",\"地址\",\"旧路\",\"新路\"]"
            }
            Self::RegexCountColumnArguments => {
                "填写新列名、来源列和正则表达式，例如 [\"次数\",\"备注\",\"\\\\d+\"]"
            }
            Self::RegexCountColumnInputTooLarge => "正则计数列的来源文本超过 16 KiB，请缩小表格范围",
            Self::RegexReplaceColumnArguments => {
                "填写新列名、来源列、正则和替换文本，例如 [\"结果\",\"备注\",\"([0-9]+)\",\"编号$1\"]"
            }
            Self::RegexReplaceColumnInputTooLarge => "正则替换列的来源文本超过 16 KiB，请缩小表格范围",
            Self::JsonPointerColumnArguments => {
                "填写新列名、含 JSON 的来源列和指针，例如 [\"城市\",\"数据\",\"/城市\"]"
            }
            Self::JsonWildcardColumnArguments => {
                "填写新列名、含 JSON 的来源列和路径数组，例如 [\"金额\",\"数据\",[\"订单\",null,\"金额\"]]"
            }
            Self::JsonPointerColumn(error) => error.message(),
            Self::UrlParameterColumnArguments => {
                "填写新列名、网址或查询串的来源列和参数名，例如 [\"标签\",\"网址\",\"tag\"]"
            }
            Self::UnitConversionTarget(cause) => cause.message(),
            Self::GroupedColumnArguments => {
                "填写新列名、数值列和分组列，例如 [\"结果\",\"金额\",[\"部门\"]]"
            }
            Self::GroupCountColumnArguments => {
                "填写新列名和分组列数组，例如 [\"记录数\",[\"部门\"]]"
            }
            Self::ShiftedColumnArguments => {
                "填写新列名、来源列、正整数行距和分组列，例如 [\"上期编号\",\"编号\",1,[\"部门\"]]"
            }
            Self::MovingColumnArguments => {
                "填写新列名、数值列、分组列和窗口大小，例如 [\"近两笔\",\"金额\",[\"部门\"],2]。窗口为 1 至 128"
            }
            Self::UnicodeColumnArguments => {
                "填写新列名、来源列和 NFC、NFD、NFKC 或 NFKD，例如 [\"标准名\",\"名称\",\"NFC\"]"
            }
            Self::EditDistanceColumnArguments => {
                "填写新列名和两个文本来源列，例如 [\"距离\",\"原文\",\"现文\"]"
            }
            Self::CommonSubstringColumnArguments => {
                "填写新列名和两个文本来源列，例如 [\"片段\",\"原名\",\"现名\"]"
            }
            Self::MovingQuantileColumnArguments => {
                "填写新列名、数值列、分组列、窗口大小和分位概率，例如 [\"P25\",\"金额\",[\"部门\"],2,\"25%\"]"
            }
            Self::MovingDispersionColumnArguments => {
                "填写新列名、数值列、分组列和窗口大小。总体窗口为 1 至 128，样本窗口为 2 至 128"
            }
            Self::DataRowValue { cause, .. } => cause,
            Self::StackArguments => "用 JSON 字符串数组填写 2 至 16 张含表头的 CSV 表格",
            Self::PipelineArguments => "用 JSON 字符串数组填写 1 至 8 条单行 CSV 命令，再粘贴原表格",
            Self::NestedPipeline => "每一步填写一条 CSV 命令",
            Self::PipelineOutputTooLarge => "处理结果超过 1 MiB，请缩小表格范围",
            Self::PipelineStep { cause, .. } => cause,
            Self::ComputedColumnArguments => {
                "用 JSON 数组填写新列名、两列和运算，例如 [\"金额\",2,\"*\",3]"
            }
            Self::FormulaColumnArguments => {
                "用 JSON 数组填写新列名、来源列和算式，例如 [\"平方\",2,\"x**2\"] 或 [\"金额\",2,3,\"x*y+1/6\"]"
            }
            Self::FormulaColumnLimit => "公式列每次最多处理 32 条数据",
            Self::FormulaEvaluation(error) => error.message(),
            Self::FunctionColumnArguments => {
                "填写新列名、一至两列和算式，例如 [\"余弦\",1,\"cosd(x)\"] 或 [\"加权\",1,2,\"cosd(x)*y\"]"
            }
            Self::FunctionFilterArguments => {
                "填写一至两列、算式、比较符和阈值，例如 [[1,2],\"cosd(x)*y\",\">\",\"0\"]"
            }
            Self::FunctionNearFilterArguments => {
                "填写一至两列、算式、目标值和非负容差，例如 [[1],\"x+0.2\",\"0.3\",\"1e-9\"]"
            }
            Self::FunctionSortArguments => {
                "填写一至两列、算式和数值排序方向，例如 [[1,2],\"cosd(x)*y\",\"数值降序\"]"
            }
            Self::FunctionRankArguments => {
                "填写新列名、一至两列、算式和数值排序方向，例如 [\"名次\",[1,2],\"x*y\",\"数值降序\"]"
            }
            Self::FunctionSummaryArguments => {
                "填写一至两列和算式，例如 [[1,2],\"cosd(x)*y\"]"
            }
            Self::FunctionGroupSummaryArguments => {
                "填写分组列、一至两列来源和算式，例如 [[\"部门\"],[2,3],\"x*y\"]"
            }
            Self::FunctionSummaryRange => "函数统计结果超出计算范围",
            Self::FunctionColumnLimit => "CSV 函数运算每次最多处理 128 条数据",
            Self::FunctionBinArguments => {
                "填写一至两列、算式和区间边界，例如 [[1,2],\"x*y\",\"0,1/2,2\"]"
            }
            Self::FunctionBinEdges => "填写 1 至 16 个可区分的有限升序边界，用逗号隔开",
            Self::FunctionBinEmpty => "所选行没有有限的函数值",
            Self::ComputedDivisionByZero => "计算列中遇到了除以零",
            Self::ComputedNumberRange => "计算列的结果超出精确计算范围",
            Self::InvalidColumnValue => "输入列号或带双引号的列名，再输入带双引号的值",
            Self::InvalidNumberFilter => "输入列号或带双引号的列名、比较符和数值",
            Self::FilterConditions => {
                "填写 1 至 16 个 [列,比较方式,\"值\"]，例如 [[\"状态\",\"文本=\",\"完成\"],[2,\">=\",\"3/2\"]]"
            }
            Self::Regex(error) => error.message(),
            Self::SearchRowKeywords => "填写 1 至 16 个非空关键词，例如 [\"茶\",\"华东\"]",
            Self::SearchRowColumns => "所选搜索列须互不重复，最多填写 16 列",
            Self::ColumnOutOfRange => "选中的列号超出了表格列数",
            Self::InvalidNumber => "所选列须填写数字或分数，例如 -2.5、1/3 或 3e-2",
            Self::NumericIssuesTooLarge => "数值列检查结果超过 1 MiB，请缩小表格范围",
            Self::DateIssuesTooLarge => "日期列检查结果超过 1 MiB，请缩小表格范围",
            Self::DatePatternCheckArguments => "填写列名或列号列表和日期格式",
            Self::LengthCheckArguments => "填写列列表和 [长度单位,最小长度,最大长度]，最小长度不能大于最大长度",
            Self::HeaderCheckArguments => "用非空的 JSON 字符串数组填写预期列名和顺序",
            Self::HeaderCheckTooLarge => "表头检查结果超过 1 MiB，请缩小表格范围",
            Self::ColumnDirectoryTooLarge => "列清单超过 1 MiB，请缩小表格范围",
            Self::RegexCheckColumnsArguments => "填写列名或列号列表和正则表达式",
            Self::RegexCheckColumnsInputTooLarge => "所选列的文本合计超过 16 KiB，请缩小表格范围",
            Self::RegexIssuesTooLarge => "正则检查结果超过 1 MiB，请缩小表格范围",
            Self::NumericRangeCheckArguments => "填写列列表和两个数值字符串，下界须不大于上界",
            Self::TimestampRangeCheckArguments => "填写列列表和两个明确的时间戳字符串，开始须不晚于结束",
            Self::ValidationRulesArguments => "填写最多 64 条列规则，例如 [[\"日期\",\"日期\"],[\"编号\",\"正则\",\"^A\"]]",
            Self::RuleAnnotationArguments => {
                "填写新列名和规则数组，例如 [\"通过\",[[\"日期\",\"日期\"]]]"
            }
            Self::ValidationRulesTooLarge => "规则检查结果超过 1 MiB，请缩小表格范围",
            Self::AllowedValuesCheckArguments => "填写列列表和非空的允许值列表，每个允许值只写一次",
            Self::DecimalColumnsArguments => "填写不同的列和 0 至 100 位小数，例如 [[\"金额\",3],2]",
            Self::DecimalColumnsTooLarge => "小数列结果超过 1 MiB，请缩小表格范围",
            Self::SignificantColumnsArguments => "填写不同的列和 1 至 100 位有效数字，例如 [[\"金额\",3],3]",
            Self::SignificantColumnsTooLarge => "有效数字列结果超过 1 MiB，请缩小表格范围",
            Self::StatisticsColumn => "列统计需要一个列号或带双引号的列名",
            Self::ColumnBinsArguments => "填写列名或列号和边界，例如 [\"金额\",\"0,1/2,2\"]",
            Self::ObservationColumn => "填写一个列号或带双引号的列名，例如 \"金额\"",
            Self::PairMatrixColumns => "填写 2 至 8 个不同的列名或列号，例如 [\"温度\",2]",
            Self::PairMatrixRows => "所选列至少需要两行坐标完整的数值",
            Self::DistanceMatrixArguments => {
                "填写标签列和 1 至 16 个不同的坐标列，例如 [\"编号\",[\"x\",\"y\"]]"
            }
            Self::DistanceMatrixRows => "支持 1 至 32 行坐标完整的数据",
            Self::NearestPointArguments => {
                "填写标签列、坐标列、基准点的标签和 1 至 31 个结果，例如 [\"编号\",[\"x\",\"y\"],\"A\",3]"
            }
            Self::NearestPointReference => "找不到基准点，请填写一个坐标完整的数据行的标签",
            Self::StatisticsOutputTooLarge => "统计结果超过 1 MiB，请缩小表格范围",
            Self::AggregateColumn => "填写一个要汇总的数值列号或带双引号的列名",
            Self::GroupingColumns => "分组列须非空且互不重复，例如 [\"地区\",\"状态\"]",
            Self::GroupFitColumns => "填写分组列和两列不同的坐标，例如 [[\"区域\"],[\"x\",\"y\"]]",
            Self::GroupFitTooLarge => "分组拟合结果超过 1 MiB，请缩小表格范围",
            Self::FittedColumnsArguments => {
                "填写分组列、x 和 y 坐标列及两个新列名，例如 [[\"区域\"],[\"x\",\"y\"],[\"预测\",\"残差\"]]"
            }
            Self::LinearFit(error) => error.message(),
            Self::ScatterColumns => "填写 x 和 y 两列不同的列名或列号，例如 [\"时间\",\"温度\"]",
            Self::ScatterPointCount => "散点图需要 2 至 64 行同时含 x 和 y 的数据",
            Self::ScatterTooLarge => "散点图 CSV 超过 1 MiB，请缩小数据范围",
            Self::LinePlotPointCount => "折线图需要 2 至 64 行同时含 x 和 y 的数据",
            Self::TrapezoidPointCount => "梯形积分需要 2 至 64 行同时含 x 和 y 的数据",
            Self::TrapezoidDuplicateX => "梯形积分的横坐标须互不相同",
            Self::TrapezoidIntervalArguments => {
                "填写两个坐标列，或同时指定计算区间，例如 [[\"时间\",\"高度\"],[\"1/2\",2]]"
            }
            Self::TrapezoidIntervalOrder => "计算区间的左端须小于右端",
            Self::TrapezoidIntervalRange => "计算区间须在完整样点的横坐标范围内",
            Self::CumulativeTrapezoidColumnArguments => {
                "填写新列名和两个坐标列，例如 [\"累计积分\",[\"时间\",\"高度\"]]"
            }
            Self::SecantColumnArguments => {
                "填写新列名和两个坐标列，例如 [\"斜率\",[\"时间\",\"高度\"]]"
            }
            Self::SecantPointCount => "差商列需要 2 至 64 行同时含 x 和 y 的数据",
            Self::SecantDuplicateX => "差商列的横坐标须互不相同",
            Self::CoordinateWindowArguments => {
                "填写新列名、两个坐标列和正的窗口长度，例如 [\"窗均值\",[\"时间\",\"高度\"],\"3/2\"]"
            }
            Self::CoordinateWindowPointCount => "时间窗计算需要 2 至 64 个完整样点",
            Self::CoordinateInterpolationArguments => {
                "填写两个坐标列和 1 至 64 个数值横坐标，例如 [[\"时间\",\"高度\"],[\"1/2\",2]]"
            }
            Self::CoordinateInterpolationPointCount => "线性插值需要 2 至 64 行同时含 x 和 y 的数据",
            Self::CoordinateSlopePointCount => "曲线斜率需要 2 至 64 个完整样点",
            Self::CoordinateSegmentPointCount => "分段直线需要 2 至 64 个完整样点",
            Self::CoordinateDuplicateX => "完整样点的横坐标须互不相同",
            Self::CoordinateInterpolationRange => "查询横坐标须在完整样点的范围内",
            Self::CoordinateResampleArguments => {
                "填写两个坐标列和 2 至 64 个采样点，例如 [[\"时间\",\"高度\"],5]"
            }
            Self::CoordinateExtremaPointCount => "曲线极值需要 2 至 64 个完整样点",
            Self::CoordinateClipPointCount => "曲线截取需要 2 至 64 个完整样点",
            Self::CoordinateVariationPointCount => "曲线变差需要 2 至 64 个完整样点",
            Self::CoordinateLengthPointCount => "曲线长度需要 2 至 64 个完整样点",
            Self::CoordinateQuantileArguments => {
                "填写两个坐标列和分位概率，例如 [[\"时间\",\"高度\"],\"25%\"]"
            }
            Self::CoordinateQuantilePointCount => "曲线分位数需要 2 至 64 个完整样点",
            Self::CoordinateDeviationPointCount => "曲线统计需要 2 至 64 个完整样点",
            Self::CoordinateCrossingArguments => {
                "填写两个坐标列和一个目标纵坐标，例如 [[\"时间\",\"高度\"],0]"
            }
            Self::CoordinateCrossingPointCount => "水平交点需要 2 至 64 行同时含 x 和 y 的数据",
            Self::CoordinateThresholdMeasurePointCount => "超阈计算需要 2 至 64 个完整样点",
            Self::CoordinateThresholdIntervalPointCount => "超阈区间需要 2 至 64 个完整样点",
            Self::CoordinateDifferenceArguments => {
                "填写横坐标列、两条曲线的值列及新的差值列名，例如 [[\"时间\",\"实测\",\"基准\"],\"差值\"]"
            }
            Self::CoordinateDifferencePointCount => "每条曲线都需要 2 至 64 个完整样点",
            Self::CoordinateDifferenceNoOverlap => "两条曲线的横坐标范围没有交集",
            Self::CoordinateCurveArguments => {
                "填写横坐标列和两条曲线的值列，例如 [\"时间\",\"实测\",\"基准\"]"
            }
            Self::GroupSelectionArguments => {
                "填写分组列、数值列和每组保留的正整数行数，例如 [[\"地区\"],\"金额\",2]"
            }
            Self::SortFields => {
                "填写 1 至 8 个 [列,排序方式]，例如 [[\"地区\",\"文本升序\"],[2,\"数值降序\"]]"
            }
            Self::PivotColumns => {
                "用 JSON 数组填写互不重复的行、列和数值列，例如 [\"地区\",\"状态\",\"金额\"]"
            }
            Self::PivotCountColumns => {
                "用 JSON 数组填写两个不同的行、列分类，例如 [\"地区\",\"状态\"]"
            }
            Self::UnpivotArguments => {
                "用 JSON 数组填写保留列、展开后的列名和值列名，例如 [[\"地区\"],\"月份\",\"金额\"]"
            }
            Self::PivotValueColumns => {
                "用 JSON 数组填写互不重复的行、列和原值列，例如 [\"地区\",\"月份\",\"编号\"]"
            }
            Self::UnpivotColumns => "至少留一列用于展开",
            Self::PivotTooLarge => "透视表结果超过 1 MiB，请缩小数据范围",
            Self::EmptyStatisticsColumn => "请在表头下至少提供一条数据",
            Self::WeightedMeanArguments => "依次填写数值列和权重列，例如 [\"成绩\",\"权重\"]",
            Self::WeightedGroupArguments => "依次填写分组列、数值列和权重列，例如 [[\"地区\"],\"成绩\",\"权重\"]",
            Self::WeightedFrequencyArguments => "填写分组列和权重列，例如 [[\"地区\",\"状态\"],\"权重\"]",
            Self::WeightedQuantileArguments => "填写数值列、权重列和概率列表，例如 [\"金额\",\"权重\",[\"25%\",0.5]]",
            Self::GroupQuantileArguments => "填写分组列、数值列和概率列表，例如 [[\"地区\"],\"金额\",[\"25%\",0.5]]",
            Self::Statistics(error) => error.message(),
            Self::Serialization => "无法生成 JSON",
            Self::JsonNumericCells => "所选列须为 JSON 数字或空白，例如 1.50 或 2e0",
            Self::JsonNumericObjectsTooLarge => "转换后的 JSON 超过 1 MiB，请缩小表格范围",
        }
    }
}

enum FillDirection {
    Down,
    Up,
}

enum TextComparison {
    EditDistance,
    CommonSubstring,
}

enum ColumnValidation {
    Number,
    Date,
    DatePattern(Vec<Item<'static>>),
    LengthRange(TextLengthUnit, usize, usize),
    Pattern(Regex),
    NumericRange {
        lower: BigRational,
        upper: BigRational,
    },
    AllowedValues(HashSet<String>),
    Json,
    Timestamp,
    TimestampRange {
        lower: DateTime<Utc>,
        upper: DateTime<Utc>,
    },
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ColumnRuleInput {
    Simple(ColumnReference, String),
    Pattern(ColumnReference, String, String),
    Allowed(ColumnReference, String, Vec<String>),
    Length(ColumnReference, String, (String, usize, usize)),
    Range(ColumnReference, String, String, String),
}

enum ColumnRuleOutput {
    Issues,
    ValidRows,
    InvalidRows,
    Summary,
    Annotate { name: String, count_failures: bool },
}

impl ColumnValidation {
    fn timestamp_range(lower: &str, upper: &str) -> Option<Self> {
        let lower = absolute_timestamp(lower).ok()?;
        let upper = absolute_timestamp(upper).ok()?;
        (lower <= upper).then_some(Self::TimestampRange { lower, upper })
    }

    fn length_range((unit, lower, upper): (String, usize, usize)) -> Result<Self, CsvError> {
        let unit = TextLengthUnit::parse(&unit).ok_or(CsvError::LengthCheckArguments)?;
        if lower > upper {
            return Err(CsvError::LengthCheckArguments);
        }
        Ok(Self::LengthRange(unit, lower, upper))
    }

    fn date_pattern(pattern: &str) -> Result<Self, CsvError> {
        if pattern.is_empty() || pattern.contains('\0') {
            return Err(CsvError::DateFormatColumnPattern);
        }
        StrftimeItems::new(pattern)
            .parse_to_owned()
            .map(Self::DatePattern)
            .map_err(|_| CsvError::DateFormatColumnPattern)
    }

    fn allowed_values(values: Vec<String>) -> Option<Self> {
        let count = values.len();
        let values: HashSet<_> = values.into_iter().collect();
        (count > 0 && values.len() == count).then_some(Self::AllowedValues(values))
    }

    fn numeric_range(lower: &str, upper: &str) -> Option<Self> {
        let lower = parse_fraction_literal(lower).ok()?;
        let upper = parse_fraction_literal(upper).ok()?;
        (lower <= upper).then_some(Self::NumericRange { lower, upper })
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Number => "数值",
            Self::Date => "日期",
            Self::DatePattern(_) => "日期格式",
            Self::LengthRange(..) => "长度范围",
            Self::Pattern(_) => "正则",
            Self::NumericRange { .. } => "数值范围",
            Self::AllowedValues(_) => "允许值",
            Self::Json => "JSON",
            Self::Timestamp => "时间戳",
            Self::TimestampRange { .. } => "时间范围",
        }
    }

    fn accepts(&self, cell: &str) -> bool {
        match self {
            Self::Number => parse_fraction_literal(cell).is_ok(),
            Self::Date => parse_complete_date(cell).is_ok(),
            Self::DatePattern(items) => parse_formatted_date(cell.trim(), items)
                .is_ok_and(|date| (0..=9999).contains(&date.year())),
            Self::LengthRange(unit, lower, upper) => (*lower..=*upper).contains(&unit.count(cell)),
            Self::Pattern(pattern) => pattern.is_match(cell),
            Self::AllowedValues(values) => values.contains(cell),
            Self::Json => serde_json::from_str::<&RawValue>(cell).is_ok(),
            Self::Timestamp => absolute_timestamp(cell).is_ok(),
            Self::TimestampRange { lower, upper } => {
                absolute_timestamp(cell).is_ok_and(|value| lower <= &value && &value <= upper)
            }
            Self::NumericRange { lower, upper } => {
                parse_fraction_literal(cell).is_ok_and(|value| lower <= &value && &value <= upper)
            }
        }
    }
}

#[derive(Clone, Copy)]
enum PivotKind {
    Sum,
    Count,
    Values,
}

impl PivotKind {
    fn column_requirements(self) -> (usize, CsvError) {
        match self {
            Self::Sum => (3, CsvError::PivotColumns),
            Self::Count => (2, CsvError::PivotCountColumns),
            Self::Values => (3, CsvError::PivotValueColumns),
        }
    }
}

enum NumberColumnFormat {
    Decimal,
    Scientific,
    Engineering,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum UnitColumnArguments {
    CellUnits(String, ColumnReference, String),
    FixedSource(String, ColumnReference, String, String),
}

enum Operation {
    PadRows,
    JsonRows,
    JsonObjects,
    JsonNumericObjects(Vec<ColumnReference>),
    Markdown(Option<Vec<MarkdownAlignment>>),
    ConvertDelimiter {
        from: char,
        to: char,
    },
    TrimColumns(Vec<ColumnReference>),
    UnicodeNormalizedColumn(String, ColumnReference, UnicodeForm),
    CompareTextColumns(String, ColumnReference, ColumnReference, TextComparison),
    FillEmptyCells(Vec<ColumnReference>, FillDirection),
    InterpolateEmptyCells(ColumnReference, Vec<ColumnReference>),
    CountEmptyCells(Vec<ColumnReference>),
    CheckHeader(Vec<String>),
    CheckColumns(Vec<ColumnReference>, ColumnValidation),
    CheckColumnRules(Vec<(ColumnReference, ColumnValidation)>, ColumnRuleOutput),
    RoundColumns {
        columns: Vec<ColumnReference>,
        digits: u32,
        format: NumberColumnFormat,
    },
    DropEmptyRecords,
    DropEmptyColumns,
    FilterMissingRows {
        columns: Vec<ColumnReference>,
        missing: bool,
    },
    SelectRows {
        ranges: String,
        keep_selected: bool,
    },
    RenameColumn([String; 2]),
    RenameColumns(Vec<[String; 2]>),
    ReplaceValue {
        column: ColumnReference,
        previous: String,
        replacement: String,
    },
    MapValues {
        column: ColumnReference,
        replacements: HashMap<String, String>,
    },
    ConstantColumn([String; 2]),
    RowNumberColumn(String, u64),
    ConcatenateColumns {
        name: String,
        columns: Vec<ColumnReference>,
        separator: String,
    },
    TemplateColumn([String; 2]),
    FirstNonemptyColumn(String, Vec<ColumnReference>),
    RegexCaptureColumn {
        name: String,
        source: ColumnReference,
        pattern: String,
        group: String,
        expand_rows: bool,
    },
    RegexCaptureColumns {
        source: ColumnReference,
        pattern: String,
        columns: Vec<[String; 2]>,
    },
    SplitColumn {
        source: ColumnReference,
        names: Vec<String>,
        separator: String,
    },
    SplitRows {
        source: ColumnReference,
        separator: String,
    },
    ComputeColumn {
        name: String,
        left: ColumnReference,
        right: Option<ColumnReference>,
        arithmetic: ColumnArithmetic,
    },
    FunctionColumn {
        name: String,
        left: ColumnReference,
        right: Option<ColumnReference>,
        expression: String,
    },
    ConvertUnitColumn {
        name: String,
        source: ColumnReference,
        target: String,
        source_unit: Option<String>,
        exact_data: bool,
    },
    RadixColumn {
        name: String,
        source: ColumnReference,
        from: u32,
        to: u32,
        fractional: bool,
    },
    NumericComparisonColumn(String, ColumnReference, ColumnReference, NumberComparison),
    OutlierColumn(String, ColumnReference),
    DateDifferenceColumn(String, ColumnReference, ColumnReference),
    ClockDifferenceColumn(String, ColumnReference, ColumnReference),
    TimestampDifferenceColumn(String, ColumnReference, ColumnReference),
    DateOffsetColumn {
        name: String,
        source: ColumnReference,
        amounts: ColumnReference,
        unit: DateOffsetUnit,
    },
    TextColumn(String, ColumnReference, TextColumnComputation),
    JsonPointerColumn(String, ColumnReference, Vec<String>, JsonCellContent),
    JsonWildcardColumn(String, ColumnReference, Vec<Option<String>>),
    UrlParameterColumn(String, ColumnReference, String),
    JsonExpandRows(String, ColumnReference, Vec<String>),
    SpreadsheetDateColumn(
        String,
        ColumnReference,
        SpreadsheetDateSystem,
        SpreadsheetColumnTransform,
    ),
    DatePatternColumn {
        name: String,
        source: ColumnReference,
        pattern: String,
        parse_input: bool,
    },
    RunningColumn {
        name: String,
        source: ColumnReference,
        groups: Vec<ColumnReference>,
        computation: RunningColumnComputation,
    },
    RankColumn {
        name: String,
        source: ColumnReference,
        groups: Vec<ColumnReference>,
        ascending: bool,
        ranking: CsvRanking,
    },
    ShareColumn {
        name: String,
        source: ColumnReference,
        groups: Vec<ColumnReference>,
    },
    ShiftedColumn {
        name: String,
        source: ColumnReference,
        distance: usize,
        groups: Vec<ColumnReference>,
        following: bool,
    },
    GroupCountColumn {
        name: String,
        groups: Vec<ColumnReference>,
        running: bool,
    },
    ContiguousGroupColumn {
        name: String,
        groups: Vec<ColumnReference>,
        measurement: ContiguousMeasurement,
    },
    DeduplicateRows,
    DeduplicateByColumns {
        columns: Vec<ColumnReference>,
        numeric: bool,
        keep_last: bool,
    },
    DuplicatesByColumns(Vec<ColumnReference>),
    DuplicatesByNumbers(Vec<ColumnReference>),
    CountByColumns(Vec<ColumnReference>),
    FrequencyByColumns(Vec<ColumnReference>),
    AggregateByColumns {
        columns: Vec<ColumnReference>,
        amount: ColumnReference,
        aggregation: ColumnAggregation,
    },
    GroupMedian {
        columns: Vec<ColumnReference>,
        amount: ColumnReference,
    },
    GroupFit {
        columns: Vec<ColumnReference>,
        coordinates: [ColumnReference; 2],
    },
    SelectGroupExtrema {
        columns: Vec<ColumnReference>,
        amount: ColumnReference,
        limit: usize,
        highest: bool,
    },
    Pivot(Vec<ColumnReference>, PivotKind),
    Unpivot(Vec<ColumnReference>, [String; 2]),
    ColumnStatistics {
        column: ColumnReference,
        sample: bool,
    },
    ColumnBins(ColumnReference, Vec<BigRational>),
    ColumnBoxSummary(ColumnReference),
    ColumnCumulativeDistribution(ColumnReference),
    NumericProfile(Vec<ColumnReference>),
    GroupNumericProfile {
        columns: Vec<ColumnReference>,
        amount: ColumnReference,
    },
    PairMatrix {
        columns: Vec<ColumnReference>,
        statistic: PairMatrixStatistic,
    },
    DistanceMatrix {
        labels: ColumnReference,
        coordinates: Vec<ColumnReference>,
    },
    NearestPoints {
        labels: ColumnReference,
        coordinates: Vec<ColumnReference>,
        target: String,
        limit: usize,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum MarkdownAlignment {
    #[serde(alias = "默认")]
    Default,
    #[serde(alias = "左")]
    Left,
    #[serde(alias = "中")]
    Center,
    #[serde(alias = "右")]
    Right,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum ColumnReference {
    Number(usize),
    Name(String),
}

impl ColumnReference {
    fn resolve(&self, header: &[String]) -> Result<usize, CsvError> {
        match self {
            Self::Number(number) => number.checked_sub(1).ok_or(CsvError::InvalidColumns),
            Self::Name(name) => header_column(header, name),
        }
    }
}

enum SpreadsheetColumnTransform {
    DateToSerial,
    SerialToDate,
    TimeToSerial,
    SerialToTime,
}

enum ColumnArithmetic {
    Add,
    Subtract,
    Multiply,
    Divide,
    Formula(String),
}

#[derive(Clone, Copy)]
enum CsvRanking {
    Competition,
    Dense,
    PercentRank,
    CumulativeDistribution,
}

enum RunningColumnComputation {
    Sum,
    Average,
    Difference,
    RelativeChange,
    Moving {
        width: usize,
        average: bool,
    },
    MovingStatistic {
        width: usize,
        statistic: MovingColumnStatistic,
    },
}

enum MovingColumnStatistic {
    Quantile(BigRational),
    Variance { sample: bool, root: bool },
}

enum RunningColumnState<'a> {
    Accumulation(RunningTotals),
    Previous(Option<BigRational>),
    Moving {
        values: VecDeque<BigRational>,
        sum: BigRational,
        width: usize,
        average: bool,
    },
    MovingStatistic {
        values: VecDeque<BigRational>,
        width: usize,
        statistic: &'a MovingColumnStatistic,
    },
}

#[derive(Deserialize)]
#[serde(untagged)]
enum FormulaColumnArguments {
    Single((String, ColumnReference, String)),
    Pair((String, ColumnReference, ColumnReference, String)),
}

impl FormulaColumnArguments {
    fn into_parts(self) -> (String, ColumnReference, Option<ColumnReference>, String) {
        match self {
            Self::Single((name, left, expression)) => (name, left, None, expression),
            Self::Pair((name, left, right, expression)) => (name, left, Some(right), expression),
        }
    }
}

impl ColumnArithmetic {
    fn parse(operator: &str) -> Result<Self, CsvError> {
        match operator {
            "+" => Ok(Self::Add),
            "-" => Ok(Self::Subtract),
            "*" | "×" => Ok(Self::Multiply),
            "/" | "÷" => Ok(Self::Divide),
            _ => Err(CsvError::ComputedColumnArguments),
        }
    }

    fn apply(
        &self,
        left: BigRational,
        right: Option<BigRational>,
    ) -> Result<BigRational, CsvError> {
        Ok(match (self, right) {
            (Self::Add, Some(right)) => left + right,
            (Self::Subtract, Some(right)) => left - right,
            (Self::Multiply, Some(right)) => left * right,
            (Self::Divide, Some(right)) if right == BigRational::default() => {
                return Err(CsvError::ComputedDivisionByZero);
            }
            (Self::Divide, Some(right)) => left / right,
            (Self::Formula(expression), right) => {
                return calculate_fraction_with_variables(expression, Some(&left), right.as_ref())
                    .map_err(|error| match error {
                        FractionError::ZeroDenominator => CsvError::ComputedDivisionByZero,
                        FractionError::Invalid | FractionError::Incomplete => {
                            CsvError::FormulaColumnArguments
                        }
                        _ => CsvError::FormulaEvaluation(error),
                    });
            }
            (_, None) => return Err(CsvError::ComputedColumnArguments),
        })
    }
}

#[derive(Clone, Copy)]
enum ColumnAggregation {
    Sum,
    Average,
    Minimum,
    Maximum,
    Range,
}

enum ColumnCondition {
    Exact(String),
    Regex(Regex),
    Number(NumberComparison, BigRational),
}

impl ColumnCondition {
    fn matches(&self, field: &str) -> Result<bool, CsvError> {
        Ok(match self {
            Self::Exact(value) => field == value.as_str(),
            Self::Regex(pattern) => pattern.is_match(field),
            Self::Number(comparison, threshold) => {
                let value = parse_fraction_literal(field).map_err(|_| CsvError::InvalidNumber)?;
                comparison.accepts(value.cmp(threshold))
            }
        })
    }
}

struct ColumnFilter {
    column: ColumnReference,
    condition: ColumnCondition,
    expected: bool,
}

pub(super) fn transform_csv(query: &str) -> Result<String, CsvError> {
    let (operation, input) = split_argument(query).ok_or(CsvError::MissingInput)?;
    if operation == "依次处理" || operation.eq_ignore_ascii_case("pipeline") {
        return transform_csv_pipeline(input);
    }
    transform_csv_operation(operation, input)
}

fn transform_csv_operation(operation: &str, mut input: &str) -> Result<String, CsvError> {
    if let Some(delimiter) = validation_delimiter(operation) {
        return validate_csv(input, delimiter).map(|report| report.text);
    }
    if let Some(table) = coordinate_line_table(operation, input) {
        return table?.to_csv();
    }
    if let Some(mode) = CsvKeywordMatch::from_operation(operation) {
        return search_csv_rows(input, mode, false).map(|matches| write_rows(&matches.rows, ','));
    }
    if let Some(result) = csv_preview_output(operation, input) {
        return result.map(|(text, _)| text);
    }
    let operation = operation.to_ascii_lowercase();
    let kind = match operation.as_str() {
        "依次处理" | "pipeline" => return Err(CsvError::NestedPipeline),
        "列清单" | "column-directory" => return csv_column_directory(input),
        "加权平均" | "weighted-mean" => return csv_weighted_mean(input),
        "拟合列" | "fit-columns" => return append_fit_columns(input),
        "函数筛选" | "filter-function" => return filter_function_rows(input),
        "函数近似筛选" | "filter-function-near" => return filter_near_function_rows(input),
        "函数排序" | "sort-function" => return sort_function_rows(input),
        "函数分箱" | "function-bins" | "函数分箱图" | "function-bin-plot" => {
            return build_function_bin_plot(input).map(|(csv, _)| csv);
        }
        "函数排名列" | "rank-function-column" => return rank_function_rows(input),
        "函数摘要" | "summary-function" => return summarize_function_rows(input),
        "函数分组摘要" | "group-function-summary" => {
            return summarize_grouped_function_rows(input)
        }
        operation if is_scatter_operation(operation) => {
            return parse_coordinate_table(input, CsvError::ScatterPointCount)?.to_csv();
        }
        "梯形积分" | "trapezoid-integral" => {
            return trapezoid_statistic(input, TrapezoidMeasure::Integral)
        }
        "梯形平均" | "trapezoid-mean" => {
            return trapezoid_statistic(input, TrapezoidMeasure::Mean)
        }
        "梯形面积" | "trapezoid-area" => {
            return trapezoid_statistic(input, TrapezoidMeasure::Area)
        }
        "梯形累计列" | "cumulative-trapezoid-column" => {
            return append_sampled_coordinate_column(input, CoordinateColumn::CumulativeIntegral)
        }
        "差商列" | "secant-column" => {
            return append_sampled_coordinate_column(input, CoordinateColumn::Secant)
        }
        "时间窗平均列" | "time-window-mean-column" => {
            return append_coordinate_window_measure(input, CoordinateWindowMeasure::Mean)
        }
        "时间窗积分列" | "time-window-integral-column" => {
            return append_coordinate_window_measure(input, CoordinateWindowMeasure::Integral)
        }
        "线性插值" | "linear-interpolate" => return interpolate_coordinate_table(input),
        "曲线斜率" | "curve-slopes" => return coordinate_slopes(input),
        "分段直线" | "linear-segments" => return coordinate_linear_segments(input),
        "等距插值" | "resample-linear" => return resample_coordinate_table(input),
        "曲线极值" | "curve-extrema" => return coordinate_extrema(input),
        "曲线变差" | "curve-total-variation" => return coordinate_total_variation(input),
        "曲线长度" | "curve-length" => return coordinate_curve_length(input),
        "曲线分位数" | "curve-quantile" => return coordinate_curve_quantile(input),
        "曲线均方根" | "curve-rms" => {
            return coordinate_deviation_statistic(input, CoordinateDeviation::RootMeanSquare)
        }
        "曲线标准差" | "curve-standard-deviation" => {
            return coordinate_deviation_statistic(input, CoordinateDeviation::StandardDeviation)
        }
        "曲线截取" | "clip-curve" => {
            let mut table = read_coordinate_interval(input, CsvError::CoordinateClipPointCount)?;
            sort_distinct_coordinates(&mut table.samples)?;
            return table.to_csv();
        }
        "水平交点" | "horizontal-crossings" => return coordinate_horizontal_crossings(input),
        "超阈跨度" | "span-above-threshold" => {
            return coordinate_threshold_measure(input, CoordinateThresholdMeasure::Span)
        }
        "超阈面积" | "area-above-threshold" => {
            return coordinate_threshold_measure(input, CoordinateThresholdMeasure::Area)
        }
        "超阈区间" | "intervals-above-threshold" => {
            return coordinate_intervals_above_threshold(input)
        }
        "曲线差值" | "curve-difference" | "曲线对比图" | "curve-comparison-plot" => {
            let (curves, name) = read_coordinate_curve_comparison(input)?;
            return curves.to_csv(&name);
        }
        "曲线交点" | "curve-crossings" => return coordinate_curve_crossings(input),
        "曲线间面积" | "area-between-curves" => return coordinate_curve_integral(input, true),
        "曲线差积分" | "curve-difference-integral" => {
            return coordinate_curve_integral(input, false)
        }
        "补齐列" | "pad-columns" => Operation::PadRows,
        "左连接" | "left-join" => return join_csv(input, JoinKind::Left),
        "内连接" | "inner-join" => return join_csv(input, JoinKind::Inner),
        "全连接" | "full-join" => return join_csv(input, JoinKind::Full),
        "左表匹配" | "semi-join" => return join_csv(input, JoinKind::MatchedLeft),
        "左表独有" | "anti-join" => return join_csv(input, JoinKind::UnmatchedLeft),
        "表头对比" | "compare-headers" => return compare_csv_headers(input),
        "对比行" | "compare-rows" => return compare_csv_rows(input),
        "对比列" | "compare-column" => return compare_csv_column(input, ComparisonMode::Text),
        "两列差异" | "column-pair-differences" => return compare_csv_column_pair(input, false),
        "两列数值差异" | "numeric-column-pair-differences" => {
            return compare_csv_column_pair(input, true)
        }
        "数值对比列" | "compare-numeric-column" => {
            return compare_csv_column(input, ComparisonMode::Numeric)
        }
        "对比全部列" | "compare-all-columns" => {
            return compare_csv_column(input, ComparisonMode::All)
        }
        "纵向合并" | "stack" => return stack_csv(input),
        "横向合并" | "bind-columns" => return bind_csv_columns(input),
        "转json" | "json" => Operation::JsonRows,
        "对象" | "表头" | "objects" => Operation::JsonObjects,
        "数值对象" | "numeric-objects" => {
            let (columns, data) =
                column_reference_arguments(input, CsvError::NamedColumnArguments)?;
            input = data;
            Operation::JsonNumericObjects(columns)
        }
        "markdown" | "表格" => Operation::Markdown(None),
        "从markdown" | "from-markdown" => return read_markdown_table(input),
        "markdown对齐" | "markdown-align" => {
            let (alignments, data) = json_argument_and_data(input, CsvError::MarkdownAlignments)?;
            input = data;
            Operation::Markdown(Some(alignments))
        }
        "转tsv" | "tsv" => Operation::ConvertDelimiter {
            from: ',',
            to: '\t',
        },
        "自动转csv" | "auto-csv" => return auto_convert_delimiter(input),
        "分隔符转换" | "convert-delimiter" => {
            let ([from, to], data) =
                json_argument_and_data::<[char; 2]>(input, CsvError::DelimiterArguments)?;
            if [from, to]
                .into_iter()
                .any(|separator| matches!(separator, '\0' | '\r' | '\n' | '"'))
            {
                return Err(CsvError::DelimiterArguments);
            }
            input = data;
            Operation::ConvertDelimiter { from, to }
        }
        "去重" | "deduplicate" => Operation::DeduplicateRows,
        "去空行" | "drop-empty-rows" => Operation::DropEmptyRecords,
        "去空列" | "drop-empty-columns" => Operation::DropEmptyColumns,
        "完整行" | "complete-rows" | "缺失行" | "incomplete-rows" => {
            let (columns, data) =
                column_reference_arguments(input, CsvError::NamedColumnArguments)?;
            input = data;
            Operation::FilterMissingRows {
                columns,
                missing: matches!(operation.as_str(), "缺失行" | "incomplete-rows"),
            }
        }
        operation @ ("取行" | "pick-rows" | "删行" | "drop-rows") => {
            let (ranges, data) = split_argument(input).ok_or(CsvError::MissingInput)?;
            input = data;
            Operation::SelectRows {
                ranges: ranges.to_owned(),
                keep_selected: matches!(operation, "取行" | "pick-rows"),
            }
        }
        "取单元格" | "pick-cell" => return pick_csv_cell(input),
        "分组取行" | "pick-group-row" => return select_group_rows(input),
        "去空白" | "trim-columns" => {
            let (columns, data) =
                column_reference_arguments(input, CsvError::NamedColumnArguments)?;
            input = data;
            Operation::TrimColumns(columns)
        }
        "unicode规范化列" | "unicode-normalize-column" => {
            let ((name, source, form), data) =
                json_argument_and_data::<(String, ColumnReference, String)>(
                    input,
                    CsvError::UnicodeColumnArguments,
                )?;
            let form = UnicodeForm::parse(&form).ok_or(CsvError::UnicodeColumnArguments)?;
            if name.trim().is_empty() || name.contains('\0') {
                return Err(CsvError::UnicodeColumnArguments);
            }
            input = data;
            Operation::UnicodeNormalizedColumn(name, source, form)
        }
        comparison @ ("编辑距离列"
        | "edit-distance-column"
        | "公共片段列"
        | "common-substring-column") => {
            let (comparison, argument_error) = match comparison {
                "编辑距离列" | "edit-distance-column" => (
                    TextComparison::EditDistance,
                    CsvError::EditDistanceColumnArguments,
                ),
                _ => (
                    TextComparison::CommonSubstring,
                    CsvError::CommonSubstringColumnArguments,
                ),
            };
            let ((name, first, second), data) = json_argument_and_data::<(
                String,
                ColumnReference,
                ColumnReference,
            )>(input, argument_error)?;
            if name.trim().is_empty() || name.contains('\0') {
                return Err(argument_error);
            }
            input = data;
            Operation::CompareTextColumns(name, first, second, comparison)
        }
        kind @ ("向下填充" | "fill-down" | "向上填充" | "fill-up") => {
            let (columns, data) =
                column_reference_arguments(input, CsvError::NamedColumnArguments)?;
            input = data;
            let direction = if matches!(kind, "向上填充" | "fill-up") {
                FillDirection::Up
            } else {
                FillDirection::Down
            };
            Operation::FillEmptyCells(columns, direction)
        }
        "插值填充" | "fill-interpolate" => {
            let ((column, groups), data) = json_argument_and_data::<(
                ColumnReference,
                Vec<ColumnReference>,
            )>(input, CsvError::InterpolationColumns)?;
            input = data;
            Operation::InterpolateEmptyCells(column, groups)
        }
        "空值统计" | "count-empty" => {
            let (columns, data) =
                column_reference_arguments(input, CsvError::NamedColumnArguments)?;
            input = data;
            Operation::CountEmptyCells(columns)
        }
        "检查数值列"
        | "check-numeric-columns"
        | "检查日期列"
        | "check-date-columns"
        | "检查json列"
        | "check-json-columns"
        | "检查时间戳列"
        | "check-timestamp-columns" => {
            let (columns, data) =
                column_reference_arguments(input, CsvError::NamedColumnArguments)?;
            input = data;
            let validation = match operation.as_str() {
                "检查日期列" | "check-date-columns" => ColumnValidation::Date,
                "检查json列" | "check-json-columns" => ColumnValidation::Json,
                "检查时间戳列" | "check-timestamp-columns" => ColumnValidation::Timestamp,
                _ => ColumnValidation::Number,
            };
            Operation::CheckColumns(columns, validation)
        }
        "检查表头" | "check-header" => {
            let invalid = CsvError::HeaderCheckArguments;
            let (expected, data) = json_argument_and_data::<Vec<String>>(input, invalid)?;
            if expected.is_empty() {
                return Err(invalid);
            }
            input = data;
            Operation::CheckHeader(expected)
        }
        "检查长度范围" | "check-length-range" => {
            let invalid = CsvError::LengthCheckArguments;
            let ((columns, bounds), data) = json_argument_and_data::<(
                Vec<ColumnReference>,
                (String, usize, usize),
            )>(input, invalid)?;
            if columns.is_empty() {
                return Err(invalid);
            }
            let validation = ColumnValidation::length_range(bounds)?;
            input = data;
            Operation::CheckColumns(columns, validation)
        }
        "检查日期格式" | "check-date-format" => {
            let invalid = CsvError::DatePatternCheckArguments;
            let ((columns, pattern), data) =
                json_argument_and_data::<(Vec<ColumnReference>, String)>(input, invalid)?;
            if columns.is_empty() {
                return Err(invalid);
            }
            let validation = ColumnValidation::date_pattern(&pattern)?;
            input = data;
            Operation::CheckColumns(columns, validation)
        }
        "检查正则列" | "check-regex-columns" => {
            let ((columns, pattern), data) =
                json_argument_and_data::<(Vec<ColumnReference>, String)>(
                    input,
                    CsvError::RegexCheckColumnsArguments,
                )?;
            if columns.is_empty() {
                return Err(CsvError::RegexCheckColumnsArguments);
            }
            let pattern = compile_regex(&pattern).map_err(CsvError::Regex)?;
            input = data;
            Operation::CheckColumns(columns, ColumnValidation::Pattern(pattern))
        }
        "检查时间范围" | "check-timestamp-range" => {
            let invalid = CsvError::TimestampRangeCheckArguments;
            let ((columns, [lower, upper]), data) =
                json_argument_and_data::<(Vec<ColumnReference>, [String; 2])>(input, invalid)?;
            if columns.is_empty() {
                return Err(invalid);
            }
            let validation = ColumnValidation::timestamp_range(&lower, &upper).ok_or(invalid)?;
            input = data;
            Operation::CheckColumns(columns, validation)
        }
        "检查数值范围" | "check-numeric-range" => {
            let ((columns, [lower, upper]), data) =
                json_argument_and_data::<(Vec<ColumnReference>, [String; 2])>(
                    input,
                    CsvError::NumericRangeCheckArguments,
                )?;
            let invalid = CsvError::NumericRangeCheckArguments;
            if columns.is_empty() {
                return Err(invalid);
            }
            let validation = ColumnValidation::numeric_range(&lower, &upper).ok_or(invalid)?;
            input = data;
            Operation::CheckColumns(columns, validation)
        }
        "检查允许值" | "check-allowed-values" => {
            let invalid = CsvError::AllowedValuesCheckArguments;
            let ((columns, values), data) =
                json_argument_and_data::<(Vec<ColumnReference>, Vec<String>)>(input, invalid)?;
            if columns.is_empty() {
                return Err(invalid);
            }
            let validation = ColumnValidation::allowed_values(values).ok_or(invalid)?;
            input = data;
            Operation::CheckColumns(columns, validation)
        }
        "检查规则"
        | "check-rules"
        | "规则保留行"
        | "keep-valid-rows"
        | "规则问题行"
        | "keep-invalid-rows"
        | "规则统计"
        | "rule-summary"
        | "规则标记列"
        | "rule-pass-column"
        | "规则失败数列"
        | "rule-failure-count-column" => {
            let invalid = CsvError::ValidationRulesArguments;
            let (rules, data, output) = match operation.as_str() {
                "规则标记列"
                | "rule-pass-column"
                | "规则失败数列"
                | "rule-failure-count-column" => {
                    let annotation_error = CsvError::RuleAnnotationArguments;
                    let ((name, rules), data) = json_argument_and_data::<(
                        String,
                        Vec<ColumnRuleInput>,
                    )>(input, annotation_error)?;
                    if name.trim().is_empty() || name.contains('\0') {
                        return Err(annotation_error);
                    }
                    let count_failures = matches!(
                        operation.as_str(),
                        "规则失败数列" | "rule-failure-count-column"
                    );
                    (
                        rules,
                        data,
                        ColumnRuleOutput::Annotate {
                            name,
                            count_failures,
                        },
                    )
                }
                kind => {
                    let (rules, data) =
                        json_argument_and_data::<Vec<ColumnRuleInput>>(input, invalid)?;
                    let output = match kind {
                        "规则保留行" | "keep-valid-rows" => ColumnRuleOutput::ValidRows,
                        "规则问题行" | "keep-invalid-rows" => ColumnRuleOutput::InvalidRows,
                        "规则统计" | "rule-summary" => ColumnRuleOutput::Summary,
                        _ => ColumnRuleOutput::Issues,
                    };
                    (rules, data, output)
                }
            };
            if !(1..=64).contains(&rules.len()) {
                return Err(invalid);
            }
            let rules = rules
                .into_iter()
                .map(|rule| match rule {
                    ColumnRuleInput::Simple(column, kind) => {
                        let validation = match kind.to_ascii_lowercase().as_str() {
                            "数值" | "number" => ColumnValidation::Number,
                            "日期" | "date" => ColumnValidation::Date,
                            "json" => ColumnValidation::Json,
                            "时间戳" | "timestamp" => ColumnValidation::Timestamp,
                            _ => return Err(invalid),
                        };
                        Ok((column, validation))
                    }
                    ColumnRuleInput::Pattern(column, kind, pattern)
                        if kind == "正则" || kind.eq_ignore_ascii_case("regex") =>
                    {
                        let pattern = compile_regex(&pattern).map_err(CsvError::Regex)?;
                        Ok((column, ColumnValidation::Pattern(pattern)))
                    }
                    ColumnRuleInput::Pattern(column, kind, pattern)
                        if kind == "日期格式" || kind.eq_ignore_ascii_case("date-format") =>
                    {
                        Ok((column, ColumnValidation::date_pattern(&pattern)?))
                    }
                    ColumnRuleInput::Range(column, kind, lower, upper)
                        if kind == "数值范围" || kind.eq_ignore_ascii_case("numeric-range") =>
                    {
                        let range =
                            ColumnValidation::numeric_range(&lower, &upper).ok_or(invalid)?;
                        Ok((column, range))
                    }
                    ColumnRuleInput::Range(column, kind, lower, upper)
                        if kind == "时间范围" || kind.eq_ignore_ascii_case("timestamp-range") =>
                    {
                        let range =
                            ColumnValidation::timestamp_range(&lower, &upper).ok_or(invalid)?;
                        Ok((column, range))
                    }
                    ColumnRuleInput::Allowed(column, kind, values)
                        if kind == "允许值" || kind.eq_ignore_ascii_case("allowed-values") =>
                    {
                        Ok((
                            column,
                            ColumnValidation::allowed_values(values).ok_or(invalid)?,
                        ))
                    }
                    ColumnRuleInput::Length(column, kind, bounds)
                        if kind == "长度范围" || kind.eq_ignore_ascii_case("length-range") =>
                    {
                        Ok((column, ColumnValidation::length_range(bounds)?))
                    }
                    _ => Err(invalid),
                })
                .collect::<Result<Vec<_>, _>>()?;
            input = data;
            Operation::CheckColumnRules(rules, output)
        }
        "小数列"
        | "decimal-columns"
        | "有效数字列"
        | "significant-columns"
        | "工程计数列"
        | "engineering-columns" => {
            let format = match operation.as_str() {
                "工程计数列" | "engineering-columns" => NumberColumnFormat::Engineering,
                "有效数字列" | "significant-columns" => NumberColumnFormat::Scientific,
                _ => NumberColumnFormat::Decimal,
            };
            let significant = !matches!(format, NumberColumnFormat::Decimal);
            let invalid = if significant {
                CsvError::SignificantColumnsArguments
            } else {
                CsvError::DecimalColumnsArguments
            };
            let ((columns, digits), data) =
                json_argument_and_data::<(Vec<ColumnReference>, u32)>(input, invalid)?;
            if columns.is_empty()
                || !(u32::from(significant)..=MAX_DECIMAL_PLACES).contains(&digits)
            {
                return Err(invalid);
            }
            input = data;
            Operation::RoundColumns {
                columns,
                digits,
                format,
            }
        }
        "改列名" | "rename-header" => {
            let (names, data) = json_argument_and_data(input, CsvError::RenameArguments)?;
            input = data;
            Operation::RenameColumn(names)
        }
        "批量改列名" | "rename-headers" => {
            let (names, data) = json_argument_and_data(input, CsvError::BulkRenameArguments)?;
            input = data;
            Operation::RenameColumns(names)
        }
        "替换值" | "replace-value" => {
            let ((column, previous, replacement), data) =
                json_argument_and_data::<(ColumnReference, String, String)>(
                    input,
                    CsvError::ReplaceValueArguments,
                )?;
            if replacement.contains('\0') {
                return Err(CsvError::NullReplacementValue);
            }
            input = data;
            Operation::ReplaceValue {
                column,
                previous,
                replacement,
            }
        }
        "映射值" | "map-values" => {
            let ((column, pairs), data) = json_argument_and_data::<(
                ColumnReference,
                Vec<[String; 2]>,
            )>(input, CsvError::MapValueArguments)?;
            if !(1..=MAX_VALUE_MAPPINGS).contains(&pairs.len()) {
                return Err(CsvError::MapValueArguments);
            }
            let mut replacements = HashMap::with_capacity(pairs.len());
            for [previous, replacement] in pairs {
                if replacement.contains('\0') {
                    return Err(CsvError::NullReplacementValue);
                }
                if replacements.insert(previous, replacement).is_some() {
                    return Err(CsvError::MapValueArguments);
                }
            }
            input = data;
            Operation::MapValues {
                column,
                replacements,
            }
        }
        "常量列" | "constant-column" => {
            let (column, data) = json_argument_and_data(input, CsvError::ConstantColumnArguments)?;
            input = data;
            Operation::ConstantColumn(column)
        }
        "加行号" | "number-rows" => {
            let ((name, first), data) =
                json_argument_and_data::<(String, u64)>(input, CsvError::RowNumberArguments)?;
            input = data;
            Operation::RowNumberColumn(name, first)
        }
        "拼接列" | "concat-columns" => {
            let ((name, columns, separator), data) =
                json_argument_and_data::<(String, Vec<ColumnReference>, String)>(
                    input,
                    CsvError::ConcatenatedColumnArguments,
                )?;
            input = data;
            Operation::ConcatenateColumns {
                name,
                columns,
                separator,
            }
        }
        "模板列" | "template-column" => {
            let (arguments, data) =
                json_argument_and_data(input, CsvError::TemplateColumnArguments)?;
            input = data;
            Operation::TemplateColumn(arguments)
        }
        "首个非空" | "coalesce-columns" => {
            let ((name, columns), data) = json_argument_and_data::<(String, Vec<ColumnReference>)>(
                input,
                CsvError::FirstNonemptyColumnArguments,
            )?;
            input = data;
            Operation::FirstNonemptyColumn(name, columns)
        }
        kind @ ("正则提取列" | "regex-capture-column" | "正则展开行" | "regex-expand-rows") =>
        {
            let ((name, source, pattern, group), data) =
                json_argument_and_data::<(String, ColumnReference, String, String)>(
                    input,
                    CsvError::RegexCaptureColumnArguments,
                )?;
            input = data;
            Operation::RegexCaptureColumn {
                name,
                source,
                pattern,
                group,
                expand_rows: matches!(kind, "正则展开行" | "regex-expand-rows"),
            }
        }
        "正则提取多列" | "regex-capture-columns" => {
            let ((source, pattern, columns), data) =
                json_argument_and_data::<(ColumnReference, String, Vec<[String; 2]>)>(
                    input,
                    CsvError::RegexCaptureColumnsArguments,
                )?;
            input = data;
            Operation::RegexCaptureColumns {
                source,
                pattern,
                columns,
            }
        }
        "拆分列" | "split-column" => {
            let ((source, names, separator), data) =
                json_argument_and_data::<(ColumnReference, Vec<String>, String)>(
                    input,
                    CsvError::SplitColumnArguments,
                )?;
            input = data;
            Operation::SplitColumn {
                source,
                names,
                separator,
            }
        }
        "拆成行" | "split-to-rows" => {
            let ((source, separator), data) = json_argument_and_data::<(ColumnReference, String)>(
                input,
                CsvError::SplitRowsArguments,
            )?;
            input = data;
            Operation::SplitRows { source, separator }
        }
        kind
        @ ("日期加天列" | "date-offset-column" | "日期加月列" | "date-month-offset-column") =>
        {
            let ((name, source, amounts), data) =
                json_argument_and_data::<(String, ColumnReference, ColumnReference)>(
                    input,
                    CsvError::DateOffsetColumnArguments,
                )?;
            input = data;
            Operation::DateOffsetColumn {
                name,
                source,
                amounts,
                unit: match kind {
                    "日期加月列" | "date-month-offset-column" => DateOffsetUnit::Months,
                    _ => DateOffsetUnit::Days,
                },
            }
        }
        "日期加工作日列" | "workday-offset-column" => {
            let (arguments, data) = json_argument_and_data::<WorkdayOffsetArguments>(
                input,
                CsvError::WorkdayOffsetColumnArguments,
            )?;
            let (name, source, amounts, exceptions) = match arguments {
                WorkdayOffsetArguments::Standard((name, source, amounts)) => {
                    (name, source, amounts, HashMap::new())
                }
                WorkdayOffsetArguments::WithExceptions((name, source, amounts, dates)) => (
                    name,
                    source,
                    amounts,
                    workday_exceptions_from_dates(dates)
                        .map_err(|_| CsvError::WorkdayOffsetExceptions)?,
                ),
            };
            input = data;
            Operation::DateOffsetColumn {
                name,
                source,
                amounts,
                unit: DateOffsetUnit::Workdays(exceptions),
            }
        }
        kind @ ("长度列" | "length-column" | "截取列" | "slice-column") => {
            let slice = matches!(kind, "截取列" | "slice-column");
            let invalid = if slice {
                CsvError::TextSliceColumnArguments
            } else {
                CsvError::TextLengthColumnArguments
            };
            let ((name, source, argument), data) =
                json_argument_and_data::<(String, ColumnReference, String)>(input, invalid)?;
            if name.trim().is_empty() || name.contains('\0') {
                return Err(invalid);
            }
            let computation = if slice {
                TextColumnComputation::Slice(
                    GraphemeRange::parse(argument.trim()).map_err(|_| invalid)?,
                )
            } else {
                TextColumnComputation::Length(TextLengthUnit::parse(&argument).ok_or(invalid)?)
            };
            input = data;
            Operation::TextColumn(name, source, computation)
        }
        kind @ ("左填充列" | "left-pad-column" | "右填充列" | "right-pad-column") => {
            let ((name, source, width, fill), data) =
                json_argument_and_data::<(String, ColumnReference, usize, char)>(
                    input,
                    CsvError::TextPadColumnArguments,
                )?;
            if name.trim().is_empty()
                || name.contains('\0')
                || width > MAX_PADDING_WIDTH
                || fill == '\0'
            {
                return Err(CsvError::TextPadColumnArguments);
            }
            input = data;
            let computation = TextColumnComputation::Pad {
                width,
                fill,
                left: matches!(kind, "左填充列" | "left-pad-column"),
            };
            Operation::TextColumn(name, source, computation)
        }
        "替换列" | "replace-column" => {
            let ((name, source, needle, replacement), data) =
                json_argument_and_data::<(String, ColumnReference, String, String)>(
                    input,
                    CsvError::TextReplaceColumnArguments,
                )?;
            if name.trim().is_empty()
                || needle.is_empty()
                || [&name, &needle, &replacement]
                    .iter()
                    .any(|text| text.contains('\0'))
            {
                return Err(CsvError::TextReplaceColumnArguments);
            }
            input = data;
            Operation::TextColumn(
                name,
                source,
                TextColumnComputation::Replace(needle, replacement),
            )
        }
        "正则计数列" | "regex-count-column" => {
            let ((name, source, pattern), data) =
                json_argument_and_data::<(String, ColumnReference, String)>(
                    input,
                    CsvError::RegexCountColumnArguments,
                )?;
            if name.trim().is_empty() || name.contains('\0') {
                return Err(CsvError::RegexCountColumnArguments);
            }
            let pattern = compile_regex(&pattern).map_err(CsvError::Regex)?;
            input = data;
            Operation::TextColumn(name, source, TextColumnComputation::RegexCount(pattern))
        }
        "正则替换列" | "regex-replace-column" => {
            let ((name, source, pattern, replacement), data) =
                json_argument_and_data::<(String, ColumnReference, String, String)>(
                    input,
                    CsvError::RegexReplaceColumnArguments,
                )?;
            if name.trim().is_empty()
                || [&name, &pattern, &replacement]
                    .iter()
                    .any(|text| text.contains('\0'))
            {
                return Err(CsvError::RegexReplaceColumnArguments);
            }
            let pattern = compile_regex(&pattern).map_err(CsvError::Regex)?;
            input = data;
            Operation::TextColumn(
                name,
                source,
                TextColumnComputation::RegexReplace(pattern, replacement),
            )
        }
        "url参数列" | "url-parameter-column" => {
            let ((name, source, key), data) =
                json_argument_and_data::<(String, ColumnReference, String)>(
                    input,
                    CsvError::UrlParameterColumnArguments,
                )?;
            if name.trim().is_empty() || name.contains('\0') {
                return Err(CsvError::UrlParameterColumnArguments);
            }
            input = data;
            Operation::UrlParameterColumn(name, source, key)
        }
        kind @ ("json取值列"
        | "json-pointer-column"
        | "json类型列"
        | "json-type-column"
        | "json长度列"
        | "json-length-column"
        | "json存在列"
        | "json-exists-column"
        | "json展开行"
        | "json-expand-rows") => {
            let ((name, source, pointer), data) =
                json_argument_and_data::<(String, ColumnReference, String)>(
                    input,
                    CsvError::JsonPointerColumnArguments,
                )?;
            if name.trim().is_empty() || name.contains('\0') {
                return Err(CsvError::JsonPointerColumnArguments);
            }
            let keys = json_pointer_tokens(&pointer).map_err(CsvError::JsonPointerColumn)?;
            input = data;
            match kind {
                "json展开行" | "json-expand-rows" => {
                    Operation::JsonExpandRows(name, source, keys)
                }
                _ => {
                    let content = match kind {
                        "json类型列" | "json-type-column" => JsonCellContent::Type,
                        "json长度列" | "json-length-column" => JsonCellContent::Length,
                        "json存在列" | "json-exists-column" => JsonCellContent::Exists,
                        _ => JsonCellContent::Value,
                    };
                    Operation::JsonPointerColumn(name, source, keys, content)
                }
            }
        }
        "json通配列" | "json-wildcard-column" => {
            let invalid = CsvError::JsonWildcardColumnArguments;
            let ((name, source, segments), data) =
                json_argument_and_data::<(String, ColumnReference, Vec<Option<String>>)>(
                    input, invalid,
                )?;
            if name.trim().is_empty() || name.contains('\0') {
                return Err(invalid);
            }
            input = data;
            Operation::JsonWildcardColumn(name, source, segments)
        }
        kind @ ("表格序号列"
        | "spreadsheet-serial-column"
        | "序号转日期列"
        | "serial-date-column"
        | "表格时刻序号列"
        | "spreadsheet-time-serial-column"
        | "序号转日期时刻列"
        | "serial-datetime-column") => {
            let invalid = CsvError::SpreadsheetDateColumnArguments;
            let ((name, source, year), data) =
                json_argument_and_data::<(String, ColumnReference, u16)>(input, invalid)?;
            if name.trim().is_empty() || name.contains('\0') {
                return Err(invalid);
            }
            let system = SpreadsheetDateSystem::from_year(year).ok_or(invalid)?;
            let mode = match kind {
                "序号转日期列" | "serial-date-column" => {
                    SpreadsheetColumnTransform::SerialToDate
                }
                "表格时刻序号列" | "spreadsheet-time-serial-column" => {
                    SpreadsheetColumnTransform::TimeToSerial
                }
                "序号转日期时刻列" | "serial-datetime-column" => {
                    SpreadsheetColumnTransform::SerialToTime
                }
                _ => SpreadsheetColumnTransform::DateToSerial,
            };
            input = data;
            Operation::SpreadsheetDateColumn(name, source, system, mode)
        }
        kind @ ("日期格式列" | "date-format-column" | "日期解析列" | "date-parse-column") =>
        {
            let ((name, source, pattern), data) =
                json_argument_and_data::<(String, ColumnReference, String)>(
                    input,
                    CsvError::DateFormatColumnArguments,
                )?;
            input = data;
            Operation::DatePatternColumn {
                name,
                source,
                pattern,
                parse_input: matches!(kind, "日期解析列" | "date-parse-column"),
            }
        }
        "离群列" | "outlier-column" => {
            let ((name, source), data) = json_argument_and_data::<(String, ColumnReference)>(
                input,
                CsvError::OutlierColumnArguments,
            )?;
            input = data;
            Operation::OutlierColumn(name, source)
        }
        "数值比较列" | "numeric-comparison-column" => {
            let ((name, left, right, operator), data) =
                json_argument_and_data::<(String, ColumnReference, ColumnReference, String)>(
                    input,
                    CsvError::NumericComparisonColumnArguments,
                )?;
            let comparison = NumberComparison::parse(operator.trim())
                .ok_or(CsvError::NumericComparisonColumnArguments)?;
            input = data;
            Operation::NumericComparisonColumn(name, left, right, comparison)
        }
        "日期差列" | "date-difference-column" => {
            let ((name, start, end), data) =
                json_argument_and_data::<(String, ColumnReference, ColumnReference)>(
                    input,
                    CsvError::DateDifferenceColumnArguments,
                )?;
            input = data;
            Operation::DateDifferenceColumn(name, start, end)
        }
        "时长数值列" | "duration-number-column" => {
            let invalid = CsvError::DurationNumberColumnArguments;
            let ((name, source, target), data) =
                json_argument_and_data::<(String, ColumnReference, String)>(input, invalid)?;
            if name.trim().is_empty() || name.contains('\0') {
                return Err(invalid);
            }
            let convert = exact_duration_converter(&target).map_err(|_| invalid)?;
            return append_transformed_csv_column(data, &name, source, |row, value| {
                convert(value).map_err(|error| CsvError::DataRowValue {
                    row,
                    cause: error.message(),
                })
            });
        }
        "数值对齐列" | "numeric-align-column" => {
            let invalid = CsvError::NumericAlignmentColumnArguments;
            let ((name, source, step, direction), data) =
                json_argument_and_data::<(String, ColumnReference, String, String)>(
                    input, invalid,
                )?;
            if name.trim().is_empty() || name.contains('\0') {
                return Err(invalid);
            }
            let convert = fraction_alignment_converter(&step, &direction).map_err(|_| invalid)?;
            return append_transformed_csv_column(data, &name, source, |row, value| {
                convert(value).map_err(|error| CsvError::DataRowValue {
                    row,
                    cause: error.message(),
                })
            });
        }
        "时间戳对齐列" | "timestamp-align-column" => {
            let invalid = CsvError::TimestampAlignmentColumnArguments;
            let ((name, source, step, direction), data) =
                json_argument_and_data::<(String, ColumnReference, String, String)>(
                    input, invalid,
                )?;
            if name.trim().is_empty() || name.contains('\0') {
                return Err(invalid);
            }
            let convert = timestamp_alignment_converter(&step, &direction).map_err(|_| invalid)?;
            return append_transformed_csv_column(data, &name, source, |row, value| {
                convert(value).map_err(|error| CsvError::DataRowValue {
                    row,
                    cause: error.message(),
                })
            });
        }
        "时区换算列" | "timestamp-zone-column" => {
            let invalid = CsvError::TimestampZoneColumnArguments;
            let ((name, source, target), data) =
                json_argument_and_data::<(String, ColumnReference, String)>(input, invalid)?;
            if name.trim().is_empty() || name.contains('\0') {
                return Err(invalid);
            }
            let convert = timestamp_zone_converter(&target).map_err(|_| invalid)?;
            return append_transformed_csv_column(data, &name, source, |row, value| {
                convert(value).map_err(|error| CsvError::DataRowValue {
                    row,
                    cause: error.message(),
                })
            });
        }
        kind @ ("网段归属列" | "cidr-column" | "网段名称列" | "cidr-name-column") => {
            return append_cidr_column(input, matches!(kind, "网段名称列" | "cidr-name-column"));
        }
        "时间戳差列" | "timestamp-difference-column" => {
            let ((name, start, end), data) =
                json_argument_and_data::<(String, ColumnReference, ColumnReference)>(
                    input,
                    CsvError::TimestampDifferenceColumnArguments,
                )?;
            input = data;
            Operation::TimestampDifferenceColumn(name, start, end)
        }
        "组内时间戳差列" | "group-timestamp-difference-column" => {
            return append_group_timestamp_offsets(input, GroupTimestampReference::Previous)
        }
        "组内时间戳偏移列" | "group-timestamp-offset-column" => {
            return append_group_timestamp_offsets(input, GroupTimestampReference::First)
        }
        "会话编号列" | "timestamp-session-column" => return append_timestamp_sessions(input),
        "时间窗口计数列" | "timestamp-window-count-column" => {
            return append_timestamp_window(input, TimestampWindowStatistic::Count)
        }
        "时间窗口合计列" | "timestamp-window-sum-column" => {
            return append_timestamp_window(input, TimestampWindowStatistic::Sum)
        }
        "时间窗口平均列" | "timestamp-window-average-column" => {
            return append_timestamp_window(input, TimestampWindowStatistic::Average)
        }
        "时刻差列" | "clock-difference-column" => {
            let ((name, start, end), data) =
                json_argument_and_data::<(String, ColumnReference, ColumnReference)>(
                    input,
                    CsvError::ClockDifferenceColumnArguments,
                )?;
            input = data;
            Operation::ClockDifferenceColumn(name, start, end)
        }
        kind @ ("进制列" | "radix-column" | "进制小数列" | "radix-fraction-column") => {
            let ((name, source, from, to), data) =
                json_argument_and_data::<(String, ColumnReference, u32, u32)>(
                    input,
                    CsvError::RadixColumnArguments,
                )?;
            if name.trim().is_empty()
                || name.contains('\0')
                || [from, to].iter().any(|base| !(2..=36).contains(base))
            {
                return Err(CsvError::RadixColumnArguments);
            }
            input = data;
            Operation::RadixColumn {
                name,
                source,
                from,
                to,
                fractional: matches!(kind, "进制小数列" | "radix-fraction-column"),
            }
        }
        kind @ ("数据换算列" | "convert-data-column" | "单位换算列" | "convert-unit-column") =>
        {
            let exact_data = matches!(kind, "数据换算列" | "convert-data-column");
            let (arguments, data) = json_argument_and_data::<UnitColumnArguments>(
                input,
                CsvError::UnitConversionColumnArguments,
            )?;
            let (name, source, target, source_unit) = match arguments {
                UnitColumnArguments::CellUnits(name, source, target) => {
                    (name, source, target, None)
                }
                UnitColumnArguments::FixedSource(name, source, unit, target) if !exact_data => {
                    (name, source, target, Some(unit))
                }
                UnitColumnArguments::FixedSource(..) => {
                    return Err(CsvError::UnitConversionColumnArguments)
                }
            };
            input = data;
            Operation::ConvertUnitColumn {
                name,
                source,
                target,
                source_unit,
                exact_data,
            }
        }
        kind @ ("累计合计列"
        | "running-sum-column"
        | "累计平均列"
        | "running-average-column"
        | "差分列"
        | "difference-column"
        | "变化率列"
        | "relative-change-column") => {
            let ((name, source, groups), data) =
                json_argument_and_data::<(String, ColumnReference, Vec<ColumnReference>)>(
                    input,
                    CsvError::GroupedColumnArguments,
                )?;
            input = data;
            Operation::RunningColumn {
                name,
                source,
                groups,
                computation: match kind {
                    "差分列" | "difference-column" => RunningColumnComputation::Difference,
                    "变化率列" | "relative-change-column" => {
                        RunningColumnComputation::RelativeChange
                    }
                    "累计平均列" | "running-average-column" => {
                        RunningColumnComputation::Average
                    }
                    _ => RunningColumnComputation::Sum,
                },
            }
        }
        kind @ ("移动平均列" | "moving-average-column" | "移动合计列" | "moving-sum-column") =>
        {
            let ((name, source, groups, width), data) =
                json_argument_and_data::<(String, ColumnReference, Vec<ColumnReference>, usize)>(
                    input,
                    CsvError::MovingColumnArguments,
                )?;
            if !(1..=MAX_VALUES).contains(&width) || name.trim().is_empty() || name.contains('\0') {
                return Err(CsvError::MovingColumnArguments);
            }
            input = data;
            Operation::RunningColumn {
                name,
                source,
                groups,
                computation: RunningColumnComputation::Moving {
                    width,
                    average: matches!(kind, "移动平均列" | "moving-average-column"),
                },
            }
        }
        "移动分位数列" | "moving-quantile-column" => {
            let ((name, source, groups, width, probability), data) =
                json_argument_and_data::<(
                    String,
                    ColumnReference,
                    Vec<ColumnReference>,
                    usize,
                    String,
                )>(input, CsvError::MovingQuantileColumnArguments)?;
            if !(1..=MAX_VALUES).contains(&width) || name.trim().is_empty() || name.contains('\0') {
                return Err(CsvError::MovingQuantileColumnArguments);
            }
            let probability =
                parse_probability(probability.trim()).map_err(CsvError::Statistics)?;
            input = data;
            Operation::RunningColumn {
                name,
                source,
                groups,
                computation: RunningColumnComputation::MovingStatistic {
                    width,
                    statistic: MovingColumnStatistic::Quantile(probability),
                },
            }
        }
        kind @ ("移动样本方差列"
        | "moving-sample-variance-column"
        | "移动总体方差列"
        | "moving-population-variance-column"
        | "移动样本标准差列"
        | "moving-sample-stdev-column"
        | "移动总体标准差列"
        | "moving-population-stdev-column") => {
            let ((name, source, groups, width), data) =
                json_argument_and_data::<(String, ColumnReference, Vec<ColumnReference>, usize)>(
                    input,
                    CsvError::MovingDispersionColumnArguments,
                )?;
            let sample = matches!(
                kind,
                "移动样本方差列"
                    | "moving-sample-variance-column"
                    | "移动样本标准差列"
                    | "moving-sample-stdev-column"
            );
            let root = matches!(
                kind,
                "移动样本标准差列"
                    | "moving-sample-stdev-column"
                    | "移动总体标准差列"
                    | "moving-population-stdev-column"
            );
            if !((1 + usize::from(sample))..=MAX_VALUES).contains(&width)
                || name.trim().is_empty()
                || name.contains('\0')
            {
                return Err(CsvError::MovingDispersionColumnArguments);
            }
            input = data;
            Operation::RunningColumn {
                name,
                source,
                groups,
                computation: RunningColumnComputation::MovingStatistic {
                    width,
                    statistic: MovingColumnStatistic::Variance { sample, root },
                },
            }
        }
        kind @ ("排名列"
        | "rank-column"
        | "升序排名列"
        | "ascending-rank-column"
        | "密集排名列"
        | "dense-rank-column"
        | "升序密集排名列"
        | "ascending-dense-rank-column"
        | "百分位排名列"
        | "percent-rank-column"
        | "降序百分位排名列"
        | "descending-percent-rank-column"
        | "累积分布列"
        | "cume-dist-column"
        | "降序累积分布列"
        | "descending-cume-dist-column"
        | "占比列"
        | "share-column") => {
            let ((name, source, groups), data) =
                json_argument_and_data::<(String, ColumnReference, Vec<ColumnReference>)>(
                    input,
                    CsvError::GroupedColumnArguments,
                )?;
            input = data;
            if matches!(kind, "占比列" | "share-column") {
                Operation::ShareColumn {
                    name,
                    source,
                    groups,
                }
            } else {
                let (ascending, ranking) = match kind {
                    "升序排名列" | "ascending-rank-column" => (true, CsvRanking::Competition),
                    "密集排名列" | "dense-rank-column" => (false, CsvRanking::Dense),
                    "升序密集排名列" | "ascending-dense-rank-column" => {
                        (true, CsvRanking::Dense)
                    }
                    "百分位排名列" | "percent-rank-column" => (true, CsvRanking::PercentRank),
                    "降序百分位排名列" | "descending-percent-rank-column" => {
                        (false, CsvRanking::PercentRank)
                    }
                    "累积分布列" | "cume-dist-column" => {
                        (true, CsvRanking::CumulativeDistribution)
                    }
                    "降序累积分布列" | "descending-cume-dist-column" => {
                        (false, CsvRanking::CumulativeDistribution)
                    }
                    _ => (false, CsvRanking::Competition),
                };
                Operation::RankColumn {
                    name,
                    source,
                    groups,
                    ascending,
                    ranking,
                }
            }
        }
        kind @ ("前值列" | "lag-column" | "后值列" | "lead-column") => {
            let ((name, source, distance, groups), data) =
                json_argument_and_data::<(String, ColumnReference, usize, Vec<ColumnReference>)>(
                    input,
                    CsvError::ShiftedColumnArguments,
                )?;
            if distance == 0 {
                return Err(CsvError::ShiftedColumnArguments);
            }
            input = data;
            Operation::ShiftedColumn {
                name,
                source,
                distance,
                groups,
                following: matches!(kind, "后值列" | "lead-column"),
            }
        }
        kind @ ("组内计数列" | "group-count-column" | "组内序号列" | "group-row-number-column") =>
        {
            let ((name, groups), data) = json_argument_and_data::<(String, Vec<ColumnReference>)>(
                input,
                CsvError::GroupCountColumnArguments,
            )?;
            input = data;
            Operation::GroupCountColumn {
                name,
                groups,
                running: matches!(kind, "组内序号列" | "group-row-number-column"),
            }
        }
        kind @ ("连续分组列"
        | "run-id-column"
        | "段内序号列"
        | "run-row-number-column"
        | "段内计数列"
        | "run-length-column") => {
            let ((name, groups), data) = json_argument_and_data::<(String, Vec<ColumnReference>)>(
                input,
                CsvError::GroupCountColumnArguments,
            )?;
            input = data;
            Operation::ContiguousGroupColumn {
                name,
                groups,
                measurement: match kind {
                    "段内序号列" | "run-row-number-column" => ContiguousMeasurement::Position,
                    "段内计数列" | "run-length-column" => ContiguousMeasurement::Length,
                    _ => ContiguousMeasurement::Group,
                },
            }
        }
        "计算列" | "compute-column" => {
            let ((name, left, operator, right), data) =
                json_argument_and_data::<(String, ColumnReference, String, ColumnReference)>(
                    input,
                    CsvError::ComputedColumnArguments,
                )?;
            input = data;
            Operation::ComputeColumn {
                name,
                left,
                right: Some(right),
                arithmetic: ColumnArithmetic::parse(&operator)?,
            }
        }
        "公式列" | "formula-column" => {
            let (arguments, data) = json_argument_and_data::<FormulaColumnArguments>(
                input,
                CsvError::FormulaColumnArguments,
            )?;
            let (name, left, right, expression) = arguments.into_parts();
            if name.trim().is_empty() || name.contains('\0') || expression.trim().is_empty() {
                return Err(CsvError::FormulaColumnArguments);
            }
            input = data;
            Operation::ComputeColumn {
                name,
                left,
                right,
                arithmetic: ColumnArithmetic::Formula(normalize(&expression)),
            }
        }
        "函数列" | "function-column" => {
            let (arguments, data) = json_argument_and_data::<FormulaColumnArguments>(
                input,
                CsvError::FunctionColumnArguments,
            )?;
            let (name, left, right, expression) = arguments.into_parts();
            if name.trim().is_empty()
                || name.contains('\0')
                || expression.trim().is_empty()
                || expression.chars().take(QUERY_LIMIT + 1).count() > QUERY_LIMIT
            {
                return Err(CsvError::FunctionColumnArguments);
            }
            input = data;
            Operation::FunctionColumn {
                name,
                left,
                right,
                expression,
            }
        }
        "按列去重"
        | "deduplicate-columns"
        | "数值去重"
        | "deduplicate-numeric"
        | "按列去重留末"
        | "deduplicate-columns-last"
        | "数值去重留末"
        | "deduplicate-numeric-last" => {
            let (columns, data) =
                column_reference_arguments(input, CsvError::NamedColumnArguments)?;
            input = data;
            let (numeric, keep_last) = match operation.as_str() {
                "数值去重留末" | "deduplicate-numeric-last" => (true, true),
                "按列去重留末" | "deduplicate-columns-last" => (false, true),
                "数值去重" | "deduplicate-numeric" => (true, false),
                _ => (false, false),
            };
            Operation::DeduplicateByColumns {
                columns,
                numeric,
                keep_last,
            }
        }
        "查重复" | "duplicates-by-columns" | "数值查重复" | "duplicates-numeric" => {
            let (columns, data) =
                column_reference_arguments(input, CsvError::NamedColumnArguments)?;
            input = data;
            if matches!(operation.as_str(), "数值查重复" | "duplicates-numeric") {
                Operation::DuplicatesByNumbers(columns)
            } else {
                Operation::DuplicatesByColumns(columns)
            }
        }
        "分组计数" | "group-count" => {
            let (columns, data) = column_reference_arguments(input, CsvError::GroupingColumns)?;
            input = data;
            Operation::CountByColumns(columns)
        }
        "分组频数" | "group-frequency" => {
            let (columns, data) = column_reference_arguments(input, CsvError::GroupingColumns)?;
            input = data;
            Operation::FrequencyByColumns(columns)
        }
        "分组合计" | "group-sum" | "分组平均" | "group-average" | "分组最小" | "group-min"
        | "分组最大" | "group-max" | "分组极差" | "group-range" => {
            let (columns, rest) = column_reference_arguments(input, CsvError::GroupingColumns)?;
            let (amount, data) = single_column_argument(rest, CsvError::AggregateColumn)?;
            input = data;
            Operation::AggregateByColumns {
                columns,
                amount,
                aggregation: match operation.as_str() {
                    "分组平均" | "group-average" => ColumnAggregation::Average,
                    "分组最小" | "group-min" => ColumnAggregation::Minimum,
                    "分组最大" | "group-max" => ColumnAggregation::Maximum,
                    "分组极差" | "group-range" => ColumnAggregation::Range,
                    _ => ColumnAggregation::Sum,
                },
            }
        }
        "分组中位数" | "group-median" => {
            let (columns, rest) = column_reference_arguments(input, CsvError::GroupingColumns)?;
            let (amount, data) = single_column_argument(rest, CsvError::AggregateColumn)?;
            input = data;
            Operation::GroupMedian { columns, amount }
        }
        "分组时长合计" | "group-duration-sum" => {
            let (columns, rest) = column_reference_arguments(input, CsvError::GroupingColumns)?;
            let (amount, data) = single_column_argument(rest, CsvError::AggregateColumn)?;
            let rows = read_table_for_preview(data, ',')?;
            return sum_durations_by_columns(&rows, &columns, amount);
        }
        "分组数值摘要" | "group-numeric-profile" => {
            let (columns, rest) = column_reference_arguments(input, CsvError::GroupingColumns)?;
            let (amount, data) = single_column_argument(rest, CsvError::AggregateColumn)?;
            input = data;
            Operation::GroupNumericProfile { columns, amount }
        }
        "分组拟合" | "group-fit" => {
            let ((columns, coordinates), data) = json_argument_and_data::<(
                Vec<ColumnReference>,
                [ColumnReference; 2],
            )>(input, CsvError::GroupFitColumns)?;
            if columns.is_empty() {
                return Err(CsvError::GroupingColumns);
            }
            input = data;
            Operation::GroupFit {
                columns,
                coordinates,
            }
        }
        label @ ("组内最高" | "group-top" | "组内最低" | "group-bottom") => {
            let ((columns, amount, limit), data) =
                json_argument_and_data::<(Vec<ColumnReference>, ColumnReference, usize)>(
                    input,
                    CsvError::GroupSelectionArguments,
                )?;
            if columns.is_empty() || limit == 0 {
                return Err(CsvError::GroupSelectionArguments);
            }
            input = data;
            Operation::SelectGroupExtrema {
                columns,
                amount,
                limit,
                highest: matches!(label, "组内最高" | "group-top"),
            }
        }
        "列统计" | "column-stats" | "列总体" | "column-population" => {
            let (column, data) = single_column_argument(input, CsvError::StatisticsColumn)?;
            input = data;
            Operation::ColumnStatistics {
                column,
                sample: matches!(operation.as_str(), "列统计" | "column-stats"),
            }
        }
        "列分箱" | "column-bins" | "列分箱图" | "column-bin-plot" => {
            let (column, edges, data) = column_bin_arguments(input)?;
            input = data;
            Operation::ColumnBins(column, edges)
        }
        "列箱线摘要" | "column-box-summary" | "列箱线图" | "column-box-plot" => {
            let (column, data) = single_column_argument(input, CsvError::ObservationColumn)?;
            input = data;
            Operation::ColumnBoxSummary(column)
        }
        "列累计分布" | "column-ecdf" | "列累计分布图" | "column-ecdf-plot" => {
            let (column, data) = single_column_argument(input, CsvError::ObservationColumn)?;
            input = data;
            Operation::ColumnCumulativeDistribution(column)
        }
        "数值列摘要" | "numeric-profile" => {
            let (columns, data) =
                column_reference_arguments(input, CsvError::NamedColumnArguments)?;
            input = data;
            Operation::NumericProfile(columns)
        }
        "相关矩阵"
        | "correlation-matrix"
        | "协方差矩阵"
        | "covariance-matrix"
        | "总体协方差矩阵"
        | "population-covariance-matrix"
        | "秩相关矩阵"
        | "spearman-matrix"
        | "相关图"
        | "correlation-plot"
        | "秩相关图"
        | "spearman-plot" => {
            let (columns, data) = pair_matrix_arguments(input)?;
            input = data;
            let statistic = match operation.as_str() {
                "秩相关矩阵" | "spearman-matrix" | "秩相关图" | "spearman-plot" => {
                    PairMatrixStatistic::Spearman
                }
                "协方差矩阵" | "covariance-matrix" => {
                    PairMatrixStatistic::Covariance { sample: true }
                }
                "总体协方差矩阵" | "population-covariance-matrix" => {
                    PairMatrixStatistic::Covariance { sample: false }
                }
                _ => PairMatrixStatistic::Correlation,
            };
            Operation::PairMatrix { columns, statistic }
        }
        "距离矩阵" | "distance-matrix" => {
            let invalid = CsvError::DistanceMatrixArguments;
            let ((labels, coordinates), data) =
                json_argument_and_data::<(ColumnReference, Vec<ColumnReference>)>(input, invalid)?;
            if !(1..=16).contains(&coordinates.len()) {
                return Err(invalid);
            }
            input = data;
            Operation::DistanceMatrix {
                labels,
                coordinates,
            }
        }
        "最近点" | "nearest-points" => {
            let invalid = CsvError::NearestPointArguments;
            let ((labels, coordinates, target, limit), data) =
                json_argument_and_data::<(ColumnReference, Vec<ColumnReference>, String, usize)>(
                    input, invalid,
                )?;
            if !(1..=16).contains(&coordinates.len())
                || !(1..=31).contains(&limit)
                || target.trim().is_empty()
            {
                return Err(invalid);
            }
            input = data;
            Operation::NearestPoints {
                labels,
                coordinates,
                target,
                limit,
            }
        }
        "透视合计" | "pivot-sum" | "透视计数" | "pivot-count" | "透视原值" | "pivot-values" =>
        {
            let kind = match operation.as_str() {
                "透视计数" | "pivot-count" => PivotKind::Count,
                "透视原值" | "pivot-values" => PivotKind::Values,
                _ => PivotKind::Sum,
            };
            let (column_count, invalid) = kind.column_requirements();
            let (columns, data) = json_argument_and_data::<Vec<ColumnReference>>(input, invalid)?;
            if columns.len() != column_count {
                return Err(invalid);
            }
            input = data;
            Operation::Pivot(columns, kind)
        }
        "逆透视" | "unpivot" => {
            let ((columns, label, value), data) =
                json_argument_and_data(input, CsvError::UnpivotArguments)?;
            input = data;
            Operation::Unpivot(columns, [label, value])
        }
        _ => return Err(CsvError::UnknownOperation),
    };
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    if input.is_empty() {
        return Err(CsvError::MissingInput);
    }

    let delimiter = match kind {
        Operation::ConvertDelimiter { from, .. } => from,
        _ => ',',
    };
    let rows = if matches!(kind, Operation::PadRows | Operation::DropEmptyRecords) {
        read_delimited_rows(input, delimiter, false, None).map_err(|error| error.kind)?
    } else {
        read_rows(input, delimiter)?
    };
    match kind {
        Operation::PadRows => pad_rows(rows),
        Operation::JsonRows => serde_json::to_string(&rows).map_err(|_| CsvError::Serialization),
        Operation::JsonObjects => json_objects(&rows, None),
        Operation::JsonNumericObjects(columns) => json_objects(&rows, Some(&columns)),
        Operation::Markdown(alignments) => markdown_table(&rows, alignments.as_deref()),
        Operation::ConvertDelimiter { to, .. } => Ok(write_rows(&rows, to)),
        Operation::TrimColumns(columns) => trim_column_values(rows, &columns),
        Operation::UnicodeNormalizedColumn(name, source, form) => {
            let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
            let source = source.resolve(header)?;
            validate_column_width(&[source], header.len())?;
            write_appended_column(header, records, &name, |row| {
                Ok(form.normalize(&row[source]))
            })
        }
        Operation::CompareTextColumns(name, first, second, comparison) => {
            let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
            let first = first.resolve(header)?;
            let second = second.resolve(header)?;
            validate_column_width(&[first, second], header.len())?;
            let mut remaining_work = MAX_TEXT_COMPARISON_WORK;
            let mut row_number = 0;
            write_appended_column(header, records, &name, |row| {
                row_number += 1;
                let (first, second) = (&row[first], &row[second]);
                match comparison {
                    TextComparison::EditDistance => {
                        grapheme_edit_distance(first, second, &mut remaining_work)
                            .map(|distance| Cow::Owned(distance.to_string()))
                    }
                    TextComparison::CommonSubstring => {
                        grapheme_common_substring(first, second, &mut remaining_work)
                            .map(Cow::Borrowed)
                    }
                }
                .map_err(|error| CsvError::DataRowValue {
                    row: row_number,
                    cause: error.message(),
                })
            })
        }
        Operation::FillEmptyCells(columns, direction) => {
            fill_empty_csv_cells(&rows, &columns, direction)
        }
        Operation::InterpolateEmptyCells(column, groups) => {
            interpolate_empty_csv_cells(&rows, column, &groups)
        }
        Operation::CountEmptyCells(columns) => count_empty_cells(&rows, &columns),
        Operation::CheckHeader(expected) => check_header(&rows, &expected),
        Operation::CheckColumns(columns, validation) => check_columns(&rows, &columns, &validation),
        Operation::CheckColumnRules(rules, output) => check_column_rules(&rows, &rules, output),
        Operation::RoundColumns {
            columns,
            digits,
            format,
        } => round_csv_columns(&rows, &columns, digits, format),
        Operation::DropEmptyRecords => drop_empty_csv_records(&rows),
        Operation::DropEmptyColumns => drop_empty_csv_columns(&rows),
        Operation::FilterMissingRows { columns, missing } => {
            filter_missing_rows(&rows, &columns, missing)
        }
        Operation::SelectRows {
            ranges,
            keep_selected,
        } => select_data_rows(&rows, &ranges, keep_selected),
        Operation::RenameColumn(names) => rename_column(rows, names),
        Operation::RenameColumns(names) => rename_columns(rows, names),
        Operation::ReplaceValue {
            column,
            previous,
            replacement,
        } => replace_column_values(rows, column, &previous, &replacement),
        Operation::MapValues {
            column,
            replacements,
        } => map_column_values(&rows, column, &replacements),
        Operation::ConstantColumn([name, value]) => add_constant_column(&rows, &name, &value),
        Operation::RowNumberColumn(name, first) => number_csv_rows(&rows, &name, first),
        Operation::ConcatenateColumns {
            name,
            columns,
            separator,
        } => concatenate_columns(&rows, &name, &columns, &separator),
        Operation::TemplateColumn([name, template]) => template_column(&rows, &name, &template),
        Operation::FirstNonemptyColumn(name, columns) => {
            first_nonempty_csv_column(&rows, &name, &columns)
        }
        Operation::RegexCaptureColumn {
            name,
            source,
            pattern,
            group,
            expand_rows,
        } => regex_capture_column(&rows, &name, source, &pattern, &group, expand_rows),
        Operation::RegexCaptureColumns {
            source,
            pattern,
            columns,
        } => regex_capture_columns(&rows, source, &pattern, &columns),
        Operation::SplitColumn {
            source,
            names,
            separator,
        } => split_column(&rows, source, &names, &separator),
        Operation::SplitRows { source, separator } => split_csv_rows(&rows, source, &separator),
        Operation::ComputeColumn {
            name,
            left,
            right,
            arithmetic,
        } => compute_column(rows, name, left, right, arithmetic),
        Operation::FunctionColumn {
            name,
            left,
            right,
            expression,
        } => {
            if rows.len().saturating_sub(1) > MAX_FUNCTION_RECORDS {
                return Err(CsvError::FunctionColumnLimit);
            }
            let sample = |x: &str, y: Option<&str>| {
                evaluate_csv_function(&expression, x, y)?
                    .map(format_result)
                    .transpose()
                    .map(Option::unwrap_or_default)
                    .map_err(|error| error.message())
            };
            match right {
                Some(right) => append_from_two_columns(
                    &rows,
                    &name,
                    left,
                    right,
                    CsvError::FunctionColumnArguments,
                    |x, y| sample(x, Some(y)),
                ),
                None => append_from_one_column(&rows, &name, left, |x| sample(x, None)),
            }
        }
        Operation::ConvertUnitColumn {
            name,
            source,
            target,
            source_unit,
            exact_data,
        } => {
            if exact_data {
                append_converted_unit_column(&rows, &name, source, &target, exact_data_converter)
            } else {
                append_converted_unit_column(&rows, &name, source, &target, |target| {
                    quantity_number_converter(target, source_unit.as_deref())
                })
            }
        }
        Operation::OutlierColumn(name, source) => flag_outlier_column(&rows, &name, source),
        Operation::NumericComparisonColumn(name, left, right, comparison) => {
            append_from_two_columns(
                &rows,
                &name,
                left,
                right,
                CsvError::NumericComparisonColumnArguments,
                |left, right| {
                    let parse = |cell| parse_fraction_literal(cell).map_err(FractionError::message);
                    let order = parse(left)?.cmp(&parse(right)?);
                    Ok(comparison.accepts(order).to_string())
                },
            )
        }
        Operation::DateDifferenceColumn(name, start, end) => append_from_two_columns(
            &rows,
            &name,
            start,
            end,
            CsvError::DateDifferenceColumnArguments,
            |start, end| {
                let parse_date = |date| parse_complete_date(date).map_err(|error| error.message());
                let days = parse_date(end)?
                    .signed_duration_since(parse_date(start)?)
                    .num_days();
                Ok(days.to_string())
            },
        ),
        Operation::TimestampDifferenceColumn(name, start, end) => append_from_two_columns(
            &rows,
            &name,
            start,
            end,
            CsvError::TimestampDifferenceColumnArguments,
            |start, end| timestamp_clock_difference(start, end).map_err(|error| error.message()),
        ),
        Operation::ClockDifferenceColumn(name, start, end) => append_from_two_columns(
            &rows,
            &name,
            start,
            end,
            CsvError::ClockDifferenceColumnArguments,
            |start, end| {
                elapsed_clock_interval(&normalize(start), &normalize(end))
                    .map(|nanos| format_clock_duration(nanos, ""))
                    .map_err(|error| error.message())
            },
        ),
        Operation::DateOffsetColumn {
            name,
            source,
            amounts,
            unit,
        } => date_offset_column(&rows, &name, source, amounts, unit),
        Operation::RadixColumn {
            name,
            source,
            from,
            to,
            fractional,
        } => append_from_one_column(&rows, &name, source, |value| {
            let result = if fractional {
                convert_radix_fraction(value, from, to)
            } else {
                convert_radix_integer(value, from, to)
            };
            result.map_err(|error| error.message())
        }),
        Operation::UrlParameterColumn(name, source, key) => {
            append_from_one_column(&rows, &name, source, |address| {
                url_parameter_values(address, &key).map_err(|error| error.message())
            })
        }
        Operation::JsonPointerColumn(name, source, keys, content) => {
            let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
            let source = source.resolve(header)?;
            validate_column_width(&[source], header.len())?;
            let mut row_number = 0;
            write_appended_column(header, records, &name, |row| {
                row_number += 1;
                json_cell_at_pointer(&row[source], &keys, content).map_err(|error| {
                    CsvError::DataRowValue {
                        row: row_number,
                        cause: error.message(),
                    }
                })
            })
        }
        Operation::JsonWildcardColumn(name, source, segments) => {
            append_from_one_column(&rows, &name, source, |value| {
                json_wildcard_values(value, &segments).map_err(|error| error.message())
            })
        }
        Operation::JsonExpandRows(name, source, keys) => {
            let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
            let source = source.resolve(header)?;
            validate_column_width(&[source], header.len())?;
            let mut row_number = 0;
            write_expanded_column(header, records, &name, |row| {
                row_number += 1;
                json_array_cells_at_pointer(&row[source], &keys).map_err(|error| {
                    CsvError::DataRowValue {
                        row: row_number,
                        cause: error.message(),
                    }
                })
            })
        }
        Operation::TextColumn(name, source, computation) => {
            let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
            let source = source.resolve(header)?;
            validate_column_width(&[source], header.len())?;
            let size_error = match &computation {
                TextColumnComputation::RegexCount(_) => {
                    Some(CsvError::RegexCountColumnInputTooLarge)
                }
                TextColumnComputation::RegexReplace(..) => {
                    Some(CsvError::RegexReplaceColumnInputTooLarge)
                }
                _ => None,
            };
            if let Some(error) = size_error.filter(|_| {
                records.iter().map(|row| row[source].len()).sum::<usize>()
                    > MAX_REGEX_COLUMN_INPUT_BYTES
            }) {
                return Err(error);
            }
            write_appended_column(header, records, &name, |row| {
                let cell = &row[source];
                Ok(match &computation {
                    TextColumnComputation::Length(unit) => Cow::Owned(unit.count(cell).to_string()),
                    TextColumnComputation::Slice(range) => Cow::Borrowed(range.slice(cell)),
                    TextColumnComputation::RegexCount(pattern) => {
                        Cow::Owned(pattern.find_iter(cell).count().to_string())
                    }
                    TextColumnComputation::RegexReplace(pattern, replacement) => {
                        Cow::Owned(replace_regex_matches(cell, pattern, replacement).map_err(
                            |error| match error {
                                TextError::ResultTooLarge => CsvError::AddedColumnTooLarge,
                                error => CsvError::Regex(error),
                            },
                        )?)
                    }
                    TextColumnComputation::Replace(needle, replacement) => {
                        replace_literal_matches(cell, needle, replacement).map_err(|error| {
                            match error {
                                TextError::ResultTooLarge => CsvError::AddedColumnTooLarge,
                                _ => CsvError::TextReplaceColumnArguments,
                            }
                        })?
                    }
                    TextColumnComputation::Pad { width, fill, left } => {
                        let missing = width.saturating_sub(TextLengthUnit::CodePoints.count(cell));
                        if missing == 0 {
                            Cow::Borrowed(cell.as_str())
                        } else {
                            let mut padded =
                                String::with_capacity(cell.len() + missing * fill.len_utf8());
                            append_padded_text(&mut padded, cell, *fill, missing, *left);
                            Cow::Owned(padded)
                        }
                    }
                })
            })
        }
        Operation::SpreadsheetDateColumn(name, source, system, mode) => {
            append_from_one_column(&rows, &name, source, |value| {
                let value = normalize(value);
                match mode {
                    SpreadsheetColumnTransform::SerialToDate => {
                        system.date_from_serial(&value).map(|date| date.to_string())
                    }
                    SpreadsheetColumnTransform::DateToSerial => parse_complete_date(&value)
                        .and_then(|date| system.serial_from_date(date))
                        .map(|serial| serial.to_string()),
                    SpreadsheetColumnTransform::TimeToSerial => {
                        system.time_serial_from_text(&value)
                    }
                    SpreadsheetColumnTransform::SerialToTime => system.time_from_serial(&value),
                }
                .map_err(|error| error.message())
            })
        }
        Operation::DatePatternColumn {
            name,
            source,
            pattern,
            parse_input,
        } => date_pattern_column(&rows, &name, source, &pattern, parse_input),
        Operation::RunningColumn {
            name,
            source,
            groups,
            computation,
        } => running_column(&rows, &name, source, &groups, computation),
        Operation::RankColumn {
            name,
            source,
            groups,
            ascending,
            ranking,
        } => rank_column(&rows, &name, source, &groups, ascending, ranking),
        Operation::ShareColumn {
            name,
            source,
            groups,
        } => share_column(&rows, &name, source, &groups),
        Operation::ShiftedColumn {
            name,
            source,
            distance,
            groups,
            following,
        } => shifted_csv_column(&rows, &name, source, distance, &groups, following),
        Operation::GroupCountColumn {
            name,
            groups,
            running,
        } => grouped_count_column(&rows, &name, &groups, running),
        Operation::ContiguousGroupColumn {
            name,
            groups,
            measurement,
        } => contiguous_group_column(&rows, &name, &groups, measurement),
        Operation::DeduplicateRows => {
            let mut seen = HashSet::new();
            Ok(write_rows(rows.iter().filter(|row| seen.insert(*row)), ','))
        }
        Operation::DeduplicateByColumns {
            columns,
            numeric,
            keep_last,
        } => {
            if numeric {
                deduplicate_by_columns(&rows, &columns, keep_last, numeric_column_key)
            } else {
                deduplicate_by_columns(&rows, &columns, keep_last, |row, columns| {
                    Ok(column_key(row, columns))
                })
            }
        }
        Operation::DuplicatesByColumns(columns) => {
            find_duplicate_columns(&rows, &columns, |row, columns| Ok(column_key(row, columns)))
        }
        Operation::DuplicatesByNumbers(columns) => {
            find_duplicate_columns(&rows, &columns, numeric_column_key)
        }
        Operation::CountByColumns(columns) => count_by_columns(&rows, &columns),
        Operation::FrequencyByColumns(columns) => group_frequencies(&rows, &columns)?.csv(),
        Operation::AggregateByColumns {
            columns,
            amount,
            aggregation,
        } => aggregate_by_columns(&rows, &columns, amount, aggregation),
        Operation::GroupMedian { columns, amount } => median_by_columns(&rows, &columns, amount),
        Operation::GroupFit {
            columns,
            coordinates,
        } => fit_by_columns(&rows, &columns, &coordinates),
        Operation::SelectGroupExtrema {
            columns,
            amount,
            limit,
            highest,
        } => select_group_extrema(&rows, &columns, amount, limit, highest),
        Operation::Pivot(columns, kind) => pivot_csv(&rows, &columns, kind),
        Operation::Unpivot(columns, names) => unpivot(&rows, &columns, &names),
        Operation::ColumnStatistics { column, sample } => {
            summarize_unweighted(numeric_column_values(&rows, column)?, sample)
                .map_err(csv_column_statistics_error)
        }
        Operation::ColumnBins(column, edges) => {
            report_binned_unweighted(numeric_column_values(&rows, column)?, &edges)
                .map_err(csv_column_statistics_error)
        }
        Operation::ColumnBoxSummary(column) => {
            let (_, values) = column_observations(&rows, column)?;
            report_box_unweighted(values).map_err(csv_column_statistics_error)
        }
        Operation::ColumnCumulativeDistribution(column) => {
            let (_, values) = column_observations(&rows, column)?;
            super::statistics::report_cumulative_unweighted(values)
                .map_err(csv_column_statistics_error)
        }
        Operation::NumericProfile(columns) => numeric_column_profile(&rows, &columns),
        Operation::GroupNumericProfile { columns, amount } => {
            grouped_numeric_profile(&rows, &columns, amount)
        }
        Operation::PairMatrix { columns, statistic } => {
            column_pair_matrix(&rows, &columns, statistic, false).map(|(csv, _)| csv)
        }
        Operation::DistanceMatrix {
            labels,
            coordinates,
        } => row_distance_matrix(&rows, labels, &coordinates),
        Operation::NearestPoints {
            labels,
            coordinates,
            target,
            limit,
        } => nearest_point_rows(&rows, labels, &coordinates, &target, limit),
    }
}

enum JoinKind {
    Inner,
    Left,
    Full,
    MatchedLeft,
    UnmatchedLeft,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum JoinKey {
    One(ColumnReference),
    Multiple(Vec<ColumnReference>),
}

impl JoinKey {
    fn resolve(self, header: &[String]) -> Result<Vec<usize>, CsvError> {
        match self {
            Self::One(column) => resolve_distinct_columns(header, &[column], CsvError::JoinColumns),
            Self::Multiple(columns) => {
                if columns.is_empty() {
                    return Err(CsvError::JoinColumns);
                }
                resolve_distinct_columns(header, &columns, CsvError::JoinColumns)
            }
        }
    }
}

fn join_csv(input: &str, kind: JoinKind) -> Result<String, CsvError> {
    let ([left_key, right_key], data) =
        json_argument_and_data::<[JoinKey; 2]>(input, CsvError::JoinArguments)?;
    let [left, right] = read_csv_pair(data, CsvError::JoinArguments)?;
    let (left_header, left_rows) = left.split_first().ok_or(CsvError::MissingInput)?;
    let (right_header, right_rows) = right.split_first().ok_or(CsvError::MissingInput)?;
    let left_key = left_key.resolve(left_header)?;
    let right_key = right_key.resolve(right_header)?;
    if left_key.len() != right_key.len() {
        return Err(CsvError::JoinColumns);
    }
    if matches!(kind, JoinKind::MatchedLeft | JoinKind::UnmatchedLeft) {
        let keep_matched = matches!(kind, JoinKind::MatchedLeft);
        let right_keys: HashSet<_> = right_rows
            .iter()
            .map(|row| column_key(row, &right_key))
            .collect();
        let selected = left_rows
            .iter()
            .filter(|row| right_keys.contains(&column_key(row, &left_key)) == keep_matched);
        let mut output = String::new();
        for row in std::iter::once(left_header).chain(selected) {
            append_bounded_csv_row(
                &mut output,
                row.iter().map(String::as_str),
                CsvError::JoinTooLarge,
            )?;
        }
        return Ok(output);
    }
    let mut right_key_columns = vec![false; right_header.len()];
    for &column in &right_key {
        right_key_columns[column] = true;
    }

    let mut headers = HashSet::new();
    if left_header
        .iter()
        .map(String::as_str)
        .chain(joined_csv_columns(right_header, &right_key_columns))
        .any(|name| name.is_empty() || !headers.insert(name))
    {
        return Err(CsvError::InvalidHeader);
    }
    let mut by_key = HashMap::<Vec<&str>, Vec<&[String]>>::new();
    for row in right_rows {
        by_key
            .entry(column_key(row, &right_key))
            .or_default()
            .push(row);
    }
    let mut output = String::new();
    append_bounded_csv_row(
        &mut output,
        left_header
            .iter()
            .map(String::as_str)
            .chain(joined_csv_columns(right_header, &right_key_columns)),
        CsvError::JoinTooLarge,
    )?;
    for left in left_rows {
        let fields = left.iter().map(String::as_str);
        if let Some(matches) = by_key.get(&column_key(left, &left_key)) {
            for right in matches {
                append_bounded_csv_row(
                    &mut output,
                    fields
                        .clone()
                        .chain(joined_csv_columns(right, &right_key_columns)),
                    CsvError::JoinTooLarge,
                )?;
            }
        } else if matches!(kind, JoinKind::Left | JoinKind::Full) {
            append_bounded_csv_row(
                &mut output,
                fields.chain(std::iter::repeat_n(
                    "",
                    right_header.len() - right_key.len(),
                )),
                CsvError::JoinTooLarge,
            )?;
        }
    }
    if matches!(kind, JoinKind::Full) {
        let left_keys: HashSet<_> = left_rows
            .iter()
            .map(|row| column_key(row, &left_key))
            .collect();
        let mut fields = vec![""; left_header.len()];
        for right in right_rows {
            let key = column_key(right, &right_key);
            if !left_keys.contains(&key) {
                fields.fill("");
                for (&column, value) in left_key.iter().zip(key) {
                    fields[column] = value;
                }
                append_bounded_csv_row(
                    &mut output,
                    fields
                        .iter()
                        .copied()
                        .chain(joined_csv_columns(right, &right_key_columns)),
                    CsvError::JoinTooLarge,
                )?;
            }
        }
    }
    Ok(output)
}

fn csv_stack_inputs(input: &str) -> Result<Vec<String>, CsvError> {
    let inputs: Vec<String> = serde_json::from_str(input).map_err(|_| CsvError::StackArguments)?;
    if !(2..=MAX_STACK_TABLES).contains(&inputs.len()) {
        return Err(CsvError::StackArguments);
    }
    Ok(inputs)
}

fn stack_csv(input: &str) -> Result<String, CsvError> {
    let inputs = csv_stack_inputs(input)?;
    let tables = inputs
        .iter()
        .map(|table| read_rows(table.strip_prefix('\u{feff}').unwrap_or(table), ','))
        .collect::<Result<Vec<_>, _>>()?;
    let mut header = Vec::new();
    let mut positions = HashMap::new();
    let mut projections = Vec::with_capacity(tables.len());
    for table in &tables {
        let columns = table.first().ok_or(CsvError::MissingInput)?;
        let mut seen = HashSet::new();
        let mut projection = Vec::with_capacity(columns.len());
        for name in columns {
            let name = name.as_str();
            if name.is_empty() || !seen.insert(name) {
                return Err(CsvError::InvalidHeader);
            }
            let index = positions.entry(name).or_insert_with(|| {
                let index = header.len();
                header.push(name);
                index
            });
            projection.push(*index);
        }
        projections.push(projection);
    }
    let mut row_fields = vec![""; header.len()];
    let mut output = String::new();
    append_bounded_csv_row(&mut output, header.into_iter(), CsvError::JoinTooLarge)?;
    for (table, projection) in tables.iter().zip(projections) {
        for row in &table[1..] {
            row_fields.fill("");
            for (&index, cell) in projection.iter().zip(row) {
                row_fields[index] = cell.as_str();
            }
            append_bounded_csv_row(
                &mut output,
                row_fields.iter().copied(),
                CsvError::JoinTooLarge,
            )?;
        }
    }
    Ok(output)
}

fn bind_csv_columns(input: &str) -> Result<String, CsvError> {
    let tables = csv_stack_inputs(input)?
        .iter()
        .map(|table| read_table_for_preview(table, ','))
        .collect::<Result<Vec<_>, _>>()?;
    let widths: Vec<_> = tables.iter().map(|table| table[0].len()).collect();
    let record_count = tables.iter().map(Vec::len).max().unwrap_or_default();
    let mut output = String::new();
    for index in 0..record_count {
        let cells = tables.iter().zip(&widths).flat_map(|(table, &width)| {
            let row = table.get(index).map_or(&[][..], Vec::as_slice);
            row.iter()
                .map(String::as_str)
                .chain(std::iter::repeat_n("", width - row.len()))
        });
        append_bounded_csv_row(&mut output, cells, CsvError::JoinTooLarge)?;
    }
    Ok(output)
}

fn read_csv_pair(data: &str, invalid: CsvError) -> Result<[Vec<Vec<String>>; 2], CsvError> {
    let [left, right] = serde_json::from_str::<[String; 2]>(data).map_err(|_| invalid)?;
    Ok([
        read_rows(left.strip_prefix('\u{feff}').unwrap_or(&left), ',')?,
        read_rows(right.strip_prefix('\u{feff}').unwrap_or(&right), ',')?,
    ])
}

fn compare_csv_headers(input: &str) -> Result<String, CsvError> {
    let [old, new] = read_csv_pair(input, CsvError::HeaderComparisonArguments)?;
    let old = old.first().ok_or(CsvError::MissingInput)?;
    let new = new.first().ok_or(CsvError::MissingInput)?;
    let old_positions = named_header_positions(old)?;
    let new_positions = named_header_positions(new)?;
    let mut output = String::from("列名,原序号,新序号,变更");
    for (index, column) in old.iter().enumerate() {
        let old_index = index + 1;
        let new_index = new_positions.get(column.as_str()).copied();
        if new_index == Some(old_index) {
            continue;
        }
        let change = if new_index.is_some() {
            "位置变化"
        } else {
            "删除"
        };
        let new_index = new_index.map_or_else(String::new, |index| index.to_string());
        append_bounded_csv_row(
            &mut output,
            [
                column.as_str(),
                old_index.to_string().as_str(),
                new_index.as_str(),
                change,
            ]
            .into_iter(),
            CsvError::ColumnComparisonTooLarge,
        )?;
    }
    for (index, column) in new.iter().enumerate() {
        if !old_positions.contains_key(column.as_str()) {
            append_bounded_csv_row(
                &mut output,
                [
                    column.as_str(),
                    "",
                    (index + 1).to_string().as_str(),
                    "新增",
                ]
                .into_iter(),
                CsvError::ColumnComparisonTooLarge,
            )?;
        }
    }
    Ok(output)
}

fn named_header_positions(header: &[String]) -> Result<HashMap<&str, usize>, CsvError> {
    let mut positions = HashMap::with_capacity(header.len());
    for (index, column) in header.iter().enumerate() {
        if column.is_empty() || positions.insert(column.as_str(), index + 1).is_some() {
            return Err(CsvError::InvalidHeader);
        }
    }
    Ok(positions)
}

fn align_comparison_headers(old: &[String], new: &[String]) -> Result<Vec<usize>, CsvError> {
    named_header_positions(old)?;
    let new_positions = named_header_positions(new)?;
    if old.len() != new_positions.len() {
        return Err(CsvError::ComparisonHeaderMismatch);
    }
    old.iter()
        .map(|name| new_positions.get(name.as_str()).map(|index| index - 1))
        .collect::<Option<Vec<_>>>()
        .ok_or(CsvError::ComparisonHeaderMismatch)
}

fn compare_csv_rows(input: &str) -> Result<String, CsvError> {
    let [old, new] = read_csv_pair(input, CsvError::HeaderComparisonArguments)?;
    let (old_header, old_rows) = old.split_first().ok_or(CsvError::MissingInput)?;
    let (new_header, new_rows) = new.split_first().ok_or(CsvError::MissingInput)?;
    let old_order: Vec<_> = (0..old_header.len()).collect();
    let new_order = align_comparison_headers(old_header, new_header)?;
    let mut new_positions: HashMap<_, VecDeque<_>> = HashMap::new();
    for (index, row) in new_rows.iter().enumerate() {
        new_positions
            .entry(column_key(row, &new_order))
            .or_default()
            .push_back(index);
    }

    let mut output = String::new();
    let labels: Vec<_> = old_header
        .iter()
        .map(|name| format!("字段 {name}"))
        .collect();
    append_bounded_csv_row(
        &mut output,
        ["变更", "数据行"]
            .into_iter()
            .chain(labels.iter().map(String::as_str)),
        CsvError::ColumnComparisonTooLarge,
    )?;
    let mut append_change = |change, index: usize, fields: Vec<&str>| {
        append_bounded_csv_row(
            &mut output,
            [change, (index + 1).to_string().as_str()]
                .into_iter()
                .chain(fields),
            CsvError::ColumnComparisonTooLarge,
        )
    };
    let mut new_matched = vec![false; new_rows.len()];
    for (index, row) in old_rows.iter().enumerate() {
        let fields = column_key(row, &old_order);
        if let Some(new_index) = new_positions.get_mut(&fields).and_then(VecDeque::pop_front) {
            new_matched[new_index] = true;
        } else {
            append_change("删除", index, fields)?;
        }
    }
    for (index, row) in new_rows.iter().enumerate() {
        if !new_matched[index] {
            append_change("新增", index, column_key(row, &new_order))?;
        }
    }
    Ok(output)
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ComparisonColumns {
    One(ColumnReference),
    Multiple(Vec<ColumnReference>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ComparisonMode {
    Text,
    Numeric,
    All,
}

fn compare_csv_column(input: &str, mode: ComparisonMode) -> Result<String, CsvError> {
    let all_columns = mode == ComparisonMode::All;
    let (key_columns, columns, label_columns, data, invalid) = if all_columns {
        let invalid = CsvError::AllColumnComparisonArguments;
        let (keys, data) = json_argument_and_data::<Vec<ColumnReference>>(input, invalid)?;
        (keys, Vec::new(), true, data, invalid)
    } else {
        let invalid = CsvError::ColumnComparisonArguments;
        let ((keys, selection), data) =
            json_argument_and_data::<(Vec<ColumnReference>, ComparisonColumns)>(input, invalid)?;
        let (columns, label_columns) = match selection {
            ComparisonColumns::One(column) => (vec![column], false),
            ComparisonColumns::Multiple(columns) => (columns, true),
        };
        (keys, columns, label_columns, data, invalid)
    };
    if key_columns.is_empty() || (!all_columns && columns.is_empty()) {
        return Err(invalid);
    }
    let [old, new] = read_csv_pair(data, invalid)?;
    let (old_header, old_rows) = old.split_first().ok_or(CsvError::MissingInput)?;
    let (new_header, new_rows) = new.split_first().ok_or(CsvError::MissingInput)?;
    let resolve = |header: &[String], columns: &[ColumnReference]| {
        resolve_distinct_columns(header, columns, invalid)
    };
    let old_keys = resolve(old_header, &key_columns)?;
    let (new_keys, old_columns, new_columns) = if all_columns {
        let new_order = align_comparison_headers(old_header, new_header)?;
        let old_columns: Vec<_> = (0..old_header.len())
            .filter(|index| !old_keys.contains(index))
            .collect();
        if old_columns.is_empty() {
            return Err(CsvError::NoComparedColumns);
        }
        let in_new = |&index: &usize| new_order[index];
        let new_keys = old_keys.iter().map(in_new).collect();
        let new_columns = old_columns.iter().map(in_new).collect();
        (new_keys, old_columns, new_columns)
    } else {
        (
            resolve(new_header, &key_columns)?,
            resolve(old_header, &columns)?,
            resolve(new_header, &columns)?,
        )
    };
    let numeric = mode == ComparisonMode::Numeric;
    let column_labels: Vec<_> = old_columns
        .iter()
        .map(|&index| label_columns.then_some(old_header[index].as_str()))
        .collect();
    let mut new_values = HashMap::new();
    for row in new_rows {
        let key = column_key(row, &new_keys);
        let values = new_columns
            .iter()
            .map(|&column| {
                let value = row[column].as_str();
                comparable_column_value(value, numeric).map(|normalized| (value, normalized))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if new_values.insert(key, values).is_some() {
            return Err(CsvError::DuplicateComparisonKey);
        }
    }
    let mut output = String::new();
    let key_labels: Vec<_> = (1..=key_columns.len())
        .map(|index| format!("键{index}"))
        .collect();
    append_bounded_csv_row(
        &mut output,
        key_labels
            .iter()
            .map(String::as_str)
            .chain(label_columns.then_some("列"))
            .chain(["变更", "原值", "新值"]),
        CsvError::ColumnComparisonTooLarge,
    )?;
    let mut old_seen = HashSet::new();
    for row in old_rows {
        let key = column_key(row, &old_keys);
        if !old_seen.insert(key.clone()) {
            return Err(CsvError::DuplicateComparisonKey);
        }
        let replacement = new_values.remove(&key);
        for (position, (&column, &label)) in old_columns.iter().zip(&column_labels).enumerate() {
            let old_value = row[column].as_str();
            let comparable = comparable_column_value(old_value, numeric)?;
            let (change, new_value) = match replacement.as_ref().map(|values| &values[position]) {
                Some((_, new_comparable)) if *new_comparable == comparable => continue,
                Some((new_value, _)) => ("修改", *new_value),
                None => ("删除", ""),
            };
            write_column_comparison_change(
                &mut output,
                &key,
                label,
                [change, old_value, new_value],
            )?;
        }
    }
    for row in new_rows {
        let key = column_key(row, &new_keys);
        if let Some(values) = new_values.remove(&key) {
            for ((value, _), &label) in values.into_iter().zip(&column_labels) {
                write_column_comparison_change(&mut output, &key, label, ["新增", "", value])?;
            }
        }
    }
    Ok(output)
}

fn compare_csv_column_pair(input: &str, numeric: bool) -> Result<String, CsvError> {
    let invalid = CsvError::ColumnPairDifferenceArguments;
    let (sources, data) = json_argument_and_data::<[ColumnReference; 2]>(input, invalid)?;
    let rows = read_rows(data.strip_prefix('\u{feff}').unwrap_or(data), ',')?;
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let sources = resolve_distinct_columns(header, &sources, invalid)?;
    let &[left, right] = sources.as_slice() else {
        return Err(invalid);
    };
    let mut output = String::from("数据行,原值,新值");
    for (index, record) in records.iter().enumerate() {
        let row = index + 1;
        let old = record[left].as_str();
        let new = record[right].as_str();
        let with_row = |error: CsvError| CsvError::DataRowValue {
            row,
            cause: error.message(),
        };
        if comparable_column_value(old, numeric).map_err(with_row)?
            != comparable_column_value(new, numeric).map_err(with_row)?
        {
            let row_label = row.to_string();
            append_bounded_csv_row(
                &mut output,
                [row_label.as_str(), old, new].into_iter(),
                CsvError::ColumnComparisonTooLarge,
            )?;
        }
    }
    Ok(output)
}

fn write_column_comparison_change(
    output: &mut String,
    key: &[&str],
    column: Option<&str>,
    cells: [&str; 3],
) -> Result<(), CsvError> {
    append_bounded_csv_row(
        output,
        key.iter().copied().chain(column).chain(cells),
        CsvError::ColumnComparisonTooLarge,
    )
}

fn comparable_column_value(value: &str, numeric: bool) -> Result<Cow<'_, str>, CsvError> {
    if !numeric {
        return Ok(Cow::Borrowed(value));
    }
    if value.trim().is_empty() {
        return Ok(Cow::Borrowed(""));
    }
    parse_fraction_literal(value)
        .map(|value| Cow::Owned(value.to_string()))
        .map_err(|_| CsvError::InvalidNumber)
}

fn joined_csv_columns<'a>(row: &'a [String], omitted: &'a [bool]) -> impl Iterator<Item = &'a str> {
    row.iter()
        .zip(omitted)
        .filter(|(_, omitted)| !**omitted)
        .map(|(field, _)| field.as_str())
}

pub(super) fn append_bounded_csv_row<'a, E: Copy>(
    output: &mut String,
    fields: impl Iterator<Item = &'a str>,
    too_large: E,
) -> Result<(), E> {
    let mut separator = if output.is_empty() { "" } else { "\r\n" };
    for field in fields {
        append_bounded_csv_field(output, separator, field, too_large)?;
        separator = ",";
    }
    Ok(())
}

fn append_bounded_csv_field<E>(
    output: &mut String,
    separator: &str,
    field: &str,
    too_large: E,
) -> Result<(), E> {
    let field = csv_cell(field);
    if output.len() + separator.len() + field.len() > MAX_EXPANDED_CSV_BYTES {
        return Err(too_large);
    }
    output.push_str(separator);
    output.push_str(&field);
    Ok(())
}

fn transform_csv_pipeline(input: &str) -> Result<String, CsvError> {
    let (steps, data) = json_argument_and_data::<Vec<String>>(input, CsvError::PipelineArguments)?;
    if !(1..=MAX_PIPELINE_STEPS).contains(&steps.len())
        || steps
            .iter()
            .any(|step| step.trim().is_empty() || step.contains(['\r', '\n', '\0']))
    {
        return Err(CsvError::PipelineArguments);
    }
    let mut current = Cow::Borrowed(data);
    for (index, step) in steps.iter().enumerate() {
        let query = format!("{}\n{current}", step.trim());
        let (operation, input) = split_argument(&query).expect("命令和数据之间已经添加换行");
        let output =
            transform_csv_operation(operation, input).map_err(|error| CsvError::PipelineStep {
                index: index + 1,
                cause: error.message(),
            })?;
        if output.len() > MAX_EXPANDED_CSV_BYTES {
            return Err(CsvError::PipelineStep {
                index: index + 1,
                cause: CsvError::PipelineOutputTooLarge.message(),
            });
        }
        current = Cow::Owned(output);
    }
    Ok(current.into_owned())
}

fn split_argument(input: &str) -> Option<(&str, &str)> {
    let (argument, rest) = input.split_once(char::is_whitespace)?;
    Some((
        argument,
        input[argument.len()..].strip_prefix("\r\n").unwrap_or(rest),
    ))
}

fn column_arguments(input: &str) -> Result<(Vec<usize>, &str), CsvError> {
    let (columns, data) = split_argument(input).ok_or(CsvError::MissingInput)?;
    Ok((column_indices(columns)?, data))
}

fn column_reference_arguments(
    input: &str,
    invalid: CsvError,
) -> Result<(Vec<ColumnReference>, &str), CsvError> {
    if input.trim_start().starts_with('[') {
        let (columns, data) = json_argument_and_data::<Vec<ColumnReference>>(input, invalid)?;
        if columns.is_empty() {
            return Err(invalid);
        }
        return Ok((columns, data));
    }
    let (columns, data) = column_arguments(input)?;
    Ok((
        columns
            .into_iter()
            .map(|index| ColumnReference::Number(index + 1))
            .collect(),
        data,
    ))
}

fn single_column_argument(
    input: &str,
    error: CsvError,
) -> Result<(ColumnReference, &str), CsvError> {
    if input.trim_start().starts_with('"') {
        return column_reference_argument(input, error);
    }
    let (columns, data) = column_arguments(input)?;
    let [column] = columns.as_slice() else {
        return Err(error);
    };
    Ok((ColumnReference::Number(*column + 1), data))
}

fn column_value_arguments(input: &str) -> Result<(ColumnReference, String, &str), CsvError> {
    let (column, input) = column_reference_argument(input, CsvError::InvalidColumnValue)?;
    let (value, data) = json_argument_and_data(input, CsvError::InvalidColumnValue)?;
    Ok((column, value, data))
}

fn column_reference_argument(
    input: &str,
    error: CsvError,
) -> Result<(ColumnReference, &str), CsvError> {
    if input.trim_start().starts_with('"') {
        let (name, rest) = json_argument_and_data(input, error)?;
        return Ok((ColumnReference::Name(name), rest));
    }
    let (number, rest) = split_argument(input).ok_or(error)?;
    Ok((ColumnReference::Number(column_number(number)?), rest))
}

fn json_argument_and_data<T: DeserializeOwned>(
    input: &str,
    error: CsvError,
) -> Result<(T, &str), CsvError> {
    super::operators::json_argument_and_tail(input).ok_or(error)
}

fn column_indices(input: &str) -> Result<Vec<usize>, CsvError> {
    let mut seen = HashSet::new();
    input
        .split(',')
        .map(|part| {
            let index = column_index(part)?;
            if !seen.insert(index) {
                return Err(CsvError::InvalidColumns);
            }
            Ok(index)
        })
        .collect()
}

fn column_index(input: &str) -> Result<usize, CsvError> {
    column_number(input).map(|number| number - 1)
}

fn column_number(input: &str) -> Result<usize, CsvError> {
    if !input.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(CsvError::InvalidColumns);
    }
    input
        .parse::<usize>()
        .ok()
        .filter(|&number| number > 0)
        .ok_or(CsvError::InvalidColumns)
}

fn pick_csv_cell(input: &str) -> Result<String, CsvError> {
    let invalid = CsvError::CellArguments;
    let ((row, column), data) = json_argument_and_data::<(i128, ColumnReference)>(input, invalid)?;
    if row == 0 {
        return Err(invalid);
    }
    let mut rows = read_table_for_preview(data, ',')?;
    Ok(std::mem::take(csv_cell_mut(&mut rows, row, column)?))
}

fn set_csv_cell(input: &str) -> Result<Vec<Vec<String>>, CsvError> {
    let invalid = CsvError::SetCellArguments;
    let ((row, column, text), data) =
        json_argument_and_data::<(i128, ColumnReference, String)>(input, invalid)?;
    if row == 0 {
        return Err(invalid);
    }
    if text.contains('\0') {
        return Err(CsvError::NullReplacementValue);
    }
    let mut rows = read_table_for_preview(data, ',')?;
    *csv_cell_mut(&mut rows, row, column)? = text;
    Ok(rows)
}

fn set_csv_cells(input: &str) -> Result<Vec<Vec<String>>, CsvError> {
    let invalid = CsvError::SetCellsArguments;
    let (edits, data) =
        json_argument_and_data::<Vec<(i128, ColumnReference, String)>>(input, invalid)?;
    if edits.is_empty() {
        return Err(invalid);
    }
    let mut rows = read_table_for_preview(data, ',')?;
    for (index, (row, column, text)) in edits.into_iter().enumerate() {
        let cell = if row == 0 {
            Err(CsvError::SetCellArguments)
        } else if text.contains('\0') {
            Err(CsvError::NullReplacementValue)
        } else {
            csv_cell_mut(&mut rows, row, column)
        };
        *cell.map_err(|error| CsvError::CellEdit {
            index: index + 1,
            cause: error.message(),
        })? = text;
    }
    Ok(rows)
}

fn csv_cell_mut(
    rows: &mut [Vec<String>],
    row: i128,
    column: ColumnReference,
) -> Result<&mut String, CsvError> {
    let header = rows.first().ok_or(CsvError::MissingInput)?;
    let column = column.resolve(header)?;
    validate_column_width(&[column], header.len())?;
    let index = relative_data_row_index(row, rows.len() - 1).ok_or(CsvError::MissingDataRow)?;
    Ok(&mut rows[index + 1][column])
}

fn relative_data_row_index(row: i128, count: usize) -> Option<usize> {
    let index = if row < 0 {
        count as i128 + row
    } else {
        row - 1
    };
    usize::try_from(index).ok().filter(|&index| index < count)
}

fn select_group_rows(input: &str) -> Result<String, CsvError> {
    let invalid = CsvError::GroupRowArguments;
    let ((groups, row), data) =
        json_argument_and_data::<(Vec<ColumnReference>, i128)>(input, invalid)?;
    if row == 0 {
        return Err(invalid);
    }
    let rows = read_table_for_preview(data, ',')?;
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let groups = resolve_distinct_columns(header, &groups, CsvError::GroupingColumns)?;
    let selected = grouped_row_positions(records, &groups)
        .into_iter()
        .filter_map(|positions| {
            let index = relative_data_row_index(row, positions.len())?;
            Some(&records[positions[index]])
        });
    let mut output = String::new();
    for record in std::iter::once(header).chain(selected) {
        append_bounded_csv_row(
            &mut output,
            record.iter().map(String::as_str),
            CsvError::GroupRowsTooLarge,
        )?;
    }
    Ok(output)
}

fn select_data_rows(
    rows: &[Vec<String>],
    ranges: &str,
    keep_selected: bool,
) -> Result<String, CsvError> {
    Ok(write_rows(
        selected_rows_with_header(rows, ranges, keep_selected)?,
        ',',
    ))
}

fn selected_rows_with_header<'a>(
    rows: &'a [Vec<String>],
    ranges: &str,
    keep_selected: bool,
) -> Result<impl Iterator<Item = &'a Vec<String>>, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let selected = select_index_ranges(ranges, records.len()).map_err(|error| match error {
        IndexRangeError::Invalid => CsvError::InvalidRowRange,
        IndexRangeError::OutOfBounds => CsvError::MissingDataRow,
    })?;
    let picked = records
        .iter()
        .zip(selected)
        .filter_map(move |(row, selected)| (selected == keep_selected).then_some(row));
    Ok(std::iter::once(header).chain(picked))
}

fn pick_table_region(input: &str) -> Result<Vec<Vec<String>>, CsvError> {
    let invalid = CsvError::TableRegionArguments;
    let ((ranges, columns), data) =
        json_argument_and_data::<(String, Vec<ColumnReference>)>(input, invalid)?;
    if columns.is_empty() {
        return Err(invalid);
    }
    let rows = read_table_for_preview(data, ',')?;
    let header = rows.first().ok_or(CsvError::MissingInput)?;
    let columns = resolve_distinct_columns(header, &columns, invalid)?;
    let selected = selected_rows_with_header(&rows, &ranges, true)?;
    Ok(selected
        .map(|row| columns.iter().map(|&column| row[column].clone()).collect())
        .collect())
}

fn drop_empty_csv_records(rows: &[Vec<String>]) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let retained = records
        .iter()
        .filter(|row| row.iter().any(|cell| !cell.is_empty()));
    if retained.clone().any(|row| row.len() != header.len()) {
        return Err(CsvError::DifferentColumnCount);
    }
    Ok(write_rows(std::iter::once(header).chain(retained), ','))
}

fn drop_columns(
    rows: &[Vec<String>],
    columns: &[ColumnReference],
) -> Result<Vec<Vec<String>>, CsvError> {
    let header = rows.first().ok_or(CsvError::MissingInput)?;
    let removed: HashSet<_> =
        resolve_distinct_columns(header, columns, CsvError::NamedColumnArguments)?
            .into_iter()
            .collect();
    if removed.len() == header.len() {
        return Err(CsvError::NoColumnsRemain);
    }
    let kept: Vec<_> = (0..header.len())
        .filter(|column| !removed.contains(column))
        .collect();
    select_columns(rows, &kept)
}

fn filter_missing_rows(
    rows: &[Vec<String>],
    columns: &[ColumnReference],
    missing: bool,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let columns = resolve_distinct_columns(header, columns, CsvError::NamedColumnArguments)?;
    let selected = records
        .iter()
        .filter(|row| columns.iter().any(|&column| row[column].trim().is_empty()) == missing);
    Ok(write_rows(std::iter::once(header).chain(selected), ','))
}

fn drop_empty_csv_columns(rows: &[Vec<String>]) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let columns: Vec<_> = (0..header.len())
        .filter(|&column| records.is_empty() || records.iter().any(|row| !row[column].is_empty()))
        .collect();
    if columns.is_empty() {
        return Err(CsvError::NoColumnsRemain);
    }
    select_columns(rows, &columns).map(|selected| write_rows(&selected, ','))
}

fn select_columns(rows: &[Vec<String>], columns: &[usize]) -> Result<Vec<Vec<String>>, CsvError> {
    let width = rows.first().ok_or(CsvError::MissingInput)?.len();
    validate_column_width(columns, width)?;
    Ok(rows
        .iter()
        .map(|row| columns.iter().map(|&index| row[index].clone()).collect())
        .collect())
}

fn select_named_columns(
    rows: &[Vec<String>],
    names: &[String],
) -> Result<Vec<Vec<String>>, CsvError> {
    let header = rows.first().ok_or(CsvError::MissingInput)?;
    let mut seen = HashSet::with_capacity(names.len());
    if names.is_empty() || names.iter().any(|name| !seen.insert(name)) {
        return Err(CsvError::InvalidColumnNames);
    }
    let columns = names
        .iter()
        .map(|name| header_column(header, name))
        .collect::<Result<Vec<_>, _>>()?;
    select_columns(rows, &columns)
}

fn header_column(header: &[String], name: &str) -> Result<usize, CsvError> {
    let mut matches = header
        .iter()
        .enumerate()
        .filter(|(_, value)| value.as_str() == name);
    let (index, _) = matches.next().ok_or(CsvError::MissingColumnName)?;
    if matches.next().is_some() {
        return Err(CsvError::AmbiguousColumnName);
    }
    Ok(index)
}

fn rename_column(mut rows: Vec<Vec<String>>, [old, new]: [String; 2]) -> Result<String, CsvError> {
    let header = rows.first_mut().ok_or(CsvError::MissingInput)?;
    if new.trim().is_empty() {
        return Err(CsvError::RenameArguments);
    }
    let column = header_column(header, &old)?;
    if old != new && header.contains(&new) {
        return Err(CsvError::HeaderNameConflict);
    }
    header[column] = new;
    Ok(write_rows(&rows, ','))
}

fn rename_columns(mut rows: Vec<Vec<String>>, names: Vec<[String; 2]>) -> Result<String, CsvError> {
    let header = rows.first_mut().ok_or(CsvError::MissingInput)?;
    if names.is_empty() {
        return Err(CsvError::BulkRenameArguments);
    }
    let mut replacements = HashMap::with_capacity(names.len());
    for [old, new] in names {
        if new.trim().is_empty() || new.contains('\0') {
            return Err(CsvError::BulkRenameArguments);
        }
        let index = header_column(header, &old)?;
        if replacements.insert(index, new).is_some() {
            return Err(CsvError::BulkRenameArguments);
        }
    }
    let mut final_names = HashSet::new();
    if header
        .iter()
        .enumerate()
        .any(|(index, name)| !final_names.insert(replacements.get(&index).unwrap_or(name)))
    {
        return Err(CsvError::HeaderNameConflict);
    }
    for (index, name) in replacements {
        header[index] = name;
    }
    Ok(write_rows(&rows, ','))
}

fn replace_column_values(
    mut rows: Vec<Vec<String>>,
    column: ColumnReference,
    previous: &str,
    replacement: &str,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first_mut().ok_or(CsvError::MissingInput)?;
    let column = column.resolve(header)?;
    validate_column_width(&[column], header.len())?;
    for row in records {
        if row[column] == previous {
            replacement.clone_into(&mut row[column]);
        }
    }
    Ok(write_rows(&rows, ','))
}

fn map_column_values(
    rows: &[Vec<String>],
    column: ColumnReference,
    replacements: &HashMap<String, String>,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let column = column.resolve(header)?;
    validate_column_width(&[column], header.len())?;
    let mut output = String::new();
    append_bounded_csv_row(
        &mut output,
        header.iter().map(String::as_str),
        CsvError::MapValueTooLarge,
    )?;
    for row in records {
        let fields = row.iter().enumerate().map(|(index, value)| {
            if index == column {
                replacements.get(value).unwrap_or(value).as_str()
            } else {
                value.as_str()
            }
        });
        append_bounded_csv_row(&mut output, fields, CsvError::MapValueTooLarge)?;
    }
    Ok(output)
}

fn trim_column_values(
    mut rows: Vec<Vec<String>>,
    columns: &[ColumnReference],
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first_mut().ok_or(CsvError::MissingInput)?;
    let columns = resolve_distinct_columns(header, columns, CsvError::NamedColumnArguments)?;
    for row in records {
        for &column in &columns {
            let cell = &mut row[column];
            let trimmed = cell.trim();
            if trimmed.len() != cell.len() {
                *cell = trimmed.to_owned();
            }
        }
    }
    Ok(write_rows(&rows, ','))
}

fn count_empty_cells(
    rows: &[Vec<String>],
    columns: &[ColumnReference],
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let columns = resolve_distinct_columns(header, columns, CsvError::NamedColumnArguments)?;
    let mut output = vec![vec![
        "列号".to_owned(),
        "列名".to_owned(),
        "空单元格".to_owned(),
        "非空单元格".to_owned(),
    ]];
    for column in columns {
        let empty = records.iter().filter(|row| row[column].is_empty()).count();
        output.push(vec![
            (column + 1).to_string(),
            header[column].clone(),
            empty.to_string(),
            (records.len() - empty).to_string(),
        ]);
    }
    Ok(write_rows(&output, ','))
}

fn round_csv_columns(
    rows: &[Vec<String>],
    columns: &[ColumnReference],
    digits: u32,
    format: NumberColumnFormat,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let (invalid, too_large, format): (_, _, fn(BigRational, u32) -> String) = match format {
        NumberColumnFormat::Decimal => (
            CsvError::DecimalColumnsArguments,
            CsvError::DecimalColumnsTooLarge,
            rounded_decimal,
        ),
        NumberColumnFormat::Scientific => (
            CsvError::SignificantColumnsArguments,
            CsvError::SignificantColumnsTooLarge,
            format_significant_digits,
        ),
        NumberColumnFormat::Engineering => (
            CsvError::SignificantColumnsArguments,
            CsvError::SignificantColumnsTooLarge,
            format_engineering_notation,
        ),
    };
    let selected: HashSet<_> = resolve_distinct_columns(header, columns, invalid)?
        .into_iter()
        .collect();
    let mut output = String::new();
    append_bounded_csv_row(&mut output, header.iter().map(String::as_str), too_large)?;
    for row in records {
        let mut separator = "\r\n";
        for (index, field) in row.iter().enumerate() {
            let value = if selected.contains(&index) && !field.is_empty() {
                let number = parse_fraction_literal(field).map_err(|_| CsvError::InvalidNumber)?;
                Cow::Owned(format(number, digits))
            } else {
                Cow::Borrowed(field.as_str())
            };
            append_bounded_csv_field(&mut output, separator, &value, too_large)?;
            separator = ",";
        }
    }
    Ok(output)
}

fn check_header(rows: &[Vec<String>], expected: &[String]) -> Result<String, CsvError> {
    let actual = rows.first().ok_or(CsvError::MissingInput)?;
    let mut output = String::from("列号,问题,预期列名,实际列名");
    for column in 0..actual.len().max(expected.len()) {
        let wanted = expected.get(column).map(String::as_str);
        let found = actual.get(column).map(String::as_str);
        if wanted == found {
            continue;
        }
        let issue = match (wanted, found) {
            (None, _) => "多余列",
            (_, None) => "缺少列",
            _ => "列名不同",
        };
        let number = (column + 1).to_string();
        append_bounded_csv_row(
            &mut output,
            [
                number.as_str(),
                issue,
                wanted.unwrap_or_default(),
                found.unwrap_or_default(),
            ]
            .into_iter(),
            CsvError::HeaderCheckTooLarge,
        )?;
    }
    Ok(output)
}

fn check_columns(
    rows: &[Vec<String>],
    columns: &[ColumnReference],
    validation: &ColumnValidation,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let columns = resolve_distinct_columns(header, columns, CsvError::NamedColumnArguments)?;
    let too_large = match validation {
        ColumnValidation::Number | ColumnValidation::NumericRange { .. } => {
            CsvError::NumericIssuesTooLarge
        }
        ColumnValidation::Date | ColumnValidation::DatePattern(_) => CsvError::DateIssuesTooLarge,
        ColumnValidation::Pattern(_) => CsvError::RegexIssuesTooLarge,
        ColumnValidation::LengthRange(..)
        | ColumnValidation::AllowedValues(_)
        | ColumnValidation::Json
        | ColumnValidation::Timestamp
        | ColumnValidation::TimestampRange { .. } => CsvError::ValidationRulesTooLarge,
    };
    let rules: Vec<_> = columns
        .into_iter()
        .map(|column| (column, validation))
        .collect();
    report_invalid_cells(header, records, &rules, false, too_large)
}

fn check_column_rules(
    rows: &[Vec<String>],
    rules: &[(ColumnReference, ColumnValidation)],
    output: ColumnRuleOutput,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let rules = rules
        .iter()
        .map(|(column, validation)| {
            let column = column.resolve(header)?;
            validate_column_width(&[column], header.len())?;
            Ok((column, validation))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if matches!(output, ColumnRuleOutput::Issues) {
        return report_invalid_cells(
            header,
            records,
            &rules,
            true,
            CsvError::ValidationRulesTooLarge,
        );
    }
    validate_regex_inputs(records, &rules)?;
    if matches!(output, ColumnRuleOutput::Summary) {
        return summarize_column_rules(header, records, &rules);
    }
    let passes_all = |row: &[String]| {
        rules
            .iter()
            .all(|&(column, rule)| rule.accepts(&row[column]))
    };
    if let ColumnRuleOutput::Annotate {
        name,
        count_failures,
    } = &output
    {
        return write_appended_column(header, records, name, |row| {
            let value = if *count_failures {
                rules
                    .iter()
                    .filter(|&&(column, rule)| !rule.accepts(&row[column]))
                    .count()
            } else {
                usize::from(passes_all(row))
            };
            Ok(value.to_string())
        });
    }
    let keep_valid = matches!(output, ColumnRuleOutput::ValidRows);
    let matches = records.iter().filter(|row| passes_all(row) == keep_valid);
    let mut csv = String::new();
    for row in std::iter::once(header).chain(matches) {
        append_bounded_csv_row(
            &mut csv,
            row.iter().map(String::as_str),
            CsvError::ValidationRulesTooLarge,
        )?;
    }
    Ok(csv)
}

fn summarize_column_rules(
    header: &[String],
    records: &[Vec<String>],
    rules: &[(usize, &ColumnValidation)],
) -> Result<String, CsvError> {
    let mut output = String::from("规则号,列号,列名,规则,通过数,失败数");
    for (index, &(column, rule)) in rules.iter().enumerate() {
        let passed = records
            .iter()
            .filter(|row| rule.accepts(&row[column]))
            .count();
        let number = (index + 1).to_string();
        let column_number = (column + 1).to_string();
        let failures = (records.len() - passed).to_string();
        let passed = passed.to_string();
        append_bounded_csv_row(
            &mut output,
            [
                number.as_str(),
                column_number.as_str(),
                header[column].as_str(),
                rule.label(),
                passed.as_str(),
                failures.as_str(),
            ]
            .into_iter(),
            CsvError::ValidationRulesTooLarge,
        )?;
    }
    Ok(output)
}

fn validate_regex_inputs(
    records: &[Vec<String>],
    rules: &[(usize, &ColumnValidation)],
) -> Result<(), CsvError> {
    let regex_columns: Vec<_> = rules
        .iter()
        .filter_map(|&(column, rule)| {
            matches!(rule, ColumnValidation::Pattern(_)).then_some(column)
        })
        .collect();
    if !regex_columns.is_empty()
        && records
            .iter()
            .flat_map(|record| {
                regex_columns
                    .iter()
                    .map(move |&column| record[column].len())
            })
            .sum::<usize>()
            > MAX_REGEX_COLUMN_INPUT_BYTES
    {
        return Err(CsvError::RegexCheckColumnsInputTooLarge);
    }
    Ok(())
}

fn report_invalid_cells(
    header: &[String],
    records: &[Vec<String>],
    rules: &[(usize, &ColumnValidation)],
    include_rules: bool,
    too_large: CsvError,
) -> Result<String, CsvError> {
    validate_regex_inputs(records, rules)?;
    let mut output = String::from(if include_rules {
        "数据行,列号,列名,规则号,规则,内容"
    } else {
        "数据行,列号,列名,内容"
    });
    for (index, record) in records.iter().enumerate() {
        let row_number = (index + 1).to_string();
        for (index, &(column, validation)) in rules.iter().enumerate() {
            let cell = &record[column];
            if validation.accepts(cell) {
                continue;
            }
            let column_number = (column + 1).to_string();
            let rule_number = include_rules.then(|| (index + 1).to_string());
            let rule_details = rule_number
                .as_deref()
                .map(|number| [number, validation.label()]);
            append_bounded_csv_row(
                &mut output,
                [
                    row_number.as_str(),
                    column_number.as_str(),
                    header[column].as_str(),
                ]
                .into_iter()
                .chain(rule_details.into_iter().flatten())
                .chain([cell.as_str()]),
                too_large,
            )?;
        }
    }
    Ok(output)
}

fn fill_empty_csv_cells(
    rows: &[Vec<String>],
    columns: &[ColumnReference],
    direction: FillDirection,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let columns = resolve_distinct_columns(header, columns, CsvError::NamedColumnArguments)?;
    let mut output = String::new();
    append_bounded_csv_row(
        &mut output,
        header.iter().map(String::as_str),
        CsvError::FillTooLarge,
    )?;
    let mut sources: Vec<_> = columns
        .into_iter()
        .map(|column| {
            let following = matches!(direction, FillDirection::Up).then(|| {
                records
                    .iter()
                    .map(move |row| row[column].as_str())
                    .filter(|cell| !cell.is_empty())
                    .peekable()
            });
            (column, "", following)
        })
        .collect();
    let mut fields = Vec::with_capacity(header.len());
    for row in records {
        fields.clear();
        fields.extend(row.iter().map(String::as_str));
        for (column, previous, following) in &mut sources {
            let cell = &mut fields[*column];
            match (cell.is_empty(), following) {
                (true, Some(following)) => {
                    *cell = following.peek().copied().unwrap_or_default();
                }
                (false, Some(following)) => {
                    following.next();
                }
                (true, None) => *cell = *previous,
                (false, None) => *previous = *cell,
            }
        }
        append_bounded_csv_row(&mut output, fields.iter().copied(), CsvError::FillTooLarge)?;
    }
    Ok(output)
}

fn interpolate_empty_csv_cells(
    rows: &[Vec<String>],
    source: ColumnReference,
    groups: &[ColumnReference],
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let source = source.resolve(header)?;
    validate_column_width(&[source], header.len())?;
    let groups = resolve_distinct_columns(header, groups, CsvError::InterpolationColumns)?;
    let mut values = Vec::with_capacity(records.len());
    for (index, row) in records.iter().enumerate() {
        let field = &row[source];
        let value = (!field.is_empty())
            .then(|| parse_fraction_literal(field))
            .transpose()
            .map_err(|error| CsvError::DataRowValue {
                row: index + 1,
                cause: error.message(),
            })?;
        values.push(value);
    }
    let mut fills = vec![None; records.len()];
    let mut fill_bytes = 0usize;
    for positions in grouped_row_positions(records, &groups) {
        let mut previous: Option<(usize, &BigRational)> = None;
        for (position, &row) in positions.iter().enumerate() {
            let Some(end) = values[row].as_ref() else {
                continue;
            };
            if let Some((start_position, start)) = previous {
                let span = position - start_position;
                for (offset, &row) in positions[start_position + 1..position].iter().enumerate() {
                    let progress = BigRational::new((offset + 1).into(), span.into());
                    let fill = interpolate_fraction(start, end, &progress)
                        .map_err(|error| CsvError::DataRowValue {
                            row: row + 1,
                            cause: error.message(),
                        })?
                        .to_string();
                    fill_bytes = fill_bytes.saturating_add(fill.len());
                    if fill_bytes > MAX_EXPANDED_CSV_BYTES {
                        return Err(CsvError::FillTooLarge);
                    }
                    fills[row] = Some(fill);
                }
            }
            previous = Some((position, end));
        }
    }

    let too_large = CsvError::FillTooLarge;
    let mut output = String::new();
    append_bounded_csv_row(&mut output, header.iter().map(String::as_str), too_large)?;
    for (row, fill) in records.iter().zip(&fills) {
        let fields = row.iter().enumerate().map(|(column, field)| {
            if column == source {
                fill.as_deref().unwrap_or(field)
            } else {
                field.as_str()
            }
        });
        append_bounded_csv_row(&mut output, fields, too_large)?;
    }
    Ok(output)
}

fn add_constant_column(rows: &[Vec<String>], name: &str, value: &str) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    if name.trim().is_empty() || name.contains('\0') {
        return Err(CsvError::ConstantColumnArguments);
    }
    if value.contains('\0') {
        return Err(CsvError::NullReplacementValue);
    }
    write_appended_column(header, records, name, |_| Ok(value))
}

fn number_csv_rows(rows: &[Vec<String>], name: &str, first: u64) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    if name.trim().is_empty() || name.contains('\0') {
        return Err(CsvError::RowNumberArguments);
    }
    let mut numbers = first..=u64::MAX;
    write_appended_column(header, records, name, |_| {
        numbers
            .next()
            .map(|number| number.to_string())
            .ok_or(CsvError::RowNumberOverflow)
    })
}

fn concatenate_columns(
    rows: &[Vec<String>],
    name: &str,
    columns: &[ColumnReference],
    separator: &str,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    if name.trim().is_empty() || name.contains('\0') || columns.len() < 2 {
        return Err(CsvError::ConcatenatedColumnArguments);
    }
    if separator.contains('\0') {
        return Err(CsvError::NullReplacementValue);
    }
    let columns = resolve_distinct_columns(header, columns, CsvError::ConcatenatedColumnArguments)?;
    let separator_bytes = separator.len().saturating_mul(columns.len() - 1);
    write_appended_column(header, records, name, |row| {
        let parts = column_key(row, &columns);
        let bytes = parts.iter().fold(separator_bytes, |sum, field| {
            sum.saturating_add(field.len())
        });
        if bytes > MAX_EXPANDED_CSV_BYTES {
            return Err(CsvError::AddedColumnTooLarge);
        }
        Ok(parts.join(separator))
    })
}

fn template_column(rows: &[Vec<String>], name: &str, template: &str) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    if name.trim().is_empty() || name.contains('\0') {
        return Err(CsvError::TemplateColumnArguments);
    }
    if template.contains('\0') {
        return Err(CsvError::NullReplacementValue);
    }
    let parts = parse_template(template, CsvError::TemplateColumnArguments, |field| {
        if let Some(number) = field
            .strip_prefix('#')
            .filter(|number| !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()))
        {
            let number = number.parse().map_err(|_| CsvError::InvalidColumns)?;
            let index = ColumnReference::Number(number).resolve(header)?;
            validate_column_width(&[index], header.len())?;
            Ok(index)
        } else {
            header_column(header, field)
        }
    })?;
    write_appended_column(header, records, name, |row| {
        let mut value = String::new();
        for part in &parts {
            let chunk = match *part {
                TemplatePart::Literal(literal) => literal,
                TemplatePart::Field(index) => &row[index],
            };
            if value.len().saturating_add(chunk.len()) > MAX_EXPANDED_CSV_BYTES {
                return Err(CsvError::AddedColumnTooLarge);
            }
            value.push_str(chunk);
        }
        Ok(value)
    })
}

fn regex_capture_column(
    rows: &[Vec<String>],
    name: &str,
    source: ColumnReference,
    pattern: &str,
    group: &str,
    expand_rows: bool,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    if name.trim().is_empty() || name.contains('\0') {
        return Err(CsvError::RegexCaptureColumnArguments);
    }
    let source = source.resolve(header)?;
    validate_column_width(&[source], header.len())?;
    if expand_rows
        && records.iter().map(|row| row[source].len()).sum::<usize>() > MAX_REGEX_COLUMN_INPUT_BYTES
    {
        return Err(CsvError::RegexCaptureColumnsInputTooLarge);
    }
    let pattern = compile_regex(pattern).map_err(CsvError::Regex)?;
    let group = regex_group_index(&pattern, group).map_err(CsvError::Regex)?;
    if expand_rows {
        let pattern = &pattern;
        return write_expanded_column(header, records, name, move |row| {
            let mut matches = pattern.captures_iter(&row[source]).peekable();
            let unmatched = matches.peek().is_none();
            let cells = matches
                .map(move |captures| captures.get(group).map_or("", |matched| matched.as_str()));
            Ok(cells.chain(unmatched.then_some("")))
        });
    }
    write_appended_column(header, records, name, |row| {
        let field = &row[source];
        Ok(pattern
            .captures(field)
            .and_then(|captures| captures.get(group))
            .map_or("", |matched| matched.as_str()))
    })
}

fn regex_capture_columns(
    rows: &[Vec<String>],
    source: ColumnReference,
    pattern: &str,
    columns: &[[String; 2]],
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let names = || columns.iter().map(|[name, _]| name);
    let mut seen = HashSet::new();
    if !(1..=MAX_REGEX_CAPTURE_COLUMNS).contains(&columns.len())
        || names().any(|name| name.trim().is_empty() || name.contains('\0') || !seen.insert(name))
    {
        return Err(CsvError::RegexCaptureColumnsArguments);
    }
    if names().any(|name| header.contains(name)) {
        return Err(CsvError::HeaderNameConflict);
    }
    let source = source.resolve(header)?;
    validate_column_width(&[source], header.len())?;
    if records.iter().map(|row| row[source].len()).sum::<usize>() > MAX_REGEX_COLUMN_INPUT_BYTES {
        return Err(CsvError::RegexCaptureColumnsInputTooLarge);
    }
    let pattern = compile_regex(pattern).map_err(CsvError::Regex)?;
    let groups = columns
        .iter()
        .map(|[_, group]| regex_group_index(&pattern, group))
        .collect::<Result<Vec<_>, _>>()
        .map_err(CsvError::Regex)?;

    let mut output = String::new();
    append_bounded_csv_row(
        &mut output,
        header.iter().chain(names()).map(String::as_str),
        CsvError::AddedColumnTooLarge,
    )?;
    for row in records {
        let captures = pattern.captures(&row[source]);
        let fields = groups.iter().map(|&group| {
            captures
                .as_ref()
                .and_then(|captures| captures.get(group))
                .map_or("", |matched| matched.as_str())
        });
        append_bounded_csv_row(
            &mut output,
            row.iter().map(String::as_str).chain(fields),
            CsvError::AddedColumnTooLarge,
        )?;
    }
    Ok(output)
}

fn first_nonempty_csv_column(
    rows: &[Vec<String>],
    name: &str,
    columns: &[ColumnReference],
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    if name.trim().is_empty() || name.contains('\0') || columns.len() < 2 {
        return Err(CsvError::FirstNonemptyColumnArguments);
    }
    let columns =
        resolve_distinct_columns(header, columns, CsvError::FirstNonemptyColumnArguments)?;
    write_appended_column(header, records, name, |row| {
        Ok(columns
            .iter()
            .map(|&index| row[index].as_str())
            .find(|value| !value.is_empty())
            .unwrap_or(""))
    })
}

fn parse_formatted_date(text: &str, items: &[Item<'_>]) -> Result<NaiveDate, chrono::ParseError> {
    let mut parsed = Parsed::new();
    parse(&mut parsed, text, items.iter())?;
    parsed.to_naive_date()
}

fn date_pattern_column(
    rows: &[Vec<String>],
    name: &str,
    source: ColumnReference,
    pattern: &str,
    parse_input: bool,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    if name.trim().is_empty() || name.contains('\0') {
        return Err(CsvError::DateFormatColumnArguments);
    }
    let source = source.resolve(header)?;
    validate_column_width(&[source], header.len())?;
    if pattern.is_empty() || pattern.contains('\0') {
        return Err(CsvError::DateFormatColumnPattern);
    }
    let items = StrftimeItems::new(pattern)
        .parse()
        .map_err(|_| CsvError::DateFormatColumnPattern)?;
    let format_date = |date: NaiveDate| -> Result<String, CsvError> {
        let mut result = String::new();
        write!(result, "{}", date.format_with_items(items.iter()))
            .map_err(|_| CsvError::DateFormatColumnPattern)?;
        Ok(result)
    };
    if !parse_input {
        // 只有表头时也检查输出格式是否需要日期之外的信息。
        format_date(NaiveDate::default())?;
    }
    let mut row_number = 0;
    write_appended_column(header, records, name, |row| {
        row_number += 1;
        let text = row[source].trim();
        if text.is_empty() {
            return Ok(String::new());
        }
        let data_error = |cause| CsvError::DataRowValue {
            row: row_number,
            cause,
        };
        if parse_input {
            let date =
                parse_formatted_date(text, &items).map_err(|_| data_error("日期不符合指定格式"))?;
            if !(0..=9999).contains(&date.year()) {
                return Err(data_error("年份须在 0000 至 9999 之间"));
            }
            return Ok(date.to_string());
        }
        let date = parse_complete_date(text).map_err(|error| data_error(error.message()))?;
        format_date(date)
    })
}

enum TextColumnComputation {
    Length(TextLengthUnit),
    Slice(GraphemeRange),
    RegexCount(Regex),
    RegexReplace(Regex, String),
    Replace(String, String),
    Pad {
        width: usize,
        fill: char,
        left: bool,
    },
}

#[derive(Deserialize)]
#[serde(untagged)]
enum WorkdayOffsetArguments {
    Standard((String, ColumnReference, ColumnReference)),
    WithExceptions((String, ColumnReference, ColumnReference, [Vec<String>; 2])),
}

enum DateOffsetUnit {
    Days,
    Months,
    Workdays(HashMap<NaiveDate, bool>),
}

fn date_offset_column(
    rows: &[Vec<String>],
    name: &str,
    source: ColumnReference,
    amounts: ColumnReference,
    unit: DateOffsetUnit,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    if name.trim().is_empty() || name.contains('\0') {
        return Err(CsvError::DateOffsetColumnArguments);
    }
    let source = source.resolve(header)?;
    let amounts = amounts.resolve(header)?;
    validate_column_width(&[source, amounts], header.len())?;
    let amount_error = match unit {
        DateOffsetUnit::Days => "天数须为整数且在支持范围内",
        DateOffsetUnit::Months => "月数须为整数且在支持范围内",
        DateOffsetUnit::Workdays(_) => "工作日数须为整数且在支持范围内",
    };
    let mut row_number = 0;
    write_appended_column(header, records, name, |row| {
        row_number += 1;
        let (date, offset) = (row[source].trim(), row[amounts].trim());
        if date.is_empty() || offset.is_empty() {
            return Ok(String::new());
        }
        let at_row = |cause| CsvError::DataRowValue {
            row: row_number,
            cause,
        };
        let date = parse_complete_date(date).map_err(|error| at_row(error.message()))?;
        let offset = parse_fraction_literal(offset)
            .ok()
            .filter(BigRational::is_integer)
            .and_then(|amount| i64::try_from(amount.to_integer()).ok())
            .ok_or_else(|| at_row(amount_error))?;
        let (amount, subtract) = (offset.unsigned_abs(), offset < 0);
        match &unit {
            DateOffsetUnit::Days => shift_calendar_days(date, amount, subtract),
            DateOffsetUnit::Months => shift_calendar_months(date, amount, subtract),
            DateOffsetUnit::Workdays(exceptions) => {
                shift_calendar_workdays(date, amount, subtract, exceptions)
            }
        }
        .map(|date| date.to_string())
        .map_err(|error| at_row(error.message()))
    })
}

fn evaluate_csv_function(
    expression: &str,
    x: &str,
    y: Option<&str>,
) -> Result<Option<f64>, &'static str> {
    let parse = |value: &str| evaluate(value).map_err(|_| CsvError::InvalidNumber.message());
    let x = parse(x)?;
    let y = y.map(parse).transpose()?;
    sample_at_coordinates(expression, x, y).map_err(|error| error.message())
}

fn filter_function_rows(input: &str) -> Result<String, CsvError> {
    let invalid = CsvError::FunctionFilterArguments;
    let ((columns, expression, operator, threshold), data) =
        json_argument_and_data::<(Vec<ColumnReference>, String, String, String)>(input, invalid)?;
    let comparison = NumberComparison::parse(operator.trim()).ok_or(invalid)?;
    let threshold = evaluate(&threshold).map_err(|_| invalid)?;
    let function = read_function_rows(data, &columns, &expression, invalid)?;
    Ok(select_function_rows(function, |value| {
        value
            .partial_cmp(&threshold)
            .is_some_and(|order| comparison.accepts(order))
    }))
}

fn filter_near_function_rows(input: &str) -> Result<String, CsvError> {
    let invalid = CsvError::FunctionNearFilterArguments;
    let ((columns, expression, target, tolerance), data) =
        json_argument_and_data::<(Vec<ColumnReference>, String, String, String)>(input, invalid)?;
    let target = evaluate(&target).map_err(|_| invalid)?;
    let tolerance = evaluate(&tolerance).map_err(|_| invalid)?;
    if tolerance < 0.0 {
        return Err(invalid);
    }
    let function = read_function_rows(data, &columns, &expression, invalid)?;
    Ok(select_function_rows(function, |value| {
        (value - target).abs() <= tolerance
    }))
}

fn select_function_rows(function: FunctionRows, accepts: impl Fn(f64) -> bool) -> String {
    let FunctionRows { rows, values } = function;
    let selected = rows[1..]
        .iter()
        .zip(values)
        .filter_map(|(row, value)| value.is_some_and(&accepts).then_some(row));
    write_rows(std::iter::once(&rows[0]).chain(selected), ',')
}

fn sort_function_rows(input: &str) -> Result<String, CsvError> {
    let invalid = CsvError::FunctionSortArguments;
    let ((columns, expression, order), data) =
        json_argument_and_data::<(Vec<ColumnReference>, String, FieldSortOrder)>(input, invalid)?;
    if !order.numeric() {
        return Err(invalid);
    }
    let FunctionRows { rows, values } = read_function_rows(data, &columns, &expression, invalid)?;
    let mut records: Vec<_> = rows[1..].iter().zip(values).collect();
    records.sort_by(|(_, left), (_, right)| match (left, right) {
        (Some(left), Some(right)) => {
            let relative_order = left.partial_cmp(right).expect("有限函数值可比较");
            if order.descending() {
                relative_order.reverse()
            } else {
                relative_order
            }
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    });
    Ok(write_rows(
        std::iter::once(&rows[0]).chain(records.into_iter().map(|(row, _)| row)),
        ',',
    ))
}

fn rank_function_rows(input: &str) -> Result<String, CsvError> {
    let invalid = CsvError::FunctionRankArguments;
    let ((name, columns, expression, order), data) =
        json_argument_and_data::<(String, Vec<ColumnReference>, String, FieldSortOrder)>(
            input, invalid,
        )?;
    if !order.numeric() {
        return Err(invalid);
    }
    let ascending = !order.descending();
    if name.trim().is_empty() || name.contains('\0') {
        return Err(invalid);
    }
    let FunctionRows { rows, values } = read_function_rows(data, &columns, &expression, invalid)?;
    let mut ranks = vec![String::new(); values.len()];
    let mut ranked_values: Vec<_> = values
        .into_iter()
        .enumerate()
        .filter_map(|(index, value)| value.map(|value| (value, index)))
        .collect();
    assign_csv_ranks(
        &mut ranked_values,
        ascending,
        CsvRanking::Competition,
        &mut ranks,
    );
    write_csv_ranks(&rows[0], &rows[1..], &name, ranks)
}

const FUNCTION_SUMMARY_COLUMNS: [&str; 7] = [
    "数据行",
    "有效",
    "空值",
    "最小值",
    "最大值",
    "平均值",
    "中位数",
];

fn summarize_function_rows(input: &str) -> Result<String, CsvError> {
    let invalid = CsvError::FunctionSummaryArguments;
    let ((columns, expression), data) =
        json_argument_and_data::<(Vec<ColumnReference>, String)>(input, invalid)?;
    let FunctionRows { values, .. } = read_function_rows(data, &columns, &expression, invalid)?;
    let total = values.len();
    let values: Vec<_> = values.into_iter().flatten().collect();
    let fields = function_summary_fields(total, &values)?;
    let mut csv = FUNCTION_SUMMARY_COLUMNS.join(",");
    append_bounded_csv_row(
        &mut csv,
        fields.iter().map(String::as_str),
        CsvError::StatisticsOutputTooLarge,
    )?;
    Ok(csv)
}

fn summarize_grouped_function_rows(input: &str) -> Result<String, CsvError> {
    let invalid = CsvError::FunctionGroupSummaryArguments;
    let ((groups, columns, expression), data) =
        json_argument_and_data::<(Vec<ColumnReference>, Vec<ColumnReference>, String)>(
            input, invalid,
        )?;
    if groups.is_empty() {
        return Err(invalid);
    }
    let FunctionRows { rows, values } = read_function_rows(data, &columns, &expression, invalid)?;
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let groups = resolve_distinct_columns(header, &groups, invalid)?;
    let mut csv = String::new();
    let limit = CsvError::StatisticsOutputTooLarge;
    append_bounded_csv_row(
        &mut csv,
        groups
            .iter()
            .map(|&index| header[index].as_str())
            .chain(FUNCTION_SUMMARY_COLUMNS),
        limit,
    )?;
    for positions in grouped_row_positions(records, &groups) {
        let key = &records[positions[0]];
        let finite: Vec<_> = positions
            .iter()
            .filter_map(|&index| values[index])
            .collect();
        let fields = function_summary_fields(positions.len(), &finite)?;
        append_bounded_csv_row(
            &mut csv,
            groups
                .iter()
                .map(|&index| key[index].as_str())
                .chain(fields.iter().map(String::as_str)),
            limit,
        )?;
    }
    Ok(csv)
}

fn function_summary_fields(total: usize, values: &[f64]) -> Result<Vec<String>, CsvError> {
    let count = values.len();
    let stats = [
        values.iter().copied().min_by(f64::total_cmp),
        values.iter().copied().max_by(f64::total_cmp),
        (count > 0).then(|| average(values)),
        (count > 0).then(|| median(values)),
    ];
    let mut row = vec![
        total.to_string(),
        count.to_string(),
        (total - count).to_string(),
    ];
    for value in stats {
        row.push(
            value
                .map(format_result)
                .transpose()
                .map_err(|_| CsvError::FunctionSummaryRange)?
                .unwrap_or_default(),
        );
    }
    Ok(row)
}

struct FunctionRows {
    rows: Vec<Vec<String>>,
    values: Vec<Option<f64>>,
}

fn read_function_rows(
    data: &str,
    columns: &[ColumnReference],
    expression: &str,
    invalid: CsvError,
) -> Result<FunctionRows, CsvError> {
    if !(1..=2).contains(&columns.len())
        || expression.trim().is_empty()
        || expression.chars().take(QUERY_LIMIT + 1).count() > QUERY_LIMIT
    {
        return Err(invalid);
    }
    let data = data.strip_prefix('\u{feff}').unwrap_or(data);
    if data.is_empty() {
        return Err(CsvError::MissingInput);
    }
    let rows = read_rows(data, ',')?;
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    if records.len() > MAX_FUNCTION_RECORDS {
        return Err(CsvError::FunctionColumnLimit);
    }
    let left = columns[0].resolve(header)?;
    let right = columns
        .get(1)
        .map(|column| column.resolve(header))
        .transpose()?;
    validate_column_width(&[left, right.unwrap_or(left)], header.len())?;
    let mut values = Vec::with_capacity(records.len());
    for (index, row) in records.iter().enumerate() {
        let x = row[left].trim();
        let y = right.map(|column| row[column].trim());
        if x.is_empty() || y.is_some_and(str::is_empty) {
            values.push(None);
            continue;
        }
        let value =
            evaluate_csv_function(expression, x, y).map_err(|cause| CsvError::DataRowValue {
                row: index + 1,
                cause,
            })?;
        values.push(value);
    }
    Ok(FunctionRows { rows, values })
}

fn append_cidr_column(input: &str, use_name: bool) -> Result<String, CsvError> {
    let invalid = CsvError::CidrColumnArguments;
    let ((name, source, networks), data) =
        json_argument_and_data::<(String, ColumnReference, Box<RawValue>)>(input, invalid)?;
    if name.trim().is_empty() || name.contains('\0') {
        return Err(invalid);
    }
    let lookup = LongestPrefixLookup::parse(networks.get()).map_err(CsvError::CidrLookup)?;
    if use_name && !lookup.has_names() {
        return Err(invalid);
    }
    append_transformed_csv_column(data, &name, source, |row, value| {
        let address = normalize(value)
            .trim()
            .parse()
            .map_err(|_| CsvError::DataRowValue {
                row,
                cause: CidrError::Address.message(),
            })?;
        Ok(lookup
            .find(address)
            .map(|(prefix, name)| if use_name { name.to_owned() } else { prefix })
            .unwrap_or_default())
    })
}

enum GroupTimestampReference {
    Previous,
    First,
}

fn append_group_timestamp_offsets(
    input: &str,
    reference: GroupTimestampReference,
) -> Result<String, CsvError> {
    let invalid = CsvError::GroupTimestampDifferenceColumnArguments;
    let ((name, source, groups), data) =
        json_argument_and_data::<(String, ColumnReference, Vec<ColumnReference>)>(input, invalid)?;
    if name.trim().is_empty() || name.contains('\0') {
        return Err(invalid);
    }
    let rows = read_table_for_preview(data, ',')?;
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let source = source.resolve(header)?;
    validate_column_width(&[source], header.len())?;
    let groups = resolve_distinct_columns(header, &groups, CsvError::GroupingColumns)?;
    let mut timestamps = HashMap::new();
    let mut row_number = 0;
    write_appended_column(header, records, &name, |row| {
        row_number += 1;
        let cell = row[source].trim();
        if cell.is_empty() {
            return Ok(String::new());
        }
        let at_row = |cause| CsvError::DataRowValue {
            row: row_number,
            cause,
        };
        let current = absolute_timestamp(cell).map_err(|error| at_row(error.message()))?;
        let key = column_key(row, &groups);
        let origin = match reference {
            GroupTimestampReference::Previous => {
                let Some(previous) = timestamps.insert(key, current) else {
                    return Ok(String::new());
                };
                previous
            }
            GroupTimestampReference::First => *timestamps.entry(key).or_insert(current),
        };
        format_absolute_elapsed(origin, current).map_err(|error| at_row(error.message()))
    })
}

fn append_timestamp_sessions(input: &str) -> Result<String, CsvError> {
    let invalid = CsvError::GroupTimestampSessionArguments;
    let ((name, source, groups, gap), data) =
        json_argument_and_data::<(String, ColumnReference, Vec<ColumnReference>, String)>(
            input, invalid,
        )?;
    let max_gap = duration_nanoseconds(&normalize(&gap)).map_err(|_| invalid)?;
    if name.trim().is_empty() || name.contains('\0') || max_gap < 0 {
        return Err(invalid);
    }
    let rows = read_table_for_preview(data, ',')?;
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let source = source.resolve(header)?;
    validate_column_width(&[source], header.len())?;
    let groups = resolve_distinct_columns(header, &groups, CsvError::GroupingColumns)?;
    let mut sessions = HashMap::new();
    let mut row_number = 0;
    write_appended_column(header, records, &name, |row| {
        row_number += 1;
        let cell = row[source].trim();
        if cell.is_empty() {
            return Ok(String::new());
        }
        let current = absolute_timestamp(cell).map_err(|error| CsvError::DataRowValue {
            row: row_number,
            cause: error.message(),
        })?;
        let (previous, session) = sessions
            .entry(column_key(row, &groups))
            .or_insert((current, 1usize));
        let elapsed = absolute_elapsed_nanoseconds(*previous, current);
        if elapsed < 0 || elapsed > max_gap {
            *session += 1;
        }
        *previous = current;
        Ok(session.to_string())
    })
}

#[derive(Clone, Copy)]
enum TimestampWindowStatistic {
    Count,
    Sum,
    Average,
}

fn append_timestamp_window(
    input: &str,
    statistic: TimestampWindowStatistic,
) -> Result<String, CsvError> {
    let invalid = match statistic {
        TimestampWindowStatistic::Count => CsvError::GroupTimestampWindowArguments,
        _ => CsvError::GroupTimestampWindowValueArguments,
    };
    let (name, source, amount, groups, span, data) =
        if matches!(statistic, TimestampWindowStatistic::Count) {
            let ((name, source, groups, span), data) =
                json_argument_and_data::<(String, ColumnReference, Vec<ColumnReference>, String)>(
                    input, invalid,
                )?;
            (name, source, None, groups, span, data)
        } else {
            let ((name, source, amount, groups, span), data) = json_argument_and_data::<(
                String,
                ColumnReference,
                ColumnReference,
                Vec<ColumnReference>,
                String,
            )>(input, invalid)?;
            (name, source, Some(amount), groups, span, data)
        };
    let window = duration_nanoseconds(&normalize(&span)).map_err(|_| invalid)?;
    if name.trim().is_empty() || name.contains('\0') || window < 0 {
        return Err(invalid);
    }
    let rows = read_table_for_preview(data, ',')?;
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let source = source.resolve(header)?;
    validate_column_width(&[source], header.len())?;
    let amount = amount
        .map(|column| {
            let index = column.resolve(header)?;
            validate_column_width(&[index], header.len())?;
            Ok::<_, CsvError>(index)
        })
        .transpose()?;
    let groups = resolve_distinct_columns(header, &groups, CsvError::GroupingColumns)?;
    let mut timelines: HashMap<_, Vec<_>> = HashMap::new();
    for (index, row) in records.iter().enumerate() {
        let cell = row[source].trim();
        if cell.is_empty() {
            continue;
        }
        let time = absolute_timestamp(cell).map_err(|error| CsvError::DataRowValue {
            row: index + 1,
            cause: error.message(),
        })?;
        let value = amount
            .map(|column| row[column].trim())
            .filter(|cell| !cell.is_empty())
            .map(parse_fraction_literal)
            .transpose()
            .map_err(|error| CsvError::DataRowValue {
                row: index + 1,
                cause: error.message(),
            })?;
        timelines
            .entry(column_key(row, &groups))
            .or_default()
            .push((time, index, value));
    }
    let mut results = vec![String::new(); records.len()];
    let mut result_bytes = 0;
    for timeline in timelines.values_mut() {
        timeline.sort_unstable_by_key(|(time, index, _)| (*time, *index));
        let mut first = 0;
        let mut end = 0;
        let mut sum = BigRational::default();
        let mut value_count = 0;
        for simultaneous in timeline.chunk_by(|left, right| left.0 == right.0) {
            let time = simultaneous[0].0;
            while first < end && absolute_elapsed_nanoseconds(timeline[first].0, time) > window {
                if let Some(value) = &timeline[first].2 {
                    sum -= value;
                    ensure_exact_size(&sum).map_err(|_| CsvError::ComputedNumberRange)?;
                    value_count -= 1;
                }
                first += 1;
            }
            end += simultaneous.len();
            for (_, _, value) in simultaneous {
                if let Some(value) = value {
                    sum += value;
                    ensure_exact_size(&sum).map_err(|_| CsvError::ComputedNumberRange)?;
                    value_count += 1;
                }
            }
            let result = match statistic {
                TimestampWindowStatistic::Count => (end - first).to_string(),
                _ if value_count == 0 => String::new(),
                TimestampWindowStatistic::Sum => sum.to_string(),
                TimestampWindowStatistic::Average => {
                    let mean = &sum / BigInt::from(value_count);
                    ensure_exact_size(&mean).map_err(|_| CsvError::ComputedNumberRange)?;
                    mean.to_string()
                }
            };
            result_bytes += result.len() * simultaneous.len();
            if result_bytes > MAX_EXPANDED_CSV_BYTES {
                return Err(CsvError::AddedColumnTooLarge);
            }
            for &(_, index, _) in simultaneous {
                results[index].clone_from(&result);
            }
        }
    }
    let mut results = results.into_iter();
    write_appended_column(header, records, &name, |_| {
        Ok(results.next().unwrap_or_default())
    })
}

fn append_from_one_column(
    rows: &[Vec<String>],
    name: &str,
    source: ColumnReference,
    compute: impl Fn(&str) -> Result<String, &'static str>,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let source = source.resolve(header)?;
    validate_column_width(&[source], header.len())?;
    let mut row_number = 0;
    write_appended_column(header, records, name, |row| {
        row_number += 1;
        let value = row[source].trim_start();
        if value.is_empty() {
            return Ok(String::new());
        }
        compute(value).map_err(|cause| CsvError::DataRowValue {
            row: row_number,
            cause,
        })
    })
}

fn append_from_two_columns(
    rows: &[Vec<String>],
    name: &str,
    start: ColumnReference,
    end: ColumnReference,
    bad_arguments: CsvError,
    compute: impl Fn(&str, &str) -> Result<String, &'static str>,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    if name.trim().is_empty() || name.contains('\0') {
        return Err(bad_arguments);
    }
    let start = start.resolve(header)?;
    let end = end.resolve(header)?;
    validate_column_width(&[start, end], header.len())?;
    let mut row_number = 0;
    write_appended_column(header, records, name, |row| {
        row_number += 1;
        let (start, end) = (row[start].trim(), row[end].trim());
        if start.is_empty() || end.is_empty() {
            return Ok(String::new());
        }
        compute(start, end).map_err(|cause| CsvError::DataRowValue {
            row: row_number,
            cause,
        })
    })
}

fn append_converted_unit_column<F, T>(
    rows: &[Vec<String>],
    name: &str,
    source: ColumnReference,
    target: &str,
    converter: impl FnOnce(&str) -> Result<F, ConversionError>,
) -> Result<String, CsvError>
where
    F: Fn(&str) -> Result<T, ConversionError>,
    T: Display,
{
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    if name.trim().is_empty() || name.contains('\0') {
        return Err(CsvError::UnitConversionColumnArguments);
    }
    let source = source.resolve(header)?;
    validate_column_width(&[source], header.len())?;
    let convert = converter(target).map_err(CsvError::UnitConversionTarget)?;
    let mut row_number = 0;
    write_appended_column(header, records, name, |row| {
        row_number += 1;
        let value = row[source].trim();
        if value.is_empty() {
            return Ok(String::new());
        }
        convert(value)
            .map(|value| value.to_string())
            .map_err(|cause| CsvError::DataRowValue {
                row: row_number,
                cause: cause.message(),
            })
    })
}

fn running_column(
    rows: &[Vec<String>],
    name: &str,
    source: ColumnReference,
    groups: &[ColumnReference],
    computation: RunningColumnComputation,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    if name.trim().is_empty() || name.contains('\0') {
        return Err(CsvError::GroupedColumnArguments);
    }
    let source = source.resolve(header)?;
    validate_column_width(&[source], header.len())?;
    let groups = resolve_distinct_columns(header, groups, CsvError::GroupingColumns)?;
    let mut group_states = HashMap::new();
    let mut row_number = 0;
    write_appended_column(header, records, name, |row| {
        row_number += 1;
        let error_at_row = |cause| CsvError::DataRowValue {
            row: row_number,
            cause,
        };
        let value = row[source].trim();
        let value = (!value.is_empty())
            .then(|| parse_fraction_literal(value))
            .transpose()
            .map_err(|error| error_at_row(error.message()))?;
        let state = group_states
            .entry(column_key(row, &groups))
            .or_insert_with(|| match &computation {
                RunningColumnComputation::Difference | RunningColumnComputation::RelativeChange => {
                    RunningColumnState::Previous(None)
                }
                RunningColumnComputation::Sum | RunningColumnComputation::Average => {
                    RunningColumnState::Accumulation(RunningTotals::default())
                }
                RunningColumnComputation::Moving { width, average } => RunningColumnState::Moving {
                    values: VecDeque::new(),
                    sum: BigRational::default(),
                    width: *width,
                    average: *average,
                },
                RunningColumnComputation::MovingStatistic { width, statistic } => {
                    RunningColumnState::MovingStatistic {
                        values: VecDeque::new(),
                        width: *width,
                        statistic,
                    }
                }
            });
        match state {
            RunningColumnState::Accumulation(total) => {
                if let Some(value) = value {
                    total
                        .push(value)
                        .map_err(|error| error_at_row(error.message()))?;
                }
                total
                    .format(matches!(computation, RunningColumnComputation::Average))
                    .map_err(|error| error_at_row(error.message()))
            }
            RunningColumnState::Previous(previous) => {
                let Some(value) = value else {
                    return Ok(String::new());
                };
                let Some(prior) = previous.as_ref() else {
                    *previous = Some(value);
                    return Ok(String::new());
                };
                let mut change = &value - prior;
                if matches!(computation, RunningColumnComputation::RelativeChange) {
                    if prior.numer().sign() == Sign::NoSign {
                        return Err(error_at_row(StatisticsError::ZeroPreviousValue.message()));
                    }
                    change /= prior;
                }
                *previous = Some(value);
                ensure_exact_size(&change).map_err(|error| error_at_row(error.message()))?;
                Ok(change.to_string())
            }
            RunningColumnState::Moving {
                values,
                sum,
                width,
                average,
            } => {
                let Some(value) = value else {
                    return Ok(String::new());
                };
                if values.len() == *width {
                    *sum -= values.pop_front().expect("非空窗口应有首项");
                }
                *sum += &value;
                ensure_exact_size(sum).map_err(|error| error_at_row(error.message()))?;
                values.push_back(value);
                if values.len() < *width {
                    return Ok(String::new());
                }
                if !*average {
                    return Ok(sum.to_string());
                }
                let mean = &*sum / BigInt::from(*width);
                ensure_exact_size(&mean).map_err(|error| error_at_row(error.message()))?;
                Ok(mean.to_string())
            }
            RunningColumnState::MovingStatistic {
                values,
                width,
                statistic,
            } => {
                let Some(value) = value else {
                    return Ok(String::new());
                };
                if values.len() == *width {
                    let _ = values.pop_front();
                }
                values.push_back(value);
                if values.len() < *width {
                    return Ok(String::new());
                }
                match statistic {
                    MovingColumnStatistic::Quantile(probability) => {
                        let mut ordered: Vec<_> = values.iter().collect();
                        ordered.sort_unstable();
                        let quantile =
                            interpolate_ordered_quantile(*width as u64, probability, |rank| {
                                ordered[rank as usize]
                            })
                            .map_err(|error| error_at_row(error.message()))?;
                        Ok(quantile.to_string())
                    }
                    MovingColumnStatistic::Variance { sample, root } => {
                        let variance =
                            variance_unweighted(values.iter().cloned().collect(), *sample)
                                .map_err(|error| error_at_row(error.message()))?
                                .expect("已检查样本窗口至少包含两个数值");
                        if *root {
                            return Ok(format_square_root(&variance));
                        }
                        Ok(variance.to_string())
                    }
                }
            }
        }
    })
}

fn flag_outlier_column(
    rows: &[Vec<String>],
    name: &str,
    source: ColumnReference,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    if name.trim().is_empty() || name.contains('\0') {
        return Err(CsvError::OutlierColumnArguments);
    }
    let source = source.resolve(header)?;
    validate_column_width(&[source], header.len())?;
    let mut samples = Vec::new();
    let mut values = Vec::with_capacity(records.len());
    for (index, row) in records.iter().enumerate() {
        let cell = row[source].trim();
        if cell.is_empty() {
            values.push(None);
            continue;
        }
        let value = parse_fraction_literal(cell).map_err(|error| CsvError::DataRowValue {
            row: index + 1,
            cause: error.message(),
        })?;
        samples.push(value.clone());
        values.push(Some(value));
    }
    if samples.is_empty() {
        return write_appended_column(header, records, name, |_| Ok(""));
    }
    let (lower, upper) = unweighted_box_fences(samples).map_err(CsvError::Statistics)?;
    let mut values = values.into_iter();
    write_appended_column(header, records, name, |_| {
        Ok(match values.next().flatten() {
            None => "",
            Some(value) if value < lower || value > upper => "true",
            Some(_) => "false",
        })
    })
}

fn rank_column(
    rows: &[Vec<String>],
    name: &str,
    source: ColumnReference,
    groups: &[ColumnReference],
    ascending: bool,
    ranking: CsvRanking,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    if name.trim().is_empty() || name.contains('\0') {
        return Err(CsvError::GroupedColumnArguments);
    }
    let source = source.resolve(header)?;
    validate_column_width(&[source], header.len())?;
    let groups = resolve_distinct_columns(header, groups, CsvError::GroupingColumns)?;
    let mut grouped_rows: HashMap<_, Vec<_>> = HashMap::new();
    for (index, row) in records.iter().enumerate() {
        let cell = row[source].trim();
        if cell.is_empty() {
            continue;
        }
        let value = parse_fraction_literal(cell).map_err(|error| CsvError::DataRowValue {
            row: index + 1,
            cause: error.message(),
        })?;
        grouped_rows
            .entry(column_key(row, &groups))
            .or_default()
            .push((value, index));
    }
    let mut ranks = vec![String::new(); records.len()];
    for group in grouped_rows.values_mut() {
        assign_csv_ranks(group, ascending, ranking, &mut ranks);
    }
    write_csv_ranks(header, records, name, ranks)
}

fn assign_csv_ranks<Value: PartialOrd>(
    group: &mut [(Value, usize)],
    ascending: bool,
    ranking: CsvRanking,
    ranks: &mut [String],
) {
    group.sort_unstable_by(|left, right| {
        let order = left.0.partial_cmp(&right.0).expect("参与排名的数值可比较");
        if ascending {
            order
        } else {
            order.reverse()
        }
    });
    let total = group.len();
    let mut earlier = 0;
    for (position, peers) in group.chunk_by(|left, right| left.0 == right.0).enumerate() {
        let end = earlier + peers.len();
        let rank = match ranking {
            CsvRanking::Competition => (earlier + 1).to_string(),
            CsvRanking::Dense => (position + 1).to_string(),
            CsvRanking::PercentRank if total == 1 => "0".into(),
            CsvRanking::PercentRank => {
                BigRational::new(earlier.into(), (total - 1).into()).to_string()
            }
            CsvRanking::CumulativeDistribution => {
                BigRational::new(end.into(), total.into()).to_string()
            }
        };
        for (_, index) in peers {
            ranks[*index].clone_from(&rank);
        }
        earlier = end;
    }
}

fn write_csv_ranks(
    header: &[String],
    records: &[Vec<String>],
    name: &str,
    ranks: Vec<String>,
) -> Result<String, CsvError> {
    let mut ranks = ranks.into_iter();
    write_appended_column(header, records, name, |_| {
        Ok(ranks.next().unwrap_or_default())
    })
}

fn share_column(
    rows: &[Vec<String>],
    name: &str,
    source: ColumnReference,
    groups: &[ColumnReference],
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    if name.trim().is_empty() || name.contains('\0') {
        return Err(CsvError::GroupedColumnArguments);
    }
    let source = source.resolve(header)?;
    validate_column_width(&[source], header.len())?;
    let groups = resolve_distinct_columns(header, groups, CsvError::GroupingColumns)?;
    let mut totals: HashMap<_, BigRational> = HashMap::new();
    let mut values = Vec::with_capacity(records.len());
    for (index, record) in records.iter().enumerate() {
        let cell = record[source].trim();
        if cell.is_empty() {
            values.push(None);
            continue;
        }
        let data_error = |cause| CsvError::DataRowValue {
            row: index + 1,
            cause,
        };
        let value = parse_fraction_literal(cell).map_err(|error| data_error(error.message()))?;
        let total = totals.entry(column_key(record, &groups)).or_default();
        *total += &value;
        ensure_exact_size(total).map_err(|error| data_error(error.message()))?;
        values.push(Some(value));
    }
    let mut row_number = 0;
    write_appended_column(header, records, name, |record| {
        let value = values[row_number].as_ref();
        row_number += 1;
        let Some(value) = value else {
            return Ok(String::new());
        };
        let total = &totals[&column_key(record, &groups)];
        if total.numer().sign() == Sign::NoSign {
            return Err(CsvError::DataRowValue {
                row: row_number,
                cause: "同组数值合计为零",
            });
        }
        let share = value / total;
        ensure_exact_size(&share).map_err(|error| CsvError::DataRowValue {
            row: row_number,
            cause: error.message(),
        })?;
        Ok(share.to_string())
    })
}

fn shifted_csv_column(
    rows: &[Vec<String>],
    name: &str,
    source: ColumnReference,
    distance: usize,
    groups: &[ColumnReference],
    following: bool,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    if name.trim().is_empty() || name.contains('\0') {
        return Err(CsvError::ShiftedColumnArguments);
    }
    let source = source.resolve(header)?;
    validate_column_width(&[source], header.len())?;
    let groups = resolve_distinct_columns(header, groups, CsvError::ShiftedColumnArguments)?;
    let mut shifted = vec![""; records.len()];
    for positions in grouped_row_positions(records, &groups) {
        for (position, &record_index) in positions.iter().enumerate() {
            let target = if following {
                position.checked_add(distance)
            } else {
                position.checked_sub(distance)
            };
            if let Some(&target) = target.and_then(|position| positions.get(position)) {
                shifted[record_index] = &records[target][source];
            }
        }
    }
    let mut shifted = shifted.into_iter();
    write_appended_column(header, records, name, |_| {
        Ok(shifted.next().unwrap_or_default())
    })
}

fn grouped_count_column(
    rows: &[Vec<String>],
    name: &str,
    groups: &[ColumnReference],
    running: bool,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    if name.trim().is_empty() || name.contains('\0') {
        return Err(CsvError::GroupCountColumnArguments);
    }
    let groups = resolve_distinct_columns(header, groups, CsvError::GroupCountColumnArguments)?;
    let mut counts = HashMap::<_, usize>::new();
    if !running {
        for row in records {
            *counts.entry(column_key(row, &groups)).or_default() += 1;
        }
    }
    write_appended_column(header, records, name, |row| {
        let key = column_key(row, &groups);
        let count = if running {
            let count = counts.entry(key).or_default();
            *count += 1;
            *count
        } else {
            counts[&key]
        };
        Ok(count.to_string())
    })
}

#[derive(Clone, Copy)]
enum ContiguousMeasurement {
    Group,
    Position,
    Length,
}

fn contiguous_group_column(
    rows: &[Vec<String>],
    name: &str,
    groups: &[ColumnReference],
    measurement: ContiguousMeasurement,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    if name.trim().is_empty() || name.contains('\0') {
        return Err(CsvError::GroupCountColumnArguments);
    }
    let groups = resolve_distinct_columns(header, groups, CsvError::GroupCountColumnArguments)?;
    let mut values = contiguous_csv_groups(records, &groups)
        .enumerate()
        .flat_map(|(index, group)| {
            let length = group.len();
            (1..=length).map(move |position| match measurement {
                ContiguousMeasurement::Group => index + 1,
                ContiguousMeasurement::Position => position,
                ContiguousMeasurement::Length => length,
            })
        });
    write_appended_column(header, records, name, |_| {
        let value = values.next().expect("每条记录均对应一个段内数值");
        Ok(value.to_string())
    })
}

fn contiguous_csv_groups<'a>(
    records: &'a [Vec<String>],
    groups: &'a [usize],
) -> impl Iterator<Item = &'a [Vec<String>]> + 'a {
    records.chunk_by(move |left, right| groups.iter().all(|&column| left[column] == right[column]))
}

pub(super) fn append_transformed_csv_column<E: Copy + From<CsvError>>(
    input: &str,
    name: &str,
    source: ColumnReference,
    mut transform: impl FnMut(usize, &str) -> Result<String, E>,
) -> Result<String, E> {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    if input.is_empty() {
        return Err(CsvError::MissingInput.into());
    }
    let rows = read_rows(input, ',')?;
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let source = source.resolve(header)?;
    validate_column_width(&[source], header.len())?;
    let mut row_number = 0;
    write_appended_column(header, records, name, |row| {
        row_number += 1;
        let value = &row[source];
        if value.trim().is_empty() {
            return Ok(String::new());
        }
        transform(row_number, value)
    })
}

fn write_appended_column<'a, T: AsRef<str>, E: Copy + From<CsvError>>(
    header: &[String],
    records: &'a [Vec<String>],
    name: &str,
    mut cell_for: impl FnMut(&'a [String]) -> Result<T, E>,
) -> Result<String, E> {
    write_expanded_column(header, records, name, |row| {
        cell_for(row).map(std::iter::once)
    })
}

fn write_expanded_column<'a, T: AsRef<str>, I: IntoIterator<Item = T>, E: Copy + From<CsvError>>(
    header: &[String],
    records: &'a [Vec<String>],
    name: &str,
    mut cells_for: impl FnMut(&'a [String]) -> Result<I, E>,
) -> Result<String, E> {
    if header.iter().any(|cell| cell == name) {
        return Err(CsvError::HeaderNameConflict.into());
    }
    let mut output = String::new();
    append_bounded_csv_row(
        &mut output,
        header
            .iter()
            .map(String::as_str)
            .chain(std::iter::once(name)),
        E::from(CsvError::AddedColumnTooLarge),
    )?;
    for row in records {
        for cell in cells_for(row)? {
            append_bounded_csv_row(
                &mut output,
                row.iter()
                    .map(String::as_str)
                    .chain(std::iter::once(cell.as_ref())),
                E::from(CsvError::AddedColumnTooLarge),
            )?;
        }
    }
    Ok(output)
}

fn split_column(
    rows: &[Vec<String>],
    source: ColumnReference,
    names: &[String],
    separator: &str,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let mut seen = HashSet::new();
    if names.len() < 2
        || separator.is_empty()
        || separator.contains('\0')
        || names
            .iter()
            .any(|name| name.trim().is_empty() || name.contains('\0') || !seen.insert(name))
    {
        return Err(CsvError::SplitColumnArguments);
    }
    if names.iter().any(|name| header.contains(name)) {
        return Err(CsvError::HeaderNameConflict);
    }
    let source = source.resolve(header)?;
    validate_column_width(&[source], header.len())?;
    let mut output = String::new();
    append_bounded_csv_row(
        &mut output,
        header.iter().chain(names).map(String::as_str),
        CsvError::AddedColumnTooLarge,
    )?;
    for row in records {
        let parts = row[source]
            .splitn(names.len(), separator)
            .chain(std::iter::repeat(""))
            .take(names.len());
        append_bounded_csv_row(
            &mut output,
            row.iter().map(String::as_str).chain(parts),
            CsvError::AddedColumnTooLarge,
        )?;
    }
    Ok(output)
}

fn split_csv_rows(
    rows: &[Vec<String>],
    source: ColumnReference,
    separator: &str,
) -> Result<String, CsvError> {
    if separator.is_empty() || separator.contains('\0') {
        return Err(CsvError::SplitRowsArguments);
    }
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let source = source.resolve(header)?;
    validate_column_width(&[source], header.len())?;
    let too_large = CsvError::SplitRowsTooLarge;
    let mut output = String::new();
    append_bounded_csv_row(&mut output, header.iter().map(String::as_str), too_large)?;
    for row in records {
        for part in row[source].split(separator) {
            let fields = row.iter().enumerate().map(|(column, value)| {
                if column == source {
                    part
                } else {
                    value.as_str()
                }
            });
            append_bounded_csv_row(&mut output, fields, too_large)?;
        }
    }
    Ok(output)
}

fn compute_column(
    mut rows: Vec<Vec<String>>,
    name: String,
    left: ColumnReference,
    right: Option<ColumnReference>,
    arithmetic: ColumnArithmetic,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first_mut().ok_or(CsvError::MissingInput)?;
    if name.trim().is_empty() || name.contains('\0') {
        return Err(CsvError::ComputedColumnArguments);
    }
    let left = left.resolve(header)?;
    let right = right.map(|column| column.resolve(header)).transpose()?;
    validate_column_width(&[left, right.unwrap_or(left)], header.len())?;
    if header.contains(&name) {
        return Err(CsvError::HeaderNameConflict);
    }
    if matches!(arithmetic, ColumnArithmetic::Formula(_)) && records.len() > MAX_FORMULA_RECORDS {
        return Err(CsvError::FormulaColumnLimit);
    }
    header.push(name);
    for row in records {
        let left_value = parse_fraction_literal(&row[left]).map_err(|_| CsvError::InvalidNumber)?;
        let right_value = right
            .map(|column| parse_fraction_literal(&row[column]))
            .transpose()
            .map_err(|_| CsvError::InvalidNumber)?;
        let result = arithmetic.apply(left_value, right_value)?;
        ensure_exact_size(&result).map_err(|_| CsvError::ComputedNumberRange)?;
        row.push(result.to_string());
    }
    Ok(write_rows(&rows, ','))
}

fn deduplicate_by_columns<'a, Key: Eq + Hash>(
    rows: &'a [Vec<String>],
    columns: &[ColumnReference],
    keep_last: bool,
    key: impl Fn(&'a [String], &[usize]) -> Result<Key, CsvError>,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let columns = resolve_distinct_columns(header, columns, CsvError::NamedColumnArguments)?;
    let mut selected = HashMap::new();
    let mut keep = vec![true; records.len()];
    for (position, row) in records.iter().enumerate() {
        match selected.entry(key(row, &columns)?) {
            Entry::Occupied(mut previous) if keep_last => {
                keep[previous.insert(position)] = false;
            }
            Entry::Occupied(_) => keep[position] = false,
            Entry::Vacant(entry) => {
                entry.insert(position);
            }
        }
    }
    let unique_records = records
        .iter()
        .zip(keep)
        .filter_map(|(row, keep)| keep.then_some(row));
    Ok(write_rows(
        std::iter::once(header).chain(unique_records),
        ',',
    ))
}

fn find_duplicate_columns<'a, Key: Eq + Hash>(
    rows: &'a [Vec<String>],
    columns: &[ColumnReference],
    key: impl Fn(&'a [String], &[usize]) -> Result<Key, CsvError>,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let columns = resolve_distinct_columns(header, columns, CsvError::NamedColumnArguments)?;
    let mut recent = HashMap::new();
    let mut duplicates = vec![false; records.len()];
    for (position, row) in records.iter().enumerate() {
        if let Some(previous) = recent.insert(key(row, &columns)?, position) {
            duplicates[previous] = true;
            duplicates[position] = true;
        }
    }
    let duplicate_rows = records
        .iter()
        .zip(duplicates)
        .filter_map(|(row, duplicate)| duplicate.then_some(row));
    Ok(write_rows(
        std::iter::once(header).chain(duplicate_rows),
        ',',
    ))
}

struct CsvFilterQuery<'a> {
    data: &'a str,
    filters: Vec<ColumnFilter>,
    any: bool,
}

fn csv_filter_preview(operation: &str, input: &str) -> Option<Result<CsvPreviewData, CsvError>> {
    let operation = operation.to_ascii_lowercase();
    let parsed = match operation.as_str() {
        "筛选" | "filter" | "排除" | "exclude" | "正则筛选" | "regex-filter" | "正则排除"
        | "regex-exclude" => text_csv_filter(&operation, input),
        "数值筛选" | "filter-numeric" => numeric_csv_filter(input),
        "多条件筛选" | "filter-all" | "任一条件筛选" | "filter-any" => {
            combined_csv_filter(
                input,
                matches!(operation.as_str(), "任一条件筛选" | "filter-any"),
            )
        }
        _ => return None,
    };
    Some(parsed.and_then(|query| {
        let rows = read_table_for_preview(query.data, ',')?;
        let rows = filter_multiple_columns(rows, &query.filters, query.any)?;
        Ok((write_rows(&rows, ','), rows))
    }))
}

fn text_csv_filter<'a>(operation: &str, input: &'a str) -> Result<CsvFilterQuery<'a>, CsvError> {
    let (column, value, data) = column_value_arguments(input)?;
    let condition = match operation {
        "筛选" | "filter" | "排除" | "exclude" => ColumnCondition::Exact(value),
        _ => ColumnCondition::Regex(compile_regex(&value).map_err(CsvError::Regex)?),
    };
    let expected = matches!(operation, "筛选" | "filter" | "正则筛选" | "regex-filter");
    Ok(CsvFilterQuery {
        data,
        filters: vec![ColumnFilter {
            column,
            condition,
            expected,
        }],
        any: false,
    })
}

fn numeric_csv_filter(input: &str) -> Result<CsvFilterQuery<'_>, CsvError> {
    let invalid = CsvError::InvalidNumberFilter;
    let (column, rest) = column_reference_argument(input.trim_start(), invalid)?;
    let (operator, rest) = split_argument(rest.trim_start()).ok_or(invalid)?;
    let (threshold, data) = split_argument(rest.trim_start()).ok_or(invalid)?;
    let comparison = NumberComparison::parse(operator).ok_or(invalid)?;
    let threshold = parse_fraction_literal(threshold).map_err(|_| CsvError::InvalidNumber)?;
    Ok(CsvFilterQuery {
        data,
        filters: vec![ColumnFilter {
            column,
            condition: ColumnCondition::Number(comparison, threshold),
            expected: true,
        }],
        any: false,
    })
}

fn combined_csv_filter(input: &str, any: bool) -> Result<CsvFilterQuery<'_>, CsvError> {
    let (filters, data) = json_argument_and_data::<Vec<(ColumnReference, String, String)>>(
        input,
        CsvError::FilterConditions,
    )?;
    if !(1..=MAX_FILTER_CONDITIONS).contains(&filters.len()) {
        return Err(CsvError::FilterConditions);
    }
    let filters = filters
        .into_iter()
        .map(|(column, operator, value)| {
            let operator = operator.to_ascii_lowercase();
            let (condition, expected) = match operator.as_str() {
                "文本=" | "text=" => (ColumnCondition::Exact(value), true),
                "文本!=" | "text!=" => (ColumnCondition::Exact(value), false),
                "正则" | "regex" => (
                    ColumnCondition::Regex(compile_regex(&value).map_err(CsvError::Regex)?),
                    true,
                ),
                "正则排除" | "regex-not" => (
                    ColumnCondition::Regex(compile_regex(&value).map_err(CsvError::Regex)?),
                    false,
                ),
                _ => {
                    let comparison =
                        NumberComparison::parse(&operator).ok_or(CsvError::FilterConditions)?;
                    let threshold =
                        parse_fraction_literal(&value).map_err(|_| CsvError::InvalidNumber)?;
                    (ColumnCondition::Number(comparison, threshold), true)
                }
            };
            Ok(ColumnFilter {
                column,
                condition,
                expected,
            })
        })
        .collect::<Result<Vec<_>, CsvError>>()?;
    Ok(CsvFilterQuery { data, filters, any })
}

fn filter_multiple_columns(
    rows: Vec<Vec<String>>,
    filters: &[ColumnFilter],
    any: bool,
) -> Result<Vec<Vec<String>>, CsvError> {
    let mut records = rows.into_iter();
    let header = records.next().ok_or(CsvError::MissingInput)?;
    let columns = filters
        .iter()
        .map(|filter| filter.column.resolve(&header))
        .collect::<Result<Vec<_>, _>>()?;
    validate_column_width(&columns, header.len())?;
    let mut selected = vec![header];
    for row in records {
        let mut matched = !any;
        for (&column, filter) in columns.iter().zip(filters) {
            // 即使已有筛选结论，也检查该记录的其余数值条件。
            let accepted = filter.condition.matches(&row[column])? == filter.expected;
            matched = if any {
                matched || accepted
            } else {
                matched && accepted
            };
        }
        if matched {
            selected.push(row);
        }
    }
    Ok(selected)
}

fn resolve_distinct_columns(
    header: &[String],
    columns: &[ColumnReference],
    duplicate: CsvError,
) -> Result<Vec<usize>, CsvError> {
    let mut seen = HashSet::new();
    let columns = columns
        .iter()
        .map(|column| {
            let index = column.resolve(header)?;
            if !seen.insert(index) {
                return Err(duplicate);
            }
            Ok(index)
        })
        .collect::<Result<Vec<_>, _>>()?;
    validate_column_width(&columns, header.len())?;
    Ok(columns)
}

fn count_by_columns(rows: &[Vec<String>], columns: &[ColumnReference]) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let columns = resolve_distinct_columns(header, columns, CsvError::GroupingColumns)?;
    Ok(grouped_csv(
        header,
        &columns,
        group_counts(records, &columns),
        "计数",
    ))
}

fn group_counts<'a>(
    records: &'a [Vec<String>],
    columns: &[usize],
) -> HashMap<Vec<&'a str>, (usize, usize)> {
    let mut groups = HashMap::new();
    for (position, row) in records.iter().enumerate() {
        groups
            .entry(column_key(row, columns))
            .and_modify(|(_, count)| *count += 1)
            .or_insert((position, 1_usize));
    }
    groups
}

struct GroupFrequency<'a> {
    key: Vec<&'a str>,
    count: usize,
}

struct GroupFrequencies<'a> {
    headings: Vec<&'a str>,
    groups: Vec<GroupFrequency<'a>>,
    observations: usize,
}

fn group_frequencies<'a>(
    rows: &'a [Vec<String>],
    columns: &[ColumnReference],
) -> Result<GroupFrequencies<'a>, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let columns = resolve_distinct_columns(header, columns, CsvError::GroupingColumns)?;
    let mut groups: Vec<_> = group_counts(records, &columns).into_iter().collect();
    groups.sort_unstable_by_key(|(_, (position, count))| (std::cmp::Reverse(*count), *position));
    Ok(GroupFrequencies {
        headings: columns
            .iter()
            .map(|&index| header[index].as_str())
            .collect(),
        groups: groups
            .into_iter()
            .map(|(key, (_, count))| GroupFrequency { key, count })
            .collect(),
        observations: records.len(),
    })
}

impl GroupFrequencies<'_> {
    fn csv(&self) -> Result<String, CsvError> {
        let mut output = String::new();
        let limit = CsvError::StatisticsOutputTooLarge;
        append_bounded_csv_row(
            &mut output,
            self.headings.iter().copied().chain(["次数", "频率"]),
            limit,
        )?;
        for group in &self.groups {
            let fraction =
                BigRational::new(group.count.into(), self.observations.into()).to_string();
            let count = group.count.to_string();
            append_bounded_csv_row(
                &mut output,
                group
                    .key
                    .iter()
                    .copied()
                    .chain([count.as_str(), fraction.as_str()]),
                limit,
            )?;
        }
        Ok(output)
    }

    fn plot(&self) -> Result<BinPlot, CsvError> {
        const MAX_VISIBLE_GROUPS: usize = 8;
        if self.groups.is_empty() {
            return Err(CsvError::EmptyStatisticsColumn);
        }
        let (labels, counts) = self
            .groups
            .iter()
            .take(MAX_VISIBLE_GROUPS)
            .map(|group| {
                let label = group
                    .key
                    .iter()
                    .map(|value| csv_cell(value))
                    .collect::<Vec<_>>()
                    .join(",");
                (label, group.count as u64)
            })
            .unzip();
        Ok(
            BinPlot::from_categories(labels, counts, self.observations as u64, self.groups.len())
                .with_label(self.headings.join(" / ")),
        )
    }
}

fn aggregate_by_columns(
    rows: &[Vec<String>],
    columns: &[ColumnReference],
    amount: ColumnReference,
    aggregation: ColumnAggregation,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let columns = resolve_distinct_columns(header, columns, CsvError::GroupingColumns)?;
    let amount = amount.resolve(header)?;
    validate_column_width(&[amount], header.len())?;
    let mut groups = HashMap::new();
    for (position, row) in records.iter().enumerate() {
        let value = parse_fraction_literal(&row[amount]).map_err(|_| CsvError::InvalidNumber)?;
        let (_, (aggregate, count, minimum)) = groups
            .entry(column_key(row, &columns))
            .or_insert_with(|| (position, (BigRational::default(), 0_usize, None)));
        match aggregation {
            ColumnAggregation::Sum | ColumnAggregation::Average => *aggregate += value,
            ColumnAggregation::Minimum if *count == 0 || value < *aggregate => {
                *aggregate = value;
            }
            ColumnAggregation::Maximum if *count == 0 || value > *aggregate => {
                *aggregate = value;
            }
            ColumnAggregation::Range => match minimum {
                None => {
                    *minimum = Some(value.clone());
                    *aggregate = value;
                }
                Some(minimum) if value < *minimum => *minimum = value,
                Some(_) if value > *aggregate => *aggregate = value,
                _ => {}
            },
            _ => {}
        }
        *count += 1;
        ensure_exact_size(aggregate)
            .map_err(|error| CsvError::Statistics(StatisticsError::Evaluation(error)))?;
    }
    if matches!(
        aggregation,
        ColumnAggregation::Average | ColumnAggregation::Range
    ) {
        for (_, (total, count, minimum)) in groups.values_mut() {
            if matches!(aggregation, ColumnAggregation::Average) {
                *total = std::mem::take(total) / BigInt::from(*count);
            } else {
                *total -= minimum.take().expect("每组至少包含一个数值");
            }
            ensure_exact_size(total)
                .map_err(|error| CsvError::Statistics(StatisticsError::Evaluation(error)))?;
        }
    }
    let groups = groups
        .into_iter()
        .map(|(key, (position, (value, _, _)))| (key, (position, value)));
    let label = match aggregation {
        ColumnAggregation::Sum => "合计",
        ColumnAggregation::Average => "平均值",
        ColumnAggregation::Minimum => "最小值",
        ColumnAggregation::Maximum => "最大值",
        ColumnAggregation::Range => "极差",
    };
    Ok(grouped_csv(header, &columns, groups, label))
}

fn sum_durations_by_columns(
    rows: &[Vec<String>],
    columns: &[ColumnReference],
    duration: ColumnReference,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let columns = resolve_distinct_columns(header, columns, CsvError::GroupingColumns)?;
    let duration = duration.resolve(header)?;
    validate_column_width(&[duration], header.len())?;
    let mut groups = HashMap::<_, (usize, Option<i128>)>::new();
    for (position, row) in records.iter().enumerate() {
        let (_, total) = groups
            .entry(column_key(row, &columns))
            .or_insert((position, None));
        let input = row[duration].trim();
        if input.is_empty() {
            continue;
        }
        let error = |cause: DurationError| CsvError::DataRowValue {
            row: position + 1,
            cause: cause.message(),
        };
        let duration = duration_nanoseconds(input).map_err(error)?;
        *total = Some(
            total
                .unwrap_or_default()
                .checked_add(duration)
                .ok_or_else(|| error(DurationError::OutOfRange))?,
        );
    }
    let groups = groups.into_iter().map(|(key, (position, value))| {
        let total = value.map_or_else(String::new, |nanos| {
            let sign = if nanos.is_negative() { "-" } else { "" };
            format_clock_duration(nanos.unsigned_abs(), sign)
        });
        (key, (position, total))
    });
    Ok(grouped_csv(header, &columns, groups, "时长合计"))
}

fn median_by_columns(
    rows: &[Vec<String>],
    columns: &[ColumnReference],
    amount: ColumnReference,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let columns = resolve_distinct_columns(header, columns, CsvError::GroupingColumns)?;
    let amount = amount.resolve(header)?;
    validate_column_width(&[amount], header.len())?;
    let mut groups = HashMap::new();
    for (position, row) in records.iter().enumerate() {
        let (_, values) = groups
            .entry(column_key(row, &columns))
            .or_insert_with(|| (position, Vec::new()));
        if values.len() == MAX_VALUES {
            return Err(CsvError::Statistics(StatisticsError::TooManyValues));
        }
        let value = parse_fraction_literal(&row[amount]).map_err(|_| CsvError::InvalidNumber)?;
        values.push(value);
    }
    let medians = groups
        .into_iter()
        .map(|(key, (position, mut values))| {
            values.sort_unstable();
            let median = median_sorted_values(&values)
                .map_err(|error| CsvError::Statistics(StatisticsError::Evaluation(error)))?;
            Ok((key, (position, median)))
        })
        .collect::<Result<Vec<_>, CsvError>>()?;
    Ok(grouped_csv(header, &columns, medians, "中位数"))
}

fn is_scatter_operation(operation: &str) -> bool {
    operation == "散点图" || operation.eq_ignore_ascii_case("scatter")
}

pub(super) fn scatter_arguments(input: &str) -> Option<&str> {
    let (operation, rest) = split_argument(input)?;
    is_scatter_operation(operation).then_some(rest)
}

pub(super) fn build_scatter_plot(input: &str) -> Result<(String, ScatterPlot), CsvError> {
    let table = parse_coordinate_table(input, CsvError::ScatterPointCount)?;
    let csv = table.to_csv()?;
    let points: Vec<_> = table
        .samples
        .into_iter()
        .map(|sample| sample.point)
        .collect();
    let plot = ScatterPlot::from_points(&points).with_axis_names(table.headers);
    Ok((csv, plot))
}

struct CsvCoordinates {
    headers: [String; 2],
    samples: Vec<CoordinateSample>,
}

struct CoordinateSample {
    row_index: usize,
    point: Point,
}

fn sort_distinct_coordinates(samples: &mut [CoordinateSample]) -> Result<(), CsvError> {
    samples.sort_unstable_by(|left, right| left.point.0.cmp(&right.point.0));
    if samples
        .windows(2)
        .any(|pair| pair[0].point.0 == pair[1].point.0)
    {
        return Err(CsvError::CoordinateDuplicateX);
    }
    Ok(())
}

impl CsvCoordinates {
    fn to_csv(&self) -> Result<String, CsvError> {
        let mut output = String::new();
        let limit = CsvError::ScatterTooLarge;
        append_bounded_csv_row(&mut output, self.headers.iter().map(String::as_str), limit)?;
        for CoordinateSample { point: (x, y), .. } in &self.samples {
            let coordinates = [x.to_string(), y.to_string()];
            append_bounded_csv_row(&mut output, coordinates.iter().map(String::as_str), limit)?;
        }
        Ok(output)
    }
}

fn parse_coordinate_table(
    input: &str,
    point_count_error: CsvError,
) -> Result<CsvCoordinates, CsvError> {
    let invalid = CsvError::ScatterColumns;
    let (columns, data) = json_argument_and_data::<[ColumnReference; 2]>(input, invalid)?;
    read_coordinate_table(data, &columns, invalid, point_count_error)
}

fn read_coordinate_table(
    data: &str,
    columns: &[ColumnReference; 2],
    invalid: CsvError,
    point_count_error: CsvError,
) -> Result<CsvCoordinates, CsvError> {
    let data = data.strip_prefix('\u{feff}').unwrap_or(data);
    if data.is_empty() {
        return Err(CsvError::MissingInput);
    }
    let rows = read_rows(data, ',')?;
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let columns = resolve_distinct_columns(header, columns, invalid)?;
    let headers = [header[columns[0]].clone(), header[columns[1]].clone()];
    let samples = collect_coordinate_samples(records, &columns, point_count_error)?;
    Ok(CsvCoordinates { headers, samples })
}

fn read_coordinate_queries(
    input: &str,
    point_count_error: CsvError,
) -> Result<(CsvCoordinates, Vec<Box<RawValue>>), CsvError> {
    let invalid = CsvError::CoordinateInterpolationArguments;
    let ((columns, coordinates), data) =
        json_argument_and_data::<([ColumnReference; 2], Vec<Box<RawValue>>)>(input, invalid)?;
    if coordinates.is_empty() || coordinates.len() > MAX_POINTS {
        return Err(invalid);
    }
    let mut table = read_coordinate_table(data, &columns, invalid, point_count_error)?;
    sort_distinct_coordinates(&mut table.samples)?;
    Ok((table, coordinates))
}

fn interpolate_coordinate_table(input: &str) -> Result<String, CsvError> {
    let (mut table, coordinates) =
        read_coordinate_queries(input, CsvError::CoordinateInterpolationPointCount)?;
    let samples = coordinates
        .iter()
        .enumerate()
        .map(|(row_index, raw)| {
            let x = coordinate_argument(raw, CsvError::CoordinateInterpolationArguments)?;
            let y = interpolate_coordinate_value(&table.samples, &x)?;
            Ok(CoordinateSample {
                row_index,
                point: (x, y),
            })
        })
        .collect::<Result<Vec<_>, CsvError>>()?;
    table.samples = samples;
    table.to_csv()
}

fn resample_coordinate_table(input: &str) -> Result<String, CsvError> {
    let invalid = CsvError::CoordinateResampleArguments;
    let ((columns, count), data) =
        json_argument_and_data::<([ColumnReference; 2], usize)>(input, invalid)?;
    if !(2..=MAX_POINTS).contains(&count) {
        return Err(invalid);
    }
    let mut table = read_coordinate_table(
        data,
        &columns,
        invalid,
        CsvError::CoordinateInterpolationPointCount,
    )?;
    sort_distinct_coordinates(&mut table.samples)?;
    let first = &table.samples[0].point.0;
    let last = &table.samples.last().expect("至少有两个完整坐标").point.0;
    let intervals = BigInt::from(count - 1);
    let samples = (0..count)
        .map(|row_index| {
            let progress = BigRational::new(BigInt::from(row_index), intervals.clone());
            let x = interpolate_fraction(first, last, &progress)
                .map_err(|_| CsvError::ComputedNumberRange)?;
            let y = interpolate_coordinate_value(&table.samples, &x)?;
            Ok(CoordinateSample {
                row_index,
                point: (x, y),
            })
        })
        .collect::<Result<Vec<_>, CsvError>>()?;
    table.samples = samples;
    table.to_csv()
}

fn interpolate_coordinate_value(
    samples: &[CoordinateSample],
    x: &BigRational,
) -> Result<BigRational, CsvError> {
    match samples.binary_search_by(|sample| sample.point.0.cmp(x)) {
        Ok(index) => Ok(samples[index].point.1.clone()),
        Err(right) if right > 0 && right < samples.len() => {
            let (left_x, left_y) = &samples[right - 1].point;
            let (right_x, right_y) = &samples[right].point;
            let progress = (x - left_x) / (right_x - left_x);
            interpolate_fraction(left_y, right_y, &progress)
                .map_err(|_| CsvError::ComputedNumberRange)
        }
        Err(_) => Err(CsvError::CoordinateInterpolationRange),
    }
}

fn coordinate_slopes(input: &str) -> Result<String, CsvError> {
    let (table, coordinates) = read_coordinate_queries(input, CsvError::CoordinateSlopePointCount)?;
    let samples = &table.samples;
    let format_slope = |segment: Option<usize>| -> Result<String, CsvError> {
        let Some(index) = segment else {
            return Ok(String::new());
        };
        coordinate_secant_slope(&samples[index].point, &samples[index + 1].point)
            .map(|value| value.to_string())
    };
    let mut csv = String::from("横坐标,左侧斜率,右侧斜率");
    for raw in coordinates {
        let x = coordinate_argument(&raw, CsvError::CoordinateInterpolationArguments)?;
        let [left, right] = match samples.binary_search_by(|sample| sample.point.0.cmp(&x)) {
            Ok(index) => [
                index.checked_sub(1),
                (index + 1 < samples.len()).then_some(index),
            ],
            Err(index) if index > 0 && index < samples.len() => [Some(index - 1); 2],
            Err(_) => return Err(CsvError::CoordinateInterpolationRange),
        };
        let row = [x.to_string(), format_slope(left)?, format_slope(right)?];
        append_bounded_csv_row(
            &mut csv,
            row.iter().map(String::as_str),
            CsvError::StatisticsOutputTooLarge,
        )?;
    }
    Ok(csv)
}

fn coordinate_linear_segments(input: &str) -> Result<String, CsvError> {
    let mut table = parse_coordinate_table(input, CsvError::CoordinateSegmentPointCount)?;
    sort_distinct_coordinates(&mut table.samples)?;
    let mut csv = String::from("起始横坐标,结束横坐标,斜率,截距");
    for pair in table.samples.windows(2) {
        let (left_x, left_y) = &pair[0].point;
        let right_x = &pair[1].point.0;
        let slope = coordinate_secant_slope(&pair[0].point, &pair[1].point)?;
        let intercept = left_y - &slope * left_x;
        ensure_exact_size(&intercept).map_err(|_| CsvError::ComputedNumberRange)?;
        let row = [
            left_x.to_string(),
            right_x.to_string(),
            slope.to_string(),
            intercept.to_string(),
        ];
        append_bounded_csv_row(
            &mut csv,
            row.iter().map(String::as_str),
            CsvError::StatisticsOutputTooLarge,
        )?;
    }
    Ok(csv)
}

fn coordinate_argument(raw: &RawValue, invalid: CsvError) -> Result<BigRational, CsvError> {
    let text = raw.get();
    let text = if text.starts_with('"') {
        Cow::Owned(serde_json::from_str::<String>(text).map_err(|_| invalid)?)
    } else {
        Cow::Borrowed(text)
    };
    parse_fraction_literal(&text).map_err(|_| invalid)
}

fn read_coordinate_level(
    input: &str,
    point_count_error: CsvError,
) -> Result<(CsvCoordinates, BigRational), CsvError> {
    let invalid = CsvError::CoordinateCrossingArguments;
    let ((columns, target), data) =
        json_argument_and_data::<([ColumnReference; 2], Box<RawValue>)>(input, invalid)?;
    let target = coordinate_argument(&target, invalid)?;
    let mut table = read_coordinate_table(data, &columns, invalid, point_count_error)?;
    sort_distinct_coordinates(&mut table.samples)?;
    Ok((table, target))
}

fn coordinate_horizontal_crossings(input: &str) -> Result<String, CsvError> {
    let (table, target) = read_coordinate_level(input, CsvError::CoordinateCrossingPointCount)?;
    horizontal_crossings_csv(&table.samples, &target)
}

enum CoordinateThresholdMeasure {
    Span,
    Area,
}

fn coordinate_threshold_measure(
    input: &str,
    measure: CoordinateThresholdMeasure,
) -> Result<String, CsvError> {
    let (table, target) =
        read_coordinate_level(input, CsvError::CoordinateThresholdMeasurePointCount)?;
    let two = BigInt::from(2u8);
    let mut total = BigRational::default();
    for pair in table.samples.windows(2) {
        let (left_x, left_y) = &pair[0].point;
        let (right_x, right_y) = &pair[1].point;
        let high = left_y.max(right_y);
        if high <= &target {
            continue;
        }
        let width = right_x - left_x;
        let low = left_y.min(right_y);
        let peak = high - &target;
        let included_width = if low >= &target {
            width
        } else {
            width * &peak / (high - low)
        };
        total += match measure {
            CoordinateThresholdMeasure::Span => included_width,
            CoordinateThresholdMeasure::Area => {
                let base = low.max(&target) - &target;
                included_width * (peak + base) / &two
            }
        };
        ensure_exact_size(&total).map_err(|_| CsvError::ComputedNumberRange)?;
    }
    Ok(total.to_string())
}

fn coordinate_intervals_above_threshold(input: &str) -> Result<String, CsvError> {
    let (table, target) =
        read_coordinate_level(input, CsvError::CoordinateThresholdIntervalPointCount)?;
    let mut csv = String::from("起始横坐标,结束横坐标,包含起点,包含终点");
    let mut append_interval =
        |start: &BigRational, end: &BigRational, start_included: bool, end_included: bool| {
            let included = |value| if value { "是" } else { "否" };
            let row = [
                start.to_string(),
                end.to_string(),
                included(start_included).to_owned(),
                included(end_included).to_owned(),
            ];
            append_bounded_csv_row(
                &mut csv,
                row.iter().map(String::as_str),
                CsvError::StatisticsOutputTooLarge,
            )
        };
    let first = &table.samples[0].point;
    let mut interval_start = (first.1 > target).then(|| (first.0.clone(), true));
    for pair in table.samples.windows(2) {
        let left = &pair[0].point;
        let right = &pair[1].point;
        match (left.1 > target, right.1 > target) {
            (false, true) => {
                let start = crossing_inside_segment(left, right, &target)?
                    .unwrap_or_else(|| left.0.clone());
                interval_start = Some((start, false));
            }
            (true, false) => {
                let (start, included) = interval_start.take().expect("超阈区间已开始");
                let crossing = crossing_inside_segment(left, right, &target)?;
                let end = crossing.as_ref().unwrap_or(&right.0);
                append_interval(&start, end, included, false)?;
            }
            _ => {}
        }
    }
    if let Some((start, included)) = interval_start {
        let end = &table.samples.last().expect("至少有两个完整坐标").point.0;
        append_interval(&start, end, included, true)?;
    }
    Ok(csv)
}

fn horizontal_crossings_csv(
    samples: &[CoordinateSample],
    target: &BigRational,
) -> Result<String, CsvError> {
    let mut csv = String::from("起始横坐标,结束横坐标,类型");
    let mut index = 0;
    while let Some(sample) = samples.get(index) {
        let (x, y) = &sample.point;
        let row = if y == target {
            let first = index;
            while samples
                .get(index + 1)
                .is_some_and(|next| next.point.1 == *target)
            {
                index += 1;
            }
            let kind = if first == index {
                "交点"
            } else {
                "重合区间"
            };
            Some([
                x.to_string(),
                samples[index].point.0.to_string(),
                kind.to_owned(),
            ])
        } else if let Some(next) = samples.get(index + 1) {
            crossing_inside_segment(&sample.point, &next.point, target)?
                .map(|x| [x.to_string(), x.to_string(), "交点".to_owned()])
        } else {
            None
        };
        if let Some(row) = row {
            append_bounded_csv_row(
                &mut csv,
                row.iter().map(String::as_str),
                CsvError::ComputedNumberRange,
            )?;
        }
        index += 1;
    }
    Ok(csv)
}

fn crossing_inside_segment(
    (left_x, left_y): &Point,
    (right_x, right_y): &Point,
    target: &BigRational,
) -> Result<Option<BigRational>, CsvError> {
    let left_side = left_y.cmp(target);
    let right_side = right_y.cmp(target);
    if left_side == right_side || left_side == Ordering::Equal || right_side == Ordering::Equal {
        return Ok(None);
    }
    let progress = (target - left_y) / (right_y - left_y);
    interpolate_fraction(left_x, right_x, &progress)
        .map(Some)
        .map_err(|_| CsvError::ComputedNumberRange)
}

fn collect_coordinate_samples(
    records: &[Vec<String>],
    columns: &[usize],
    point_count_error: CsvError,
) -> Result<Vec<CoordinateSample>, CsvError> {
    let mut samples = Vec::new();
    for (index, row) in records.iter().enumerate() {
        let x = row[columns[0]].trim();
        let y = row[columns[1]].trim();
        if x.is_empty() || y.is_empty() {
            continue;
        }
        if samples.len() == MAX_POINTS {
            return Err(point_count_error);
        }
        let parse = |value| {
            parse_fraction_literal(value).map_err(|error| CsvError::DataRowValue {
                row: index + 1,
                cause: error.message(),
            })
        };
        samples.push(CoordinateSample {
            row_index: index,
            point: (parse(x)?, parse(y)?),
        });
    }
    if samples.len() < 2 {
        return Err(point_count_error);
    }
    Ok(samples)
}

struct AlignedCoordinateCurves {
    headers: [String; 3],
    samples: Vec<(BigRational, [BigRational; 3])>,
}

impl AlignedCoordinateCurves {
    fn to_csv(&self, name: &str) -> Result<String, CsvError> {
        let mut csv = String::new();
        let limit = CsvError::ColumnComparisonTooLarge;
        let headers = self
            .headers
            .iter()
            .map(String::as_str)
            .chain(std::iter::once(name));
        append_bounded_csv_row(&mut csv, headers, limit)?;
        for (x, [left_y, right_y, difference]) in &self.samples {
            let row = [
                x.to_string(),
                left_y.to_string(),
                right_y.to_string(),
                difference.to_string(),
            ];
            append_bounded_csv_row(&mut csv, row.iter().map(String::as_str), limit)?;
        }
        Ok(csv)
    }

    fn into_chart(self, name: &str) -> Result<(String, ScatterPlot), CsvError> {
        let csv = self.to_csv(name)?;
        let [x_name, left_name, right_name] = self.headers;
        let series = [left_name, right_name]
            .into_iter()
            .enumerate()
            .map(|(index, label)| SampledSeries {
                label,
                points: self
                    .samples
                    .iter()
                    .map(|(x, y)| (x.clone(), y[index].clone()))
                    .collect(),
            })
            .collect();
        let plot =
            ScatterPlot::from_connected_series(series).with_axis_names([x_name, "数值".to_owned()]);
        Ok((csv, plot))
    }
}

fn read_coordinate_curve_comparison(
    input: &str,
) -> Result<(AlignedCoordinateCurves, String), CsvError> {
    let invalid = CsvError::CoordinateDifferenceArguments;
    let ((columns, name), data) =
        json_argument_and_data::<([ColumnReference; 3], String)>(input, invalid)?;
    if name.trim().is_empty() || name.contains('\0') {
        return Err(invalid);
    }
    let curves = align_coordinate_curves(data, &columns, invalid)?;
    if curves.headers.contains(&name) {
        return Err(invalid);
    }
    Ok((curves, name))
}

fn align_coordinate_curves(
    data: &str,
    columns: &[ColumnReference; 3],
    invalid: CsvError,
) -> Result<AlignedCoordinateCurves, CsvError> {
    let data = data.strip_prefix('\u{feff}').unwrap_or(data);
    let rows = read_rows(data, ',')?;
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let columns = resolve_distinct_columns(header, columns, invalid)?;
    let [x, left_y, right_y] = [columns[0], columns[1], columns[2]];
    let point_count_error = CsvError::CoordinateDifferencePointCount;
    let mut left = collect_coordinate_samples(records, &[x, left_y], point_count_error)?;
    let mut right = collect_coordinate_samples(records, &[x, right_y], point_count_error)?;
    sort_distinct_coordinates(&mut left)?;
    sort_distinct_coordinates(&mut right)?;
    let start = (&left[0].point.0).max(&right[0].point.0);
    let end = (&left.last().expect("至少有两个完整坐标").point.0)
        .min(&right.last().expect("至少有两个完整坐标").point.0);
    if start > end {
        return Err(CsvError::CoordinateDifferenceNoOverlap);
    }
    let mut coordinates: Vec<_> = left
        .iter()
        .chain(&right)
        .map(|sample| &sample.point.0)
        .filter(|&x| x >= start && x <= end)
        .collect();
    coordinates.sort_unstable();
    coordinates.dedup();

    let headers = [
        header[x].clone(),
        header[left_y].clone(),
        header[right_y].clone(),
    ];
    let samples = coordinates
        .into_iter()
        .map(|x| {
            let left_y = interpolate_coordinate_value(&left, x)?;
            let right_y = interpolate_coordinate_value(&right, x)?;
            let difference = &left_y - &right_y;
            ensure_exact_size(&difference).map_err(|_| CsvError::ComputedNumberRange)?;
            Ok((x.clone(), [left_y, right_y, difference]))
        })
        .collect::<Result<Vec<_>, CsvError>>()?;
    Ok(AlignedCoordinateCurves { headers, samples })
}

fn coordinate_curve_crossings(input: &str) -> Result<String, CsvError> {
    horizontal_crossings_csv(&aligned_curve_differences(input)?, &BigRational::default())
}

fn coordinate_curve_integral(input: &str, absolute_area: bool) -> Result<String, CsvError> {
    let mut samples = aligned_curve_differences(input)?;
    let total = integrate_trapezoid_samples(&mut samples, absolute_area, |_, _| {})?;
    Ok(total.to_string())
}

fn aligned_curve_differences(input: &str) -> Result<Vec<CoordinateSample>, CsvError> {
    let invalid = CsvError::CoordinateCurveArguments;
    let (columns, data) = json_argument_and_data::<[ColumnReference; 3]>(input, invalid)?;
    Ok(align_coordinate_curves(data, &columns, invalid)?
        .samples
        .into_iter()
        .enumerate()
        .map(|(row_index, (x, [_, _, difference]))| CoordinateSample {
            row_index,
            point: (x, difference),
        })
        .collect())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TrapezoidMeasure {
    Integral,
    Mean,
    Area,
}

fn trapezoid_statistic(input: &str, measure: TrapezoidMeasure) -> Result<String, CsvError> {
    let mut table = read_coordinate_interval(input, CsvError::TrapezoidPointCount)?;
    let mut value = integrate_trapezoid_samples(
        &mut table.samples,
        measure == TrapezoidMeasure::Area,
        |_, _| {},
    )?;
    if measure == TrapezoidMeasure::Mean {
        let first = &table.samples[0].point.0;
        let last = &table.samples.last().expect("至少有两个完整坐标").point.0;
        value /= last - first;
        ensure_exact_size(&value).map_err(|_| CsvError::ComputedNumberRange)?;
    }
    Ok(value.to_string())
}

fn read_coordinate_interval(
    input: &str,
    point_count_error: CsvError,
) -> Result<CsvCoordinates, CsvError> {
    let (argument, data) =
        json_argument_and_data::<Box<RawValue>>(input, CsvError::ScatterColumns)?;
    if let Ok(columns) = serde_json::from_str::<[ColumnReference; 2]>(argument.get()) {
        return read_coordinate_table(data, &columns, CsvError::ScatterColumns, point_count_error);
    }

    let invalid = CsvError::TrapezoidIntervalArguments;
    let (columns, [left, right]) =
        serde_json::from_str::<([ColumnReference; 2], [Box<RawValue>; 2])>(argument.get())
            .map_err(|_| invalid)?;
    let left = coordinate_argument(&left, invalid)?;
    let right = coordinate_argument(&right, invalid)?;
    if left >= right {
        return Err(CsvError::TrapezoidIntervalOrder);
    }
    let mut table = read_coordinate_table(data, &columns, invalid, point_count_error)?;
    sort_distinct_coordinates(&mut table.samples)?;
    let first = &table.samples[0].point.0;
    let last = &table.samples.last().expect("至少有两个完整坐标").point.0;
    if left < *first || right > *last {
        return Err(CsvError::TrapezoidIntervalRange);
    }

    let left_y = interpolate_coordinate_value(&table.samples, &left)?;
    let right_y = interpolate_coordinate_value(&table.samples, &right)?;
    table
        .samples
        .retain(|sample| sample.point.0 > left && sample.point.0 < right);
    table.samples.insert(
        0,
        CoordinateSample {
            row_index: 0,
            point: (left, left_y),
        },
    );
    table.samples.push(CoordinateSample {
        row_index: table.samples.len(),
        point: (right, right_y),
    });
    Ok(table)
}

enum CoordinateDeviation {
    RootMeanSquare,
    StandardDeviation,
}

fn coordinate_deviation_statistic(
    input: &str,
    measure: CoordinateDeviation,
) -> Result<String, CsvError> {
    let mut table = read_coordinate_interval(input, CsvError::CoordinateDeviationPointCount)?;
    sort_distinct_coordinates(&mut table.samples)?;
    let mut triple_square_integral = BigRational::default();
    for pair in table.samples.windows(2) {
        let (left_x, left_y) = &pair[0].point;
        let (right_x, right_y) = &pair[1].point;
        let triple_mean_square = left_y * left_y + left_y * right_y + right_y * right_y;
        triple_square_integral += (right_x - left_x) * triple_mean_square;
        ensure_exact_size(&triple_square_integral).map_err(|_| CsvError::ComputedNumberRange)?;
    }
    let first = &table.samples[0].point.0;
    let last = &table.samples.last().expect("至少有两个完整坐标").point.0;
    let span = last - first;
    let mean_square = triple_square_integral / (&span * BigInt::from(3u8));
    let squared = match measure {
        CoordinateDeviation::RootMeanSquare => mean_square,
        CoordinateDeviation::StandardDeviation => {
            let mean = integrate_trapezoid_samples(&mut table.samples, false, |_, _| {})? / span;
            mean_square - &mean * &mean
        }
    };
    ensure_exact_size(&squared).map_err(|_| CsvError::ComputedNumberRange)?;
    Ok(format_square_root(&squared))
}

#[derive(Default)]
struct CurveHeightEvent {
    level_width: BigRational,
    density_change: BigRational,
}

fn csv_probability(probability: &RawValue, invalid: CsvError) -> Result<BigRational, CsvError> {
    let raw = probability.get();
    let probability = if raw.starts_with('"') {
        Cow::Owned(serde_json::from_str::<String>(raw).map_err(|_| invalid)?)
    } else {
        Cow::Borrowed(raw)
    };
    parse_probability(probability.trim()).map_err(CsvError::Statistics)
}

fn coordinate_curve_quantile(input: &str) -> Result<String, CsvError> {
    let invalid = CsvError::CoordinateQuantileArguments;
    let ((columns, probability), data) =
        json_argument_and_data::<([ColumnReference; 2], Box<RawValue>)>(input, invalid)?;
    let probability = csv_probability(&probability, invalid)?;
    let mut table = read_coordinate_table(
        data,
        &columns,
        invalid,
        CsvError::CoordinateQuantilePointCount,
    )?;
    sort_distinct_coordinates(&mut table.samples)?;
    let mut heights = BTreeMap::<&BigRational, CurveHeightEvent>::new();
    for pair in table.samples.windows(2) {
        let (left_x, left_y) = &pair[0].point;
        let (right_x, right_y) = &pair[1].point;
        let width = right_x - left_x;
        let low = left_y.min(right_y);
        let high = left_y.max(right_y);
        // 水平段的横向跨度集中在一个高度，斜线段的跨度均匀分布在端点之间。
        if low == high {
            let event = heights.entry(low).or_default();
            event.level_width += width;
            ensure_exact_size(&event.level_width).map_err(|_| CsvError::ComputedNumberRange)?;
        } else {
            let density = width / (high - low);
            let start = heights.entry(low).or_default();
            start.density_change += &density;
            ensure_exact_size(&start.density_change).map_err(|_| CsvError::ComputedNumberRange)?;
            let end = heights.entry(high).or_default();
            end.density_change -= density;
            ensure_exact_size(&end.density_change).map_err(|_| CsvError::ComputedNumberRange)?;
        }
    }

    let first_x = &table.samples[0].point.0;
    let last_x = &table.samples.last().expect("至少有两个完整坐标").point.0;
    let target = (last_x - first_x) * probability;
    ensure_exact_size(&target).map_err(|_| CsvError::ComputedNumberRange)?;
    let zero = BigRational::default();
    let mut accumulated = zero.clone();
    let mut density = zero.clone();
    let mut previous = *heights.first_key_value().expect("曲线至少有一个高度").0;
    for (height, event) in heights {
        let before_level = &accumulated + &density * (height - previous);
        if target <= before_level && density > zero {
            let value = previous + (target - accumulated) / density;
            ensure_exact_size(&value).map_err(|_| CsvError::ComputedNumberRange)?;
            return Ok(value.to_string());
        }
        accumulated = before_level + event.level_width;
        if target <= accumulated {
            return Ok(height.to_string());
        }
        density += event.density_change;
        ensure_exact_size(&accumulated).map_err(|_| CsvError::ComputedNumberRange)?;
        ensure_exact_size(&density).map_err(|_| CsvError::ComputedNumberRange)?;
        previous = height;
    }
    unreachable!("曲线分位数应在样点高度范围内")
}

fn coordinate_curve_length(input: &str) -> Result<String, CsvError> {
    let mut table = read_coordinate_interval(input, CsvError::CoordinateLengthPointCount)?;
    sort_distinct_coordinates(&mut table.samples)?;
    let squared_distances = table.samples.windows(2).map(|pair| {
        let (left_x, left_y) = &pair[0].point;
        let (right_x, right_y) = &pair[1].point;
        let dx = right_x - left_x;
        let dy = right_y - left_y;
        &dx * &dx + &dy * &dy
    });
    format_square_root_sum(squared_distances).map_err(|_| CsvError::ComputedNumberRange)
}

fn coordinate_total_variation(input: &str) -> Result<String, CsvError> {
    let mut table = read_coordinate_interval(input, CsvError::CoordinateVariationPointCount)?;
    sort_distinct_coordinates(&mut table.samples)?;
    let mut total = BigRational::default();
    for pair in table.samples.windows(2) {
        let left = &pair[0].point.1;
        let right = &pair[1].point.1;
        total += left.max(right) - left.min(right);
        ensure_exact_size(&total).map_err(|_| CsvError::ComputedNumberRange)?;
    }
    Ok(total.to_string())
}

fn coordinate_extrema(input: &str) -> Result<String, CsvError> {
    let mut table = read_coordinate_interval(input, CsvError::CoordinateExtremaPointCount)?;
    sort_distinct_coordinates(&mut table.samples)?;
    let values = table.samples.iter().map(|sample| &sample.point.1);
    let min = values.clone().min().expect("至少有两个完整坐标");
    let max = values.max().expect("至少有两个完整坐标");
    let mut csv = String::from("类型,起始横坐标,结束横坐标,纵坐标");
    for run in table
        .samples
        .chunk_by(|left, right| left.point.1 == right.point.1)
    {
        let (start, y) = &run[0].point;
        let kind = match (y == min, y == max) {
            (true, true) => "最小值与最大值",
            (true, false) => "最小值",
            (false, true) => "最大值",
            (false, false) => continue,
        };
        let end = &run[run.len() - 1].point.0;
        let row = [start.to_string(), end.to_string(), y.to_string()];
        append_bounded_csv_row(
            &mut csv,
            std::iter::once(kind).chain(row.iter().map(String::as_str)),
            CsvError::StatisticsOutputTooLarge,
        )?;
    }
    Ok(csv)
}

fn coordinate_line_table(operation: &str, input: &str) -> Option<Result<CsvCoordinates, CsvError>> {
    if operation == "梯形累计图" || operation.eq_ignore_ascii_case("cumulative-trapezoid-plot")
    {
        return Some(cumulative_trapezoid_coordinates(input));
    }
    (operation == "折线图" || operation.eq_ignore_ascii_case("line-plot")).then(|| {
        let mut table = parse_coordinate_table(input, CsvError::LinePlotPointCount)?;
        sort_distinct_coordinates(&mut table.samples)?;
        Ok(table)
    })
}

pub(super) fn coordinate_line_chart(
    query: &str,
) -> Option<Result<(String, ScatterPlot), CsvError>> {
    let (operation, input) = split_argument(query)?;
    if operation == "曲线对比图" || operation.eq_ignore_ascii_case("curve-comparison-plot") {
        return Some(
            read_coordinate_curve_comparison(input)
                .and_then(|(curves, name)| curves.into_chart(&name)),
        );
    }
    let table = coordinate_line_table(operation, input)?;
    Some(table.and_then(|table| {
        let csv = table.to_csv()?;
        let points: Vec<_> = table
            .samples
            .into_iter()
            .map(|sample| sample.point)
            .collect();
        let plot = ScatterPlot::from_connected_points(&points).with_axis_names(table.headers);
        Ok((csv, plot))
    }))
}

fn cumulative_trapezoid_coordinates(input: &str) -> Result<CsvCoordinates, CsvError> {
    let mut table = parse_coordinate_table(input, CsvError::TrapezoidPointCount)?;
    let mut totals = Vec::with_capacity(table.samples.len());
    integrate_trapezoid_samples(&mut table.samples, false, |sample, total| {
        totals.push(CoordinateSample {
            row_index: sample.row_index,
            point: (sample.point.0.clone(), total.clone()),
        });
    })?;
    table.samples = totals;
    table.headers[1] = "累计积分".to_owned();
    Ok(table)
}

enum CoordinateColumn {
    CumulativeIntegral,
    Secant,
}

fn append_sampled_coordinate_column(
    input: &str,
    operation: CoordinateColumn,
) -> Result<String, CsvError> {
    let (invalid, point_count_error) = match operation {
        CoordinateColumn::CumulativeIntegral => (
            CsvError::CumulativeTrapezoidColumnArguments,
            CsvError::TrapezoidPointCount,
        ),
        CoordinateColumn::Secant => (CsvError::SecantColumnArguments, CsvError::SecantPointCount),
    };
    let ((name, coordinates), data) =
        json_argument_and_data::<(String, [ColumnReference; 2])>(input, invalid)?;
    append_coordinate_column(
        data,
        &name,
        &coordinates,
        invalid,
        point_count_error,
        |samples, values| match operation {
            CoordinateColumn::CumulativeIntegral => {
                integrate_trapezoid_samples(samples, false, |sample, total| {
                    values[sample.row_index] = total.to_string();
                })?;
                Ok(())
            }
            CoordinateColumn::Secant => fill_secant_slopes(samples, values),
        },
    )
}

enum CoordinateWindowMeasure {
    Mean,
    Integral,
}

fn append_coordinate_window_measure(
    input: &str,
    measure: CoordinateWindowMeasure,
) -> Result<String, CsvError> {
    let invalid = CsvError::CoordinateWindowArguments;
    let ((name, coordinates, width), data) =
        json_argument_and_data::<(String, [ColumnReference; 2], Box<RawValue>)>(input, invalid)?;
    let width = coordinate_argument(&width, invalid)?;
    if width <= BigRational::default() {
        return Err(invalid);
    }
    append_coordinate_column(
        data,
        &name,
        &coordinates,
        invalid,
        CsvError::CoordinateWindowPointCount,
        |samples, values| fill_coordinate_window_measurements(samples, values, &width, measure),
    )
}

fn append_coordinate_column(
    data: &str,
    name: &str,
    coordinates: &[ColumnReference; 2],
    invalid: CsvError,
    point_count_error: CsvError,
    fill: impl FnOnce(&mut [CoordinateSample], &mut [String]) -> Result<(), CsvError>,
) -> Result<String, CsvError> {
    if name.trim().is_empty() || name.contains('\0') {
        return Err(invalid);
    }
    let data = data.strip_prefix('\u{feff}').unwrap_or(data);
    let rows = read_rows(data, ',')?;
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let columns = resolve_distinct_columns(header, coordinates, invalid)?;
    let mut samples = collect_coordinate_samples(records, &columns, point_count_error)?;
    let mut values = vec![String::new(); records.len()];
    fill(&mut samples, &mut values)?;
    let mut values = values.into_iter();
    write_appended_column(header, records, name, |_| {
        Ok(values.next().expect("每行均有一个追加单元格"))
    })
}

fn fill_coordinate_window_measurements(
    samples: &mut [CoordinateSample],
    cells: &mut [String],
    width: &BigRational,
    measure: CoordinateWindowMeasure,
) -> Result<(), CsvError> {
    sort_distinct_coordinates(samples)?;
    let mut areas = Vec::with_capacity(samples.len());
    integrate_trapezoid_samples(samples, false, |_, total| areas.push(total.clone()))?;
    let first_x = &samples[0].point.0;
    let two = BigInt::from(2u8);
    for (index, sample) in samples.iter().enumerate().skip(1) {
        let x = &sample.point.0;
        let start = (x - width).max(first_x.clone());
        let preceding = samples[..index].partition_point(|sample| sample.point.0 <= start) - 1;
        let (preceding_x, preceding_y) = &samples[preceding].point;
        let start_y = interpolate_coordinate_value(samples, &start)?;
        let partial_area = (&start - preceding_x) * (preceding_y + start_y) / &two;
        let integral = &areas[index] - &areas[preceding] - partial_area;
        let value = match measure {
            CoordinateWindowMeasure::Mean => integral / (x - start),
            CoordinateWindowMeasure::Integral => integral,
        };
        ensure_exact_size(&value).map_err(|_| CsvError::ComputedNumberRange)?;
        cells[sample.row_index] = value.to_string();
    }
    Ok(())
}

fn fill_secant_slopes(
    samples: &mut [CoordinateSample],
    cells: &mut [String],
) -> Result<(), CsvError> {
    samples.sort_unstable_by(|left, right| left.point.0.cmp(&right.point.0));
    for pair in samples.windows(2) {
        if pair[0].point.0 == pair[1].point.0 {
            return Err(CsvError::SecantDuplicateX);
        }
        let slope = coordinate_secant_slope(&pair[0].point, &pair[1].point)?;
        cells[pair[1].row_index] = slope.to_string();
    }
    Ok(())
}

fn coordinate_secant_slope(
    (left_x, left_y): &Point,
    (right_x, right_y): &Point,
) -> Result<BigRational, CsvError> {
    let slope = (right_y - left_y) / (right_x - left_x);
    ensure_exact_size(&slope).map_err(|_| CsvError::ComputedNumberRange)?;
    Ok(slope)
}

fn integrate_trapezoid_samples(
    samples: &mut [CoordinateSample],
    absolute_area: bool,
    mut visit: impl FnMut(&CoordinateSample, &BigRational),
) -> Result<BigRational, CsvError> {
    samples.sort_unstable_by(|left, right| left.point.0.cmp(&right.point.0));
    let two = BigInt::from(2u8);
    let mut total = BigRational::default();
    visit(&samples[0], &total);
    for pair in samples.windows(2) {
        let (left, first) = &pair[0].point;
        let (right, last) = &pair[1].point;
        if left == right {
            return Err(CsvError::TrapezoidDuplicateX);
        }
        let doubled_mean_height = if absolute_area && first.numer().sign() != last.numer().sign() {
            // 两侧三角形的水平宽度与各自的高度成正比。
            let height_sum = first.max(last) - first.min(last);
            (first * first + last * last) / height_sum
        } else {
            let sum = first + last;
            if absolute_area && sum.numer().sign() == Sign::Minus {
                -sum
            } else {
                sum
            }
        };
        let segment = (right - left) * doubled_mean_height / &two;
        ensure_exact_size(&segment).map_err(|_| CsvError::ComputedNumberRange)?;
        total += segment;
        ensure_exact_size(&total).map_err(|_| CsvError::ComputedNumberRange)?;
        visit(&pair[1], &total);
    }
    Ok(total)
}

fn append_fit_columns(input: &str) -> Result<String, CsvError> {
    let invalid = CsvError::FittedColumnsArguments;
    let ((group_by, coordinates, names), data) =
        json_argument_and_data::<(Vec<ColumnReference>, [ColumnReference; 2], [String; 2])>(
            input, invalid,
        )?;
    let data = data.strip_prefix('\u{feff}').unwrap_or(data);
    if data.is_empty() {
        return Err(CsvError::MissingInput);
    }
    let rows = read_rows(data, ',')?;
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    if names[0] == names[1]
        || names
            .iter()
            .any(|name| name.trim().is_empty() || name.contains('\0') || header.contains(name))
    {
        return Err(invalid);
    }
    let group_by = resolve_distinct_columns(header, &group_by, CsvError::GroupingColumns)?;
    let coordinates = resolve_distinct_columns(header, &coordinates, invalid)?;
    let mut sums = HashMap::<_, PairedSums>::new();
    let mut values = Vec::with_capacity(records.len());
    for (index, row) in records.iter().enumerate() {
        let parse = |column: usize| {
            let text = row[column].trim();
            (!text.is_empty())
                .then(|| parse_fraction_literal(text))
                .transpose()
                .map_err(|error| CsvError::DataRowValue {
                    row: index + 1,
                    cause: error.message(),
                })
        };
        let x = parse(coordinates[0])?;
        let y = parse(coordinates[1])?;
        if let (Some(x), Some(y)) = (&x, &y) {
            sums.entry(column_key(row, &group_by))
                .or_default()
                .record(x, y)
                .map_err(|error| CsvError::DataRowValue {
                    row: index + 1,
                    cause: error.message(),
                })?;
        }
        values.push((x, y));
    }
    let mut fits = HashMap::with_capacity(sums.len());
    for (key, sums) in sums {
        match sums.fit_line() {
            Ok(fit) => {
                fits.insert(key, fit);
            }
            Err(LinearFitError::ConstantX) => {}
            Err(error) => return Err(CsvError::LinearFit(error)),
        }
    }

    let mut output = String::new();
    let limit = CsvError::AddedColumnTooLarge;
    append_bounded_csv_row(
        &mut output,
        header.iter().chain(&names).map(String::as_str),
        limit,
    )?;
    for (index, (row, (x, y))) in records.iter().zip(values).enumerate() {
        let fit = fits.get(&column_key(row, &group_by));
        let at_row = |cause| CsvError::DataRowValue {
            row: index + 1,
            cause,
        };
        let [predicted, residual] = if let (Some(fit), Some(x)) = (fit, x) {
            let predicted = fit.predict(&x).map_err(|error| at_row(error.message()))?;
            let residual = if let Some(y) = y {
                let value = y - &predicted;
                ensure_exact_size(&value).map_err(|error| at_row(error.message()))?;
                value.to_string()
            } else {
                String::new()
            };
            [predicted.to_string(), residual]
        } else {
            Default::default()
        };
        let new_fields = [predicted.as_str(), residual.as_str()];
        append_bounded_csv_row(
            &mut output,
            row.iter().map(String::as_str).chain(new_fields),
            limit,
        )?;
    }
    Ok(output)
}

fn fit_by_columns(
    rows: &[Vec<String>],
    columns: &[ColumnReference],
    coordinates: &[ColumnReference; 2],
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let columns = resolve_distinct_columns(header, columns, CsvError::GroupingColumns)?;
    let coordinates = resolve_distinct_columns(header, coordinates, CsvError::GroupFitColumns)?;
    let mut groups = HashMap::new();
    for (position, row) in records.iter().enumerate() {
        let (_, sums) = groups
            .entry(column_key(row, &columns))
            .or_insert_with(|| (position, PairedSums::default()));
        let x = row[coordinates[0]].trim();
        let y = row[coordinates[1]].trim();
        if x.is_empty() || y.is_empty() {
            continue;
        }
        let at_row = |cause| CsvError::DataRowValue {
            row: position + 1,
            cause,
        };
        let x = parse_fraction_literal(x).map_err(|error| at_row(error.message()))?;
        let y = parse_fraction_literal(y).map_err(|error| at_row(error.message()))?;
        sums.record(&x, &y)
            .map_err(|error| at_row(error.message()))?;
    }
    let mut groups: Vec<_> = groups.into_iter().collect();
    groups.sort_unstable_by_key(|(_, (position, _))| *position);
    let mut output = String::new();
    let limit = CsvError::GroupFitTooLarge;
    let labels = ["观测数", "斜率", "截距", "R²"];
    append_bounded_csv_row(
        &mut output,
        columns
            .iter()
            .map(|&index| header[index].as_str())
            .chain(labels),
        limit,
    )?;
    for (key, (_, sums)) in groups {
        let [slope, intercept, r_squared] = match sums.fit_line() {
            Ok(fit) => [
                fit.slope.to_string(),
                fit.intercept.to_string(),
                fit.r_squared()
                    .map_err(CsvError::LinearFit)?
                    .map_or_else(String::new, |value| value.to_string()),
            ],
            Err(LinearFitError::ConstantX) => Default::default(),
            Err(error) => return Err(CsvError::LinearFit(error)),
        };
        let count = sums.count().to_string();
        let values = [
            count.as_str(),
            slope.as_str(),
            intercept.as_str(),
            r_squared.as_str(),
        ];
        append_bounded_csv_row(&mut output, key.into_iter().chain(values), limit)?;
    }
    Ok(output)
}

fn select_group_extrema(
    rows: &[Vec<String>],
    columns: &[ColumnReference],
    amount: ColumnReference,
    limit: usize,
    highest: bool,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let columns = resolve_distinct_columns(header, columns, CsvError::GroupingColumns)?;
    let amount = amount.resolve(header)?;
    validate_column_width(&[amount], header.len())?;
    let mut group_indices = HashMap::new();
    let mut groups: Vec<Vec<_>> = Vec::new();
    for row in records {
        let value = parse_fraction_literal(&row[amount]).map_err(|_| CsvError::InvalidNumber)?;
        let index = *group_indices
            .entry(column_key(row, &columns))
            .or_insert_with(|| {
                let index = groups.len();
                groups.push(Vec::new());
                index
            });
        groups[index].push((value, row));
    }
    let selected = groups.into_iter().flat_map(|mut group| {
        group.sort_by(|(left, _), (right, _)| {
            if highest {
                right.cmp(left)
            } else {
                left.cmp(right)
            }
        });
        group.into_iter().take(limit).map(|(_, row)| row)
    });
    Ok(write_rows(std::iter::once(header).chain(selected), ','))
}

fn grouped_csv<'a, T: ToString>(
    header: &[String],
    columns: &[usize],
    groups: impl IntoIterator<Item = (Vec<&'a str>, (usize, T))>,
    label: &str,
) -> String {
    let mut groups: Vec<_> = groups.into_iter().collect();
    groups.sort_unstable_by_key(|(_, (position, _))| *position);
    let mut output = Vec::with_capacity(groups.len() + 1);
    let mut header: Vec<_> = columns.iter().map(|&index| header[index].clone()).collect();
    header.push(label.to_owned());
    output.push(header);
    output.extend(groups.into_iter().map(|(key, (_, value))| {
        let mut row: Vec<_> = key.into_iter().map(str::to_owned).collect();
        row.push(value.to_string());
        row
    }));
    write_rows(&output, ',')
}

fn numeric_column_key(row: &[String], columns: &[usize]) -> Result<Vec<BigRational>, CsvError> {
    columns
        .iter()
        .map(|&column| parse_fraction_literal(&row[column]).map_err(|_| CsvError::InvalidNumber))
        .collect()
}

fn column_key<'a>(row: &'a [String], columns: &[usize]) -> Vec<&'a str> {
    columns.iter().map(|&index| row[index].as_str()).collect()
}

fn grouped_row_positions(records: &[Vec<String>], columns: &[usize]) -> Vec<Vec<usize>> {
    let mut group_positions = HashMap::new();
    let mut groups = Vec::<Vec<usize>>::new();
    for (index, record) in records.iter().enumerate() {
        let group = *group_positions
            .entry(column_key(record, columns))
            .or_insert_with(|| {
                groups.push(Vec::new());
                groups.len() - 1
            });
        groups[group].push(index);
    }
    groups
}

fn pivot_csv(
    rows: &[Vec<String>],
    columns: &[ColumnReference],
    kind: PivotKind,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let counting = matches!(kind, PivotKind::Count);
    let (_, invalid) = kind.column_requirements();
    let columns = resolve_distinct_columns(header, columns, invalid)?;
    let (row_column, column_column) = (columns[0], columns[1]);
    let amount_column = columns.get(2).copied();
    let one = BigRational::from_integer(1.into());
    let mut row_labels = Vec::new();
    let mut column_labels = Vec::new();
    let mut row_positions = HashMap::new();
    let mut column_positions = HashMap::new();
    let mut totals = HashMap::<_, BigRational>::new();
    let mut original_values = HashMap::new();
    for (index, record) in records.iter().enumerate() {
        let row_label = record[row_column].as_str();
        let column_label = record[column_column].as_str();
        let row = *row_positions.entry(row_label).or_insert_with(|| {
            row_labels.push(row_label);
            row_labels.len() - 1
        });
        let column = *column_positions.entry(column_label).or_insert_with(|| {
            column_labels.push(column_label);
            column_labels.len() - 1
        });
        let location = (row, column);
        if matches!(kind, PivotKind::Values) {
            let value = record[columns[2]].as_str();
            if original_values.insert(location, value).is_some() {
                return Err(CsvError::DataRowValue {
                    row: index + 1,
                    cause: "行列组合重复，请为每个组合保留一条记录",
                });
            }
            continue;
        }
        let value = match amount_column {
            Some(column) => {
                parse_fraction_literal(&record[column]).map_err(|_| CsvError::InvalidNumber)?
            }
            None => one.clone(),
        };
        let total = totals.entry(location).or_default();
        *total += value;
        ensure_exact_size(total)
            .map_err(|error| CsvError::Statistics(StatisticsError::Evaluation(error)))?;
    }

    let mut output = csv_cell(&header[row_column]).into_owned();
    for label in &column_labels {
        output.push(',');
        output.push_str(&csv_cell(label));
    }
    let minimum_cell_bytes = 1 + usize::from(counting);
    let minimum_body_bytes = column_labels
        .len()
        .saturating_mul(row_labels.len())
        .saturating_mul(minimum_cell_bytes);
    if output.len().saturating_add(minimum_body_bytes) > MAX_PIVOT_BYTES {
        return Err(CsvError::PivotTooLarge);
    }
    for (row, label) in row_labels.into_iter().enumerate() {
        output.push_str("\r\n");
        output.push_str(&csv_cell(label));
        for column in 0..column_labels.len() {
            output.push(',');
            if let Some(value) = totals.get(&(row, column)) {
                output.push_str(&value.to_string());
            } else if let Some(value) = original_values.get(&(row, column)) {
                output.push_str(&csv_cell(value));
            } else if counting {
                output.push('0');
            }
        }
        if output.len() > MAX_PIVOT_BYTES {
            return Err(CsvError::PivotTooLarge);
        }
    }
    Ok(output)
}

fn unpivot(
    rows: &[Vec<String>],
    keep: &[ColumnReference],
    names: &[String; 2],
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let keep = resolve_distinct_columns(header, keep, CsvError::UnpivotArguments)?;
    if keep.len() == header.len() {
        return Err(CsvError::UnpivotColumns);
    }
    if names
        .iter()
        .any(|name| name.is_empty() || name.contains('\0'))
    {
        return Err(CsvError::UnpivotArguments);
    }
    if names[0] == names[1]
        || names
            .iter()
            .any(|name| keep.iter().any(|&column| header[column] == *name))
    {
        return Err(CsvError::HeaderNameConflict);
    }

    let kept_columns = keep.iter().copied().collect::<HashSet<_>>();
    let expanded = header
        .iter()
        .enumerate()
        .filter(|(column, _)| !kept_columns.contains(column))
        .map(|(column, label)| (column, csv_cell(label)))
        .collect::<Vec<_>>();
    let mut output = keep
        .iter()
        .map(|&column| &header[column])
        .chain(names)
        .map(|name| csv_cell(name))
        .collect::<Vec<_>>()
        .join(",");
    for record in records {
        let mut prefix = keep
            .iter()
            .map(|&column| csv_cell(&record[column]))
            .collect::<Vec<_>>()
            .join(",");
        if !keep.is_empty() {
            prefix.push(',');
        }
        for (column, label) in &expanded {
            let value = csv_cell(&record[*column]);
            let row_bytes = 3 + prefix.len() + label.len() + value.len();
            if output.len() + row_bytes > MAX_PIVOT_BYTES {
                return Err(CsvError::PivotTooLarge);
            }
            output.push_str("\r\n");
            output.push_str(&prefix);
            output.push_str(label);
            output.push(',');
            output.push_str(&value);
        }
    }
    Ok(output)
}

fn sort_csv_for_preview(
    input: &str,
    descending: bool,
    numeric: bool,
) -> Result<CsvPreviewData, CsvError> {
    let (columns, data) = column_reference_arguments(input, CsvError::NamedColumnArguments)?;
    let mut records = read_table_for_preview(data, ',')?.into_iter();
    let header = records.next().ok_or(CsvError::MissingInput)?;
    let columns = resolve_distinct_columns(&header, &columns, CsvError::NamedColumnArguments)?;
    let ordered = order_records(records, &columns, descending, numeric)?;
    let rows: Vec<_> = std::iter::once(header).chain(ordered).collect();
    Ok((write_rows(&rows, ','), rows))
}

struct SortRecord {
    original_index: usize,
    cells: Vec<String>,
}

impl AsRef<[String]> for SortRecord {
    fn as_ref(&self) -> &[String] {
        &self.cells
    }
}

impl SortRecord {
    fn from_input(rows: impl Iterator<Item = Vec<String>>) -> impl Iterator<Item = Self> {
        rows.enumerate().map(|(original_index, cells)| Self {
            original_index,
            cells,
        })
    }
}

fn sort_by_timestamps(input: &str, descending: bool) -> Result<CsvPreviewData, CsvError> {
    let (columns, data) = column_reference_arguments(input, CsvError::NamedColumnArguments)?;
    let mut records = read_table_for_preview(data, ',')?.into_iter();
    let header = records.next().ok_or(CsvError::MissingInput)?;
    let columns = resolve_distinct_columns(&header, &columns, CsvError::NamedColumnArguments)?;
    let records = order_timestamp_records(SortRecord::from_input(records), &columns, descending)?;
    let rows: Vec<_> = std::iter::once(header)
        .chain(records.into_iter().map(|record| record.cells))
        .collect();
    Ok((write_rows(&rows, ','), rows))
}

fn order_timestamp_records(
    records: impl Iterator<Item = SortRecord>,
    columns: &[usize],
    descending: bool,
) -> Result<Vec<SortRecord>, CsvError> {
    let mut records = records
        .map(|record| {
            let times = columns
                .iter()
                .map(|&column| {
                    absolute_timestamp(&record.cells[column]).map_err(|error| {
                        CsvError::DataRowValue {
                            row: record.original_index + 1,
                            cause: error.message(),
                        }
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok((times, record))
        })
        .collect::<Result<Vec<_>, CsvError>>()?;
    records.sort_by(|(left, _), (right, _)| {
        if descending {
            right.cmp(left)
        } else {
            left.cmp(right)
        }
    });
    Ok(records.into_iter().map(|(_, record)| record).collect())
}

#[derive(Deserialize)]
#[serde(untagged)]
enum CsvFieldSortOrder {
    Common(FieldSortOrder),
    Timestamp(TimestampSortOrder),
}

#[derive(Deserialize)]
enum TimestampSortOrder {
    #[serde(rename = "时间戳升序", alias = "timestamp-asc")]
    Ascending,
    #[serde(rename = "时间戳降序", alias = "timestamp-desc")]
    Descending,
}

fn sort_with_orders(input: &str) -> Result<CsvPreviewData, CsvError> {
    let (rules, data) = json_argument_and_data::<Vec<(ColumnReference, CsvFieldSortOrder)>>(
        input,
        CsvError::SortFields,
    )?;
    if !(1..=MAX_SORT_FIELDS).contains(&rules.len()) {
        return Err(CsvError::SortFields);
    }
    let mut records = read_table_for_preview(data, ',')?.into_iter();
    let header = records.next().ok_or(CsvError::MissingInput)?;
    let mut seen = HashSet::new();
    let fields = rules
        .into_iter()
        .map(|(reference, order)| {
            let column = reference.resolve(&header)?;
            validate_column_width(&[column], header.len())?;
            if !seen.insert(column) {
                return Err(CsvError::SortFields);
            }
            Ok((column, order))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut ordered: Vec<_> = SortRecord::from_input(records).collect();
    for (column, order) in fields.into_iter().rev() {
        ordered = match order {
            CsvFieldSortOrder::Common(order) => order_records(
                ordered.into_iter(),
                &[column],
                order.descending(),
                order.numeric(),
            )?,
            CsvFieldSortOrder::Timestamp(order) => order_timestamp_records(
                ordered.into_iter(),
                &[column],
                matches!(order, TimestampSortOrder::Descending),
            )?,
        };
    }
    let rows: Vec<_> = std::iter::once(header)
        .chain(ordered.into_iter().map(|record| record.cells))
        .collect();
    Ok((write_rows(&rows, ','), rows))
}

fn order_records<Row: AsRef<[String]>>(
    records: impl Iterator<Item = Row>,
    columns: &[usize],
    descending: bool,
    numeric: bool,
) -> Result<Vec<Row>, CsvError> {
    let direction = |order: Ordering| {
        if descending {
            order.reverse()
        } else {
            order
        }
    };
    let ordered = if numeric {
        let mut keyed = records
            .map(|row| {
                let key = columns
                    .iter()
                    .map(|&column| {
                        parse_fraction_literal(&row.as_ref()[column])
                            .map_err(|_| CsvError::InvalidNumber)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok((key, row))
            })
            .collect::<Result<Vec<_>, CsvError>>()?;
        keyed.sort_by(|(left, _), (right, _)| direction(left.cmp(right)));
        keyed.into_iter().map(|(_, row)| row).collect::<Vec<_>>()
    } else {
        let mut ordered: Vec<_> = records.collect();
        ordered.sort_by(|left, right| {
            let order = columns
                .iter()
                .map(|&column| compare_natural(&left.as_ref()[column], &right.as_ref()[column]))
                .find(|order| !order.is_eq())
                .unwrap_or(Ordering::Equal);
            direction(order)
        });
        ordered
    };
    Ok(ordered)
}

pub(super) fn csv_table_preview(query: &str) -> Option<Result<(String, TablePreview), CsvError>> {
    let (operation, data) = split_argument(query)?;
    if let Some(mode) = CsvKeywordMatch::from_operation(operation) {
        return Some(search_csv_rows(data, mode, true).map(|matches| {
            let text = write_rows(&matches.rows, ',');
            let table =
                TablePreview::from_rows(matches.rows).with_highlighted_cells(matches.highlighted);
            (text, table)
        }));
    }
    if matches!(
        operation.to_ascii_lowercase().as_str(),
        "检查表头"
            | "check-header"
            | "列清单"
            | "column-directory"
            | "纵向合并"
            | "stack"
            | "横向合并"
            | "bind-columns"
            | "检查数值列"
            | "check-numeric-columns"
            | "检查日期列"
            | "check-date-columns"
            | "检查json列"
            | "check-json-columns"
            | "检查时间戳列"
            | "check-timestamp-columns"
            | "检查长度范围"
            | "check-length-range"
            | "检查日期格式"
            | "check-date-format"
            | "检查正则列"
            | "check-regex-columns"
            | "检查时间范围"
            | "check-timestamp-range"
            | "检查数值范围"
            | "check-numeric-range"
            | "检查允许值"
            | "check-allowed-values"
            | "检查规则"
            | "check-rules"
            | "规则保留行"
            | "keep-valid-rows"
            | "规则问题行"
            | "keep-invalid-rows"
            | "规则统计"
            | "rule-summary"
            | "规则标记列"
            | "rule-pass-column"
            | "规则失败数列"
            | "rule-failure-count-column"
            | "表头对比"
            | "compare-headers"
            | "对比行"
            | "compare-rows"
            | "对比列"
            | "compare-column"
            | "两列差异"
            | "column-pair-differences"
            | "两列数值差异"
            | "numeric-column-pair-differences"
            | "数值对比列"
            | "compare-numeric-column"
            | "正则提取多列"
            | "regex-capture-columns"
            | "正则展开行"
            | "regex-expand-rows"
            | "拆成行"
            | "split-to-rows"
            | "工程计数列"
            | "engineering-columns"
            | "模板列"
            | "template-column"
            | "按列去重留末"
            | "deduplicate-columns-last"
            | "数值去重留末"
            | "deduplicate-numeric-last"
            | "对比全部列"
            | "compare-all-columns"
            | "分组时长合计"
            | "group-duration-sum"
            | "分组取行"
            | "pick-group-row"
            | "单位换算列"
            | "convert-unit-column"
            | "时长数值列"
            | "duration-number-column"
            | "表格时刻序号列"
            | "spreadsheet-time-serial-column"
            | "序号转日期时刻列"
            | "serial-datetime-column"
            | "组内时间戳差列"
            | "group-timestamp-difference-column"
            | "时间戳对齐列"
            | "timestamp-align-column"
            | "数值对齐列"
            | "numeric-align-column"
            | "组内时间戳偏移列"
            | "group-timestamp-offset-column"
            | "会话编号列"
            | "timestamp-session-column"
            | "时间窗口计数列"
            | "timestamp-window-count-column"
            | "时间窗口合计列"
            | "timestamp-window-sum-column"
            | "时间窗口平均列"
            | "timestamp-window-average-column"
            | "移动样本方差列"
            | "moving-sample-variance-column"
            | "移动总体方差列"
            | "moving-population-variance-column"
            | "移动样本标准差列"
            | "moving-sample-stdev-column"
            | "移动总体标准差列"
            | "moving-population-stdev-column"
            | "百分位排名列"
            | "percent-rank-column"
            | "降序百分位排名列"
            | "descending-percent-rank-column"
            | "累积分布列"
            | "cume-dist-column"
            | "降序累积分布列"
            | "descending-cume-dist-column"
            | "段内计数列"
            | "run-length-column"
            | "网段归属列"
            | "cidr-column"
            | "网段名称列"
            | "cidr-name-column"
    ) {
        return Some(transform_csv_operation(operation, data).and_then(|text| {
            let table = parse_table_preview(&text)?;
            Ok((text, table))
        }));
    }
    Some(
        csv_preview_output(operation, data)?
            .map(|(text, rows)| (text, TablePreview::from_rows(rows))),
    )
}

type CsvPreviewData = (String, Vec<Vec<String>>);

fn csv_preview_output(operation: &str, input: &str) -> Option<Result<CsvPreviewData, CsvError>> {
    if operation == "命中位置" || operation.eq_ignore_ascii_case("keyword-positions") {
        return Some(csv_keyword_positions(input));
    }
    if operation == "分组加权平均" || operation.eq_ignore_ascii_case("group-weighted-mean") {
        return Some(csv_grouped_weighted_mean(input));
    }
    if operation == "加权分位表" || operation.eq_ignore_ascii_case("weighted-quantiles-csv") {
        return Some(csv_weighted_quantiles(input));
    }
    if operation == "分组分位表" || operation.eq_ignore_ascii_case("group-quantiles") {
        return Some(csv_grouped_quantiles(input));
    }
    if operation == "连续段摘要" || operation.eq_ignore_ascii_case("run-summary") {
        return Some(csv_run_summary(input));
    }
    if operation == "分组时间戳摘要" || operation.eq_ignore_ascii_case("group-timestamp-summary")
    {
        return Some(csv_grouped_timestamps(input, GroupTimestampReport::Summary));
    }
    if operation == "分组最早时间行"
        || operation.eq_ignore_ascii_case("group-earliest-timestamp-row")
    {
        return Some(csv_grouped_timestamps(
            input,
            GroupTimestampReport::EarliestRows,
        ));
    }
    if operation == "分组最晚时间行" || operation.eq_ignore_ascii_case("group-latest-timestamp-row")
    {
        return Some(csv_grouped_timestamps(
            input,
            GroupTimestampReport::LatestRows,
        ));
    }
    if operation == "分组加权频数" || operation.eq_ignore_ascii_case("group-weighted-frequency")
    {
        return Some(csv_grouped_weighted_frequency(input));
    }
    if operation == "列值概览" || operation.eq_ignore_ascii_case("value-profile") {
        return Some(csv_value_profile(input));
    }
    let sorts = [
        ("排序", "sort-columns", false, false),
        ("降序", "sort-columns-desc", true, false),
        ("数值排序", "sort-numeric", false, true),
        ("数值降序", "sort-numeric-desc", true, true),
    ];
    if let Some((_, _, descending, numeric)) = sorts
        .into_iter()
        .find(|(name, alias, _, _)| operation == *name || operation.eq_ignore_ascii_case(alias))
    {
        return Some(sort_csv_for_preview(input, descending, numeric));
    }
    if operation == "多列排序" || operation.eq_ignore_ascii_case("sort-by-fields") {
        return Some(sort_with_orders(input));
    }
    let timestamp_sorts = [
        ("时间戳排序", "sort-timestamps", false),
        ("时间戳降序", "sort-timestamps-desc", true),
    ];
    if let Some((_, _, descending)) = timestamp_sorts
        .into_iter()
        .find(|(name, alias, _)| operation == *name || operation.eq_ignore_ascii_case(alias))
    {
        return Some(sort_by_timestamps(input, descending));
    }
    if let Some(delimiter) = table_preview_delimiter(operation) {
        return Some(read_preview_output(input, delimiter));
    }
    csv_layout_preview(operation, input).or_else(|| csv_filter_preview(operation, input))
}

fn csv_layout_preview(operation: &str, input: &str) -> Option<Result<CsvPreviewData, CsvError>> {
    let projected = match operation.to_ascii_lowercase().as_str() {
        "转置" | "transpose" => read_table_for_preview(input, ',').map(|rows| transpose(&rows)),
        "选列" | "pick-columns" => column_arguments(input).and_then(|(columns, data)| {
            select_columns(&read_table_for_preview(data, ',')?, &columns)
        }),
        "选列名" | "pick-by-header" => {
            json_argument_and_data::<Vec<String>>(input, CsvError::InvalidColumnNames).and_then(
                |(names, data)| select_named_columns(&read_table_for_preview(data, ',')?, &names),
            )
        }
        "去列" | "drop-columns" => {
            column_reference_arguments(input, CsvError::NamedColumnArguments).and_then(
                |(columns, data)| drop_columns(&read_table_for_preview(data, ',')?, &columns),
            )
        }
        "取区域" | "pick-region" => pick_table_region(input),
        "改单元格" | "set-cell" => set_csv_cell(input),
        "批量改单元格" | "set-cells" => set_csv_cells(input),
        _ => return None,
    };
    Some(projected.map(|rows| (write_rows(&rows, ','), rows)))
}

fn csv_column_directory(input: &str) -> Result<String, CsvError> {
    let rows = read_table_for_preview(input, ',')?;
    let header = rows.first().ok_or(CsvError::MissingInput)?;
    let mut uses = HashMap::new();
    for name in header {
        *uses.entry(name.as_str()).or_insert(0usize) += 1;
    }
    let mut output = String::from("列号,列名,同名列数");
    for (index, name) in header.iter().enumerate() {
        let number = (index + 1).to_string();
        let count = uses[name.as_str()].to_string();
        append_bounded_csv_row(
            &mut output,
            [number.as_str(), name.as_str(), count.as_str()].into_iter(),
            CsvError::ColumnDirectoryTooLarge,
        )?;
    }
    Ok(output)
}

fn csv_value_profile(input: &str) -> Result<CsvPreviewData, CsvError> {
    let invalid = CsvError::NamedColumnArguments;
    let (columns, data) = column_reference_arguments(input, invalid)?;
    let rows = read_table_for_preview(data, ',')?;
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let columns = resolve_distinct_columns(header, &columns, invalid)?;
    let headings = [
        "列号",
        "列名",
        "空单元格",
        "非空单元格",
        "不同非空值",
        "重复值种数",
    ];
    let mut csv = headings.join(",");
    let mut summary = vec![headings.into_iter().map(str::to_owned).collect()];
    for column in columns {
        let mut counts = HashMap::new();
        let mut nonempty = 0;
        for cell in records
            .iter()
            .map(|row| row[column].as_str())
            .filter(|cell| !cell.is_empty())
        {
            *counts.entry(cell).or_insert(0usize) += 1;
            nonempty += 1;
        }
        let repeated = counts.values().filter(|&&count| count > 1).count();
        let row = vec![
            (column + 1).to_string(),
            header[column].clone(),
            (records.len() - nonempty).to_string(),
            nonempty.to_string(),
            counts.len().to_string(),
            repeated.to_string(),
        ];
        append_bounded_csv_row(
            &mut csv,
            row.iter().map(String::as_str),
            CsvError::StatisticsOutputTooLarge,
        )?;
        summary.push(row);
    }
    Ok((csv, summary))
}

struct CsvKeyword {
    original: String,
    key: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum CsvKeywordArgument {
    One(String),
    List(Vec<String>),
    Scoped(CsvKeywordScope),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CsvKeywordScope {
    #[serde(rename = "词", alias = "keywords")]
    keywords: Vec<String>,
    #[serde(rename = "列", alias = "columns")]
    columns: Vec<ColumnReference>,
}

struct CsvKeywordQuery<'a> {
    keywords: Vec<CsvKeyword>,
    columns: Vec<ColumnReference>,
    data: &'a str,
}

fn read_csv_keywords(input: &str) -> Result<CsvKeywordQuery<'_>, CsvError> {
    let invalid = CsvError::SearchRowKeywords;
    let (argument, data) = json_argument_and_data::<CsvKeywordArgument>(input, invalid)?;
    let (keywords, columns) = match argument {
        CsvKeywordArgument::One(word) if word.is_empty() => {
            return Ok(CsvKeywordQuery {
                keywords: Vec::new(),
                columns: Vec::new(),
                data,
            });
        }
        CsvKeywordArgument::One(word) => (vec![word], Vec::new()),
        CsvKeywordArgument::List(words) => (words, Vec::new()),
        CsvKeywordArgument::Scoped(scope) => (scope.keywords, scope.columns),
    };
    if !(1..=MAX_FILTER_CONDITIONS).contains(&keywords.len())
        || keywords.iter().any(|word| word.trim().is_empty())
    {
        return Err(invalid);
    }
    if columns.len() > MAX_FILTER_CONDITIONS {
        return Err(CsvError::SearchRowColumns);
    }
    let keywords = keywords
        .into_iter()
        .map(|original| CsvKeyword {
            key: csv_keyword_key(&original),
            original,
        })
        .collect();
    Ok(CsvKeywordQuery {
        keywords,
        columns,
        data,
    })
}

fn keyword_column_indices(
    header: &[String],
    columns: &[ColumnReference],
) -> Result<Vec<usize>, CsvError> {
    if columns.is_empty() {
        return Ok((0..header.len()).collect());
    }
    let mut selected = resolve_distinct_columns(header, columns, CsvError::SearchRowColumns)?;
    selected.sort_unstable();
    Ok(selected)
}

fn csv_keyword_key(text: &str) -> String {
    text.chars()
        .map(|ch| narrow_ascii_graphic(ch).to_ascii_lowercase())
        .collect()
}

enum CsvKeywordMatch {
    All,
    Any,
    None,
}

impl CsvKeywordMatch {
    fn from_operation(operation: &str) -> Option<Self> {
        match operation.to_ascii_lowercase().as_str() {
            "搜索行" | "search-rows" => Some(Self::All),
            "搜索任一" | "search-any-rows" => Some(Self::Any),
            "排除关键词" | "exclude-keywords" => Some(Self::None),
            _ => None,
        }
    }
}

struct CsvRowMatches {
    rows: Vec<Vec<String>>,
    highlighted: Vec<(usize, usize)>,
}

fn search_csv_rows(
    input: &str,
    mode: CsvKeywordMatch,
    highlight: bool,
) -> Result<CsvRowMatches, CsvError> {
    let CsvKeywordQuery {
        keywords,
        columns,
        data,
    } = read_csv_keywords(input)?;
    let rows = read_table_for_preview(data, ',')?;
    if keywords.is_empty() {
        return Ok(CsvRowMatches {
            rows,
            highlighted: Vec::new(),
        });
    }
    let mut rows = rows.into_iter();
    let header = rows.next().ok_or(CsvError::MissingInput)?;
    let columns = keyword_column_indices(&header, &columns)?;
    let mut matches = CsvRowMatches {
        rows: vec![header],
        highlighted: Vec::new(),
    };
    for row in rows {
        let cells: Vec<_> = columns
            .iter()
            .map(|&index| csv_keyword_key(&row[index]))
            .collect();
        let contains =
            |word: &CsvKeyword| cells.iter().any(|cell| cell.contains(word.key.as_str()));
        let keep = match mode {
            CsvKeywordMatch::All => keywords.iter().all(contains),
            CsvKeywordMatch::Any => keywords.iter().any(contains),
            CsvKeywordMatch::None => !keywords.iter().any(contains),
        };
        if !keep {
            continue;
        }
        if highlight && !matches!(mode, CsvKeywordMatch::None) {
            let row_index = matches.rows.len();
            matches
                .highlighted
                .extend(
                    columns
                        .iter()
                        .copied()
                        .zip(cells)
                        .filter_map(|(column, cell)| {
                            keywords
                                .iter()
                                .any(|word| cell.contains(word.key.as_str()))
                                .then_some((row_index, column))
                        }),
                );
        }
        matches.rows.push(row);
    }
    Ok(matches)
}

fn csv_keyword_positions(input: &str) -> Result<CsvPreviewData, CsvError> {
    let CsvKeywordQuery {
        keywords,
        columns,
        data,
    } = read_csv_keywords(input)?;
    let rows = read_table_for_preview(data, ',')?;
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let selected = keyword_column_indices(header, &columns)?;
    let columns = ["数据行号", "列号", "列名", "关键词", "内容"];
    let mut csv = columns.join(",");
    let mut matches_table = vec![columns.map(str::to_owned).to_vec()];
    for (record, row) in records.iter().enumerate() {
        for &column in &selected {
            let cell = &row[column];
            let key = csv_keyword_key(cell);
            let matches: Vec<_> = keywords
                .iter()
                .filter(|keyword| key.contains(keyword.key.as_str()))
                .map(|keyword| keyword.original.as_str())
                .collect();
            if matches.is_empty() {
                continue;
            }
            let matched = serde_json::to_string(&matches).map_err(|_| CsvError::Serialization)?;
            let fields = vec![
                (record + 1).to_string(),
                (column + 1).to_string(),
                header[column].clone(),
                matched,
                cell.clone(),
            ];
            append_bounded_csv_row(
                &mut csv,
                fields.iter().map(String::as_str),
                CsvError::StatisticsOutputTooLarge,
            )?;
            matches_table.push(fields);
        }
    }
    Ok((csv, matches_table))
}

fn table_preview_delimiter(operation: &str) -> Option<char> {
    if operation == "表格预览" || operation.eq_ignore_ascii_case("table-preview") {
        Some(',')
    } else if operation.eq_ignore_ascii_case("从tsv") || operation.eq_ignore_ascii_case("from-tsv")
    {
        Some('\t')
    } else {
        None
    }
}

fn read_preview_output(input: &str, delimiter: char) -> Result<CsvPreviewData, CsvError> {
    let rows = read_table_for_preview(input, delimiter)?;
    let text = if delimiter == ',' {
        input.to_owned()
    } else {
        write_rows(&rows, ',')
    };
    Ok((text, rows))
}

pub(super) fn parse_table_preview(input: &str) -> Result<TablePreview, CsvError> {
    read_table_for_preview(input, ',').map(TablePreview::from_rows)
}

fn read_table_for_preview(input: &str, delimiter: char) -> Result<Vec<Vec<String>>, CsvError> {
    let data = input.strip_prefix('\u{feff}').unwrap_or(input);
    if data.is_empty() {
        return Err(CsvError::MissingInput);
    }
    read_rows(data, delimiter)
}

pub(super) fn csv_count_chart(query: &str) -> Option<Result<(String, BinPlot), CsvError>> {
    let (operation, rest) = split_argument(query)?;
    match operation.to_ascii_lowercase().as_str() {
        "列分箱图" | "column-bin-plot" => Some(build_column_bin_plot(rest)),
        "函数分箱图" | "function-bin-plot" => Some(build_function_bin_plot(rest)),
        "分组频数图" | "group-frequency-plot" => Some(build_group_frequency_plot(rest)),
        _ => None,
    }
}

fn build_group_frequency_plot(input: &str) -> Result<(String, BinPlot), CsvError> {
    let (columns, data) = column_reference_arguments(input, CsvError::GroupingColumns)?;
    let data = data.strip_prefix('\u{feff}').unwrap_or(data);
    if data.is_empty() {
        return Err(CsvError::MissingInput);
    }
    let rows = read_rows(data, ',')?;
    let table = group_frequencies(&rows, &columns)?;
    Ok((table.csv()?, table.plot()?))
}

fn column_bin_arguments(
    input: &str,
) -> Result<(ColumnReference, Vec<BigRational>, &str), CsvError> {
    let ((column, edges), data) =
        json_argument_and_data::<(ColumnReference, String)>(input, CsvError::ColumnBinsArguments)?;
    let edges = parse_bin_edges(&edges).map_err(CsvError::Statistics)?;
    Ok((column, edges, data))
}

fn build_column_bin_plot(input: &str) -> Result<(String, BinPlot), CsvError> {
    let (column, edges, data) = column_bin_arguments(input)?;
    let data = data.strip_prefix('\u{feff}').unwrap_or(data);
    if data.is_empty() {
        return Err(CsvError::MissingInput);
    }
    let rows = read_rows(data, ',')?;
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let column = column.resolve(header)?;
    validate_column_width(&[column], header.len())?;
    let values = numeric_column_values_at(records, column)?;
    let (csv, plot) =
        plot_binned_unweighted(values, &edges).map_err(csv_column_statistics_error)?;
    Ok((csv, plot.with_label(header[column].clone())))
}

fn build_function_bin_plot(input: &str) -> Result<(String, BinPlot), CsvError> {
    fn bin_error(error: StatisticsError) -> CsvError {
        match error {
            StatisticsError::Incomplete => CsvError::FunctionBinEmpty,
            StatisticsError::InvalidBinEdges => CsvError::FunctionBinEdges,
            error => CsvError::Statistics(error),
        }
    }

    let invalid = CsvError::FunctionBinArguments;
    let ((columns, expression, edges), data) =
        json_argument_and_data::<(Vec<ColumnReference>, String, String)>(input, invalid)?;
    let FunctionRows { values, .. } = read_function_rows(data, &columns, &expression, invalid)?;
    let (csv, plot) = plot_binned_float_values(values.into_iter().flatten().collect(), &edges)
        .map_err(bin_error)?;
    Ok((csv, plot.with_label(expression.trim().to_owned())))
}

pub(super) fn column_box_plot_arguments(query: &str) -> Option<&str> {
    let (operation, rest) = split_argument(query)?;
    (operation == "列箱线图" || operation.eq_ignore_ascii_case("column-box-plot")).then_some(rest)
}

pub(super) fn build_column_box_plot(input: &str) -> Result<(String, BoxPlot), CsvError> {
    let (column, rows) = read_observation_table(input)?;
    let (name, values) = column_observations(&rows, column)?;
    let (text, plot) = plot_box_unweighted(values).map_err(csv_column_statistics_error)?;
    Ok((text, plot.with_label(name.to_owned())))
}

pub(super) fn column_cumulative_plot_arguments(query: &str) -> Option<&str> {
    let (operation, rest) = split_argument(query)?;
    (operation == "列累计分布图" || operation.eq_ignore_ascii_case("column-ecdf-plot"))
        .then_some(rest)
}

pub(super) fn build_column_cumulative_plot(input: &str) -> Result<(String, ScatterPlot), CsvError> {
    let (column, rows) = read_observation_table(input)?;
    let (name, values) = column_observations(&rows, column)?;
    let (csv, plot) = super::statistics::plot_cumulative_unweighted(values)
        .map_err(csv_column_statistics_error)?;
    Ok((csv, plot.with_axis_names([name.to_owned(), String::new()])))
}

fn read_observation_table(input: &str) -> Result<(ColumnReference, Vec<Vec<String>>), CsvError> {
    let (column, data) = single_column_argument(input, CsvError::ObservationColumn)?;
    let data = data.strip_prefix('\u{feff}').unwrap_or(data);
    if data.is_empty() {
        return Err(CsvError::MissingInput);
    }
    Ok((column, read_rows(data, ',')?))
}

fn column_observations(
    rows: &[Vec<String>],
    column: ColumnReference,
) -> Result<(&str, Vec<BigRational>), CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let column = column.resolve(header)?;
    validate_column_width(&[column], header.len())?;
    let mut values = Vec::new();
    for (index, record) in records.iter().enumerate() {
        let text = record[column].trim();
        if text.is_empty() {
            continue;
        }
        let value = parse_fraction_literal(text).map_err(|error| CsvError::DataRowValue {
            row: index + 1,
            cause: error.message(),
        })?;
        if values.len() == MAX_VALUES {
            return Err(CsvError::Statistics(StatisticsError::TooManyValues));
        }
        values.push(value);
    }
    Ok((&header[column], values))
}

fn numeric_column_values(
    rows: &[Vec<String>],
    column: ColumnReference,
) -> Result<Vec<BigRational>, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let column = column.resolve(header)?;
    validate_column_width(&[column], header.len())?;
    numeric_column_values_at(records, column)
}

fn numeric_column_values_at(
    records: &[Vec<String>],
    column: usize,
) -> Result<Vec<BigRational>, CsvError> {
    records
        .iter()
        .map(|row| parse_fraction_literal(&row[column]).map_err(|_| CsvError::InvalidNumber))
        .collect()
}

const NUMERIC_PROFILE_HEADINGS: [&str; 7] = [
    "数值数",
    "空白数",
    "非数值数",
    "最小值",
    "最大值",
    "平均值",
    "中位数",
];

fn csv_weighted_mean(input: &str) -> Result<String, CsvError> {
    let ([value, weight], data) =
        json_argument_and_data::<[ColumnReference; 2]>(input, CsvError::WeightedMeanArguments)?;
    let rows = read_table_for_preview(data, ',')?;
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let [value, weight] = resolve_weighted_columns(header, [value, weight])?;

    let mut mean = WeightedMean::default();
    for (index, row) in records.iter().enumerate() {
        record_weighted_csv_row(&mut mean, &row[value], &row[weight], index + 1)?;
    }
    mean.finish().map_err(CsvError::Statistics)
}

fn resolve_weighted_columns(
    header: &[String],
    [value, weight]: [ColumnReference; 2],
) -> Result<[usize; 2], CsvError> {
    let columns = [value.resolve(header)?, weight.resolve(header)?];
    validate_column_width(&columns, header.len())?;
    Ok(columns)
}

fn record_weighted_csv_row(
    mean: &mut WeightedMean,
    value: &str,
    weight: &str,
    row: usize,
) -> Result<(), CsvError> {
    if value.trim().is_empty() || weight.trim().is_empty() {
        return Ok(());
    }
    parse_weighted_pair(value, &normalize(weight))
        .and_then(|pair| mean.record(pair))
        .map_err(|error| CsvError::DataRowValue {
            row,
            cause: if error == StatisticsError::InvalidWeight {
                "权重须为非负数"
            } else {
                error.message()
            },
        })
}

fn csv_weighted_quantiles(input: &str) -> Result<CsvPreviewData, CsvError> {
    let invalid = CsvError::WeightedQuantileArguments;
    let ((value, weight, probabilities), data) =
        json_argument_and_data::<(ColumnReference, ColumnReference, Vec<Box<RawValue>>)>(
            input, invalid,
        )?;
    let probabilities = csv_quantile_probabilities(&probabilities, invalid)?;
    let rows = read_table_for_preview(data, ',')?;
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let [value, weight] = resolve_weighted_columns(header, [value, weight])?;
    let mut distribution = WeightedDistribution::default();
    let mut count = 0;
    for (index, row) in records.iter().enumerate() {
        let (value, weight) = (row[value].trim(), row[weight].trim());
        if value.is_empty() || weight.is_empty() {
            continue;
        }
        count += 1;
        let row_error = |error: StatisticsError| CsvError::DataRowValue {
            row: index + 1,
            cause: error.message(),
        };
        if count > MAX_VALUES {
            return Err(row_error(StatisticsError::TooManyValues));
        }
        let pair = parse_weighted_pair(value, &normalize(weight)).map_err(row_error)?;
        distribution.record(pair).map_err(row_error)?;
    }
    let csv = distribution
        .quantiles_csv(probabilities)
        .map_err(CsvError::Statistics)?;
    let rows = read_table_for_preview(&csv, ',')?;
    Ok((csv, rows))
}

fn csv_quantile_probabilities(
    probabilities: &[Box<RawValue>],
    invalid: CsvError,
) -> Result<Vec<BigRational>, CsvError> {
    if !(1..=MAX_QUANTILE_PROBABILITIES).contains(&probabilities.len()) {
        return Err(CsvError::Statistics(StatisticsError::QuantileList));
    }
    probabilities
        .iter()
        .map(|probability| csv_probability(probability, invalid))
        .collect()
}

fn csv_grouped_quantiles(input: &str) -> Result<CsvPreviewData, CsvError> {
    let invalid = CsvError::GroupQuantileArguments;
    let ((groups, value, probabilities), data) =
        json_argument_and_data::<(Vec<ColumnReference>, ColumnReference, Vec<Box<RawValue>>)>(
            input, invalid,
        )?;
    let probabilities = csv_quantile_probabilities(&probabilities, invalid)?;
    let probability_labels: Vec<_> = probabilities.iter().map(ToString::to_string).collect();
    let rows = read_table_for_preview(data, ',')?;
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let groups = resolve_distinct_columns(header, &groups, CsvError::GroupingColumns)?;
    let value = value.resolve(header)?;
    validate_column_width(&[value], header.len())?;

    let mut csv = String::new();
    let limit = CsvError::StatisticsOutputTooLarge;
    append_bounded_csv_row(
        &mut csv,
        groups
            .iter()
            .map(|&index| header[index].as_str())
            .chain(["概率", "分位数"]),
        limit,
    )?;
    for group in grouped_row_positions(records, &groups) {
        let mut numbers = Vec::new();
        for &index in &group {
            let cell = records[index][value].trim();
            if cell.is_empty() {
                continue;
            }
            let at_row = |cause| CsvError::DataRowValue {
                row: index + 1,
                cause,
            };
            if numbers.len() == MAX_VALUES {
                return Err(at_row(StatisticsError::TooManyValues.message()));
            }
            numbers.push(parse_fraction_literal(cell).map_err(|error| at_row(error.message()))?);
        }
        numbers.sort_unstable();
        let key = column_key(&records[group[0]], &groups);
        for (probability, label) in probabilities.iter().zip(&probability_labels) {
            let quantile = if numbers.is_empty() {
                String::new()
            } else {
                interpolate_ordered_quantile(numbers.len() as u64, probability, |rank| {
                    &numbers[rank as usize]
                })
                .map_err(CsvError::Statistics)?
                .to_string()
            };
            append_bounded_csv_row(
                &mut csv,
                key.iter()
                    .copied()
                    .chain([label.as_str(), quantile.as_str()]),
                limit,
            )?;
        }
    }
    let rows = read_table_for_preview(&csv, ',')?;
    Ok((csv, rows))
}

enum GroupTimestampReport {
    Summary,
    EarliestRows,
    LatestRows,
}

fn csv_grouped_timestamps(
    input: &str,
    report: GroupTimestampReport,
) -> Result<CsvPreviewData, CsvError> {
    let invalid = CsvError::GroupTimestampSummaryArguments;
    let ((groups, source), data) =
        json_argument_and_data::<(Vec<ColumnReference>, ColumnReference)>(input, invalid)?;
    let rows = read_table_for_preview(data, ',')?;
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let groups = resolve_distinct_columns(header, &groups, CsvError::GroupingColumns)?;
    let source = source.resolve(header)?;
    validate_column_width(&[source], header.len())?;

    let mut csv = String::new();
    let limit = CsvError::StatisticsOutputTooLarge;
    if matches!(report, GroupTimestampReport::Summary) {
        append_bounded_csv_row(
            &mut csv,
            groups.iter().map(|&column| header[column].as_str()).chain([
                "有效时间戳",
                "最早",
                "最晚",
                "跨度",
            ]),
            limit,
        )?;
    } else {
        append_bounded_csv_row(&mut csv, header.iter().map(String::as_str), limit)?;
    }
    for group in grouped_row_positions(records, &groups) {
        let mut count = 0;
        let mut earliest: Option<(DateTime<Utc>, usize)> = None;
        let mut latest: Option<(DateTime<Utc>, usize)> = None;
        for &index in &group {
            let cell = records[index][source].as_str();
            if cell.trim().is_empty() {
                continue;
            }
            let current = absolute_timestamp(cell).map_err(|error| CsvError::DataRowValue {
                row: index + 1,
                cause: error.message(),
            })?;
            if earliest.is_none_or(|(time, _)| current < time) {
                earliest = Some((current, index));
            }
            if latest.is_none_or(|(time, _)| current > time) {
                latest = Some((current, index));
            }
            count += 1;
        }
        if !matches!(report, GroupTimestampReport::Summary) {
            let selected = if matches!(report, GroupTimestampReport::EarliestRows) {
                earliest
            } else {
                latest
            };
            if let Some((_, index)) = selected {
                append_bounded_csv_row(&mut csv, records[index].iter().map(String::as_str), limit)?;
            }
            continue;
        }
        let (first, last, span) = match earliest.zip(latest) {
            Some(((start, first), (end, last))) => {
                let span = format_absolute_elapsed(start, end).map_err(|error| {
                    CsvError::DataRowValue {
                        row: group[0] + 1,
                        cause: error.message(),
                    }
                })?;
                (
                    records[first][source].as_str(),
                    records[last][source].as_str(),
                    span,
                )
            }
            None => ("", "", String::new()),
        };
        let count = count.to_string();
        let key = column_key(&records[group[0]], &groups);
        append_bounded_csv_row(
            &mut csv,
            key.into_iter()
                .chain([count.as_str(), first, last, span.as_str()]),
            limit,
        )?;
    }
    let rows = read_table_for_preview(&csv, ',')?;
    Ok((csv, rows))
}

fn csv_run_summary(input: &str) -> Result<CsvPreviewData, CsvError> {
    let invalid = CsvError::NamedColumnArguments;
    let (groups, data) = json_argument_and_data::<Vec<ColumnReference>>(input, invalid)?;
    let rows = read_table_for_preview(data, ',')?;
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let groups = resolve_distinct_columns(header, &groups, invalid)?;

    let mut csv = String::new();
    let limit = CsvError::StatisticsOutputTooLarge;
    append_bounded_csv_row(
        &mut csv,
        std::iter::once("段号")
            .chain(groups.iter().map(|&index| header[index].as_str()))
            .chain(["起始数据行", "末尾数据行", "记录数"]),
        limit,
    )?;
    let mut last = 0;
    for (index, run) in contiguous_csv_groups(records, &groups).enumerate() {
        let number = (index + 1).to_string();
        let first = (last + 1).to_string();
        last += run.len();
        let [last_row, length] = [last.to_string(), run.len().to_string()];
        append_bounded_csv_row(
            &mut csv,
            std::iter::once(number.as_str())
                .chain(groups.iter().map(|&column| run[0][column].as_str()))
                .chain([first.as_str(), last_row.as_str(), length.as_str()]),
            limit,
        )?;
    }
    let rows = read_table_for_preview(&csv, ',')?;
    Ok((csv, rows))
}

fn csv_grouped_weighted_frequency(input: &str) -> Result<CsvPreviewData, CsvError> {
    let invalid = CsvError::WeightedFrequencyArguments;
    let ((groups, weight), data) =
        json_argument_and_data::<(Vec<ColumnReference>, ColumnReference)>(input, invalid)?;
    if groups.is_empty() {
        return Err(invalid);
    }
    let rows = read_table_for_preview(data, ',')?;
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let grouped_by = resolve_distinct_columns(header, &groups, CsvError::GroupingColumns)?;
    let weight_column = weight.resolve(header)?;
    validate_column_width(&[weight_column], header.len())?;

    let mut weights = HashMap::<Vec<&str>, (usize, BigRational)>::new();
    let mut total = BigRational::default();
    for (index, row) in records.iter().enumerate() {
        let (_, group_weight) = weights
            .entry(column_key(row, &grouped_by))
            .or_insert_with(|| (index, BigRational::default()));
        let input_weight = row[weight_column].trim();
        if input_weight.is_empty() {
            continue;
        }
        let weight = parse_statistical_weight(&normalize(input_weight)).map_err(|error| {
            CsvError::DataRowValue {
                row: index + 1,
                cause: error.message(),
            }
        })?;
        *group_weight += &weight;
        total += weight;
        ensure_exact_size(group_weight)
            .and_then(|()| ensure_exact_size(&total))
            .map_err(|error| CsvError::DataRowValue {
                row: index + 1,
                cause: error.message(),
            })?;
    }
    if !records.is_empty() && total.numer().sign() == Sign::NoSign {
        return Err(CsvError::Statistics(StatisticsError::ZeroTotalWeight));
    }
    let mut weights: Vec<_> = weights.into_iter().collect();
    weights.sort_unstable_by(|(_, (left_index, left)), (_, (right_index, right))| {
        right.cmp(left).then_with(|| left_index.cmp(right_index))
    });
    let headings = grouped_by
        .iter()
        .map(|&column| header[column].clone())
        .chain(["权重合计".to_owned(), "占比".to_owned()])
        .collect::<Vec<_>>();
    let mut csv = String::new();
    let limit = CsvError::StatisticsOutputTooLarge;
    append_bounded_csv_row(&mut csv, headings.iter().map(String::as_str), limit)?;
    let mut summary = vec![headings];
    for (key, (_, weight)) in weights {
        let share = &weight / &total;
        ensure_exact_size(&share).map_err(|_| CsvError::ComputedNumberRange)?;
        let row = key
            .into_iter()
            .map(str::to_owned)
            .chain([weight.to_string(), share.to_string()])
            .collect::<Vec<_>>();
        append_bounded_csv_row(&mut csv, row.iter().map(String::as_str), limit)?;
        summary.push(row);
    }
    Ok((csv, summary))
}

fn csv_grouped_weighted_mean(input: &str) -> Result<CsvPreviewData, CsvError> {
    let ((groups, value, weight), data) = json_argument_and_data::<(
        Vec<ColumnReference>,
        ColumnReference,
        ColumnReference,
    )>(input, CsvError::WeightedGroupArguments)?;
    if groups.is_empty() {
        return Err(CsvError::WeightedGroupArguments);
    }
    let rows = read_table_for_preview(data, ',')?;
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let grouped_by = resolve_distinct_columns(header, &groups, CsvError::GroupingColumns)?;
    let [value, weight] = resolve_weighted_columns(header, [value, weight])?;

    let mut groups = HashMap::<Vec<&str>, (usize, WeightedMean)>::new();
    for (index, row) in records.iter().enumerate() {
        let (_, mean) = groups
            .entry(column_key(row, &grouped_by))
            .or_insert_with(|| (index, WeightedMean::default()));
        record_weighted_csv_row(mean, &row[value], &row[weight], index + 1)?;
    }
    let mut groups: Vec<_> = groups.into_iter().collect();
    groups.sort_unstable_by_key(|(_, (index, _))| *index);
    let headings = grouped_by
        .iter()
        .map(|&index| header[index].clone())
        .chain(["权重合计".to_owned(), "加权平均".to_owned()])
        .collect::<Vec<_>>();
    let mut csv = String::new();
    let limit = CsvError::StatisticsOutputTooLarge;
    append_bounded_csv_row(&mut csv, headings.iter().map(String::as_str), limit)?;
    let mut result = vec![headings];
    for (keys, (_, mean)) in groups {
        let total = mean.total_weight().to_string();
        let value = match mean.finish() {
            Ok(value) => value,
            Err(StatisticsError::ZeroTotalWeight) => String::new(),
            Err(error) => return Err(CsvError::Statistics(error)),
        };
        let row = keys
            .into_iter()
            .map(str::to_owned)
            .chain([total, value])
            .collect::<Vec<_>>();
        append_bounded_csv_row(&mut csv, row.iter().map(String::as_str), limit)?;
        result.push(row);
    }
    Ok((csv, result))
}

fn numeric_column_profile(
    rows: &[Vec<String>],
    columns: &[ColumnReference],
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let columns = resolve_distinct_columns(header, columns, CsvError::NamedColumnArguments)?;
    let mut output = String::new();
    let limit = CsvError::StatisticsOutputTooLarge;
    append_bounded_csv_row(
        &mut output,
        ["列号", "列名"].into_iter().chain(NUMERIC_PROFILE_HEADINGS),
        limit,
    )?;
    for column in columns {
        let cells = records
            .iter()
            .enumerate()
            .map(|(index, row)| (index + 1, row[column].as_str()));
        let values = numeric_profile(cells)?;
        let ordinal = (column + 1).to_string();
        append_bounded_csv_row(
            &mut output,
            [ordinal.as_str(), header[column].as_str()]
                .into_iter()
                .chain(values.iter().map(String::as_str)),
            limit,
        )?;
    }
    Ok(output)
}

fn grouped_numeric_profile(
    rows: &[Vec<String>],
    columns: &[ColumnReference],
    amount: ColumnReference,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let columns = resolve_distinct_columns(header, columns, CsvError::GroupingColumns)?;
    let amount = amount.resolve(header)?;
    validate_column_width(&[amount], header.len())?;
    let mut output = String::new();
    let limit = CsvError::StatisticsOutputTooLarge;
    append_bounded_csv_row(
        &mut output,
        columns
            .iter()
            .map(|&index| header[index].as_str())
            .chain(NUMERIC_PROFILE_HEADINGS),
        limit,
    )?;
    for group in grouped_row_positions(records, &columns) {
        let values = numeric_profile(
            group
                .iter()
                .map(|&index| (index + 1, records[index][amount].as_str())),
        )?;
        let key = column_key(&records[group[0]], &columns);
        append_bounded_csv_row(
            &mut output,
            key.into_iter().chain(values.iter().map(String::as_str)),
            limit,
        )?;
    }
    Ok(output)
}

fn numeric_profile<'a>(
    cells: impl Iterator<Item = (usize, &'a str)>,
) -> Result<[String; 7], CsvError> {
    let mut numbers = Vec::new();
    let mut totals = RunningTotals::default();
    let mut blanks = 0usize;
    let mut other = 0usize;
    for (row, cell) in cells {
        let cell = cell.trim();
        if cell.is_empty() {
            blanks += 1;
            continue;
        }
        match parse_fraction_literal(cell) {
            Ok(number) => {
                if numbers.len() == MAX_VALUES {
                    return Err(CsvError::Statistics(StatisticsError::TooManyValues));
                }
                totals.push(number.clone()).map_err(CsvError::Statistics)?;
                numbers.push(number);
            }
            Err(error @ (FractionError::OutOfRange | FractionError::TooLong)) => {
                return Err(CsvError::DataRowValue {
                    row,
                    cause: error.message(),
                });
            }
            Err(_) => other += 1,
        }
    }
    numbers.sort_unstable();
    let [min, max, average, median] = if let Some(min) = numbers.first() {
        let max = numbers.last().expect("至少有一个数值");
        let average = totals.format(true).map_err(CsvError::Statistics)?;
        let median = median_sorted_values(&numbers)
            .map_err(|error| CsvError::Statistics(StatisticsError::Evaluation(error)))?;
        [
            min.to_string(),
            max.to_string(),
            average,
            median.to_string(),
        ]
    } else {
        Default::default()
    };
    Ok([
        numbers.len().to_string(),
        blanks.to_string(),
        other.to_string(),
        min,
        max,
        average,
        median,
    ])
}

#[derive(Clone, Copy)]
enum PairMatrixStatistic {
    Correlation,
    Spearman,
    Covariance { sample: bool },
}

struct CsvPoint<'a> {
    row: usize,
    label: &'a str,
    coordinates: Vec<BigRational>,
}

fn csv_point_records<'a>(
    rows: &'a [Vec<String>],
    labels: ColumnReference,
    coordinates: &[ColumnReference],
) -> Result<(&'a str, Vec<CsvPoint<'a>>), CsvError> {
    const MAX_ROWS: usize = 32;
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let invalid = CsvError::DistanceMatrixArguments;
    let labels = labels.resolve(header)?;
    validate_column_width(&[labels], header.len())?;
    let coordinates = resolve_distinct_columns(header, coordinates, invalid)?;
    if coordinates.contains(&labels) {
        return Err(invalid);
    }
    let mut points = Vec::new();
    let mut seen_labels = HashSet::new();
    for (index, record) in records.iter().enumerate() {
        if coordinates
            .iter()
            .any(|&column| record[column].trim().is_empty())
        {
            continue;
        }
        if points.len() == MAX_ROWS {
            return Err(CsvError::DistanceMatrixRows);
        }
        let label = record[labels].as_str();
        let row = index + 1;
        if label.trim().is_empty() || !seen_labels.insert(label) {
            return Err(CsvError::DataRowValue {
                row,
                cause: "坐标点的标签须非空且互不重复",
            });
        }
        let values = coordinates
            .iter()
            .map(|&column| {
                parse_fraction_literal(record[column].trim()).map_err(|error| {
                    CsvError::DataRowValue {
                        row,
                        cause: error.message(),
                    }
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        points.push(CsvPoint {
            row,
            label,
            coordinates: values,
        });
    }
    if points.is_empty() {
        return Err(CsvError::DistanceMatrixRows);
    }
    Ok((header[labels].as_str(), points))
}

fn csv_point_distance_squared(
    left: &CsvPoint<'_>,
    right: &CsvPoint<'_>,
) -> Result<BigRational, CsvError> {
    squared_euclidean_distance(&left.coordinates, &right.coordinates).map_err(|error| {
        CsvError::DataRowValue {
            row: right.row,
            cause: error.message(),
        }
    })
}

fn row_distance_matrix(
    rows: &[Vec<String>],
    labels: ColumnReference,
    coordinates: &[ColumnReference],
) -> Result<String, CsvError> {
    let (heading, points) = csv_point_records(rows, labels, coordinates)?;
    let mut matrix = vec![vec!["0".to_owned(); points.len()]; points.len()];
    for (left, a) in points.iter().enumerate() {
        for (right, b) in points.iter().enumerate().skip(left + 1) {
            let distance = format_square_root(&csv_point_distance_squared(a, b)?);
            matrix[left][right] = distance.clone();
            matrix[right][left] = distance;
        }
    }
    let mut output = String::new();
    let limit = CsvError::StatisticsOutputTooLarge;
    append_bounded_csv_row(
        &mut output,
        std::iter::once(heading).chain(points.iter().map(|point| point.label)),
        limit,
    )?;
    for (point, distances) in points.iter().zip(matrix) {
        append_bounded_csv_row(
            &mut output,
            std::iter::once(point.label).chain(distances.iter().map(String::as_str)),
            limit,
        )?;
    }
    Ok(output)
}

fn nearest_point_rows(
    rows: &[Vec<String>],
    labels: ColumnReference,
    coordinates: &[ColumnReference],
    target: &str,
    limit: usize,
) -> Result<String, CsvError> {
    let (_, points) = csv_point_records(rows, labels, coordinates)?;
    let reference = points
        .iter()
        .find(|point| point.label == target)
        .ok_or(CsvError::NearestPointReference)?;
    let mut candidates = points
        .iter()
        .filter(|point| point.row != reference.row)
        .map(|point| Ok((point, csv_point_distance_squared(reference, point)?)))
        .collect::<Result<Vec<_>, CsvError>>()?;
    candidates
        .sort_unstable_by(|(a, left), (b, right)| left.cmp(right).then_with(|| a.row.cmp(&b.row)));
    let mut output = "数据行,标签,距离".to_owned();
    for (point, distance_squared) in candidates.into_iter().take(limit) {
        let row = point.row.to_string();
        let distance = format_square_root(&distance_squared);
        append_bounded_csv_row(
            &mut output,
            [row.as_str(), point.label, distance.as_str()].into_iter(),
            CsvError::StatisticsOutputTooLarge,
        )?;
    }
    Ok(output)
}

fn pair_matrix_arguments(input: &str) -> Result<(Vec<ColumnReference>, &str), CsvError> {
    let (columns, data) = column_reference_arguments(input, CsvError::PairMatrixColumns)?;
    if !(2..=8).contains(&columns.len()) {
        return Err(CsvError::PairMatrixColumns);
    }
    Ok((columns, data))
}

pub(super) fn correlation_plot_arguments(query: &str) -> Option<(&str, bool)> {
    let (operation, rest) = split_argument(query)?;
    match operation.to_ascii_lowercase().as_str() {
        "相关图" | "correlation-plot" => Some((rest, false)),
        "秩相关图" | "spearman-plot" => Some((rest, true)),
        _ => None,
    }
}

pub(super) fn build_correlation_plot(
    input: &str,
    rank: bool,
) -> Result<(String, CorrelationPlot), CsvError> {
    let (columns, data) = pair_matrix_arguments(input)?;
    let data = data.strip_prefix('\u{feff}').unwrap_or(data);
    if data.is_empty() {
        return Err(CsvError::MissingInput);
    }
    let rows = read_rows(data, ',')?;
    let statistic = if rank {
        PairMatrixStatistic::Spearman
    } else {
        PairMatrixStatistic::Correlation
    };
    let (csv, plot) = column_pair_matrix(&rows, &columns, statistic, true)?;
    Ok((csv, plot.expect("相关图保留图表数据")))
}

fn column_pair_matrix(
    rows: &[Vec<String>],
    columns: &[ColumnReference],
    statistic: PairMatrixStatistic,
    show_plot: bool,
) -> Result<(String, Option<CorrelationPlot>), CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let columns = resolve_distinct_columns(header, columns, CsvError::PairMatrixColumns)?;
    let width = columns.len();
    let mut values_by_column = vec![Vec::new(); width];
    let mut complete_rows = Vec::new();
    for (index, row) in records.iter().enumerate() {
        let cells: Vec<_> = columns.iter().map(|&column| row[column].trim()).collect();
        if cells.iter().any(|cell| cell.is_empty()) {
            continue;
        }
        let at_row = |cause| CsvError::DataRowValue {
            row: index + 1,
            cause,
        };
        for (values, cell) in values_by_column.iter_mut().zip(cells) {
            values.push(parse_fraction_literal(cell).map_err(|error| at_row(error.message()))?);
        }
        complete_rows.push(index + 1);
        if complete_rows.len() > MAX_VALUES {
            return Err(CsvError::Statistics(StatisticsError::TooManyValues));
        }
    }
    if complete_rows.len() < 2 {
        return Err(CsvError::PairMatrixRows);
    }
    if matches!(statistic, PairMatrixStatistic::Spearman) {
        for values in &mut values_by_column {
            *values = doubled_midranks(values);
        }
    }
    let mut matrix = vec![vec![CorrelationCell::default(); width]; width];
    for (left, x) in values_by_column.iter().enumerate() {
        for (right, y) in values_by_column.iter().enumerate().skip(left) {
            let mut pair = PairedSums::default();
            for ((x, y), &row) in x.iter().zip(y).zip(&complete_rows) {
                pair.record(x, y).map_err(|error| CsvError::DataRowValue {
                    row,
                    cause: error.message(),
                })?;
            }
            let value = match statistic {
                PairMatrixStatistic::Correlation | PairMatrixStatistic::Spearman => {
                    pair.correlation().map(|value| CorrelationCell {
                        exact: value.exact(),
                        hundredths: show_plot.then(|| value.hundredths()),
                    })
                }
                PairMatrixStatistic::Covariance { sample } => {
                    pair.covariance(sample).map(|covariance| CorrelationCell {
                        exact: covariance.to_string(),
                        hundredths: None,
                    })
                }
            };
            let coefficient = match value {
                Ok(value) => value,
                Err(LinearFitError::ConstantCoordinate) => continue,
                Err(error) => return Err(CsvError::LinearFit(error)),
            };
            matrix[left][right] = coefficient.clone();
            matrix[right][left] = coefficient;
        }
    }

    let mut output = String::new();
    let names: Vec<_> = columns
        .iter()
        .map(|&column| header[column].as_str())
        .collect();
    let limit = CsvError::StatisticsOutputTooLarge;
    append_bounded_csv_row(
        &mut output,
        std::iter::once("列").chain(names.iter().copied()),
        limit,
    )?;
    for (&name, coefficients) in names.iter().zip(&matrix) {
        let fields =
            std::iter::once(name).chain(coefficients.iter().map(|cell| cell.exact.as_str()));
        append_bounded_csv_row(&mut output, fields, limit)?;
    }
    let plot = show_plot.then(|| {
        let axis_names = columns
            .iter()
            .zip(&names)
            .map(|(&index, &name)| {
                if name.trim().is_empty() {
                    format!("第 {} 列", index + 1)
                } else if names.iter().filter(|&&other| other == name).count() > 1 {
                    format!("{} {name}", index + 1)
                } else {
                    name.to_owned()
                }
            })
            .collect();
        CorrelationPlot::new(
            axis_names,
            matrix,
            complete_rows.len(),
            matches!(statistic, PairMatrixStatistic::Spearman),
        )
    });
    Ok((output, plot))
}

fn csv_column_statistics_error(error: StatisticsError) -> CsvError {
    match error {
        StatisticsError::Incomplete => CsvError::EmptyStatisticsColumn,
        error => CsvError::Statistics(error),
    }
}

fn validate_column_width(columns: &[usize], width: usize) -> Result<(), CsvError> {
    if columns.iter().any(|&index| index >= width) {
        return Err(CsvError::ColumnOutOfRange);
    }
    Ok(())
}

fn json_objects(
    rows: &[Vec<String>],
    numeric_columns: Option<&[ColumnReference]>,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let mut names = HashSet::new();
    if header
        .iter()
        .any(|name| name.trim().is_empty() || !names.insert(name))
    {
        return Err(CsvError::InvalidHeader);
    }
    let numeric_columns = numeric_columns
        .map(|columns| {
            resolve_distinct_columns(header, columns, CsvError::NamedColumnArguments)
                .map(|indices| indices.into_iter().collect::<HashSet<_>>())
        })
        .transpose()?;
    let objects = records.iter().map(|fields| CsvObject {
        header,
        fields,
        numeric_columns: numeric_columns.as_ref(),
    });
    if numeric_columns.is_none() {
        return serde_json::to_string(&objects.collect::<Vec<_>>())
            .map_err(|_| CsvError::Serialization);
    }
    let mut output = String::from("[");
    for (index, object) in objects.enumerate() {
        let row = serde_json::to_string(&object).map_err(|_| CsvError::JsonNumericCells)?;
        let separator = if index == 0 { "" } else { "," };
        if output.len() + separator.len() + row.len() + 1 > MAX_EXPANDED_CSV_BYTES {
            return Err(CsvError::JsonNumericObjectsTooLarge);
        }
        output.push_str(separator);
        output.push_str(&row);
    }
    output.push(']');
    Ok(output)
}

fn markdown_table(
    rows: &[Vec<String>],
    alignments: Option<&[MarkdownAlignment]>,
) -> Result<String, CsvError> {
    let width = rows[0].len();
    if alignments.is_some_and(|alignments| alignments.len() != width) {
        return Err(CsvError::MarkdownAlignments);
    }
    let rules = (0..width)
        .map(
            |column| match alignments.map(|alignments| &alignments[column]) {
                Some(MarkdownAlignment::Left) => ":---",
                Some(MarkdownAlignment::Center) => ":---:",
                Some(MarkdownAlignment::Right) => "---:",
                Some(MarkdownAlignment::Default) | None => "---",
            },
        )
        .collect::<Vec<_>>();
    let mut lines = Vec::with_capacity(rows.len() + 1);
    for (index, row) in rows.iter().enumerate() {
        let cells = row
            .iter()
            .map(|cell| markdown_cell(cell))
            .collect::<Vec<_>>();
        lines.push(format!("| {} |", cells.join(" | ")));
        if index == 0 {
            lines.push(format!("| {} |", rules.join(" | ")));
        }
    }
    Ok(lines.join("\n"))
}

fn read_markdown_table(input: &str) -> Result<String, CsvError> {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input).trim();
    let mut lines = newline_pattern().split(input);
    let (Some(header), Some(alignment)) = (lines.next(), lines.next()) else {
        return Err(CsvError::MarkdownTable);
    };
    let (header, header_has_pipe) = markdown_row(header);
    let (alignment, alignment_has_pipe) = markdown_row(alignment);
    let width = header.len();
    if !(header_has_pipe || alignment_has_pipe)
        || width == 0
        || width != alignment.len()
        || !alignment.iter().all(|cell| {
            let dashes = cell.strip_prefix(':').unwrap_or(cell);
            let dashes = dashes.strip_suffix(':').unwrap_or(dashes);
            !dashes.is_empty() && dashes.bytes().all(|byte| byte == b'-')
        })
    {
        return Err(CsvError::MarkdownTable);
    }
    let mut rows = vec![header];
    for line in lines {
        let (row, _) = markdown_row(line);
        if row.is_empty() {
            return Err(CsvError::MarkdownTable);
        }
        if row.len() > width {
            return Err(CsvError::DifferentColumnCount);
        }
        rows.push(row);
    }
    pad_rows(rows)
}

fn markdown_row(line: &str) -> (Vec<String>, bool) {
    let line = line.trim();
    let inner = line.strip_prefix('|');
    let line = inner.unwrap_or(line);
    let mut escaped = false;
    let mut cells = line
        .split(move |character| {
            let separator = character == '|' && !escaped;
            escaped = character == '\\' && !escaped;
            separator
        })
        .collect::<Vec<_>>();
    let has_pipe = inner.is_some() || cells.len() > 1;
    if cells.last() == Some(&"") {
        cells.pop();
    }
    (
        cells.into_iter().map(decode_markdown_cell).collect(),
        has_pipe,
    )
}

fn decode_markdown_cell(input: &str) -> String {
    let mut input = input.trim();
    let mut output = String::with_capacity(input.len());
    while !input.is_empty() {
        if let Some(rest) = ["<br>", "<br/>", "<br />"]
            .into_iter()
            .find_map(|tag| input.strip_prefix(tag))
        {
            output.push('\n');
            input = rest;
            continue;
        }
        let mut characters = input.chars();
        let character = characters.next().expect("单元格还有字符");
        if character == '\\' && characters.clone().next().is_some_and(markdown_needs_escape) {
            output.push(characters.next().expect("转义符后还有字符"));
        } else {
            output.push(character);
        }
        input = characters.as_str();
    }
    output
}

fn markdown_needs_escape(character: char) -> bool {
    matches!(
        character,
        '\\' | '|' | '`' | '*' | '_' | '~' | '[' | ']' | '<' | '>' | '&' | '$'
    )
}

fn markdown_cell(cell: &str) -> String {
    let mut result = String::with_capacity(cell.len());
    let mut chars = cell.chars().peekable();
    while let Some(ch) = chars.next() {
        if matches!(ch, '\r' | '\n') {
            if ch == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            result.push_str("<br>");
            continue;
        }
        if markdown_needs_escape(ch) {
            result.push('\\');
        }
        result.push(ch);
    }
    result
}

fn transpose(rows: &[Vec<String>]) -> Vec<Vec<String>> {
    let columns = rows.first().map_or(0, Vec::len);
    (0..columns)
        .map(|column| rows.iter().map(|row| row[column].clone()).collect())
        .collect()
}

pub(super) fn write_rows<'a, Row: AsRef<[String]> + ?Sized + 'a>(
    rows: impl IntoIterator<Item = &'a Row>,
    delimiter: char,
) -> String {
    let separator = delimiter.to_string();
    rows.into_iter()
        .map(|row| {
            row.as_ref()
                .iter()
                .map(|cell| separated_cell(cell, delimiter))
                .collect::<Vec<_>>()
                .join(&separator)
        })
        .collect::<Vec<_>>()
        .join("\r\n")
}

pub(super) fn csv_cell(cell: &str) -> Cow<'_, str> {
    separated_cell(cell, ',')
}

fn separated_cell(cell: &str, delimiter: char) -> Cow<'_, str> {
    if cell.is_empty()
        || cell.starts_with('\u{feff}')
        || cell.contains([delimiter, '"', '\r', '\n'])
    {
        Cow::Owned(format!("\"{}\"", cell.replace('"', "\"\"")))
    } else {
        Cow::Borrowed(cell)
    }
}

fn pad_rows(mut rows: Vec<Vec<String>>) -> Result<String, CsvError> {
    let width = rows
        .iter()
        .map(Vec::len)
        .max()
        .ok_or(CsvError::MissingInput)?;
    let missing: usize = rows.iter().map(|row| width - row.len()).sum();
    if missing > MAX_ADDED_CELLS {
        return Err(CsvError::TooManyAddedCells);
    }
    for row in &mut rows {
        row.resize_with(width, String::new);
    }
    Ok(write_rows(&rows, ','))
}

pub(super) fn csv_validation_report(
    query: &str,
) -> Option<Result<InputValidationReport, CsvError>> {
    let (operation, input) = split_argument(query)?;
    let delimiter = validation_delimiter(operation)?;
    let offset = query.len() - input.len();
    Some(validate_csv(input, delimiter).map(|report| report.offset_error_span(offset)))
}

fn validation_delimiter(operation: &str) -> Option<char> {
    if super::is_validation_operation(operation) {
        Some(',')
    } else if operation.eq_ignore_ascii_case("校验tsv")
        || operation.eq_ignore_ascii_case("validate-tsv")
    {
        Some('\t')
    } else {
        None
    }
}

fn validate_csv(input: &str, delimiter: char) -> Result<InputValidationReport, CsvError> {
    let data = input.strip_prefix('\u{feff}').unwrap_or(input);
    if data.is_empty() {
        return Err(CsvError::MissingInput);
    }
    let offset = input.len() - data.len();
    let format = if delimiter == '\t' { "TSV" } else { "CSV" };
    let mut record_spans = Vec::new();
    Ok(
        match read_delimited_rows(data, delimiter, false, Some(&mut record_spans)) {
            Ok(rows) => {
                let width = rows[0].len();
                let error_span = rows.iter().zip(record_spans).find_map(|(row, range)| {
                    (row.len() != width).then_some(offset + range.start..offset + range.end)
                });
                InputValidationReport {
                    text: report_csv_columns(&rows),
                    error_span,
                }
            }
            Err(error) => InputValidationReport {
                text: format!(
                    "{format} 语法错误 · 第 {} 条记录 · 第 {} 列\n{}",
                    error.record,
                    error.column,
                    error.kind.message()
                ),
                error_span: Some(offset + error.span.start..offset + error.span.end),
            },
        },
    )
}

fn report_csv_columns(rows: &[Vec<String>]) -> String {
    let width = rows[0].len();
    let mismatches: Vec<_> = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.len() != width)
        .map(|(index, row)| format!("第 {} 条记录有 {} 列", index + 1, row.len()))
        .collect();
    if mismatches.is_empty() {
        return format!("{} 条记录 · 每条 {} 列", rows.len(), width);
    }
    let header = format!(
        "{} 条记录 · 首条 {} 列 · {} 条列数不一致",
        rows.len(),
        width,
        mismatches.len()
    );
    format!("{header}\n{}", mismatches.join("\n"))
}

struct CsvReadError {
    kind: CsvError,
    record: usize,
    column: usize,
    span: Range<usize>,
}

fn auto_convert_delimiter(input: &str) -> Result<String, CsvError> {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    if input.is_empty() {
        return Err(CsvError::MissingInput);
    }
    let mut candidates = [',', '\t', ';', '|']
        .into_iter()
        .filter(|&delimiter| input.contains(delimiter))
        .filter_map(|delimiter| read_rows(input, delimiter).ok())
        .filter(|rows| rows.len() >= 2 && rows[0].len() >= 2);
    let rows = candidates.next().ok_or(CsvError::UnknownDelimiter)?;
    if candidates.next().is_some() {
        return Err(CsvError::AmbiguousDelimiter);
    }
    Ok(write_rows(&rows, ','))
}

fn read_rows(input: &str, delimiter: char) -> Result<Vec<Vec<String>>, CsvError> {
    read_delimited_rows(input, delimiter, true, None).map_err(|error| error.kind)
}

fn read_delimited_rows(
    mut input: &str,
    delimiter: char,
    require_uniform_width: bool,
    mut record_spans: Option<&mut Vec<Range<usize>>>,
) -> Result<Vec<Vec<String>>, CsvReadError> {
    let source_length = input.len();
    let mut record_start = 0;
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut fields = Vec::new();
    loop {
        let record = rows.len() + 1;
        let column = fields.len() + 1;
        let at_field = |kind, span| CsvReadError {
            kind,
            record,
            column,
            span,
        };
        let field_start = source_length - input.len();
        let (field, rest) = read_field(input, delimiter).map_err(|kind| {
            let span = if kind == CsvError::UnclosedQuote {
                field_start..source_length
            } else {
                let quote = field_start + input.find('"').unwrap_or_default();
                quote..quote + 1
            };
            at_field(kind, span)
        })?;
        fields.push(field);
        if let Some(next) = rest.strip_prefix(delimiter) {
            input = next;
            continue;
        }
        let field_end = source_length - rest.len();
        input = if rest.is_empty() {
            rest
        } else {
            rest.strip_prefix("\r\n")
                .or_else(|| rest.strip_prefix('\n'))
                .ok_or_else(|| {
                    let width = rest.chars().next().map_or(0, char::len_utf8);
                    at_field(CsvError::InvalidSeparator, field_end..field_end + width)
                })?
        };
        if require_uniform_width
            && rows
                .first()
                .is_some_and(|first| first.len() != fields.len())
        {
            return Err(at_field(
                CsvError::DifferentColumnCount,
                field_start..field_end,
            ));
        }
        rows.push(std::mem::take(&mut fields));
        if let Some(spans) = record_spans.as_deref_mut() {
            spans.push(record_start..field_end);
            record_start = source_length - input.len();
        }
        if input.is_empty() {
            return Ok(rows);
        }
    }
}

fn read_field(input: &str, delimiter: char) -> Result<(String, &str), CsvError> {
    if let Some(mut quoted) = input.strip_prefix('"') {
        let mut value = String::new();
        loop {
            let (part, rest) = quoted.split_once('"').ok_or(CsvError::UnclosedQuote)?;
            value.push_str(part);
            let Some(next) = rest.strip_prefix('"') else {
                return Ok((value, rest));
            };
            value.push('"');
            quoted = next;
        }
    }
    let end = input
        .find([delimiter, '\r', '\n', '"'])
        .unwrap_or(input.len());
    let (field, rest) = input.split_at(end);
    if rest.starts_with('"') {
        return Err(CsvError::InvalidSeparator);
    }
    Ok((field.to_owned(), rest))
}

struct CsvObject<'a> {
    header: &'a [String],
    fields: &'a [String],
    numeric_columns: Option<&'a HashSet<usize>>,
}

impl Serialize for CsvObject<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut object = serializer.serialize_map(Some(self.header.len()))?;
        for (index, (key, value)) in self.header.iter().zip(self.fields).enumerate() {
            if self
                .numeric_columns
                .is_some_and(|columns| columns.contains(&index))
            {
                let value = value.trim();
                let number = if value.is_empty() {
                    None
                } else {
                    if !matches!(value.as_bytes().first(), Some(b'-' | b'0'..=b'9')) {
                        return Err(serde::ser::Error::custom("预期 JSON 数字"));
                    }
                    Some(
                        serde_json::from_str::<&RawValue>(value)
                            .map_err(<S::Error as serde::ser::Error>::custom)?,
                    )
                };
                object.serialize_entry(key, &number)?;
            } else {
                object.serialize_entry(key, value)?;
            }
        }
        object.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bins_a_selected_csv_column_using_exact_fraction_edges() {
        assert_eq!(
            transform_csv(
                "列分箱 [\"金额\",\"0,1/2,2\"] 编号,金额\n001,-1\n002,0\n003,0.50\n004,1\n005,2.00"
            )
            .as_deref(),
            Ok("下界,上界,次数,频率\r\n-∞,0,1,1/5\r\n0,1/2,1,1/5\r\n1/2,2,2,2/5\r\n2,+∞,1,1/5"),
        );
        for (query, error) in [
            (
                r#"column-bins [3,"0"] 编号,金额"#,
                CsvError::ColumnOutOfRange,
            ),
            (
                r#"列分箱 [2,"0"] 编号,金额"#,
                CsvError::EmptyStatisticsColumn,
            ),
            (
                r#"列分箱 ["金额","1/2,0.5"] 编号,金额"#,
                CsvError::Statistics(StatisticsError::InvalidBinEdges),
            ),
            (
                "列分箱 [2,\"0\"] 编号,金额\n001,坏",
                CsvError::InvalidNumber,
            ),
        ] {
            assert_eq!(transform_csv(query), Err(error), "{query}");
        }
    }

    #[test]
    fn keeps_complete_highest_or_lowest_rows_per_group_with_stable_ties() {
        let input = "地区,班次,金额,备注\n北,A,1/2,甲\n南,A,-2,乙\n北,A,2e0,丙\n北,B,9,丁\n北,A,0.50,戊\n南,A,1/3,己";
        assert_eq!(
            transform_csv(&format!("组内最高 [[\"地区\",2],\"金额\",2] {input}")).as_deref(),
            Ok("地区,班次,金额,备注\r\n北,A,2e0,丙\r\n北,A,1/2,甲\r\n南,A,1/3,己\r\n南,A,-2,乙\r\n北,B,9,丁")
        );
        assert_eq!(
            transform_csv(&format!("group-bottom [[1,2],3,2] {input}")).as_deref(),
            Ok("地区,班次,金额,备注\r\n北,A,1/2,甲\r\n北,A,0.50,戊\r\n南,A,-2,乙\r\n南,A,1/3,己\r\n北,B,9,丁")
        );
        assert_eq!(
            transform_csv("组内最高 [[1],2,9] 地区,金额").as_deref(),
            Ok("地区,金额"),
        );
        for (query, error) in [
            (
                "组内最高 [[1],2,0] 地区,金额",
                CsvError::GroupSelectionArguments,
            ),
            (
                "组内最低 [[],2,2] 地区,金额",
                CsvError::GroupSelectionArguments,
            ),
            (
                "组内最高 [[1,\"地区\"],2,3] 地区,金额",
                CsvError::GroupingColumns,
            ),
            (
                "组内最高 [[1],2,1] 地区,金额\n北,待确认",
                CsvError::InvalidNumber,
            ),
        ] {
            assert_eq!(transform_csv(query), Err(error), "{query}");
        }
    }

    #[test]
    fn integrates_exact_signed_and_unsigned_areas_between_aligned_curves() {
        let data = "[1,2,3] 时间,实测,基准\n3,0,3\n0,0,0\n1,2,";
        for (operation, expected) in [("曲线间面积", "3"), ("curve-difference-integral", "-3/2")]
        {
            assert_eq!(
                transform_csv(&format!("{operation} {data}")).as_deref(),
                Ok(expected)
            );
        }
        assert_eq!(
            transform_csv("area-between-curves [1,2,3] x,a,b\n0,0,\n1,2,3\n2,,4").as_deref(),
            Ok("0")
        );
        assert_eq!(
            transform_csv("曲线差积分 [1,2,3] x,a,b\n0,0,\n1,2,\n2,,3\n3,,4"),
            Err(CsvError::CoordinateDifferenceNoOverlap)
        );
    }

    #[test]
    fn finds_intersections_and_overlaps_between_independently_sampled_curves() {
        assert_eq!(
            transform_csv("曲线交点 [1,2,3] x,a,b\n4,1,1\n0,0,0\n1,1,1\n2,2,\n3,0,1\n5,1,0")
                .as_deref(),
            Ok("起始横坐标,结束横坐标,类型\r\n0,1,重合区间\r\n5/2,5/2,交点\r\n4,4,交点")
        );
        assert_eq!(
            transform_csv("curve-crossings [1,2,3] x,a,b\n0,0,1\n1,0,1").as_deref(),
            Ok("起始横坐标,结束横坐标,类型")
        );
        assert_eq!(
            transform_csv("曲线交点 [1,2,3] x,a,b\n0,0,\n1,2,2\n2,,3").as_deref(),
            Ok("起始横坐标,结束横坐标,类型\r\n1,1,交点")
        );
    }

    #[test]
    fn plots_distinct_connected_curves_with_the_exact_difference_table() {
        let query = "[[1,2,3],\"差值\"] 时间,实测,基准\n5,,10\n0,0,\n2,4,\n1,,3\n4,4,\n3,,6";
        let (csv, plot) = coordinate_line_chart(&format!("曲线对比图 {query}"))
            .unwrap()
            .unwrap();
        assert_eq!(plot.point_count(), 8);
        assert!(plot.has_curve());
        assert_eq!(
            transform_csv(&format!("curve-comparison-plot {query}")),
            Ok(csv)
        );
    }

    #[test]
    fn aligns_independently_sampled_curves_at_all_knots_in_their_shared_range() {
        let data = "时间,实测,基准\n5,,10\n0,0,\n2,4,\n1,,3\n4,4,\n3,,6";
        assert_eq!(
            transform_csv(&format!(
                "曲线差值 [[\"时间\",\"实测\",\"基准\"],\"差值\"] {data}"
            ))
            .as_deref(),
            Ok("时间,实测,基准,差值\r\n1,2,3,-1\r\n2,4,9/2,-1/2\r\n3,4,6,-2\r\n4,4,8,-4")
        );
        assert_eq!(
            transform_csv("curve-difference [[1,2,3],\"差值\"] x,a,b\n0,0,\n1,2,3\n2,,4")
                .as_deref(),
            Ok("x,a,b,差值\r\n1,2,3,-1")
        );
        assert_eq!(
            transform_csv("曲线差值 [[1,2,3],\"差值\"] x,a,b\n0,0,\n1,2,\n2,,3\n3,,4"),
            Err(CsvError::CoordinateDifferenceNoOverlap)
        );
        assert_eq!(
            transform_csv("曲线差值 [[1,2,3],\"差值\"] x,a,b\n1/2,0,0\n0.5,2,\n2,,3"),
            Err(CsvError::CoordinateDuplicateX)
        );
        assert_eq!(
            transform_csv(&format!("曲线差值 [[1,2,3],\"时间\"] {data}")),
            Err(CsvError::CoordinateDifferenceArguments)
        );
    }

    #[test]
    fn inverts_the_exact_height_distribution_including_level_segments() {
        let data = "x,y\n7,2\n3,2\n6,\n1,0\n0,0";
        for (probability, expected) in [
            ("0", "0"),
            ("\"1/7\"", "0"),
            ("\"25%\"", "3/4"),
            ("\"2/7\"", "1"),
            ("0.5", "2"),
            ("1", "2"),
        ] {
            assert_eq!(
                transform_csv(&format!("曲线分位数 [[1,2],{probability}] {data}")).as_deref(),
                Ok(expected)
            );
        }
        assert_eq!(
            transform_csv("curve-quantile [[1,2],\"50%\"] x,y\n0,0\n2,4\n4,0").as_deref(),
            Ok("2")
        );
        assert_eq!(
            transform_csv("曲线分位数 [[1,2],\"25%\"] x,y\n0,2\n4,-2").as_deref(),
            Ok("-1")
        );
        assert_eq!(
            transform_csv("曲线分位数 [[1,2],1.1] x,y\n0,0\n2,4"),
            Err(CsvError::Statistics(StatisticsError::InvalidProbability))
        );
    }

    #[test]
    fn measures_exact_continuous_spread_around_the_interval_mean() {
        let data = "x,y\n3,3\n0,0\n2,";
        assert_eq!(
            transform_csv(&format!("曲线标准差 [1,2] {data}")).as_deref(),
            Ok("√(3/4)")
        );
        assert_eq!(
            transform_csv(&format!("curve-standard-deviation [[1,2],[0,1]] {data}")).as_deref(),
            Ok("√(1/12)")
        );
        assert_eq!(
            transform_csv("曲线标准差 [1,2] x,y\n0,1/3\n1,1/3\n3,1/3").as_deref(),
            Ok("0")
        );
    }

    #[test]
    fn integrates_the_square_of_the_linear_curve_over_exact_coordinates() {
        let data = "x,y\n3,3\n0,0\n2,";
        assert_eq!(
            transform_csv(&format!("曲线均方根 [1,2] {data}")).as_deref(),
            Ok("√3")
        );
        assert_eq!(
            transform_csv(&format!("curve-rms [[1,2],[\"1/2\",\"5/2\"]] {data}")).as_deref(),
            Ok("√(31/12)")
        );
        assert_eq!(
            transform_csv("曲线均方根 [1,2] x,y\n-1,-2\n1,-2").as_deref(),
            Ok("2")
        );
        assert_eq!(
            transform_csv("曲线均方根 [1,2] x,y\n-1,-1\n1,1").as_deref(),
            Ok("√(1/3)")
        );
    }

    #[test]
    fn sums_exact_curve_lengths_as_rational_values_and_radicals() {
        let triangle = "x,y\n6,0\n3,4\n0,0\n2,";
        assert_eq!(
            transform_csv(&format!("曲线长度 [1,2] {triangle}")).as_deref(),
            Ok("10")
        );
        assert_eq!(
            transform_csv(&format!("curve-length [[1,2],[1,5]] {triangle}")).as_deref(),
            Ok("20/3")
        );
        assert_eq!(
            transform_csv("曲线长度 [1,2] x,y\n0,0\n1,1\n2,0\n5,4").as_deref(),
            Ok("5 + 2√2")
        );
        assert_eq!(
            transform_csv("曲线长度 [1,2] x,y\n0,0\n1/2,1/2").as_deref(),
            Ok("√(1/2)")
        );
    }

    #[test]
    fn sums_height_changes_without_cancelling_opposite_directions() {
        let data = "x,y\n3,0\n1,2\n2,\n0,-1";
        assert_eq!(
            transform_csv(&format!("曲线变差 [[1,2],[\"1/2\",\"5/2\"]] {data}")).as_deref(),
            Ok("3")
        );
        assert_eq!(
            transform_csv(&format!("curve-total-variation [1,2] {data}")).as_deref(),
            Ok("5")
        );
        assert_eq!(
            transform_csv("曲线变差 [1,2] x,y\n-1,2\n1,2").as_deref(),
            Ok("0")
        );
    }

    #[test]
    fn keeps_original_knots_and_adds_only_exact_clipped_endpoints() {
        let data = "x,y\n3,3\n1,2\n2,\n0,0";
        assert_eq!(
            transform_csv(&format!(
                "曲线截取 [[\"x\",\"y\"],[\"1/2\",\"5/2\"]] {data}"
            ))
            .as_deref(),
            Ok("x,y\r\n1/2,1\r\n1,2\r\n5/2,11/4")
        );
        assert_eq!(
            transform_csv(&format!("clip-curve [[1,2],[1,3]] {data}")).as_deref(),
            Ok("x,y\r\n1,2\r\n3,3")
        );
        assert_eq!(
            transform_csv(&format!("曲线截取 [[1,2],[0,5]] {data}")),
            Err(CsvError::TrapezoidIntervalRange)
        );
    }

    #[test]
    fn reports_exact_curve_extrema_and_plateaus_over_the_selected_coordinate_interval() {
        let data = "时间,高度\n4,2\n2,2\n0,0\n3,1\n1,2";
        assert_eq!(
            transform_csv(&format!("曲线极值 [[1,2],[\"1/2\",3]] {data}")).as_deref(),
            Ok("类型,起始横坐标,结束横坐标,纵坐标\r\n最小值,1/2,1/2,1\r\n最大值,1,2,2\r\n最小值,3,3,1")
        );
        assert_eq!(
            transform_csv("curve-extrema [1,2] x,y\n0,3\n1,3\n2,3").as_deref(),
            Ok("类型,起始横坐标,结束横坐标,纵坐标\r\n最小值与最大值,0,2,3")
        );
        assert_eq!(
            transform_csv(&format!("曲线极值 [[1,2],[0,5]] {data}")),
            Err(CsvError::TrapezoidIntervalRange)
        );
    }

    #[test]
    fn measures_only_the_requested_interval_using_exact_interpolated_endpoints() {
        let data = "x,y\n3,2\n2,\n0,-2\n1,0";
        let input = format!("[[\"x\",\"y\"],[\"1/2\",\"5/2\"]] {data}");
        for (operation, expected) in [
            ("梯形积分", "7/8"),
            ("trapezoid-mean", "7/16"),
            ("trapezoid-area", "11/8"),
        ] {
            assert_eq!(
                transform_csv(&format!("{operation} {input}")).as_deref(),
                Ok(expected)
            );
        }
        for (bounds, error) in [
            ("[2,2]", CsvError::TrapezoidIntervalOrder),
            ("[3,1]", CsvError::TrapezoidIntervalOrder),
            ("[-1,2]", CsvError::TrapezoidIntervalRange),
            ("[1,4]", CsvError::TrapezoidIntervalRange),
            ("[1,\"间距\"]", CsvError::TrapezoidIntervalArguments),
        ] {
            assert_eq!(
                transform_csv(&format!("梯形积分 [[1,2],{bounds}] {data}")),
                Err(error)
            );
        }
        assert_eq!(
            transform_csv(&format!("梯形积分 [[1,2],[0,3]] {data}")),
            transform_csv(&format!("梯形积分 [1,2] {data}"))
        );
    }

    #[test]
    fn splits_crossing_segments_when_measuring_unsigned_trapezoid_area() {
        let input = "[1,2] x,y\n3,2\n0,-1";
        assert_eq!(
            transform_csv(&format!("梯形面积 {input}")).as_deref(),
            Ok("5/2")
        );
        assert_eq!(
            transform_csv(&format!("梯形积分 {input}")).as_deref(),
            Ok("3/2")
        );
        assert_eq!(
            transform_csv("trapezoid-area [1,2] x,y\n0,-1\n1,-3\n2,0").as_deref(),
            Ok("7/2")
        );
        assert_eq!(
            transform_csv("梯形面积 [1,2] x,y\n0,0\n2,0").as_deref(),
            Ok("0")
        );
    }

    #[test]
    fn plots_an_exact_line_through_complete_samples_in_horizontal_order() {
        let query =
            "折线图 [\"时间\",\"高度\"] 编号,时间,高度\n003,3,2\n001,0,-1/2\n002,2,\n004,1,0";
        let (csv, plot) = coordinate_line_chart(query).unwrap().unwrap();
        assert_eq!(plot.point_count(), 3);
        assert!(plot.has_curve());
        assert_eq!(csv, "时间,高度\r\n0,-1/2\r\n1,0\r\n3,2");
        assert_eq!(transform_csv(query), Ok(csv));
        assert!(matches!(
            coordinate_line_chart("LINE-PLOT [1,2] x,y\n1/2,0\n0.5,1"),
            Some(Err(CsvError::CoordinateDuplicateX))
        ));
    }

    #[test]
    fn plots_exact_cumulative_trapezoid_values_in_horizontal_order() {
        let query = "梯形累计图 [\"时间\",\"高度\"] 时间,高度\n3,2\n0,-1\n2,\n1,0";
        let (csv, plot) = coordinate_line_chart(query).unwrap().unwrap();
        assert_eq!(plot.point_count(), 3);
        assert_eq!(csv, "时间,累计积分\r\n0,0\r\n1,-1/2\r\n3,3/2");
        assert_eq!(transform_csv(query), Ok(csv));
    }

    #[test]
    fn weights_trapezoid_means_by_exact_horizontal_spans() {
        assert_eq!(
            transform_csv("梯形平均 [\"x\",\"y\"] x,y\n3,2\n2,\n0,0\n1,2").as_deref(),
            Ok("5/3")
        );
        assert_eq!(
            transform_csv("trapezoid-mean [2,1] 高度,时间\n-3,2\n-1,0").as_deref(),
            Ok("-2")
        );
        assert_eq!(
            transform_csv("梯形平均 [1,2] x,y\n1/2,2\n0.50,3"),
            Err(CsvError::TrapezoidDuplicateX)
        );
    }

    #[test]
    fn resamples_irregular_points_on_an_exact_even_grid() {
        let data = "x,y\n3,3\n1,2\n2,\n0,0";
        assert_eq!(
            transform_csv(&format!("等距插值 [[\"x\",\"y\"],5] {data}")).as_deref(),
            Ok("x,y\r\n0,0\r\n3/4,3/2\r\n3/2,9/4\r\n9/4,21/8\r\n3,3")
        );
        assert_eq!(
            transform_csv(&format!("resample-linear [[1,2],2] {data}")).as_deref(),
            Ok("x,y\r\n0,0\r\n3,3")
        );
        assert_eq!(
            transform_csv(&format!("等距插值 [[1,2],65] {data}")),
            Err(CsvError::CoordinateResampleArguments)
        );
    }

    #[test]
    fn integrates_only_the_exact_positive_excess_over_a_threshold() {
        let data = "x,y\n6,-1\n2,1\n4,1\n3,\n0,-1";
        assert_eq!(
            transform_csv(&format!("超阈面积 [[1,2],0] {data}")).as_deref(),
            Ok("3")
        );
        assert_eq!(
            transform_csv(&format!("area-above-threshold [[1,2],\"1/2\"] {data}")).as_deref(),
            Ok("5/4")
        );
        assert_eq!(
            transform_csv(&format!("超阈面积 [[1,2],1] {data}")).as_deref(),
            Ok("0")
        );
        assert_eq!(
            transform_csv("超阈面积 [[1,2],\"-1/4\"] x,y\n-2,1/2\n-1,3/2").as_deref(),
            Ok("5/4")
        );
    }

    #[test]
    fn sums_exact_horizontal_spans_strictly_above_the_requested_level() {
        let data = "x,y\n6,-1\n2,1\n4,1\n3,\n0,-1";
        assert_eq!(
            transform_csv(&format!("超阈跨度 [[1,2],0] {data}")).as_deref(),
            Ok("4")
        );
        assert_eq!(
            transform_csv(&format!("span-above-threshold [[1,2],\"1/2\"] {data}")).as_deref(),
            Ok("3")
        );
        assert_eq!(
            transform_csv("超阈跨度 [[1,2],0] x,y\n0,0\n2,0\n3,1\n4,0").as_deref(),
            Ok("2")
        );
    }

    #[test]
    fn preserves_open_threshold_boundaries_and_separates_intervals_touching_at_a_point() {
        let data = "x,y\n6,-1\n1,0\n4,0\n3,0\n2,2\n5,1\n0,1";
        assert_eq!(
            transform_csv(&format!("超阈区间 [[1,2],0] {data}")).as_deref(),
            Ok("起始横坐标,结束横坐标,包含起点,包含终点\r\n0,1,是,否\r\n1,3,否,否\r\n4,11/2,否,否")
        );
        assert_eq!(
            transform_csv("intervals-above-threshold [[1,2],\"-1/2\"] x,y\n0,0\n2,0").as_deref(),
            Ok("起始横坐标,结束横坐标,包含起点,包含终点\r\n0,2,是,是")
        );
        assert_eq!(
            transform_csv("超阈区间 [[1,2],0] x,y\n0,-1\n1,0\n2,-1").as_deref(),
            Ok("起始横坐标,结束横坐标,包含起点,包含终点")
        );
    }

    #[test]
    fn reports_exact_crossings_and_collapses_consecutive_equal_samples_to_an_interval() {
        let query = "水平交点 [[1,2],0] x,y\n4,0\n1,0\n2,2\n3,-2\n0,0";
        assert_eq!(
            transform_csv(query).as_deref(),
            Ok("起始横坐标,结束横坐标,类型\r\n0,1,重合区间\r\n5/2,5/2,交点\r\n4,4,交点")
        );
        assert_eq!(
            transform_csv("horizontal-crossings [[1,2],[1]] x,y\n0,0\n1,1"),
            Err(CsvError::CoordinateCrossingArguments)
        );
        assert_eq!(
            transform_csv("horizontal-crossings [[1,2],\"3/2\"] x,y\n0,0\n1,1").as_deref(),
            Ok("起始横坐标,结束横坐标,类型")
        );
        assert_eq!(
            transform_csv("水平交点 [[1,2],0] x,y\n1/2,0\n0.50,1"),
            Err(CsvError::CoordinateDuplicateX)
        );
    }

    #[test]
    fn exports_exact_piecewise_linear_coefficients_with_each_segments_domain() {
        let data = "x,y\n3,3\n0,-1/2\n2,\n1,2";
        assert_eq!(
            transform_csv(&format!("分段直线 [\"x\",\"y\"] {data}")).as_deref(),
            Ok("起始横坐标,结束横坐标,斜率,截距\r\n0,1,5/2,-1/2\r\n1,3,1/2,3/2")
        );
        assert_eq!(
            transform_csv("linear-segments [1,2] x,y\n-1,-2\n1,-2").as_deref(),
            Ok("起始横坐标,结束横坐标,斜率,截距\r\n-1,1,0,-2")
        );
        assert_eq!(
            transform_csv("分段直线 [1,2] x,y\n1/2,0\n0.50,2"),
            Err(CsvError::CoordinateDuplicateX)
        );
    }

    #[test]
    fn reports_exact_one_sided_slopes_at_knots_and_within_line_segments() {
        let data = "x,y\n3,3\n0,0\n2,\n1,2";
        assert_eq!(
            transform_csv(&format!("曲线斜率 [[1,2],[0,\"1/2\",1,3]] {data}")).as_deref(),
            Ok("横坐标,左侧斜率,右侧斜率\r\n0,\"\",2\r\n1/2,2,2\r\n1,2,1/2\r\n3,1/2,\"\"")
        );
        assert_eq!(
            transform_csv(&format!("curve-slopes [[1,2],[4]] {data}")),
            Err(CsvError::CoordinateInterpolationRange)
        );
    }

    #[test]
    fn interpolates_exact_values_at_requested_coordinates_in_input_order() {
        let data = "x,y\n3,3\n1,2\n2,\n0,0";
        assert_eq!(
            transform_csv(&format!("线性插值 [[\"x\",\"y\"],[\"1/2\",2,0,3]] {data}")).as_deref(),
            Ok("x,y\r\n1/2,1\r\n2,5/2\r\n0,0\r\n3,3")
        );
        assert_eq!(
            transform_csv(&format!("linear-interpolate [[1,2],[4]] {data}")),
            Err(CsvError::CoordinateInterpolationRange)
        );
        assert_eq!(
            transform_csv("线性插值 [[1,2],[0.5]] x,y\n1/2,1\n0.50,2"),
            Err(CsvError::CoordinateDuplicateX)
        );
        assert_eq!(
            transform_csv(&format!("线性插值 [[1,2],[true]] {data}")),
            Err(CsvError::CoordinateInterpolationArguments)
        );
    }

    #[test]
    fn appends_exact_secant_slopes_in_the_input_order_and_preserves_gaps() {
        let input = "差商列 [\"斜率\",[\"x\",\"y\"]] 编号,x,y\n003,3,3\n000,0,0\n002,2,\n001,1,2";
        assert_eq!(
            transform_csv(input).as_deref(),
            Ok("编号,x,y,斜率\r\n003,3,3,1/2\r\n000,0,0,\"\"\r\n002,2,\"\",\"\"\r\n001,1,2,2")
        );
        assert_eq!(
            transform_csv("secant-column [\"斜率\",[1,2]] x,y\n1/2,2\n0.50,3"),
            Err(CsvError::SecantDuplicateX)
        );
        assert_eq!(
            transform_csv("差商列 [\"斜率\",[1,2]] x,y\n1/2,2\n0.50,"),
            Err(CsvError::SecantPointCount)
        );
    }

    #[test]
    fn averages_exact_trailing_windows_at_irregular_samples_in_original_row_order() {
        let data = "编号,x,y\n003,3,4\n000,0,0\n002,2,\n001,1,2";
        assert_eq!(
            transform_csv(&format!(
                "时间窗平均列 [\"窗均值\",[\"x\",\"y\"],\"3/2\"] {data}"
            ))
            .as_deref(),
            Ok("编号,x,y,窗均值\r\n003,3,4,13/4\r\n000,0,0,\"\"\r\n002,2,\"\",\"\"\r\n001,1,2,1")
        );
        assert_eq!(
            transform_csv("time-window-mean-column [\"均值\",[1,2],2] x,y\n0,0\n1,2\n3,4")
                .as_deref(),
            Ok("x,y,均值\r\n0,0,\"\"\r\n1,2,1\r\n3,4,3")
        );
        assert_eq!(
            transform_csv(&format!("时间窗平均列 [\"均值\",[2,3],0] {data}")),
            Err(CsvError::CoordinateWindowArguments)
        );
        assert_eq!(
            transform_csv("时间窗平均列 [\"均值\",[1,2],1] x,y\n1/2,1\n0.5,2"),
            Err(CsvError::CoordinateDuplicateX)
        );
    }

    #[test]
    fn integrates_exact_trailing_windows_at_irregular_samples_in_original_row_order() {
        let query =
            "时间窗积分列 [\"窗口积分\",[2,3],\"3/2\"] 编号,x,y\n003,3,4\n000,0,0\n002,2,\n001,1,2";
        assert_eq!(
            transform_csv(query).as_deref(),
            Ok("编号,x,y,窗口积分\r\n003,3,4,39/8\r\n000,0,0,\"\"\r\n002,2,\"\",\"\"\r\n001,1,2,1")
        );
        assert_eq!(
            transform_csv("time-window-integral-column [\"面积\",[1,2],3] x,y\n0,-2\n2,-2")
                .as_deref(),
            Ok("x,y,面积\r\n0,-2,\"\"\r\n2,-2,-4")
        );
    }

    #[test]
    fn appends_cumulative_trapezoids_in_the_original_csv_row_order() {
        let input =
            "梯形累计列 [\"积分\",[\"x\",\"y\"]] 编号,x,y\n003,3,2\n000,0,-1\n002,2,\n001,1,0";
        assert_eq!(
            transform_csv(input).as_deref(),
            Ok("编号,x,y,积分\r\n003,3,2,3/2\r\n000,0,-1,0\r\n002,2,\"\",\"\"\r\n001,1,0,-1/2")
        );
        assert_eq!(
            transform_csv("梯形累计列 [\"y\",[1,2]] x,y\n0,0\n1,1"),
            Err(CsvError::HeaderNameConflict)
        );
        assert_eq!(
            transform_csv("cumulative-trapezoid-column [\"积分\",[1,2]] x,y\n1/2,2\n0.50,3"),
            Err(CsvError::TrapezoidDuplicateX)
        );
    }

    #[test]
    fn integrates_piecewise_linear_coordinates_in_x_order_without_rounding() {
        assert_eq!(
            transform_csv("梯形积分 [\"时间\",\"高度\"] 时间,高度\n3,2\n0,-1\n2,\n1,0").as_deref(),
            Ok("3/2")
        );
        assert_eq!(
            transform_csv("trapezoid-integral [2,1] 高度,时间\n-1,1/2\n-2,2").as_deref(),
            Ok("-9/4")
        );
        assert_eq!(
            transform_csv("梯形积分 [1,2] x,y\n1/2,2\n0.50,3"),
            Err(CsvError::TrapezoidDuplicateX)
        );
        assert_eq!(
            transform_csv("梯形积分 [1,2] x,y\n1/2,2\n0.50,"),
            Err(CsvError::TrapezoidPointCount)
        );
    }

    #[test]
    fn csv_scatter_selects_numeric_columns_in_axis_order_and_skips_blanks() {
        let data = "\u{feff}记录,温度,\"分钟,累计\"\r\nA,1.50,0\r\nB,,1\r\nC,7/4,2e0";
        assert_eq!(
            transform_csv(&format!(r#"散点图 [3,"温度"] {data}"#)).as_deref(),
            Ok("\"分钟,累计\",温度\r\n0,3/2\r\n2,7/4")
        );
        assert_eq!(
            transform_csv("scatter [1,2] x,y\n1,2\n3,"),
            Err(CsvError::ScatterPointCount)
        );
        assert_eq!(
            transform_csv("散点图 [1,\"x\"] x,y\n1,2\n3,4"),
            Err(CsvError::ScatterColumns)
        );
        assert!(matches!(
            transform_csv("散点图 [1,2] x,y\n1,2\n待定,3"),
            Err(CsvError::DataRowValue { row: 2, .. })
        ));
    }

    #[test]
    fn appends_group_predictions_and_residuals_in_the_original_csv_order() {
        let data = "区域,x,y\r\n北,0,0\r\n南,2,4\r\n北,1,1\r\n北,2,1\r\n北,3,\r\n南,2,5\r\n北,,1";
        assert_eq!(
            transform_csv(&format!(r#"拟合列 [["区域"],["x",3],["预测","残差"]] {data}"#)).as_deref(),
            Ok("区域,x,y,预测,残差\r\n北,0,0,1/6,-1/6\r\n南,2,4,\"\",\"\"\r\n北,1,1,2/3,1/3\r\n北,2,1,7/6,-1/6\r\n北,3,\"\",5/3,\"\"\r\n南,2,5,\"\",\"\"\r\n北,\"\",1,\"\",\"\"")
        );
        assert_eq!(
            transform_csv("fit-columns [[],[1,2],[\"预测\",\"残差\"]] x,y\n0,2\n1,3\n2,")
                .as_deref(),
            Ok("x,y,预测,残差\r\n0,2,2,0\r\n1,3,3,0\r\n2,\"\",4,\"\"")
        );
        assert_eq!(
            transform_csv("拟合列 [[],[1,2],[\"x\",\"残差\"]] x,y"),
            Err(CsvError::FittedColumnsArguments)
        );
    }

    #[test]
    fn fits_csv_groups_in_first_seen_order_and_keeps_unfittable_groups() {
        let data = "地区,x,y\r\n\"北,区\",0,1/2\r\n南,2,1\r\n东,,3\r\n\"北,区\",1,3/2\r\n南,2,3\r\n西,0,4\r\n西,1,4";
        assert_eq!(
            transform_csv(&format!("分组拟合 [[\"地区\"],[2,\"y\"]] {data}")).as_deref(),
            Ok("地区,观测数,斜率,截距,R²\r\n\"北,区\",2,1,1/2,1\r\n南,2,\"\",\"\",\"\"\r\n东,0,\"\",\"\",\"\"\r\n西,2,0,4,\"\"")
        );
        assert_eq!(
            transform_csv("group-fit [[1],[2,3]] 地区,x,y").as_deref(),
            Ok("地区,观测数,斜率,截距,R²")
        );
        assert_eq!(
            transform_csv("分组拟合 [[1],[2,\"x\"]] 地区,x,y"),
            Err(CsvError::GroupFitColumns)
        );
        assert!(matches!(
            transform_csv("分组拟合 [[1],[2,3]] 地区,x,y\n北,1,3\n南,2,待定"),
            Err(CsvError::DataRowValue { row: 2, .. })
        ));
    }

    #[test]
    fn charts_the_most_frequent_csv_groups_and_exports_all_exact_counts() {
        let data = "\u{feff}状态\n001\n1\n001\n\"\"\nA\nB\nC\nD\nE\nF\nG";
        let (csv, chart) = csv_count_chart(&format!(r#"分组频数图 ["状态"] {data}"#))
            .unwrap()
            .unwrap();
        assert_eq!(chart.bin_count(), 8);
        assert_eq!(csv.lines().count(), 11);
        assert!(csv.starts_with("状态,次数,频率\r\n001,2,2/11\r\n1,1,1/11\r\n\"\",1,1/11"));
        assert!(csv.ends_with("G,1,1/11"));
        assert_eq!(
            transform_csv(&format!(r#"分组频数 ["状态"] {data}"#)).as_deref(),
            Ok(csv.as_str())
        );
        assert!(matches!(
            csv_count_chart("group-frequency-plot [1] 状态"),
            Some(Err(CsvError::EmptyStatisticsColumn))
        ));
    }

    #[test]
    fn computes_symmetric_exact_distances_for_complete_csv_rows() {
        let input =
            "距离矩阵 [\"编号\",[\"x\",3]] 编号,x,y\n001,0,0\n跳过,,99\n1,3/5,4/5\n\"甲,乙\",1,1";
        assert_eq!(
            transform_csv(input).as_deref(),
            Ok("编号,001,1,\"甲,乙\"\r\n001,0,1,√2\r\n1,1,0,√(1/5)\r\n\"甲,乙\",√2,√(1/5),0")
        );
        assert_eq!(
            transform_csv("distance-matrix [1,[2]] 编号,x\nA,1/3").as_deref(),
            Ok("编号,A\r\nA,0")
        );
        assert_eq!(
            transform_csv("距离矩阵 [1,[2]] 编号,x\nA,0\nA,1"),
            Err(CsvError::DataRowValue {
                row: 2,
                cause: "坐标点的标签须非空且互不重复"
            })
        );
        assert_eq!(
            transform_csv("距离矩阵 [1,[1]] 编号,x\nA,0"),
            Err(CsvError::DistanceMatrixArguments)
        );
        assert_eq!(
            transform_csv("距离矩阵 [1,[2]] 编号,x\nA,"),
            Err(CsvError::DistanceMatrixRows)
        );
        assert!(matches!(
            transform_csv("距离矩阵 [1,[2]] 编号,x\nA,待定"),
            Err(CsvError::DataRowValue { row: 1, .. })
        ));
        let mut too_many = "距离矩阵 [1,[2]] 编号,x".to_owned();
        for index in 0..33 {
            too_many.push_str(&format!("\n{index},0"));
        }
        assert_eq!(transform_csv(&too_many), Err(CsvError::DistanceMatrixRows));
    }

    #[test]
    fn finds_nearest_complete_csv_points_in_exact_distance_then_source_order() {
        let input = "最近点 [1,[2,3],\"001\",4] 编号,x,y\n001,0,0\n远,2,2\n等距甲,1,0\n略过,1,\n等距乙,0,-1\n分数,1/3,0";
        assert_eq!(
            transform_csv(input).as_deref(),
            Ok("数据行,标签,距离\r\n6,分数,1/3\r\n3,等距甲,1\r\n5,等距乙,1\r\n2,远,√8")
        );
        assert_eq!(
            transform_csv("nearest-points [1,[2],\"A\",1] 编号,x\nA,1/3").as_deref(),
            Ok("数据行,标签,距离")
        );
        assert_eq!(
            transform_csv("最近点 [1,[2],\"A\",1] 编号,x\nA,\nB,0"),
            Err(CsvError::NearestPointReference)
        );
        assert_eq!(
            transform_csv("最近点 [1,[2],\"A\",0] 编号,x\nA,0"),
            Err(CsvError::NearestPointArguments)
        );
    }

    #[test]
    fn groups_raw_csv_values_by_frequency_and_uses_first_seen_order_for_ties() {
        let data = "编号,状态\n001,完\n1,待\n001,完\n\"\",\"\"\n\"\",\"\"\n001,完\n1,待";
        assert_eq!(
            transform_csv(&format!(r#"分组频数 ["编号",2] {data}"#)).as_deref(),
            Ok("编号,状态,次数,频率\r\n001,完,3,3/7\r\n1,待,2,2/7\r\n\"\",\"\",2,2/7")
        );
        assert_eq!(
            transform_csv("group-frequency [1] 编号").as_deref(),
            Ok("编号,次数,频率")
        );
    }

    #[test]
    fn filters_complete_and_missing_csv_rows_without_changing_record_contents() {
        let data = "编号,金额,日期,备注\n001,0,2024-02-29,\"示例\n第二行\"\n002,1/2, ,乙\n003,,2026-11-23,丙\n004,1/3,2026-11-23,";
        assert_eq!(
            transform_csv(&format!(r#"完整行 ["金额",3] {data}"#)).as_deref(),
            Ok("编号,金额,日期,备注\r\n001,0,2024-02-29,\"示例\n第二行\"\r\n004,1/3,2026-11-23,\"\"")
        );
        assert_eq!(
            transform_csv(&format!(r#"缺失行 [2,"日期"] {data}"#)).as_deref(),
            Ok("编号,金额,日期,备注\r\n002,1/2, ,乙\r\n003,\"\",2026-11-23,丙")
        );
    }

    #[test]
    fn groups_csv_medians_in_first_seen_order_with_exact_even_and_odd_counts() {
        assert_eq!(
            transform_csv("分组中位数 [\"地区\",2] \"金额\" 地区,班次,金额\n\"南,区\",B,1/2\n北,A,5\n\"南,区\",B,3/2\n北,A,2e0\n北,A,3\n北,B,1.50").as_deref(),
            Ok("地区,班次,中位数\r\n\"南,区\",B,1\r\n北,A,3\r\n北,B,3/2"),
        );
        assert_eq!(
            transform_csv("group-median 1 2 地区,金额").as_deref(),
            Ok("地区,中位数"),
        );
        assert_eq!(
            transform_csv("分组中位数 [1] 2 地区,金额\n甲,待确认"),
            Err(CsvError::InvalidNumber),
        );
        let input = format!(
            "分组中位数 [1] 2 地区,金额\n{}",
            "甲,1\n".repeat(MAX_VALUES + 1)
        );
        assert_eq!(
            transform_csv(&input),
            Err(CsvError::Statistics(StatisticsError::TooManyValues)),
        );
    }

    #[test]
    fn writes_selected_csv_cells_as_raw_json_numbers_or_null() {
        let input = "编号,金额,数量\n001,1.50,9007199254740993\n002, ,2e0\n003,-0,";
        assert_eq!(
            transform_csv(&format!("数值对象 [[\"金额\"],3] {input}")),
            Err(CsvError::NamedColumnArguments),
        );
        assert_eq!(
            transform_csv(&format!("数值对象 [\"金额\",3] {input}")).as_deref(),
            Ok(
                r#"[{"编号":"001","金额":1.50,"数量":9007199254740993},{"编号":"002","金额":null,"数量":2e0},{"编号":"003","金额":-0,"数量":null}]"#
            ),
        );
        assert_eq!(
            transform_csv("numeric-objects [2] 编号,金额").as_deref(),
            Ok("[]"),
        );
        assert_eq!(
            transform_csv("对象 编号,金额\n001,1.50").as_deref(),
            Ok(r#"[{"编号":"001","金额":"1.50"}]"#),
        );
        for field in ["1/3", "001", "true", "null", "[1]", "1e", "0xff"] {
            assert_eq!(
                transform_csv(&format!("数值对象 [1] 金额\n{field}")),
                Err(CsvError::JsonNumericCells),
                "{field}",
            );
        }
    }

    #[test]
    fn formats_selected_csv_columns_with_exact_ties_to_even_and_preserves_empty_cells() {
        assert_eq!(
            transform_csv("小数列 [[\"金额\",3],2] 编号,金额,比率,备注\n001,2.675,1/8,\"茶,热\"\n002,,-0.005,\n003,100000000000000000001/3,-2.685,正常").as_deref(),
            Ok("编号,金额,比率,备注\r\n001,2.68,0.12,\"茶,热\"\r\n002,\"\",0.00,\"\"\r\n003,33333333333333333333.67,-2.68,正常")
        );
        assert_eq!(
            transform_csv("decimal-columns [[1],0] 金额\n0.5\n1.5\n2.5").as_deref(),
            Ok("金额\r\n0\r\n2\r\n2"),
        );
        assert_eq!(
            transform_csv("小数列 [[1,2],2] 金额,比例").as_deref(),
            Ok("金额,比例"),
        );
        for (query, error) in [
            (
                "小数列 [[\"金额\",1],2] 金额",
                CsvError::DecimalColumnsArguments,
            ),
            ("小数列 [[],2] 金额", CsvError::DecimalColumnsArguments),
            ("小数列 [[1],101] 金额", CsvError::DecimalColumnsArguments),
            ("小数列 [[2],2] 金额", CsvError::ColumnOutOfRange),
            ("小数列 [[1],2] 金额\n坏", CsvError::InvalidNumber),
        ] {
            assert_eq!(transform_csv(query), Err(error), "{query}");
        }
    }

    #[test]
    fn formats_selected_csv_columns_as_exact_significant_digits() {
        assert_eq!(
            transform_csv(
                "有效数字列 [[2,\"比率\"],3] 编号,金额,比率\n001,9.995,1/3\n002,-1.245,0.0012345\n003,0,"
            )
            .as_deref(),
            Ok("编号,金额,比率\r\n001,1.00e1,3.33e-1\r\n002,-1.24e0,1.23e-3\r\n003,0.00e0,\"\"")
        );
        assert_eq!(
            transform_csv("significant-columns [[1],1] 金额\n-9.5\n2.5\n3.5").as_deref(),
            Ok("金额\r\n-1e1\r\n2e0\r\n4e0")
        );
        for (query, error) in [
            (
                "有效数字列 [[\"金额\",1],3] 金额",
                CsvError::SignificantColumnsArguments,
            ),
            (
                "有效数字列 [[1],0] 金额",
                CsvError::SignificantColumnsArguments,
            ),
            (
                "有效数字列 [[1],101] 金额",
                CsvError::SignificantColumnsArguments,
            ),
            ("有效数字列 [[1],3] 金额\n坏", CsvError::InvalidNumber),
        ] {
            assert_eq!(transform_csv(query), Err(error), "{query}");
        }
    }

    #[test]
    fn previews_engineering_notation_for_selected_csv_columns() {
        let query = "工程计数列 [[\"金额\",3],3] 编号,金额,比率\n001,9.995,0.0012345\n002,-999.5,1/3\n003,,0";
        let expected =
            "编号,金额,比率\r\n001,10.0e0,1.23e-3\r\n002,-1.00e3,333e-3\r\n003,\"\",0.00e0";
        assert_eq!(transform_csv(query).as_deref(), Ok(expected));
        let (text, table) = csv_table_preview(query).unwrap().unwrap();
        assert_eq!(text, expected);
        assert_eq!(table.row_pages(), 1);
        assert_eq!(
            transform_csv("engineering-columns [[1],0] 金额\n10"),
            Err(CsvError::SignificantColumnsArguments),
        );
    }

    #[test]
    fn compares_timestamps_across_offsets_with_inclusive_nanosecond_bounds() {
        let input = "检查时间范围 [[\"时间\"],[\"0\",\"1ns\"]] 时间\n1970-01-01T08:00:00+08:00\n0.000001ms\n2ns\n-1ns";
        assert_eq!(
            transform_csv(input).as_deref(),
            Ok("数据行,列号,列名,内容\r\n3,1,时间,2ns\r\n4,1,时间,-1ns"),
        );
        assert_eq!(
            transform_csv("检查规则 [[1,\"时间范围\",\"0\",\"1ns\"]] 时间\n2ns").as_deref(),
            Ok("数据行,列号,列名,规则号,规则,内容\r\n1,1,时间,1,时间范围,2ns"),
        );
        assert_eq!(
            transform_csv(r#"检查时间范围 [[1],["1ns","0"]] 时间"#),
            Err(CsvError::TimestampRangeCheckArguments),
        );
    }

    #[test]
    fn sorts_csv_timestamps_on_the_utc_timeline_and_keeps_equal_keys_in_input_order() {
        let table = "编号,时间\nA,1970-01-01T08:00:00+08:00\nB,1ns\nC,0\nD,-0.000001ms";
        assert_eq!(
            transform_csv(&format!(r#"时间戳排序 ["时间"] {table}"#)).as_deref(),
            Ok("编号,时间\r\nD,-0.000001ms\r\nA,1970-01-01T08:00:00+08:00\r\nC,0\r\nB,1ns"),
        );
        assert_eq!(
            transform_csv(&format!("sort-timestamps-desc 2 {table}")).as_deref(),
            Ok("编号,时间\r\nB,1ns\r\nA,1970-01-01T08:00:00+08:00\r\nC,0\r\nD,-0.000001ms"),
        );
        assert!(matches!(
            transform_csv("时间戳排序 2 编号,时间\nA,0\nB,2026-11-01 01:30 America/New_York"),
            Err(CsvError::DataRowValue { row: 2, .. }),
        ));
    }

    #[test]
    fn reports_ambiguous_and_inexact_timestamps_with_their_source_cells() {
        let input = "检查时间戳列 [\"时间\"] 时间\n-0.000001ms\n2026-11-01T01:30:00-04:00\n2026-11-01 01:30 America/New_York\n0.1ns\n现在";
        assert_eq!(
            transform_csv(input).as_deref(),
            Ok("数据行,列号,列名,内容\r\n3,1,时间,2026-11-01 01:30 America/New_York\r\n4,1,时间,0.1ns\r\n5,1,时间,现在"),
        );
        assert_eq!(
            transform_csv("检查规则 [[1,\"时间戳\"]] 时间\n\"\"").as_deref(),
            Ok("数据行,列号,列名,规则号,规则,内容\r\n1,1,时间,1,时间戳,\"\""),
        );
    }

    #[test]
    fn checks_full_json_values_with_locations_for_invalid_cells() {
        let input =
            "检查JSON列 [\"数据\"] 编号,数据\nA,null\nB,\"{\"\"x\"\":1}\"\nC,True\nD,\nE,\"[1,]\"";
        assert_eq!(
            transform_csv(input).as_deref(),
            Ok("数据行,列号,列名,内容\r\n3,2,数据,True\r\n4,2,数据,\"\"\r\n5,2,数据,\"[1,]\""),
        );
        assert_eq!(
            transform_csv("检查规则 [[1,\"JSON\"]] 数据\nfalse\n普通文本").as_deref(),
            Ok("数据行,列号,列名,规则号,规则,内容\r\n2,1,数据,1,JSON,普通文本"),
        );
    }

    #[test]
    fn checks_header_order_and_distinguishes_missing_from_empty_names() {
        let cases = [
            (
                r#"["编号","金额",""] 金额,编号"#,
                "1,列名不同,编号,金额\r\n2,列名不同,金额,编号\r\n3,缺少列,\"\",\"\"",
            ),
            (r#"["编号"] 编号,"需,确认""#, "2,多余列,\"\",\"需,确认\""),
            (r#"["编号",""] 编号,"#, ""),
        ];
        for (input, issues) in cases {
            let mut expected = String::from("列号,问题,预期列名,实际列名");
            if !issues.is_empty() {
                expected.push_str("\r\n");
                expected.push_str(issues);
            }
            assert_eq!(
                transform_csv(&format!("检查表头 {input}")).as_deref(),
                Ok(expected.as_str()),
            );
        }
        assert_eq!(
            transform_csv("检查表头 [] 编号"),
            Err(CsvError::HeaderCheckArguments),
        );
    }

    #[test]
    fn checks_inclusive_text_lengths_in_the_selected_unicode_unit() {
        let table = "编号,备注\nA,👩‍💻\nB,e\u{301}\nC,北京\nD,";
        assert_eq!(
            transform_csv(&format!(r#"检查长度范围 [["备注"],["字素",1,1]] {table}"#)).as_deref(),
            Ok("数据行,列号,列名,内容\r\n3,2,备注,北京\r\n4,2,备注,\"\""),
        );
        assert_eq!(
            transform_csv("检查规则 [[2,\"长度范围\",[\"UTF-8 字节\",0,5]]] 编号,备注\nA,北京\nB,")
                .as_deref(),
            Ok("数据行,列号,列名,规则号,规则,内容\r\n1,2,备注,1,长度范围,北京"),
        );
        assert_eq!(
            transform_csv(r#"检查长度范围 [[1],["字素",3,1]] 备注"#),
            Err(CsvError::LengthCheckArguments),
        );
    }

    #[test]
    fn checks_dates_by_the_given_format_and_preserves_invalid_cells() {
        let query = "检查日期格式 [[\"日期\"],\"%d/%m/%Y\"] 编号,日期\nA,29/02/2024\nB,29/02/2023\nC,2024-02-29\nD,";
        assert_eq!(
            transform_csv(query).as_deref(),
            Ok("数据行,列号,列名,内容\r\n2,2,日期,29/02/2023\r\n3,2,日期,2024-02-29\r\n4,2,日期,\"\""),
        );
        assert_eq!(
            transform_csv("check-rules [[1,\"date-format\",\"%Y-%j\"]] 日期\n2024-366\n2023-366")
                .as_deref(),
            Ok("数据行,列号,列名,规则号,规则,内容\r\n2,1,日期,1,日期格式,2023-366"),
        );
        assert_eq!(
            transform_csv(r#"检查日期格式 [[1],"%Q"] 日期"#),
            Err(CsvError::DateFormatColumnPattern),
        );
    }

    #[test]
    fn checks_csv_values_without_folding_case_or_trimming_and_can_allow_blank_cells() {
        let query = "检查允许值 [[\"状态\"],[\"待办\",\"完成\",\"\"]] 编号,状态\nA,待办\nB,完成 \nC,\nD,未知";
        assert_eq!(
            transform_csv(query).as_deref(),
            Ok("数据行,列号,列名,内容\r\n2,2,状态,完成 \r\n4,2,状态,未知"),
        );
        assert_eq!(
            transform_csv("检查规则 [[1,\"允许值\",[\"ok\",\"\"]]] 状态\nOK").as_deref(),
            Ok("数据行,列号,列名,规则号,规则,内容\r\n1,1,状态,1,允许值,OK"),
        );
        assert_eq!(
            transform_csv(r#"检查允许值 [[1],["完成","完成"]] 状态"#),
            Err(CsvError::AllowedValuesCheckArguments),
        );
    }

    #[test]
    fn appends_rule_pass_flags_and_failure_counts_to_every_original_record() {
        let rules = r#"[[1,"正则","^[AB]$"],["日期","日期"]]"#;
        let input =
            "编号,日期,备注\nA,2024-02-29,\"有效,原文\"\nB,2023-02-29,一项失败\nX,,两项失败";
        let flags = format!("规则标记列 [\"通过\",{rules}] {input}");
        let expected = concat!(
            "编号,日期,备注,通过\r\nA,2024-02-29,\"有效,原文\",1\r\n",
            "B,2023-02-29,一项失败,0\r\nX,\"\",两项失败,0",
        );
        assert_eq!(transform_csv(&flags).as_deref(), Ok(expected));
        let (text, _) = csv_table_preview(&flags)
            .expect("规则标记列应可预览")
            .unwrap();
        assert_eq!(text, expected);
        let counts = format!("rule-failure-count-column [\"失败数\",{rules}] {input}");
        assert_eq!(
            transform_csv(&counts).as_deref(),
            Ok(concat!(
                "编号,日期,备注,失败数\r\nA,2024-02-29,\"有效,原文\",0\r\n",
                "B,2023-02-29,一项失败,1\r\nX,\"\",两项失败,2",
            ))
        );
        let collision = format!("规则标记列 [\"编号\",{rules}] {input}");
        assert_eq!(transform_csv(&collision), Err(CsvError::HeaderNameConflict));
    }

    #[test]
    fn summarizes_every_rule_in_query_order_including_rules_with_no_records() {
        let rules = r#"[[1,"数值"],["比率","数值范围","0","1"]]"#;
        assert_eq!(
            transform_csv(&format!("规则统计 {rules} 比率\n1/3\n1.2\n未知")).as_deref(),
            Ok("规则号,列号,列名,规则,通过数,失败数\r\n1,1,比率,数值,2,1\r\n2,1,比率,数值范围,1,2"),
        );
        assert_eq!(
            transform_csv(&format!("规则统计 {rules} 比率")).as_deref(),
            Ok("规则号,列号,列名,规则,通过数,失败数\r\n1,1,比率,数值,0,0\r\n2,1,比率,数值范围,0,0"),
        );
    }

    #[test]
    fn keeps_each_csv_record_according_to_all_its_column_rules() {
        let input = "[[\"编号\",\"正则\",\"^[AB]$\"],[2,\"日期\"]] 编号,日期,备注\nA,2024-02-29,\"有效,原文\"\nB,2023-02-29,日期错误\nX,2024-01-01,编号错误";
        assert_eq!(
            transform_csv(&format!("规则保留行 {input}")).as_deref(),
            Ok("编号,日期,备注\r\nA,2024-02-29,\"有效,原文\""),
        );
        assert_eq!(
            transform_csv(&format!("keep-invalid-rows {input}")).as_deref(),
            Ok("编号,日期,备注\r\nB,2023-02-29,日期错误\r\nX,2024-01-01,编号错误"),
        );
        assert_eq!(
            transform_csv("规则问题行 [[1,\"日期\"]] 日期\n2024-02-29").as_deref(),
            Ok("日期"),
        );
    }

    #[test]
    fn checks_different_column_rules_and_reports_the_rule_that_failed() {
        let query = "检查规则 [[\"日期\",\"日期\"],[\"编号\",\"正则\",\"^A[0-9]+$\"],[3,\"数值范围\",\"0\",\"1\"]] 日期,编号,比率\n2024-02-29,A01,1\n2023-02-29,B02,1.01";
        assert_eq!(
            transform_csv(query).as_deref(),
            Ok("数据行,列号,列名,规则号,规则,内容\r\n2,1,日期,1,日期,2023-02-29\r\n2,2,编号,2,正则,B02\r\n2,3,比率,3,数值范围,1.01"),
        );
        assert_eq!(
            transform_csv(r#"check-rules [[1,"date","^A"]] 日期"#),
            Err(CsvError::ValidationRulesArguments),
        );
    }

    #[test]
    fn reports_values_outside_an_exact_inclusive_csv_range() {
        let query = "检查数值范围 [[\"比率\"],[\"1/4\",\"3/4\"]] 编号,比率\nA,1/4\nB,0.750\nC,0.751\nD,未填\nE,0.24";
        assert_eq!(
            transform_csv(query).as_deref(),
            Ok("数据行,列号,列名,内容\r\n3,2,比率,0.751\r\n4,2,比率,未填\r\n5,2,比率,0.24"),
        );
        assert_eq!(
            transform_csv(r#"check-numeric-range [[1],["1","0"]] 数值"#),
            Err(CsvError::NumericRangeCheckArguments),
        );
    }

    #[test]
    fn reports_csv_cells_that_do_not_match_the_chosen_regex() {
        let input = "检查正则列 [[\"编号\"],\"^[A-Z][0-9]{2}$\"] 编号,状态\nA01,通过\nB7,待复核\n,未知\nC23,完成";
        assert_eq!(
            transform_csv(input).as_deref(),
            Ok("数据行,列号,列名,内容\r\n2,1,编号,B7\r\n3,1,编号,\"\""),
        );
        assert_eq!(
            transform_csv("check-regex-columns [[1],\"^$\"] 编号,状态\n,无编号").as_deref(),
            Ok("数据行,列号,列名,内容"),
        );
        let large = format!(
            "检查正则列 [[1],\"^B\"] 编号\n{}",
            "x".repeat(MAX_REGEX_COLUMN_INPUT_BYTES + 1),
        );
        assert_eq!(
            transform_csv(&large),
            Err(CsvError::RegexCheckColumnsInputTooLarge),
        );
    }

    #[test]
    fn reports_invalid_complete_dates_with_original_values_and_coordinates() {
        let input = "检查日期列 [2,\"开始\"] 开始,结束\n2024-W09-4,2024年3月1日\n2023-02-29,\n今天,2024-02-29";
        assert_eq!(
            transform_csv(input).as_deref(),
            Ok("数据行,列号,列名,内容\r\n2,2,结束,\"\"\r\n2,1,开始,2023-02-29\r\n3,1,开始,今天"),
        );
        assert_eq!(
            transform_csv("check-date-columns [\"日期\"] 日期\n 2024-02-29 ").as_deref(),
            Ok("数据行,列号,列名,内容"),
        );
    }

    #[test]
    fn lists_invalid_exact_csv_values_by_data_row_and_requested_column_order() {
        assert_eq!(
            transform_csv(concat!(
                r#"检查数值列 [3,"金额"] 编号,金额,数量"#,
                "\n001,1.50,2\n002,\"待,看\", \n003,1/3,\"未\n录\"\n004,3e-2,",
            ))
            .as_deref(),
            Ok(concat!(
                "数据行,列号,列名,内容\r\n",
                "2,3,数量, \r\n",
                "2,2,金额,\"待,看\"\r\n",
                "3,3,数量,\"未\n录\"\r\n",
                "4,3,数量,\"\"",
            ))
        );
        assert_eq!(
            transform_csv("check-numeric-columns 2 编号,金额\n001,9007199254740993").as_deref(),
            Ok("数据行,列号,列名,内容")
        );
        assert_eq!(
            transform_csv(r#"检查数值列 ["金额",2] 编号,金额"#),
            Err(CsvError::NamedColumnArguments)
        );
    }

    #[test]
    fn applies_csv_commands_in_order_and_reports_the_first_failing_step() {
        assert_eq!(
            transform_csv(concat!(
                r#"依次处理 ["去空行","删行 2","选列 2,1"] 编号,商品"#,
                "\n001,茶\n\n002,梨\n003,\"咖啡,热\""
            ))
            .as_deref(),
            Ok("商品,编号\r\n茶,001\r\n\"咖啡,热\",003")
        );
        assert_eq!(
            transform_csv("PIPELINE [\"选列 1\",\"对象\"] 编号,金额\n001,1.50").as_deref(),
            Ok(r#"[{"编号":"001"}]"#)
        );
        assert_eq!(
            transform_csv(r#"依次处理 ["去空行","选列 3","去重"] 编号,商品"#),
            Err(CsvError::PipelineStep {
                index: 2,
                cause: CsvError::ColumnOutOfRange.message(),
            })
        );
        assert_eq!(
            transform_csv(r#"依次处理 ["pipeline []"] 编号"#),
            Err(CsvError::PipelineStep {
                index: 1,
                cause: CsvError::NestedPipeline.message(),
            })
        );
        for query in [r#"依次处理 [] 编号"#, r#"依次处理 [" \n"] 编号"#] {
            assert_eq!(transform_csv(query), Err(CsvError::PipelineArguments));
        }
    }

    #[test]
    fn joins_tables_by_text_in_left_and_right_record_order_with_bounded_output() {
        for (query, expected) in [
            (
                r#"左连接 ["编号","键"] ["编号,商品\n001,茶\n002,米","键,金额\n001,1.50\n001,2e0\n1,4"]"#,
                "编号,商品,金额\r\n001,茶,1.50\r\n001,茶,2e0\r\n002,米,\"\"",
            ),
            (
                r#"inner-join [1,2] ["编号,商品\n001,\"茶,热\"\n002,米","价格,键\n1.50,001"]"#,
                "编号,商品,价格\r\n001,\"茶,热\",1.50",
            ),
            (
                r#"内连接 [1,1] ["ID,状态\na,草稿","键,说明"]"#,
                "ID,状态,说明",
            ),
            (
                r#"左连接 [1,1] ["ID,状态\na,草稿","键"]"#,
                "ID,状态\r\na,草稿",
            ),
        ] {
            assert_eq!(transform_csv(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, expected) in [
            (
                r#"左连接 [1,1] ["ID,状态\na,草稿","键,状态\na,已完成"]"#,
                CsvError::InvalidHeader,
            ),
            (
                r#"内连接 [1,3] ["ID\na","键,值\na,1"]"#,
                CsvError::ColumnOutOfRange,
            ),
            (r#"左连接 [1,1] ["ID"]"#, CsvError::JoinArguments),
            (
                r#"左连接 [1,1] ["ID,状态\na","键"]"#,
                CsvError::DifferentColumnCount,
            ),
        ] {
            assert_eq!(transform_csv(query), Err(expected), "{query}");
        }
        let table = format!(
            "ID,值\n{}",
            "a,123456789012345678901234567890\n".repeat(180)
        );
        let right = table.replacen("ID,值", "键,参照", 1);
        let source = serde_json::to_string(&[table, right]).unwrap();
        assert_eq!(
            transform_csv(&format!("左连接 [1,1] {source}")),
            Err(CsvError::JoinTooLarge)
        );
    }

    #[test]
    fn joins_by_multiple_columns_and_restores_every_right_only_key_in_left_column_order() {
        let source = r#"[["区","月"],[2,"月份"]] ["区,月,品\n北,01,茶\n北,02,米","月份,区域,额\n01,北,1.50\n01,南,2e0\n03,北,3"]"#;
        for (operation, expected) in [
            (
                "全连接",
                "区,月,品,额\r\n北,01,茶,1.50\r\n北,02,米,\"\"\r\n南,01,\"\",2e0\r\n北,03,\"\",3",
            ),
            ("左连接", "区,月,品,额\r\n北,01,茶,1.50\r\n北,02,米,\"\""),
            ("内连接", "区,月,品,额\r\n北,01,茶,1.50"),
        ] {
            assert_eq!(
                transform_csv(&format!("{operation} {source}")).as_deref(),
                Ok(expected)
            );
        }
        for keys in [
            r#"[["区","月"],"区域"]"#,
            r#"[["区",1],["区域","月份"]]"#,
            r#"[[],[]]"#,
        ] {
            assert_eq!(
                transform_csv(&format!(r#"全连接 {keys} ["区,月,品","月份,区域,额"]"#)),
                Err(CsvError::JoinColumns)
            );
        }
    }

    #[test]
    fn compares_csv_numeric_column_values_exactly_and_distinguishes_zero_from_blank() {
        let tables = r#"[[1],2] ["编号,金额\n001,1.50\n002,\n003,9007199254740993\n004,1/2\n006,  ","编号,金额\n001,3/2\n002,0\n003,9007199254740992\n004,0.75\n006,\n005,2e0"]"#;
        assert_eq!(
            transform_csv(&format!("数值对比列 {tables}")).as_deref(),
            Ok("键1,变更,原值,新值\r\n002,修改,\"\",0\r\n003,修改,9007199254740993,9007199254740992\r\n004,修改,1/2,0.75\r\n005,新增,\"\",2e0")
        );
        for tables in [
            r#"[[1],2] ["编号,金额\n001,待确认","编号,金额"]"#,
            r#"[[1],2] ["编号,金额","编号,金额\n002,待确认"]"#,
        ] {
            assert_eq!(
                transform_csv(&format!("数值对比列 {tables}")),
                Err(CsvError::InvalidNumber)
            );
        }
    }

    #[test]
    fn compares_csv_header_positions_and_lists_new_columns_in_new_table_order() {
        let query = r#"表头对比 ["编号,状态,金额\n001,完成,1.50","金额,编号,备注\n1.50,001,茶"]"#;
        let expected = "列名,原序号,新序号,变更\r\n编号,1,2,位置变化\r\n状态,2,\"\",删除\r\n金额,3,1,位置变化\r\n备注,\"\",3,新增";
        assert_eq!(transform_csv(query).as_deref(), Ok(expected));
        assert_eq!(csv_table_preview(query).unwrap().unwrap().0, expected);
        assert_eq!(
            transform_csv(r#"compare-headers ["编号,状态","编号,状态"]"#).as_deref(),
            Ok("列名,原序号,新序号,变更")
        );
        for query in [
            r#"表头对比 ["编号,编号","编号"]"#,
            r#"表头对比 ["编号","编号,"]"#,
        ] {
            assert_eq!(transform_csv(query), Err(CsvError::InvalidHeader));
        }
    }

    #[test]
    fn compares_csv_rows_by_text_and_pairs_duplicate_occurrences_in_order() {
        let query = r#"对比行 ["编号,备注\n001,同值\n001,同值\n002,\"逗号,值\"\n003,\n005,相同","备注,编号\n同值,001\n\"逗号,值\",004\n,003\n相同,005\n\"逗号,值\",004"]"#;
        let expected = "变更,数据行,字段 编号,字段 备注\r\n删除,2,001,同值\r\n删除,3,002,\"逗号,值\"\r\n新增,2,004,\"逗号,值\"\r\n新增,5,004,\"逗号,值\"";
        assert_eq!(transform_csv(query).as_deref(), Ok(expected));
        assert_eq!(csv_table_preview(query).unwrap().unwrap().0, expected);
        assert_eq!(
            transform_csv(
                r#"compare-rows ["编号,备注\n001,茶\n002,梨","备注,编号\n梨,002\n茶,001"]"#
            )
            .as_deref(),
            Ok("变更,数据行,字段 编号,字段 备注"),
        );
        assert_eq!(
            transform_csv(r#"对比行 ["编号,备注","编号,状态"]"#),
            Err(CsvError::ComparisonHeaderMismatch),
        );
        assert_eq!(
            transform_csv(r#"对比行 ["编号,编号","编号,备注"]"#),
            Err(CsvError::InvalidHeader),
        );
    }

    #[test]
    fn compares_all_non_key_columns_by_header_even_when_headers_are_reordered() {
        let query = r#"对比全部列 [1,"地区"] ["编号,地区,状态,金额\n001,北,草稿,1.50\n002,南,完成,3","金额,编号,状态,地区\n1.5,001,完成,北\n4,003,待办,南"]"#;
        let expected = "键1,键2,列,变更,原值,新值\r\n001,北,状态,修改,草稿,完成\r\n001,北,金额,修改,1.50,1.5\r\n002,南,状态,删除,完成,\"\"\r\n002,南,金额,删除,3,\"\"\r\n003,南,状态,新增,\"\",待办\r\n003,南,金额,新增,\"\",4";
        assert_eq!(transform_csv(query).as_deref(), Ok(expected));
        let (csv, _) = csv_table_preview(query).expect("整表差异可预览").unwrap();
        assert_eq!(csv, expected);
        for (query, error) in [
            (
                r#"对比全部列 [1] ["编号,金额","编号,状态"]"#,
                CsvError::ComparisonHeaderMismatch,
            ),
            (
                r#"对比全部列 [1] ["编号,金额,金额","编号,金额,状态"]"#,
                CsvError::InvalidHeader,
            ),
            (
                r#"对比全部列 [1] ["编号","编号"]"#,
                CsvError::NoComparedColumns,
            ),
            (
                r#"对比全部列 [1] ["编号,金额\n001,1\n001,2","金额,编号"]"#,
                CsvError::DuplicateComparisonKey,
            ),
            (
                r#"对比全部列 [] ["编号,金额","金额,编号"]"#,
                CsvError::AllColumnComparisonArguments,
            ),
        ] {
            assert_eq!(transform_csv(query), Err(error), "{query}");
        }
    }

    #[test]
    fn compares_multiple_columns_by_key_in_requested_order_and_preserves_report_text() {
        let query = r#"对比列 [["编号"],["状态","金额"]] ["编号,状态,金额\n001,草稿,1.50\n002,完成,3","金额,编号,状态\n1.5,001,完成\n4,003,待办"]"#;
        let expected = concat!(
            "键1,列,变更,原值,新值\r\n",
            "001,状态,修改,草稿,完成\r\n",
            "001,金额,修改,1.50,1.5\r\n",
            "002,状态,删除,完成,\"\"\r\n",
            "002,金额,删除,3,\"\"\r\n",
            "003,状态,新增,\"\",待办\r\n",
            "003,金额,新增,\"\",4",
        );
        assert_eq!(transform_csv(query).as_deref(), Ok(expected));
        let (preview, _) = csv_table_preview(query).unwrap().unwrap();
        assert_eq!(preview, expected);
        assert_eq!(
            transform_csv(r#"数值对比列 [["编号"],["金额","数量"]] ["编号,金额,数量\n001,1.50,2\n002,0.50,1","数量,编号,金额\n3,001,3/2\n1,002,1/2"]"#).as_deref(),
            Ok("键1,列,变更,原值,新值\r\n001,数量,修改,2,3"),
        );
        assert_eq!(
            transform_csv(r#"对比列 [[1],["金额",2]] ["编号,金额","编号,金额"]"#),
            Err(CsvError::ColumnComparisonArguments)
        );
    }

    #[test]
    fn compares_named_columns_with_composite_keys_and_preserves_change_order() {
        let query = r#"对比列 [["编号","班次"],"金额"] ["编号,班次,金额\n001,A,1.50\n002,B,3\n004,A,0\n001,B,","金额,班次,编号\n9,C,003\n1.5,A,001\n0,A,004\n2,B,001\n8,D,005"]"#;
        assert_eq!(
            transform_csv(query).as_deref(),
            Ok("键1,键2,变更,原值,新值\r\n001,A,修改,1.50,1.5\r\n002,B,删除,3,\"\"\r\n001,B,修改,\"\",2\r\n003,C,新增,\"\",9\r\n005,D,新增,\"\",8")
        );
        assert_eq!(
            transform_csv(r#"compare-column [[1],2] ["编号,金额\n001,1","编号,金额\n001,1"]"#)
                .as_deref(),
            Ok("键1,变更,原值,新值"),
        );
        assert_eq!(
            transform_csv(
                r#"对比列 [[1],2] ["编号,说明\n\"a,b\",\"旧,值\"","编号,说明\n\"a,b\",\"新,值\""]"#,
            )
            .as_deref(),
            Ok("键1,变更,原值,新值\r\n\"a,b\",修改,\"旧,值\",\"新,值\""),
        );
        for (query, error) in [
            (
                r#"对比列 [[],2] ["编号,金额","编号,金额"]"#,
                CsvError::ColumnComparisonArguments,
            ),
            (
                r#"对比列 [[1],2] ["编号,金额\n001,1\n001,2","编号,金额"]"#,
                CsvError::DuplicateComparisonKey,
            ),
            (
                r#"对比列 [[1],2] ["编号,金额","编号,金额\n001,1\n001,2"]"#,
                CsvError::DuplicateComparisonKey,
            ),
            (
                r#"对比列 [[1],"金额"] ["编号,金额","编号,备注"]"#,
                CsvError::MissingColumnName,
            ),
        ] {
            assert_eq!(transform_csv(query), Err(error), "{query}");
        }
    }

    #[test]
    fn full_join_keeps_unmatched_rows_and_their_original_keys() {
        for (query, expected) in [
            (
                r#"全连接 ["编号","键"] ["商品,编号\n茶,001\n米,002","键,金额\n003,2e0\n001,1.50\n003,4"]"#,
                "商品,编号,金额\r\n茶,001,1.50\r\n米,002,\"\"\r\n\"\",003,2e0\r\n\"\",003,4",
            ),
            (
                r#"full-join [1,2] ["编号","备注,键\n\"茶,热\",001\n末尾,"]"#,
                "编号,备注\r\n001,\"茶,热\"\r\n\"\",末尾",
            ),
        ] {
            assert_eq!(transform_csv(query).as_deref(), Ok(expected), "{query}");
        }
    }

    #[test]
    fn selects_left_rows_by_key_presence_without_expanding_duplicate_matches() {
        let inputs = r#"[["区",2],[2,"期"]] ["区,月,额\n北,01,1.50\n北,01,1e0\n北,02,2\n北,1,3\n南,,4","期,区域,额\n01,北,5\n01,北,6\n,南,7"]"#;
        for (operation, expected) in [
            (
                "左表匹配",
                "区,月,额\r\n北,01,1.50\r\n北,01,1e0\r\n南,\"\",4",
            ),
            ("左表独有", "区,月,额\r\n北,02,2\r\n北,1,3"),
        ] {
            assert_eq!(
                transform_csv(&format!("{operation} {inputs}")).as_deref(),
                Ok(expected),
            );
        }
        let no_right_rows = r#"[1,1] ["编号,额\n001,1.50","键"]"#;
        assert_eq!(
            transform_csv(&format!("semi-join {no_right_rows}")).as_deref(),
            Ok("编号,额"),
        );
        assert_eq!(
            transform_csv(&format!("anti-join {no_right_rows}")).as_deref(),
            Ok("编号,额\r\n001,1.50"),
        );
        assert_eq!(
            transform_csv(r#"左表独有 [[1,2],1] ["编号,额","编号,值"]"#),
            Err(CsvError::JoinColumns),
        );
    }

    #[test]
    fn stacks_tables_by_exact_header_and_fills_columns_missing_from_a_source() {
        for (query, expected) in [
            (
                r#"纵向合并 ["编号,金额\n001,1.50","金额,编号,备注\n2e0,002,\"茶,热\"","状态,编号\n完成,003"]"#,
                "编号,金额,备注,状态\r\n001,1.50,\"\",\"\"\r\n002,2e0,\"茶,热\",\"\"\r\n003,\"\",\"\",完成",
            ),
            (
                r#"stack ["ID,字段","备注,id\n\"甲\n乙\",001"]"#,
                "ID,字段,备注,id\r\n\"\",\"\",\"甲\n乙\",001",
            ),
        ] {
            assert_eq!(transform_csv(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            transform_csv(r#"纵向合并 ["ID,ID","编号\n001"]"#),
            Err(CsvError::InvalidHeader)
        );
        assert_eq!(
            transform_csv(r#"纵向合并 ["ID\n001"]"#),
            Err(CsvError::StackArguments)
        );
    }

    #[test]
    fn maps_whole_cells_once_per_row_and_preserves_unmatched_fields() {
        let source = concat!(
            "映射值 [\"状态\",[[\"P\",\"D\"],[\"D\",\"P\"],[\"\",\"未填\"]]] 状态,编号,备注\n",
            "P,001,\"甲,乙\"\nD,002,原样\n,003,原样\n P,004,空格",
        );
        assert_eq!(
            transform_csv(source).as_deref(),
            Ok("状态,编号,备注\r\nD,001,\"甲,乙\"\r\nP,002,原样\r\n未填,003,原样\r\n P,004,空格")
        );
        assert_eq!(
            transform_csv("map-values [1,[[\"P\",\"D\"],[\"P\",\"Q\"]]] 状态,编号\nP,001"),
            Err(CsvError::MapValueArguments)
        );
        assert_eq!(
            transform_csv(r#"map-values [1,[["P","\u0000"]]] 状态,编号"#),
            Err(CsvError::NullReplacementValue)
        );
    }

    #[test]
    fn normalizes_unicode_into_a_new_column_while_retaining_source_text() {
        assert_eq!(
            transform_csv(
                "Unicode规范化列 [\"标准名\",\"名称\",\"NFC\"] 编号,名称\n001, Cafe\u{301} \n002,",
            )
            .as_deref(),
            Ok("编号,名称,标准名\r\n001, Cafe\u{301} , Café \r\n002,\"\",\"\""),
        );
        assert_eq!(
            transform_csv(r#"unicode-normalize-column ["标准名",1,"nfkc"] 名称"#).as_deref(),
            Ok("名称,标准名"),
        );
        assert_eq!(
            transform_csv(r#"Unicode规范化列 ["结果",1,"UTF16"] 名称"#),
            Err(CsvError::UnicodeColumnArguments),
        );
        assert_eq!(
            transform_csv(r#"Unicode规范化列 ["名称",1,"NFC"] 名称"#),
            Err(CsvError::HeaderNameConflict),
        );
    }

    #[test]
    fn adds_grapheme_edit_distances_to_csv_without_rewriting_the_source_columns() {
        assert_eq!(
            transform_csv(concat!(
                "编辑距离列 [\"距离\",\"原文\",3] 编号,原文,现文\n",
                "001,kitten,sitting\n002,猫👩‍💻,猫🧑‍💻\n003, ,\n004,,"
            ))
            .as_deref(),
            Ok(concat!(
                "编号,原文,现文,距离\r\n",
                "001,kitten,sitting,3\r\n002,猫👩‍💻,猫🧑‍💻,1\r\n",
                "003, ,\"\",1\r\n004,\"\",\"\",0"
            )),
        );
        assert_eq!(
            transform_csv(r#"edit-distance-column ["距离",1,2] 前,后"#).as_deref(),
            Ok("前,后,距离"),
        );
        assert_eq!(
            transform_csv(r#"编辑距离列 ["距离",1,3] 前,后"#),
            Err(CsvError::ColumnOutOfRange),
        );
        let too_long = format!(
            "编辑距离列 [\"距离\",1,2] 前,后\n茶,茶\n{},茶",
            "茶".repeat(513)
        );
        assert_eq!(
            transform_csv(&too_long),
            Err(CsvError::DataRowValue {
                row: 2,
                cause: TextError::TooManyEditGraphemes.message()
            }),
        );
        let repeated = format!("\n{},{}", "a".repeat(512), "b".repeat(512));
        let too_much = format!("编辑距离列 [\"距离\",1,2] 前,后{}", repeated.repeat(4));
        assert_eq!(
            transform_csv(&too_much),
            Err(CsvError::DataRowValue {
                row: 4,
                cause: TextError::EditDistanceWorkLimit.message()
            }),
        );
    }

    #[test]
    fn appends_longest_common_fragments_to_csv_and_bounds_the_work() {
        assert_eq!(
            transform_csv(concat!(
                "公共片段列 [\"片段\",\"原名\",3] 编号,原名,现名\n",
                "001,北京报告2024.txt,上海报告2024.csv\n",
                "002,ab-cd,cd+ab\n",
                "003,猫👩‍💻,狗👩‍💻\n",
                "004,,茶\n",
                "005,\"a,b!\",\"a,b?\""
            ))
            .as_deref(),
            Ok(concat!(
                "编号,原名,现名,片段\r\n",
                "001,北京报告2024.txt,上海报告2024.csv,报告2024.\r\n",
                "002,ab-cd,cd+ab,ab\r\n",
                "003,猫👩‍💻,狗👩‍💻,👩‍💻\r\n",
                "004,\"\",茶,\"\"\r\n",
                "005,\"a,b!\",\"a,b?\",\"a,b\""
            ))
        );
        assert_eq!(
            transform_csv(r#"公共片段列 ["前",1,2] 前,后"#),
            Err(CsvError::HeaderNameConflict)
        );
        let repeated = format!("\n{},{}", "a".repeat(512), "b".repeat(512));
        let input = format!("公共片段列 [\"片段\",1,2] 前,后{}", repeated.repeat(4));
        assert_eq!(
            transform_csv(&input),
            Err(CsvError::DataRowValue {
                row: 4,
                cause: TextError::CommonSubstringWorkLimit.message()
            })
        );
    }

    #[test]
    fn trims_only_the_selected_data_cells_and_keeps_internal_whitespace() {
        assert_eq!(
            transform_csv(concat!(
                "去空白 [\"名称\",2] 名称,备注,编号\n",
                "\"\u{3000}茶, 热  \",\"\t 热\u{a0}\",\" 001 \"\n",
                " 梨 ,\" \n甲\n乙\n \", 002 ",
            ))
            .as_deref(),
            Ok("名称,备注,编号\r\n\"茶, 热\",热, 001 \r\n梨,\"甲\n乙\", 002 ")
        );
        assert_eq!(
            transform_csv("trim-columns 1 编号,备注\n\" \", 甲 ").as_deref(),
            Ok("编号,备注\r\n\"\", 甲 ")
        );
        assert_eq!(
            transform_csv("去空白 [\"名称\",1] 名称,编号\n茶,001"),
            Err(CsvError::NamedColumnArguments)
        );
    }

    #[test]
    fn fills_selected_empty_cells_from_the_previous_nonempty_value_with_a_size_limit() {
        let input = concat!(
            "向下填充 [\"地区\",2] 地区,月份,编号\n",
            ",01,001\n\"华,北\",,002\n,,003\n ,02,004\n,,005",
        );
        assert_eq!(
            transform_csv(input).as_deref(),
            Ok("地区,月份,编号\r\n\"\",01,001\r\n\"华,北\",01,002\r\n\"华,北\",01,003\r\n ,02,004\r\n ,02,005")
        );
        assert_eq!(
            transform_csv("fill-down 1 地区,编号").as_deref(),
            Ok("地区,编号")
        );
        assert_eq!(
            transform_csv("向下填充 [1,\"地区\"] 地区,编号\n华北,001"),
            Err(CsvError::NamedColumnArguments)
        );
        let large = "x".repeat(4096);
        let many_blanks = "\n,002".repeat(260);
        assert_eq!(
            transform_csv(&format!("fill-down 1 地区,编号\n{large},001{many_blanks}")),
            Err(CsvError::FillTooLarge)
        );
    }

    #[test]
    fn fills_bounded_gaps_between_exact_csv_values_in_each_group() {
        assert_eq!(
            transform_csv(
                "插值填充 [\"量\",[\"组\"]] 组,量\nA,1/3\nB,\nA,\nB,1.0\nA,\nB,\nA,4/3\nB,3\nA,"
            )
            .as_deref(),
            Ok("组,量\r\nA,1/3\r\nB,\"\"\r\nA,2/3\r\nB,1.0\r\nA,1\r\nB,2\r\nA,4/3\r\nB,3\r\nA,\"\"")
        );
        assert_eq!(
            transform_csv("fill-interpolate [1,[]] 值\n2.50\n\"\"\n2.5").as_deref(),
            Ok("值\r\n2.50\r\n5/2\r\n2.5")
        );
        assert!(matches!(
            transform_csv("插值填充 [2,[1]] 组,值\nA,1\nB,待定\nA,3"),
            Err(CsvError::DataRowValue { row: 2, .. })
        ));
    }

    #[test]
    fn fills_selected_empty_cells_from_the_following_nonempty_value() {
        let input = concat!(
            "向上填充 [\"地区\",2] 地区,月份,编号\n",
            ",,001\n\"华,北\",,002\n,02,003\n ,03,004\n,,005",
        );
        assert_eq!(
            transform_csv(input).as_deref(),
            Ok("地区,月份,编号\r\n\"华,北\",02,001\r\n\"华,北\",02,002\r\n ,02,003\r\n ,03,004\r\n\"\",\"\",005")
        );
        assert_eq!(
            transform_csv("fill-up 1 地区,编号").as_deref(),
            Ok("地区,编号")
        );
        let large = "x".repeat(4096);
        let many_blanks = ",001\n".repeat(260);
        assert_eq!(
            transform_csv(&format!("fill-up 1 地区,编号\n{many_blanks}{large},002")),
            Err(CsvError::FillTooLarge)
        );
    }

    #[test]
    fn counts_exactly_empty_csv_cells_by_selected_column_including_header_only_input() {
        let input = concat!(
            "空值统计 [\"备注\",1] 编号,备注,地区\n",
            "001,,华北\n,\" \",华北\n002,\"\",华南\n003,\"上\n下\",华南",
        );
        assert_eq!(
            transform_csv(input).as_deref(),
            Ok("列号,列名,空单元格,非空单元格\r\n2,备注,2,2\r\n1,编号,1,3")
        );
        assert_eq!(
            transform_csv("count-empty 1 编号,备注").as_deref(),
            Ok("列号,列名,空单元格,非空单元格\r\n1,编号,0,0")
        );
        assert_eq!(
            transform_csv("空值统计 [2,\"备注\"] 编号,备注\n001,茶"),
            Err(CsvError::NamedColumnArguments)
        );
    }

    #[test]
    fn lists_csv_columns_in_order_including_blank_and_duplicate_names() {
        let query = "列清单 \u{feff}A,a,A,\" A \",,\"B,C\",\"甲\n乙\"\n1,2,3,4,5,6,7";
        let expected = concat!(
            "列号,列名,同名列数\r\n1,A,2\r\n2,a,1\r\n3,A,2\r\n4, A ,1\r\n",
            "5,\"\",1\r\n6,\"B,C\",1\r\n7,\"甲\n乙\",1",
        );
        assert_eq!(transform_csv(query).as_deref(), Ok(expected));
        assert_eq!(
            csv_table_preview(query)
                .unwrap()
                .map(|(text, _)| text)
                .as_deref(),
            Ok(expected)
        );
        assert_eq!(
            transform_csv("column-directory ,n,").as_deref(),
            Ok("列号,列名,同名列数\r\n1,\"\",2\r\n2,n,1\r\n3,\"\",2")
        );
        assert_eq!(
            transform_csv("列清单 A,B\n1"),
            Err(CsvError::DifferentColumnCount)
        );
        let large = format!("列清单 {}", "x".repeat(MAX_EXPANDED_CSV_BYTES));
        assert_eq!(
            transform_csv(&large),
            Err(CsvError::ColumnDirectoryTooLarge)
        );
    }

    #[test]
    fn profiles_csv_column_value_counts_without_coercing_or_trimming_values() {
        let query = "列值概览 [\"备注\",1] 编号,备注\r\n001,\"茶,咖啡\"\r\n1,\"茶,咖啡\"\r\n001,\" \"\r\n,\"\"";
        let expected = "列号,列名,空单元格,非空单元格,不同非空值,重复值种数\r\n2,备注,1,3,2,1\r\n1,编号,1,3,2,1";
        assert_eq!(transform_csv(query).as_deref(), Ok(expected));
        assert_eq!(
            csv_table_preview(query)
                .expect("列概览应显示为表格")
                .map(|(text, _)| text)
                .as_deref(),
            Ok(expected)
        );
        assert_eq!(
            transform_csv("value-profile 1 编号").as_deref(),
            Ok("列号,列名,空单元格,非空单元格,不同非空值,重复值种数\r\n1,编号,0,0,0,0")
        );
        assert_eq!(
            transform_csv("列值概览 [2,\"备注\"] 编号,备注\n001,茶"),
            Err(CsvError::NamedColumnArguments)
        );
        assert_eq!(
            transform_csv("列值概览 2 编号,备注\n001"),
            Err(CsvError::DifferentColumnCount)
        );
    }

    #[test]
    fn pads_short_csv_records_to_the_largest_width_and_keeps_cells_exact() {
        assert_eq!(
            transform_csv("补齐列 编号,备注\n001,\"上\n下\",1.50\n002").as_deref(),
            Ok("编号,备注,\"\"\r\n001,\"上\n下\",1.50\r\n002,\"\",\"\"")
        );
        assert_eq!(
            transform_csv("pad-columns A,B\n001,").as_deref(),
            Ok("A,B\r\n001,\"\"")
        );
        assert_eq!(
            transform_csv("补齐列 A,B\n001,\"未闭合"),
            Err(CsvError::UnclosedQuote)
        );
        let header = format!("{}X", ",".repeat(256));
        let short_rows = "Y\n".repeat(65);
        assert_eq!(
            transform_csv(&format!("补齐列 {header}\n{short_rows}")),
            Err(CsvError::TooManyAddedCells)
        );
    }

    #[test]
    fn reports_csv_width_mismatches_by_record_including_multiline_fields() {
        assert_eq!(
            transform_csv("校验 编号,说明\r\n001,\"茶\n咖啡\"\r\n002,梨,额外\r\n003").as_deref(),
            Ok("4 条记录 · 首条 2 列 · 2 条列数不一致\n第 3 条记录有 3 列\n第 4 条记录有 1 列")
        );
        assert_eq!(
            transform_csv("validate \u{feff}编号,说明\r\n001,").as_deref(),
            Ok("2 条记录 · 每条 2 列")
        );
        assert_eq!(transform_csv("校验 "), Err(CsvError::MissingInput));
    }

    #[test]
    fn reports_the_record_and_field_for_csv_lexical_errors() {
        assert_eq!(
            transform_csv("校验 编号,说明\r\n001,\"茶\n咖啡\"\r\n002,\"未闭合").as_deref(),
            Ok("CSV 语法错误 · 第 3 条记录 · 第 2 列\n字段的双引号尚未闭合")
        );
        assert_eq!(
            transform_csv("校验 a,b\n\"x\"extra,y").as_deref(),
            Ok("CSV 语法错误 · 第 2 条记录 · 第 1 列\n检查引号和分隔符，引号内的双引号用两个双引号表示")
        );
        assert_eq!(
            transform_csv("转JSON a,b\n1,\"缺引号"),
            Err(CsvError::UnclosedQuote)
        );
    }

    #[test]
    fn reports_spans_for_csv_syntax_errors_without_changing_copyable_reports() {
        for (query, invalid) in [
            (
                "校验\r\n\u{feff}编号,说明\r\n01,\"甲\n乙\"\r\n02,\" 未结束 ",
                "\" 未结束 ",
            ),
            ("VALIDATE 编号,备注\r\n01,\"甲\"丁", "丁"),
            ("校验 编号,备注\n01,带\"引号", "\""),
            ("校验 a,b\n1,2\r3,4", "\r"),
        ] {
            let report = csv_validation_report(query).unwrap().unwrap();
            assert_eq!(&query[report.error_span.unwrap()], invalid, "{query}");
            assert_eq!(transform_csv(query).as_deref(), Ok(report.text.as_str()));
        }
        let query = "校验 \u{feff}a,b\r\n01,\"甲\n乙\",多余\r\n02";
        let mismatch = csv_validation_report(query).unwrap().unwrap();
        assert_eq!(&query[mismatch.error_span.unwrap()], "01,\"甲\n乙\",多余");
        assert!(mismatch.text.contains("列数不一致"));
        assert!(csv_validation_report("校验 a,b\n1,2")
            .unwrap()
            .unwrap()
            .error_span
            .is_none());
    }

    #[test]
    fn locates_tsv_syntax_and_column_count_errors_without_converting_the_input() {
        let width_mismatch =
            "校验TSV\r\n\u{feff}编号\t备注\r\n001\t\"茶\n咖啡\"\r\n002\t梨\t额外\r\n003";
        let report = csv_validation_report(width_mismatch).unwrap().unwrap();
        assert_eq!(&width_mismatch[report.error_span.unwrap()], "002\t梨\t额外");
        assert!(report.text.contains("第 3 条记录有 3 列"));
        assert_eq!(transform_csv(width_mismatch), Ok(report.text));

        let bad_quote = "VALIDATE-TSV a\tb\n1\t\"茶\"坏";
        let report = csv_validation_report(bad_quote).unwrap().unwrap();
        assert_eq!(&bad_quote[report.error_span.unwrap()], "坏");
        assert!(report.text.contains("TSV 语法错误 · 第 2 条记录 · 第 2 列"));
        assert_eq!(transform_csv(bad_quote), Ok(report.text));

        let valid = csv_validation_report("校验TSV 编号\t备注\n001\t茶,咖啡")
            .unwrap()
            .unwrap();
        assert!(valid.error_span.is_none());
        assert_eq!(valid.text, "2 条记录 · 每条 2 列");
    }

    #[test]
    fn generates_markdown_with_leading_trailing_or_both_alignment_colons() {
        let input = "Markdown对齐 [\"左\",\"center\",\"右\",\"default\"] 编号,商品,数量,备注\n001,茶|咖啡,2,\"上\n下\"";
        assert_eq!(
            transform_csv(input).as_deref(),
            Ok("| 编号 | 商品 | 数量 | 备注 |\n| :--- | :---: | ---: | --- |\n| 001 | 茶\\|咖啡 | 2 | 上<br>下 |")
        );
        for query in [
            r#"Markdown对齐 ["左"] 编号,商品"#,
            r#"markdown-align ["unexpected"] 编号"#,
        ] {
            assert_eq!(transform_csv(query), Err(CsvError::MarkdownAlignments));
        }
    }

    #[test]
    fn reads_markdown_table_cells_with_escapes_breaks_and_missing_columns() {
        assert_eq!(
            transform_csv("从Markdown | 编号 | 商品 | 备注 |\n| :--- | :---: | ---: |\n| 001 | 茶\\|咖啡 | 上<br>下 |\n| 002 | **米** |").as_deref(),
            Ok("编号,商品,备注\r\n001,茶|咖啡,\"上\n下\"\r\n002,**米**,\"\"")
        );
        assert_eq!(
            transform_csv("from-markdown A | B\r\n--- | ---\rx").as_deref(),
            Ok("A,B\r\nx,\"\"")
        );
        let csv = "编号,备注,原样\r\n001,\"茶|咖啡\n下一行\",<br> \\x *词*";
        let markdown = transform_csv(&format!("Markdown {csv}")).unwrap();
        assert_eq!(
            transform_csv(&format!("从Markdown\n{markdown}")).as_deref(),
            Ok(csv)
        );
        for (query, error) in [
            ("从Markdown 标题\n---", CsvError::MarkdownTable),
            ("从Markdown 标题\\|小节\n---", CsvError::MarkdownTable),
            ("从Markdown A | B\n---", CsvError::MarkdownTable),
            ("从Markdown | A |\n| : |\n| x |", CsvError::MarkdownTable),
            (
                "从Markdown | A |\n| --- |\n| x | y |",
                CsvError::DifferentColumnCount,
            ),
            (
                "从Markdown | A |\n| --- |\n\n| x |",
                CsvError::MarkdownTable,
            ),
        ] {
            assert_eq!(transform_csv(query), Err(error), "{query}");
        }
    }

    #[test]
    fn drops_only_columns_with_no_nonempty_data_cells() {
        for (query, expected) in [
            (
                "去空列 编号,备注,状态\n001,,0\n002,\"\",茶",
                "编号,状态\r\n001,0\r\n002,茶",
            ),
            ("drop-empty-columns 空格,空\n ,\n,", "空格\r\n \r\n\"\""),
            ("去空列 编号,备注", "编号,备注"),
        ] {
            assert_eq!(transform_csv(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            transform_csv("去空列 编号,备注\n,"),
            Err(CsvError::NoColumnsRemain)
        );
        assert_eq!(
            transform_csv("去空列 编号,备注\n001"),
            Err(CsvError::DifferentColumnCount)
        );
    }

    #[test]
    fn drops_csv_columns_by_name_or_index_in_original_column_order() {
        let source = "编号,状态,备注,金额\n001,完成,原样,1.50\n002,待办,\"甲,乙\",2e0";
        for operation in [r#"去列 ["备注",2]"#, "drop-columns 3,2"] {
            assert_eq!(
                transform_csv(&format!("{operation}\n{source}")).as_deref(),
                Ok("编号,金额\r\n001,1.50\r\n002,2e0")
            );
        }
        for (query, error) in [
            (r#"去列 [1,"状态"] 编号,状态"#, CsvError::NoColumnsRemain),
            (
                r#"去列 [1,"编号"] 编号,状态"#,
                CsvError::NamedColumnArguments,
            ),
            (r#"去列 ["备注"] 编号,状态"#, CsvError::MissingColumnName),
            ("去列 3 编号,状态", CsvError::ColumnOutOfRange),
        ] {
            assert_eq!(transform_csv(query), Err(error), "{query}");
        }
    }

    #[test]
    fn appends_one_constant_csv_column_with_escaping_and_a_size_limit() {
        assert_eq!(
            transform_csv(concat!(
                r#"常量列 ["复核,来源","人工,复核\n完成"] 编号,商品"#,
                "\n001,茶\n002,梨"
            ))
            .as_deref(),
            Ok("编号,商品,\"复核,来源\"\r\n001,茶,\"人工,复核\n完成\"\r\n002,梨,\"人工,复核\n完成\"")
        );
        assert_eq!(
            transform_csv(r#"constant-column ["说明",""] 编号"#).as_deref(),
            Ok("编号,说明")
        );
        assert_eq!(
            transform_csv("常量列 [\"说明\",\"\"] 编号\n001").as_deref(),
            Ok("编号,说明\r\n001,\"\"")
        );
        assert_eq!(
            transform_csv(r#"常量列 ["编号","人工"] 编号"#),
            Err(CsvError::HeaderNameConflict)
        );
        assert_eq!(
            transform_csv(r#"常量列 ["来源","\u0000"] 编号"#),
            Err(CsvError::NullReplacementValue)
        );
        let constant = "x".repeat(4096);
        let records = "\n001".repeat(260);
        assert_eq!(
            transform_csv(&format!("常量列 [\"来源\",\"{constant}\"] 编号{records}")),
            Err(CsvError::AddedColumnTooLarge)
        );
    }

    #[test]
    fn appends_contiguous_csv_row_numbers_and_counts_multiline_fields_once() {
        assert_eq!(
            transform_csv("加行号 [\"序号\",100] 编号,商品\n001,\"茶\n梨\"\n002,苹果").as_deref(),
            Ok("编号,商品,序号\r\n001,\"茶\n梨\",100\r\n002,苹果,101")
        );
        assert_eq!(
            transform_csv("number-rows [\"序号\",18446744073709551615] 编号\n001").as_deref(),
            Ok("编号,序号\r\n001,18446744073709551615")
        );
        assert_eq!(
            transform_csv("number-rows [\"序号\",18446744073709551615] 编号\n001\n002"),
            Err(CsvError::RowNumberOverflow)
        );
        assert_eq!(
            transform_csv(r#"加行号 ["编号",1] 编号"#),
            Err(CsvError::HeaderNameConflict)
        );
        assert_eq!(
            transform_csv(r#"加行号 ["序号",-1] 编号"#),
            Err(CsvError::RowNumberArguments)
        );
    }

    #[test]
    fn concatenates_chosen_csv_columns_in_order_and_keeps_empty_values() {
        assert_eq!(
            transform_csv(concat!(
                r#"拼接列 ["标签",["城市",2],", "] 编号,地区,城市"#,
                "\n001,福建,福州\n002,浙江,"
            ))
            .as_deref(),
            Ok("编号,地区,城市,标签\r\n001,福建,福州,\"福州, 福建\"\r\n002,浙江,\"\",\", 浙江\"")
        );
        assert_eq!(
            transform_csv(r#"concat-columns ["地址",["省","市"],""] 省,市"#).as_deref(),
            Ok("省,市,地址")
        );
        assert_eq!(
            transform_csv(r#"拼接列 ["地址",["省",1],"-"] 省,市"#),
            Err(CsvError::ConcatenatedColumnArguments)
        );
        assert_eq!(
            transform_csv(r#"拼接列 ["省",[1,2],"-"] 省,市"#),
            Err(CsvError::HeaderNameConflict)
        );
    }

    #[test]
    fn fills_csv_templates_with_original_cells_and_escapes_only_template_braces() {
        let query = concat!(
            r#"模板列 ["标签","{#1} | {{{城市}}} | {编号}, {编号}"]"#,
            "\n编号,城市\n001,福州\n002,{编号}\n003,"
        );
        let expected = concat!(
            "编号,城市,标签\r\n",
            "001,福州,\"001 | {福州} | 001, 001\"\r\n",
            "002,{编号},\"002 | {{编号}} | 002, 002\"\r\n",
            "003,\"\",\"003 | {} | 003, 003\""
        );
        assert_eq!(transform_csv(query).as_deref(), Ok(expected));
        assert_eq!(csv_table_preview(query).unwrap().unwrap().0, expected);
        for (template, header, error) in [
            ("{城市", "城市", CsvError::TemplateColumnArguments),
            ("{地名}", "城市", CsvError::MissingColumnName),
            ("{城市}", "城市,城市", CsvError::AmbiguousColumnName),
            ("{#2}", "城市", CsvError::ColumnOutOfRange),
        ] {
            let query = format!(r#"template-column ["标签","{template}"] {header}"#);
            assert_eq!(transform_csv(&query), Err(error));
        }
        let template = "{#1}".repeat(1025);
        let large = format!(
            "模板列 [\"标签\",\"{template}\"] 城市\n{}",
            "x".repeat(1024)
        );
        assert_eq!(transform_csv(&large), Err(CsvError::AddedColumnTooLarge));
    }

    #[test]
    fn expands_each_non_overlapping_capture_and_preserves_unmatched_csv_rows() {
        let query = r#"正则展开行 ["编号","原文","[A-Z](?:=([0-9]+))?","1"]
原文,标签
A=007 B=99,甲
B,乙
无,丙"#;
        let expected =
            "原文,标签,编号\r\nA=007 B=99,甲,007\r\nA=007 B=99,甲,99\r\nB,乙,\"\"\r\n无,丙,\"\"";
        assert_eq!(transform_csv(query).as_deref(), Ok(expected));
        let (text, table) = csv_table_preview(query).unwrap().unwrap();
        assert_eq!(text, expected);
        assert_eq!(table.page_csv(0, 0), expected);
        assert_eq!(
            transform_csv("regex-expand-rows [\"匹配\",1,\"猫|🐱\",\"0\"]\n原文\n🐱猫").as_deref(),
            Ok("原文,匹配\r\n🐱猫,🐱\r\n🐱猫,猫"),
        );
        let oversized = format!(
            "正则展开行 [\"新列\",1,\"x\",\"0\"]\n来源\n{}",
            "x".repeat(MAX_REGEX_COLUMN_INPUT_BYTES + 1)
        );
        assert_eq!(
            transform_csv(&oversized),
            Err(CsvError::RegexCaptureColumnsInputTooLarge),
        );
    }

    #[test]
    fn appends_first_regex_capture_by_named_or_numbered_group() {
        let query = "正则提取列 [\"省份\",\"地址\",\"^(?P<province>[^省]+)省\",\"province\"]\n地址,编号\n江苏省南京,1\n上海市,2\n浙江省杭州,3";
        assert_eq!(
            transform_csv(query).as_deref(),
            Ok("地址,编号,省份\r\n江苏省南京,1,江苏\r\n上海市,2,\"\"\r\n浙江省杭州,3,浙江")
        );
        assert_eq!(
            transform_csv(
                "regex-capture-column [\"号\",1,\"([A-Z])([0-9]+)?\",\"2\"]\n原文\nA12 A99\nB"
            )
            .as_deref(),
            Ok("原文,号\r\nA12 A99,12\r\nB,\"\"")
        );
        assert_eq!(
            transform_csv("正则提取列 [\"结果\",1,\"(A)\",\"2\"]\n原文"),
            Err(CsvError::Regex(TextError::UnknownRegexGroup))
        );
    }

    #[test]
    fn appends_multiple_groups_from_each_first_csv_cell_match() {
        let query = concat!(
            r#"正则提取多列 ["原文","(?<key>[A-Z])(?:=([0-9]+))?",[["编号","2"],["标记","key"],["匹配","0"]]]"#,
            "\n原文,备注\nA=007 B=99,第一条\nB,第二条\n略过,空结果",
        );
        assert_eq!(
            transform_csv(query).as_deref(),
            Ok(concat!(
                "原文,备注,编号,标记,匹配\r\n",
                "A=007 B=99,第一条,007,A,A=007\r\n",
                "B,第二条,\"\",B,B\r\n",
                "略过,空结果,\"\",\"\",\"\"",
            ))
        );
        let (csv, _) = csv_table_preview(query)
            .expect("正则提取多列应可预览")
            .unwrap();
        assert_eq!(csv, transform_csv(query).unwrap());
        assert_eq!(
            transform_csv(r#"regex-capture-columns [1,"(A)",[["结果","2"]]] 来源"#),
            Err(CsvError::Regex(TextError::UnknownRegexGroup))
        );
        assert_eq!(
            transform_csv(r#"正则提取多列 [1,"(A)",[["重复","1"],["重复","0"]]] 来源"#),
            Err(CsvError::RegexCaptureColumnsArguments)
        );
        assert_eq!(
            transform_csv(r#"正则提取多列 [1,"(A)",[["来源","1"]]] 来源"#),
            Err(CsvError::HeaderNameConflict)
        );
        assert_eq!(
            transform_csv(&format!(
                "正则提取多列 [1,\"x\",[[\"结果\",\"0\"]]] 来源\n{}",
                "x".repeat(MAX_REGEX_COLUMN_INPUT_BYTES + 1),
            )),
            Err(CsvError::RegexCaptureColumnsInputTooLarge)
        );
    }

    #[test]
    fn appends_the_first_nonempty_selected_csv_cell_in_the_requested_order() {
        assert_eq!(
            transform_csv(concat!(
                r#"首个非空 ["联系",["手机",3]] 编号,手机,座机"#,
                "\n001,,021-1000\n002,  ,021-2000\n003,,\n004,\"123,456\",000"
            ))
            .as_deref(),
            Ok(concat!(
                "编号,手机,座机,联系\r\n",
                "001,\"\",021-1000,021-1000\r\n002,  ,021-2000,  \r\n",
                "003,\"\",\"\",\"\"\r\n004,\"123,456\",000,\"123,456\""
            ))
        );
        assert_eq!(
            transform_csv(r#"coalesce-columns ["联系",["手机",2]] 编号,手机,座机"#),
            Err(CsvError::FirstNonemptyColumnArguments)
        );
        assert_eq!(
            transform_csv(r#"coalesce-columns ["手机",[2,3]] 编号,手机,座机"#),
            Err(CsvError::HeaderNameConflict)
        );
    }

    #[test]
    fn splits_csv_values_into_rows_preserving_fields_order_and_empty_parts() {
        let query = concat!(
            "拆成行 [\"标签\",\"::\"] 编号,标签,备注\n",
            "001,茶::咖啡,\"保留,逗号\"\n002,::山::,尾\n003,,空",
        );
        let expected = concat!(
            "编号,标签,备注\r\n001,茶,\"保留,逗号\"\r\n001,咖啡,\"保留,逗号\"\r\n",
            "002,\"\",尾\r\n002,山,尾\r\n002,\"\",尾\r\n003,\"\",空",
        );
        assert_eq!(transform_csv(query).as_deref(), Ok(expected));
        assert_eq!(csv_table_preview(query).unwrap().unwrap().0, expected);
        assert_eq!(
            transform_csv("split-to-rows [1,\"\\n\"] 标签,编号\n\"甲\n乙\",09").as_deref(),
            Ok("标签,编号\r\n甲,09\r\n乙,09"),
        );
        assert_eq!(
            transform_csv("拆成行 [1,\"/\"] 标签").as_deref(),
            Ok("标签")
        );
        for (query, error) in [
            ("拆成行 [1,\"\"] 标签", CsvError::SplitRowsArguments),
            ("拆成行 [1,\"\\u0000\"] 标签", CsvError::SplitRowsArguments),
            ("拆成行 [2,\"/\"] 标签", CsvError::ColumnOutOfRange),
        ] {
            assert_eq!(transform_csv(query), Err(error));
        }
        let too_large = format!(
            "拆成行 [1,\"/\"] 标签,说明\n{}x,{}",
            "x/".repeat(1024),
            "A".repeat(1024)
        );
        assert_eq!(transform_csv(&too_large), Err(CsvError::SplitRowsTooLarge));
    }

    #[test]
    fn splits_one_csv_column_into_named_fields_and_keeps_the_remainder() {
        assert_eq!(
            transform_csv(concat!(
                r#"拆分列 ["地址",["省","市,区"],"/"] 编号,地址"#,
                "\n001,福建/福州/鼓楼\n002,浙江\n003,/杭州\n004,"
            ))
            .as_deref(),
            Ok(concat!(
                "编号,地址,省,\"市,区\"\r\n",
                "001,福建/福州/鼓楼,福建,福州/鼓楼\r\n",
                "002,浙江,浙江,\"\"\r\n003,/杭州,\"\",杭州\r\n004,\"\",\"\",\"\""
            ))
        );
        assert_eq!(
            transform_csv("split-column [1,[\"月份\",\"日期\"],\"::\"] 日期值\n02::03").as_deref(),
            Ok("日期值,月份,日期\r\n02::03,02,03")
        );
        for (query, error) in [
            (
                r#"拆分列 [1,["省","市"],""] 地址"#,
                CsvError::SplitColumnArguments,
            ),
            (
                r#"拆分列 [1,["省","省"],"/"] 地址"#,
                CsvError::SplitColumnArguments,
            ),
            (
                r#"拆分列 [1,["省","地址"],"/"] 地址"#,
                CsvError::HeaderNameConflict,
            ),
            (
                r#"拆分列 [2,["省","市"],"/"] 地址"#,
                CsvError::ColumnOutOfRange,
            ),
        ] {
            assert_eq!(transform_csv(query), Err(error), "{query}");
        }
    }

    #[test]
    fn renames_multiple_csv_headers_simultaneously_with_unchanged_records() {
        assert_eq!(
            transform_csv(concat!(
                r#"批量改列名 [["代码","简称"],["简称","代码"],["说明","备注,简介"]] 代码,简称,说明"#,
                "\n001,中文,\"带,逗号\""
            ))
            .as_deref(),
            Ok("简称,代码,\"备注,简介\"\r\n001,中文,\"带,逗号\"")
        );
        for (query, error) in [
            (
                r#"rename-headers [["A","B"],["B","C"]] A,B,C"#,
                CsvError::HeaderNameConflict,
            ),
            (
                r#"rename-headers [["A","X"],["A","Y"]] A,B"#,
                CsvError::BulkRenameArguments,
            ),
            (
                r#"rename-headers [["A",""]] A,B"#,
                CsvError::BulkRenameArguments,
            ),
            (
                r#"rename-headers [["B","X"]] A,A"#,
                CsvError::MissingColumnName,
            ),
            (
                r#"rename-headers [["A","X"]] A,A"#,
                CsvError::AmbiguousColumnName,
            ),
        ] {
            assert_eq!(transform_csv(query), Err(error), "{query}");
        }
    }

    #[test]
    fn replaces_only_matching_cells_in_a_selected_csv_column() {
        let source = "编号,状态,备注\n001,待办,待办\n002,\"待办 \",记录\n003,完成,待办";
        assert_eq!(
            transform_csv(&format!(
                "替换值 [\"状态\",\"待办\",\"进行,稍后\"]\n{source}"
            ))
            .as_deref(),
            Ok("编号,状态,备注\r\n001,\"进行,稍后\",待办\r\n002,待办 ,记录\r\n003,完成,待办")
        );
        assert_eq!(
            transform_csv("replace-value [2,\"\",\"补填\"] 编号,备注\n001,\n002,原值").as_deref(),
            Ok("编号,备注\r\n001,补填\r\n002,原值")
        );
        for (query, error) in [
            (
                r#"替换值 ["状态","待办",1] 编号,状态"#,
                CsvError::ReplaceValueArguments,
            ),
            (
                r#"替换值 ["状态","待办","\u0000"] 编号,状态"#,
                CsvError::NullReplacementValue,
            ),
            (
                r#"替换值 ["状态","待办","完成"] 状态,状态"#,
                CsvError::AmbiguousColumnName,
            ),
        ] {
            assert_eq!(transform_csv(query), Err(error), "{query}");
        }
    }

    #[test]
    fn resolves_named_columns_for_stable_sorting_and_deduplication() {
        let data = "编号,文件名,金额\n001,文件10,1.50\n002,文件2,3/2\n003,文件2,7/5\n004,文件1,2e0";
        for (operation, expected) in [
            (
                r#"排序 ["文件名",1]"#,
                "编号,文件名,金额\r\n004,文件1,2e0\r\n002,文件2,3/2\r\n003,文件2,7/5\r\n001,文件10,1.50",
            ),
            (
                r#"数值降序 ["金额"]"#,
                "编号,文件名,金额\r\n004,文件1,2e0\r\n001,文件10,1.50\r\n002,文件2,3/2\r\n003,文件2,7/5",
            ),
            (
                r#"按列去重 ["文件名"]"#,
                "编号,文件名,金额\r\n001,文件10,1.50\r\n002,文件2,3/2\r\n004,文件1,2e0",
            ),
        ] {
            assert_eq!(transform_csv(&format!("{operation}\n{data}")).as_deref(), Ok(expected));
        }
        for (query, expected) in [
            (
                r#"排序 ["编号",1] 编号,金额"#,
                CsvError::NamedColumnArguments,
            ),
            (
                r#"数值排序 ["金额"] 编号,金额,金额"#,
                CsvError::AmbiguousColumnName,
            ),
            (
                r#"按列去重 ["名称"] 编号,金额"#,
                CsvError::MissingColumnName,
            ),
        ] {
            assert_eq!(transform_csv(query), Err(expected), "{query}");
        }
    }

    #[test]
    fn picks_a_data_cell_without_treating_embedded_line_breaks_as_records() {
        let data = "编号,备注\r\n001,茶\r\n002,\" 梨,\r\n\"\"苹果\"\" \"\r\n003,";
        assert_eq!(
            transform_csv(&format!("取单元格 [2,\"备注\"] {data}")).as_deref(),
            Ok(" 梨,\r\n\"苹果\" ")
        );
        assert_eq!(
            transform_csv(&format!("PICK-CELL [1,1] {data}")).as_deref(),
            Ok("001")
        );
        assert_eq!(
            transform_csv(&format!("取单元格 [3,2] {data}")).as_deref(),
            Ok("")
        );
        assert_eq!(
            transform_csv(&format!("取单元格 [0,1] {data}")),
            Err(CsvError::CellArguments)
        );
        assert_eq!(
            transform_csv(&format!("取单元格 [4,1] {data}")),
            Err(CsvError::MissingDataRow)
        );
        assert_eq!(
            transform_csv(&format!("取单元格 [-1,1] {data}")).as_deref(),
            Ok("003")
        );
        assert_eq!(
            transform_csv(&format!("取单元格 [-2,\"备注\"] {data}")).as_deref(),
            Ok(" 梨,\r\n\"苹果\" ")
        );
        assert_eq!(
            transform_csv(&format!("取单元格 [-3,1] {data}")).as_deref(),
            Ok("001")
        );
        for query in [
            format!("取单元格 [-4,1] {data}"),
            "取单元格 [-1,1] 编号,备注".to_owned(),
            format!("取单元格 [18446744073709551615,1] {data}"),
            format!("取单元格 [-170141183460469231731687303715884105728,1] {data}"),
        ] {
            assert_eq!(
                transform_csv(&query),
                Err(CsvError::MissingDataRow),
                "{query}"
            );
        }
    }

    #[test]
    fn takes_each_groups_relative_row_without_counting_the_header() {
        let data = "组,编号,备注\nA,001,甲\nB,002,乙\nA,003,\"跨\n行\"\nA,004,丁";
        let query = format!("分组取行 [[\"组\"],-1] {data}");
        let expected = "组,编号,备注\r\nA,004,丁\r\nB,002,乙";
        assert_eq!(transform_csv(&query).as_deref(), Ok(expected));
        let (csv, _) = csv_table_preview(&query)
            .expect("分组取行应可预览")
            .unwrap();
        assert_eq!(csv, expected);
        assert_eq!(
            transform_csv(&format!("PICK-GROUP-ROW [[1],2] {data}")).as_deref(),
            Ok("组,编号,备注\r\nA,003,\"跨\n行\"")
        );
        assert_eq!(
            transform_csv(&format!("分组取行 [[],-5] {data}")).as_deref(),
            Ok("组,编号,备注")
        );
        assert_eq!(
            transform_csv(&format!("分组取行 [[1],0] {data}")),
            Err(CsvError::GroupRowArguments)
        );
    }

    #[test]
    fn selects_csv_records_in_input_order_preserving_multiline_fields() {
        let data = "编号,备注\r\n001,茶\r\n002,\"梨\r\n苹果\"\r\n003,咖啡\r\n004,柿子";
        assert_eq!(
            transform_csv(&format!("取行 4,2-3,2\r\n{data}")).as_deref(),
            Ok("编号,备注\r\n002,\"梨\r\n苹果\"\r\n003,咖啡\r\n004,柿子")
        );
        assert_eq!(
            transform_csv("删行 1,3-4,4 编号,备注\n001,茶\n002,\"冷\n饮\"\n003,梨\n004,茶")
                .as_deref(),
            Ok("编号,备注\r\n002,\"冷\n饮\"")
        );
        assert_eq!(
            transform_csv("drop-rows 1-2 编号,备注\n001,茶\n002,梨").as_deref(),
            Ok("编号,备注")
        );
        for (query, error) in [
            ("pick-rows 0 编号,备注\n001,茶", CsvError::InvalidRowRange),
            ("pick-rows 3-2 编号,备注\n001,茶", CsvError::InvalidRowRange),
            ("pick-rows 2 编号,备注\n001,茶", CsvError::MissingDataRow),
            ("pick-rows 1 编号,备注", CsvError::MissingDataRow),
            ("drop-rows 2 编号,备注\n001,茶", CsvError::MissingDataRow),
        ] {
            assert_eq!(transform_csv(query), Err(error), "{query}");
        }
    }

    #[test]
    fn drops_empty_csv_records_with_or_without_delimiters_and_keeps_nonempty_fields() {
        assert_eq!(
            transform_csv("去空行 编号,备注\n\n,\n\"\",\"\"\n001, \n002,\"冷\n饮\"\n,").as_deref(),
            Ok("编号,备注\r\n001, \r\n002,\"冷\n饮\"")
        );
        assert_eq!(
            transform_csv("drop-empty-rows \u{feff}编号,备注\r\n,,\r\n,").as_deref(),
            Ok("编号,备注")
        );
        assert_eq!(
            transform_csv("去空行 编号,备注\n001"),
            Err(CsvError::DifferentColumnCount)
        );
        assert_eq!(
            transform_csv("去空行 编号,备注\n\""),
            Err(CsvError::UnclosedQuote)
        );
    }

    #[test]
    fn pivots_csv_using_first_seen_labels_exact_sums_and_blank_missing_cells() {
        let data = "地区,状态,金额\n北,完成,1/2\n南,\"待办,稍后\",0\n北,完成,2/3\n南,完成,-1";
        for query in [
            format!("透视合计 [\"地区\",\"状态\",\"金额\"]\n{data}"),
            format!("pivot-sum [1,\"状态\",3]\r\n{data}"),
        ] {
            assert_eq!(
                transform_csv(&query).as_deref(),
                Ok("地区,完成,\"待办,稍后\"\r\n北,7/6,\r\n南,-1,0"),
                "{query}"
            );
        }
        assert_eq!(
            transform_csv("透视合计 [1,2,3] 地区,状态,金额").as_deref(),
            Ok("地区")
        );
        for (query, expected) in [
            (
                r#"透视合计 [1,"地区",3] 地区,状态,金额"#,
                CsvError::PivotColumns,
            ),
            (
                "透视合计 [1,2,4] 地区,状态,金额",
                CsvError::ColumnOutOfRange,
            ),
            (
                "透视合计 [1,2,3] 地区,状态,金额\n北,完成,待定",
                CsvError::InvalidNumber,
            ),
        ] {
            assert_eq!(transform_csv(query), Err(expected), "{query}");
        }
    }

    #[test]
    fn counts_csv_pivot_pairs_and_outputs_zero_for_unobserved_pairs() {
        let input = "地区,状态,备注\n南,\"待办,稍后\",x\n北,完成,y\n南,完成,z\n南,完成,z";
        assert_eq!(
            transform_csv(&format!("透视计数 [1,\"状态\"] {input}")).as_deref(),
            Ok("地区,\"待办,稍后\",完成\r\n南,1,2\r\n北,0,1")
        );
        assert_eq!(
            transform_csv("pivot-count [1,2] 地区,状态\n南,\n北,完成").as_deref(),
            Ok("地区,\"\",完成\r\n南,1,0\r\n北,0,1")
        );
        assert_eq!(
            transform_csv("透视计数 [1,2] 地区,状态").as_deref(),
            Ok("地区")
        );
        for (query, error) in [
            (
                "透视计数 [1,2,3] 地区,状态,金额",
                CsvError::PivotCountColumns,
            ),
            (
                "透视计数 [1,\"地区\"] 地区,状态",
                CsvError::PivotCountColumns,
            ),
            ("透视计数 [1,3] 地区,状态", CsvError::ColumnOutOfRange),
        ] {
            assert_eq!(transform_csv(query), Err(error), "{query}");
        }
    }

    #[test]
    fn appends_previous_or_following_csv_values_within_each_group() {
        let data = "部门,编号\n研发,001\n销售,101\n研发,002\n销售,\n研发,003";
        assert_eq!(
            transform_csv(&format!(r#"前值列 ["上期",2,1,["部门"]] {data}"#)).as_deref(),
            Ok("部门,编号,上期\r\n研发,001,\"\"\r\n销售,101,\"\"\r\n研发,002,001\r\n销售,\"\",101\r\n研发,003,002")
        );
        assert_eq!(
            transform_csv(&format!(r#"lead-column ["下期","编号",1,[1]] {data}"#)).as_deref(),
            Ok("部门,编号,下期\r\n研发,001,002\r\n销售,101,\"\"\r\n研发,002,003\r\n销售,\"\",\"\"\r\n研发,003,\"\"")
        );
        assert_eq!(
            transform_csv("前值列 [\"之前\",1,2,[]] 编号\n001\n\"\"\n003\n004").as_deref(),
            Ok("编号,之前\r\n001,\"\"\r\n\"\",\"\"\r\n003,001\r\n004,\"\"")
        );
        assert_eq!(
            transform_csv("后值列 [\"之后\",1,0,[]] 编号"),
            Err(CsvError::ShiftedColumnArguments)
        );
        let distant = format!("后值列 [\"之后\",1,{},[]] 编号\n001\n002", usize::MAX);
        assert_eq!(
            transform_csv(&distant).as_deref(),
            Ok("编号,之后\r\n001,\"\"\r\n002,\"\"")
        );
    }

    #[test]
    fn pivots_original_csv_values_and_rejects_duplicate_cells() {
        let data = "地区,月份,原值\n北,\"二月,补记\",001\n南,一月,\"待\n核\"\n北,一月,\"说\"\"好\"\"\"\n东,一月,";
        assert_eq!(
            transform_csv(&format!(r#"透视原值 ["地区",2,"原值"] {data}"#)).as_deref(),
            Ok("地区,\"二月,补记\",一月\r\n北,001,\"说\"\"好\"\"\"\r\n南,,\"待\n核\"\r\n东,,\"\"")
        );
        assert_eq!(
            transform_csv("pivot-values [1,2,3] 地区,月份,原值").as_deref(),
            Ok("地区")
        );
        assert_eq!(
            transform_csv("透视原值 [1,2,3] 地区,月份,原值\n北,二月,\n北,一月,001\n北,二月,002"),
            Err(CsvError::DataRowValue {
                row: 3,
                cause: "行列组合重复，请为每个组合保留一条记录",
            })
        );
        assert_eq!(
            transform_csv(r#"透视原值 [1,"地区",3] 地区,月份,原值"#),
            Err(CsvError::PivotValueColumns)
        );
        assert_eq!(
            transform_csv("透视原值 [1,2] 地区,月份"),
            Err(CsvError::PivotValueColumns)
        );
    }

    #[test]
    fn unpivots_csv_by_record_and_keeps_quoted_empty_and_numeric_cells() {
        assert_eq!(
            transform_csv(
                "逆透视 [[\"地区\",2],\"月份\",\"金额\"]\n地区,编号,一月,\"二月,补录\"\n北,001,2.50,\n南,002,1/3,3"
            )
            .as_deref(),
            Ok("地区,编号,月份,金额\r\n北,001,一月,2.50\r\n北,001,\"二月,补录\",\"\"\r\n南,002,一月,1/3\r\n南,002,\"二月,补录\",3")
        );
        assert_eq!(
            transform_csv("unpivot [[],\"字段\",\"值\"] 一,二\n甲,乙").as_deref(),
            Ok("字段,值\r\n一,甲\r\n二,乙")
        );
        for (query, error) in [
            (
                r#"逆透视 [[1,"地区"],"月份","金额"] 地区,一月"#,
                CsvError::UnpivotArguments,
            ),
            (
                r#"逆透视 [[1,2],"月份","金额"] 地区,一月"#,
                CsvError::UnpivotColumns,
            ),
            (
                r#"逆透视 [[1],"地区","金额"] 地区,一月"#,
                CsvError::HeaderNameConflict,
            ),
        ] {
            assert_eq!(transform_csv(query), Err(error), "{query}");
        }

        let header = format!("地区,{}月", "月,".repeat(650));
        let row = format!("{},{}1", "x".repeat(1800), "1,".repeat(650));
        assert_eq!(
            transform_csv(&format!("unpivot [[1],\"月份\",\"金额\"] {header}\n{row}")),
            Err(CsvError::PivotTooLarge)
        );
    }

    #[test]
    fn rejects_quadratic_pivot_output_before_expanding_missing_cells() {
        for kind in ["pivot-sum", "pivot-values"] {
            let mut query = format!("{kind} [1,2,3] 地区,状态,金额");
            for index in 0..1025 {
                query.push_str(&format!("\n行{index},列{index},1"));
            }
            assert_eq!(transform_csv(&query), Err(CsvError::PivotTooLarge));
        }
    }

    #[test]
    fn turns_csv_rows_into_markdown_table_rows_without_interpreting_cell_content() {
        assert_eq!(markdown_cell(r"前\|后"), r"前\\\|后");
        assert_eq!(
            transform_csv("Markdown 编号,备注\r\n001,\"甲|乙\r\n*斜体* <a> &\"").as_deref(),
            Ok("| 编号 | 备注 |\n| --- | --- |\n| 001 | 甲\\|乙<br>\\*斜体\\* \\<a\\> \\& |")
        );
    }

    #[test]
    fn transposes_csv_and_keeps_empty_and_quoted_cells() {
        let csv = "a,b\r\n\"含,逗号\",\"他说\"\"好\"\"\"\r\n,\r\n";
        let transposed = transform_csv(&format!("转置 {csv}")).unwrap();
        assert_eq!(
            transposed,
            "a,\"含,逗号\",\"\"\r\nb,\"他说\"\"好\"\"\",\"\""
        );
        let restored = transform_csv(&format!("转置 {transposed}")).unwrap();
        assert_eq!(read_rows(&restored, ','), read_rows(csv, ','));
        assert_eq!(transform_csv("transpose \"\"").as_deref(), Ok("\"\""));
        assert_eq!(
            transform_csv("transpose \"\u{feff}编号\"").as_deref(),
            Ok("\"\u{feff}编号\"")
        );
    }

    #[test]
    fn deduplicates_csv_rows_by_fields_in_first_seen_order() {
        let csv = "编号,备注\n001,\"茶,咖啡\"\n\"001\",\"茶,咖啡\"\n002,未选\n001,\"茶,咖啡\"\n003,\n003,\"\"";
        assert_eq!(
            transform_csv(&format!("去重 {csv}")).as_deref(),
            Ok("编号,备注\r\n001,\"茶,咖啡\"\r\n002,未选\r\n003,\"\""),
        );
        assert_eq!(
            transform_csv("deduplicate x\nx \nx").as_deref(),
            Ok("x\r\nx ")
        );
    }

    #[test]
    fn finds_csv_duplicates_with_exact_numeric_keys_and_keeps_original_values() {
        assert_eq!(
            transform_csv(concat!(
                "数值查重复 [\"金额\",2] 编号,组,金额\n",
                "001,1,1.50\n002,1e0,3/2\n003,2,1.50\n004,1,1.5e0\n005,1,5/4"
            ))
            .as_deref(),
            Ok("编号,组,金额\r\n001,1,1.50\r\n002,1e0,3/2\r\n004,1,1.5e0")
        );
        assert_eq!(
            transform_csv("duplicates-numeric 2 编号,金额\n001,未知"),
            Err(CsvError::InvalidNumber)
        );
    }

    #[test]
    fn finds_every_csv_row_with_a_duplicate_column_key_in_original_order() {
        let source = concat!(
            "查重复 [\"编号\",2] 编号,地区,记录\n",
            "001,北,第一条\n001,南,单独\n002,北,第二组\n001,北,再次\n",
            "1,北,前导零不同\n002,北,再次\n001,北,第 3 次"
        );
        assert_eq!(
            transform_csv(source).as_deref(),
            Ok(concat!(
                "编号,地区,记录\r\n001,北,第一条\r\n002,北,第二组\r\n",
                "001,北,再次\r\n002,北,再次\r\n001,北,第 3 次"
            ))
        );
        assert_eq!(
            transform_csv("duplicates-by-columns 1 编号,记录\n001,茶\n002,梨").as_deref(),
            Ok("编号,记录")
        );
    }

    #[test]
    fn deduplicates_selected_csv_columns_and_retains_first_record_and_header() {
        let csv = "编号,区域,备注\r\n001,东,\"茶,咖啡\"\r\n001,东,第二条\r\n001,西,第三条\r\n编号,区域,数据\r\n编号,区域,重复";
        assert_eq!(
            transform_csv(&format!("按列去重 1,2\r\n{csv}")).as_deref(),
            Ok("编号,区域,备注\r\n001,东,\"茶,咖啡\"\r\n001,西,第三条\r\n编号,区域,数据")
        );
        assert_eq!(
            transform_csv("deduplicate-columns 1 组,值\n,首行\n\"\",重复\n ,空格").as_deref(),
            Ok("组,值\r\n\"\",首行\r\n ,空格")
        );
        assert_eq!(
            transform_csv("按列去重 2 编号,名称").as_deref(),
            Ok("编号,名称")
        );
        assert_eq!(
            transform_csv("按列去重 2 编号"),
            Err(CsvError::ColumnOutOfRange)
        );
        assert_eq!(
            transform_csv("按列去重 1,1 编号"),
            Err(CsvError::InvalidColumns)
        );
    }

    #[test]
    fn deduplicates_csv_by_last_record_in_original_order() {
        let text =
            "编号,地区,备注\n001,北,首条\n002,南,独有\n001,北,中间\n1,北,前导零不同\n001,北,末条";
        let preview = csv_table_preview(&format!("按列去重留末 [\"编号\",2] {text}"))
            .expect("按列去重留末应可预览")
            .unwrap();
        assert_eq!(
            preview.0,
            "编号,地区,备注\r\n002,南,独有\r\n1,北,前导零不同\r\n001,北,末条"
        );
        assert_eq!(
            transform_csv("deduplicate-columns-last 1 编号,备注").as_deref(),
            Ok("编号,备注")
        );

        let numeric = "编号,金额,税率\n001,1.50,1/4\n002,2,1/4\n003,3/2,0.25\n004,1.5e0,1/3";
        let preview = csv_table_preview(&format!("数值去重留末 [\"金额\",3] {numeric}"))
            .expect("数值去重留末应可预览")
            .unwrap();
        assert_eq!(
            preview.0,
            "编号,金额,税率\r\n002,2,1/4\r\n003,3/2,0.25\r\n004,1.5e0,1/3"
        );
        assert_eq!(
            transform_csv(&format!("deduplicate-numeric-last 2 {numeric}")).as_deref(),
            Ok("编号,金额,税率\r\n002,2,1/4\r\n004,1.5e0,1/3")
        );
        assert_eq!(
            transform_csv("数值去重留末 2 编号,金额\n001,\n002,1"),
            Err(CsvError::InvalidNumber)
        );
    }

    #[test]
    fn mixes_utc_timestamp_order_with_text_and_numeric_keys() {
        let data = concat!(
            "编号,组,时刻,金额\n",
            "A,组2,1ns,3\n",
            "B,组10,-1ns,9\n",
            "C,组2,1970-01-01T01:00:00+01:00,2\n",
            "D,组2,0,2\n",
            "E,组2,0,4",
        );
        let query = format!(
            "多列排序 [[\"组\",\"文本升序\"],[\"时刻\",\"时间戳升序\"],[\"金额\",\"数值降序\"]] {data}"
        );
        let preview = csv_table_preview(&query)
            .expect("多列排序应可预览")
            .unwrap();
        assert_eq!(
            preview.0,
            concat!(
                "编号,组,时刻,金额\r\nE,组2,0,4\r\n",
                "C,组2,1970-01-01T01:00:00+01:00,2\r\nD,组2,0,2\r\n",
                "A,组2,1ns,3\r\nB,组10,-1ns,9",
            )
        );
        assert_eq!(
            transform_csv("sort-by-fields [[2,\"timestamp-desc\"],[1,\"text-asc\"]] 编号,时刻\nA,-1ns\nB,0\nC,1ns")
                .as_deref(),
            Ok("编号,时刻\r\nC,1ns\r\nB,0\r\nA,-1ns")
        );
        assert!(matches!(
            transform_csv(
                "多列排序 [[2,\"时间戳降序\"],[3,\"数值降序\"]] 编号,时刻,金额\nA,无效,1\nB,0,9"
            ),
            Err(CsvError::DataRowValue { row: 1, .. })
        ));
    }

    #[test]
    fn stably_sorts_csv_using_mixed_column_directions_and_types() {
        let data = "编号,地区,金额\n001,区域10,1\n002,区域2,1.50\n003,区域2,2e0\n004,区域2,1.500";
        assert_eq!(
            transform_csv(&format!("多列排序 [[\"地区\",\"文本升序\"],[3,\"数值降序\"]] {data}"))
                .as_deref(),
            Ok("编号,地区,金额\r\n003,区域2,2e0\r\n002,区域2,1.50\r\n004,区域2,1.500\r\n001,区域10,1")
        );
        assert_eq!(
            transform_csv(r#"多列排序 [["地区","文本升序"],[2,"数值降序"]] 编号,地区"#),
            Err(CsvError::SortFields)
        );
        assert_eq!(
            transform_csv("sort-by-fields [[2,\"numeric-asc\"]] 地区,金额\n东,"),
            Err(CsvError::InvalidNumber)
        );
    }

    #[test]
    fn deduplicates_numeric_csv_keys_exactly_and_preserves_the_first_rows_spelling() {
        let table = "编号,金额,税率\n001,1.50,1/4\n002,3/2,0.25\n003,1.5e0,1/3\n004,5/4,1/4";
        assert_eq!(
            transform_csv(&format!("数值去重 [\"金额\",3] {table}")).as_deref(),
            Ok("编号,金额,税率\r\n001,1.50,1/4\r\n003,1.5e0,1/3\r\n004,5/4,1/4")
        );
        assert_eq!(
            transform_csv(&format!("deduplicate-numeric 2 {table}")).as_deref(),
            Ok("编号,金额,税率\r\n001,1.50,1/4\r\n004,5/4,1/4")
        );
        assert_eq!(
            transform_csv("数值去重 2 编号,金额\n001,"),
            Err(CsvError::InvalidNumber)
        );
        assert_eq!(
            transform_csv("数值去重 [\"金额\",2] 编号,金额\n001,1"),
            Err(CsvError::NamedColumnArguments)
        );
    }

    #[test]
    fn filters_csv_by_exact_quoted_column_value_and_retains_the_header() {
        let csv =
            "编号,状态,备注\r\n001,完成,\"茶,咖啡\"\r\n002,完成 ,待清理\r\n003,完成,\r\n004,,未填";
        for (arguments, expected) in [
            (
                r#"筛选 2 "完成""#,
                "编号,状态,备注\r\n001,完成,\"茶,咖啡\"\r\n003,完成,\"\"",
            ),
            (
                r#"exclude 2 "完成""#,
                "编号,状态,备注\r\n002,完成 ,待清理\r\n004,\"\",未填",
            ),
            (r#"filter 2 """#, "编号,状态,备注\r\n004,\"\",未填"),
            (r#"筛选 2 "遗漏""#, "编号,状态,备注"),
        ] {
            assert_eq!(
                transform_csv(&format!("{arguments}\r\n\u{feff}{csv}")).as_deref(),
                Ok(expected)
            );
        }
        assert_eq!(
            transform_csv("筛选 1 \"值\"  有空格,状态\r\n值,完成").as_deref(),
            Ok(" 有空格,状态\r\n值,完成")
        );
        assert_eq!(
            transform_csv(r#"筛选 1 "值"a,b"#),
            Err(CsvError::InvalidColumnValue)
        );
        assert_eq!(
            transform_csv(r#"筛选 3 "值" a,b"#),
            Err(CsvError::ColumnOutOfRange)
        );
    }

    #[test]
    fn restricts_csv_keyword_matches_to_the_named_columns_without_reordering_hits() {
        let data = "区域,备注,货品\n北区,咖啡,茶\n南区,茶,米";
        assert_eq!(
            transform_csv(&format!(r#"搜索行 {{"词":["茶"],"列":["备注"]}} {data}"#)).as_deref(),
            Ok("区域,备注,货品\r\n南区,茶,米")
        );
        let query =
            format!(r#"keyword-positions {{"keywords":["茶","咖啡"],"columns":[3,2]}} {data}"#);
        let hits = read_rows(&transform_csv(&query).unwrap(), ',').unwrap();
        assert_eq!(hits[1], ["1", "2", "备注", "[\"咖啡\"]", "咖啡"]);
        assert_eq!(hits[2], ["1", "3", "货品", "[\"茶\"]", "茶"]);
        assert_eq!(hits[3], ["2", "2", "备注", "[\"茶\"]", "茶"]);
        assert_eq!(
            transform_csv(&format!(r#"搜索行 {{"词":["茶"],"列":[2,"备注"]}} {data}"#)),
            Err(CsvError::SearchRowColumns)
        );
    }

    #[test]
    fn highlights_only_matching_cells_inside_the_csv_search_scope() {
        let data = "A,B,C,D,E,F,查找列,H\n茶,2,3,4,5,6,米,8\n茶,2,3,4,5,6,茶,8";
        let query = format!(r#"搜索行 {{"词":["茶"],"列":["查找列"]}} {data}"#);
        let (text, preview) = csv_table_preview(&query).unwrap().unwrap();
        assert_eq!(transform_csv(&query).as_deref(), Ok(text.as_str()));
        assert_eq!(text, "A,B,C,D,E,F,查找列,H\r\n茶,2,3,4,5,6,茶,8");
        assert_eq!(preview.first_match_column_page(), Some(1));
        assert!(preview.is_cell_highlighted(1, 6));
        assert!(!preview.is_cell_highlighted(1, 0));
        assert!(!preview.is_cell_highlighted(0, 6));

        let (_, preview) = csv_table_preview(&format!("搜索行 \"水\" {data}"))
            .unwrap()
            .unwrap();
        assert_eq!(preview.first_match_column_page(), None);
        assert_eq!(preview.row_pages(), 0);
    }

    #[test]
    fn searches_for_any_or_excludes_all_keywords_in_selected_columns() {
        let data = "编号,地区,商品,备注\n001,ＮＯＲＴＨ,咖啡,茶\n002,南,绿茶,米\n003,南,咖啡,茶\n004,North,茶,冰";
        let words = r#"{"词":["north","茶"],"列":["地区","商品"]}"#;
        let header = "编号,地区,商品,备注";
        for (operation, rows) in [
            ("搜索行", "\r\n004,North,茶,冰"),
            (
                "搜索任一",
                "\r\n001,ＮＯＲＴＨ,咖啡,茶\r\n002,南,绿茶,米\r\n004,North,茶,冰",
            ),
            ("EXCLUDE-KEYWORDS", "\r\n003,南,咖啡,茶"),
        ] {
            let query = format!("{operation} {words} {data}");
            let (text, _) = csv_table_preview(&query).unwrap().unwrap();
            assert_eq!(text, format!("{header}{rows}"));
            assert_eq!(transform_csv(&query).as_deref(), Ok(text.as_str()));
        }
        let (_, searched) = csv_table_preview(&format!("search-any-rows {words} {data}"))
            .unwrap()
            .unwrap();
        assert!(searched.is_cell_highlighted(1, 1));
        assert!(searched.is_cell_highlighted(2, 2));
        assert!(!searched.is_cell_highlighted(1, 3));
        let (_, excluded) = csv_table_preview(&format!("排除关键词 {words} {data}"))
            .unwrap()
            .unwrap();
        assert_eq!(excluded.first_match_column_page(), None);
        assert_eq!(
            transform_csv("排除关键词 \"\" 编号,备注\n001,茶").as_deref(),
            Ok("编号,备注\r\n001,茶")
        );
    }

    #[test]
    fn reports_matching_columns_and_original_keywords_for_each_csv_cell() {
        let query = "命中位置 [\"north\",\"茶\"] 地区,备注\nＮｏｒｔｈ,\"茶 North\r\n冰\"\n南区,米";
        let (csv, preview) = csv_table_preview(query).unwrap().unwrap();
        assert_eq!(transform_csv(query).as_deref(), Ok(csv.as_str()));
        let rows = read_rows(&csv, ',').unwrap();
        assert_eq!(rows[0], ["数据行号", "列号", "列名", "关键词", "内容"]);
        assert_eq!(rows[1], ["1", "1", "地区", "[\"north\"]", "Ｎｏｒｔｈ"]);
        assert_eq!(
            rows[2],
            ["1", "2", "备注", "[\"north\",\"茶\"]", "茶 North\r\n冰"]
        );
        assert_eq!(rows.len(), 3);
        assert_eq!(preview.row_pages(), 1);
        assert_eq!(
            transform_csv("keyword-positions \"茶\" 地区,备注\n南区,米").as_deref(),
            Ok("数据行号,列号,列名,关键词,内容")
        );
    }

    #[test]
    fn previews_selected_dropped_and_transposed_columns_with_exact_cells() {
        let data = "编号,名称,备注\n001,茶,\"甲,乙\"\n002,米,\"带\n换行\"";
        for (operation, expected) in [
            ("选列 3,1", "备注,编号\r\n\"甲,乙\",001\r\n\"带\n换行\",002"),
            (
                "选列名 [\"备注\",\"编号\"]",
                "备注,编号\r\n\"甲,乙\",001\r\n\"带\n换行\",002",
            ),
            (
                "去列 [\"名称\"]",
                "编号,备注\r\n001,\"甲,乙\"\r\n002,\"带\n换行\"",
            ),
            (
                "TRANSPOSE",
                "编号,001,002\r\n名称,茶,米\r\n备注,\"甲,乙\",\"带\n换行\"",
            ),
        ] {
            let query = format!("{operation} {data}");
            let (text, preview) = csv_table_preview(&query).unwrap().unwrap();
            assert_eq!(text, expected);
            assert_eq!(transform_csv(&query).as_deref(), Ok(expected));
            assert_eq!(preview.column_pages(), 1);
        }
    }

    #[test]
    fn extracts_csv_regions_with_ordered_columns_and_multiline_records() {
        let data = "编号,商品,备注\n001,茶,甲\n002,梨,\" 乙,丙\n丁 \"\n003,米,";
        let query = format!("取区域 [\"3,2-3\",[\"备注\",1]] {data}");
        let expected = "备注,编号\r\n\" 乙,丙\n丁 \",002\r\n\"\",003";
        let (csv, _) = csv_table_preview(&query).unwrap().unwrap();
        assert_eq!(csv, expected);
        assert_eq!(transform_csv(&query).as_deref(), Ok(expected));
        let pipeline =
            format!(r#"依次处理 ["pick-region [\"2-3\",[\"备注\",1]]","取单元格 [-1,2]"] {data}"#);
        assert_eq!(transform_csv(&pipeline).as_deref(), Ok("003"));
        for (parameters, error) in [
            (r#"["0",[1]]"#, CsvError::InvalidRowRange),
            (r#"["1-4",[1]]"#, CsvError::MissingDataRow),
            (r#"["1",["编号",1]]"#, CsvError::TableRegionArguments),
            (r#"["1",[]]"#, CsvError::TableRegionArguments),
            (r#"["1",[4]]"#, CsvError::ColumnOutOfRange),
        ] {
            assert_eq!(
                transform_csv(&format!("取区域 {parameters} {data}")),
                Err(error),
                "{parameters}"
            );
        }
    }

    #[test]
    fn binds_table_columns_in_input_order_and_pads_shorter_tables() {
        let query = r#"横向合并 ["编号,备注\n001,\"茶,热\"\n002,米","编号,金额\nX,1.50","待填"]"#;
        let expected =
            "编号,备注,编号,金额,待填\r\n001,\"茶,热\",X,1.50,\"\"\r\n002,米,\"\",\"\",\"\"";
        assert_eq!(transform_csv(query).as_deref(), Ok(expected));
        assert_eq!(
            csv_table_preview(query)
                .unwrap()
                .map(|(text, _)| text)
                .as_deref(),
            Ok(expected)
        );
        assert_eq!(
            transform_csv(r#"bind-columns ["\ufeff编号",",备注"]"#).as_deref(),
            Ok("编号,\"\",备注")
        );
        assert_eq!(
            transform_csv(r#"bind-columns ["编号",""]"#),
            Err(CsvError::MissingInput)
        );
        assert_eq!(
            transform_csv(r#"bind-columns ["x,y\n1","备注"]"#),
            Err(CsvError::DifferentColumnCount)
        );
        let large = format!(
            r#"bind-columns ["A\n{}","B"]"#,
            "x".repeat(MAX_EXPANDED_CSV_BYTES)
        );
        assert_eq!(transform_csv(&large), Err(CsvError::JoinTooLarge));
    }

    #[test]
    fn replaces_an_addressed_csv_cell_without_rewriting_other_cell_values() {
        let data = "编号,备注\r\n001,\" 原样\n备注 \"\r\n002,旧\r\n003,末";
        let query = format!(r#"改单元格 [2,"备注","含,逗号\n\"引号\" "] {data}"#);
        let expected =
            "编号,备注\r\n001,\" 原样\n备注 \"\r\n002,\"含,逗号\n\"\"引号\"\" \"\r\n003,末";
        let (csv, _) = csv_table_preview(&query).unwrap().unwrap();
        assert_eq!(csv, expected);
        assert_eq!(transform_csv(&query).as_deref(), Ok(expected));
        let clear = format!(r#"SET-CELL [-1,2,""] {data}"#);
        assert_eq!(
            transform_csv(&clear).as_deref(),
            Ok("编号,备注\r\n001,\" 原样\n备注 \"\r\n002,旧\r\n003,\"\"")
        );
        let pipeline =
            format!(r#"依次处理 ["改单元格 [-2,\"备注\",\"核对\"]","取单元格 [2,2]"] {data}"#);
        assert_eq!(transform_csv(&pipeline).as_deref(), Ok("核对"));
        for (parameters, error) in [
            (r#"[0,2,"新"]"#, CsvError::SetCellArguments),
            (r#"[-4,2,"新"]"#, CsvError::MissingDataRow),
            (r#"[1,"名称","新"]"#, CsvError::MissingColumnName),
            (r#"[1,2,99]"#, CsvError::SetCellArguments),
            (r#"[1,2,"\u0000"]"#, CsvError::NullReplacementValue),
        ] {
            assert_eq!(
                transform_csv(&format!("改单元格 {parameters} {data}")),
                Err(error),
                "{parameters}"
            );
        }
    }

    #[test]
    fn applies_ordered_csv_cell_edits_preserving_the_rest_of_the_table() {
        let data = "编号,备注\r\n001,原样\r\n002,旧\r\n003,末";
        let query = format!(
            r#"批量改单元格 [[1,2,"更新"],[-1,"备注",""],[-3,"备注"," 最后\n值 "]] {data}"#
        );
        let expected = "编号,备注\r\n001,\" 最后\n值 \"\r\n002,旧\r\n003,\"\"";
        assert_eq!(transform_csv(&query).as_deref(), Ok(expected));
        assert_eq!(
            csv_table_preview(&query)
                .unwrap()
                .map(|(text, _)| text)
                .as_deref(),
            Ok(expected)
        );
        let pipeline = format!(
            r#"依次处理 ["批量改单元格 [[-1,2,\"已核对\"]]","取单元格 [-1,\"备注\"]"] {data}"#
        );
        assert_eq!(transform_csv(&pipeline).as_deref(), Ok("已核对"));
        let invalid = format!(r#"set-cells [[1,2,"更新"],[4,"备注","值"]] {data}"#);
        assert_eq!(
            transform_csv(&invalid),
            Err(CsvError::CellEdit {
                index: 2,
                cause: CsvError::MissingDataRow.message()
            })
        );
        let invalid = format!(r#"set-cells [[1,"备注","\u0000"]] {data}"#);
        assert_eq!(
            transform_csv(&invalid),
            Err(CsvError::CellEdit {
                index: 1,
                cause: CsvError::NullReplacementValue.message()
            })
        );
        assert_eq!(
            transform_csv(&format!("set-cells [] {data}")),
            Err(CsvError::SetCellsArguments)
        );
    }

    #[test]
    fn previews_timestamp_sorting_and_mixed_column_orders_with_original_values() {
        let data = "编号,时间,组,金额\n慢,3600,甲10,2\n同一,1970-01-01T01:00:00+01:00,甲2,1.5\n同二,0,甲2,3/2\n高,-1,甲2,4";
        for (operation, expected_order) in [
            ("时间戳排序 [\"时间\"]", ["高", "同一", "同二", "慢"]),
            ("SORT-TIMESTAMPS-DESC 2", ["慢", "同一", "同二", "高"]),
            (
                "多列排序 [[\"组\",\"文本升序\"],[\"金额\",\"数值降序\"]]",
                ["高", "同一", "同二", "慢"],
            ),
        ] {
            let query = format!("{operation} {data}");
            let (csv, preview) = csv_table_preview(&query).unwrap().unwrap();
            assert_eq!(transform_csv(&query).as_deref(), Ok(csv.as_str()));
            let sorted = read_rows(&csv, ',').unwrap();
            let names: Vec<_> = sorted.iter().skip(1).map(|row| row[0].as_str()).collect();
            assert_eq!(names, expected_order);
            assert!(sorted.iter().any(|row| row[3] == "3/2"));
            assert_eq!(preview.column_pages(), 1);
        }
    }

    #[test]
    fn shows_csv_sorting_results_as_tables_with_exact_values_and_stable_ties() {
        let data = "标记,价格\n文件10,3/2\n文件2,1.50\n文件3,2";
        for (operation, expected) in [
            (
                "排序 [\"标记\"]",
                "标记,价格\r\n文件2,1.50\r\n文件3,2\r\n文件10,3/2",
            ),
            (
                "SORT-NUMERIC-DESC 2",
                "标记,价格\r\n文件3,2\r\n文件10,3/2\r\n文件2,1.50",
            ),
        ] {
            let query = format!("{operation} {data}");
            let (csv, preview) = csv_table_preview(&query).unwrap().unwrap();
            assert_eq!(csv, expected);
            assert_eq!(transform_csv(&query).as_deref(), Ok(expected));
            assert_eq!(preview.row_pages(), 1);
        }
    }

    #[test]
    fn finds_rows_containing_all_keywords_across_cells_and_previews_selected_records() {
        let data = "编号,区域,品名\n001,North,绿茶\n002,South,绿茶\n003,North,咖啡\n004,ＮＯＲＴＨ,\"茶,咖啡\"";
        let query = format!("搜索行 [\"ｎｏｒｔｈ\",\"茶\"] {data}");
        let expected = "编号,区域,品名\r\n001,North,绿茶\r\n004,ＮＯＲＴＨ,\"茶,咖啡\"";
        assert_eq!(transform_csv(&query).as_deref(), Ok(expected));
        let (text, preview) = csv_table_preview(&query).unwrap().unwrap();
        assert_eq!(text, expected);
        assert_eq!(preview.row_pages(), 1);
        assert_eq!(preview.column_pages(), 1);
        assert_eq!(
            transform_csv(&format!("SEARCH-ROWS [\"North绿茶\"] {data}")).as_deref(),
            Ok("编号,区域,品名")
        );
        assert_eq!(
            transform_csv(&format!("搜索行 [\" \"] {data}")),
            Err(CsvError::SearchRowKeywords)
        );
        let rows = read_rows(data, ',').unwrap();
        assert_eq!(
            transform_csv(&format!("搜索行 \"\" {data}")),
            Ok(write_rows(&rows, ','))
        );
    }

    #[test]
    fn reports_exact_csv_quantiles_per_group_and_keeps_empty_groups() {
        let query = "分组分位表 [[\"地区\",2],\"金额\",[\"25%\",0.5,\"3/4\"]] 地区,状态,金额\n南,完成,1/2\n北,完成,10\n南,完成,2\n南,完成,\n西,待办,";
        let expected = "地区,状态,概率,分位数\r\n南,完成,1/4,7/8\r\n南,完成,1/2,5/4\r\n南,完成,3/4,13/8\r\n北,完成,1/4,10\r\n北,完成,1/2,10\r\n北,完成,3/4,10\r\n西,待办,1/4,\"\"\r\n西,待办,1/2,\"\"\r\n西,待办,3/4,\"\"";
        assert_eq!(transform_csv(query).as_deref(), Ok(expected));
        let (csv, _) = csv_table_preview(query).expect("分组分位表可预览").unwrap();
        assert_eq!(csv, expected);
        assert_eq!(
            transform_csv("group-quantiles [[],2,[1,0,0.5]] 地区,金额\n南,2\n北,8\n西,5")
                .as_deref(),
            Ok("概率,分位数\r\n1,8\r\n0,2\r\n1/2,5"),
        );
        assert_eq!(
            transform_csv("分组分位表 [[1],2,[0]] 地区,金额").as_deref(),
            Ok("地区,概率,分位数"),
        );
        assert!(matches!(
            transform_csv("分组分位表 [[1],2,[0.5]] 地区,金额\n南,1\n北,\n南,待定"),
            Err(CsvError::DataRowValue { row: 3, .. }),
        ));
        assert_eq!(
            transform_csv("分组分位表 [[1,\"地区\"],2,[0.5]] 地区,金额\n南,1"),
            Err(CsvError::GroupingColumns),
        );
        assert_eq!(
            transform_csv("分组分位表 [[1],2,[]] 地区,金额\n南,1"),
            Err(CsvError::Statistics(StatisticsError::QuantileList)),
        );
    }

    #[test]
    fn selects_csv_weighted_quantile_columns_and_reports_invalid_data_rows() {
        let query = "加权分位表 [2,\"权重\",[\"25%\",0.5,\"3/4\"]] 编号,金额,权重\n001,1,1\n002,2,1\n003,4,50%\n004,4,3/2\n005,,9";
        let expected = "概率,分位数\r\n1/4,1\r\n1/2,2\r\n3/4,4";
        assert_eq!(transform_csv(query).as_deref(), Ok(expected));
        assert_eq!(
            csv_table_preview(query)
                .expect("选列加权分位表应可预览")
                .map(|(text, _)| text)
                .as_deref(),
            Ok(expected)
        );
        assert!(matches!(
            transform_csv("加权分位表 [1,2,[0.5]] 值,权重\r\n1,1\r\n2,-1"),
            Err(CsvError::DataRowValue { row: 2, .. })
        ));
    }

    #[test]
    fn groups_csv_weighted_frequencies_by_exact_labels_and_reports_shares() {
        let query = "分组加权频数 [[\"地区\",2],\"权重\"] 地区,状态,权重\r\n北,完成,0.5\r\n南,完成,3/2\r\n北,完成,25%\r\n西,待办,0\r\n北,待办,";
        let expected = "地区,状态,权重合计,占比\r\n南,完成,3/2,2/3\r\n北,完成,3/4,1/3\r\n西,待办,0,0\r\n北,待办,0,0";
        assert_eq!(transform_csv(query).as_deref(), Ok(expected));
        assert_eq!(
            csv_table_preview(query)
                .expect("分组加权频数应支持表格预览")
                .map(|(text, _)| text)
                .as_deref(),
            Ok(expected)
        );
        assert_eq!(
            transform_csv("GROUP-WEIGHTED-FREQUENCY [[1],2] 类别,权重").as_deref(),
            Ok("类别,权重合计,占比")
        );
        assert_eq!(
            transform_csv("分组加权频数 [[1],2] 类别,权重\n甲,0\n乙,"),
            Err(CsvError::Statistics(StatisticsError::ZeroTotalWeight))
        );
        assert!(matches!(
            transform_csv("分组加权频数 [[1],2] 类别,权重\n甲,1\n乙,-1/2"),
            Err(CsvError::DataRowValue { row: 2, .. })
        ));
    }

    #[test]
    fn previews_weighted_means_in_first_group_order_and_leaves_zero_means_blank() {
        let data = "区域,成绩,权重\n\"南,一部\",80,30%\n北,12,2\n\"南,一部\",90,70%\n东,6,0\n东,,3";
        let query = format!(r#"分组加权平均 [["区域"],"成绩","权重"] {data}"#);
        let expected = "区域,权重合计,加权平均\r\n\"南,一部\",1,87\r\n北,2,12\r\n东,0,\"\"";
        let (text, table) = csv_table_preview(&query).unwrap().unwrap();
        assert_eq!(text, expected);
        assert_eq!(transform_csv(&query).as_deref(), Ok(expected));
        assert_eq!(table.row_pages(), 1);
        assert!(matches!(
            transform_csv("group-weighted-mean [[1],2,3] 地区,值,权重\n南,1,2\n北,2,坏"),
            Err(CsvError::DataRowValue { row: 2, .. })
        ));
        assert_eq!(
            transform_csv("分组加权平均 [[],2,3] 地区,值,权重"),
            Err(CsvError::WeightedGroupArguments)
        );
    }

    #[test]
    fn computes_exact_weighted_mean_from_two_csv_columns_and_identifies_errors() {
        let data = "项目,成绩,权重\n试卷,80,30%\n忽略,,70%\n实验,90,70％";
        for op in [r#"加权平均 ["成绩","权重"]"#, "weighted-mean [2,3]"] {
            assert_eq!(transform_csv(&format!("{op} {data}")).as_deref(), Ok("87"));
        }
        assert_eq!(
            transform_csv("加权平均 [1,2] 成绩,权重\n80,0\n90,0%"),
            Err(CsvError::Statistics(StatisticsError::ZeroTotalWeight))
        );
        assert!(matches!(
            transform_csv("加权平均 [1,2] 成绩,权重\n80,30%\n90,-1"),
            Err(CsvError::DataRowValue { row: 2, .. })
        ));
        assert_eq!(
            transform_csv("加权平均 [1] 成绩,权重"),
            Err(CsvError::WeightedMeanArguments)
        );
    }

    #[test]
    fn previews_single_csv_filters_and_preserves_complete_matching_rows() {
        let data = "编号,金额,备注\n001,1.50,\"甲,乙\"\n002,3/2,茶\n003,2,米";
        for (operation, expected) in [
            (
                r#"筛选 "备注" "甲,乙""#,
                "编号,金额,备注\r\n001,1.50,\"甲,乙\"",
            ),
            (
                r#"REGEX-EXCLUDE 3 "^(茶|米)$""#,
                "编号,金额,备注\r\n001,1.50,\"甲,乙\"",
            ),
            (
                "数值筛选 2 >= 3/2",
                "编号,金额,备注\r\n001,1.50,\"甲,乙\"\r\n002,3/2,茶\r\n003,2,米",
            ),
            (
                r#"排除 "编号" "002""#,
                "编号,金额,备注\r\n001,1.50,\"甲,乙\"\r\n003,2,米",
            ),
            (r#"FILTER 1 "004""#, "编号,金额,备注"),
        ] {
            let query = format!("{operation} {data}");
            let (text, preview) = csv_table_preview(&query).unwrap().unwrap();
            assert_eq!(text, expected, "{operation}");
            assert_eq!(
                transform_csv(&query).as_deref(),
                Ok(expected),
                "{operation}"
            );
            assert_eq!(preview.row_pages(), usize::from(expected.contains("\r\n")));
        }
    }

    #[test]
    fn combines_text_numeric_and_regex_filters_preserving_original_record_order() {
        let input = "编号,状态,金额,备注\r\n001,完成,1.50,\"咖啡,热\"\r\n002,完成,3/2,茶\r\n003,待确认,1e1,\"咖啡\n中杯\"\r\n004,完成,2e1,水";
        for (arguments, expected) in [
            (
                r#"多条件筛选 [["状态","文本=","完成"],[3,">=","3/2"],[3,"<","2"],["备注","正则","^咖啡"]]"#,
                "编号,状态,金额,备注\r\n001,完成,1.50,\"咖啡,热\"",
            ),
            (
                r#"任一条件筛选 [["状态","文本!=","完成"],["备注","正则排除","^(茶|水)$"]]"#,
                "编号,状态,金额,备注\r\n001,完成,1.50,\"咖啡,热\"\r\n003,待确认,1e1,\"咖啡\n中杯\"",
            ),
            (
                r#"filter-all [["状态","text=","忽略"]]"#,
                "编号,状态,金额,备注",
            ),
        ] {
            assert_eq!(
                transform_csv(&format!("{arguments}\n{input}")).as_deref(),
                Ok(expected),
            );
            let query = format!("{arguments}\n{input}");
            let (text, preview) = csv_table_preview(&query).unwrap().unwrap();
            assert_eq!(text, expected);
            assert_eq!(preview.row_pages(), usize::from(expected.contains("\r\n")));
        }
        for (query, error) in [
            (r#"多条件筛选 [] A,B"#, CsvError::FilterConditions),
            (
                r#"任一条件筛选 [[3,"文本=","x"]] A,B"#,
                CsvError::ColumnOutOfRange,
            ),
            (
                r#"filter-all [[1,"?","0"]] A,B"#,
                CsvError::FilterConditions,
            ),
            (
                "多条件筛选 [[1,\"文本=\",\"x\"],[2,\">=\",\"1\"]] A,B\nz,坏",
                CsvError::InvalidNumber,
            ),
            (
                "任一条件筛选 [[1,\"文本=\",\"x\"],[2,\">=\",\"1\"]] A,B\nx,坏",
                CsvError::InvalidNumber,
            ),
        ] {
            assert_eq!(transform_csv(query), Err(error), "{query}");
        }
    }

    #[test]
    fn filters_csv_cells_with_regex_while_preserving_quotes_and_order() {
        let csv = "编号,备注\n001,\"咖啡,热\"\n002,冷茶\n003,\"咖啡\n常温\"\n004,茶";
        assert_eq!(
            transform_csv(&format!("正则筛选 2 \"^咖啡\"\n{csv}")).as_deref(),
            Ok("编号,备注\r\n001,\"咖啡,热\"\r\n003,\"咖啡\n常温\"")
        );
        assert_eq!(
            transform_csv(&format!("regex-exclude 2 \"^咖啡\"\n{csv}")).as_deref(),
            Ok("编号,备注\r\n002,冷茶\r\n004,茶")
        );
        assert_eq!(
            transform_csv(&format!("regex-filter 2 \"[\"\n{csv}")),
            Err(CsvError::Regex(TextError::InvalidRegex))
        );
        assert_eq!(
            transform_csv(&format!("regex-filter 3 \"^咖啡\"\n{csv}")),
            Err(CsvError::ColumnOutOfRange)
        );
    }

    #[test]
    fn filters_numeric_cells_exactly_and_keeps_original_text() {
        let table = "编号,价格\n001,1.50\n002,1.499999999999999999\n003,3/2\n004,2e0";
        for (operator, expected) in [
            ("<", "编号,价格\r\n002,1.499999999999999999"),
            (
                "≤",
                "编号,价格\r\n001,1.50\r\n002,1.499999999999999999\r\n003,3/2",
            ),
            ("=", "编号,价格\r\n001,1.50\r\n003,3/2"),
            ("!=", "编号,价格\r\n002,1.499999999999999999\r\n004,2e0"),
            (">=", "编号,价格\r\n001,1.50\r\n003,3/2\r\n004,2e0"),
            (">", "编号,价格\r\n004,2e0"),
        ] {
            assert_eq!(
                transform_csv(&format!("数值筛选 2 {operator} 3/2\n{table}")).as_deref(),
                Ok(expected)
            );
        }
        assert_eq!(
            transform_csv("filter-numeric 2 >= 1e1\n编号,价格\n001,未知"),
            Err(CsvError::InvalidNumber)
        );
        assert_eq!(
            transform_csv("filter-numeric 2 => 1\n编号,价格\n001,2"),
            Err(CsvError::InvalidNumberFilter)
        );
    }

    #[test]
    fn counts_csv_column_combinations_in_first_seen_order() {
        let input =
            "分组计数 2,1 编号,地区,内容\n001,南,甲\n002,,乙\n001,北,丙\n001,南,丁\n002,\"\",戊";
        assert_eq!(
            transform_csv(input).as_deref(),
            Ok("地区,编号,计数\r\n南,001,2\r\n\"\",002,2\r\n北,001,1")
        );
        assert_eq!(
            transform_csv("group-count 1 组,内容").as_deref(),
            Ok("组,计数")
        );
        assert_eq!(
            transform_csv("分组计数 3 组,内容"),
            Err(CsvError::ColumnOutOfRange)
        );
    }

    #[test]
    fn sorts_csv_by_natural_column_order_and_preserves_ties() {
        let data = "编号,名称,区域,备注\n001,项目10,东,\"茶,咖啡\"\n002,项目2,西,甲\n003,项目02,东,乙\n004,项目2,东,丙";
        assert_eq!(
            transform_csv(&format!("排序 2,3\n{data}")).as_deref(),
            Ok("编号,名称,区域,备注\r\n003,项目02,东,乙\r\n004,项目2,东,丙\r\n002,项目2,西,甲\r\n001,项目10,东,\"茶,咖啡\"")
        );
        assert_eq!(
            transform_csv(&format!("sort-columns-desc 2,3\n{data}")).as_deref(),
            Ok("编号,名称,区域,备注\r\n001,项目10,东,\"茶,咖啡\"\r\n002,项目2,西,甲\r\n003,项目02,东,乙\r\n004,项目2,东,丙")
        );
        assert_eq!(transform_csv("排序 1 a,b").as_deref(), Ok("a,b"));
        assert_eq!(transform_csv("降序 3 a,b"), Err(CsvError::ColumnOutOfRange));
    }

    #[test]
    fn flags_csv_outliers_using_exact_fences_and_preserves_row_order() {
        assert_eq!(
            transform_csv("离群列 [\"离群\",\"金额\"] 编号,金额\n001,100\n002,1\n003,2\n004,3\n005,4\n006,")
                .as_deref(),
            Ok("编号,金额,离群\r\n001,100,true\r\n002,1,false\r\n003,2,false\r\n004,3,false\r\n005,4,false\r\n006,\"\",\"\"")
        );
        assert_eq!(
            transform_csv("outlier-column [\"离群\",1] 金额\n1\n2\n3\n4\n7").as_deref(),
            Ok("金额,离群\r\n1,false\r\n2,false\r\n3,false\r\n4,false\r\n7,false")
        );
        assert_eq!(
            transform_csv("离群列 [\"离群\",1] 金额").as_deref(),
            Ok("金额,离群")
        );
        assert!(matches!(
            transform_csv("离群列 [\"离群\",1] 金额\n1\n待定"),
            Err(CsvError::DataRowValue { row: 2, .. })
        ));
    }

    #[test]
    fn calculates_spearman_matrix_with_average_ranks_for_numerical_ties() {
        let data = "x,y,z,固定\n0,0,2,5\n1,1.0,1,5\n2,1/1,0,5\n8,,9,6";
        assert_eq!(
            transform_csv(&format!(r#"秩相关矩阵 ["x",2,"z","固定"] {data}"#)).as_deref(),
            Ok(concat!(
                "列,x,y,z,固定\r\n",
                "x,1,√(3/4),-1,\"\"\r\n",
                "y,√(3/4),1,-√(3/4),\"\"\r\n",
                "z,-1,-√(3/4),1,\"\"\r\n",
                "固定,\"\",\"\",\"\",\"\"",
            ))
        );
        assert_eq!(
            transform_csv("spearman-matrix [2,1] x,y\n0,50\n1,100\n2,500").as_deref(),
            Ok("列,y,x\r\ny,1,1\r\nx,1,1")
        );
    }

    #[test]
    fn calculates_exact_sample_and_population_covariance_matrices() {
        let data = "x,y,固定\n-1,2,5\n0,0,5\n1,-2,5\n8,,6";
        assert_eq!(
            transform_csv(&format!(r#"协方差矩阵 ["x",2,"固定"] {data}"#)).as_deref(),
            Ok("列,x,y,固定\r\nx,1,-2,0\r\ny,-2,4,0\r\n固定,0,0,0")
        );
        assert_eq!(
            transform_csv(&format!("population-covariance-matrix [1,2] {data}")).as_deref(),
            Ok("列,x,y\r\nx,2/3,-4/3\r\ny,-4/3,8/3")
        );
    }

    #[test]
    fn calculates_a_symmetric_exact_correlation_matrix_from_complete_csv_rows() {
        let data = "编号,x,y,z,固定\nA,-1,2,1,5\nB,0,0,-2,5\nC,1,-2,1,5\nD,8,,9,6";
        assert_eq!(
            transform_csv(&format!(r#"相关矩阵 ["z",2,"y","固定"] {data}"#)).as_deref(),
            Ok("列,z,x,y,固定\r\nz,1,0,0,\"\"\r\nx,0,1,-1,\"\"\r\ny,0,-1,1,\"\"\r\n固定,\"\",\"\",\"\",\"\"")
        );
        assert_eq!(
            transform_csv("correlation-matrix [1,2] x,y\n1,2\n3,"),
            Err(CsvError::PairMatrixRows)
        );
        assert_eq!(
            transform_csv("相关矩阵 [1,\"x\"] x,y\n1,2\n3,4"),
            Err(CsvError::PairMatrixColumns)
        );
        assert!(matches!(
            transform_csv("相关矩阵 [1,2] x,y\n1,2\n待定,4"),
            Err(CsvError::DataRowValue { row: 2, .. })
        ));
    }

    #[test]
    fn profiles_each_csv_group_in_order_with_exact_statistics_and_missing_counts() {
        let data =
            "地区,阶段,金额\n南,批,1/2\n北,现货,待定\n南,批,2\n北,现货, \n南,现货,\n南,批,待定";
        assert_eq!(
            transform_csv(&format!(r#"分组数值摘要 ["地区",2] "金额" {data}"#)).as_deref(),
            Ok(concat!(
                "地区,阶段,数值数,空白数,非数值数,最小值,最大值,平均值,中位数\r\n",
                "南,批,2,0,1,1/2,2,5/4,5/4\r\n",
                "北,现货,0,1,1,\"\",\"\",\"\",\"\"\r\n",
                "南,现货,0,1,0,\"\",\"\",\"\",\"\"",
            ))
        );
    }

    #[test]
    fn profiles_selected_csv_columns_without_confusing_missing_and_text_values() {
        let data = "编号,金额,空列,系数\r\n001,1/2,,1.50\r\n002,待定, ,6/4\r\n003,2,,";
        assert_eq!(
            transform_csv(&format!(r#"数值列摘要 [4,"金额",3] {data}"#)).as_deref(),
            Ok("列号,列名,数值数,空白数,非数值数,最小值,最大值,平均值,中位数\r\n4,系数,2,1,0,3/2,3/2,3/2,3/2\r\n2,金额,2,0,1,1/2,2,5/4,5/4\r\n3,空列,0,3,0,\"\",\"\",\"\",\"\"")
        );
        assert_eq!(
            transform_csv("numeric-profile [1] 金额").as_deref(),
            Ok("列号,列名,数值数,空白数,非数值数,最小值,最大值,平均值,中位数\r\n1,金额,0,0,0,\"\",\"\",\"\",\"\"")
        );
        assert_eq!(
            transform_csv("数值列摘要 [1,\"金额\"] 金额\n2"),
            Err(CsvError::NamedColumnArguments)
        );
        let many = format!("numeric-profile [1] 金额\n{}", "1\n".repeat(MAX_VALUES + 1));
        assert_eq!(
            transform_csv(&many),
            Err(CsvError::Statistics(StatisticsError::TooManyValues))
        );
    }

    #[test]
    fn summarizes_only_the_selected_numeric_csv_column() {
        let data = "编号,数值\n001,1/2\n002,1.5\n003,2.5e0";
        for (query, last_line) in [
            ("列统计 2", "样本方差 1 · 标准差 1"),
            ("列总体 2", "总体方差 2/3 · 标准差 √(2/3)"),
            (r#"列统计 "数值""#, "样本方差 1 · 标准差 1"),
            (r#"列总体 "数值""#, "总体方差 2/3 · 标准差 √(2/3)"),
        ] {
            assert_eq!(
                transform_csv(&format!("{query}\n{data}")),
                Ok(format!("数量 3 · 合计 9/2\n平均值 3/2 · 中位数 3/2\n最小值 1/2 · 最大值 5/2\n{last_line}"))
            );
        }
        for (query, error) in [
            ("列统计 1,2 编号,金额", CsvError::StatisticsColumn),
            ("列统计 3 编号,金额", CsvError::ColumnOutOfRange),
            ("列总体 2 编号,金额", CsvError::EmptyStatisticsColumn),
            ("列统计 2 编号,金额\n001,", CsvError::InvalidNumber),
            ("列统计 2 编号,金额\n001,1+2", CsvError::InvalidNumber),
            (r#"列统计 "金额" 编号,数值"#, CsvError::MissingColumnName),
            (
                r#"列总体 "金额" 编号,金额,金额"#,
                CsvError::AmbiguousColumnName,
            ),
        ] {
            assert_eq!(transform_csv(query), Err(error), "{query}");
        }
    }

    #[test]
    fn parses_formatted_csv_dates_without_changing_blank_or_original_cells() {
        let query = "日期解析列 [\"标准日期\",\"原日期\",\"%d/%m/%Y\"]\n原日期,编号\n31/01/2024,甲\n29/02/2024,乙\n,丙";
        assert_eq!(
            transform_csv(query).as_deref(),
            Ok("原日期,编号,标准日期\r\n31/01/2024,甲,2024-01-31\r\n29/02/2024,乙,2024-02-29\r\n\"\",丙,\"\"")
        );
        assert_eq!(
            transform_csv(
                "date-parse-column [\"日\",1,\"%d/%m/%Y\"]\n日期\n31/01/2024\n29/02/2023"
            ),
            Err(CsvError::DataRowValue {
                row: 2,
                cause: "日期不符合指定格式"
            })
        );
        assert_eq!(
            transform_csv("日期解析列 [\"日\",1,\"%Q\"]\n日期"),
            Err(CsvError::DateFormatColumnPattern)
        );
    }

    #[test]
    fn appends_spreadsheet_time_serials_with_exact_subday_precision() {
        let to = "表格时刻序号列 [\"序号\",\"时刻\",1904]\n设备,时刻\nA,1904-01-01 06:00\nB,\nC,1904-01-01 00:00:00.000000001";
        let expected = concat!(
            "设备,时刻,序号\r\n",
            "A,1904-01-01 06:00,0.25\r\n",
            "B,\"\",\"\"\r\n",
            "C,1904-01-01 00:00:00.000000001,1/86400000000000",
        );
        assert_eq!(transform_csv(to).as_deref(), Ok(expected));
        let (csv, _) = csv_table_preview(to).expect("序号列应可预览").unwrap();
        assert_eq!(csv, expected);
        assert_eq!(
            transform_csv("序号转日期时刻列 [\"时刻\",2,1900] 设备,序号\nA,45351.5\nB,").as_deref(),
            Ok("设备,序号,时刻\r\nA,45351.5,2024-02-29 12:00:00\r\nB,\"\",\"\""),
        );
        assert!(matches!(
            transform_csv("serial-datetime-column [\"时刻\",2,1900] 设备,序号\nA,45351.5\nB,60.5"),
            Err(CsvError::DataRowValue { row: 2, .. }),
        ));
    }

    #[test]
    fn appends_spreadsheet_dates_or_serials_without_rewriting_input_cells() {
        assert_eq!(
            transform_csv("表格序号列 [\"序号\",\"日期\",1900]\n日期,编号\n2024-02-29,001\n,002\n2024-W09-5,003").as_deref(),
            Ok("日期,编号,序号\r\n2024-02-29,001,45351\r\n\"\",002,\"\"\r\n2024-W09-5,003,45352")
        );
        assert_eq!(
            transform_csv("序号转日期列 [\"日期\",1,1904]\n序号\n0\n39267\n\"\"").as_deref(),
            Ok("序号,日期\r\n0,1904-01-01\r\n39267,2011-07-05\r\n\"\",\"\"")
        );
        assert_eq!(
            transform_csv("序号转日期列 [\"日期\",1,1900]\n序号\n59\n60"),
            Err(CsvError::DataRowValue {
                row: 2,
                cause: "1900 系统的序号 60 是兼容占位，对应无效日期 1900-02-29",
            })
        );
        assert_eq!(
            transform_csv("表格序号列 [\"序号\",1,2000]\n日期\n2024-02-29"),
            Err(CsvError::SpreadsheetDateColumnArguments)
        );
    }

    #[test]
    fn formats_complete_dates_as_a_column_and_validates_the_pattern() {
        let query = "日期格式列 [\"年月\",\"日期\",\"%Y年%m月\"]\n日期,编号\n2024-02-29,甲\n2024-W09-5,乙\n2024年1月31日,丙\n,丁";
        assert_eq!(
            transform_csv(query).as_deref(),
            Ok("日期,编号,年月\r\n2024-02-29,甲,2024年02月\r\n2024-W09-5,乙,2024年03月\r\n2024年1月31日,丙,2024年01月\r\n\"\",丁,\"\"")
        );
        assert_eq!(
            transform_csv("date-format-column [\"月\",1,\"%Y,%m\"]\n日期\n2024-01-31").as_deref(),
            Ok("日期,月\r\n2024-01-31,\"2024,01\"")
        );
        assert_eq!(
            transform_csv("日期格式列 [\"时\",1,\"%H\"]\n日期"),
            Err(CsvError::DateFormatColumnPattern)
        );
        assert!(matches!(
            transform_csv("日期格式列 [\"月\",1,\"%m\"]\n日期\n2024-02-29\n2024-02-30"),
            Err(CsvError::DataRowValue { row: 2, .. })
        ));
    }

    #[test]
    fn shifts_csv_dates_by_signed_months_and_clamps_at_month_end() {
        let query = "日期加月列 [\"到期日期\",\"日期\",\"月数\"]\n日期,月数\n2024-01-31,1\n2024-03-31,-1\n2023-01-31,1\n2024-02-29,12\n2024-01-01,";
        assert_eq!(
            transform_csv(query).as_deref(),
            Ok("日期,月数,到期日期\r\n2024-01-31,1,2024-02-29\r\n2024-03-31,-1,2024-02-29\r\n2023-01-31,1,2023-02-28\r\n2024-02-29,12,2025-02-28\r\n2024-01-01,\"\",\"\"")
        );
        assert_eq!(
            transform_csv("date-month-offset-column [\"结果\",1,2]\n日期,月数\n2024-02-29,1/2"),
            Err(CsvError::DataRowValue {
                row: 1,
                cause: "月数须为整数且在支持范围内"
            })
        );
        assert!(matches!(
            transform_csv("日期加月列 [\"结果\",1,2]\n日期,月数\n9999-12-31,1"),
            Err(CsvError::DataRowValue { row: 1, .. })
        ));
    }

    #[test]
    fn shifts_each_csv_date_by_workdays_across_weekends_in_both_directions() {
        let query = "日期加工作日列 [\"到期日期\",\"日期\",\"工作日数\"]\n日期,工作日数\n2024-03-01,1\n2024-03-04,-1\n2024-03-02,0\n2024-03-03,-1\n2024-03-01,";
        assert_eq!(
            transform_csv(query).as_deref(),
            Ok("日期,工作日数,到期日期\r\n2024-03-01,1,2024-03-04\r\n2024-03-04,-1,2024-03-01\r\n2024-03-02,0,2024-03-02\r\n2024-03-03,-1,2024-03-01\r\n2024-03-01,\"\",\"\"")
        );
        assert_eq!(
            transform_csv(
                "workday-offset-column [\"结果\",1,2]\n日期,工作日数\n2024-03-01,1\n2024-03-01,1/2"
            ),
            Err(CsvError::DataRowValue {
                row: 2,
                cause: "工作日数须为整数且在支持范围内"
            })
        );
        assert!(matches!(
            transform_csv("日期加工作日列 [\"结果\",1,2]\n日期,工作日数\n9999-12-31,1"),
            Err(CsvError::DataRowValue { row: 1, .. })
        ));
    }

    #[test]
    fn applies_one_custom_workday_calendar_to_every_csv_record() {
        let query = "日期加工作日列 [\"到期日期\",\"日期\",\"工作日数\",[[\"2024-03-04\"],[\"2024-03-02\"]]]\n日期,工作日数\n2024-03-01,1\n2024-03-01,2\n2024-03-05,-1\n2024-03-04,0\n,1";
        assert_eq!(
            transform_csv(query).as_deref(),
            Ok("日期,工作日数,到期日期\r\n2024-03-01,1,2024-03-02\r\n2024-03-01,2,2024-03-05\r\n2024-03-05,-1,2024-03-02\r\n2024-03-04,0,2024-03-04\r\n\"\",1,\"\"")
        );
        for calendar in [
            r#"[["2024-03-04"],["2024-03-04"]]"#,
            r#"[[],["2024-02-30"]]"#,
        ] {
            let query = format!("workday-offset-column [\"结果\",1,2,{calendar}] 日期,工作日数");
            assert_eq!(
                transform_csv(&query),
                Err(CsvError::WorkdayOffsetExceptions)
            );
        }
        assert_eq!(
            transform_csv("workday-offset-column [\"结果\",1,2,[[]]] 日期,工作日数"),
            Err(CsvError::WorkdayOffsetColumnArguments)
        );
    }

    #[test]
    fn appends_csv_cell_lengths_for_each_unicode_unit() {
        let csv = "编号,文本\n1,👩‍💻\n2,e\u{301}\n3,";
        for (unit, first, second) in [
            ("字素", 1, 1),
            ("码位", 3, 2),
            ("UTF-16 单元", 5, 2),
            ("UTF-8 字节", 11, 3),
        ] {
            let query = format!("长度列 [\"长度\",\"文本\",\"{unit}\"]\n{csv}");
            assert_eq!(
                transform_csv(&query).unwrap(),
                format!("编号,文本,长度\r\n1,👩‍💻,{first}\r\n2,e\u{301},{second}\r\n3,\"\",0")
            );
        }
        assert_eq!(
            transform_csv("length-column [\"长度\",1,\"未知\"] 文本"),
            Err(CsvError::TextLengthColumnArguments)
        );
        assert_eq!(
            transform_csv("length-column [\"文本\",1,\"utf16\"] 文本"),
            Err(CsvError::HeaderNameConflict)
        );
        assert_eq!(
            transform_csv("length-column [\"长度\",2,\"utf8\"] 文本"),
            Err(CsvError::ColumnOutOfRange)
        );
    }

    #[test]
    fn appends_slices_of_complete_graphemes_without_splitting_csv_records() {
        let query = "截取列 [\"摘要\",\"文本\",\"2-3\"]\n编号,文本\n1,A👩‍💻e\u{301}Z\n2,北\n3,\n4,\"甲\r\n乙丙\"";
        assert_eq!(
            transform_csv(query).as_deref(),
            Ok("编号,文本,摘要\r\n1,A👩‍💻e\u{301}Z,👩‍💻e\u{301}\r\n2,北,\"\"\r\n3,\"\",\"\"\r\n4,\"甲\r\n乙丙\",\"\r\n乙\"")
        );
        assert_eq!(
            transform_csv("slice-column [\"摘要\",1,\"1\"] 文本\n👩‍💻Z").as_deref(),
            Ok("文本,摘要\r\n👩‍💻Z,👩‍💻")
        );
        assert_eq!(
            transform_csv("截取列 [\"尾两字\",1,\"-2..-1\"] 文本\nA👩‍💻e\u{301}\n北\n\"\"")
                .as_deref(),
            Ok("文本,尾两字\r\nA👩‍💻e\u{301},👩‍💻e\u{301}\r\n北,北\r\n\"\",\"\"")
        );
        for range in ["0", "3-2", "2-"] {
            let query = format!("截取列 [\"摘要\",1,\"{range}\"] 文本");
            assert_eq!(
                transform_csv(&query),
                Err(CsvError::TextSliceColumnArguments)
            );
        }
    }

    #[test]
    fn replaces_every_exact_substring_and_keeps_original_csv_cells_intact() {
        let query = "替换列 [\"新地址\",\"地址\",\"旧路\",\"新,路\"]\n编号,地址\n1,旧路12号旧路口\n2,\"北街\r\n南街\"\n3,";
        assert_eq!(
            transform_csv(query).as_deref(),
            Ok("编号,地址,新地址\r\n1,旧路12号旧路口,\"新,路12号新,路口\"\r\n2,\"北街\r\n南街\",\"北街\r\n南街\"\r\n3,\"\",\"\"")
        );
        assert_eq!(
            transform_csv("replace-column [\"简写\",1,\".\",\"\"] 源\nA.B.").as_deref(),
            Ok("源,简写\r\nA.B.,AB")
        );
        assert_eq!(
            transform_csv("替换列 [\"新列\",1,\"\",\"x\"] 原列"),
            Err(CsvError::TextReplaceColumnArguments)
        );
        let query = format!(
            "替换列 [\"新列\",1,\"a\",\"{}\"] 原列\n{}",
            "x".repeat(1100),
            "a".repeat(1100)
        );
        assert_eq!(transform_csv(&query), Err(CsvError::AddedColumnTooLarge));
    }

    #[test]
    fn expands_json_array_cells_into_rows_and_preserves_empty_origins() {
        let query = r#"JSON展开行 ["商品","数据","/商品"]
编号,数据
001,"{""商品"":[""茶,热"",9007199254740993.00,null]}"
002,"{""商品"":[]}"
003,{}
004,"{""商品"":null}"
005,"#;
        let expected = [
            "编号,数据,商品",
            r#"001,"{""商品"":[""茶,热"",9007199254740993.00,null]}","茶,热""#,
            r#"001,"{""商品"":[""茶,热"",9007199254740993.00,null]}",9007199254740993.00"#,
            r#"001,"{""商品"":[""茶,热"",9007199254740993.00,null]}","""#,
            r#"002,"{""商品"":[]}","""#,
            r#"003,{},"""#,
            r#"004,"{""商品"":null}","""#,
            r#"005,"","""#,
        ]
        .join("\r\n");
        assert_eq!(transform_csv(query).as_deref(), Ok(expected.as_str()));
        assert_eq!(
            transform_csv(
                r#"json-expand-rows ["商品","数据",""] 数据
[]
{}"#
            ),
            Err(CsvError::DataRowValue {
                row: 2,
                cause: JsonError::ExpectedArray.message(),
            })
        );
        let array = format!("[{}0]", "0,".repeat(160));
        let query = format!(
            "JSON展开行 [\"产品\",\"数据\",\"\"] 文本,数据\n{},\"{array}\"",
            "a".repeat(7500),
        );
        assert_eq!(transform_csv(&query), Err(CsvError::AddedColumnTooLarge));
    }

    #[test]
    fn marks_present_json_pointer_cells_even_when_the_value_is_null_or_zero() {
        let query = r#"JSON存在列 ["有金额","数据","/金额"]
编号,数据
1,"{""金额"":null}"
2,{}
3,
4,"{""金额"":0}""#;
        let expected = [
            "编号,数据,有金额",
            r#"1,"{""金额"":null}",true"#,
            "2,{},false",
            r#"3,"",false"#,
            r#"4,"{""金额"":0}",true"#,
        ]
        .join("\r\n");
        assert_eq!(transform_csv(query).as_deref(), Ok(expected.as_str()));
        assert_eq!(
            transform_csv(
                r#"json-exists-column ["存在",1,""] 原列
null
false"#
            )
            .as_deref(),
            Ok("原列,存在\r\nnull,true\r\nfalse,true")
        );
    }

    #[test]
    fn counts_array_elements_and_object_entries_in_json_pointer_cells() {
        let query = r#"JSON长度列 ["项数","数据","/商品"]
编号,数据
1,"{""商品"":[""茶"",{""名称"":""梨""}]}"
2,"{""商品"":{""茶"":1,""茶"":2,""梨"":3}}"
3,"{""商品"":[]}"
4,"{""商品"":""茶""}"
5,{}
6,"{""商品"":null}"
7,"{""商品"":{}}""#;
        let expected = [
            "编号,数据,项数",
            r#"1,"{""商品"":[""茶"",{""名称"":""梨""}]}",2"#,
            r#"2,"{""商品"":{""茶"":1,""茶"":2,""梨"":3}}",3"#,
            r#"3,"{""商品"":[]}",0"#,
            r#"4,"{""商品"":""茶""}","""#,
            r#"5,{},"""#,
            r#"6,"{""商品"":null}","""#,
            r#"7,"{""商品"":{}}",0"#,
        ]
        .join("\r\n");
        assert_eq!(transform_csv(query).as_deref(), Ok(expected.as_str()));
        assert_eq!(
            transform_csv(
                r#"json-length-column ["项数",1,""] 原列
[]
[1]"#
            )
            .as_deref(),
            Ok("原列,项数\r\n[],0\r\n[1],1")
        );
    }

    #[test]
    fn reports_json_pointer_cell_types_and_distinguishes_null_from_missing_values() {
        let query = r#"JSON类型列 ["类型","数据","/金额"]
编号,数据
1,"{""金额"":9007199254740993.00}"
2,"{""金额"":""12""}"
3,"{""金额"":null}"
4,{}
5,"{""金额"":[]}"
6,"{""金额"":false}"
7,"{""金额"":{}}""#;
        let expected = [
            r#"编号,数据,类型"#,
            r#"1,"{""金额"":9007199254740993.00}",number"#,
            r#"2,"{""金额"":""12""}",string"#,
            r#"3,"{""金额"":null}",null"#,
            r#"4,{},"""#,
            r#"5,"{""金额"":[]}",array"#,
            r#"6,"{""金额"":false}",boolean"#,
            r#"7,"{""金额"":{}}",object"#,
        ]
        .join("\r\n");
        assert_eq!(transform_csv(query).as_deref(), Ok(expected.as_str()));
        assert_eq!(
            transform_csv(
                r#"json-type-column ["类型",1,""] 原列
true
null"#
            )
            .as_deref(),
            Ok("原列,类型\r\ntrue,boolean\r\nnull,null")
        );
    }

    #[test]
    fn converts_csv_integer_cells_across_radices_without_rounding() {
        let query = concat!(
            "进制列 [\"十进制\",\"编号\",16,10] 项目,编号\n",
            "甲,ff\n乙,-a_b\n丙,20_0000_0000_0001\n丁,",
        );
        assert_eq!(
            transform_csv(query).as_deref(),
            Ok("项目,编号,十进制\r\n甲,ff,255\r\n乙,-a_b,-171\r\n丙,20_0000_0000_0001,9007199254740993\r\n丁,\"\",\"\"")
        );
        assert!(matches!(
            transform_csv("radix-column [\"结果\",1,2,10] 编号\n101\n2"),
            Err(CsvError::DataRowValue { row: 2, .. })
        ));
        assert_eq!(
            transform_csv("进制列 [\"结果\",1,1,10] 编号"),
            Err(CsvError::RadixColumnArguments)
        );
    }

    #[test]
    fn converts_finite_and_repeating_radix_fractions_in_csv() {
        assert_eq!(
            transform_csv(concat!(
                "进制小数列 [\"二进制\",\"值\",10,2]\n编号,值\n",
                "001,10.625\n002,0.1\n003,0.(9)\n004, -0.5 \n005,"
            ))
            .as_deref(),
            Ok(concat!(
                "编号,值,二进制\r\n001,10.625,1010.101\r\n002,0.1,0.0(0011)\r\n",
                "003,0.(9),1\r\n004, -0.5 ,-0.1\r\n005,\"\",\"\""
            ))
        );
        assert_eq!(
            transform_csv("radix-fraction-column [\"结果\",1,2,10]\n值\n0.0(0011)").as_deref(),
            Ok("值,结果\r\n0.0(0011),0.1"),
        );
        assert!(matches!(
            transform_csv("进制小数列 [\"结果\",1,2,10]\n值\n0.1\n0.(2)"),
            Err(CsvError::DataRowValue { row: 2, .. })
        ));
    }

    #[test]
    fn extracts_url_parameter_arrays_per_csv_row_and_shows_decode_error_locations() {
        let csv = concat!(
            "URL参数列 [\"标签\",\"网址\",\"tag\"] 编号,网址\n",
            "1,https://example.invalid/?tag=a+1&t%61g=b%2Bc&Tag=ignored&tag=#fragment\n",
            "2,https://example.invalid/?other=tea\n3,\n4,tag=%00",
        );
        let output = transform_csv(csv).unwrap();
        let rows = read_rows(&output, ',').unwrap();
        assert_eq!(rows[0], ["编号", "网址", "标签"]);
        assert_eq!(rows[1][2], r#"["a 1","b+c",""]"#);
        assert_eq!(rows[2][2], "[]");
        assert_eq!(rows[3][2], "");
        assert_eq!(rows[4][2], r#"["\u0000"]"#);
        assert!(matches!(
            transform_csv(
                "url-parameter-column [\"标签\",2,\"tag\"] 编号,网址\n1,tag=茶\n2,tag=%GG"
            ),
            Err(CsvError::DataRowValue { row: 2, .. })
        ));
    }

    #[test]
    fn adds_json_wildcard_arrays_to_csv_without_losing_original_cells() {
        let record = r#"001,"{""订单"":[{""金额"":1.20},{""备注"":""缺价""},{""金额"":null}]}""#;
        let input = format!(
            "JSON通配列 [\"金额数组\",\"详情\",[\"订单\",null,\"金额\"]]\n编号,详情\n{record}\n002,{{}}\n003,"
        );
        assert_eq!(
            transform_csv(&input).as_deref(),
            Ok(format!(
                "编号,详情,金额数组\r\n{record},\"[1.20,null]\"\r\n002,{{}},[]\r\n003,\"\",\"\""
            )
            .as_str())
        );
        assert_eq!(
            transform_csv("JSON通配列 [\"金额数组\",1,[null]]\n详情\n{}\n[1"),
            Err(CsvError::DataRowValue {
                row: 2,
                cause: JsonError::Incomplete.message(),
            })
        );
        assert_eq!(
            transform_csv("JSON通配列 [\"金额\",1,[true]]\n详情"),
            Err(CsvError::JsonWildcardColumnArguments)
        );
    }

    #[test]
    fn extracts_json_pointer_cells_and_preserves_exact_values_and_blank_rows() {
        let query = r#"JSON取值列 ["结果","数据","/a~1b"]
行,数据
1,"{""a/b"":""茶,水""}"
2,"{""a/b"":9007199254740993.00}"
3,"{""a/b"":null}"
4,{}
5,
6,"{""a/b"":[1, 2]}""#;
        let expected = [
            r#"行,数据,结果"#,
            r#"1,"{""a/b"":""茶,水""}","茶,水""#,
            r#"2,"{""a/b"":9007199254740993.00}",9007199254740993.00"#,
            r#"3,"{""a/b"":null}","""#,
            r#"4,{},"""#,
            r#"5,"","""#,
            r#"6,"{""a/b"":[1, 2]}","[1,2]""#,
        ]
        .join("\r\n");
        assert_eq!(transform_csv(query).as_deref(), Ok(expected.as_str()));
        assert_eq!(
            transform_csv("json-pointer-column [\"值\",1,\"\"] 原列\nnull\nfalse").as_deref(),
            Ok("原列,值\r\nnull,\"\"\r\nfalse,false")
        );
        assert_eq!(
            transform_csv("JSON取值列 [\"值\",1,\"/x~\"] 原列"),
            Err(CsvError::JsonPointerColumn(JsonError::InvalidPointer))
        );
        assert_eq!(
            transform_csv("JSON取值列 [\"值\",1,\"/x\"] 原列\n{}\n[1"),
            Err(CsvError::DataRowValue {
                row: 2,
                cause: JsonError::Incomplete.message()
            })
        );
    }

    #[test]
    fn replaces_regex_matches_in_each_csv_cell_and_expands_capture_groups() {
        let query = r#"正则替换列 ["日期","原值","(?<y>[0-9]{4})/([0-9]{2})/([0-9]{2})","${y}-$2-$3"]
编号,原值
1,2026/11/21
2,待确认
3,"2024/01/02,2025/03/04""#;
        assert_eq!(
            transform_csv(query).as_deref(),
            Ok(concat!(
                "编号,原值,日期\r\n",
                "1,2026/11/21,2026-11-21\r\n",
                "2,待确认,待确认\r\n",
                "3,\"2024/01/02,2025/03/04\",\"2024-01-02,2025-03-04\"",
            ))
        );
        assert_eq!(
            transform_csv(&format!(
                "regex-replace-column [\"结果\",1,\"x\",\"y\"] 来源\n{}",
                "x".repeat(MAX_REGEX_COLUMN_INPUT_BYTES + 1)
            )),
            Err(CsvError::RegexReplaceColumnInputTooLarge)
        );
    }

    #[test]
    fn counts_nonoverlapping_regex_matches_separately_for_each_csv_cell() {
        let query = "正则计数列 [\"次数\",\"备注\",\"[0-9]+\"]\n编号,备注\n1,A12 B34 C5\n2,无编号\n3,\n4,\"上1\r\n下2\"";
        assert_eq!(
            transform_csv(query).as_deref(),
            Ok("编号,备注,次数\r\n1,A12 B34 C5,3\r\n2,无编号,0\r\n3,\"\",0\r\n4,\"上1\r\n下2\",2")
        );
        assert_eq!(
            transform_csv("regex-count-column [\"次数\",1,\"^|$\"] 备注\n甲乙").as_deref(),
            Ok("备注,次数\r\n甲乙,2")
        );
        assert_eq!(
            transform_csv("正则计数列 [\"次数\",1,\"[\"] 原列"),
            Err(CsvError::Regex(TextError::InvalidRegex))
        );
        assert_eq!(
            transform_csv("正则计数列 [\"原列\",1,\"[0-9]+\"] 原列"),
            Err(CsvError::HeaderNameConflict)
        );
        let csv = format!(
            "正则计数列 [\"次数\",1,\"x\"] 备注\n{}",
            "x".repeat(MAX_REGEX_COLUMN_INPUT_BYTES + 1)
        );
        assert_eq!(
            transform_csv(&csv),
            Err(CsvError::RegexCountColumnInputTooLarge)
        );
    }

    #[test]
    fn pads_left_or_right_by_unicode_code_points_and_preserves_long_cells() {
        let csv = "行,编号\n1,7\n2,12345\n3,\n4,e\u{301}";
        let query = format!("左填充列 [\"新编号\",\"编号\",4,\"0\"]\n{csv}");
        assert_eq!(
            transform_csv(&query).as_deref(),
            Ok("行,编号,新编号\r\n1,7,0007\r\n2,12345,12345\r\n3,\"\",0000\r\n4,e\u{301},00e\u{301}")
        );
        let query = format!("right-pad-column [\"新编号\",2,2,\"界\"]\n{csv}");
        assert_eq!(
            transform_csv(&query).as_deref(),
            Ok("行,编号,新编号\r\n1,7,7界\r\n2,12345,12345\r\n3,\"\",界界\r\n4,e\u{301},e\u{301}")
        );
        for (width, fill) in [(4097, "0"), (2, ""), (2, "ab"), (2, r"\u0000")] {
            let query = format!("左填充列 [\"新列\",1,{width},\"{fill}\"] 原列");
            assert_eq!(transform_csv(&query), Err(CsvError::TextPadColumnArguments));
        }
        let rows = "\nx".repeat(300);
        let query = format!("左填充列 [\"大列\",1,4096,\"界\"] 原列{rows}");
        assert_eq!(transform_csv(&query), Err(CsvError::AddedColumnTooLarge));
    }

    #[test]
    fn appends_dates_after_applying_each_records_signed_calendar_day_offset() {
        let query = "日期加天列 [\"到期日期\",\"日期\",2]\n日期,天数\n2024-02-28,2\n2024-W09-5,-1\n2024年3月1日,2/2\n2024-02-27,";
        assert_eq!(
            transform_csv(query).as_deref(),
            Ok("日期,天数,到期日期\r\n2024-02-28,2,2024-03-01\r\n2024-W09-5,-1,2024-02-29\r\n2024年3月1日,2/2,2024-03-02\r\n2024-02-27,\"\",\"\"")
        );
        for (date, offset) in [
            ("2024-02-30", "1"),
            ("2024-02-28", "1/2"),
            ("2024-02-28", "-9223372036854775808"),
            ("9999-12-31", "1"),
        ] {
            let query = format!("日期加天列 [\"结果\",1,2]\n日期,天数\n{date},{offset}");
            assert!(matches!(
                transform_csv(&query),
                Err(CsvError::DataRowValue { row: 1, .. })
            ));
        }
    }

    #[test]
    fn appends_signed_calendar_day_differences_and_preserves_blank_cells() {
        let query = "日期差列 [\"间隔天数\",\"开始\",2]\n开始,结束\n2024-02-28,2024-03-01\n2024-03-03,2024年3月1日\n2024-W09-3,2024-W09-5\n2024-03-01,";
        assert_eq!(
            transform_csv(query).as_deref(),
            Ok("开始,结束,间隔天数\r\n2024-02-28,2024-03-01,2\r\n2024-03-03,2024年3月1日,-2\r\n2024-W09-3,2024-W09-5,2\r\n2024-03-01,\"\",\"\"")
        );
        for invalid in ["2024-02-30", "2024-03-01 12:00", "今天"] {
            let query = format!(
                "日期差列 [\"天数\",1,2]\n开始,结束\n2024-02-28,2024-03-01\n{invalid},2024-03-01"
            );
            assert!(matches!(
                transform_csv(&query),
                Err(CsvError::DataRowValue { row: 2, .. })
            ));
        }
    }

    #[test]
    fn selects_the_complete_row_at_each_groups_first_earliest_or_latest_timestamp() {
        let data = concat!(
            "设备,时刻,标签\n",
            "A,2026-03-08T03:30:00-04:00,较晚\n",
            "B,1ns,首次\n",
            "C,,无时刻\n",
            "A,2026-03-08T01:30:00-05:00,较早\n",
            "B,1970-01-01T00:00:00.000000001Z,同一时刻\n",
            "B,2ns,较晚",
        );
        let earliest = format!("分组最早时间行 [[\"设备\"],\"时刻\"] {data}");
        assert_eq!(
            transform_csv(&earliest).as_deref(),
            Ok("设备,时刻,标签\r\nA,2026-03-08T01:30:00-05:00,较早\r\nB,1ns,首次")
        );
        let latest = format!("GROUP-LATEST-TIMESTAMP-ROW [[1],2] {data}");
        let (csv, _) = csv_table_preview(&latest)
            .expect("时间边界行应可预览")
            .unwrap();
        assert_eq!(
            csv,
            "设备,时刻,标签\r\nA,2026-03-08T03:30:00-04:00,较晚\r\nB,2ns,较晚"
        );
        assert_eq!(
            transform_csv("分组最晚时间行 [[],2] 设备,时刻\nA,").as_deref(),
            Ok("设备,时刻")
        );
    }

    #[test]
    fn summarizes_original_timestamp_endpoints_and_exact_utc_spans_per_group() {
        let query = "分组时间戳摘要 [[\"设备\"],\"时刻\"] 设备,时刻\nA,2026-03-08T03:30:00-04:00\nB,2ns\nC,\nA,2026-03-08T01:30:00-05:00\nA,2026-03-08T06:30:00Z\nB,1ns";
        let expected = concat!(
            "设备,有效时间戳,最早,最晚,跨度\r\n",
            "A,3,2026-03-08T01:30:00-05:00,2026-03-08T03:30:00-04:00,01:00:00\r\n",
            "B,2,1ns,2ns,00:00:00.000000001\r\n",
            "C,0,\"\",\"\",\"\"",
        );
        assert_eq!(transform_csv(query).as_deref(), Ok(expected));
        let (csv, _) = csv_table_preview(query)
            .expect("分组时间戳摘要可预览")
            .unwrap();
        assert_eq!(csv, expected);
        assert_eq!(
            transform_csv("GROUP-TIMESTAMP-SUMMARY [[],1] 时刻\n1ns").as_deref(),
            Ok("有效时间戳,最早,最晚,跨度\r\n1,1ns,1ns,00:00:00"),
        );
        assert!(matches!(
            transform_csv(
                "分组时间戳摘要 [[1],2] 设备,时刻\nA,1ns\nA,2026-11-01 01:30 America/New_York"
            ),
            Err(CsvError::DataRowValue { row: 2, .. }),
        ));
    }

    #[test]
    fn appends_signed_elapsed_times_since_each_groups_previous_valid_timestamp() {
        let query = "组内时间戳差列 [\"间隔\",\"时刻\",[\"设备\"]] 设备,时刻\nA,2026-03-08T01:30:00-05:00\nB,2ns\nA,\nA,2026-03-08T03:30:00-04:00\nB,1ns";
        let expected = concat!(
            "设备,时刻,间隔\r\n",
            "A,2026-03-08T01:30:00-05:00,\"\"\r\n",
            "B,2ns,\"\"\r\n",
            "A,\"\",\"\"\r\n",
            "A,2026-03-08T03:30:00-04:00,01:00:00\r\n",
            "B,1ns,-00:00:00.000000001",
        );
        assert_eq!(transform_csv(query).as_deref(), Ok(expected));
        let (csv, _) = csv_table_preview(query).expect("时长列应可预览").unwrap();
        assert_eq!(csv, expected);
        assert_eq!(
            transform_csv(
                "GROUP-TIMESTAMP-DIFFERENCE-COLUMN [\"间隔\",2,[]] 设备,时刻\nA,1s\nB,1s\nC,4s"
            )
            .as_deref(),
            Ok("设备,时刻,间隔\r\nA,1s,\"\"\r\nB,1s,00:00:00\r\nC,4s,00:00:03"),
        );
        assert!(matches!(
            transform_csv("组内时间戳差列 [\"间隔\",2,[1]] 设备,时刻\nA,1s\nB,\nA,2026-11-01 01:30 America/New_York"),
            Err(CsvError::DataRowValue { row: 3, .. }),
        ));
    }

    #[test]
    fn counts_each_groups_timestamp_window_with_inclusive_boundaries_and_ties() {
        let query = concat!(
            "时间窗口计数列 [\"近1ns\",\"时刻\",[\"设备\"],\"1ns\"] 设备,时刻\n",
            "A,4ns\nA,2ns\nA,3ns\nB,3ns\nA,3ns\nA,\nB,2ns",
        );
        let expected = concat!(
            "设备,时刻,近1ns\r\nA,4ns,3\r\nA,2ns,1\r\nA,3ns,3\r\nB,3ns,2\r\n",
            "A,3ns,3\r\nA,\"\",\"\"\r\nB,2ns,1",
        );
        assert_eq!(transform_csv(query).as_deref(), Ok(expected));
        let (text, table) = csv_table_preview(query).unwrap().unwrap();
        assert_eq!(text, expected);
        assert_eq!(table.row_pages(), 2);
        assert_eq!(
            transform_csv(concat!(
                "timestamp-window-count-column [\"次数\",1,[],\"0ns\"] 时刻\n",
                "0s\n1970-01-01T08:00:00+08:00\n1ns",
            ))
            .as_deref(),
            Ok("时刻,次数\r\n0s,2\r\n1970-01-01T08:00:00+08:00,2\r\n1ns,1")
        );
        assert!(matches!(
            transform_csv("时间窗口计数列 [\"次数\",1,[],\"1ns\"] 时刻\n0s\n坏值"),
            Err(CsvError::DataRowValue { row: 2, .. }),
        ));
    }

    #[test]
    fn aggregates_exact_values_in_timestamp_windows_without_reordering_rows() {
        let data =
            "设备,时刻,金额\nA,3ns,\nB,4ns,\nA,1ns,1/3\nA,3ns,0.5\nA,4ns,-0.50\nA,8ns,\nB,,坏值";
        let columns = r#"["窗口","时刻","金额",["设备"],"2ns"]"#;
        for (command, expected) in [
            (
                "时间窗口合计列",
                "设备,时刻,金额,窗口\r\nA,3ns,\"\",5/6\r\nB,4ns,\"\",\"\"\r\nA,1ns,1/3,1/3\r\nA,3ns,0.5,5/6\r\nA,4ns,-0.50,0\r\nA,8ns,\"\",\"\"\r\nB,\"\",坏值,\"\"",
            ),
            (
                "时间窗口平均列",
                "设备,时刻,金额,窗口\r\nA,3ns,\"\",5/12\r\nB,4ns,\"\",\"\"\r\nA,1ns,1/3,1/3\r\nA,3ns,0.5,5/12\r\nA,4ns,-0.50,0\r\nA,8ns,\"\",\"\"\r\nB,\"\",坏值,\"\"",
            ),
        ] {
            let query = format!("{command} {columns} {data}");
            assert_eq!(transform_csv(&query).as_deref(), Ok(expected));
            let (text, table) = csv_table_preview(&query).unwrap().unwrap();
            assert_eq!(text, expected);
            assert_eq!(table.row_pages(), 2);
        }
        assert_eq!(
            transform_csv("timestamp-window-sum-column [\"窗口\",1,2,[],\"0ns\"] 时刻,金额\n0s,9007199254740993\n1970-01-01T08:00:00+08:00,1\n1ns,-1").as_deref(),
            Ok("时刻,金额,窗口\r\n0s,9007199254740993,9007199254740994\r\n1970-01-01T08:00:00+08:00,1,9007199254740994\r\n1ns,-1,-1")
        );
        assert!(matches!(
            transform_csv("时间窗口平均列 [\"平均\",1,2,[],\"1ns\"] 时刻,金额\n0s,3\n1ns,坏值"),
            Err(CsvError::DataRowValue { row: 2, .. }),
        ));
    }

    #[test]
    fn numbers_timestamp_sessions_by_each_groups_actual_gap_and_row_order() {
        let query = "会话编号列 [\"会话\",\"时刻\",[\"设备\"],\"1ns\"] 设备,时刻\nA,1ns\nB,2ns\nA,2ns\nA,4ns\nA,\nA,3ns\nB,3ns";
        let expected = concat!(
            "设备,时刻,会话\r\n",
            "A,1ns,1\r\n",
            "B,2ns,1\r\n",
            "A,2ns,1\r\n",
            "A,4ns,2\r\n",
            "A,\"\",\"\"\r\n",
            "A,3ns,3\r\n",
            "B,3ns,1",
        );
        assert_eq!(transform_csv(query).as_deref(), Ok(expected));
        let (csv, _) = csv_table_preview(query)
            .expect("会话编号列应可预览")
            .unwrap();
        assert_eq!(csv, expected);
        assert_eq!(
            transform_csv("会话编号列 [\"会话\",1,[],\"-1ns\"] 时刻\n1ns"),
            Err(CsvError::GroupTimestampSessionArguments),
        );
    }

    #[test]
    fn appends_signed_elapsed_times_since_each_groups_first_valid_timestamp() {
        let query = "组内时间戳偏移列 [\"偏移\",\"时刻\",[\"设备\"]] 设备,时刻\nA,3s\nB,10ns\nA,8s\nA,\nB,5ns\nA,1s";
        let expected = concat!(
            "设备,时刻,偏移\r\n",
            "A,3s,00:00:00\r\n",
            "B,10ns,00:00:00\r\n",
            "A,8s,00:00:05\r\n",
            "A,\"\",\"\"\r\n",
            "B,5ns,-00:00:00.000000005\r\n",
            "A,1s,-00:00:02",
        );
        assert_eq!(transform_csv(query).as_deref(), Ok(expected));
        let (csv, _) = csv_table_preview(query).expect("时长列应可预览").unwrap();
        assert_eq!(csv, expected);
        assert_eq!(
            transform_csv(
                "GROUP-TIMESTAMP-OFFSET-COLUMN [\"偏移\",2,[]] 设备,时刻\nA,\nB,3s\nC,1s\nA,8s"
            )
            .as_deref(),
            Ok("设备,时刻,偏移\r\nA,\"\",\"\"\r\nB,3s,00:00:00\r\nC,1s,-00:00:02\r\nA,8s,00:00:05"),
        );
    }

    #[test]
    fn appends_a_unit_conversion_column_with_affine_and_linear_units() {
        let query = "单位换算列 [\"摄氏\",\"温度\",\"°C\"]\n编号,温度\n001,32 °F\n002,212 °F\n003,";
        let expected = "编号,温度,摄氏\r\n001,32 °F,0\r\n002,212 °F,100\r\n003,\"\",\"\"";
        assert_eq!(transform_csv(query).as_deref(), Ok(expected));
        let (text, preview) = csv_table_preview(query).unwrap().unwrap();
        assert_eq!(text, expected);
        assert_eq!(preview.page_csv(0, 0), expected);
        assert_eq!(
            transform_csv("convert-unit-column [\"cm\",1,\"cm\"] 长度\n1 in\n0.5 m").as_deref(),
            Ok("长度,cm\r\n1 in,2.54\r\n0.5 m,50"),
        );
        assert_eq!(
            transform_csv("单位换算列 [\"cm\",1,\"cm\"] 长度\n1 m\n1 kg"),
            Err(CsvError::DataRowValue {
                row: 2,
                cause: ConversionError::DifferentDimensions.message(),
            }),
        );
    }

    #[test]
    fn converts_numeric_csv_cells_from_a_fixed_source_unit() {
        let query =
            "单位换算列 [\"厘米\",\"长度\",\"m\",\"cm\"]\n编号,长度\n001,1.5\n002,1/4\n003,";
        assert_eq!(
            transform_csv(query).as_deref(),
            Ok("编号,长度,厘米\r\n001,1.5,150\r\n002,1/4,25\r\n003,\"\",\"\""),
        );
        assert_eq!(
            transform_csv("单位换算列 [\"摄氏\",1,\"°F\",\"°C\"] 温度\n32\n212").as_deref(),
            Ok("温度,摄氏\r\n32,0\r\n212,100"),
        );
        assert_eq!(
            transform_csv("单位换算列 [\"厘米\",1,\"kg\",\"cm\"] 长度"),
            Err(CsvError::UnitConversionTarget(
                ConversionError::DifferentDimensions
            )),
        );
        assert_eq!(
            transform_csv("单位换算列 [\"厘米\",1,\"m\",\"cm\"] 长度\n2\n1 m"),
            Err(CsvError::DataRowValue {
                row: 2,
                cause: ConversionError::InvalidAmount.message(),
            }),
        );
        assert_eq!(
            transform_csv("数据换算列 [\"二进制容量\",1,\"B\",\"KiB\"] 容量\n1024"),
            Err(CsvError::UnitConversionColumnArguments),
        );
    }

    #[test]
    fn appends_an_exact_data_conversion_column_with_mixed_units_and_blanks() {
        let capacities = "数据换算列 [\"KiB\",\"容量\",\"KiB\"]\n编号,容量\n001,9007199254740993 B\n002,1/3 KiB\n003,";
        assert_eq!(
            transform_csv(capacities).as_deref(),
            Ok("编号,容量,KiB\r\n001,9007199254740993 B,9007199254740993/1024\r\n002,1/3 KiB,1/3\r\n003,\"\",\"\"")
        );
        let rates = "数据换算列 [\"MiB/s\",2,\"MiB/s\"]\n编号,速率\n001,8 Mbps\n002,5/2 MiB/s";
        assert_eq!(
            transform_csv(rates).as_deref(),
            Ok("编号,速率,MiB/s\r\n001,8 Mbps,15625/16384\r\n002,5/2 MiB/s,5/2")
        );
        for (query, expected) in [
            (
                "数据换算列 [\"KiB\",2,\"KiB\"]\n编号,容量\n001,1 KiB\n002,4 Mbps",
                CsvError::DataRowValue {
                    row: 2,
                    cause: ConversionError::DifferentDimensions.message(),
                },
            ),
            (
                "数据换算列 [\"长度\",1,\"m\"]\n容量\n1 KiB",
                CsvError::UnitConversionTarget(ConversionError::ExactDataUnit),
            ),
            (
                "数据换算列 [\"容量\",1,\"KiB\"]\n容量\n1 KiB",
                CsvError::HeaderNameConflict,
            ),
        ] {
            assert_eq!(transform_csv(query), Err(expected), "{query}");
        }
    }

    #[test]
    fn computes_grouped_differences_from_the_previous_nonempty_value() {
        let query = "差分列 [\"变化\",\"金额\",[\"部门\"]]\n部门,金额\n北,1/3\n南,2\n北,\n北,0.5\n南,3/2\n北,2";
        assert_eq!(
            transform_csv(query).as_deref(),
            Ok("部门,金额,变化\r\n北,1/3,\"\"\r\n南,2,\"\"\r\n北,\"\",\"\"\r\n北,0.5,1/6\r\n南,3/2,-1/2\r\n北,2,3/2")
        );
        assert!(matches!(
            transform_csv("difference-column [\"差\",1,[]]\n金额\n2\n有误"),
            Err(CsvError::DataRowValue { row: 2, .. })
        ));
    }

    #[test]
    fn appends_grouped_relative_changes_and_reports_the_row_after_a_zero_base() {
        let query = "变化率列 [\"变化率\",\"金额\",[\"部门\"]]\n部门,金额\n北,1/3\n南,2\n北,\n北,0.5\n南,3/2\n北,0";
        assert_eq!(
            transform_csv(query).as_deref(),
            Ok("部门,金额,变化率\r\n北,1/3,\"\"\r\n南,2,\"\"\r\n北,\"\",\"\"\r\n北,0.5,1/2\r\n南,3/2,-1/4\r\n北,0,-1")
        );
        assert_eq!(
            transform_csv("relative-change-column [\"率\",1,[]]\n金额\n0\n\"\"\n2"),
            Err(CsvError::DataRowValue {
                row: 3,
                cause: StatisticsError::ZeroPreviousValue.message(),
            })
        );
    }

    #[test]
    fn appends_grouped_moving_sample_variance_and_population_deviation() {
        let source = "\n部门,金额\n北,1/3\n南,3\n北,\n北,4/3\n南,7\n北,10/3";
        let expected = [
            ("移动样本方差列", "1/2", "8", "2"),
            ("移动总体方差列", "1/4", "4", "1"),
            ("moving-sample-stdev-column", "√(1/2)", "√8", "√2"),
            ("moving-population-stdev-column", "1/2", "2", "1"),
        ];
        for (kind, first, south, second) in expected {
            let input = format!("{kind} [\"结果\",\"金额\",[\"部门\"],2]{source}");
            let output = format!(
                "部门,金额,结果\r\n北,1/3,\"\"\r\n南,3,\"\"\r\n北,\"\",\"\"\r\n北,4/3,{first}\r\n南,7,{south}\r\n北,10/3,{second}"
            );
            assert_eq!(transform_csv(&input), Ok(output.clone()), "{kind}");
            let (text, table) = csv_table_preview(&input).unwrap().unwrap();
            assert_eq!(text, output);
            assert_eq!(table.page_csv(0, 0), output);
        }
        assert_eq!(
            transform_csv("移动样本方差列 [\"结果\",1,[],1] 金额"),
            Err(CsvError::MovingDispersionColumnArguments)
        );
        assert_eq!(
            transform_csv("移动总体方差列 [\"结果\",1,[],1]\n金额\n-1/3").as_deref(),
            Ok("金额,结果\r\n-1/3,0")
        );
    }

    #[test]
    fn appends_grouped_moving_quantiles_without_counting_blank_rows() {
        assert_eq!(
            transform_csv(
                "移动分位数列 [\"P25\",\"金额\",[\"部门\"],2,\"25%\"]\n部门,金额\n北,4\n南,8\n北,1\n北,\n南,2\n北,3",
            ).as_deref(),
            Ok("部门,金额,P25\r\n北,4,\"\"\r\n南,8,\"\"\r\n北,1,7/4\r\n北,\"\",\"\"\r\n南,2,7/2\r\n北,3,3/2"),
        );
        assert_eq!(
            transform_csv("moving-quantile-column [\"P100\",1,[],1,\"100%\"]\n金额\n-2/3")
                .as_deref(),
            Ok("金额,P100\r\n-2/3,-2/3"),
        );
        assert_eq!(
            transform_csv("移动分位数列 [\"P25\",1,[],0,\"25%\"] 金额"),
            Err(CsvError::MovingQuantileColumnArguments),
        );
        assert_eq!(
            transform_csv("移动分位数列 [\"P25\",1,[],2,\"125%\"] 金额"),
            Err(CsvError::Statistics(StatisticsError::InvalidProbability)),
        );
    }

    #[test]
    fn moves_a_bounded_window_per_group_without_counting_blank_rows() {
        let query = "移动平均列 [\"近两笔\",\"金额\",[\"部门\"],2]\n部门,金额\n北,1/3\n南,2\n北,\n北,0.5\n南,3/2\n北,2";
        assert_eq!(
            transform_csv(query).as_deref(),
            Ok("部门,金额,近两笔\r\n北,1/3,\"\"\r\n南,2,\"\"\r\n北,\"\",\"\"\r\n北,0.5,5/12\r\n南,3/2,7/4\r\n北,2,5/4")
        );
        assert_eq!(
            transform_csv("moving-sum-column [\"结果\",1,[],2]\n金额\n1/3\n1/2\n2").as_deref(),
            Ok("金额,结果\r\n1/3,\"\"\r\n1/2,5/6\r\n2,5/2")
        );
        for width in [0, MAX_VALUES + 1] {
            assert_eq!(
                transform_csv(&format!("移动平均列 [\"结果\",1,[],{width}]\n金额\n1")),
                Err(CsvError::MovingColumnArguments)
            );
        }
    }

    #[test]
    fn appends_longest_prefixes_or_names_without_rewriting_the_address_column() {
        let source = "编号,地址\n001,１９２.０.２.１９０\n002,2001:db8::7\n003,192.0.3.1\n004,";
        let networks =
            r#"[["192.0.2.0/24","办公"],["192.0.2.128/25","实验室, Ａ"],["2001:db8::/32","IPv6"]]"#;
        for (operation, expected) in [
            (
                "网段归属列",
                "编号,地址,结果\r\n001,１９２.０.２.１９０,192.0.2.128/25\r\n002,2001:db8::7,2001:db8::/32\r\n003,192.0.3.1,\"\"\r\n004,\"\",\"\"",
            ),
            (
                "cidr-name-column",
                "编号,地址,结果\r\n001,１９２.０.２.１９０,\"实验室, Ａ\"\r\n002,2001:db8::7,IPv6\r\n003,192.0.3.1,\"\"\r\n004,\"\",\"\"",
            ),
        ] {
            let query = format!(r#"{operation} ["结果","地址",{networks}] {source}"#);
            assert_eq!(transform_csv(&query).as_deref(), Ok(expected));
            assert_eq!(csv_table_preview(&query).unwrap().unwrap().0, expected);
        }
        assert!(matches!(
            transform_csv(
                "网段归属列 [\"网段\",2,[\"192.0.2.0/24\"]] 编号,地址\n001,192.0.2.1\n002,非地址"
            ),
            Err(CsvError::DataRowValue { row: 2, .. })
        ));
        assert_eq!(
            transform_csv(r#"网段名称列 ["名称",1,["192.0.2.0/24"]] 地址"#),
            Err(CsvError::CidrColumnArguments)
        );
    }

    #[test]
    fn aligns_csv_numbers_to_exact_step_multiples_and_preserves_the_source_column() {
        let query = "数值对齐列 [\"对齐值\",\"金额\",\"1/4\",\"就近\"] 编号,金额\n001,-3/8\n002,0.625\n003,9007199254740993.125\n004,";
        let expected = concat!(
            "编号,金额,对齐值\r\n",
            "001,-3/8,-1/2\r\n",
            "002,0.625,1/2\r\n",
            "003,9007199254740993.125,9007199254740993\r\n",
            "004,\"\",\"\"",
        );
        assert_eq!(transform_csv(query).as_deref(), Ok(expected));
        assert_eq!(csv_table_preview(query).unwrap().unwrap().0, expected);
        for (direction, result) in [("向下", "-1/2"), ("ceil", "-1/4")] {
            let query =
                format!("numeric-align-column [\"对齐\",1,\"0.25\",\"{direction}\"] 金额\n-3/8");
            assert_eq!(
                transform_csv(&query).unwrap(),
                format!("金额,对齐\r\n-3/8,{result}")
            );
        }
        for query in [
            "数值对齐列 [\"对齐\",1,\"0\",\"就近\"] 金额",
            "数值对齐列 [\"对齐\",1,\"-1/4\",\"就近\"] 金额",
            "数值对齐列 [\"对齐\",1,\"1/4\",\"随机\"] 金额",
        ] {
            assert_eq!(
                transform_csv(query),
                Err(CsvError::NumericAlignmentColumnArguments)
            );
        }
        assert!(matches!(
            transform_csv("数值对齐列 [\"对齐\",1,\"1/4\",\"向上\"] 金额\n1/2\n坏值"),
            Err(CsvError::DataRowValue { row: 2, .. })
        ));
    }

    #[test]
    fn aligns_each_timestamp_to_utc_boundaries_with_its_original_row() {
        let query = "时间戳对齐列 [\"区间开始\",\"时刻\",\"15分钟\",\"向下\"] 编号,时刻\nA,-1s\nB,1970-01-01T08:17:30+08:00\nC,900s\nD,";
        let expected = concat!(
            "编号,时刻,区间开始\r\n",
            "A,-1s,1969-12-31T23:45:00Z\r\n",
            "B,1970-01-01T08:17:30+08:00,1970-01-01T00:15:00Z\r\n",
            "C,900s,1970-01-01T00:15:00Z\r\n",
            "D,\"\",\"\"",
        );
        assert_eq!(transform_csv(query).as_deref(), Ok(expected));
        let (preview, _) = csv_table_preview(query)
            .expect("时间戳对齐列应可预览")
            .unwrap();
        assert_eq!(preview, expected);
        assert_eq!(
            transform_csv("TIMESTAMP-ALIGN-COLUMN [\"边界\",1,\"2ns\",\"nearest\"] 时刻\n-5ns")
                .as_deref(),
            Ok("时刻,边界\r\n-5ns,1969-12-31T23:59:59.999999996Z")
        );
        assert_eq!(
            transform_csv("时间戳对齐列 [\"边界\",1,\"0秒\",\"向上\"] 时刻"),
            Err(CsvError::TimestampAlignmentColumnArguments)
        );
        assert!(matches!(
            transform_csv("时间戳对齐列 [\"边界\",1,\"1s\",\"向上\"] 时刻\n1ns\n无效"),
            Err(CsvError::DataRowValue { row: 2, .. })
        ));
    }

    #[test]
    fn appends_timestamp_conversions_in_the_target_zone_and_reports_the_source_row() {
        let input = "时区换算列 [\"UTC时间\",\"时间\",\"UTC\"] 编号,时间\nA,1970-01-01T08:00:00+08:00\nB,-1ns\nC,";
        assert_eq!(
            transform_csv(input).as_deref(),
            Ok("编号,时间,UTC时间\r\nA,1970-01-01T08:00:00+08:00,1970-01-01T00:00:00Z\r\nB,-1ns,1969-12-31T23:59:59.999999999Z\r\nC,\"\",\"\""),
        );
        assert!(matches!(
            transform_csv(
                "时区换算列 [\"UTC时间\",1,\"UTC\"] 时间\n0\n2026-03-08 02:30 America/New_York"
            ),
            Err(CsvError::DataRowValue { row: 2, .. }),
        ));
        assert_eq!(
            transform_csv(r#"时区换算列 ["标准",1,"Invalid/Zone"] 时间"#),
            Err(CsvError::TimestampZoneColumnArguments),
        );
    }

    #[test]
    fn appends_signed_elapsed_timestamps_with_real_offsets_and_nanoseconds() {
        let query = "时间戳差列 [\"时长\",\"开始\",2] 开始,结束\n2026-03-08 01:30 America/New_York,2026-03-08 03:30 America/New_York\n1ns,-1ns\n0,";
        assert_eq!(
            transform_csv(query).as_deref(),
            Ok("开始,结束,时长\r\n2026-03-08 01:30 America/New_York,2026-03-08 03:30 America/New_York,01:00:00\r\n1ns,-1ns,-00:00:00.000000002\r\n0,\"\",\"\""),
        );
        assert!(matches!(
            transform_csv(
                "时间戳差列 [\"时长\",1,2] 开始,结束\n0,1\n0,2026-11-01 01:30 America/New_York"
            ),
            Err(CsvError::DataRowValue { row: 2, .. }),
        ));
    }

    #[test]
    fn appends_clock_differences_across_midnight_and_reports_invalid_times() {
        let query = "时刻差列 [\"经过时长\",\"开始\",\"结束\"]\n开始,结束\n23:45,01:15\n０９：００：００．２５０,09:00:01\n12:00,12:00\n09:00,";
        assert_eq!(
            transform_csv(query).as_deref(),
            Ok("开始,结束,经过时长\r\n23:45,01:15,01:30:00\r\n０９：００：００．２５０,09:00:01,00:00:00.75\r\n12:00,12:00,00:00:00\r\n09:00,\"\",\"\"")
        );
        assert!(matches!(
            transform_csv("clock-difference-column [\"时长\",1,2]\n开始,结束\n09:00,24:00"),
            Err(CsvError::DataRowValue { row: 1, .. })
        ));
    }

    #[test]
    fn summarizes_one_based_boundaries_of_consecutive_csv_records() {
        let query = "连续段摘要 [\"设备\",2] 设备,状态,记录\nA,开,\"甲\n乙\"\nA,开,丙\nB,开,丁\nA,开,戊\nA,,末\nA,,落";
        let expected = "段号,设备,状态,起始数据行,末尾数据行,记录数\r\n1,A,开,1,2,2\r\n2,B,开,3,3,1\r\n3,A,开,4,4,1\r\n4,A,\"\",5,6,2";
        assert_eq!(transform_csv(query).as_deref(), Ok(expected));
        let (csv, _) = csv_table_preview(query).expect("连续段摘要可预览").unwrap();
        assert_eq!(csv, expected);
        assert_eq!(
            transform_csv("RUN-SUMMARY [] 编号\n001\n002").as_deref(),
            Ok("段号,起始数据行,末尾数据行,记录数\r\n1,1,2,2"),
        );
        assert_eq!(
            transform_csv("连续段摘要 [1] 部门").as_deref(),
            Ok("段号,部门,起始数据行,末尾数据行,记录数"),
        );
    }

    #[test]
    fn groups_consecutive_rows_and_restarts_positions_when_the_key_changes() {
        let csv = "部门,月份,编号\n北,11,001\n北,11,002\n南,11,003\n北,11,004\n北,12,005\n,12,006\n,12,007";
        assert_eq!(
            transform_csv(&format!("连续分组列 [\"分组\",[\"部门\",2]]\n{csv}")).as_deref(),
            Ok("部门,月份,编号,分组\r\n北,11,001,1\r\n北,11,002,1\r\n南,11,003,2\r\n北,11,004,3\r\n北,12,005,4\r\n\"\",12,006,5\r\n\"\",12,007,5"),
        );
        assert_eq!(
            transform_csv(&format!("段内序号列 [\"位置\",[\"部门\",2]]\n{csv}")).as_deref(),
            Ok("部门,月份,编号,位置\r\n北,11,001,1\r\n北,11,002,2\r\n南,11,003,1\r\n北,11,004,1\r\n北,12,005,1\r\n\"\",12,006,1\r\n\"\",12,007,2"),
        );
        assert_eq!(
            transform_csv("run-row-number-column [\"位置\",[]] 地区\n北\n南").as_deref(),
            Ok("地区,位置\r\n北,1\r\n南,2"),
        );
        let query = format!("段内计数列 [\"段长\",[\"部门\",2]]\n{csv}");
        let expected = "部门,月份,编号,段长\r\n北,11,001,2\r\n北,11,002,2\r\n南,11,003,1\r\n北,11,004,1\r\n北,12,005,1\r\n\"\",12,006,2\r\n\"\",12,007,2";
        assert_eq!(transform_csv(&query).as_deref(), Ok(expected));
        let (csv, _) = csv_table_preview(&query)
            .expect("段内计数列可预览")
            .unwrap();
        assert_eq!(csv, expected);
        assert_eq!(
            transform_csv("RUN-LENGTH-COLUMN [\"段长\",[]] 部门\n北\n南").as_deref(),
            Ok("部门,段长\r\n北,2\r\n南,2"),
        );
        assert_eq!(
            transform_csv("run-id-column [\"地区\",[]] 地区"),
            Err(CsvError::HeaderNameConflict),
        );
    }

    #[test]
    fn appends_total_or_running_counts_for_each_group_in_original_row_order() {
        let csv = "部门,月份,编号\n北,11,001\n南,11,002\n北,12,003\n北,11,004\n,11,005\n,11,006";
        assert_eq!(
            transform_csv(&format!("组内计数列 [\"记录数\",[\"部门\",2]]\n{csv}")).as_deref(),
            Ok("部门,月份,编号,记录数\r\n北,11,001,2\r\n南,11,002,1\r\n北,12,003,1\r\n北,11,004,2\r\n\"\",11,005,2\r\n\"\",11,006,2")
        );
        assert_eq!(
            transform_csv(&format!("组内序号列 [\"序号\",[\"部门\",2]]\n{csv}")).as_deref(),
            Ok("部门,月份,编号,序号\r\n北,11,001,1\r\n南,11,002,1\r\n北,12,003,1\r\n北,11,004,2\r\n\"\",11,005,1\r\n\"\",11,006,2")
        );
        assert_eq!(
            transform_csv("group-count-column [\"记录数\",[]] 地区\n北\n南").as_deref(),
            Ok("地区,记录数\r\n北,2\r\n南,2")
        );
        assert_eq!(
            transform_csv("group-row-number-column [\"序号\",[\"地区\",1]] 地区\n北"),
            Err(CsvError::GroupCountColumnArguments)
        );
        assert_eq!(
            transform_csv("group-count-column [\"地区\",[]] 地区\n北"),
            Err(CsvError::HeaderNameConflict)
        );
    }

    #[test]
    fn appends_exact_shares_of_each_group_and_reports_zero_totals() {
        let query =
            "占比列 [\"占比\",\"金额\",[\"部门\"]]\n部门,金额\n北,1/3\n南,2\n北,2/3\n南,6\n北,";
        assert_eq!(transform_csv(query).as_deref(), Ok("部门,金额,占比\r\n北,1/3,1/3\r\n南,2,1/4\r\n北,2/3,2/3\r\n南,6,3/4\r\n北,\"\",\"\""));
        assert_eq!(
            transform_csv("share-column [\"占比\",1,[]]\n金额\n1\n-1"),
            Err(CsvError::DataRowValue {
                row: 1,
                cause: "同组数值合计为零"
            })
        );
        assert_eq!(
            transform_csv("占比列 [\"占比\",1,[]]\n金额\n\"\"").as_deref(),
            Ok("金额,占比\r\n\"\",\"\"")
        );
    }

    #[test]
    fn dense_ranks_keep_equal_exact_values_tied_and_do_not_skip_numbers() {
        let query =
            "密集排名列 [\"名次\",\"金额\",[\"部门\"]]\n部门,金额\n北,1/2\n南,2\n北,3/2\n北,0.50\n南,1\n北,0\n北,";
        assert_eq!(
            transform_csv(query).as_deref(),
            Ok("部门,金额,名次\r\n北,1/2,2\r\n南,2,1\r\n北,3/2,1\r\n北,0.50,2\r\n南,1,2\r\n北,0,3\r\n北,\"\",\"\"")
        );
        assert_eq!(
            transform_csv("ascending-dense-rank-column [\"名次\",1,[]]\n金额\n1/2\n0.50\n3/2")
                .as_deref(),
            Ok("金额,名次\r\n1/2,1\r\n0.50,1\r\n3/2,2")
        );
    }

    #[test]
    fn appends_grouped_competition_ranks_without_reordering_records() {
        let query = "排名列 [\"名次\",\"金额\",[\"部门\"]]\n部门,金额\n北,1/2\n南,2\n北,3/2\n北,0.50\n南,1\n北,0\n北,";
        assert_eq!(
            transform_csv(query).as_deref(),
            Ok("部门,金额,名次\r\n北,1/2,2\r\n南,2,1\r\n北,3/2,1\r\n北,0.50,2\r\n南,1,2\r\n北,0,4\r\n北,\"\",\"\"")
        );
        assert_eq!(
            transform_csv("升序排名列 [\"名次\",1,[]]\n金额\n1/2\n0.50\n3/2").as_deref(),
            Ok("金额,名次\r\n1/2,1\r\n0.50,1\r\n3/2,3")
        );
        assert!(matches!(
            transform_csv("rank-column [\"名次\",1,[]]\n金额\n1\n坏值"),
            Err(CsvError::DataRowValue { row: 2, .. })
        ));
    }

    #[test]
    fn reports_exact_percent_ranks_and_cumulative_distribution_with_ties() {
        let data = "部门,金额\n北,1/2\n南,2\n北,3/2\n北,0.50\n北,0\n北,";
        for (command, expected) in [
            (
                "百分位排名列",
                "部门,金额,位置\r\n北,1/2,1/3\r\n南,2,0\r\n北,3/2,1\r\n北,0.50,1/3\r\n北,0,0\r\n北,\"\",\"\"",
            ),
            (
                "累积分布列",
                "部门,金额,位置\r\n北,1/2,3/4\r\n南,2,1\r\n北,3/2,1\r\n北,0.50,3/4\r\n北,0,1/4\r\n北,\"\",\"\"",
            ),
            (
                "descending-percent-rank-column",
                "部门,金额,位置\r\n北,1/2,1/3\r\n南,2,0\r\n北,3/2,0\r\n北,0.50,1/3\r\n北,0,1\r\n北,\"\",\"\"",
            ),
            (
                "降序累积分布列",
                "部门,金额,位置\r\n北,1/2,3/4\r\n南,2,1\r\n北,3/2,1/4\r\n北,0.50,3/4\r\n北,0,1\r\n北,\"\",\"\"",
            ),
        ] {
            let query = format!("{command} [\"位置\",2,[\"部门\"]]\n{data}");
            assert_eq!(transform_csv(&query).as_deref(), Ok(expected));
            let (text, table) = csv_table_preview(&query).unwrap().unwrap();
            assert_eq!(text, expected);
            assert_eq!(table.page_csv(0, 0), expected);
        }
        assert_eq!(
            transform_csv("cume-dist-column [\"位置\",1,[]]\n金额").as_deref(),
            Ok("金额,位置")
        );
        assert!(matches!(
            transform_csv("percent-rank-column [\"位置\",1,[]]\n金额\n坏值"),
            Err(CsvError::DataRowValue { row: 1, .. })
        ));
    }

    #[test]
    fn appends_grouped_running_sums_and_averages_in_original_csv_order() {
        let data = "部门,金额\n北,1/3\n南,2\n北,0.5\n北,\n南,-1/2\n西,";
        for (operation, expected) in [
            (
                "累计合计列",
                "部门,金额,累计\r\n北,1/3,1/3\r\n南,2,2\r\n北,0.5,5/6\r\n北,\"\",5/6\r\n南,-1/2,3/2\r\n西,\"\",0",
            ),
            (
                "累计平均列",
                "部门,金额,累计\r\n北,1/3,1/3\r\n南,2,2\r\n北,0.5,5/12\r\n北,\"\",5/12\r\n南,-1/2,3/4\r\n西,\"\",\"\"",
            ),
        ] {
            let query = format!("{operation} [\"累计\",2,[\"部门\"]]\n{data}");
            assert_eq!(transform_csv(&query).as_deref(), Ok(expected));
        }
        assert_eq!(
            transform_csv("累计合计列 [\"累计\",1,[]]\n金额\n1/3\n2/3").as_deref(),
            Ok("金额,累计\r\n1/3,1/3\r\n2/3,1")
        );
        assert!(matches!(
            transform_csv("累计合计列 [\"累计\",1,[]]\n金额\n1\n坏值"),
            Err(CsvError::DataRowValue { row: 2, .. })
        ));
    }

    #[test]
    fn reports_differing_values_from_two_columns_of_a_single_table() {
        let text =
            "两列差异 [\"原备注\",3] 编号,原备注,新备注\n001,相同,相同\n002,旧,\"新,值\"\n003,,茶";
        assert_eq!(
            transform_csv(text).as_deref(),
            Ok("数据行,原值,新值\r\n2,旧,\"新,值\"\r\n3,\"\",茶")
        );
        let exact = "两列数值差异 [1,\"新值\"] 原值,新值\n1.50,3/2\n,0\n2,3";
        assert_eq!(
            transform_csv(exact).as_deref(),
            Ok("数据行,原值,新值\r\n2,\"\",0\r\n3,2,3")
        );
        assert!(matches!(
            transform_csv("两列数值差异 [1,2] 原值,新值\n1,1\n坏值,2"),
            Err(CsvError::DataRowValue { row: 2, .. })
        ));
        assert_eq!(
            transform_csv("两列差异 [1,\"原值\"] 原值,新值\n相同,相同"),
            Err(CsvError::ColumnPairDifferenceArguments)
        );
    }

    #[test]
    fn appends_an_exact_numeric_comparison_in_each_csv_record() {
        let input = concat!(
            "数值比较列 [\"达标\",\"实测\",2,\">=\"] 实测,门槛\n",
            "1.50,3/2\n1,2\n9007199254740993,9007199254740992\n,0",
        );
        assert_eq!(
            transform_csv(input).as_deref(),
            Ok("实测,门槛,达标\r\n1.50,3/2,true\r\n1,2,false\r\n9007199254740993,9007199254740992,true\r\n\"\",0,\"\"")
        );
        assert_eq!(
            transform_csv(
                r#"numeric-comparison-column ["结果",1,"乙","≠"] 甲,乙
0.00,0"#
            )
            .as_deref(),
            Ok("甲,乙,结果\r\n0.00,0,false")
        );
        assert!(matches!(
            transform_csv("数值比较列 [\"达标\",1,2,\">=\"] 实测,门槛\n3,2\n坏值,2"),
            Err(CsvError::DataRowValue { row: 2, .. })
        ));
        assert_eq!(
            transform_csv("数值比较列 [\"达标\",1,2,\"=>\"] 实测,门槛\n3,2"),
            Err(CsvError::NumericComparisonColumnArguments)
        );
    }

    #[test]
    fn summarizes_function_values_by_existing_group_keys_in_source_order() {
        let input = "函数分组摘要 [[1,\"月份\"],[3,4],\"x*y\"] 部门,月份,x,y\n\"西,区\",11,2,3\n东,11,-1,1\n\"西,区\",11,4,2\n\"西,区\",12,,\n东,11,,";
        assert_eq!(
            transform_csv(input).as_deref(),
            Ok("部门,月份,数据行,有效,空值,最小值,最大值,平均值,中位数\r\n\"西,区\",11,2,2,0,6,8,7,7\r\n东,11,2,1,1,-1,-1,-1,-1\r\n\"西,区\",12,1,0,1,\"\",\"\",\"\",\"\"")
        );
        assert_eq!(
            transform_csv("group-function-summary [[1],[2],\"sqrt(x)\"] 组,数值").as_deref(),
            Ok("组,数据行,有效,空值,最小值,最大值,平均值,中位数")
        );
        assert_eq!(
            transform_csv("函数分组摘要 [[1,\"组\"],[2],\"x\"] 组,数值\n甲,1"),
            Err(CsvError::FunctionGroupSummaryArguments)
        );
    }

    #[test]
    fn summarizes_finite_function_values_and_counts_the_blank_or_undefined_rows() {
        let header = "数据行,有效,空值,最小值,最大值,平均值,中位数";
        assert_eq!(
            transform_csv("函数摘要 [[\"角度\",2],\"cosd(x)*y\"] 角度,权重\n0,2\n180,3\n90,4\n,2")
                .unwrap(),
            format!("{header}\r\n4,3,1,-3,2,-0.333333333333333,0")
        );
        assert_eq!(
            transform_csv("summary-function [[1],\"sqrt(x)\"] 数值\n-1").unwrap(),
            format!("{header}\r\n1,0,1,\"\",\"\",\"\",\"\"")
        );
        assert!(matches!(
            transform_csv("函数摘要 [[1],\"x\"] 数值\n1\n错误"),
            Err(CsvError::DataRowValue { row: 2, .. })
        ));
    }

    #[test]
    fn ranks_function_values_without_reordering_csv_rows() {
        assert_eq!(
            transform_csv("函数排名列 [\"名次\",[\"角度\",3],\"cosd(x)*y\",\"数值降序\"] 编号,角度,权重\n001,0,3\n002,180,2\n003,360,3\n004,,9\n005,90,4").as_deref(),
            Ok("编号,角度,权重,名次\r\n001,0,3,1\r\n002,180,2,4\r\n003,360,3,1\r\n004,\"\",9,\"\"\r\n005,90,4,3")
        );
        assert_eq!(
            transform_csv(
                "rank-function-column [\"名次\",[1],\"sqrt(x)\",\"numeric-asc\"] 数值\n4\n-1\n1"
            )
            .as_deref(),
            Ok("数值,名次\r\n4,2\r\n-1,\"\"\r\n1,1")
        );
    }

    #[test]
    fn sorts_csv_rows_by_calculator_values_and_keeps_ties_stable_and_empty_last() {
        assert_eq!(
            transform_csv("函数排序 [[\"角度\",3],\"cosd(x)*y\",\"数值降序\"] 编号,角度,权重\n001,0,3\n002,180,2\n003,360,3\n004,,9\n005,90,4").as_deref(),
            Ok("编号,角度,权重\r\n001,0,3\r\n003,360,3\r\n005,90,4\r\n002,180,2\r\n004,\"\",9")
        );
        assert_eq!(
            transform_csv("sort-function [[1],\"sqrt(x)\",\"numeric-asc\"] 数值\n4\n-1\n1")
                .as_deref(),
            Ok("数值\r\n1\r\n4\r\n-1")
        );
        assert_eq!(
            transform_csv("函数排序 [[1],\"x\",\"文本升序\"] X\n1"),
            Err(CsvError::FunctionSortArguments)
        );
    }

    #[test]
    fn filters_function_values_with_an_explicit_absolute_tolerance() {
        let data = "编号,数值\n001,0.1\n002,0.11\n003,\n004,0.10";
        assert_eq!(
            transform_csv(&format!(
                "函数近似筛选 [[2],\"x+0.2\",\"0.3\",\"1e-9\"] {data}"
            ))
            .as_deref(),
            Ok("编号,数值\r\n001,0.1\r\n004,0.10")
        );
        assert_eq!(
            transform_csv(
                "filter-function-near [[1,2],\"x*y\",\"2\",\"1/4\"] x,y\n1,1.75\n1,2.25\n1,2.5"
            )
            .as_deref(),
            Ok("x,y\r\n1,1.75\r\n1,2.25")
        );
        assert_eq!(
            transform_csv("函数近似筛选 [[1],\"x\",\"0\",\"-1\"] x\n0"),
            Err(CsvError::FunctionNearFilterArguments)
        );
    }

    #[test]
    fn filters_original_csv_rows_using_one_or_two_columns_as_function_inputs() {
        assert_eq!(
            transform_csv("函数筛选 [[\"角度\",3],\"cosd(x)*y\",\">\",\"0\"] 编号,角度,权重\n001,0,2\n002,180,3\n003,90,4\n004,,5\n005,360,6").as_deref(),
            Ok("编号,角度,权重\r\n001,0,2\r\n005,360,6")
        );
        assert_eq!(
            transform_csv("filter-function [[1],\"sqrt(x)\",\">=\",\"3/2\"] 数值\n-1\n1\n4")
                .as_deref(),
            Ok("数值\r\n4")
        );
        assert!(matches!(
            transform_csv("函数筛选 [[1,2],\"x+y\",\">\",\"0\"] X,Y\n0,2\n1,坏"),
            Err(CsvError::DataRowValue { row: 2, .. })
        ));
    }

    #[test]
    fn appends_calculator_functions_of_two_columns_and_keeps_original_cells() {
        assert_eq!(
            transform_csv(
                "函数列 [\"加权\",\"角度\",2,\"cosd(X)*Y\"] 角度,权重\n0,2\n180,3\n90,\n,4"
            )
            .as_deref(),
            Ok("角度,权重,加权\r\n0,2,2\r\n180,3,-3\r\n90,\"\",\"\"\r\n\"\",4,\"\"")
        );
        assert_eq!(
            transform_csv("function-column [\"结果\",1,2,\"sqrt(x-y)\"] A,B\n1,2\n4,3").as_deref(),
            Ok("A,B,结果\r\n1,2,\"\"\r\n4,3,1")
        );
        assert_eq!(
            transform_csv("函数列 [\"结果\",1,2,\"x+y\"] A,B\n0,2\n1,坏"),
            Err(CsvError::DataRowValue {
                row: 2,
                cause: CsvError::InvalidNumber.message()
            })
        );
        assert!(matches!(
            transform_csv("函数列 [\"结果\",1,\"y\"] A\n1"),
            Err(CsvError::DataRowValue { row: 1, .. })
        ));
    }

    #[test]
    fn appends_calculator_function_values_with_blank_cells_and_row_errors() {
        assert_eq!(
            transform_csv(
                "函数列 [\"根号\",\"数值\",\"sqrt(x)\"] 编号,数值\n001,4\n002,-1\n003,\n004,1/4"
            )
            .as_deref(),
            Ok("编号,数值,根号\r\n001,4,2\r\n002,-1,\"\"\r\n003,\"\",\"\"\r\n004,1/4,0.5")
        );
        assert_eq!(
            transform_csv("function-column [\"余弦\",1,\"cosd(x)\"] 角度\n0\n文字"),
            Err(CsvError::DataRowValue {
                row: 2,
                cause: CsvError::InvalidNumber.message()
            })
        );
        assert!(matches!(
            transform_csv("函数列 [\"函数值\",1,\"f(x)\"] 角度\n0"),
            Err(CsvError::DataRowValue { row: 1, .. })
        ));
        let over_limit = format!("函数列 [\"结果\",1,\"x\"] 数值{}", "\n1".repeat(129));
        assert_eq!(
            transform_csv(&over_limit),
            Err(CsvError::FunctionColumnLimit)
        );
    }

    #[test]
    fn computes_a_formula_column_using_a_single_named_or_numbered_column() {
        for source in ["1", "\"数量\""] {
            let expression = format!("公式列 [\"平方\",{source},\"x**2\"]\n数量\n1/3\n-3");
            assert_eq!(
                transform_csv(&expression).as_deref(),
                Ok("数量,平方\r\n1/3,1/9\r\n-3,9")
            );
        }
        assert_eq!(
            transform_csv("公式列 [\"结果\",1,\"x+y\"]\n数量\n1"),
            Err(CsvError::FormulaColumnArguments)
        );
    }

    #[test]
    fn appends_an_exact_formula_column_without_rewriting_input_cells() {
        assert_eq!(
            transform_csv(
                "公式列 [\"含费金额\",\"数量\",3,\"x*y+1/6\"] 商品,数量,单价\n\"茶,咖啡\",3,2.50\n梨,2,1/3"
            )
            .as_deref(),
            Ok("商品,数量,单价,含费金额\r\n\"茶,咖啡\",3,2.50,23/3\r\n梨,2,1/3,5/6")
        );
        for (query, error) in [
            (
                "formula-column [\"商\",1,2,\"x/y\"]\nx,y\n1,0",
                CsvError::ComputedDivisionByZero,
            ),
            (
                "公式列 [\"结果\",1,2,\"x+z\"]\nx,y\n1,2",
                CsvError::FormulaColumnArguments,
            ),
            (
                "公式列 [\"x\",1,2,\"x+y\"]\nx,y\n1,2",
                CsvError::HeaderNameConflict,
            ),
        ] {
            assert_eq!(transform_csv(query), Err(error));
        }
        assert_eq!(
            transform_csv(&format!(
                "公式列 [\"和\",1,2,\"x+y\"]\nx,y{}",
                "\n1,2".repeat(MAX_FORMULA_RECORDS + 1)
            )),
            Err(CsvError::FormulaColumnLimit)
        );
    }

    #[test]
    fn calculates_a_new_column_with_exact_cells_and_validates_operands() {
        let table = "商品,数量,单价\n茶,3,2.50\n咖啡,2,1/3";
        assert_eq!(
            transform_csv(&format!("计算列 [\"金额\",2,\"*\",3]\n{table}")).as_deref(),
            Ok("商品,数量,单价,金额\r\n茶,3,2.50,15/2\r\n咖啡,2,1/3,2/3")
        );
        for arguments in [r#"["金额","数量","*","单价"]"#, r#"["金额",2,"*","单价"]"#] {
            assert_eq!(
                transform_csv(&format!("compute-column {arguments}\n{table}")).as_deref(),
                Ok("商品,数量,单价,金额\r\n茶,3,2.50,15/2\r\n咖啡,2,1/3,2/3")
            );
        }
        assert_eq!(
            transform_csv("计算列 [\"金额\",\"价格\",\"*\",1]\n价格,价格\n1,2"),
            Err(CsvError::AmbiguousColumnName)
        );
        assert_eq!(
            transform_csv("计算列 [\"金额\",\"缺列\",\"*\",1]\n价格,数量\n1,2"),
            Err(CsvError::MissingColumnName)
        );
        for (operator, expected) in [("+", "5/6"), ("-", "1/6"), ("/", "3/2")] {
            assert_eq!(
                transform_csv(&format!(
                    "计算列 [\"新列\",1,\"{operator}\",2] x,y\n1/2,1/3"
                ))
                .as_deref(),
                Ok(format!("x,y,新列\r\n1/2,1/3,{expected}").as_str())
            );
        }
        for (query, error) in [
            (
                "计算列 [\"数量\",2,\"+\",3]\nx,数量,z\n茶,3,2.50",
                CsvError::HeaderNameConflict,
            ),
            (
                "计算列 [\"商\",1,\"/\",2]\nx,y\n1,0",
                CsvError::ComputedDivisionByZero,
            ),
            (
                "计算列 [\"金额\",3,\"*\",2]\nx,y\n1,2",
                CsvError::ColumnOutOfRange,
            ),
            (
                "计算列 [\"金额\",1,\"-\",2]\nx,y\n1,空",
                CsvError::InvalidNumber,
            ),
        ] {
            assert_eq!(transform_csv(query), Err(error));
        }
    }

    #[test]
    fn renames_one_unique_csv_header_without_rewriting_cell_contents() {
        assert_eq!(
            transform_csv("改列名 [\"旧名\",\"备注,说明\"] 编号,旧名\n001,\"茶,咖啡\"").as_deref(),
            Ok("编号,\"备注,说明\"\r\n001,\"茶,咖啡\"")
        );
        assert_eq!(
            transform_csv("rename-header [\"A\",\"A\"] A,B").as_deref(),
            Ok("A,B")
        );
        for (query, error) in [
            (r#"改列名 ["A"] A,B"#, CsvError::RenameArguments),
            (r#"改列名 ["A"," "] A,B"#, CsvError::RenameArguments),
            (r#"改列名 ["A","B"] A,B"#, CsvError::HeaderNameConflict),
            (r#"改列名 ["a","Z"] A,B"#, CsvError::MissingColumnName),
            (r#"改列名 ["A","Z"] A,A"#, CsvError::AmbiguousColumnName),
        ] {
            assert_eq!(transform_csv(query), Err(error), "{query}");
        }
    }

    #[test]
    fn takes_group_minimum_and_maximum_by_exact_value_in_first_occurrence_order() {
        let table = "地区,类别,金额\n北,甲,3/2\n南,乙,-1.50\n北,甲,2e0\n南,乙,-2.5\n北,乙,1";
        assert_eq!(
            transform_csv(&format!("分组最小 [\"地区\",2] \"金额\" {table}")).as_deref(),
            Ok("地区,类别,最小值\r\n北,甲,3/2\r\n南,乙,-5/2\r\n北,乙,1")
        );
        assert_eq!(
            transform_csv(&format!("group-max 1,2 3 {table}")).as_deref(),
            Ok("地区,类别,最大值\r\n北,甲,2\r\n南,乙,-3/2\r\n北,乙,1")
        );
        assert_eq!(
            transform_csv(&format!("分组极差 [\"地区\",2] \"金额\" {table}")).as_deref(),
            Ok("地区,类别,极差\r\n北,甲,1/2\r\n南,乙,1\r\n北,乙,0")
        );
        assert_eq!(
            transform_csv("group-range 1 2 地区,金额").as_deref(),
            Ok("地区,极差")
        );
        assert_eq!(
            transform_csv("分组最大 1 2 地区,金额\n北,无"),
            Err(CsvError::InvalidNumber)
        );
    }

    #[test]
    fn converts_csv_duration_cells_to_exact_numbers_usable_in_later_steps() {
        let query = "时长数值列 [\"分钟\",\"时长\",\"min\"] 编号,时长\n001,1秒\n002,PT30S\n003,-0.25秒\n004,";
        let result = "编号,时长,分钟\r\n001,1秒,1/60\r\n002,PT30S,1/2\r\n003,-0.25秒,-1/240\r\n004,\"\",\"\"";
        assert_eq!(transform_csv(query).as_deref(), Ok(result));
        assert_eq!(csv_table_preview(query).unwrap().unwrap().0, result);
        assert!(matches!(
            transform_csv("duration-number-column [\"分钟\",1,\"min\"] 时长\n1秒\n无效"),
            Err(CsvError::DataRowValue { row: 2, .. })
        ));
        assert_eq!(
            transform_csv("时长数值列 [\"分钟\",1,\"iso\"] 时长"),
            Err(CsvError::DurationNumberColumnArguments)
        );
    }

    #[test]
    fn sums_signed_durations_by_csv_groups_with_mixed_notation_and_blank_values() {
        let data = "人员,时长\n北,01:30\n南,PT45S\n北,30.25秒\n空,\n南,-15秒\n负,-1ns\n北,";
        let query = format!(r#"分组时长合计 ["人员"] "时长" {data}"#);
        let result =
            "人员,时长合计\r\n北,00:02:00.25\r\n南,00:00:30\r\n空,\"\"\r\n负,-00:00:00.000000001";
        assert_eq!(transform_csv(&query).as_deref(), Ok(result));
        assert_eq!(csv_table_preview(&query).unwrap().unwrap().0, result);
        assert!(matches!(
            transform_csv("group-duration-sum 1 2 人员,时长\n北,01:00\n南,无效"),
            Err(CsvError::DataRowValue { row: 2, .. })
        ));
    }

    #[test]
    fn aggregates_csv_values_by_column_tuple_in_first_occurrence_order() {
        let data = "编号,地区,状态,金额\n001,\"东,区\",完成,1/2\n002,西,待办,2.5e0\n003,\"东,区\",完成,1.5\n004,\"东,区\",待办,-1";
        assert_eq!(
            transform_csv(&format!("分组合计 3,2 4\n{data}")).as_deref(),
            Ok("状态,地区,合计\r\n完成,\"东,区\",2\r\n待办,西,5/2\r\n待办,\"东,区\",-1")
        );
        assert_eq!(
            transform_csv("group-sum 1 2 名称,金额").as_deref(),
            Ok("名称,合计")
        );
        assert_eq!(
            transform_csv(&format!("分组平均 3,2 4\n{data}")).as_deref(),
            Ok("状态,地区,平均值\r\n完成,\"东,区\",1\r\n待办,西,5/2\r\n待办,\"东,区\",-1")
        );
        assert_eq!(
            transform_csv("group-average 1 2 名称,金额\n甲,1/2\n甲,2/3").as_deref(),
            Ok("名称,平均值\r\n甲,7/12")
        );
        assert_eq!(
            transform_csv("分组平均 1 2 名称,金额").as_deref(),
            Ok("名称,平均值")
        );
        assert_eq!(
            transform_csv(&format!("分组平均 [\"状态\",2] \"金额\"\n{data}")).as_deref(),
            Ok("状态,地区,平均值\r\n完成,\"东,区\",1\r\n待办,西,5/2\r\n待办,\"东,区\",-1")
        );
        assert_eq!(
            transform_csv(&format!("分组计数 [\"状态\"]\n{data}")).as_deref(),
            Ok("状态,计数\r\n完成,2\r\n待办,2")
        );
        for (query, error) in [
            ("分组合计 1 1,2 名称,金额", CsvError::AggregateColumn),
            ("分组合计 1 3 名称,金额", CsvError::ColumnOutOfRange),
            ("分组合计 1 2 名称,金额\n甲,待定", CsvError::InvalidNumber),
            (
                r#"分组计数 ["名称",1] 名称,金额"#,
                CsvError::GroupingColumns,
            ),
            (
                r#"分组计数 ["名称"] 名称,名称"#,
                CsvError::AmbiguousColumnName,
            ),
            (
                r#"分组平均 ["名称"] "金额" 名称,单价"#,
                CsvError::MissingColumnName,
            ),
        ] {
            assert_eq!(transform_csv(query), Err(error), "{query}");
        }
    }

    #[test]
    fn filters_csv_by_exact_header_name_for_each_condition_kind() {
        let data = "编号,当前 状态,单价\n001,完成,1.50\n002,待办,1.49\n003,已完成,2e0";
        let header = "编号,当前 状态,单价";
        for (operation, rows) in [
            (r#"筛选 "当前 状态" "完成""#, "001,完成,1.50"),
            (r#"正则排除 "当前 状态" "完成$""#, "002,待办,1.49"),
            (
                r#"数值筛选 "单价" >= 3/2"#,
                "001,完成,1.50\r\n003,已完成,2e0",
            ),
        ] {
            let expected = format!("{header}\r\n{rows}");
            assert_eq!(
                transform_csv(&format!("{operation} {data}")).as_deref(),
                Ok(expected.as_str()),
                "{operation}"
            );
        }
        for (query, error) in [
            (
                r#"筛选 "状态" "完成" 编号,当前状态"#,
                CsvError::MissingColumnName,
            ),
            (
                r#"数值筛选 "价格" >= 1 价格,价格"#,
                CsvError::AmbiguousColumnName,
            ),
        ] {
            assert_eq!(transform_csv(query), Err(error), "{query}");
        }
    }

    #[test]
    fn sorts_csv_by_exact_numeric_keys_without_rewriting_cells() {
        let data = "编号,金额,优先级\n005,9007199254740993,0\n001,01.50,2\n004,9007199254740992,0\n003,-2.5,0\n002,3/2,1\n006,+1.5,1";
        assert_eq!(
            transform_csv(&format!("数值排序 2,3\n{data}")).as_deref(),
            Ok("编号,金额,优先级\r\n003,-2.5,0\r\n002,3/2,1\r\n006,+1.5,1\r\n001,01.50,2\r\n004,9007199254740992,0\r\n005,9007199254740993,0")
        );
        assert_eq!(
            transform_csv(&format!("sort-numeric-desc 2,3\n{data}")).as_deref(),
            Ok("编号,金额,优先级\r\n005,9007199254740993,0\r\n004,9007199254740992,0\r\n001,01.50,2\r\n002,3/2,1\r\n006,+1.5,1\r\n003,-2.5,0")
        );
        assert_eq!(transform_csv("数值排序 1 a,b").as_deref(), Ok("a,b"));
        assert_eq!(
            transform_csv("数值排序 2 编号,金额\n001,2e2\n002,-1.5E-1\n003,.25").as_deref(),
            Ok("编号,金额\r\n002,-1.5E-1\r\n003,.25\r\n001,2e2")
        );
        assert_eq!(
            transform_csv("数值降序 3 a,b"),
            Err(CsvError::ColumnOutOfRange)
        );
        for invalid in ["", "待确认", "1/0", "1/2/3", "1+2", "--2", "1.2.3"] {
            assert_eq!(
                transform_csv(&format!("数值排序 2 编号,金额\n001,{invalid}")),
                Err(CsvError::InvalidNumber),
                "{invalid:?}"
            );
        }
    }

    #[test]
    fn selects_csv_columns_in_the_requested_order_and_keeps_quoted_fields() {
        let csv = "编号,略过,备注\r\n001,未选,\"茶,咖啡\"\r\n002,,\"含\r\n换行\"\r\n003,未选,";
        let result = transform_csv(&format!("选列 3,1\r\n\u{feff}{csv}")).unwrap();
        assert_eq!(
            result,
            "备注,编号\r\n\"茶,咖啡\",001\r\n\"含\r\n换行\",002\r\n\"\",003"
        );
        assert_eq!(
            transform_csv("pick-columns 1  保留前导空格,忽略").as_deref(),
            Ok(" 保留前导空格"),
        );
        for (query, error) in [
            ("选列 0 a,b", CsvError::InvalidColumns),
            ("选列 1,1 a,b", CsvError::InvalidColumns),
            ("选列 1, a,b", CsvError::InvalidColumns),
            ("选列 3 a,b", CsvError::ColumnOutOfRange),
            ("选列 1 a,b\nc", CsvError::DifferentColumnCount),
        ] {
            assert_eq!(transform_csv(query), Err(error), "{query}");
        }
    }

    #[test]
    fn selects_csv_columns_by_exact_header_and_preserves_requested_order() {
        assert_eq!(
            transform_csv(
                "选列名 [\"备注,说明\",\"编号\"]\r\n编号,名称,\"备注,说明\"\r\n001,茶,\"含,逗号\""
            )
            .as_deref(),
            Ok("\"备注,说明\",编号\r\n\"含,逗号\",001")
        );
        assert_eq!(
            transform_csv("pick-by-header [\" \",\"\"]  ,\n末尾,空").as_deref(),
            Ok(" ,\"\"\r\n末尾,空")
        );
        assert_eq!(
            transform_csv("选列名 [\"编号\"] 编号,重复,重复\n001,甲,乙").as_deref(),
            Ok("编号\r\n001")
        );
        for (query, error) in [
            (r#"选列名 [] a,b"#, CsvError::InvalidColumnNames),
            (r#"选列名 ["a","a"] a,b"#, CsvError::InvalidColumnNames),
            (r#"选列名 ["a"]a,b"#, CsvError::InvalidColumnNames),
            (r#"选列名 ["a"] A,b"#, CsvError::MissingColumnName),
            (r#"选列名 ["a"] a,a"#, CsvError::AmbiguousColumnName),
        ] {
            assert_eq!(transform_csv(query), Err(error), "{query}");
        }
    }

    #[test]
    fn recognizes_a_unique_delimiter_with_uniform_quoted_records() {
        for (input, expected) in [
            (
                "自动转CSV 编号;金额;备注\n001;1,50;\"茶;浓\"\n002;;\"甲\n乙\"",
                "编号,金额,备注\r\n001,\"1,50\",茶;浓\r\n002,\"\",\"甲\n乙\"",
            ),
            (
                "auto-csv \u{feff}编号\t名称\n001\t茶",
                "编号,名称\r\n001,茶",
            ),
            ("自动转CSV 甲|乙\n1|2", "甲,乙\r\n1,2"),
            ("自动转CSV 甲,乙\n1,2", "甲,乙\r\n1,2"),
        ] {
            assert_eq!(transform_csv(input).as_deref(), Ok(expected), "{input}");
        }
        assert_eq!(
            transform_csv("自动转CSV a,b;c\n1,2;3"),
            Err(CsvError::AmbiguousDelimiter),
        );
        for input in ["自动转CSV a,b", "自动转CSV a\nb", "自动转CSV a;b\n1"] {
            assert_eq!(
                transform_csv(input),
                Err(CsvError::UnknownDelimiter),
                "{input}"
            );
        }
    }

    #[test]
    fn converts_explicit_delimiters_while_preserving_quotes_and_leading_empty_fields() {
        let semicolons =
            "分隔符转换 [\";\",\",\"] 编号;金额;备注\n001;1,50;\"茶;浓\"\n002;;\"甲\n乙\"";
        assert_eq!(
            transform_csv(semicolons).as_deref(),
            Ok("编号,金额,备注\r\n001,\"1,50\",茶;浓\r\n002,\"\",\"甲\n乙\"")
        );
        let tab_to_pipe = "convert-delimiter [\"\\t\",\"|\"]\r\n\t备注\r\n001\t茶|梨";
        assert_eq!(
            transform_csv(tab_to_pipe).as_deref(),
            Ok("\"\"|备注\r\n001|\"茶|梨\"")
        );
        assert_eq!(
            transform_csv("分隔符转换 [\"、\",\"|\"] 编号、名称\n001、茶").as_deref(),
            Ok("编号|名称\r\n001|茶")
        );
        for arguments in [
            r#"["",","]"#,
            r#"["||",","]"#,
            r#"["\"",";"]"#,
            r#"[";","\n"]"#,
        ] {
            assert_eq!(
                transform_csv(&format!("分隔符转换 {arguments} 编号;数量")),
                Err(CsvError::DelimiterArguments)
            );
        }
    }

    #[test]
    fn translates_csv_and_quoted_tsv_without_changing_fields() {
        let csv = "编号,备注,值\r\n001,\"含,逗号\",含\t制表符\r\n002,\"含\r\n换行\",";
        let tsv = "编号\t备注\t值\r\n001\t含,逗号\t\"含\t制表符\"\r\n002\t\"含\r\n换行\"\t\"\"";
        assert_eq!(transform_csv(&format!("转TSV {csv}")).as_deref(), Ok(tsv));
        let restored = transform_csv(&format!("从TSV\n\u{feff}{tsv}")).unwrap();
        assert_eq!(read_rows(&restored, ','), read_rows(csv, ','));
        assert_eq!(transform_csv("tsv \"\"").as_deref(), Ok("\"\""));
        assert_eq!(transform_csv("from-tsv \"\"").as_deref(), Ok("\"\""));
        assert_eq!(
            transform_csv("从TSV a\tb\nc"),
            Err(CsvError::DifferentColumnCount)
        );
        assert_eq!(
            transform_csv("从TSV \"甲\"x\t乙"),
            Err(CsvError::InvalidSeparator)
        );
    }

    #[test]
    fn converts_csv_to_json_without_losing_cell_text() {
        assert_eq!(
            transform_csv("转JSON  编号,名称,备注\r\n001,\"茶,咖啡\",\"他写下\"\"你好\"\"\r\n下一行\"\r\n002,,末尾 "),
            Ok("[[\" 编号\",\"名称\",\"备注\"],[\"001\",\"茶,咖啡\",\"他写下\\\"你好\\\"\\r\\n下一行\"],[\"002\",\"\",\"末尾 \"]]".to_owned())
        );
        assert_eq!(
            transform_csv("JSON\r\n\u{feff}a,b,\nc,d,\n").as_deref(),
            Ok("[[\"a\",\"b\",\"\"],[\"c\",\"d\",\"\"]]")
        );
        assert_eq!(transform_csv("json \"\"").as_deref(), Ok("[[\"\"]]"));
        assert_eq!(
            transform_csv("对象 编号,名称\n001,茶\n002,咖啡").as_deref(),
            Ok("[{\"编号\":\"001\",\"名称\":\"茶\"},{\"编号\":\"002\",\"名称\":\"咖啡\"}]")
        );
        assert_eq!(transform_csv("objects a,b").as_deref(), Ok("[]"));
        for (query, error) in [
            ("json ", CsvError::MissingInput),
            ("json \"a", CsvError::UnclosedQuote),
            ("json \"a\"x", CsvError::InvalidSeparator),
            ("json a\"b", CsvError::InvalidSeparator),
            ("json a,b\n1", CsvError::DifferentColumnCount),
            ("objects a,a\n1,2", CsvError::InvalidHeader),
            ("objects a,\n1,2", CsvError::InvalidHeader),
        ] {
            assert_eq!(transform_csv(query), Err(error), "{query}");
        }
    }
}
