use super::calculator::{evaluate, format_result, normalize, CalculationError};
use super::operators::split_top_level_commas;

const MAX_COMBINED_LEVELS: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DecibelError {
    Arguments,
    LevelCount,
    NonPositiveRatio,
    OutOfRange,
    Evaluation(CalculationError),
}

impl DecibelError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Arguments => "填写分贝操作和数值，例如 功率比 100。按 F1 查看用法",
            Self::LevelCount => "用逗号分隔 2 至 128 个分贝值，例如 叠加 60 dB,60 dB",
            Self::NonPositiveRatio => "功率比和幅度比须大于零",
            Self::OutOfRange => "换算结果超出可表示的范围",
            Self::Evaluation(error) => error.message(),
        }
    }
}

pub(super) fn convert_decibels(query: &str) -> Result<String, DecibelError> {
    let input = normalize(query);
    let (operation, expression) = input
        .trim()
        .split_once(char::is_whitespace)
        .ok_or(DecibelError::Arguments)?;
    let (factor, to_ratio) = match operation.to_ascii_lowercase().as_str() {
        "功率比" | "power-ratio" => (10.0, false),
        "幅度比" | "amplitude-ratio" => (20.0, false),
        "转功率比" | "to-power-ratio" => (10.0, true),
        "转幅度比" | "to-amplitude-ratio" => (20.0, true),
        "叠加" | "功率叠加" | "power-sum" => return combine_levels(expression),
        _ => return Err(DecibelError::Arguments),
    };
    let value = if to_ratio {
        parse_level(expression)?
    } else {
        evaluate(&expression.trim().to_ascii_lowercase()).map_err(DecibelError::Evaluation)?
    };
    if to_ratio {
        let ratio = 10.0_f64.powf(value / factor);
        if !ratio.is_finite() || ratio == 0.0 {
            return Err(DecibelError::OutOfRange);
        }
        return format_result(ratio).map_err(DecibelError::Evaluation);
    }
    if value <= 0.0 {
        return Err(DecibelError::NonPositiveRatio);
    }
    format_level(factor * value.log10())
}

fn parse_level(expression: &str) -> Result<f64, DecibelError> {
    let expression = expression.trim().to_ascii_lowercase();
    let value = expression
        .strip_suffix("db")
        .or_else(|| expression.strip_suffix("分贝"))
        .unwrap_or(&expression);
    evaluate(value).map_err(DecibelError::Evaluation)
}

fn combine_levels(expression: &str) -> Result<String, DecibelError> {
    let levels: Vec<_> = split_top_level_commas(expression)
        .take(MAX_COMBINED_LEVELS + 1)
        .collect();
    if !(2..=MAX_COMBINED_LEVELS).contains(&levels.len()) {
        return Err(DecibelError::LevelCount);
    }
    let levels = levels
        .into_iter()
        .map(parse_level)
        .collect::<Result<Vec<_>, _>>()?;
    let peak = levels.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    // 以最大级为基准，避免将较大的分贝值直接转换为功率时溢出。
    let relative_power: f64 = levels
        .into_iter()
        .map(|level| 10.0_f64.powf((level - peak) / 10.0))
        .sum();
    format_level(peak + 10.0 * relative_power.log10())
}

fn format_level(level: f64) -> Result<String, DecibelError> {
    let level = format_result(level).map_err(DecibelError::Evaluation)?;
    Ok(format!("{level} dB"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_log_levels_and_ratios_both_ways() {
        for (query, expected) in [
            ("功率比 100", "20 dB"),
            ("幅度比 100", "40 dB"),
            ("功率比 1/100", "-20 dB"),
            ("幅度比 1", "0 dB"),
            ("转幅度比 20 dB", "10"),
            ("转功率比 20 dB", "100"),
            ("转功率比 -20分贝", "0.01"),
            ("TO-AMPLITUDE-RATIO （４０／２） ＤＢ", "10"),
            ("转幅度比 0", "1"),
        ] {
            assert_eq!(convert_decibels(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            convert_decibels("功率比 0"),
            Err(DecibelError::NonPositiveRatio)
        );
        assert_eq!(
            convert_decibels("幅度比 -2"),
            Err(DecibelError::NonPositiveRatio)
        );
        assert_eq!(
            convert_decibels("转功率比 4000 dB"),
            Err(DecibelError::OutOfRange)
        );
        assert_eq!(
            convert_decibels("转功率比 -4000 dB"),
            Err(DecibelError::OutOfRange)
        );
        assert_eq!(convert_decibels("混合 2"), Err(DecibelError::Arguments));
    }

    #[test]
    fn sums_power_levels_stably_and_keeps_expression_commas_inside_calls() {
        for (query, expected) in [
            ("叠加 60 dB,60 dB", "63.0102999566398 dB"),
            ("功率叠加 -20分贝，-20 dB", "-16.9897000433602 dB"),
            ("POWER-SUM max(40,60) dB,(30*2) DB", "63.0102999566398 dB"),
            ("叠加 4000 dB,4000 dB", "4003.01029995664 dB"),
            ("叠加 4000 dB,-4000 dB", "4000 dB"),
        ] {
            assert_eq!(convert_decibels(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(convert_decibels("叠加 60"), Err(DecibelError::LevelCount));
        let many = std::iter::repeat_n("60", MAX_COMBINED_LEVELS + 1)
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(
            convert_decibels(&format!("叠加 {many}")),
            Err(DecibelError::LevelCount)
        );
    }
}
