use chrono::{Datelike, Days, Months, NaiveDate, TimeDelta, Weekday};

use super::calculator::normalize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DateError {
    Incomplete,
    InvalidDate,
    InvalidWeek,
    InvalidQuarter,
    InvalidOperation,
    OutOfRange,
}

impl DateError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "可以输入日期、天数或星期几，例如 日期: 今天 星期几",
            Self::InvalidDate => "请输入有效日期 YYYY-MM-DD 或月份 YYYY-MM",
            Self::InvalidWeek => "请输入有效的 ISO 周号，例如 2026-W47",
            Self::InvalidQuarter => "请输入季度编号 1 至 4，例如 2026-Q4",
            Self::InvalidOperation => "可查星期几、加减日期或工作日，或用两个日期相减",
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

fn date_operand(input: &str, today: NaiveDate) -> Result<(NaiveDate, &str), DateError> {
    if let Some((offset, rest)) = relative_day(input) {
        let date = today.checked_add_signed(TimeDelta::days(i64::from(offset)));
        return Ok((in_range(date)?, rest));
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

fn calendar_period_parts(input: &str, marker: [char; 2]) -> Option<(&str, &str)> {
    let (year, rest) = input.split_at_checked(4)?;
    Some((year, rest.strip_prefix('-')?.strip_prefix(marker)?))
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

fn relative_month(input: &str) -> Option<i8> {
    match input.to_ascii_lowercase().as_str() {
        "本月" | "这个月" | "this month" => Some(0),
        "上月" | "上个月" | "last month" => Some(-1),
        "下月" | "下个月" | "next month" => Some(1),
        _ => None,
    }
}

pub(super) fn looks_like_date_query(query: &str) -> bool {
    let input = normalize(query);
    let input = input.trim();
    let date_tail = |rest: &str| rest.is_empty() || rest.starts_with(['+', '-']);
    if let Some((year, ordinal)) = calendar_period_parts(input, ['W', 'w'])
        .or_else(|| calendar_period_parts(input, ['Q', 'q']))
    {
        return year.bytes().all(|byte| byte.is_ascii_digit())
            && ordinal.bytes().all(|byte| byte.is_ascii_digit());
    }
    if let Some(date) = weekday_query(input).or_else(|| week_number_query(input)) {
        return date.is_empty()
            || relative_day(date).is_some_and(|(_, rest)| date_tail(rest))
            || iso_date_prefix(date).is_some_and(|(_, rest)| date_tail(rest));
    }
    relative_month(input).is_some() || relative_day(input).is_some_and(|(_, rest)| date_tail(rest))
}

fn is_workday_unit(unit: &str) -> bool {
    matches!(
        unit,
        "工作日" | "个工作日" | "business day" | "business days" | "workday" | "workdays"
    )
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
    let start = in_range(Some(monday))?;
    let end = in_range(monday.checked_add_days(Days::new(6)))?;
    Ok(format!("{start} 至 {end}"))
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

fn month_range(start: NaiveDate, months: u32) -> Result<String, DateError> {
    let end = in_range(
        start
            .checked_add_months(Months::new(months))
            .and_then(|next| next.pred_opt()),
    )?;
    let workdays =
        workday_difference(end, start) + i64::from(start.weekday().number_from_monday() <= 5);
    let days = end.signed_duration_since(start).num_days() + 1;
    Ok(format!("{start} 至 {end}\n{days} 天 · {workdays} 工作日"))
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
    let unit = unit.trim().to_ascii_lowercase();
    if is_workday_unit(&unit) {
        return in_range(shift_workdays(date, count()?, subtract));
    }
    let (is_months, multiplier) = match unit.as_str() {
        "" | "天" | "d" | "day" | "days" => (false, 1),
        "周" | "星期" | "w" | "week" | "weeks" => (false, 7),
        "月" | "个月" | "mo" | "month" | "months" => (true, 1),
        "年" | "y" | "yr" | "year" | "years" => (true, 12),
        _ => return Err(DateError::InvalidOperation),
    };
    let amount = count()?
        .checked_mul(multiplier)
        .ok_or(DateError::OutOfRange)?;
    let adjusted = if is_months {
        let months = u32::try_from(amount)
            .map(Months::new)
            .map_err(|_| DateError::OutOfRange)?;
        if subtract {
            date.checked_sub_months(months)
        } else {
            date.checked_add_months(months)
        }
    } else {
        let days = Days::new(amount);
        if subtract {
            date.checked_sub_days(days)
        } else {
            date.checked_add_days(days)
        }
    };
    in_range(adjusted)
}

pub(super) fn calculate_date(input: &str, today: NaiveDate) -> Result<String, DateError> {
    let input = normalize(input);
    let input = input.trim();
    if let Some((year, week)) = calendar_period_parts(input, ['W', 'w']) {
        return iso_week_range(year, week);
    }
    if let Some((year, quarter)) = calendar_period_parts(input, ['Q', 'q']) {
        return quarter_range(year, quarter);
    }
    if let Some((year, month)) = iso_month_parts(input) {
        return month_range(
            NaiveDate::from_ymd_opt(year, month, 1).ok_or(DateError::InvalidDate)?,
            1,
        );
    }
    if let Some(offset) = relative_month(input) {
        let current = in_range(today.with_day(1))?;
        let start = match offset {
            -1 => current.checked_sub_months(Months::new(1)),
            1 => current.checked_add_months(Months::new(1)),
            _ => Some(current),
        };
        return month_range(in_range(start)?, 1);
    }
    let weekday = weekday_query(input);
    let week_number = week_number_query(input);
    let display_query = weekday.or(week_number);
    let input = display_query.map_or(input, |input| if input.is_empty() { "今天" } else { input });
    let (date, rest) = date_operand(input, today)?;
    let display_date = |date: NaiveDate| {
        const DAYS: [&str; 7] = [
            "星期一",
            "星期二",
            "星期三",
            "星期四",
            "星期五",
            "星期六",
            "星期日",
        ];
        if weekday.is_some() {
            Ok(DAYS[date.weekday().num_days_from_monday() as usize].to_owned())
        } else if week_number.is_some() {
            let week = date.iso_week();
            let year = week.year();
            if !(0..=9999).contains(&year) {
                return Err(DateError::OutOfRange);
            }
            Ok(format!("{year:04}-W{:02}", week.week()))
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
    if subtract && (operand.get(4..5) == Some("-") || relative_day(operand).is_some()) {
        let (other, suffix) = date_operand(operand, today)?;
        if display_query.is_some() {
            return Err(DateError::InvalidOperation);
        }
        let suffix = suffix.to_ascii_lowercase();
        return if suffix.is_empty() {
            Ok(format!(
                "{} 天",
                date.signed_duration_since(other).num_days()
            ))
        } else if is_workday_unit(&suffix) {
            Ok(format!("{} 工作日", workday_difference(date, other)))
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
