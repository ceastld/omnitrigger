use std::borrow::Cow;

use chrono::{DateTime, FixedOffset};
use serde_json::value::RawValue;

use super::{
    bin_plot::BinPlot,
    box_plot::BoxPlot,
    calendar::{calculate_date, is_date_report},
    complex::calculate_complex,
    conversion::{calculate_circuit, convert_wave},
    correlation_plot::CorrelationPlot,
    csv_tools::{
        append_bounded_csv_row, append_transformed_csv_column, build_column_box_plot,
        build_correlation_plot, build_scatter_plot, column_box_plot_arguments,
        correlation_plot_arguments, csv_count_chart, csv_table_preview, parse_table_preview,
        scatter_arguments, transform_csv, ColumnReference, CsvError,
    },
    decibel::convert_decibels,
    duration::convert_duration,
    equation::{equation_chart, solve_equation, solve_linear_system},
    fraction::{fraction_csv_report, simplify_fraction},
    json_tools::{
        json_csv_report, json_lines, parse_json_line, read_json_array, read_unique_object_entries,
        transform_json, JsonError,
    },
    linear_fit::{calculate_linear_fit, calculate_plot, plot_request},
    matrix::calculate_matrix,
    operators::json_argument_and_tail,
    path_tools::path_component,
    percentage::compare_percentages,
    pitch::convert_pitch,
    radix::convert_arbitrary_radix,
    scatter_plot::ScatterPlot,
    scope::{scoped_query, Scope},
    solid::calculate_solid,
    statistics::{
        bin_plot_arguments, box_plot_arguments, calculate_bin_plot, calculate_box_plot,
        calculate_statistics, cumulative_plot_arguments, plot_cumulative_distribution,
        statistics_csv_report,
    },
    table_preview::TablePreview,
    tempo::calculate_tempo,
    text_tools::transform_text,
    timestamp::{convert_timestamp, is_timestamp_report},
    toml_tools::{toml_csv_report, transform_toml},
    url_encoding::convert_url_component,
};

const MAX_PIPELINE_STEPS: usize = 8;
const MAX_PIPELINE_OUTPUT_BYTES: usize = 1024 * 1024;
const STEP_SYNTAX: &str =
    "每一步填写带前缀的文本、CSV、JSON、TOML、URL、路径、日期、时间戳、时长、进制、音高、节拍、分贝、立体、波长、电路、方程、联立、分数或统计命令";
const STEP_RESULT_SLOT: &str = "{结果}";
const STEP_TEMPLATE_SYNTAX: &str = "模板的工具前缀之后须正好有一个 {结果}";

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum PipelineStep {
    Append(String),
    Embed(StepTemplate),
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct StepTemplate {
    #[serde(rename = "模板")]
    command: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DataPipelineError {
    Arguments,
    CsvColumnArguments,
    Csv(CsvError),
    Json(JsonError),
    JsonLine {
        line: usize,
        cause: JsonError,
    },
    Step {
        index: usize,
        cause: &'static str,
    },
    CsvTableStep {
        index: usize,
        row: usize,
        cause: &'static str,
    },
    RowStep {
        line: usize,
        index: usize,
        cause: &'static str,
    },
    RowOutputTooLarge {
        line: usize,
    },
    CsvRowStep {
        row: usize,
        index: usize,
        cause: &'static str,
    },
    JsonItemStep {
        item: usize,
        index: usize,
        cause: &'static str,
    },
    JsonArrayTooLarge {
        item: usize,
    },
    JsonFieldStep {
        field: usize,
        index: usize,
        cause: &'static str,
    },
    JsonObjectTooLarge {
        field: usize,
    },
}

impl From<CsvError> for DataPipelineError {
    fn from(error: CsvError) -> Self {
        Self::Csv(error)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DataPipelineMode {
    LastResult,
    StepsCsv,
    Rows,
    RowsCsv,
    ColumnCsv,
    JsonArray,
    JsonObject,
    JsonLines,
}

#[derive(Clone, Copy)]
enum PipelineLocation {
    Whole,
    Line(usize),
    CsvRow(usize),
    JsonItem(usize),
    JsonField(usize),
}

pub(super) fn split_pipeline_input(input: &str) -> (DataPipelineMode, &str) {
    let input = input.trim_start();
    let (operation, arguments) = input.split_once(char::is_whitespace).unwrap_or((input, ""));
    match operation.to_ascii_lowercase().as_str() {
        "步骤csv" | "steps-csv" => (DataPipelineMode::StepsCsv, arguments),
        "逐行" | "rows" => (DataPipelineMode::Rows, arguments),
        "逐行csv" | "rows-csv" => (DataPipelineMode::RowsCsv, arguments),
        "csv列" | "csv-column" => (DataPipelineMode::ColumnCsv, arguments),
        "json数组" | "json-array" => (DataPipelineMode::JsonArray, arguments),
        "json对象" | "json-object" => (DataPipelineMode::JsonObject, arguments),
        "json行" | "json-lines" => (DataPipelineMode::JsonLines, arguments),
        _ => (DataPipelineMode::LastResult, input),
    }
}

pub(super) enum PipelinePreview {
    Scatter(ScatterPlot),
    BoxPlot(BoxPlot),
    Bins(BinPlot),
    Correlation(CorrelationPlot),
    Table(TablePreview),
}

enum PipelineView<'a> {
    Text,
    Trace,
    Preview(&'a mut Option<PipelinePreview>),
}

pub(super) fn transform_data_pipeline(
    input: &str,
    now: DateTime<FixedOffset>,
) -> Result<(String, Option<PipelinePreview>), DataPipelineError> {
    let (mode, input) = split_pipeline_input(input);
    let mut preview = None;
    let text = run_data_pipeline(mode, input, now, &mut preview)?;
    if matches!(
        mode,
        DataPipelineMode::StepsCsv | DataPipelineMode::RowsCsv | DataPipelineMode::ColumnCsv
    ) {
        preview = Some(PipelinePreview::Table(parse_table_preview(&text)?));
    }
    Ok((text, preview))
}

fn run_data_pipeline(
    mode: DataPipelineMode,
    input: &str,
    now: DateTime<FixedOffset>,
    preview: &mut Option<PipelinePreview>,
) -> Result<String, DataPipelineError> {
    let mut remaining_work = MAX_PIPELINE_OUTPUT_BYTES * MAX_PIPELINE_STEPS;
    if mode == DataPipelineMode::ColumnCsv {
        let ((name, column, steps), source) =
            json_argument_and_tail::<(String, ColumnReference, Vec<PipelineStep>)>(input)
                .ok_or(DataPipelineError::CsvColumnArguments)?;
        if name.trim().is_empty()
            || name.contains('\0')
            || !(1..=MAX_PIPELINE_STEPS).contains(&steps.len())
            || source.trim().is_empty()
        {
            return Err(DataPipelineError::CsvColumnArguments);
        }
        return append_transformed_csv_column(source, &name, column, |row, value| {
            run_pipeline_steps(
                &steps,
                value,
                now,
                PipelineView::Text,
                PipelineLocation::CsvRow(row),
                &mut remaining_work,
            )
        });
    }
    let (steps, source) =
        json_argument_and_tail::<Vec<PipelineStep>>(input).ok_or(DataPipelineError::Arguments)?;
    if !(1..=MAX_PIPELINE_STEPS).contains(&steps.len()) || source.trim().is_empty() {
        return Err(DataPipelineError::Arguments);
    }
    match mode {
        DataPipelineMode::JsonArray => {
            let values = read_json_array(source.trim_start()).map_err(DataPipelineError::Json)?;
            run_pipeline_json(
                values.into_iter().map(|value| (None, value)),
                false,
                &steps,
                now,
                &mut remaining_work,
            )
        }
        DataPipelineMode::JsonObject => {
            let entries =
                read_unique_object_entries(source.trim_start()).map_err(DataPipelineError::Json)?;
            run_pipeline_json(
                entries.into_iter().map(|(key, value)| (Some(key), value)),
                true,
                &steps,
                now,
                &mut remaining_work,
            )
        }
        DataPipelineMode::JsonLines => {
            run_pipeline_json_lines(&steps, source, now, &mut remaining_work)
        }
        DataPipelineMode::Rows | DataPipelineMode::RowsCsv => run_pipeline_rows(
            &steps,
            source,
            now,
            mode == DataPipelineMode::RowsCsv,
            &mut remaining_work,
        ),
        _ => run_pipeline_steps(
            &steps,
            source,
            now,
            if mode == DataPipelineMode::StepsCsv {
                PipelineView::Trace
            } else {
                PipelineView::Preview(preview)
            },
            PipelineLocation::Whole,
            &mut remaining_work,
        ),
    }
}

fn run_pipeline_json<'a>(
    values: impl IntoIterator<Item = (Option<String>, &'a RawValue)>,
    object: bool,
    steps: &[PipelineStep],
    now: DateTime<FixedOffset>,
    remaining_work: &mut usize,
) -> Result<String, DataPipelineError> {
    let (open, close) = if object { ("{", '}') } else { ("[", ']') };
    let mut result = String::from(open);
    for (index, (key, value)) in values.into_iter().enumerate() {
        let item = index + 1;
        let location = if object {
            PipelineLocation::JsonField(item)
        } else {
            PipelineLocation::JsonItem(item)
        };
        let encoded = run_pipeline_json_value(value, steps, now, location, remaining_work)?;
        let key = key
            .map(|key| serde_json::to_string(&key))
            .transpose()
            .map_err(|_| DataPipelineError::Json(JsonError::Invalid))?;
        let separator = if index == 0 { "" } else { "," };
        let key_bytes = key.as_ref().map_or(0, |key| key.len() + 1);
        if result.len() + separator.len() + key_bytes + encoded.len() + 1
            > MAX_PIPELINE_OUTPUT_BYTES
        {
            return Err(if object {
                DataPipelineError::JsonObjectTooLarge { field: item }
            } else {
                DataPipelineError::JsonArrayTooLarge { item }
            });
        }
        result.push_str(separator);
        if let Some(key) = key {
            result.push_str(&key);
            result.push(':');
        }
        result.push_str(&encoded);
    }
    result.push(close);
    Ok(result)
}

fn run_pipeline_json_lines(
    steps: &[PipelineStep],
    source: &str,
    now: DateTime<FixedOffset>,
    remaining_work: &mut usize,
) -> Result<String, DataPipelineError> {
    let mut result = String::new();
    for (line, input) in json_lines(source) {
        let value =
            parse_json_line(input).map_err(|cause| DataPipelineError::JsonLine { line, cause })?;
        let encoded = run_pipeline_json_value(
            value,
            steps,
            now,
            PipelineLocation::Line(line),
            remaining_work,
        )
        .map_err(|error| match error {
            DataPipelineError::Json(cause) => DataPipelineError::JsonLine { line, cause },
            _ => error,
        })?;
        let separator = if result.is_empty() { "" } else { "\n" };
        if result.len() + separator.len() + encoded.len() > MAX_PIPELINE_OUTPUT_BYTES {
            return Err(DataPipelineError::RowOutputTooLarge { line });
        }
        result.push_str(separator);
        result.push_str(&encoded);
    }
    Ok(result)
}

fn run_pipeline_json_value(
    value: &RawValue,
    steps: &[PipelineStep],
    now: DateTime<FixedOffset>,
    location: PipelineLocation,
    remaining_work: &mut usize,
) -> Result<String, DataPipelineError> {
    let raw = value.get();
    let input = if raw.starts_with('"') {
        Cow::Owned(
            serde_json::from_str::<String>(raw)
                .map_err(|_| DataPipelineError::Json(JsonError::Invalid))?,
        )
    } else {
        Cow::Borrowed(raw)
    };
    let output = run_pipeline_steps(
        steps,
        &input,
        now,
        PipelineView::Text,
        location,
        remaining_work,
    )?;
    serde_json::to_string(&output).map_err(|_| DataPipelineError::Json(JsonError::Invalid))
}

fn run_pipeline_rows(
    steps: &[PipelineStep],
    source: &str,
    now: DateTime<FixedOffset>,
    csv: bool,
    remaining_work: &mut usize,
) -> Result<String, DataPipelineError> {
    let mut result = if csv {
        String::from("输入,结果")
    } else {
        String::new()
    };
    let mut separator = "";
    for (index, input) in source.lines().enumerate() {
        if input.trim().is_empty() {
            continue;
        }
        let line = index + 1;
        let output = run_pipeline_steps(
            steps,
            input,
            now,
            PipelineView::Text,
            PipelineLocation::Line(line),
            remaining_work,
        )?;
        let error = DataPipelineError::RowOutputTooLarge { line };
        if csv {
            append_bounded_csv_row(&mut result, [input, output.as_str()].into_iter(), error)?;
        } else {
            if result.len() + separator.len() + output.len() > MAX_PIPELINE_OUTPUT_BYTES {
                return Err(error);
            }
            result.push_str(separator);
            result.push_str(&output);
            separator = "\n";
        }
    }
    Ok(result)
}

fn run_pipeline_steps(
    steps: &[PipelineStep],
    source: &str,
    now: DateTime<FixedOffset>,
    mut view: PipelineView<'_>,
    location: PipelineLocation,
    remaining_work: &mut usize,
) -> Result<String, DataPipelineError> {
    let mut current = Cow::Borrowed(source);
    let mut trace = matches!(&view, PipelineView::Trace).then(|| String::from("步骤,命令,结果"));
    for (index, step) in steps.iter().enumerate() {
        let index = index + 1;
        let error = |cause| match location {
            PipelineLocation::Whole => DataPipelineError::Step { index, cause },
            PipelineLocation::Line(line) => DataPipelineError::RowStep { line, index, cause },
            PipelineLocation::CsvRow(row) => DataPipelineError::CsvRowStep { row, index, cause },
            PipelineLocation::JsonItem(item) => {
                DataPipelineError::JsonItemStep { item, index, cause }
            }
            PipelineLocation::JsonField(field) => DataPipelineError::JsonFieldStep {
                field,
                index,
                cause,
            },
        };
        let csv_error = |cause: CsvError| match cause {
            CsvError::DataRowValue { row, cause }
                if matches!(location, PipelineLocation::Whole) =>
            {
                DataPipelineError::CsvTableStep { index, row, cause }
            }
            cause => error(cause.message()),
        };
        let (raw_step, embed) = match step {
            PipelineStep::Append(command) => (command, false),
            PipelineStep::Embed(template) => (&template.command, true),
        };
        if raw_step.contains(['\r', '\n', '\0']) {
            return Err(error(STEP_SYNTAX));
        }
        let (scope, operation) = scoped_query(raw_step.trim()).ok_or_else(|| error(STEP_SYNTAX))?;
        let query = if embed {
            let (before, after) = operation
                .split_once(STEP_RESULT_SLOT)
                .filter(|(_, after)| !after.contains(STEP_RESULT_SLOT))
                .ok_or_else(|| error(STEP_TEMPLATE_SYNTAX))?;
            Cow::Owned(format!("{before}{current}{after}"))
        } else if operation.is_empty() {
            Cow::Borrowed(current.as_ref())
        } else {
            Cow::Owned(format!("{operation}\n{current}"))
        };
        let command_text = if embed { query.as_ref() } else { operation };
        let command = command_text.split_whitespace().next().unwrap_or_default();
        if command.is_empty()
            && !matches!(
                scope,
                Scope::Date
                    | Scope::Timestamp
                    | Scope::Duration
                    | Scope::Radix
                    | Scope::Pitch
                    | Scope::Wave
                    | Scope::Circuit
                    | Scope::Equation
                    | Scope::LinearSystem
                    | Scope::LinearFit
                    | Scope::Fraction
                    | Scope::Statistics
            )
        {
            return Err(error(STEP_SYNTAX));
        }
        if command == "依次处理" || command.eq_ignore_ascii_case("pipeline") {
            return Err(error("请把子步骤直接写在同一个列表中"));
        }
        let preview_text = match &mut view {
            PipelineView::Preview(preview) if index == steps.len() => {
                prepare_pipeline_preview(scope, &query, now, &error, &csv_error)
                    .transpose()?
                    .map(|(text, prepared)| {
                        **preview = Some(prepared);
                        text
                    })
            }
            _ => None,
        };
        let output = if let Some(text) = preview_text {
            text
        } else if scope == Scope::Csv {
            transform_csv(&query).map_err(csv_error)?
        } else {
            match scope {
                Scope::TextTools => transform_text(&query).map_err(|cause| cause.message()),
                Scope::Json => transform_json(&query).map_err(|cause| cause.message()),
                Scope::Toml => transform_toml(&query).map_err(|cause| cause.message()),
                Scope::UrlEncoding => {
                    convert_url_component(&query).map_err(|cause| cause.message())
                }
                Scope::Path => path_component(&query).map_err(|cause| cause.message()),
                Scope::Date => {
                    calculate_date(&query, now.date_naive()).map_err(|cause| cause.message())
                }
                Scope::Timestamp => {
                    convert_timestamp(&query, now.to_utc()).map_err(|cause| cause.message())
                }
                Scope::Duration => convert_duration(&query).map_err(|cause| cause.message()),
                Scope::Radix => convert_arbitrary_radix(&query).map_err(|cause| cause.message()),
                Scope::Pitch => convert_pitch(&query).map_err(|cause| cause.message()),
                Scope::Tempo => calculate_tempo(&query).map_err(|cause| cause.message()),
                Scope::Decibel => convert_decibels(&query).map_err(|cause| cause.message()),
                Scope::Solid => calculate_solid(&query).map_err(|cause| cause.message()),
                Scope::Complex => calculate_complex(&query).map_err(|cause| cause.message()),
                Scope::Wave => convert_wave(&query).map_err(|cause| cause.message()),
                Scope::Circuit => calculate_circuit(&query).map_err(|cause| cause.message()),
                Scope::Equation => solve_equation(&query).map_err(|cause| cause.message()),
                Scope::LinearSystem => solve_linear_system(&query).map_err(|cause| cause.message()),
                Scope::Matrix => calculate_matrix(&query).map_err(|cause| cause.message()),
                Scope::LinearFit => calculate_linear_fit(&query).map_err(|cause| cause.message()),
                Scope::Fraction => simplify_fraction(&query).map_err(|cause| cause.message()),
                Scope::Percentage => compare_percentages(&query).map_err(|cause| cause.message()),
                Scope::Statistics => calculate_statistics(&query).map_err(|cause| cause.message()),
                _ => return Err(error(STEP_SYNTAX)),
            }
            .map_err(error)?
        };
        if output.len() > MAX_PIPELINE_OUTPUT_BYTES {
            return Err(error("处理结果超过 1 MiB，请缩小数据范围"));
        }
        *remaining_work = remaining_work
            .checked_sub(output.len())
            .ok_or_else(|| error("中间结果合计超过 8 MiB，请缩小数据范围"))?;
        if let Some(trace) = &mut trace {
            let number = index.to_string();
            append_bounded_csv_row(
                trace,
                [number.as_str(), raw_step.as_str(), output.as_str()].into_iter(),
                error("步骤CSV超过 1 MiB，请缩小数据范围"),
            )?;
        }
        current = Cow::Owned(output);
    }
    Ok(trace.unwrap_or_else(|| current.into_owned()))
}

fn prepare_pipeline_preview(
    scope: Scope,
    query: &str,
    now: DateTime<FixedOffset>,
    error: &impl Fn(&'static str) -> DataPipelineError,
    csv_error: &impl Fn(CsvError) -> DataPipelineError,
) -> Option<Result<(String, PipelinePreview), DataPipelineError>> {
    match scope {
        Scope::Toml => toml_csv_report(query).map(|report| {
            report_preview(report.map_err(|cause| cause.message()), error, csv_error)
        }),
        Scope::Json => json_csv_report(query).map(|report| {
            report_preview(report.map_err(|cause| cause.message()), error, csv_error)
        }),
        Scope::Date if is_date_report(query) => {
            let report = calculate_date(query, now.date_naive()).map_err(|cause| cause.message());
            Some(report_preview(report, error, csv_error))
        }
        Scope::Fraction => fraction_csv_report(query).map(|report| {
            report_preview(report.map_err(|cause| cause.message()), error, csv_error)
        }),
        Scope::Timestamp if is_timestamp_report(query) => {
            let report = convert_timestamp(query, now.to_utc()).map_err(|cause| cause.message());
            Some(report_preview(report, error, csv_error))
        }
        Scope::Csv => csv_table_preview(query)
            .map(|result| {
                result
                    .map(|(text, table)| (text, PipelinePreview::Table(table)))
                    .map_err(csv_error)
            })
            .or_else(|| {
                scatter_arguments(query).map(|input| {
                    build_scatter_plot(input)
                        .map(|(text, plot)| (text, PipelinePreview::Scatter(plot)))
                        .map_err(csv_error)
                })
            })
            .or_else(|| {
                super::csv_tools::coordinate_line_chart(query).map(|result| {
                    result
                        .map(|(text, plot)| (text, PipelinePreview::Scatter(plot)))
                        .map_err(csv_error)
                })
            })
            .or_else(|| {
                csv_count_chart(query).map(|result| {
                    result
                        .map(|(text, plot)| (text, PipelinePreview::Bins(plot)))
                        .map_err(csv_error)
                })
            })
            .or_else(|| {
                column_box_plot_arguments(query).map(|input| {
                    build_column_box_plot(input)
                        .map(|(text, plot)| (text, PipelinePreview::BoxPlot(plot)))
                        .map_err(csv_error)
                })
            })
            .or_else(|| {
                super::csv_tools::column_cumulative_plot_arguments(query).map(|input| {
                    super::csv_tools::build_column_cumulative_plot(input)
                        .map(|(text, plot)| (text, PipelinePreview::Scatter(plot)))
                        .map_err(csv_error)
                })
            })
            .or_else(|| {
                correlation_plot_arguments(query).map(|(input, rank)| {
                    build_correlation_plot(input, rank)
                        .map(|(text, plot)| (text, PipelinePreview::Correlation(plot)))
                        .map_err(csv_error)
                })
            }),
        Scope::Equation => equation_chart(query).map(|result| {
            result
                .map(|(text, plot)| (text, PipelinePreview::Scatter(plot)))
                .map_err(|cause| error(cause.message()))
        }),
        Scope::LinearFit => plot_request(query).map(|request| {
            calculate_plot(request)
                .map(|(text, plot)| (text, PipelinePreview::Scatter(plot)))
                .map_err(|cause| error(cause.message()))
        }),
        Scope::Statistics => box_plot_arguments(query)
            .map(|input| {
                calculate_box_plot(input)
                    .map(|(text, plot)| (text, PipelinePreview::BoxPlot(plot)))
                    .map_err(|cause| error(cause.message()))
            })
            .or_else(|| {
                bin_plot_arguments(query).map(|input| {
                    calculate_bin_plot(input)
                        .map(|(text, plot)| (text, PipelinePreview::Bins(plot)))
                        .map_err(|cause| error(cause.message()))
                })
            })
            .or_else(|| {
                cumulative_plot_arguments(query).map(|input| {
                    plot_cumulative_distribution(input)
                        .map(|(text, plot)| (text, PipelinePreview::Scatter(plot)))
                        .map_err(|cause| error(cause.message()))
                })
            })
            .or_else(|| {
                statistics_csv_report(query).map(|report| {
                    report_preview(report.map_err(|cause| cause.message()), error, csv_error)
                })
            }),
        _ => None,
    }
}

fn report_preview(
    report: Result<String, &'static str>,
    error: &impl Fn(&'static str) -> DataPipelineError,
    csv_error: &impl Fn(CsvError) -> DataPipelineError,
) -> Result<(String, PipelinePreview), DataPipelineError> {
    let text = report.map_err(error)?;
    let table = parse_table_preview(&text).map_err(csv_error)?;
    Ok((text, PipelinePreview::Table(table)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_pipeline(input: &str) -> Result<String, DataPipelineError> {
        let now = DateTime::parse_from_rfc3339("2024-02-18T23:30:00-08:00").unwrap();
        transform_data_pipeline(input, now).map(|(text, _)| text)
    }

    #[test]
    fn passes_exact_matrices_and_interpolation_rows_between_tools() {
        assert_eq!(
            run_pipeline(r#"["矩阵: 转置","矩阵: 行列式"] 1,2; 3,4"#).as_deref(),
            Ok("-2"),
        );
        assert_eq!(
            run_pipeline(r#"["拟合: 插值 1/2,2 @","CSV: 选列名 [\"插值\"]"] 0,1; 1,2; 2,5"#)
                .as_deref(),
            Ok("插值\r\n5/4\r\n5"),
        );
        assert_eq!(
            run_pipeline(r#"["拟合:"] 0,1; 1,3; 2,5"#).as_deref(),
            Ok("斜率 2 · 截距 1\nR² 1"),
        );
    }

    #[test]
    fn passes_exact_equation_coefficients_and_system_inputs_between_steps() {
        assert_eq!(
            run_pipeline(r#"["方程: 由根","方程:"] 1/2,3/2,-2"#).as_deref(),
            Ok("x₁ = 1/2 · x₂ = 3/2"),
        );
        assert_eq!(
            run_pipeline(r#"["联立:"] 2,1,5; 1,-1,1"#).as_deref(),
            Ok("x = 2 · y = 1"),
        );
    }

    #[test]
    fn combines_timestamp_output_with_other_steps_and_reuses_the_request_clock() {
        let query = r#"["时间戳: 日区间","CSV: 选列名 [\"开始UTC\",\"结束UTC\"]"] 2026-03-08 America/New_York"#;
        assert_eq!(
            run_pipeline(query).as_deref(),
            Ok("开始UTC,结束UTC\r\n2026-03-08T05:00:00Z,2026-03-09T04:00:00Z"),
        );
        assert_eq!(
            run_pipeline(r#"["时间戳:","时间戳:"] 现在"#).as_deref(),
            Ok("2024-02-19T07:30:00Z"),
        );
        let now = DateTime::parse_from_rfc3339("2024-02-18T23:30:00-08:00").unwrap();
        let report = r#"["时间戳: 时区CSV"] 现在 to UTC, +08:00"#;
        let (text, preview) = transform_data_pipeline(report, now).unwrap();
        assert_eq!(
            text,
            "时区,时间\r\nUTC,2024-02-19T07:30:00Z\r\n+08:00,2024-02-19T15:30:00+08:00"
        );
        assert!(matches!(preview, Some(PipelinePreview::Table(table)) if table.row_pages() == 1));
    }

    #[test]
    fn sends_text_through_csv_and_json_steps_without_losing_numeric_spelling() {
        let input = concat!(
            r#"["文本: 去空行","CSV: 数值对象 [2]","JSON: 取值 /1/金额"]"#,
            "\r\n编号,金额\r\n001,1.50\r\n\r\n002,2e0"
        );
        assert_eq!(run_pipeline(input).as_deref(), Ok("2e0"));
        assert_eq!(
            run_pipeline(r#"["JSON: 转CSV","CSV: 选列 2"] [{"编号":"001","金额":1.50}]"#)
                .as_deref(),
            Ok("金额\r\n1.50")
        );
    }

    #[test]
    fn uses_wavelength_conversion_as_a_pipeline_step_with_default_arguments() {
        assert_eq!(
            run_pipeline(r#"["URL: 解码","波长:"] 500%20nm"#).as_deref(),
            Ok("599.584916 THz")
        );
        assert_eq!(
            run_pipeline(r#"["分数: 计算","波长: 10 Hz @"] 36/10"#),
            Err(DataPipelineError::Step {
                index: 2,
                cause: "在 @ 后填写大于零的传播速度，例如 343 m/s",
            })
        );
    }

    #[test]
    fn combines_url_path_and_date_tools_with_following_steps() {
        for (input, expected) in [
            (
                r#"["URL: 参数JSON","JSON: 取值 /0/1"] https://example.invalid/?q=%E8%8C%B6+1"#,
                "\"茶 1\"",
            ),
            (
                r#"["路径: 文件名","文本: 大写"] C:/Docs/report.txt"#,
                "REPORT.TXT",
            ),
            (
                r#"["日期: 月末序列","文本: 按行拼接 \", \""] 本月,2"#,
                "2024-02-29, 2024-03-31",
            ),
        ] {
            assert_eq!(run_pipeline(input).as_deref(), Ok(expected), "{input}");
        }
    }

    #[test]
    fn feeds_rational_cartesian_complex_results_into_later_steps() {
        let commands = r#"[{"模板":"复数: 乘方 {结果} @ 2"},"复数: 共轭","复数: 模"] 3+4i"#;
        assert_eq!(run_pipeline(commands).as_deref(), Ok("25"));
        let trace = run_pipeline(&format!("步骤CSV {commands}")).unwrap();
        assert!(trace.contains(",-7 + 24i"));
        assert!(trace.contains(",-7 - 24i"));
        assert!(trace.ends_with(",25"));
        assert!(matches!(
            run_pipeline(r#"[{"模板":"复数: 除法 {结果} @ 0"}] 3+4i"#),
            Err(DataPipelineError::Step {
                index: 1,
                cause: "复数的除数不能为零",
            })
        ));
    }

    #[test]
    fn passes_values_between_calculation_tools() {
        for (input, expected) in [
            (r#"["分数: 计算","节拍: 速度 4秒 @"] 2*4"#, "120 BPM"),
            (r#"["分贝: 转功率比","分数: 计算"] 20 dB"#, "100"),
            (
                r#"["电路: 并联","电路: 分压 12 V @ 1 kΩ,"] 2 kΩ,2 kΩ"#,
                "6 V",
            ),
            (r#"["电路:"] 5 V,20 Ω"#, "250 mA · 1.25 W"),
            (
                r#"["分数: 计算","立体: 球"] 1/4+1/4"#,
                "表面积 π · 体积 π/6",
            ),
            (
                r#"["分数: 计算",{"模板":"百分比: 重复变化 {结果} @ +50%,2"},"百分比: 精确变化 2 到"] 3/2+1/2"#,
                "125%",
            ),
            (r#"["百分比: 精确占比 1 占"] 3"#, "33.(3)%"),
        ] {
            assert_eq!(run_pipeline(input).as_deref(), Ok(expected), "{input}");
        }
    }

    #[test]
    fn embeds_the_previous_value_at_one_explicit_position() {
        for (input, expected) in [
            (
                r#"["文本: 去空行",{"模板":"电路: 分压 {结果} @ 1 kΩ,2 kΩ"}] 12 V"#,
                "8 V",
            ),
            (
                "逐行 [{\"模板\":\"分数: 计算 2*({结果})+1\"}] 3/4\n2",
                "5/2\n5",
            ),
            (r#"["文本: 逐行加前缀 \"{结果}\""] 茶"#, "{结果}茶"),
        ] {
            assert_eq!(run_pipeline(input).as_deref(), Ok(expected), "{input}");
        }
        for step in ["分数: 计算 1", "分数: 计算 {结果} + {结果}"] {
            let query = format!(r#"[{{"模板":"{step}"}}] 1"#);
            assert_eq!(
                run_pipeline(&query),
                Err(DataPipelineError::Step {
                    index: 1,
                    cause: STEP_TEMPLATE_SYNTAX,
                }),
            );
        }
        assert!(matches!(
            run_pipeline(r#"[{"模板":"百分比: 重复原值 {结果} @ -100%,2"}] 0"#),
            Err(DataPipelineError::Step {
                index: 1,
                cause: "存在 -100% 的变动，无法唯一反推原值",
            })
        ));
        assert!(matches!(
            run_pipeline(r#"[{"模板":"文本: {结果}"}] 依次处理 ["大写"] tea"#),
            Err(DataPipelineError::Step { index: 1, .. })
        ));
    }

    #[test]
    fn preserves_the_csv_table_row_in_pipeline_step_errors() {
        let data = "地区,月份,编号\n北,一月,001\n北,一月,002";
        let steps = r#"["文本: 去空行","CSV: 透视原值 [1,2,3]"]"#;
        for mode in ["", "步骤CSV "] {
            assert_eq!(
                run_pipeline(&format!("{mode}{steps} {data}")),
                Err(DataPipelineError::CsvTableStep {
                    index: 2,
                    row: 2,
                    cause: "行列组合重复，请为每个组合保留一条记录",
                })
            );
        }
        assert!(matches!(
            run_pipeline("[\"CSV: 列累计分布图 2\"] 编号,x\nA,1\nB,待定"),
            Err(DataPipelineError::CsvTableStep {
                index: 1,
                row: 2,
                ..
            })
        ));
    }

    #[test]
    fn combines_duration_radix_and_pitch_operations_with_data_steps() {
        for (input, expected) in [
            (
                r#"["时长: 分摊 01:30:00 @","文本: 按行拼接 \", \""] 1:2"#,
                "00:30:00, 01:00:00",
            ),
            (r#"["进制: 罗马数字","分数: 计算"] CDXCIX"#, "499"),
            (r#"["进制: 小数"] 0.1 从 10 到 2"#, "0.0(0011)"),
            (
                r#"["进制: 罗马数字","音高: MIDI"] LXIX"#,
                "A4 · MIDI 69 · 440 Hz",
            ),
        ] {
            assert_eq!(run_pipeline(input).as_deref(), Ok(expected), "{input}");
        }
        assert!(matches!(
            run_pipeline(r#"["文本: 大写","进制: 罗马数字"] ic"#),
            Err(DataPipelineError::Step { index: 2, .. }),
        ));
    }

    #[test]
    fn records_each_step_preserving_multiline_outputs_and_quoted_commands() {
        let query = concat!(
            r#"["文本: 大写","文本: 按行拼接 \", \""]"#,
            "\n茶 a\n咖啡 b"
        );
        let expected = concat!(
            "步骤,命令,结果\r\n",
            "1,文本: 大写,\"茶 A\n咖啡 B\"\r\n",
            "2,\"文本: 按行拼接 \"\", \"\"\",\"茶 A, 咖啡 B\""
        );
        for command in ["步骤CSV", "STEPS-CSV"] {
            assert_eq!(
                run_pipeline(&format!("{command} {query}")).as_deref(),
                Ok(expected)
            );
        }
        assert_eq!(run_pipeline(query).as_deref(), Ok("茶 A, 咖啡 B"));
        assert_eq!(run_pipeline("步骤CSV"), Err(DataPipelineError::Arguments));
        assert!(matches!(
            run_pipeline(r#"步骤CSV ["文本: 大写","JSON: 压缩"] tea"#),
            Err(DataPipelineError::Step { index: 2, .. })
        ));

        let large = format!(
            r#"步骤CSV ["文本: 大写","文本: 小写"] {}"#,
            "a".repeat(MAX_PIPELINE_OUTPUT_BYTES / 2)
        );
        assert_eq!(
            run_pipeline(&large),
            Err(DataPipelineError::Step {
                index: 2,
                cause: "步骤CSV超过 1 MiB，请缩小数据范围",
            })
        );
    }

    #[test]
    fn uses_toml_conversion_before_selecting_a_json_value() {
        assert_eq!(
            run_pipeline(r#"["TOML: 转JSON","JSON: 取值 /port"] port = 8000"#).as_deref(),
            Ok("8000"),
        );
    }

    #[test]
    fn runs_exact_calculations_and_statistics_on_data_from_a_previous_step() {
        let table_query = concat!(
            r#"["CSV: 选列 2","文本: 间隔取行 2,1","统计: 分位数 50%"]"#,
            "\n名称,金额\n茶,1/2\n咖啡,5/2\n豆浆,3/2"
        );
        assert_eq!(run_pipeline(table_query).as_deref(), Ok("3/2"));
        assert_eq!(
            run_pipeline(concat!(
                r#"["文本: 按行拼接 \"+\"","分数: 计算"]"#,
                "\n1/3\n1/6"
            ))
            .as_deref(),
            Ok("1/2"),
        );
        assert!(matches!(
            run_pipeline(r#"["文本: 大写","分数: 计算"] tea"#),
            Err(DataPipelineError::Step { index: 2, .. })
        ));
    }

    #[test]
    fn uses_the_default_operation_for_a_bare_numeric_scope() {
        for (input, expected) in [
            (r#"["节拍: 时长","时长:"] 120 BPM @ 3/2"#, "00:00:00.75"),
            (r#"["JSON: 取值 /值","分数:"] {"值":0.250}"#, "1/4"),
            (r#"["音高："] A4"#, "A4 · MIDI 69 · 440 Hz"),
            (r#"["进制:"] ff 从 16 到 10"#, "255"),
        ] {
            assert_eq!(run_pipeline(input).as_deref(), Ok(expected), "{input}");
        }
    }

    #[test]
    fn processes_json_array_members_in_order_using_the_original_numeric_text() {
        for (input, expected) in [
            (
                r#"JSON数组 ["JSON: 取值 /金额","分数:"] [{"金额":0.50},{"金额":1.25}]"#,
                r#"["1/2","5/4"]"#,
            ),
            (
                r#"json-array ["文本: 大写"] ["a\nb","001",9007199254740993,1.20,null]"#,
                r#"["A\nB","001","9007199254740993","1.20","NULL"]"#,
            ),
            (r#"JSON数组 ["文本: 大写"] []"#, "[]"),
        ] {
            assert_eq!(run_pipeline(input).as_deref(), Ok(expected), "{input}");
        }
        assert_eq!(
            run_pipeline(r#"JSON数组 ["分数:"] {"金额":1}"#),
            Err(DataPipelineError::Json(JsonError::ExpectedArray))
        );
        assert!(matches!(
            run_pipeline(r#"JSON数组 ["JSON: 取值 /金额","分数:"] [{"金额":1},{"金额":null}]"#),
            Err(DataPipelineError::JsonItemStep {
                item: 2,
                index: 2,
                ..
            })
        ));
        let value = "a".repeat(MAX_PIPELINE_OUTPUT_BYTES / 2);
        let input = format!(r#"JSON数组 ["文本: 大写"] ["{value}","{value}"]"#);
        assert_eq!(
            run_pipeline(&input),
            Err(DataPipelineError::JsonArrayTooLarge { item: 2 })
        );
    }

    #[test]
    fn maps_json_lines_to_single_line_strings_and_reports_source_lines() {
        let query = "JSON行 [\"文本: 大写\"]\r\n\"a\\nb\"\r\n\t\r\n1.50\r\n9007199254740993";
        assert_eq!(
            run_pipeline(query).as_deref(),
            Ok("\"A\\nB\"\n\"1.50\"\n\"9007199254740993\"")
        );
        assert_eq!(
            run_pipeline("json-lines [\"文本: 大写\"]\r\n1\r\n\t\r\n{"),
            Err(DataPipelineError::JsonLine {
                line: 3,
                cause: JsonError::InvalidLine,
            })
        );
        assert!(matches!(
            run_pipeline("JSON行 [\"分数:\"]\n0.5\n\nnull"),
            Err(DataPipelineError::RowStep {
                line: 3,
                index: 1,
                ..
            })
        ));
        let invalid_string = "JSON行 [\"文本: 大写\"]\n\"a\"\n\"\\uD800\"";
        assert!(matches!(
            run_pipeline(invalid_string),
            Err(DataPipelineError::JsonLine { line: 2, .. })
        ));
        let value = "a".repeat(MAX_PIPELINE_OUTPUT_BYTES / 2);
        let input = format!("JSON行 [\"文本: 大写\"]\n\"{value}\"\n\"{value}\"");
        assert_eq!(
            run_pipeline(&input),
            Err(DataPipelineError::RowOutputTooLarge { line: 2 })
        );
    }

    #[test]
    fn maps_object_values_in_field_order_and_reports_field_failures() {
        for (input, expected) in [
            (
                r#"JSON对象 ["分数:"] {"乙":1.25,"甲":0.50}"#,
                r#"{"乙":"5/4","甲":"1/2"}"#,
            ),
            (
                r#"JSON-OBJECT ["文本: 大写"] {"a\"b":"x\ny","big":9007199254740993}"#,
                r#"{"a\"b":"X\nY","big":"9007199254740993"}"#,
            ),
            (r#"JSON对象 ["分数:"] {}"#, "{}"),
        ] {
            assert_eq!(run_pipeline(input).as_deref(), Ok(expected), "{input}");
        }
        assert_eq!(
            run_pipeline(r#"JSON对象 ["分数:"] [1]"#),
            Err(DataPipelineError::Json(JsonError::ExpectedObject))
        );
        assert_eq!(
            run_pipeline(r#"JSON对象 ["分数:"] {"x":1,"\u0078":2}"#),
            Err(DataPipelineError::Json(JsonError::DuplicateKey))
        );
        assert!(matches!(
            run_pipeline(r#"JSON对象 ["分数:"] {"甲":1,"乙":null}"#),
            Err(DataPipelineError::JsonFieldStep {
                field: 2,
                index: 1,
                ..
            })
        ));
        let value = "x".repeat(MAX_PIPELINE_OUTPUT_BYTES / 2);
        let input = format!(r#"JSON对象 ["文本: 大写"] {{"a":"{value}","b":"{value}"}}"#);
        assert_eq!(
            run_pipeline(&input),
            Err(DataPipelineError::JsonObjectTooLarge { field: 2 })
        );
    }

    #[test]
    fn appends_csv_pipeline_results_without_changing_source_fields_or_blank_cells() {
        let input = concat!(
            r#"CSV列 ["精确金额","明细",["JSON: 取值 /金额","分数:"]]"#,
            "\r\n编号,明细\r\n001,\"{\"\"金额\"\":0.50}\"\r\n002,\"{\"\"金额\"\":1.25}\"\r\n003,"
        );
        assert_eq!(
            run_pipeline(input).as_deref(),
            Ok(concat!(
                "编号,明细,精确金额",
                "\r\n001,\"{\"\"金额\"\":0.50}\",1/2",
                "\r\n002,\"{\"\"金额\"\":1.25}\",5/4",
                "\r\n003,\"\",\"\""
            ))
        );

        let multiline = "csv-column [\"大写\",2,[\"文本: 大写\"]]\n编号,文本\n001,\"a,b\ncd\"";
        assert_eq!(
            run_pipeline(multiline).as_deref(),
            Ok("编号,文本,大写\r\n001,\"a,b\ncd\",\"A,B\nCD\"")
        );
        assert!(matches!(
            run_pipeline(
                "csv-column [\"精确值\",2,[\"JSON: 取值 /金额\",\"分数:\"]]\n编号,文本\n001,\"{\n\"\"金额\"\":1\n}\"\n002,\n003,\"{\"\"金额\"\":null}\""
            ),
            Err(DataPipelineError::CsvRowStep { row: 3, index: 2, .. })
        ));
        assert_eq!(
            run_pipeline("CSV列 [\"编号\",1,[\"文本: 大写\"]]\n编号\n001"),
            Err(DataPipelineError::Csv(CsvError::HeaderNameConflict))
        );
        assert_eq!(
            run_pipeline("CSV列 [\"大写\",3,[\"文本: 大写\"]]\n编号,文本\n001,abc"),
            Err(DataPipelineError::Csv(CsvError::ColumnOutOfRange))
        );
    }

    #[test]
    fn previews_csv_pipeline_reports_without_changing_quotes_or_multiline_cells() {
        let now = DateTime::parse_from_rfc3339("2024-02-18T23:30:00-08:00").unwrap();
        for (query, expected) in [
            (
                "步骤CSV [\"文本: 大写\"] 茶 a\n咖啡 b",
                "步骤,命令,结果\r\n1,文本: 大写,\"茶 A\n咖啡 B\"",
            ),
            (
                "逐行CSV [\"文本: 大写\"] 茶,a\n咖啡,b",
                "输入,结果\r\n\"茶,a\",\"茶,A\"\r\n\"咖啡,b\",\"咖啡,B\"",
            ),
            (
                "CSV列 [\"结果\",\"原文\",[\"文本: 大写\"]] 原文\n\"茶,a\"",
                "原文,结果\r\n\"茶,a\",\"茶,A\"",
            ),
        ] {
            let (text, preview) = transform_data_pipeline(query, now).unwrap();
            assert_eq!(text, expected, "{query}");
            assert!(
                matches!(preview, Some(PipelinePreview::Table(table)) if table.row_pages() == 1),
                "{query}"
            );
        }
    }

    #[test]
    fn processes_each_source_line_through_the_same_steps() {
        assert_eq!(
            run_pipeline("逐行 [\"分数: 计算\",\"立体: 球\"]\r\n1/4+1/4\r\n\r\n1").as_deref(),
            Ok("表面积 π · 体积 π/6\n表面积 4π · 体积 4π/3")
        );
        assert_eq!(
            run_pipeline(concat!(
                r#"ROWS-CSV ["JSON: 取值 /金额","分数:"]"#,
                "\n{\"金额\":0.50}\n{\"金额\":1.25}"
            ))
            .as_deref(),
            Ok("输入,结果\r\n\"{\"\"金额\"\":0.50}\",1/2\r\n\"{\"\"金额\"\":1.25}\",5/4")
        );
        assert_eq!(
            run_pipeline("逐行 [\"文本: 公共片段\"]\n\"甲\" \"乙\"\n\"甲茶\" \"乙茶\"").as_deref(),
            Ok("\n茶")
        );
        assert!(matches!(
            run_pipeline("逐行 [\"文本: 大写\",\"分数:\"]\n1/2\n\n茶"),
            Err(DataPipelineError::RowStep {
                line: 3,
                index: 2,
                ..
            })
        ));
        let data = "a".repeat(MAX_PIPELINE_OUTPUT_BYTES / 4);
        let repeated = format!("逐行 [\"文本: 大写\"]\n{data}\n{data}\n{data}\n{data}");
        assert_eq!(
            run_pipeline(&repeated),
            Err(DataPipelineError::RowOutputTooLarge { line: 4 })
        );
    }

    #[test]
    fn identifies_the_failing_step_and_requires_a_single_command_per_step() {
        for input in [
            r#"["JSON: 压缩","CSV: 选列 3"] {"金额":1.50}"#,
            r#"["JSON: 压缩","应用: 终端"] {}"#,
            r#"["JSON: 压缩","JSON: PIPELINE []"] {}"#,
            r#"["JSON: 压缩","文本: 去空行\n文本: 大写"] {}"#,
            r#"["JSON: 压缩","CSV: "] {}"#,
            r#"["文本: 小写","URL: 解码"] %zz"#,
            r#"["文本: 小写","日期: 月末序列"] 2024-13,2"#,
        ] {
            assert!(
                matches!(
                    run_pipeline(input),
                    Err(DataPipelineError::Step { index: 2, .. })
                ),
                "{input}"
            );
        }
        for input in [r#"[] {}"#, r#"["JSON: 压缩"]"#, r#"["JSON: 压缩"] "#] {
            assert_eq!(run_pipeline(input), Err(DataPipelineError::Arguments));
        }
    }
}
