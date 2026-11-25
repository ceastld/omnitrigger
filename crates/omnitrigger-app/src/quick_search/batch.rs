use std::ops::Range;

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
    let mut offset = 0;
    for (index, part) in input.split_inclusive('\n').enumerate() {
        if index + 1 == line {
            let trimmed = part.trim();
            let start = offset + part.len() - part.trim_start().len();
            return (!trimmed.is_empty()).then_some(start..start + trimmed.len());
        }
        offset += part.len();
    }
    None
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

pub(super) fn batch_calculation_report(
    input: &str,
    mut calculate: impl FnMut(&str) -> Result<String, &'static str>,
) -> Result<Option<String>, CsvError> {
    const HEADER: &str = "行号,算式,结果,错误";
    let mut output = HEADER.to_owned();
    for (line, query) in nonempty_batch_lines(input) {
        let line = line.to_string();
        let result = calculate(query);
        let (result, error) = match &result {
            Ok(value) => (value.as_str(), ""),
            Err(error) => ("", *error),
        };
        append_bounded_csv_row(
            &mut output,
            [line.as_str(), query, result, error].into_iter(),
            CsvError::PipelineOutputTooLarge,
        )?;
    }
    Ok((output.len() > HEADER.len()).then_some(output))
}

fn nonempty_batch_lines(input: &str) -> impl Iterator<Item = (usize, &str)> {
    input.lines().enumerate().filter_map(|(index, line)| {
        let query = line.trim();
        (!query.is_empty()).then_some((index + 1, query))
    })
}

fn batch_results<E>(
    input: &str,
    mut operation: impl FnMut(&str) -> Result<String, E>,
) -> Result<Vec<String>, BatchError<E>> {
    let mut values = Vec::new();
    for (line, query) in nonempty_batch_lines(input) {
        let value = operation(query).map_err(|cause| BatchError::Row { line, cause })?;
        values.push(value);
    }
    if values.is_empty() {
        return Err(BatchError::Empty);
    }
    Ok(values)
}
