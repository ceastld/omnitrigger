use std::borrow::Cow;

use chrono::NaiveDate;

use super::{
    calendar::calculate_date,
    csv_tools::{append_bounded_csv_row, transform_csv},
    decibel::convert_decibels,
    duration::convert_duration,
    fraction::simplify_fraction,
    json_tools::transform_json,
    operators::json_argument_and_tail,
    path_tools::path_component,
    pitch::convert_pitch,
    radix::convert_arbitrary_radix,
    scope::{scoped_query, Scope},
    solid::calculate_solid,
    statistics::calculate_statistics,
    tempo::calculate_tempo,
    text_tools::transform_text,
    toml_tools::transform_toml,
    url_encoding::convert_url_component,
};

const MAX_PIPELINE_STEPS: usize = 8;
const MAX_PIPELINE_OUTPUT_BYTES: usize = 1024 * 1024;
const STEP_SYNTAX: &str =
    "每一步填写带前缀的文本、CSV、JSON、TOML、URL、路径、日期、时长、进制、音高、节拍、分贝、立体、分数或统计命令";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DataPipelineError {
    Arguments,
    Step {
        index: usize,
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
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DataPipelineMode {
    LastResult,
    StepsCsv,
    Rows,
    RowsCsv,
}

pub(super) fn split_pipeline_input(input: &str) -> (DataPipelineMode, &str) {
    let input = input.trim_start();
    let (operation, arguments) = input.split_once(char::is_whitespace).unwrap_or((input, ""));
    match operation.to_ascii_lowercase().as_str() {
        "步骤csv" | "steps-csv" => (DataPipelineMode::StepsCsv, arguments),
        "逐行" | "rows" => (DataPipelineMode::Rows, arguments),
        "逐行csv" | "rows-csv" => (DataPipelineMode::RowsCsv, arguments),
        _ => (DataPipelineMode::LastResult, input),
    }
}

pub(super) fn transform_data_pipeline(
    input: &str,
    today: NaiveDate,
) -> Result<String, DataPipelineError> {
    let (mode, input) = split_pipeline_input(input);
    let (steps, source) =
        json_argument_and_tail::<Vec<String>>(input).ok_or(DataPipelineError::Arguments)?;
    if !(1..=MAX_PIPELINE_STEPS).contains(&steps.len()) || source.trim().is_empty() {
        return Err(DataPipelineError::Arguments);
    }
    let mut remaining_work = MAX_PIPELINE_OUTPUT_BYTES * MAX_PIPELINE_STEPS;
    if matches!(mode, DataPipelineMode::Rows | DataPipelineMode::RowsCsv) {
        run_pipeline_rows(
            &steps,
            source,
            today,
            mode == DataPipelineMode::RowsCsv,
            &mut remaining_work,
        )
    } else {
        run_pipeline_steps(
            &steps,
            source,
            today,
            mode == DataPipelineMode::StepsCsv,
            None,
            &mut remaining_work,
        )
    }
}

fn run_pipeline_rows(
    steps: &[String],
    source: &str,
    today: NaiveDate,
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
        let output = run_pipeline_steps(steps, input, today, false, Some(line), remaining_work)?;
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
    steps: &[String],
    source: &str,
    today: NaiveDate,
    trace_steps: bool,
    line: Option<usize>,
    remaining_work: &mut usize,
) -> Result<String, DataPipelineError> {
    let mut current = Cow::Borrowed(source);
    let mut trace = trace_steps.then(|| String::from("步骤,命令,结果"));
    for (index, step) in steps.iter().enumerate() {
        let index = index + 1;
        let error = |cause| match line {
            Some(line) => DataPipelineError::RowStep { line, index, cause },
            None => DataPipelineError::Step { index, cause },
        };
        if step.contains(['\r', '\n', '\0']) {
            return Err(error(STEP_SYNTAX));
        }
        let (scope, operation) = scoped_query(step.trim()).ok_or_else(|| error(STEP_SYNTAX))?;
        let command = operation.split_whitespace().next().unwrap_or_default();
        if command.is_empty()
            && !matches!(
                scope,
                Scope::Date
                    | Scope::Duration
                    | Scope::Radix
                    | Scope::Pitch
                    | Scope::Fraction
                    | Scope::Statistics
            )
        {
            return Err(error(STEP_SYNTAX));
        }
        if command == "依次处理" || command.eq_ignore_ascii_case("pipeline") {
            return Err(error("请把子步骤直接写在同一个列表中"));
        }
        let query = if operation.is_empty() {
            Cow::Borrowed(current.as_ref())
        } else {
            Cow::Owned(format!("{operation}\n{current}"))
        };
        let output = match scope {
            Scope::TextTools => transform_text(&query).map_err(|cause| cause.message()),
            Scope::Csv => transform_csv(&query).map_err(|cause| cause.message()),
            Scope::Json => transform_json(&query).map_err(|cause| cause.message()),
            Scope::Toml => transform_toml(&query).map_err(|cause| cause.message()),
            Scope::UrlEncoding => convert_url_component(&query).map_err(|cause| cause.message()),
            Scope::Path => path_component(&query).map_err(|cause| cause.message()),
            Scope::Date => calculate_date(&query, today).map_err(|cause| cause.message()),
            Scope::Duration => convert_duration(&query).map_err(|cause| cause.message()),
            Scope::Radix => convert_arbitrary_radix(&query).map_err(|cause| cause.message()),
            Scope::Pitch => convert_pitch(&query).map_err(|cause| cause.message()),
            Scope::Tempo => calculate_tempo(&query).map_err(|cause| cause.message()),
            Scope::Decibel => convert_decibels(&query).map_err(|cause| cause.message()),
            Scope::Solid => calculate_solid(&query).map_err(|cause| cause.message()),
            Scope::Fraction => simplify_fraction(&query).map_err(|cause| cause.message()),
            Scope::Statistics => calculate_statistics(&query).map_err(|cause| cause.message()),
            _ => return Err(error(STEP_SYNTAX)),
        }
        .map_err(error)?;
        if output.len() > MAX_PIPELINE_OUTPUT_BYTES {
            return Err(error("处理结果超过 1 MiB，请缩小数据范围"));
        }
        *remaining_work = remaining_work
            .checked_sub(output.len())
            .ok_or_else(|| error("逐行处理中间结果合计超过 8 MiB，请缩小数据范围"))?;
        if let Some(trace) = &mut trace {
            let number = index.to_string();
            append_bounded_csv_row(
                trace,
                [number.as_str(), step.as_str(), output.as_str()].into_iter(),
                error("步骤CSV超过 1 MiB，请缩小数据范围"),
            )?;
        }
        current = Cow::Owned(output);
    }
    Ok(trace.unwrap_or_else(|| current.into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_pipeline(input: &str) -> Result<String, DataPipelineError> {
        transform_data_pipeline(input, NaiveDate::from_ymd_opt(2024, 2, 18).unwrap())
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
    fn passes_values_between_math_tempo_decibel_and_solid_tools() {
        for (input, expected) in [
            (r#"["分数: 计算","节拍: 速度 4秒 @"] 2*4"#, "120 BPM"),
            (r#"["分贝: 转功率比","分数: 计算"] 20 dB"#, "100"),
            (
                r#"["分数: 计算","立体: 球"] 1/4+1/4"#,
                "表面积 π · 体积 π/6",
            ),
        ] {
            assert_eq!(run_pipeline(input).as_deref(), Ok(expected), "{input}");
        }
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
