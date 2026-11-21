use super::calculator::normalize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DurationError {
    Incomplete,
    Invalid,
    OutOfRange,
}

impl DurationError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入时长，例如 时长: 01:30:00 或 时长: 5400秒",
            Self::Invalid => "使用分:秒、时:分:秒或带单位的时长，可用 + 或 - 计算",
            Self::OutOfRange => "时长超出可转换范围",
        }
    }
}

pub(super) fn convert_duration(query: &str) -> Result<String, DurationError> {
    let input = normalize(query).to_ascii_lowercase();
    let input = input.trim();
    let mut negative = input.starts_with('-');
    let mut remaining = input.strip_prefix(['-', '+']).unwrap_or(input);
    let show_seconds = !remaining.contains(['+', '-']) && remaining.contains(':');
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
        let seconds = if term.contains(':') {
            parse_clock(term)?
        } else {
            parse_units(term)?
        };
        let seconds = i128::from(seconds);
        total += if negative { -seconds } else { seconds };
        if end == remaining.len() {
            break;
        }
        negative = remaining.as_bytes()[end] == b'-';
        remaining = &remaining[end + 1..];
    }

    let seconds = u64::try_from(total.unsigned_abs()).map_err(|_| DurationError::OutOfRange)?;
    let sign = if total < 0 { "-" } else { "" };
    if show_seconds {
        Ok(format!("{sign}{seconds} 秒"))
    } else {
        let (hours, minutes, remainder) = (seconds / 3_600, seconds / 60 % 60, seconds % 60);
        Ok(format!("{sign}{hours:02}:{minutes:02}:{remainder:02}"))
    }
}

fn number(input: &str) -> Result<u64, DurationError> {
    if input.is_empty() {
        return Err(DurationError::Incomplete);
    }
    if !input.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(DurationError::Invalid);
    }
    input.parse().map_err(|_| DurationError::OutOfRange)
}

fn parse_clock(input: &str) -> Result<u64, DurationError> {
    let parts: Vec<_> = input.split(':').take(4).map(str::trim).collect();
    let (hours, minutes, seconds) = match parts.as_slice() {
        [minutes, seconds] => (0, number(minutes)?, number(seconds)?),
        [hours, minutes, seconds] => {
            let minutes = number(minutes)?;
            if minutes >= 60 {
                return Err(DurationError::Invalid);
            }
            (number(hours)?, minutes, number(seconds)?)
        }
        _ => return Err(DurationError::Invalid),
    };
    if seconds >= 60 {
        return Err(DurationError::Invalid);
    }
    hours
        .checked_mul(3_600)
        .and_then(|hours| hours.checked_add(minutes.checked_mul(60)?))
        .and_then(|minutes| minutes.checked_add(seconds))
        .ok_or(DurationError::OutOfRange)
}

fn parse_units(mut input: &str) -> Result<u64, DurationError> {
    if input.bytes().all(|byte| byte.is_ascii_digit()) {
        return number(input);
    }
    let mut seconds = 0u64;
    while !input.is_empty() {
        let digit_count = input.bytes().take_while(u8::is_ascii_digit).count();
        if digit_count == 0 {
            return Err(DurationError::Invalid);
        }
        let (digits, suffix) = input.split_at(digit_count);
        let suffix = suffix.trim_start();
        if suffix.is_empty() {
            return Err(DurationError::Incomplete);
        }
        let (rest, seconds_per_unit) = [
            ("天", 86_400),
            ("小时", 3_600),
            ("分钟", 60),
            ("秒钟", 1),
            ("时", 3_600),
            ("分", 60),
            ("秒", 1),
            ("hours", 3_600),
            ("hour", 3_600),
            ("hrs", 3_600),
            ("hr", 3_600),
            ("h", 3_600),
            ("minutes", 60),
            ("minute", 60),
            ("min", 60),
            ("m", 60),
            ("seconds", 1),
            ("second", 1),
            ("sec", 1),
            ("s", 1),
            ("days", 86_400),
            ("day", 86_400),
            ("d", 86_400),
        ]
        .into_iter()
        .find_map(|(unit, scale)| suffix.strip_prefix(unit).map(|rest| (rest, scale)))
        .ok_or(DurationError::Invalid)?;
        seconds = number(digits)?
            .checked_mul(seconds_per_unit)
            .and_then(|duration| seconds.checked_add(duration))
            .ok_or(DurationError::OutOfRange)?;
        input = rest.trim_start();
    }
    Ok(seconds)
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
        assert_eq!(convert_duration("1.2s"), Err(DurationError::Invalid));
        assert_eq!(
            convert_duration("18446744073709551615h"),
            Err(DurationError::OutOfRange)
        );
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
