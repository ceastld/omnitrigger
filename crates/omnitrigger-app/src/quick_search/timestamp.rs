use std::collections::BTreeMap;

use chrono::{
    DateTime, Datelike, Days, FixedOffset, Local, Months, NaiveDate, NaiveTime, SecondsFormat,
    TimeDelta, Timelike, Utc,
};
use jiff::{
    civil::DateTime as CivilDateTime,
    tz::{AmbiguousOffset, Offset, TimeZone},
    Timestamp,
};

use super::calculator::normalize;
use super::calendar::{iso_quarter_start, parse_complete_date};
use super::conversion::split_conversion;
use super::csv_tools::csv_cell;
use super::duration::{
    elapsed_duration, format_clock_duration, format_duration_unit, DurationError, NANOS_PER_SECOND,
};

const MAX_TARGET_ZONES: usize = 8;
const MAX_SERIES_TIMESTAMPS: usize = 128;
const MAX_DAILY_DURATION_ROWS: usize = 128;
const MAX_ABSOLUTE_INTERVALS: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum TimestampError {
    Incomplete,
    Invalid,
    InvalidOffset,
    TooManyTargetZones,
    ZoneTableArguments,
    ZoneCandidateArguments,
    CivilDayArguments,
    CivilPeriodArguments,
    DailyDurationArguments,
    TooManyDurationDays,
    OffsetChangesArguments,
    DailyClockArguments,
    InvalidSourceZone,
    SourceClockGap,
    SourceClockFold,
    InvalidDuration,
    IntervalEndpoints,
    IntervalOutput(DurationError),
    AbsoluteIntervalsArguments,
    AbsoluteIntervalOrder,
    AlignmentArguments,
    SeriesArguments,
    UnsupportedPrecision,
    OutOfRange,
}

impl TimestampError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入时间戳或带时区的时间，例如 时间戳: 1700000000000ms",
            Self::Invalid => "输入 Unix 时间戳或带 UTC 偏移量或地区时区的日期时间",
            Self::InvalidOffset => "输入目标时区，例如 本地、+08:00 或 Asia/Shanghai",
            Self::TooManyTargetZones => "可用逗号隔开最多 8 个目标时区",
            Self::ZoneTableArguments => {
                "填写同一时刻和目标时区，例如 时间戳: 时区CSV 0 to UTC, Asia/Shanghai"
            }
            Self::ZoneCandidateArguments => {
                "填写当地日期、时间和地区时区，例如 2026-11-01 01:30 America/New_York"
            }
            Self::CivilDayArguments => "填写完整日期或顺序正确的日期范围，以及明确时区",
            Self::CivilPeriodArguments => {
                "填写 ISO 周、年月、季度或年份及明确时区，例如 2026-Q1 America/New_York"
            }
            Self::DailyDurationArguments => "用“到”分隔起止时刻，起点须在终点之前或相同，再用 to 指定时区",
            Self::TooManyDurationDays => "逐日时长最多输出 128 个当地日期",
            Self::OffsetChangesArguments => "填写四位年份和明确时区，例如 2026 America/New_York",
            Self::DailyClockArguments => "填写当地起始时刻、明确时区和 1 至 128 天，例如 2026-03-07 02:30 America/New_York @ 3",
            Self::InvalidSourceZone => "检查原时间的地区时区，例如 Asia/Shanghai",
            Self::SourceClockGap => "该地区跳过了这一时刻，请检查输入的钟表时间",
            Self::SourceClockFold => "该地区的钟表时间出现两次，可用“时区候选”查看两次的 UTC 偏移",
            Self::InvalidDuration => "请输入要加减的时长，例如 时间戳: 现在 + 90分钟",
            Self::IntervalEndpoints => "用 到 或 至 隔开两个带时区的时间",
            Self::IntervalOutput(error) => error.message(),
            Self::AbsoluteIntervalsArguments => {
                "填写 1 至 32 段 [开始,结束]，例如 [[\"0s\",\"10s\"],[\"5s\",\"15s\"]]"
            }
            Self::AbsoluteIntervalOrder => "每段时间的结束须晚于开始",
            Self::AlignmentArguments => "填写正的时间步长和时间，例如 15分钟 @ 1700000000",
            Self::SeriesArguments => "填写起始时刻、正步长和 1 至 128 项，可指定一个目标时区",
            Self::UnsupportedPrecision => "请输入可用 Unix 纳秒表示的时间",
            Self::OutOfRange => "结果超出四位年份的时间范围",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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
            _ => format!("{count} {}", self.symbol()),
        }
    }

    fn symbol(self) -> &'static str {
        match self {
            Self::Seconds => "s",
            Self::Milliseconds => "ms",
            Self::Microseconds => "µs",
            Self::Nanoseconds => "ns",
        }
    }
}

enum DisplayZone {
    Utc,
    Local,
    Offset(FixedOffset),
    Named(TimeZone),
}

enum DisplayTargets<'a> {
    Single(DisplayZone),
    Multiple(Vec<(&'a str, DisplayZone)>),
    Epoch(TimestampUnit),
}

#[derive(Clone, Copy)]
enum TimestampAlignment {
    Floor,
    Ceil,
    Nearest,
}

pub(super) fn convert_timestamp(query: &str, now: DateTime<Utc>) -> Result<String, TimestampError> {
    convert_timestamp_with_local(query, now, |time| time.with_timezone(&Local).fixed_offset())
}

pub(super) fn is_offset_timestamp(text: &str) -> bool {
    text.as_bytes().get(10) == Some(&b'T') && parse_explicit_time(text, None).is_ok()
}

enum TimestampReport {
    Zones,
    CivilDays,
    CivilPeriod(CivilPeriod),
    DailyDurations,
    OffsetChanges,
    DailyClock,
    AbsoluteIntervals(IntervalOperation),
}

enum CivilPeriod {
    Week,
    Month,
    Quarter,
    Year,
}

enum IntervalOperation {
    Merge,
    Intersection,
    Difference,
}

type AbsoluteInterval = (DateTime<Utc>, DateTime<Utc>);

impl TimestampReport {
    fn from_operation(operation: &str) -> Option<Self> {
        match operation.to_ascii_lowercase().as_str() {
            "时区csv" | "zone-csv" => Some(Self::Zones),
            "日区间" | "day-intervals" => Some(Self::CivilDays),
            "周区间" | "week-intervals" => Some(Self::CivilPeriod(CivilPeriod::Week)),
            "月区间" | "month-intervals" => Some(Self::CivilPeriod(CivilPeriod::Month)),
            "季度区间" | "quarter-intervals" => Some(Self::CivilPeriod(CivilPeriod::Quarter)),
            "年区间" | "year-intervals" => Some(Self::CivilPeriod(CivilPeriod::Year)),
            "逐日时长" | "daily-durations" => Some(Self::DailyDurations),
            "偏移变更" | "offset-changes" => Some(Self::OffsetChanges),
            "每日同刻" | "daily-clock" => Some(Self::DailyClock),
            "区间合并" | "merge-intervals" => {
                Some(Self::AbsoluteIntervals(IntervalOperation::Merge))
            }
            "区间交集" | "intersect-intervals" => {
                Some(Self::AbsoluteIntervals(IntervalOperation::Intersection))
            }
            "区间差集" | "subtract-intervals" => {
                Some(Self::AbsoluteIntervals(IntervalOperation::Difference))
            }
            _ => None,
        }
    }
}

pub(super) fn is_timestamp_report(query: &str) -> bool {
    query
        .split_whitespace()
        .next()
        .is_some_and(|operation| TimestampReport::from_operation(&normalize(operation)).is_some())
}

fn convert_timestamp_with_local(
    query: &str,
    now: DateTime<Utc>,
    to_local: impl Fn(DateTime<Utc>) -> DateTime<FixedOffset>,
) -> Result<String, TimestampError> {
    let input = normalize(query);
    let input = input.trim();
    let (operation, argument) = input.split_once(char::is_whitespace).unwrap_or((input, ""));
    if operation == "间隔" || operation.eq_ignore_ascii_case("between") {
        return timestamp_interval(argument, now);
    }
    if operation == "覆盖时长" || operation.eq_ignore_ascii_case("covered-duration") {
        return absolute_intervals_covered_duration(argument, now);
    }
    if operation == "时区候选" || operation.eq_ignore_ascii_case("zone-candidates") {
        return timestamp_candidates(argument, &to_local);
    }
    let table = match TimestampReport::from_operation(operation) {
        Some(TimestampReport::CivilDays) => return civil_day_intervals(argument),
        Some(TimestampReport::CivilPeriod(period)) => {
            return civil_period_intervals(argument, period)
        }
        Some(TimestampReport::DailyDurations) => return daily_durations(argument, now),
        Some(TimestampReport::OffsetChanges) => return timezone_offset_changes(argument),
        Some(TimestampReport::DailyClock) => return daily_clock_series(argument),
        Some(TimestampReport::AbsoluteIntervals(operation)) => {
            return absolute_intervals_csv(argument, now, operation);
        }
        Some(TimestampReport::Zones) => true,
        None => false,
    };
    let input = if table { argument.trim_start() } else { input };
    let (input, target_zone) = if let Some((source, target)) = split_conversion(input) {
        let targets = if table {
            DisplayTargets::Multiple(parse_named_display_targets(target)?)
        } else {
            parse_display_targets(target)?
        };
        (source, Some(targets))
    } else if table {
        return Err(TimestampError::ZoneTableArguments);
    } else {
        (input, None)
    };
    let lower = input.to_ascii_lowercase();
    let local_suffix = if target_zone.is_none() {
        lower
            .strip_suffix("本地")
            .or_else(|| lower.strip_suffix(" local"))
            .or_else(|| (lower == "local").then_some(""))
    } else {
        None
    };
    let target_zone =
        target_zone.or(local_suffix.map(|_| DisplayTargets::Single(DisplayZone::Local)));
    let lower = local_suffix.unwrap_or(&lower).trim_end();
    let input = &input[..lower.len()];
    if matches!(input, "" | "+" | "-") {
        return Err(TimestampError::Incomplete);
    }

    let display_time = |time: DateTime<Utc>, format| match &target_zone {
        None => format_display_time(time, format, &DisplayZone::Utc, &to_local),
        Some(DisplayTargets::Single(zone)) => format_display_time(time, format, zone, &to_local),
        Some(DisplayTargets::Epoch(unit)) => {
            ensure_four_digit_year(time.year())?;
            format_timestamp_delta(unix_nanos(time), Some(unit.symbol()))
        }
        Some(DisplayTargets::Multiple(zones)) => {
            let rows = zones
                .iter()
                .map(|(name, zone)| {
                    let clock = format_display_time(time, format, zone, &to_local)?;
                    Ok(if table {
                        format!("{},{}", csv_cell(name), csv_cell(&clock))
                    } else {
                        format!("{name}\n{clock}")
                    })
                })
                .collect::<Result<Vec<_>, TimestampError>>()?;
            Ok(if table {
                format!("时区,时间\r\n{}", rows.join("\r\n"))
            } else {
                rows.join("\n")
            })
        }
    };
    let operation = input.split_whitespace().next().unwrap_or("");
    if operation == "序列" || operation.eq_ignore_ascii_case("timestamp-series") {
        if matches!(target_zone.as_ref(), Some(DisplayTargets::Multiple(_))) {
            return Err(TimestampError::SeriesArguments);
        }
        return timestamp_series(input[operation.len()..].trim_start(), now, display_time);
    }
    let alignment = match operation.to_ascii_lowercase().as_str() {
        "向下对齐" | "floor-time" => Some(TimestampAlignment::Floor),
        "向上对齐" | "ceil-time" => Some(TimestampAlignment::Ceil),
        "就近对齐" | "nearest-time" => Some(TimestampAlignment::Nearest),
        _ => None,
    };
    if let Some(alignment) = alignment {
        let time = align_timestamp(input[operation.len()..].trim_start(), now, alignment)?;
        let unit = TimestampUnit::from_subsecond(time.timestamp_subsec_nanos());
        return display_time(time, unit.date_format());
    }
    if let Some((sign, input)) = lower
        .strip_prefix("now")
        .or_else(|| lower.strip_prefix("现在"))
        .and_then(|rest| rest.trim_start().split_at_checked(1))
        .filter(|(sign, _)| matches!(*sign, "+" | "-"))
    {
        let elapsed = elapsed_duration(input).map_err(|error| match error {
            DurationError::Incomplete => TimestampError::Incomplete,
            DurationError::OutOfRange => TimestampError::OutOfRange,
            DurationError::Precision => TimestampError::UnsupportedPrecision,
            DurationError::Invalid
            | DurationError::InvalidClockTime
            | DurationError::InvalidOverlapArguments
            | DurationError::InvalidBreakArguments
            | DurationError::BreakOutOfRange
            | DurationError::InvalidUnit
            | DurationError::ExactUnitArguments
            | DurationError::InvalidFrequency
            | DurationError::InvalidQuantileArguments
            | DurationError::InvalidScaleFactor
            | DurationError::InvalidRoundingArguments
            | DurationError::InvalidAllocation
            | DurationError::Allocation(_)
            | DurationError::DivisionByZero => TimestampError::InvalidDuration,
        })?;
        let unit = TimestampUnit::from_subsecond(elapsed.subsec_nanos());
        let delta = TimeDelta::from_std(elapsed).map_err(|_| TimestampError::OutOfRange)?;
        let signed_delta = if sign == "-" { -delta } else { delta };
        let time = now
            .checked_add_signed(signed_delta)
            .ok_or(TimestampError::OutOfRange)?;
        if target_zone.is_some() {
            return display_time(time, unit.date_format());
        }
        ensure_four_digit_year(time.year())?;
        return Ok(unit.format_unix(time));
    }

    let suffixed = TimestampUnit::split_suffix(lower);
    let (unit, quantity) = suffixed.unwrap_or((TimestampUnit::Seconds, lower));
    let quantity = quantity.trim_end();
    if matches!(quantity, "现在" | "now") {
        if target_zone.is_some() {
            return display_time(now, unit.date_format());
        }
        return Ok(unit.format_unix(now));
    }
    let source = named_time_source(input);
    if source.is_none() && (suffixed.is_some() || unix_number(input)) {
        let time = parse_unix(quantity, unit)?;
        let precision = unit.max(TimestampUnit::from_subsecond(time.timestamp_subsec_nanos()));
        return display_time(time, precision.date_format());
    }

    let time = parse_explicit_time(input, source)?;
    let unit = TimestampUnit::from_subsecond(time.timestamp_subsec_nanos());
    if target_zone.is_some() {
        display_time(time, unit.date_format())
    } else {
        Ok(unit.format_unix(time))
    }
}

fn format_display_time(
    time: DateTime<Utc>,
    format: SecondsFormat,
    zone: &DisplayZone,
    to_local: &impl Fn(DateTime<Utc>) -> DateTime<FixedOffset>,
) -> Result<String, TimestampError> {
    ensure_four_digit_year(time.year())?;
    let zoned = match zone {
        DisplayZone::Utc => return Ok(time.to_rfc3339_opts(format, true)),
        DisplayZone::Local => to_local(time),
        DisplayZone::Offset(offset) => time.with_timezone(offset),
        DisplayZone::Named(zone) => {
            let nanos = i128::from(time.timestamp()) * 1_000_000_000
                + i128::from(time.timestamp_subsec_nanos());
            let instant =
                Timestamp::from_nanosecond(nanos).map_err(|_| TimestampError::OutOfRange)?;
            let offset = FixedOffset::east_opt(zone.to_offset(instant).seconds())
                .ok_or(TimestampError::OutOfRange)?;
            time.with_timezone(&offset)
        }
    };
    ensure_four_digit_year(zoned.year())?;
    if zoned.offset().local_minus_utc() % 60 == 0 {
        return Ok(zoned.to_rfc3339_opts(format, false));
    }
    // 旧时区偏移可能包含秒，保留实际偏移量。
    let clock = zoned.naive_local().and_utc().to_rfc3339_opts(format, true);
    Ok(format!(
        "{}{}",
        clock.trim_end_matches('Z'),
        zoned.format("%::z")
    ))
}

fn named_time_source(input: &str) -> Option<(&str, &str)> {
    input
        .rsplit_once(char::is_whitespace)
        .filter(|(_, zone)| zone.contains('/') || zone.eq_ignore_ascii_case("utc"))
}

fn civil_day_intervals(input: &str) -> Result<String, TimestampError> {
    let invalid = TimestampError::CivilDayArguments;
    let (dates, zone) = input
        .trim()
        .rsplit_once(char::is_whitespace)
        .ok_or(invalid)?;
    let (start, end) = dates.split_once(['到', '至']).unwrap_or((dates, dates));
    let start = parse_complete_date(start).map_err(|_| invalid)?;
    let end = parse_complete_date(end).map_err(|_| invalid)?;
    if start > end {
        return Err(invalid);
    }
    let zone = explicit_timezone(zone, invalid)?;
    let next_day = end.succ_opt().ok_or(TimestampError::OutOfRange)?;
    civil_intervals(start, next_day, zone)
}

fn civil_period_intervals(input: &str, period: CivilPeriod) -> Result<String, TimestampError> {
    let invalid = TimestampError::CivilPeriodArguments;
    let (value, zone) = input
        .trim()
        .rsplit_once(char::is_whitespace)
        .ok_or(invalid)?;
    let value = value.trim_end();
    let start = match period {
        CivilPeriod::Week if value.len() == 8 => parse_complete_date(&format!("{value}-1")),
        CivilPeriod::Month if value.len() == 7 => parse_complete_date(&format!("{value}-01")),
        CivilPeriod::Quarter => iso_quarter_start(value).ok_or(invalid)?,
        CivilPeriod::Year if value.len() == 4 => parse_complete_date(&format!("{value}-01-01")),
        _ => return Err(invalid),
    }
    .map_err(|_| invalid)?;
    let end = match period {
        CivilPeriod::Week => start.checked_add_days(Days::new(7)),
        CivilPeriod::Month => start.checked_add_months(Months::new(1)),
        CivilPeriod::Quarter => start.checked_add_months(Months::new(3)),
        CivilPeriod::Year => start.checked_add_months(Months::new(12)),
    }
    .ok_or(TimestampError::OutOfRange)?;
    civil_intervals(start, end, explicit_timezone(zone, invalid)?)
}

fn civil_intervals(
    start: NaiveDate,
    end_exclusive: NaiveDate,
    zone: TimeZone,
) -> Result<String, TimestampError> {
    let seconds = |date: NaiveDate| date.and_time(NaiveTime::MIN).and_utc().timestamp();
    let day_start = seconds(start);
    let day_end = seconds(end_exclusive);
    let first = Timestamp::new(day_start - i64::from(Offset::MAX.seconds()), 0)
        .map_err(|_| TimestampError::OutOfRange)?;
    let last = Timestamp::new(day_end - i64::from(Offset::MIN.seconds()), 0)
        .map_err(|_| TimestampError::OutOfRange)?;
    let mut range_start = first;
    let mut intervals = Vec::<(i64, i64)>::new();
    // 每段的 UTC 偏移恒定，可以直接把当地日期的边界映射到 UTC。
    for range_end in zone
        .following(first)
        .map(|transition| transition.timestamp())
        .take_while(|transition| *transition < last)
        .chain(std::iter::once(last))
    {
        let offset = i64::from(zone.to_offset(range_start).seconds());
        let start = (day_start - offset).max(range_start.as_second());
        let end = (day_end - offset).min(range_end.as_second());
        if start < end {
            if let Some(previous) = intervals.last_mut().filter(|previous| previous.1 == start) {
                previous.1 = end;
            } else {
                intervals.push((start, end));
            }
        }
        range_start = range_end;
    }
    let mut output = String::from("开始UTC,结束UTC,时长");
    for (start, end) in intervals {
        let duration = format_clock_duration(
            i128::from(end - start).unsigned_abs() * NANOS_PER_SECOND,
            "",
        );
        output.push_str(&format!(
            "\r\n{},{},{}",
            utc_second(start)?,
            utc_second(end)?,
            duration
        ));
    }
    Ok(output)
}

fn timezone_offset_changes(input: &str) -> Result<String, TimestampError> {
    let invalid = TimestampError::OffsetChangesArguments;
    let (year, zone) = input
        .trim()
        .split_once(char::is_whitespace)
        .ok_or(invalid)?;
    if year.len() != 4 || !year.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid);
    }
    let year = year.parse::<i32>().map_err(|_| invalid)?;
    let zone = explicit_timezone(zone, invalid)?;
    let new_year = |year| {
        NaiveDate::from_ymd_opt(year, 1, 1)
            .ok_or(TimestampError::OutOfRange)
            .map(|date| date.and_time(NaiveTime::MIN).and_utc().timestamp())
    };
    let begin = Timestamp::new(new_year(year)? - 1, 0).map_err(|_| TimestampError::OutOfRange)?;
    let end = Timestamp::new(new_year(year + 1)? - 1, 0).map_err(|_| TimestampError::OutOfRange)?;
    let mut previous = zone.to_offset(begin);
    let mut output = String::from("生效UTC,变更前偏移,变更后偏移,钟表变化");
    for transition in zone
        .following(begin)
        .take_while(|transition| transition.timestamp() <= end)
    {
        let next = transition.offset();
        if previous == next {
            continue;
        }
        let delta = next.seconds() - previous.seconds();
        let sign = if delta < 0 { "-" } else { "+" };
        let adjustment =
            format_clock_duration(u128::from(delta.unsigned_abs()) * NANOS_PER_SECOND, sign);
        let effective = utc_second(transition.timestamp().as_second())?;
        output.push_str(&format!("\r\n{effective},{previous},{next},{adjustment}"));
        previous = next;
    }
    Ok(output)
}

fn daily_clock_series(input: &str) -> Result<String, TimestampError> {
    let invalid = TimestampError::DailyClockArguments;
    let (source, count) = input.split_once('@').ok_or(invalid)?;
    let (clock, zone) = source
        .trim()
        .rsplit_once(char::is_whitespace)
        .ok_or(invalid)?;
    let clock = clock.trim_end();
    let count = count.trim().parse::<usize>().map_err(|_| invalid)?;
    if !(1..=MAX_SERIES_TIMESTAMPS).contains(&count) {
        return Err(invalid);
    }
    if has_extra_subsecond_digits(clock) {
        return Err(TimestampError::UnsupportedPrecision);
    }
    let zone = explicit_timezone(zone, invalid)?;
    let mut clock = parse_named_clock(clock)?;
    let mut output = String::from("当地时间,UTC,状态");
    for index in 0..count {
        let unresolved = zone.to_ambiguous_timestamp(clock);
        let mut add =
            |candidate: Result<Timestamp, jiff::Error>, status| -> Result<(), TimestampError> {
                let time = utc_from_timestamp(candidate.map_err(|_| TimestampError::OutOfRange)?)?;
                ensure_four_digit_year(time.year())?;
                let unit = TimestampUnit::from_subsecond(time.timestamp_subsec_nanos());
                let utc = time.to_rfc3339_opts(unit.date_format(), true);
                output.push_str(&format!("\r\n{clock},{utc},{status}"));
                Ok(())
            };
        match unresolved.offset() {
            AmbiguousOffset::Gap { .. } => output.push_str(&format!("\r\n{clock},\"\",跳过")),
            AmbiguousOffset::Fold { .. } => {
                add(unresolved.earlier(), "第一次")?;
                add(unresolved.later(), "第二次")?;
            }
            AmbiguousOffset::Unambiguous { .. } => add(unresolved.unambiguous(), "正常")?,
        }
        if index + 1 < count {
            clock = clock.tomorrow().map_err(|_| TimestampError::OutOfRange)?;
        }
    }
    Ok(output)
}

fn explicit_timezone(zone: &str, invalid: TimestampError) -> Result<TimeZone, TimestampError> {
    match parse_display_zone(zone)? {
        DisplayZone::Utc => Ok(TimeZone::UTC),
        DisplayZone::Offset(offset) => Offset::from_seconds(offset.local_minus_utc())
            .map(|offset| offset.to_time_zone())
            .map_err(|_| TimestampError::InvalidOffset),
        DisplayZone::Named(zone) => Ok(zone),
        DisplayZone::Local => Err(invalid),
    }
}

fn utc_second(seconds: i64) -> Result<String, TimestampError> {
    let time = DateTime::from_timestamp(seconds, 0).ok_or(TimestampError::OutOfRange)?;
    ensure_four_digit_year(time.year())?;
    Ok(time.to_rfc3339_opts(SecondsFormat::Secs, true))
}

fn timestamp_candidates(
    input: &str,
    to_local: &impl Fn(DateTime<Utc>) -> DateTime<FixedOffset>,
) -> Result<String, TimestampError> {
    let (clock, name) =
        named_time_source(input.trim()).ok_or(TimestampError::ZoneCandidateArguments)?;
    let zone = TimeZone::get(name).map_err(|_| TimestampError::InvalidSourceZone)?;
    let clock = clock.trim_end();
    let unresolved = zone.to_ambiguous_timestamp(parse_named_clock(clock)?);
    let zone = DisplayZone::Named(zone);
    let display = |candidate: Result<Timestamp, jiff::Error>| {
        let time = utc_from_timestamp(candidate.map_err(|_| TimestampError::OutOfRange)?)?;
        let time = validate_explicit_time(clock, time)?;
        let unit = TimestampUnit::from_subsecond(time.timestamp_subsec_nanos());
        format_display_time(time, unit.date_format(), &zone, to_local)
    };
    match unresolved.offset() {
        AmbiguousOffset::Gap { .. } => Err(TimestampError::SourceClockGap),
        AmbiguousOffset::Fold { .. } => {
            let earlier = display(unresolved.earlier())?;
            let later = display(unresolved.later())?;
            Ok(format!("{earlier}\n{later}"))
        }
        AmbiguousOffset::Unambiguous { .. } => display(unresolved.unambiguous()),
    }
}

fn parse_explicit_time(
    input: &str,
    source: Option<(&str, &str)>,
) -> Result<DateTime<Utc>, TimestampError> {
    let time = if let Some((clock, name)) = source {
        parse_named_source(clock.trim_end(), name)?
    } else {
        parse_offset_time(input)?
    };
    validate_explicit_time(input, time)
}

fn validate_explicit_time(
    input: &str,
    time: DateTime<Utc>,
) -> Result<DateTime<Utc>, TimestampError> {
    ensure_four_digit_year(time.year())?;
    let fraction = time.timestamp_subsec_nanos();
    if fraction >= 1_000_000_000 || has_extra_subsecond_digits(input) {
        return Err(TimestampError::UnsupportedPrecision);
    }
    Ok(time)
}

fn has_extra_subsecond_digits(input: &str) -> bool {
    input.split_once('.').is_some_and(|(_, digits)| {
        digits
            .bytes()
            .take_while(u8::is_ascii_digit)
            .skip(9)
            .any(|digit| digit != b'0')
    })
}

fn align_timestamp(
    input: &str,
    now: DateTime<Utc>,
    alignment: TimestampAlignment,
) -> Result<DateTime<Utc>, TimestampError> {
    let (step, value) = input
        .split_once('@')
        .ok_or(TimestampError::AlignmentArguments)?;
    let step = timestamp_alignment_step(step)?;
    let time = interval_endpoint(value.trim(), Some(now))?;
    align_instant(time, step, alignment)
}

fn timestamp_alignment_step(input: &str) -> Result<i128, TimestampError> {
    elapsed_duration(input)
        .ok()
        .filter(|duration| !duration.is_zero())
        .map(|duration| duration.as_nanos() as i128)
        .ok_or(TimestampError::AlignmentArguments)
}

fn align_instant(
    time: DateTime<Utc>,
    step: i128,
    alignment: TimestampAlignment,
) -> Result<DateTime<Utc>, TimestampError> {
    let nanos = unix_nanos(time);
    let remainder = nanos.rem_euclid(step);
    let use_later_boundary = match alignment {
        TimestampAlignment::Floor => false,
        TimestampAlignment::Ceil => remainder != 0,
        TimestampAlignment::Nearest => remainder >= step - remainder,
    };
    let aligned = nanos - remainder + if use_later_boundary { step } else { 0 };
    timestamp_from_unix_units(aligned, TimestampUnit::Nanoseconds)
}

fn timestamp_series(
    input: &str,
    now: DateTime<Utc>,
    display: impl Fn(DateTime<Utc>, SecondsFormat) -> Result<String, TimestampError>,
) -> Result<String, TimestampError> {
    let mut parts = input.split('@').map(str::trim);
    let (Some(start), Some(step), Some(count), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(TimestampError::SeriesArguments);
    };
    if start.is_empty() {
        return Err(TimestampError::SeriesArguments);
    }
    let step = elapsed_duration(step)
        .ok()
        .filter(|duration| !duration.is_zero())
        .ok_or(TimestampError::SeriesArguments)?;
    let count = count
        .parse::<usize>()
        .ok()
        .filter(|count| (1..=MAX_SERIES_TIMESTAMPS).contains(count))
        .ok_or(TimestampError::SeriesArguments)?;
    let start = interval_endpoint(start, Some(now))?;
    let format = TimestampUnit::from_subsecond(start.timestamp_subsec_nanos())
        .max(TimestampUnit::from_subsecond(step.subsec_nanos()))
        .date_format();
    let start = unix_nanos(start);
    let step = step.as_nanos() as i128;
    let lines = (0..count)
        .map(|index| {
            let time = timestamp_from_unix_units(
                start + step * index as i128,
                TimestampUnit::Nanoseconds,
            )?;
            display(time, format)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(lines.join("\n"))
}

fn unix_nanos(time: DateTime<Utc>) -> i128 {
    i128::from(time.timestamp()) * NANOS_PER_SECOND as i128
        + i128::from(time.timestamp_subsec_nanos())
}

fn timestamp_interval(input: &str, now: DateTime<Utc>) -> Result<String, TimestampError> {
    let (input, output) =
        split_conversion(input).map_or((input, None), |(input, output)| (input, Some(output)));
    let (start, end) = interval_nanos(input, now, TimestampError::IntervalEndpoints)?;
    format_timestamp_delta(end - start, output)
}

fn parse_absolute_intervals(
    input: &str,
    now: DateTime<Utc>,
) -> Result<Vec<AbsoluteInterval>, TimestampError> {
    let invalid = TimestampError::AbsoluteIntervalsArguments;
    let intervals: Vec<[String; 2]> = serde_json::from_str(input).map_err(|_| invalid)?;
    if !(1..=MAX_ABSOLUTE_INTERVALS).contains(&intervals.len()) {
        return Err(invalid);
    }
    intervals
        .into_iter()
        .map(|[start, end]| {
            let start = interval_endpoint(start.trim(), Some(now))?;
            let end = interval_endpoint(end.trim(), Some(now))?;
            if start >= end {
                return Err(TimestampError::AbsoluteIntervalOrder);
            }
            Ok((start, end))
        })
        .collect()
}

fn absolute_intervals_covered_duration(
    input: &str,
    now: DateTime<Utc>,
) -> Result<String, TimestampError> {
    let total = merge_absolute_intervals(parse_absolute_intervals(input, now)?)
        .into_iter()
        .map(|(start, end)| absolute_elapsed_nanoseconds(start, end))
        .sum();
    format_timestamp_delta(total, None)
}

fn absolute_intervals_csv(
    input: &str,
    now: DateTime<Utc>,
    operation: IntervalOperation,
) -> Result<String, TimestampError> {
    let intervals = parse_absolute_intervals(input, now)?;
    let intervals = match operation {
        IntervalOperation::Merge => merge_absolute_intervals(intervals),
        IntervalOperation::Intersection => intervals
            .into_iter()
            .reduce(|(start, end), (other_start, other_end)| {
                (start.max(other_start), end.min(other_end))
            })
            .filter(|(start, end)| start < end)
            .into_iter()
            .collect(),
        IntervalOperation::Difference => subtract_absolute_intervals(intervals),
    };
    let mut result = String::from("开始UTC,结束UTC,时长");
    for (start, end) in intervals {
        let duration = format_absolute_elapsed(start, end)?;
        result.push_str(&format!(
            "\r\n{},{},{duration}",
            format_utc_instant(start),
            format_utc_instant(end),
        ));
    }
    Ok(result)
}

fn merge_absolute_intervals(mut intervals: Vec<AbsoluteInterval>) -> Vec<AbsoluteInterval> {
    intervals.sort_unstable();
    let mut merged: Vec<AbsoluteInterval> = Vec::new();
    for (start, end) in intervals {
        if let Some((_, merged_end)) = merged
            .last_mut()
            .filter(|(_, merged_end)| start <= *merged_end)
        {
            *merged_end = (*merged_end).max(end);
        } else {
            merged.push((start, end));
        }
    }
    merged
}

fn subtract_absolute_intervals(mut intervals: Vec<AbsoluteInterval>) -> Vec<AbsoluteInterval> {
    let (source, cuts) = intervals.split_first_mut().expect("已验证至少有一段时间");
    let (mut start, end) = *source;
    cuts.sort_unstable();
    let mut remaining = Vec::new();
    for &(cut_start, cut_end) in cuts.iter() {
        if cut_start >= end {
            break;
        }
        if start < cut_start {
            remaining.push((start, cut_start));
        }
        start = start.max(cut_end);
        if start >= end {
            break;
        }
    }
    if start < end {
        remaining.push((start, end));
    }
    remaining
}

fn interval_nanos(
    input: &str,
    now: DateTime<Utc>,
    invalid: TimestampError,
) -> Result<(i128, i128), TimestampError> {
    let (start, end) = input.split_once(['到', '至']).ok_or(invalid)?;
    let timeline_nanos = |value: &str| {
        let value = value.trim();
        if value.is_empty() {
            return Err(invalid);
        }
        interval_endpoint(value, Some(now)).map(unix_nanos)
    };
    Ok((timeline_nanos(start)?, timeline_nanos(end)?))
}

fn daily_durations(input: &str, now: DateTime<Utc>) -> Result<String, TimestampError> {
    const SECONDS_PER_DAY: i128 = 86_400;
    const NANOS_PER_DAY: i128 = SECONDS_PER_DAY * NANOS_PER_SECOND as i128;
    let invalid = TimestampError::DailyDurationArguments;
    let (input, zone) = split_conversion(input).ok_or(invalid)?;
    let zone = explicit_timezone(zone, invalid)?;
    let (start, end) = interval_nanos(input, now, invalid)?;
    if end < start {
        return Err(invalid);
    }
    let mut output = String::from("当地日期,时长");
    if start == end {
        return Ok(output);
    }
    let timestamp =
        |nanos| Timestamp::from_nanosecond(nanos).map_err(|_| TimestampError::OutOfRange);
    let first = timestamp(start)?;
    let last = timestamp(end)?;
    let mut totals = BTreeMap::<i128, u128>::new();
    let mut offset_start = first;
    let mut cursor = start;
    // 先沿 UTC 时间线按偏移变化切分，再在固定偏移的各段内按当地午夜切分。
    for offset_end in zone
        .following(first)
        .map(|transition| transition.timestamp())
        .take_while(|transition| *transition < last)
        .chain(std::iter::once(last))
    {
        let next_transition = offset_end.as_nanosecond();
        let offset = i128::from(zone.to_offset(offset_start).seconds()) * NANOS_PER_SECOND as i128;
        while cursor < next_transition {
            let day = (cursor + offset).div_euclid(NANOS_PER_DAY);
            let midnight = (day + 1) * NANOS_PER_DAY - offset;
            let stop = next_transition.min(midnight);
            *totals.entry(day).or_default() += (stop - cursor) as u128;
            if totals.len() > MAX_DAILY_DURATION_ROWS {
                return Err(TimestampError::TooManyDurationDays);
            }
            cursor = stop;
        }
        offset_start = offset_end;
    }
    for (day, nanos) in totals {
        let midnight = DateTime::from_timestamp((day * SECONDS_PER_DAY) as i64, 0)
            .ok_or(TimestampError::OutOfRange)?;
        ensure_four_digit_year(midnight.year())?;
        let date = midnight.date_naive();
        let duration = format_clock_duration(nanos, "");
        output.push_str(&format!("\r\n{date},{duration}"));
    }
    Ok(output)
}

pub(super) fn timestamp_clock_difference(start: &str, end: &str) -> Result<String, TimestampError> {
    format_absolute_elapsed(absolute_timestamp(start)?, absolute_timestamp(end)?)
}

pub(super) fn format_absolute_elapsed(
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<String, TimestampError> {
    format_timestamp_delta(absolute_elapsed_nanoseconds(start, end), None)
}

pub(super) fn absolute_elapsed_nanoseconds(start: DateTime<Utc>, end: DateTime<Utc>) -> i128 {
    unix_nanos(end) - unix_nanos(start)
}

pub(super) fn absolute_timestamp(source: &str) -> Result<DateTime<Utc>, TimestampError> {
    interval_endpoint(normalize(source).trim(), None)
}

pub(super) fn timestamp_zone_converter(
    target: &str,
) -> Result<impl Fn(&str) -> Result<String, TimestampError>, TimestampError> {
    let zone = parse_display_zone(&normalize(target))?;
    Ok(move |source: &str| {
        let time = absolute_timestamp(source)?;
        let unit = TimestampUnit::from_subsecond(time.timestamp_subsec_nanos());
        format_display_time(time, unit.date_format(), &zone, &|time| {
            time.with_timezone(&Local).fixed_offset()
        })
    })
}

pub(super) fn timestamp_alignment_converter(
    step: &str,
    direction: &str,
) -> Result<impl Fn(&str) -> Result<String, TimestampError>, TimestampError> {
    let step = timestamp_alignment_step(&normalize(step))?;
    let direction = match normalize(direction).trim().to_ascii_lowercase().as_str() {
        "向下" | "floor" => TimestampAlignment::Floor,
        "向上" | "ceil" => TimestampAlignment::Ceil,
        "就近" | "nearest" => TimestampAlignment::Nearest,
        _ => return Err(TimestampError::AlignmentArguments),
    };
    Ok(move |input: &str| {
        let time = align_instant(absolute_timestamp(input)?, step, direction)?;
        Ok(format_utc_instant(time))
    })
}

fn format_utc_instant(time: DateTime<Utc>) -> String {
    let unit = TimestampUnit::from_subsecond(time.timestamp_subsec_nanos());
    time.to_rfc3339_opts(unit.date_format(), true)
}

fn format_timestamp_delta(delta: i128, output: Option<&str>) -> Result<String, TimestampError> {
    let sign = if delta.is_negative() { "-" } else { "" };
    if let Some(unit) = output {
        return format_duration_unit(delta.unsigned_abs(), sign, &unit.to_ascii_lowercase())
            .map_err(TimestampError::IntervalOutput);
    }
    Ok(format_clock_duration(delta.unsigned_abs(), sign))
}

fn interval_endpoint(
    value: &str,
    now: Option<DateTime<Utc>>,
) -> Result<DateTime<Utc>, TimestampError> {
    let lower = value.to_ascii_lowercase();
    let (unit, quantity) =
        TimestampUnit::split_suffix(&lower).unwrap_or((TimestampUnit::Seconds, &lower));
    let time = if matches!(lower.as_str(), "现在" | "now") {
        now.ok_or(TimestampError::Invalid)?
    } else if unix_number(quantity.trim_end()) {
        parse_unix(quantity.trim_end(), unit)?
    } else {
        return parse_explicit_time(value, named_time_source(value));
    };
    ensure_four_digit_year(time.year())?;
    if time.timestamp_subsec_nanos() >= NANOS_PER_SECOND as u32 {
        return Err(TimestampError::UnsupportedPrecision);
    }
    Ok(time)
}

fn parse_named_source(clock: &str, name: &str) -> Result<DateTime<Utc>, TimestampError> {
    let zone = TimeZone::get(name).map_err(|_| TimestampError::InvalidSourceZone)?;
    let unresolved = zone.to_ambiguous_timestamp(parse_named_clock(clock)?);
    match unresolved.offset() {
        AmbiguousOffset::Gap { .. } => return Err(TimestampError::SourceClockGap),
        AmbiguousOffset::Fold { .. } => return Err(TimestampError::SourceClockFold),
        AmbiguousOffset::Unambiguous { .. } => {}
    }
    let instant = unresolved
        .unambiguous()
        .map_err(|_| TimestampError::OutOfRange)?;
    utc_from_timestamp(instant)
}

fn parse_named_clock(clock: &str) -> Result<CivilDateTime, TimestampError> {
    let (date, time) = clock
        .split_once(|ch: char| matches!(ch, 'T' | 't') || ch.is_whitespace())
        .ok_or(TimestampError::Invalid)?;
    let date = NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|_| TimestampError::Invalid)?;
    ensure_four_digit_year(date.year())?;
    let time = time.trim();
    let time = NaiveTime::parse_from_str(time, "%H:%M:%S%.f")
        .or_else(|_| NaiveTime::parse_from_str(time, "%H:%M"))
        .map_err(|_| TimestampError::Invalid)?;
    if time.nanosecond() >= 1_000_000_000 {
        return Err(TimestampError::UnsupportedPrecision);
    }
    CivilDateTime::new(
        date.year() as i16,
        date.month() as i8,
        date.day() as i8,
        time.hour() as i8,
        time.minute() as i8,
        time.second() as i8,
        time.nanosecond() as i32,
    )
    .map_err(|_| TimestampError::OutOfRange)
}

fn utc_from_timestamp(instant: Timestamp) -> Result<DateTime<Utc>, TimestampError> {
    let nanos = instant.as_nanosecond();
    let seconds =
        i64::try_from(nanos.div_euclid(1_000_000_000)).map_err(|_| TimestampError::OutOfRange)?;
    DateTime::from_timestamp(seconds, nanos.rem_euclid(1_000_000_000) as u32)
        .ok_or(TimestampError::OutOfRange)
}

fn parse_offset_time(input: &str) -> Result<DateTime<Utc>, TimestampError> {
    if let Ok(value) = DateTime::parse_from_rfc3339(input) {
        return Ok(value.with_timezone(&Utc));
    }
    let (clock, offset) = input
        .len()
        .checked_sub(9)
        .and_then(|index| input.split_at_checked(index))
        .filter(|(_, offset)| offset.starts_with(['+', '-']))
        .ok_or(TimestampError::Invalid)?;
    let DisplayZone::Offset(offset) =
        parse_display_zone(offset).map_err(|_| TimestampError::Invalid)?
    else {
        return Err(TimestampError::Invalid);
    };
    // 仍用 RFC 3339 解析日期和钟表时间，秒级偏移单独应用。
    let clock =
        DateTime::parse_from_rfc3339(&format!("{clock}Z")).map_err(|_| TimestampError::Invalid)?;
    clock
        .naive_utc()
        .and_local_timezone(offset)
        .single()
        .map(|date| date.with_timezone(&Utc))
        .ok_or(TimestampError::OutOfRange)
}

fn parse_display_targets(input: &str) -> Result<DisplayTargets<'_>, TimestampError> {
    if !input.contains([',', '、']) {
        let unit = input.trim().to_ascii_lowercase();
        if let Some((unit, "")) = TimestampUnit::split_suffix(&unit) {
            return Ok(DisplayTargets::Epoch(unit));
        }
        return parse_display_zone(input).map(DisplayTargets::Single);
    }
    parse_named_display_targets(input).map(DisplayTargets::Multiple)
}

fn parse_named_display_targets(input: &str) -> Result<Vec<(&str, DisplayZone)>, TimestampError> {
    let zones = input
        .split([',', '、'])
        .map(str::trim)
        .take(MAX_TARGET_ZONES + 1)
        .collect::<Vec<_>>();
    if zones.len() > MAX_TARGET_ZONES {
        return Err(TimestampError::TooManyTargetZones);
    }
    zones
        .into_iter()
        .map(|name| Ok((name, parse_display_zone(name)?)))
        .collect()
}

fn parse_display_zone(input: &str) -> Result<DisplayZone, TimestampError> {
    let original = input.trim();
    let input = original.to_ascii_lowercase();
    match input.as_str() {
        "" => return Err(TimestampError::Incomplete),
        "utc" | "z" => return Ok(DisplayZone::Utc),
        "local" | "本地" => return Ok(DisplayZone::Local),
        _ => {}
    }
    let input = input.strip_prefix("utc").unwrap_or(&input).trim_start();
    let (sign, magnitude) = match input.as_bytes() {
        [b'+', ..] => (1, &input[1..]),
        [b'-', ..] => (-1, &input[1..]),
        _ => {
            return TimeZone::get(original)
                .map(DisplayZone::Named)
                .map_err(|_| TimestampError::InvalidOffset);
        }
    };
    if !magnitude.is_ascii() {
        return Err(TimestampError::InvalidOffset);
    }
    let (hours, minutes, seconds) = match magnitude.as_bytes() {
        [_] | [_, _] => (magnitude, "00", "00"),
        [_, _, _, _] => (&magnitude[..2], &magnitude[2..], "00"),
        [_, _, b':', _, _] => (&magnitude[..2], &magnitude[3..], "00"),
        [_, _, b':', _, _, b':', _, _] => (&magnitude[..2], &magnitude[3..5], &magnitude[6..]),
        _ => return Err(TimestampError::InvalidOffset),
    };
    let component = |digits: &str| {
        if digits.bytes().all(|byte| byte.is_ascii_digit()) {
            digits
                .parse::<i32>()
                .map_err(|_| TimestampError::InvalidOffset)
        } else {
            Err(TimestampError::InvalidOffset)
        }
    };
    let hours = component(hours)?;
    let minutes = component(minutes)?;
    let seconds = component(seconds)?;
    if minutes >= 60 || seconds >= 60 {
        return Err(TimestampError::InvalidOffset);
    }
    FixedOffset::east_opt(sign * (hours * 3_600 + minutes * 60 + seconds))
        .map(DisplayZone::Offset)
        .ok_or(TimestampError::InvalidOffset)
}

fn integer(input: &str) -> bool {
    let digits = input.strip_prefix(['-', '+']).unwrap_or(input);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

fn unix_number(input: &str) -> bool {
    integer(input) || unix_decimal_parts(input).is_some()
}

fn unix_decimal_parts(input: &str) -> Option<(&str, &str)> {
    let (whole, fraction) = input.split_once('.')?;
    ((integer(whole) || matches!(whole, "" | "+" | "-"))
        && !fraction.is_empty()
        && fraction.bytes().all(|byte| byte.is_ascii_digit()))
    .then_some((whole, fraction))
}

fn parse_unix(input: &str, unit: TimestampUnit) -> Result<DateTime<Utc>, TimestampError> {
    if let Some((whole, fraction)) = unix_decimal_parts(input) {
        let scale = unit.nanos_per_unit();
        let digits = scale.ilog10() as usize;
        let (kept, excess) = fraction.split_at(digits.min(fraction.len()));
        if excess.bytes().any(|digit| digit != b'0') {
            return Err(TimestampError::UnsupportedPrecision);
        }
        let parse_digits = |digits: &str| {
            if digits.is_empty() {
                Ok(0)
            } else {
                digits
                    .parse::<i128>()
                    .map_err(|_| TimestampError::OutOfRange)
            }
        };
        let base = parse_digits(whole.trim_start_matches(['+', '-']))?;
        let remainder = parse_digits(kept)? * (scale / 10_i128.pow(kept.len() as u32));
        let nanos = base
            .checked_mul(scale)
            .and_then(|base| base.checked_add(remainder))
            .ok_or(TimestampError::OutOfRange)?;
        let nanos = if whole.starts_with('-') {
            -nanos
        } else {
            nanos
        };
        return timestamp_from_unix_units(nanos, TimestampUnit::Nanoseconds);
    }
    if !integer(input) {
        return Err(TimestampError::Invalid);
    }
    let value: i128 = input.parse().map_err(|_| TimestampError::OutOfRange)?;
    timestamp_from_unix_units(value, unit)
}

fn timestamp_from_unix_units(
    value: i128,
    unit: TimestampUnit,
) -> Result<DateTime<Utc>, TimestampError> {
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
    fn reads_fractional_unix_timestamps_without_rounding_or_losing_the_sign() {
        let now = DateTime::UNIX_EPOCH;
        for (input, expected) in [
            ("-0.000001 ms", "1969-12-31T23:59:59.999999999Z"),
            ("1.5ms", "1970-01-01T00:00:00.001500Z"),
            ("+.5s", "1970-01-01T00:00:00.500Z"),
            ("1.0000ns to ns", "1 ns"),
            ("间隔 -.5s 到 0.5s to ms", "1000 ms"),
        ] {
            assert_eq!(
                convert_timestamp(input, now).as_deref(),
                Ok(expected),
                "{input}"
            );
        }
        assert_eq!(
            convert_timestamp("0.0000000001s", now),
            Err(TimestampError::UnsupportedPrecision),
        );
    }

    #[test]
    fn converts_to_unix_units_with_exact_fractional_and_negative_values() {
        let now = DateTime::from_timestamp(0, 1_234_567).unwrap();
        for (input, expected) in [
            ("-1ns to ms", "-0.000001 ms"),
            ("1970-01-01T08:00:00.123456789+08:00 to ns", "123456789 ns"),
            ("1500ms to s", "1.5 s"),
            ("1ms to 微秒", "1000 µs"),
            ("现在 to ms", "1.234567 ms"),
            ("序列 0 @ 1ns @ 2 to ns", "0 ns\n1 ns"),
        ] {
            assert_eq!(
                convert_timestamp(input, now).as_deref(),
                Ok(expected),
                "{input}"
            );
        }
    }

    #[test]
    fn repeats_a_local_clock_time_by_date_and_preserves_gap_and_fold_information() {
        let now = DateTime::UNIX_EPOCH;
        assert_eq!(
            convert_timestamp("每日同刻 2026-03-07 02:30 America/New_York @ 3", now).as_deref(),
            Ok("当地时间,UTC,状态\r\n2026-03-07T02:30:00,2026-03-07T07:30:00Z,正常\r\n2026-03-08T02:30:00,\"\",跳过\r\n2026-03-09T02:30:00,2026-03-09T06:30:00Z,正常"),
        );
        assert_eq!(
            convert_timestamp("daily-clock 2026-11-01 01:30 America/New_York @ 1", now).as_deref(),
            Ok("当地时间,UTC,状态\r\n2026-11-01T01:30:00,2026-11-01T05:30:00Z,第一次\r\n2026-11-01T01:30:00,2026-11-01T06:30:00Z,第二次"),
        );
        assert_eq!(
            convert_timestamp(
                "每日同刻 2026-03-08 02:30:00.0000000001 America/New_York @ 1",
                now
            ),
            Err(TimestampError::UnsupportedPrecision),
        );
    }

    #[test]
    fn lists_utc_offset_changes_in_the_requested_utc_year() {
        let now = DateTime::UNIX_EPOCH;
        assert_eq!(
            convert_timestamp("偏移变更 2026 America/New_York", now).as_deref(),
            Ok("生效UTC,变更前偏移,变更后偏移,钟表变化\r\n2026-03-08T07:00:00Z,-05,-04,+01:00:00\r\n2026-11-01T06:00:00Z,-04,-05,-01:00:00"),
        );
        assert_eq!(
            convert_timestamp("offset-changes 2026 Asia/Shanghai", now).as_deref(),
            Ok("生效UTC,变更前偏移,变更后偏移,钟表变化"),
        );
        assert_eq!(
            convert_timestamp("偏移变更 26 UTC", now),
            Err(TimestampError::OffsetChangesArguments),
        );
    }

    #[test]
    fn merges_and_intersects_half_open_intervals_on_the_utc_timeline() {
        let now = DateTime::UNIX_EPOCH;
        let query = concat!(
            r#"区间交集 [["2026-03-08T01:30:00-05:00","2026-03-08T04:00:00-04:00"],"#,
            r#"["2026-03-08T07:00:00Z","2026-03-08T08:15:00Z"]]"#,
        );
        assert!(is_timestamp_report(query));
        assert_eq!(
            convert_timestamp(query, now).as_deref(),
            Ok("开始UTC,结束UTC,时长\r\n2026-03-08T07:00:00Z,2026-03-08T08:00:00Z,01:00:00")
        );
        assert_eq!(
            convert_timestamp(
                r#"MERGE-INTERVALS [["5s","6s"],["现在","1s"],["-1s","0s"]]"#,
                now
            )
            .as_deref(),
            Ok(concat!(
                "开始UTC,结束UTC,时长\r\n",
                "1969-12-31T23:59:59Z,1970-01-01T00:00:01Z,00:00:02\r\n",
                "1970-01-01T00:00:05Z,1970-01-01T00:00:06Z,00:00:01",
            ))
        );
        assert_eq!(
            convert_timestamp(r#"区间交集 [["0s","1ns"],["1ns","2ns"]]"#, now).as_deref(),
            Ok("开始UTC,结束UTC,时长")
        );
        assert_eq!(
            convert_timestamp(r#"区间合并 [["1ns","0s"]]"#, now),
            Err(TimestampError::AbsoluteIntervalOrder)
        );
    }

    #[test]
    fn sums_duration_after_merging_overlapping_absolute_intervals() {
        let now = DateTime::UNIX_EPOCH;
        let query = r#"覆盖时长 [["0s","10s"],["5s","15s"],["20s","26s"]]"#;
        assert_eq!(convert_timestamp(query, now).as_deref(), Ok("00:00:21"));
        assert_eq!(
            convert_timestamp(
                r#"COVERED-DURATION [["10ns","12ns"],["-1ns","2ns"],["现在","3ns"],["3ns","4ns"]]"#,
                now,
            )
            .as_deref(),
            Ok("00:00:00.000000007"),
        );
        assert_eq!(
            convert_timestamp(r#"覆盖时长 [["0s","0s"]]"#, now),
            Err(TimestampError::AbsoluteIntervalOrder),
        );
        assert_eq!(
            convert_timestamp("覆盖时长 []", now),
            Err(TimestampError::AbsoluteIntervalsArguments),
        );
    }

    #[test]
    fn subtracts_overlapping_and_touching_cuts_from_one_absolute_interval() {
        let now = DateTime::UNIX_EPOCH;
        let query = concat!(
            r#"区间差集 [["现在","20s"],["10s","25s"],["2s","4s"],"#,
            r#"["3s","5s"],["-10s","0s"]]"#,
        );
        assert!(is_timestamp_report(query));
        assert_eq!(
            convert_timestamp(query, now).as_deref(),
            Ok(concat!(
                "开始UTC,结束UTC,时长\r\n",
                "1970-01-01T00:00:00Z,1970-01-01T00:00:02Z,00:00:02\r\n",
                "1970-01-01T00:00:05Z,1970-01-01T00:00:10Z,00:00:05",
            )),
        );
        assert_eq!(
            convert_timestamp(r#"subtract-intervals [["0s","1ns"],["-1ns","1ns"]]"#, now)
                .as_deref(),
            Ok("开始UTC,结束UTC,时长")
        );
        assert_eq!(
            convert_timestamp(r#"区间差集 [["0s","1ns"],["1ns","2ns"]]"#, now).as_deref(),
            Ok("开始UTC,结束UTC,时长\r\n1970-01-01T00:00:00Z,1970-01-01T00:00:00.000000001Z,00:00:00.000000001")
        );
    }

    #[test]
    fn partitions_a_local_date_into_all_of_its_utc_intervals() {
        for (query, expected) in [
            ("2026-03-08 America/New_York", "\r\n2026-03-08T05:00:00Z,2026-03-09T04:00:00Z,23:00:00"),
            ("2026-03-07 到 2026-03-09 America/New_York", "\r\n2026-03-07T05:00:00Z,2026-03-10T04:00:00Z,71:00:00"),
            ("2026-11-01 America/New_York", "\r\n2026-11-01T04:00:00Z,2026-11-02T05:00:00Z,25:00:00"),
            ("2024-02-29 UTC+8", "\r\n2024-02-28T16:00:00Z,2024-02-29T16:00:00Z,24:00:00"),
            ("1987-10-25 America/St_Johns", "\r\n1987-10-25T02:30:00Z,1987-10-25T02:31:00Z,00:01:00\r\n1987-10-25T03:30:00Z,1987-10-26T03:30:00Z,24:00:00"),
            ("2011-12-30 Pacific/Apia", ""),
        ] {
            assert_eq!(
                convert_timestamp(&format!("日区间 {query}"), DateTime::UNIX_EPOCH).as_deref(),
                Ok(format!("开始UTC,结束UTC,时长{expected}").as_str()),
                "{query}",
            );
        }
        assert_eq!(
            convert_timestamp("日区间 2023-02-29 UTC", DateTime::UNIX_EPOCH),
            Err(TimestampError::CivilDayArguments),
        );
        assert_eq!(
            convert_timestamp("日区间 2026-03-09 至 2026-03-07 UTC", DateTime::UNIX_EPOCH),
            Err(TimestampError::CivilDayArguments),
        );
    }

    #[test]
    fn previews_full_local_calendar_periods_on_the_utc_timeline() {
        let now = DateTime::UNIX_EPOCH;
        for (query, expected) in [
            (
                "周区间 2026-W10 America/New_York",
                "2026-03-02T05:00:00Z,2026-03-09T04:00:00Z,167:00:00",
            ),
            (
                "week-intervals 2020-w53 UTC",
                "2020-12-28T00:00:00Z,2021-01-04T00:00:00Z,168:00:00",
            ),
            (
                "月区间 2026-03 America/New_York",
                "2026-03-01T05:00:00Z,2026-04-01T04:00:00Z,743:00:00",
            ),
            (
                "month-intervals 2024-02 UTC",
                "2024-02-01T00:00:00Z,2024-03-01T00:00:00Z,696:00:00",
            ),
            (
                "季度区间 2026-Q1 America/New_York",
                "2026-01-01T05:00:00Z,2026-04-01T04:00:00Z,2159:00:00",
            ),
            (
                "quarter-intervals 2026-q4 America/New_York",
                "2026-10-01T04:00:00Z,2027-01-01T05:00:00Z,2209:00:00",
            ),
            (
                "年区间 2024 UTC",
                "2024-01-01T00:00:00Z,2025-01-01T00:00:00Z,8784:00:00",
            ),
        ] {
            assert!(is_timestamp_report(query));
            assert_eq!(
                convert_timestamp(query, now),
                Ok(format!("开始UTC,结束UTC,时长\r\n{expected}")),
                "{query}",
            );
        }
        assert_eq!(
            convert_timestamp("周区间 2021-W53 UTC", now),
            Err(TimestampError::CivilPeriodArguments),
        );
        assert_eq!(
            convert_timestamp("月区间 2026-13 UTC", now),
            Err(TimestampError::CivilPeriodArguments),
        );
        for query in ["季度区间 2026-Q5 UTC", "年区间 2024-02 UTC"] {
            assert_eq!(
                convert_timestamp(query, now),
                Err(TimestampError::CivilPeriodArguments),
                "{query}",
            );
        }
    }

    #[test]
    fn splits_elapsed_time_by_local_date_even_across_offset_changes_and_midnight() {
        for (query, rows) in [
            (
                "逐日时长 2026-03-07T14:00:00-05:00 到 2026-03-09T10:00:00-04:00 to America/New_York",
                "\r\n2026-03-07,10:00:00\r\n2026-03-08,23:00:00\r\n2026-03-09,10:00:00",
            ),
            (
                "daily-durations 2026-11-01T00:00:00-04:00 至 2026-11-02T00:00:00-05:00 to America/New_York",
                "\r\n2026-11-01,25:00:00",
            ),
            (
                "逐日时长 2011-12-29T23:00:00-10:00 到 2011-12-31T01:00:00+14:00 to Pacific/Apia",
                "\r\n2011-12-29,01:00:00\r\n2011-12-31,01:00:00",
            ),
            (
                "逐日时长 -1ns 到 1ns to UTC",
                "\r\n1969-12-31,00:00:00.000000001\r\n1970-01-01,00:00:00.000000001",
            ),
            ("逐日时长 0 到 0 to UTC+8", ""),
        ] {
            assert!(is_timestamp_report(query));
            let expected = format!("当地日期,时长{rows}");
            assert_eq!(convert_timestamp(query, DateTime::UNIX_EPOCH), Ok(expected), "{query}");
        }
        assert_eq!(
            convert_timestamp("逐日时长 1 到 0 to UTC", DateTime::UNIX_EPOCH),
            Err(TimestampError::DailyDurationArguments)
        );
        assert_eq!(
            convert_timestamp(
                "逐日时长 0 到 2026-01-01T00:00:00Z to UTC",
                DateTime::UNIX_EPOCH
            ),
            Err(TimestampError::TooManyDurationDays)
        );
    }

    #[test]
    fn generates_timestamps_at_equal_elapsed_steps_with_consistent_precision() {
        let now = DateTime::from_timestamp(1000, 123_000_000).unwrap();
        for (query, expected) in [
            (
                "序列 0 @ 15分钟 @ 3 to UTC+8",
                "1970-01-01T08:00:00+08:00\n1970-01-01T08:15:00+08:00\n1970-01-01T08:30:00+08:00",
            ),
            (
                "timestamp-series -1ns @ 1ns @ 3",
                "1969-12-31T23:59:59.999999999Z\n1970-01-01T00:00:00.000000000Z\n1970-01-01T00:00:00.000000001Z",
            ),
            ("序列 现在 @ 1秒 @ 1", "1970-01-01T00:16:40.123Z"),
        ] {
            assert_eq!(convert_timestamp(query, now).as_deref(), Ok(expected), "{query}");
        }
        for query in [
            "序列 0 @ 0秒 @ 2",
            "序列 0 @ 1秒 @ 0",
            "序列 0 @ 1秒 @ 129",
            "序列 0 @ 1秒 @ 2 to UTC,Asia/Shanghai",
        ] {
            assert_eq!(
                convert_timestamp(query, now),
                Err(TimestampError::SeriesArguments)
            );
        }
        assert_eq!(
            convert_timestamp("序列 9999-12-31T23:59:59Z @ 1秒 @ 2", now),
            Err(TimestampError::OutOfRange)
        );
    }

    #[test]
    fn aligns_instants_to_epoch_steps_with_exact_negative_and_subsecond_values() {
        let now = DateTime::from_timestamp(1000, 250_000_000).unwrap();
        for (query, expected) in [
            (
                "向下对齐 15分钟 @ 1970-01-01T00:17:30Z",
                "1970-01-01T00:15:00Z",
            ),
            (
                "向上对齐 15分钟 @ 1970-01-01T00:17:30Z",
                "1970-01-01T00:30:00Z",
            ),
            ("floor-time 1秒 @ -1ns", "1969-12-31T23:59:59Z"),
            ("ceil-time 1秒 @ -1ns", "1970-01-01T00:00:00Z"),
            (
                "就近对齐 15分钟 @ 1970-01-01T00:22:29Z",
                "1970-01-01T00:15:00Z",
            ),
            (
                "就近对齐 15分钟 @ 1970-01-01T00:22:30Z",
                "1970-01-01T00:30:00Z",
            ),
            ("nearest-time 1秒 @ -501ms", "1969-12-31T23:59:59Z"),
            ("nearest-time 1秒 @ -500ms", "1970-01-01T00:00:00Z"),
            ("nearest-time 2ns @ 2ns", "1970-01-01T00:00:00.000000002Z"),
            ("向上对齐 100ms @ 现在", "1970-01-01T00:16:40.300Z"),
            ("向下对齐 2ns @ 2ns", "1970-01-01T00:00:00.000000002Z"),
            (
                "向上对齐 15分钟 @ 2026-03-08 01:59:59 America/New_York to America/New_York",
                "2026-03-08T03:00:00-04:00",
            ),
        ] {
            assert_eq!(
                convert_timestamp(query, now).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert_eq!(
            convert_timestamp("向下对齐 0秒 @ 1000", now),
            Err(TimestampError::AlignmentArguments)
        );
    }

    #[test]
    fn accepts_unix_quantities_and_the_supplied_current_time_as_interval_endpoints() {
        let now = DateTime::from_timestamp(1000, 250_000_000).unwrap();
        for (query, expected) in [
            ("间隔 现在 到 1500 to s", "499.75 s"),
            ("between -1ns 至 +1NS to ns", "2 ns"),
            ("间隔 -1毫秒 到 1970-01-01T00:00:00Z to ms", "1 ms"),
            ("间隔 now 到 1970-01-01T00:16:40.25Z", "00:00:00"),
        ] {
            assert_eq!(
                convert_timestamp(query, now).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert_eq!(
            convert_timestamp("间隔 0 到 9999999999999999999999999999999999999999ns", now),
            Err(TimestampError::OutOfRange)
        );
    }

    #[test]
    fn calculates_intervals_on_the_utc_timeline_with_nanosecond_precision() {
        let now = DateTime::UNIX_EPOCH;
        for (query, expected) in [
            (
                "间隔 2026-03-08 01:30 America/New_York 到 2026-03-08 03:30 America/New_York",
                "01:00:00",
            ),
            (
                "between 2026-11-01T01:30:00-04:00 至 2026-11-01T01:30:00-05:00",
                "01:00:00",
            ),
            (
                "间隔 1970-01-01T00:00:00Z 到 1969-12-31T23:59:59.999999999Z",
                "-00:00:00.000000001",
            ),
            (
                "间隔 1900-01-01T00:00:00Z 至 2300-01-01T00:00:00Z",
                "3506328:00:00",
            ),
            (
                "间隔 1970-01-01T00:00:00Z 到 1970-01-02T00:00:02.5Z to iso",
                "P1DT2.5S",
            ),
            (
                "间隔 1970-01-01T00:00:00.5Z 到 1970-01-01T00:00:00Z TO MS",
                "-500 ms",
            ),
        ] {
            assert_eq!(
                convert_timestamp(query, now).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        for (query, error) in [
            ("间隔", TimestampError::IntervalEndpoints),
            (
                "间隔 到 2026-01-01T00:00:00Z",
                TimestampError::IntervalEndpoints,
            ),
            (
                "间隔 1970-01-01T00:00:00Z 到 1970-01-02T00:00:00Z to bytes",
                TimestampError::IntervalOutput(DurationError::InvalidUnit),
            ),
            (
                "间隔 2026-03-08 02:30 America/New_York 到 2026-03-08T08:00:00Z",
                TimestampError::SourceClockGap,
            ),
            (
                "间隔 2026-11-01 01:30 America/New_York 到 2026-11-01T08:00:00Z",
                TimestampError::SourceClockFold,
            ),
            (
                "间隔 1970-01-01T00:00:00.0000000001Z 到 1970-01-01T00:00:00Z",
                TimestampError::UnsupportedPrecision,
            ),
        ] {
            assert_eq!(convert_timestamp(query, now), Err(error), "{query}");
        }
    }

    #[test]
    fn exports_same_instant_to_named_time_zone_rows_with_exact_offset_and_precision() {
        let now = DateTime::from_timestamp(0, 0).unwrap();
        assert_eq!(
            convert_timestamp(
                "时区CSV 2026-07-01 09:00 America/New_York to UTC, Asia/Shanghai, America/Los_Angeles",
                now,
            )
            .as_deref(),
            Ok(concat!(
                "时区,时间\r\n",
                "UTC,2026-07-01T13:00:00Z\r\n",
                "Asia/Shanghai,2026-07-01T21:00:00+08:00\r\n",
                "America/Los_Angeles,2026-07-01T06:00:00-07:00",
            ))
        );
        assert_eq!(
            convert_timestamp("zone-csv 现在 + 1ns to UTC", now).as_deref(),
            Ok("时区,时间\r\nUTC,1970-01-01T00:00:00.000000001Z")
        );
        assert_eq!(
            convert_timestamp("时区CSV 0", now),
            Err(TimestampError::ZoneTableArguments)
        );
    }

    #[test]
    fn converts_one_instant_to_several_ordered_time_zones() {
        let now = DateTime::from_timestamp(0, 0).unwrap();
        assert_eq!(
            convert_timestamp("2026-07-01 09:00 America/New_York to UTC, Asia/Shanghai, America/Los_Angeles", now)
                .as_deref(),
            Ok("UTC\n2026-07-01T13:00:00Z\nAsia/Shanghai\n2026-07-01T21:00:00+08:00\nAmerica/Los_Angeles\n2026-07-01T06:00:00-07:00")
        );
        let offset = FixedOffset::east_opt(8 * 3_600).unwrap();
        assert_eq!(
            convert_timestamp_with_local("现在 + 0.5s to 本地、UTC", now, |time| time
                .with_timezone(&offset))
            .as_deref(),
            Ok("本地\n1970-01-01T08:00:00.500+08:00\nUTC\n1970-01-01T00:00:00.500Z")
        );
        assert_eq!(
            convert_timestamp("0 to UTC, nowhere", now),
            Err(TimestampError::InvalidOffset)
        );
        assert_eq!(
            convert_timestamp("0 to UTC,UTC,UTC,UTC,UTC,UTC,UTC,UTC,UTC", now),
            Err(TimestampError::TooManyTargetZones)
        );
    }

    #[test]
    fn lists_both_dst_occurrences_with_explicit_offsets_and_rejects_gaps() {
        let now = DateTime::from_timestamp(0, 0).unwrap();
        for (query, expected) in [
            (
                "时区候选 2026-11-01 01:30 America/New_York",
                "2026-11-01T01:30:00-04:00\n2026-11-01T01:30:00-05:00",
            ),
            (
                "zone-candidates 2026-11-01 01:30:00.123456789 America/New_York",
                "2026-11-01T01:30:00.123456789-04:00\n2026-11-01T01:30:00.123456789-05:00",
            ),
            (
                "时区候选 2026-07-01 09:00 Asia/Shanghai",
                "2026-07-01T09:00:00+08:00",
            ),
        ] {
            assert_eq!(
                convert_timestamp(query, now).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        for (query, error) in [
            (
                "时区候选 2026-11-01 01:30",
                TimestampError::ZoneCandidateArguments,
            ),
            (
                "时区候选 2026-11-01 01:30 Example/Unknown",
                TimestampError::InvalidSourceZone,
            ),
            (
                "时区候选 2026-03-08 02:30 America/New_York",
                TimestampError::SourceClockGap,
            ),
            (
                "时区候选 2026-11-01 01:30:00.1234567891 America/New_York",
                TimestampError::UnsupportedPrecision,
            ),
            (
                "2026-11-01 01:30 America/New_York to UTC",
                TimestampError::SourceClockFold,
            ),
        ] {
            assert_eq!(convert_timestamp(query, now), Err(error), "{query}");
        }
    }

    #[test]
    fn converts_to_named_time_zones_with_seasonal_and_historical_offsets() {
        let now = DateTime::from_timestamp(0, 0).unwrap();
        for (query, expected) in [
            (
                "2026-03-08T06:59:59Z to America/New_York",
                "2026-03-08T01:59:59-05:00",
            ),
            (
                "2026-03-08T07:00:00Z to america/new_york",
                "2026-03-08T03:00:00-04:00",
            ),
            (
                "1700000000123ms to Asia/Kolkata",
                "2023-11-15T03:43:20.123+05:30",
            ),
            (
                "1900-01-01T00:00:00.123Z to Europe/Paris",
                "1900-01-01T00:09:21.123+00:09:21",
            ),
            ("现在 + 1s to Asia/Shanghai", "1970-01-01T08:00:01+08:00"),
        ] {
            assert_eq!(
                convert_timestamp(query, now).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert_eq!(
            convert_timestamp("0 to Asia/Unknown", now),
            Err(TimestampError::InvalidOffset),
        );
    }

    #[test]
    fn accepts_a_named_source_zone_and_reports_skipped_or_repeated_local_times() {
        let now = DateTime::from_timestamp(0, 0).unwrap();
        for (query, expected) in [
            (
                "2026-07-01 09:00 America/New_York to Asia/Shanghai",
                "2026-07-01T21:00:00+08:00",
            ),
            (
                "2026-11-21T08:45:00.125 America/Los_Angeles to UTC",
                "2026-11-21T16:45:00.125Z",
            ),
            ("1970-01-01 08:00 Asia/Shanghai", "0"),
            ("1969-12-31 23:59:59.999999999 UTC", "-1 ns"),
        ] {
            assert_eq!(
                convert_timestamp(query, now).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        for (query, error) in [
            (
                "2026-03-08 02:30 America/New_York",
                TimestampError::SourceClockGap,
            ),
            (
                "2026-11-01 01:30 America/New_York",
                TimestampError::SourceClockFold,
            ),
            (
                "2026-11-21 09:00 America/Unknown",
                TimestampError::InvalidSourceZone,
            ),
            ("2026-11-21 25:00 Asia/Shanghai", TimestampError::Invalid),
            (
                "2026-11-21 09:00+08:00 Asia/Shanghai",
                TimestampError::Invalid,
            ),
            (
                "2016-12-31 23:59:60 UTC",
                TimestampError::UnsupportedPrecision,
            ),
        ] {
            assert_eq!(convert_timestamp(query, now), Err(error), "{query}");
        }
    }

    #[test]
    fn parses_and_displays_offsets_with_seconds_without_losing_the_instant() {
        let now = DateTime::from_timestamp(0, 0).unwrap();
        for (query, expected) in [
            ("0 to +00:09:21", "1970-01-01T00:09:21+00:09:21"),
            ("0 to -00:00:01", "1969-12-31T23:59:59-00:00:01"),
            (
                "1900-01-01T00:09:21.123+00:09:21 to UTC",
                "1900-01-01T00:00:00.123Z",
            ),
            ("1969-12-31T23:59:59.999999999+00:00:00", "-1 ns"),
        ] {
            assert_eq!(
                convert_timestamp(query, now).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        for query in ["0 to +00:00:60", "0 to +00:60:00", "0 to -24:00:00"] {
            assert_eq!(
                convert_timestamp(query, now),
                Err(TimestampError::InvalidOffset)
            );
        }
        assert_eq!(
            convert_timestamp("1900-01-01T00:00:00+00:09:60", now),
            Err(TimestampError::Invalid),
        );
    }

    #[test]
    fn shifts_current_time_by_a_duration_and_preserves_requested_precision() {
        let now = DateTime::from_timestamp_millis(1_700_000_000_123).unwrap();
        for (input, expected) in [
            ("now + 90m", "1700005400"),
            ("现在 − 250ms", "1699999999873 ms"),
            ("now+1ns to UTC", "2023-11-14T22:13:20.123000001Z"),
            ("NOW + 90m to UTC+8", "2023-11-15T07:43:20+08:00"),
        ] {
            assert_eq!(
                convert_timestamp(input, now).as_deref(),
                Ok(expected),
                "{input}"
            );
        }
        for (input, error) in [
            ("现在 +", TimestampError::Incomplete),
            ("now + abc", TimestampError::InvalidDuration),
            ("now - 0.1ns", TimestampError::UnsupportedPrecision),
            ("now + 4000000天", TimestampError::OutOfRange),
        ] {
            assert_eq!(convert_timestamp(input, now), Err(error), "{input}");
        }
    }

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
            ("0 to UTC+8", "1970-01-01T08:00:00+08:00"),
            ("0 转 ＋０５３０", "1970-01-01T05:30:00+05:30"),
            ("0 to UTC -0330", "1969-12-31T20:30:00-03:30"),
            ("1970-01-01T08:00:00+08:00 to UTC", "1970-01-01T00:00:00Z"),
            (
                "1970-01-01T08:00:00.000000001+08:00 to Z",
                "1970-01-01T00:00:00.000000001Z",
            ),
            ("now ms to utc", "2023-11-14T22:13:20.123Z"),
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
            ("0 to +24", TimestampError::InvalidOffset),
            ("0 to +0060", TimestampError::InvalidOffset),
            ("0 to +080", TimestampError::InvalidOffset),
            ("0 to +0+00", TimestampError::InvalidOffset),
            ("0 to +八", TimestampError::InvalidOffset),
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
    fn displays_local_time_with_an_explicit_target_or_suffix() {
        let now = DateTime::from_timestamp(1_700_000_000, 123_456_000).unwrap();
        let utc_plus_eight = FixedOffset::east_opt(8 * 3_600).unwrap();
        let convert = |query| {
            convert_timestamp_with_local(query, now, |time| time.with_timezone(&utc_plus_eight))
        };
        for (query, expected) in [
            ("+0 本地", "1970-01-01T08:00:00+08:00"),
            ("-1ms LOCAL", "1970-01-01T07:59:59.999+08:00"),
            ("1970-01-01T00:00:00Z 本地", "1970-01-01T08:00:00+08:00"),
            ("1970-01-01T00:00:00Z to local", "1970-01-01T08:00:00+08:00"),
            ("-1 ms 转 本地", "1970-01-01T07:59:59.999+08:00"),
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
