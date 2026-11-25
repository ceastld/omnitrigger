use std::{iter, ops::Range, sync::OnceLock};

use regex::Regex;

use super::{
    cidr::calculate_cidr,
    conversion::convert_wave,
    csv_tools::{append_bounded_csv_row, csv_cell, CsvError},
    decibel::convert_decibels,
    solid::calculate_solid,
    tempo::calculate_tempo,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BatchError<E> {
    Empty,
    Row { line: usize, cause: E },
}

pub(super) fn batch_arguments<'a>(query: &'a str, name: &str, alias: &str) -> Option<&'a str> {
    let (operation, arguments) = query.split_once(char::is_whitespace).unwrap_or((query, ""));
    (operation == name || operation.eq_ignore_ascii_case(alias)).then_some(arguments)
}

pub(super) enum BatchCalculationKind {
    Standard,
    StandardCsv,
    StandardReport,
    Exact,
    ExactCsv,
    ExactReport,
}

pub(super) fn batch_calculation_input(input: &str) -> (BatchCalculationKind, &str) {
    for (name, alias, kind) in [
        ("分数", "exact", BatchCalculationKind::Exact),
        ("分数对照", "exact-csv", BatchCalculationKind::ExactCsv),
        ("对照", "csv", BatchCalculationKind::StandardCsv),
        ("明细", "report", BatchCalculationKind::StandardReport),
        (
            "分数明细",
            "exact-report",
            BatchCalculationKind::ExactReport,
        ),
    ] {
        if let Some(arguments) = batch_arguments(input, name, alias) {
            return (kind, arguments);
        }
    }
    (BatchCalculationKind::Standard, input)
}

pub(super) fn batch_line_range(input: &str, line: usize) -> Option<Range<usize>> {
    nonempty_batch_lines(input)
        .find(|&(number, _, _)| number == line)
        .map(|(_, range, _)| range)
}

pub(super) enum BatchToolOutput {
    Lines(String),
    Csv(String),
}

pub(super) fn map_tool_batch(
    query: &str,
) -> Option<Result<BatchToolOutput, BatchError<&'static str>>> {
    let (operation, input) = query.split_once(char::is_whitespace).unwrap_or((query, ""));
    let operation = operation.to_ascii_lowercase();
    let csv_operation = operation
        .strip_suffix("对照")
        .or_else(|| operation.strip_suffix("-csv"));
    let convert: fn(&str) -> Result<String, &'static str> =
        match csv_operation.unwrap_or(&operation) {
            "节拍" | "tempo" => |line| calculate_tempo(line).map_err(|error| error.message()),
            "分贝" | "decibel" => |line| convert_decibels(line).map_err(|error| error.message()),
            "立体" | "solid" => |line| calculate_solid(line).map_err(|error| error.message()),
            "波长" | "wave" => |line| convert_wave(line).map_err(|error| error.message()),
            "网段" | "cidr" => |line| calculate_cidr(line).map_err(|error| error.message()),
            _ => return None,
        };
    Some(if csv_operation.is_some() {
        map_batch_csv(input, "输入", convert).map(BatchToolOutput::Csv)
    } else {
        map_batch_lines(input, convert).map(BatchToolOutput::Lines)
    })
}

pub(super) fn map_batch_lines<E>(
    input: &str,
    operation: impl FnMut(&str) -> Result<String, E>,
) -> Result<String, BatchError<E>> {
    batch_results(input, operation).map(|values| values.join("\n"))
}

pub(super) fn map_batch_csv<E>(
    input: &str,
    input_header: &str,
    mut operation: impl FnMut(&str) -> Result<String, E>,
) -> Result<String, BatchError<E>> {
    let rows = batch_results(input, |query| {
        let result = operation(query)?;
        Ok(format!("{},{}", csv_cell(query), csv_cell(&result)))
    })?;
    Ok(format!(
        "{},结果\r\n{}",
        csv_cell(input_header),
        rows.join("\r\n")
    ))
}

pub(super) struct CalculationReport {
    pub(super) csv: String,
    pub(super) input_errors: Vec<Range<usize>>,
}

pub(super) fn batch_calculation_report(
    input: &str,
    mut calculate: impl FnMut(&str) -> Result<String, &'static str>,
) -> Result<Option<CalculationReport>, CsvError> {
    const HEADER: &str = "行号,算式,结果,错误";
    let mut csv = HEADER.to_owned();
    let mut input_errors = Vec::new();
    for (line, range, query) in nonempty_batch_lines(input) {
        let line = line.to_string();
        let result = calculate(query);
        let (result, error) = match &result {
            Ok(value) => (value.as_str(), ""),
            Err(error) => {
                input_errors.push(range);
                ("", *error)
            }
        };
        append_bounded_csv_row(
            &mut csv,
            [line.as_str(), query, result, error].into_iter(),
            CsvError::PipelineOutputTooLarge,
        )?;
    }
    Ok((csv.len() > HEADER.len()).then_some(CalculationReport { csv, input_errors }))
}

fn nonempty_batch_lines(input: &str) -> impl Iterator<Item = (usize, Range<usize>, &str)> {
    static BREAKS: OnceLock<Regex> = OnceLock::new();
    let breaks = BREAKS
        .get_or_init(|| Regex::new(r"\r\n|[\r\n\u{2028}\u{2029}]").expect("固定的换行表达式有效"));
    let mut start = 0;
    breaks
        .find_iter(input)
        .map(|line_break| line_break.range())
        .chain(iter::once(input.len()..input.len()))
        .enumerate()
        .filter_map(move |(index, ending)| {
            let line = &input[start..ending.start];
            let rest = line.trim_start();
            let query_start = ending.start - rest.len();
            let query = rest.trim_end();
            start = ending.end;
            (!query.is_empty()).then_some((
                index + 1,
                query_start..query_start + query.len(),
                query,
            ))
        })
}

fn batch_results<E>(
    input: &str,
    mut operation: impl FnMut(&str) -> Result<String, E>,
) -> Result<Vec<String>, BatchError<E>> {
    let mut values = Vec::new();
    for (line, _, query) in nonempty_batch_lines(input) {
        let value = operation(query).map_err(|cause| BatchError::Row { line, cause })?;
        values.push(value);
    }
    if values.is_empty() {
        return Err(BatchError::Empty);
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_the_same_line_numbers_for_batch_results_and_input_selections() {
        let input = "甲\r\n \r乙\u{2028}\u{2029} 丙 \n🍵";
        assert_eq!(
            map_batch_lines(input, |line| Ok::<_, ()>(line.to_owned())).as_deref(),
            Ok("甲\n乙\n丙\n🍵")
        );
        for (number, text) in [(1, "甲"), (3, "乙"), (5, "丙"), (6, "🍵")] {
            assert_eq!(&input[batch_line_range(input, number).unwrap()], text);
        }
        for number in [0, 2, 4, 7] {
            assert!(batch_line_range(input, number).is_none());
        }
        assert_eq!(
            batch_calculation_report(input, |line| Ok(line.to_owned()))
                .unwrap()
                .as_ref()
                .map(|report| report.csv.as_str()),
            Some("行号,算式,结果,错误\r\n1,甲,甲,\"\"\r\n3,乙,乙,\"\"\r\n5,丙,丙,\"\"\r\n6,🍵,🍵,\"\""),
        );
    }
}
