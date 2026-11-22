use std::cmp::Ordering;

use num_rational::BigRational;

use super::{
    calculator::normalize,
    fraction::{calculate_fraction, FractionError},
    operators::split_top_level_commas,
};

const MAX_INTERVALS: usize = 32;
const EMPTY_SET: &str = "∅";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum IntervalError {
    Incomplete,
    Operation,
    Count,
    Notation,
    Reversed,
    Evaluation(FractionError),
}

impl IntervalError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete | Self::Operation => {
                "输入交集或合并，再填区间，例如 交集 [1/2,3); (2,5]"
            }
            Self::Count => "最多填写 32 个区间，用分号或换行分开",
            Self::Notation => "用方括号包含端点，用圆括号排除端点，两个端点用逗号分开",
            Self::Reversed => "区间的左端点须小于或等于右端点",
            Self::Evaluation(FractionError::Invalid | FractionError::Incomplete) => {
                "端点须为整数、有限小数、分数或精确算式"
            }
            Self::Evaluation(error) => error.message(),
        }
    }
}

struct Endpoint {
    value: BigRational,
    included: bool,
}

struct Interval {
    start: Endpoint,
    end: Endpoint,
}

impl Interval {
    fn is_nonempty(&self) -> bool {
        match self.start.value.cmp(&self.end.value) {
            Ordering::Less => true,
            Ordering::Equal => self.start.included && self.end.included,
            Ordering::Greater => false,
        }
    }

    fn notation(&self) -> String {
        let left = if self.start.included { '[' } else { '(' };
        let right = if self.end.included { ']' } else { ')' };
        format!("{left}{},{}{right}", self.start.value, self.end.value)
    }
}

pub(super) fn calculate_intervals(query: &str) -> Result<String, IntervalError> {
    let normalized = normalize(query);
    let input = normalized.trim();
    let (operation, arguments) = input.split_once(char::is_whitespace).unwrap_or((input, ""));
    let intersection = match operation.to_ascii_lowercase().as_str() {
        "交集" | "intersect" => true,
        "合并" | "并集" | "union" => false,
        "" => return Err(IntervalError::Incomplete),
        _ => return Err(IntervalError::Operation),
    };
    let arguments = arguments.trim();
    if arguments.is_empty() {
        return Err(IntervalError::Incomplete);
    }
    let parts: Vec<_> = arguments
        .split([';', '\n'])
        .take(MAX_INTERVALS + 1)
        .collect();
    if parts.len() > MAX_INTERVALS {
        return Err(IntervalError::Count);
    }
    let intervals = parts
        .into_iter()
        .map(parse_interval)
        .collect::<Result<Vec<_>, _>>()?;
    if intersection {
        return Ok(intersect_intervals(intervals)
            .map_or_else(|| EMPTY_SET.to_owned(), |interval| interval.notation()));
    }
    let merged = merge_intervals(intervals.into_iter().flatten().collect());
    if merged.is_empty() {
        return Ok(EMPTY_SET.to_owned());
    }
    Ok(merged
        .iter()
        .map(Interval::notation)
        .collect::<Vec<_>>()
        .join("\n"))
}

fn parse_interval(input: &str) -> Result<Option<Interval>, IntervalError> {
    let input = input.trim();
    let left = match input.as_bytes().first() {
        Some(b'[') => true,
        Some(b'(') => false,
        _ => return Err(IntervalError::Notation),
    };
    let right = match input.as_bytes().last() {
        Some(b']') => true,
        Some(b')') => false,
        _ => return Err(IntervalError::Notation),
    };
    let mut terms = split_top_level_commas(&input[1..input.len() - 1]);
    let (Some(start), Some(end), None) = (terms.next(), terms.next(), terms.next()) else {
        return Err(IntervalError::Notation);
    };
    let interval = Interval {
        start: Endpoint {
            value: calculate_fraction(start).map_err(IntervalError::Evaluation)?,
            included: left,
        },
        end: Endpoint {
            value: calculate_fraction(end).map_err(IntervalError::Evaluation)?,
            included: right,
        },
    };
    if interval.start.value > interval.end.value {
        return Err(IntervalError::Reversed);
    }
    Ok(interval.is_nonempty().then_some(interval))
}

fn intersect_intervals(intervals: Vec<Option<Interval>>) -> Option<Interval> {
    let mut remaining = intervals.into_iter();
    let mut result = remaining.next()??;
    for interval in remaining {
        let interval = interval?;
        match result.start.value.cmp(&interval.start.value) {
            Ordering::Less => result.start = interval.start,
            Ordering::Equal => result.start.included &= interval.start.included,
            Ordering::Greater => {}
        }
        match result.end.value.cmp(&interval.end.value) {
            Ordering::Greater => result.end = interval.end,
            Ordering::Equal => result.end.included &= interval.end.included,
            Ordering::Less => {}
        }
    }
    result.is_nonempty().then_some(result)
}

fn merge_intervals(mut intervals: Vec<Interval>) -> Vec<Interval> {
    intervals.sort_unstable_by(|left, right| {
        left.start
            .value
            .cmp(&right.start.value)
            .then_with(|| right.start.included.cmp(&left.start.included))
    });
    let mut merged: Vec<Interval> = Vec::new();
    for interval in intervals {
        if let Some(last) = merged.last_mut() {
            let touching = interval.start.value == last.end.value
                && (interval.start.included || last.end.included);
            if interval.start.value < last.end.value || touching {
                match last.end.value.cmp(&interval.end.value) {
                    Ordering::Less => last.end = interval.end,
                    Ordering::Equal => last.end.included |= interval.end.included,
                    Ordering::Greater => {}
                }
                continue;
            }
        }
        merged.push(interval);
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intersects_exact_numbers_and_accounts_for_both_endpoint_kinds() {
        for (query, expected) in [
            ("交集 [1/2,3); (2,5]", "(2,3)"),
            ("INTERSECT [0,1]; [1,3]", "[1,1]"),
            ("交集 [0,1); [1,3]", EMPTY_SET),
            ("交集 [0,9]; (2,4]; [2,4)", "(2,4)"),
            ("交集 [0,9]; (3,3]", EMPTY_SET),
            ("交集 ［－１，comb(3,2)］；［0,1］", "[0,1]"),
        ] {
            assert_eq!(
                calculate_intervals(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
    }

    #[test]
    fn merges_only_when_the_union_has_no_gap_and_validates_all_ranges() {
        for (query, expected) in [
            ("合并 [0,1); [1,2]; (3,4)", "[0,2]\n(3,4)"),
            ("union (2,3]; [1,2)", "[1,2)\n(2,3]"),
            ("合并 (1,4); [1,4]; (3,3]", "[1,4]"),
            ("合并 [0,1); [1,1]\n(1,2]", "[0,2]"),
            ("合并 (3,3); [3,3)", EMPTY_SET),
        ] {
            assert_eq!(
                calculate_intervals(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert_eq!(
            calculate_intervals("交集 (1,1); [2,1]"),
            Err(IntervalError::Reversed)
        );
        assert_eq!(
            calculate_intervals("合并 (1)"),
            Err(IntervalError::Notation)
        );
        assert_eq!(
            calculate_intervals("合并 (3,4] ; a"),
            Err(IntervalError::Notation)
        );
        let too_many = format!("合并 {}", ["[0,1]"; MAX_INTERVALS + 1].join(";"));
        assert_eq!(calculate_intervals(&too_many), Err(IntervalError::Count));
    }
}
