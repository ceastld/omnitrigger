use std::{borrow::Borrow, collections::BTreeMap};

use num_bigint::{BigInt, Sign};
use num_rational::BigRational;

use super::bin_plot::BinPlot;
use super::box_plot::BoxPlot;
use super::calculator::{evaluate, format_round_trip, normalize};
use super::csv_tools::append_bounded_csv_row;
use super::fraction::{
    calculate_fraction, combination_count, ensure_exact_size, format_signed_square_root,
    format_square_root, rational_power, FractionError, MAX_COMBINATORIAL_ARGUMENT,
};
use super::linear_fit::{
    calculate_correlation, calculate_covariance, calculate_kendall_correlation,
    calculate_rank_correlation, LinearFitError,
};
use super::matrix::{format_matrix, parse_matrix, MatrixError};
use super::operators::split_top_level_commas;
use super::scatter_plot::ScatterPlot;

pub(super) const MAX_VALUES: usize = 128;
const MAX_BIN_EDGES: usize = 16;
pub(super) const MAX_WEIGHTED_QUANTILES: usize = 16;
const MAX_BINOMIAL_TRIALS: u32 = 128;
const MAX_GEOMETRIC_TRIALS: u32 = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StatisticsError {
    Incomplete,
    TooManyValues,
    InvalidFrequency,
    FrequencyOverflow,
    InvalidWeight,
    ZeroTotalWeight,
    WeightedReportTooLarge,
    WeightedQuantileList,
    InvalidProbability,
    InvalidTrimShare,
    InvalidBinEdges,
    CumulativeThreshold,
    BinomialArguments,
    BinomialTableArguments,
    ProbabilityTableTooLarge,
    GeometricArguments,
    HypergeometricArguments,
    InvalidContingencyTable,
    ZeroContingencyMargin,
    ContingencyInput(MatrixError),
    InvalidWindowWidth,
    WindowQuantileArguments,
    SampleWindowWidth,
    AdjacentValueCount,
    ZeroPreviousValue,
    NonPositiveHarmonicValue,
    InvalidGiniValues,
    GiniMeanDifferenceCount,
    StandardErrorCount,
    StandardScoresConstant,
    VariationSampleCount,
    VariationZeroMean,
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
            Self::InvalidWeight => "填写 数值@非负权重，或粘贴数值和权重两列",
            Self::ZeroTotalWeight => "权重合计须大于零",
            Self::WeightedReportTooLarge => "加权分布结果超过 1 MiB，请缩短数值或减少数据",
            Self::WeightedQuantileList => "用方括号填写 1 至 16 个概率，例如 [25%,50%,75%]",
            Self::InvalidProbability => "输入 0 到 1 的比例或百分数，例如 1/4 或 25%",
            Self::InvalidTrimShare => "每端处理的比例须为 0 到小于 50%，例如 10%",
            Self::InvalidBinEdges => "按升序填写 1 至 16 个不同的分箱边界，用 @ 隔开边界和数值",
            Self::CumulativeThreshold => "用 @ 隔开阈值和观测值，例如 2 @ 1,2,2,3",
            Self::BinomialArguments => "依次输入试验次数、成功次数和单次成功概率，最多 128 次试验",
            Self::BinomialTableArguments => {
                "用逗号填写试验次数和单次成功概率，例如 3,1/3，最多 128 次试验"
            }
            Self::ProbabilityTableTooLarge => "概率表超过 CSV 输出上限",
            Self::GeometricArguments => {
                "用逗号填写试验次数和单次成功概率，例如 3,25%。次数为 1 至 1024，概率须大于零"
            }
            Self::HypergeometricArguments => {
                "依次填写总数、目标项数、抽取数和命中数，例如 10,4,3,2。总数最多 4096"
            }
            Self::InvalidContingencyTable => "填写至少两行两列非负整数，用分号或换行分隔各行",
            Self::ZeroContingencyMargin => "列联表每行和每列的合计须大于零",
            Self::ContingencyInput(error) => error.message(),
            Self::InvalidWindowWidth => "窗口大小须为正整数，且不能超过数值个数",
            Self::WindowQuantileArguments => "先填窗口宽度和分位概率，例如 3,25%，再输入数值",
            Self::SampleWindowWidth => "移动样本方差与样本标准差至少需要两个数值",
            Self::AdjacentValueCount => "计算相邻项的变化至少需要两个数值",
            Self::ZeroPreviousValue => "计算变化率时，每对数值的前一项须非零",
            Self::NonPositiveHarmonicValue => "调和平均需要所有数值大于零",
            Self::InvalidGiniValues => "基尼系数需要非负观测值，且总和须大于零",
            Self::GiniMeanDifferenceCount => "基尼平均差需要至少两个观测值",
            Self::StandardErrorCount => "均值标准误至少需要两个观测值",
            Self::StandardScoresConstant => "标准分需要至少两个不同的观测值",
            Self::VariationSampleCount => "样本变异系数至少需要两个观测值",
            Self::VariationZeroMean => "平均值为零时无法计算变异系数",
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
    Quantiles(QuantileReport),
    Frequencies(FrequencyReport),
    CumulativeAt {
        threshold: BigRational,
        frequency: bool,
    },
    Bins(Vec<BigRational>),
    RootMeanSquare,
    HarmonicMean,
    GiniCoefficient,
    GiniMeanDifference,
    StandardError,
    CoefficientOfVariation {
        sample: bool,
    },
    StandardScores {
        sample: bool,
    },
    MedianAbsoluteDeviation,
    MeanAbsoluteDeviation,
    TailAdjustedMean {
        share: BigRational,
        winsorized: bool,
    },
}

#[derive(Clone, Copy)]
enum FrequencyReport {
    Table,
    Csv,
    Cumulative,
    Mode,
}

pub(super) fn calculate_statistics(query: &str) -> Result<String, StatisticsError> {
    let normalized = normalize(query);
    let input = normalized.trim();
    let (operation, values) = operation_and_arguments(input);
    let operation = operation.to_ascii_lowercase();
    if let Some(report) = WeightedFrequencyReport::from_name(&operation) {
        return weighted_frequency_csv(values.trim(), report);
    }
    let (operation, input) = match operation.as_str() {
        "总体" | "population" => (StatisticsOperation::Population, values.trim()),
        "样本" | "sample" => (StatisticsOperation::Sample, values.trim()),
        "四分位" | "quartiles" => (
            StatisticsOperation::Quantiles(QuantileReport::Quartiles),
            values.trim(),
        ),
        "箱线摘要" | "box-summary" | "箱线图" | "box-plot" => (
            StatisticsOperation::Quantiles(QuantileReport::BoxSummary),
            values.trim(),
        ),
        "频数表" | "frequency-table" => (
            StatisticsOperation::Frequencies(FrequencyReport::Table),
            values.trim(),
        ),
        "频数csv" | "frequency-csv" => (
            StatisticsOperation::Frequencies(FrequencyReport::Csv),
            values.trim(),
        ),
        "累计分布"
        | "cumulative-distribution"
        | "ecdf"
        | "累计分布图"
        | "cumulative-distribution-plot"
        | "ecdf-plot" => (
            StatisticsOperation::Frequencies(FrequencyReport::Cumulative),
            values.trim(),
        ),
        operation @ ("累计次数" | "cumulative-count" | "累计频率" | "cumulative-frequency") =>
        {
            let (threshold, input) = cumulative_arguments(values)?;
            (
                StatisticsOperation::CumulativeAt {
                    threshold,
                    frequency: matches!(operation, "累计频率" | "cumulative-frequency"),
                },
                input,
            )
        }
        "众数" | "mode" => (
            StatisticsOperation::Frequencies(FrequencyReport::Mode),
            values.trim(),
        ),
        "分箱" | "histogram" | "分箱图" | "bin-plot" => {
            let (edges, input) = bin_arguments(values)?;
            (StatisticsOperation::Bins(edges), input)
        }
        "均方根" | "rms" => (StatisticsOperation::RootMeanSquare, values.trim()),
        "调和平均" | "harmonic-mean" => (StatisticsOperation::HarmonicMean, values.trim()),
        "基尼系数" | "gini" | "gini-coefficient" => {
            (StatisticsOperation::GiniCoefficient, values.trim())
        }
        "基尼平均差" | "gini-mean-difference" => {
            (StatisticsOperation::GiniMeanDifference, values.trim())
        }
        "标准误" | "均值标准误" | "sem" | "standard-error" => {
            (StatisticsOperation::StandardError, values.trim())
        }
        "变异系数" | "样本变异系数" | "coefficient-of-variation" => (
            StatisticsOperation::CoefficientOfVariation { sample: true },
            values.trim(),
        ),
        "总体变异系数" | "population-coefficient-of-variation" => (
            StatisticsOperation::CoefficientOfVariation { sample: false },
            values.trim(),
        ),
        "标准分" | "z-score" => (
            StatisticsOperation::StandardScores { sample: true },
            values.trim(),
        ),
        "总体标准分" | "population-z-score" => (
            StatisticsOperation::StandardScores { sample: false },
            values.trim(),
        ),
        "中位绝对偏差" | "mad" | "median-absolute-deviation" => {
            (StatisticsOperation::MedianAbsoluteDeviation, values.trim())
        }
        "平均绝对偏差" | "aad" | "average-absolute-deviation" | "mean-absolute-deviation" => {
            (StatisticsOperation::MeanAbsoluteDeviation, values.trim())
        }
        "加权平均" | "weighted-mean" => return weighted_mean(values.trim()),
        "加权方差" | "加权总体方差" | "weighted-variance" => {
            return weighted_variance(values.trim()).map(|variance| variance.to_string());
        }
        "加权标准差" | "加权总体标准差" | "weighted-stddev" => {
            return weighted_variance(values.trim()).map(|variance| format_square_root(&variance));
        }
        "加权分位数" | "weighted-quantile" => return weighted_quantile(values.trim()),
        "加权分位表" | "weighted-quantiles-csv" => {
            return weighted_quantiles_csv(values.trim())
        }
        "加权累计权重" | "weighted-cumulative-weight" => {
            return weighted_cumulative_value(values, false);
        }
        "加权累计比例" | "weighted-cumulative-share" => {
            return weighted_cumulative_value(values, true);
        }
        "移动合计" | "moving-sum" => return moving_sum_or_average(values.trim(), false),
        "移动平均" | "moving-average" => return moving_sum_or_average(values.trim(), true),
        "移动样本方差" | "moving-sample-variance" => {
            return moving_dispersion(values.trim(), true, ToString::to_string)
        }
        "移动总体方差" | "moving-population-variance" => {
            return moving_dispersion(values.trim(), false, ToString::to_string)
        }
        "移动样本标准差" | "moving-sample-standard-deviation" => {
            return moving_dispersion(values.trim(), true, format_square_root)
        }
        "移动总体标准差" | "moving-population-standard-deviation" => {
            return moving_dispersion(values.trim(), false, format_square_root)
        }
        "移动中位数" | "moving-median" => return moving_median(values.trim()),
        "移动分位数" | "moving-quantile" => return moving_quantile(values.trim()),
        "移动最小" | "moving-min" => {
            return moving_extrema(values.trim(), MovingExtrema::Minimum)
        }
        "移动最大" | "moving-max" => {
            return moving_extrema(values.trim(), MovingExtrema::Maximum)
        }
        "移动极差" | "moving-range" => {
            return moving_extrema(values.trim(), MovingExtrema::Range)
        }
        "累计合计" | "running-sum" => return running_totals(values.trim(), false),
        "累计平均" | "running-average" => return running_totals(values.trim(), true),
        "二项概率" | "binomial" => return binomial_probability(values, BinomialEvent::Exactly),
        "二项概率表" | "binomial-table" => return binomial_csv(values),
        "二项累计" | "binomial-cdf" => {
            return binomial_probability(values, BinomialEvent::AtMost)
        }
        "二项至少" | "binomial-at-least" => {
            return binomial_probability(values, BinomialEvent::AtLeast)
        }
        "几何概率" | "geometric" => return geometric_probability(values, false),
        "几何累计" | "geometric-cdf" => return geometric_probability(values, true),
        "超几何概率" | "hypergeometric" => return hypergeometric_probability(values),
        "独立性卡方" | "chi-square" => return contingency_table(values, false),
        "期望频数" | "expected-counts" => return contingency_table(values, true),
        "差分" | "相邻差" | "difference" => return adjacent_changes(values.trim(), false),
        "变化率" | "relative-change" => return adjacent_changes(values.trim(), true),
        "协方差" | "样本协方差" | "covariance" => {
            return calculate_covariance(values, true).map_err(StatisticsError::Paired);
        }
        "总体协方差" | "population-covariance" => {
            return calculate_covariance(values, false).map_err(StatisticsError::Paired);
        }
        "相关系数" | "correlation" => {
            return calculate_correlation(values).map_err(StatisticsError::Paired);
        }
        "秩相关" | "斯皮尔曼" | "spearman" => {
            return calculate_rank_correlation(values).map_err(StatisticsError::Paired);
        }
        "肯德尔" | "肯德尔秩相关" | "kendall" => {
            return calculate_kendall_correlation(values).map_err(StatisticsError::Paired);
        }
        "分位数" | "quantile" => {
            let (probability, values) = operation_and_arguments(values.trim());
            (
                StatisticsOperation::Quantiles(QuantileReport::Single(parse_probability(
                    probability,
                )?)),
                values.trim(),
            )
        }
        operation @ ("截尾平均" | "trimmed-mean" | "缩尾平均" | "winsorized-mean") => {
            let (share, values) = operation_and_arguments(values.trim());
            let share = parse_probability(share)?;
            if share >= BigRational::new(BigInt::from(1u8), BigInt::from(2u8)) {
                return Err(StatisticsError::InvalidTrimShare);
            }
            (
                StatisticsOperation::TailAdjustedMean {
                    share,
                    winsorized: matches!(operation, "缩尾平均" | "winsorized-mean"),
                },
                values.trim(),
            )
        }
        _ => (StatisticsOperation::Sample, input),
    };
    let (values, count) = read_observations(input)?;
    match operation {
        StatisticsOperation::Sample | StatisticsOperation::Population => summarize(
            values,
            count,
            matches!(operation, StatisticsOperation::Sample),
        ),
        StatisticsOperation::Quantiles(report) => report_quantiles(values, count, report),
        StatisticsOperation::Frequencies(report) => Ok(report_frequencies(values, count, report)),
        StatisticsOperation::CumulativeAt {
            threshold,
            frequency,
        } => {
            let matching: u64 = values
                .into_iter()
                .filter(|(value, _)| value <= &threshold)
                .map(|(_, repetitions)| repetitions)
                .sum();
            if frequency {
                Ok(BigRational::new(BigInt::from(matching), BigInt::from(count)).to_string())
            } else {
                Ok(matching.to_string())
            }
        }
        StatisticsOperation::Bins(edges) => Ok(report_binned_frequencies(values, count, &edges)),
        StatisticsOperation::MedianAbsoluteDeviation => {
            median_absolute_deviation(values, count).map(|value| value.to_string())
        }
        StatisticsOperation::MeanAbsoluteDeviation => {
            mean_absolute_deviation(values, count).map(|value| value.to_string())
        }
        StatisticsOperation::TailAdjustedMean { share, winsorized } => {
            tail_adjusted_mean(values, count, share, winsorized).map(|value| value.to_string())
        }
        StatisticsOperation::GiniCoefficient => {
            gini_coefficient(values, count).map(|value| value.to_string())
        }
        StatisticsOperation::GiniMeanDifference => {
            gini_mean_difference(values, count).map(|value| value.to_string())
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
        StatisticsOperation::StandardScores { sample } => standard_scores(&values, count, sample),
        StatisticsOperation::CoefficientOfVariation { sample } => {
            coefficient_of_variation(&values, count, sample)
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
    let mut totals = RunningTotals::default();
    let mut output = Vec::with_capacity(terms.len());
    for term in terms {
        let value = calculate_fraction(term).map_err(StatisticsError::Evaluation)?;
        totals.push(value)?;
        output.push(totals.format(average)?);
    }
    Ok(output.join("\n"))
}

#[derive(Default)]
pub(super) struct RunningTotals {
    total: BigRational,
    count: usize,
}

impl RunningTotals {
    pub(super) fn push(&mut self, value: BigRational) -> Result<(), StatisticsError> {
        self.total = bounded(&self.total + value)?;
        self.count += 1;
        Ok(())
    }

    pub(super) fn format(&self, average: bool) -> Result<String, StatisticsError> {
        if !average {
            return Ok(self.total.to_string());
        }
        if self.count == 0 {
            return Ok(String::new());
        }
        bounded(&self.total / BigInt::from(self.count)).map(|value| value.to_string())
    }
}

fn adjacent_changes(input: &str, relative: bool) -> Result<String, StatisticsError> {
    let terms = observation_terms(input)?;
    if terms.len() < 2 {
        return Err(StatisticsError::AdjacentValueCount);
    }
    let values = terms
        .into_iter()
        .map(|term| calculate_fraction(term).map_err(StatisticsError::Evaluation))
        .collect::<Result<Vec<_>, _>>()?;
    values
        .windows(2)
        .map(|pair| {
            let previous = &pair[0];
            let difference = bounded(&pair[1] - previous)?;
            if relative {
                if previous.numer().sign() == Sign::NoSign {
                    return Err(StatisticsError::ZeroPreviousValue);
                }
                bounded(difference / previous).map(|value| value.to_string())
            } else {
                Ok(difference.to_string())
            }
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|lines| lines.join("\n"))
}

fn sliding_window_values(input: &str) -> Result<(usize, Vec<BigRational>), StatisticsError> {
    let (width, input) = operation_and_arguments(input);
    parse_window_values(width, input)
}

fn parse_window_values(
    width: &str,
    input: &str,
) -> Result<(usize, Vec<BigRational>), StatisticsError> {
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

fn moving_sum_or_average(input: &str, average: bool) -> Result<String, StatisticsError> {
    let (width, values) = sliding_window_values(input)?;
    let divisor = average.then(|| BigRational::from_integer(BigInt::from(width)));
    let mut sum = BigRational::default();
    let mut results = Vec::with_capacity(values.len() - width + 1);
    for (index, value) in values.iter().enumerate() {
        if index >= width {
            sum -= &values[index - width];
        }
        sum = bounded(sum + value)?;
        if index + 1 >= width {
            results.push(match &divisor {
                Some(divisor) => bounded(&sum / divisor)?.to_string(),
                None => sum.to_string(),
            });
        }
    }
    Ok(results.join("\n"))
}

fn moving_dispersion(
    input: &str,
    sample: bool,
    format: fn(&BigRational) -> String,
) -> Result<String, StatisticsError> {
    let (width, values) = sliding_window_values(input)?;
    let divisor = width - usize::from(sample);
    if divisor == 0 {
        return Err(StatisticsError::SampleWindowWidth);
    }
    let count = BigRational::from_integer(BigInt::from(width));
    let divisor = BigRational::from_integer(BigInt::from(divisor));
    let observations: Vec<_> = values.into_iter().map(|value| (value, 1)).collect();
    let mut results = Vec::with_capacity(observations.len() - width + 1);
    for window in observations.windows(width) {
        let mean = bounded(observation_sum(window)? / &count)?;
        let sum = squared_deviation_sum(window, &mean)?;
        results.push(format(&bounded(sum / &divisor)?));
    }
    Ok(results.join("\n"))
}

fn moving_median(input: &str) -> Result<String, StatisticsError> {
    let (width, values) = sliding_window_values(input)?;
    let mut ordered: Vec<&BigRational> = Vec::with_capacity(width);
    let mut medians = Vec::with_capacity(values.len() - width + 1);
    for window in values.windows(width) {
        ordered.extend(window);
        ordered.sort_unstable();
        let median = median_sorted_values(&ordered).map_err(StatisticsError::Evaluation)?;
        medians.push(median.to_string());
        ordered.clear();
    }
    Ok(medians.join("\n"))
}

fn moving_quantile(input: &str) -> Result<String, StatisticsError> {
    let (parameters, source) = operation_and_arguments(input);
    let (width, probability) = parameters
        .split_once(',')
        .ok_or(StatisticsError::WindowQuantileArguments)?;
    let probability = parse_probability(probability)?;
    let (width, values) = parse_window_values(width, source)?;
    let mut ordered: Vec<&BigRational> = Vec::with_capacity(width);
    let mut results = Vec::with_capacity(values.len() - width + 1);
    for window in values.windows(width) {
        ordered.extend(window);
        ordered.sort_unstable();
        let result = interpolate_ordered_quantile(width as u64, &probability, |rank| {
            ordered[rank as usize]
        })?;
        results.push(result.to_string());
        ordered.clear();
    }
    Ok(results.join("\n"))
}

pub(super) fn median_sorted_values(
    values: &[impl Borrow<BigRational>],
) -> Result<BigRational, FractionError> {
    let count = values.len();
    let middle = values[count / 2].borrow();
    if count.is_multiple_of(2) {
        let previous = values[count / 2 - 1].borrow();
        let value = (previous + middle) / BigInt::from(2);
        ensure_exact_size(&value)?;
        Ok(value)
    } else {
        Ok(middle.clone())
    }
}

#[derive(Clone, Copy)]
enum MovingExtrema {
    Minimum,
    Maximum,
    Range,
}

fn moving_extrema(input: &str, operation: MovingExtrema) -> Result<String, StatisticsError> {
    let (width, values) = sliding_window_values(input)?;
    let results = values
        .windows(width)
        .map(|window| {
            let minimum = || window.iter().min().expect("窗口至少包含一个数值");
            let maximum = || window.iter().max().expect("窗口至少包含一个数值");
            Ok(match operation {
                MovingExtrema::Minimum => minimum().to_string(),
                MovingExtrema::Maximum => maximum().to_string(),
                MovingExtrema::Range => bounded(maximum() - minimum())?.to_string(),
            })
        })
        .collect::<Result<Vec<_>, StatisticsError>>()?;
    Ok(results.join("\n"))
}

fn weighted_mean(input: &str) -> Result<String, StatisticsError> {
    let mut mean = WeightedMean::default();
    for term in observation_terms(input)? {
        mean.record(read_weighted_observation(term)?)?;
    }
    mean.finish()
}

#[derive(Default)]
pub(super) struct WeightedMean {
    total_value: BigRational,
    total_weight: BigRational,
}

impl WeightedMean {
    pub(super) fn record(
        &mut self,
        (value, weight): (BigRational, BigRational),
    ) -> Result<(), StatisticsError> {
        let total_value = bounded(&self.total_value + value * &weight)?;
        let total_weight = bounded(&self.total_weight + weight)?;
        self.total_value = total_value;
        self.total_weight = total_weight;
        Ok(())
    }

    pub(super) fn finish(self) -> Result<String, StatisticsError> {
        if self.total_weight.numer().sign() == Sign::NoSign {
            return Err(StatisticsError::ZeroTotalWeight);
        }
        bounded(self.total_value / self.total_weight).map(|value| value.to_string())
    }

    pub(super) fn total_weight(&self) -> &BigRational {
        &self.total_weight
    }
}

fn weighted_variance(input: &str) -> Result<BigRational, StatisticsError> {
    let distribution = read_weighted_distribution(input)?;
    let total_value = distribution
        .weights
        .iter()
        .try_fold(BigRational::default(), |total, (value, weight)| {
            bounded(total + value * weight)
        })?;
    let mean = bounded(total_value / &distribution.total)?;
    let sum_of_squares = distribution.weights.into_iter().try_fold(
        BigRational::default(),
        |total, (value, weight)| {
            let distance = value - &mean;
            bounded(total + &distance * &distance * weight)
        },
    )?;
    bounded(sum_of_squares / distribution.total)
}

fn weighted_quantile(input: &str) -> Result<String, StatisticsError> {
    let (probability, input) = operation_and_arguments(input);
    let probability = parse_probability(probability)?;
    let distribution = read_weighted_distribution(input.trim())?;
    weighted_quantile_value(&distribution, probability).map(ToString::to_string)
}

fn weighted_quantile_value(
    distribution: &WeightedDistribution,
    probability: BigRational,
) -> Result<&BigRational, StatisticsError> {
    let target = bounded(&distribution.total * probability)?;
    let mut values = distribution.weights.iter();
    let (last_value, _) = values.next_back().ok_or(StatisticsError::ZeroTotalWeight)?;
    let mut cumulative = BigRational::default();
    for (value, weight) in values {
        cumulative = bounded(cumulative + weight)?;
        if cumulative >= target {
            return Ok(value);
        }
    }
    Ok(last_value)
}

fn weighted_quantiles_csv(input: &str) -> Result<String, StatisticsError> {
    let (probabilities, values) = input
        .strip_prefix('[')
        .and_then(|input| input.split_once(']'))
        .ok_or(StatisticsError::WeightedQuantileList)?;
    let probabilities: Vec<_> = split_top_level_commas(probabilities)
        .take(MAX_WEIGHTED_QUANTILES + 1)
        .collect();
    if probabilities.is_empty()
        || probabilities.len() > MAX_WEIGHTED_QUANTILES
        || probabilities.iter().any(|value| value.trim().is_empty())
    {
        return Err(StatisticsError::WeightedQuantileList);
    }
    let probabilities = probabilities
        .into_iter()
        .map(|value| parse_probability(value.trim()))
        .collect::<Result<Vec<_>, _>>()?;
    let distribution = read_weighted_distribution(values.trim_start())?;
    distribution.quantiles_csv(probabilities)
}

#[derive(Default)]
pub(super) struct WeightedDistribution {
    weights: BTreeMap<BigRational, BigRational>,
    total: BigRational,
}

impl WeightedDistribution {
    pub(super) fn record(
        &mut self,
        (value, weight): (BigRational, BigRational),
    ) -> Result<(), StatisticsError> {
        match weight.numer().sign() {
            Sign::NoSign => return Ok(()),
            Sign::Minus => return Err(StatisticsError::InvalidWeight),
            Sign::Plus => {}
        }
        let total = bounded(&self.total + &weight)?;
        let group_weight = match self.weights.get(&value) {
            Some(previous) => bounded(previous + weight)?,
            None => bounded(weight)?,
        };
        self.weights.insert(value, group_weight);
        self.total = total;
        Ok(())
    }

    pub(super) fn quantiles_csv(
        &self,
        probabilities: Vec<BigRational>,
    ) -> Result<String, StatisticsError> {
        if self.weights.is_empty() {
            return Err(StatisticsError::ZeroTotalWeight);
        }
        let mut csv = String::from("概率,分位数");
        for probability in probabilities {
            let label = probability.to_string();
            let value = weighted_quantile_value(self, probability)?.to_string();
            append_bounded_csv_row(
                &mut csv,
                [label.as_str(), value.as_str()].into_iter(),
                StatisticsError::WeightedReportTooLarge,
            )?;
        }
        Ok(csv)
    }
}

fn cumulative_arguments(input: &str) -> Result<(BigRational, &str), StatisticsError> {
    let (threshold, values) = input
        .split_once('@')
        .filter(|(threshold, _)| !threshold.trim().is_empty())
        .ok_or(StatisticsError::CumulativeThreshold)?;
    let threshold = calculate_fraction(threshold.trim()).map_err(StatisticsError::Evaluation)?;
    Ok((threshold, values.trim()))
}

fn weighted_cumulative_value(input: &str, as_share: bool) -> Result<String, StatisticsError> {
    let (threshold, values) = cumulative_arguments(input)?;
    let distribution = read_weighted_distribution(values)?;
    let weight = distribution
        .weights
        .range(..=threshold)
        .try_fold(BigRational::default(), |sum, (_, weight)| {
            bounded(sum + weight)
        })?;
    let result = if as_share {
        bounded(weight / distribution.total)?
    } else {
        weight
    };
    Ok(result.to_string())
}

fn read_weighted_distribution(input: &str) -> Result<WeightedDistribution, StatisticsError> {
    let mut distribution = WeightedDistribution::default();
    for term in observation_terms(input)? {
        distribution.record(read_weighted_observation(term)?)?;
    }
    if distribution.weights.is_empty() {
        return Err(StatisticsError::ZeroTotalWeight);
    }
    Ok(distribution)
}

enum WeightedFrequencyReport {
    Individual,
    Cumulative,
}

impl WeightedFrequencyReport {
    fn from_name(operation: &str) -> Option<Self> {
        match operation {
            "加权频数csv" | "weighted-frequency-csv" => Some(Self::Individual),
            "加权累积分布" | "weighted-cdf" => Some(Self::Cumulative),
            _ => None,
        }
    }
}

pub(super) fn statistics_csv_report(query: &str) -> Option<Result<String, StatisticsError>> {
    let (operation, _) = operation_and_arguments(query.trim());
    let operation = normalize(operation).to_ascii_lowercase();
    (WeightedFrequencyReport::from_name(&operation).is_some()
        || matches!(
            operation.as_str(),
            "频数csv"
                | "frequency-csv"
                | "加权分位表"
                | "weighted-quantiles-csv"
                | "二项概率表"
                | "binomial-table"
                | "累计分布"
                | "cumulative-distribution"
                | "ecdf"
                | "分箱"
                | "histogram"
        ))
    .then(|| calculate_statistics(query))
}

fn weighted_frequency_csv(
    input: &str,
    report: WeightedFrequencyReport,
) -> Result<String, StatisticsError> {
    let distribution = read_weighted_distribution(input)?;
    let mut cumulative = BigRational::default();
    let mut csv = String::from(match report {
        WeightedFrequencyReport::Individual => "值,权重,比例",
        WeightedFrequencyReport::Cumulative => "值,累计权重,累计比例",
    });
    for (value, weight) in distribution.weights {
        let reported_weight = match report {
            WeightedFrequencyReport::Individual => &weight,
            WeightedFrequencyReport::Cumulative => {
                cumulative = bounded(cumulative + &weight)?;
                &cumulative
            }
        };
        let share = bounded(reported_weight / &distribution.total)?;
        let fields = [
            value.to_string(),
            reported_weight.to_string(),
            share.to_string(),
        ];
        append_bounded_csv_row(
            &mut csv,
            fields.iter().map(String::as_str),
            StatisticsError::WeightedReportTooLarge,
        )?;
    }
    Ok(csv)
}

fn read_weighted_observation(term: &str) -> Result<(BigRational, BigRational), StatisticsError> {
    let (value, weight) = term
        .split_once('@')
        .or_else(|| {
            term.split_once('\t')
                .filter(|(_, weight)| !weight.contains('\t'))
        })
        .ok_or(StatisticsError::InvalidWeight)?;
    parse_weighted_pair(value, weight)
}

pub(super) fn parse_weighted_pair(
    value: &str,
    weight: &str,
) -> Result<(BigRational, BigRational), StatisticsError> {
    let value = calculate_fraction(value).map_err(StatisticsError::Evaluation)?;
    Ok((value, parse_statistical_weight(weight)?))
}

pub(super) fn parse_statistical_weight(weight: &str) -> Result<BigRational, StatisticsError> {
    let weight = if let Some(percent) = weight.trim().strip_suffix('%') {
        calculate_fraction(percent).map(|value| value / BigInt::from(100))
    } else {
        calculate_fraction(weight)
    }
    .map_err(StatisticsError::Evaluation)?;
    if weight.numer().sign() == Sign::Minus {
        return Err(StatisticsError::InvalidWeight);
    }
    Ok(weight)
}

fn report_frequencies(
    values: Vec<(BigRational, u64)>,
    count: u64,
    report: FrequencyReport,
) -> String {
    let frequencies = frequency_groups(values);
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
    if matches!(report, FrequencyReport::Cumulative) {
        return cumulative_csv(&frequencies, count);
    }
    let (header, delimiter, newline) = if matches!(report, FrequencyReport::Csv) {
        ("值,次数,频率", ',', "\r\n")
    } else {
        ("值\t次数\t频率", '\t', "\n")
    };
    std::iter::once(header.to_owned())
        .chain(frequencies.into_iter().map(|(value, repetitions)| {
            let share = BigRational::new(repetitions.into(), count.into());
            format!("{value}{delimiter}{repetitions}{delimiter}{share}")
        }))
        .collect::<Vec<_>>()
        .join(newline)
}

fn frequency_groups(values: Vec<(BigRational, u64)>) -> BTreeMap<BigRational, u64> {
    let mut frequencies = BTreeMap::<_, u64>::new();
    for (value, repetitions) in values {
        // 频数合计已在调用前检查，任一分组都不会超过合计。
        *frequencies.entry(value).or_default() += repetitions;
    }
    frequencies
}

fn cumulative_rows(
    frequencies: &BTreeMap<BigRational, u64>,
    count: u64,
) -> impl Iterator<Item = (&BigRational, u64, BigRational)> {
    frequencies
        .iter()
        .scan(0u64, move |total, (value, count_at_value)| {
            *total += *count_at_value;
            Some((
                value,
                *total,
                BigRational::new((*total).into(), count.into()),
            ))
        })
}

fn cumulative_csv(frequencies: &BTreeMap<BigRational, u64>, count: u64) -> String {
    std::iter::once("值,累计次数,累计频率".to_owned())
        .chain(
            cumulative_rows(frequencies, count)
                .map(|(value, total, share)| format!("{value},{total},{share}")),
        )
        .collect::<Vec<_>>()
        .join("\r\n")
}

pub(super) fn cumulative_plot_arguments(input: &str) -> Option<&str> {
    let (operation, values) = operation_and_arguments(input.trim());
    (operation == "累计分布图"
        || operation.eq_ignore_ascii_case("cumulative-distribution-plot")
        || operation.eq_ignore_ascii_case("ecdf-plot"))
    .then_some(values.trim())
}

pub(super) fn plot_cumulative_distribution(
    input: &str,
) -> Result<(String, ScatterPlot), StatisticsError> {
    let input = normalize(input);
    let (values, count) = read_observations(input.trim())?;
    Ok(prepare_cumulative_plot(values, count))
}

pub(super) fn report_cumulative_unweighted(
    values: Vec<BigRational>,
) -> Result<String, StatisticsError> {
    let values = unweighted_observations(values)?;
    let count = values.len() as u64;
    Ok(cumulative_csv(&frequency_groups(values), count))
}

pub(super) fn plot_cumulative_unweighted(
    values: Vec<BigRational>,
) -> Result<(String, ScatterPlot), StatisticsError> {
    let values = unweighted_observations(values)?;
    let count = values.len() as u64;
    Ok(prepare_cumulative_plot(values, count))
}

fn prepare_cumulative_plot(values: Vec<(BigRational, u64)>, count: u64) -> (String, ScatterPlot) {
    let frequencies = frequency_groups(values);
    let points = cumulative_rows(&frequencies, count)
        .map(|(value, _, share)| (value.clone(), share))
        .collect::<Vec<_>>();
    let plot = ScatterPlot::from_cumulative_distribution(&points, count);
    (cumulative_csv(&frequencies, count), plot)
}

pub(super) fn parse_bin_edges(input: &str) -> Result<Vec<BigRational>, StatisticsError> {
    parse_bin_edges_with(input, |term| {
        calculate_fraction(term).map_err(StatisticsError::Evaluation)
    })
}

fn parse_bin_edges_with<Value: PartialOrd>(
    input: &str,
    evaluate_term: impl Fn(&str) -> Result<Value, StatisticsError>,
) -> Result<Vec<Value>, StatisticsError> {
    let terms: Vec<_> = split_top_level_commas(input)
        .take(MAX_BIN_EDGES + 1)
        .collect();
    if terms.is_empty()
        || terms.len() > MAX_BIN_EDGES
        || terms.iter().any(|term| term.trim().is_empty())
    {
        return Err(StatisticsError::InvalidBinEdges);
    }
    let edges: Vec<_> = terms
        .into_iter()
        .map(evaluate_term)
        .collect::<Result<_, _>>()?;
    if edges.windows(2).any(|edge| edge[0] >= edge[1]) {
        return Err(StatisticsError::InvalidBinEdges);
    }
    Ok(edges)
}

fn bin_arguments(input: &str) -> Result<(Vec<BigRational>, &str), StatisticsError> {
    let (edges, data) = input
        .split_once('@')
        .ok_or(StatisticsError::InvalidBinEdges)?;
    Ok((parse_bin_edges(edges)?, data.trim()))
}

pub(super) fn bin_plot_arguments(input: &str) -> Option<&str> {
    let (operation, values) = operation_and_arguments(input.trim());
    (operation == "分箱图" || operation.eq_ignore_ascii_case("bin-plot")).then_some(values.trim())
}

pub(super) fn calculate_bin_plot(input: &str) -> Result<(String, BinPlot), StatisticsError> {
    let input = normalize(input);
    let (edges, data) = bin_arguments(&input)?;
    let (values, count) = read_observations(data)?;
    Ok(plot_binned_frequencies(values, count, &edges))
}

pub(super) fn plot_binned_unweighted(
    values: Vec<BigRational>,
    edges: &[BigRational],
) -> Result<(String, BinPlot), StatisticsError> {
    let values = unweighted_observations(values)?;
    let count = values.len() as u64;
    Ok(plot_binned_frequencies(values, count, edges))
}

fn plot_binned_frequencies(
    values: Vec<(BigRational, u64)>,
    count: u64,
    edges: &[BigRational],
) -> (String, BinPlot) {
    let bins = count_bins(values, edges);
    plot_bin_counts(
        bin_labels(edges.iter().map(ToString::to_string)),
        bins,
        count,
    )
}

pub(super) fn plot_binned_float_values(
    values: Vec<f64>,
    input: &str,
) -> Result<(String, BinPlot), StatisticsError> {
    let edges = parse_bin_edges_with(input, |term| {
        evaluate(term).map_err(|_| StatisticsError::InvalidBinEdges)
    })?;
    if values.is_empty() {
        return Err(StatisticsError::Incomplete);
    }
    let count = values.len() as u64;
    let bins = count_bins(values.into_iter().map(|value| (value, 1)), &edges);
    let labels = bin_labels(edges.into_iter().map(format_round_trip));
    Ok(plot_bin_counts(labels, bins, count))
}

fn plot_bin_counts(labels: Vec<String>, bins: Vec<u64>, count: u64) -> (String, BinPlot) {
    let csv = bin_csv(&labels, &bins, count);
    (csv, BinPlot::new(labels, bins, count))
}

fn report_binned_frequencies(
    values: Vec<(BigRational, u64)>,
    count: u64,
    edges: &[BigRational],
) -> String {
    let labels = bin_labels(edges.iter().map(ToString::to_string));
    bin_csv(&labels, &count_bins(values, edges), count)
}

fn count_bins<Value: PartialOrd>(
    values: impl IntoIterator<Item = (Value, u64)>,
    edges: &[Value],
) -> Vec<u64> {
    let mut bins = vec![0; edges.len() + 1];
    for (value, occurrences) in values {
        let index = edges.partition_point(|edge| edge <= &value);
        // 各组频数的总和已在调用前检查。
        bins[index] += occurrences;
    }
    bins
}

fn bin_labels(edges: impl Iterator<Item = String>) -> Vec<String> {
    std::iter::once("-∞".to_owned())
        .chain(edges)
        .chain(std::iter::once("+∞".to_owned()))
        .collect()
}

fn bin_csv(edges: &[String], bins: &[u64], count: u64) -> String {
    std::iter::once("下界,上界,次数,频率".to_owned())
        .chain(edges.windows(2).zip(bins).map(|(edge, &occurrences)| {
            let share = BigRational::new(occurrences.into(), count.into());
            format!("{},{},{occurrences},{share}", edge[0], edge[1])
        }))
        .collect::<Vec<_>>()
        .join("\r\n")
}

pub(super) fn summarize_unweighted(
    values: Vec<BigRational>,
    sample: bool,
) -> Result<String, StatisticsError> {
    let values = unweighted_observations(values)?;
    let count = values.len() as u64;
    summarize(values, count, sample)
}

pub(super) fn report_binned_unweighted(
    values: Vec<BigRational>,
    edges: &[BigRational],
) -> Result<String, StatisticsError> {
    let values = unweighted_observations(values)?;
    let count = values.len() as u64;
    Ok(report_binned_frequencies(values, count, edges))
}

fn unweighted_observations(
    values: Vec<BigRational>,
) -> Result<Vec<(BigRational, u64)>, StatisticsError> {
    if values.is_empty() {
        return Err(StatisticsError::Incomplete);
    }
    if values.len() > MAX_VALUES {
        return Err(StatisticsError::TooManyValues);
    }
    Ok(values.into_iter().map(|value| (value, 1)).collect())
}

fn observation_sum(values: &[(BigRational, u64)]) -> Result<BigRational, StatisticsError> {
    values
        .iter()
        .try_fold(BigRational::default(), |sum, (value, repetitions)| {
            bounded(sum + value * BigInt::from(*repetitions))
        })
}

fn coefficient_of_variation(
    values: &[(BigRational, u64)],
    count: u64,
    sample: bool,
) -> Result<String, StatisticsError> {
    let divisor = count - u64::from(sample);
    if divisor == 0 {
        return Err(StatisticsError::VariationSampleCount);
    }
    let average = bounded(observation_sum(values)? / BigInt::from(count))?;
    let sign = average.numer().sign();
    if sign == Sign::NoSign {
        return Err(StatisticsError::VariationZeroMean);
    }
    let deviation_sum = squared_deviation_sum(values, &average)?;
    let variance = bounded(deviation_sum / BigInt::from(divisor))?;
    let squared = bounded(variance / (&average * &average))?;
    if squared.numer().sign() == Sign::NoSign {
        return Ok("0".into());
    }
    Ok(format_signed_square_root(&squared, sign))
}

fn standard_scores(
    values: &[(BigRational, u64)],
    count: u64,
    sample: bool,
) -> Result<String, StatisticsError> {
    let average = bounded(observation_sum(values)? / BigInt::from(count))?;
    let deviation_sum = squared_deviation_sum(values, &average)?;
    if deviation_sum == BigRational::default() {
        return Err(StatisticsError::StandardScoresConstant);
    }
    let variance = bounded(deviation_sum / BigInt::from(count - u64::from(sample)))?;
    let mut rows = Vec::with_capacity(values.len());
    for (value, _) in values {
        let difference = value - &average;
        let squared_score = bounded(&difference * &difference / &variance)?;
        rows.push(format_signed_square_root(
            &squared_score,
            difference.numer().sign(),
        ));
    }
    Ok(rows.join("\n"))
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

fn gini_coefficient(
    values: Vec<(BigRational, u64)>,
    count: u64,
) -> Result<BigRational, StatisticsError> {
    if values
        .iter()
        .any(|(value, _)| value.numer().sign() == Sign::Minus)
    {
        return Err(StatisticsError::InvalidGiniValues);
    }
    let (value_sum, difference_sum) = gini_totals(values)?;
    if value_sum.numer().sign() == Sign::NoSign {
        return Err(StatisticsError::InvalidGiniValues);
    }
    bounded(difference_sum / (value_sum * BigInt::from(count)))
}

fn gini_mean_difference(
    values: Vec<(BigRational, u64)>,
    count: u64,
) -> Result<BigRational, StatisticsError> {
    if count < 2 {
        return Err(StatisticsError::GiniMeanDifferenceCount);
    }
    let (_, difference_sum) = gini_totals(values)?;
    let twice_pairs = BigInt::from(count) * BigInt::from(count - 1);
    bounded(difference_sum * BigInt::from(2) / twice_pairs)
}

fn gini_totals(
    mut values: Vec<(BigRational, u64)>,
) -> Result<(BigRational, BigRational), StatisticsError> {
    values.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
    let mut seen = 0_u64;
    let mut value_sum = BigRational::default();
    let mut difference_sum = BigRational::default();
    for (value, repetitions) in values {
        let multiplicity = BigInt::from(repetitions);
        // 只累计当前值与前面观测的差，组内所有差值都为零。
        let group_difference = (&value * BigInt::from(seen) - &value_sum) * &multiplicity;
        difference_sum = bounded(difference_sum + group_difference)?;
        value_sum = bounded(value_sum + value * multiplicity)?;
        seen += repetitions;
    }
    Ok((value_sum, difference_sum))
}

fn tail_adjusted_mean(
    mut values: Vec<(BigRational, u64)>,
    count: u64,
    share: BigRational,
    winsorized: bool,
) -> Result<BigRational, StatisticsError> {
    let trim = u64::try_from((share * BigInt::from(count)).to_integer())
        .map_err(|_| StatisticsError::FrequencyOverflow)?;
    let limit = count - trim;
    values.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
    let replacement_sum = if winsorized {
        let lower = observation_at(&values, trim);
        let upper = observation_at(&values, limit - 1);
        (lower + upper) * BigInt::from(trim)
    } else {
        BigRational::default()
    };
    let mut rank = 0;
    for (_, repetitions) in &mut values {
        let next = rank + *repetitions;
        *repetitions = next.min(limit).saturating_sub(rank.max(trim));
        rank = next;
    }
    let denominator = if winsorized { count } else { limit - trim };
    bounded((observation_sum(&values)? + replacement_sum) / BigInt::from(denominator))
}

fn median_absolute_deviation(
    mut values: Vec<(BigRational, u64)>,
    count: u64,
) -> Result<BigRational, StatisticsError> {
    values.sort_by(|(left, _), (right, _)| left.cmp(right));
    let center = sorted_median(&values, count)?;
    replace_with_absolute_deviations(&mut values, &center)?;
    values.sort_by(|(left, _), (right, _)| left.cmp(right));
    sorted_median(&values, count)
}

fn mean_absolute_deviation(
    mut values: Vec<(BigRational, u64)>,
    count: u64,
) -> Result<BigRational, StatisticsError> {
    let divisor = BigInt::from(count);
    let center = bounded(observation_sum(&values)? / &divisor)?;
    replace_with_absolute_deviations(&mut values, &center)?;
    bounded(observation_sum(&values)? / divisor)
}

fn replace_with_absolute_deviations(
    values: &mut [(BigRational, u64)],
    center: &BigRational,
) -> Result<(), StatisticsError> {
    for (value, _) in values {
        let deviation = std::mem::take(value) - center;
        *value = bounded(if deviation.numer().sign() == Sign::Minus {
            -deviation
        } else {
            deviation
        })?;
    }
    Ok(())
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

enum BinomialEvent {
    Exactly,
    AtMost,
    AtLeast,
}

fn binomial_probability(input: &str, event: BinomialEvent) -> Result<String, StatisticsError> {
    let mut terms = split_top_level_commas(input).map(str::trim);
    let (Some(trials), Some(successes), Some(probability), None) =
        (terms.next(), terms.next(), terms.next(), terms.next())
    else {
        return Err(StatisticsError::BinomialArguments);
    };
    let trials = binomial_trial_count(trials, StatisticsError::BinomialArguments)?;
    let successes = successes
        .parse::<u32>()
        .ok()
        .filter(|&successes| successes <= trials)
        .ok_or(StatisticsError::BinomialArguments)?;
    let probability = parse_probability(probability)?;
    let one = BigRational::from_integer(BigInt::from(1u8));
    // 至少成功 k 次等价于至多失败 n-k 次。
    let (successes, probability) = if matches!(event, BinomialEvent::AtLeast) {
        (trials - successes, &one - probability)
    } else {
        (successes, probability)
    };
    let cumulative = !matches!(event, BinomialEvent::Exactly);
    if (cumulative && successes == trials) || trials == 0 {
        return Ok("1".to_owned());
    }
    if probability == one {
        return Ok(u8::from(successes == trials).to_string());
    }
    if probability.numer().sign() == Sign::NoSign {
        return Ok(u8::from(cumulative || successes == 0).to_string());
    }
    let (denominator, mut masses) = binomial_masses(trials, &probability)?;
    let successes = successes as usize;
    let numerator = if cumulative {
        masses.take(successes + 1).sum()
    } else {
        masses.nth(successes).expect("成功次数不超过试验次数")
    };
    Ok(BigRational::new(numerator, denominator).to_string())
}

fn binomial_trial_count(input: &str, invalid: StatisticsError) -> Result<u32, StatisticsError> {
    input
        .parse::<u32>()
        .ok()
        .filter(|&trials| trials <= MAX_BINOMIAL_TRIALS)
        .ok_or(invalid)
}

fn binomial_masses(
    trials: u32,
    probability: &BigRational,
) -> Result<(BigInt, impl Iterator<Item = BigInt> + '_), StatisticsError> {
    let failures = probability.denom() - probability.numer();
    let certain = failures.sign() == Sign::NoSign;
    let (mut mass, denominator) = if certain {
        (BigInt::default(), BigInt::from(1u8))
    } else {
        let one = BigRational::from_integer(BigInt::from(1u8));
        let no_success = rational_power(
            one - probability,
            BigRational::from_integer(BigInt::from(trials)),
        )
        .map_err(StatisticsError::Evaluation)?;
        (no_success.numer().clone(), no_success.denom().clone())
    };
    // 各项概率共用单次概率分母的 n 次幂，可对分子做整数递推。
    let numerators = (0..=trials).map(move |count| {
        if certain {
            return BigInt::from(u8::from(count == trials));
        }
        let here = mass.clone();
        if count < trials {
            let numerator = &mass * probability.numer() * BigInt::from(trials - count);
            let divisor = &failures * BigInt::from(count + 1);
            mass = numerator / divisor;
        }
        here
    });
    Ok((denominator, numerators))
}

fn binomial_csv(input: &str) -> Result<String, StatisticsError> {
    let mut terms = split_top_level_commas(input).map(str::trim);
    let (Some(trials), Some(probability), None) = (terms.next(), terms.next(), terms.next()) else {
        return Err(StatisticsError::BinomialTableArguments);
    };
    let trials = binomial_trial_count(trials, StatisticsError::BinomialTableArguments)?;
    let probability = parse_probability(probability)?;
    let (denominator, masses) = binomial_masses(trials, &probability)?;
    let mut cumulative = BigInt::default();
    let mut output = String::from("成功次数,恰好概率,至多概率,至少概率");
    let format_probability =
        |numerator| BigRational::new(numerator, denominator.clone()).to_string();
    for (count, mass) in masses.enumerate() {
        let at_least = format_probability(&denominator - &cumulative);
        cumulative += &mass;
        let row = [
            count.to_string(),
            format_probability(mass),
            format_probability(cumulative.clone()),
            at_least,
        ];
        append_bounded_csv_row(
            &mut output,
            row.iter().map(String::as_str),
            StatisticsError::ProbabilityTableTooLarge,
        )?;
    }
    Ok(output)
}

fn geometric_probability(input: &str, cumulative: bool) -> Result<String, StatisticsError> {
    let mut terms = split_top_level_commas(input).map(str::trim);
    let (Some(trials), Some(probability), None) = (terms.next(), terms.next(), terms.next()) else {
        return Err(StatisticsError::GeometricArguments);
    };
    let trials = trials
        .parse::<u32>()
        .ok()
        .filter(|trials| (1..=MAX_GEOMETRIC_TRIALS).contains(trials))
        .ok_or(StatisticsError::GeometricArguments)?;
    let probability = parse_probability(probability)?;
    if probability.numer().sign() == Sign::NoSign {
        return Err(StatisticsError::GeometricArguments);
    }

    let one = BigRational::from_integer(BigInt::from(1u8));
    let failures = if cumulative { trials } else { trials - 1 };
    let all_fail = rational_power(
        &one - &probability,
        BigRational::from_integer(BigInt::from(failures)),
    )
    .map_err(StatisticsError::Evaluation)?;
    let result = if cumulative {
        one - all_fail
    } else {
        all_fail * probability
    };
    Ok(bounded(result)?.to_string())
}

fn hypergeometric_probability(input: &str) -> Result<String, StatisticsError> {
    let values = input
        .split(',')
        .take(5)
        .map(|term| {
            term.trim()
                .parse::<u32>()
                .map_err(|_| StatisticsError::HypergeometricArguments)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let &[total, marked, draws, hits] = values.as_slice() else {
        return Err(StatisticsError::HypergeometricArguments);
    };
    if total > MAX_COMBINATORIAL_ARGUMENT || marked > total || draws > total {
        return Err(StatisticsError::HypergeometricArguments);
    }
    if hits > marked || hits > draws || draws - hits > total - marked {
        return Ok("0".to_owned());
    }
    let numerator =
        combination_count(marked, hits) * combination_count(total - marked, draws - hits);
    let probability = BigRational::new(numerator, combination_count(total, draws));
    Ok(bounded(probability)?.to_string())
}

fn contingency_table(input: &str, show_expected: bool) -> Result<String, StatisticsError> {
    let table = parse_matrix(input).map_err(StatisticsError::ContingencyInput)?;
    let rows = table.len();
    let columns = table[0].len();
    if rows < 2
        || columns < 2
        || table
            .iter()
            .flatten()
            .any(|count| !count.is_integer() || count.numer().sign() == Sign::Minus)
    {
        return Err(StatisticsError::InvalidContingencyTable);
    }

    let row_totals: Vec<BigInt> = table
        .iter()
        .map(|row| row.iter().map(BigRational::numer).sum())
        .collect();
    let column_totals: Vec<BigInt> = (0..columns)
        .map(|column| table.iter().map(|row| row[column].numer()).sum())
        .collect();
    if row_totals
        .iter()
        .chain(&column_totals)
        .any(|total| total.sign() == Sign::NoSign)
    {
        return Err(StatisticsError::ZeroContingencyMargin);
    }

    let total: BigInt = row_totals.iter().sum();
    let expected = row_totals
        .iter()
        .map(|row| {
            column_totals
                .iter()
                .map(|column| bounded(BigRational::new(row * column, total.clone())))
                .collect::<Result<Vec<_>, _>>()
        })
        .collect::<Result<Vec<_>, _>>()?;
    if show_expected {
        return Ok(format_matrix(expected));
    }

    let chi_square = table
        .iter()
        .flatten()
        .zip(expected.iter().flatten())
        .try_fold(BigRational::default(), |sum, (observed, expected)| {
            let difference = observed - expected;
            bounded(sum + &difference * &difference / expected)
        })?;
    let degrees_of_freedom = (rows - 1) * (columns - 1);
    Ok(format!("χ² {chi_square}\n自由度 {degrees_of_freedom}"))
}

pub(super) fn parse_probability(input: &str) -> Result<BigRational, StatisticsError> {
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

enum QuantileReport {
    Single(BigRational),
    Quartiles,
    BoxSummary,
}

fn report_quantiles(
    mut values: Vec<(BigRational, u64)>,
    count: u64,
    report: QuantileReport,
) -> Result<String, StatisticsError> {
    values.sort_by(|(left, _), (right, _)| left.cmp(right));
    let quantile = |probability: &BigRational| interpolate_quantile(&values, count, probability);
    if let QuantileReport::Single(probability) = &report {
        return quantile(probability).map(|value| value.to_string());
    }
    let (lower, upper) = quartile_bounds(&values, count)?;
    let middle = quantile(&BigRational::new(1.into(), 2.into()))?;
    let quartiles = format!("Q1 {lower} · 中位数 {middle} · Q3 {upper}");
    if matches!(report, QuantileReport::BoxSummary) {
        Ok(box_details(&values, &lower, &upper)?.report(&quartiles))
    } else {
        let spread = bounded(&upper - &lower)?;
        Ok(format!("{quartiles}\n四分位距 {spread}"))
    }
}

pub(super) fn unweighted_box_fences(
    values: Vec<BigRational>,
) -> Result<(BigRational, BigRational), StatisticsError> {
    let mut observations = unweighted_observations(values)?;
    observations.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
    let (lower, upper) = quartile_bounds(&observations, observations.len() as u64)?;
    inner_fences(&lower, &upper)
}

fn quartile_bounds(
    values: &[(BigRational, u64)],
    count: u64,
) -> Result<(BigRational, BigRational), StatisticsError> {
    let lower = interpolate_quantile(values, count, &BigRational::new(1.into(), 4.into()))?;
    let upper = interpolate_quantile(values, count, &BigRational::new(3.into(), 4.into()))?;
    Ok((lower, upper))
}

fn inner_fences(
    lower: &BigRational,
    upper: &BigRational,
) -> Result<(BigRational, BigRational), StatisticsError> {
    let spread = bounded(upper - lower)?;
    let offset = bounded(spread * BigRational::new(3.into(), 2.into()))?;
    Ok((bounded(lower - &offset)?, bounded(upper + offset)?))
}

pub(super) fn box_plot_arguments(input: &str) -> Option<&str> {
    let (operation, values) = operation_and_arguments(input.trim());
    (operation == "箱线图" || operation.eq_ignore_ascii_case("box-plot")).then_some(values.trim())
}

pub(super) fn calculate_box_plot(input: &str) -> Result<(String, BoxPlot), StatisticsError> {
    let input = normalize(input);
    let (values, count) = read_observations(input.trim())?;
    prepare_box_plot(values, count)
}

pub(super) fn report_box_unweighted(values: Vec<BigRational>) -> Result<String, StatisticsError> {
    let values = unweighted_observations(values)?;
    let count = values.len() as u64;
    report_quantiles(values, count, QuantileReport::BoxSummary)
}

pub(super) fn plot_box_unweighted(
    values: Vec<BigRational>,
) -> Result<(String, BoxPlot), StatisticsError> {
    let values = unweighted_observations(values)?;
    let count = values.len() as u64;
    prepare_box_plot(values, count)
}

fn prepare_box_plot(
    mut values: Vec<(BigRational, u64)>,
    count: u64,
) -> Result<(String, BoxPlot), StatisticsError> {
    values.sort_by(|(left, _), (right, _)| left.cmp(right));
    let (lower, upper) = quartile_bounds(&values, count)?;
    let middle = interpolate_quantile(&values, count, &BigRational::new(1.into(), 2.into()))?;
    let details = box_details(&values, &lower, &upper)?;
    let min = &values.first().expect("至少含一个观测值").0;
    let max = &values.last().expect("至少含一个观测值").0;
    let plot = BoxPlot::new(
        [min, max],
        [&lower, &middle, &upper],
        details.whiskers,
        &details.outliers,
    );
    let quartiles = format!("Q1 {lower} · 中位数 {middle} · Q3 {upper}");
    Ok((details.report(&quartiles), plot))
}

struct BoxDetails<'a> {
    fences: [BigRational; 2],
    whiskers: [&'a BigRational; 2],
    outliers: BTreeMap<&'a BigRational, u64>,
}

fn box_details<'a>(
    values: &'a [(BigRational, u64)],
    lower: &BigRational,
    upper: &BigRational,
) -> Result<BoxDetails<'a>, StatisticsError> {
    let (lower_fence, upper_fence) = inner_fences(lower, upper)?;
    let first_inside = values.partition_point(|(value, _)| value < &lower_fence);
    let after_inside = values.partition_point(|(value, _)| value <= &upper_fence);
    let lower_whisker = &values[first_inside].0;
    let upper_whisker = &values[after_inside - 1].0;
    let mut outliers = BTreeMap::<_, u64>::new();
    for (value, repetitions) in values[..first_inside].iter().chain(&values[after_inside..]) {
        *outliers.entry(value).or_default() += repetitions;
    }
    Ok(BoxDetails {
        fences: [lower_fence, upper_fence],
        whiskers: [lower_whisker, upper_whisker],
        outliers,
    })
}

impl BoxDetails<'_> {
    fn report(&self, quartiles: &str) -> String {
        let [lower_fence, upper_fence] = &self.fences;
        let [lower_whisker, upper_whisker] = self.whiskers;
        let outliers = if self.outliers.is_empty() {
            "无".to_owned()
        } else {
            self.outliers
                .iter()
                .map(|(value, repetitions)| format!("{value}（{repetitions} 次）"))
                .collect::<Vec<_>>()
                .join("、")
        };
        format!("{quartiles}\n内围栏 {lower_fence} … {upper_fence}\n须端 {lower_whisker} … {upper_whisker}\n离群值 {outliers}")
    }
}

fn interpolate_quantile(
    values: &[(BigRational, u64)],
    count: u64,
    probability: &BigRational,
) -> Result<BigRational, StatisticsError> {
    interpolate_ordered_quantile(count, probability, |rank| observation_at(values, rank))
}

pub(super) fn interpolate_ordered_quantile<'a>(
    count: u64,
    probability: &BigRational,
    at_rank: impl Fn(u64) -> &'a BigRational,
) -> Result<BigRational, StatisticsError> {
    let position = probability * BigInt::from(count - 1);
    let rank =
        u64::try_from(position.to_integer()).expect("quantile rank within observation count");
    let lower = at_rank(rank);
    if position.is_integer() {
        return Ok(lower.clone());
    }
    let upper = at_rank(rank + 1);
    let fraction = position - BigInt::from(rank);
    bounded(lower + (upper - lower) * fraction)
}

type WeightedObservations = Vec<(BigRational, u64)>;

fn read_observations(input: &str) -> Result<(WeightedObservations, u64), StatisticsError> {
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
    Ok((values, count))
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
    fn computes_exact_sample_and_population_variance_for_each_complete_window() {
        for (query, expected) in [
            ("移动样本方差 2 1/2,1,2,4", "1/8\n1/2\n2"),
            ("移动总体方差 2 1/2,1,2,4", "1/16\n1/4\n1"),
            ("moving-population-variance 1 -3/2,5", "0\n0"),
            (
                "moving-sample-variance 2 9007199254740992,9007199254740993,9007199254740994",
                "1/2\n1/2",
            ),
        ] {
            assert_eq!(
                calculate_statistics(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert_eq!(
            calculate_statistics("移动样本方差 1 5,6"),
            Err(StatisticsError::SampleWindowWidth)
        );
        assert_eq!(
            calculate_statistics("移动总体方差 3 5,6"),
            Err(StatisticsError::InvalidWindowWidth)
        );
    }

    #[test]
    fn computes_exact_sample_and_population_standard_deviation_for_each_complete_window() {
        for (query, expected) in [
            ("移动样本标准差 2 1/2,1,2,4", "√(1/8)\n√(1/2)\n√2"),
            ("移动总体标准差 2 1/2,1,2,4", "1/4\n1/2\n1"),
            ("moving-population-standard-deviation 1 -3/2,5", "0\n0"),
            (
                "moving-sample-standard-deviation 2 9007199254740992,9007199254740993",
                "√(1/2)",
            ),
        ] {
            assert_eq!(
                calculate_statistics(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert_eq!(
            calculate_statistics("移动样本标准差 1 5,6"),
            Err(StatisticsError::SampleWindowWidth)
        );
    }

    #[test]
    fn computes_pearson_chi_square_and_expected_counts_exactly() {
        for (input, expected) in [
            ("独立性卡方 1,2; 3,4", "χ² 5/63\n自由度 1"),
            ("期望频数 1,2; 3,4", "6/5, 9/5\n14/5, 21/5"),
            ("chi-square 0,1; 1,0", "χ² 2\n自由度 1"),
            ("独立性卡方 2,3,4; 4,6,8", "χ² 0\n自由度 2"),
        ] {
            assert_eq!(
                calculate_statistics(input).as_deref(),
                Ok(expected),
                "{input}"
            );
        }
        for input in [
            "独立性卡方 1,2",
            "独立性卡方 -1,2; 3,4",
            "期望频数 1/3,2; 3,4",
        ] {
            assert_eq!(
                calculate_statistics(input),
                Err(StatisticsError::InvalidContingencyTable),
            );
        }
        for input in ["独立性卡方 0,0; 1,1", "期望频数 0,1; 0,4"] {
            assert_eq!(
                calculate_statistics(input),
                Err(StatisticsError::ZeroContingencyMargin),
            );
        }
        assert_eq!(
            calculate_statistics("期望频数 1,2; 3,4,5"),
            Err(StatisticsError::ContingencyInput(MatrixError::UnevenRows)),
        );
    }

    #[test]
    fn computes_exact_first_success_and_cumulative_probabilities() {
        for (input, expected) in [
            ("几何概率 3,25%", "9/64"),
            ("几何累计 3,1/4", "37/64"),
            ("GEOMETRIC 1,1", "1"),
            ("几何概率 2,1", "0"),
            ("GEOMETRIC-CDF 1024,1", "1"),
            ("几何累计 1,1/3", "1/3"),
        ] {
            assert_eq!(
                calculate_statistics(input).as_deref(),
                Ok(expected),
                "{input}"
            );
        }
        for input in [
            "几何概率 0,25%",
            "几何概率 1025,1",
            "几何累计 3,0",
            "几何概率 2,1,1",
        ] {
            assert_eq!(
                calculate_statistics(input),
                Err(StatisticsError::GeometricArguments),
                "{input}",
            );
        }
        assert_eq!(
            calculate_statistics("几何概率 1,2"),
            Err(StatisticsError::InvalidProbability),
        );
    }

    #[test]
    fn lists_exact_binomial_masses_and_inclusive_tails() {
        let header = "成功次数,恰好概率,至多概率,至少概率";
        for (query, rows) in [
            (
                "二项概率表 3,1/3",
                "0,8/27,8/27,1\r\n1,4/9,20/27,19/27\r\n2,2/9,26/27,7/27\r\n3,1/27,1,1/27",
            ),
            ("二项概率表 1,100%", "0,0,0,1\r\n1,1,1,1"),
            ("binomial-table 2,0", "0,1,1,1\r\n1,0,1,0\r\n2,0,1,0"),
            ("二项概率表 0,100%", "0,1,1,1"),
        ] {
            let expected = format!("{header}\r\n{rows}");
            assert_eq!(calculate_statistics(query), Ok(expected.clone()));
            assert_eq!(statistics_csv_report(query).unwrap(), Ok(expected));
        }
        assert_eq!(
            calculate_statistics("二项概率表 129,1/3"),
            Err(StatisticsError::BinomialTableArguments)
        );
    }

    #[test]
    fn computes_exact_binomial_mass_and_both_tails_including_endpoints() {
        for (query, expected) in [
            ("二项概率 ４，２，５０％", "3/8"),
            ("二项累计 4,2,1/2", "11/16"),
            ("binomial 3,1,1/3", "4/9"),
            ("binomial-cdf 3,1,1/3", "20/27"),
            ("二项至少 4,3,25%", "13/256"),
            ("binomial-at-least 3,2,1/3", "7/27"),
            ("二项至少 0,0,1/3", "1"),
            ("二项至少 6,0,0", "1"),
            ("二项至少 6,1,0", "0"),
            ("二项至少 6,6,1", "1"),
            ("二项至少 6,6,1/2", "1/64"),
            ("二项概率 0,0,1/3", "1"),
            ("二项概率 6,2,0", "0"),
            ("二项累计 6,2,0", "1"),
            ("二项累计 6,2,1", "0"),
            ("二项概率 6,6,1", "1"),
            ("二项累计 128,128,1/3", "1"),
        ] {
            assert_eq!(
                calculate_statistics(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert_eq!(
            calculate_statistics("二项概率 6,7,50%"),
            Err(StatisticsError::BinomialArguments)
        );
        assert_eq!(
            calculate_statistics("二项概率 129,0,1"),
            Err(StatisticsError::BinomialArguments)
        );
        assert_eq!(
            calculate_statistics("二项累计 0,0,101%"),
            Err(StatisticsError::InvalidProbability)
        );
    }

    #[test]
    fn calculates_gini_mean_differences_between_distinct_observations() {
        for (query, expected) in [
            ("基尼平均差 1,2,5", "8/3"),
            ("GINI-MEAN-DIFFERENCE -1/2,0,1/2", "2/3"),
            ("基尼平均差 频数 0@2,3@2", "2"),
            (
                "基尼平均差 频数 0@1000000000,1@1000000000",
                "1000000000/1999999999",
            ),
            ("基尼平均差 0,0,0", "0"),
        ] {
            assert_eq!(
                calculate_statistics(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert_eq!(
            calculate_statistics("基尼平均差 1/2"),
            Err(StatisticsError::GiniMeanDifferenceCount)
        );
    }

    #[test]
    fn calculates_exact_gini_coefficients_for_nonnegative_observations() {
        for (query, expected) in [
            ("基尼系数 0,0,4,4", "1/2"),
            ("基尼系数 2,1,0", "4/9"),
            ("基尼系数 2/3,2", "1/4"),
            ("GINI 频数 1@2,4@2", "3/10"),
            (
                "gini-coefficient frequency 0@1000000000,1@1000000000",
                "1/2",
            ),
            ("基尼系数 1,1,1", "0"),
            ("基尼系数 2**80", "0"),
        ] {
            assert_eq!(
                calculate_statistics(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        for query in ["基尼系数 -1,1,2", "基尼系数 0,0"] {
            assert_eq!(
                calculate_statistics(query),
                Err(StatisticsError::InvalidGiniValues),
                "{query}"
            );
        }
    }

    #[test]
    fn replaces_tail_values_for_winsorized_means_using_exact_frequencies() {
        for (query, expected) in [
            ("缩尾平均 20% 100,0,30,3,2", "67/5"),
            ("缩尾平均 20% -100,-3,-2,-1,7", "-2"),
            ("缩尾平均 1/5 频数 0@2,1@3,10@4,100@1", "11/2"),
            ("WINSORIZED-MEAN 20% 频数 0@1000000000,1@1000000000", "1/2"),
            ("缩尾平均 49% 1/3", "1/3"),
        ] {
            assert_eq!(
                calculate_statistics(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert_eq!(
            calculate_statistics("缩尾平均 50% 1,2"),
            Err(StatisticsError::InvalidTrimShare)
        );
    }

    #[test]
    fn trims_both_tails_by_exact_observation_count_without_expanding_frequencies() {
        for (query, expected) in [
            ("截尾平均 25% 100,1,3,2", "5/2"),
            ("截尾平均 1/4 频数 10@2,0@4,2@2", "1"),
            ("trimmed-mean 10% 1/2,1,2", "7/6"),
            ("截尾平均 0 频数 0@1000000000,3@1000000000", "3/2"),
            ("截尾平均 49% 1,1,2", "1"),
        ] {
            assert_eq!(
                calculate_statistics(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert_eq!(
            calculate_statistics("截尾平均 50% 1,2"),
            Err(StatisticsError::InvalidTrimShare)
        );
    }

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
    fn computes_mean_absolute_deviation_at_the_mean_with_exact_frequencies() {
        for (query, expected) in [
            ("平均绝对偏差 0,1/2,1,10", "57/16"),
            ("平均绝对偏差 频数 0@2,3@2,100@1", "788/25"),
            ("AAD frequency -3@1000000000,2@1000000000", "5/2"),
            ("mean-absolute-deviation 0,1/2,3/2", "5/9"),
            ("average-absolute-deviation 2**80", "0"),
        ] {
            assert_eq!(
                calculate_statistics(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert_eq!(
            calculate_statistics("平均绝对偏差"),
            Err(StatisticsError::Incomplete)
        );
    }

    #[test]
    fn compares_adjacent_values_in_order_with_exact_directions() {
        assert_eq!(
            calculate_statistics("差分 3/2,1\n5/2").as_deref(),
            Ok("-1/2\n3/2")
        );
        assert_eq!(
            calculate_statistics("relative-change 20,15,45").as_deref(),
            Ok("-1/4\n2")
        );
        assert_eq!(
            calculate_statistics("变化率 -2,-1,0").as_deref(),
            Ok("-1/2\n-1")
        );
        assert_eq!(
            calculate_statistics("差分 1"),
            Err(StatisticsError::AdjacentValueCount)
        );
        assert_eq!(
            calculate_statistics("变化率 1,0,2"),
            Err(StatisticsError::ZeroPreviousValue)
        );
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
    fn reports_each_complete_window_minimum_maximum_and_range_as_exact_values() {
        for (query, expected) in [
            ("移动最小 3 1/2,100,2,3,-1", "1/2\n2\n-1"),
            ("moving-max 3 1/2,100,2,3,-1", "100\n100\n3"),
            ("移动最大 1 comb(4,2),-2,-2", "6\n-2\n-2"),
            ("moving-min 2 2/3,1,-1/2", "2/3\n-1/2"),
            ("移动极差 3 1/2,100,2,3,-1", "199/2\n98\n4"),
            ("moving-range 1 comb(4,2),-2,-2", "0\n0\n0"),
        ] {
            assert_eq!(
                calculate_statistics(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert_eq!(
            calculate_statistics("移动最大 3 1,2"),
            Err(StatisticsError::InvalidWindowWidth)
        );
    }

    #[test]
    fn interpolates_exact_quantiles_for_complete_sliding_windows() {
        for (query, expected) in [
            ("移动分位数 3,25% 4,1,3,2,5", "2\n3/2\n5/2"),
            ("moving-quantile 3,3/4 4,1,3,2,5", "7/2\n5/2\n4"),
            ("移动分位数 1,0 1/3,0.5,-2", "1/3\n1/2\n-2"),
            ("移动分位数 2,100% 1,3,2", "3\n3"),
        ] {
            assert_eq!(
                calculate_statistics(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        for (query, error) in [
            (
                "移动分位数 3 4,1,3,2,5",
                StatisticsError::WindowQuantileArguments,
            ),
            (
                "移动分位数 3,101% 1,2,3",
                StatisticsError::InvalidProbability,
            ),
            ("移动分位数 3,25% 1,2", StatisticsError::InvalidWindowWidth),
        ] {
            assert_eq!(calculate_statistics(query), Err(error), "{query}");
        }
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
            ("移动合计 2 1/2,1,2,4", "3/2\n3\n6"),
            ("moving-sum 1 comb(4,2),-3/2,0", "6\n-3/2\n0"),
            (
                "移动合计 2 9007199254740992,9007199254740993",
                "18014398509481985",
            ),
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
    fn calculates_exact_probabilities_for_sampling_without_replacement() {
        for (query, expected) in [
            ("超几何概率 10,4,3,2", "3/10"),
            ("hypergeometric 10,4,3,0", "1/6"),
            ("超几何概率 10,10,3,3", "1"),
            ("超几何概率 10,4,3,4", "0"),
            ("超几何概率 10,8,4,0", "0"),
            ("超几何概率 0,0,0,0", "1"),
        ] {
            assert_eq!(
                calculate_statistics(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        for query in [
            "超几何概率 10,11,1,0",
            "超几何概率 10,1,11,0",
            "超几何概率 4097,1,1,0",
            "超几何概率 10,1,1,-1",
        ] {
            assert_eq!(
                calculate_statistics(query),
                Err(StatisticsError::HypergeometricArguments)
            );
        }
    }

    #[test]
    fn computes_exact_spearman_correlation_with_average_ranks_for_ties() {
        for (input, result) in [
            ("秩相关 0,1; 1,2; 2,100", "1"),
            ("秩相关 0,0; 1,1; 2,1", "√(3/4)"),
            ("spearman 1\t3\r\n1\t2\r\n2\t1", "-√(3/4)"),
        ] {
            assert_eq!(
                calculate_statistics(input).as_deref(),
                Ok(result),
                "{input}"
            );
        }
        assert_eq!(
            calculate_statistics("秩相关 1,1; 1,2"),
            Err(StatisticsError::Paired(LinearFitError::ConstantCoordinate))
        );
    }

    #[test]
    fn computes_exact_kendall_tau_b_with_ties_in_one_or_both_coordinates() {
        for (input, expected) in [
            ("肯德尔 0,0; 1,1; 2,1", "√(2/3)"),
            ("肯德尔秩相关 0,0; 0,0; 1,1; 2,1", "√(4/5)"),
            ("肯德尔 0,0; 1,1; 2,0", "0"),
            ("kendall 1\t3\r\n1\t2\r\n2\t1", "-√(2/3)"),
            ("肯德尔 12,1; 2,4; 1,7; 12,1; 2,0", "-√(2/9)"),
            ("肯德尔 9007199254740992,1/2; 9007199254740993,-1/2", "-1"),
        ] {
            assert_eq!(
                calculate_statistics(input).as_deref(),
                Ok(expected),
                "{input}"
            );
        }
        for input in ["肯德尔 1,1; 1,2", "肯德尔 1,2; 2,2", "肯德尔 1,1; 1,1"] {
            assert_eq!(
                calculate_statistics(input),
                Err(StatisticsError::Paired(LinearFitError::ConstantCoordinate)),
            );
        }
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
    fn accepts_copied_two_column_numeric_ranges_in_weighted_statistics() {
        let values = "80\t30%\r\n90\t70%";
        for (command, expected) in [
            ("加权平均", "87"),
            ("加权分位数 50%", "90"),
            ("加权方差", "21"),
            ("加权标准差", "√21"),
            (
                "加权累积分布",
                "值,累计权重,累计比例\r\n80,3/10,3/10\r\n90,1,1",
            ),
        ] {
            assert_eq!(
                calculate_statistics(&format!("{command}\n{values}")).as_deref(),
                Ok(expected)
            );
        }
        assert_eq!(
            calculate_statistics("加权平均 1\t2\t3"),
            Err(StatisticsError::InvalidWeight)
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
    fn keeps_weighted_population_variance_and_standard_deviation_exact() {
        for (query, expected) in [
            ("加权方差 0@1,4@3", "3"),
            ("加权标准差 0@25%,4@75%", "√3"),
            ("weighted-variance 0@1/4,1/2@3/4", "3/64"),
            ("加权总体方差 -4@1,-4@3,200@0", "0"),
            ("weighted-stddev 2@3/2", "0"),
        ] {
            assert_eq!(
                calculate_statistics(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert_eq!(
            calculate_statistics("加权方差 0@0,4@0"),
            Err(StatisticsError::ZeroTotalWeight)
        );
        assert_eq!(
            calculate_statistics("加权标准差 0@-1,4@3"),
            Err(StatisticsError::InvalidWeight)
        );
    }

    #[test]
    fn finds_the_first_positive_weight_value_reaching_the_exact_quantile() {
        for (query, expected) in [
            ("加权分位数 60% 3@40%,1@10%,2@50%", "2"),
            ("加权分位数 61% 3@40%,1@10%,2@50%", "3"),
            ("weighted-quantile 0 -10@0,5/3@1/4,2@3/4", "5/3"),
            ("weighted-quantile 1 2@1/4,8@0,5/3@3/4", "2"),
            ("加权分位数 1/2 -2@1/3,-2@1/3,7@1/3", "-2"),
            ("加权分位数 50% -1@2\n2@3", "2"),
        ] {
            assert_eq!(
                calculate_statistics(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        for (query, expected) in [
            ("加权分位数 120% 3@1", StatisticsError::InvalidProbability),
            ("加权分位数 50% 4@0,8@0%", StatisticsError::ZeroTotalWeight),
            ("加权分位数 50% 4@-1/2,8@1", StatisticsError::InvalidWeight),
            ("加权分位数 50% 4,8@1", StatisticsError::InvalidWeight),
        ] {
            assert_eq!(calculate_statistics(query), Err(expected), "{query}");
        }
    }

    #[test]
    fn reports_exact_standard_scores_in_original_order_with_optional_frequencies() {
        assert_eq!(
            calculate_statistics("标准分 3,1,2").as_deref(),
            Ok("1\n-1\n0")
        );
        assert_eq!(
            calculate_statistics("总体标准分 0,2").as_deref(),
            Ok("-1\n1")
        );
        assert_eq!(
            calculate_statistics("z-score 0,2").as_deref(),
            Ok("-√(1/2)\n√(1/2)")
        );
        assert_eq!(
            calculate_statistics("标准分 频数 1@2,4@1").as_deref(),
            Ok("-√(1/3)\n√(4/3)")
        );
        for query in ["标准分 1", "总体标准分 2,2,2", "标准分 频数 3@5"] {
            assert_eq!(
                calculate_statistics(query),
                Err(StatisticsError::StandardScoresConstant),
            );
        }
    }

    #[test]
    fn calculates_exact_coefficients_of_variation_for_samples_and_populations() {
        for (query, expected) in [
            ("变异系数 1,2,3", "1/2"),
            ("总体变异系数 1,2,3", "√(1/6)"),
            ("coefficient-of-variation 频数 1@1,2@1,3@1", "1/2"),
            ("样本变异系数 -1,-2,-3", "-1/2"),
            ("变异系数 -5,-5", "0"),
            ("总体变异系数 5", "0"),
        ] {
            assert_eq!(
                calculate_statistics(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert_eq!(
            calculate_statistics("变异系数 5"),
            Err(StatisticsError::VariationSampleCount)
        );
        assert_eq!(
            calculate_statistics("变异系数 -1,1"),
            Err(StatisticsError::VariationZeroMean)
        );
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
    fn computes_exact_weight_and_share_at_a_threshold_including_equal_values() {
        for (query, expected) in [
            ("加权累计权重 2 @ 1@1,2@1/2,3@2", "3/2"),
            ("加权累计比例 1+1 @ 1@1,2@1/2,3@2", "3/7"),
            ("weighted-cumulative-share 0 @ 1@1,2@1/2,3@2", "0"),
            ("weighted-cumulative-share 3 @ 1\t1\n2\t1/2\n3\t2", "1"),
        ] {
            assert_eq!(
                calculate_statistics(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert_eq!(
            calculate_statistics("加权累计比例 @ 1@2"),
            Err(StatisticsError::CumulativeThreshold)
        );
        assert_eq!(
            calculate_statistics("加权累计权重 2 @ 1@0,2@0"),
            Err(StatisticsError::ZeroTotalWeight)
        );
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
    fn reports_cumulative_count_and_frequency_at_any_exact_threshold() {
        for (query, expected) in [
            ("累计次数 2 @ 1,2,2,3", "3"),
            ("累计频率 2 @ 1,2,2,3", "3/4"),
            ("累计次数 5/4 @ 1,3/2,2", "1"),
            ("累计次数 -1 @ 1,2,3", "0"),
            ("累计频率 7 @ 1,2,3", "1"),
            ("累计频率 1/2 @ 频数 0.50@3,3/2@1", "3/4"),
            ("cumulative-count 1 + 1 @ frequency 1@2,3@2", "2"),
            (
                "累计频率 1 @ 频数 1@1,2@18446744073709551614",
                "1/18446744073709551615",
            ),
        ] {
            assert_eq!(
                calculate_statistics(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        for (query, error) in [
            ("累计频率 2 1,2", StatisticsError::CumulativeThreshold),
            ("累计次数 @ 1,2", StatisticsError::CumulativeThreshold),
            ("累计次数 2 @", StatisticsError::Incomplete),
            (
                "累计次数 2 @ 频数 1@18446744073709551615,3@1",
                StatisticsError::FrequencyOverflow,
            ),
        ] {
            assert_eq!(calculate_statistics(query), Err(error), "{query}");
        }
    }

    #[test]
    fn reports_exact_cumulative_distribution_including_weighted_ties() {
        assert_eq!(
            calculate_statistics("累计分布 3,1/2,0.5,3,5").as_deref(),
            Ok("值,累计次数,累计频率\r\n1/2,2,2/5\r\n3,4,4/5\r\n5,5,1")
        );
        assert_eq!(
            calculate_statistics("ecdf 频数 2@18446744073709551614,1@1").as_deref(),
            Ok("值,累计次数,累计频率\r\n1,1,1/18446744073709551615\r\n2,18446744073709551615,1")
        );
        assert_eq!(
            calculate_statistics("累计分布 频数 2@18446744073709551615,1@1"),
            Err(StatisticsError::FrequencyOverflow)
        );
    }

    #[test]
    fn bins_exact_values_and_weighted_frequencies_without_expanding_observations() {
        assert_eq!(
            calculate_statistics("分箱 0,1/2,2 @ -1,0,0.50,1,2").as_deref(),
            Ok("下界,上界,次数,频率\r\n-∞,0,1,1/5\r\n0,1/2,1,1/5\r\n1/2,2,2,2/5\r\n2,+∞,1,1/5")
        );
        assert_eq!(
            calculate_statistics("histogram (0+0),1 @ 频数 0@18446744073709551614,-1@1").as_deref(),
            Ok("下界,上界,次数,频率\r\n-∞,0,1,1/18446744073709551615\r\n0,1,18446744073709551614,18446744073709551614/18446744073709551615\r\n1,+∞,0,0")
        );
        for input in [
            "分箱 0 1,2",
            "分箱 @ 1",
            "分箱 0, @ 1",
            "分箱 0.50,1/2 @ 1",
            "分箱 1,0 @ 1",
        ] {
            assert_eq!(
                calculate_statistics(input),
                Err(StatisticsError::InvalidBinEdges),
                "{input}"
            );
        }
        assert_eq!(
            calculate_statistics("分箱 0 @ 频数 2@18446744073709551615,1@1"),
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
    fn summarizes_box_fences_and_counts_observations_strictly_outside_them() {
        for (query, output) in [
            (
                "箱线摘要 100,3,1,4,2",
                "Q1 2 · 中位数 3 · Q3 4\n内围栏 -1 … 7\n须端 1 … 4\n离群值 100（1 次）",
            ),
            (
                "box-summary 1,2,3,4,7",
                "Q1 2 · 中位数 3 · Q3 4\n内围栏 -1 … 7\n须端 1 … 7\n离群值 无",
            ),
            (
                "箱线摘要 频数 6@1,5@1000000000,6@1,4@3",
                "Q1 5 · 中位数 5 · Q3 5\n内围栏 5 … 5\n须端 5 … 5\n离群值 4（3 次）、6（2 次）",
            ),
            (
                "箱线摘要 -1/2",
                "Q1 -1/2 · 中位数 -1/2 · Q3 -1/2\n内围栏 -1/2 … -1/2\n须端 -1/2 … -1/2\n离群值 无",
            ),
        ] {
            assert_eq!(
                calculate_statistics(query).as_deref(),
                Ok(output),
                "{query}"
            );
        }
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
