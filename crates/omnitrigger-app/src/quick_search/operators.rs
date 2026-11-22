use std::cmp::Ordering;

use super::calculator::CalculationError;

pub(super) const MAX_SORT_FIELDS: usize = 8;

pub(super) fn json_argument_and_tail<T: serde::de::DeserializeOwned>(
    input: &str,
) -> Option<(T, &str)> {
    let input = input.trim_start();
    let mut values = serde_json::Deserializer::from_str(input).into_iter::<T>();
    let value = values.next()?.ok()?;
    let remaining = &input[values.byte_offset()..];
    let data = remaining
        .strip_prefix("\r\n")
        .or_else(|| remaining.strip_prefix(char::is_whitespace))?;
    Some((value, data))
}

#[derive(Clone, Copy, serde::Deserialize)]
pub(super) enum FieldSortOrder {
    #[serde(rename = "文本升序", alias = "text-asc")]
    TextAscending,
    #[serde(rename = "文本降序", alias = "text-desc")]
    TextDescending,
    #[serde(rename = "数值升序", alias = "numeric-asc")]
    NumberAscending,
    #[serde(rename = "数值降序", alias = "numeric-desc")]
    NumberDescending,
}

impl FieldSortOrder {
    pub(super) fn descending(self) -> bool {
        matches!(self, Self::TextDescending | Self::NumberDescending)
    }

    pub(super) fn numeric(self) -> bool {
        matches!(self, Self::NumberAscending | Self::NumberDescending)
    }
}

pub(super) enum IndexRangeError {
    Invalid,
    OutOfBounds,
}

pub(super) fn select_index_ranges(
    ranges: &str,
    length: usize,
) -> Result<Vec<bool>, IndexRangeError> {
    let mut selected = vec![false; length];
    let parse_index = |number: &str| {
        number
            .parse::<usize>()
            .ok()
            .filter(|&index| index > 0)
            .ok_or(IndexRangeError::Invalid)
    };
    for range in ranges.split(',') {
        let (first, last) = range.split_once('-').unwrap_or((range, range));
        let start = parse_index(first)?;
        let end = parse_index(last)?;
        if end < start {
            return Err(IndexRangeError::Invalid);
        }
        selected
            .get_mut(start - 1..end)
            .ok_or(IndexRangeError::OutOfBounds)?
            .fill(true);
    }
    Ok(selected)
}

pub(super) fn split_top_level_commas(input: &str) -> impl Iterator<Item = &str> {
    let mut depth = 0usize;
    input.split(move |ch| {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ => {}
        }
        depth == 0 && ch == ','
    })
}

#[derive(Clone, Copy)]
pub(super) enum NumberComparison {
    Less,
    LessOrEqual,
    Equal,
    Different,
    GreaterOrEqual,
    Greater,
}

impl NumberComparison {
    pub(super) fn parse(operator: &str) -> Option<Self> {
        match operator {
            "<" => Some(Self::Less),
            "<=" | "≤" => Some(Self::LessOrEqual),
            "=" | "==" => Some(Self::Equal),
            "!=" | "≠" => Some(Self::Different),
            ">=" | "≥" => Some(Self::GreaterOrEqual),
            ">" => Some(Self::Greater),
            _ => None,
        }
    }

    pub(super) fn accepts(self, order: Ordering) -> bool {
        match self {
            Self::Less => order.is_lt(),
            Self::LessOrEqual => order.is_le(),
            Self::Equal => order.is_eq(),
            Self::Different => !order.is_eq(),
            Self::GreaterOrEqual => order.is_ge(),
            Self::Greater => order.is_gt(),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum Operator {
    Add,
    Subtract,
    Multiply,
    Divide,
    FloorDivide,
    Remainder,
    Power,
}

impl Operator {
    pub(super) fn prefix(input: &str) -> Option<(Self, usize)> {
        if input.starts_with("**") {
            return Some((Self::Power, 2));
        }
        if input.starts_with("//") {
            return Some((Self::FloorDivide, 2));
        }
        let symbol = input.chars().next()?;
        let operator = match symbol {
            '+' => Self::Add,
            '-' => Self::Subtract,
            '*' | '×' => Self::Multiply,
            '/' => Self::Divide,
            '%' => Self::Remainder,
            '^' => Self::Power,
            _ => return None,
        };
        Some((operator, symbol.len_utf8()))
    }

    pub(super) fn binding_power(self) -> (u8, u8) {
        match self {
            Self::Add | Self::Subtract => (1, 2),
            Self::Multiply | Self::Divide | Self::FloorDivide | Self::Remainder => (3, 4),
            Self::Power => (6, 5),
        }
    }

    pub(super) fn apply(self, left: f64, right: f64) -> Result<f64, CalculationError> {
        if matches!(self, Self::Divide | Self::FloorDivide | Self::Remainder) && right == 0. {
            return Err(CalculationError::DivisionByZero);
        }
        Ok(match self {
            Self::Add => left + right,
            Self::Subtract => left - right,
            Self::Multiply => left * right,
            Self::Divide => left / right,
            Self::FloorDivide | Self::Remainder => {
                let remainder = left % right;
                let adjust =
                    remainder != 0. && remainder.is_sign_negative() != right.is_sign_negative();
                if matches!(self, Self::FloorDivide) {
                    // 先移除余数再还原整数商，避免直接相除时舍入到下一整数。
                    let quotient = (left - remainder) / right - if adjust { 1. } else { 0. };
                    let floor = quotient.floor();
                    if quotient - floor > 0.5 {
                        floor + 1.
                    } else {
                        floor
                    }
                } else if adjust {
                    remainder + right
                } else {
                    remainder
                }
            }
            Self::Power => left.powf(right),
        })
    }
}
