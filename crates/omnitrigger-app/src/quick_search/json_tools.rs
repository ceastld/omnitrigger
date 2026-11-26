use std::{
    borrow::Cow,
    cmp::Ordering,
    collections::{HashMap, HashSet},
    fmt,
    ops::Range,
};

use num_bigint::BigInt;
use num_rational::BigRational;
use serde::de::{IgnoredAny, MapAccess, Visitor};
use serde_json::value::RawValue;

use super::csv_tools::{append_bounded_csv_row, csv_cell};
use super::fraction::{ensure_exact_size, parse_fraction_literal};
use super::operators::{FieldSortOrder, NumberComparison, MAX_SORT_FIELDS};
use super::statistics::{median_sorted_values, MAX_VALUES};
use super::text_tools::compare_natural;
use super::{input_line_column, is_validation_operation, InputTableReport, InputValidationReport};

const MAX_PRETTY_BYTES: usize = 1024 * 1024;
const DEFAULT_INDENT_WIDTH: usize = 2;
const MAX_INDENT_WIDTH: usize = 16;
const MAX_PATCH_STEPS: usize = 32;
const MAX_MERGE_PATCH_DEPTH: usize = 64;
const MAX_PIPELINE_STEPS: usize = 8;
const MAX_FILTER_CONDITIONS: usize = 16;
const MAX_UNIQUE_PATHS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum JsonError {
    Incomplete,
    UnknownOperation,
    Invalid,
    LayoutTooLarge,
    IndentArguments,
    InvalidPointer,
    InvalidPointerSegment,
    PointerSelection,
    WildcardPathArguments,
    SourceAndTargetPaths,
    MoveIntoDescendant,
    PatchArguments,
    PatchOperation,
    PatchLimit,
    PatchOutputLimit,
    PatchTestFailed,
    PatchStep { index: usize, cause: &'static str },
    MergePatchArguments,
    MergePatchDepth,
    MergePatchOutputLimit,
    PipelineArguments,
    NestedPipeline,
    PipelineOutputTooLarge,
    PipelineStep { index: usize, cause: &'static str },
    ComparisonTooLarge,
    PointerListTooLarge,
    KeySearchArguments,
    SearchPatternArguments,
    TypeSearchArguments,
    ValueSearchArguments,
    DuplicateKeyReportTooLarge,
    DuplicateKeyPolicy,
    FlattenedObjectTooLarge,
    ExpectedObject,
    ExpectedCollection,
    ExpectedArray,
    ArraySliceArguments,
    ExpectedArrayAtPointer,
    ExpectedObjectItem,
    DefaultFieldArguments,
    ObjectOutputTooLarge,
    InvalidLine,
    MissingValue,
    DuplicateKey,
    KeyArguments,
    MissingKey,
    RenameConflict,
    CsvRows,
    CsvLineObjects,
    CsvRowHeaders,
    CsvDuplicateKey,
    CsvNullCharacter,
    CsvOutputTooLarge,
    SelectionKeys,
    SelectionConflict,
    FilterValue,
    ReplacementValue,
    RemoveRoot,
    NumberFilter,
    FilterConditions,
    SortFields,
    UniquePaths,
    LineUniquePaths,
    ExpectedScalar,
    ExpectedNumber,
    GroupStatisticsPaths,
    GroupStatisticsNumber,
    GroupStatisticsRange,
    GroupStatisticsLimit,
    ExpectedString,
}

impl JsonError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入操作和 JSON 文本",
            Self::UnknownOperation => "输入 ? JSON 查看可用操作",
            Self::Invalid => "JSON 语法无效",
            Self::LayoutTooLarge => "JSON 格式化结果超过 1 MiB",
            Self::IndentArguments => "先填写 1 至 16 的缩进空格数，再粘贴 JSON",
            Self::InvalidPointer => "JSON 路径无效，示例 /商品/0/价格",
            Self::InvalidPointerSegment => "指针分段须为字符串或非负整数",
            Self::PointerSelection => "用 JSON 字符串数组填写互不重复的指针，再粘贴 JSON",
            Self::WildcardPathArguments => {
                "填写由字符串和 null 组成的路径数组，例如 [\"订单\",null,\"金额\"]"
            }
            Self::SourceAndTargetPaths => "用 JSON 数组填写来源和目标两个路径，再粘贴 JSON",
            Self::MoveIntoDescendant => "目标路径不能位于要移动的值内",
            Self::PatchArguments => {
                "填写补丁数组和原 JSON，每步填写 op、path 及所需的 value 或 from"
            }
            Self::PatchOperation => "op 可用 add、remove、replace、move、copy 或 test",
            Self::PatchLimit => "补丁最多可包含 32 步",
            Self::PatchOutputLimit => "补丁中间结果超过 1 MiB",
            Self::PatchTestFailed => "指定路径上的 JSON 值与预期不同",
            Self::PatchStep { cause, .. } => cause,
            Self::MergePatchArguments => "先填写 JSON 合并补丁，再粘贴原 JSON",
            Self::MergePatchDepth => "合并补丁最多处理 64 层嵌套对象",
            Self::MergePatchOutputLimit => "合并补丁结果超过 1 MiB",
            Self::PipelineArguments => "填写 1 至 8 条单行 JSON 命令，再粘贴要处理的数据",
            Self::NestedPipeline => "每一步填写一条 JSON 命令",
            Self::PipelineOutputTooLarge => "处理结果超过 1 MiB，请缩小数据范围",
            Self::PipelineStep { cause, .. } => cause,
            Self::ComparisonTooLarge => "JSON 对比结果超过 1 MiB",
            Self::PointerListTooLarge => "指针列表超过 1 MiB",
            Self::KeySearchArguments => {
                "先输入带双引号的键名，再输入 JSON，例如 \"编号\" {\"编号\":1}"
            }
            Self::SearchPatternArguments => "用双引号括起有效的正则表达式，再输入 JSON",
            Self::TypeSearchArguments => {
                "填写一个类型或类型数组，可选 object、array、string、number、boolean 和 null"
            }
            Self::ValueSearchArguments => "先输入要查找的 JSON 标量，再输入文档，例如 1.5 [1,1.50]",
            Self::DuplicateKeyReportTooLarge => "重复键报告超过 1 MiB，请缩小数据范围",
            Self::DuplicateKeyPolicy => "在 JSON 前填写留首或留末，指定保留哪次出现的键",
            Self::FlattenedObjectTooLarge => "JSON 展平结果超过 1 MiB",
            Self::ExpectedObject => "请输入 JSON 对象",
            Self::ExpectedCollection => "请输入 JSON 数组或对象以统计顶层条目数",
            Self::ExpectedArray => "请输入 JSON 数组",
            Self::ArraySliceArguments => {
                "填写 [起始下标,结束下标] 或 [起始下标,结束下标,非零步长]，可用 null 省略参数"
            }
            Self::ExpectedArrayAtPointer => "每一项的指定 JSON 路径都须对应数组",
            Self::ExpectedObjectItem => "数组中的每一项都应是 JSON 对象",
            Self::DefaultFieldArguments => {
                "填写 [键名,默认值] 或默认值对象，再粘贴 JSON 对象或对象数组"
            }
            Self::ObjectOutputTooLarge => "JSON 对象处理结果超过 1 MiB",
            Self::InvalidLine => "每个非空行需要一个完整的 JSON 值",
            Self::MissingValue => "JSON 路径没有对应的值",
            Self::DuplicateKey => "JSON 路径遇到了重复的键",
            Self::KeyArguments => "用 JSON 字符串数组填写键名，改键需要两个键名",
            Self::MissingKey => "顶层对象中找不到要改名的键",
            Self::RenameConflict => "改键要求原键唯一且新键名可用",
            Self::CsvRows => "输入等宽且有数据列的数组，或至少有一个键的对象列表",
            Self::CsvLineObjects => "每行填写一个 JSON 对象，至少有一行包含字段",
            Self::CsvRowHeaders => "先填写非空且互不重复的列名，再粘贴等宽的行数组",
            Self::CsvDuplicateKey => "同一对象的 CSV 列名需要唯一",
            Self::CsvNullCharacter => "CSV 单元格不能包含空字符 U+0000",
            Self::CsvOutputTooLarge => "CSV 转换结果超过 1 MiB",
            Self::SelectionKeys => "选键需要至少一个互不重复的键名",
            Self::SelectionConflict => "待选键在同一对象中重复出现",
            Self::FilterValue => "路径后请输入 JSON 字符串、数字、布尔值或 null，再粘贴数组",
            Self::ReplacementValue => "路径后填入一个完整的新 JSON 值，再用空格隔开原 JSON",
            Self::RemoveRoot => "删路径需要指向一个对象成员或数组项",
            Self::NumberFilter => "路径后填写比较符和 JSON 数字，例如 /价格 >= 1.5",
            Self::FilterConditions => {
                "填写 1 至 16 个 [路径,比较符,值]，例如 [[\"/金额\",\">=\",1.5]]"
            }
            Self::SortFields => {
                "填写 1 至 8 个 [路径,排序方式]，方式可为文本升序、文本降序、数值升序或数值降序"
            }
            Self::UniquePaths => "填写 2 至 8 个不同路径，例如 [\"/地区\",\"/月份\"]",
            Self::LineUniquePaths => "填写 1 至 8 个不同路径，例如 [\"/编号\"]",
            Self::ExpectedScalar => "指定路径的值须为字符串、数字、布尔值或 null",
            Self::ExpectedNumber => "数值排序需要每一项的指定路径都对应 JSON 数字",
            Self::GroupStatisticsPaths => {
                "用 JSON 数组填写分组和数值路径，例如 [\"/地区\",\"/金额\"]"
            }
            Self::GroupStatisticsNumber => "数值路径须在每一项中对应 JSON 数字",
            Self::GroupStatisticsRange => "分组数值或结果超出精确计算范围",
            Self::GroupStatisticsLimit => "每组最多可统计 128 个数值",
            Self::ExpectedString => "文本排序需要每一项的指定路径都对应 JSON 字符串",
        }
    }
}

pub(super) fn transform_json(query: &str) -> Result<String, JsonError> {
    transform_json_operation(query, true)
}

enum JsonCsvOperation {
    Convert,
    RowsWithHeaders,
    Lines,
    Paths,
    LinePaths,
    WildcardPaths,
    LineGroupCounts,
    LineGroupSummaries,
    LineGroupMedians,
    LineDuplicates,
    ValidateLines,
    Differences,
    DuplicateKeys,
    Leaves,
    Keys,
    KeyPatterns,
    Values,
    ValuePatterns,
    PointerPatterns,
    Types,
}

impl JsonCsvOperation {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "转csv" | "to-csv" => Some(Self::Convert),
            "行数组转csv" | "rows-to-csv" => Some(Self::RowsWithHeaders),
            "行转csv" | "jsonl-to-csv" => Some(Self::Lines),
            "路径转csv" | "pointers-to-csv" => Some(Self::Paths),
            "行路径转csv" | "jsonl-pointers-to-csv" => Some(Self::LinePaths),
            "通配路径csv" | "wildcard-paths-csv" => Some(Self::WildcardPaths),
            "行分组计数csv" | "jsonl-group-counts-csv" => Some(Self::LineGroupCounts),
            "行分组统计csv" | "jsonl-group-statistics-csv" => Some(Self::LineGroupSummaries),
            "行分组中位数csv" | "jsonl-group-median-csv" => Some(Self::LineGroupMedians),
            "行重复位置csv" | "jsonl-duplicate-positions-csv" => Some(Self::LineDuplicates),
            "校验行csv" | "validate-jsonl-csv" => Some(Self::ValidateLines),
            "对比csv" | "diff-csv" => Some(Self::Differences),
            "重复键csv" | "duplicate-keys-csv" => Some(Self::DuplicateKeys),
            "叶子csv" | "leaves-csv" => Some(Self::Leaves),
            "找键csv" | "find-key-csv" => Some(Self::Keys),
            "正则找键csv" | "find-key-regex-csv" => Some(Self::KeyPatterns),
            "找值csv" | "find-value-csv" => Some(Self::Values),
            "正则找值csv" | "find-value-regex-csv" => Some(Self::ValuePatterns),
            "正则找路径csv" | "find-pointer-regex-csv" => Some(Self::PointerPatterns),
            "找类型csv" | "find-type-csv" => Some(Self::Types),
            _ => None,
        }
    }

    fn has_headers(&self, source: &str) -> bool {
        match self {
            Self::Convert => source
                .strip_prefix('[')
                .is_some_and(|rows| rows.trim_start().starts_with('{')),
            Self::RowsWithHeaders
            | Self::Lines
            | Self::Paths
            | Self::LinePaths
            | Self::WildcardPaths
            | Self::LineGroupCounts
            | Self::LineGroupSummaries
            | Self::LineGroupMedians
            | Self::LineDuplicates
            | Self::ValidateLines
            | Self::Differences
            | Self::DuplicateKeys
            | Self::Leaves
            | Self::Keys
            | Self::KeyPatterns
            | Self::Values
            | Self::ValuePatterns
            | Self::PointerPatterns
            | Self::Types => true,
        }
    }

    fn run(self, source: &str) -> Result<String, JsonError> {
        let source = match self {
            Self::ValidateLines => source,
            _ => source.trim_start(),
        };
        match self {
            Self::Convert => json_to_csv(source),
            Self::RowsWithHeaders => json_array_rows_with_headers(source),
            Self::Lines => json_lines_to_csv(source),
            Self::Paths => json_paths_to_csv(source, read_json_array),
            Self::LinePaths => json_paths_to_csv(source, read_json_lines),
            Self::WildcardPaths => json_wildcard_paths_csv(source),
            Self::LineGroupCounts => count_json_line_groups_csv(source),
            Self::LineGroupSummaries => summarize_json_line_groups_csv(source),
            Self::LineGroupMedians => median_json_line_groups_csv(source),
            Self::LineDuplicates => duplicate_json_line_positions(source),
            Self::ValidateLines => validate_json_lines_csv(source).map(|report| report.csv),
            Self::Differences => compare_json_values(source, ComparisonOutput::Csv),
            Self::DuplicateKeys => report_duplicate_json_keys(source, true),
            Self::Leaves => {
                write_json_pointer_results(source, PointerSelection::Leaves, PointerOutput::Csv)
            }
            Self::Keys => find_json_keys(source, PointerOutput::Csv),
            Self::KeyPatterns => find_json_pattern(source, PointerOutput::Csv, PatternTarget::Key),
            Self::Values => find_json_values(source, PointerOutput::Csv),
            Self::ValuePatterns => {
                find_json_pattern(source, PointerOutput::Csv, PatternTarget::Value)
            }
            Self::PointerPatterns => {
                find_json_pattern(source, PointerOutput::Csv, PatternTarget::Pointer)
            }
            Self::Types => find_json_types(source, PointerOutput::Csv),
        }
    }
}

pub(super) fn json_csv_report(query: &str) -> Option<Result<String, JsonError>> {
    let (operation, source) = split_json_operation(query)?;
    let operation = JsonCsvOperation::from_name(&operation.to_ascii_lowercase())?;
    operation
        .has_headers(source.trim_start())
        .then(|| operation.run(source))
}

fn split_json_operation(query: &str) -> Option<(&str, &str)> {
    let (operation, source) = query.split_once(char::is_whitespace)?;
    let source = query[operation.len()..]
        .strip_prefix("\r\n")
        .unwrap_or(source);
    Some((operation, source))
}

pub(super) fn json_validation_report(
    query: &str,
) -> Option<Result<InputValidationReport, JsonError>> {
    let (operation, source) = split_json_operation(query)?;
    let report = if is_validation_operation(operation) {
        validate_json(source)
    } else if is_jsonl_validation_operation(operation) {
        validate_json_lines(source)
    } else {
        return None;
    };
    Some(report.map(|report| report.offset_error_span(query.len() - source.len())))
}

pub(super) fn json_validation_table(query: &str) -> Option<Result<InputTableReport, JsonError>> {
    let (operation, source) = split_json_operation(query)?;
    if !matches!(
        JsonCsvOperation::from_name(&operation.to_ascii_lowercase()),
        Some(JsonCsvOperation::ValidateLines)
    ) {
        return None;
    }
    Some(
        validate_json_lines_csv(source)
            .map(|report| report.offset_input_errors(query.len() - source.len())),
    )
}

pub(super) fn is_jsonl_validation_operation(operation: &str) -> bool {
    operation == "校验行" || operation.eq_ignore_ascii_case("validate-jsonl")
}

fn validate_json(source: &str) -> Result<InputValidationReport, JsonError> {
    if source
        .bytes()
        .all(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
    {
        return Err(JsonError::Incomplete);
    }
    let error = match serde_json::from_str::<IgnoredAny>(source) {
        Ok(_) => {
            return Ok(InputValidationReport {
                text: "JSON 有效".into(),
                error_span: None,
            });
        }
        Err(error) => error,
    };
    let mut start = if error.is_eof() {
        source.len()
    } else {
        let line_start: usize = source
            .split_inclusive('\n')
            .take(error.line().saturating_sub(1))
            .map(str::len)
            .sum();
        // serde_json 的列号按字节计算，遇到换行可报告下一行的第 0 列。
        (line_start + error.column())
            .saturating_sub(1)
            .min(source.len())
    };
    while !source.is_char_boundary(start) {
        start -= 1;
    }
    let end = source[start..]
        .chars()
        .next()
        .map_or(start, |ch| start + ch.len_utf8());
    let (line, column) = input_line_column(&source[..start]);
    let reason = json_line_error_reason(&error);
    Ok(InputValidationReport {
        text: format!("{reason} · 第 {line} 行 · 第 {column} 列"),
        error_span: Some(start..end),
    })
}

pub(super) enum JsonResultFormat {
    Json,
    JsonLines,
    Csv,
}

pub(super) fn result_format(query: &str, output: &str) -> Option<JsonResultFormat> {
    let (operation, _) = split_json_operation(query)?;
    let operation = operation.to_ascii_lowercase();
    if JsonCsvOperation::from_name(&operation).is_some() {
        return Some(JsonResultFormat::Csv);
    }
    if is_json_lines_output(&operation) {
        return Some(JsonResultFormat::JsonLines);
    }
    if matches!(operation.as_str(), "依次处理" | "pipeline") {
        return None;
    }
    serde_json::from_str::<&RawValue>(output)
        .is_ok()
        .then_some(JsonResultFormat::Json)
}

fn is_json_lines_output(operation: &str) -> bool {
    matches!(
        operation,
        "展开数组"
            | "to-jsonl"
            | "行筛选"
            | "filter-jsonl"
            | "行任一筛选"
            | "filter-any-jsonl"
            | "行路径去重"
            | "unique-jsonl-by-pointers"
            | "行路径去重留末"
            | "last-jsonl-by-pointers"
    )
}

fn transform_json_operation(query: &str, allow_pipeline: bool) -> Result<String, JsonError> {
    let (operation, raw_source) = split_json_operation(query).ok_or(JsonError::Incomplete)?;
    let operation = operation.to_ascii_lowercase();
    if is_validation_operation(&operation) {
        return validate_json(raw_source).map(|report| report.text);
    }
    if is_jsonl_validation_operation(&operation) {
        return validate_json_lines(raw_source).map(|report| report.text);
    }
    if let Some(operation) = JsonCsvOperation::from_name(&operation) {
        return operation.run(raw_source);
    }
    let source = raw_source.trim_start();
    let indent_width = match operation.as_str() {
        "依次处理" | "pipeline" if allow_pipeline => return transform_json_pipeline(source),
        "依次处理" | "pipeline" => return Err(JsonError::NestedPipeline),
        "格式化" | "format" | "pretty" => Some(DEFAULT_INDENT_WIDTH),
        "压缩" | "compact" | "minify" => None,
        "缩进" | "indent" => {
            let (width, json) = source
                .split_once(char::is_whitespace)
                .unwrap_or((source, ""));
            let width = width
                .parse::<usize>()
                .ok()
                .filter(|width| (1..=MAX_INDENT_WIDTH).contains(width))
                .ok_or(JsonError::IndentArguments)?;
            return format_json(json.trim_start(), Some(width));
        }
        "去注释" | "strip-comments" => {
            return format_json(&strip_json_comments(source)?, Some(DEFAULT_INDENT_WIDTH));
        }
        "键排序" | "sort-keys" => return rewrite_object_keys(source, ObjectKeyMode::Sort),
        "取值" | "get" | "pointer" => return select_json_value(source),
        "类型" | "type" => return json_value_type(source),
        "取多路径" | "get-paths" => return select_json_paths(source),
        "通配取值" | "select-wildcard" => return select_json_wildcard(source),
        "取列" | "column" => return select_json_column(source),
        "合并子数组" | "flatten-arrays" => return flatten_json_arrays(source),
        "有路径" | "has-pointer" => return filter_json_by_pointer(source, true),
        "缺路径" | "missing-pointer" => return filter_json_by_pointer(source, false),
        "筛选" | "filter" => return filter_json_by_scalar(source, true),
        "排除" | "exclude" => return filter_json_by_scalar(source, false),
        "数值筛选" | "filter-numeric" => return filter_json_numbers(source),
        "多条件筛选" | "filter-all" => {
            return filter_json_conditions(source, true, ConditionRecords::Array)
        }
        "任一条件筛选" | "filter-any" => {
            return filter_json_conditions(source, false, ConditionRecords::Array)
        }
        "行筛选" | "filter-jsonl" => {
            return filter_json_conditions(source, true, ConditionRecords::Lines)
        }
        "行任一筛选" | "filter-any-jsonl" => {
            return filter_json_conditions(source, false, ConditionRecords::Lines)
        }
        "按值去重" | "unique-by-pointer" => return deduplicate_json_by_scalar(source),
        "行路径去重" | "unique-jsonl-by-pointers" => return deduplicate_json_lines(source),
        "多路径去重" | "unique-by-pointers" => return match_json_by_pointers(source, false),
        "多路径去重留末" | "last-by-pointers" => {
            return retain_last_json_by_paths(source, false)
        }
        "行路径去重留末" | "last-jsonl-by-pointers" => {
            return retain_last_json_by_paths(source, true)
        }
        "多路径查重复" | "duplicates-by-pointers" => {
            return match_json_by_pointers(source, true)
        }
        "分组计数" | "group-count" => return count_json_by_scalar(source),
        "分组明细" | "group-records" => return group_json_records(source),
        "分组统计" | "group-statistics" => return summarize_json_groups(source),
        "分组中位数" | "group-median" => return median_json_groups(source),
        "数值排序" | "sort-numeric" => return sort_json_numbers(source, false),
        "数值降序" | "sort-numeric-desc" => return sort_json_numbers(source, true),
        "文本排序" | "sort-text" => return sort_json_text(source, false),
        "文本降序" | "sort-text-desc" => return sort_json_text(source, true),
        "多路径排序" | "sort-by-pointers" => return sort_json_by_pointers(source),
        "键名" | "keys" => return json_keys(source),
        "长度" | "length" => return json_length(source),
        "结构统计" | "structure-stats" => return json_structure_stats(source),
        "切片" | "slice" => return slice_json_array(source),
        "展开数组" | "to-jsonl" => return json_array_to_lines(source),
        "合并行" | "from-jsonl" => return json_lines_to_array(source),
        "合并对象" | "merge-objects" => return merge_json_objects(source),
        "生成指针" | "build-pointer" => return build_json_pointer(source),
        "解析指针" | "parse-pointer" => return parse_json_pointer(source),
        "列指针" | "列路径" | "list-pointers" => {
            return write_json_pointer_results(
                source,
                PointerSelection::Leaves,
                PointerOutput::Paths,
            )
        }
        "找键" | "find-key" => return find_json_keys(source, PointerOutput::Paths),
        "正则找键" | "find-key-regex" => {
            return find_json_pattern(source, PointerOutput::Paths, PatternTarget::Key)
        }
        "找值" | "find-value" => return find_json_values(source, PointerOutput::Paths),
        "正则找值" | "find-value-regex" => {
            return find_json_pattern(source, PointerOutput::Paths, PatternTarget::Value)
        }
        "正则找路径" | "find-pointer-regex" => {
            return find_json_pattern(source, PointerOutput::Paths, PatternTarget::Pointer)
        }
        "找类型" | "find-type" => return find_json_types(source, PointerOutput::Paths),
        "重复键" | "duplicate-keys" => return report_duplicate_json_keys(source, false),
        "去重复键" | "deduplicate-keys" => return remove_duplicate_json_keys(source),
        "展平" | "flatten" => {
            return write_json_pointer_results(
                source,
                PointerSelection::Leaves,
                PointerOutput::Flattened,
            )
        }
        "改键" | "rename-key" => return rename_json_key(source),
        "改值" | "replace" => return write_json_value(source, false),
        "加路径" | "add-path" => return write_json_value(source, true),
        "复制路径" | "copy-path" => return transfer_json_value(source, false),
        "移动路径" | "move-path" => return transfer_json_value(source, true),
        "应用补丁" | "apply-patch" => return apply_json_patch(source),
        "合并补丁" | "merge-patch" => return apply_json_merge_patch(source),
        "对比" | "diff" => return compare_json_values(source, ComparisonOutput::Json),
        "删路径" | "remove-path" => return remove_json_path(source),
        "去键" | "remove-keys" => return remove_json_keys(source),
        "去空字段" | "remove-null-fields" => return remove_null_json_fields(source),
        "递归去空字段" | "remove-null-fields-deep" => {
            return rewrite_object_keys(source, ObjectKeyMode::DropNull);
        }
        "补字段" | "fill-missing-field" => return fill_missing_json_fields(source),
        "选键" | "pick-keys" => return pick_json_keys(source),
        _ => return Err(JsonError::UnknownOperation),
    };
    format_json(source, indent_width)
}

fn transform_json_pipeline(input: &str) -> Result<String, JsonError> {
    let mut arguments = serde_json::Deserializer::from_str(input).into_iter::<Vec<String>>();
    let steps = arguments
        .next()
        .ok_or(JsonError::PipelineArguments)?
        .map_err(|_| JsonError::PipelineArguments)?;
    let remaining = &input[arguments.byte_offset()..];
    let data = remaining
        .strip_prefix("\r\n")
        .or_else(|| remaining.strip_prefix(char::is_whitespace))
        .ok_or(JsonError::PipelineArguments)?;
    if !(1..=MAX_PIPELINE_STEPS).contains(&steps.len())
        || steps
            .iter()
            .any(|step| step.trim().is_empty() || step.contains(['\r', '\n', '\0']))
        || data.trim().is_empty()
    {
        return Err(JsonError::PipelineArguments);
    }
    let mut current = Cow::Borrowed(data);
    for (index, step) in steps.iter().enumerate() {
        let query = format!("{}\n{current}", step.trim());
        let output =
            transform_json_operation(&query, false).map_err(|error| JsonError::PipelineStep {
                index: index + 1,
                cause: error.message(),
            })?;
        if output.len() > MAX_PRETTY_BYTES {
            return Err(JsonError::PipelineStep {
                index: index + 1,
                cause: JsonError::PipelineOutputTooLarge.message(),
            });
        }
        current = Cow::Owned(output);
    }
    Ok(current.into_owned())
}

fn format_json(source: &str, indent_width: Option<usize>) -> Result<String, JsonError> {
    if source.is_empty() {
        return Err(JsonError::Incomplete);
    }
    // 通过解析验证语法，再调整原文空白，保留键顺序、数字和字符串拼写。
    serde_json::from_str::<IgnoredAny>(source).map_err(json_parse_error)?;
    layout_json(source, indent_width)
}

fn strip_json_comments(source: &str) -> Result<String, JsonError> {
    let mut masked = source.as_bytes().to_vec();
    let mut quoted = false;
    let mut offset = 0;
    while let Some(&byte) = masked.get(offset) {
        match byte {
            b'\\' if quoted => {
                offset += 2;
                continue;
            }
            b'"' => quoted = !quoted,
            b'/' if !quoted => {
                let end = match masked.get(offset + 1) {
                    Some(b'/') => masked[offset + 2..]
                        .iter()
                        .position(|&byte| matches!(byte, b'\r' | b'\n'))
                        .map_or(masked.len(), |length| offset + 2 + length),
                    Some(b'*') => source[offset + 2..]
                        .find("*/")
                        .map(|length| offset + 4 + length)
                        .ok_or(JsonError::Incomplete)?,
                    _ => {
                        offset += 1;
                        continue;
                    }
                };
                // 用空白隔开相邻标记，例如 1/*说明*/2 仍是两个数字。
                for byte in &mut masked[offset..end] {
                    if !matches!(*byte, b'\r' | b'\n') {
                        *byte = b' ';
                    }
                }
                offset = end;
                continue;
            }
            _ => {}
        }
        offset += 1;
    }
    String::from_utf8(masked).map_err(|_| JsonError::Invalid)
}

fn layout_json(source: &str, indent_width: Option<usize>) -> Result<String, JsonError> {
    let mut result = String::with_capacity(source.len());
    let mut depth = 0;
    let mut previous = ' ';
    let mut quoted = false;
    let mut escaped = false;
    for ch in source.chars() {
        if quoted {
            result.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                quoted = false;
            }
            continue;
        }
        if matches!(ch, ' ' | '\t' | '\n' | '\r') {
            continue;
        }
        if let Some(indent_width) = indent_width {
            let line_break = if matches!(ch, '}' | ']') {
                depth -= 1;
                !matches!(previous, '{' | '[')
            } else {
                matches!(previous, '{' | '[' | ',')
            };
            if line_break {
                let indent = depth * indent_width;
                if result.len() + indent + 1 > MAX_PRETTY_BYTES {
                    return Err(JsonError::LayoutTooLarge);
                }
                result.push('\n');
                result.extend(std::iter::repeat_n(' ', indent));
            } else if previous == ':' {
                result.push(' ');
            }
            if matches!(ch, '{' | '[') {
                depth += 1;
            }
        }
        result.push(ch);
        quoted = ch == '"';
        previous = ch;
    }
    Ok(result)
}

fn json_parse_error(error: serde_json::Error) -> JsonError {
    if error.is_eof() {
        JsonError::Incomplete
    } else {
        JsonError::Invalid
    }
}

fn select_json_value(query: &str) -> Result<String, JsonError> {
    let (pointer, source) = pointer_and_source(query)?;
    let root = serde_json::from_str::<&RawValue>(source).map_err(json_parse_error)?;
    let value = select_json_path(root, &pointer)?;
    Ok(value.get().to_owned())
}

fn json_value_type(query: &str) -> Result<String, JsonError> {
    let (pointer, source) = pointer_and_source(query)?;
    let root = serde_json::from_str::<&RawValue>(source).map_err(json_parse_error)?;
    let value = select_json_path(root, &pointer)?;
    serde_json::to_string(json_type_name(value)).map_err(json_parse_error)
}

fn json_type_name(value: &RawValue) -> &'static str {
    match value.get().as_bytes()[0] {
        b'{' => "object",
        b'[' => "array",
        b'"' => "string",
        b't' | b'f' => "boolean",
        b'n' => "null",
        _ => "number",
    }
}

fn select_json_paths(query: &str) -> Result<String, JsonError> {
    let (pointers, source) = selected_pointers(query)?;
    let root = serde_json::from_str::<&RawValue>(source).map_err(json_parse_error)?;
    let entries = pointers
        .into_iter()
        .map(|pointer| {
            let value = select_json_path(root, &pointer)?;
            Ok((pointer, value))
        })
        .collect::<Result<Vec<_>, JsonError>>()?;
    let result = write_object_entries(entries)?;
    if result.len() > MAX_PRETTY_BYTES {
        return Err(JsonError::LayoutTooLarge);
    }
    Ok(result)
}

fn wildcard_arguments(query: &str) -> Result<(Vec<Option<String>>, &str), JsonError> {
    let (path, source) = json_value_and_source(query, JsonError::WildcardPathArguments)?;
    let segments = serde_json::from_str::<Vec<Option<String>>>(path.get())
        .map_err(|_| JsonError::WildcardPathArguments)?;
    Ok((segments, source))
}

fn select_json_wildcard(query: &str) -> Result<String, JsonError> {
    let (segments, source) = wildcard_arguments(query)?;
    json_wildcard_values(source, &segments)
}

pub(super) fn json_wildcard_values(
    source: &str,
    segments: &[Option<String>],
) -> Result<String, JsonError> {
    let mut output = String::from("[");
    visit_json_wildcard_matches(source, segments, false, |_, value| {
        let separator = if output.len() > 1 { "," } else { "" };
        let raw = value.get();
        if output.len() + separator.len() + raw.len() + 1 > MAX_PRETTY_BYTES {
            return Err(JsonError::LayoutTooLarge);
        }
        output.push_str(separator);
        output.push_str(raw);
        Ok(())
    })?;
    output.push(']');
    Ok(output)
}

fn json_wildcard_paths_csv(query: &str) -> Result<String, JsonError> {
    let (segments, source) = wildcard_arguments(query)?;
    let mut output = String::from("指针,类型,原值");
    visit_json_wildcard_matches(source, &segments, true, |pointer, value| {
        if pointer.contains('\0') {
            return Err(JsonError::CsvNullCharacter);
        }
        append_bounded_csv_row(
            &mut output,
            [pointer, json_type_name(value), value.get()].into_iter(),
            JsonError::CsvOutputTooLarge,
        )
    })?;
    Ok(output)
}

fn visit_json_wildcard_matches(
    source: &str,
    segments: &[Option<String>],
    include_pointer: bool,
    mut visit: impl FnMut(&str, &RawValue) -> Result<(), JsonError>,
) -> Result<(), JsonError> {
    let root = serde_json::from_str::<&RawValue>(source).map_err(json_parse_error)?;
    let mut pending = vec![(root, 0, None::<(usize, String)>)];
    let mut pointer = String::new();
    while let Some((value, depth, key)) = pending.pop() {
        if let Some((parent_length, key)) = key {
            pointer.truncate(parent_length);
            append_pointer_key(&mut pointer, &key);
        }
        let Some(segment) = segments.get(depth) else {
            visit(&pointer, value)?;
            continue;
        };
        let parent_length = pointer.len();
        if let Some(key) = segment {
            match select_member(value, key) {
                Ok(child) => pending.push((
                    child,
                    depth + 1,
                    include_pointer.then(|| (parent_length, key.clone())),
                )),
                Err(JsonError::MissingValue) => (),
                Err(error) => return Err(error),
            }
        } else {
            let raw = value.get();
            // 倒序入栈使弹出的子项沿用原文顺序。
            match raw.as_bytes()[0] {
                b'{' => pending.extend(read_unique_object_entries(raw)?.into_iter().rev().map(
                    |(key, child)| {
                        (
                            child,
                            depth + 1,
                            include_pointer.then_some((parent_length, key)),
                        )
                    },
                )),
                b'[' => pending.extend(read_json_array(raw)?.into_iter().enumerate().rev().map(
                    |(index, child)| {
                        (
                            child,
                            depth + 1,
                            include_pointer.then(|| (parent_length, index.to_string())),
                        )
                    },
                )),
                _ => (),
            }
        }
    }
    Ok(())
}

pub(super) fn selected_pointers(query: &str) -> Result<(Vec<String>, &str), JsonError> {
    let (pointers, source) = strings_and_source(query, JsonError::PointerSelection)?;
    if pointers.is_empty() || pointers.iter().collect::<HashSet<_>>().len() != pointers.len() {
        return Err(JsonError::PointerSelection);
    }
    Ok((pointers, source))
}

fn select_json_path<'a>(root: &'a RawValue, pointer: &str) -> Result<&'a RawValue, JsonError> {
    pointer_keys(pointer)?.try_fold(root, |value, key| select_member(value, &key?))
}

fn write_json_value(query: &str, add: bool) -> Result<String, JsonError> {
    let (pointer, query) = pointer_and_source(query)?;
    let (replacement, source) = json_value_and_source(query, JsonError::ReplacementValue)?;
    let root = serde_json::from_str::<&RawValue>(source).map_err(json_parse_error)?;
    edit_json_value(source, root, &pointer, replacement.get(), add)
}

fn transfer_json_value(query: &str, remove_source: bool) -> Result<String, JsonError> {
    let (paths, source) = strings_and_source(query, JsonError::SourceAndTargetPaths)?;
    let [from, to] = paths.as_slice() else {
        return Err(JsonError::SourceAndTargetPaths);
    };
    let root = serde_json::from_str::<&RawValue>(source).map_err(json_parse_error)?;
    transfer_value(source, root, from, to, remove_source)
}

fn transfer_value(
    source: &str,
    root: &RawValue,
    from: &str,
    to: &str,
    remove_source: bool,
) -> Result<String, JsonError> {
    if !remove_source {
        let value = select_json_path(root, from)?.get();
        return edit_json_value(source, root, to, value, true);
    }
    let from_keys = pointer_keys(from)?.collect::<Result<Vec<_>, _>>()?;
    let value = select_pointer_keys(root, &from_keys)?.get();
    let to_keys = pointer_keys(to)?.collect::<Result<Vec<_>, _>>()?;
    if from_keys == to_keys {
        return Ok(source.to_owned());
    }
    if to_keys.starts_with(&from_keys) {
        return Err(JsonError::MoveIntoDescendant);
    }
    let remaining = remove_json_value(source, root, &from_keys)?;
    let remaining_root = serde_json::from_str::<&RawValue>(&remaining).map_err(json_parse_error)?;
    edit_json_value(&remaining, remaining_root, to, value, true)
}

#[derive(serde::Deserialize)]
struct JsonPatchStep<'a> {
    op: String,
    path: String,
    from: Option<String>,
    #[serde(borrow, default, deserialize_with = "present_patch_value")]
    value: Option<&'a RawValue>,
}

fn present_patch_value<'de, D>(deserializer: D) -> Result<Option<&'de RawValue>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    serde::Deserialize::deserialize(deserializer).map(Some)
}

fn apply_json_patch(source: &str) -> Result<String, JsonError> {
    let (patch, source) = json_value_and_source(source, JsonError::PatchArguments)?;
    let steps = read_json_array(patch.get()).map_err(|_| JsonError::PatchArguments)?;
    if steps.len() > MAX_PATCH_STEPS {
        return Err(JsonError::PatchLimit);
    }
    serde_json::from_str::<&RawValue>(source).map_err(json_parse_error)?;
    let mut document = Cow::Borrowed(source);
    for (index, step) in steps.into_iter().enumerate() {
        let result = serde_json::from_str::<JsonPatchStep<'_>>(step.get())
            .map_err(|_| JsonError::PatchArguments)
            .and_then(|step| apply_patch_step(&document, step))
            .map_err(|error| JsonError::PatchStep {
                index: index + 1,
                cause: error.message(),
            })?;
        if let Some(result) = result {
            if result.len() > MAX_PRETTY_BYTES {
                return Err(JsonError::PatchStep {
                    index: index + 1,
                    cause: JsonError::PatchOutputLimit.message(),
                });
            }
            document = Cow::Owned(result);
        }
    }
    Ok(document.into_owned())
}

enum MergePatchMember<'a> {
    Original(&'a RawValue),
    Change(Option<&'a RawValue>, &'a RawValue),
}

fn apply_json_merge_patch(source: &str) -> Result<String, JsonError> {
    let (patch, source) = json_value_and_source(source, JsonError::MergePatchArguments)?;
    let original = serde_json::from_str::<&RawValue>(source).map_err(json_parse_error)?;
    let mut output = String::new();
    write_merge_patch(Some(original), patch, MAX_MERGE_PATCH_DEPTH, &mut output)?;
    Ok(output)
}

fn write_merge_patch(
    target: Option<&RawValue>,
    patch: &RawValue,
    remaining_depth: usize,
    output: &mut String,
) -> Result<(), JsonError> {
    if patch.get().starts_with('{') {
        let depth = remaining_depth
            .checked_sub(1)
            .ok_or(JsonError::MergePatchDepth)?;
        let changes = read_unique_object_entries(patch.get())?;
        let mut by_name: HashMap<_, _> = changes
            .iter()
            .map(|(name, value)| (name.as_str(), *value))
            .collect();
        let original = target
            .filter(|target| target.get().starts_with('{'))
            .map(|target| read_unique_object_entries(target.get()))
            .transpose()?
            .unwrap_or_default();
        let mut members = Vec::with_capacity(original.len() + changes.len());
        for (name, value) in original {
            let member = match by_name.remove(name.as_str()) {
                Some(change) if change.get() == "null" => continue,
                Some(change) => MergePatchMember::Change(Some(value), change),
                None => MergePatchMember::Original(value),
            };
            members.push((name, member));
        }
        for (name, change) in &changes {
            if by_name.contains_key(name.as_str()) && change.get() != "null" {
                members.push((name.clone(), MergePatchMember::Change(None, change)));
            }
        }
        output.push('{');
        for (index, (key, member)) in members.into_iter().enumerate() {
            if index > 0 {
                output.push(',');
            }
            output.push_str(&serde_json::to_string(&key).expect("JSON 键名可以序列化"));
            output.push(':');
            match member {
                MergePatchMember::Original(value) => output.push_str(value.get()),
                MergePatchMember::Change(value, change) => {
                    write_merge_patch(value, change, depth, output)?;
                }
            }
            if output.len() > MAX_PRETTY_BYTES {
                return Err(JsonError::MergePatchOutputLimit);
            }
        }
        output.push('}');
    } else {
        output.push_str(patch.get());
    }
    if output.len() > MAX_PRETTY_BYTES {
        return Err(JsonError::MergePatchOutputLimit);
    }
    Ok(())
}

fn apply_patch_step(source: &str, step: JsonPatchStep<'_>) -> Result<Option<String>, JsonError> {
    let root = serde_json::from_str::<&RawValue>(source).map_err(json_parse_error)?;
    let JsonPatchStep {
        op,
        path,
        from,
        value,
    } = step;
    let value = || value.ok_or(JsonError::PatchArguments);
    let from = || from.as_deref().ok_or(JsonError::PatchArguments);
    match op.as_str() {
        "add" | "replace" => {
            edit_json_value(source, root, &path, value()?.get(), op == "add").map(Some)
        }
        "remove" => {
            let keys = pointer_keys(&path)?.collect::<Result<Vec<_>, _>>()?;
            remove_json_value(source, root, &keys).map(Some)
        }
        "copy" | "move" => transfer_value(source, root, from()?, &path, op == "move").map(Some),
        "test" => {
            let actual = select_json_path(root, &path)?;
            let mut matches = true;
            compare_json_trees(actual, value()?, |_| {
                matches = false;
                Ok(false)
            })?;
            if matches {
                Ok(None)
            } else {
                Err(JsonError::PatchTestFailed)
            }
        }
        _ => Err(JsonError::PatchOperation),
    }
}

fn edit_json_value(
    source: &str,
    root: &RawValue,
    pointer: &str,
    replacement: &str,
    add: bool,
) -> Result<String, JsonError> {
    let (range, value) = if add && !pointer.is_empty() {
        let keys = pointer_keys(pointer)?.collect::<Result<Vec<_>, _>>()?;
        let (key, ancestors) = keys.split_last().ok_or(JsonError::InvalidPointer)?;
        let parent = select_pointer_keys(root, ancestors)?.get();
        json_addition(source, parent, key, replacement)?
    } else {
        let selected = select_json_path(root, pointer)?.get();
        (
            raw_value_span(source, selected)?,
            Cow::Borrowed(replacement),
        )
    };
    let mut output = source.to_owned();
    output.replace_range(range, &value);
    Ok(output)
}

fn json_addition<'a>(
    source: &str,
    parent: &str,
    key: &str,
    value: &'a str,
) -> Result<(Range<usize>, Cow<'a, str>), JsonError> {
    let (empty, member) = match parent.as_bytes()[0] {
        b'[' => {
            let items = read_json_array(parent)?;
            let index = if key == "-" {
                items.len()
            } else {
                json_array_index(key)?
            };
            if let Some(next) = items.get(index) {
                let start = raw_value_span(source, next.get())?.start;
                return Ok((start..start, Cow::Owned(format!("{value},"))));
            }
            if index != items.len() {
                return Err(JsonError::MissingValue);
            }
            (items.is_empty(), Cow::Borrowed(value))
        }
        b'{' => {
            let entries = read_object_entries(parent)?;
            let mut existing = entries.iter().filter(|(name, _)| name == key);
            if let Some((_, existing_value)) = existing.next() {
                if existing.next().is_some() {
                    return Err(JsonError::DuplicateKey);
                }
                return Ok((
                    raw_value_span(source, existing_value.get())?,
                    Cow::Borrowed(value),
                ));
            }
            let key = serde_json::to_string(key).expect("对象键可以序列化为 JSON 字符串");
            (entries.is_empty(), Cow::Owned(format!("{key}:{value}")))
        }
        _ => return Err(JsonError::MissingValue),
    };
    let before_close = parent[..parent.len() - 1].trim_end_matches([' ', '\t', '\r', '\n']);
    let end = raw_value_span(source, parent)?.start + before_close.len();
    let member = if empty {
        member
    } else {
        Cow::Owned(format!(",{member}"))
    };
    Ok((end..end, member))
}

fn raw_value_span(source: &str, selected: &str) -> Result<Range<usize>, JsonError> {
    // 借用的 RawValue 直接指向输入切片，可以用字节位置保留其余原文。
    let start = selected
        .as_ptr()
        .addr()
        .checked_sub(source.as_ptr().addr())
        .ok_or(JsonError::Invalid)?;
    let end = start
        .checked_add(selected.len())
        .ok_or(JsonError::Invalid)?;
    let range = start..end;
    (source.get(range.clone()) == Some(selected))
        .then_some(range)
        .ok_or(JsonError::Invalid)
}

fn remove_json_path(query: &str) -> Result<String, JsonError> {
    let (pointer, source) = pointer_and_source(query)?;
    let keys = pointer_keys(&pointer)?.collect::<Result<Vec<_>, _>>()?;
    if keys.is_empty() {
        return Err(JsonError::RemoveRoot);
    }
    let root = serde_json::from_str::<&RawValue>(source).map_err(json_parse_error)?;
    remove_json_value(source, root, &keys)
}

fn remove_json_value(source: &str, root: &RawValue, keys: &[String]) -> Result<String, JsonError> {
    let (key, ancestors) = keys.split_last().ok_or(JsonError::RemoveRoot)?;
    let parent = select_pointer_keys(root, ancestors)?.get();
    let (spans, index) = match parent.as_bytes()[0] {
        b'[' => {
            let index = json_array_index(key)?;
            let items = read_json_array(parent)?;
            let spans = items
                .into_iter()
                .map(|item| raw_value_span(source, item.get()))
                .collect::<Result<Vec<_>, _>>()?;
            (spans, index)
        }
        b'{' => object_member_spans(source, parent, key)?,
        _ => return Err(JsonError::MissingValue),
    };
    let selected = spans.get(index).ok_or(JsonError::MissingValue)?;
    let range = if let Some(next) = spans.get(index + 1) {
        selected.start..next.start
    } else if index > 0 {
        spans[index - 1].end..selected.end
    } else {
        selected.clone()
    };
    let mut output = source.to_owned();
    output.replace_range(range, "");
    Ok(output)
}

fn object_member_spans(
    source: &str,
    parent: &str,
    key: &str,
) -> Result<(Vec<Range<usize>>, usize), JsonError> {
    let mut spans = Vec::new();
    let mut found = None;
    let mut previous = raw_value_span(source, parent)?.start + 1;
    for (name, value) in read_object_entries(parent)? {
        if name == key && found.replace(spans.len()).is_some() {
            return Err(JsonError::DuplicateKey);
        }
        let value = raw_value_span(source, value.get())?;
        // 上一项的值到本项的值之间，第一个双引号开始本项的键。
        let name_offset = source
            .get(previous..value.start)
            .and_then(|prefix| prefix.find('"'))
            .ok_or(JsonError::Invalid)?;
        spans.push(previous + name_offset..value.end);
        previous = value.end;
    }
    Ok((spans, found.ok_or(JsonError::MissingValue)?))
}

fn select_json_column(query: &str) -> Result<String, JsonError> {
    let (pointer, source) = pointer_and_source(query)?;
    let items = read_json_array(source)?;
    let keys = pointer_keys(&pointer)?.collect::<Result<Vec<_>, _>>()?;
    let values = items
        .into_iter()
        .map(|item| select_pointer_keys(item, &keys))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(serde_json::to_string(&values).expect("有效的原始 JSON 值可以直接序列化"))
}

fn flatten_json_arrays(query: &str) -> Result<String, JsonError> {
    let (pointer, source) = pointer_and_source(query)?;
    let keys = pointer_keys(&pointer)?.collect::<Result<Vec<_>, _>>()?;
    let mut values = Vec::new();
    for item in read_json_array(source)? {
        let selected = select_pointer_keys(item, &keys)?;
        if !selected.get().starts_with('[') {
            return Err(JsonError::ExpectedArrayAtPointer);
        }
        values.extend(read_json_array(selected.get())?);
    }
    Ok(serde_json::to_string(&values).expect("有效的原始 JSON 值可以直接序列化"))
}

fn filter_json_by_pointer(query: &str, keep_present: bool) -> Result<String, JsonError> {
    let (pointer, source) = pointer_and_source(query)?;
    filter_json_items(&pointer, source, keep_present, |value| Ok(value.is_some()))
}

fn filter_json_by_scalar(query: &str, keep_match: bool) -> Result<String, JsonError> {
    let (pointer, query) = pointer_and_source(query)?;
    let (value, source) = json_value_and_source(query, JsonError::FilterValue)?;
    let expected = json_scalar(value)?.ok_or(JsonError::FilterValue)?;
    filter_json_items(&pointer, source, keep_match, |value| {
        Ok(match value {
            Some(value) => json_scalar(value)?.is_some_and(|scalar| scalar == expected),
            None => false,
        })
    })
}

fn json_value_and_source(query: &str, invalid: JsonError) -> Result<(&RawValue, &str), JsonError> {
    let mut values = serde_json::Deserializer::from_str(query).into_iter::<&RawValue>();
    let value = values
        .next()
        .ok_or(JsonError::Incomplete)?
        .map_err(|error| {
            if error.is_eof() {
                JsonError::Incomplete
            } else {
                invalid
            }
        })?;
    let source = &query[values.byte_offset()..];
    if source.is_empty() {
        return Err(JsonError::Incomplete);
    }
    if !source.starts_with(char::is_whitespace) {
        return Err(invalid);
    }
    Ok((value, source.trim_start()))
}

fn filter_json_numbers(query: &str) -> Result<String, JsonError> {
    let (pointer, query) = pointer_and_source(query)?;
    let (operator, rest) = query
        .split_once(char::is_whitespace)
        .ok_or(JsonError::Incomplete)?;
    let comparison = NumberComparison::parse(operator).ok_or(JsonError::NumberFilter)?;
    let (threshold, source) = rest
        .trim_start()
        .split_once(char::is_whitespace)
        .ok_or(JsonError::Incomplete)?;
    let threshold: &RawValue =
        serde_json::from_str(threshold).map_err(|_| JsonError::NumberFilter)?;
    let Some(JsonScalar::Number(threshold)) = json_scalar(threshold)? else {
        return Err(JsonError::NumberFilter);
    };
    filter_json_items(&pointer, source.trim_start(), true, |value| {
        let Some(value) = value else {
            return Ok(false);
        };
        Ok(matches!(
            json_scalar(value)?,
            Some(JsonScalar::Number(number)) if comparison.accepts(number.compare(&threshold))
        ))
    })
}

struct JsonCondition {
    keys: Vec<String>,
    comparison: NumberComparison,
    expected: JsonScalar,
}

impl JsonCondition {
    fn matches(&self, item: &RawValue) -> Result<bool, JsonError> {
        let Some(value) = optional_pointer_value(item, &self.keys)? else {
            return Ok(false);
        };
        let Some(actual) = json_scalar(value)? else {
            return Ok(false);
        };
        Ok(match (&actual, &self.expected) {
            (JsonScalar::Number(actual), JsonScalar::Number(expected)) => {
                self.comparison.accepts(actual.compare(expected))
            }
            _ if matches!(self.comparison, NumberComparison::Equal) => actual == self.expected,
            _ if matches!(self.comparison, NumberComparison::Different) => actual != self.expected,
            _ => false,
        })
    }
}

enum ConditionRecords {
    Array,
    Lines,
}

fn filter_json_conditions(
    query: &str,
    match_all: bool,
    records: ConditionRecords,
) -> Result<String, JsonError> {
    let (input, source) = json_value_and_source(query, JsonError::FilterConditions)?;
    let conditions = serde_json::from_str::<Vec<(String, String, &RawValue)>>(input.get())
        .map_err(|_| JsonError::FilterConditions)?;
    if !(1..=MAX_FILTER_CONDITIONS).contains(&conditions.len()) {
        return Err(JsonError::FilterConditions);
    }
    let conditions = conditions
        .into_iter()
        .map(|(pointer, comparison, raw)| {
            let keys = pointer_keys(&pointer)?.collect::<Result<Vec<_>, _>>()?;
            let comparison =
                NumberComparison::parse(&comparison).ok_or(JsonError::FilterConditions)?;
            let expected = json_scalar(raw)?.ok_or(JsonError::FilterConditions)?;
            if !matches!(
                comparison,
                NumberComparison::Equal | NumberComparison::Different
            ) && !matches!(expected, JsonScalar::Number(_))
            {
                return Err(JsonError::NumberFilter);
            }
            Ok(JsonCondition {
                keys,
                comparison,
                expected,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let keep = |item: &RawValue| {
        for condition in &conditions {
            let matches = condition.matches(item)?;
            if matches != match_all {
                return Ok(matches);
            }
        }
        Ok(match_all)
    };
    match records {
        ConditionRecords::Array => retain_json_items(read_json_array(source)?, keep),
        ConditionRecords::Lines => {
            let mut matched = Vec::new();
            for (_, line) in json_lines(source) {
                let item = parse_json_line(line)?;
                if keep(item)? {
                    matched.push(item.get());
                }
            }
            Ok(matched.join("\n"))
        }
    }
}

fn deduplicate_json_by_scalar(query: &str) -> Result<String, JsonError> {
    let (pointer, source) = pointer_and_source(query)?;
    let mut seen = HashSet::new();
    filter_json_items(&pointer, source, true, |value| {
        let value = value.ok_or(JsonError::MissingValue)?;
        let scalar = json_scalar(value)?.ok_or(JsonError::ExpectedScalar)?;
        Ok(seen.insert(scalar))
    })
}

fn distinct_json_paths_and_source(
    query: &str,
    minimum: usize,
    invalid: JsonError,
) -> Result<(Vec<Vec<String>>, &str), JsonError> {
    let (input, source) = json_value_and_source(query, invalid)?;
    let pointers = serde_json::from_str::<Vec<String>>(input.get()).map_err(|_| invalid)?;
    if !(minimum..=MAX_UNIQUE_PATHS).contains(&pointers.len()) {
        return Err(invalid);
    }
    let mut distinct = HashSet::new();
    let paths = pointers
        .iter()
        .map(|pointer| {
            if !distinct.insert(pointer) {
                return Err(invalid);
            }
            pointer_keys(pointer)?.collect::<Result<Vec<_>, _>>()
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((paths, source))
}

fn json_scalar_key(item: &RawValue, paths: &[Vec<String>]) -> Result<Vec<JsonScalar>, JsonError> {
    paths
        .iter()
        .map(|path| {
            let value = select_pointer_keys(item, path)?;
            json_scalar(value)?.ok_or(JsonError::ExpectedScalar)
        })
        .collect()
}

fn deduplicate_json_lines(query: &str) -> Result<String, JsonError> {
    let (paths, source) = distinct_json_paths_and_source(query, 1, JsonError::LineUniquePaths)?;
    let mut seen = HashSet::new();
    let mut retained = Vec::new();
    for item in read_json_lines(source)? {
        if seen.insert(json_scalar_key(item, &paths)?) {
            retained.push(item.get());
        }
    }
    Ok(retained.join("\n"))
}

fn duplicate_json_line_positions(query: &str) -> Result<String, JsonError> {
    let (paths, source) = distinct_json_paths_and_source(query, 1, JsonError::LineUniquePaths)?;
    let records = json_lines(source)
        .map(|(line, text)| {
            let item = parse_json_line(text)?;
            Ok((line, item.get(), json_scalar_key(item, &paths)?))
        })
        .collect::<Result<Vec<_>, JsonError>>()?;
    let mut counts = HashMap::new();
    for (_, _, key) in &records {
        *counts.entry(key).or_insert(0usize) += 1;
    }
    let mut csv = String::from("行号,出现次数,原始JSON");
    for (line, item, key) in &records {
        let count = counts[key];
        if count > 1 {
            append_bounded_csv_row(
                &mut csv,
                [line.to_string().as_str(), count.to_string().as_str(), *item].into_iter(),
                JsonError::CsvOutputTooLarge,
            )?;
        }
    }
    Ok(csv)
}

fn retain_last_json_by_paths(query: &str, json_lines: bool) -> Result<String, JsonError> {
    let (minimum, invalid) = if json_lines {
        (1, JsonError::LineUniquePaths)
    } else {
        (2, JsonError::UniquePaths)
    };
    let (paths, source) = distinct_json_paths_and_source(query, minimum, invalid)?;
    let items = if json_lines {
        read_json_lines(source)?
    } else {
        read_json_array(source)?
    };
    let mut last_position = HashMap::new();
    for (position, &item) in items.iter().enumerate() {
        last_position.insert(json_scalar_key(item, &paths)?, position);
    }
    let mut positions: Vec<_> = last_position.into_values().collect();
    positions.sort_unstable();
    let retained = positions.into_iter().map(|position| items[position]);
    if json_lines {
        return Ok(retained.map(RawValue::get).collect::<Vec<_>>().join("\n"));
    }
    Ok(serde_json::to_string(&retained.collect::<Vec<_>>())
        .expect("有效的原始 JSON 值可以直接序列化"))
}

fn match_json_by_pointers(query: &str, duplicates_only: bool) -> Result<String, JsonError> {
    let (paths, source) = distinct_json_paths_and_source(query, 2, JsonError::UniquePaths)?;
    let key_for = |item: &RawValue| json_scalar_key(item, &paths);
    let items = read_json_array(source)?;
    if !duplicates_only {
        let mut seen = HashSet::new();
        return retain_json_items(items, |item| Ok(seen.insert(key_for(item)?)));
    }
    let keys = items
        .iter()
        .map(|item| key_for(item))
        .collect::<Result<Vec<_>, _>>()?;
    let mut first_occurrences = HashSet::new();
    let repeated: HashSet<_> = keys
        .iter()
        .filter(|key| !first_occurrences.insert(*key))
        .collect();
    let retained: Vec<_> = items
        .into_iter()
        .zip(&keys)
        .filter_map(|(item, key)| repeated.contains(&key).then_some(item))
        .collect();
    Ok(serde_json::to_string(&retained).expect("有效的原始 JSON 值可以直接序列化"))
}

fn count_json_by_scalar(query: &str) -> Result<String, JsonError> {
    #[derive(serde::Serialize)]
    struct Group<'a> {
        #[serde(rename = "值")]
        value: JsonGroupValue<'a>,
        #[serde(rename = "计数")]
        count: usize,
    }

    write_json_groups_from_query(
        query,
        |value| Group { value, count: 0 },
        |group, _| {
            group.count += 1;
            Ok(())
        },
    )
}

fn count_json_line_groups_csv(query: &str) -> Result<String, JsonError> {
    let (pointer, source) = pointer_and_source(query)?;
    let groups = collect_json_groups(
        &pointer,
        source,
        read_json_lines,
        |value| (value, 0usize),
        |(_, count), _| {
            *count += 1;
            Ok(())
        },
    )?;
    let mut output = String::from("值,类型,计数");
    for (value, count) in groups {
        append_bounded_csv_row(
            &mut output,
            [
                value.get(),
                json_type_name(value),
                count.to_string().as_str(),
            ]
            .into_iter(),
            JsonError::CsvOutputTooLarge,
        )?;
    }
    Ok(output)
}

fn group_json_records(query: &str) -> Result<String, JsonError> {
    #[derive(serde::Serialize)]
    struct Group<'a> {
        #[serde(rename = "值")]
        value: JsonGroupValue<'a>,
        #[serde(rename = "项目")]
        records: Vec<&'a RawValue>,
    }

    write_json_groups_from_query(
        query,
        |value| Group {
            value,
            records: Vec::new(),
        },
        |group, item| {
            group.records.push(item);
            Ok(())
        },
    )
}

#[derive(serde::Serialize)]
#[serde(untagged)]
enum JsonGroupValue<'a> {
    Scalar(&'a RawValue),
    Values(Vec<&'a RawValue>),
}

fn write_json_groups_from_query<'a, Group: serde::Serialize>(
    query: &'a str,
    create: impl Fn(JsonGroupValue<'a>) -> Group,
    append: impl Fn(&mut Group, &'a RawValue) -> Result<(), JsonError>,
) -> Result<String, JsonError> {
    let multiple_paths = query.trim_start().starts_with('[');
    let groups = if multiple_paths {
        let (paths, source) = distinct_json_paths_and_source(query, 2, JsonError::UniquePaths)?;
        collect_json_groups_by(
            read_json_array(source)?,
            |item| {
                let mut key = Vec::with_capacity(paths.len());
                let mut values = Vec::with_capacity(paths.len());
                for path in &paths {
                    let value = select_pointer_keys(item, path)?;
                    key.push(json_scalar(value)?.ok_or(JsonError::ExpectedScalar)?);
                    values.push(value);
                }
                Ok((key, values))
            },
            |values| create(JsonGroupValue::Values(values)),
            append,
        )?
    } else {
        let (pointer, source) = pointer_and_source(query)?;
        collect_json_groups(
            &pointer,
            source,
            read_json_array,
            |value| create(JsonGroupValue::Scalar(value)),
            append,
        )?
    };
    let output = serde_json::to_string(&groups).expect("含原始 JSON 值的分组可以直接序列化");
    if multiple_paths && output.len() > MAX_PRETTY_BYTES {
        return Err(JsonError::ObjectOutputTooLarge);
    }
    Ok(output)
}

#[derive(serde::Serialize)]
struct JsonGroupSummary<'a> {
    #[serde(rename = "值")]
    value: &'a RawValue,
    #[serde(rename = "计数")]
    count: usize,
    #[serde(rename = "合计")]
    sum: String,
    #[serde(rename = "平均")]
    mean: String,
}

fn summarize_json_groups(query: &str) -> Result<String, JsonError> {
    let summaries = json_group_summaries(query, read_json_array)?;
    Ok(serde_json::to_string(&summaries).expect("有效 JSON 值和字符串可以序列化"))
}

fn summarize_json_line_groups_csv(query: &str) -> Result<String, JsonError> {
    let summaries = json_group_summaries(query, read_json_lines)?;
    let mut output = String::from("值,类型,计数,合计,平均");
    for summary in summaries {
        append_bounded_csv_row(
            &mut output,
            [
                summary.value.get(),
                json_type_name(summary.value),
                summary.count.to_string().as_str(),
                &summary.sum,
                &summary.mean,
            ]
            .into_iter(),
            JsonError::CsvOutputTooLarge,
        )?;
    }
    Ok(output)
}

fn json_group_summaries(
    query: &str,
    read_items: fn(&str) -> Result<Vec<&RawValue>, JsonError>,
) -> Result<Vec<JsonGroupSummary<'_>>, JsonError> {
    struct Group<'a> {
        value: &'a RawValue,
        count: usize,
        sum: BigRational,
    }

    let (pointers, source) = strings_and_source(query, JsonError::GroupStatisticsPaths)?;
    let [group, amount] = pointers.as_slice() else {
        return Err(JsonError::GroupStatisticsPaths);
    };
    let amount_keys = pointer_keys(amount)?.collect::<Result<Vec<_>, _>>()?;
    let groups = collect_json_groups(
        group,
        source,
        read_items,
        |value| Group {
            value,
            count: 0,
            sum: BigRational::default(),
        },
        |group, item| {
            group.sum += json_group_number(select_pointer_keys(item, &amount_keys)?)?;
            ensure_exact_size(&group.sum).map_err(|_| JsonError::GroupStatisticsRange)?;
            group.count += 1;
            Ok(())
        },
    )?;
    groups
        .into_iter()
        .map(|group| {
            let mean = &group.sum / BigInt::from(group.count);
            ensure_exact_size(&mean).map_err(|_| JsonError::GroupStatisticsRange)?;
            Ok(JsonGroupSummary {
                value: group.value,
                count: group.count,
                sum: group.sum.to_string(),
                mean: mean.to_string(),
            })
        })
        .collect()
}

fn json_group_number(value: &RawValue) -> Result<BigRational, JsonError> {
    let value = value.get();
    if !matches!(value.as_bytes()[0], b'-' | b'0'..=b'9') {
        return Err(JsonError::GroupStatisticsNumber);
    }
    parse_fraction_literal(value).map_err(|_| JsonError::GroupStatisticsRange)
}

#[derive(serde::Serialize)]
struct JsonGroupMedian<'a> {
    #[serde(rename = "值")]
    value: &'a RawValue,
    #[serde(skip)]
    count: usize,
    #[serde(rename = "中位数")]
    median: String,
}

fn median_json_groups(query: &str) -> Result<String, JsonError> {
    let mut output = String::from("[");
    for (index, group) in json_group_medians(query, read_json_array)?.enumerate() {
        let group = group?;
        let item = serde_json::to_string(&group).expect("有效 JSON 值和字符串可以序列化");
        let separator = if index == 0 { "" } else { "," };
        if output.len() + separator.len() + item.len() + 1 > MAX_PRETTY_BYTES {
            return Err(JsonError::ObjectOutputTooLarge);
        }
        output.push_str(separator);
        output.push_str(&item);
    }
    output.push(']');
    Ok(output)
}

fn median_json_line_groups_csv(query: &str) -> Result<String, JsonError> {
    let mut output = String::from("值,类型,计数,中位数");
    for group in json_group_medians(query, read_json_lines)? {
        let group = group?;
        append_bounded_csv_row(
            &mut output,
            [
                group.value.get(),
                json_type_name(group.value),
                group.count.to_string().as_str(),
                &group.median,
            ]
            .into_iter(),
            JsonError::CsvOutputTooLarge,
        )?;
    }
    Ok(output)
}

fn json_group_medians(
    query: &str,
    read_items: fn(&str) -> Result<Vec<&RawValue>, JsonError>,
) -> Result<impl Iterator<Item = Result<JsonGroupMedian<'_>, JsonError>>, JsonError> {
    let (pointers, source) = strings_and_source(query, JsonError::GroupStatisticsPaths)?;
    let [group, amount] = pointers.as_slice() else {
        return Err(JsonError::GroupStatisticsPaths);
    };
    let amount_keys = pointer_keys(amount)?.collect::<Result<Vec<_>, _>>()?;
    let groups = collect_json_groups(
        group,
        source,
        read_items,
        |value| (value, Vec::new()),
        |(_, numbers), item| {
            if numbers.len() == MAX_VALUES {
                return Err(JsonError::GroupStatisticsLimit);
            }
            numbers.push(json_group_number(select_pointer_keys(item, &amount_keys)?)?);
            Ok(())
        },
    )?;
    Ok(groups.into_iter().map(|(value, mut numbers)| {
        numbers.sort_unstable();
        let median = median_sorted_values(&numbers).map_err(|_| JsonError::GroupStatisticsRange)?;
        Ok(JsonGroupMedian {
            value,
            count: numbers.len(),
            median: median.to_string(),
        })
    }))
}

fn collect_json_groups<'a, Group>(
    pointer: &str,
    source: &'a str,
    read_items: fn(&str) -> Result<Vec<&RawValue>, JsonError>,
    create: impl Fn(&'a RawValue) -> Group,
    append: impl Fn(&mut Group, &'a RawValue) -> Result<(), JsonError>,
) -> Result<Vec<Group>, JsonError> {
    let keys = pointer_keys(pointer)?.collect::<Result<Vec<_>, _>>()?;
    collect_json_groups_by(
        read_items(source)?,
        |item| {
            let value = select_pointer_keys(item, &keys)?;
            let scalar = json_scalar(value)?.ok_or(JsonError::ExpectedScalar)?;
            Ok((scalar, value))
        },
        create,
        append,
    )
}

fn collect_json_groups_by<'a, Key: Eq + std::hash::Hash, Value, Group>(
    items: Vec<&'a RawValue>,
    key_for: impl Fn(&'a RawValue) -> Result<(Key, Value), JsonError>,
    create: impl Fn(Value) -> Group,
    append: impl Fn(&mut Group, &'a RawValue) -> Result<(), JsonError>,
) -> Result<Vec<Group>, JsonError> {
    let mut positions = HashMap::new();
    let mut groups = Vec::new();
    for item in items {
        let (key, value) = key_for(item)?;
        let position = *positions.entry(key).or_insert_with(|| {
            let position = groups.len();
            groups.push(create(value));
            position
        });
        append(&mut groups[position], item)?;
    }
    Ok(groups)
}

fn sort_json_numbers(query: &str, descending: bool) -> Result<String, JsonError> {
    sort_json_by(query, descending, json_sort_number, JsonNumber::compare)
}

fn sort_json_text(query: &str, descending: bool) -> Result<String, JsonError> {
    sort_json_by(query, descending, json_sort_text, |left, right| {
        compare_natural(left, right)
    })
}

fn json_sort_number(value: &RawValue) -> Result<JsonNumber, JsonError> {
    match json_scalar(value)? {
        Some(JsonScalar::Number(number)) => Ok(number),
        _ => Err(JsonError::ExpectedNumber),
    }
}

fn json_sort_text(value: &RawValue) -> Result<String, JsonError> {
    match json_scalar(value)? {
        Some(JsonScalar::String(text)) => Ok(text),
        _ => Err(JsonError::ExpectedString),
    }
}

fn sort_json_by_pointers(query: &str) -> Result<String, JsonError> {
    let (input, source) = json_value_and_source(query, JsonError::SortFields)?;
    let rules = serde_json::from_str::<Vec<(String, FieldSortOrder)>>(input.get())
        .map_err(|_| JsonError::SortFields)?;
    if !(1..=MAX_SORT_FIELDS).contains(&rules.len()) {
        return Err(JsonError::SortFields);
    }
    let mut seen = HashSet::new();
    let fields = rules
        .iter()
        .map(|(pointer, order)| {
            if !seen.insert(pointer) {
                return Err(JsonError::SortFields);
            }
            let keys = pointer_keys(pointer)?.collect::<Result<Vec<_>, _>>()?;
            Ok((keys, *order))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut items = read_json_array(source)?;
    // 稳定排序先处理低优先级路径，再处理高优先级路径。
    for (keys, order) in fields.into_iter().rev() {
        let descending = order.descending();
        items = if order.numeric() {
            sort_json_items(
                items,
                &keys,
                descending,
                json_sort_number,
                JsonNumber::compare,
            )?
        } else {
            sort_json_items(items, &keys, descending, json_sort_text, |left, right| {
                compare_natural(left, right)
            })?
        };
    }
    Ok(serde_json::to_string(&items).expect("有效的原始 JSON 值可以直接序列化"))
}

fn sort_json_by<Key>(
    query: &str,
    descending: bool,
    key: impl Fn(&RawValue) -> Result<Key, JsonError>,
    compare: impl Fn(&Key, &Key) -> Ordering,
) -> Result<String, JsonError> {
    let (pointer, source) = pointer_and_source(query)?;
    let keys = pointer_keys(&pointer)?.collect::<Result<Vec<_>, _>>()?;
    let values = sort_json_items(read_json_array(source)?, &keys, descending, key, compare)?;
    Ok(serde_json::to_string(&values).expect("有效的原始 JSON 值可以直接序列化"))
}

fn sort_json_items<'a, Key>(
    items: Vec<&'a RawValue>,
    keys: &[String],
    descending: bool,
    key: impl Fn(&RawValue) -> Result<Key, JsonError>,
    compare: impl Fn(&Key, &Key) -> Ordering,
) -> Result<Vec<&'a RawValue>, JsonError> {
    let mut items = items
        .into_iter()
        .map(|item| {
            let value = select_pointer_keys(item, keys)?;
            Ok((key(value)?, item))
        })
        .collect::<Result<Vec<_>, _>>()?;
    items.sort_by(|(left, _), (right, _)| {
        if descending {
            compare(right, left)
        } else {
            compare(left, right)
        }
    });
    Ok(items.into_iter().map(|(_, item)| item).collect())
}

#[derive(PartialEq, Eq, Hash)]
struct JsonNumber {
    sign: i8,
    decimal_position: BigInt,
    digits: String,
}

impl JsonNumber {
    fn compare(&self, other: &Self) -> Ordering {
        self.sign.cmp(&other.sign).then_with(|| {
            // 相同位置的有效数位可直接比较，末尾的零已在规范化时移除。
            let magnitude = self
                .decimal_position
                .cmp(&other.decimal_position)
                .then_with(|| self.digits.cmp(&other.digits));
            if self.sign < 0 {
                magnitude.reverse()
            } else {
                magnitude
            }
        })
    }
}

fn filter_json_items(
    pointer: &str,
    source: &str,
    keep_match: bool,
    mut matches_value: impl FnMut(Option<&RawValue>) -> Result<bool, JsonError>,
) -> Result<String, JsonError> {
    let items = read_json_array(source)?;
    let keys = pointer_keys(pointer)?.collect::<Result<Vec<_>, _>>()?;
    retain_json_items(items, |item| {
        Ok(matches_value(optional_pointer_value(item, &keys)?)? == keep_match)
    })
}

fn optional_pointer_value<'a>(
    item: &'a RawValue,
    keys: &[String],
) -> Result<Option<&'a RawValue>, JsonError> {
    match select_pointer_keys(item, keys) {
        Ok(value) => Ok(Some(value)),
        Err(JsonError::MissingValue) => Ok(None),
        Err(error) => Err(error),
    }
}

fn retain_json_items(
    items: Vec<&RawValue>,
    mut retain: impl FnMut(&RawValue) -> Result<bool, JsonError>,
) -> Result<String, JsonError> {
    let mut retained = Vec::new();
    for item in items {
        if retain(item)? {
            retained.push(item);
        }
    }
    Ok(serde_json::to_string(&retained).expect("有效的原始 JSON 值可以直接序列化"))
}

#[derive(PartialEq, Eq, Hash)]
enum JsonScalar {
    String(String),
    Number(JsonNumber),
    Boolean(bool),
    Null,
}

fn json_scalar(value: &RawValue) -> Result<Option<JsonScalar>, JsonError> {
    let raw = value.get();
    let scalar = match raw.as_bytes()[0] {
        b'{' | b'[' => return Ok(None),
        b'"' => JsonScalar::String(serde_json::from_str(raw).map_err(json_parse_error)?),
        b't' | b'f' => JsonScalar::Boolean(raw == "true"),
        b'n' => JsonScalar::Null,
        _ => {
            let (mantissa, exponent) = raw.split_once(['e', 'E']).unwrap_or((raw, "0"));
            let negative = mantissa.starts_with('-');
            let mantissa = mantissa.strip_prefix('-').unwrap_or(mantissa);
            let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
            let all_digits = format!("{whole}{fraction}");
            let without_trailing_zeros = all_digits.trim_end_matches('0');
            let digits = without_trailing_zeros.trim_start_matches('0');
            let (sign, decimal_position) = if digits.is_empty() {
                (0, BigInt::default())
            } else {
                // 将十进制位置留在指数中，避免展开很长的零串。
                let exponent: BigInt = exponent.parse().map_err(|_| JsonError::Invalid)?;
                let decimal_position = exponent + BigInt::from(whole.len())
                    - BigInt::from(without_trailing_zeros.len() - digits.len());
                (if negative { -1 } else { 1 }, decimal_position)
            };
            JsonScalar::Number(JsonNumber {
                sign,
                digits: digits.to_owned(),
                decimal_position,
            })
        }
    };
    Ok(Some(scalar))
}

fn select_pointer_keys<'a>(item: &'a RawValue, keys: &[String]) -> Result<&'a RawValue, JsonError> {
    keys.iter()
        .try_fold(item, |value, key| select_member(value, key))
}

pub(super) fn read_json_array(source: &str) -> Result<Vec<&RawValue>, JsonError> {
    if !source.starts_with('[') {
        serde_json::from_str::<IgnoredAny>(source).map_err(json_parse_error)?;
        return Err(JsonError::ExpectedArray);
    }
    serde_json::from_str(source).map_err(json_parse_error)
}

fn slice_json_array(query: &str) -> Result<String, JsonError> {
    let invalid = JsonError::ArraySliceArguments;
    let (parameters, source) = json_value_and_source(query, invalid)?;
    let parameters =
        serde_json::from_str::<Vec<Option<i64>>>(parameters.get()).map_err(|_| invalid)?;
    let (start, stop, step) = match parameters.as_slice() {
        [start, stop] => (*start, *stop, 1),
        [start, stop, step] => (*start, *stop, step.unwrap_or(1)),
        _ => return Err(invalid),
    };
    if step == 0 {
        return Err(invalid);
    }
    let values = read_json_array(source)?;
    let length = values.len() as i64;
    let reverse = step < 0;
    let (minimum, maximum) = if reverse {
        (-1, length - 1)
    } else {
        (0, length)
    };
    let resolve = |bound: Option<i64>, default| {
        bound.map_or(default, |bound| {
            let index = if bound < 0 { bound + length } else { bound };
            index.clamp(minimum, maximum)
        })
    };
    let (first, last) = if reverse {
        (maximum, minimum)
    } else {
        (minimum, maximum)
    };
    let mut index = resolve(start, first);
    let stop = resolve(stop, last);
    let mut selected = Vec::new();
    while if reverse { index > stop } else { index < stop } {
        selected.push(values[index as usize].get());
        index = index.saturating_add(step);
    }
    Ok(format!("[{}]", selected.join(",")))
}

fn json_array_to_lines(source: &str) -> Result<String, JsonError> {
    let lines = read_json_array(source)?
        .into_iter()
        .map(|value| layout_json(value.get(), None))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(lines.join("\n"))
}

fn json_lines_to_array(source: &str) -> Result<String, JsonError> {
    if source.is_empty() {
        return Err(JsonError::Incomplete);
    }
    let values = json_lines(source)
        .map(|(_, line)| {
            let value = parse_json_line(line)?;
            layout_json(value.get(), None)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(format!("[{}]", values.join(",")))
}

fn json_lines_to_csv(source: &str) -> Result<String, JsonError> {
    json_object_rows_to_csv(read_json_lines(source)?).map_err(|error| match error {
        JsonError::CsvRows => JsonError::CsvLineObjects,
        _ => error,
    })
}

pub(super) fn read_json_lines(source: &str) -> Result<Vec<&RawValue>, JsonError> {
    json_lines(source)
        .map(|(_, line)| parse_json_line(line))
        .collect()
}

pub(super) fn parse_json_line(line: &str) -> Result<&RawValue, JsonError> {
    serde_json::from_str::<&RawValue>(line).map_err(|_| JsonError::InvalidLine)
}

pub(super) fn json_lines(source: &str) -> impl Iterator<Item = (usize, &str)> {
    json_lines_with_spans(source).map(|(number, _, line)| (number, line))
}

fn json_lines_with_spans(source: &str) -> impl Iterator<Item = (usize, Range<usize>, &str)> {
    let mut start = 0;
    source
        .split_inclusive('\n')
        .enumerate()
        .filter_map(move |(index, raw)| {
            let line = match raw.strip_suffix('\n') {
                Some(line) => line.strip_suffix('\r').unwrap_or(line),
                None => raw,
            };
            let range = start..start + line.len();
            start += raw.len();
            (!line.trim_matches([' ', '\t', '\r']).is_empty()).then_some((index + 1, range, line))
        })
}

fn validate_json_lines(source: &str) -> Result<InputValidationReport, JsonError> {
    let mut valid = 0;
    let mut errors = Vec::new();
    let mut error_span = None;
    for (line_number, range, line) in json_lines_with_spans(source) {
        match serde_json::from_str::<&RawValue>(line) {
            Ok(_) => valid += 1,
            Err(error) => {
                error_span.get_or_insert(range);
                let reason = json_line_error_reason(&error);
                errors.push(format!("第 {line_number} 行 · {reason}"));
            }
        }
    }
    if valid == 0 && errors.is_empty() {
        return Err(JsonError::Incomplete);
    }
    errors.insert(0, format!("{} 行有效，{} 行无效", valid, errors.len()));
    Ok(InputValidationReport {
        text: errors.join("\n"),
        error_span,
    })
}

fn validate_json_lines_csv(source: &str) -> Result<InputTableReport, JsonError> {
    let headers = "行号,状态,类型,原因";
    let mut csv = String::from(headers);
    let mut input_errors = Vec::new();
    for (line_number, range, line) in json_lines_with_spans(source) {
        let (status, kind, reason) = match serde_json::from_str::<&RawValue>(line) {
            Ok(value) => ("有效", json_type_name(value), ""),
            Err(error) => {
                input_errors.push(range);
                ("无效", "", json_line_error_reason(&error))
            }
        };
        let line_number = line_number.to_string();
        append_bounded_csv_row(
            &mut csv,
            [line_number.as_str(), status, kind, reason].into_iter(),
            JsonError::CsvOutputTooLarge,
        )?;
    }
    if csv.len() == headers.len() {
        return Err(JsonError::Incomplete);
    }
    Ok(InputTableReport { csv, input_errors })
}

fn json_line_error_reason(error: &serde_json::Error) -> &'static str {
    if error.is_eof() {
        "JSON 尚未完整"
    } else {
        "JSON 语法无效"
    }
}

fn merge_json_objects(source: &str) -> Result<String, JsonError> {
    let mut members = HashMap::new();
    for object in read_json_array(source)? {
        let raw = object.get();
        if !raw.starts_with('{') {
            return Err(JsonError::ExpectedObjectItem);
        }
        for (key, value) in read_object_entries(raw)? {
            let next_position = members.len();
            members
                .entry(key)
                .and_modify(|(_, previous)| *previous = value)
                .or_insert((next_position, value));
        }
    }
    let mut ordered: Vec<_> = members.into_iter().collect();
    ordered.sort_unstable_by_key(|(_, (position, _))| *position);
    write_object_entries(ordered.into_iter().map(|(key, (_, value))| (key, value)))
}

pub(super) fn read_unique_object_entries(
    source: &str,
) -> Result<Vec<(String, &RawValue)>, JsonError> {
    let entries = read_object_entries(source)?;
    let mut seen = HashSet::with_capacity(entries.len());
    if entries.iter().any(|(key, _)| !seen.insert(key)) {
        return Err(JsonError::DuplicateKey);
    }
    Ok(entries)
}

fn read_object_entries(source: &str) -> Result<Vec<(String, &RawValue)>, JsonError> {
    if !source.starts_with('{') {
        serde_json::from_str::<IgnoredAny>(source).map_err(json_parse_error)?;
        return Err(JsonError::ExpectedObject);
    }
    let mut parser = serde_json::Deserializer::from_str(source);
    let entries = serde::Deserializer::deserialize_map(&mut parser, ObjectEntries)
        .map_err(json_parse_error)?;
    parser.end().map_err(json_parse_error)?;
    Ok(entries)
}

enum ObjectKeyMode {
    Sort,
    Deduplicate { keep_last: bool },
    DropNull,
}

fn remove_duplicate_json_keys(query: &str) -> Result<String, JsonError> {
    let (policy, source) = query
        .split_once(char::is_whitespace)
        .ok_or(JsonError::DuplicateKeyPolicy)?;
    let keep_last = match policy.to_ascii_lowercase().as_str() {
        "留首" | "first" => false,
        "留末" | "last" => true,
        _ => return Err(JsonError::DuplicateKeyPolicy),
    };
    rewrite_object_keys(
        source.trim_start(),
        ObjectKeyMode::Deduplicate { keep_last },
    )
}

fn rewrite_object_keys(source: &str, mode: ObjectKeyMode) -> Result<String, JsonError> {
    enum Part<'a> {
        Value(&'a RawValue),
        Key(String),
        Symbol(char),
    }

    let root = serde_json::from_str::<&RawValue>(source).map_err(json_parse_error)?;
    let mut pending = vec![Part::Value(root)];
    let mut result = String::new();
    while let Some(part) = pending.pop() {
        match part {
            Part::Symbol(symbol) => result.push(symbol),
            Part::Key(key) => {
                let key = serde_json::to_string(&key).expect("JSON 键名可以序列化");
                result.push_str(&key);
            }
            Part::Value(value) => {
                let raw = value.get();
                match raw.as_bytes()[0] {
                    b'{' => {
                        let mut entries = read_object_entries(raw)?;
                        match mode {
                            ObjectKeyMode::Sort => {
                                entries.sort_unstable_by(|left, right| left.0.cmp(&right.0));
                                if entries.windows(2).any(|pair| pair[0].0 == pair[1].0) {
                                    return Err(JsonError::DuplicateKey);
                                }
                            }
                            ObjectKeyMode::Deduplicate { keep_last } => {
                                if keep_last {
                                    entries.reverse();
                                }
                                let mut seen = HashSet::with_capacity(entries.len());
                                entries.retain(|(name, _)| seen.insert(name.clone()));
                                if keep_last {
                                    entries.reverse();
                                }
                            }
                            ObjectKeyMode::DropNull => {
                                let mut seen = HashSet::with_capacity(entries.len());
                                if entries.iter().any(|(key, _)| !seen.insert(key)) {
                                    return Err(JsonError::DuplicateKey);
                                }
                                entries.retain(|(_, value)| value.get() != "null");
                            }
                        }
                        result.push('{');
                        pending.push(Part::Symbol('}'));
                        for (index, (key, child)) in entries.into_iter().enumerate().rev() {
                            pending.push(Part::Value(child));
                            pending.push(Part::Symbol(':'));
                            pending.push(Part::Key(key));
                            if index != 0 {
                                pending.push(Part::Symbol(','));
                            }
                        }
                    }
                    b'[' => {
                        let elements = read_json_array(raw)?;
                        result.push('[');
                        pending.push(Part::Symbol(']'));
                        for (index, child) in elements.into_iter().enumerate().rev() {
                            pending.push(Part::Value(child));
                            if index != 0 {
                                pending.push(Part::Symbol(','));
                            }
                        }
                    }
                    _ => result.push_str(raw),
                }
            }
        }
        if result.len() > MAX_PRETTY_BYTES {
            return Err(JsonError::LayoutTooLarge);
        }
    }
    Ok(result)
}

fn write_object_entries<'a>(
    entries: impl IntoIterator<Item = (String, &'a RawValue)>,
) -> Result<String, JsonError> {
    let entries = entries
        .into_iter()
        .map(|(key, value)| {
            let key = serde_json::to_string(&key).expect("JSON 键名可以序列化");
            let value = layout_json(value.get(), None)?;
            Ok(format!("{key}:{value}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(format!("{{{}}}", entries.join(",")))
}

fn strings_and_source(query: &str, invalid: JsonError) -> Result<(Vec<String>, &str), JsonError> {
    let mut arguments = serde_json::Deserializer::from_str(query).into_iter::<Vec<String>>();
    let names = arguments
        .next()
        .ok_or(JsonError::Incomplete)?
        .map_err(|error| {
            if error.is_eof() {
                JsonError::Incomplete
            } else {
                invalid
            }
        })?;
    let source = query[arguments.byte_offset()..].trim_start();
    if source.is_empty() {
        return Err(JsonError::Incomplete);
    }
    Ok((names, source))
}

fn rename_json_key(query: &str) -> Result<String, JsonError> {
    let (names, source) = strings_and_source(query, JsonError::KeyArguments)?;
    let [original, replacement] = names.as_slice() else {
        return Err(JsonError::KeyArguments);
    };
    let mut entries = read_object_entries(source)?;
    let mut matches = entries
        .iter()
        .enumerate()
        .filter(|(_, (key, _))| key == original);
    let (position, _) = matches.next().ok_or(JsonError::MissingKey)?;
    if matches.next().is_some()
        || (original != replacement && entries.iter().any(|(key, _)| key == replacement))
    {
        return Err(JsonError::RenameConflict);
    }
    entries[position].0.clone_from(replacement);
    write_object_entries(entries)
}

fn remove_json_keys(query: &str) -> Result<String, JsonError> {
    let (names, source) = strings_and_source(query, JsonError::KeyArguments)?;
    if names.is_empty() {
        return Err(JsonError::KeyArguments);
    }
    let names: HashSet<_> = names.into_iter().collect();
    write_object_entries(
        read_object_entries(source)?
            .into_iter()
            .filter(|(key, _)| !names.contains(key)),
    )
}

fn remove_null_json_fields(source: &str) -> Result<String, JsonError> {
    map_json_objects(source, |raw| {
        let entries = read_object_entries(raw)?;
        let mut names = HashSet::new();
        if entries.iter().any(|(key, _)| !names.insert(key)) {
            return Err(JsonError::DuplicateKey);
        }
        write_object_entries(
            entries
                .into_iter()
                .filter(|(_, value)| value.get() != "null"),
        )
    })
}

fn fill_missing_json_fields(query: &str) -> Result<String, JsonError> {
    let (argument, source) = json_value_and_source(query, JsonError::DefaultFieldArguments)?;
    let defaults = if argument.get().starts_with('{') {
        let defaults = read_object_entries(argument.get())?;
        let mut names = HashSet::new();
        if defaults.iter().any(|(name, _)| !names.insert(name)) {
            return Err(JsonError::DuplicateKey);
        }
        defaults
    } else {
        vec![serde_json::from_str::<(String, &RawValue)>(argument.get())
            .map_err(|_| JsonError::DefaultFieldArguments)?]
    };
    map_json_objects(source, |raw| {
        let entries = read_object_entries(raw)?;
        let missing = {
            let mut names = HashSet::new();
            if entries.iter().any(|(key, _)| !names.insert(key)) {
                return Err(JsonError::DuplicateKey);
            }
            defaults
                .iter()
                .filter(|(name, _)| !names.contains(name))
                .cloned()
                .collect::<Vec<_>>()
        };
        write_object_entries(entries.into_iter().chain(missing))
    })
}

fn json_to_csv(source: &str) -> Result<String, JsonError> {
    let rows = read_json_array(source)?;
    match rows.first().and_then(|row| row.get().chars().next()) {
        None => Ok(String::new()),
        Some('[') => json_array_rows_to_csv(rows, None),
        Some('{') => json_object_rows_to_csv(rows),
        _ => Err(JsonError::CsvRows),
    }
}

fn json_array_rows_with_headers(query: &str) -> Result<String, JsonError> {
    let (headers, source) = strings_and_source(query, JsonError::CsvRowHeaders)?;
    if headers.is_empty()
        || headers.iter().any(|header| header.trim().is_empty())
        || headers.iter().collect::<HashSet<_>>().len() != headers.len()
    {
        return Err(JsonError::CsvRowHeaders);
    }
    let mut output = headers
        .iter()
        .map(|header| encode_csv_cell(header))
        .collect::<Result<Vec<_>, _>>()?
        .join(",");
    let records = json_array_rows_to_csv(read_json_array(source)?, Some(headers.len()))?;
    let additional_bytes = if records.is_empty() {
        0
    } else {
        records.len() + 2
    };
    if output.len().saturating_add(additional_bytes) > MAX_PRETTY_BYTES {
        return Err(JsonError::CsvOutputTooLarge);
    }
    if !records.is_empty() {
        output.push_str("\r\n");
        output.push_str(&records);
    }
    Ok(output)
}

fn json_paths_to_csv(
    query: &str,
    read_rows: impl FnOnce(&str) -> Result<Vec<&RawValue>, JsonError>,
) -> Result<String, JsonError> {
    let (pointers, source) = selected_pointers(query)?;
    let columns = pointers
        .iter()
        .map(|pointer| pointer_keys(pointer)?.collect::<Result<Vec<_>, _>>())
        .collect::<Result<Vec<_>, _>>()?;
    let headers = pointers
        .iter()
        .map(|pointer| encode_csv_cell(pointer))
        .collect::<Result<Vec<_>, _>>()?;
    let mut output = headers.join(",");
    if output.len() > MAX_PRETTY_BYTES {
        return Err(JsonError::CsvOutputTooLarge);
    }
    for row in read_rows(source)? {
        let cells = columns
            .iter()
            .map(|keys| match select_pointer_keys(row, keys) {
                Ok(value) => json_csv_cell(value),
                Err(JsonError::MissingValue) => encode_csv_cell(""),
                Err(error) => Err(error),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let record = cells.join(",");
        if output.len().saturating_add(record.len()).saturating_add(2) > MAX_PRETTY_BYTES {
            return Err(JsonError::CsvOutputTooLarge);
        }
        output.push_str("\r\n");
        output.push_str(&record);
    }
    Ok(output)
}

fn pick_json_keys(query: &str) -> Result<String, JsonError> {
    let (names, source) = strings_and_source(query, JsonError::KeyArguments)?;
    let wanted: HashSet<_> = names.iter().map(String::as_str).collect();
    if names.is_empty() || names.len() != wanted.len() {
        return Err(JsonError::SelectionKeys);
    }
    map_json_objects(source, |raw| {
        let mut selected = HashMap::new();
        for (key, value) in read_object_entries(raw)? {
            if wanted.contains(key.as_str()) && selected.insert(key, value).is_some() {
                return Err(JsonError::SelectionConflict);
            }
        }
        write_object_entries(names.iter().filter_map(|key| selected.remove_entry(key)))
    })
}

fn map_json_objects(
    source: &str,
    mut transform: impl FnMut(&str) -> Result<String, JsonError>,
) -> Result<String, JsonError> {
    if !source.starts_with('[') {
        let output = transform(source)?;
        return (output.len() <= MAX_PRETTY_BYTES)
            .then_some(output)
            .ok_or(JsonError::ObjectOutputTooLarge);
    }
    let mut output = String::from("[");
    for (index, item) in read_json_array(source)?.into_iter().enumerate() {
        let raw = item.get();
        if !raw.starts_with('{') {
            return Err(JsonError::ExpectedObjectItem);
        }
        let object = transform(raw)?;
        let comma = usize::from(index != 0);
        if output
            .len()
            .saturating_add(object.len())
            .saturating_add(comma + 1)
            > MAX_PRETTY_BYTES
        {
            return Err(JsonError::ObjectOutputTooLarge);
        }
        if index != 0 {
            output.push(',');
        }
        output.push_str(&object);
    }
    output.push(']');
    Ok(output)
}

fn json_array_rows_to_csv(
    rows: Vec<&RawValue>,
    mut width: Option<usize>,
) -> Result<String, JsonError> {
    let mut output = Vec::with_capacity(rows.len());
    for row in rows {
        if !row.get().starts_with('[') {
            return Err(JsonError::CsvRows);
        }
        let cells = read_json_array(row.get())?;
        if cells.is_empty()
            || width
                .replace(cells.len())
                .is_some_and(|previous| previous != cells.len())
        {
            return Err(JsonError::CsvRows);
        }
        let record = cells
            .into_iter()
            .map(json_csv_cell)
            .collect::<Result<Vec<_>, _>>()?;
        output.push(record.join(","));
    }
    Ok(output.join("\r\n"))
}

fn json_object_rows_to_csv(rows: Vec<&RawValue>) -> Result<String, JsonError> {
    let mut order = Vec::new();
    let mut positions = HashMap::new();
    let mut entries = Vec::with_capacity(rows.len());
    for row in rows {
        if !row.get().starts_with('{') {
            return Err(JsonError::CsvRows);
        }
        let mut fields = HashMap::new();
        for (name, value) in read_object_entries(row.get())? {
            let next = positions.len();
            let position = *positions.entry(name.clone()).or_insert_with(|| {
                order.push(name);
                next
            });
            if fields.insert(position, value).is_some() {
                return Err(JsonError::CsvDuplicateKey);
            }
        }
        entries.push(fields);
    }
    if order.is_empty() {
        return Err(JsonError::CsvRows);
    }

    if order.iter().any(|name| name.contains('\0')) {
        return Err(JsonError::CsvNullCharacter);
    }
    let mut output = String::new();
    let limit = JsonError::CsvOutputTooLarge;
    append_bounded_csv_row(&mut output, order.iter().map(String::as_str), limit)?;
    for fields in entries {
        let record = (0..order.len())
            .map(|index| match fields.get(&index) {
                Some(value) => json_csv_text(value),
                None => Ok(Cow::Borrowed("")),
            })
            .collect::<Result<Vec<_>, _>>()?;
        append_bounded_csv_row(&mut output, record.iter().map(|cell| cell.as_ref()), limit)?;
    }
    Ok(output)
}

fn json_csv_cell(value: &RawValue) -> Result<String, JsonError> {
    let text = json_csv_text(value)?;
    Ok(csv_cell(&text).into_owned())
}

#[derive(Clone, Copy)]
pub(super) enum JsonCellContent {
    Value,
    Type,
    Length,
    Exists,
}

pub(super) fn json_cell_at_pointer<'a>(
    source: &'a str,
    keys: &[String],
    content: JsonCellContent,
) -> Result<Cow<'a, str>, JsonError> {
    let value = json_cell_value_at_pointer(source, keys)?;
    match (content, value) {
        (JsonCellContent::Exists, value) => Ok(Cow::Borrowed(if value.is_some() {
            "true"
        } else {
            "false"
        })),
        (_, None) => Ok(Cow::Borrowed("")),
        (JsonCellContent::Value, Some(value)) => json_csv_text(value),
        (JsonCellContent::Type, Some(value)) => Ok(Cow::Borrowed(json_type_name(value))),
        (JsonCellContent::Length, Some(value)) => match value.get().as_bytes()[0] {
            b'{' | b'[' => json_length(value.get()).map(Cow::Owned),
            _ => Ok(Cow::Borrowed("")),
        },
    }
}

fn json_cell_value_at_pointer<'a>(
    source: &'a str,
    keys: &[String],
) -> Result<Option<&'a RawValue>, JsonError> {
    if source.trim().is_empty() {
        return Ok(None);
    }
    let root = serde_json::from_str::<&RawValue>(source).map_err(json_parse_error)?;
    match select_pointer_keys(root, keys) {
        Ok(value) => Ok(Some(value)),
        Err(JsonError::MissingValue) => Ok(None),
        Err(error) => Err(error),
    }
}

pub(super) fn json_array_cells_at_pointer<'a>(
    source: &'a str,
    keys: &[String],
) -> Result<Vec<Cow<'a, str>>, JsonError> {
    let mut cells = match json_cell_value_at_pointer(source, keys)? {
        Some(value) if value.get() != "null" => read_json_array(value.get())?
            .into_iter()
            .map(json_csv_text)
            .collect::<Result<Vec<_>, _>>()?,
        _ => Vec::new(),
    };
    if cells.is_empty() {
        cells.push(Cow::Borrowed(""));
    }
    Ok(cells)
}

fn json_csv_text(value: &RawValue) -> Result<Cow<'_, str>, JsonError> {
    let raw = value.get();
    let text = match raw.as_bytes()[0] {
        b'"' => Cow::Owned(serde_json::from_str::<String>(raw).map_err(json_parse_error)?),
        b'{' | b'[' => Cow::Owned(layout_json(raw, None)?),
        b'n' => Cow::Borrowed(""),
        _ => Cow::Borrowed(raw),
    };
    if text.contains('\0') {
        return Err(JsonError::CsvNullCharacter);
    }
    Ok(text)
}

fn encode_csv_cell(value: &str) -> Result<String, JsonError> {
    if value.contains('\0') {
        return Err(JsonError::CsvNullCharacter);
    }
    Ok(csv_cell(value).into_owned())
}

fn build_json_pointer(source: &str) -> Result<String, JsonError> {
    let mut pointer = String::new();
    for segment in read_json_array(source)? {
        let raw = segment.get();
        let token = if raw.starts_with('"') {
            Cow::Owned(serde_json::from_str::<String>(raw).map_err(json_parse_error)?)
        } else if raw.bytes().all(|byte| byte.is_ascii_digit()) {
            Cow::Borrowed(raw)
        } else {
            return Err(JsonError::InvalidPointerSegment);
        };
        append_pointer_key(&mut pointer, &token);
    }
    if pointer.is_empty() || pointer.contains(|ch: char| ch.is_whitespace() || ch.is_control()) {
        serde_json::to_string(&pointer).map_err(json_parse_error)
    } else {
        Ok(pointer)
    }
}

pub(super) fn append_pointer_key(pointer: &mut String, key: &str) {
    pointer.push('/');
    for ch in key.chars() {
        match ch {
            '~' => pointer.push_str("~0"),
            '/' => pointer.push_str("~1"),
            _ => pointer.push(ch),
        }
    }
}

fn find_json_keys(query: &str, output: PointerOutput) -> Result<String, JsonError> {
    let invalid = JsonError::KeySearchArguments;
    let (key, source) = json_value_and_source(query, invalid)?;
    let key = serde_json::from_str::<String>(key.get()).map_err(|_| invalid)?;
    write_json_pointer_results(source, PointerSelection::Key(&key), output)
}

enum PatternTarget {
    Key,
    Value,
    Pointer,
}

fn find_json_pattern(
    query: &str,
    output: PointerOutput,
    target: PatternTarget,
) -> Result<String, JsonError> {
    let invalid = JsonError::SearchPatternArguments;
    let (pattern, source) = json_value_and_source(query, invalid)?;
    let pattern = serde_json::from_str::<String>(pattern.get()).map_err(|_| invalid)?;
    let regex = super::text_tools::compile_regex(&pattern).map_err(|_| invalid)?;
    let selection = match target {
        PatternTarget::Key => PointerSelection::KeyPattern(&regex),
        PatternTarget::Value => PointerSelection::ValuePattern(&regex),
        PatternTarget::Pointer => PointerSelection::PathPattern(&regex),
    };
    write_json_pointer_results(source, selection, output)
}

fn find_json_types(query: &str, output: PointerOutput) -> Result<String, JsonError> {
    let invalid = JsonError::TypeSearchArguments;
    let (types, source) = json_value_and_source(query, invalid)?;
    let types: Vec<String> = if types.get().starts_with('"') {
        vec![serde_json::from_str(types.get()).map_err(|_| invalid)?]
    } else {
        serde_json::from_str(types.get()).map_err(|_| invalid)?
    };
    if !(1..=6).contains(&types.len())
        || types.iter().any(|kind| {
            !matches!(
                kind.as_str(),
                "object" | "array" | "string" | "number" | "boolean" | "null"
            )
        })
    {
        return Err(invalid);
    }
    write_json_pointer_results(source, PointerSelection::Types(&types), output)
}

fn find_json_values(query: &str, output: PointerOutput) -> Result<String, JsonError> {
    let (value, source) = json_value_and_source(query, JsonError::ValueSearchArguments)?;
    let scalar = json_scalar(value)?.ok_or(JsonError::ValueSearchArguments)?;
    write_json_pointer_results(source, PointerSelection::Value(&scalar), output)
}

#[derive(Clone, Copy)]
enum PointerSelection<'a> {
    Leaves,
    Key(&'a str),
    KeyPattern(&'a regex::Regex),
    Value(&'a JsonScalar),
    ValuePattern(&'a regex::Regex),
    PathPattern(&'a regex::Regex),
    Types(&'a [String]),
}

#[derive(Clone, Copy)]
enum PointerOutput {
    Paths,
    Flattened,
    Csv,
}

fn write_json_pointer_results(
    source: &str,
    selection: PointerSelection<'_>,
    output: PointerOutput,
) -> Result<String, JsonError> {
    let include_values = matches!(output, PointerOutput::Flattened);
    let (opening, closing, size_error) = match output {
        PointerOutput::Flattened => ("{", Some('}'), JsonError::FlattenedObjectTooLarge),
        PointerOutput::Csv => ("指针,类型,原值", None, JsonError::CsvOutputTooLarge),
        PointerOutput::Paths => ("[", Some(']'), JsonError::PointerListTooLarge),
    };
    let root = serde_json::from_str::<&RawValue>(source).map_err(json_parse_error)?;
    let mut pending = vec![(root, None::<(usize, String)>, false)];
    let mut path = String::new();
    let mut result = String::from(opening);
    while let Some((value, key, matches_key)) = pending.pop() {
        if let Some((parent_length, key)) = key {
            path.truncate(parent_length);
            append_pointer_key(&mut path, &key);
        }
        let raw = value.get();
        let kind = raw.as_bytes()[0];
        let children = match kind {
            b'{' => read_unique_object_entries(raw)?,
            b'[' => read_json_array(raw)?
                .into_iter()
                .enumerate()
                .map(|(index, child)| (index.to_string(), child))
                .collect(),
            _ => Vec::new(),
        };
        let emit = match selection {
            PointerSelection::Leaves => children.is_empty(),
            PointerSelection::Key(_) | PointerSelection::KeyPattern(_) => matches_key,
            PointerSelection::Value(expected) => json_scalar(value)?.as_ref() == Some(expected),
            PointerSelection::ValuePattern(regex) if kind == b'"' => {
                let decoded: String = serde_json::from_str(raw).map_err(json_parse_error)?;
                regex.is_match(&decoded)
            }
            PointerSelection::ValuePattern(_) => false,
            PointerSelection::PathPattern(regex) => regex.is_match(&path),
            PointerSelection::Types(kinds) => {
                let kind = json_type_name(value);
                kinds.iter().any(|candidate| candidate.as_str() == kind)
            }
        };
        for (key, child) in children.into_iter().rev() {
            let matches_key = kind == b'{'
                && match selection {
                    PointerSelection::Key(expected) => expected == key.as_str(),
                    PointerSelection::KeyPattern(regex) => regex.is_match(&key),
                    _ => false,
                };
            pending.push((child, Some((path.len(), key)), matches_key));
        }
        if !emit {
            continue;
        }
        if matches!(output, PointerOutput::Csv) {
            if path.contains('\0') {
                return Err(JsonError::CsvNullCharacter);
            }
            append_bounded_csv_row(
                &mut result,
                [path.as_str(), json_type_name(value), raw].into_iter(),
                size_error,
            )?;
            continue;
        }
        let encoded = serde_json::to_string(&path).map_err(json_parse_error)?;
        let has_previous = result.len() > 1;
        let value_length = if include_values { 1 + raw.len() } else { 0 };
        if result.len() + encoded.len() + value_length + usize::from(has_previous) + 1
            > MAX_PRETTY_BYTES
        {
            return Err(size_error);
        }
        if has_previous {
            result.push(',');
        }
        result.push_str(&encoded);
        if include_values {
            result.push(':');
            result.push_str(raw);
        }
    }
    if let Some(closing) = closing {
        result.push(closing);
    }
    Ok(result)
}

fn report_duplicate_json_keys(source: &str, csv: bool) -> Result<String, JsonError> {
    let root = serde_json::from_str::<&RawValue>(source).map_err(json_parse_error)?;
    let mut pending = vec![(root, None::<(usize, String)>)];
    let mut path = String::new();
    let mut result = String::from(if csv {
        "对象路径,对象起始字节,键,次数"
    } else {
        "["
    });
    while let Some((value, key)) = pending.pop() {
        if let Some((parent_length, key)) = key {
            path.truncate(parent_length);
            append_pointer_key(&mut path, &key);
        }
        let raw = value.get();
        match raw.as_bytes()[0] {
            b'{' => {
                let entries = read_object_entries(raw)?;
                append_duplicate_json_keys(&mut result, source, raw, &path, &entries, csv)?;
                for (key, child) in entries.into_iter().rev() {
                    pending.push((child, Some((path.len(), key))));
                }
            }
            b'[' => {
                for (index, child) in read_json_array(raw)?.into_iter().enumerate().rev() {
                    pending.push((child, Some((path.len(), index.to_string()))));
                }
            }
            _ => {}
        }
    }
    if !csv {
        if result.len() > 1 {
            result.push('\n');
        }
        result.push(']');
    }
    Ok(result)
}

fn append_duplicate_json_keys(
    result: &mut String,
    source: &str,
    raw: &str,
    path: &str,
    entries: &[(String, &RawValue)],
    csv: bool,
) -> Result<(), JsonError> {
    let mut counts = HashMap::with_capacity(entries.len());
    for (name, _) in entries {
        *counts.entry(name.as_str()).or_insert(0usize) += 1;
    }
    if counts.len() == entries.len() {
        return Ok(());
    }
    let object_start = raw_value_span(source, raw)?.start + 1;
    for (name, _) in entries {
        let Some(count) = counts.remove(name.as_str()).filter(|&count| count > 1) else {
            continue;
        };
        if csv {
            if path.contains('\0') || name.contains('\0') {
                return Err(JsonError::CsvNullCharacter);
            }
            append_bounded_csv_row(
                result,
                [
                    path,
                    object_start.to_string().as_str(),
                    name,
                    count.to_string().as_str(),
                ]
                .into_iter(),
                JsonError::DuplicateKeyReportTooLarge,
            )?;
            continue;
        }
        let row = serde_json::json!({
            "对象路径": path,
            "对象起始字节": object_start,
            "键": name,
            "次数": count,
        })
        .to_string();
        let has_previous = result.len() > 1;
        if result.len() + row.len() + usize::from(has_previous) + 3 > MAX_PRETTY_BYTES {
            return Err(JsonError::DuplicateKeyReportTooLarge);
        }
        if has_previous {
            result.push(',');
        }
        result.push('\n');
        result.push_str(&row);
    }
    Ok(())
}

fn parse_json_pointer(source: &str) -> Result<String, JsonError> {
    if source.is_empty() {
        return Err(JsonError::Incomplete);
    }
    let pointer = if source.starts_with('"') {
        Cow::Owned(serde_json::from_str::<String>(source).map_err(pointer_parse_error)?)
    } else {
        Cow::Borrowed(source)
    };
    let tokens = json_pointer_tokens(&pointer)?;
    serde_json::to_string(&tokens).map_err(json_parse_error)
}

pub(super) fn json_pointer_tokens(pointer: &str) -> Result<Vec<String>, JsonError> {
    pointer_keys(pointer)?.collect()
}

#[derive(serde::Serialize)]
struct JsonDifference<'a> {
    #[serde(rename = "路径")]
    pointer: &'a str,
    #[serde(rename = "左", skip_serializing_if = "Option::is_none")]
    left: Option<&'a RawValue>,
    #[serde(rename = "右", skip_serializing_if = "Option::is_none")]
    right: Option<&'a RawValue>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ComparisonOutput {
    Json,
    Csv,
}

fn compare_json_values(source: &str, format: ComparisonOutput) -> Result<String, JsonError> {
    let (left, right) = json_value_and_source(source, JsonError::Invalid)?;
    let right = serde_json::from_str::<&RawValue>(right).map_err(json_parse_error)?;
    let mut output = String::from(match format {
        ComparisonOutput::Json => "[",
        ComparisonOutput::Csv => "指针,变化,左值,右值",
    });
    compare_json_trees(left, right, |difference| {
        if format == ComparisonOutput::Csv {
            if difference.pointer.contains('\0') {
                return Err(JsonError::CsvNullCharacter);
            }
            let change = match (difference.left, difference.right) {
                (None, _) => "新增",
                (_, None) => "移除",
                _ => "修改",
            };
            append_bounded_csv_row(
                &mut output,
                [
                    difference.pointer,
                    change,
                    difference.left.map_or("", RawValue::get),
                    difference.right.map_or("", RawValue::get),
                ]
                .into_iter(),
                JsonError::ComparisonTooLarge,
            )?;
            return Ok(true);
        }
        let encoded = serde_json::to_string(&difference).map_err(json_parse_error)?;
        if output.len() + encoded.len() + 2 > MAX_PRETTY_BYTES {
            return Err(JsonError::ComparisonTooLarge);
        }
        if output.len() > 1 {
            output.push(',');
        }
        output.push_str(&encoded);
        Ok(true)
    })?;
    if format == ComparisonOutput::Json {
        output.push(']');
    }
    Ok(output)
}

fn compare_json_trees(
    left: &RawValue,
    right: &RawValue,
    mut record: impl for<'a> FnMut(JsonDifference<'a>) -> Result<bool, JsonError>,
) -> Result<(), JsonError> {
    let mut pending = vec![(Some(left), Some(right), None::<(usize, String)>)];
    let mut pointer = String::new();
    while let Some((left, right, key)) = pending.pop() {
        if let Some((parent_length, key)) = key {
            pointer.truncate(parent_length);
            append_pointer_key(&mut pointer, &key);
        }
        if let (Some(left), Some(right)) = (left, right) {
            let children = match (left.get().as_bytes()[0], right.get().as_bytes()[0]) {
                (b'{', b'{') => Some(compare_object_members(left, right)?),
                (b'[', b'[') => {
                    let left = read_json_array(left.get())?;
                    let right = read_json_array(right.get())?;
                    Some(
                        (0..left.len().max(right.len()))
                            .map(|index| {
                                (
                                    index.to_string(),
                                    left.get(index).copied(),
                                    right.get(index).copied(),
                                )
                            })
                            .collect(),
                    )
                }
                _ => None,
            };
            if let Some(children) = children {
                pending.extend(
                    children
                        .into_iter()
                        .rev()
                        .map(|(key, left, right)| (left, right, Some((pointer.len(), key)))),
                );
                continue;
            }
            let left_scalar = json_scalar(left)?;
            if left_scalar.is_some() && left_scalar == json_scalar(right)? {
                continue;
            }
        }
        let difference = JsonDifference {
            pointer: &pointer,
            left,
            right,
        };
        if !record(difference)? {
            break;
        }
    }
    Ok(())
}

type ComparedJsonMember<'a> = (String, Option<&'a RawValue>, Option<&'a RawValue>);

fn compare_object_members<'a>(
    left: &'a RawValue,
    right: &'a RawValue,
) -> Result<Vec<ComparedJsonMember<'a>>, JsonError> {
    let right = read_object_entries(right.get())?;
    let mut remaining = HashMap::with_capacity(right.len());
    for (key, value) in &right {
        if remaining.insert(key.as_str(), *value).is_some() {
            return Err(JsonError::DuplicateKey);
        }
    }
    let left = read_object_entries(left.get())?;
    let mut seen = HashSet::with_capacity(left.len());
    let mut children = Vec::with_capacity(left.len().max(right.len()));
    for (key, value) in &left {
        if !seen.insert(key) {
            return Err(JsonError::DuplicateKey);
        }
        children.push((key.clone(), Some(*value), remaining.remove(key.as_str())));
    }
    children.extend(
        right
            .iter()
            .filter(|(key, _)| remaining.contains_key(key.as_str()))
            .map(|(key, value)| (key.clone(), None, Some(*value))),
    );
    Ok(children)
}

fn pointer_parse_error(error: serde_json::Error) -> JsonError {
    if error.is_eof() {
        JsonError::Incomplete
    } else {
        JsonError::InvalidPointer
    }
}

pub(super) fn pointer_and_source(query: &str) -> Result<(String, &str), JsonError> {
    let (pointer, source) = if query.starts_with('"') {
        let mut strings = serde_json::Deserializer::from_str(query).into_iter::<String>();
        let pointer = strings
            .next()
            .ok_or(JsonError::Incomplete)?
            .map_err(pointer_parse_error)?;
        let rest = &query[strings.byte_offset()..];
        if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
            return Err(JsonError::InvalidPointer);
        }
        (pointer, rest)
    } else {
        let (pointer, rest) = query
            .split_once(char::is_whitespace)
            .ok_or(JsonError::Incomplete)?;
        (pointer.to_owned(), rest)
    };
    Ok((pointer, source.trim_start()))
}

fn pointer_keys(
    pointer: &str,
) -> Result<impl Iterator<Item = Result<String, JsonError>> + '_, JsonError> {
    let path = if pointer.is_empty() {
        None
    } else {
        Some(pointer.strip_prefix('/').ok_or(JsonError::InvalidPointer)?)
    };
    Ok(path
        .into_iter()
        .flat_map(|path| path.split('/'))
        .map(decode_pointer_key))
}

fn select_member<'a>(value: &'a RawValue, key: &str) -> Result<&'a RawValue, JsonError> {
    let raw = value.get();
    match raw.as_bytes()[0] {
        b'{' => serde::Deserializer::deserialize_map(
            &mut serde_json::Deserializer::from_str(raw),
            ObjectMember(key),
        )
        .map_err(json_parse_error)?,
        b'[' => {
            let index = json_array_index(key)?;
            let items = serde_json::from_str::<Vec<&RawValue>>(raw).map_err(json_parse_error)?;
            items.get(index).copied().ok_or(JsonError::MissingValue)
        }
        _ => Err(JsonError::MissingValue),
    }
}

pub(super) fn json_array_index(key: &str) -> Result<usize, JsonError> {
    if (key.len() > 1 && key.starts_with('0')) || !key.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(JsonError::MissingValue);
    }
    key.parse().map_err(|_| JsonError::MissingValue)
}

fn json_keys(source: &str) -> Result<String, JsonError> {
    if !source.starts_with('{') {
        serde_json::from_str::<IgnoredAny>(source).map_err(json_parse_error)?;
        return Err(JsonError::ExpectedObject);
    }
    let keys = read_object_keys(source)?;
    Ok(serde_json::to_string(&keys).expect("键名列表可以序列化为 JSON"))
}

fn read_object_keys(source: &str) -> Result<Vec<String>, JsonError> {
    let mut parser = serde_json::Deserializer::from_str(source);
    let keys =
        serde::Deserializer::deserialize_map(&mut parser, ObjectKeys).map_err(json_parse_error)?;
    parser.end().map_err(json_parse_error)?;
    Ok(keys)
}

fn json_length(source: &str) -> Result<String, JsonError> {
    let length = match source.as_bytes().first().copied() {
        Some(b'{') => read_object_keys(source)?.len(),
        Some(b'[') => serde_json::from_str::<Vec<IgnoredAny>>(source)
            .map_err(json_parse_error)?
            .len(),
        _ => {
            serde_json::from_str::<IgnoredAny>(source).map_err(json_parse_error)?;
            return Err(JsonError::ExpectedCollection);
        }
    };
    Ok(length.to_string())
}

#[derive(Default, serde::Serialize)]
struct JsonStructureStats {
    #[serde(rename = "最大深度")]
    max_depth: usize,
    #[serde(rename = "值总数")]
    values: usize,
    #[serde(rename = "对象")]
    objects: usize,
    #[serde(rename = "数组")]
    arrays: usize,
    #[serde(rename = "字符串")]
    strings: usize,
    #[serde(rename = "数字")]
    numbers: usize,
    #[serde(rename = "布尔")]
    booleans: usize,
    #[serde(rename = "空值")]
    nulls: usize,
    #[serde(rename = "对象字段")]
    fields: usize,
}

fn json_structure_stats(source: &str) -> Result<String, JsonError> {
    serde_json::from_str::<IgnoredAny>(source).map_err(json_parse_error)?;
    // 校验后按原始字节统计，保留重复键，也无需将数字转换成机器数值。
    let mut stats = JsonStructureStats::default();
    let mut depth = 0;
    let mut previous = b':';
    let mut quoted = false;
    let mut escaped = false;
    for byte in source.bytes() {
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
            continue;
        }
        let starts_value = matches!(previous, b':' | b'[' | b',');
        let count = match byte {
            b' ' | b'\t' | b'\n' | b'\r' => continue,
            b'{' => Some(&mut stats.objects),
            b'[' => Some(&mut stats.arrays),
            b'"' => {
                quoted = true;
                Some(&mut stats.strings)
            }
            b'-' | b'0'..=b'9' if starts_value => Some(&mut stats.numbers),
            b't' | b'f' if starts_value => Some(&mut stats.booleans),
            b'n' if starts_value => Some(&mut stats.nulls),
            _ => None,
        };
        if let Some(count) = count {
            *count += 1;
            stats.values += 1;
            stats.max_depth = stats.max_depth.max(depth);
        }
        match byte {
            b'{' | b'[' => depth += 1,
            b'}' | b']' => depth -= 1,
            b':' => stats.fields += 1,
            _ => {}
        }
        previous = byte;
    }
    // 每个对象字段有一个已计入的键名，把它从字符串和值总数中扣除。
    stats.strings -= stats.fields;
    stats.values -= stats.fields;
    serde_json::to_string_pretty(&stats).map_err(json_parse_error)
}

fn decode_pointer_key(token: &str) -> Result<String, JsonError> {
    let mut decoded = String::with_capacity(token.len());
    let mut chars = token.chars();
    while let Some(ch) = chars.next() {
        decoded.push(match ch {
            '~' => match chars.next() {
                Some('0') => '~',
                Some('1') => '/',
                _ => return Err(JsonError::InvalidPointer),
            },
            ch => ch,
        });
    }
    Ok(decoded)
}

struct ObjectKeys;

impl<'de> Visitor<'de> for ObjectKeys {
    type Value = Vec<String>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON object")
    }

    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
        let mut keys = Vec::new();
        while let Some((key, _)) = map.next_entry::<String, IgnoredAny>()? {
            keys.push(key);
        }
        Ok(keys)
    }
}

struct ObjectMember<'a>(&'a str);

impl<'de> Visitor<'de> for ObjectMember<'_> {
    type Value = Result<&'de RawValue, JsonError>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON object")
    }

    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
        let mut found = Ok(None);
        while let Some((key, value)) = map.next_entry::<String, &RawValue>()? {
            if key == self.0 {
                found = match found {
                    Ok(None) => Ok(Some(value)),
                    _ => Err(JsonError::DuplicateKey),
                };
            }
        }
        Ok(found.and_then(|value| value.ok_or(JsonError::MissingValue)))
    }
}

struct ObjectEntries;

impl<'de> Visitor<'de> for ObjectEntries {
    type Value = Vec<(String, &'de RawValue)>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON object")
    }

    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
        let mut entries = Vec::new();
        while let Some(entry) = map.next_entry()? {
            entries.push(entry);
        }
        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_whole_json_and_maps_parser_positions_to_input_characters() {
        for (query, expected, location) in [
            (
                "校验\r\n{\r\n  \"参数\": \"茶\"坏\r\n}",
                "坏",
                "第 2 行 · 第 12 列",
            ),
            ("VALIDATE [\"😀\",好]", "好", "第 1 行 · 第 6 列"),
            ("校验 [1,\r\n  ", "", "第 2 行 · 第 3 列"),
            ("校验 [1\n", "", "第 2 行 · 第 1 列"),
            ("校验 1\u{a0}", "\u{a0}", "第 1 行 · 第 2 列"),
        ] {
            let report = json_validation_report(query).unwrap().unwrap();
            let range = report.error_span.unwrap();
            assert_eq!(&query[range.clone()], expected);
            if expected.is_empty() {
                assert_eq!(range, query.len()..query.len());
                assert!(report.text.starts_with("JSON 尚未完整"));
            }
            assert!(report.text.contains(location), "{}", report.text);
            assert_eq!(transform_json(query), Ok(report.text));
        }
        for input in [
            "null",
            "{}",
            " [9007199254740993000000000000, 1e99999] \r\n",
        ] {
            let report = json_validation_report(&format!("校验 {input}"))
                .unwrap()
                .unwrap();
            assert!(report.error_span.is_none());
            assert_eq!(report.text, "JSON 有效");
        }
        assert!(matches!(
            json_validation_report("校验 \r\n\t"),
            Some(Err(JsonError::Incomplete))
        ));
        assert!(json_validation_report("校验行CSV {}").is_none());
    }

    #[test]
    fn selects_the_first_invalid_json_line_excluding_its_line_break() {
        for (query, first_invalid, location) in [
            (
                "校验行\r\n{}\r\n \t\r\n  \"茶\"坏  \r\nnull\r\n[",
                "  \"茶\"坏  ",
                "第 3 行",
            ),
            ("VALIDATE-JSONL 1\n  未修复  ", "  未修复  ", "第 2 行"),
        ] {
            let report = json_validation_report(query).unwrap().unwrap();
            assert_eq!(&query[report.error_span.unwrap()], first_invalid);
            assert!(report.text.contains(location));
            assert_eq!(transform_json(query), Ok(report.text));
        }
        let valid = json_validation_report("校验行 null\r\n \t\r\n{}")
            .unwrap()
            .unwrap();
        assert_eq!(valid.text, "2 行有效，0 行无效");
        assert!(valid.error_span.is_none());
        assert!(matches!(
            json_validation_report("校验行 \r\n\t"),
            Some(Err(JsonError::Incomplete))
        ));
    }

    #[test]
    fn groups_json_numeric_medians_by_exact_scalar_keys_in_first_seen_order() {
        let query = r#"分组中位数 ["/组","/值/金额"] [{"组":1.0,"值":{"金额":1.50}},{"组":"北","值":{"金额":4}},{"组":1e0,"值":{"金额":2e0}},{"组":"北","值":{"金额":2}},{"组":"北","值":{"金额":3}}]"#;
        assert_eq!(
            transform_json(query).as_deref(),
            Ok(r#"[{"值":1.0,"中位数":"7/4"},{"值":"北","中位数":"3"}]"#),
        );
        assert_eq!(
            transform_json(r#"group-median ["/组","/金额"] []"#).as_deref(),
            Ok("[]"),
        );
        assert_eq!(
            transform_json(r#"分组中位数 ["/组","/金额"] [{"组":"北","金额":"1.50"}]"#),
            Err(JsonError::GroupStatisticsNumber),
        );
        let rows = vec![r#"{"组":"北","金额":1}"#; MAX_VALUES + 1].join(",");
        let query = format!(r#"分组中位数 ["/组","/金额"] [{rows}]"#);
        assert_eq!(transform_json(&query), Err(JsonError::GroupStatisticsLimit));
    }

    #[test]
    fn strips_json_comments_and_preserves_literals() {
        let input = "去注释 {\r\n// 备注\r\n\"网址\":\"https://example.invalid/\",/* 跨行\r\n忽略 */\"数\":1.50,\"文字\":\"转义\\\" // /* 保留 */\"}";
        let expected = r#"{
  "网址": "https://example.invalid/",
  "数": 1.50,
  "文字": "转义\" // /* 保留 */"
}"#;
        assert_eq!(transform_json(input).as_deref(), Ok(expected));
        assert_eq!(
            transform_json("strip-comments 2e2 // 行尾").as_deref(),
            Ok("2e2")
        );
        assert_eq!(
            transform_json("去注释 [1/*说明*/2]"),
            Err(JsonError::Invalid)
        );
        assert_eq!(
            transform_json("去注释 [1,/* 未关闭"),
            Err(JsonError::Incomplete)
        );
        assert_eq!(
            transform_json("格式化 { /* 备注 */ }"),
            Err(JsonError::Invalid)
        );
    }

    #[test]
    fn validates_every_json_line_and_retains_source_positions_after_blank_lines() {
        assert_eq!(
            transform_json("校验行\r\n \t\r\n1e999\r\n错误文本\r\n{\"编号\":\r\nnull").as_deref(),
            Ok("2 行有效，2 行无效\n第 3 行 · JSON 语法无效\n第 4 行 · JSON 尚未完整")
        );
        assert_eq!(
            transform_json("validate-jsonl []\ntrue").as_deref(),
            Ok("2 行有效，0 行无效")
        );
        assert_eq!(transform_json("校验行 \n \t\n"), Err(JsonError::Incomplete));
    }

    #[test]
    fn reports_json_lines_types_and_syntax_errors_with_original_row_numbers() {
        let query = "校验行CSV\r\n \t\r\n1e999\r\n\r\n错误文本\r\n{\"编号\":\r\nnull";
        let expected = "行号,状态,类型,原因\r\n2,有效,number,\"\"\r\n4,无效,\"\",JSON 语法无效\r\n5,无效,\"\",JSON 尚未完整\r\n6,有效,null,\"\"";
        assert_eq!(transform_json(query).as_deref(), Ok(expected));
        assert_eq!(
            json_csv_report(query)
                .expect("校验结果应显示为表格")
                .as_deref(),
            Ok(expected)
        );
        let report = json_validation_table(query).unwrap().unwrap();
        assert_eq!(report.csv, expected);
        let invalid: Vec<_> = report
            .input_errors
            .into_iter()
            .map(|range| &query[range])
            .collect();
        assert_eq!(invalid, ["错误文本", "{\"编号\":"]);
        assert_eq!(
            transform_json("validate-jsonl-csv\r\n\t\r\n"),
            Err(JsonError::Incomplete)
        );
    }

    #[test]
    fn converts_nested_json_paths_to_ordered_csv_columns() {
        assert_eq!(
            transform_json(r#"路径转CSV ["/订单/金额","/a~1b"] [{"订单":{"金额":1.50},"a/b":"茶,咖啡"},{"订单":{"金额":9007199254740993},"a/b":null},{}]"#).as_deref(),
            Ok("/订单/金额,/a~1b\r\n1.50,\"茶,咖啡\"\r\n9007199254740993,\"\"\r\n\"\",\"\"")
        );
        assert_eq!(
            transform_json(r#"pointers-to-csv ["","/0"] [[1e2],[]]"#).as_deref(),
            Ok("\"\",/0\r\n[1e2],1e2\r\n[],\"\"")
        );
        assert_eq!(
            transform_json(r#"路径转CSV ["/x"] []"#).as_deref(),
            Ok("/x")
        );
        for (query, error) in [
            (r#"路径转CSV ["/~2"] []"#, JsonError::InvalidPointer),
            (r#"路径转CSV ["/x","/x"] []"#, JsonError::PointerSelection),
            (
                r#"路径转CSV ["/x"] [{"x":1,"x":2}]"#,
                JsonError::DuplicateKey,
            ),
            (r#"路径转CSV ["/x"] {}"#, JsonError::ExpectedArray),
        ] {
            assert_eq!(transform_json(query), Err(error), "{query}");
        }
    }

    #[test]
    fn extracts_json_lines_paths_into_ordered_csv_columns() {
        let options = r#"["/订单/金额","/a~1b","/0"]"#;
        let records =
            "{\"订单\":{\"金额\":1.50},\"a/b\":\"茶,咖啡\"}\r\n \t\r\n[9007199254740993]\r\nnull";
        let query = format!("行路径转CSV {options}\r\n{records}");
        let expected =
            "/订单/金额,/a~1b,/0\r\n1.50,\"茶,咖啡\",\"\"\r\n\"\",\"\",9007199254740993\r\n\"\",\"\",\"\"";
        assert_eq!(transform_json(&query).as_deref(), Ok(expected));
        assert_eq!(
            json_csv_report(&query)
                .expect("路径列应显示为带表头的表格")
                .as_deref(),
            Ok(expected)
        );
        assert_eq!(
            transform_json("jsonl-pointers-to-csv [\"/x\"] {\"x\":1}\n{"),
            Err(JsonError::InvalidLine)
        );
        assert_eq!(
            transform_json(r#"行路径转CSV ["/x"] {"x":1,"x":2}"#),
            Err(JsonError::DuplicateKey)
        );
    }

    #[test]
    fn counts_json_groups_by_multiple_typed_scalar_paths_in_first_seen_order() {
        let query = concat!(
            r#"分组计数 ["/地区","/金额"] [{"地区":"北","金额":1.50},"#,
            r#"{"地区":"南","金额":1.5},{"地区":"北","金额":1.5e0},{"地区":"北","金额":"1.5"}]"#,
        );
        assert_eq!(
            transform_json(query).as_deref(),
            Ok(
                r#"[{"值":["北",1.50],"计数":2},{"值":["南",1.5],"计数":1},{"值":["北","1.5"],"计数":1}]"#
            )
        );
        assert_eq!(
            transform_json(r#"group-count ["/a","/b"] []"#).as_deref(),
            Ok("[]")
        );
        assert_eq!(
            transform_json(r#"分组计数 ["/a","/a"] []"#),
            Err(JsonError::UniquePaths)
        );
        assert_eq!(
            transform_json(r#"分组计数 ["/a","/b"] [{"a":1,"b":[]}]"#),
            Err(JsonError::ExpectedScalar)
        );
    }

    #[test]
    fn groups_full_json_records_by_multiple_paths_without_reencoding_them() {
        let query = concat!(
            r#"分组明细 ["/地区","/状态"] [{"地区":"北","状态":"完成","金额":1.50},"#,
            r#"{"地区":"北","状态":"待办","金额":2e0},{"地区":"北","状态":"完成","金额":3}]"#,
        );
        let expected = concat!(
            r#"[{"值":["北","完成"],"项目":[{"地区":"北","状态":"完成","金额":1.50},"#,
            r#"{"地区":"北","状态":"完成","金额":3}]},"#,
            r#"{"值":["北","待办"],"项目":[{"地区":"北","状态":"待办","金额":2e0}]}]"#,
        );
        assert_eq!(transform_json(query).as_deref(), Ok(expected));
        assert_eq!(
            transform_json(r#"分组明细 ["/a","/b"] [{"a":1,"b":{}}]"#),
            Err(JsonError::ExpectedScalar)
        );
    }

    #[test]
    fn summarizes_json_groups_with_exact_nested_amounts() {
        let query = r#"分组统计 ["/组","/值/金额"] [{"组":1.0,"值":{"金额":1.50}},{"组":"北","值":{"金额":-2}},{"组":1e0,"值":{"金额":2e0}}]"#;
        assert_eq!(
            transform_json(query).as_deref(),
            Ok(
                r#"[{"值":1.0,"计数":2,"合计":"7/2","平均":"7/4"},{"值":"北","计数":1,"合计":"-2","平均":"-2"}]"#
            )
        );
        assert_eq!(
            transform_json(r#"group-statistics ["/组","/金额"] []"#).as_deref(),
            Ok("[]")
        );
        assert_eq!(
            transform_json(r#"分组统计 ["/组","/金额"] [{"组":"北","金额":"1.50"}]"#),
            Err(JsonError::GroupStatisticsNumber)
        );
        assert_eq!(
            transform_json(r#"分组统计 ["/组"] []"#),
            Err(JsonError::GroupStatisticsPaths)
        );
    }

    #[test]
    fn lists_grouped_json_records_preserving_the_order_and_number_spellings() {
        let query = r#"分组明细 /状态 [{"状态":"完成","金额":1.50},{"状态":"待办","金额":2e0},{"状态":"\u5b8c\u6210","金额":9007199254740993}]"#;
        assert_eq!(
            transform_json(query).as_deref(),
            Ok(
                r#"[{"值":"完成","项目":[{"状态":"完成","金额":1.50},{"状态":"\u5b8c\u6210","金额":9007199254740993}]},{"值":"待办","项目":[{"状态":"待办","金额":2e0}]}]"#
            )
        );
        assert_eq!(
            transform_json(r#"group-records "" [1,2,1.0,null]"#).as_deref(),
            Ok(r#"[{"值":1,"项目":[1,1.0]},{"值":2,"项目":[2]},{"值":null,"项目":[null]}]"#)
        );
        assert_eq!(transform_json("分组明细 /状态 []").as_deref(), Ok("[]"));
        assert_eq!(
            transform_json("分组明细 /状态 [{}]"),
            Err(JsonError::MissingValue)
        );
    }

    #[test]
    fn sorts_object_keys_through_arrays_and_preserves_raw_literal_values() {
        let input = r#"键排序 {"z":[{"b":9007199254740993,"a":1.20e2},{}],"\u0061":{"t":"\u0062","b":null},"A":[]}"#;
        assert_eq!(
            transform_json(input).as_deref(),
            Ok(
                r#"{"A":[],"a":{"b":null,"t":"\u0062"},"z":[{"a":1.20e2,"b":9007199254740993},{}]}"#
            )
        );
        assert_eq!(
            transform_json(r#"sort-keys [{"a":1,"\u0061":2}]"#),
            Err(JsonError::DuplicateKey)
        );
        let query = format!(
            "sort-keys {}{{\"z\":1,\"a\":2}}{}",
            "[".repeat(256),
            "]".repeat(256)
        );
        assert_eq!(
            transform_json(&query).unwrap(),
            format!("{}{{\"a\":2,\"z\":1}}{}", "[".repeat(256), "]".repeat(256))
        );
    }

    #[test]
    fn enumerates_resolvable_leaf_pointers_in_document_order() {
        let source = r#"{"商品":[{"a/b":9007199254740993},{"~":null},[]],"":"", "a\nb":{}}"#;
        let listed =
            write_json_pointer_results(source, PointerSelection::Leaves, PointerOutput::Paths)
                .unwrap();
        assert_eq!(
            listed,
            r#"["/商品/0/a~1b","/商品/1/~0","/商品/2","/","/a\nb"]"#
        );
        let paths: Vec<String> = serde_json::from_str(&listed).unwrap();
        for (path, expected) in paths
            .iter()
            .zip(["9007199254740993", "null", "[]", r#""""#, "{}"])
        {
            let path = serde_json::to_string(path).unwrap();
            assert_eq!(
                select_json_value(&format!("{path} {source}")).as_deref(),
                Ok(expected)
            );
        }
        for source in ["[]", "{}", "23", "null"] {
            assert_eq!(
                write_json_pointer_results(source, PointerSelection::Leaves, PointerOutput::Paths)
                    .as_deref(),
                Ok(r#"[""]"#)
            );
        }
        assert_eq!(
            write_json_pointer_results(
                r#"{"a":{"x":1,"\u0078":2}}"#,
                PointerSelection::Leaves,
                PointerOutput::Paths
            ),
            Err(JsonError::DuplicateKey)
        );
        assert_eq!(
            write_json_pointer_results("[1,]", PointerSelection::Leaves, PointerOutput::Paths),
            Err(JsonError::Invalid)
        );
    }

    #[test]
    fn keeps_the_explicit_first_or_last_occurrence_of_each_object_key() {
        let source =
            r#"{"a":1.00,"z":[{"k":1.20e+2,"k":9007199254740993}],"\u0061":-0.0,"b":"\u0062"}"#;
        assert_eq!(
            transform_json(&format!("去重复键 留首 {source}")).as_deref(),
            Ok(r#"{"a":1.00,"z":[{"k":1.20e+2}],"b":"\u0062"}"#)
        );
        assert_eq!(
            transform_json(&format!("deduplicate-keys LAST {source}")).as_deref(),
            Ok(r#"{"z":[{"k":9007199254740993}],"a":-0.0,"b":"\u0062"}"#)
        );
        for query in ["去重复键 {\"z\":1}", "deduplicate-keys middle {}"] {
            assert_eq!(transform_json(query), Err(JsonError::DuplicateKeyPolicy));
        }
        assert_eq!(
            transform_json("去重复键 留首 [{\"a\":1},{\"a\":2}]").as_deref(),
            Ok(r#"[{"a":1},{"a":2}]"#)
        );
        assert_eq!(
            transform_json("去重复键 留末 [1,]"),
            Err(JsonError::Invalid)
        );
    }

    #[test]
    fn reports_each_duplicate_key_with_its_object_path_and_source_position() {
        let first = r#"{"x":1,"\u0078":2,"y":0,"y":1,"x":3}"#;
        let second = r#"{"\u0000":1,"\u0000":2}"#;
        let source = format!(r#"{{"a/b~":[{first}],"a/b~":[{second}]}}"#);
        let report = transform_json(&format!("重复键 {source}")).unwrap();
        let rows: Vec<serde_json::Value> = serde_json::from_str(&report).unwrap();
        assert_eq!(
            rows,
            [
                serde_json::json!({
                    "对象路径": "", "对象起始字节": 1, "键": "a/b~", "次数": 2
                }),
                serde_json::json!({
                    "对象路径": "/a~1b~0/0", "对象起始字节": source.find(first).unwrap() + 1,
                    "键": "x", "次数": 3
                }),
                serde_json::json!({
                    "对象路径": "/a~1b~0/0", "对象起始字节": source.find(first).unwrap() + 1,
                    "键": "y", "次数": 2
                }),
                serde_json::json!({
                    "对象路径": "/a~1b~0/0", "对象起始字节": source.find(second).unwrap() + 1,
                    "键": "\u{0}", "次数": 2
                }),
            ]
        );
        assert_eq!(
            transform_json(r#"duplicate-keys [{"相同":1},{"相同":2}]"#).as_deref(),
            Ok("[]")
        );
        assert_eq!(transform_json("重复键 [0,]"), Err(JsonError::Invalid));
        let rows = [r#"{"x":0,"x":1}"#; 256].join(",");
        let source = format!(r#"{{"{}":[{rows}]}}"#, "x".repeat(4096));
        assert_eq!(
            report_duplicate_json_keys(&source, false),
            Err(JsonError::DuplicateKeyReportTooLarge)
        );
    }

    #[test]
    fn reports_duplicate_keys_in_distinct_objects_with_the_same_csv_path() {
        let first = r#"{"列,名":1,"列,名":2}"#;
        let second = r#"{"":3,"":4}"#;
        let source = format!(r#"{{"茶/杯":{first},"茶/杯":{second}}}"#);
        let query = format!("重复键CSV {source}");
        let expected = format!(
            "对象路径,对象起始字节,键,次数\r\n\"\",1,茶/杯,2\r\n/茶~1杯,{},\"列,名\",2\r\n/茶~1杯,{},\"\",2",
            source.find(first).unwrap() + 1,
            source.find(second).unwrap() + 1,
        );
        assert_eq!(transform_json(&query).as_deref(), Ok(expected.as_str()));
        assert_eq!(
            json_csv_report(&query).unwrap().as_deref(),
            Ok(expected.as_str())
        );
        assert_eq!(
            transform_json(r#"duplicate-keys-csv [{"相同":1},{"相同":2}]"#).as_deref(),
            Ok("对象路径,对象起始字节,键,次数"),
        );
        assert_eq!(
            transform_json(r#"重复键CSV {"\u0000":1,"\u0000":2}"#),
            Err(JsonError::CsvNullCharacter),
        );
    }

    #[test]
    fn finds_exact_json_scalar_values_in_document_order() {
        for (query, expected) in [
            (
                r#"找值 1.5 {"值":[1.500,1.5e0,"1.5"],"a/b":{"~":1.5}}"#,
                r#"["/值/0","/值/1","/a~1b/~0"]"#,
            ),
            (
                "find-value 9007199254740993 [9007199254740992,9007199254740993,9007199254740993.00]",
                r#"["/1","/2"]"#,
            ),
            ("找值 true [false,true,1]", r#"["/1"]"#),
            ("找值 null null", r#"[""]"#),
            (r#"找值 null {"值":"null"}"#, "[]"),
        ] {
            assert_eq!(transform_json(query).as_deref(), Ok(expected));
        }
        assert_eq!(
            transform_json("找值 [] {}"),
            Err(JsonError::ValueSearchArguments)
        );
        assert_eq!(
            transform_json(r#"找值 1 {"重复":1,"重复":2}"#),
            Err(JsonError::DuplicateKey)
        );
    }

    #[test]
    fn finds_matching_object_keys_in_input_order_and_escapes_pointer_tokens() {
        let source =
            r#"{"订单":[{"条目":{"编号":1}},{"编号":2}],"编号":{"子":"x"},"别名":{"编/号":2}}"#;
        assert_eq!(
            transform_json(&format!("找键 \"编号\" {source}")).as_deref(),
            Ok(r#"["/订单/0/条目/编号","/订单/1/编号","/编号"]"#)
        );
        assert_eq!(
            transform_json(r#"find-key "a~b" [{"元/数据":{"a~b":[]}}]"#).as_deref(),
            Ok(r#"["/0/元~1数据/a~0b"]"#)
        );
        assert_eq!(
            transform_json(r#"找键 "0" [{"0":null}]"#).as_deref(),
            Ok(r#"["/0/0"]"#)
        );
        assert_eq!(
            transform_json(r#"找键 "" {"":1}"#).as_deref(),
            Ok(r#"["/"]"#)
        );
        assert_eq!(transform_json(r#"找键 "缺失" [1,2]"#).as_deref(), Ok("[]"));
        assert_eq!(
            transform_json("找键 1 {}"),
            Err(JsonError::KeySearchArguments)
        );
        assert_eq!(
            transform_json(r#"找键 "x" {"对象":{"x":null,"\u0078":1}}"#),
            Err(JsonError::DuplicateKey)
        );
    }

    #[test]
    fn searches_nested_json_key_patterns_and_preserves_real_pointers_and_literals() {
        let query = r#"正则找键CSV "^金额[~/]" {"金额/元":1.50,"订单":[{"金额~券":2e2,"金额":3}],"其他":{"金额/元":null}}"#;
        let expected = concat!(
            "指针,类型,原值\r\n/金额~1元,number,1.50",
            "\r\n/订单/0/金额~0券,number,2e2\r\n/其他/金额~1元,null,null"
        );
        assert_eq!(transform_json(query).as_deref(), Ok(expected));
        assert_eq!(json_csv_report(query).unwrap().as_deref(), Ok(expected));
        assert_eq!(
            transform_json(r#"find-key-regex "^[0-9]+$" [{"0":1},2]"#).as_deref(),
            Ok(r#"["/0/0"]"#)
        );
        assert_eq!(
            transform_json(r#"正则找键 "[" {"金额":1}"#),
            Err(JsonError::SearchPatternArguments)
        );
    }

    #[test]
    fn searches_decoded_json_string_values_and_keeps_their_original_literals() {
        let query = r#"正则找值CSV "^北京$" {"城市":"\u5317京","备选":["北京","北京南"]}"#;
        let expected = concat!(
            "指针,类型,原值\r\n/城市,string,\"\"\"\\u5317京\"\"\"",
            "\r\n/备选/0,string,\"\"\"北京\"\"\""
        );
        assert_eq!(transform_json(query).as_deref(), Ok(expected));
        assert_eq!(json_csv_report(query).unwrap().as_deref(), Ok(expected));
        assert_eq!(
            transform_json(r#"find-value-regex "^1$" [1,"1",{"1":false}]"#).as_deref(),
            Ok(r#"["/1"]"#)
        );
        assert_eq!(
            transform_json(r#"正则找值 "^北京$" "北京""#).as_deref(),
            Ok(r#"[""]"#)
        );
    }

    #[test]
    fn searches_escaped_json_pointer_paths_in_document_order() {
        let query = r#"正则找路径CSV "^/订单/[0-9]+/金额$" {"订单":[{"金额":1.50},{"金额":null}],"金额":3}"#;
        let expected = "指针,类型,原值\r\n/订单/0/金额,number,1.50\r\n/订单/1/金额,null,null";
        assert_eq!(transform_json(query).as_deref(), Ok(expected));
        assert_eq!(json_csv_report(query).unwrap().as_deref(), Ok(expected));
        assert_eq!(
            transform_json(r#"find-pointer-regex "^/a~1b$" {"a/b":1,"a":{"b":2}}"#).as_deref(),
            Ok(r#"["/a~1b"]"#)
        );
        assert_eq!(
            transform_json(r#"正则找路径 "^$" {"订单":[1]}"#).as_deref(),
            Ok(r#"[""]"#)
        );
    }

    #[test]
    fn searches_multiple_json_value_types_and_includes_the_root_when_selected() {
        let query = r#"找类型CSV ["number","null"] {"数量":1.50,"状态":[null,"1.5",true]}"#;
        let expected = "指针,类型,原值\r\n/数量,number,1.50\r\n/状态/0,null,null";
        assert_eq!(transform_json(query).as_deref(), Ok(expected));
        assert_eq!(json_csv_report(query).unwrap().as_deref(), Ok(expected));
        assert_eq!(
            transform_json(r#"find-type "array" [[],{"嵌套":[1]}]"#).as_deref(),
            Ok(r#"["","/0","/1/嵌套"]"#)
        );
        assert_eq!(
            transform_json(r#"找类型 ["float"] [1.5]"#),
            Err(JsonError::TypeSearchArguments)
        );
    }

    #[test]
    fn previews_wildcard_matches_with_exact_pointers_types_and_raw_values() {
        let query = r#"通配路径CSV [null,null,"金额"] {"a/b":[{"金额":1.20},{},{"金额":null}],"~":[{"金额":9007199254740993}]}"#;
        let expected = concat!(
            "指针,类型,原值\r\n/a~1b/0/金额,number,1.20\r\n",
            "/a~1b/2/金额,null,null\r\n/~0/0/金额,number,9007199254740993",
        );
        assert_eq!(transform_json(query).as_deref(), Ok(expected));
        assert_eq!(json_csv_report(query).unwrap().as_deref(), Ok(expected));
        assert_eq!(
            transform_json(r#"wildcard-paths-csv [] 1.00"#).as_deref(),
            Ok("指针,类型,原值\r\n\"\",number,1.00"),
        );
        assert_eq!(
            transform_json(r#"通配路径CSV ["不存在"] {"字段":0}"#).as_deref(),
            Ok("指针,类型,原值"),
        );
        assert_eq!(
            transform_json(r#"通配路径CSV [null] {"\u0000":1}"#),
            Err(JsonError::CsvNullCharacter),
        );
    }

    #[test]
    fn selects_values_across_array_and_object_wildcards_in_document_order() {
        for (query, expected) in [
            (
                r#"通配取值 ["订单",null,"金额"] {"订单":[{"金额":1.20},{"状态":"草稿"},{"金额":null},{"金额":9007199254740993}]}"#,
                "[1.20,null,9007199254740993]",
            ),
            (
                r#"select-wildcard [null,null,"*"] {"南":[{"*":"\u8336"}],"北":[{"*":"米","a/b":1}]}"#,
                r#"["\u8336","米"]"#,
            ),
            (r#"通配取值 ["0","a/b"] [{"a/b":true}]"#, "[true]"),
            (r#"通配取值 ["01"] [0,1]"#, "[]"),
            (r#"通配取值 [null] "茶""#, "[]"),
            (r#"通配取值 [] {"x":1.00}"#, r#"[{"x":1.00}]"#),
        ] {
            assert_eq!(transform_json(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            transform_json(r#"通配取值 [false] {}"#),
            Err(JsonError::WildcardPathArguments)
        );
        assert_eq!(
            transform_json(r#"通配取值 [null] {"a":1,"\u0061":2}"#),
            Err(JsonError::DuplicateKey)
        );
    }

    #[test]
    fn exports_json_leaves_with_pointer_type_and_original_literal() {
        assert_eq!(
            transform_json(
                r#"叶子CSV {"报告":[{"a/b":9007199254740993},{"~":1.20e+2},[]],"空":null,"字符串":"\u8336","带,号":{},"":true}"#
            )
            .as_deref(),
            Ok(concat!(
                "指针,类型,原值\r\n",
                "/报告/0/a~1b,number,9007199254740993\r\n",
                "/报告/1/~0,number,1.20e+2\r\n",
                "/报告/2,array,[]\r\n",
                "/空,null,null\r\n",
                "/字符串,string,\"\"\"\\u8336\"\"\"\r\n",
                "\"/带,号\",object,{}\r\n",
                "/,boolean,true"
            ))
        );
        assert_eq!(
            transform_json("leaves-csv []").as_deref(),
            Ok("指针,类型,原值\r\n\"\",array,[]")
        );
        assert_eq!(
            transform_json(r#"叶子CSV {"a":1,"\u0061":2}"#),
            Err(JsonError::DuplicateKey)
        );
        assert_eq!(
            transform_json(r#"叶子CSV {"\u0000":1}"#),
            Err(JsonError::CsvNullCharacter)
        );
    }

    #[test]
    fn flattens_values_under_escaped_pointers_and_keeps_raw_literals() {
        let json = r#"{"商品":[{"a/b":9007199254740993},{"~":1.20e+2},[]],"":"\u0061","a\nb":{}}"#;
        assert_eq!(
            transform_json(&format!("展平 {json}")).as_deref(),
            Ok(
                r#"{"/商品/0/a~1b":9007199254740993,"/商品/1/~0":1.20e+2,"/商品/2":[],"/":"\u0061","/a\nb":{}}"#
            )
        );
        for (source, expected) in [
            ("12.30", r#"{"":12.30}"#),
            ("[]", r#"{"":[]}"#),
            ("{}", r#"{"":{}}"#),
        ] {
            assert_eq!(
                transform_json(&format!("flatten {source}")).as_deref(),
                Ok(expected)
            );
        }
        assert_eq!(
            transform_json(r#"展平 {"a":{"x":1,"\u0078":2}}"#),
            Err(JsonError::DuplicateKey)
        );
    }

    #[test]
    fn filters_json_items_by_pointer_presence_and_keeps_original_values() {
        let source = r#"[{"id":"001","a":{"b":null},"n":9007199254740993},{"id":"002","a": {}},{"id":"003","a":{"b":false}},null]"#;
        for (operation, expected) in [
            (
                "有路径",
                r#"[{"id":"001","a":{"b":null},"n":9007199254740993},{"id":"003","a":{"b":false}}]"#,
            ),
            ("missing-pointer", r#"[{"id":"002","a": {}},null]"#),
        ] {
            assert_eq!(
                transform_json(&format!("{operation} /a/b {source}")).as_deref(),
                Ok(expected)
            );
        }
        assert_eq!(
            transform_json(r#"has-pointer /0 [[null],[],[false]]"#).as_deref(),
            Ok("[[null],[false]]")
        );
        assert_eq!(
            transform_json(r#"有路径 "" [3,{},null]"#).as_deref(),
            Ok("[3,{},null]")
        );
        assert_eq!(
            transform_json("有路径 /a~2b []"),
            Err(JsonError::InvalidPointer)
        );
        assert_eq!(
            transform_json(r#"缺路径 /a [{"a":1,"a":2}]"#),
            Err(JsonError::DuplicateKey)
        );
    }

    #[test]
    fn filters_json_items_by_exact_scalar_value() {
        let source = r#"[{"id":"001","a/b":"完成"},{"id":"002","a/b":"\u5b8c\u6210"},{"id":"003"},{"id":"004","a/b":null},{"id":"005","a/b":["完成"]}]"#;
        assert_eq!(
            transform_json(&format!(r#"筛选 /a~1b "完成" {source}"#)).as_deref(),
            Ok(r#"[{"id":"001","a/b":"完成"},{"id":"002","a/b":"\u5b8c\u6210"}]"#)
        );
        assert_eq!(
            transform_json(&format!(r#"exclude /a~1b "完成" {source}"#)).as_deref(),
            Ok(r#"[{"id":"003"},{"id":"004","a/b":null},{"id":"005","a/b":["完成"]}]"#)
        );
        for (query, expected) in [
            (
                r#"filter "" 9007199254740993 [9007199254740992,"9007199254740993",9007199254740993.0,90071992547409930e-1]"#,
                "[9007199254740993.0,90071992547409930e-1]",
            ),
            (
                r#"filter "" 1e+99999999999999999999 [10e99999999999999999998,1e99999999999999999998]"#,
                "[10e99999999999999999998]",
            ),
            (
                r#"filter "" 0 [-0.00e-99999999999999999999,0,0.01,false]"#,
                "[-0.00e-99999999999999999999,0]",
            ),
            (
                r#"筛选 "/a b" null [{"a b":false},{"a b":null},{}]"#,
                r#"[{"a b":null}]"#,
            ),
            (r#"filter "" true [1,"true",true,false,null]"#, "[true]"),
        ] {
            assert_eq!(transform_json(query).as_deref(), Ok(expected));
        }
        for (query, error) in [
            ("筛选 /a {} []", JsonError::FilterValue),
            (r#"筛选 /a "完成"[]"#, JsonError::FilterValue),
            ("筛选 /a true", JsonError::Incomplete),
            ("筛选 /a null {}", JsonError::ExpectedArray),
            (r#"筛选 /a 1 [{"a":1,"a":2}]"#, JsonError::DuplicateKey),
        ] {
            assert_eq!(transform_json(query), Err(error), "{query}");
        }
    }

    #[test]
    fn keeps_the_first_json_item_for_each_exact_scalar_value() {
        let source = r#"[{"键":9007199254740993,"序":"001"},{"键":90071992547409930e-1,"序":"002"},{"键":9007199254740992,"序":"003"},{"键":"9007199254740993","序":"004"},{"键":null,"序":"005"},{"键":null,"序":"006"}]"#;
        assert_eq!(
            transform_json(&format!("按值去重 /键 {source}")).as_deref(),
            Ok(
                r#"[{"键":9007199254740993,"序":"001"},{"键":9007199254740992,"序":"003"},{"键":"9007199254740993","序":"004"},{"键":null,"序":"005"}]"#
            )
        );
        assert_eq!(
            transform_json(r#"unique-by-pointer "" ["茶","\u8336",0,-0.0,false,false]"#).as_deref(),
            Ok(r#"["茶",0,false]"#)
        );
        for (query, error) in [
            ("按值去重 /键 [{}]", JsonError::MissingValue),
            (r#"按值去重 /键 [{"键":[]}]"#, JsonError::ExpectedScalar),
            (r#"按值去重 /键 [{"键":1,"键":2}]"#, JsonError::DuplicateKey),
        ] {
            assert_eq!(transform_json(query), Err(error), "{query}");
        }
    }

    #[test]
    fn counts_exact_json_scalars_in_first_seen_order_with_raw_values() {
        let values = r#"[1.50,"1.5",1.5,9007199254740993,9007199254740992,null,"\u8336","茶",null,1.500,90071992547409930e-1]"#;
        assert_eq!(
            transform_json(&format!(r#"group-count "" {values}"#)).as_deref(),
            Ok(
                r#"[{"值":1.50,"计数":3},{"值":"1.5","计数":1},{"值":9007199254740993,"计数":2},{"值":9007199254740992,"计数":1},{"值":null,"计数":2},{"值":"\u8336","计数":2}]"#
            )
        );
        assert_eq!(transform_json("分组计数 /状态 []").as_deref(), Ok("[]"));
        for (input, error) in [
            ("分组计数 /状态 [{}]", JsonError::MissingValue),
            (r#"分组计数 /状态 [{"状态":[]}]"#, JsonError::ExpectedScalar),
            (
                r#"分组计数 /状态 [{"状态":1,"状态":2}]"#,
                JsonError::DuplicateKey,
            ),
        ] {
            assert_eq!(transform_json(input), Err(error), "{input}");
        }
    }

    #[test]
    fn filters_json_items_that_match_at_least_one_condition_in_original_order() {
        assert_eq!(
            transform_json(
                r#"任一条件筛选 [["/状态","=","完成"],["/金额",">=",9007199254740993]] [{"金额":9007199254740993.0},{"状态":"完成"},{"状态":"待办","金额":9007199254740992},{"状态":"完成","金额":9007199254740994},{}]"#
            )
            .as_deref(),
            Ok(r#"[{"金额":9007199254740993.0},{"状态":"完成"},{"状态":"完成","金额":9007199254740994}]"#)
        );
        assert_eq!(
            transform_json(r#"filter-any [["/批准","=",null],["/完成","=",true]] [{},{"批准":null},{"批准":false},{"完成":true}]"#).as_deref(),
            Ok(r#"[{"批准":null},{"完成":true}]"#)
        );
        assert_eq!(
            transform_json(r#"filter-any [] []"#),
            Err(JsonError::FilterConditions)
        );
    }

    #[test]
    fn removes_top_level_null_json_fields_and_keeps_order_and_scalar_spelling() {
        assert_eq!(
            transform_json(
                r#"去空字段 [{"编号":"001","备注":null,"已确认":false,"金额":1.50},{"编号":"002","备注":""},{"备注":null}]"#
            )
            .as_deref(),
            Ok(r#"[{"编号":"001","已确认":false,"金额":1.50},{"编号":"002","备注":""},{}]"#)
        );
        assert_eq!(
            transform_json(r#"remove-null-fields {"空":null,"内层":{"空":null},"行":[null]}"#)
                .as_deref(),
            Ok(r#"{"内层":{"空":null},"行":[null]}"#)
        );
        assert_eq!(
            transform_json(r#"remove-null-fields [{"a":null},3]"#),
            Err(JsonError::ExpectedObjectItem)
        );
        assert_eq!(
            transform_json(r#"remove-null-fields {"a":0,"\u0061":null}"#),
            Err(JsonError::DuplicateKey)
        );
    }

    #[test]
    fn fills_only_missing_json_fields_and_limits_repeated_defaults() {
        assert_eq!(
            transform_json(
                r#"补字段 {"币种":"CNY","金额":1.50,"备注":null} [{"编号":"001","金额":null},{"编号":"002","币种":"USD"}]"#
            )
            .as_deref(),
            Ok(concat!(
                r#"[{"编号":"001","金额":null,"币种":"CNY","备注":null},"#,
                r#"{"编号":"002","币种":"USD","金额":1.50,"备注":null}]"#,
            ))
        );
        assert_eq!(
            transform_json(r#"补字段 {"a":1,"\u0061":2} {}"#),
            Err(JsonError::DuplicateKey)
        );
        assert_eq!(
            transform_json(
                r#"补字段 ["金额",1.50] [{"编号":"001"},{"编号":"002","金额":null},{"金额":0.00}]"#
            )
            .as_deref(),
            Ok(r#"[{"编号":"001","金额":1.50},{"编号":"002","金额":null},{"金额":0.00}]"#)
        );
        assert_eq!(
            transform_json(r#"fill-missing-field ["",{"a":1e0}] {"编号":"001"}"#).as_deref(),
            Ok(r#"{"编号":"001","":{"a":1e0}}"#)
        );
        assert_eq!(
            transform_json(r#"补字段 ["a",9] [{"\u0061":1,"a":2}]"#),
            Err(JsonError::DuplicateKey)
        );
        assert_eq!(
            transform_json(r#"补字段 ["a",9] [3]"#),
            Err(JsonError::ExpectedObjectItem)
        );
        assert_eq!(
            transform_json(r#"补字段 ["a"] [{}]"#),
            Err(JsonError::DefaultFieldArguments)
        );
        let default = "x".repeat(65_536);
        let source = ["{}"; 16].join(",");
        assert_eq!(
            transform_json(&format!(r#"补字段 ["a","{default}"] [{source}]"#)),
            Err(JsonError::ObjectOutputTooLarge)
        );
    }

    #[test]
    fn keeps_the_first_json_item_for_each_exact_tuple_of_pointer_values() {
        assert_eq!(
            transform_json(
                r#"多路径去重 ["/地区","/月份"] [{"地区":"北","月份":1.00,"编号":"001"},{"地区":"南","月份":1},{"地区":"北","月份":1e0,"编号":"002"},{"地区":"北","月份":null}]"#
            )
            .as_deref(),
            Ok(r#"[{"地区":"北","月份":1.00,"编号":"001"},{"地区":"南","月份":1},{"地区":"北","月份":null}]"#)
        );
        for (input, error) in [
            (
                r#"unique-by-pointers ["/a","/a"] []"#,
                JsonError::UniquePaths,
            ),
            (
                r#"unique-by-pointers ["/a","/b"] [{"a":0}]"#,
                JsonError::MissingValue,
            ),
            (
                r#"unique-by-pointers ["/a","/b"] [{"a":[],"b":null}]"#,
                JsonError::ExpectedScalar,
            ),
        ] {
            assert_eq!(transform_json(input), Err(error), "{input}");
        }
    }

    #[test]
    fn retains_every_item_in_each_repeated_tuple_and_keeps_original_order() {
        assert_eq!(
            transform_json(concat!(
                r#"多路径查重复 ["/地区","/月份"] [{"地区":"北","月份":1.00,"编号":"001"},"#,
                r#"{"地区":"南","月份":null},{"地区":"北","月份":"1"},"#,
                r#"{"地区":"北","月份":1e0,"编号":"002"},{"地区":"南","月份":null,"编号":"003"}]"#,
            ))
            .as_deref(),
            Ok(concat!(
                r#"[{"地区":"北","月份":1.00,"编号":"001"},{"地区":"南","月份":null},"#,
                r#"{"地区":"北","月份":1e0,"编号":"002"},{"地区":"南","月份":null,"编号":"003"}]"#,
            ))
        );
        assert_eq!(
            transform_json(r#"duplicates-by-pointers ["/a","/b"] [{"a":1,"b":2}]"#).as_deref(),
            Ok("[]")
        );
        assert_eq!(
            transform_json(r#"多路径查重复 ["/a","/b"] [{"a":1,"b":2},{"a":1}]"#),
            Err(JsonError::MissingValue)
        );
    }

    #[test]
    fn keeps_last_records_by_exact_path_values_in_their_original_order() {
        let source = [
            r#"{"组":"A","键":1.00,"状态":"旧"}"#,
            r#"{"组":"B","键":null}"#,
            r#"{"组":"A","键":"1"}"#,
            r#"{"组":"A","键":1e0,"状态":"新"}"#,
        ];
        let expected = &source[1..];
        let query = format!("多路径去重留末 [\"/组\",\"/键\"] [{}]", source.join(","));
        assert_eq!(
            transform_json(&query),
            Ok(format!("[{}]", expected.join(",")))
        );
        let query = format!("行路径去重留末 [\"/键\"]\n{}", source.join("\r\n"));
        let output = expected.join("\n");
        assert_eq!(transform_json(&query), Ok(output.clone()));
        assert!(matches!(
            result_format(&query, &output),
            Some(JsonResultFormat::JsonLines)
        ));
        assert_eq!(
            transform_json(r#"last-by-pointers ["/组","/键"] [{"组":"A"}]"#),
            Err(JsonError::MissingValue),
        );
        assert_eq!(
            transform_json(r#"last-by-pointers ["/组","/键"] []"#).as_deref(),
            Ok("[]"),
        );
    }

    #[test]
    fn deduplicates_json_lines_by_one_or_more_exact_paths_preserving_first_records() {
        let source = [
            r#"{"订单":{"id":"001"},"件数":1.00,"状态":"首次"}"#,
            r#"{"订单":{"id":"001"},"件数":1e0,"状态":"重复"}"#,
            r#"{"订单":{"id":"001"},"件数":"1","状态":"字符串"}"#,
            r#"{"订单":{"id":"002"},"件数":1,"状态":"另一订单"}"#,
        ];
        let input = source.join("\r\n");
        let query = format!("行路径去重 [\"/订单/id\",\"/件数\"]\n{input}");
        let output = [source[0], source[2], source[3]].join("\n");
        assert_eq!(transform_json(&query), Ok(output.clone()));
        assert!(matches!(
            result_format(&query, &output),
            Some(JsonResultFormat::JsonLines)
        ));
        assert_eq!(
            transform_json(&format!("unique-jsonl-by-pointers [\"/订单/id\"] {input}")),
            Ok([source[0], source[3]].join("\n"))
        );
        assert_eq!(
            transform_json("行路径去重 [] 1"),
            Err(JsonError::LineUniquePaths)
        );
        assert_eq!(
            transform_json(&format!("{query}\n{{\"订单\":{{\"id\":\"003\"}}}}")),
            Err(JsonError::MissingValue)
        );
    }

    #[test]
    fn lists_all_duplicate_json_line_positions_in_original_order() {
        let first = r#"{"编号":"001","件数":1.00}"#;
        let single = r#"{"编号":"001","件数":"1"}"#;
        let last = r#"{"件数":1e0,"编号":"001"}"#;
        let options = r#"["/编号","/件数"]"#;
        let query = format!("行重复位置CSV {options}\r\n{first}\r\n\r\n{single}\r\n{last}");
        let expected = format!(
            "行号,出现次数,原始JSON\r\n1,2,{}\r\n4,2,{}",
            super::super::csv_tools::csv_cell(first),
            super::super::csv_tools::csv_cell(last),
        );
        assert_eq!(transform_json(&query).as_deref(), Ok(expected.as_str()));
        assert_eq!(
            json_csv_report(&query).unwrap().as_deref(),
            Ok(expected.as_str())
        );
        assert_eq!(
            transform_json(&format!("jsonl-duplicate-positions-csv {options} {single}")).as_deref(),
            Ok("行号,出现次数,原始JSON"),
        );
        assert_eq!(
            transform_json(&format!("{query}\n{{\"编号\":\"003\"}}")),
            Err(JsonError::MissingValue),
        );
    }

    #[test]
    fn computes_exact_json_line_group_medians_for_even_and_odd_groups() {
        let source = "{\"组\":1.00,\"金额\":1.50}\r\n{\"组\":\"北\",\"金额\":4}\r\n{\"组\":1e0,\"金额\":2}\r\n{\"组\":\"北\",\"金额\":1}\r\n{\"组\":\"北\",\"金额\":9}";
        let query = format!("行分组中位数CSV [\"/组\",\"/金额\"]\n{source}");
        let expected = "值,类型,计数,中位数\r\n1.00,number,2,7/4\r\n\"\"\"北\"\"\",string,3,4";
        assert_eq!(transform_json(&query).as_deref(), Ok(expected));
        assert_eq!(json_csv_report(&query).unwrap().as_deref(), Ok(expected));
        let too_many = vec![r#"{"组":1,"金额":1}"#; MAX_VALUES + 1].join("\n");
        assert_eq!(
            transform_json(&format!("行分组中位数CSV [\"/组\",\"/金额\"]\n{too_many}")),
            Err(JsonError::GroupStatisticsLimit)
        );
    }

    #[test]
    fn summarizes_json_line_groups_with_exact_sums_and_means() {
        let source =
            "{\"组\":1.00,\"金额\":1.50}\r\n\r\n{\"组\":\"北\",\"金额\":2}\r\n{\"组\":1e0,\"金额\":0.75}";
        let query = format!("行分组统计CSV [\"/组\",\"/金额\"]\n{source}");
        let expected =
            "值,类型,计数,合计,平均\r\n1.00,number,2,9/4,9/8\r\n\"\"\"北\"\"\",string,1,2,2";
        assert_eq!(transform_json(&query).as_deref(), Ok(expected));
        assert_eq!(json_csv_report(&query).unwrap().as_deref(), Ok(expected));
        assert_eq!(
            transform_json(&format!("{query}\n{{\"组\":1,\"金额\":\"0.75\"}}")),
            Err(JsonError::GroupStatisticsNumber)
        );
    }

    #[test]
    fn counts_json_lines_by_exact_scalar_and_preserves_the_first_spelling() {
        let source =
            "{\"金额\":1.50}\r\n\r\n{\"金额\":\"1.5\"}\r\n{\"金额\":15e-1}\r\n{\"金额\":null}";
        let query = format!("行分组计数CSV /金额\r\n{source}");
        let expected = "值,类型,计数\r\n1.50,number,2\r\n\"\"\"1.5\"\"\",string,1\r\nnull,null,1";
        assert_eq!(transform_json(&query).as_deref(), Ok(expected));
        assert_eq!(json_csv_report(&query).unwrap().as_deref(), Ok(expected));
        assert_eq!(
            transform_json(&format!("{query}\n{{}}")),
            Err(JsonError::MissingValue)
        );
        assert_eq!(
            transform_json(&format!("{query}\n{{\"金额\":[]}}")),
            Err(JsonError::ExpectedScalar)
        );
    }

    #[test]
    fn filters_json_lines_without_rounding_or_converting_records_to_an_array() {
        let conditions = r#"[["/状态","=","完成"],["/金额",">=",9007199254740993]]"#;
        let first = r#"{"编号":"001", "状态":"完成","金额":9007199254740993.00}"#;
        let second = r#"{"编号":"002","状态":"完成","金额":9007199254740992.99}"#;
        let third = r#"{"编号":"003","状态":"待办","金额":9007199254740994}"#;
        let source = format!(
            "{first}\r\n\r\n  {second}\r\n{third}\r\n{}",
            r#"{"编号":"004","金额":"9007199254740993"}"#
        );
        let all = format!("行筛选 {conditions}\n{source}");
        assert_eq!(transform_json(&all).as_deref(), Ok(first));
        assert!(matches!(
            result_format(&all, first),
            Some(JsonResultFormat::JsonLines)
        ));
        assert_eq!(
            transform_json(&format!("filter-any-jsonl {conditions} {source}")),
            Ok(format!("{first}\n{second}\n{third}"))
        );
        assert_eq!(
            transform_json(&format!("行筛选 {conditions}\n{source}\n{{")),
            Err(JsonError::InvalidLine)
        );
        assert_eq!(
            transform_json(&format!(r#"行筛选 {conditions} {{"状态":"待办"}}"#)),
            Ok(String::new())
        );
    }

    #[test]
    fn filters_json_items_that_match_every_scalar_or_exact_number_condition() {
        for (query, expected) in [
            (
                r#"多条件筛选 [["/状态","=","完成"],["/金额",">=",9007199254740993],["/批准","!=",null]] [{"状态":"完成","金额":9007199254740993.00,"批准":true},{"状态":"完成","金额":9007199254740992,"批准":true},{"状态":"待办","金额":9007199254740994,"批准":true},{"状态":"完成","金额":9007199254740993,"批准":null},{"状态":"完成","金额":9007199254740993}]"#,
                r#"[{"状态":"完成","金额":9007199254740993.00,"批准":true}]"#,
            ),
            (
                r#"filter-all [["","!=",null]] [null,["数据"],1.00,"true"]"#,
                r#"[1.00,"true"]"#,
            ),
            (
                r#"多条件筛选 [["/a~1b","=",1.5],["/a~1b","<",2]] [{"a/b":15e-1},{"a/b":"1.5"}]"#,
                r#"[{"a/b":15e-1}]"#,
            ),
        ] {
            assert_eq!(transform_json(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            transform_json(r#"多条件筛选 [["/金额",">",null]] []"#),
            Err(JsonError::NumberFilter)
        );
        assert_eq!(
            transform_json(r#"多条件筛选 [] []"#),
            Err(JsonError::FilterConditions)
        );
    }

    #[test]
    fn filters_only_json_numbers_with_exact_comparisons() {
        let source = r#"[{"价格":1.50},{"价格":1.49},{"价格":2e0},{"价格":"9"},{},{"价格":null}]"#;
        assert_eq!(
            transform_json(&format!("数值筛选 /价格 >= 1.5 {source}")).as_deref(),
            Ok(r#"[{"价格":1.50},{"价格":2e0}]"#)
        );
        assert_eq!(
            transform_json(&format!("数值筛选 /价格 != 1.5 {source}")).as_deref(),
            Ok(r#"[{"价格":1.49},{"价格":2e0}]"#)
        );
        assert_eq!(
            transform_json(
                r#"filter-numeric "" > 9007199254740992 [9007199254740993,9007199254740992,1e10000000000000000000]"#
            )
            .as_deref(),
            Ok("[9007199254740993,1e10000000000000000000]")
        );
        for query in [r#"数值筛选 "" => 1 [2]"#, r#"数值筛选 "" >= "1" [2]"#] {
            assert_eq!(
                transform_json(query),
                Err(JsonError::NumberFilter),
                "{query}"
            );
        }
    }

    #[test]
    fn sorts_json_numbers_by_exact_magnitude_and_keeps_tied_items_in_order() {
        let values = "[0,-2.50,3e10000000000000000000,1.2,1e10000000000000000000,-1E-4,9007199254740993,9007199254740992,1.200,-0.0,1e-10000000000000000000]";
        for (operation, expected) in [
            ("数值排序", "[-2.50,-1E-4,0,-0.0,1e-10000000000000000000,1.2,1.200,9007199254740992,9007199254740993,1e10000000000000000000,3e10000000000000000000]"),
            ("sort-numeric-desc", "[3e10000000000000000000,1e10000000000000000000,9007199254740993,9007199254740992,1.2,1.200,1e-10000000000000000000,0,-0.0,-1E-4,-2.50]"),
        ] {
            assert_eq!(
                transform_json(&format!(r#"{operation} "" {values}"#)).as_deref(),
                Ok(expected)
            );
        }
        assert_eq!(
            transform_json(r#"数值排序 /n [{"n":1.001,"id":"A"},{"n":1,"id":"B"}]"#).as_deref(),
            Ok(r#"[{"n":1,"id":"B"},{"n":1.001,"id":"A"}]"#)
        );
        for (query, error) in [
            (r#"数值排序 /n [{"n":"1"}]"#, JsonError::ExpectedNumber),
            (r#"数值排序 /n [{}]"#, JsonError::MissingValue),
            (r#"数值降序 /n [{"n":1,"n":2}]"#, JsonError::DuplicateKey),
        ] {
            assert_eq!(transform_json(query), Err(error), "{query}");
        }
    }

    #[test]
    fn sorts_json_strings_naturally_without_rewriting_values_or_ties() {
        let source = r#"[{"文件名":"文件10"},{"文件名":"文件2"},{"文件名":"\u6587\u4ef602"},{"文件名":"文件1"}]"#;
        assert_eq!(
            transform_json(&format!("文本排序 /文件名 {source}")).as_deref(),
            Ok(
                r#"[{"文件名":"文件1"},{"文件名":"文件2"},{"文件名":"\u6587\u4ef602"},{"文件名":"文件10"}]"#
            )
        );
        assert_eq!(
            transform_json(&format!("文本降序 /文件名 {source}")).as_deref(),
            Ok(
                r#"[{"文件名":"文件10"},{"文件名":"文件2"},{"文件名":"\u6587\u4ef602"},{"文件名":"文件1"}]"#
            )
        );
        assert_eq!(
            transform_json(r#"sort-text "" ["x10","x2"]"#).as_deref(),
            Ok(r#"["x2","x10"]"#)
        );
        assert_eq!(
            transform_json(r#"文本排序 /名称 [{"名称":2}]"#),
            Err(JsonError::ExpectedString)
        );
        assert_eq!(
            transform_json("文本排序 /名称 [{}]"),
            Err(JsonError::MissingValue)
        );
    }

    #[test]
    fn sorts_json_by_multiple_path_priorities_with_stable_natural_and_exact_comparisons() {
        let source = concat!(
            r#"多路径排序 [["/组","文本升序"],["/额","numeric-desc"]] ["#,
            r#"{"组":"文件10","额":1},"#,
            r#"{"组":"文件2","额":1.50},"#,
            r#"{"组":"文件2","额":2e0},"#,
            r#"{"组":"文件2","额":1.500}]"#,
        );
        assert_eq!(
            transform_json(source).as_deref(),
            Ok(concat!(
                r#"[{"组":"文件2","额":2e0},"#,
                r#"{"组":"文件2","额":1.50},"#,
                r#"{"组":"文件2","额":1.500},"#,
                r#"{"组":"文件10","额":1}]"#,
            ))
        );
        for (query, error) in [
            (
                r#"多路径排序 [["/a","文本升序"],["/a","数值升序"]] []"#,
                JsonError::SortFields,
            ),
            (
                r#"多路径排序 [["/a","数值升序"]] [{"a":"1"}]"#,
                JsonError::ExpectedNumber,
            ),
            (
                r#"多路径排序 [["/a","文本降序"]] [{}]"#,
                JsonError::MissingValue,
            ),
        ] {
            assert_eq!(transform_json(query), Err(error), "{query}");
        }
    }

    #[test]
    fn lists_deep_pointers_iteratively_and_bounds_the_serialized_result() {
        let depth = 256;
        let nested = format!("{}null{}", "[".repeat(depth), "]".repeat(depth));
        assert_eq!(
            write_json_pointer_results(&nested, PointerSelection::Leaves, PointerOutput::Paths)
                .unwrap(),
            format!(r#"["{}"]"#, "/0".repeat(depth))
        );
        let wide = format!(r#"{{"{}":[{}0]}}"#, "x".repeat(1500), "0,".repeat(849));
        assert_eq!(
            write_json_pointer_results(&wide, PointerSelection::Leaves, PointerOutput::Paths),
            Err(JsonError::PointerListTooLarge)
        );
        assert_eq!(
            write_json_pointer_results(&wide, PointerSelection::Leaves, PointerOutput::Flattened),
            Err(JsonError::FlattenedObjectTooLarge)
        );
        assert_eq!(
            write_json_pointer_results(&wide, PointerSelection::Leaves, PointerOutput::Csv),
            Err(JsonError::CsvOutputTooLarge)
        );
    }

    #[test]
    fn slices_arrays_with_python_indices_and_keeps_the_raw_elements() {
        let data = r#"[0,1.20e+2,9007199254740993,"茶",{ "x":null }]"#;
        for (parameters, expected) in [
            ("[1,4,2]", r#"[1.20e+2,"茶"]"#),
            ("[-2,null]", r#"["茶",{ "x":null }]"#),
            ("[null,null,-2]", r#"[{ "x":null },9007199254740993,0]"#),
            ("[null,-1,-1]", "[]"),
            ("[0,null,null]", data),
            ("[8,-10]", "[]"),
            ("[100,-100,-9223372036854775808]", r#"[{ "x":null }]"#),
            ("[-9223372036854775808,100,9223372036854775807]", "[0]"),
        ] {
            assert_eq!(
                transform_json(&format!("切片 {parameters} {data}")).as_deref(),
                Ok(expected),
                "{parameters}"
            );
        }
        assert_eq!(
            transform_json("SLICE [null,null,-1] []").as_deref(),
            Ok("[]")
        );
        assert_eq!(
            transform_json(r#"依次处理 ["切片 [1,3]","长度"] [1,2,3,4]"#).as_deref(),
            Ok("2")
        );
        for parameters in ["[0]", "[0,2,0]", "[0,3,1,9]", "[0,3,1.5]"] {
            assert_eq!(
                transform_json(&format!("切片 {parameters} {data}")),
                Err(JsonError::ArraySliceArguments),
                "{parameters}"
            );
        }
        assert_eq!(
            transform_json(r#"切片 [0,1] {"0":1}"#),
            Err(JsonError::ExpectedArray)
        );
    }

    #[test]
    fn merges_object_arrays_in_first_key_order_with_the_last_raw_value() {
        let input = r#"合并对象 [{"z":9007199254740993,"a":[ 1 ]},{"a":1e3,"b":"\u0058","内层":{"x":1}},{"内层":{ "y":2 }}]"#;
        assert_eq!(
            transform_json(input).as_deref(),
            Ok(r#"{"z":9007199254740993,"a":1e3,"b":"\u0058","内层":{"y":2}}"#),
        );
        assert_eq!(
            transform_json(r#"merge-objects [{"\u0061":1,"a":2},{"a":3}]"#).as_deref(),
            Ok(r#"{"a":3}"#),
        );
        assert_eq!(transform_json("合并对象 []").as_deref(), Ok("{}"));
        assert_eq!(
            transform_json(r#"合并对象 [{"x":1},2]"#),
            Err(JsonError::ExpectedObjectItem),
        );
        assert_eq!(
            transform_json(r#"合并对象 {"x":1}"#),
            Err(JsonError::ExpectedArray),
        );
        assert_eq!(transform_json("合并对象 ["), Err(JsonError::Incomplete));
    }

    #[test]
    fn converts_json_lines_without_changing_values_or_splitting_strings() {
        let array = r#"[{"a":"\u5317\n京","a":9.90},9007199254740993,null]"#;
        let lines = "{\"a\":\"\\u5317\\n京\",\"a\":9.90}\n9007199254740993\nnull";
        assert_eq!(
            transform_json(&format!("展开数组 {array}")).as_deref(),
            Ok(lines)
        );
        assert_eq!(
            transform_json(&format!("合并行 {}", lines.replace('\n', "\r\n \t\r\n"))).as_deref(),
            Ok(array)
        );
        assert_eq!(
            transform_json("to-jsonl [ {\n \"k\": \"a b\" \n}, [1,2] ]").as_deref(),
            Ok("{\"k\":\"a b\"}\n[1,2]")
        );
        assert_eq!(
            transform_json("from-jsonl 1 2\n3"),
            Err(JsonError::InvalidLine)
        );
        assert_eq!(
            transform_json("合并行 {\n\"x\":1\n}"),
            Err(JsonError::InvalidLine)
        );
        assert_eq!(transform_json("展开数组 []").as_deref(), Ok(""));
        assert_eq!(transform_json("展开数组 {}"), Err(JsonError::ExpectedArray));
    }

    #[test]
    fn selects_distinct_pointers_in_requested_order_and_preserves_numbers() {
        assert_eq!(
            transform_json(r#"取多路径 ["/订单/0/商品","/订单/0/价格"] {"订单":[{"价格":9007199254740993,"商品":"茶"}]}"#).as_deref(),
            Ok(r#"{"/订单/0/商品":"茶","/订单/0/价格":9007199254740993}"#)
        );
        assert_eq!(
            transform_json(r#"get-paths ["","/a~1b"] {"a/b": {"n":1.20}}"#).as_deref(),
            Ok(r#"{"":{"a/b":{"n":1.20}},"/a~1b":{"n":1.20}}"#)
        );
        for (query, error) in [
            (
                r#"get-paths ["/","\u002f"] {"":0}"#,
                JsonError::PointerSelection,
            ),
            (r#"get-paths [] []"#, JsonError::PointerSelection),
            (r#"get-paths ["/foo~2"] {}"#, JsonError::InvalidPointer),
            (r#"get-paths ["/missing"] {}"#, JsonError::MissingValue),
        ] {
            assert_eq!(transform_json(query), Err(error));
        }
    }

    #[test]
    fn selects_a_json_pointer_from_each_array_entry_and_preserves_raw_values() {
        for (input, expected) in [
            (r#"取列 /价格 [{"价格":9.90},{"价格":2e2}]"#, "[9.90,2e2]"),
            (
                r#"column "/a b/0" [{"a b":[9007199254740993]},{"a b":[null]}]"#,
                "[9007199254740993,null]",
            ),
            (r#"取列 "" [1,"北京"]"#, r#"[1,"北京"]"#),
            (r#"取列 /状态 []"#, "[]"),
        ] {
            assert_eq!(transform_json(input).as_deref(), Ok(expected), "{input}");
        }
        for (input, error) in [
            (r#"取列 /x [{"x":1},{}]"#, JsonError::MissingValue),
            (r#"取列 /x [{"x":1,"x":2}]"#, JsonError::DuplicateKey),
            (r#"取列 /~2 []"#, JsonError::InvalidPointer),
            (r#"取列 /x {"x":1}"#, JsonError::ExpectedArray),
        ] {
            assert_eq!(transform_json(input), Err(error), "{input}");
        }
    }

    #[test]
    fn flattens_array_values_at_a_pointer_preserving_order_and_raw_values() {
        for (query, expected) in [
            (
                r#"合并子数组 "/a b" [{"a b":[9007199254740993,1.50]},{"a b":[]},{"a b":[1e999,"\u8336",[2]]}]"#,
                r#"[9007199254740993,1.50,1e999,"\u8336",[2]]"#,
            ),
            (r#"flatten-arrays "" [[1],[],[2,[3]]]"#, "[1,2,[3]]"),
            ("合并子数组 /商品 []", "[]"),
        ] {
            assert_eq!(transform_json(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            (r#"合并子数组 /x [{"x":[]},{}]"#, JsonError::MissingValue),
            (
                r#"合并子数组 /x [{"x":null}]"#,
                JsonError::ExpectedArrayAtPointer,
            ),
            (
                r#"合并子数组 /x [{"x":[],"x":[]}]"#,
                JsonError::DuplicateKey,
            ),
            ("合并子数组 /~2 []", JsonError::InvalidPointer),
            ("合并子数组 /x {}", JsonError::ExpectedArray),
        ] {
            assert_eq!(transform_json(query), Err(error), "{query}");
        }
    }

    #[test]
    fn recursively_drops_null_object_fields_and_preserves_all_array_positions() {
        let source =
            r#"{"备注":null,"订单":[{"金额":1.50,"备注":null},null,{"说明":""}],"确认":false}"#;
        assert_eq!(
            transform_json(&format!("递归去空字段 {source}")).as_deref(),
            Ok(r#"{"订单":[{"金额":1.50},null,{"说明":""}],"确认":false}"#)
        );
        assert_eq!(
            transform_json(r#"remove-null-fields-deep [null,{"中":{"空":null}},{"数":1e0},false]"#)
                .as_deref(),
            Ok(r#"[null,{"中":{}},{"数":1e0},false]"#)
        );
        assert_eq!(
            transform_json(r#"去空字段 {"备注":null,"订单":{"备注":null}}"#).as_deref(),
            Ok(r#"{"订单":{"备注":null}}"#)
        );
        assert_eq!(
            transform_json(r#"递归去空字段 {"订单":[{"a":null,"\u0061":1}]}"#),
            Err(JsonError::DuplicateKey)
        );
        assert_eq!(transform_json("递归去空字段 null").as_deref(), Ok("null"));
    }

    #[test]
    fn reports_the_type_at_a_json_pointer_without_rewriting_values() {
        let source = r#"{"a/b":[9007199254740993,1e100000000000000000000,true,null,"1",{},[]]}"#;
        for (pointer, wanted) in [
            ("/a~1b", r#""array""#),
            ("/a~1b/0", r#""number""#),
            ("/a~1b/1", r#""number""#),
            ("/a~1b/2", r#""boolean""#),
            ("/a~1b/3", r#""null""#),
            ("/a~1b/4", r#""string""#),
            ("/a~1b/5", r#""object""#),
            ("/a~1b/6", r#""array""#),
        ] {
            let query = format!("类型 {pointer} {source}");
            assert_eq!(transform_json(&query).as_deref(), Ok(wanted), "{query}");
        }
        assert_eq!(
            transform_json(r#"type "" false"#).as_deref(),
            Ok(r#""boolean""#)
        );
        assert_eq!(
            transform_json("类型 /缺少 {}"),
            Err(JsonError::MissingValue)
        );
        assert_eq!(
            transform_json(r#"类型 /a {"a":null,"a":1}"#),
            Err(JsonError::DuplicateKey)
        );
    }

    #[test]
    fn counts_nested_json_value_types_with_exact_numbers_and_repeated_fields() {
        let result = transform_json(
            r#"结构统计 {"x":[-1.2e-100000000000000000000,true,false,null,"a:\"[{}]"],"x":{}}"#,
        )
        .expect("valid statistics");
        let stats: serde_json::Value = serde_json::from_str(&result).expect("statistics are JSON");
        assert_eq!(stats["最大深度"], 2);
        assert_eq!(stats["值总数"], 8);
        assert_eq!(stats["对象"], 2);
        assert_eq!(stats["数组"], 1);
        assert_eq!(stats["数字"], 1);
        assert_eq!(stats["布尔"], 2);
        assert_eq!(stats["空值"], 1);
        assert_eq!(stats["字符串"], 1);
        assert_eq!(stats["对象字段"], 2);
        for source in ["[]", "{}", "null", "-1.5e-2", "\"a:b\""] {
            let result = transform_json(&format!("structure-stats {source}")).unwrap();
            let stats = serde_json::from_str::<serde_json::Value>(&result).unwrap();
            assert_eq!(stats["最大深度"], 0, "{source}");
            assert_eq!(stats["值总数"], 1, "{source}");
        }
        assert_eq!(
            transform_json("依次处理 [\"结构统计\",\"取值 /最大深度\"] [null,[]]").as_deref(),
            Ok("1")
        );
        assert_eq!(transform_json("结构统计 [1,]"), Err(JsonError::Invalid));
    }

    #[test]
    fn counts_only_top_level_array_elements_or_object_entries() {
        for (query, expected) in [
            (r#"长度 [null, {"x":[1,2]}, 1e999]"#, "3"),
            (r#"length {"x":1,"x":2,"y":{}}"#, "3"),
            ("长度 []", "0"),
            ("长度 {}", "0"),
        ] {
            assert_eq!(transform_json(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            ("长度 [1,", JsonError::Incomplete),
            ("长度 [1,]", JsonError::Invalid),
            ("长度 {} []", JsonError::Invalid),
            ("长度 true", JsonError::ExpectedCollection),
        ] {
            assert_eq!(transform_json(query), Err(error), "{query}");
        }
    }

    #[test]
    fn lists_top_level_object_keys_in_order_including_duplicate_names() {
        for (query, expected) in [
            (
                r#"键名 {"Z":{"nested":1},"a":1e999,"\u0061":null}"#,
                r#"["Z","a","a"]"#,
            ),
            (
                r#"keys {"\u5317\u4EAC":1,"控制\n符":null,"":0}"#,
                r#"["北京","控制\n符",""]"#,
            ),
            ("键名 {}", "[]"),
        ] {
            assert_eq!(transform_json(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            ("keys [1,2]", JsonError::ExpectedObject),
            ("keys \"北京\"", JsonError::ExpectedObject),
            ("keys {", JsonError::Incomplete),
            ("keys {\"a\":1,}", JsonError::Invalid),
            ("keys {} []", JsonError::Invalid),
            ("keys [,", JsonError::Invalid),
        ] {
            assert_eq!(transform_json(query), Err(error), "{query}");
        }
    }

    #[test]
    fn adjusts_json_whitespace_without_rounding_or_reordering() {
        let source = r#"{"z":[" 北京 \"字\" ",{}],"n":9007199254740993,"n":1e999}"#;
        let expected = r#"{
  "z": [
    " 北京 \"字\" ",
    {}
  ],
  "n": 9007199254740993,
  "n": 1e999
}"#;
        assert_eq!(
            transform_json(&format!("格式化 {source}")).as_deref(),
            Ok(expected)
        );
        assert_eq!(
            transform_json(&format!("minify {expected}")).as_deref(),
            Ok(source)
        );
        assert_eq!(
            transform_json("pretty [[],{}]").unwrap(),
            "[\n  [],\n  {}\n]"
        );
        assert_eq!(
            transform_json("pretty -0.001200e+02").unwrap(),
            "-0.001200e+02"
        );
    }

    #[test]
    fn formats_json_with_a_custom_indent_and_preserves_token_spelling() {
        let query = r#"缩进 4 {"b":[1.20,"\u0042"],"a":{}}"#;
        let expected =
            "{\n    \"b\": [\n        1.20,\n        \"\\u0042\"\n    ],\n    \"a\": {}\n}";
        assert_eq!(transform_json(query).as_deref(), Ok(expected));
        assert_eq!(
            transform_json("indent 1\r\n[1,2]").as_deref(),
            Ok("[\n 1,\n 2\n]")
        );
        for query in ["缩进 0 {}", "缩进 17 {}", "indent 1.5 {}"] {
            assert_eq!(
                transform_json(query),
                Err(JsonError::IndentArguments),
                "{query}"
            );
        }
        assert_eq!(transform_json("缩进 4"), Err(JsonError::Incomplete));
        assert_eq!(transform_json("缩进 4 [1,]"), Err(JsonError::Invalid));
        let nested = format!("缩进 16 {}0{}", "[".repeat(300), "]".repeat(300));
        assert_eq!(transform_json(&nested), Err(JsonError::LayoutTooLarge));
    }

    #[test]
    fn distinguishes_incomplete_invalid_and_large_layouts() {
        assert_eq!(transform_json("格式化 {"), Err(JsonError::Incomplete));
        assert_eq!(transform_json("压缩 [1,]"), Err(JsonError::Invalid));
        assert_eq!(transform_json("压缩 true false"), Err(JsonError::Invalid));
        assert_eq!(transform_json("未知 {}"), Err(JsonError::UnknownOperation));
        let nested = format!("{}0{}", "[".repeat(1024), "]".repeat(1024));
        assert_eq!(
            transform_json(&format!("格式化 {nested}")),
            Err(JsonError::LayoutTooLarge)
        );
        assert_eq!(transform_json(&format!("压缩 {nested}")).unwrap(), nested);
    }

    #[test]
    fn projects_one_object_or_all_object_rows_in_requested_key_order() {
        for (query, expected) in [
            (
                r#"选键 ["名","值"] {"值":2e2,"名":"\u4f60","调试":true}"#,
                r#"{"名":"\u4f60","值":2e2}"#,
            ),
            (
                r#"选键 ["编号","状态"] [{"状态":true,"编号":9007199254740993},{"编号":"002","备注":"茶"}]"#,
                r#"[{"编号":9007199254740993,"状态":true},{"编号":"002"}]"#,
            ),
            (r#"选键 ["编号"] [{"名称":"茶","名称":"咖啡"}]"#, "[{}]"),
            (r#"选键 ["空"] []"#, "[]"),
        ] {
            assert_eq!(transform_json(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            (r#"选键 ["a","a"] {}"#, JsonError::SelectionKeys),
            (r#"选键 [] {}"#, JsonError::SelectionKeys),
            (r#"选键 ["a"] {"a":1,"a":2}"#, JsonError::SelectionConflict),
            (r#"选键 ["a"] [{},2]"#, JsonError::ExpectedObjectItem),
        ] {
            assert_eq!(transform_json(query), Err(error), "{query}");
        }
    }

    #[test]
    fn converts_json_arrays_and_objects_to_csv_with_original_number_spellings() {
        assert_eq!(
            transform_json(r#"转CSV [["编号","备注"],["001","茶,咖啡"],["002","他说\"好\"\n下一行"],[9007199254740993,1.20]]"#).as_deref(),
            Ok("编号,备注\r\n001,\"茶,咖啡\"\r\n002,\"他说\"\"好\"\"\n下一行\"\r\n9007199254740993,1.20")
        );
        assert_eq!(
            transform_json(r#"转CSV [{"编号":"001","数量":1.20},{"编号":"002","备注":"甲,乙"},{"编号":"003","数量":null,"备注":{ "x":2 }}]"#).as_deref(),
            Ok("编号,数量,备注\r\n001,1.20,\"\"\r\n002,\"\",\"甲,乙\"\r\n003,\"\",\"{\"\"x\"\":2}\"")
        );
        assert_eq!(transform_json("转CSV []").as_deref(), Ok(""));
    }

    #[test]
    fn rejects_json_rows_that_cannot_be_represented_as_a_csv_table() {
        for (query, error) in [
            ("转CSV [1,2]", JsonError::CsvRows),
            ("转CSV [[1],[2,3]]", JsonError::CsvRows),
            ("转CSV [[]]", JsonError::CsvRows),
            ("转CSV [{}]", JsonError::CsvRows),
            (r#"转CSV [[],{"a":1}]"#, JsonError::CsvRows),
            (r#"转CSV [{"a":1,"a":2}]"#, JsonError::CsvDuplicateKey),
            (r#"转CSV [["\u0000"]]"#, JsonError::CsvNullCharacter),
            (r#"转CSV [{"\u0000":1}]"#, JsonError::CsvNullCharacter),
        ] {
            assert_eq!(transform_json(query), Err(error), "{query}");
        }
    }

    #[test]
    fn renames_and_removes_top_level_json_keys_without_reencoding_values() {
        for (query, expected) in [
            (
                r#"改键 ["旧","新"] {"前":1e2,"旧":9007199254740993,"后":"\u4f60"}"#,
                r#"{"前":1e2,"新":9007199254740993,"后":"\u4f60"}"#,
            ),
            (
                r#"改键 ["","含 空格"]{"":{ "金额": 1.20 }, "其他":0,"其他":1}"#,
                r#"{"含 空格":{"金额":1.20},"其他":0,"其他":1}"#,
            ),
            (r#"改键 ["同","同"] {"同":1}"#, r#"{"同":1}"#),
            (
                r#"去键 ["调试","备注"] {"调试":0,"调试":1,"数据":{"调试":1},"备注":"x"}"#,
                r#"{"数据":{"调试":1}}"#,
            ),
            (r#"去键 ["已删除"] {"值":3.0}"#, r#"{"值":3.0}"#),
            (r#"去键 ["值"] {"值":3.0}"#, "{}"),
        ] {
            assert_eq!(transform_json(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            (r#"改键 ["旧","新"] {"新":2}"#, JsonError::MissingKey),
            (
                r#"改键 ["旧","新"] {"旧":1,"新":2}"#,
                JsonError::RenameConflict,
            ),
            (
                r#"改键 ["旧","新"] {"旧":1,"旧":2}"#,
                JsonError::RenameConflict,
            ),
            (r#"改键 ["旧"] {"旧":1}"#, JsonError::KeyArguments),
            (r#"去键 [1] {"值":1}"#, JsonError::KeyArguments),
            (r#"去键 [] {}"#, JsonError::KeyArguments),
            (r#"去键 ["值"] []"#, JsonError::ExpectedObject),
        ] {
            assert_eq!(transform_json(query), Err(error), "{query}");
        }
    }

    #[test]
    fn converts_between_json_pointer_tokens_and_query_ready_pointers() {
        for (query, expected) in [
            (
                r#"生成指针 ["商品",0,"a/b","x~y",""]"#,
                "/商品/0/a~1b/x~0y/",
            ),
            (r#"生成指针 [9007199254740993]"#, "/9007199254740993"),
            (r#"生成指针 ["a\u0000","b c"]"#, r#""/a\u0000/b c""#),
            ("生成指针 []", "\"\""),
            (
                "解析指针 /商品/0/a~1b/x~0y/",
                r#"["商品","0","a/b","x~y",""]"#,
            ),
            ("解析指针 /a~01", r#"["a~1"]"#),
            (r#"解析指针 """#, "[]"),
            (r#"解析指针 "/a\u0000/b c""#, r#"["a\u0000","b c"]"#),
        ] {
            assert_eq!(transform_json(query).as_deref(), Ok(expected), "{query}");
        }
        let pointer = transform_json(r#"生成指针 ["a b",0]"#).unwrap();
        let query = format!(r#"取值 {pointer} {{"a b":[42]}}"#);
        assert_eq!(transform_json(&query).as_deref(), Ok("42"));
        for (query, error) in [
            ("生成指针 [true]", JsonError::InvalidPointerSegment),
            ("生成指针 [-1]", JsonError::InvalidPointerSegment),
            ("解析指针 /a~2b", JsonError::InvalidPointer),
            ("解析指针 没有斜线", JsonError::InvalidPointer),
        ] {
            assert_eq!(transform_json(query), Err(error), "{query}");
        }
    }

    #[test]
    fn resolves_json_pointer_without_rewriting_the_selected_value() {
        assert_eq!(
            transform_json(r#"取值 /商品/0/价格 {"商品":[{"价格":9007199254740993}]}"#).as_deref(),
            Ok("9007199254740993")
        );
        assert_eq!(
            transform_json(r#"get "/a~1b/~01/ " {"a/b":{"~1":{" ":1e999}}}"#).as_deref(),
            Ok("1e999")
        );
        assert_eq!(transform_json(r#"get / {"":null}"#).as_deref(), Ok("null"));
        assert_eq!(
            transform_json(r#"get "" [ 1 , "\u5317" ]"#).as_deref(),
            Ok(r#"[ 1 , "\u5317" ]"#)
        );
        assert_eq!(
            transform_json(r#"get /a {"a":[ 1 , 2 ],"b":0,"b":1}"#).as_deref(),
            Ok("[ 1 , 2 ]")
        );
    }

    #[test]
    fn replaces_only_the_pointer_value_and_keeps_other_json_bytes() {
        assert_eq!(
            transform_json(
                r#"改值 /a~1b/1 {"新":1.50} { "a/b":[3.50,"\u597d"], "其它":1e2, "same":"\u597d" }"#
            )
            .as_deref(),
            Ok(r#"{ "a/b":[3.50,{"新":1.50}], "其它":1e2, "same":"\u597d" }"#)
        );
        assert_eq!(
            transform_json(r#"replace "" null {"x":1}"#).as_deref(),
            Ok("null")
        );
        for (query, error) in [
            (r#"改值 /x "新" {"x":1,"x":2}"#, JsonError::DuplicateKey),
            (r#"改值 /- 9 [1,2]"#, JsonError::MissingValue),
            (r#"改值 /x {}{"x":1}"#, JsonError::ReplacementValue),
            (r#"改值 /x 1"#, JsonError::Incomplete),
        ] {
            assert_eq!(transform_json(query), Err(error), "{query}");
        }
    }

    #[test]
    fn adds_members_and_array_items_without_rewriting_existing_data() {
        for (query, expected) in [
            (
                r#"加路径 /订单/1 {"金额":3.50} {"订单":[{"金额":1.00}, {"金额":2e0}],"税率":0.10}"#,
                r#"{"订单":[{"金额":1.00}, {"金额":3.50},{"金额":2e0}],"税率":0.10}"#,
            ),
            (
                r#"add-path /a~1b 1.50 { "x":0.10 }"#,
                r#"{ "x":0.10,"a/b":1.50 }"#,
            ),
            (r#"加路径 /x false {"\u0078":null}"#, r#"{"\u0078":false}"#),
            (r#"加路径 /- "\u597d" [  ]"#, r#"["\u597d"  ]"#),
            (r#"加路径 /1 false [1 ]"#, r#"[1,false ]"#),
            (r#"加路径 "/a/b" 1 {"a":{}}"#, r#"{"a":{"b":1}}"#),
            (r#"加路径 "" 1.00 {"a":2}"#, r#"1.00"#),
        ] {
            let output = transform_json(query).unwrap();
            assert_eq!(output, expected, "{query}");
            assert!(serde_json::from_str::<&RawValue>(&output).is_ok());
        }
        for (query, error) in [
            (r#"加路径 /x 3 {"x":1,"\u0078":2}"#, JsonError::DuplicateKey),
            (r#"加路径 /2 3 [1]"#, JsonError::MissingValue),
            (r#"加路径 /01 3 [1]"#, JsonError::MissingValue),
            (r#"加路径 /a/b 3 {"b":2}"#, JsonError::MissingValue),
            (r#"加路径 /a/b 3 {"a":null}"#, JsonError::MissingValue),
        ] {
            assert_eq!(transform_json(query), Err(error), "{query}");
        }
    }

    #[test]
    fn copies_raw_values_into_an_existing_destination_container() {
        for (query, expected) in [
            (
                r#"复制路径 ["/订单/0","/归档/-"] {"订单":[{"金额":1.50,"名字":"\u8336"}],"归档":[]}"#,
                r#"{"订单":[{"金额":1.50,"名字":"\u8336"}],"归档":[{"金额":1.50,"名字":"\u8336"}]}"#,
            ),
            (
                r#"copy-path ["/a","/b"] {"a":1.50,"b":0}"#,
                r#"{"a":1.50,"b":1.50}"#,
            ),
            (
                r#"复制路径 ["/0","/1"] [0.10, 1.50]"#,
                r#"[0.10, 0.10,1.50]"#,
            ),
            (r#"复制路径 ["/0",""] [0.10]"#, r#"0.10"#),
        ] {
            assert_eq!(transform_json(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            (
                r#"复制路径 ["/a"] {"a":1}"#,
                JsonError::SourceAndTargetPaths,
            ),
            (r#"复制路径 ["/a","/b"] {}"#, JsonError::MissingValue),
            (r#"复制路径 ["/a","/b/c"] {"a":1}"#, JsonError::MissingValue),
            (
                r#"复制路径 ["/a","/b"] {"a":1,"a":2}"#,
                JsonError::DuplicateKey,
            ),
        ] {
            assert_eq!(transform_json(query), Err(error), "{query}");
        }
    }

    #[test]
    fn pipelines_json_operations_and_keeps_literal_spelling_across_formats() {
        assert_eq!(
            transform_json(r#"依次处理 ["去空字段","补字段 {\"币种\":\"CNY\"}","选键 [\"编号\",\"金额\",\"币种\"]","转CSV"] [{"编号":"001","金额":1.50,"备注":null},{"编号":"002","金额":2e0,"币种":"USD"}]"#).as_deref(),
            Ok("编号,金额,币种\r\n001,1.50,CNY\r\n002,2e0,USD"),
        );
        assert_eq!(
            transform_json("pipeline [\"合并行\",\"展开数组\",\"合并行\"]\r\n{\"金额\":1.50}\r\n{\"金额\":2e0}").as_deref(),
            Ok("[{\"金额\":1.50},{\"金额\":2e0}]"),
        );
        assert_eq!(
            transform_json(r#"依次处理 ["压缩","取值 /缺失","格式化"] {"金额":1.50}"#),
            Err(JsonError::PipelineStep {
                index: 2,
                cause: JsonError::MissingValue.message(),
            }),
        );
        assert_eq!(
            transform_json(r#"依次处理 ["pipeline []"] {}"#),
            Err(JsonError::PipelineStep {
                index: 1,
                cause: JsonError::NestedPipeline.message(),
            }),
        );
        for query in [
            r#"依次处理 ["压缩"]{}"#,
            r#"依次处理 [] {}"#,
            r#"依次处理 ["压缩"] "#,
            r#"依次处理 ["压缩\n格式化"] {}"#,
        ] {
            assert_eq!(
                transform_json(query),
                Err(JsonError::PipelineArguments),
                "{query}"
            );
        }
    }

    #[test]
    fn applies_json_patch_steps_in_order_with_exact_test_values() {
        let patch = r#"应用补丁 [{"op":"test","path":"/a/0","value":1e0},{"op":"remove","path":"/a/0"},{"op":"copy","from":"/a/0","path":"/copy"},{"op":"move","from":"/a/0","path":"/a/-"},{"op":"add","path":"/a/0","value":null},{"op":"replace","path":"/a/1","value":3.50}] {"a":[1.00,2e0,"\\u8336"],"old":0.10}"#;
        assert_eq!(
            transform_json(patch).as_deref(),
            Ok(r#"{"a":[null,3.50,2e0],"old":0.10,"copy":2e0}"#)
        );
        assert_eq!(
            transform_json(r#"apply-patch [] { "a": 1.50 }"#).as_deref(),
            Ok(r#"{ "a": 1.50 }"#)
        );
        let failure = r#"应用补丁 [{"op":"add","path":"/a","value":null},{"op":"test","path":"/a","value":false},{"op":"remove","path":"/a"}] {}"#;
        assert_eq!(
            transform_json(failure),
            Err(JsonError::PatchStep {
                index: 2,
                cause: JsonError::PatchTestFailed.message()
            })
        );
        assert_eq!(
            transform_json(r#"应用补丁 [{"op":"add","path":"/a"}] {}"#),
            Err(JsonError::PatchStep {
                index: 1,
                cause: JsonError::PatchArguments.message()
            })
        );
        let step = r#"{"op":"test","path":"","value":null}"#;
        let steps = std::iter::repeat_n(step, MAX_PATCH_STEPS + 1)
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(
            transform_json(&format!("应用补丁 [{steps}] null")),
            Err(JsonError::PatchLimit)
        );
    }

    #[test]
    fn merge_patch_removes_members_and_replaces_or_recursively_merges_values() {
        let query = r#"合并补丁 {"a":{"x":null,"m":2.00},"b":[ null, 3e2 ],"p":{"missing":null},"z":null} {"b":[1],"z":true,"a":{"x":3,"n":9007199254740993},"k":"\u8336"}"#;
        assert_eq!(
            transform_json(query).as_deref(),
            Ok(r#"{"b":[ null, 3e2 ],"a":{"n":9007199254740993,"m":2.00},"k":"\u8336","p":{}}"#)
        );
        for (query, expected) in [
            ("merge-patch null {}", "null"),
            ("merge-patch [1e0,null] 3", "[1e0,null]"),
            (r#"merge-patch {"x":{"a":null}} [1,2]"#, r#"{"x":{}}"#),
            (r#"merge-patch {"x":{"a":null}} {"x":1}"#, r#"{"x":{}}"#),
            (
                r#"依次处理 ["合并补丁 {\"a\":1.50}","取值 /a"] {"a":1}"#,
                "1.50",
            ),
        ] {
            assert_eq!(transform_json(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            transform_json(r#"合并补丁 {"a":1,"\u0061":2} {}"#),
            Err(JsonError::DuplicateKey)
        );
        assert_eq!(
            transform_json(r#"合并补丁 {"a":{}} {"a":{"x":1,"x":2}}"#),
            Err(JsonError::DuplicateKey)
        );
        let depth = MAX_MERGE_PATCH_DEPTH + 1;
        let deep = format!(
            r#"合并补丁 {}0{} {{}}"#,
            r#"{"x":"#.repeat(depth),
            "}".repeat(depth)
        );
        assert_eq!(transform_json(&deep), Err(JsonError::MergePatchDepth));
        let large = format!(r#"合并补丁 "{}" {}"#, "x".repeat(MAX_PRETTY_BYTES), "{}");
        assert_eq!(
            transform_json(&large),
            Err(JsonError::MergePatchOutputLimit)
        );
    }

    #[test]
    fn moves_raw_values_and_applies_the_target_path_after_removal() {
        for (query, expected) in [
            (
                r#"移动路径 ["/订单/0","/归档/-"] {"订单":[{"金额":1.50,"名字":"\u8336"}],"归档":[]}"#,
                r#"{"订单":[],"归档":[{"金额":1.50,"名字":"\u8336"}]}"#,
            ),
            (
                r#"move-path ["/0","/2"] [0.10,1.50,2e0]"#,
                "[1.50,2e0,0.10]",
            ),
            (
                r#"移动路径 ["/a","/ab"] {"a":1.50,"ab":0}"#,
                r#"{"ab":1.50}"#,
            ),
            (r#"移动路径 ["/a","/a"] { "a": 1.50 }"#, r#"{ "a": 1.50 }"#),
            (r#"移动路径 ["/0",""] [0.10]"#, "0.10"),
            (r#"移动路径 ["",""] {"a":1}"#, r#"{"a":1}"#),
        ] {
            assert_eq!(transform_json(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            (
                r#"移动路径 ["/a"] {"a":1}"#,
                JsonError::SourceAndTargetPaths,
            ),
            (r#"移动路径 ["/a","/a"] {}"#, JsonError::MissingValue),
            (
                r#"移动路径 ["/a~1b","/a~1b/child"] {"a/b":{}}"#,
                JsonError::MoveIntoDescendant,
            ),
            (
                r#"移动路径 ["","/a"] {"a":1}"#,
                JsonError::MoveIntoDescendant,
            ),
            (
                r#"移动路径 ["/0","/2/a"] [1,{"a":2},{}]"#,
                JsonError::MissingValue,
            ),
        ] {
            assert_eq!(transform_json(query), Err(error), "{query}");
        }
    }

    #[test]
    fn removes_existing_members_and_items_without_reserializing_other_json() {
        for (query, expected) in [
            (
                r#"删路径 /a~1b/1 {"a/b":[1.50, 2 , 2],"keep":1e2}"#,
                r#"{"a/b":[1.50, 2],"keep":1e2}"#,
            ),
            (
                r#"remove-path /a {"a": 1,  "\u0062" : 2.0 }"#,
                r#"{"\u0062" : 2.0 }"#,
            ),
            (
                r#"删路径 /arr/2 {"arr": [10, 20 , 30 ]}"#,
                r#"{"arr": [10, 20 ]}"#,
            ),
            (r#"删路径 /a { "a" : null }"#, "{  }"),
            (r#"删路径 /0 [null]"#, "[]"),
        ] {
            assert_eq!(transform_json(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            (r#"删路径 "" {"a":1}"#, JsonError::RemoveRoot),
            (r#"删路径 /a {"a":1,"\u0061":2}"#, JsonError::DuplicateKey),
            (r#"删路径 /01 [1,2]"#, JsonError::MissingValue),
            (r#"删路径 /a/b {"a":null}"#, JsonError::MissingValue),
        ] {
            assert_eq!(transform_json(query), Err(error), "{query}");
        }
    }

    #[test]
    fn compares_two_documents_by_pointer_and_preserves_changed_lexemes() {
        for (query, expected) in [
            (
                r#"对比 {"a/b":[1.00,null],"旧":2e0} {"a/b":[1,true],"新":null}"#,
                r#"[{"路径":"/a~1b/1","左":null,"右":true},{"路径":"/旧","左":2e0},{"路径":"/新","右":null}]"#,
            ),
            (r#"diff {"b":-0e9,"a":"\u8336"} {"a":"茶","b":0}"#, "[]"),
            (r#"对比 [1] [1,2.50]"#, r#"[{"路径":"/1","右":2.50}]"#),
            (r#"对比 [] {}"#, r#"[{"路径":"","左":[],"右":{}}]"#),
            (
                r#"对比 "左" false"#,
                r#"[{"路径":"","左":"左","右":false}]"#,
            ),
        ] {
            assert_eq!(transform_json(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            transform_json(r#"对比 {"x":1,"\u0078":2} {"x":1}"#),
            Err(JsonError::DuplicateKey)
        );
        assert_eq!(transform_json("对比 {}"), Err(JsonError::Incomplete));
    }

    #[test]
    fn compares_json_in_csv_preserving_null_absence_and_original_values() {
        let query = r#"对比CSV {"a/b":[1.00,null],"旧":2e0} {"a/b":[1,true],"新":null}"#;
        let expected = "指针,变化,左值,右值\r\n/a~1b/1,修改,null,true\r\n/旧,移除,2e0,\"\"\r\n/新,新增,\"\",null";
        assert_eq!(transform_json(query).as_deref(), Ok(expected));
        assert_eq!(
            json_csv_report(query).expect("差异应显示为表格").as_deref(),
            Ok(expected)
        );
        assert_eq!(
            transform_json("diff-csv [1.50] [1.5]").as_deref(),
            Ok("指针,变化,左值,右值")
        );
        assert_eq!(
            transform_json(r#"对比CSV {"a\u0000":1} {"a\u0000":2}"#),
            Err(JsonError::CsvNullCharacter)
        );
    }

    #[test]
    fn distinguishes_bad_pointers_missing_values_and_duplicate_keys() {
        for query in ["get /a~2 {}", "get a {}", r#"get "/a"{}"#] {
            assert_eq!(transform_json(query), Err(JsonError::InvalidPointer));
        }
        for query in ["get /a {}", "get /- [1]", "get /01 [1,2]", "get /0 null"] {
            assert_eq!(transform_json(query), Err(JsonError::MissingValue));
        }
        assert_eq!(
            transform_json(r#"get /a {"a":1,"\u0061":2}"#),
            Err(JsonError::DuplicateKey)
        );
        assert_eq!(transform_json("get /a {"), Err(JsonError::Incomplete));
    }
}
