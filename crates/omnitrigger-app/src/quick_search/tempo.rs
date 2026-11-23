use num_bigint::Sign;
use num_rational::BigRational;

use super::calculator::normalize;
use super::duration::{elapsed_duration, DurationError, NANOS_PER_SECOND};
use super::fraction::{calculate_fraction, ensure_exact_size, FractionError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TempoError {
    Arguments,
    NonPositiveValue,
    Duration(DurationError),
    Fraction(FractionError),
}

impl TempoError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Arguments => "填写速度、时长或拍数，例如 拍数 90秒 @ 120 BPM。按 F1 查看用法",
            Self::NonPositiveValue => "速度、拍数和时长均须大于零",
            Self::Duration(error) => error.message(),
            Self::Fraction(error) => error.message(),
        }
    }
}

enum TempoOperation {
    Duration,
    Speed,
    Beats,
}

pub(super) fn calculate_tempo(query: &str) -> Result<String, TempoError> {
    let input = normalize(query);
    let input = input.trim();
    let (operation, arguments) = input.split_once(char::is_whitespace).unwrap_or((input, ""));
    let (input, operation) = match operation.to_ascii_lowercase().as_str() {
        "速度" | "bpm" => (arguments, TempoOperation::Speed),
        "时长" | "duration" => (arguments, TempoOperation::Duration),
        "拍数" | "beats" => (arguments, TempoOperation::Beats),
        _ => (input, TempoOperation::Duration),
    };
    let (value, argument) = match input.split_once('@') {
        Some(pair) => pair,
        None if matches!(operation, TempoOperation::Beats) => return Err(TempoError::Arguments),
        None => (input, "1"),
    };
    if value.trim().is_empty() || argument.trim().is_empty() {
        return Err(TempoError::Arguments);
    }
    let (result, unit) = match operation {
        TempoOperation::Speed => {
            let beats = positive_fraction(argument)?;
            (beats / duration_minutes(value)?, "BPM")
        }
        TempoOperation::Duration => {
            let beats = positive_fraction(argument)?;
            let minute_ms = BigRational::from_integer(60_000.into());
            (minute_ms * beats / positive_tempo(value)?, "ms")
        }
        TempoOperation::Beats => (duration_minutes(value)? * positive_tempo(argument)?, "拍"),
    };
    ensure_exact_size(&result).map_err(TempoError::Fraction)?;
    Ok(format!("{result} {unit}"))
}

fn duration_minutes(query: &str) -> Result<BigRational, TempoError> {
    let nanos = elapsed_duration(query.trim())
        .map_err(TempoError::Duration)?
        .as_nanos();
    if nanos == 0 {
        return Err(TempoError::NonPositiveValue);
    }
    Ok(BigRational::new(
        nanos.into(),
        (60 * NANOS_PER_SECOND).into(),
    ))
}

fn positive_tempo(query: &str) -> Result<BigRational, TempoError> {
    let lower = query.trim().to_ascii_lowercase();
    positive_fraction(lower.strip_suffix("bpm").unwrap_or(&lower))
}

fn positive_fraction(query: &str) -> Result<BigRational, TempoError> {
    let value = calculate_fraction(query.trim()).map_err(TempoError::Fraction)?;
    if value.numer().sign() != Sign::Plus {
        return Err(TempoError::NonPositiveValue);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_beat_counts_and_periods_with_exact_ratios() {
        for (query, expected) in [
            ("120 BPM", "500 ms"),
            ("120 BPM @ 3/2", "750 ms"),
            ("时长 90 @ 1", "2000/3 ms"),
            ("ＤＵＲＡＴＩＯＮ　１２０ ＢＰＭ ＠ １／２", "250 ms"),
            ("速度 1.5秒 @ 3", "120 BPM"),
            ("BPM 01.75秒 @ 3", "720/7 BPM"),
            ("速度 500ms", "120 BPM"),
            ("速度 1ns @ 1", "60000000000 BPM"),
            ("拍数 90秒 @ 120 BPM", "180 拍"),
            ("拍数 0.75秒 @ 90 BPM", "9/8 拍"),
            ("ＢＥＡＴＳ　９００ｍｓ　＠　１００ ＢＰＭ", "3/2 拍"),
        ] {
            assert_eq!(calculate_tempo(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(calculate_tempo("0 BPM"), Err(TempoError::NonPositiveValue));
        assert_eq!(
            calculate_tempo("120 @ -1"),
            Err(TempoError::NonPositiveValue)
        );
        assert_eq!(
            calculate_tempo("速度 0ns"),
            Err(TempoError::NonPositiveValue)
        );
        assert_eq!(calculate_tempo("时长 120 @ "), Err(TempoError::Arguments));
        assert_eq!(calculate_tempo("拍数 750ms"), Err(TempoError::Arguments));
        for query in ["拍数 0ns @ 120 BPM", "拍数 1s @ 0 BPM"] {
            assert_eq!(calculate_tempo(query), Err(TempoError::NonPositiveValue));
        }
    }
}
