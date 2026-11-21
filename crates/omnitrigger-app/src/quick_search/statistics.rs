use num_bigint::BigInt;
use num_rational::BigRational;

use super::calculator::normalize;
use super::fraction::{calculate_fraction, ensure_exact_size, format_square_root, FractionError};
use super::operators::split_top_level_commas;

const MAX_VALUES: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StatisticsError {
    Incomplete,
    TooManyValues,
    InvalidFrequency,
    FrequencyOverflow,
    Evaluation(FractionError),
}

impl StatisticsError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "用逗号或换行隔开数值，例如 1,3,5",
            Self::TooManyValues => "最多可统计 128 个值或频数组",
            Self::InvalidFrequency => "每组用 数值@次数，次数须为正整数",
            Self::FrequencyOverflow => "频数合计超出支持范围",
            Self::Evaluation(FractionError::Invalid | FractionError::Incomplete) => {
                "请输入整数、有限小数、分数或精确算式"
            }
            Self::Evaluation(error) => error.message(),
        }
    }
}

pub(super) fn calculate_statistics(query: &str) -> Result<String, StatisticsError> {
    let normalized = normalize(query);
    let input = normalized.trim();
    let (operation, values) = operation_and_arguments(input);
    let (sample, input) = match operation.to_ascii_lowercase().as_str() {
        "总体" | "population" => (false, values.trim()),
        "样本" | "sample" => (true, values.trim()),
        _ => (true, input),
    };
    let (operation, values) = operation_and_arguments(input);
    let use_frequencies = matches!(
        operation.to_ascii_lowercase().as_str(),
        "频数" | "frequency"
    );
    let input = if use_frequencies {
        values.trim()
    } else {
        input
    };
    let terms: Vec<_> = input
        .lines()
        .flat_map(split_top_level_commas)
        .take(MAX_VALUES + 1)
        .collect();
    if terms.is_empty() {
        return Err(StatisticsError::Incomplete);
    }
    if terms.len() > MAX_VALUES {
        return Err(StatisticsError::TooManyValues);
    }
    let mut values = terms
        .into_iter()
        .map(|term| parse_observation(term, use_frequencies))
        .collect::<Result<Vec<_>, _>>()?;
    let count = values.iter().try_fold(0_u64, |count, (_, repetitions)| {
        count
            .checked_add(*repetitions)
            .ok_or(StatisticsError::FrequencyOverflow)
    })?;
    let sum = values
        .iter()
        .try_fold(BigRational::default(), |sum, (value, repetitions)| {
            bounded(sum + value * BigInt::from(*repetitions))
        })?;
    let average = bounded(&sum / BigInt::from(count))?;

    values.sort_by(|(left, _), (right, _)| left.cmp(right));
    let middle = observation_at(&values, count / 2);
    let median = if count.is_multiple_of(2) {
        let previous = observation_at(&values, count / 2 - 1);
        bounded((previous + middle) / BigInt::from(2))?
    } else {
        middle.clone()
    };
    let mut result = format!(
        "数量 {count} · 合计 {sum}\n平均值 {average} · 中位数 {median}\n最小值 {} · 最大值 {}",
        values[0].0,
        values.last().expect("a nonempty sample").0,
    );
    let variance_divisor = count - u64::from(sample);
    if variance_divisor > 0 {
        let deviation_sum =
            values
                .iter()
                .try_fold(BigRational::default(), |total, (value, repetitions)| {
                    let difference = value - &average;
                    bounded(total + &difference * &difference * BigInt::from(*repetitions))
                })?;
        let variance = bounded(deviation_sum / BigInt::from(variance_divisor))?;
        let label = if sample { "样本" } else { "总体" };
        result.push_str(&format!(
            "\n{label}方差 {variance} · 标准差 {}",
            format_square_root(&variance)
        ));
    }
    Ok(result)
}

fn operation_and_arguments(input: &str) -> (&str, &str) {
    input.split_once(char::is_whitespace).unwrap_or((input, ""))
}

fn parse_observation(
    term: &str,
    use_frequencies: bool,
) -> Result<(BigRational, u64), StatisticsError> {
    let (value, repetitions) = if use_frequencies {
        let (value, frequency) = term
            .split_once('@')
            .ok_or(StatisticsError::InvalidFrequency)?;
        let frequency = frequency.trim();
        if !frequency
            .split('_')
            .all(|digits| !digits.is_empty() && digits.bytes().all(|digit| digit.is_ascii_digit()))
        {
            return Err(StatisticsError::InvalidFrequency);
        }
        let repetitions = frequency
            .replace('_', "")
            .parse::<u64>()
            .ok()
            .filter(|count| *count > 0)
            .ok_or(StatisticsError::InvalidFrequency)?;
        (value, repetitions)
    } else {
        (term, 1)
    };
    let value = calculate_fraction(value).map_err(StatisticsError::Evaluation)?;
    Ok((value, repetitions))
}

fn observation_at(values: &[(BigRational, u64)], mut rank: u64) -> &BigRational {
    for (value, repetitions) in values {
        if rank < *repetitions {
            return value;
        }
        rank -= *repetitions;
    }
    unreachable!("observation rank is within the total frequency")
}

fn bounded(value: BigRational) -> Result<BigRational, StatisticsError> {
    ensure_exact_size(&value).map_err(StatisticsError::Evaluation)?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_population_variance_when_explicitly_selected() {
        assert_eq!(
            calculate_statistics("总体 1,3,5").as_deref(),
            Ok("数量 3 · 合计 9\n平均值 3 · 中位数 3\n最小值 1 · 最大值 5\n总体方差 8/3 · 标准差 √(8/3)"),
        );
        assert_eq!(
            calculate_statistics("population 1/2").as_deref(),
            Ok("数量 1 · 合计 1/2\n平均值 1/2 · 中位数 1/2\n最小值 1/2 · 最大值 1/2\n总体方差 0 · 标准差 0"),
        );
        assert_eq!(
            calculate_statistics("样本 1,3,5"),
            calculate_statistics("1,3,5"),
        );
        assert_eq!(
            calculate_statistics("总体"),
            Err(StatisticsError::Incomplete)
        );
    }

    #[test]
    fn summarizes_frequency_groups_as_repeated_observations_without_expanding_them() {
        for (grouped, expanded) in [
            ("频数 5@2,1@2,3@1", "1,1,3,5,5"),
            ("总体 频数 1/2@2\n3/4@2", "总体 1/2,1/2,3/4,3/4"),
            ("sample frequency ５＠１", "5"),
            ("频数 comb(4,2)@2,-2@2,6@1", "-2,-2,6,6,6"),
        ] {
            assert_eq!(
                calculate_statistics(grouped),
                calculate_statistics(expanded)
            );
        }
        let large = calculate_statistics("频数 5@1_000_000_000,7@1").unwrap();
        assert!(large.contains("数量 1000000001 · 合计 5000000007"));
        assert!(large.contains("中位数 5"));
        assert!(large.contains("样本方差 4/1000000001"));
        for query in [
            "频数 1",
            "频数 1@0",
            "频数 1@-1",
            "频数 1@1/2",
            "频数 1@2@3",
            "频数 1@1__2",
        ] {
            assert_eq!(
                calculate_statistics(query),
                Err(StatisticsError::InvalidFrequency)
            );
        }
        assert_eq!(
            calculate_statistics("频数 9@18446744073709551615,1@1"),
            Err(StatisticsError::FrequencyOverflow)
        );
    }

    #[test]
    fn summarizes_samples_with_exact_rational_statistics() {
        for (query, expected) in [
            (
                "５，１，３",
                "数量 3 · 合计 9\n平均值 3 · 中位数 3\n最小值 1 · 最大值 5\n样本方差 4 · 标准差 2",
            ),
            (
                "1/2\n3/4",
                "数量 2 · 合计 5/4\n平均值 5/8 · 中位数 5/8\n最小值 1/2 · 最大值 3/4\n样本方差 1/32 · 标准差 √(1/32)",
            ),
            (
                "9007199254740993,9007199254740995",
                "数量 2 · 合计 18014398509481988\n平均值 9007199254740994 · 中位数 9007199254740994\n最小值 9007199254740993 · 最大值 9007199254740995\n样本方差 2 · 标准差 √2",
            ),
            (
                "comb(3,2) - 5.5",
                "数量 1 · 合计 -5/2\n平均值 -5/2 · 中位数 -5/2\n最小值 -5/2 · 最大值 -5/2",
            ),
        ] {
            assert_eq!(calculate_statistics(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(calculate_statistics(""), Err(StatisticsError::Incomplete));
        assert_eq!(
            calculate_statistics(&format!("{}1", "1,".repeat(MAX_VALUES))),
            Err(StatisticsError::TooManyValues)
        );
    }
}
