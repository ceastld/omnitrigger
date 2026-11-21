use chrono::{DateTime, Datelike, SecondsFormat, Utc};

use super::calculator::normalize;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum TimestampError {
    Incomplete,
    Invalid,
    UnsupportedPrecision,
    OutOfRange,
}

impl TimestampError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入时间戳或带时区的时间，例如 时间戳: 1700000000000ms",
            Self::Invalid => "请输入 Unix 秒、毫秒或带时区的 RFC 3339 时间",
            Self::UnsupportedPrecision => "请输入可以用整秒或毫秒表示的时间",
            Self::OutOfRange => "结果超出四位年份的时间范围",
        }
    }
}

pub(super) fn convert_timestamp(query: &str) -> Result<String, TimestampError> {
    let input = normalize(query);
    let input = input.trim();
    if matches!(input, "" | "+" | "-") {
        return Err(TimestampError::Incomplete);
    }

    let lower = input.to_ascii_lowercase();
    let milliseconds = lower
        .strip_suffix("ms")
        .or_else(|| lower.strip_suffix("毫秒"));
    let seconds = lower.strip_suffix('s').or_else(|| lower.strip_suffix('秒'));
    if let Some(number) = milliseconds.or(seconds) {
        return to_utc(number.trim_end(), milliseconds.is_some());
    }
    if integer(input) {
        return to_utc(input, false);
    }

    let date = DateTime::parse_from_rfc3339(input).map_err(|_| TimestampError::Invalid)?;
    ensure_four_digit_year(date.with_timezone(&Utc).year())?;
    let fraction = date.timestamp_subsec_nanos();
    let extra_precision = input.split_once('.').is_some_and(|(_, digits)| {
        digits
            .bytes()
            .take_while(u8::is_ascii_digit)
            .skip(3)
            .any(|digit| digit != b'0')
    });
    if fraction >= 1_000_000_000 || extra_precision {
        return Err(TimestampError::UnsupportedPrecision);
    }
    if fraction == 0 {
        Ok(date.timestamp().to_string())
    } else {
        Ok(format!("{} ms", date.timestamp_millis()))
    }
}

fn integer(input: &str) -> bool {
    let digits = input.strip_prefix(['-', '+']).unwrap_or(input);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

fn to_utc(input: &str, milliseconds: bool) -> Result<String, TimestampError> {
    if !integer(input) {
        return Err(TimestampError::Invalid);
    }
    let value = input.parse().map_err(|_| TimestampError::OutOfRange)?;
    let (time, format) = if milliseconds {
        (
            DateTime::<Utc>::from_timestamp_millis(value),
            SecondsFormat::Millis,
        )
    } else {
        (
            DateTime::<Utc>::from_timestamp(value, 0),
            SecondsFormat::Secs,
        )
    };
    let time = time.ok_or(TimestampError::OutOfRange)?;
    ensure_four_digit_year(time.year())?;
    Ok(time.to_rfc3339_opts(format, true))
}

fn ensure_four_digit_year(year: i32) -> Result<(), TimestampError> {
    if (0..=9999).contains(&year) {
        Ok(())
    } else {
        Err(TimestampError::OutOfRange)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_unix_seconds_and_milliseconds_in_both_directions() {
        for (query, expected) in [
            ("0", "1970-01-01T00:00:00Z"),
            ("+1700000000 秒", "2023-11-14T22:13:20Z"),
            ("1700000000123MS", "2023-11-14T22:13:20.123Z"),
            ("-1 ms", "1969-12-31T23:59:59.999Z"),
            ("1970-01-01T08:00:00+08:00", "0"),
            ("1969-12-31T23:59:59.999Z", "-1 ms"),
        ] {
            assert_eq!(convert_timestamp(query), Ok(expected.into()), "{query}");
        }
        assert_eq!(convert_timestamp(""), Err(TimestampError::Incomplete));
        assert_eq!(
            convert_timestamp("2026-11-21T00:00:00"),
            Err(TimestampError::Invalid)
        );
        assert_eq!(
            convert_timestamp("1970-01-01T00:00:00.0001Z"),
            Err(TimestampError::UnsupportedPrecision)
        );
        assert_eq!(
            convert_timestamp("1970-01-01T00:00:00.0000000001Z"),
            Err(TimestampError::UnsupportedPrecision)
        );
        assert_eq!(
            convert_timestamp("999999999999999999999"),
            Err(TimestampError::OutOfRange)
        );
    }
}
