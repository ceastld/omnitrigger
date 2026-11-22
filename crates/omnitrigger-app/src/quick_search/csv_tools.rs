use std::{
    borrow::Cow,
    cmp::Ordering,
    collections::{HashMap, HashSet},
};

use num_rational::BigRational;
use serde::{
    de::DeserializeOwned,
    ser::{Serialize, SerializeMap, Serializer},
};

use super::{
    fraction::{ensure_exact_size, parse_fraction_literal},
    statistics::{summarize_unweighted, StatisticsError},
    text_tools::compare_natural,
};

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
    MissingColumnName,
    AmbiguousColumnName,
    RenameArguments,
    RenameConflict,
    InvalidColumnValue,
    ColumnOutOfRange,
    InvalidNumber,
    StatisticsColumn,
    TotalColumn,
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
            Self::MissingColumnName => "表头中找不到所选列名",
            Self::AmbiguousColumnName => "表头中存在多个同名列",
            Self::RenameArguments => "用 JSON 数组填写原列名和非空的新列名",
            Self::RenameConflict => "表头中已经有这个新列名",
            Self::InvalidColumnValue => "请输入列号和带双引号的值，再粘贴表格",
            Self::ColumnOutOfRange => "选中的列号超出了表格列数",
            Self::InvalidNumber => "所选列须填写数字或分数，例如 -2.5、1/3 或 3e-2",
            Self::StatisticsColumn => "列统计需要一个列号，列号从 1 开始",
            Self::TotalColumn => "填写一个需要合计的列号，列号从 1 开始",
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
    RenameColumn([String; 2]),
    DeduplicateRows,
    DeduplicateByColumns(Vec<usize>),
    FilterColumn {
        column: usize,
        value: String,
        keep_matches: bool,
    },
    CountByColumns(Vec<usize>),
    SumByColumns {
        columns: Vec<usize>,
        amount: usize,
    },
    ColumnStatistics {
        column: usize,
        sample: bool,
    },
    SortByColumns {
        columns: Vec<usize>,
        descending: bool,
        numeric: bool,
    },
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
        "按列去重" | "deduplicate-columns" => {
            let (columns, data) = column_arguments(input)?;
            input = data;
            Operation::DeduplicateByColumns(columns)
        }
        "筛选" | "filter" | "排除" | "exclude" => {
            let (column, value, data) = column_value_arguments(input)?;
            input = data;
            Operation::FilterColumn {
                column,
                value,
                keep_matches: matches!(operation.as_str(), "筛选" | "filter"),
            }
        }
        "分组计数" | "group-count" => {
            let (columns, data) = column_arguments(input)?;
            input = data;
            Operation::CountByColumns(columns)
        }
        "分组合计" | "group-sum" => {
            let (columns, rest) = column_arguments(input)?;
            let (amounts, data) = column_arguments(rest)?;
            let [amount] = amounts.as_slice() else {
                return Err(CsvError::TotalColumn);
            };
            input = data;
            Operation::SumByColumns {
                columns,
                amount: *amount,
            }
        }
        "列统计" | "column-stats" | "列总体" | "column-population" => {
            let (columns, data) = column_arguments(input)?;
            let [column] = columns.as_slice() else {
                return Err(CsvError::StatisticsColumn);
            };
            input = data;
            Operation::ColumnStatistics {
                column: *column,
                sample: matches!(operation.as_str(), "列统计" | "column-stats"),
            }
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
        Operation::RenameColumn(names) => rename_column(rows, names),
        Operation::DeduplicateRows => {
            let mut seen = HashSet::new();
            Ok(write_rows(rows.iter().filter(|row| seen.insert(*row)), ','))
        }
        Operation::DeduplicateByColumns(columns) => deduplicate_by_columns(&rows, &columns),
        Operation::FilterColumn {
            column,
            value,
            keep_matches,
        } => filter_column(&rows, column, &value, keep_matches),
        Operation::CountByColumns(columns) => count_by_columns(&rows, &columns),
        Operation::SumByColumns { columns, amount } => sum_by_columns(&rows, &columns, amount),
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

fn column_value_arguments(input: &str) -> Result<(usize, String, &str), CsvError> {
    let (column, input) = split_argument(input).ok_or(CsvError::InvalidColumnValue)?;
    let column = column_index(column)?;
    let (value, data) = json_argument_and_data(input, CsvError::InvalidColumnValue)?;
    Ok((column, value, data))
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
    if !input.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(CsvError::InvalidColumns);
    }
    input
        .parse::<usize>()
        .ok()
        .and_then(|number| number.checked_sub(1))
        .ok_or(CsvError::InvalidColumns)
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
        return Err(CsvError::RenameConflict);
    }
    header[column] = new;
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
    column: usize,
    value: &str,
    keep_matches: bool,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    validate_column_width(&[column], header.len())?;
    let matched = records
        .iter()
        .filter(|row| (row[column] == value) == keep_matches);
    Ok(write_rows(std::iter::once(header).chain(matched), ','))
}

fn count_by_columns(rows: &[Vec<String>], columns: &[usize]) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    validate_column_width(columns, header.len())?;
    let mut groups = HashMap::new();
    for (position, row) in records.iter().enumerate() {
        groups
            .entry(column_key(row, columns))
            .and_modify(|(_, count)| *count += 1)
            .or_insert((position, 1_usize));
    }
    Ok(grouped_csv(header, columns, groups, "计数"))
}

fn sum_by_columns(
    rows: &[Vec<String>],
    columns: &[usize],
    amount: usize,
) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
    validate_column_width(columns, header.len())?;
    validate_column_width(&[amount], header.len())?;
    let mut groups = HashMap::new();
    for (position, row) in records.iter().enumerate() {
        let value = parse_fraction_literal(&row[amount]).map_err(|_| CsvError::InvalidNumber)?;
        let (_, total) = groups
            .entry(column_key(row, columns))
            .or_insert_with(|| (position, BigRational::default()));
        *total += value;
        ensure_exact_size(total)
            .map_err(|error| CsvError::Statistics(StatisticsError::Evaluation(error)))?;
    }
    Ok(grouped_csv(header, columns, groups, "合计"))
}

fn grouped_csv<T: ToString>(
    header: &[String],
    columns: &[usize],
    groups: HashMap<Vec<&str>, (usize, T)>,
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

fn summarize_column(rows: &[Vec<String>], column: usize, sample: bool) -> Result<String, CsvError> {
    let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
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
        ] {
            assert_eq!(transform_csv(query), Err(error), "{query}");
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
            (r#"改列名 ["A","B"] A,B"#, CsvError::RenameConflict),
            (r#"改列名 ["a","Z"] A,B"#, CsvError::MissingColumnName),
            (r#"改列名 ["A","Z"] A,A"#, CsvError::AmbiguousColumnName),
        ] {
            assert_eq!(transform_csv(query), Err(error), "{query}");
        }
    }

    #[test]
    fn sums_csv_values_by_column_tuple_in_first_occurrence_order() {
        let data = "编号,地区,状态,金额\n001,\"东,区\",完成,1/2\n002,西,待办,2.5e0\n003,\"东,区\",完成,1.5\n004,\"东,区\",待办,-1";
        assert_eq!(
            transform_csv(&format!("分组合计 3,2 4\n{data}")).as_deref(),
            Ok("状态,地区,合计\r\n完成,\"东,区\",2\r\n待办,西,5/2\r\n待办,\"东,区\",-1")
        );
        assert_eq!(
            transform_csv("group-sum 1 2 名称,金额").as_deref(),
            Ok("名称,合计")
        );
        for (query, error) in [
            ("分组合计 1 1,2 名称,金额", CsvError::TotalColumn),
            ("分组合计 1 3 名称,金额", CsvError::ColumnOutOfRange),
            ("分组合计 1 2 名称,金额\n甲,待定", CsvError::InvalidNumber),
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
