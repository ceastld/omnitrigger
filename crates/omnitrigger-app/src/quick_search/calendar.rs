use chrono::{Datelike, Days, Months, NaiveDate};

use super::calculator::normalize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DateError {
    Incomplete,
    InvalidDate,
    InvalidOperation,
    OutOfRange,
}

impl DateError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "可以输入日期、天数或星期几，例如 日期: 今天 星期几",
            Self::InvalidDate => "请输入有效日期，格式为 YYYY-MM-DD",
            Self::InvalidOperation => "可查星期几、按天周月年加减，或用两个日期相减",
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
        ("明天", 1),
        ("tomorrow", 1),
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

fn date_operand(input: &str, today: NaiveDate) -> Result<(NaiveDate, &str), DateError> {
    if let Some((offset, rest)) = relative_day(input) {
        let date = match offset {
            -1 => today.pred_opt(),
            1 => today.succ_opt(),
            _ => Some(today),
        };
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
    let (date, rest) = input.split_at_checked(10).ok_or(DateError::InvalidDate)?;
    if !date.bytes().enumerate().all(|(index, byte)| match index {
        4 | 7 => byte == b'-',
        _ => byte.is_ascii_digit(),
    }) {
        return Err(DateError::InvalidDate);
    }
    let date = NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|_| DateError::InvalidDate)?;
    Ok((date, rest.trim()))
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

fn apply_date_offset(
    date: NaiveDate,
    operand: &str,
    subtract: bool,
) -> Result<NaiveDate, DateError> {
    let unit_start = operand
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(operand.len());
    let (amount, unit) = operand.split_at(unit_start);
    let (is_months, multiplier) = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "天" | "d" | "day" | "days" => (false, 1),
        "周" | "星期" | "w" | "week" | "weeks" => (false, 7),
        "月" | "个月" | "mo" | "month" | "months" => (true, 1),
        "年" | "y" | "yr" | "year" | "years" => (true, 12),
        _ => return Err(DateError::InvalidOperation),
    };
    if amount.is_empty() {
        return Err(DateError::InvalidOperation);
    }
    let amount = amount
        .parse::<u64>()
        .ok()
        .and_then(|amount| amount.checked_mul(multiplier))
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
    let weekday = weekday_query(input);
    let input = weekday.map_or(input, |input| if input.is_empty() { "今天" } else { input });
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
            DAYS[date.weekday().num_days_from_monday() as usize].to_owned()
        } else {
            date.to_string()
        }
    };
    if rest.is_empty() {
        return Ok(display_date(date));
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
        if !suffix.is_empty() || weekday.is_some() {
            return Err(DateError::InvalidOperation);
        }
        return Ok(format!(
            "{} 天",
            date.signed_duration_since(other).num_days()
        ));
    }
    apply_date_offset(date, operand, subtract).map(display_date)
}

#[cfg(test)]
mod tests {
    use super::*;

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
