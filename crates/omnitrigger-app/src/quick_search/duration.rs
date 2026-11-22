use std::time::Duration;

use num_bigint::BigInt;
use num_rational::BigRational;

use super::calculator::{format_result, normalize};
use super::conversion::split_conversion;
use super::fraction::calculate_fraction;

pub(super) const NANOS_PER_SECOND: u128 = 1_000_000_000;
const NANOS_PER_DAY: u128 = 86_400 * NANOS_PER_SECOND;
const MAX_DURATION_NANOS: u128 = (u64::MAX as u128 + 1) * NANOS_PER_SECOND - 1;
// 去掉末尾的零后，精度由单位纳秒数中的因子 2 和 5 决定。一天含有 16 个因子 2。
const MAX_FRACTION_DIGITS: usize = 16;
const SCALE_OPERATORS: [char; 3] = ['*', '×', '/'];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DurationError {
    Incomplete,
    Invalid,
    InvalidClockTime,
    InvalidBreakArguments,
    BreakOutOfRange,
    InvalidUnit,
    OutOfRange,
    Precision,
    DivisionByZero,
    InvalidFrequency,
    InvalidScaleFactor,
}

impl DurationError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入时长，例如 时长: 01:30:00 或 时长: 5400秒",
            Self::Invalid => "使用分:秒、时:分:秒、带单位的时长或 PnDTnHnMnS",
            Self::InvalidClockTime => "输入两个 24 小时时刻，例如 23:45 到 01:15",
            Self::InvalidBreakArguments => {
                "用竖线分隔时刻区间和休息时长，例如 23:45 到 01:15 | 30分钟"
            }
            Self::BreakOutOfRange => "休息时长须在零和区间总时长之间",
            Self::InvalidUnit => "请选择 s、ms、µs、ns、min、h、d 或 iso",
            Self::OutOfRange => "时长超出可转换范围",
            Self::Precision => "时长最小可精确到 1 纳秒",
            Self::DivisionByZero => "时长不能除以零",
            Self::InvalidFrequency => "按 时长@次数 填写每组，次数须为正整数",
            Self::InvalidScaleFactor => "用 @ 分隔时长和缩放系数，系数可写 2/3、50% 或精确算式",
        }
    }
}

pub(super) fn convert_duration(query: &str) -> Result<String, DurationError> {
    let input = normalize(query).to_ascii_lowercase();
    let operation = input.split_whitespace().next().unwrap_or_default();
    if matches!(operation, "排序" | "sort" | "降序" | "sort-desc") {
        let values = query
            .trim_start()
            .split_once(char::is_whitespace)
            .map_or("", |(_, values)| values);
        return sort_durations(values, matches!(operation, "降序" | "sort-desc"));
    }
    let input = input.trim();
    let (input, output) = match split_conversion(input) {
        Some((source, output)) => (source, Some(output)),
        None => (input, None),
    };
    let (operation, values) = input.split_once(char::is_whitespace).unwrap_or((input, ""));
    if matches!(operation, "比率" | "比例" | "ratio") {
        if output.is_some() {
            return Err(DurationError::Invalid);
        }
        return duration_ratio(values);
    }
    let (total, show_seconds) = match operation {
        "合计" | "sum" => (aggregate_duration_list(values, false)?, false),
        "缩放" | "scale" => (scaled_duration(values)?, false),
        "平均" | "average" => (aggregate_duration_list(values, true)?, false),
        "频数平均" | "frequency-average" => (frequency_average(values)?, false),
        "频数中位数" | "frequency-median" => (median_duration_list(values, true)?, false),
        "扣除休息" | "subtract-break" => (elapsed_after_break(values)?, false),
        "中位数" | "median" => (median_duration_list(values, false)?, false),
        "最短" | "min" => (duration_extremes(values)?.0, false),
        "最长" | "max" => (duration_extremes(values)?.1, false),
        "极差" | "range" => {
            let (shortest, longest) = duration_extremes(values)?;
            (
                longest
                    .checked_sub(shortest)
                    .ok_or(DurationError::OutOfRange)?,
                false,
            )
        }
        _ => duration_value(input)?,
    };
    let nanos = bounded_duration(Some(total.unsigned_abs()))?;
    let sign = if total < 0 { "-" } else { "" };
    if let Some(output) = output {
        return format_duration_unit(nanos, sign, output);
    }
    if show_seconds {
        let seconds = nanos / NANOS_PER_SECOND;
        let suffix = fraction_suffix(nanos % NANOS_PER_SECOND, 9);
        Ok(format!("{sign}{seconds}{suffix} 秒"))
    } else {
        Ok(format_clock_duration(nanos, sign))
    }
}

fn duration_value(input: &str) -> Result<(i128, bool), DurationError> {
    if let Some((start, end)) = input.split_once(['到', '至']) {
        let start = time_of_day(start)?;
        let end = time_of_day(end)?;
        let elapsed = (end + NANOS_PER_DAY - start) % NANOS_PER_DAY;
        Ok((elapsed as i128, false))
    } else {
        signed_duration(input)
    }
}

fn elapsed_after_break(input: &str) -> Result<i128, DurationError> {
    let (interval, break_duration) = input
        .split_once('|')
        .filter(|(interval, _)| interval.contains(['到', '至']))
        .ok_or(DurationError::InvalidBreakArguments)?;
    let (elapsed, _) = duration_value(interval)?;
    let (break_duration, _) = signed_duration(break_duration.trim())?;
    if !(0..=elapsed).contains(&break_duration) {
        return Err(DurationError::BreakOutOfRange);
    }
    Ok(elapsed - break_duration)
}

fn duration_terms(input: &str) -> impl Iterator<Item = &str> {
    input
        .lines()
        .filter(|line| !line.trim().is_empty())
        .flat_map(|line| line.split([',', '，']))
        .map(str::trim)
}

fn bounded_duration_value(term: &str) -> Result<i128, DurationError> {
    let (nanos, _) = duration_value(term)?;
    bounded_duration(Some(nanos.unsigned_abs()))?;
    Ok(nanos)
}

fn scaled_duration(input: &str) -> Result<i128, DurationError> {
    let (duration, factor) = input
        .split_once('@')
        .ok_or(DurationError::InvalidScaleFactor)?;
    let nanos = bounded_duration_value(duration.trim())?;
    let factor = factor.trim();
    let (expression, denominator) = factor
        .strip_suffix('%')
        .map_or((factor, 1), |expression| (expression.trim(), 100));
    let factor = calculate_fraction(expression).map_err(|_| DurationError::InvalidScaleFactor)?;
    let scaled = factor * BigInt::from(nanos) / BigInt::from(denominator);
    if !scaled.is_integer() {
        return Err(DurationError::Precision);
    }
    i128::try_from(scaled.numer()).map_err(|_| DurationError::OutOfRange)
}

fn duration_ratio(input: &str) -> Result<String, DurationError> {
    let (left, right) = input.split_once('@').ok_or(DurationError::Incomplete)?;
    let left = bounded_duration_value(left.trim())?;
    let right = bounded_duration_value(right.trim())?;
    if right == 0 {
        return Err(DurationError::DivisionByZero);
    }
    Ok(BigRational::new(left.into(), right.into()).to_string())
}

fn duration_list_values(input: &str) -> impl Iterator<Item = Result<i128, DurationError>> + '_ {
    duration_terms(input).map(bounded_duration_value)
}

fn sort_durations(input: &str, descending: bool) -> Result<String, DurationError> {
    let mut values = duration_terms(input)
        .map(|term| {
            let normalized = normalize(term).to_ascii_lowercase();
            bounded_duration_value(&normalized).map(|nanos| (nanos, term))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if values.is_empty() {
        return Err(DurationError::Incomplete);
    }
    values.sort_by(|(left, _), (right, _)| {
        if descending {
            right.cmp(left)
        } else {
            left.cmp(right)
        }
    });
    Ok(values
        .into_iter()
        .map(|(_, term)| term)
        .collect::<Vec<_>>()
        .join("\n"))
}

fn aggregate_duration_list(input: &str, average: bool) -> Result<i128, DurationError> {
    let mut total = 0_i128;
    let mut count = 0_i128;
    for nanos in duration_list_values(input) {
        total = total.checked_add(nanos?).ok_or(DurationError::OutOfRange)?;
        count += 1;
    }
    if count == 0 {
        return Err(DurationError::Incomplete);
    }
    if !average {
        return Ok(total);
    }
    if total % count != 0 {
        return Err(DurationError::Precision);
    }
    Ok(total / count)
}

fn frequency_duration_values(
    input: &str,
) -> impl Iterator<Item = Result<(i128, u64), DurationError>> + '_ {
    duration_terms(input).map(|term| {
        let (value, repetitions) = term
            .split_once('@')
            .ok_or(DurationError::InvalidFrequency)?;
        let repetitions = repetitions
            .trim()
            .parse::<u64>()
            .ok()
            .filter(|&count| count > 0)
            .ok_or(DurationError::InvalidFrequency)?;
        let nanos = bounded_duration_value(value.trim())?;
        Ok((nanos, repetitions))
    })
}

fn frequency_average(input: &str) -> Result<i128, DurationError> {
    let mut total = BigInt::default();
    let mut count = BigInt::default();
    for term in frequency_duration_values(input) {
        let (nanos, repetitions) = term?;
        total += BigInt::from(nanos) * BigInt::from(repetitions);
        count += repetitions;
    }
    if count == BigInt::default() {
        return Err(DurationError::Incomplete);
    }
    if &total % &count != BigInt::default() {
        return Err(DurationError::Precision);
    }
    i128::try_from(total / count).map_err(|_| DurationError::OutOfRange)
}

fn duration_extremes(input: &str) -> Result<(i128, i128), DurationError> {
    let mut values = duration_list_values(input);
    let first = values.next().ok_or(DurationError::Incomplete)??;
    values.try_fold((first, first), |(shortest, longest), current| {
        let current = current?;
        Ok((shortest.min(current), longest.max(current)))
    })
}

fn median_duration_list(input: &str, use_frequencies: bool) -> Result<i128, DurationError> {
    let mut values = if use_frequencies {
        frequency_duration_values(input).collect::<Result<Vec<_>, _>>()?
    } else {
        duration_list_values(input)
            .map(|value| value.map(|value| (value, 1)))
            .collect::<Result<Vec<_>, _>>()?
    };
    let count: u128 = values.iter().map(|&(_, count)| u128::from(count)).sum();
    if count == 0 {
        return Err(DurationError::Incomplete);
    }
    values.sort_unstable_by_key(|&(nanos, _)| nanos);
    let mut center_rank = (count - 1) / 2;
    for (index, &(nanos, repetitions)) in values.iter().enumerate() {
        let repetitions = u128::from(repetitions);
        if center_rank >= repetitions {
            center_rank -= repetitions;
            continue;
        }
        if !count.is_multiple_of(2) || center_rank + 1 < repetitions {
            return Ok(nanos);
        }
        let sum = nanos
            .checked_add(values[index + 1].0)
            .ok_or(DurationError::OutOfRange)?;
        if sum % 2 != 0 {
            return Err(DurationError::Precision);
        }
        return Ok(sum / 2);
    }
    Err(DurationError::Incomplete)
}

pub(super) fn format_clock_duration(nanos: u128, sign: &str) -> String {
    let seconds = nanos / NANOS_PER_SECOND;
    let suffix = fraction_suffix(nanos % NANOS_PER_SECOND, 9);
    let (hours, minutes, remainder) = (seconds / 3_600, seconds / 60 % 60, seconds % 60);
    format!("{sign}{hours:02}:{minutes:02}:{remainder:02}{suffix}")
}

pub(super) fn is_clock_duration(text: &str) -> bool {
    text.contains(':') && parse_clock(text.strip_prefix('-').unwrap_or(text)).is_ok()
}

fn signed_duration(input: &str) -> Result<(i128, bool), DurationError> {
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

    Ok((total, show_seconds))
}

fn time_of_day(input: &str) -> Result<u128, DurationError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(DurationError::Incomplete);
    }
    let (_, remainder) = input
        .split_once(':')
        .ok_or(DurationError::InvalidClockTime)?;
    // 时刻的两段写法是时分，parse_clock 的两段写法是分秒。
    let nanos = if remainder.contains(':') {
        parse_clock(input)
    } else {
        parse_clock(&format!("{input}:00"))
    }
    .map_err(|error| match error {
        DurationError::Invalid | DurationError::OutOfRange => DurationError::InvalidClockTime,
        error => error,
    })?;
    if nanos >= NANOS_PER_DAY {
        return Err(DurationError::InvalidClockTime);
    }
    Ok(nanos)
}

fn fraction_suffix(fraction: u128, digits: usize) -> String {
    if fraction == 0 {
        String::new()
    } else {
        format!(".{fraction:0digits$}")
            .trim_end_matches('0')
            .to_owned()
    }
}

pub(super) fn format_duration_unit(
    nanos: u128,
    sign: &str,
    output: &str,
) -> Result<String, DurationError> {
    let (symbol, divisor, digits) = match output.trim() {
        "" => return Err(DurationError::Incomplete),
        "s" | "秒" => ("s", NANOS_PER_SECOND, Some(9)),
        "ms" | "毫秒" => ("ms", 1_000_000, Some(6)),
        "us" | "µs" | "μs" | "微秒" => ("µs", 1_000, Some(3)),
        "ns" | "纳秒" => ("ns", 1, Some(0)),
        "min" | "m" | "分" | "分钟" => ("min", 60 * NANOS_PER_SECOND, None),
        "h" | "时" | "小时" => ("h", 3_600 * NANOS_PER_SECOND, None),
        "d" | "天" => ("d", NANOS_PER_DAY, None),
        "iso" | "iso8601" => return Ok(format_iso_duration(nanos, sign)),
        _ => return Err(DurationError::InvalidUnit),
    };
    let amount = if let Some(digits) = digits {
        let whole = nanos / divisor;
        let suffix = fraction_suffix(nanos % divisor, digits);
        format!("{whole}{suffix}")
    } else {
        format_result(nanos as f64 / divisor as f64).map_err(|_| DurationError::OutOfRange)?
    };
    Ok(format!("{sign}{amount} {symbol}"))
}

fn format_iso_duration(nanos: u128, sign: &str) -> String {
    let days = nanos / NANOS_PER_DAY;
    let rest = nanos % NANOS_PER_DAY;
    let seconds = rest / NANOS_PER_SECOND;
    let fraction = rest % NANOS_PER_SECOND;
    let mut output = format!("{sign}P");
    if days != 0 {
        output.push_str(&format!("{days}D"));
        if rest == 0 {
            return output;
        }
    }
    output.push('T');
    let hours = seconds / 3_600;
    let minutes = seconds / 60 % 60;
    let seconds = seconds % 60;
    if hours != 0 {
        output.push_str(&format!("{hours}H"));
    }
    if minutes != 0 {
        output.push_str(&format!("{minutes}M"));
    }
    if seconds != 0 || fraction != 0 || hours == 0 && minutes == 0 {
        output.push_str(&format!("{seconds}{}S", fraction_suffix(fraction, 9)));
    }
    output
}

pub(super) fn elapsed_duration(input: &str) -> Result<Duration, DurationError> {
    let input = normalize(input).to_ascii_lowercase();
    let nanos = parse_term(input.trim())?;
    Ok(Duration::new(
        (nanos / NANOS_PER_SECOND) as u64,
        (nanos % NANOS_PER_SECOND) as u32,
    ))
}

fn parse_term(term: &str) -> Result<u128, DurationError> {
    let end = term.find(SCALE_OPERATORS).unwrap_or(term.len());
    let (duration, scale) = term.split_at(end);
    let duration = duration.trim_end();
    let nanos = if let Some(iso) = duration.strip_prefix('p') {
        parse_iso_duration(iso)?
    } else if duration.contains(':') {
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

fn parse_iso_duration(input: &str) -> Result<u128, DurationError> {
    let (date, time) = input
        .split_once('t')
        .map_or((input, None), |(date, time)| (date, Some(time)));
    let mut total = match date.strip_suffix('d') {
        Some(days) => bounded_duration(number(days)?.checked_mul(NANOS_PER_DAY))?,
        None if date.is_empty() => 0,
        None => return Err(DurationError::Invalid),
    };
    let Some(mut time) = time else {
        return if date.is_empty() {
            Err(DurationError::Incomplete)
        } else {
            Ok(total)
        };
    };
    if time.is_empty() {
        return Err(DurationError::Incomplete);
    }
    for (suffix, nanos_per_unit) in [
        ('h', 3_600 * NANOS_PER_SECOND),
        ('m', 60 * NANOS_PER_SECOND),
        ('s', NANOS_PER_SECOND),
    ] {
        if let Some((amount, next)) = time.split_once(suffix) {
            let nanos = if suffix == 's' {
                parse_amount(amount, nanos_per_unit)?
            } else {
                bounded_duration(number(amount)?.checked_mul(nanos_per_unit))?
            };
            total = bounded_duration(total.checked_add(nanos))?;
            time = next;
        }
    }
    if time.is_empty() {
        Ok(total)
    } else {
        Err(DurationError::Invalid)
    }
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
    fn orders_mixed_duration_notation_by_nanoseconds_and_keeps_ties_stable() {
        assert_eq!(
            convert_duration("排序 02:40，PT2M\n00:55,02:00").as_deref(),
            Ok("00:55\nPT2M\n02:00\n02:40")
        );
        assert_eq!(
            convert_duration("ＳＯＲＴ－ＤＥＳＣ 1ns,-2ns,0.000000001s").as_deref(),
            Ok("1ns\n0.000000001s\n-2ns")
        );
        assert_eq!(convert_duration("排序"), Err(DurationError::Incomplete));
        assert_eq!(
            convert_duration("排序 0.5ns,1ns"),
            Err(DurationError::Precision)
        );
    }

    #[test]
    fn divides_two_durations_as_exact_ratios_without_rounding_to_nanoseconds() {
        for (query, expected) in [
            ("比率 45分钟 @ 2小时", "3/8"),
            ("比率 23:45 到 01:15 @ 2小时", "3/4"),
            ("比例 00:00:01.25 + 500ms @ 7秒", "1/4"),
            ("ratio 1ns @ 3ns", "1/3"),
            ("比率 2秒 @ -3秒", "-2/3"),
            ("比率 0秒 @ 1ns", "0"),
        ] {
            assert_eq!(convert_duration(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            convert_duration("比率 1秒 @ 0秒"),
            Err(DurationError::DivisionByZero),
        );
        assert_eq!(convert_duration("比率 1秒"), Err(DurationError::Incomplete),);
        assert_eq!(
            convert_duration("比率 1秒 @ 2秒 to ms"),
            Err(DurationError::Invalid),
        );
    }

    #[test]
    fn finds_shortest_longest_and_spread_of_signed_durations() {
        for (query, expected) in [
            ("最短 01:25,PT2M,00:55", "00:00:55"),
            ("最长 01:25,02:40,00:55 to s", "160 s"),
            ("极差 01:25,02:40,00:55", "00:01:45"),
            ("range -2ns\n1ns to ns", "3 ns"),
            ("range 03:20", "00:00:00"),
        ] {
            assert_eq!(convert_duration(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(convert_duration("最短"), Err(DurationError::Incomplete));
        assert_eq!(
            convert_duration("range -18446744073709551615s,18446744073709551615s"),
            Err(DurationError::OutOfRange)
        );
    }

    #[test]
    fn finds_the_exact_middle_duration_in_mixed_lists() {
        for (query, expected) in [
            ("中位数 01:25,02:40,00:55", "00:01:25"),
            ("median 1s,3s to ms", "2000 ms"),
            ("中位数 -2ns,0ns to ns", "-1 ns"),
            ("中位数 23:50 到 00:10\n10分钟\n0ns to iso", "PT10M"),
            ("中位数 05:20", "00:05:20"),
        ] {
            assert_eq!(convert_duration(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(convert_duration("中位数"), Err(DurationError::Incomplete));
        assert_eq!(
            convert_duration("中位数 0ns,1ns"),
            Err(DurationError::Precision)
        );
    }

    #[test]
    fn calculates_exact_duration_medians_with_frequency_groups() {
        for (query, expected) in [
            ("频数中位数 01:00@2,02:00@2", "00:01:30"),
            ("频数中位数 2s@2,-1s@1,1s@1 to ms", "1500 ms"),
            ("frequency-median 2s@4,1s@1", "00:00:02"),
            (
                "频数中位数 01:00@18446744073709551615,02:00@18446744073709551615",
                "00:01:30",
            ),
        ] {
            assert_eq!(convert_duration(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            ("频数中位数", DurationError::Incomplete),
            ("频数中位数 1s@0", DurationError::InvalidFrequency),
            ("频数中位数 1ns@1,2ns@1", DurationError::Precision),
        ] {
            assert_eq!(convert_duration(query), Err(error), "{query}");
        }
    }

    #[test]
    fn scales_durations_by_exact_fractions_percentages_and_expressions() {
        for (query, expected) in [
            ("缩放 01:30:00 @ 2/3", "01:00:00"),
            ("缩放 23:45 到 01:15 @ 50% to min", "45 min"),
            ("缩放 3ns @ -1/3", "-00:00:00.000000001"),
            ("scale PT2H @ 1/(1+2) to s", "2400 s"),
            ("缩放 1小时 @ 0%", "00:00:00"),
        ] {
            assert_eq!(convert_duration(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            ("缩放 2s", DurationError::InvalidScaleFactor),
            ("缩放 2s @ 茶", DurationError::InvalidScaleFactor),
            ("缩放 1ns @ 50%", DurationError::Precision),
        ] {
            assert_eq!(convert_duration(query), Err(error), "{query}");
        }
    }

    #[test]
    fn calculates_exact_frequency_averages_without_expanding_observations() {
        for (input, expected) in [
            ("频数平均 01:00@3,02:00@1", "00:01:15"),
            ("frequency-average -2s@1,1s@2 to ns", "0 ns"),
            (
                "频数平均 01:30@18446744073709551615,01:30@18446744073709551615",
                "00:01:30",
            ),
        ] {
            assert_eq!(convert_duration(input).as_deref(), Ok(expected));
        }
        for (input, error) in [
            ("频数平均 2s@0", DurationError::InvalidFrequency),
            ("频数平均 2s@1.5", DurationError::InvalidFrequency),
            ("频数平均 1ns@1,2ns@1", DurationError::Precision),
            ("频数平均", DurationError::Incomplete),
        ] {
            assert_eq!(convert_duration(input), Err(error));
        }
    }

    #[test]
    fn aggregates_duration_lists_exactly_with_existing_unit_and_clock_rules() {
        for (query, expected) in [
            ("合计 01:25,02:40,00:55", "00:05:00"),
            ("平均 01:25,02:40,00:55", "00:01:40"),
            ("合计 23:45 到 01:15，500ms to iso", "PT1H30M0.5S"),
            ("average -2s,1s to ms", "-500 ms"),
            ("sum 1秒\r\n\r\n500ms\n-250ms to ns", "1250000000 ns"),
            ("合计 P1DT1S,2s * 3", "24:00:07"),
        ] {
            assert_eq!(convert_duration(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(convert_duration("合计"), Err(DurationError::Incomplete));
        assert_eq!(
            convert_duration("合计 1s,,2s"),
            Err(DurationError::Incomplete)
        );
        assert_eq!(
            convert_duration("平均 1ns,2ns"),
            Err(DurationError::Precision)
        );
    }

    #[test]
    fn subtracts_exact_break_duration_from_elapsed_time_across_midnight() {
        for (query, expected) in [
            ("扣除休息 23:45 到 01:15 | 20分钟 + 10分钟", "01:00:00"),
            ("扣除休息 23:59:59.999999998 至 00:00 ｜ 1ns to ns", "1 ns"),
            ("subtract-break 08:00 到 08:30 | PT30M", "00:00:00"),
            ("扣除休息 08:00 到 08:30 | 15:00 to min", "15 min"),
        ] {
            assert_eq!(convert_duration(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, expected) in [
            (
                "扣除休息 08:00 到 08:15",
                DurationError::InvalidBreakArguments,
            ),
            (
                "扣除休息 08:00 | 10分钟",
                DurationError::InvalidBreakArguments,
            ),
            (
                "扣除休息 08:00 到 08:15 | -1s",
                DurationError::BreakOutOfRange,
            ),
            (
                "扣除休息 08:00 到 08:15 | 16min",
                DurationError::BreakOutOfRange,
            ),
        ] {
            assert_eq!(convert_duration(query), Err(expected), "{query}");
        }
    }

    #[test]
    fn calculates_elapsed_time_between_clock_times_including_midnight() {
        for (query, expected) in [
            ("08:15 到 10:30", "02:15:00"),
            ("23:45 到 01:15", "01:30:00"),
            ("２３：４５ 至 ０１：１５ to min", "90 min"),
            ("23:59:59.999999999 到 00:00", "00:00:00.000000001"),
            ("03:45 到 03:45", "00:00:00"),
        ] {
            assert_eq!(convert_duration(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            ("24:00 到 01:15", DurationError::InvalidClockTime),
            ("01:60 到 02:00", DurationError::InvalidClockTime),
            ("01:00 到", DurationError::Incomplete),
            ("8 到 9", DurationError::InvalidClockTime),
        ] {
            assert_eq!(convert_duration(query), Err(error), "{query}");
        }
    }

    #[test]
    fn converts_whole_duration_expressions_to_explicit_units() {
        for (query, expected) in [
            ("01:30:00 to min", "90 min"),
            ("45分钟 * 12 转 小时", "9 h"),
            ("-1s + 250ms to ms", "-750 ms"),
            ("1ns to μs", "0.001 µs"),
            ("1ns to min", "1.66666666666667e-11 min"),
            (
                "18446744073709551615999999999ns to ns",
                "18446744073709551615999999999 ns",
            ),
        ] {
            assert_eq!(convert_duration(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(convert_duration("05:00 to"), Err(DurationError::Incomplete));
        assert_eq!(
            convert_duration("05:00 to kg"),
            Err(DurationError::InvalidUnit)
        );
    }

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
    fn converts_iso_day_time_durations_without_rounding() {
        for (query, expected) in [
            ("P1DT2.5S", "24:00:02.5"),
            ("PT.5S", "00:00:00.5"),
            ("PT90M to iso", "PT1H30M"),
            ("1天2.5秒 to iso", "P1DT2.5S"),
            ("0ns to iso", "PT0S"),
            ("-2d to iso8601", "-P2D"),
            ("P1D - PT1S", "23:59:59"),
            ("PT0.000000001S * 8 to iso", "PT0.000000008S"),
        ] {
            assert_eq!(convert_duration(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            ("P", DurationError::Incomplete),
            ("P1DT", DurationError::Incomplete),
            ("P2M", DurationError::Invalid),
            ("P1W", DurationError::Invalid),
            ("PT1M1H", DurationError::Invalid),
            ("PT1.5H", DurationError::Invalid),
            ("PT1H1H", DurationError::Invalid),
            ("PT0.0000000001S", DurationError::Precision),
        ] {
            assert_eq!(convert_duration(query), Err(error), "{query}");
        }
        let max = format_iso_duration(MAX_DURATION_NANOS, "").to_ascii_lowercase();
        assert_eq!(parse_term(&max), Ok(MAX_DURATION_NANOS));
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
