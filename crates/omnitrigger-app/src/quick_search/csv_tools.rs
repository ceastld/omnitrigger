use std::{
    borrow::Cow,
    cmp::Ordering,
    collections::{HashMap, HashSet},
};

use num_bigint::BigInt;
use num_rational::BigRational;
use regex::Regex;
use serde::{
    de::DeserializeOwned,
    ser::{Serialize, SerializeMap, Serializer},
    Deserialize,
};

use super::{
    fraction::{ensure_exact_size, parse_fraction_literal},
    operators::{select_index_ranges, IndexRangeError, NumberComparison},
    statistics::{summarize_unweighted, StatisticsError},
    text_tools::{compare_natural, compile_regex, TextError},
};

const MAX_PIVOT_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CsvError {
    MissingInput,
    UnknownOperation,
    UnclosedQuote,
    InvalidSeparator,
    DifferentColumnCount,
    InvalidHeader,
    InvalidColumns,
    InvalidColumnNames,
    InvalidRowRange,
    MissingDataRow,
    MissingColumnName,
    AmbiguousColumnName,
    RenameArguments,
    HeaderNameConflict,
    ComputedColumnArguments,
    ComputedDivisionByZero,
    ComputedNumberRange,
    InvalidColumnValue,
    InvalidNumberFilter,
    Regex(TextError),
    ColumnOutOfRange,
    InvalidNumber,
    StatisticsColumn,
    AggregateColumn,
    GroupingColumns,
    PivotColumns,
    PivotTooLarge,
    EmptyStatisticsColumn,
    Statistics(StatisticsError),
    Serialization,
}

impl CsvError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::MissingInput => "在操作名后粘贴表格，可用换行开始数据",
            Self::UnknownOperation => "输入 ? CSV 查看可用操作",
            Self::UnclosedQuote => "字段的双引号尚未闭合",
            Self::InvalidSeparator => "检查引号和分隔符，引号内的双引号用两个双引号表示",
            Self::DifferentColumnCount => "每行的列数须一致",
            Self::InvalidHeader => "CSV 表头的列名须非空且唯一",
            Self::InvalidColumns => "用逗号隔开互不重复的列号，列号从 1 开始",
            Self::InvalidColumnNames => "用 JSON 字符串数组输入互不重复的列名",
            Self::InvalidRowRange => "填写从 1 开始的数据行号或区间，例如 2,4-6",
            Self::MissingDataRow => "指定行号超出了表头之后的数据行数",
            Self::MissingColumnName => "表头中找不到所选列名",
            Self::AmbiguousColumnName => "表头中存在多个同名列",
            Self::RenameArguments => "用 JSON 数组填写原列名和非空的新列名",
            Self::HeaderNameConflict => "表头中已经有这个新列名",
            Self::ComputedColumnArguments => {
                "用 JSON 数组填写新列名、两列和运算，例如 [\"金额\",2,\"*\",3]"
            }
            Self::ComputedDivisionByZero => "计算列中遇到了除以零",
            Self::ComputedNumberRange => "计算列的结果超出精确计算范围",
            Self::InvalidColumnValue => "输入列号或带双引号的列名，再输入带双引号的值",
            Self::InvalidNumberFilter => "输入列号或带双引号的列名、比较符和数值",
            Self::Regex(error) => error.message(),
            Self::ColumnOutOfRange => "选中的列号超出了表格列数",
            Self::InvalidNumber => "所选列须填写数字或分数，例如 -2.5、1/3 或 3e-2",
            Self::StatisticsColumn => "列统计需要一个列号或带双引号的列名",
            Self::AggregateColumn => "填写一个要汇总的数值列号或带双引号的列名",
            Self::GroupingColumns => "分组列须非空且互不重复，例如 [\"地区\",\"状态\"]",
            Self::PivotColumns => {
                "用 JSON 数组填写互不重复的行、列和数值列，例如 [\"地区\",\"状态\",\"金额\"]"
            }
            Self::PivotTooLarge => "透视表结果超过 1 MiB，请缩小数据范围",
            Self::EmptyStatisticsColumn => "请在表头下至少提供一条数据",
            Self::Statistics(error) => error.message(),
            Self::Serialization => "无法生成 JSON",
        }
    }
}

enum Operation {
    JsonRows,
    JsonObjects,
    Markdown,
    Transpose,
    ToTsv,
    FromTsv,
    SelectColumns(Vec<usize>),
    SelectNamedColumns(Vec<String>),
    SelectRows(String),
    RenameColumn([String; 2]),
    ComputeColumn {
        name: String,
        left: ColumnReference,
        right: ColumnReference,
        arithmetic: ColumnArithmetic,
    },
    DeduplicateRows,
    DeduplicateByColumns(Vec<usize>),
    FilterColumn {
        column: ColumnReference,
        condition: ColumnCondition,
        keep_matches: bool,
    },
    CountByColumns(Vec<ColumnReference>),
    AggregateByColumns {
        columns: Vec<ColumnReference>,
        amount: ColumnReference,
        average: bool,
    },
    PivotSum([ColumnReference; 3]),
    ColumnStatistics {
        column: ColumnReference,
        sample: bool,
    },
    SortByColumns {
        columns: Vec<usize>,
        descending: bool,
        numeric: bool,
    },
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
        })
    }
}

enum ColumnCondition {
    Exact(String),
    Regex(Regex),
    Number(NumberComparison, BigRational),
}

pub(super) fn transform_csv(query: &str) -> Result<String, CsvError> {
    let (operation, mut input) = split_argument(query).ok_or(CsvError::MissingInput)?;
    let operation = operation.to_ascii_lowercase();
    let kind = match operation.as_str() {
        "转json" | "json" => Operation::JsonRows,
        "对象" | "表头" | "objects" => Operation::JsonObjects,
        "markdown" | "表格" => Operation::Markdown,
        "转置" | "transpose" => Operation::Transpose,
        "转tsv" | "tsv" => Operation::ToTsv,
        "从tsv" | "from-tsv" => Operation::FromTsv,
        "去重" | "deduplicate" => Operation::DeduplicateRows,
        "取行" | "pick-rows" => {
            let (ranges, data) = split_argument(input).ok_or(CsvError::MissingInput)?;
            input = data;
            Operation::SelectRows(ranges.to_owned())
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
        "改列名" | "rename-header" => {
            let (names, data) = json_argument_and_data(input, CsvError::RenameArguments)?;
            input = data;
            Operation::RenameColumn(names)
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
        "按列去重" | "deduplicate-columns" => {
            let (columns, data) = column_arguments(input)?;
            input = data;
            Operation::DeduplicateByColumns(columns)
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
        "分组计数" | "group-count" => {
            let (columns, data) = grouping_column_arguments(input)?;
            input = data;
            Operation::CountByColumns(columns)
        }
        "分组合计" | "group-sum" | "分组平均" | "group-average" => {
            let (columns, rest) = grouping_column_arguments(input)?;
            let (amount, data) = single_column_argument(rest, CsvError::AggregateColumn)?;
            input = data;
            Operation::AggregateByColumns {
                columns,
                amount,
                average: matches!(operation.as_str(), "分组平均" | "group-average"),
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
        "排序" | "sort-columns" | "降序" | "sort-columns-desc" | "数值排序" | "sort-numeric"
        | "数值降序" | "sort-numeric-desc" => {
            let (columns, data) = column_arguments(input)?;
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
    let rows = read_rows(input, delimiter)?;
    match kind {
        Operation::JsonRows => serde_json::to_string(&rows).map_err(|_| CsvError::Serialization),
        Operation::JsonObjects => json_objects(&rows),
        Operation::Markdown => Ok(markdown_table(&rows)),
        Operation::Transpose => Ok(transpose(&rows)),
        Operation::ToTsv => Ok(write_rows(&rows, '\t')),
        Operation::FromTsv => Ok(write_rows(&rows, ',')),
        Operation::SelectColumns(columns) => select_columns(&rows, &columns),
        Operation::SelectNamedColumns(names) => select_named_columns(&rows, &names),
        Operation::SelectRows(ranges) => select_data_rows(&rows, &ranges),
        Operation::RenameColumn(names) => rename_column(rows, names),
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
        Operation::DeduplicateByColumns(columns) => deduplicate_by_columns(&rows, &columns),
        Operation::FilterColumn {
            column,
            condition,
            keep_matches,
        } => filter_column(&rows, column, &condition, keep_matches),
        Operation::CountByColumns(columns) => count_by_columns(&rows, &columns),
        Operation::AggregateByColumns {
            columns,
            amount,
            average,
        } => aggregate_by_columns(&rows, &columns, amount, average),
        Operation::PivotSum(columns) => pivot_sum(&rows, &columns),
        Operation::ColumnStatistics { column, sample } => summarize_column(&rows, column, sample),
        Operation::SortByColumns {
            columns,
            descending,
            numeric,
        } => sort_by_columns(&rows, &columns, descending, numeric),
    }
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

fn grouping_column_arguments(input: &str) -> Result<(Vec<ColumnReference>, &str), CsvError> {
    if input.trim_start().starts_with('[') {
        let (columns, data) =
            json_argument_and_data::<Vec<ColumnReference>>(input, CsvError::GroupingColumns)?;
        if columns.is_empty() {
            return Err(CsvError::GroupingColumns);
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

fn select_data_rows(rows: &[Vec<String>], ranges: &str) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let selected = select_index_ranges(ranges, records.len()).map_err(|error| match error {
        IndexRangeError::Invalid => CsvError::InvalidRowRange,
        IndexRangeError::OutOfBounds => CsvError::MissingDataRow,
    })?;
    let picked = records
        .iter()
        .zip(selected)
        .filter_map(|(row, selected)| selected.then_some(row));
    Ok(write_rows(std::iter::once(header).chain(picked), ','))
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

fn deduplicate_by_columns(rows: &[Vec<String>], columns: &[usize]) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    validate_column_width(columns, header.len())?;
    let mut seen = HashSet::new();
    let unique_records = records
        .iter()
        .filter(|row| seen.insert(column_key(row, columns)));
    Ok(write_rows(
        std::iter::once(header).chain(unique_records),
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
        let matched = match condition {
            ColumnCondition::Exact(value) => row[column] == value.as_str(),
            ColumnCondition::Regex(pattern) => pattern.is_match(&row[column]),
            ColumnCondition::Number(comparison, threshold) => {
                let value =
                    parse_fraction_literal(&row[column]).map_err(|_| CsvError::InvalidNumber)?;
                comparison.accepts(value.cmp(threshold))
            }
        };
        if matched == keep_matches {
            selected.push(row);
        }
    }
    Ok(write_rows(selected, ','))
}

fn resolve_grouping_columns(
    header: &[String],
    columns: &[ColumnReference],
) -> Result<Vec<usize>, CsvError> {
    let mut seen = HashSet::new();
    let columns = columns
        .iter()
        .map(|column| {
            let index = column.resolve(header)?;
            if !seen.insert(index) {
                return Err(CsvError::GroupingColumns);
            }
            Ok(index)
        })
        .collect::<Result<Vec<_>, _>>()?;
    validate_column_width(&columns, header.len())?;
    Ok(columns)
}

fn count_by_columns(rows: &[Vec<String>], columns: &[ColumnReference]) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let columns = resolve_grouping_columns(header, columns)?;
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
    average: bool,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let columns = resolve_grouping_columns(header, columns)?;
    let amount = amount.resolve(header)?;
    validate_column_width(&[amount], header.len())?;
    let mut groups = HashMap::new();
    for (position, row) in records.iter().enumerate() {
        let value = parse_fraction_literal(&row[amount]).map_err(|_| CsvError::InvalidNumber)?;
        let (_, (total, count)) = groups
            .entry(column_key(row, &columns))
            .or_insert_with(|| (position, (BigRational::default(), 0_usize)));
        *total += value;
        *count += 1;
        ensure_exact_size(total)
            .map_err(|error| CsvError::Statistics(StatisticsError::Evaluation(error)))?;
    }
    if average {
        for (_, (total, count)) in groups.values_mut() {
            *total = std::mem::take(total) / BigInt::from(*count);
            ensure_exact_size(total)
                .map_err(|error| CsvError::Statistics(StatisticsError::Evaluation(error)))?;
        }
    }
    let groups = groups
        .into_iter()
        .map(|(key, (position, (value, _)))| (key, (position, value)));
    Ok(grouped_csv(
        header,
        &columns,
        groups,
        if average { "平均值" } else { "合计" },
    ))
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

fn column_key<'a>(row: &'a [String], columns: &[usize]) -> Vec<&'a str> {
    columns.iter().map(|&index| row[index].as_str()).collect()
}

fn pivot_sum(rows: &[Vec<String>], columns: &[ColumnReference; 3]) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let columns = resolve_grouping_columns(header, columns).map_err(|error| match error {
        CsvError::GroupingColumns => CsvError::PivotColumns,
        error => error,
    })?;
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

fn sort_by_columns(
    rows: &[Vec<String>],
    columns: &[usize],
    descending: bool,
    numeric: bool,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    validate_column_width(columns, header.len())?;
    let direction = |order: Ordering| {
        if descending {
            order.reverse()
        } else {
            order
        }
    };
    let ordered = if numeric {
        let mut keyed = records
            .iter()
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
        let mut ordered: Vec<_> = records.iter().collect();
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
    Ok(write_rows(std::iter::once(header).chain(ordered), ','))
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

fn json_objects(rows: &[Vec<String>]) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    let mut names = HashSet::new();
    if header
        .iter()
        .any(|name| name.trim().is_empty() || !names.insert(name))
    {
        return Err(CsvError::InvalidHeader);
    }
    let objects: Vec<_> = records
        .iter()
        .map(|fields| CsvObject { header, fields })
        .collect();
    serde_json::to_string(&objects).map_err(|_| CsvError::Serialization)
}

fn markdown_table(rows: &[Vec<String>]) -> String {
    let mut lines = Vec::with_capacity(rows.len() + 1);
    for (index, row) in rows.iter().enumerate() {
        let cells = row
            .iter()
            .map(|cell| markdown_cell(cell))
            .collect::<Vec<_>>();
        lines.push(format!("| {} |", cells.join(" | ")));
        if index == 0 {
            lines.push(format!("|{}", " --- |".repeat(row.len())));
        }
    }
    lines.join("\n")
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
        if matches!(
            ch,
            '\\' | '|' | '`' | '*' | '_' | '~' | '[' | ']' | '<' | '>' | '&' | '$'
        ) {
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

fn read_rows(mut input: &str, delimiter: char) -> Result<Vec<Vec<String>>, CsvError> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut fields = Vec::new();
    loop {
        let (field, rest) = read_field(input, delimiter)?;
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
                .ok_or(CsvError::InvalidSeparator)?
        };
        if rows
            .first()
            .is_some_and(|first| first.len() != fields.len())
        {
            return Err(CsvError::DifferentColumnCount);
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
}

impl Serialize for CsvObject<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut object = serializer.serialize_map(Some(self.header.len()))?;
        for (key, value) in self.header.iter().zip(self.fields) {
            object.serialize_entry(key, value)?;
        }
        object.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_csv_records_in_input_order_preserving_multiline_fields() {
        let data = "编号,备注\r\n001,茶\r\n002,\"梨\r\n苹果\"\r\n003,咖啡\r\n004,柿子";
        assert_eq!(
            transform_csv(&format!("取行 4,2-3,2\r\n{data}")).as_deref(),
            Ok("编号,备注\r\n002,\"梨\r\n苹果\"\r\n003,咖啡\r\n004,柿子")
        );
        for (query, error) in [
            ("pick-rows 0 编号,备注\n001,茶", CsvError::InvalidRowRange),
            ("pick-rows 3-2 编号,备注\n001,茶", CsvError::InvalidRowRange),
            ("pick-rows 2 编号,备注\n001,茶", CsvError::MissingDataRow),
            ("pick-rows 1 编号,备注", CsvError::MissingDataRow),
        ] {
            assert_eq!(transform_csv(query), Err(error), "{query}");
        }
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
