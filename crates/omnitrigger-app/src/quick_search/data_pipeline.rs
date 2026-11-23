use std::borrow::Cow;

use super::{
    csv_tools::transform_csv,
    json_tools::transform_json,
    operators::json_argument_and_tail,
    scope::{scoped_query, Scope},
    text_tools::transform_text,
};

const MAX_PIPELINE_STEPS: usize = 8;
const MAX_PIPELINE_OUTPUT_BYTES: usize = 1024 * 1024;
const STEP_SYNTAX: &str = "每一步填写带前缀的文本、CSV 或 JSON 命令";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DataPipelineError {
    Arguments,
    Step { index: usize, cause: &'static str },
}

pub(super) fn transform_data_pipeline(input: &str) -> Result<String, DataPipelineError> {
    let (steps, source) =
        json_argument_and_tail::<Vec<String>>(input).ok_or(DataPipelineError::Arguments)?;
    if !(1..=MAX_PIPELINE_STEPS).contains(&steps.len()) || source.trim().is_empty() {
        return Err(DataPipelineError::Arguments);
    }
    let mut current = Cow::Borrowed(source);
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
            _ => return Err(error(STEP_SYNTAX)),
        }
        .map_err(error)?;
        if output.len() > MAX_PIPELINE_OUTPUT_BYTES {
            return Err(error("处理结果超过 1 MiB，请缩小数据范围"));
        }
        current = Cow::Owned(output);
    }
    Ok(current.into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sends_text_through_csv_and_json_steps_without_losing_numeric_spelling() {
        let input = concat!(
            r#"["文本: 去空行","CSV: 数值对象 [2]","JSON: 取值 /1/金额"]"#,
            "\r\n编号,金额\r\n001,1.50\r\n\r\n002,2e0"
        );
        assert_eq!(transform_data_pipeline(input).as_deref(), Ok("2e0"));
        assert_eq!(
            transform_data_pipeline(
                r#"["JSON: 转CSV","CSV: 选列 2"] [{"编号":"001","金额":1.50}]"#
            )
            .as_deref(),
            Ok("金额\r\n1.50")
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
        ] {
            assert!(
                matches!(
                    transform_data_pipeline(input),
                    Err(DataPipelineError::Step { index: 2, .. })
                ),
                "{input}"
            );
        }
        for input in [r#"[] {}"#, r#"["JSON: 压缩"]"#, r#"["JSON: 压缩"] "#] {
            assert_eq!(
                transform_data_pipeline(input),
                Err(DataPipelineError::Arguments)
            );
        }
    }
}
