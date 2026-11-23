use num_bigint::Sign;
use num_rational::BigRational;

use super::calculator::normalize;
use super::fraction::{calculate_fraction, ensure_exact_size, FractionError};

const MAX_SHARES: usize = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RatioAllocationError {
    Incomplete,
    ShareCount,
    NegativeWeight,
    ZeroTotalWeight,
    Evaluation(FractionError),
}

impl RatioAllocationError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入总量和配比，例如 120 @ 2:3:5",
            Self::ShareCount => "输入 2 至 24 个权重，用冒号隔开",
            Self::NegativeWeight => "分配权重须为非负数",
            Self::ZeroTotalWeight => "至少一个分配权重须大于零",
            Self::Evaluation(error) => error.message(),
        }
    }
}

pub(super) fn allocate_ratio(query: &str) -> Result<String, RatioAllocationError> {
    let input = normalize(query);
    let (total, weights) = split_allocation(&input)?;
    let total = calculate_fraction(total).map_err(RatioAllocationError::Evaluation)?;
    let shares = allocate_shares(&total, weights)?;
    Ok(shares
        .into_iter()
        .map(|share| share.to_string())
        .collect::<Vec<_>>()
        .join(" · "))
}

pub(super) fn split_allocation(input: &str) -> Result<(&str, Vec<&str>), RatioAllocationError> {
    let (total, weights) = input
        .split_once(['@', '按'])
        .ok_or(RatioAllocationError::Incomplete)?;
    if total.trim().is_empty() || weights.trim().is_empty() {
        return Err(RatioAllocationError::Incomplete);
    }
    let weights: Vec<_> = weights.split(':').take(MAX_SHARES + 1).collect();
    if !(2..=MAX_SHARES).contains(&weights.len()) {
        return Err(RatioAllocationError::ShareCount);
    }
    if weights.iter().any(|weight| weight.trim().is_empty()) {
        return Err(RatioAllocationError::Incomplete);
    }
    Ok((total, weights))
}

pub(super) fn allocate_shares(
    total: &BigRational,
    weights: Vec<&str>,
) -> Result<Vec<BigRational>, RatioAllocationError> {
    let mut parsed_weights = Vec::with_capacity(weights.len());
    let mut sum = BigRational::default();
    for weight in weights {
        let weight = calculate_fraction(weight).map_err(RatioAllocationError::Evaluation)?;
        if weight.numer().sign() == Sign::Minus {
            return Err(RatioAllocationError::NegativeWeight);
        }
        sum += &weight;
        ensure_exact_size(&sum).map_err(RatioAllocationError::Evaluation)?;
        parsed_weights.push(weight);
    }
    if sum.numer().sign() == Sign::NoSign {
        return Err(RatioAllocationError::ZeroTotalWeight);
    }

    let mut shares = Vec::with_capacity(parsed_weights.len());
    for weight in parsed_weights {
        let share = (total * weight) / &sum;
        ensure_exact_size(&share).map_err(RatioAllocationError::Evaluation)?;
        shares.push(share);
    }
    Ok(shares)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocates_amounts_exactly_in_input_order() {
        for (input, expected) in [
            ("120 @ 2:3:5", "24 · 36 · 60"),
            ("1 按 1:1:1", "1/3 · 1/3 · 1/3"),
            ("−９ ＠ ０：１：２", "0 · -3 · -6"),
            ("3.0 @ 1/2:1/4", "2 · 1"),
            ("0 @ 2:3", "0 · 0"),
        ] {
            assert_eq!(allocate_ratio(input).as_deref(), Ok(expected), "{input}");
        }
    }

    #[test]
    fn rejects_empty_and_invalid_ratio_weights() {
        for (input, error) in [
            ("120", RatioAllocationError::Incomplete),
            ("120 @ 2:", RatioAllocationError::Incomplete),
            ("120 @ 2", RatioAllocationError::ShareCount),
            ("120 @ 1:-1", RatioAllocationError::NegativeWeight),
            ("120 @ 0:0", RatioAllocationError::ZeroTotalWeight),
        ] {
            assert_eq!(allocate_ratio(input), Err(error), "{input}");
        }
        let excessive = format!("1 @ {}1", "1:".repeat(MAX_SHARES));
        assert_eq!(
            allocate_ratio(&excessive),
            Err(RatioAllocationError::ShareCount)
        );
    }
}
