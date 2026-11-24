use super::{
    cidr::calculate_cidr, conversion::convert_wave, csv_tools::csv_cell, decibel::convert_decibels,
    solid::calculate_solid, tempo::calculate_tempo,
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

fn batch_results<E>(
    input: &str,
    mut operation: impl FnMut(&str) -> Result<String, E>,
) -> Result<Vec<String>, BatchError<E>> {
    let mut values = Vec::new();
    for (index, line) in input.lines().enumerate() {
        let query = line.trim();
        if query.is_empty() {
            continue;
        }
        let value = operation(query).map_err(|cause| BatchError::Row {
            line: index + 1,
            cause,
        })?;
        values.push(value);
    }
    if values.is_empty() {
        return Err(BatchError::Empty);
    }
    Ok(values)
}
