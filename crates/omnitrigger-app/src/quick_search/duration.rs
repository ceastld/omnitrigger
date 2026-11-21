use super::calculator::normalize;

const NANOS_PER_SECOND: u128 = 1_000_000_000;
const MAX_DURATION_NANOS: u128 = (u64::MAX as u128 + 1) * NANOS_PER_SECOND - 1;
// 去掉末尾的零后，精度由单位纳秒数中的因子 2 和 5 决定。一天含有 16 个因子 2。
const MAX_FRACTION_DIGITS: usize = 16;
const SCALE_OPERATORS: [char; 3] = ['*', '×', '/'];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DurationError {
    Incomplete,
    Invalid,
    OutOfRange,
    Precision,
    DivisionByZero,
}

impl DurationError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入时长，例如 时长: 01:30:00 或 时长: 5400秒",
            Self::Invalid => "使用分:秒、时:分:秒或带单位的时长，可加减或乘除整数",
            Self::OutOfRange => "时长超出可转换范围",
            Self::Precision => "时长最小可精确到 1 纳秒",
            Self::DivisionByZero => "时长不能除以零",
        }
    }
}

pub(super) fn convert_duration(query: &str) -> Result<String, DurationError> {
    let input = normalize(query).to_ascii_lowercase();
    let input = input.trim();
    let mut negative = input.starts_with('-');
    let mut remaining = input.strip_prefix(['-', '+']).unwrap_or(input);
    let show_seconds = remaining.contains(':')
        && !remaining.contains(['+', '-'])
        && !remaining.contains(SCALE_OPERATORS);
    let mut total = 0i128;
    loop {
        let end = remaining.find(['+', '-']).unwrap_or(remaining.len());
        let term = remaining[..end].trim();
        if term.is_empty() {
            return Err(if end == remaining.len() {
                DurationError::Incomplete
            } else {
                DurationError::Invalid
            });
        }
        let nanos = parse_term(term)? as i128;
        total = total
            .checked_add(if negative { -nanos } else { nanos })
            .ok_or(DurationError::OutOfRange)?;
        if end == remaining.len() {
            break;
        }
        negative = remaining.as_bytes()[end] == b'-';
        remaining = &remaining[end + 1..];
    }

    let nanos = bounded_duration(Some(total.unsigned_abs()))?;
    let seconds = nanos / NANOS_PER_SECOND;
    let fraction = nanos % NANOS_PER_SECOND;
    let suffix = if fraction == 0 {
        String::new()
    } else {
        format!(".{fraction:09}").trim_end_matches('0').to_owned()
    };
    let sign = if total < 0 { "-" } else { "" };
    if show_seconds {
        Ok(format!("{sign}{seconds}{suffix} 秒"))
    } else {
        let (hours, minutes, remainder) = (seconds / 3_600, seconds / 60 % 60, seconds % 60);
        Ok(format!(
            "{sign}{hours:02}:{minutes:02}:{remainder:02}{suffix}"
        ))
    }
}

fn parse_term(term: &str) -> Result<u128, DurationError> {
    let end = term.find(SCALE_OPERATORS).unwrap_or(term.len());
    let (duration, scale) = term.split_at(end);
    let duration = duration.trim_end();
    let nanos = if duration.contains(':') {
        parse_clock(duration)?
    } else {
        parse_units(duration)?
    };
    let Some(operator) = scale.chars().next() else {
        return Ok(nanos);
    };
    let factor = number(scale[operator.len_utf8()..].trim())?;
    if operator != '/' {
        return bounded_duration(nanos.checked_mul(factor));
    }
    if factor == 0 {
        return Err(DurationError::DivisionByZero);
    }
    if !nanos.is_multiple_of(factor) {
        return Err(DurationError::Precision);
    }
    Ok(nanos / factor)
}

fn bounded_duration(nanos: Option<u128>) -> Result<u128, DurationError> {
    nanos
        .filter(|&nanos| nanos <= MAX_DURATION_NANOS)
        .ok_or(DurationError::OutOfRange)
}

fn number(input: &str) -> Result<u128, DurationError> {
    if input.is_empty() {
        return Err(DurationError::Incomplete);
    }
    if !input.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(DurationError::Invalid);
    }
    input.parse().map_err(|_| DurationError::OutOfRange)
}

fn parse_amount(input: &str, nanos_per_unit: u128) -> Result<u128, DurationError> {
    let (whole, fractional_nanos) = if let Some((whole, fraction)) = input.split_once('.') {
        if fraction.is_empty() {
            return Err(DurationError::Incomplete);
        }
        if !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(DurationError::Invalid);
        }
        let fraction = fraction.trim_end_matches('0');
        if fraction.len() > MAX_FRACTION_DIGITS {
            return Err(DurationError::Precision);
        }
        let partial_unit = if fraction.is_empty() {
            0
        } else {
            let numerator = number(fraction)? * nanos_per_unit;
            let denominator = 10_u128.pow(fraction.len() as u32);
            if !numerator.is_multiple_of(denominator) {
                return Err(DurationError::Precision);
            }
            numerator / denominator
        };
        (
            if whole.is_empty() { 0 } else { number(whole)? },
            partial_unit,
        )
    } else {
        (number(input)?, 0)
    };
    bounded_duration(
        whole
            .checked_mul(nanos_per_unit)
            .and_then(|nanos| nanos.checked_add(fractional_nanos)),
    )
}

fn parse_clock(input: &str) -> Result<u128, DurationError> {
    let parts: Vec<_> = input.split(':').take(4).map(str::trim).collect();
    let (hours, minutes, seconds) = match parts.as_slice() {
        [minutes, seconds] => (0, number(minutes)?, *seconds),
        [hours, minutes, seconds] => {
            let minutes = number(minutes)?;
            if minutes >= 60 {
                return Err(DurationError::Invalid);
            }
            (number(hours)?, minutes, *seconds)
        }
        _ => return Err(DurationError::Invalid),
    };
    let nanos = parse_amount(seconds, NANOS_PER_SECOND)?;
    if nanos >= 60 * NANOS_PER_SECOND {
        return Err(DurationError::Invalid);
    }
    bounded_duration(
        hours
            .checked_mul(60)
            .and_then(|hours_in_minutes| hours_in_minutes.checked_add(minutes))
            .and_then(|minutes| minutes.checked_mul(60 * NANOS_PER_SECOND))
            .and_then(|whole_nanos| whole_nanos.checked_add(nanos)),
    )
}

fn parse_units(mut input: &str) -> Result<u128, DurationError> {
    let is_amount_byte = |byte: &u8| byte.is_ascii_digit() || *byte == b'.';
    if input.as_bytes().iter().all(is_amount_byte) {
        return parse_amount(input, NANOS_PER_SECOND);
    }
    let mut nanos = 0u128;
    while !input.is_empty() {
        let digit_count = input.bytes().take_while(is_amount_byte).count();
        if digit_count == 0 {
            return Err(DurationError::Invalid);
        }
        let (digits, suffix) = input.split_at(digit_count);
        let suffix = suffix.trim_start();
        if suffix.is_empty() {
            return Err(DurationError::Incomplete);
        }
        let (rest, nanos_per_unit) = [
            ("毫秒", 1_000_000),
            ("微秒", 1_000),
            ("纳秒", 1),
            ("ms", 1_000_000),
            ("us", 1_000),
            ("µs", 1_000),
            ("μs", 1_000),
            ("ns", 1),
            ("天", 86_400 * NANOS_PER_SECOND),
            ("小时", 3_600 * NANOS_PER_SECOND),
            ("分钟", 60 * NANOS_PER_SECOND),
            ("秒钟", NANOS_PER_SECOND),
            ("时", 3_600 * NANOS_PER_SECOND),
            ("分", 60 * NANOS_PER_SECOND),
            ("秒", NANOS_PER_SECOND),
            ("hours", 3_600 * NANOS_PER_SECOND),
            ("hour", 3_600 * NANOS_PER_SECOND),
            ("hrs", 3_600 * NANOS_PER_SECOND),
            ("hr", 3_600 * NANOS_PER_SECOND),
            ("h", 3_600 * NANOS_PER_SECOND),
            ("minutes", 60 * NANOS_PER_SECOND),
            ("minute", 60 * NANOS_PER_SECOND),
            ("min", 60 * NANOS_PER_SECOND),
            ("m", 60 * NANOS_PER_SECOND),
            ("seconds", NANOS_PER_SECOND),
            ("second", NANOS_PER_SECOND),
            ("sec", NANOS_PER_SECOND),
            ("s", NANOS_PER_SECOND),
            ("days", 86_400 * NANOS_PER_SECOND),
            ("day", 86_400 * NANOS_PER_SECOND),
            ("d", 86_400 * NANOS_PER_SECOND),
        ]
        .into_iter()
        .find_map(|(unit, scale)| suffix.strip_prefix(unit).map(|rest| (rest, scale)))
        .ok_or(DurationError::Invalid)?;
        let amount = parse_amount(digits, nanos_per_unit)?;
        nanos = bounded_duration(nanos.checked_add(amount))?;
        input = rest.trim_start();
    }
    Ok(nanos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_clock_and_unit_durations_using_exact_seconds() {
        for (query, expected) in [
            ("01:30:00", "5400 秒"),
            ("90:00", "5400 秒"),
            ("−1:00", "-60 秒"),
            ("5400", "01:30:00"),
            ("2小时20分钟10秒", "02:20:10"),
            ("1h 15m 5s", "01:15:05"),
            ("３６ ｈ", "36:00:00"),
            ("-0s", "00:00:00"),
        ] {
            assert_eq!(convert_duration(query), Ok(expected.into()), "{query}");
        }
        assert_eq!(convert_duration("1:"), Err(DurationError::Incomplete));
        assert_eq!(convert_duration("1:60:00"), Err(DurationError::Invalid));
        assert_eq!(convert_duration("10:60"), Err(DurationError::Invalid));
        assert_eq!(convert_duration("1.2.3s"), Err(DurationError::Invalid));
        assert_eq!(
            convert_duration("18446744073709551615h"),
            Err(DurationError::OutOfRange)
        );
    }

    #[test]
    fn adds_and_formats_fractional_durations_exactly() {
        for (query, expected) in [
            ("00:00:01.25 + 500ms", "00:00:01.75"),
            ("59.5s + 500ms", "00:01:00"),
            ("0:00.000000001", "0.000000001 秒"),
            ("-0:00.1250", "-0.125 秒"),
            ("1.5h 200微秒 1ns", "01:30:00.000200001"),
            ("1ms - 1.001ms", "-00:00:00.000001"),
            (".5", "00:00:00.5"),
            ("0.00000000001h", "00:00:00.000000036"),
            ("18446744073709551616ns", "5124095:34:33.709551616"),
            (
                "18446744073709551615.999999999s + 1ns - 1ns",
                "5124095576030431:00:15.999999999",
            ),
        ] {
            assert_eq!(convert_duration(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(convert_duration("0.1ns"), Err(DurationError::Precision));
        assert_eq!(
            convert_duration("0.0000000001s"),
            Err(DurationError::Precision)
        );
        assert_eq!(convert_duration("1:60.001"), Err(DurationError::Invalid));
        assert_eq!(convert_duration("1:0."), Err(DurationError::Incomplete));
        assert_eq!(
            convert_duration("18446744073709551616s"),
            Err(DurationError::OutOfRange)
        );
    }

    #[test]
    fn scales_duration_terms_by_integers_before_adding_them() {
        for (query, expected) in [
            ("45分钟 * 12", "09:00:00"),
            ("01:30:00 / 3", "00:30:00"),
            ("1:00 + 20秒 × 3", "00:02:00"),
            ("-3ns / 3 + 2ns", "00:00:00.000000001"),
            ("5s * 0", "00:00:00"),
            ("1h＊２", "02:00:00"),
        ] {
            assert_eq!(convert_duration(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            ("1h / 0", DurationError::DivisionByZero),
            ("1ns / 2", DurationError::Precision),
            ("1h *", DurationError::Incomplete),
            ("1h */2", DurationError::Invalid),
            ("1h * 1.5", DurationError::Invalid),
            ("1h * 1h", DurationError::Invalid),
            ("18446744073709551615s * 2", DurationError::OutOfRange),
        ] {
            assert_eq!(convert_duration(query), Err(error), "{query}");
        }
    }

    #[test]
    fn combines_signed_durations_without_rounding() {
        for (query, expected) in [
            ("01:30:00 - 25分钟", "01:05:00"),
            ("30:00 + 45m + 90", "01:16:30"),
            ("-1h30m + 30m", "-01:00:00"),
            ("1天＋2小时", "26:00:00"),
            ("2days - 48h", "00:00:00"),
            ("18446744073709551615s + 1s - 1s", "5124095576030431:00:15"),
        ] {
            assert_eq!(convert_duration(query), Ok(expected.into()), "{query}");
        }
        assert_eq!(convert_duration("1h-"), Err(DurationError::Incomplete));
        assert_eq!(convert_duration("1h+-30m"), Err(DurationError::Invalid));
        assert_eq!(
            convert_duration("18446744073709551615s + 1s"),
            Err(DurationError::OutOfRange),
        );
    }
}
