use std::{
    borrow::Cow,
    cmp::Ordering,
    collections::{HashMap, HashSet},
    hash::Hash,
};

use num_bigint::BigInt;
use num_rational::BigRational;
use regex::Regex;
use serde::{
    de::DeserializeOwned,
    ser::{Serialize, SerializeMap, Serializer},
    Deserialize,
};
use serde_json::value::RawValue;

use super::{
    calculator::normalize,
    fraction::{
        calculate_fraction_with_variables, ensure_exact_size, parse_fraction_literal,
        rounded_decimal, FractionError, MAX_DECIMAL_PLACES,
    },
    operators::{
        select_index_ranges, FieldSortOrder, IndexRangeError, NumberComparison, MAX_SORT_FIELDS,
    },
    statistics::{summarize_unweighted, StatisticsError},
    text_tools::{compare_natural, compile_regex, newline_pattern, TextError},
};

const MAX_PIVOT_BYTES: usize = 1024 * 1024;
const MAX_EXPANDED_CSV_BYTES: usize = 1024 * 1024;
const MAX_STACK_TABLES: usize = 16;
const MAX_VALUE_MAPPINGS: usize = 32;
const MAX_FORMULA_RECORDS: usize = 32;
const MAX_ADDED_CELLS: usize = 16_384;
const MAX_PIPELINE_STEPS: usize = 8;
const MAX_FILTER_CONDITIONS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CsvError {
    MissingInput,
    UnknownOperation,
    UnclosedQuote,
    InvalidSeparator,
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
    FillTooLarge,
    ConstantColumnArguments,
    RowNumberArguments,
    RowNumberOverflow,
    ConcatenatedColumnArguments,
    FirstNonemptyColumnArguments,
    SplitColumnArguments,
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
    ComputedDivisionByZero,
    ComputedNumberRange,
    InvalidColumnValue,
    InvalidNumberFilter,
    FilterConditions,
    Regex(TextError),
    ColumnOutOfRange,
    InvalidNumber,
    NumericIssuesTooLarge,
    DecimalColumnsArguments,
    DecimalColumnsTooLarge,
    StatisticsColumn,
    AggregateColumn,
    GroupingColumns,
    SortFields,
    PivotColumns,
    UnpivotArguments,
    UnpivotColumns,
    PivotTooLarge,
    EmptyStatisticsColumn,
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
            Self::FillTooLarge => "填充结果超过 1 MiB，请缩小表格范围",
            Self::ConstantColumnArguments => {
                "用 JSON 数组填写新列名和固定文本，例如 [\"来源\",\"人工复核\"]"
            }
            Self::RowNumberArguments => "填写新列名和非负整数起点，例如 [\"序号\",1]",
            Self::RowNumberOverflow => "行号超过无符号 64 位整数范围",
            Self::ConcatenatedColumnArguments => {
                "填写新列名、至少两列和分隔符，例如 [\"地址\",[\"省\",\"市\"],\"-\"]"
            }
            Self::FirstNonemptyColumnArguments => {
                "填写新列名和至少两个候选列，例如 [\"联系方式\",[\"手机\",\"座机\"]]"
            }
            Self::SplitColumnArguments => {
                "填写来源列、至少两个不同的新列名和非空分隔符，例如 [\"地址\",[\"省\",\"市\"],\"/\"]"
            }
            Self::AddedColumnTooLarge => "追加列后的结果超过 1 MiB，请缩小表格范围",
            Self::StackArguments => "用 JSON 字符串数组填写 2 至 16 张含表头的 CSV 表格",
            Self::PipelineArguments => "用 JSON 字符串数组填写 1 至 8 条单行 CSV 命令，再粘贴原表格",
            Self::NestedPipeline => "每一步填写一条 CSV 命令",
            Self::PipelineOutputTooLarge => "处理结果超过 1 MiB，请缩小表格范围",
            Self::PipelineStep { cause, .. } => cause,
            Self::ComputedColumnArguments => {
                "用 JSON 数组填写新列名、两列和运算，例如 [\"金额\",2,\"*\",3]"
            }
            Self::FormulaColumnArguments => {
                "用 JSON 数组填写新列名、x 列、y 列和算式，例如 [\"金额\",2,3,\"x*y+1/6\"]"
            }
            Self::FormulaColumnLimit => "公式列每次最多处理 32 条数据",
            Self::FormulaEvaluation(error) => error.message(),
            Self::ComputedDivisionByZero => "计算列中遇到了除以零",
            Self::ComputedNumberRange => "计算列的结果超出精确计算范围",
            Self::InvalidColumnValue => "输入列号或带双引号的列名，再输入带双引号的值",
            Self::InvalidNumberFilter => "输入列号或带双引号的列名、比较符和数值",
            Self::FilterConditions => {
                "填写 1 至 16 个 [列,比较方式,\"值\"]，例如 [[\"状态\",\"文本=\",\"完成\"],[2,\">=\",\"3/2\"]]"
            }
            Self::Regex(error) => error.message(),
            Self::ColumnOutOfRange => "选中的列号超出了表格列数",
            Self::InvalidNumber => "所选列须填写数字或分数，例如 -2.5、1/3 或 3e-2",
            Self::NumericIssuesTooLarge => "数值列检查结果超过 1 MiB，请缩小表格范围",
            Self::DecimalColumnsArguments => "填写不同的列和 0 至 100 位小数，例如 [[\"金额\",3],2]",
            Self::DecimalColumnsTooLarge => "小数列结果超过 1 MiB，请缩小表格范围",
            Self::StatisticsColumn => "列统计需要一个列号或带双引号的列名",
            Self::AggregateColumn => "填写一个要汇总的数值列号或带双引号的列名",
            Self::GroupingColumns => "分组列须非空且互不重复，例如 [\"地区\",\"状态\"]",
            Self::SortFields => {
                "填写 1 至 8 个 [列,排序方式]，例如 [[\"地区\",\"文本升序\"],[2,\"数值降序\"]]"
            }
            Self::PivotColumns => {
                "用 JSON 数组填写互不重复的行、列和数值列，例如 [\"地区\",\"状态\",\"金额\"]"
            }
            Self::UnpivotArguments => {
                "用 JSON 数组填写保留列、展开后的列名和值列名，例如 [[\"地区\"],\"月份\",\"金额\"]"
            }
            Self::UnpivotColumns => "至少留一列用于展开",
            Self::PivotTooLarge => "透视表结果超过 1 MiB，请缩小数据范围",
            Self::EmptyStatisticsColumn => "请在表头下至少提供一条数据",
            Self::Statistics(error) => error.message(),
            Self::Serialization => "无法生成 JSON",
            Self::JsonNumericCells => "所选列须为 JSON 数字或空白，例如 1.50 或 2e0",
            Self::JsonNumericObjectsTooLarge => "转换后的 JSON 超过 1 MiB，请缩小表格范围",
        }
    }
}

enum Operation {
    Validate,
    PadRows,
    JsonRows,
    JsonObjects,
    JsonNumericObjects(Vec<ColumnReference>),
    Markdown(Option<Vec<MarkdownAlignment>>),
    Transpose,
    ToTsv,
    FromTsv,
    SelectColumns(Vec<usize>),
    SelectNamedColumns(Vec<String>),
    DropColumns(Vec<ColumnReference>),
    TrimColumns(Vec<ColumnReference>),
    FillDown(Vec<ColumnReference>),
    CountEmptyCells(Vec<ColumnReference>),
    CheckNumericColumns(Vec<ColumnReference>),
    DecimalColumns(Vec<ColumnReference>, u32),
    DropEmptyRecords,
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
    FirstNonemptyColumn(String, Vec<ColumnReference>),
    SplitColumn {
        source: ColumnReference,
        names: Vec<String>,
        separator: String,
    },
    ComputeColumn {
        name: String,
        left: ColumnReference,
        right: ColumnReference,
        arithmetic: ColumnArithmetic,
    },
    DeduplicateRows,
    DeduplicateByColumns(Vec<ColumnReference>),
    DuplicatesByColumns(Vec<ColumnReference>),
    DuplicatesByNumbers(Vec<ColumnReference>),
    DeduplicateByNumbers(Vec<ColumnReference>),
    FilterColumn {
        column: ColumnReference,
        condition: ColumnCondition,
        keep_matches: bool,
    },
    FilterMultiple {
        filters: Vec<ColumnFilter>,
        any: bool,
    },
    CountByColumns(Vec<ColumnReference>),
    AggregateByColumns {
        columns: Vec<ColumnReference>,
        amount: ColumnReference,
        aggregation: ColumnAggregation,
    },
    PivotSum([ColumnReference; 3]),
    Unpivot(Vec<ColumnReference>, [String; 2]),
    ColumnStatistics {
        column: ColumnReference,
        sample: bool,
    },
    SortByColumns {
        columns: Vec<ColumnReference>,
        descending: bool,
        numeric: bool,
    },
    SortWithOrders(Vec<(ColumnReference, FieldSortOrder)>),
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
enum ColumnReference {
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

enum ColumnArithmetic {
    Add,
    Subtract,
    Multiply,
    Divide,
    Formula(String),
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

    fn apply(&self, left: BigRational, right: BigRational) -> Result<BigRational, CsvError> {
        Ok(match self {
            Self::Add => left + right,
            Self::Subtract => left - right,
            Self::Multiply => left * right,
            Self::Divide if right == BigRational::default() => {
                return Err(CsvError::ComputedDivisionByZero);
            }
            Self::Divide => left / right,
            Self::Formula(expression) => {
                return calculate_fraction_with_variables(expression, Some(&left), Some(&right))
                    .map_err(|error| match error {
                        FractionError::ZeroDenominator => CsvError::ComputedDivisionByZero,
                        FractionError::Invalid | FractionError::Incomplete => {
                            CsvError::FormulaColumnArguments
                        }
                        _ => CsvError::FormulaEvaluation(error),
                    });
            }
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
    let operation = operation.to_ascii_lowercase();
    let kind = match operation.as_str() {
        "依次处理" | "pipeline" => return Err(CsvError::NestedPipeline),
        "校验" | "validate" => Operation::Validate,
        "补齐列" | "pad-columns" => Operation::PadRows,
        "左连接" | "left-join" => return join_csv(input, JoinKind::Left),
        "内连接" | "inner-join" => return join_csv(input, JoinKind::Inner),
        "全连接" | "full-join" => return join_csv(input, JoinKind::Full),
        "左表匹配" | "semi-join" => return join_csv(input, JoinKind::MatchedLeft),
        "左表独有" | "anti-join" => return join_csv(input, JoinKind::UnmatchedLeft),
        "纵向合并" | "stack" => return stack_csv(input),
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
        "转置" | "transpose" => Operation::Transpose,
        "转tsv" | "tsv" => Operation::ToTsv,
        "从tsv" | "from-tsv" => Operation::FromTsv,
        "去重" | "deduplicate" => Operation::DeduplicateRows,
        "去空行" | "drop-empty-rows" => Operation::DropEmptyRecords,
        operation @ ("取行" | "pick-rows" | "删行" | "drop-rows") => {
            let (ranges, data) = split_argument(input).ok_or(CsvError::MissingInput)?;
            input = data;
            Operation::SelectRows {
                ranges: ranges.to_owned(),
                keep_selected: matches!(operation, "取行" | "pick-rows"),
            }
        }
        "选列" | "pick-columns" => {
            let (columns, data) = column_arguments(input)?;
            input = data;
            Operation::SelectColumns(columns)
        }
        "选列名" | "pick-by-header" => {
            let (names, data) = json_argument_and_data(input, CsvError::InvalidColumnNames)?;
            input = data;
            Operation::SelectNamedColumns(names)
        }
        "去列" | "drop-columns" => {
            let (columns, data) =
                column_reference_arguments(input, CsvError::NamedColumnArguments)?;
            input = data;
            Operation::DropColumns(columns)
        }
        "去空白" | "trim-columns" => {
            let (columns, data) =
                column_reference_arguments(input, CsvError::NamedColumnArguments)?;
            input = data;
            Operation::TrimColumns(columns)
        }
        "向下填充" | "fill-down" => {
            let (columns, data) =
                column_reference_arguments(input, CsvError::NamedColumnArguments)?;
            input = data;
            Operation::FillDown(columns)
        }
        "空值统计" | "count-empty" => {
            let (columns, data) =
                column_reference_arguments(input, CsvError::NamedColumnArguments)?;
            input = data;
            Operation::CountEmptyCells(columns)
        }
        "检查数值列" | "check-numeric-columns" => {
            let (columns, data) =
                column_reference_arguments(input, CsvError::NamedColumnArguments)?;
            input = data;
            Operation::CheckNumericColumns(columns)
        }
        "小数列" | "decimal-columns" => {
            let ((columns, places), data) = json_argument_and_data::<(Vec<ColumnReference>, u32)>(
                input,
                CsvError::DecimalColumnsArguments,
            )?;
            if columns.is_empty() || places > MAX_DECIMAL_PLACES {
                return Err(CsvError::DecimalColumnsArguments);
            }
            input = data;
            Operation::DecimalColumns(columns, places)
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
        "首个非空" | "coalesce-columns" => {
            let ((name, columns), data) = json_argument_and_data::<(String, Vec<ColumnReference>)>(
                input,
                CsvError::FirstNonemptyColumnArguments,
            )?;
            input = data;
            Operation::FirstNonemptyColumn(name, columns)
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
                right,
                arithmetic: ColumnArithmetic::parse(&operator)?,
            }
        }
        "公式列" | "formula-column" => {
            let ((name, left, right, expression), data) =
                json_argument_and_data::<(String, ColumnReference, ColumnReference, String)>(
                    input,
                    CsvError::FormulaColumnArguments,
                )?;
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
        "按列去重" | "deduplicate-columns" | "数值去重" | "deduplicate-numeric" => {
            let (columns, data) =
                column_reference_arguments(input, CsvError::NamedColumnArguments)?;
            input = data;
            if matches!(operation.as_str(), "数值去重" | "deduplicate-numeric") {
                Operation::DeduplicateByNumbers(columns)
            } else {
                Operation::DeduplicateByColumns(columns)
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
        "筛选" | "filter" | "排除" | "exclude" | "正则筛选" | "regex-filter" | "正则排除"
        | "regex-exclude" => {
            let (column, value, data) = column_value_arguments(input)?;
            input = data;
            let condition = match operation.as_str() {
                "筛选" | "filter" | "排除" | "exclude" => ColumnCondition::Exact(value),
                _ => ColumnCondition::Regex(compile_regex(&value).map_err(CsvError::Regex)?),
            };
            Operation::FilterColumn {
                column,
                condition,
                keep_matches: matches!(
                    operation.as_str(),
                    "筛选" | "filter" | "正则筛选" | "regex-filter"
                ),
            }
        }
        "数值筛选" | "filter-numeric" => {
            let (column, rest) =
                column_reference_argument(input.trim_start(), CsvError::InvalidNumberFilter)?;
            let (operator, rest) =
                split_argument(rest.trim_start()).ok_or(CsvError::InvalidNumberFilter)?;
            let (threshold, data) =
                split_argument(rest.trim_start()).ok_or(CsvError::InvalidNumberFilter)?;
            let comparison =
                NumberComparison::parse(operator).ok_or(CsvError::InvalidNumberFilter)?;
            let threshold =
                parse_fraction_literal(threshold).map_err(|_| CsvError::InvalidNumber)?;
            input = data;
            Operation::FilterColumn {
                column,
                condition: ColumnCondition::Number(comparison, threshold),
                keep_matches: true,
            }
        }
        "多条件筛选" | "filter-all" | "任一条件筛选" | "filter-any" => {
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
                            let comparison = NumberComparison::parse(&operator)
                                .ok_or(CsvError::FilterConditions)?;
                            let threshold = parse_fraction_literal(&value)
                                .map_err(|_| CsvError::InvalidNumber)?;
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
            input = data;
            Operation::FilterMultiple {
                filters,
                any: matches!(operation.as_str(), "任一条件筛选" | "filter-any"),
            }
        }
        "分组计数" | "group-count" => {
            let (columns, data) = column_reference_arguments(input, CsvError::GroupingColumns)?;
            input = data;
            Operation::CountByColumns(columns)
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
        "列统计" | "column-stats" | "列总体" | "column-population" => {
            let (column, data) = single_column_argument(input, CsvError::StatisticsColumn)?;
            input = data;
            Operation::ColumnStatistics {
                column,
                sample: matches!(operation.as_str(), "列统计" | "column-stats"),
            }
        }
        "透视合计" | "pivot-sum" => {
            let (columns, data) = json_argument_and_data(input, CsvError::PivotColumns)?;
            input = data;
            Operation::PivotSum(columns)
        }
        "逆透视" | "unpivot" => {
            let ((columns, label, value), data) =
                json_argument_and_data(input, CsvError::UnpivotArguments)?;
            input = data;
            Operation::Unpivot(columns, [label, value])
        }
        "多列排序" | "sort-by-fields" => {
            let (rules, data) = json_argument_and_data::<Vec<(ColumnReference, FieldSortOrder)>>(
                input,
                CsvError::SortFields,
            )?;
            if !(1..=MAX_SORT_FIELDS).contains(&rules.len()) {
                return Err(CsvError::SortFields);
            }
            input = data;
            Operation::SortWithOrders(rules)
        }
        "排序" | "sort-columns" | "降序" | "sort-columns-desc" | "数值排序" | "sort-numeric"
        | "数值降序" | "sort-numeric-desc" => {
            let (columns, data) =
                column_reference_arguments(input, CsvError::NamedColumnArguments)?;
            input = data;
            Operation::SortByColumns {
                columns,
                descending: matches!(
                    operation.as_str(),
                    "降序" | "sort-columns-desc" | "数值降序" | "sort-numeric-desc"
                ),
                numeric: matches!(
                    operation.as_str(),
                    "数值排序" | "sort-numeric" | "数值降序" | "sort-numeric-desc"
                ),
            }
        }
        _ => return Err(CsvError::UnknownOperation),
    };
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    if input.is_empty() {
        return Err(CsvError::MissingInput);
    }

    let delimiter = match kind {
        Operation::FromTsv => '\t',
        _ => ',',
    };
    let rows = if matches!(
        kind,
        Operation::Validate | Operation::PadRows | Operation::DropEmptyRecords
    ) {
        match read_delimited_rows(input, delimiter, false) {
            Ok(rows) => rows,
            Err(error) if matches!(kind, Operation::Validate) => {
                return Ok(format!(
                    "CSV 语法错误 · 第 {} 条记录 · 第 {} 列\n{}",
                    error.record,
                    error.column,
                    error.kind.message()
                ));
            }
            Err(error) => return Err(error.kind),
        }
    } else {
        read_rows(input, delimiter)?
    };
    match kind {
        Operation::Validate => Ok(report_csv_columns(&rows)),
        Operation::PadRows => pad_rows(rows),
        Operation::JsonRows => serde_json::to_string(&rows).map_err(|_| CsvError::Serialization),
        Operation::JsonObjects => json_objects(&rows, None),
        Operation::JsonNumericObjects(columns) => json_objects(&rows, Some(&columns)),
        Operation::Markdown(alignments) => markdown_table(&rows, alignments.as_deref()),
        Operation::Transpose => Ok(transpose(&rows)),
        Operation::ToTsv => Ok(write_rows(&rows, '\t')),
        Operation::FromTsv => Ok(write_rows(&rows, ',')),
        Operation::SelectColumns(columns) => select_columns(&rows, &columns),
        Operation::SelectNamedColumns(names) => select_named_columns(&rows, &names),
        Operation::DropColumns(columns) => drop_columns(&rows, &columns),
        Operation::TrimColumns(columns) => trim_column_values(rows, &columns),
        Operation::FillDown(columns) => fill_down_csv(&rows, &columns),
        Operation::CountEmptyCells(columns) => count_empty_cells(&rows, &columns),
        Operation::CheckNumericColumns(columns) => check_numeric_columns(&rows, &columns),
        Operation::DecimalColumns(columns, places) => {
            format_decimal_columns(&rows, &columns, places)
        }
        Operation::DropEmptyRecords => drop_empty_csv_records(&rows),
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
        Operation::FirstNonemptyColumn(name, columns) => {
            first_nonempty_csv_column(&rows, &name, &columns)
        }
        Operation::SplitColumn {
            source,
            names,
            separator,
        } => split_column(&rows, source, &names, &separator),
        Operation::ComputeColumn {
            name,
            left,
            right,
            arithmetic,
        } => compute_column(rows, name, left, right, arithmetic),
        Operation::DeduplicateRows => {
            let mut seen = HashSet::new();
            Ok(write_rows(rows.iter().filter(|row| seen.insert(*row)), ','))
        }
        Operation::DeduplicateByColumns(columns) => {
            deduplicate_by_columns(&rows, &columns, |row, columns| Ok(column_key(row, columns)))
        }
        Operation::DuplicatesByColumns(columns) => {
            find_duplicate_columns(&rows, &columns, |row, columns| Ok(column_key(row, columns)))
        }
        Operation::DuplicatesByNumbers(columns) => {
            find_duplicate_columns(&rows, &columns, numeric_column_key)
        }
        Operation::DeduplicateByNumbers(columns) => {
            deduplicate_by_columns(&rows, &columns, numeric_column_key)
        }
        Operation::FilterColumn {
            column,
            condition,
            keep_matches,
        } => filter_column(&rows, column, &condition, keep_matches),
        Operation::FilterMultiple { filters, any } => filter_multiple_columns(&rows, &filters, any),
        Operation::CountByColumns(columns) => count_by_columns(&rows, &columns),
        Operation::AggregateByColumns {
            columns,
            amount,
            aggregation,
        } => aggregate_by_columns(&rows, &columns, amount, aggregation),
        Operation::PivotSum(columns) => pivot_sum(&rows, &columns),
        Operation::Unpivot(columns, names) => unpivot(&rows, &columns, &names),
        Operation::ColumnStatistics { column, sample } => summarize_column(&rows, column, sample),
        Operation::SortByColumns {
            columns,
            descending,
            numeric,
        } => sort_by_columns(&rows, &columns, descending, numeric),
        Operation::SortWithOrders(rules) => sort_with_orders(&rows, rules),
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
    let [left, right] =
        serde_json::from_str::<[String; 2]>(data).map_err(|_| CsvError::JoinArguments)?;
    let left = read_rows(left.strip_prefix('\u{feff}').unwrap_or(&left), ',')?;
    let right = read_rows(right.strip_prefix('\u{feff}').unwrap_or(&right), ',')?;
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

fn stack_csv(input: &str) -> Result<String, CsvError> {
    let inputs: Vec<String> = serde_json::from_str(input).map_err(|_| CsvError::StackArguments)?;
    if !(2..=MAX_STACK_TABLES).contains(&inputs.len()) {
        return Err(CsvError::StackArguments);
    }
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

fn joined_csv_columns<'a>(row: &'a [String], omitted: &'a [bool]) -> impl Iterator<Item = &'a str> {
    row.iter()
        .zip(omitted)
        .filter(|(_, omitted)| !**omitted)
        .map(|(field, _)| field.as_str())
}

fn append_bounded_csv_row<'a>(
    output: &mut String,
    fields: impl Iterator<Item = &'a str>,
    too_large: CsvError,
) -> Result<(), CsvError> {
    let mut separator = if output.is_empty() { "" } else { "\r\n" };
    for field in fields {
        append_bounded_csv_field(output, separator, field, too_large)?;
        separator = ",";
    }
    Ok(())
}

fn append_bounded_csv_field(
    output: &mut String,
    separator: &str,
    field: &str,
    too_large: CsvError,
) -> Result<(), CsvError> {
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
    let input = input.trim_start();
    let mut values = serde_json::Deserializer::from_str(input).into_iter::<T>();
    let value = values.next().ok_or(error)?.map_err(|_| error)?;
    let (_, data) = split_argument(&input[values.byte_offset()..])
        .filter(|(argument, _)| argument.is_empty())
        .ok_or(error)?;
    Ok((value, data))
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

fn select_data_rows(
    rows: &[Vec<String>],
    ranges: &str,
    keep_selected: bool,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let selected = select_index_ranges(ranges, records.len()).map_err(|error| match error {
        IndexRangeError::Invalid => CsvError::InvalidRowRange,
        IndexRangeError::OutOfBounds => CsvError::MissingDataRow,
    })?;
    let picked = records
        .iter()
        .zip(selected)
        .filter_map(|(row, selected)| (selected == keep_selected).then_some(row));
    Ok(write_rows(std::iter::once(header).chain(picked), ','))
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

fn drop_columns(rows: &[Vec<String>], columns: &[ColumnReference]) -> Result<String, CsvError> {
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

fn select_columns(rows: &[Vec<String>], columns: &[usize]) -> Result<String, CsvError> {
    let width = rows.first().ok_or(CsvError::MissingInput)?.len();
    validate_column_width(columns, width)?;
    let selected: Vec<_> = rows
        .iter()
        .map(|row| columns.iter().map(|&index| row[index].clone()).collect())
        .collect();
    Ok(write_rows(&selected, ','))
}

fn select_named_columns(rows: &[Vec<String>], names: &[String]) -> Result<String, CsvError> {
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

fn format_decimal_columns(
    rows: &[Vec<String>],
    columns: &[ColumnReference],
    places: u32,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let selected: HashSet<_> =
        resolve_distinct_columns(header, columns, CsvError::DecimalColumnsArguments)?
            .into_iter()
            .collect();
    let mut output = String::new();
    let too_large = CsvError::DecimalColumnsTooLarge;
    append_bounded_csv_row(&mut output, header.iter().map(String::as_str), too_large)?;
    for row in records {
        let mut separator = "\r\n";
        for (index, field) in row.iter().enumerate() {
            let value = if selected.contains(&index) && !field.is_empty() {
                let number = parse_fraction_literal(field).map_err(|_| CsvError::InvalidNumber)?;
                Cow::Owned(rounded_decimal(number, places))
            } else {
                Cow::Borrowed(field.as_str())
            };
            append_bounded_csv_field(&mut output, separator, &value, too_large)?;
            separator = ",";
        }
    }
    Ok(output)
}

fn check_numeric_columns(
    rows: &[Vec<String>],
    columns: &[ColumnReference],
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let columns = resolve_distinct_columns(header, columns, CsvError::NamedColumnArguments)?;
    let mut output = String::from("数据行,列号,列名,内容");
    for (index, record) in records.iter().enumerate() {
        let row_number = (index + 1).to_string();
        for &column in &columns {
            let cell = &record[column];
            if parse_fraction_literal(cell).is_ok() {
                continue;
            }
            let column_number = (column + 1).to_string();
            append_bounded_csv_row(
                &mut output,
                [
                    row_number.as_str(),
                    column_number.as_str(),
                    header[column].as_str(),
                    cell.as_str(),
                ]
                .into_iter(),
                CsvError::NumericIssuesTooLarge,
            )?;
        }
    }
    Ok(output)
}

fn fill_down_csv(rows: &[Vec<String>], columns: &[ColumnReference]) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let columns = resolve_distinct_columns(header, columns, CsvError::NamedColumnArguments)?;
    let mut output = String::new();
    append_bounded_csv_row(
        &mut output,
        header.iter().map(String::as_str),
        CsvError::FillTooLarge,
    )?;
    let mut last = vec![""; columns.len()];
    let mut fields = Vec::with_capacity(header.len());
    for row in records {
        fields.clear();
        fields.extend(row.iter().map(String::as_str));
        for (&column, latest) in columns.iter().zip(&mut last) {
            if fields[column].is_empty() {
                fields[column] = *latest;
            } else {
                *latest = fields[column];
            }
        }
        append_bounded_csv_row(&mut output, fields.iter().copied(), CsvError::FillTooLarge)?;
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

fn write_appended_column<'a, T: AsRef<str>>(
    header: &[String],
    records: &'a [Vec<String>],
    name: &str,
    mut cell_for: impl FnMut(&'a [String]) -> Result<T, CsvError>,
) -> Result<String, CsvError> {
    if header.iter().any(|cell| cell == name) {
        return Err(CsvError::HeaderNameConflict);
    }
    let mut output = String::new();
    append_bounded_csv_row(
        &mut output,
        header
            .iter()
            .map(String::as_str)
            .chain(std::iter::once(name)),
        CsvError::AddedColumnTooLarge,
    )?;
    for row in records {
        let cell = cell_for(row)?;
        append_bounded_csv_row(
            &mut output,
            row.iter()
                .map(String::as_str)
                .chain(std::iter::once(cell.as_ref())),
            CsvError::AddedColumnTooLarge,
        )?;
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

fn compute_column(
    mut rows: Vec<Vec<String>>,
    name: String,
    left: ColumnReference,
    right: ColumnReference,
    arithmetic: ColumnArithmetic,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first_mut().ok_or(CsvError::MissingInput)?;
    if name.trim().is_empty() || name.contains('\0') {
        return Err(CsvError::ComputedColumnArguments);
    }
    let left = left.resolve(header)?;
    let right = right.resolve(header)?;
    validate_column_width(&[left, right], header.len())?;
    if header.contains(&name) {
        return Err(CsvError::HeaderNameConflict);
    }
    if matches!(arithmetic, ColumnArithmetic::Formula(_)) && records.len() > MAX_FORMULA_RECORDS {
        return Err(CsvError::FormulaColumnLimit);
    }
    header.push(name);
    for row in records {
        let left_value = parse_fraction_literal(&row[left]).map_err(|_| CsvError::InvalidNumber)?;
        let right_value =
            parse_fraction_literal(&row[right]).map_err(|_| CsvError::InvalidNumber)?;
        let result = arithmetic.apply(left_value, right_value)?;
        ensure_exact_size(&result).map_err(|_| CsvError::ComputedNumberRange)?;
        row.push(result.to_string());
    }
    Ok(write_rows(&rows, ','))
}

fn deduplicate_by_columns<'a, Key: Eq + Hash>(
    rows: &'a [Vec<String>],
    columns: &[ColumnReference],
    key: impl Fn(&'a [String], &[usize]) -> Result<Key, CsvError>,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let columns = resolve_distinct_columns(header, columns, CsvError::NamedColumnArguments)?;
    let mut seen = HashSet::new();
    let mut unique_records = vec![header];
    for row in records {
        if seen.insert(key(row, &columns)?) {
            unique_records.push(row);
        }
    }
    Ok(write_rows(unique_records, ','))
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

fn filter_column(
    rows: &[Vec<String>],
    column: ColumnReference,
    condition: &ColumnCondition,
    keep_matches: bool,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let column = column.resolve(header)?;
    validate_column_width(&[column], header.len())?;
    let mut selected = vec![header];
    for row in records {
        if condition.matches(&row[column])? == keep_matches {
            selected.push(row);
        }
    }
    Ok(write_rows(selected, ','))
}

fn filter_multiple_columns(
    rows: &[Vec<String>],
    filters: &[ColumnFilter],
    any: bool,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let columns = filters
        .iter()
        .map(|filter| filter.column.resolve(header))
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
    Ok(write_rows(selected, ','))
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
    let mut groups = HashMap::new();
    for (position, row) in records.iter().enumerate() {
        groups
            .entry(column_key(row, &columns))
            .and_modify(|(_, count)| *count += 1)
            .or_insert((position, 1_usize));
    }
    Ok(grouped_csv(header, &columns, groups, "计数"))
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

fn pivot_sum(rows: &[Vec<String>], columns: &[ColumnReference; 3]) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let columns = resolve_distinct_columns(header, columns, CsvError::PivotColumns)?;
    let (row_column, column_column, amount_column) = (columns[0], columns[1], columns[2]);
    let mut row_labels = Vec::new();
    let mut column_labels = Vec::new();
    let mut row_positions = HashMap::new();
    let mut column_positions = HashMap::new();
    let mut totals = HashMap::<_, BigRational>::new();
    for record in records {
        let value =
            parse_fraction_literal(&record[amount_column]).map_err(|_| CsvError::InvalidNumber)?;
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
        let total = totals.entry((row, column)).or_default();
        *total += value;
        ensure_exact_size(total)
            .map_err(|error| CsvError::Statistics(StatisticsError::Evaluation(error)))?;
    }

    let mut output = csv_cell(&header[row_column]).into_owned();
    for label in &column_labels {
        output.push(',');
        output.push_str(&csv_cell(label));
    }
    let separators = column_labels.len().saturating_mul(row_labels.len());
    if output.len().saturating_add(separators) > MAX_PIVOT_BYTES {
        return Err(CsvError::PivotTooLarge);
    }
    for (row, label) in row_labels.into_iter().enumerate() {
        output.push_str("\r\n");
        output.push_str(&csv_cell(label));
        for column in 0..column_labels.len() {
            output.push(',');
            if let Some(value) = totals.get(&(row, column)) {
                output.push_str(&value.to_string());
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

fn sort_by_columns(
    rows: &[Vec<String>],
    columns: &[ColumnReference],
    descending: bool,
    numeric: bool,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let columns = resolve_distinct_columns(header, columns, CsvError::NamedColumnArguments)?;
    let ordered = order_records(records.iter(), &columns, descending, numeric)?;
    Ok(write_rows(std::iter::once(header).chain(ordered), ','))
}

fn sort_with_orders(
    rows: &[Vec<String>],
    rules: Vec<(ColumnReference, FieldSortOrder)>,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let mut seen = HashSet::new();
    let fields = rules
        .into_iter()
        .map(|(reference, order)| {
            let column = reference.resolve(header)?;
            validate_column_width(&[column], header.len())?;
            if !seen.insert(column) {
                return Err(CsvError::SortFields);
            }
            Ok((column, order))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut ordered: Vec<_> = records.iter().collect();
    for (column, order) in fields.into_iter().rev() {
        ordered = order_records(
            ordered.into_iter(),
            &[column],
            order.descending(),
            order.numeric(),
        )?;
    }
    Ok(write_rows(std::iter::once(header).chain(ordered), ','))
}

fn order_records<'a>(
    records: impl Iterator<Item = &'a Vec<String>>,
    columns: &[usize],
    descending: bool,
    numeric: bool,
) -> Result<Vec<&'a Vec<String>>, CsvError> {
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
                        parse_fraction_literal(&row[column]).map_err(|_| CsvError::InvalidNumber)
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
                .map(|&column| compare_natural(&left[column], &right[column]))
                .find(|order| !order.is_eq())
                .unwrap_or(Ordering::Equal);
            direction(order)
        });
        ordered
    };
    Ok(ordered)
}

fn summarize_column(
    rows: &[Vec<String>],
    column: ColumnReference,
    sample: bool,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let column = column.resolve(header)?;
    validate_column_width(&[column], header.len())?;
    let values = records
        .iter()
        .map(|row| parse_fraction_literal(&row[column]).map_err(|_| CsvError::InvalidNumber))
        .collect::<Result<Vec<_>, _>>()?;
    summarize_unweighted(values, sample).map_err(|error| match error {
        StatisticsError::Incomplete => CsvError::EmptyStatisticsColumn,
        error => CsvError::Statistics(error),
    })
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

fn transpose(rows: &[Vec<String>]) -> String {
    let columns = rows.first().map_or(0, Vec::len);
    (0..columns)
        .map(|column| {
            rows.iter()
                .map(|row| csv_cell(&row[column]))
                .collect::<Vec<_>>()
                .join(",")
        })
        .collect::<Vec<_>>()
        .join("\r\n")
}

fn write_rows<'a>(rows: impl IntoIterator<Item = &'a Vec<String>>, delimiter: char) -> String {
    let separator = delimiter.to_string();
    rows.into_iter()
        .map(|row| {
            row.iter()
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
}

fn read_rows(input: &str, delimiter: char) -> Result<Vec<Vec<String>>, CsvError> {
    read_delimited_rows(input, delimiter, true).map_err(|error| error.kind)
}

fn read_delimited_rows(
    mut input: &str,
    delimiter: char,
    require_uniform_width: bool,
) -> Result<Vec<Vec<String>>, CsvReadError> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut fields = Vec::new();
    loop {
        let record = rows.len() + 1;
        let column = fields.len() + 1;
        let at_field = |kind| CsvReadError {
            kind,
            record,
            column,
        };
        let (field, rest) = read_field(input, delimiter).map_err(at_field)?;
        fields.push(field);
        if let Some(next) = rest.strip_prefix(delimiter) {
            input = next;
            continue;
        }
        input = if rest.is_empty() {
            rest
        } else {
            rest.strip_prefix("\r\n")
                .or_else(|| rest.strip_prefix('\n'))
                .ok_or_else(|| at_field(CsvError::InvalidSeparator))?
        };
        if require_uniform_width
            && rows
                .first()
                .is_some_and(|first| first.len() != fields.len())
        {
            return Err(at_field(CsvError::DifferentColumnCount));
        }
        rows.push(std::mem::take(&mut fields));
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
        let mut query = "pivot-sum [1,2,3] 地区,状态,金额".to_owned();
        for index in 0..1025 {
            query.push_str(&format!("\n行{index},列{index},1"));
        }
        assert_eq!(transform_csv(&query), Err(CsvError::PivotTooLarge));
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
