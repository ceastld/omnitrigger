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
    Step { index: usize, cause: &'static str },
}

pub(super) fn step_csv_arguments(input: &str) -> Option<&str> {
    let mut parts = input.trim_start().splitn(2, char::is_whitespace);
    let operation = parts.next()?;
    (operation.eq_ignore_ascii_case("步骤CSV") || operation.eq_ignore_ascii_case("steps-csv"))
        .then(|| parts.next().unwrap_or(""))
}

pub(super) fn transform_data_pipeline(
    input: &str,
    today: NaiveDate,
) -> Result<String, DataPipelineError> {
    let csv_arguments = step_csv_arguments(input);
    let (steps, source) = json_argument_and_tail::<Vec<String>>(csv_arguments.unwrap_or(input))
        .ok_or(DataPipelineError::Arguments)?;
    if !(1..=MAX_PIPELINE_STEPS).contains(&steps.len()) || source.trim().is_empty() {
        return Err(DataPipelineError::Arguments);
    }
    let mut current = Cow::Borrowed(source);
    let mut trace = csv_arguments.map(|_| String::from("步骤,命令,结果"));
    for (index, step) in steps.iter().enumerate() {
        let error = |cause| DataPipelineError::Step {
            index: index + 1,
            cause,
        };
        if step.contains(['\r', '\n', '\0']) {
            return Err(error(STEP_SYNTAX));
        }
        let (scope, operation) = scoped_query(step.trim()).ok_or_else(|| error(STEP_SYNTAX))?;
        let command = operation
            .split_whitespace()
            .next()
            .ok_or_else(|| error(STEP_SYNTAX))?;
        if command == "依次处理" || command.eq_ignore_ascii_case("pipeline") {
            return Err(error("请把子步骤直接写在同一个列表中"));
        }
        let query = format!("{operation}\n{current}");
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
        if let Some(trace) = &mut trace {
            let number = (index + 1).to_string();
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
