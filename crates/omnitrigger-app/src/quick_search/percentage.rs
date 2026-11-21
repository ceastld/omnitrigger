use super::{
    calculator::{evaluate, format_result, normalize},
    conversion::split_conversion,
};

#[derive(Clone, Copy)]
pub(super) enum Percentage {
    Portion,
    Increase,
    Decrease,
    Discount,
}

impl Percentage {
    pub(super) fn apply(self, base: f64, rate: f64) -> f64 {
        let fraction = rate
            / if matches!(self, Self::Discount) {
                10.
            } else {
                100.
            };
        match self {
            Self::Portion | Self::Discount => base * fraction,
            Self::Increase => base * (1. + fraction),
            Self::Decrease => base * (1. - fraction),
        }
    }
}

/// 保留基数与比例两边的算式，使用统一的计算规则。
pub(super) fn percentage_operands(input: &str) -> Option<(Percentage, &str, &str)> {
    if !input.contains(['%', '折']) {
        return None;
    }
    let lower = input.to_ascii_lowercase();
    let separator = [
        ("的", Percentage::Portion),
        ("增加", Percentage::Increase),
        ("减少", Percentage::Decrease),
        ("打", Percentage::Discount),
        (" increased by ", Percentage::Increase),
        (" decreased by ", Percentage::Decrease),
    ]
    .into_iter()
    .filter_map(|(separator, mode)| {
        lower
            .rfind(separator)
            .map(|start| (start, start + separator.len(), mode))
    })
    .max_by_key(|(start, _, _)| *start);
    if let Some((start, end, mode)) = separator {
        let suffix = if matches!(mode, Percentage::Discount) {
            '折'
        } else {
            '%'
        };
        return Some((
            mode,
            input[..start].trim(),
            input[end..].trim().strip_suffix(suffix)?.trim_end(),
        ));
    }

    let (rate, tail) = input.rsplit_once('%')?;
    let mut tail = tail.trim_start().splitn(2, char::is_whitespace);
    if !tail.next()?.eq_ignore_ascii_case("of") {
        return None;
    }
    Some((
        Percentage::Portion,
        tail.next().unwrap_or_default().trim(),
        rate.trim_end(),
    ))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PercentageError {
    Incomplete,
    UnknownOperation,
    InvalidOperands,
    ZeroReference,
    OutOfRange,
}

impl PercentageError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入操作和两个值，例如 百分比: 变化 200 到 150",
            Self::UnknownOperation => "可用的操作有 变化、占比 和 百分点",
            Self::InvalidOperands => "请输入两个有效的数值，计算百分点时加上 %",
            Self::ZeroReference => "原值或总量不能为零",
            Self::OutOfRange => "结果超出可计算范围",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Comparison {
    Change,
    Share,
    Points,
}

pub(super) fn compare_percentages(input: &str) -> Result<String, PercentageError> {
    let input = normalize(input).to_ascii_lowercase();
    let (operation, operands) = input
        .split_once(char::is_whitespace)
        .ok_or(PercentageError::Incomplete)?;
    let comparison = match operation {
        "变化" | "change" => Comparison::Change,
        "占比" | "share" => Comparison::Share,
        "百分点" | "points" => Comparison::Points,
        _ => return Err(PercentageError::UnknownOperation),
    };
    let operands = operands.trim();
    let sides = match comparison {
        Comparison::Share => operands
            .split_once('占')
            .or_else(|| operands.split_once(" of ")),
        _ => operands
            .split_once('到')
            .or_else(|| split_conversion(operands)),
    };
    let (left, right) = sides.ok_or(PercentageError::Incomplete)?;
    let eval = |input: &str| {
        let input = input.trim();
        if input.is_empty() {
            return Err(PercentageError::Incomplete);
        }
        let input = if comparison == Comparison::Points {
            input
                .strip_suffix('%')
                .ok_or(PercentageError::InvalidOperands)?
        } else {
            input
        };
        evaluate(input).map_err(|_| PercentageError::InvalidOperands)
    };
    let left = eval(left)?;
    let right = eval(right)?;
    let (value, suffix) = match comparison {
        Comparison::Change => {
            if left == 0. {
                return Err(PercentageError::ZeroReference);
            }
            let difference = right - left;
            let change = if difference.is_finite() {
                ratio_percent(difference, left)
            } else {
                ratio_percent(right, left) - 100.
            };
            (change, "%")
        }
        Comparison::Share => {
            if right == 0. {
                return Err(PercentageError::ZeroReference);
            }
            (ratio_percent(left, right), "%")
        }
        Comparison::Points => (right - left, " 个百分点"),
    };
    let value = format_result(value).map_err(|_| PercentageError::OutOfRange)?;
    Ok(format!("{value}{suffix}"))
}

fn ratio_percent(value: f64, reference: f64) -> f64 {
    let scaled = value * 100.;
    if scaled.is_finite() {
        scaled / reference
    } else {
        value / reference * 100.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculates_signed_change_share_and_percentage_points() {
        for (query, expected) in [
            ("变化 200 到 150", "-25%"),
            ("变化 (100+50) -> 180", "20%"),
            ("CHANGE 1e308 to -1e308", "-200%"),
            ("占比 18 占 60", "30%"),
            ("share 10 of 40", "25%"),
            ("占比 1e308 占 1e308", "100%"),
            ("百分点 15％ 到 21.5％", "6.5 个百分点"),
            ("points 25% → 20%", "-5 个百分点"),
        ] {
            assert_eq!(
                compare_percentages(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        for (query, error) in [
            ("变化 1 到", PercentageError::Incomplete),
            ("变化 0 到 1", PercentageError::ZeroReference),
            ("占比 1 占 0", PercentageError::ZeroReference),
            ("百分点 1 到 2%", PercentageError::InvalidOperands),
            ("变化 1e-308 到 1e308", PercentageError::OutOfRange),
            ("未知 1 到 2", PercentageError::UnknownOperation),
        ] {
            assert_eq!(compare_percentages(query), Err(error), "{query}");
        }
    }
}
