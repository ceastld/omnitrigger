use std::collections::BTreeMap;

use num_bigint::{BigInt, Sign};
use num_rational::BigRational;

use super::calculator::normalize;
use super::fraction::{calculate_fraction, ensure_exact_size, format_square_root, FractionError};
use super::linear_fit::{calculate_correlation, calculate_covariance, LinearFitError};
use super::operators::split_top_level_commas;

const MAX_VALUES: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StatisticsError {
    Incomplete,
    TooManyValues,
    InvalidFrequency,
    FrequencyOverflow,
    InvalidWeight,
    ZeroTotalWeight,
    InvalidProbability,
    InvalidWindowWidth,
    NonPositiveHarmonicValue,
    StandardErrorCount,
    Paired(LinearFitError),
    Evaluation(FractionError),
}

impl StatisticsError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "用逗号或换行隔开数值，例如 1,3,5",
            Self::TooManyValues => "最多可统计 128 个值或频数组",
            Self::InvalidFrequency => "每组用 数值@次数，次数须为正整数",
            Self::FrequencyOverflow => "频数合计超出支持范围",
            Self::InvalidWeight => "用 数值@非负权重，权重可写成分数或百分数",
            Self::ZeroTotalWeight => "权重合计须大于零",
            Self::InvalidProbability => "输入 0 到 1 的比例或百分数，例如 1/4 或 25%",
            Self::InvalidWindowWidth => "窗口大小须为正整数，且不能超过数值个数",
            Self::NonPositiveHarmonicValue => "调和平均需要所有数值大于零",
            Self::StandardErrorCount => "均值标准误至少需要两个观测值",
            Self::Paired(error) => error.message(),
            Self::Evaluation(FractionError::Invalid | FractionError::Incomplete) => {
                "请输入整数、有限小数、分数或精确算式"
            }
            Self::Evaluation(error) => error.message(),
        }
    }
}

enum StatisticsOperation {
    Sample,
    Population,
    Quartiles,
    Quantile(BigRational),
    Frequencies(FrequencyReport),
    RootMeanSquare,
    HarmonicMean,
    StandardError,
    MedianAbsoluteDeviation,
}

#[derive(Clone, Copy)]
enum FrequencyReport {
    Table,
    Cumulative,
    Mode,
}

pub(super) fn calculate_statistics(query: &str) -> Result<String, StatisticsError> {
    let normalized = normalize(query);
    let input = normalized.trim();
    let (operation, values) = operation_and_arguments(input);
    let (operation, input) = match operation.to_ascii_lowercase().as_str() {
        "总体" | "population" => (StatisticsOperation::Population, values.trim()),
        "样本" | "sample" => (StatisticsOperation::Sample, values.trim()),
        "四分位" | "quartiles" => (StatisticsOperation::Quartiles, values.trim()),
        "频数表" | "frequency-table" => (
            StatisticsOperation::Frequencies(FrequencyReport::Table),
            values.trim(),
        ),
        "累计分布" | "cumulative-distribution" | "ecdf" => (
            StatisticsOperation::Frequencies(FrequencyReport::Cumulative),
            values.trim(),
        ),
        "众数" | "mode" => (
            StatisticsOperation::Frequencies(FrequencyReport::Mode),
            values.trim(),
        ),
        "均方根" | "rms" => (StatisticsOperation::RootMeanSquare, values.trim()),
        "调和平均" | "harmonic-mean" => (StatisticsOperation::HarmonicMean, values.trim()),
        "标准误" | "均值标准误" | "sem" | "standard-error" => {
            (StatisticsOperation::StandardError, values.trim())
        }
        "中位绝对偏差" | "mad" | "median-absolute-deviation" => {
            (StatisticsOperation::MedianAbsoluteDeviation, values.trim())
        }
        "加权平均" | "weighted-mean" => return weighted_mean(values.trim()),
        "移动平均" | "moving-average" => return moving_average(values.trim()),
        "移动中位数" | "moving-median" => return moving_median(values.trim()),
        "累计合计" | "running-sum" => return running_totals(values.trim(), false),
        "累计平均" | "running-average" => return running_totals(values.trim(), true),
        "协方差" | "样本协方差" | "covariance" => {
            return calculate_covariance(values, true).map_err(StatisticsError::Paired);
        }
        "总体协方差" | "population-covariance" => {
            return calculate_covariance(values, false).map_err(StatisticsError::Paired);
        }
        "相关系数" | "correlation" => {
            return calculate_correlation(values).map_err(StatisticsError::Paired);
        }
        "分位数" | "quantile" => {
            let (probability, values) = operation_and_arguments(values.trim());
            (
                StatisticsOperation::Quantile(parse_probability(probability)?),
                values.trim(),
            )
        }
        _ => (StatisticsOperation::Sample, input),
    };
    let (prefix, values) = operation_and_arguments(input);
    let use_frequencies = matches!(prefix.to_ascii_lowercase().as_str(), "频数" | "frequency");
    let input = if use_frequencies {
        values.trim()
    } else {
        input
    };
    let values = observation_terms(input)?
        .into_iter()
        .map(|term| parse_observation(term, use_frequencies))
        .collect::<Result<Vec<_>, _>>()?;
    let count = values.iter().try_fold(0_u64, |count, (_, repetitions)| {
        count
            .checked_add(*repetitions)
            .ok_or(StatisticsError::FrequencyOverflow)
    })?;
    match operation {
        StatisticsOperation::Sample | StatisticsOperation::Population => summarize(
            values,
            count,
            matches!(operation, StatisticsOperation::Sample),
        ),
        StatisticsOperation::Quartiles => report_quantiles(values, count, None),
        StatisticsOperation::Quantile(probability) => {
            report_quantiles(values, count, Some(&probability))
        }
        StatisticsOperation::Frequencies(report) => Ok(report_frequencies(values, count, report)),
        StatisticsOperation::MedianAbsoluteDeviation => {
            median_absolute_deviation(values, count).map(|value| value.to_string())
        }
        StatisticsOperation::RootMeanSquare => {
            let squares = squared_deviation_sum(&values, &BigRational::default())?;
            let mean_square = bounded(squares / BigInt::from(count))?;
            Ok(format_square_root(&mean_square))
        }
        StatisticsOperation::StandardError => {
            if count < 2 {
                return Err(StatisticsError::StandardErrorCount);
            }
            let average = bounded(observation_sum(&values)? / BigInt::from(count))?;
            let deviation_sum = squared_deviation_sum(&values, &average)?;
            let divisor = BigInt::from(count) * BigInt::from(count - 1);
            let squared_error = bounded(deviation_sum / divisor)?;
            Ok(format_square_root(&squared_error))
        }
        StatisticsOperation::HarmonicMean => {
            let reciprocal_sum = values.into_iter().try_fold(
                BigRational::default(),
                |sum, (value, repetitions)| {
                    if value.numer().sign() != Sign::Plus {
                        return Err(StatisticsError::NonPositiveHarmonicValue);
                    }
                    bounded(sum + value.recip() * BigInt::from(repetitions))
                },
            )?;
            let mean = bounded(BigRational::from_integer(BigInt::from(count)) / reciprocal_sum)?;
            Ok(mean.to_string())
        }
    }
}

fn observation_terms(input: &str) -> Result<Vec<&str>, StatisticsError> {
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
    Ok(terms)
}

fn running_totals(input: &str, average: bool) -> Result<String, StatisticsError> {
    let terms = observation_terms(input)?;
    let mut total = BigRational::default();
    let mut output = Vec::with_capacity(terms.len());
    for (index, term) in terms.into_iter().enumerate() {
        let value = calculate_fraction(term).map_err(StatisticsError::Evaluation)?;
        total = bounded(total + value)?;
        let value = if average {
            bounded(&total / BigRational::from_integer(BigInt::from(index + 1)))?.to_string()
        } else {
            total.to_string()
        };
        output.push(value);
    }
    Ok(output.join("\n"))
}

fn sliding_window_values(input: &str) -> Result<(usize, Vec<BigRational>), StatisticsError> {
    let (width, input) = operation_and_arguments(input);
    let width = width
        .parse::<usize>()
        .ok()
        .filter(|width| *width > 0)
        .ok_or(StatisticsError::InvalidWindowWidth)?;
    let terms = observation_terms(input.trim())?;
    if width > terms.len() {
        return Err(StatisticsError::InvalidWindowWidth);
    }
    let values = terms
        .into_iter()
        .map(|term| calculate_fraction(term).map_err(StatisticsError::Evaluation))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((width, values))
}

fn moving_average(input: &str) -> Result<String, StatisticsError> {
    let (width, values) = sliding_window_values(input)?;
    let divisor = BigRational::from_integer(BigInt::from(width));
    let mut sum = BigRational::default();
    let mut averages = Vec::with_capacity(values.len() - width + 1);
    for (index, value) in values.iter().enumerate() {
        if index >= width {
            sum -= &values[index - width];
        }
        sum = bounded(sum + value)?;
        if index + 1 >= width {
            averages.push(bounded(&sum / &divisor)?.to_string());
        }
    }
    Ok(averages.join("\n"))
}

fn moving_median(input: &str) -> Result<String, StatisticsError> {
    let (width, values) = sliding_window_values(input)?;
    let mut ordered: Vec<&BigRational> = Vec::with_capacity(width);
    let mut medians = Vec::with_capacity(values.len() - width + 1);
    for window in values.windows(width) {
        ordered.extend(window);
        ordered.sort_unstable();
        let middle = ordered[width / 2];
        let median = if width.is_multiple_of(2) {
            let previous = ordered[width / 2 - 1];
            bounded((previous + middle) / BigInt::from(2))?.to_string()
        } else {
            middle.to_string()
        };
        medians.push(median);
        ordered.clear();
    }
    Ok(medians.join("\n"))
}

fn weighted_mean(input: &str) -> Result<String, StatisticsError> {
    let mut total_weight = BigRational::default();
    let mut total_value = BigRational::default();
    for term in observation_terms(input)? {
        let (value, weight) = term.split_once('@').ok_or(StatisticsError::InvalidWeight)?;
        let value = calculate_fraction(value).map_err(StatisticsError::Evaluation)?;
        let weight = if let Some(percent) = weight.trim().strip_suffix('%') {
            calculate_fraction(percent).map(|value| value / BigInt::from(100))
        } else {
            calculate_fraction(weight)
        }
        .map_err(StatisticsError::Evaluation)?;
        if weight.numer().sign() == Sign::Minus {
            return Err(StatisticsError::InvalidWeight);
        }
        total_value = bounded(total_value + value * &weight)?;
        total_weight = bounded(total_weight + weight)?;
    }
    if total_weight.numer().sign() == Sign::NoSign {
        return Err(StatisticsError::ZeroTotalWeight);
    }
    bounded(total_value / total_weight).map(|value| value.to_string())
}

fn report_frequencies(
    values: Vec<(BigRational, u64)>,
    count: u64,
    report: FrequencyReport,
) -> String {
    let mut frequencies = BTreeMap::<_, u64>::new();
    for (value, repetitions) in values {
        // 频数合计已在调用前检查，任一分组都不会超过合计。
        *frequencies.entry(value).or_default() += repetitions;
    }
    if matches!(report, FrequencyReport::Mode) {
        let highest = *frequencies.values().max().expect("至少有一个观测值");
        if highest == 1 {
            return "无众数 · 各值仅出现一次".to_owned();
        }
        let modes = frequencies
            .into_iter()
            .filter(|(_, repetitions)| *repetitions == highest)
            .map(|(value, _)| value.to_string())
            .collect::<Vec<_>>()
            .join("、");
        return format!("众数 {modes} · 各出现 {highest} 次");
    }
    let cumulative = matches!(report, FrequencyReport::Cumulative);
    let header = if cumulative {
        "值\t累计次数\t累计频率"
    } else {
        "值\t次数\t频率"
    };
    let mut total = 0;
    std::iter::once(header.to_owned())
        .chain(frequencies.into_iter().map(|(value, repetitions)| {
            total += repetitions;
            let occurrences = if cumulative { total } else { repetitions };
            let share = BigRational::new(occurrences.into(), count.into());
            format!("{value}\t{occurrences}\t{share}")
        }))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn summarize_unweighted(
    values: Vec<BigRational>,
    sample: bool,
) -> Result<String, StatisticsError> {
    if values.is_empty() {
        return Err(StatisticsError::Incomplete);
    }
    if values.len() > MAX_VALUES {
        return Err(StatisticsError::TooManyValues);
    }
    let count = values.len() as u64;
    summarize(
        values.into_iter().map(|value| (value, 1)).collect(),
        count,
        sample,
    )
}

fn observation_sum(values: &[(BigRational, u64)]) -> Result<BigRational, StatisticsError> {
    values
        .iter()
        .try_fold(BigRational::default(), |sum, (value, repetitions)| {
            bounded(sum + value * BigInt::from(*repetitions))
        })
}

fn summarize(
    mut values: Vec<(BigRational, u64)>,
    count: u64,
    sample: bool,
) -> Result<String, StatisticsError> {
    let sum = observation_sum(&values)?;
    let average = bounded(&sum / BigInt::from(count))?;

    values.sort_by(|(left, _), (right, _)| left.cmp(right));
    let median = sorted_median(&values, count)?;
    let mut result = format!(
        "数量 {count} · 合计 {sum}\n平均值 {average} · 中位数 {median}\n最小值 {} · 最大值 {}",
        values[0].0,
        values.last().expect("a nonempty sample").0,
    );
    let variance_divisor = count - u64::from(sample);
    if variance_divisor > 0 {
        let deviation_sum = squared_deviation_sum(&values, &average)?;
        let variance = bounded(deviation_sum / BigInt::from(variance_divisor))?;
        let label = if sample { "样本" } else { "总体" };
        result.push_str(&format!(
            "\n{label}方差 {variance} · 标准差 {}",
            format_square_root(&variance)
        ));
    }
    Ok(result)
}

fn sorted_median(
    values: &[(BigRational, u64)],
    count: u64,
) -> Result<BigRational, StatisticsError> {
    let middle = observation_at(values, count / 2);
    if count.is_multiple_of(2) {
        let previous = observation_at(values, count / 2 - 1);
        return bounded((previous + middle) / BigInt::from(2));
    }
    Ok(middle.clone())
}

fn median_absolute_deviation(
    mut values: Vec<(BigRational, u64)>,
    count: u64,
) -> Result<BigRational, StatisticsError> {
    values.sort_by(|(left, _), (right, _)| left.cmp(right));
    let center = sorted_median(&values, count)?;
    for (value, _) in &mut values {
        let deviation = std::mem::take(value) - &center;
        *value = bounded(if deviation.numer().sign() == Sign::Minus {
            -deviation
        } else {
            deviation
        })?;
    }
    values.sort_by(|(left, _), (right, _)| left.cmp(right));
    sorted_median(&values, count)
}

fn squared_deviation_sum(
    values: &[(BigRational, u64)],
    center: &BigRational,
) -> Result<BigRational, StatisticsError> {
    values
        .iter()
        .try_fold(BigRational::default(), |total, (value, repetitions)| {
            let difference = value - center;
            bounded(total + &difference * &difference * BigInt::from(*repetitions))
        })
}

fn parse_probability(input: &str) -> Result<BigRational, StatisticsError> {
    let probability = if let Some(percent) = input.strip_suffix('%') {
        calculate_fraction(percent).map(|value| value / BigInt::from(100))
    } else {
        calculate_fraction(input)
    }
    .map_err(|_| StatisticsError::InvalidProbability)?;
    if probability.numer().sign() == Sign::Minus || probability.numer() > probability.denom() {
        return Err(StatisticsError::InvalidProbability);
    }
    Ok(probability)
}

fn report_quantiles(
    mut values: Vec<(BigRational, u64)>,
    count: u64,
    probability: Option<&BigRational>,
) -> Result<String, StatisticsError> {
    values.sort_by(|(left, _), (right, _)| left.cmp(right));
    let quantile = |probability: &BigRational| interpolate_quantile(&values, count, probability);
    if let Some(probability) = probability {
        return quantile(probability).map(|value| value.to_string());
    }
    let lower = quantile(&BigRational::new(1.into(), 4.into()))?;
    let middle = quantile(&BigRational::new(1.into(), 2.into()))?;
    let upper = quantile(&BigRational::new(3.into(), 4.into()))?;
    let spread = bounded(&upper - &lower)?;
    Ok(format!(
        "Q1 {lower} · 中位数 {middle} · Q3 {upper}\n四分位距 {spread}"
    ))
}

fn interpolate_quantile(
    values: &[(BigRational, u64)],
    count: u64,
    probability: &BigRational,
) -> Result<BigRational, StatisticsError> {
    let position = probability * BigInt::from(count - 1);
    let rank =
        u64::try_from(position.to_integer()).expect("quantile rank within observation count");
    let lower = observation_at(values, rank);
    if position.is_integer() {
        return Ok(lower.clone());
    }
    let upper = observation_at(values, rank + 1);
    let fraction = position - BigInt::from(rank);
    bounded(lower + (upper - lower) * fraction)
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
    fn computes_unscaled_median_absolute_deviation_with_frequencies() {
        for (query, expected) in [
            ("中位绝对偏差 0,1/2,1,10", "1/2"),
            ("mad 1,1,2,2,100", "1"),
            ("中位绝对偏差 频数 0@2,3@2,100@1", "3"),
            ("mad frequency 0@1000000000,2@1000000000", "1"),
            ("中位绝对偏差 2**80", "0"),
        ] {
            assert_eq!(
                calculate_statistics(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
    }

    #[test]
    fn computes_exact_running_totals_and_averages_in_input_order() {
        for (query, expected) in [
            ("累计合计 1/2,1,2,4", "1/2\n3/2\n7/2\n15/2"),
            ("累计平均 1/2,1,2,4", "1/2\n3/4\n7/6\n15/8"),
            (
                "running-sum -1/3\r\n1/3\r\n2**80",
                "-1/3\n0\n1208925819614629174706176",
            ),
            ("running-average 0,-1,0", "0\n-1/2\n-1/3"),
        ] {
            assert_eq!(
                calculate_statistics(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert_eq!(
            calculate_statistics("累计平均"),
            Err(StatisticsError::Incomplete)
        );
    }

    #[test]
    fn reports_exact_medians_for_complete_sliding_windows() {
        for (query, expected) in [
            ("移动中位数 3 1/2,100,2,3,-1", "2\n3\n2"),
            ("moving-median 2 -1\r\n1/2\r\n1\r\n2", "-1/4\n3/4\n3/2"),
            ("移动中位数 1 comb(4,2),-2,-2", "6\n-2\n-2"),
            ("移动中位数 4 7,0,3,1", "2"),
        ] {
            assert_eq!(
                calculate_statistics(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert_eq!(
            calculate_statistics("移动中位数 3 1,2"),
            Err(StatisticsError::InvalidWindowWidth)
        );
        assert_eq!(
            calculate_statistics("移动中位数 2"),
            Err(StatisticsError::Incomplete)
        );
    }

    #[test]
    fn reports_exact_averages_for_complete_sliding_windows() {
        for (query, expected) in [
            ("移动平均 2 1/2,1,2,4", "3/4\n3/2\n3"),
            ("moving-average 3 1/2\r\n1\r\n2\r\n4", "7/6\n7/3"),
            ("移动平均 1 0,comb(4,2),-2", "0\n6\n-2"),
            (
                "移动平均 2 9007199254740992,9007199254740993",
                "18014398509481985/2",
            ),
        ] {
            assert_eq!(
                calculate_statistics(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        for query in ["移动平均 0 1,2", "移动平均 1/2 1,2", "移动平均 3 1,2"] {
            assert_eq!(
                calculate_statistics(query),
                Err(StatisticsError::InvalidWindowWidth)
            );
        }
        assert_eq!(
            calculate_statistics("移动平均 2"),
            Err(StatisticsError::Incomplete)
        );
    }

    #[test]
    fn reports_signed_exact_pearson_correlation() {
        for (input, expected) in [
            ("相关系数 0,0; 1,1; 2,1", "√(3/4)"),
            ("相关系数 0,0; 1,-1; 2,-1", "-√(3/4)"),
            ("correlation 0,0; 1,1; 2,0", "0"),
            ("相关系数 9007199254740992,1/2; 9007199254740993,-1/2", "-1"),
        ] {
            assert_eq!(
                calculate_statistics(input).as_deref(),
                Ok(expected),
                "{input}"
            );
        }
        for input in ["相关系数 1,1; 1,2", "相关系数 1,2; 2,2"] {
            assert_eq!(
                calculate_statistics(input),
                Err(StatisticsError::Paired(LinearFitError::ConstantCoordinate))
            );
        }
    }

    #[test]
    fn computes_sample_and_population_covariance_for_exact_pairs() {
        for (input, expected) in [
            ("协方差 0,0; 1,1; 2,1", "1/2"),
            ("总体协方差 0,0; 1,1; 2,1", "1/3"),
            ("样本协方差 0,1/2\r\n2,-1/2", "-1"),
            ("COVARIANCE 9007199254740992,0; 9007199254740993,1", "1/2"),
            ("总体协方差 1,1; 1,4", "0"),
        ] {
            assert_eq!(
                calculate_statistics(input).as_deref(),
                Ok(expected),
                "{input}"
            );
        }
        assert_eq!(
            calculate_statistics("协方差 1,2"),
            Err(StatisticsError::Paired(LinearFitError::Incomplete))
        );
        assert_eq!(
            calculate_statistics("协方差 1,2; 3,4,5"),
            Err(StatisticsError::Paired(LinearFitError::InvalidPoint))
        );
    }

    #[test]
    fn computes_exact_weighted_means_with_fractional_and_percentage_weights() {
        for (query, expected) in [
            ("加权平均 80@30%,90@70%", "87"),
            ("加权平均 1/3@0\n7/3@1/2\n1/3@1/2", "4/3"),
            ("weighted-mean comb(4,2)@1.5,2@300%", "10/3"),
            ("加权平均 -2@2e-1,3@3e-1", "1"),
        ] {
            assert_eq!(calculate_statistics(query).as_deref(), Ok(expected));
        }
        for (query, expected) in [
            ("加权平均 4@0,8@0%", StatisticsError::ZeroTotalWeight),
            ("加权平均 4@-1/2,8@1", StatisticsError::InvalidWeight),
            ("加权平均 4,8@1", StatisticsError::InvalidWeight),
            ("加权平均", StatisticsError::Incomplete),
        ] {
            assert_eq!(calculate_statistics(query), Err(expected), "{query}");
        }
    }

    #[test]
    fn calculates_exact_standard_error_of_the_mean_for_samples_and_frequencies() {
        for (query, expected) in [
            ("标准误 1,3,5", "√(4/3)"),
            ("sem 1,3", "1"),
            ("均值标准误 频数 1@2,3@2", "√(1/3)"),
            ("标准误 频数 4@18446744073709551615", "0"),
        ] {
            assert_eq!(
                calculate_statistics(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        for query in ["标准误 5", "标准误 频数 5@1"] {
            assert_eq!(
                calculate_statistics(query),
                Err(StatisticsError::StandardErrorCount)
            );
        }
        assert_eq!(
            calculate_statistics("标准误"),
            Err(StatisticsError::Incomplete)
        );
    }

    #[test]
    fn calculates_exact_harmonic_means_with_frequencies_and_requires_positive_values() {
        for (query, expected) in [
            ("调和平均 36,72", "48"),
            ("harmonic-mean frequency 1/2@2,2@1", "2/3"),
            ("调和平均 2/7", "2/7"),
        ] {
            assert_eq!(calculate_statistics(query).as_deref(), Ok(expected));
        }
        for query in ["调和平均 1,0", "调和平均 频数 -1@2,3@1"] {
            assert_eq!(
                calculate_statistics(query),
                Err(StatisticsError::NonPositiveHarmonicValue)
            );
        }
    }

    #[test]
    fn computes_root_mean_square_with_fractions_and_frequency_groups() {
        for (query, expected) in [
            ("均方根 1/2,3/2", "√(5/4)"),
            ("rms -3", "3"),
            ("均方根 0", "0"),
            ("均方根 频数 0@2,6@1,-8@1", "5"),
        ] {
            assert_eq!(calculate_statistics(query).as_deref(), Ok(expected));
        }
    }

    #[test]
    fn reports_exact_frequency_tables_and_tied_modes() {
        for (query, expected) in [
            (
                "频数表 3,1/2,0.5,3,5",
                "值\t次数\t频率\n1/2\t2\t2/5\n3\t2\t2/5\n5\t1\t1/5",
            ),
            (
                "频数表 频数 5@2,1/3@3,5@1",
                "值\t次数\t频率\n1/3\t3\t1/2\n5\t3\t1/2",
            ),
            ("众数 3,1/2,0.5,3,5", "众数 1/2、3 · 各出现 2 次"),
            (
                "mode 频数 2@18446744073709551615",
                "众数 2 · 各出现 18446744073709551615 次",
            ),
            ("众数 1,2,3", "无众数 · 各值仅出现一次"),
        ] {
            assert_eq!(calculate_statistics(query).as_deref(), Ok(expected));
        }
        assert_eq!(
            calculate_statistics("频数表 频数 5@18446744073709551615,5@1"),
            Err(StatisticsError::FrequencyOverflow)
        );
    }

    #[test]
    fn reports_exact_cumulative_distribution_including_weighted_ties() {
        assert_eq!(
            calculate_statistics("累计分布 3,1/2,0.5,3,5").as_deref(),
            Ok("值\t累计次数\t累计频率\n1/2\t2\t2/5\n3\t4\t4/5\n5\t5\t1")
        );
        assert_eq!(
            calculate_statistics("ecdf 频数 2@18446744073709551614,1@1").as_deref(),
            Ok("值\t累计次数\t累计频率\n1\t1\t1/18446744073709551615\n2\t18446744073709551615\t1")
        );
        assert_eq!(
            calculate_statistics("累计分布 频数 2@18446744073709551615,1@1"),
            Err(StatisticsError::FrequencyOverflow)
        );
    }

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
    fn interpolates_exact_quantiles_across_observations_or_frequency_groups() {
        for (query, expected) in [
            (
                "四分位 4,1,3,2",
                "Q1 7/4 · 中位数 5/2 · Q3 13/4\n四分位距 3/2",
            ),
            (
                "quartiles 频数 1@2,4@2",
                "Q1 1 · 中位数 5/2 · Q3 4\n四分位距 3",
            ),
            ("分位数 1/4 1/2,3/2", "3/4"),
            (
                "分位数 25% 9007199254740993,9007199254740995",
                "18014398509481987/2",
            ),
            ("quantile 0 6,-2,1", "-2"),
            ("分位数 100% 频数 9@18446744073709551615", "9"),
            ("分位数 60% 1", "1"),
        ] {
            assert_eq!(
                calculate_statistics(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        for query in ["分位数", "分位数 -1/4 1,2", "分位数 101% 1,2"] {
            assert_eq!(
                calculate_statistics(query),
                Err(StatisticsError::InvalidProbability),
                "{query}"
            );
        }
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
