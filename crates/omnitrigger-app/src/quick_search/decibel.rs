use super::calculator::{evaluate, format_result, normalize, CalculationError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DecibelError {
    Arguments,
    NonPositiveRatio,
    OutOfRange,
    Evaluation(CalculationError),
}

impl DecibelError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Arguments => "填写功率比、幅度比、转功率比或转幅度比，例如 功率比 100",
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
        _ => return Err(DecibelError::Arguments),
    };
    let expression = expression.trim().to_ascii_lowercase();
    let expression = if to_ratio {
        expression
            .strip_suffix("db")
            .or_else(|| expression.strip_suffix("分贝"))
            .unwrap_or(&expression)
    } else {
        &expression
    };
    let value = evaluate(expression).map_err(DecibelError::Evaluation)?;
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
    let level = format_result(factor * value.log10()).map_err(DecibelError::Evaluation)?;
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
}
