use num_bigint::{BigInt, Sign};
use num_rational::BigRational;

use super::{
    calculator::{evaluate, format_result, normalize},
    conversion::split_conversion,
    fraction::{calculate_fraction, ensure_exact_size, FractionError},
    operators::split_top_level_commas,
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
    InvalidDiscount,
    InvalidCompoundOperands,
    NonInvertibleChange,
    ZeroReference,
    OutOfRange,
    Evaluation(FractionError),
}

impl PercentageError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入操作和两个值，例如 百分比: 变化 200 到 150",
            Self::UnknownOperation => "输入 ? 百分比 查看可用操作",
            Self::InvalidOperands => "请输入两个有效的数值，计算百分点时加上 %",
            Self::InvalidDiscount => "折后金额须非负，折扣须大于 0% 且不超过 100%",
            Self::InvalidCompoundOperands => "输入数值和带百分号的变动，例如 100 @ +10%, -10%",
            Self::NonInvertibleChange => "存在 -100% 的变动，无法唯一反推原值",
            Self::ZeroReference => "原值或总量不能为零",
            Self::OutOfRange => "结果超出可计算范围",
            Self::Evaluation(error) => error.message(),
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
    if matches!(operation, "折前价" | "original-price") {
        return original_price(operands);
    }
    if matches!(operation, "连续变化" | "compound") {
        return compound_percentages(operands, false);
    }
    if matches!(operation, "原值" | "reverse-compound") {
        return compound_percentages(operands, true);
    }
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

fn compound_percentages(operands: &str, reverse: bool) -> Result<String, PercentageError> {
    let (base, rates) = operands
        .split_once('@')
        .ok_or(PercentageError::InvalidCompoundOperands)?;
    let mut value = calculate_fraction(base).map_err(PercentageError::Evaluation)?;
    let one = BigRational::from_integer(BigInt::from(1));
    let hundred = BigInt::from(100);
    let mut rates: Vec<_> = split_top_level_commas(rates).collect();
    if reverse {
        rates.reverse();
    }
    for rate in rates {
        let amount = rate
            .trim()
            .strip_suffix('%')
            .ok_or(PercentageError::InvalidCompoundOperands)?;
        let amount = calculate_fraction(amount).map_err(PercentageError::Evaluation)?;
        let factor = &one + amount / &hundred;
        if reverse {
            if factor.numer().sign() == Sign::NoSign {
                return Err(PercentageError::NonInvertibleChange);
            }
            value /= factor;
        } else {
            value *= factor;
        }
        ensure_exact_size(&value).map_err(PercentageError::Evaluation)?;
    }
    Ok(value.to_string())
}

fn ratio_percent(value: f64, reference: f64) -> f64 {
    let scaled = value * 100.;
    if scaled.is_finite() {
        scaled / reference
    } else {
        value / reference * 100.
    }
}

fn original_price(operands: &str) -> Result<String, PercentageError> {
    let (paid, discount) = operands
        .split_once('按')
        .or_else(|| operands.split_once(" at "))
        .ok_or(PercentageError::Incomplete)?;
    let (paid, discount) = (paid.trim(), discount.trim());
    if paid.is_empty() || discount.is_empty() {
        return Err(PercentageError::Incomplete);
    }
    let (rate, scale) = if let Some(rate) = discount.strip_suffix('折') {
        (rate, 10.)
    } else if let Some(rate) = discount.strip_suffix('%') {
        (rate, 100.)
    } else {
        return Err(PercentageError::InvalidDiscount);
    };
    let paid = evaluate(paid).map_err(|_| PercentageError::InvalidDiscount)?;
    let rate = evaluate(rate.trim()).map_err(|_| PercentageError::InvalidDiscount)?;
    if paid < 0. || !(0. ..=scale).contains(&rate) || rate == 0. {
        return Err(PercentageError::InvalidDiscount);
    }
    let fraction = rate / scale;
    let amount = if fraction < f64::MIN_POSITIVE {
        // 折扣极小时先相除，避免比例下溢到零或损失有效位。
        paid / rate * scale
    } else {
        paid / fraction
    };
    format_result(amount).map_err(|_| PercentageError::OutOfRange)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverses_successive_percentage_changes_with_exact_factors() {
        for (query, expected) in [
            ("原值 99 @ +10%, -10%", "100"),
            ("原值 230 @ +15%", "200"),
            ("reverse-compound 1/3 @ +1/3%, -25%", "400/903"),
            ("原值 -30 @ -150%", "60"),
            ("原值 9007199254740993 @ 0%", "9007199254740993"),
        ] {
            assert_eq!(
                compare_percentages(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert_eq!(
            compare_percentages("原值 0 @ +20%, -100%, +10%"),
            Err(PercentageError::NonInvertibleChange),
        );
        assert_eq!(
            compare_percentages("原值 100 @ +10"),
            Err(PercentageError::InvalidCompoundOperands),
        );
    }

    #[test]
    fn applies_successive_percentage_changes_without_rounding() {
        for (query, expected) in [
            ("连续变化 100 @ +10%, -10%", "99"),
            ("compound 1/3 @ 50%, -25%", "3/8"),
            ("连续变化 9007199254740993 @ 0%", "9007199254740993"),
            ("连续变化 4 @ -100%, +20%", "0"),
            ("连续变化 8 @ comb(4,2)%", "212/25"),
        ] {
            assert_eq!(
                compare_percentages(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert_eq!(
            compare_percentages("连续变化 100 @ 10"),
            Err(PercentageError::InvalidCompoundOperands)
        );
        assert_eq!(
            compare_percentages("compound 1/0 @ 15%"),
            Err(PercentageError::Evaluation(FractionError::ZeroDenominator))
        );
    }

    #[test]
    fn recovers_an_original_amount_from_a_discounted_amount() {
        for (query, expected) in [
            ("折前价 170 按 8.5折", "200"),
            ("original-price (160+10) at 85%", "200"),
            ("折前价 85 按 １００％", "85"),
            ("折前价 0 按 5e-324%", "0"),
        ] {
            assert_eq!(
                compare_percentages(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        for query in [
            "折前价 170 按 0折",
            "折前价 170 按 101%",
            "折前价 -170 按 8.5折",
        ] {
            assert_eq!(
                compare_percentages(query),
                Err(PercentageError::InvalidDiscount),
                "{query}"
            );
        }
        assert_eq!(
            compare_percentages("折前价 170 按"),
            Err(PercentageError::Incomplete)
        );
        assert_eq!(
            compare_percentages("折前价 1e308 按 1%"),
            Err(PercentageError::OutOfRange)
        );
    }

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
