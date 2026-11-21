use chrono::{DateTime, Datelike, FixedOffset, Local, SecondsFormat, Utc};

use super::calculator::normalize;
use super::conversion::split_conversion;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum TimestampError {
    Incomplete,
    Invalid,
    InvalidOffset,
    UnsupportedPrecision,
    OutOfRange,
}

impl TimestampError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入时间戳或带时区的时间，例如 时间戳: 1700000000000ms",
            Self::Invalid => "请输入 Unix 秒、毫秒、微秒、纳秒或带时区的 RFC 3339 时间",
            Self::InvalidOffset => "请输入目标时区偏移，例如 +08:00 或 -05:30",
            Self::UnsupportedPrecision => "请输入可用 Unix 纳秒表示的时间",
            Self::OutOfRange => "结果超出四位年份的时间范围",
        }
    }
}

#[derive(Clone, Copy)]
enum TimestampUnit {
    Seconds = 1,
    Milliseconds = 1_000,
    Microseconds = 1_000_000,
    Nanoseconds = 1_000_000_000,
}

impl TimestampUnit {
    fn split_suffix(input: &str) -> Option<(Self, &str)> {
        use TimestampUnit::*;
        [
            (Milliseconds, &["ms", "毫秒"][..]),
            (Microseconds, &["us", "µs", "μs", "微秒"][..]),
            (Nanoseconds, &["ns", "纳秒"][..]),
            (Seconds, &["s", "秒"][..]),
        ]
        .into_iter()
        .find_map(|(unit, suffixes)| {
            suffixes
                .iter()
                .find_map(|&suffix| input.strip_suffix(suffix).map(|quantity| (unit, quantity)))
        })
    }

    fn from_subsecond(nanos: u32) -> Self {
        if nanos == 0 {
            Self::Seconds
        } else if nanos.is_multiple_of(1_000_000) {
            Self::Milliseconds
        } else if nanos.is_multiple_of(1_000) {
            Self::Microseconds
        } else {
            Self::Nanoseconds
        }
    }

    fn nanos_per_unit(self) -> i128 {
        1_000_000_000 / self as i128
    }

    fn date_format(self) -> SecondsFormat {
        match self {
            Self::Seconds => SecondsFormat::Secs,
            Self::Milliseconds => SecondsFormat::Millis,
            Self::Microseconds => SecondsFormat::Micros,
            Self::Nanoseconds => SecondsFormat::Nanos,
        }
    }

    fn format_unix(self, time: DateTime<Utc>) -> String {
        let whole = i128::from(time.timestamp()) * self as i128;
        let count = whole + i128::from(time.timestamp_subsec_nanos()) / self.nanos_per_unit();
        match self {
            Self::Seconds => count.to_string(),
            Self::Milliseconds => format!("{count} ms"),
            Self::Microseconds => format!("{count} µs"),
            Self::Nanoseconds => format!("{count} ns"),
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
    let (input, offset) = if let Some((source, target)) = split_conversion(input) {
        (source, Some(parse_offset(target)?))
    } else {
        (input, None)
    };
    let lower = input.to_ascii_lowercase();
    let local_suffix = if offset.is_none() {
        lower
            .strip_suffix("本地")
            .or_else(|| lower.strip_suffix(" local"))
            .or_else(|| (lower == "local").then_some(""))
    } else {
        None
    };
    let display_in_zone = local_suffix.is_some() || offset.is_some();
    let lower = local_suffix.unwrap_or(&lower).trim_end();
    let input = &input[..lower.len()];
    if matches!(input, "" | "+" | "-") {
        return Err(TimestampError::Incomplete);
    }

    let display_time = |time: DateTime<Utc>, format| {
        ensure_four_digit_year(time.year())?;
        let zoned = offset
            .map(|offset| time.with_timezone(&offset))
            .or_else(|| local_suffix.is_some().then(|| to_local(time)));
        if let Some(zoned) = zoned {
            ensure_four_digit_year(zoned.year())?;
            Ok(zoned.to_rfc3339_opts(format, false))
        } else {
            Ok(time.to_rfc3339_opts(format, true))
        }
    };
    let suffixed = TimestampUnit::split_suffix(lower);
    let (unit, quantity) = suffixed.unwrap_or((TimestampUnit::Seconds, lower));
    let quantity = quantity.trim_end();
    if matches!(quantity, "现在" | "now") {
        if display_in_zone {
            return display_time(now, unit.date_format());
        }
        return Ok(unit.format_unix(now));
    }
    if suffixed.is_some() || integer(input) {
        return display_time(parse_unix(quantity, unit)?, unit.date_format());
    }

    let date = DateTime::parse_from_rfc3339(input).map_err(|_| TimestampError::Invalid)?;
    ensure_four_digit_year(date.with_timezone(&Utc).year())?;
    let fraction = date.timestamp_subsec_nanos();
    let extra_precision = input.split_once('.').is_some_and(|(_, digits)| {
        digits
            .bytes()
            .take_while(u8::is_ascii_digit)
            .skip(9)
            .any(|digit| digit != b'0')
    });
    if fraction >= 1_000_000_000 || extra_precision {
        return Err(TimestampError::UnsupportedPrecision);
    }
    let unit = TimestampUnit::from_subsecond(fraction);
    let time = date.with_timezone(&Utc);
    if display_in_zone {
        display_time(time, unit.date_format())
    } else {
        Ok(unit.format_unix(time))
    }
}

fn parse_offset(input: &str) -> Result<FixedOffset, TimestampError> {
    let input = input.trim().to_ascii_lowercase();
    let input = input.strip_prefix("utc").unwrap_or(&input).trim_start();
    if input.is_empty() {
        return Err(TimestampError::Incomplete);
    }
    let [b'+' | b'-', hour_tens, hours, b':', minute_tens, minutes] = input.as_bytes() else {
        return Err(TimestampError::InvalidOffset);
    };
    if ![hour_tens, hours, minute_tens, minutes]
        .iter()
        .all(|digit| digit.is_ascii_digit())
    {
        return Err(TimestampError::InvalidOffset);
    }
    input.parse().map_err(|_| TimestampError::InvalidOffset)
}

fn integer(input: &str) -> bool {
    let digits = input.strip_prefix(['-', '+']).unwrap_or(input);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

fn parse_unix(input: &str, unit: TimestampUnit) -> Result<DateTime<Utc>, TimestampError> {
    if !integer(input) {
        return Err(TimestampError::Invalid);
    }
    let value: i128 = input.parse().map_err(|_| TimestampError::OutOfRange)?;
    let scale = unit as i128;
    let seconds = i64::try_from(value.div_euclid(scale)).map_err(|_| TimestampError::OutOfRange)?;
    let nanos = (value.rem_euclid(scale) * unit.nanos_per_unit()) as u32;
    DateTime::<Utc>::from_timestamp(seconds, nanos).ok_or(TimestampError::OutOfRange)
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
    fn displays_unix_and_rfc3339_times_at_explicit_offsets() {
        let now = DateTime::from_timestamp_millis(1_700_000_000_123).unwrap();
        for (input, expected) in [
            (
                "1700000000123ms to UTC+08:00",
                "2023-11-15T06:13:20.123+08:00",
            ),
            (
                "1970-01-01T00:00:00Z to -03:30",
                "1969-12-31T20:30:00-03:30",
            ),
            (
                "1970-01-01T01:00:00.123+01:00 转 +08:00",
                "1970-01-01T08:00:00.123+08:00",
            ),
            ("now ms to +05:30", "2023-11-15T03:43:20.123+05:30"),
            ("0 to +00:00", "1970-01-01T00:00:00+00:00"),
        ] {
            assert_eq!(
                convert_timestamp(input, now).as_deref(),
                Ok(expected),
                "{input}"
            );
        }
        for (input, error) in [
            ("1700000000 to", TimestampError::Incomplete),
            ("1700000000 to +24:00", TimestampError::InvalidOffset),
            ("1700000000 to +00:60", TimestampError::InvalidOffset),
            ("1700000000 to +0:00", TimestampError::InvalidOffset),
            ("9999-12-31T23:30:00Z to +01:00", TimestampError::OutOfRange),
        ] {
            assert_eq!(convert_timestamp(input, now), Err(error), "{input}");
        }
    }

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
            convert("1970-01-01T00:00:00.0000000001Z"),
            Err(TimestampError::UnsupportedPrecision)
        );
        assert_eq!(
            convert("999999999999999999999"),
            Err(TimestampError::OutOfRange)
        );
    }

    #[test]
    fn preserves_microseconds_and_nanoseconds_across_unix_time_and_offsets() {
        let now = DateTime::from_timestamp(1_700_000_000, 123_456_789).unwrap();
        for (query, expected) in [
            ("1700000000123456µs", "2023-11-14T22:13:20.123456Z"),
            ("-1 μs", "1969-12-31T23:59:59.999999Z"),
            ("-1ns", "1969-12-31T23:59:59.999999999Z"),
            ("1969-12-31T23:59:59.999999999Z", "-1 ns"),
            ("1970-01-01T00:00:00.0001Z", "100 µs"),
            ("1970-01-01T00:00:00.000000001000Z", "1 ns"),
            (
                "1970-01-01T00:00:00.000000001Z to -03:30",
                "1969-12-31T20:30:00.000000001-03:30",
            ),
            ("9999-12-31T23:59:59.000000001Z", "253402300799000000001 ns"),
            ("253402300799000000001ns", "9999-12-31T23:59:59.000000001Z"),
            ("现在微秒", "1700000000123456 µs"),
            ("现在纳秒", "1700000000123456789 ns"),
            ("now ns to +08:00", "2023-11-15T06:13:20.123456789+08:00"),
        ] {
            assert_eq!(
                convert_timestamp(query, now).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert_eq!(
            convert_timestamp("2016-12-31T23:59:60Z", now),
            Err(TimestampError::UnsupportedPrecision)
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
