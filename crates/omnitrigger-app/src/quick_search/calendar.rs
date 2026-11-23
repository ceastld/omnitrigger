use std::{
    collections::HashMap,
    fmt::Write as _,
    ops::{Bound, RangeBounds},
};

use chrono::{Datelike, Days, Months, NaiveDate, TimeDelta, Weekday};

use super::calculator::normalize;

const MAX_LISTED_DATES: usize = 366;
const MAX_DATE_RANGES: usize = 32;
const MAX_SPLIT_RANGES: usize = 366;
const WEEKDAY_LABELS: [&str; 7] = [
    "星期一",
    "星期二",
    "星期三",
    "星期四",
    "星期五",
    "星期六",
    "星期日",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DateError {
    Incomplete,
    InvalidDate,
    InvalidMonthCalendar,
    InvalidWeek,
    InvalidQuarter,
    InvalidYearDay,
    InvalidWeekdayOccurrence,
    MissingWeekdayOccurrence,
    InvalidWorkdayOrdinal,
    InvalidWorkdayPosition,
    MissingWorkdayOrdinal,
    InvalidWorkdaySeries,
    InvalidOperation,
    InvalidDateList,
    DateSeriesArguments,
    InvalidDateRanges,
    RangeSplitArguments,
    WeekdayCountArguments,
    InvalidDateInterval,
    WorkdayExceptions,
    PatternArguments,
    InvalidDatePattern,
    DatePatternMismatch,
    ReversedDateRange,
    NoMatchingDates,
    TooManyDates,
    TooManyMonthRanges,
    TooManyWeekRanges,
    TooManyQuarterRanges,
    TooManyHalfYearRanges,
    TooManyYearRanges,
    OutOfRange,
}

impl DateError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "可以输入日期、天数或星期几，例如 日期: 今天 星期几",
            Self::InvalidDate => "输入有效日期或月份，例如 2026年11月21日 或 2026-11",
            Self::InvalidMonthCalendar => "填写月份，例如 2024-02、本月、上月或下月",
            Self::InvalidWeek => "请输入有效的 ISO 周号或周日期，例如 2026-W47 或 2026-W47-6",
            Self::InvalidQuarter => "请输入季度编号 1 至 4，例如 2026-Q4",
            Self::InvalidYearDay => "请输入该年有效的天数，例如 2024 第60天",
            Self::InvalidWeekdayOccurrence => {
                "请输入第 1 至第 5 个或最后一个星期几，例如 日期: 2026-11 最后一个星期四"
            }
            Self::MissingWeekdayOccurrence => "该月没有指定次序的星期",
            Self::InvalidWorkdayOrdinal => {
                "填写月份和工作日序号，例如 2026-11 @ 3。用 -1 表示最后一个工作日"
            }
            Self::InvalidWorkdayPosition => "填写单个日期，例如 2026-11-04 或 今天",
            Self::MissingWorkdayOrdinal => "该月的工作日不足指定序号",
            Self::InvalidWorkdaySeries => {
                "填写起始月份、1 至 366 个月和工作日序号，例如 2026-11,3 @ -1"
            }
            Self::InvalidOperation => "可查星期几、加减日期或工作日，或用两个日期相减",
            Self::InvalidDateList => "例如 日期: 列出 2026-11-01 到 2026-11-30 每周一、三、五",
            Self::DateSeriesArguments => "用逗号填写起始日期和 1 至 366 个日期，例如 2024-01,3",
            Self::InvalidDateRanges => {
                "填写 2 至 32 个日期区间。每段用“至”连接日期，用分号或换行分隔各段"
            }
            Self::RangeSplitArguments => {
                "用“至”连接开始日期和结束日期，例如 2024-01-31 至 2024-03-02"
            }
            Self::InvalidDateInterval => "请输入正整数间隔，例如 每隔 3 天或每隔 2 个工作日",
            Self::WorkdayExceptions => {
                "工作日例外需要两个日期数组，先休息日再补班日，每个日期只填一次"
            }
            Self::PatternArguments => {
                "用 JSON 数组填写日期和格式，例如 [\"2026-11-21\",\"%Y%m%d\"]"
            }
            Self::InvalidDatePattern => "请使用日期格式符，例如 %Y、%m、%d 或 %F",
            Self::DatePatternMismatch => "日期文本与格式不符，或日期无效",
            Self::WeekdayCountArguments => "输入起止日期，例如 2024-02-29 至 2024-03-03",
            Self::ReversedDateRange => "开始日期须早于或等于结束日期",
            Self::NoMatchingDates => "区间内没有符合条件的日期",
            Self::TooManyDates => "最多可列出 366 个日期，请缩短区间",
            Self::TooManyMonthRanges => "最多可拆分为 366 个月份，请缩短区间",
            Self::TooManyWeekRanges => "最多可拆分为 366 周，请缩短区间",
            Self::TooManyQuarterRanges => "最多可拆分为 366 个季度，请缩短区间",
            Self::TooManyHalfYearRanges => "最多可拆分为 366 个半年度，请缩短区间",
            Self::TooManyYearRanges => "最多可拆分为 366 年，请缩短区间",
            Self::OutOfRange => "结果超出四位年份的日期范围",
        }
    }
}

fn relative_day(input: &str) -> Option<(i8, &str)> {
    [
        ("今天", 0),
        ("today", 0),
        ("昨天", -1),
        ("yesterday", -1),
        ("前天", -2),
        ("明天", 1),
        ("tomorrow", 1),
        ("后天", 2),
    ]
    .into_iter()
    .find_map(|(name, offset)| {
        let (prefix, rest) = input.split_at_checked(name.len())?;
        prefix
            .eq_ignore_ascii_case(name)
            .then_some((offset, rest.trim()))
    })
}

fn in_range(date: Option<NaiveDate>) -> Result<NaiveDate, DateError> {
    date.filter(|date| (0..=9999).contains(&date.year()))
        .ok_or(DateError::OutOfRange)
}

fn iso_date_prefix(input: &str) -> Option<(&str, &str)> {
    let (date, rest) = input.split_at_checked(10)?;
    date.bytes()
        .enumerate()
        .all(|(index, byte)| match index {
            4 | 7 => byte == b'-',
            _ => byte.is_ascii_digit(),
        })
        .then_some((date, rest.trim()))
}

fn iso_week_date_prefix(input: &str) -> Option<(&str, &str)> {
    let (date, rest) = input.split_at_checked(10)?;
    date.bytes()
        .enumerate()
        .all(|(index, byte)| match index {
            4 | 8 => byte == b'-',
            5 => matches!(byte, b'W' | b'w'),
            _ => byte.is_ascii_digit(),
        })
        .then_some((date, rest.trim()))
}

fn written_date_prefix(input: &str) -> Option<(i32, u32, u32, &str)> {
    let (year, rest) = input.split_at_checked(4)?;
    if !year.bytes().all(|digit| digit.is_ascii_digit()) {
        return None;
    }
    let (month, day, rest) = if let Some(rest) = rest.strip_prefix('年') {
        let (month, day) = rest.split_once('月')?;
        let (day, rest) = day.split_once('日')?;
        (month, day, rest)
    } else {
        let (month, tail) = rest.strip_prefix('/')?.split_once('/')?;
        let day_length = tail.bytes().take_while(u8::is_ascii_digit).count();
        let (day, rest) = tail.split_at(day_length);
        (month, day, rest)
    };
    let part = |text: &str| {
        if (1..=2).contains(&text.len()) && text.bytes().all(|digit| digit.is_ascii_digit()) {
            text.parse::<u32>().ok()
        } else {
            None
        }
    };
    Some((year.parse().ok()?, part(month)?, part(day)?, rest.trim()))
}

fn date_operand(input: &str, today: NaiveDate) -> Result<(NaiveDate, &str), DateError> {
    if let Some((offset, rest)) = relative_day(input) {
        let date = today.checked_add_signed(TimeDelta::days(i64::from(offset)));
        return Ok((in_range(date)?, rest));
    }
    if let Some((week, weekday, rest)) = relative_weekday(input) {
        let days = days_to_week_start(today, week) + i64::from(weekday.num_days_from_monday());
        let date = today.checked_add_signed(TimeDelta::days(days));
        return Ok((in_range(date)?, rest));
    }
    if let Some((offset, months, last_day, rest)) = relative_period_edge(input) {
        let start = relative_period_start(today, offset, months)?;
        let date = if last_day {
            month_end(start, months)?
        } else {
            start
        };
        return Ok((date, rest));
    }
    absolute_date_operand(input)
}

pub(super) fn parse_complete_date(input: &str) -> Result<NaiveDate, DateError> {
    let input = normalize(input);
    let (date, rest) = absolute_date_operand(input.trim())?;
    if rest.is_empty() {
        Ok(date)
    } else {
        Err(DateError::InvalidDate)
    }
}

fn absolute_date_operand(input: &str) -> Result<(NaiveDate, &str), DateError> {
    if let Some((year, month, day, rest)) = written_date_prefix(input) {
        let date = NaiveDate::from_ymd_opt(year, month, day).ok_or(DateError::InvalidDate)?;
        return Ok((date, rest));
    }
    if let Some((date, rest)) = iso_week_date_prefix(input) {
        let date = NaiveDate::parse_from_str(&date.to_ascii_uppercase(), "%G-W%V-%u")
            .map_err(|_| DateError::InvalidWeek)?;
        return Ok((in_range(Some(date))?, rest));
    }
    if input.is_empty()
        || input.len() < 10
            && input
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte == b'-')
    {
        return Err(DateError::Incomplete);
    }
    let (date, rest) = iso_date_prefix(input).ok_or(DateError::InvalidDate)?;
    let date = NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|_| DateError::InvalidDate)?;
    Ok((date, rest))
}

fn weekday_query(input: &str) -> Option<&str> {
    if let Some(date) = input
        .strip_suffix("星期几")
        .or_else(|| input.strip_suffix("周几"))
    {
        return Some(date.trim_end());
    }
    if input.eq_ignore_ascii_case("weekday") {
        return Some("");
    }
    let (date, suffix) = input.rsplit_once(char::is_whitespace)?;
    suffix
        .eq_ignore_ascii_case("weekday")
        .then_some(date.trim_end())
}

fn week_number_query(input: &str) -> Option<&str> {
    input
        .strip_suffix("第几周")
        .or_else(|| input.strip_suffix("周数"))
        .map(str::trim_end)
}

fn day_number_query(input: &str) -> Option<&str> {
    input.strip_suffix("第几天").map(str::trim_end)
}

fn week_date_query(input: &str) -> Option<&str> {
    input.strip_suffix("周日期").map(str::trim_end)
}

fn calendar_period_parts(input: &str, marker: [char; 2]) -> Option<(&str, &str)> {
    let (year, rest) = input.split_at_checked(4)?;
    Some((year, rest.strip_prefix('-')?.strip_prefix(marker)?))
}

fn iso_year(input: &str) -> Option<i32> {
    if input.len() == 4 && input.bytes().all(|digit| digit.is_ascii_digit()) {
        input.parse().ok()
    } else {
        None
    }
}

fn year_day_parts(input: &str) -> Option<(i32, &str)> {
    let (year, day) = input.split_once('第')?;
    let year = year.trim_end();
    let year = iso_year(year.strip_suffix('年').unwrap_or(year).trim_end())?;
    let day = day.trim().strip_suffix('天')?.trim_end();
    Some((year, day))
}

fn iso_month_parts(input: &str) -> Option<(i32, u32)> {
    let (year, month) = input.split_once('-')?;
    if year.len() == 4
        && month.len() == 2
        && year
            .bytes()
            .chain(month.bytes())
            .all(|digit| digit.is_ascii_digit())
    {
        Some((year.parse().ok()?, month.parse().ok()?))
    } else {
        None
    }
}

fn weekday_occurrence_parts(input: &str) -> Option<(i32, u32, &str)> {
    let (month, rule) = input.split_at_checked(7)?;
    let (year, month) = iso_month_parts(month)?;
    let rule = rule.trim();
    (rule.starts_with('第') || rule.starts_with("最后")).then_some((year, month, rule))
}

enum RelativePeriod {
    Week(i8),
    Month(i8),
    Quarter(i8),
    Year(i8),
}

fn relative_period(input: &str) -> Option<RelativePeriod> {
    use RelativePeriod::{Month, Quarter, Week, Year};
    match input.to_ascii_lowercase().as_str() {
        "本周" | "这周" | "本星期" | "this week" => Some(Week(0)),
        "上周" | "上星期" | "last week" => Some(Week(-1)),
        "下周" | "下星期" | "next week" => Some(Week(1)),
        "本月" | "这个月" | "this month" => Some(Month(0)),
        "上月" | "上个月" | "last month" => Some(Month(-1)),
        "下月" | "下个月" | "next month" => Some(Month(1)),
        "本季度" | "这季度" | "this quarter" => Some(Quarter(0)),
        "上季度" | "上个季度" | "last quarter" => Some(Quarter(-1)),
        "下季度" | "下个季度" | "next quarter" => Some(Quarter(1)),
        "今年" | "本年" | "this year" => Some(Year(0)),
        "去年" | "上年" | "last year" => Some(Year(-1)),
        "明年" | "下年" | "next year" => Some(Year(1)),
        _ => None,
    }
}

fn relative_weekday(input: &str) -> Option<(i8, Weekday, &str)> {
    let (week, rest) = input.split_at_checked(3)?;
    let week = match week {
        "本" | "这" => 0,
        "上" => -1,
        "下" => 1,
        _ => return None,
    };
    let rest = rest
        .strip_prefix('周')
        .or_else(|| rest.strip_prefix("星期"))?;
    let (day, tail) = rest.trim_start().split_at_checked(3)?;
    Some((week, chinese_weekday(day)?, tail.trim()))
}

fn days_to_week_start(today: NaiveDate, week: i8) -> i64 {
    i64::from(week) * 7 - i64::from(today.weekday().num_days_from_monday())
}

fn chinese_weekday(input: &str) -> Option<Weekday> {
    Some(match input {
        "一" => Weekday::Mon,
        "二" => Weekday::Tue,
        "三" => Weekday::Wed,
        "四" => Weekday::Thu,
        "五" => Weekday::Fri,
        "六" => Weekday::Sat,
        "日" | "天" => Weekday::Sun,
        _ => return None,
    })
}

fn relative_period_edge(input: &str) -> Option<(i8, u32, bool, &str)> {
    [
        ("本月", 0, 1),
        ("上月", -1, 1),
        ("下月", 1, 1),
        ("月", 0, 1),
        ("本季度", 0, 3),
        ("上季度", -1, 3),
        ("下季度", 1, 3),
        ("季度", 0, 3),
        ("本季", 0, 3),
        ("上季", -1, 3),
        ("下季", 1, 3),
        ("季", 0, 3),
        ("今年", 0, 12),
        ("去年", -1, 12),
        ("明年", 1, 12),
        ("年", 0, 12),
    ]
    .into_iter()
    .find_map(|(prefix, offset, months)| {
        let mut rest = input.strip_prefix(prefix)?.chars();
        let last_day = match rest.next()? {
            '初' => false,
            '底' | '末' => true,
            _ => return None,
        };
        Some((offset, months, last_day, rest.as_str().trim_start()))
    })
}

pub(super) fn looks_like_date_query(query: &str) -> bool {
    let input = normalize(query);
    let input = input.trim();
    let date_tail = |rest: &str| rest.is_empty() || rest.starts_with(['+', '-']);
    if weekday_occurrence_parts(input).is_some() || year_day_parts(input).is_some() {
        return true;
    }
    let week_date = iso_week_date_prefix(input);
    if week_date.is_some_and(|(_, rest)| date_tail(rest)) {
        return true;
    }
    if let Some((year, ordinal)) = calendar_period_parts(input, ['W', 'w'])
        .filter(|_| week_date.is_none())
        .or_else(|| calendar_period_parts(input, ['Q', 'q']))
    {
        return year.bytes().all(|byte| byte.is_ascii_digit())
            && ordinal.bytes().all(|byte| byte.is_ascii_digit());
    }
    if let Some(date) = weekday_query(input)
        .or_else(|| week_number_query(input))
        .or_else(|| day_number_query(input))
        .or_else(|| week_date_query(input))
    {
        return date.is_empty()
            || relative_day(date).is_some_and(|(_, rest)| date_tail(rest))
            || relative_weekday(date).is_some_and(|(_, _, rest)| date_tail(rest))
            || relative_period_edge(date).is_some_and(|(_, _, _, rest)| date_tail(rest))
            || iso_date_prefix(date).is_some_and(|(_, rest)| date_tail(rest))
            || iso_week_date_prefix(date).is_some_and(|(_, rest)| date_tail(rest))
            || written_date_prefix(date).is_some_and(|(_, _, _, rest)| date_tail(rest));
    }
    relative_period(input).is_some()
        || input
            .strip_suffix('年')
            .and_then(|year| iso_year(year.trim_end()))
            .is_some()
        || relative_day(input).is_some_and(|(_, rest)| date_tail(rest))
        || relative_weekday(input).is_some_and(|(_, _, rest)| date_tail(rest))
        || relative_period_edge(input).is_some_and(|(_, _, _, rest)| date_tail(rest))
        || (input.get(4..).is_some_and(|rest| rest.starts_with('年'))
            && written_date_prefix(input).is_some_and(|(_, _, _, rest)| date_tail(rest)))
}

fn is_workday_unit(unit: &str) -> bool {
    matches!(
        unit,
        "工作日" | "个工作日" | "business day" | "business days" | "workday" | "workdays"
    )
}

enum DateSelection {
    EveryDays(u32),
    EveryMonths(u32),
    Workdays {
        step: u32,
        exceptions: HashMap<NaiveDate, bool>,
    },
    Weekends,
    Weekdays([bool; 7]),
}

fn date_selection(rule: &str) -> Result<DateSelection, DateError> {
    let lower = rule.to_ascii_lowercase();
    let (rule, exceptions) = split_workday_exceptions(&lower);
    if is_workday_unit(rule) {
        return workday_selection(1, exceptions);
    }
    if exceptions.is_none() {
        match rule {
            "每天" | "daily" => return Ok(DateSelection::EveryDays(1)),
            "每月" | "每个月" | "monthly" => return Ok(DateSelection::EveryMonths(1)),
            _ => {}
        }
    }
    let interval = rule
        .strip_prefix("每隔")
        .or_else(|| {
            rule.strip_prefix('每')
                .filter(|value| !value.starts_with("周") && !value.starts_with("星期"))
        })
        .and_then(|value| {
            ["个工作日", "工作日", "个月", "月", "天"]
                .into_iter()
                .find_map(|unit| {
                    value
                        .trim_end()
                        .strip_suffix(unit)
                        .map(|count| (count, unit))
                })
        })
        .or_else(|| {
            let (keyword, value) = rule.split_once(char::is_whitespace)?;
            if keyword != "every" {
                return None;
            }
            let (count, unit) = value.trim_start().split_once(char::is_whitespace)?;
            Some((count, unit.trim()))
        });
    if let Some((count, unit)) = interval {
        if is_workday_unit(unit) {
            return workday_selection(positive_date_step(count)?, exceptions);
        }
        if exceptions.is_none() {
            match unit {
                "天" | "day" | "days" => {
                    return Ok(DateSelection::EveryDays(positive_date_step(count)?));
                }
                "个月" | "月" | "month" | "months" => {
                    return Ok(DateSelection::EveryMonths(positive_date_step(count)?));
                }
                _ => {}
            }
        }
    }
    if exceptions.is_some() {
        return Err(DateError::InvalidDateList);
    }
    match rule {
        "周末" | "weekend" | "weekends" => Ok(DateSelection::Weekends),
        rule => {
            let mut selected = [false; 7];
            for term in rule.split([',', '，', '、']) {
                let term = term.trim();
                let name = term
                    .strip_prefix("每周")
                    .or_else(|| term.strip_prefix("每星期"))
                    .or_else(|| term.strip_prefix("周"))
                    .or_else(|| term.strip_prefix("星期"))
                    .unwrap_or(term);
                let weekday = chinese_weekday(name)
                    .or_else(|| term.parse::<Weekday>().ok())
                    .ok_or(DateError::InvalidDateList)?;
                selected[weekday.num_days_from_monday() as usize] = true;
            }
            Ok(DateSelection::Weekdays(selected))
        }
    }
}

fn positive_date_step(count: &str) -> Result<u32, DateError> {
    count
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|&step| step > 0)
        .ok_or(DateError::InvalidDateInterval)
}

fn workday_selection(step: u32, input: Option<&str>) -> Result<DateSelection, DateError> {
    Ok(DateSelection::Workdays {
        step,
        exceptions: parse_workday_exceptions(input)?,
    })
}

fn split_workday_exceptions(input: &str) -> (&str, Option<&str>) {
    input
        .split_once("例外")
        .or_else(|| input.split_once(" except "))
        .map(|(rule, dates)| (rule.trim_end(), Some(dates.trim_start())))
        .unwrap_or((input, None))
}

fn parse_workday_exceptions(input: Option<&str>) -> Result<HashMap<NaiveDate, bool>, DateError> {
    let Some(input) = input else {
        return Ok(HashMap::new());
    };
    let dates = serde_json::from_str(input).map_err(|_| DateError::WorkdayExceptions)?;
    workday_exceptions_from_dates(dates)
}

pub(super) fn workday_exceptions_from_dates(
    [days_off, days_on]: [Vec<String>; 2],
) -> Result<HashMap<NaiveDate, bool>, DateError> {
    let mut exceptions = HashMap::new();
    for (dates, is_workday) in [(days_off, false), (days_on, true)] {
        for date in dates {
            let (date, "") = iso_date_prefix(&date).ok_or(DateError::WorkdayExceptions)? else {
                return Err(DateError::WorkdayExceptions);
            };
            let date = NaiveDate::parse_from_str(date, "%Y-%m-%d")
                .map_err(|_| DateError::WorkdayExceptions)?;
            if exceptions.insert(date, is_workday).is_some() {
                return Err(DateError::WorkdayExceptions);
            }
        }
    }
    Ok(exceptions)
}

fn is_workday(date: &NaiveDate, exceptions: &HashMap<NaiveDate, bool>) -> bool {
    exceptions
        .get(date)
        .copied()
        .unwrap_or_else(|| date.weekday().num_days_from_monday() < 5)
}

fn workday_adjustment(
    exceptions: &HashMap<NaiveDate, bool>,
    range: impl RangeBounds<NaiveDate>,
) -> i64 {
    exceptions
        .iter()
        .filter(|(date, _)| range.contains(*date))
        .map(|(&date, &workday)| {
            i64::from(workday) - i64::from(date.weekday().number_from_monday() <= 5)
        })
        .sum()
}

fn date_range_operands(
    input: &str,
    today: NaiveDate,
    separator_error: DateError,
) -> Result<(NaiveDate, NaiveDate, &str), DateError> {
    let (start, rest) = date_operand(input, today)?;
    let rest = rest
        .strip_prefix(['到', '至'])
        .or_else(|| {
            let (separator, end) = rest.split_once(char::is_whitespace)?;
            separator.eq_ignore_ascii_case("to").then_some(end)
        })
        .ok_or(separator_error)?;
    let (end, remaining) = date_operand(rest.trim_start(), today)?;
    Ok((start, end, remaining))
}

fn count_weekdays_csv(input: &str, today: NaiveDate) -> Result<String, DateError> {
    let input = input.trim_start();
    if input.is_empty() {
        return Err(DateError::WeekdayCountArguments);
    }
    let (start, end, rest) = date_range_operands(input, today, DateError::WeekdayCountArguments)?;
    if !rest.is_empty() {
        return Err(DateError::WeekdayCountArguments);
    }
    if end < start {
        return Err(DateError::ReversedDateRange);
    }
    let days = end.signed_duration_since(start).num_days() + 1;
    let first = start.weekday().num_days_from_monday() as usize;
    let mut counts = [days / 7; 7];
    for offset in 0..(days % 7) as usize {
        counts[(first + offset) % 7] += 1;
    }
    let mut rows = vec!["星期,天数".to_owned()];
    rows.extend(
        WEEKDAY_LABELS
            .into_iter()
            .zip(counts)
            .map(|(weekday, count)| format!("{weekday},{count}")),
    );
    Ok(rows.join("\r\n"))
}

fn list_dates(input: &str, today: NaiveDate) -> Result<String, DateError> {
    if input.is_empty() {
        return Err(DateError::InvalidDateList);
    }
    let (start, end, rule) = date_range_operands(input, today, DateError::InvalidDateList)?;
    let selection = date_selection(rule)?;
    if end < start {
        return Err(DateError::ReversedDateRange);
    }
    match &selection {
        DateSelection::EveryMonths(step) => {
            // 每次从起始日计算，短月取月底后，下一个月仍使用起始日的日号。
            let dates = (0_u32..)
                .map_while(|index| start.checked_add_months(Months::new(index.checked_mul(*step)?)))
                .take_while(|date| *date <= end);
            return format_date_list(dates);
        }
        DateSelection::Workdays { step, exceptions } if exceptions.is_empty() => {
            let first = if start.weekday().num_days_from_monday() < 5 {
                Some(start)
            } else {
                shift_workdays(start, 1, false)
            };
            return format_date_list(
                std::iter::successors(first, |date| shift_workdays(*date, u64::from(*step), false))
                    .take_while(|date| *date <= end),
            );
        }
        _ => {}
    }
    let days = end.signed_duration_since(start).num_days();
    let step = match selection {
        DateSelection::EveryDays(step) => step as usize,
        _ => 1,
    };
    let filtered_step = match &selection {
        DateSelection::Workdays { step, .. } => *step as usize,
        _ => 1,
    };
    let dates = (0..=days)
        .step_by(step)
        .map(|offset| start + TimeDelta::days(offset))
        .filter(|date| match &selection {
            DateSelection::Workdays { exceptions, .. } => is_workday(date, exceptions),
            DateSelection::Weekends => date.weekday().num_days_from_monday() >= 5,
            DateSelection::Weekdays(days) => days[date.weekday().num_days_from_monday() as usize],
            DateSelection::EveryDays(_) | DateSelection::EveryMonths(_) => true,
        })
        .step_by(filtered_step);
    format_date_list(dates)
}

enum DateRangeOperation {
    Merge,
    Intersect,
    Subtract,
}

fn combine_date_ranges(
    input: &str,
    today: NaiveDate,
    operation: DateRangeOperation,
) -> Result<String, DateError> {
    let terms = input.trim().split([';', '\n']);
    if !(2..=MAX_DATE_RANGES).contains(&terms.clone().count()) {
        return Err(DateError::InvalidDateRanges);
    }
    let mut ranges = terms
        .map(|term| {
            let (start, end, remaining) =
                date_range_operands(term.trim(), today, DateError::InvalidDateRanges)?;
            if !remaining.is_empty() {
                return Err(DateError::InvalidDateRanges);
            }
            if end < start {
                return Err(DateError::ReversedDateRange);
            }
            Ok((start, end))
        })
        .collect::<Result<Vec<_>, _>>()?;
    match operation {
        DateRangeOperation::Intersect => {
            let mut overlap = ranges[0];
            for &(start, end) in &ranges[1..] {
                overlap = (overlap.0.max(start), overlap.1.min(end));
            }
            return Ok(if overlap.0 <= overlap.1 {
                display_date_range(overlap)
            } else {
                "无重叠日期".to_owned()
            });
        }
        DateRangeOperation::Subtract => {
            let (base, exclusions) = ranges.split_at_mut(1);
            return Ok(subtract_date_ranges(base[0], exclusions));
        }
        DateRangeOperation::Merge => {}
    }
    ranges.sort_unstable();
    let mut merged = Vec::new();
    let mut current = ranges[0];
    for &(start, end) in &ranges[1..] {
        if start.signed_duration_since(current.1).num_days() <= 1 {
            current.1 = current.1.max(end);
        } else {
            merged.push(display_date_range(current));
            current = (start, end);
        }
    }
    merged.push(display_date_range(current));
    Ok(merged.join("\n"))
}

fn subtract_date_ranges(
    (start, end): (NaiveDate, NaiveDate),
    exclusions: &mut [(NaiveDate, NaiveDate)],
) -> String {
    exclusions.sort_unstable();
    let mut remaining = Vec::new();
    let mut cursor = Some(start);
    for &(excluded_start, excluded_end) in exclusions.iter() {
        let Some(next) = cursor else { break };
        if excluded_start > end {
            break;
        }
        if excluded_end < next {
            continue;
        }
        if excluded_start > next {
            remaining.push(display_date_range((
                next,
                excluded_start - TimeDelta::days(1),
            )));
        }
        cursor = excluded_end.succ_opt().filter(|&date| date <= end);
    }
    if let Some(next) = cursor {
        remaining.push(display_date_range((next, end)));
    }
    if remaining.is_empty() {
        "无剩余日期".to_owned()
    } else {
        remaining.join("\n")
    }
}

#[derive(Clone, Copy)]
enum DateRangeSplit {
    Months,
    MonthsCsv,
    WeeksCsv,
    QuartersCsv,
    HalfYearsCsv,
    YearsCsv,
    DaysCsv,
}

fn split_date_range(
    input: &str,
    today: NaiveDate,
    split: DateRangeSplit,
) -> Result<String, DateError> {
    let csv = !matches!(split, DateRangeSplit::Months);
    if input.is_empty() {
        return Err(DateError::RangeSplitArguments);
    }
    let (mut start, end, remaining) =
        date_range_operands(input.trim_start(), today, DateError::RangeSplitArguments)?;
    let exceptions = match (csv, remaining.trim_start()) {
        (_, "") => HashMap::new(),
        (true, remaining) => {
            let (first, rest) = remaining
                .split_once(char::is_whitespace)
                .unwrap_or((remaining, ""));
            let dates = remaining
                .strip_prefix("例外")
                .or_else(|| first.eq_ignore_ascii_case("except").then_some(rest))
                .ok_or(DateError::RangeSplitArguments)?;
            parse_workday_exceptions(Some(dates.trim_start()))?
        }
        _ => return Err(DateError::RangeSplitArguments),
    };
    if end < start {
        return Err(DateError::ReversedDateRange);
    }
    let mut ranges = Vec::new();
    match split {
        DateRangeSplit::Months => {}
        DateRangeSplit::MonthsCsv => ranges.push("开始,结束,天数,工作日".to_owned()),
        DateRangeSplit::WeeksCsv => ranges.push("周号,开始,结束,天数,工作日".to_owned()),
        DateRangeSplit::QuartersCsv => ranges.push("季度,开始,结束,天数,工作日".to_owned()),
        DateRangeSplit::HalfYearsCsv => ranges.push("半年度,开始,结束,天数,工作日".to_owned()),
        DateRangeSplit::YearsCsv => ranges.push("年份,开始,结束,天数,工作日".to_owned()),
        DateRangeSplit::DaysCsv => ranges.push("日期,星期,工作日".to_owned()),
    }
    for _ in 0..MAX_SPLIT_RANGES {
        let quarter = start.month0() / 3;
        let last = match split {
            DateRangeSplit::Months | DateRangeSplit::MonthsCsv => {
                month_end(in_range(start.with_day(1))?, 1)?
            }
            DateRangeSplit::WeeksCsv => {
                let days_until_sunday = u64::from(6 - start.weekday().num_days_from_monday());
                start
                    .checked_add_days(Days::new(days_until_sunday))
                    .ok_or(DateError::OutOfRange)?
            }
            DateRangeSplit::QuartersCsv => month_end(
                in_range(NaiveDate::from_ymd_opt(start.year(), quarter * 3 + 1, 1))?,
                3,
            )?,
            DateRangeSplit::HalfYearsCsv => month_end(relative_period_start(start, 0, 6)?, 6)?,
            DateRangeSplit::YearsCsv => in_range(NaiveDate::from_ymd_opt(start.year(), 12, 31))?,
            DateRangeSplit::DaysCsv => start,
        }
        .min(end);
        let row = if csv {
            let days = last.signed_duration_since(start).num_days() + 1;
            let workdays =
                inclusive_workdays(start, last) + workday_adjustment(&exceptions, start..=last);
            match split {
                DateRangeSplit::WeeksCsv => {
                    let week = start.format("%G-W%V");
                    format!("{week},{start},{last},{days},{workdays}")
                }
                DateRangeSplit::QuartersCsv => {
                    let year = start.year();
                    let number = quarter + 1;
                    format!("{year:04}-Q{number},{start},{last},{days},{workdays}")
                }
                DateRangeSplit::HalfYearsCsv => {
                    let year = start.year();
                    let half = start.month0() / 6 + 1;
                    format!("{year:04}-H{half},{start},{last},{days},{workdays}")
                }
                DateRangeSplit::YearsCsv => {
                    let year = start.year();
                    format!("{year:04},{start},{last},{days},{workdays}")
                }
                DateRangeSplit::DaysCsv => {
                    let weekday = WEEKDAY_LABELS[start.weekday().num_days_from_monday() as usize];
                    format!("{start},{weekday},{workdays}")
                }
                _ => format!("{start},{last},{days},{workdays}"),
            }
        } else {
            display_date_range((start, last))
        };
        ranges.push(row);
        if last == end {
            return Ok(ranges.join(if csv { "\r\n" } else { "\n" }));
        }
        start = in_range(last.succ_opt())?;
    }
    Err(match split {
        DateRangeSplit::Months | DateRangeSplit::MonthsCsv => DateError::TooManyMonthRanges,
        DateRangeSplit::WeeksCsv => DateError::TooManyWeekRanges,
        DateRangeSplit::QuartersCsv => DateError::TooManyQuarterRanges,
        DateRangeSplit::HalfYearsCsv => DateError::TooManyHalfYearRanges,
        DateRangeSplit::YearsCsv => DateError::TooManyYearRanges,
        DateRangeSplit::DaysCsv => DateError::TooManyDates,
    })
}

fn display_date_range((start, end): (NaiveDate, NaiveDate)) -> String {
    let days = end.signed_duration_since(start).num_days() + 1;
    format!("{start} 至 {end} · {days} 天")
}

fn format_date_list(dates: impl Iterator<Item = NaiveDate>) -> Result<String, DateError> {
    let dates: Vec<_> = dates
        .take(MAX_LISTED_DATES + 1)
        .map(|date| date.to_string())
        .collect();
    match dates.len() {
        0 => Err(DateError::NoMatchingDates),
        count if count > MAX_LISTED_DATES => Err(DateError::TooManyDates),
        _ => Ok(dates.join("\n")),
    }
}

fn iso_week_range(year: &str, week: &str) -> Result<String, DateError> {
    if week.is_empty() {
        return Err(DateError::Incomplete);
    }
    if !year.bytes().all(|byte| byte.is_ascii_digit())
        || week.len() > 2
        || !week.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(DateError::InvalidWeek);
    }
    let year = year.parse().map_err(|_| DateError::InvalidWeek)?;
    let week = week.parse().map_err(|_| DateError::InvalidWeek)?;
    let monday =
        NaiveDate::from_isoywd_opt(year, week, Weekday::Mon).ok_or(DateError::InvalidWeek)?;
    week_range(monday)
}

fn week_range(monday: NaiveDate) -> Result<String, DateError> {
    let start = in_range(Some(monday))?;
    let end = in_range(monday.checked_add_days(Days::new(6)))?;
    Ok(format!("{start} 至 {end}"))
}

fn weekday_occurrence(year: i32, month: u32, rule: &str) -> Result<String, DateError> {
    if matches!(rule, "第" | "最后") {
        return Err(DateError::Incomplete);
    }
    let (ordinal, label) = rule
        .split_once('个')
        .ok_or(DateError::InvalidWeekdayOccurrence)?;
    let ordinal = match ordinal.trim_end() {
        "最后一" => None,
        text => Some(
            text.strip_prefix('第')
                .and_then(|text| text.trim_start().parse::<u32>().ok())
                .filter(|ordinal| (1..=5).contains(ordinal))
                .ok_or(DateError::InvalidWeekdayOccurrence)?,
        ),
    };
    let label = label.trim_start();
    let day = label
        .strip_prefix("星期")
        .or_else(|| label.strip_prefix('周'))
        .and_then(chinese_weekday)
        .ok_or(DateError::InvalidWeekdayOccurrence)?;
    let start = NaiveDate::from_ymd_opt(year, month, 1).ok_or(DateError::InvalidDate)?;
    let day_number = day.num_days_from_monday();
    let date = if let Some(ordinal) = ordinal {
        let first = (day_number + 7 - start.weekday().num_days_from_monday()) % 7;
        let days = Days::new(u64::from(first + (ordinal - 1) * 7));
        start
            .checked_add_days(days)
            .filter(|date| date.month() == month)
            .ok_or(DateError::MissingWeekdayOccurrence)?
    } else {
        let end = month_end(start, 1)?;
        let offset = (end.weekday().num_days_from_monday() + 7 - day_number) % 7;
        in_range(end.checked_sub_days(Days::new(u64::from(offset))))?
    };
    Ok(date.to_string())
}

fn workday_difference(date: NaiveDate, other: NaiveDate) -> i64 {
    let difference = date.signed_duration_since(other).num_days();
    let days = difference.abs();
    let start = i64::from(date.min(other).weekday().num_days_from_monday());
    let remaining_workdays = (1..=days % 7)
        .filter(|offset| (start + offset) % 7 < 5)
        .count() as i64;
    difference.signum() * (days / 7 * 5 + remaining_workdays)
}

fn month_end(start: NaiveDate, months: u32) -> Result<NaiveDate, DateError> {
    in_range(
        start
            .checked_add_months(Months::new(months))
            .and_then(|next| next.pred_opt()),
    )
}

fn relative_period_start(
    today: NaiveDate,
    offset: i8,
    months: u32,
) -> Result<NaiveDate, DateError> {
    let first_month = today.month0() / months * months + 1;
    let shift = Months::new(u32::from(offset.unsigned_abs()) * months);
    in_range(
        NaiveDate::from_ymd_opt(today.year(), first_month, 1).and_then(|start| {
            if offset < 0 {
                start.checked_sub_months(shift)
            } else {
                start.checked_add_months(shift)
            }
        }),
    )
}

fn calendar_month_start(input: &str, today: NaiveDate) -> Result<NaiveDate, DateError> {
    if let Some((year, month)) = iso_month_parts(input) {
        NaiveDate::from_ymd_opt(year, month, 1).ok_or(DateError::InvalidMonthCalendar)
    } else if let Some(RelativePeriod::Month(offset)) = relative_period(input) {
        relative_period_start(today, offset, 1)
    } else {
        Err(DateError::InvalidMonthCalendar)
    }
}

fn month_calendar(input: &str, today: NaiveDate) -> Result<String, DateError> {
    let start = calendar_month_start(input, today)?;
    let last = month_end(start, 1)?.day();
    let blank_days = start.weekday().num_days_from_monday() as usize;
    let mut cells = vec!["  ".to_owned(); blank_days];
    cells.extend((1..=last).map(|day| format!("{day:>2}")));
    let mut grid = format!("Mo  Tu  We  Th  Fr  Sa  Su    {}", start.format("%Y-%m"));
    for week in cells.chunks(7) {
        grid.push('\n');
        grid.push_str(&week.join("  "));
    }
    Ok(grid)
}

fn date_series_parts(input: &str, error: DateError) -> Result<(&str, u32), DateError> {
    let (start, count) = input.split_once(',').ok_or(error)?;
    let count = count
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|count| (1..=MAX_LISTED_DATES as u32).contains(count))
        .ok_or(error)?;
    Ok((start.trim(), count))
}

fn date_boundary_series(
    input: &str,
    today: NaiveDate,
    months: u32,
    last_day: bool,
) -> Result<String, DateError> {
    let (start, count) = date_series_parts(input, DateError::DateSeriesArguments)?;
    let date = match calendar_month_start(start, today) {
        Ok(date) => date,
        Err(DateError::InvalidMonthCalendar) => {
            let (date, rest) = date_operand(start, today)?;
            if !rest.trim().is_empty() {
                return Err(DateError::DateSeriesArguments);
            }
            date
        }
        Err(error) => return Err(error),
    };
    let first = relative_period_start(date, 0, months)?;
    let mut dates = Vec::with_capacity(count as usize);
    for index in 0..count {
        let start = in_range(first.checked_add_months(Months::new(index * months)))?;
        let date = if last_day {
            month_end(start, months)?
        } else {
            start
        };
        dates.push(date.to_string());
    }
    Ok(dates.join("\n"))
}

fn workday_by_ordinal(input: &str, today: NaiveDate) -> Result<String, DateError> {
    let (query, exceptions) = split_workday_exceptions(input.trim());
    let (month, ordinal) = query
        .split_once('@')
        .ok_or(DateError::InvalidWorkdayOrdinal)?;
    let start = calendar_month_start(month.trim(), today)?;
    let ordinal = parse_workday_ordinal(ordinal)?;
    let exceptions = parse_workday_exceptions(exceptions)?;
    month_workday(start, ordinal, &exceptions).map(|date| date.to_string())
}

fn workday_position(input: &str, today: NaiveDate) -> Result<String, DateError> {
    let (query, exceptions) = split_workday_exceptions(input.trim());
    if query.trim().is_empty() {
        return Err(DateError::InvalidWorkdayPosition);
    }
    let (date, rest) = date_operand(query.trim(), today)?;
    if !rest.trim().is_empty() {
        return Err(DateError::InvalidWorkdayPosition);
    }
    let exceptions = parse_workday_exceptions(exceptions)?;
    if !is_workday(&date, &exceptions) {
        return Ok(format!("{date} · 休息日"));
    }
    let first = in_range(date.with_day(1))?;
    let last = month_end(first, 1)?;
    let from_start =
        inclusive_workdays(first, date) + workday_adjustment(&exceptions, first..=date);
    let from_end = inclusive_workdays(date, last) + workday_adjustment(&exceptions, date..=last);
    Ok(format!(
        "{date} · 当月第 {from_start} 个工作日\n倒数第 {from_end} 个工作日"
    ))
}

fn workday_series(input: &str, today: NaiveDate) -> Result<String, DateError> {
    let (query, exceptions) = split_workday_exceptions(input.trim());
    let (range, ordinal) = query
        .split_once('@')
        .ok_or(DateError::InvalidWorkdaySeries)?;
    let (month, count) = date_series_parts(range, DateError::InvalidWorkdaySeries)?;
    let start = calendar_month_start(month, today)?;
    let ordinal = parse_workday_ordinal(ordinal)?;
    let exceptions = parse_workday_exceptions(exceptions)?;
    let mut dates = Vec::with_capacity(count as usize);
    for offset in 0..count {
        let month = in_range(start.checked_add_months(Months::new(offset)))?;
        dates.push(month_workday(month, ordinal, &exceptions)?.to_string());
    }
    Ok(dates.join("\n"))
}

fn parse_workday_ordinal(input: &str) -> Result<i32, DateError> {
    input
        .trim()
        .parse::<i32>()
        .ok()
        .filter(|ordinal| (1..=31).contains(&ordinal.unsigned_abs()))
        .ok_or(DateError::InvalidWorkdayOrdinal)
}

fn month_workday(
    start: NaiveDate,
    ordinal: i32,
    exceptions: &HashMap<NaiveDate, bool>,
) -> Result<NaiveDate, DateError> {
    let last_day = month_end(start, 1)?.day();
    let workdays: Vec<_> = (1..=last_day)
        .filter_map(|day| start.with_day(day))
        .filter(|date| is_workday(date, exceptions))
        .collect();
    let distance = ordinal.unsigned_abs() as usize;
    let index = if ordinal.is_negative() {
        workdays.len().checked_sub(distance)
    } else {
        Some(distance - 1)
    };
    index
        .and_then(|index| workdays.get(index))
        .copied()
        .ok_or(DateError::MissingWorkdayOrdinal)
}

fn month_range(start: NaiveDate, months: u32) -> Result<String, DateError> {
    let end = month_end(start, months)?;
    let workdays = inclusive_workdays(start, end);
    let days = end.signed_duration_since(start).num_days() + 1;
    Ok(format!("{start} 至 {end}\n{days} 天 · {workdays} 工作日"))
}

fn inclusive_workdays(start: NaiveDate, end: NaiveDate) -> i64 {
    workday_difference(end, start) + i64::from(start.weekday().number_from_monday() <= 5)
}

fn quarter_range(year: &str, quarter: &str) -> Result<String, DateError> {
    if quarter.is_empty() {
        return Err(DateError::Incomplete);
    }
    let [quarter @ b'1'..=b'4'] = quarter.as_bytes() else {
        return Err(DateError::InvalidQuarter);
    };
    if !year.bytes().all(|digit| digit.is_ascii_digit()) {
        return Err(DateError::InvalidQuarter);
    }
    let year = year.parse().map_err(|_| DateError::InvalidQuarter)?;
    let first_month = 1 + u32::from(*quarter - b'1') * 3;
    let start = NaiveDate::from_ymd_opt(year, first_month, 1).ok_or(DateError::InvalidQuarter)?;
    month_range(start, 3)
}

fn year_range(year: i32) -> Result<String, DateError> {
    month_range(in_range(NaiveDate::from_ymd_opt(year, 1, 1))?, 12)
}

fn calendar_difference(date: NaiveDate, other: NaiveDate) -> Result<String, DateError> {
    let (start, end, sign) = if date < other {
        (date, other, "-")
    } else {
        (other, date, "")
    };
    let mut months =
        ((end.year() - start.year()) * 12 + end.month() as i32 - start.month() as i32) as u32;
    let after_months = |months| in_range(start.checked_add_months(Months::new(months)));
    let mut anchor = after_months(months)?;
    if anchor > end {
        months -= 1;
        anchor = after_months(months)?;
    }
    let days = end.signed_duration_since(anchor).num_days();
    Ok(format!(
        "{sign}{} 年 {} 个月 {days} 天",
        months / 12,
        months % 12
    ))
}

fn shift_workdays(mut date: NaiveDate, mut amount: u64, subtract: bool) -> Option<NaiveDate> {
    let is_weekend = |date: NaiveDate| matches!(date.weekday(), Weekday::Sat | Weekday::Sun);
    let advance = |date: NaiveDate, count| {
        if subtract {
            date.checked_sub_days(Days::new(count))
        } else {
            date.checked_add_days(Days::new(count))
        }
    };
    if amount == 0 {
        return Some(date);
    }

    if is_weekend(date) {
        // 先到第一天工作日，再整周推进，保留剩余天数的起始星期。
        while is_weekend(date) {
            date = advance(date, 1)?;
        }
        amount -= 1;
    }
    date = advance(date, (amount / 5).checked_mul(7)?)?;
    for _ in 0..amount % 5 {
        date = advance(date, 1)?;
        while is_weekend(date) {
            date = advance(date, 1)?;
        }
    }
    Some(date)
}

fn shift_workdays_with_exceptions(
    mut date: NaiveDate,
    mut amount: u64,
    subtract: bool,
    exceptions: &HashMap<NaiveDate, bool>,
) -> Option<NaiveDate> {
    if amount == 0 {
        return Some(date);
    }
    let mut exceptions: Vec<_> = exceptions
        .iter()
        .map(|(&date, &is_workday)| (date, is_workday))
        .filter(|&(exception, _)| {
            if subtract {
                exception < date
            } else {
                exception > date
            }
        })
        .collect();
    exceptions.sort_unstable_by_key(|&(exception, _)| exception);
    if subtract {
        exceptions.reverse();
    }
    for (exception, is_workday) in exceptions {
        let latest = date.max(exception);
        // 两端之间的普通工作日可按整周计算，例外日期再单独处理。
        let gap = workday_difference(latest, date.min(exception)) as u64
            - u64::from(latest.weekday().number_from_monday() <= 5);
        if amount <= gap {
            return shift_workdays(date, amount, subtract);
        }
        date = exception;
        amount -= gap + u64::from(is_workday);
        if amount == 0 {
            return Some(date);
        }
    }
    shift_workdays(date, amount, subtract)
}

fn apply_date_offset(
    date: NaiveDate,
    operand: &str,
    subtract: bool,
) -> Result<NaiveDate, DateError> {
    let unit_start = operand
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(operand.len());
    let (amount, unit) = operand.split_at(unit_start);
    if amount.is_empty() {
        return Err(DateError::InvalidOperation);
    }
    let count = || amount.parse::<u64>().map_err(|_| DateError::OutOfRange);
    let lower = unit.trim().to_ascii_lowercase();
    let (unit, exceptions) = split_workday_exceptions(&lower);
    if is_workday_unit(unit) {
        return shift_calendar_workdays(
            date,
            count()?,
            subtract,
            &parse_workday_exceptions(exceptions)?,
        );
    }
    if exceptions.is_some() {
        return Err(DateError::InvalidOperation);
    }
    let (is_months, multiplier) = match unit {
        "" | "天" | "d" | "day" | "days" => (false, 1),
        "周" | "星期" | "w" | "week" | "weeks" => (false, 7),
        "月" | "个月" | "mo" | "month" | "months" => (true, 1),
        "年" | "y" | "yr" | "year" | "years" => (true, 12),
        _ => return Err(DateError::InvalidOperation),
    };
    let amount = count()?
        .checked_mul(multiplier)
        .ok_or(DateError::OutOfRange)?;
    if !is_months {
        return shift_calendar_days(date, amount, subtract);
    }
    shift_calendar_months(date, amount, subtract)
}

pub(super) fn shift_calendar_months(
    date: NaiveDate,
    amount: u64,
    subtract: bool,
) -> Result<NaiveDate, DateError> {
    let months = u32::try_from(amount)
        .map(Months::new)
        .map_err(|_| DateError::OutOfRange)?;
    in_range(if subtract {
        date.checked_sub_months(months)
    } else {
        date.checked_add_months(months)
    })
}

pub(super) fn shift_calendar_days(
    date: NaiveDate,
    amount: u64,
    subtract: bool,
) -> Result<NaiveDate, DateError> {
    let days = Days::new(amount);
    in_range(if subtract {
        date.checked_sub_days(days)
    } else {
        date.checked_add_days(days)
    })
}

pub(super) fn shift_calendar_workdays(
    date: NaiveDate,
    amount: u64,
    subtract: bool,
    exceptions: &HashMap<NaiveDate, bool>,
) -> Result<NaiveDate, DateError> {
    in_range(shift_workdays_with_exceptions(
        date, amount, subtract, exceptions,
    ))
}

fn convert_date_pattern(input: &str, today: NaiveDate, parse: bool) -> Result<String, DateError> {
    let [value, pattern] =
        serde_json::from_str::<[String; 2]>(input).map_err(|_| DateError::PatternArguments)?;
    if pattern.is_empty() || pattern.contains('\0') {
        return Err(DateError::PatternArguments);
    }
    if parse {
        let date = NaiveDate::parse_from_str(&value, &pattern)
            .map_err(|_| DateError::DatePatternMismatch)?;
        return in_range(Some(date)).map(|date| date.to_string());
    }
    let value = normalize(&value);
    let (date, rest) = date_operand(value.trim(), today)?;
    if !rest.is_empty() {
        return Err(DateError::InvalidDate);
    }
    let mut result = String::new();
    write!(result, "{}", date.format(&pattern)).map_err(|_| DateError::InvalidDatePattern)?;
    Ok(result)
}

pub(super) fn calculate_date(input: &str, today: NaiveDate) -> Result<String, DateError> {
    let original = input.trim();
    if let Some((operation, argument)) = original.split_once(char::is_whitespace) {
        match operation.to_ascii_lowercase().as_str() {
            "格式化" | "format" => return convert_date_pattern(argument, today, false),
            "解析" | "parse-format" => return convert_date_pattern(argument, today, true),
            _ => {}
        }
    }
    let input = normalize(input);
    let input = input.trim();
    let (operation, rest) = input.split_once(char::is_whitespace).unwrap_or((input, ""));
    if operation == "列出" || operation.eq_ignore_ascii_case("list") {
        return list_dates(rest.trim_start(), today);
    }
    match operation.to_ascii_lowercase().as_str() {
        "月历" | "month-grid" => return month_calendar(rest.trim_start(), today),
        "月初序列" | "month-start-series" => {
            return date_boundary_series(rest, today, 1, false)
        }
        "月末序列" | "month-end-series" => return date_boundary_series(rest, today, 1, true),
        "季初序列" | "quarter-start-series" => {
            return date_boundary_series(rest, today, 3, false)
        }
        "季末序列" | "quarter-end-series" => return date_boundary_series(rest, today, 3, true),
        "工作日序号" | "nth-workday" => return workday_by_ordinal(rest, today),
        "工作日位置" | "workday-position" => return workday_position(rest, today),
        "工作日序列" | "workday-series" => return workday_series(rest, today),
        "合并区间" | "merge-ranges" => {
            return combine_date_ranges(rest, today, DateRangeOperation::Merge);
        }
        "区间交集" | "intersect-ranges" => {
            return combine_date_ranges(rest, today, DateRangeOperation::Intersect);
        }
        "扣除区间" | "subtract-ranges" => {
            return combine_date_ranges(rest, today, DateRangeOperation::Subtract);
        }
        "按月拆分" | "split-months" => {
            return split_date_range(rest, today, DateRangeSplit::Months)
        }
        "按月拆分csv" | "split-months-csv" => {
            return split_date_range(rest, today, DateRangeSplit::MonthsCsv)
        }
        "按周拆分csv" | "split-weeks-csv" => {
            return split_date_range(rest, today, DateRangeSplit::WeeksCsv)
        }
        "按季拆分csv" | "按季度拆分csv" | "split-quarters-csv" => {
            return split_date_range(rest, today, DateRangeSplit::QuartersCsv)
        }
        "按半年拆分csv" | "split-half-years-csv" => {
            return split_date_range(rest, today, DateRangeSplit::HalfYearsCsv)
        }
        "按年拆分csv" | "split-years-csv" => {
            return split_date_range(rest, today, DateRangeSplit::YearsCsv)
        }
        "逐日csv" | "days-csv" => return split_date_range(rest, today, DateRangeSplit::DaysCsv),
        "星期统计" | "weekday-counts" => return count_weekdays_csv(rest, today),
        _ => {}
    }
    if let Some((year, day)) = year_day_parts(input) {
        let date = day
            .parse()
            .ok()
            .and_then(|day| NaiveDate::from_yo_opt(year, day))
            .ok_or(DateError::InvalidYearDay)?;
        return Ok(date.to_string());
    }
    if let Some((year, month, rule)) = weekday_occurrence_parts(input) {
        return weekday_occurrence(year, month, rule);
    }
    if let Some((year, week)) =
        calendar_period_parts(input, ['W', 'w']).filter(|_| iso_week_date_prefix(input).is_none())
    {
        return iso_week_range(year, week);
    }
    if let Some((year, quarter)) = calendar_period_parts(input, ['Q', 'q']) {
        return quarter_range(year, quarter);
    }
    if let Some(year) = iso_year(input.strip_suffix('年').unwrap_or(input).trim_end()) {
        return year_range(year);
    }
    if let Some((year, month)) = iso_month_parts(input) {
        return month_range(
            NaiveDate::from_ymd_opt(year, month, 1).ok_or(DateError::InvalidDate)?,
            1,
        );
    }
    match relative_period(input) {
        Some(RelativePeriod::Week(offset)) => {
            let days = days_to_week_start(today, offset);
            let monday = today
                .checked_add_signed(TimeDelta::days(days))
                .ok_or(DateError::OutOfRange)?;
            return week_range(monday);
        }
        Some(RelativePeriod::Month(offset)) => {
            return month_range(relative_period_start(today, offset, 1)?, 1);
        }
        Some(RelativePeriod::Quarter(offset)) => {
            return month_range(relative_period_start(today, offset, 3)?, 3);
        }
        Some(RelativePeriod::Year(offset)) => return year_range(today.year() + i32::from(offset)),
        None => {}
    }
    let weekday = weekday_query(input);
    let week_number = week_number_query(input);
    let day_number = day_number_query(input);
    let week_date = week_date_query(input);
    let display_query = weekday.or(week_number).or(day_number).or(week_date);
    let input = display_query.map_or(input, |input| if input.is_empty() { "今天" } else { input });
    let (date, rest) = date_operand(input, today)?;
    let display_date = |date: NaiveDate| {
        if weekday.is_some() {
            Ok(WEEKDAY_LABELS[date.weekday().num_days_from_monday() as usize].to_owned())
        } else if week_number.is_some() || week_date.is_some() {
            let week = date.iso_week();
            let year = week.year();
            if !(0..=9999).contains(&year) {
                return Err(DateError::OutOfRange);
            }
            let format = if week_date.is_some() {
                "%G-W%V-%u"
            } else {
                "%G-W%V"
            };
            Ok(date.format(format).to_string())
        } else if day_number.is_some() {
            Ok(format!("第 {} 天", date.ordinal()))
        } else {
            Ok(date.to_string())
        }
    };
    if rest.is_empty() {
        return display_date(date);
    }
    let subtract = rest.starts_with('-');
    let operand = rest
        .strip_prefix(['+', '-'])
        .ok_or(DateError::InvalidOperation)?
        .trim_start();
    if operand.is_empty() {
        return Err(DateError::Incomplete);
    }
    if subtract
        && (operand.get(4..5) == Some("-")
            || relative_day(operand).is_some()
            || relative_weekday(operand).is_some()
            || relative_period_edge(operand).is_some()
            || written_date_prefix(operand).is_some())
    {
        let (other, suffix) = date_operand(operand, today)?;
        if display_query.is_some() {
            return Err(DateError::InvalidOperation);
        }
        let suffix = suffix.to_ascii_lowercase();
        let (unit, exceptions) = split_workday_exceptions(&suffix);
        return if suffix.is_empty() {
            Ok(format!(
                "{} 天",
                date.signed_duration_since(other).num_days()
            ))
        } else if is_workday_unit(unit) {
            let exceptions = parse_workday_exceptions(exceptions)?;
            let range = (
                Bound::Excluded(date.min(other)),
                Bound::Included(date.max(other)),
            );
            let adjustment = workday_adjustment(&exceptions, range);
            let sign = if date < other { -1 } else { 1 };
            Ok(format!(
                "{} 工作日",
                workday_difference(date, other) + sign * adjustment
            ))
        } else if matches!(suffix.as_str(), "年月日" | "calendar") {
            calendar_difference(date, other)
        } else {
            Err(DateError::InvalidOperation)
        };
    }
    apply_date_offset(date, operand, subtract).and_then(display_date)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_workday_overrides_to_their_months_without_changing_calendar_day_counts() {
        let today = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
        assert_eq!(
            calculate_date(
                r#"按月拆分CSV 2024-01-31 至 2024-03-02 例外 [["2024-01-31","2024-02-03"],["2024-02-01","2024-03-02","2024-04-06"]]"#,
                today
            )
            .as_deref(),
            Ok(concat!(
                "开始,结束,天数,工作日\r\n",
                "2024-01-31,2024-01-31,1,0\r\n",
                "2024-02-01,2024-02-29,29,21\r\n",
                "2024-03-01,2024-03-02,2,2",
            ))
        );
        assert_eq!(
            calculate_date(
                r#"split-months-csv 2024-03-02 to 2024-03-03 EXCEPT [[],["2024-03-02"]]"#,
                today
            )
            .as_deref(),
            Ok("开始,结束,天数,工作日\r\n2024-03-02,2024-03-03,2,1")
        );
        assert_eq!(
            calculate_date(
                r#"按月拆分CSV 2024-03-02 至 2024-03-03 例外 [["2024-03-02"],["2024-03-02"]]"#,
                today
            ),
            Err(DateError::WorkdayExceptions)
        );
    }

    #[test]
    fn exports_iso_week_segments_across_years_and_counts_workday_exceptions() {
        let today = NaiveDate::from_ymd_opt(2026, 12, 30).unwrap();
        assert_eq!(
            calculate_date(
                r#"按周拆分CSV 今天 至 2027-01-05 例外 [["2027-01-01"],["2027-01-02"]]"#,
                today
            )
            .as_deref(),
            Ok(concat!(
                "周号,开始,结束,天数,工作日\r\n",
                "2026-W53,2026-12-30,2027-01-03,5,3\r\n",
                "2027-W01,2027-01-04,2027-01-05,2,2",
            ))
        );
        assert_eq!(
            calculate_date("split-weeks-csv 2027-01-03 至 2027-01-03", today).as_deref(),
            Ok("周号,开始,结束,天数,工作日\r\n2026-W53,2027-01-03,2027-01-03,1,0")
        );
        assert_eq!(
            calculate_date("按周拆分CSV 2027-01-05 至 2027-01-04", today),
            Err(DateError::ReversedDateRange)
        );
        assert_eq!(
            calculate_date("按周拆分CSV 2020-01-01 至 2030-01-01", today),
            Err(DateError::TooManyWeekRanges)
        );
    }

    #[test]
    fn counts_each_weekday_across_complete_weeks_and_a_partial_week() {
        let today = NaiveDate::from_ymd_opt(2024, 2, 29).unwrap();
        assert_eq!(
            calculate_date("星期统计 今天 至 2024-03-09", today).as_deref(),
            Ok(concat!(
                "星期,天数\r\n星期一,1\r\n星期二,1\r\n星期三,1\r\n",
                "星期四,2\r\n星期五,2\r\n星期六,2\r\n星期日,1",
            )),
        );
        assert_eq!(
            calculate_date("WEEKDAY-COUNTS 2024-02-29 to 2024-02-29", today).as_deref(),
            Ok(concat!(
                "星期,天数\r\n星期一,0\r\n星期二,0\r\n星期三,0\r\n",
                "星期四,1\r\n星期五,0\r\n星期六,0\r\n星期日,0",
            )),
        );
        assert_eq!(
            calculate_date("星期统计 0001-01-01 至 9999-12-31", today).as_deref(),
            Ok(concat!(
                "星期,天数\r\n星期一,521723\r\n星期二,521723\r\n星期三,521723\r\n",
                "星期四,521723\r\n星期五,521723\r\n星期六,521722\r\n星期日,521722",
            )),
        );
        assert_eq!(
            calculate_date("星期统计 2024-03-03 至 今天", today),
            Err(DateError::ReversedDateRange),
        );
        assert_eq!(
            calculate_date("星期统计 今天 至 2024-03-03 额外文字", today),
            Err(DateError::WeekdayCountArguments),
        );
    }

    #[test]
    fn exports_every_date_with_its_weekday_and_explicit_workday_classification() {
        let today = NaiveDate::from_ymd_opt(2024, 2, 29).unwrap();
        assert_eq!(
            calculate_date(
                r#"逐日CSV 今天 至 2024-03-03 例外 [["2024-03-01"],["2024-03-02"]]"#,
                today
            )
            .as_deref(),
            Ok(concat!(
                "日期,星期,工作日\r\n",
                "2024-02-29,星期四,1\r\n",
                "2024-03-01,星期五,0\r\n",
                "2024-03-02,星期六,1\r\n",
                "2024-03-03,星期日,0",
            ))
        );
        assert_eq!(
            calculate_date("days-csv 2024-01-01 至 2025-01-01", today),
            Err(DateError::TooManyDates)
        );
        assert_eq!(
            calculate_date("days-csv 9999-12-31 至 9999-12-31", today).as_deref(),
            Ok("日期,星期,工作日\r\n9999-12-31,星期五,1")
        );
    }

    #[test]
    fn lists_month_and_quarter_boundaries_including_the_starting_period() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 22).unwrap();
        for (query, expected) in [
            ("月末序列 2024-01,3", "2024-01-31\n2024-02-29\n2024-03-31"),
            ("month-start-series 本月,2", "2026-11-01\n2026-12-01"),
            ("季初序列 今天,2", "2026-10-01\n2027-01-01"),
            (
                "quarter-end-series 2024-11-20,3",
                "2024-12-31\n2025-03-31\n2025-06-30",
            ),
        ] {
            assert_eq!(
                calculate_date(query, today).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert_eq!(
            calculate_date("月末序列 9999-12,2", today),
            Err(DateError::OutOfRange)
        );
        for query in ["月末序列 今天,0", "季末序列 今天,367", "月初序列"] {
            assert_eq!(
                calculate_date(query, today),
                Err(DateError::DateSeriesArguments),
                "{query}"
            );
        }
    }

    #[test]
    fn uses_relative_quarter_and_year_boundaries_and_keeps_period_queries() {
        let today = NaiveDate::from_ymd_opt(2024, 1, 10).unwrap();
        for (query, expected) in [
            ("上季末", "2023-12-31"),
            ("下季度初 星期几", "星期一"),
            ("明年初", "2025-01-01"),
            ("年底", "2024-12-31"),
            ("去年底 + 1天", "2024-01-01"),
            ("本季度", "2024-01-01 至 2024-03-31\n91 天 · 65 工作日"),
        ] {
            assert!(looks_like_date_query(query), "{query}");
            assert_eq!(
                calculate_date(query, today).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert_eq!(
            calculate_date("按季度拆分CSV 本季度初至本季度末", today).as_deref(),
            Ok("季度,开始,结束,天数,工作日\r\n2024-Q1,2024-01-01,2024-03-31,91,65")
        );
        assert!(!looks_like_date_query("本季初稿"));
        let last_year = NaiveDate::from_ymd_opt(9999, 12, 16).unwrap();
        assert_eq!(
            calculate_date("明年初", last_year),
            Err(DateError::OutOfRange)
        );
    }

    #[test]
    fn uses_relative_month_boundaries_in_date_expressions_and_ranges() {
        let today = NaiveDate::from_ymd_opt(2024, 1, 10).unwrap();
        for (query, expected) in [
            ("月初", "2024-01-01"),
            ("本月底", "2024-01-31"),
            ("上月初", "2023-12-01"),
            ("下月末", "2024-02-29"),
            ("下月底 星期几", "星期四"),
            ("本月初 + 1天", "2024-01-02"),
        ] {
            assert!(looks_like_date_query(query), "{query}");
            assert_eq!(
                calculate_date(query, today).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert_eq!(
            calculate_date("按月拆分CSV 下月初至下月底", today).as_deref(),
            Ok("开始,结束,天数,工作日\r\n2024-02-01,2024-02-29,29,21")
        );
        assert!(!looks_like_date_query("本月初稿"));
        let last_month = NaiveDate::from_ymd_opt(9999, 12, 16).unwrap();
        assert_eq!(
            calculate_date("下月初", last_month),
            Err(DateError::OutOfRange)
        );
    }

    #[test]
    fn exports_year_segments_with_partial_years_leap_days_and_workday_exceptions() {
        let today = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        assert_eq!(
            calculate_date(
                r#"按年拆分CSV 2023-12-30 至 2024-01-02 例外 [["2024-01-01"],["2023-12-30"]]"#,
                today
            )
            .as_deref(),
            Ok(concat!(
                "年份,开始,结束,天数,工作日\r\n",
                "2023,2023-12-30,2023-12-31,2,1\r\n",
                "2024,2024-01-01,2024-01-02,2,1",
            ))
        );
        assert_eq!(
            calculate_date("split-years-csv 2024-01-01 to 2024-12-31", today).as_deref(),
            Ok("年份,开始,结束,天数,工作日\r\n2024,2024-01-01,2024-12-31,366,262")
        );
        assert_eq!(
            calculate_date("按年拆分CSV 2000-01-01 至 2366-01-01", today),
            Err(DateError::TooManyYearRanges)
        );
    }

    #[test]
    fn exports_half_year_segments_with_partial_edges_and_workday_exceptions() {
        let today = NaiveDate::from_ymd_opt(2024, 6, 29).unwrap();
        assert_eq!(
            calculate_date(
                r#"按半年拆分CSV 今天 至 2024-07-02 例外 [["2024-07-01"],["2024-06-30"]]"#,
                today,
            )
            .as_deref(),
            Ok(concat!(
                "半年度,开始,结束,天数,工作日\r\n",
                "2024-H1,2024-06-29,2024-06-30,2,1\r\n",
                "2024-H2,2024-07-01,2024-07-02,2,1",
            )),
        );
        assert_eq!(
            calculate_date("split-half-years-csv 9999-12-31 to 9999-12-31", today).as_deref(),
            Ok("半年度,开始,结束,天数,工作日\r\n9999-H2,9999-12-31,9999-12-31,1,1"),
        );
        assert_eq!(
            calculate_date("按半年拆分CSV 2000-01-01 至 2183-01-01", today),
            Err(DateError::TooManyHalfYearRanges),
        );
    }

    #[test]
    fn exports_quarter_segments_with_inclusive_endpoints_and_workday_exceptions() {
        let today = NaiveDate::from_ymd_opt(2026, 12, 31).unwrap();
        assert_eq!(
            calculate_date(
                r#"按季拆分CSV 今天 至 2027-01-02 例外 [["2027-01-01"],["2027-01-02"]]"#,
                today
            )
            .as_deref(),
            Ok(concat!(
                "季度,开始,结束,天数,工作日\r\n",
                "2026-Q4,2026-12-31,2026-12-31,1,1\r\n",
                "2027-Q1,2027-01-01,2027-01-02,2,1",
            ))
        );
        assert_eq!(
            calculate_date("split-quarters-csv 9999-12-31 to 9999-12-31", today).as_deref(),
            Ok("季度,开始,结束,天数,工作日\r\n9999-Q4,9999-12-31,9999-12-31,1,1")
        );
        assert_eq!(
            calculate_date("按季度拆分CSV 2000-01-01 至 2100-01-01", today),
            Err(DateError::TooManyQuarterRanges)
        );
    }

    #[test]
    fn exports_month_segments_with_inclusive_day_and_workday_counts() {
        let today = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
        assert_eq!(
            calculate_date("按月拆分CSV 2024-01-31 至 2024-03-02", today).as_deref(),
            Ok(concat!(
                "开始,结束,天数,工作日\r\n",
                "2024-01-31,2024-01-31,1,1\r\n",
                "2024-02-01,2024-02-29,29,21\r\n",
                "2024-03-01,2024-03-02,2,1",
            ))
        );
        assert_eq!(
            calculate_date("split-months-csv 2024-03-02 to 2024-03-03", today).as_deref(),
            Ok("开始,结束,天数,工作日\r\n2024-03-02,2024-03-03,2,0")
        );
    }

    #[test]
    fn splits_inclusive_date_ranges_at_month_boundaries_and_keeps_leap_days() {
        let today = NaiveDate::from_ymd_opt(2024, 2, 29).unwrap();
        assert_eq!(
            calculate_date("按月拆分 2024-01-31 至 2024-03-02", today).as_deref(),
            Ok("2024-01-31 至 2024-01-31 · 1 天\n2024-02-01 至 2024-02-29 · 29 天\n2024-03-01 至 2024-03-02 · 2 天")
        );
        assert_eq!(
            calculate_date("split-months 9999-12-31 to 9999-12-31", today).as_deref(),
            Ok("9999-12-31 至 9999-12-31 · 1 天")
        );
        assert_eq!(
            calculate_date("按月拆分 明天至昨天", today),
            Err(DateError::ReversedDateRange)
        );
        assert_eq!(
            calculate_date("按月拆分 2010-01-01 至 2100-01-01", today),
            Err(DateError::TooManyMonthRanges)
        );
    }

    #[test]
    fn merges_contiguous_inclusive_ranges_and_intersects_all_date_ranges() {
        let today = NaiveDate::from_ymd_opt(2024, 2, 29).unwrap();
        for (query, expected) in [
            (
                "合并区间 2026-01-08 至 2026-01-10；2026-01-01 至 2026-01-03；2026-01-04 至 2026-01-06",
                "2026-01-01 至 2026-01-06 · 6 天\n2026-01-08 至 2026-01-10 · 3 天",
            ),
            (
                "区间交集 2024-02-28 到 2024-03-03\n今天 to 2024-03-01\n2024-02-28 至 今天",
                "2024-02-29 至 2024-02-29 · 1 天",
            ),
            ("intersect-ranges 昨天至昨天; 今天至今天", "无重叠日期"),
        ] {
            assert_eq!(calculate_date(query, today).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            calculate_date("合并区间 明天至昨天; 今天至今天", today),
            Err(DateError::ReversedDateRange)
        );
        assert_eq!(
            calculate_date("区间交集 昨天到今天; 昨天到今天 工作日", today),
            Err(DateError::InvalidDateRanges)
        );
        let excessive = format!(
            "合并区间 {}今天至今天",
            "今天至今天;".repeat(MAX_DATE_RANGES)
        );
        assert_eq!(
            calculate_date(&excessive, today),
            Err(DateError::InvalidDateRanges)
        );
    }

    #[test]
    fn subtracts_inclusive_date_ranges_in_order_without_counting_removed_endpoints() {
        let today = NaiveDate::from_ymd_opt(2024, 2, 29).unwrap();
        for (query, expected) in [
            (
                "扣除区间 2024-02-26 至 2024-03-05; 2024-03-04 至 2024-03-08; 今天至2024-03-01; 2024-02-25至昨天; 2024-03-01至2024-03-02",
                "2024-03-03 至 2024-03-03 · 1 天",
            ),
            (
                "subtract-ranges 2024-02-28 to 2024-03-01\n今天至今天",
                "2024-02-28 至 2024-02-28 · 1 天\n2024-03-01 至 2024-03-01 · 1 天",
            ),
            (
                "扣除区间 9999-12-30至9999-12-31; 9999-12-29至9999-12-31",
                "无剩余日期",
            ),
            (
                "扣除区间 今天至今天; 2024-02-27至昨天; 明天至2024-03-02",
                "2024-02-29 至 2024-02-29 · 1 天",
            ),
        ] {
            assert_eq!(calculate_date(query, today).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            calculate_date("扣除区间 今天至明天", today),
            Err(DateError::InvalidDateRanges)
        );
    }

    #[test]
    fn lays_out_a_monday_first_calendar_with_leap_days_and_relative_months() {
        let today = NaiveDate::from_ymd_opt(2024, 1, 31).unwrap();
        let expected = concat!(
            "Mo  Tu  We  Th  Fr  Sa  Su    2024-02\n",
            "             1   2   3   4\n",
            " 5   6   7   8   9  10  11\n",
            "12  13  14  15  16  17  18\n",
            "19  20  21  22  23  24  25\n",
            "26  27  28  29",
        );
        for query in ["月历 2024-02", "月历 下月", "MONTH-GRID next month"] {
            assert_eq!(
                calculate_date(query, today).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        let august = calculate_date("月历 2020-08", today).unwrap();
        assert_eq!(august.lines().count(), 7);
        assert_eq!(august.lines().last(), Some("31"));
        assert_eq!(
            calculate_date("月历 2024-13", today),
            Err(DateError::InvalidMonthCalendar)
        );
    }

    #[test]
    fn formats_and_parses_dates_with_explicit_patterns() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 21).unwrap();
        for (query, expected) in [
            (r#"格式化 ["2026-11-21","%Y%m%d"]"#, "20261121"),
            (
                r#"FORMAT ["今天","（%Y年%m月%d日）"]"#,
                "（2026年11月21日）",
            ),
            (r#"解析 ["20240229","%Y%m%d"]"#, "2024-02-29"),
            (r#"parse-format ["21/11/2026","%d/%m/%Y"]"#, "2026-11-21"),
        ] {
            assert_eq!(calculate_date(query, today).as_deref(), Ok(expected));
        }
        for (query, expected) in [
            (
                r#"解析 ["20230229","%Y%m%d"]"#,
                DateError::DatePatternMismatch,
            ),
            (r#"格式化 ["今天","%Q"]"#, DateError::InvalidDatePattern),
            (r#"格式化 ["今天","%H"]"#, DateError::InvalidDatePattern),
            (r#"格式化 ["今天","\u0000"]"#, DateError::PatternArguments),
            (r#"格式化 ["今天"]"#, DateError::PatternArguments),
        ] {
            assert_eq!(calculate_date(query, today), Err(expected));
        }
    }

    #[test]
    fn finds_workdays_within_a_month_from_either_end_using_exceptions() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 18).unwrap();
        for (query, expected) in [
            ("工作日序号 2026-11 @ 3", "2026-11-04"),
            ("工作日序号 本月 @ -1", "2026-11-30"),
            ("nth-workday 2024-02 @ -2", "2024-02-28"),
            (
                r#"工作日序号 2026-11 @ 3 例外 [["2026-11-02"],[]]"#,
                "2026-11-05",
            ),
            (
                r#"工作日序号 2026-11 @ 1 例外 [[],["2026-11-01"]]"#,
                "2026-11-01",
            ),
        ] {
            assert_eq!(
                calculate_date(query, today).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        for (query, error) in [
            ("工作日序号 2026-11", DateError::InvalidWorkdayOrdinal),
            ("工作日序号 2026-11 @ 0", DateError::InvalidWorkdayOrdinal),
            ("工作日序号 2026-11 @ -32", DateError::InvalidWorkdayOrdinal),
            ("工作日序号 2026-11 @ 31", DateError::MissingWorkdayOrdinal),
            ("工作日序号 2026-13 @ 2", DateError::InvalidMonthCalendar),
        ] {
            assert_eq!(calculate_date(query, today), Err(error), "{query}");
        }
    }

    #[test]
    fn lists_workdays_with_explicit_days_off_and_weekend_makeup_days() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 25).unwrap();
        for query in [
            r#"列出 2026-11-25 到 2026-11-30 工作日 例外 [["2026-11-26"],["2026-11-28"]]"#,
            r#"list 2026-11-25 to 2026-11-30 Workdays EXCEPT [["2026-11-26"],["2026-11-28"]]"#,
        ] {
            assert_eq!(
                calculate_date(query, today).as_deref(),
                Ok("2026-11-25\n2026-11-27\n2026-11-28\n2026-11-30")
            );
        }
        for dates in [
            r#"[["2026-11-26"],["2026-11-26"]]"#,
            r#"[["2026-11-31"],[]]"#,
            r#"[["2026-11-26"]]"#,
        ] {
            assert_eq!(
                calculate_date(
                    &format!("列出 2026-11-25 到 2026-11-30 工作日 例外 {dates}"),
                    today,
                ),
                Err(DateError::WorkdayExceptions)
            );
        }
    }

    #[test]
    fn lists_dates_at_business_day_intervals() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 21).unwrap();
        for (query, expected) in [
            (
                "列出 今天 到 2026-12-01 每隔 3 个工作日",
                "2026-11-23\n2026-11-26\n2026-12-01",
            ),
            (
                "list 2026-11-20 to 2026-11-24 every 2 business days",
                "2026-11-20\n2026-11-24",
            ),
            (
                r#"列出 2026-11-25 到 2026-11-30 每隔 2 个工作日 例外 [["2026-11-26"],["2026-11-28"]]"#,
                "2026-11-25\n2026-11-28",
            ),
            (
                "列出 0000-01-03 到 9999-12-31 每隔 4294967295 工作日",
                "0000-01-03",
            ),
        ] {
            assert_eq!(
                calculate_date(query, today).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert_eq!(
            calculate_date("列出 今天 到 明天 每隔 2 工作日", today),
            Err(DateError::NoMatchingDates)
        );
        assert_eq!(
            calculate_date("列出 今天 到 明天 每隔 0 工作日", today),
            Err(DateError::InvalidDateInterval)
        );
        assert_eq!(
            calculate_date("列出 今天 到 明天 每天 例外 [[],[]]", today),
            Err(DateError::InvalidDateList)
        );
    }

    #[test]
    fn lists_inclusive_date_ranges_filtered_by_day_type() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 21).unwrap();
        for (query, expected) in [
            (
                "列出 2026-11-01 到 2026-11-30 每周一",
                "2026-11-02\n2026-11-09\n2026-11-16\n2026-11-23\n2026-11-30",
            ),
            (
                "list 2026-W47-5 TO 下周一 workdays",
                "2026-11-20\n2026-11-23",
            ),
            (
                "列出 2024年2月28日 至 2024/03/01 每天",
                "2024-02-28\n2024-02-29\n2024-03-01",
            ),
            ("列出 今天 到 明天 周末", "2026-11-21\n2026-11-22"),
            ("列出 9999-12-31 到 9999-12-31 每天", "9999-12-31"),
            ("list 2026-11-01 to 2026-11-03 MON", "2026-11-02"),
            (
                "列出 2026-11-02 到 2026-11-08 每周一、三、五",
                "2026-11-02\n2026-11-04\n2026-11-06",
            ),
            (
                "list 2026-11-02 TO 2026-11-08 FRI, MON, mon, Wed",
                "2026-11-02\n2026-11-04\n2026-11-06",
            ),
            (
                "列出 2026-11-07 至 2026-11-08 周六，星期日",
                "2026-11-07\n2026-11-08",
            ),
            (
                "列出 2024-02-27 到 2024-03-06 每隔 3 天",
                "2024-02-27\n2024-03-01\n2024-03-04",
            ),
            (
                "list 2024-02-27 to 2024-03-06 EVERY 4 days",
                "2024-02-27\n2024-03-02\n2024-03-06",
            ),
            ("列出 今天 到 明天 每隔1000000000天", "2026-11-21"),
            (
                "列出 2024-01-31 到 2024-05-31 每隔 1 个月",
                "2024-01-31\n2024-02-29\n2024-03-31\n2024-04-30\n2024-05-31",
            ),
            (
                "list 2024-02-29 to 2028-03-01 Every 12 months",
                "2024-02-29\n2025-02-28\n2026-02-28\n2027-02-28\n2028-02-29",
            ),
            ("列出 9999-12-31 到 9999-12-31 每月", "9999-12-31"),
        ] {
            assert_eq!(
                calculate_date(query, today).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        for (query, error) in [
            ("列出", DateError::InvalidDateList),
            ("列出 今天 到 明天 每周八", DateError::InvalidDateList),
            ("列出 今天 到 明天 每隔0天", DateError::InvalidDateInterval),
            ("列出 今天 到 明天 每隔-2天", DateError::InvalidDateInterval),
            (
                "列出 今天 到 明天 每隔0个月",
                DateError::InvalidDateInterval,
            ),
            ("列出 今天 到 明天 每周六、", DateError::InvalidDateList),
            (
                "列出 今天 到 明天 每周六、工作日",
                DateError::InvalidDateList,
            ),
            ("列出 今天 到 明天 工作日", DateError::NoMatchingDates),
            ("列出 明天 到 今天 每天", DateError::ReversedDateRange),
            (
                "列出 2024-01-01 到 2025-01-01 每天",
                DateError::TooManyDates,
            ),
            (
                "列出 2024-01-01 到 2027-01-04 每 3 天",
                DateError::TooManyDates,
            ),
            (
                "列出 2000-01-01 到 2035-01-01 每月",
                DateError::TooManyDates,
            ),
        ] {
            assert_eq!(calculate_date(query, today), Err(error), "{query}");
        }
        let year = calculate_date("列出 2024-01-01 到 2024-12-31 每天", today).unwrap();
        assert_eq!(year.lines().count(), MAX_LISTED_DATES);
    }

    #[test]
    fn converts_iso_week_dates_and_supports_them_as_date_operands() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 21).unwrap();
        for (query, expected) in [
            ("2026-W53-5", "2027-01-01"),
            ("２０２６-ｗ４７-６", "2026-11-21"),
            ("2026-W47-6 星期几", "星期六"),
            ("2026-W47-6 + 2天", "2026-11-23"),
            ("2026-W47-6 - 2026-W47-4", "2 天"),
            ("2027-01-01 周日期", "2026-W53-5"),
            ("周日期", "2026-W47-6"),
        ] {
            assert!(looks_like_date_query(query), "{query}");
            assert_eq!(
                calculate_date(query, today).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        for query in ["2027-W53-1", "2026-W47-0", "2026-W47-8"] {
            assert_eq!(
                calculate_date(query, today),
                Err(DateError::InvalidWeek),
                "{query}"
            );
        }
    }

    #[test]
    fn converts_between_dates_and_one_based_days_of_the_year() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 21).unwrap();
        for (query, expected) in [
            ("2024 第60天", "2024-02-29"),
            ("２０２４年 第 366 天", "2024-12-31"),
            ("2024-02-29 第几天", "第 60 天"),
            ("2024-02-28 + 1天 第几天", "第 60 天"),
            ("第几天", "第 325 天"),
        ] {
            assert!(looks_like_date_query(query), "{query}");
            assert_eq!(
                calculate_date(query, today).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        for query in ["2024 第0天", "2023 第366天", "2024 第367天"] {
            assert_eq!(
                calculate_date(query, today),
                Err(DateError::InvalidYearDay),
                "{query}"
            );
        }
    }

    #[test]
    fn shows_absolute_and_relative_years_with_actual_day_counts() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 21).unwrap();
        for (query, expected) in [
            ("2024", "2024-01-01 至 2024-12-31\n366 天 · 262 工作日"),
            (
                "２０２４ 年",
                "2024-01-01 至 2024-12-31\n366 天 · 262 工作日",
            ),
            ("今年", "2026-01-01 至 2026-12-31\n365 天 · 261 工作日"),
            ("去年", "2025-01-01 至 2025-12-31\n365 天 · 261 工作日"),
            ("明年", "2027-01-01 至 2027-12-31\n365 天 · 261 工作日"),
        ] {
            assert_eq!(
                calculate_date(query, today).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert!(looks_like_date_query("２０２４年"));
        assert!(looks_like_date_query("Next Year"));
        assert!(!looks_like_date_query("2024"));
        assert_eq!(
            calculate_date("明年", NaiveDate::from_ymd_opt(9999, 12, 1).unwrap()),
            Err(DateError::OutOfRange)
        );
    }

    #[test]
    fn finds_the_workday_position_from_both_ends_of_the_month() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 4).unwrap();
        for (input, expected) in [
            (
                "工作日位置 今天",
                "2026-11-04 · 当月第 3 个工作日\n倒数第 19 个工作日",
            ),
            ("WORKDAY-POSITION 2026-11-07", "2026-11-07 · 休息日"),
            (
                r#"工作日位置 2026-11-07 例外 [["2026-11-02"],["2026-11-07"]]"#,
                "2026-11-07 · 当月第 5 个工作日\n倒数第 17 个工作日",
            ),
            (
                r#"工作日位置 2026-11-04 例外 [["2026-11-04"],[]]"#,
                "2026-11-04 · 休息日",
            ),
            (
                "工作日位置 2026-11-30",
                "2026-11-30 · 当月第 21 个工作日\n倒数第 1 个工作日",
            ),
        ] {
            assert_eq!(
                calculate_date(input, today).as_deref(),
                Ok(expected),
                "{input}"
            );
        }
        assert_eq!(
            calculate_date("工作日位置 今天 + 1天", today),
            Err(DateError::InvalidWorkdayPosition),
        );
    }

    #[test]
    fn generates_workday_ordinals_across_months_with_shared_exceptions() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 22).unwrap();
        for (input, expected) in [
            (
                "工作日序列 本月,3 @ -1",
                "2026-11-30\n2026-12-31\n2027-01-29",
            ),
            ("WORKDAY-SERIES 2024-02,2 @ 1", "2024-02-01\n2024-03-01"),
            (
                r#"工作日序列 2026-11,3 @ -1 例外 [["2026-12-31"],["2027-01-30"]]"#,
                "2026-11-30\n2026-12-30\n2027-01-30",
            ),
        ] {
            assert_eq!(
                calculate_date(input, today).as_deref(),
                Ok(expected),
                "{input}"
            );
        }
        for input in [
            "工作日序列 2026-11 @ -1",
            "工作日序列 2026-11,0 @ -1",
            "工作日序列 2026-11,367 @ -1",
        ] {
            assert_eq!(
                calculate_date(input, today),
                Err(DateError::InvalidWorkdaySeries)
            );
        }
        assert_eq!(
            calculate_date("工作日序列 2026-11,2 @ 31", today),
            Err(DateError::MissingWorkdayOrdinal),
        );
        assert_eq!(
            calculate_date("工作日序列 9999-12,2 @ 1", today),
            Err(DateError::OutOfRange),
        );
    }

    #[test]
    fn finds_the_nth_or_last_weekday_of_a_month() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 21).unwrap();
        for (query, expected) in [
            ("2026-11 第4个星期四", "2026-11-26"),
            ("2024-02 第５个周四", "2024-02-29"),
            ("2026-11 第1个星期日", "2026-11-01"),
            ("2026-11第1个周一", "2026-11-02"),
            ("2026-11 第 4 个星期四", "2026-11-26"),
            ("2026-11 最后一个星期日", "2026-11-29"),
            ("2026-11最后一个周一", "2026-11-30"),
            ("2024-02 最后一个周四", "2024-02-29"),
        ] {
            assert!(looks_like_date_query(query), "{query}");
            assert_eq!(
                calculate_date(query, today).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        for (query, error) in [
            ("2026-11 第", DateError::Incomplete),
            ("2026-11 最后", DateError::Incomplete),
            ("2026-11 第5个星期四", DateError::MissingWeekdayOccurrence),
            ("2026-11 第0个周四", DateError::InvalidWeekdayOccurrence),
            ("2026-11 第4个周八", DateError::InvalidWeekdayOccurrence),
            ("2026-13 第4个星期四", DateError::InvalidDate),
            ("2026-13 最后一个星期四", DateError::InvalidDate),
        ] {
            assert_eq!(calculate_date(query, today), Err(error), "{query}");
        }
    }

    #[test]
    fn recognizes_chinese_dates_and_slash_dates_in_calendar_queries() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 21).unwrap();
        for (query, expected) in [
            ("2024年2月29日", "2024-02-29"),
            ("２０２６年１１月２１日 星期几", "星期六"),
            ("2026/11/21 周几", "星期六"),
            ("2026/11/21 - 2026年11月1日", "20 天"),
            ("2026年11月21日 - 2026/11/1", "20 天"),
            ("2026/1/1 + 2天", "2026-01-03"),
        ] {
            assert_eq!(
                calculate_date(query, today).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert!(looks_like_date_query("2024年2月29日"));
        assert!(looks_like_date_query("2026/11/21 星期几"));
        assert!(!looks_like_date_query("2026/11/21"));
        assert_eq!(
            calculate_date("2025年2月29日", today),
            Err(DateError::InvalidDate)
        );
        assert_eq!(
            calculate_date("2026/11/32", today),
            Err(DateError::InvalidDate)
        );
    }

    #[test]
    fn reports_relative_weeks_using_local_monday_across_years() {
        let today = NaiveDate::from_ymd_opt(2027, 1, 1).unwrap();
        for (query, expected) in [
            ("本周", "2026-12-28 至 2027-01-03"),
            ("上星期", "2026-12-21 至 2026-12-27"),
            ("NEXT WEEK", "2027-01-04 至 2027-01-10"),
            ("本周一", "2026-12-28"),
            ("上星期五", "2026-12-25"),
            ("这周天", "2027-01-03"),
            ("下周五", "2027-01-08"),
            ("下周五 星期几", "星期五"),
            ("下周三 + 2天", "2027-01-08"),
            ("下周五 - 本周五", "7 天"),
        ] {
            assert!(looks_like_date_query(query), "{query}");
            assert_eq!(
                calculate_date(query, today).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        let end = NaiveDate::from_ymd_opt(9999, 12, 31).unwrap();
        assert_eq!(calculate_date("下周", end), Err(DateError::OutOfRange));
    }

    #[test]
    fn finds_relative_calendar_quarters_across_year_boundaries() {
        let today = NaiveDate::from_ymd_opt(2024, 1, 31).unwrap();
        for (query, expected) in [
            ("上季度", "2023-10-01 至 2023-12-31\n92 天 · 65 工作日"),
            ("本季度", "2024-01-01 至 2024-03-31\n91 天 · 65 工作日"),
            (
                "NEXT QUARTER",
                "2024-04-01 至 2024-06-30\n91 天 · 65 工作日",
            ),
        ] {
            assert!(looks_like_date_query(query));
            assert_eq!(calculate_date(query, today).as_deref(), Ok(expected));
        }
        for (query, today) in [
            ("上季度", NaiveDate::from_ymd_opt(0, 1, 1).unwrap()),
            ("下季度", NaiveDate::from_ymd_opt(9999, 12, 31).unwrap()),
        ] {
            assert_eq!(calculate_date(query, today), Err(DateError::OutOfRange));
        }
    }

    #[test]
    fn checks_calendar_quarters_across_leap_years_and_year_boundaries() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 21).unwrap();
        for (query, expected) in [
            ("2024-Q1", "2024-01-01 至 2024-03-31\n91 天 · 65 工作日"),
            (
                "２０２６－ｑ４",
                "2026-10-01 至 2026-12-31\n92 天 · 66 工作日",
            ),
        ] {
            assert!(looks_like_date_query(query));
            assert_eq!(
                calculate_date(query, today).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert_eq!(calculate_date("2026-Q", today), Err(DateError::Incomplete));
        assert_eq!(
            calculate_date("2026-q5", today),
            Err(DateError::InvalidQuarter)
        );
        assert_eq!(
            calculate_date("2026-Q02", today),
            Err(DateError::InvalidQuarter)
        );
        assert!(calculate_date("9999-Q4", today)
            .unwrap()
            .starts_with("9999-10-01 至 9999-12-31"));
    }

    #[test]
    fn displays_month_ranges_with_inclusive_weekday_counts() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 21).unwrap();
        for (query, expected) in [
            (
                "２０２４－０２",
                "2024-02-01 至 2024-02-29\n29 天 · 21 工作日",
            ),
            ("2026-02", "2026-02-01 至 2026-02-28\n28 天 · 20 工作日"),
            ("本月", "2026-11-01 至 2026-11-30\n30 天 · 21 工作日"),
            ("LAST MONTH", "2026-10-01 至 2026-10-31\n31 天 · 22 工作日"),
            ("下个月", "2026-12-01 至 2026-12-31\n31 天 · 23 工作日"),
        ] {
            assert_eq!(
                calculate_date(query, today).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert!(!looks_like_date_query("2024-02"));
        assert!(looks_like_date_query("本月"));
        assert_eq!(
            calculate_date("2024-13", today),
            Err(DateError::InvalidDate)
        );
        assert!(calculate_date("9999-12", today)
            .unwrap()
            .starts_with("9999-12-01 至 9999-12-31"));
        let last_month = NaiveDate::from_ymd_opt(9999, 12, 15).unwrap();
        assert_eq!(
            calculate_date("下月", last_month),
            Err(DateError::OutOfRange)
        );
    }

    #[test]
    fn recognizes_unambiguous_calendar_requests_without_a_prefix() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 21).unwrap();
        for (input, expected) in [
            ("前天", "2026-11-19"),
            ("后天 周几", "星期一"),
            ("TOMORROW + 1周", "2026-11-29"),
            ("2026-11-21 星期几", "星期六"),
            ("2026-W47", "2026-11-16 至 2026-11-22"),
            ("２０２６－Ｗ４７", "2026-11-16 至 2026-11-22"),
            ("第几周", "2026-W47"),
        ] {
            assert!(looks_like_date_query(input), "{input}");
            assert_eq!(
                calculate_date(input, today).as_deref(),
                Ok(expected),
                "{input}"
            );
        }
        for input in [
            "2026-11-21",
            "2026-11-21 + 1",
            "2026-West",
            "明天提醒",
            "会议星期几",
            "todayish",
        ] {
            assert!(!looks_like_date_query(input), "{input}");
        }
    }

    #[test]
    fn converts_dates_and_ranges_to_and_from_iso_weeks_across_year_boundaries() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 21).unwrap();
        for (input, expected) in [
            ("2027-01-01 第几周", "2026-W53"),
            ("2025-12-29 周数", "2026-W01"),
            ("第几周", "2026-W47"),
            ("今天 + 1周 第几周", "2026-W48"),
            ("2026-W47", "2026-11-16 至 2026-11-22"),
            ("2026-w53", "2026-12-28 至 2027-01-03"),
            ("２０２６－Ｗ０１", "2025-12-29 至 2026-01-04"),
        ] {
            assert_eq!(
                calculate_date(input, today).as_deref(),
                Ok(expected),
                "{input}"
            );
        }
        for (input, error) in [
            ("2025-W53", DateError::InvalidWeek),
            ("2026-W00", DateError::InvalidWeek),
            ("2026-W", DateError::Incomplete),
            ("9999-W52", DateError::OutOfRange),
            ("0000-01-01 第几周", DateError::OutOfRange),
        ] {
            assert_eq!(calculate_date(input, today), Err(error), "{input}");
        }
    }

    #[test]
    fn adjusts_workday_arithmetic_in_both_directions_using_explicit_exceptions() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 21).unwrap();
        let exceptions = r#"例外 [["2026-11-23","2026-11-26"],["2026-11-21","2026-11-28"]]"#;
        for (input, expected) in [
            ("2026-11-20 + 1工作日", "2026-11-21"),
            ("2026-11-27 + 2工作日", "2026-11-30"),
            ("2026-11-24 - 1工作日", "2026-11-21"),
            ("2026-11-24 - 0工作日", "2026-11-24"),
            ("2026-11-27 - 2026-11-24 工作日", "2 工作日"),
            ("2026-11-24 - 2026-11-27 工作日", "-2 工作日"),
            ("今天 - 今天 工作日", "0 工作日"),
            ("2026-11-01 - 本月初 工作日", "0 工作日"),
        ] {
            let query = format!("{input} {exceptions}");
            assert_eq!(
                calculate_date(&query, today).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert_eq!(
            calculate_date(
                r#"2026-11-20 + 1 BUSINESS DAY EXCEPT [["2026-11-23"],[]]"#,
                today
            )
            .as_deref(),
            Ok("2026-11-24"),
        );
        for query in [
            r#"今天 + 1工作日 例外 [["2026-11-23"],["2026-11-23"]]"#,
            "明天 - 今天 工作日 例外 null",
        ] {
            assert_eq!(
                calculate_date(query, today),
                Err(DateError::WorkdayExceptions)
            );
        }
        assert_eq!(
            calculate_date("今天 + 1天 例外 [[],[]]", today),
            Err(DateError::InvalidOperation)
        );
    }

    #[test]
    fn shifts_workdays_in_both_directions_and_excludes_weekends() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 21).unwrap();
        for (query, expected) in [
            ("2026-11-20 + 1工作日", "2026-11-23"),
            ("今天 + 5个工作日", "2026-11-27"),
            ("2026-11-22 - 1 business day", "2026-11-20"),
            ("2026-11-23 + 9 business days", "2026-12-04"),
            ("今天 + 0工作日", "2026-11-21"),
            ("2024-02-28 + 2 workdays", "2024-03-01"),
        ] {
            assert_eq!(
                calculate_date(query, today).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert_eq!(
            calculate_date("9999-12-31 + 1工作日", today),
            Err(DateError::OutOfRange)
        );
        assert_eq!(
            calculate_date("今天 + 18446744073709551615工作日", today),
            Err(DateError::OutOfRange)
        );
    }

    #[test]
    fn decomposes_date_differences_into_calendar_months_then_days() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 21).unwrap();
        for (query, expected) in [
            ("2025-03-15 - 2024-01-31 年月日", "1 年 1 个月 15 天"),
            ("2024-01-31 - 2025-03-15 年月日", "-1 年 1 个月 15 天"),
            ("2024-02-29 - 2024-01-31 年月日", "0 年 1 个月 0 天"),
            ("2025-02-28 - 2024-02-29 CALENDAR", "1 年 0 个月 0 天"),
            ("今天 - 昨天 年月日", "0 年 0 个月 1 天"),
            ("今天 - 今天 年月日", "0 年 0 个月 0 天"),
        ] {
            assert_eq!(
                calculate_date(query, today).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
    }

    #[test]
    fn counts_signed_workday_differences_from_the_day_after_the_earlier_date() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 21).unwrap();
        for (query, expected) in [
            ("2026-11-23 - 2026-11-20 工作日", "1 工作日"),
            ("2026-11-20 - 2026-11-23 WORKDAYS", "-1 工作日"),
            ("2026-11-22 - 2026-11-20 business days", "0 工作日"),
            ("2026-11-30 - 2026-11-20 工作日", "6 工作日"),
            ("今天 - 昨天 个工作日", "0 工作日"),
            ("明天 - 明天 工作日", "0 工作日"),
            ("2024-03-01 - 2024-02-28 工作日", "2 工作日"),
        ] {
            assert_eq!(
                calculate_date(query, today).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
    }

    #[test]
    fn date_arithmetic_handles_day_offsets_and_signed_differences() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 21).unwrap();
        for (expression, expected) in [
            ("2026-11-21 + 14d", "2026-12-05"),
            ("2024-02-28 + 1天", "2024-02-29"),
            ("2024-03-01 - 1 d", "2024-02-29"),
            ("今天＋１天", "2026-11-22"),
            ("TODAY", "2026-11-21"),
            ("2026-11-20 − today", "-1 天"),
            ("今天 - 2026-11-01", "20 天"),
            ("今天 + 2周", "2026-12-05"),
            ("2026-11-21 - 1W", "2026-11-14"),
            ("今天 - 昨天", "1 天"),
            ("TOMORROW + 1天", "2026-11-23"),
            ("昨天周几", "星期五"),
            ("2026-11-21 weekday", "星期六"),
            ("明天 + 1天 星期几", "星期一"),
            ("星期几", "星期六"),
        ] {
            assert_eq!(
                calculate_date(expression, today),
                Ok(expected.to_owned()),
                "{expression}"
            );
        }
        assert_eq!(
            calculate_date("2026-11-21 +", today),
            Err(DateError::Incomplete)
        );
        assert_eq!(
            calculate_date("2026-02-29", today),
            Err(DateError::InvalidDate)
        );
        assert_eq!(
            calculate_date("2026-11-21 + 1.5d", today),
            Err(DateError::InvalidOperation)
        );
        assert_eq!(
            calculate_date("9999-12-31 +1d", today),
            Err(DateError::OutOfRange)
        );
        assert_eq!(
            calculate_date("2026-11-21 + 18446744073709551615w", today),
            Err(DateError::OutOfRange)
        );
    }

    #[test]
    fn calendar_months_and_years_preserve_day_or_use_the_end_of_the_month() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 21).unwrap();
        for (query, expected) in [
            ("2024-01-31 + 1个月", "2024-02-29"),
            ("2023-01-31 + 1 MONTH", "2023-02-28"),
            ("2024-03-31 - 1mo", "2024-02-29"),
            ("2024-02-29 + 1年", "2025-02-28"),
            ("2024-02-29 + 4 years", "2028-02-29"),
            ("2024-02-29 - 1Y 星期几", "星期二"),
            ("今天 + 2 WEEKS", "2026-12-05"),
        ] {
            assert_eq!(
                calculate_date(query, today).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert_eq!(
            calculate_date("9999-12-31 + 1月", today),
            Err(DateError::OutOfRange)
        );
        assert_eq!(
            calculate_date("2024-01-31 + 1.5月", today),
            Err(DateError::InvalidOperation)
        );
    }
}
