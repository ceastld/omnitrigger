use chrono::{Datelike, Days, NaiveDate};

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
            Self::Incomplete => "可以输入日期和天数，例如 日期: 2026-11-21 + 14d",
            Self::InvalidDate => "请输入有效日期，格式为 YYYY-MM-DD",
            Self::InvalidOperation => "可加减整数天数，或用两个日期相减",
            Self::OutOfRange => "结果超出四位年份的日期范围",
        }
    }
}

fn date_operand(input: &str, today: NaiveDate) -> Result<(NaiveDate, &str), DateError> {
    if let Some(rest) = input.strip_prefix("今天").or_else(|| {
        input
            .split_at_checked(5)
            .filter(|(head, _)| head.eq_ignore_ascii_case("today"))
            .map(|(_, rest)| rest)
    }) {
        return Ok((today, rest.trim()));
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

pub(super) fn calculate_date(input: &str, today: NaiveDate) -> Result<String, DateError> {
    let input = normalize(input);
    let (date, rest) = date_operand(input.trim(), today)?;
    if rest.is_empty() {
        return Ok(date.to_string());
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
            || operand.starts_with("今天")
            || operand.eq_ignore_ascii_case("today"))
    {
        let (other, suffix) = date_operand(operand, today)?;
        if !suffix.is_empty() {
            return Err(DateError::InvalidOperation);
        }
        return Ok(format!(
            "{} 天",
            date.signed_duration_since(other).num_days()
        ));
    }
    let days = operand
        .strip_suffix(['d', 'D', '天'])
        .unwrap_or(operand)
        .trim_end();
    if days.is_empty() || !days.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(DateError::InvalidOperation);
    }
    let days = Days::new(days.parse().map_err(|_| DateError::OutOfRange)?);
    let adjusted = if subtract {
        date.checked_sub_days(days)
    } else {
        date.checked_add_days(days)
    };
    adjusted
        .filter(|date| (0..=9999).contains(&date.year()))
        .map(|date| date.to_string())
        .ok_or(DateError::OutOfRange)
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
    }
}
