use chrono::{DateTime, Datelike, FixedOffset, Local, SecondsFormat, Utc};

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

pub(super) fn convert_timestamp(query: &str, now: DateTime<Utc>) -> Result<String, TimestampError> {
    convert_timestamp_with_local(query, now, |time| time.with_timezone(&Local).fixed_offset())
}

fn convert_timestamp_with_local(
    query: &str,
    now: DateTime<Utc>,
    to_local: impl Fn(DateTime<Utc>) -> DateTime<FixedOffset>,
) -> Result<String, TimestampError> {
    let input = normalize(query);
    let input = input.trim();
    let lower = input.to_ascii_lowercase();
    let local_suffix = lower
        .strip_suffix("本地")
        .or_else(|| lower.strip_suffix(" local"))
        .or_else(|| (lower == "local").then_some(""));
    let lower = local_suffix.unwrap_or(&lower).trim_end();
    let input = &input[..lower.len()];
    if matches!(input, "" | "+" | "-") {
        return Err(TimestampError::Incomplete);
    }

    let display_time = |time: DateTime<Utc>, format| {
        ensure_four_digit_year(time.year())?;
        if local_suffix.is_some() {
            let local = to_local(time);
            ensure_four_digit_year(local.year())?;
            Ok(local.to_rfc3339_opts(format, false))
        } else {
            Ok(time.to_rfc3339_opts(format, true))
        }
    };
    let milliseconds = lower
        .strip_suffix("ms")
        .or_else(|| lower.strip_suffix("毫秒"));
    let seconds = lower.strip_suffix('s').or_else(|| lower.strip_suffix('秒'));
    let quantity = milliseconds.or(seconds);
    if matches!(quantity.unwrap_or(lower).trim_end(), "现在" | "now") {
        if local_suffix.is_some() {
            let format = if milliseconds.is_some() {
                SecondsFormat::Millis
            } else {
                SecondsFormat::Secs
            };
            return display_time(now, format);
        }
        return Ok(if milliseconds.is_some() {
            format!("{} ms", now.timestamp_millis())
        } else {
            now.timestamp().to_string()
        });
    }
    if let Some(number) = quantity {
        let (time, format) = parse_unix(number.trim_end(), milliseconds.is_some())?;
        return display_time(time, format);
    }
    if integer(input) {
        let (time, format) = parse_unix(input, false)?;
        return display_time(time, format);
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
    if local_suffix.is_some() {
        let format = if fraction == 0 {
            SecondsFormat::Secs
        } else {
            SecondsFormat::Millis
        };
        return display_time(date.with_timezone(&Utc), format);
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

fn parse_unix(
    input: &str,
    milliseconds: bool,
) -> Result<(DateTime<Utc>, SecondsFormat), TimestampError> {
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
    Ok((time, format))
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
        let now = DateTime::from_timestamp(1_700_000_000, 123_000_000).unwrap();
        let convert = |query| convert_timestamp(query, now);
        for (query, expected) in [
            ("0", "1970-01-01T00:00:00Z"),
            ("+1700000000 秒", "2023-11-14T22:13:20Z"),
            ("1700000000123MS", "2023-11-14T22:13:20.123Z"),
            ("-1 ms", "1969-12-31T23:59:59.999Z"),
            ("1970-01-01T08:00:00+08:00", "0"),
            ("1969-12-31T23:59:59.999Z", "-1 ms"),
        ] {
            assert_eq!(convert(query), Ok(expected.into()), "{query}");
        }
        assert_eq!(convert(""), Err(TimestampError::Incomplete));
        assert_eq!(convert("2026-11-21T00:00:00"), Err(TimestampError::Invalid));
        assert_eq!(
            convert("1970-01-01T00:00:00.0001Z"),
            Err(TimestampError::UnsupportedPrecision)
        );
        assert_eq!(
            convert("1970-01-01T00:00:00.0000000001Z"),
            Err(TimestampError::UnsupportedPrecision)
        );
        assert_eq!(
            convert("999999999999999999999"),
            Err(TimestampError::OutOfRange)
        );
    }

    #[test]
    fn returns_the_current_unix_time_in_requested_precision() {
        let now = DateTime::from_timestamp(1_700_000_000, 123_456_000).unwrap();
        for (query, expected) in [
            ("现在", "1700000000"),
            ("NOW", "1700000000"),
            ("现在秒", "1700000000"),
            ("现在毫秒", "1700000000123 ms"),
            ("now ms", "1700000000123 ms"),
        ] {
            assert_eq!(
                convert_timestamp(query, now),
                Ok(expected.into()),
                "{query}"
            );
        }
    }

    #[test]
    fn displays_local_time_only_with_an_explicit_suffix() {
        let now = DateTime::from_timestamp(1_700_000_000, 123_456_000).unwrap();
        let utc_plus_eight = FixedOffset::east_opt(8 * 3_600).unwrap();
        let convert = |query| {
            convert_timestamp_with_local(query, now, |time| time.with_timezone(&utc_plus_eight))
        };
        for (query, expected) in [
            ("+0 本地", "1970-01-01T08:00:00+08:00"),
            ("-1ms LOCAL", "1970-01-01T07:59:59.999+08:00"),
            ("1970-01-01T00:00:00Z 本地", "1970-01-01T08:00:00+08:00"),
            ("now local", "2023-11-15T06:13:20+08:00"),
            ("现在毫秒本地", "2023-11-15T06:13:20.123+08:00"),
            ("0", "1970-01-01T00:00:00Z"),
            ("now", "1700000000"),
        ] {
            assert_eq!(convert(query), Ok(expected.into()), "{query}");
        }
        assert_eq!(convert("本地"), Err(TimestampError::Incomplete));
        assert_eq!(
            convert("2026-11-21T00:00:00 local"),
            Err(TimestampError::Invalid)
        );
    }
}
