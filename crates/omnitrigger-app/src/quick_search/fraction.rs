use std::collections::{BTreeMap, HashMap};

use num_bigint::{BigInt, BigUint, Sign};
use num_rational::BigRational;

use super::calculator::{answer_index, normalize, CalculationError, QUERY_LIMIT};
use super::conversion::split_conversion;
use super::csv_tools::append_bounded_csv_row;
use super::linear_fit::format_power_expression;
use super::operators::{split_top_level_commas, NumberComparison, Operator};
use super::radix::{ungroup_digits, Base};
use super::statistics::{
    absolute_deviation_unweighted, interpolate_ordered_quantile, mean_square_unweighted,
    median_sorted_values, sum_squares_unweighted, variance_unweighted, weighted_quantile_value,
    StatisticsError, WeightedDistribution, WeightedMean,
};

const MAX_POWER_EXPONENT: u32 = 1024;
const MAX_MODULAR_POWER_BITS: u64 = 1024;
const MAX_FACTORIAL_ARGUMENT: u32 = 1024;
pub(super) const MAX_COMBINATORIAL_ARGUMENT: u32 = 4096;
const MAX_EXACT_BITS: u64 = 8192;
pub(super) const MAX_DECIMAL_PLACES: u32 = 100;
pub(super) const MAX_REPEATING_DECIMAL_PLACES: usize = 512;
const MAX_TABLE_INPUTS: usize = 12;
const MAX_SEQUENCE_VALUES: usize = 128;
const MAX_APPROXIMATION_DENOMINATOR: u32 = 1_000_000;
const MAX_RATIO_VALUES: usize = 16;
const MAX_CONTINUED_FRACTION_TERMS: usize = 512;
const MAX_CONVERGENT_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_PRIME_FACTORIZATION: u64 = 1_000_000_000_000;
const MAX_CONGRUENCES: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FractionError {
    Incomplete,
    Invalid,
    MissingAnswer,
    ZeroDenominator,
    OutOfRange,
    TooLong,
    NonIntegerExponent,
    ModularPowerArguments,
    ModularPowerRange,
    ModularPowerInverse,
    FactorialDomain,
    FactorialRange,
    CombinatorialDomain,
    CombinatorialRange,
    DecimalPlaces,
    SignificantDigits,
    RoundPlaces,
    InterpolationInterval,
    InterpolationRange,
    ClampBounds,
    ToleranceArguments,
    QuantileArguments,
    SampleVarianceCount,
    RationalStandardDeviation,
    RationalRootMeanSquare,
    WeightedAverageArguments,
    WeightedSpreadArguments,
    WeightedQuantileArguments,
    PositiveMeanArguments,
    RationalGeometricMean,
    AlignmentArguments,
    RationalSquareRoot,
    RootDegree,
    RationalRoot,
    IntegerSquareRoot,
    IntegerAggregate,
    PrimeFactorizationDomain,
    PrimeFactorizationRange,
    QuotientRemainderArguments,
    CongruenceArguments,
    BitwiseArguments,
    RepeatingDecimalLength,
    RepeatingDecimalSyntax,
    TableInputs,
    SampleArguments,
    SampleOutputTooLarge,
    SecantArguments,
    EqualSecantPoints,
    SequenceArguments,
    MixedFractionArguments,
    DenominatorLimit,
    ContinuedFractionArguments,
    ContinuedFractionLength,
    ConvergentOutputTooLarge,
    ComparisonArguments,
    RatioArguments,
    PercentageNotation,
    ExactRange,
}

impl FractionError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入分数或有限小数，例如 42/56 或 0.125",
            Self::Invalid => "请输入分数、有限小数或用 计算 开头的算式",
            Self::MissingAnswer => CalculationError::MissingAnswer.message(),
            Self::ZeroDenominator => "分母或除数不能为零",
            Self::OutOfRange => "分子或分母超出整数范围",
            Self::TooLong => "分数算式最多支持 256 个字符",
            Self::NonIntegerExponent => "精确分数幂的指数必须是整数",
            Self::ModularPowerArguments => "三参数 pow 的参数须为整数，模数须非零",
            Self::ModularPowerRange => "三参数 pow 的指数和模数各最多 1024 个二进制位",
            Self::ModularPowerInverse => "负指数要求底数与模数互质",
            Self::FactorialDomain => "阶乘的参数必须是非负整数",
            Self::FactorialRange => "阶乘参数最多 1024，结果最多 8192 个二进制位",
            Self::CombinatorialDomain => "组合数和排列数的参数必须是非负整数",
            Self::CombinatorialRange => "组合数和排列数的参数最多 4096，结果最多 8192 个二进制位",
            Self::DecimalPlaces => "输入 0 至 100 的小数位数，例如 分数: 小数 1/3 to 8",
            Self::SignificantDigits => "输入 1 至 100 的有效数字位数，例如 分数: 有效数字 1/3 to 4",
            Self::RoundPlaces => "round 的位数须为 -100 至 100 的整数",
            Self::InterpolationInterval => "反查和映射的原区间须有两个不同端点",
            Self::InterpolationRange => "进度须为 0 至 1，反查值须位于原区间内",
            Self::ClampBounds => "clamp 的下界须小于或等于上界",
            Self::ToleranceArguments => "isclose 须填写两个数，相对容差为 0 至小于 1，绝对容差须非负",
            Self::QuantileArguments => {
                "quantile 的概率须为 0 至 1，percentile 须为 0 至 100，且至少填写一个数值"
            }
            Self::WeightedAverageArguments => "wavg 须成对填写数值和非负权重，且至少一个权重大于零",
            Self::WeightedSpreadArguments => "wpvariance 和 wpstdev 须成对填写数值和非负权重，且至少一个权重大于零",
            Self::SampleVarianceCount => "样本方差和样本标准差至少需要两个数值",
            Self::RationalStandardDeviation => "这些数值的标准差无法表示为分数，可求方差或使用普通计算查看近似标准差",
            Self::RationalRootMeanSquare => "这些数值的均方根无法表示为分数，可用 meansquare 求均方值",
            Self::WeightedQuantileArguments => {
                "wquantile 须先填 0 至 1 的概率，wpercentile 须先填 0 至 100 的百分位。随后成对填写数值和非负权重，至少一个权重大于零"
            }
            Self::PositiveMeanArguments => "gmean 和 hmean 的每个参数须为正数",
            Self::RationalGeometricMean => "这些参数的几何平均值无法表示为分数",
            Self::AlignmentArguments => "填写数值和正步长，例如 分数: 就近对齐 2.675 to 0.05",
            Self::RationalSquareRoot => "精确开方需要非负数，且结果能表示为分数",
            Self::RootDegree => "root 的次数须为 1 至 1024 的整数",
            Self::RationalRoot => "输入值须有可表示为分数的实数次方根",
            Self::IntegerSquareRoot => "isqrt 需要非负整数",
            Self::IntegerAggregate => "gcd 和 lcm 的参数须为整数",
            Self::PrimeFactorizationDomain => "请填写非零整数",
            Self::PrimeFactorizationRange => "整数的绝对值最多为 1000000000000",
            Self::QuotientRemainderArguments => "用逗号分开被除数和除数，例如 -7,3",
            Self::CongruenceArguments => "填写 2 至 16 组余数和正模数，例如 2,3; 3,5; 2,7",
            Self::BitwiseArguments => "位运算的数值和移位量须为非负整数",
            Self::RepeatingDecimalLength => "循环小数最多展开 512 个小数位",
            Self::RepeatingDecimalSyntax => "用括号填写循环节，例如 0.(142857) 或 -2.1(6)",
            Self::TableInputs => {
                "用方括号填写 1 至 12 个数值，例如 分数: 取值表 [-2,0,3/2] x**2+1/3"
            }
            Self::SampleArguments => {
                "填写 [起点,终点,点数] 和算式，例如 [-1,1,5] x**2，支持 2 至 128 个点"
            }
            Self::SampleOutputTooLarge => "精确采样结果超过 1 MiB",
            Self::SecantArguments => "填写 [起点,终点] 和算式，例如 [0,1/3] x**2",
            Self::EqualSecantPoints => "平均变化率需要两个不同的横坐标",
            Self::SequenceArguments => "填写首项、公差或公比、项数，例如 1/3,1/6,5，最多 128 项",
            Self::MixedFractionArguments => "填写整数部分和小于 1 的非负分数，例如 -2 1/3",
            Self::DenominatorLimit => {
                "填写 1 至 1000000 的分母上限，例如 分数: 限分母 0.3333 to 100"
            }
            Self::ContinuedFractionArguments => {
                "用方括号填写连分数整数项，例如 [3,7,16]。第一项之后须为正整数"
            }
            Self::ContinuedFractionLength => "连分数最多支持 512 项",
            Self::ConvergentOutputTooLarge => "收敛分数的结果超过 1 MiB",
            Self::ComparisonArguments => {
                "用比较符连接至少两个算式，例如 分数: 比较 1/3 < 0.34 <= 1/2"
            }
            Self::RatioArguments => "输入 2 至 16 个非负数，至少一个大于零，例如 0.5,3/4,1",
            Self::PercentageNotation => "请填写带百分号的数值，例如 12.5% 或 33.(3)%",
            Self::ExactRange => "幂指数绝对值最多 1024，分子和分母各最多 8192 个二进制位",
        }
    }
}

fn sign_and_digits(input: &str) -> Result<(bool, &str), FractionError> {
    let input = input.trim();
    let negative = input.starts_with('-');
    let digits = input.strip_prefix(['-', '+']).unwrap_or(input);
    if digits.is_empty() {
        return Err(FractionError::Incomplete);
    }
    Ok((negative, digits))
}

fn unsigned(digits: &str) -> Result<u128, FractionError> {
    if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(FractionError::Invalid);
    }
    digits.parse().map_err(|_| FractionError::OutOfRange)
}

fn decimal(digits: &str) -> Result<(u128, u128), FractionError> {
    let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
    if whole.is_empty() && fraction.is_empty() {
        return Err(FractionError::Incomplete);
    }
    let whole = if whole.is_empty() {
        0
    } else {
        unsigned(whole)?
    };
    let fraction = fraction.trim_end_matches('0');
    if fraction.is_empty() {
        return Ok((whole, 1));
    }
    let part = unsigned(fraction)?;
    let digits = u32::try_from(fraction.len()).map_err(|_| FractionError::OutOfRange)?;
    let denominator = 10_u128
        .checked_pow(digits)
        .ok_or(FractionError::OutOfRange)?;
    let divisor = common_divisor(part, denominator);
    let denominator = denominator / divisor;
    let numerator = whole
        .checked_mul(denominator)
        .and_then(|amount| amount.checked_add(part / divisor))
        .ok_or(FractionError::OutOfRange)?;
    Ok((numerator, denominator))
}

pub(super) fn common_divisor(mut left: u128, mut right: u128) -> u128 {
    while right != 0 {
        (left, right) = (right, left % right);
    }
    left
}

pub(super) fn simplify_fraction(input: &str) -> Result<String, FractionError> {
    simplify_fraction_using(input, None)
}

pub(super) fn simplify_fraction_with_answers(
    input: &str,
    answer_at: &dyn Fn(usize) -> Option<BigRational>,
) -> Result<String, FractionError> {
    simplify_fraction_using(input, Some(answer_at))
}

type ExactAnswerLookup<'a> = &'a dyn Fn(usize) -> Option<BigRational>;

fn simplify_fraction_using(
    input: &str,
    answer_at: Option<ExactAnswerLookup<'_>>,
) -> Result<String, FractionError> {
    let normalized = normalize(input);
    let input = normalized.trim();
    let (verb, expression) = input.split_once(char::is_whitespace).unwrap_or((input, ""));
    let calculate = |source: &str| evaluate_fraction(source, None, None, answer_at);
    if verb == "计算" || verb.eq_ignore_ascii_case("calc") {
        let expression = expression.trim();
        return calculate(expression.strip_prefix('=').unwrap_or(expression))
            .map(|value| value.to_string());
    }
    if verb == "循环小数" || verb.eq_ignore_ascii_case("repeating-decimal") {
        return repeating_decimal(&calculate(expression.trim())?);
    }
    if verb == "百分比" || verb.eq_ignore_ascii_case("percent") {
        return format_exact_percentage(calculate(expression.trim())?);
    }
    if verb == "从百分比" || verb.eq_ignore_ascii_case("from-percent") {
        let input = expression
            .trim()
            .strip_suffix('%')
            .ok_or(FractionError::PercentageNotation)?
            .trim_end();
        let value = if input.contains('(') {
            from_repeating_decimal(input)?
        } else {
            parse_fraction_literal(input)?
        };
        let fraction = value / BigInt::from(100u8);
        ensure_exact_size(&fraction)?;
        return Ok(fraction.to_string());
    }
    if verb == "从循环小数" || verb.eq_ignore_ascii_case("from-repeating-decimal") {
        return from_repeating_decimal(expression.trim()).map(|value| value.to_string());
    }
    if verb == "取值表" || verb.eq_ignore_ascii_case("table") {
        return fraction_table(expression.trim(), FractionTableFormat::Tsv);
    }
    if let Some(report) = fraction_csv_command(verb, expression.trim()) {
        return report;
    }
    if verb == "等差数列" || verb.eq_ignore_ascii_case("arithmetic-sequence") {
        return fraction_sequence(expression, false);
    }
    if verb == "等差求和" || verb.eq_ignore_ascii_case("arithmetic-sum") {
        return fraction_sequence_sum(expression, false);
    }
    if verb == "平均变化率" || verb.eq_ignore_ascii_case("secant") {
        return fraction_secant(expression.trim()).map(|line| line.slope.to_string());
    }
    if verb == "割线式" || verb.eq_ignore_ascii_case("secant-line") {
        let line = fraction_secant(expression.trim())?;
        let intercept = line.value_at_start - &line.slope * line.start;
        ensure_exact_size(&intercept)?;
        return Ok(format_power_expression(vec![intercept, line.slope]));
    }
    if verb == "等比数列" || verb.eq_ignore_ascii_case("geometric-sequence") {
        return fraction_sequence(expression, true);
    }
    if verb == "等比求和" || verb.eq_ignore_ascii_case("geometric-sum") {
        return fraction_sequence_sum(expression, true);
    }
    if verb == "连分数" || verb.eq_ignore_ascii_case("continued-fraction") {
        return continued_fraction(calculate(expression.trim())?);
    }
    if verb == "收敛分数" || verb.eq_ignore_ascii_case("convergents") {
        return fraction_convergents(calculate(expression.trim())?);
    }
    if verb == "从连分数" || verb.eq_ignore_ascii_case("from-continued-fraction") {
        return from_continued_fraction(expression.trim()).map(|value| value.to_string());
    }
    if verb == "比较" || verb.eq_ignore_ascii_case("compare") {
        return compare_fraction_expressions(expression);
    }
    if verb == "带分数" || verb.eq_ignore_ascii_case("mixed") {
        return calculate(expression).map(format_mixed_fraction);
    }
    if verb == "从带分数" || verb.eq_ignore_ascii_case("from-mixed") {
        return parse_mixed_fraction(expression.trim()).map(|value| value.to_string());
    }
    if verb == "限分母" || verb.eq_ignore_ascii_case("limit-denominator") {
        let (value, limit) =
            split_conversion(expression.trim()).ok_or(FractionError::DenominatorLimit)?;
        let limit = limit
            .parse::<u32>()
            .ok()
            .filter(|limit| (1..=MAX_APPROXIMATION_DENOMINATOR).contains(limit))
            .ok_or(FractionError::DenominatorLimit)?;
        let approximation = approximate_fraction(calculate(value)?, limit);
        ensure_exact_size(&approximation)?;
        return Ok(approximation.to_string());
    }
    if verb == "化简比例" || verb.eq_ignore_ascii_case("ratio") {
        return simplify_ratio(expression);
    }
    if verb == "质因数分解" || verb.eq_ignore_ascii_case("factorize") {
        return prime_factorization(&calculate(expression)?);
    }
    if verb == "因数统计" || verb.eq_ignore_ascii_case("divisor-stats") {
        return positive_divisor_stats(&calculate(expression)?);
    }
    if verb == "商和余数" || verb.eq_ignore_ascii_case("divmod") {
        return describe_quotient_and_remainder(expression);
    }
    if verb == "同余方程" || verb.eq_ignore_ascii_case("crt") {
        return solve_congruences(expression);
    }
    let alignment = match verb.to_ascii_lowercase().as_str() {
        "就近对齐" | "round-step" => Some(FractionAlignment::Nearest),
        "向下对齐" | "floor-step" => Some(FractionAlignment::Down),
        "向上对齐" | "ceil-step" => Some(FractionAlignment::Up),
        _ => None,
    };
    if let Some(alignment) = alignment {
        return align_fraction(expression.trim(), alignment).map(|value| value.to_string());
    }
    if verb == "小数" || verb.eq_ignore_ascii_case("decimal") {
        let (source, places) =
            split_conversion(expression.trim()).ok_or(FractionError::Incomplete)?;
        let places = places.trim();
        let places = places
            .strip_suffix("位小数")
            .or_else(|| places.strip_suffix("位"))
            .unwrap_or(places)
            .trim_end();
        if places.is_empty() {
            return Err(FractionError::Incomplete);
        }
        let places: u32 = places.parse().map_err(|_| FractionError::DecimalPlaces)?;
        if places > MAX_DECIMAL_PLACES {
            return Err(FractionError::DecimalPlaces);
        }
        return calculate(source).map(|value| rounded_decimal(value, places));
    }
    let engineering = verb == "工程计数法" || verb.eq_ignore_ascii_case("engineering-notation");
    if engineering || verb == "有效数字" || verb.eq_ignore_ascii_case("significant-digits") {
        let (source, digits) =
            split_conversion(expression.trim()).ok_or(FractionError::Incomplete)?;
        let digits = digits.trim();
        let digits = digits.strip_suffix('位').unwrap_or(digits).trim_end();
        let digits = digits
            .parse::<u32>()
            .ok()
            .filter(|digits| (1..=MAX_DECIMAL_PLACES).contains(digits))
            .ok_or(FractionError::SignificantDigits)?;
        let format: fn(BigRational, u32) -> String = if engineering {
            format_engineering_notation
        } else {
            format_significant_digits
        };
        return calculate(source).map(|value| format(value, digits));
    }
    if input.contains('(') {
        return from_repeating_decimal(input).map(|value| value.to_string());
    }
    let (negative, numerator, denominator) =
        if let Some((numerator, denominator)) = input.split_once('/') {
            let (numerator_negative, numerator) = sign_and_digits(numerator)?;
            let (denominator_negative, denominator) = sign_and_digits(denominator)?;
            (
                numerator_negative ^ denominator_negative,
                unsigned(numerator)?,
                unsigned(denominator)?,
            )
        } else {
            let (negative, digits) = sign_and_digits(input)?;
            let (numerator, denominator) = decimal(digits)?;
            (negative, numerator, denominator)
        };
    if denominator == 0 {
        return Err(FractionError::ZeroDenominator);
    }

    let divisor = common_divisor(numerator, denominator);
    let numerator = numerator / divisor;
    let denominator = denominator / divisor;
    let sign = if negative && numerator != 0 { "-" } else { "" };
    Ok(if denominator == 1 {
        format!("{sign}{numerator}")
    } else {
        format!("{sign}{numerator}/{denominator}")
    })
}

fn prime_powers(value: &BigRational) -> Result<Vec<(u64, u32)>, FractionError> {
    if !value.is_integer() || value.numer().sign() == Sign::NoSign {
        return Err(FractionError::PrimeFactorizationDomain);
    }
    let mut remaining = u64::try_from(value.numer().magnitude())
        .ok()
        .filter(|&value| value <= MAX_PRIME_FACTORIZATION)
        .ok_or(FractionError::PrimeFactorizationRange)?;
    let mut factors = Vec::new();
    for divisor in std::iter::once(2).chain((3..).step_by(2)) {
        if divisor > remaining / divisor {
            break;
        }
        let mut count = 0;
        while remaining.is_multiple_of(divisor) {
            remaining /= divisor;
            count += 1;
        }
        if count > 0 {
            factors.push((divisor, count));
        }
    }
    if remaining > 1 {
        factors.push((remaining, 1));
    }
    Ok(factors)
}

fn prime_factorization(value: &BigRational) -> Result<String, FractionError> {
    let powers = prime_powers(value)?;
    let mut terms = Vec::new();
    if value.numer().sign() == Sign::Minus {
        terms.push("-1".to_owned());
    }
    for (prime, exponent) in powers {
        terms.push(if exponent == 1 {
            prime.to_string()
        } else {
            format!("{prime}**{exponent}")
        });
    }
    if terms.is_empty() {
        terms.push("1".to_owned());
    }
    Ok(terms.join(" * "))
}

fn positive_divisor_stats(value: &BigRational) -> Result<String, FractionError> {
    let mut count = 1_u64;
    let mut sum = 1_u64;
    for (prime, exponent) in prime_powers(value)? {
        count *= u64::from(exponent + 1);
        let mut power = 1;
        let mut prime_sum = 1;
        for _ in 0..exponent {
            power *= prime;
            prime_sum += power;
        }
        sum *= prime_sum;
    }
    Ok(format!("正因数个数 {count}\n正因数之和 {sum}"))
}

fn describe_quotient_and_remainder(input: &str) -> Result<String, FractionError> {
    let mut terms = split_top_level_commas(input);
    let (Some(dividend), Some(divisor), None) = (terms.next(), terms.next(), terms.next()) else {
        return Err(FractionError::QuotientRemainderArguments);
    };
    if dividend.trim().is_empty() || divisor.trim().is_empty() {
        return Err(FractionError::QuotientRemainderArguments);
    }
    let dividend = calculate_fraction(dividend)?;
    let divisor = calculate_fraction(divisor)?;
    if divisor.numer().sign() == Sign::NoSign {
        return Err(FractionError::ZeroDenominator);
    }
    let (quotient, remainder) = quotient_and_remainder(dividend, divisor);
    ensure_exact_size(&quotient)?;
    ensure_exact_size(&remainder)?;
    Ok(format!("商 {quotient}\n余数 {remainder}"))
}

fn quotient_and_remainder(
    dividend: BigRational,
    divisor: BigRational,
) -> (BigRational, BigRational) {
    let quotient = (&dividend / &divisor).floor();
    let remainder = dividend - divisor * &quotient;
    (quotient, remainder)
}

fn parse_congruence(input: &str) -> Result<(BigInt, BigInt), FractionError> {
    let mut terms = split_top_level_commas(input);
    let (Some(residue), Some(modulus), None) = (terms.next(), terms.next(), terms.next()) else {
        return Err(FractionError::CongruenceArguments);
    };
    if residue.trim().is_empty() || modulus.trim().is_empty() {
        return Err(FractionError::CongruenceArguments);
    }
    let residue = calculate_fraction(residue)?;
    let modulus = calculate_fraction(modulus)?;
    if !residue.is_integer() || !modulus.is_integer() || modulus.numer().sign() != Sign::Plus {
        return Err(FractionError::CongruenceArguments);
    }
    Ok((residue.to_integer(), modulus.to_integer()))
}

fn solve_congruences(input: &str) -> Result<String, FractionError> {
    let groups: Vec<_> = input
        .trim()
        .split([';', '\n'])
        .take(MAX_CONGRUENCES + 1)
        .collect();
    if !(2..=MAX_CONGRUENCES).contains(&groups.len()) {
        return Err(FractionError::CongruenceArguments);
    }
    let congruences = groups
        .into_iter()
        .map(parse_congruence)
        .collect::<Result<Vec<_>, _>>()?;
    let mut residue = BigInt::default();
    let mut period = BigInt::from(1_u8);
    for (next_residue, modulus) in congruences {
        let gcd = BigInt::from(exact_gcd(
            period.magnitude().clone(),
            modulus.magnitude().clone(),
        ));
        let delta = next_residue - &residue;
        if (&delta % &gcd).sign() != Sign::NoSign {
            return Ok("无整数解".to_owned());
        }
        let reduced = modulus / &gcd;
        if reduced.bits() == 1 {
            continue;
        }
        let combined_period = &period * &reduced;
        if combined_period.bits() > MAX_EXACT_BITS {
            return Err(FractionError::ExactRange);
        }
        // 新解形如 residue + period * k，约去公因数后可用逆元求出 k。
        let inverse = (&period / &gcd)
            .modinv(&reduced)
            .expect("约去最大公约数后，余下的模数互质");
        let mut steps = ((delta / &gcd) * inverse) % &reduced;
        if steps.sign() == Sign::Minus {
            steps += &reduced;
        }
        residue += &period * steps;
        period = combined_period;
    }
    Ok(format!("x ≡ {residue} (mod {period})"))
}

fn simplify_ratio(input: &str) -> Result<String, FractionError> {
    let terms: Vec<_> = split_top_level_commas(input)
        .take(MAX_RATIO_VALUES + 1)
        .collect();
    if !(2..=MAX_RATIO_VALUES).contains(&terms.len()) {
        return Err(FractionError::RatioArguments);
    }
    let values = terms
        .into_iter()
        .map(calculate_fraction)
        .collect::<Result<Vec<_>, _>>()?;
    if values
        .iter()
        .any(|value| value.numer().sign() == Sign::Minus)
    {
        return Err(FractionError::RatioArguments);
    }
    let mut denominator = BigUint::from(1u8);
    for value in &values {
        let part = value.denom().magnitude();
        let divisor = exact_gcd(denominator.clone(), part.clone());
        denominator = (denominator / divisor) * part;
        if denominator.bits() > MAX_EXACT_BITS {
            return Err(FractionError::ExactRange);
        }
    }
    let mut scaled = Vec::with_capacity(values.len());
    let mut divisor = BigUint::default();
    for value in values {
        let numerator = value.numer().magnitude() * (&denominator / value.denom().magnitude());
        if numerator.bits() > MAX_EXACT_BITS {
            return Err(FractionError::ExactRange);
        }
        divisor = exact_gcd(divisor, numerator.clone());
        scaled.push(numerator);
    }
    if divisor.bits() == 0 {
        return Err(FractionError::RatioArguments);
    }
    Ok(scaled
        .into_iter()
        .map(|numerator| (numerator / &divisor).to_string())
        .collect::<Vec<_>>()
        .join(":"))
}

fn compare_fraction_expressions(input: &str) -> Result<String, FractionError> {
    let operand = |text: &str| {
        let text = text.trim();
        if text.is_empty() {
            return Err(FractionError::ComparisonArguments);
        }
        calculate_fraction(text)
    };
    let (left, mut comparison, mut remainder) =
        split_fraction_comparison(input).ok_or(FractionError::ComparisonArguments)?;
    let mut previous = operand(left)?;
    let mut satisfied = true;
    loop {
        let next = split_fraction_comparison(remainder);
        let right = next.map_or(remainder, |(right, _, _)| right);
        let current = operand(right)?;
        satisfied &= comparison.accepts(previous.cmp(&current));
        let Some((_, next_comparison, next_remainder)) = next else {
            break;
        };
        previous = current;
        comparison = next_comparison;
        remainder = next_remainder;
    }
    Ok(if satisfied { "成立" } else { "不成立" }.to_owned())
}

fn split_fraction_comparison(input: &str) -> Option<(&str, NumberComparison, &str)> {
    let (start, length, comparison) = input.char_indices().find_map(|(index, character)| {
        let remainder = &input[index..];
        [2, character.len_utf8()].into_iter().find_map(|length| {
            let operator = remainder.get(..length)?;
            NumberComparison::parse(operator).map(|comparison| (index, length, comparison))
        })
    })?;
    Some((&input[..start], comparison, &input[start + length..]))
}

pub(super) fn calculate_fraction(input: &str) -> Result<BigRational, FractionError> {
    calculate_fraction_with_variables(input, None, None)
}

pub(super) fn calculate_fraction_with_variables(
    input: &str,
    x: Option<&BigRational>,
    y: Option<&BigRational>,
) -> Result<BigRational, FractionError> {
    evaluate_fraction(input, x, y, None)
}

fn evaluate_fraction(
    input: &str,
    x: Option<&BigRational>,
    y: Option<&BigRational>,
    answer_at: Option<ExactAnswerLookup<'_>>,
) -> Result<BigRational, FractionError> {
    if input.chars().take(QUERY_LIMIT + 1).count() > QUERY_LIMIT {
        return Err(FractionError::TooLong);
    }
    let mut parser = FractionParser {
        remaining: input,
        x,
        y,
        answer_at,
    };
    let value = parser.sum()?;
    if !parser.remaining.trim().is_empty() {
        return Err(FractionError::Invalid);
    }
    Ok(value)
}

enum FractionTableFormat {
    Tsv,
    Csv,
}

fn fraction_table(input: &str, format: FractionTableFormat) -> Result<String, FractionError> {
    let (points, expression) = input
        .strip_prefix('[')
        .and_then(|input| input.split_once(']'))
        .ok_or(FractionError::TableInputs)?;
    let points: Vec<_> = points.split(',').take(MAX_TABLE_INPUTS + 1).collect();
    if points.len() > MAX_TABLE_INPUTS || points.iter().any(|point| point.trim().is_empty()) {
        return Err(FractionError::TableInputs);
    }
    let (header, delimiter, newline) = match format {
        FractionTableFormat::Tsv => ("x\ty", '\t', "\n"),
        FractionTableFormat::Csv => ("x,y", ',', "\r\n"),
    };
    let mut rows = vec![header.to_owned()];
    for point in points {
        let x = parse_fraction_literal(point)?;
        let y = calculate_fraction_with_variables(expression.trim(), Some(&x), None)?;
        rows.push(format!("{x}{delimiter}{y}"));
    }
    Ok(rows.join(newline))
}

fn fraction_csv_command(verb: &str, expression: &str) -> Option<Result<String, FractionError>> {
    if verb == "精确采样" || verb.eq_ignore_ascii_case("sample-csv") {
        Some(fraction_samples(expression))
    } else if verb == "取值CSV" || verb.eq_ignore_ascii_case("table-csv") {
        Some(fraction_table(expression, FractionTableFormat::Csv))
    } else {
        None
    }
}

pub(super) fn fraction_csv_report(input: &str) -> Option<Result<String, FractionError>> {
    let normalized = normalize(input);
    let input = normalized.trim();
    let (verb, expression) = input.split_once(char::is_whitespace).unwrap_or((input, ""));
    fraction_csv_command(verb, expression.trim())
}

fn fraction_samples(input: &str) -> Result<String, FractionError> {
    let (range, expression) = input
        .strip_prefix('[')
        .and_then(|input| input.split_once(']'))
        .ok_or(FractionError::SampleArguments)?;
    let range: Vec<_> = split_top_level_commas(range).take(4).collect();
    let [start, end, count] = range.as_slice() else {
        return Err(FractionError::SampleArguments);
    };
    let count = count
        .trim()
        .parse::<usize>()
        .ok()
        .filter(|count| (2..=MAX_SEQUENCE_VALUES).contains(count))
        .ok_or(FractionError::SampleArguments)?;
    let start = calculate_fraction(start)?;
    let end = calculate_fraction(end)?;
    let step = (end - &start) / BigInt::from(count - 1);
    let mut csv = String::from("x,y");
    for index in 0..count {
        let x = &start + &step * BigInt::from(index);
        ensure_exact_size(&x)?;
        let y = calculate_fraction_with_variables(expression.trim(), Some(&x), None)?;
        let cells = [x.to_string(), y.to_string()];
        append_bounded_csv_row(
            &mut csv,
            cells.iter().map(String::as_str),
            FractionError::SampleOutputTooLarge,
        )?;
    }
    Ok(csv)
}

struct FractionSecant {
    start: BigRational,
    value_at_start: BigRational,
    slope: BigRational,
}

fn fraction_secant(input: &str) -> Result<FractionSecant, FractionError> {
    let (points, expression) = input
        .strip_prefix('[')
        .and_then(|input| input.split_once(']'))
        .ok_or(FractionError::SecantArguments)?;
    let points: Vec<_> = split_top_level_commas(points).take(3).collect();
    let [start, end] = points.as_slice() else {
        return Err(FractionError::SecantArguments);
    };
    let start = calculate_fraction(start)?;
    let end = calculate_fraction(end)?;
    if start == end {
        return Err(FractionError::EqualSecantPoints);
    }
    let expression = expression.trim();
    let left = calculate_fraction_with_variables(expression, Some(&start), None)?;
    let right = calculate_fraction_with_variables(expression, Some(&end), None)?;
    let slope = (right - &left) / (end - &start);
    ensure_exact_size(&slope)?;
    Ok(FractionSecant {
        start,
        value_at_start: left,
        slope,
    })
}

fn format_mixed_fraction(value: BigRational) -> String {
    let numerator = value.numer().magnitude();
    let denominator = value.denom().magnitude();
    let whole = numerator / denominator;
    let remainder = numerator % denominator;
    if whole.bits() == 0 || remainder.bits() == 0 {
        return value.to_string();
    }
    let sign = if value.numer().sign() == Sign::Minus {
        "-"
    } else {
        ""
    };
    format!("{sign}{whole} {remainder}/{denominator}")
}

fn parse_mixed_fraction(input: &str) -> Result<BigRational, FractionError> {
    if input.chars().take(QUERY_LIMIT + 1).count() > QUERY_LIMIT {
        return Err(FractionError::TooLong);
    }
    let Some((whole, fraction)) = input.split_once(char::is_whitespace) else {
        return parse_fraction_literal(input);
    };
    let digits = whole.strip_prefix(['-', '+']).unwrap_or(whole);
    if digits.is_empty() || !digits.bytes().all(|digit| digit.is_ascii_digit()) {
        return Err(FractionError::MixedFractionArguments);
    }
    let integer =
        BigInt::parse_bytes(digits.as_bytes(), 10).ok_or(FractionError::MixedFractionArguments)?;
    let fraction = parse_fraction_literal(fraction.trim_start())?;
    if fraction.numer().sign() == Sign::Minus || fraction.numer() >= fraction.denom() {
        return Err(FractionError::MixedFractionArguments);
    }
    let value = BigRational::from_integer(integer) + fraction;
    ensure_exact_size(&value)?;
    Ok(if whole.starts_with('-') {
        -value
    } else {
        value
    })
}

fn approximate_fraction(value: BigRational, max_denominator: u32) -> BigRational {
    let limit = BigUint::from(max_denominator);
    if value.denom().magnitude() <= &limit {
        return value;
    }
    let negative = value.numer().sign() == Sign::Minus;
    let magnitude = if negative { -value } else { value };
    let mut dividend = magnitude.numer().magnitude().clone();
    let mut divisor = magnitude.denom().magnitude().clone();
    let mut preceding = (BigUint::from(0u8), BigUint::from(1u8));
    let mut closest = (BigUint::from(1u8), BigUint::from(0u8));
    let boundary = loop {
        let coefficient = &dividend / &divisor;
        let denominator = &closest.1 * &coefficient + &preceding.1;
        if denominator > limit {
            // 下一个收敛分数已超限，保留最大的仍然可用的中间系数。
            let allowed = (&limit - &preceding.1) / &closest.1;
            break BigRational::new(
                (&closest.0 * &allowed + &preceding.0).into(),
                (&closest.1 * &allowed + &preceding.1).into(),
            );
        }
        let numerator = &closest.0 * coefficient + &preceding.0;
        preceding = std::mem::replace(&mut closest, (numerator, denominator));
        let remainder = dividend % &divisor;
        dividend = divisor;
        divisor = remainder;
    };
    let latest = BigRational::new(closest.0.into(), closest.1.into());
    let distance = |candidate: &BigRational| {
        let delta = &magnitude - candidate;
        if delta.numer().sign() == Sign::Minus {
            -delta
        } else {
            delta
        }
    };
    let order = distance(&latest)
        .cmp(&distance(&boundary))
        .then_with(|| latest.denom().cmp(boundary.denom()))
        .then_with(|| {
            if negative {
                boundary.cmp(&latest)
            } else {
                latest.cmp(&boundary)
            }
        });
    let nearest = if order.is_gt() { boundary } else { latest };
    if negative {
        -nearest
    } else {
        nearest
    }
}

fn fraction_sequence(input: &str, geometric: bool) -> Result<String, FractionError> {
    let mut values = Vec::new();
    visit_fraction_sequence(input, geometric, |value| {
        values.push(value.to_string());
        Ok(())
    })?;
    Ok(values.join("\n"))
}

fn fraction_sequence_sum(input: &str, geometric: bool) -> Result<String, FractionError> {
    let mut total = BigRational::default();
    visit_fraction_sequence(input, geometric, |value| {
        total += value;
        ensure_exact_size(&total)
    })?;
    Ok(total.to_string())
}

fn visit_fraction_sequence(
    input: &str,
    geometric: bool,
    mut visit: impl FnMut(&BigRational) -> Result<(), FractionError>,
) -> Result<(), FractionError> {
    let terms: Vec<_> = split_top_level_commas(input).take(4).collect();
    let [first, change, count] = terms.as_slice() else {
        return Err(FractionError::SequenceArguments);
    };
    let count = count
        .trim()
        .parse::<usize>()
        .ok()
        .filter(|count| (1..=MAX_SEQUENCE_VALUES).contains(count))
        .ok_or(FractionError::SequenceArguments)?;
    let mut value = calculate_fraction(first)?;
    let change = calculate_fraction(change)?;
    visit(&value)?;
    for _ in 1..count {
        if geometric {
            value *= &change;
        } else {
            value += &change;
        }
        ensure_exact_size(&value)?;
        visit(&value)?;
    }
    Ok(())
}

fn continued_fraction(value: BigRational) -> Result<String, FractionError> {
    let terms = continued_fraction_terms(value)?
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    Ok(format!("[{}]", terms.join(",")))
}

fn continued_fraction_terms(mut value: BigRational) -> Result<Vec<BigInt>, FractionError> {
    let mut terms = Vec::new();
    for _ in 0..MAX_CONTINUED_FRACTION_TERMS {
        let whole = value.floor();
        value -= &whole;
        terms.push(whole.to_integer());
        if value.numer().sign() == Sign::NoSign {
            return Ok(terms);
        }
        value = value.recip();
    }
    Err(FractionError::ContinuedFractionLength)
}

fn fraction_convergents(value: BigRational) -> Result<String, FractionError> {
    let mut preceding = (BigInt::from(0), BigInt::from(1));
    let mut current = (BigInt::from(1), BigInt::from(0));
    let mut output = String::new();
    for term in continued_fraction_terms(value)? {
        let next = (
            &term * &current.0 + &preceding.0,
            &term * &current.1 + &preceding.1,
        );
        preceding = std::mem::replace(&mut current, next);
        // 简单连分数的相邻收敛分数行列式为 ±1，各分子和分母互素。
        let line = BigRational::new_raw(current.0.clone(), current.1.clone()).to_string();
        let separator = if output.is_empty() { "" } else { "\n" };
        if output.len() + separator.len() + line.len() > MAX_CONVERGENT_OUTPUT_BYTES {
            return Err(FractionError::ConvergentOutputTooLarge);
        }
        output.push_str(separator);
        output.push_str(&line);
    }
    Ok(output)
}

fn from_continued_fraction(input: &str) -> Result<BigRational, FractionError> {
    let input = input
        .strip_prefix('[')
        .and_then(|input| input.strip_suffix(']'))
        .ok_or(FractionError::ContinuedFractionArguments)?;
    let terms: Vec<_> = input
        .split(',')
        .take(MAX_CONTINUED_FRACTION_TERMS + 1)
        .collect();
    if terms.len() > MAX_CONTINUED_FRACTION_TERMS {
        return Err(FractionError::ContinuedFractionLength);
    }
    let mut numerator = BigInt::from(1);
    let mut denominator = BigInt::default();
    for (index, term) in terms.into_iter().enumerate().rev() {
        let term: BigInt = term
            .trim()
            .parse()
            .map_err(|_| FractionError::ContinuedFractionArguments)?;
        if index != 0 && term.sign() != Sign::Plus {
            return Err(FractionError::ContinuedFractionArguments);
        }
        if term.magnitude().bits() > MAX_EXACT_BITS {
            return Err(FractionError::ExactRange);
        }
        let next = term * &numerator + denominator;
        if next.magnitude().bits() > MAX_EXACT_BITS {
            return Err(FractionError::ExactRange);
        }
        denominator = std::mem::replace(&mut numerator, next);
    }
    Ok(BigRational::new(numerator, denominator))
}

pub(super) fn parse_fraction_literal(input: &str) -> Result<BigRational, FractionError> {
    parse_fraction_literal_with_limit(input, QUERY_LIMIT)
}

pub(super) fn parse_confirmed_fraction(input: &str) -> Result<BigRational, FractionError> {
    // 分子和分母各最多 8192 位，十进制写法连符号和斜杠不足 5000 字符。
    let value = parse_fraction_literal_with_limit(input, 5_000)?;
    ensure_exact_size(&value)?;
    Ok(value)
}

fn parse_fraction_literal_with_limit(
    input: &str,
    limit: usize,
) -> Result<BigRational, FractionError> {
    let input = input.trim();
    if input.chars().take(limit + 1).count() > limit {
        return Err(FractionError::TooLong);
    }
    let mut parser = FractionParser {
        remaining: input,
        x: None,
        y: None,
        answer_at: None,
    };
    let mut value = parser.signed_number()?;
    if parser.consume('/') {
        let denominator = parser.signed_number()?;
        if denominator.numer().sign() == Sign::NoSign {
            return Err(FractionError::ZeroDenominator);
        }
        value /= denominator;
        ensure_exact_size(&value)?;
    }
    if !parser.remaining.trim().is_empty() {
        return Err(FractionError::Invalid);
    }
    Ok(value)
}

pub(super) fn format_exact_percentage(value: BigRational) -> Result<String, FractionError> {
    let scaled = value * BigInt::from(100u8);
    ensure_exact_size(&scaled)?;
    repeating_decimal(&scaled).map(|percent| format!("{percent}%"))
}

pub(super) fn repeating_decimal(value: &BigRational) -> Result<String, FractionError> {
    repeating_radix(value, 10)
}

pub(super) fn repeating_radix(value: &BigRational, base: u32) -> Result<String, FractionError> {
    let numerator = value.numer().magnitude();
    let denominator = value.denom().magnitude();
    let mut text = (numerator / denominator).to_str_radix(base);
    if value.numer().sign() == Sign::Minus {
        text.insert(0, '-');
    }
    let mut remainder = numerator % denominator;
    if remainder.bits() == 0 {
        return Ok(text);
    }
    text.push('.');
    let decimal_start = text.len();
    let mut positions = HashMap::new();
    while remainder.bits() > 0 {
        if let Some(start) = positions.insert(remainder.clone(), text.len()) {
            text.insert(start, '(');
            text.push(')');
            break;
        }
        if text.len() - decimal_start >= MAX_REPEATING_DECIMAL_PLACES {
            return Err(FractionError::RepeatingDecimalLength);
        }
        let scaled = remainder * base;
        text.push_str(&(&scaled / denominator).to_str_radix(base));
        remainder = scaled % denominator;
    }
    Ok(text)
}

fn from_repeating_decimal(input: &str) -> Result<BigRational, FractionError> {
    let (negative, input) = sign_and_digits(input)?;
    let (fixed, repeat) = input
        .strip_suffix(')')
        .and_then(|digits| digits.split_once('('))
        .ok_or(FractionError::RepeatingDecimalSyntax)?;
    let (whole, prefix) = fixed
        .split_once('.')
        .ok_or(FractionError::RepeatingDecimalSyntax)?;
    if repeat.is_empty()
        || ![whole, prefix, repeat]
            .iter()
            .all(|part| part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err(FractionError::RepeatingDecimalSyntax);
    }
    if prefix.len() + repeat.len() > MAX_REPEATING_DECIMAL_PLACES {
        return Err(FractionError::RepeatingDecimalLength);
    }

    let prefix_scale = BigUint::from(10u8).pow(prefix.len() as u32);
    let period_scale = BigUint::from(10u8).pow(repeat.len() as u32) - 1u8;
    let fixed = format!("{whole}{prefix}");
    let fixed = BigUint::parse_bytes(fixed.as_bytes(), 10).unwrap_or_default();
    let repeated = BigUint::parse_bytes(repeat.as_bytes(), 10).expect("循环节含十进制数字");
    let numerator = fixed * &period_scale + repeated;
    let sign = if negative { Sign::Minus } else { Sign::Plus };
    let value = BigRational::new(
        BigInt::from_biguint(sign, numerator),
        (prefix_scale * period_scale).into(),
    );
    ensure_exact_size(&value)?;
    Ok(value)
}

fn round_magnitude_to_even(numerator: &BigUint, denominator: &BigUint) -> BigUint {
    let mut rounded = numerator / denominator;
    let twice_remainder = (numerator - &rounded * denominator) << 1usize;
    if &twice_remainder > denominator || (&twice_remainder == denominator && rounded.bit(0)) {
        rounded += 1u8;
    }
    rounded
}

pub(super) fn rounded_decimal(value: BigRational, places: u32) -> String {
    let unit = BigUint::from(10u8).pow(places);
    let numerator = value.numer().magnitude() * unit;
    let rounded = round_magnitude_to_even(&numerator, value.denom().magnitude());
    format_scaled_decimal(rounded, value.numer().sign() == Sign::Minus, places)
}

#[derive(PartialEq, Eq)]
pub(super) struct DecimalPreview {
    pub(super) digits: String,
    equal: bool,
}

impl DecimalPreview {
    pub(super) fn label(&self) -> String {
        let mark = if self.equal { "=" } else { "≈" };
        format!("{mark} {}", self.digits)
    }
}

pub(super) fn decimal_preview(value: &BigRational) -> Option<DecimalPreview> {
    if value.is_integer() || value.numer().bits().max(value.denom().bits()) > 256 {
        return None;
    }
    let fixed = rounded_decimal(value.clone(), 12);
    let fixed = fixed.trim_end_matches('0').trim_end_matches('.');
    let decimal = if fixed == "0" || fixed.len() > 28 {
        format_significant_digits(value.clone(), 12)
    } else {
        fixed.to_owned()
    };
    let equal = parse_confirmed_fraction(&decimal).is_ok_and(|parsed| parsed == *value);
    Some(DecimalPreview {
        digits: decimal,
        equal,
    })
}

pub(super) fn format_significant_digits(value: BigRational, digits: u32) -> String {
    format_significant_notation(value, digits, 1)
}

pub(super) fn format_engineering_notation(value: BigRational, digits: u32) -> String {
    format_significant_notation(value, digits, 3)
}

fn format_significant_notation(value: BigRational, digits: u32, exponent_step: i32) -> String {
    let numerator = value.numer().magnitude();
    let denominator = value.denom().magnitude();
    let places = digits - 1;
    if numerator.bits() == 0 {
        return format!("{}e0", rounded_decimal(value, places));
    }
    // 十进制位数差给出相邻两个候选指数，用整数比较确定边界。
    let difference =
        numerator.to_str_radix(10).len() as i32 - denominator.to_str_radix(10).len() as i32;
    let scale = BigUint::from(10u8).pow(difference.unsigned_abs());
    let below_estimate = if difference < 0 {
        numerator * scale < *denominator
    } else {
        *numerator < denominator * scale
    };
    let mut exponent = difference - i32::from(below_estimate);
    let shift = places as i32 - exponent;
    let scale = BigUint::from(10u8).pow(shift.unsigned_abs());
    let mut rounded = if shift < 0 {
        round_magnitude_to_even(numerator, &(denominator * scale))
    } else {
        round_magnitude_to_even(&(numerator * scale), denominator)
    };
    if rounded == BigUint::from(10u8).pow(digits) {
        rounded /= 10u8;
        exponent += 1;
    }
    let coefficient_shift = exponent.rem_euclid(exponent_step) as u32;
    exponent -= coefficient_shift as i32;
    if coefficient_shift > places {
        rounded *= BigUint::from(10u8).pow(coefficient_shift - places);
    }
    let coefficient = format_scaled_decimal(
        rounded,
        value.numer().sign() == Sign::Minus,
        places.saturating_sub(coefficient_shift),
    );
    format!("{coefficient}e{exponent}")
}

fn format_scaled_decimal(rounded: BigUint, negative: bool, places: u32) -> String {
    let sign = if negative && rounded.bits() != 0 {
        "-"
    } else {
        ""
    };
    let digits = rounded.to_string();
    let places = places as usize;
    if places == 0 {
        format!("{sign}{digits}")
    } else if digits.len() <= places {
        format!("{sign}0.{digits:0>places$}")
    } else {
        let (whole, fraction) = digits.split_at(digits.len() - places);
        format!("{sign}{whole}.{fraction}")
    }
}

fn validate_fraction_progress(progress: &BigRational) -> Result<(), FractionError> {
    if progress.numer().sign() == Sign::Minus || progress.numer() > progress.denom() {
        return Err(FractionError::InterpolationRange);
    }
    ensure_exact_size(progress)
}

fn fraction_progress(
    start: &BigRational,
    end: &BigRational,
    value: &BigRational,
) -> Result<BigRational, FractionError> {
    if start == end {
        return Err(FractionError::InterpolationInterval);
    }
    let progress = (value - start) / (end - start);
    validate_fraction_progress(&progress)?;
    Ok(progress)
}

pub(super) fn interpolate_fraction(
    start: &BigRational,
    end: &BigRational,
    progress: &BigRational,
) -> Result<BigRational, FractionError> {
    validate_fraction_progress(progress)?;
    let result = start + (end - start) * progress;
    ensure_exact_size(&result)?;
    Ok(result)
}

#[derive(Clone, Copy)]
enum FractionAlignment {
    Nearest,
    Down,
    Up,
}

fn align_fraction(input: &str, alignment: FractionAlignment) -> Result<BigRational, FractionError> {
    let (value, step) = split_conversion(input).ok_or(FractionError::AlignmentArguments)?;
    let value = calculate_fraction(value)?;
    let step = parse_alignment_step(step)?;
    align_fraction_to_step(value, &step, alignment)
}

pub(super) fn fraction_alignment_converter(
    step: &str,
    direction: &str,
) -> Result<impl Fn(&str) -> Result<String, FractionError>, FractionError> {
    let step = parse_alignment_step(step)?;
    let alignment = match normalize(direction).trim().to_ascii_lowercase().as_str() {
        "向下" | "floor" => FractionAlignment::Down,
        "向上" | "ceil" => FractionAlignment::Up,
        "就近" | "nearest" => FractionAlignment::Nearest,
        _ => return Err(FractionError::AlignmentArguments),
    };
    Ok(move |value: &str| {
        let result = align_fraction_to_step(calculate_fraction(value)?, &step, alignment)?;
        Ok(result.to_string())
    })
}

fn parse_alignment_step(step: &str) -> Result<BigRational, FractionError> {
    let step = calculate_fraction(step)?;
    if step.numer().sign() != Sign::Plus {
        return Err(FractionError::AlignmentArguments);
    }
    Ok(step)
}

fn align_fraction_to_step(
    value: BigRational,
    step: &BigRational,
    alignment: FractionAlignment,
) -> Result<BigRational, FractionError> {
    let ratio = value / step;
    ensure_exact_size(&ratio)?;
    let multiple = match alignment {
        FractionAlignment::Nearest => rational_round(ratio, 0)?,
        FractionAlignment::Down => ratio.floor(),
        FractionAlignment::Up => ratio.ceil(),
    };
    let result = multiple * step;
    ensure_exact_size(&result)?;
    Ok(result)
}

fn rational_round(value: BigRational, places: i32) -> Result<BigRational, FractionError> {
    let unit = BigUint::from(10u8).pow(places.unsigned_abs());
    let numerator = value.numer().magnitude();
    let denominator = value.denom().magnitude();
    let rounded = if places < 0 {
        round_magnitude_to_even(numerator, &(denominator * &unit)) * &unit
    } else {
        round_magnitude_to_even(&(numerator * &unit), denominator)
    };
    let integer = BigInt::from_biguint(value.numer().sign(), rounded);
    let result = if places < 0 {
        BigRational::from_integer(integer)
    } else {
        BigRational::new(integer, unit.into())
    };
    ensure_exact_size(&result)?;
    Ok(result)
}

pub(super) fn exact_abs_square_root(value: &BigRational) -> Option<BigRational> {
    let numerator = value.numer().magnitude();
    let denominator = value.denom().magnitude();
    let top = numerator.sqrt();
    let bottom = denominator.sqrt();
    (&top * &top == *numerator && &bottom * &bottom == *denominator)
        .then(|| BigRational::new(top.into(), bottom.into()))
}

pub(super) fn format_square_root(value: &BigRational) -> String {
    debug_assert!(value.numer().sign() != Sign::Minus);
    if let Some(root) = exact_abs_square_root(value) {
        root.to_string()
    } else {
        format_radical(value)
    }
}

fn format_radical(value: &BigRational) -> String {
    if value.is_integer() {
        format!("√{value}")
    } else {
        format!("√({value})")
    }
}

pub(super) fn format_square_root_sum(
    values: impl Iterator<Item = BigRational>,
) -> Result<String, FractionError> {
    let mut exact = BigRational::default();
    let mut radicals = BTreeMap::new();
    for value in values {
        debug_assert!(value.numer().sign() != Sign::Minus);
        ensure_exact_size(&value)?;
        if let Some(root) = exact_abs_square_root(&value) {
            exact += root;
            ensure_exact_size(&exact)?;
        } else {
            *radicals.entry(value).or_insert(0usize) += 1;
        }
    }
    if radicals.is_empty() {
        return Ok(exact.to_string());
    }

    let mut terms = Vec::with_capacity(radicals.len() + 1);
    if exact.numer().sign() != Sign::NoSign {
        terms.push(exact.to_string());
    }
    terms.extend(radicals.into_iter().map(|(value, count)| {
        let term = format_radical(&value);
        if count == 1 {
            term
        } else {
            format!("{count}{term}")
        }
    }));
    Ok(terms.join(" + "))
}

pub(super) fn format_exact_multiple(value: &BigRational, symbol: &str) -> String {
    if value.numer().sign() == Sign::NoSign {
        return "0".to_owned();
    }
    let numerator = value.numer().to_string();
    let numerator = if numerator == "1" { "" } else { &numerator };
    if value.is_integer() {
        format!("{numerator}{symbol}")
    } else {
        format!("{numerator}{symbol}/{}", value.denom())
    }
}

fn exact_sample_quantile(
    ordered: &[BigRational],
    probability: &BigRational,
) -> Result<BigRational, FractionError> {
    interpolate_ordered_quantile(ordered.len() as u64, probability, |rank| {
        &ordered[rank as usize]
    })
    .map_err(|_| FractionError::ExactRange)
}

pub(super) fn ensure_exact_size(value: &BigRational) -> Result<(), FractionError> {
    if value.numer().bits().max(value.denom().bits()) > MAX_EXACT_BITS {
        return Err(FractionError::ExactRange);
    }
    Ok(())
}

pub(super) fn format_signed_square_root(value: &BigRational, sign: Sign) -> String {
    let root = format_square_root(value);
    if sign == Sign::Minus {
        format!("-{root}")
    } else {
        root
    }
}

fn modular_power(
    base: BigRational,
    exponent: BigRational,
    modulus: BigRational,
) -> Result<BigRational, FractionError> {
    if !base.is_integer()
        || !exponent.is_integer()
        || !modulus.is_integer()
        || modulus.numer().sign() == Sign::NoSign
    {
        return Err(FractionError::ModularPowerArguments);
    }
    let modulus = modulus.numer();
    let exponent = exponent.numer();
    if modulus.bits().max(exponent.bits()) > MAX_MODULAR_POWER_BITS {
        return Err(FractionError::ModularPowerRange);
    }
    if modulus.bits() == 1 {
        return Ok(BigRational::default());
    }
    let base = base.numer();
    let value = if exponent.sign() == Sign::Minus {
        let inverse = base
            .modinv(modulus)
            .ok_or(FractionError::ModularPowerInverse)?;
        inverse.modpow(&-exponent, modulus)
    } else {
        base.modpow(exponent, modulus)
    };
    Ok(BigRational::from_integer(value))
}

fn nonnegative_bits(value: &BigRational) -> Result<&BigUint, FractionError> {
    if !value.is_integer() || value.numer().sign() == Sign::Minus {
        return Err(FractionError::BitwiseArguments);
    }
    ensure_exact_size(value)?;
    Ok(value.numer().magnitude())
}

fn exact_gcd(mut left: BigUint, mut right: BigUint) -> BigUint {
    while right.bits() != 0 {
        let remainder = left % &right;
        left = right;
        right = remainder;
    }
    left
}

fn exact_rational_root(value: &BigRational, degree: u32) -> Result<BigRational, FractionError> {
    if value.numer().sign() == Sign::Minus && degree.is_multiple_of(2) {
        return Err(FractionError::RationalRoot);
    }
    let exact_root = |part: &BigInt| {
        let magnitude = part.magnitude();
        let root = magnitude.nth_root(degree);
        (root.pow(degree) == *magnitude)
            .then(|| BigInt::from_biguint(part.sign(), root))
            .ok_or(FractionError::RationalRoot)
    };
    Ok(BigRational::new(
        exact_root(value.numer())?,
        exact_root(value.denom())?,
    ))
}

pub(super) fn rational_power(
    base: BigRational,
    exponent: BigRational,
) -> Result<BigRational, FractionError> {
    if !exponent.is_integer() {
        return Err(FractionError::NonIntegerExponent);
    }
    let exponent = i32::try_from(exponent.numer()).map_err(|_| FractionError::ExactRange)?;
    let magnitude = exponent.unsigned_abs();
    if magnitude > MAX_POWER_EXPONENT {
        return Err(FractionError::ExactRange);
    }
    if exponent < 0 && base.numer().sign() == Sign::NoSign {
        return Err(FractionError::ZeroDenominator);
    }
    // 幂的二进制位数至少为 1 + (底数位数 - 1) × 指数绝对值。
    let base_bits = base.numer().bits().max(base.denom().bits());
    if base_bits
        .saturating_sub(1)
        .saturating_mul(u64::from(magnitude))
        >= MAX_EXACT_BITS
    {
        return Err(FractionError::ExactRange);
    }
    let value = base.pow(exponent);
    ensure_exact_size(&value)?;
    Ok(value)
}

fn rational_factorial(value: BigRational) -> Result<BigRational, FractionError> {
    if !value.is_integer() || value.numer().sign() == Sign::Minus {
        return Err(FractionError::FactorialDomain);
    }
    let argument = u32::try_from(value.numer()).map_err(|_| FractionError::FactorialRange)?;
    if argument > MAX_FACTORIAL_ARGUMENT {
        return Err(FractionError::FactorialRange);
    }
    let integer = (2..=argument).fold(BigInt::from(1u8), |product, factor| product * factor);
    let value = BigRational::from_integer(integer);
    ensure_exact_size(&value).map_err(|_| FractionError::FactorialRange)?;
    Ok(value)
}

enum CombinatorialFunction {
    Combination,
    Permutation,
}

impl CombinatorialFunction {
    fn calculate(
        self,
        n: BigRational,
        k: Option<BigRational>,
    ) -> Result<BigRational, FractionError> {
        let argument = |value: &BigRational| {
            if !value.is_integer() || value.numer().sign() == Sign::Minus {
                return Err(FractionError::CombinatorialDomain);
            }
            u32::try_from(value.numer())
                .ok()
                .filter(|&value| value <= MAX_COMBINATORIAL_ARGUMENT)
                .ok_or(FractionError::CombinatorialRange)
        };
        let n = argument(&n)?;
        let k = k.as_ref().map(argument).transpose()?.unwrap_or(n);
        if k > n {
            return Ok(BigRational::from_integer(BigInt::from(0u8)));
        }
        let result = match self {
            Self::Combination => combination_count(n, k),
            Self::Permutation => {
                let mut result = BigInt::from(1u8);
                for factor in n - k + 1..=n {
                    result *= factor;
                    if result.bits() > MAX_EXACT_BITS {
                        return Err(FractionError::CombinatorialRange);
                    }
                }
                result
            }
        };
        let result = BigRational::from_integer(result);
        ensure_exact_size(&result).map_err(|_| FractionError::CombinatorialRange)?;
        Ok(result)
    }
}

pub(super) fn combination_count(n: u32, k: u32) -> BigInt {
    if k > n {
        return BigInt::default();
    }
    let k = k.min(n - k);
    let mut result = BigInt::from(1u8);
    // C(n-k+i, i) 在每一步都是整数。
    for index in 1..=k {
        result *= n - k + index;
        result /= index;
    }
    result
}

struct FractionParser<'a> {
    remaining: &'a str,
    x: Option<&'a BigRational>,
    y: Option<&'a BigRational>,
    answer_at: Option<ExactAnswerLookup<'a>>,
}

impl FractionParser<'_> {
    fn consume(&mut self, symbol: char) -> bool {
        let Some(remaining) = self.remaining.trim_start().strip_prefix(symbol) else {
            return false;
        };
        self.remaining = remaining;
        true
    }

    fn sum(&mut self) -> Result<BigRational, FractionError> {
        let mut value = self.product()?;
        loop {
            if self.consume('+') {
                value += self.product()?;
            } else if self.consume('-') {
                value -= self.product()?;
            } else {
                return Ok(value);
            }
            ensure_exact_size(&value)?;
        }
    }

    fn product(&mut self) -> Result<BigRational, FractionError> {
        let mut value = self.factor()?;
        loop {
            let remaining = self.remaining.trim_start();
            let Some((
                operator @ (Operator::Multiply
                | Operator::Divide
                | Operator::FloorDivide
                | Operator::Remainder),
                width,
            )) = Operator::prefix(remaining)
            else {
                return Ok(value);
            };
            self.remaining = &remaining[width..];
            let right = self.factor()?;
            if !matches!(operator, Operator::Multiply) && right.numer().sign() == Sign::NoSign {
                return Err(FractionError::ZeroDenominator);
            }
            value = match operator {
                Operator::Multiply => value * right,
                Operator::Divide => value / right,
                Operator::FloorDivide => (value / right).floor(),
                Operator::Remainder => quotient_and_remainder(value, right).1,
                _ => unreachable!("这里只处理乘除级运算符"),
            };
            ensure_exact_size(&value)?;
        }
    }

    fn factor(&mut self) -> Result<BigRational, FractionError> {
        let mut negative = false;
        loop {
            if self.consume('-') {
                negative = !negative;
            } else if !self.consume('+') {
                break;
            }
        }
        let mut value = if self.consume('(') {
            let value = self.sum()?;
            if !self.consume(')') {
                return Err(self.expected_token());
            }
            value
        } else if self
            .remaining
            .trim_start()
            .starts_with(|character: char| character.is_ascii_alphabetic())
        {
            self.named_value()?
        } else {
            self.number()?
        };
        if self.consume('!') {
            value = rational_factorial(value)?;
        }
        let remaining = self.remaining.trim_start();
        if let Some(exponent) = remaining
            .strip_prefix("**")
            .or_else(|| remaining.strip_prefix('^'))
        {
            self.remaining = exponent;
            value = rational_power(value, self.factor()?)?;
        }
        Ok(if negative { -value } else { value })
    }

    fn expected_token(&self) -> FractionError {
        if self.remaining.trim().is_empty() {
            FractionError::Incomplete
        } else {
            FractionError::Invalid
        }
    }

    fn named_value(&mut self) -> Result<BigRational, FractionError> {
        let input = self.remaining.trim_start();
        let length = input.bytes().take_while(u8::is_ascii_alphanumeric).count();
        let (name, rest) = input.split_at(length);
        if let (Some(answer_at), Some(index)) = (self.answer_at, answer_index(name)) {
            self.remaining = rest;
            return answer_at(index).ok_or(FractionError::MissingAnswer);
        }
        let variable = match name {
            "x" => self.x,
            "y" => self.y,
            _ => None,
        };
        if let Some(value) = variable {
            self.remaining = rest;
            return Ok(value.clone());
        }
        if name.eq_ignore_ascii_case("pow") {
            self.remaining = rest;
            return self.power_function();
        }
        if name.eq_ignore_ascii_case("round") {
            self.remaining = rest;
            return self.round_function();
        }
        let name = name.to_ascii_lowercase();
        if name == "root" {
            self.remaining = rest;
            return self.root_function();
        }
        if matches!(name.as_str(), "min" | "max") {
            self.remaining = rest;
            return self.extreme_value(name == "min");
        }
        if matches!(
            name.as_str(),
            "sum" | "fsum" | "prod" | "product" | "avg" | "mean" | "median" | "hmean" | "gmean"
        ) {
            self.remaining = rest;
            return self.aggregate_function(&name);
        }
        if matches!(name.as_str(), "quantile" | "percentile") {
            self.remaining = rest;
            return self.quantile_function(name == "percentile");
        }
        if name == "iqr" {
            self.remaining = rest;
            return self.interquartile_range_function();
        }
        if matches!(name.as_str(), "mad" | "aad") {
            self.remaining = rest;
            let values = self.list_function_arguments()?;
            return absolute_deviation_unweighted(values, name == "mad")
                .map_err(|_| FractionError::ExactRange);
        }
        if matches!(name.as_str(), "variance" | "pvariance" | "stdev" | "pstdev") {
            self.remaining = rest;
            return self.variance_function(&name);
        }
        if matches!(name.as_str(), "rms" | "meansquare" | "sumsq") {
            self.remaining = rest;
            let values = self.list_function_arguments()?;
            if name == "sumsq" {
                return sum_squares_unweighted(values).map_err(|_| FractionError::ExactRange);
            }
            let square = mean_square_unweighted(values).map_err(|_| FractionError::ExactRange)?;
            if name == "rms" {
                return exact_rational_root(&square, 2)
                    .map_err(|_| FractionError::RationalRootMeanSquare);
            }
            return Ok(square);
        }
        if name == "wavg" {
            self.remaining = rest;
            return self.weighted_average_function();
        }
        if matches!(name.as_str(), "wpvariance" | "wpstdev") {
            self.remaining = rest;
            return self.weighted_population_spread(name == "wpstdev");
        }
        if matches!(name.as_str(), "wquantile" | "wpercentile") {
            self.remaining = rest;
            return self.weighted_quantile_function(name == "wpercentile");
        }
        if name == "relchange" {
            self.remaining = rest;
            let [before, after]: [BigRational; 2] = self
                .fixed_function_arguments(2)?
                .try_into()
                .expect("relchange 有两个参数");
            if before.numer().sign() == Sign::NoSign {
                return Err(FractionError::ZeroDenominator);
            }
            let rate = (after - &before) / before;
            ensure_exact_size(&rate)?;
            return Ok(rate);
        }
        if name == "isclose" {
            self.remaining = rest;
            return self.is_close_function();
        }
        if name == "clamp" {
            self.remaining = rest;
            return self.clamp_function();
        }
        if matches!(name.as_str(), "lerp" | "invlerp" | "remap") {
            self.remaining = rest;
            return self.interpolation_function(&name);
        }
        if matches!(name.as_str(), "gcd" | "lcm") {
            self.remaining = rest;
            return self.integer_aggregate(name == "lcm");
        }
        if matches!(
            name.as_str(),
            "bitand" | "bitor" | "bitxor" | "shl" | "shr" | "popcount"
        ) {
            self.remaining = rest;
            return self.bitwise_function(&name);
        }
        if matches!(
            name.as_str(),
            "sqrt" | "abs" | "isqrt" | "floor" | "ceil" | "trunc"
        ) {
            self.remaining = rest;
            if !self.consume('(') {
                return Err(self.expected_token());
            }
            let value = self.sum()?;
            if !self.consume(')') {
                return Err(self.expected_token());
            }
            let negative = value.numer().sign() == Sign::Minus;
            match name.as_str() {
                "abs" => return Ok(if negative { -value } else { value }),
                "floor" => return Ok(value.floor()),
                "ceil" => return Ok(value.ceil()),
                "trunc" => return Ok(BigRational::from_integer(value.to_integer())),
                "isqrt" if !negative && value.is_integer() => {
                    return Ok(BigRational::from_integer(
                        value.numer().magnitude().sqrt().into(),
                    ));
                }
                "isqrt" => return Err(FractionError::IntegerSquareRoot),
                _ if negative => return Err(FractionError::RationalSquareRoot),
                _ => return exact_abs_square_root(&value).ok_or(FractionError::RationalSquareRoot),
            }
        }
        let function = match name.as_str() {
            "comb" => CombinatorialFunction::Combination,
            "perm" => CombinatorialFunction::Permutation,
            _ => return Err(FractionError::Invalid),
        };

        self.remaining = rest;
        if !self.consume('(') {
            return Err(self.expected_token());
        }
        let n = self.sum()?;
        let k = if self.consume(',') {
            Some(self.sum()?)
        } else if matches!(function, CombinatorialFunction::Permutation) {
            None
        } else {
            return Err(self.expected_token());
        };
        if !self.consume(')') {
            return Err(self.expected_token());
        }
        function.calculate(n, k)
    }

    fn root_function(&mut self) -> Result<BigRational, FractionError> {
        if !self.consume('(') {
            return Err(self.expected_token());
        }
        let value = self.sum()?;
        if !self.consume(',') {
            return Err(self.expected_token());
        }
        let degree = self.sum()?;
        if !self.consume(')') {
            return Err(self.expected_token());
        }
        if !degree.is_integer() {
            return Err(FractionError::RootDegree);
        }
        let degree = u32::try_from(degree.numer())
            .ok()
            .filter(|degree| (1..=MAX_POWER_EXPONENT).contains(degree))
            .ok_or(FractionError::RootDegree)?;
        exact_rational_root(&value, degree)
    }

    fn integer_aggregate(
        &mut self,
        least_common_multiple: bool,
    ) -> Result<BigRational, FractionError> {
        if !self.consume('(') {
            return Err(self.expected_token());
        }
        let mut values = Vec::new();
        if !self.consume(')') {
            loop {
                let value = self.sum()?;
                if !value.is_integer() {
                    return Err(FractionError::IntegerAggregate);
                }
                values.push(value.numer().magnitude().clone());
                if self.consume(')') {
                    break;
                }
                if !self.consume(',') {
                    return Err(self.expected_token());
                }
            }
        }
        if least_common_multiple && values.iter().any(|value| value.bits() == 0) {
            return Ok(BigRational::default());
        }
        let mut result = BigUint::from(u8::from(least_common_multiple));
        for value in values {
            result = if least_common_multiple {
                let divisor = exact_gcd(result.clone(), value.clone());
                (result / divisor) * value
            } else {
                exact_gcd(result, value)
            };
            if result.bits() > MAX_EXACT_BITS {
                return Err(FractionError::ExactRange);
            }
        }
        Ok(BigRational::from_integer(result.into()))
    }

    fn bitwise_function(&mut self, name: &str) -> Result<BigRational, FractionError> {
        if !self.consume('(') {
            return Err(self.expected_token());
        }
        let left = self.sum()?;
        let left = nonnegative_bits(&left)?;
        if name == "popcount" {
            if !self.consume(')') {
                return Err(self.expected_token());
            }
            let count: u32 = left.to_u32_digits().into_iter().map(u32::count_ones).sum();
            return Ok(BigRational::from_integer(count.into()));
        }
        if !self.consume(',') {
            return Err(self.expected_token());
        }
        let right = self.sum()?;
        let right = nonnegative_bits(&right)?;
        if !self.consume(')') {
            return Err(self.expected_token());
        }
        let result = match name {
            "bitand" => left & right,
            "bitor" => left | right,
            "bitxor" => left ^ right,
            "shl" if left.bits() == 0 => BigUint::default(),
            "shl" => {
                let shift = u64::try_from(right)
                    .ok()
                    .filter(|&shift| shift <= MAX_EXACT_BITS - left.bits())
                    .ok_or(FractionError::ExactRange)?;
                left << shift as usize
            }
            "shr" => {
                let shift = u64::try_from(right).unwrap_or(u64::MAX);
                if shift >= left.bits() {
                    BigUint::default()
                } else {
                    left >> shift as usize
                }
            }
            _ => unreachable!("位运算函数名已检查"),
        };
        Ok(BigRational::from_integer(result.into()))
    }

    fn power_function(&mut self) -> Result<BigRational, FractionError> {
        if !self.consume('(') {
            return Err(self.expected_token());
        }
        let base = self.sum()?;
        if !self.consume(',') {
            return Err(self.expected_token());
        }
        let exponent = self.sum()?;
        let modulus = self.consume(',').then(|| self.sum()).transpose()?;
        if !self.consume(')') {
            return Err(self.expected_token());
        }
        match modulus {
            Some(modulus) => modular_power(base, exponent, modulus),
            None => rational_power(base, exponent),
        }
    }

    fn extreme_value(&mut self, minimum: bool) -> Result<BigRational, FractionError> {
        if !self.consume('(') {
            return Err(self.expected_token());
        }
        let mut result = self.sum()?;
        while self.consume(',') {
            let value = self.sum()?;
            result = if minimum {
                result.min(value)
            } else {
                result.max(value)
            };
        }
        if !self.consume(')') {
            return Err(self.expected_token());
        }
        Ok(result)
    }

    fn list_function_arguments(&mut self) -> Result<Vec<BigRational>, FractionError> {
        if !self.consume('(') {
            return Err(self.expected_token());
        }
        let mut values = vec![self.sum()?];
        while self.consume(',') {
            values.push(self.sum()?);
        }
        if !self.consume(')') {
            return Err(self.expected_token());
        }
        Ok(values)
    }

    fn aggregate_function(&mut self, name: &str) -> Result<BigRational, FractionError> {
        let mut values = self.list_function_arguments()?;
        if name == "median" {
            values.sort_unstable();
            return median_sorted_values(&values);
        }
        if matches!(name, "prod" | "product" | "gmean") {
            let geometric = name == "gmean";
            if geometric {
                if values
                    .iter()
                    .any(|value| value.numer().sign() != Sign::Plus)
                {
                    return Err(FractionError::PositiveMeanArguments);
                }
                if values.windows(2).all(|pair| pair[0] == pair[1]) {
                    return Ok(values.swap_remove(0));
                }
            }
            let count = values.len() as u32;
            let mut product = BigRational::from_integer(1.into());
            for value in values {
                product *= value;
                ensure_exact_size(&product)?;
            }
            if geometric {
                return exact_rational_root(&product, count)
                    .map_err(|_| FractionError::RationalGeometricMean);
            }
            return Ok(product);
        }
        let harmonic = name == "hmean";
        let count = values.len();
        let mut total = BigRational::default();
        for value in values {
            if harmonic && value.numer().sign() != Sign::Plus {
                return Err(FractionError::PositiveMeanArguments);
            }
            total += if harmonic { value.recip() } else { value };
            ensure_exact_size(&total)?;
        }
        let result = match name {
            "hmean" => BigRational::from_integer(count.into()) / total,
            "sum" | "fsum" => total,
            _ => total / BigInt::from(count),
        };
        ensure_exact_size(&result)?;
        Ok(result)
    }

    fn quantile_function(&mut self, percentage: bool) -> Result<BigRational, FractionError> {
        let mut values = self.list_function_arguments()?;
        if values.len() < 2 {
            return Err(FractionError::QuantileArguments);
        }
        let mut probability = values.remove(0);
        if percentage {
            probability /= BigInt::from(100);
        }
        if probability.numer().sign() == Sign::Minus || probability.numer() > probability.denom() {
            return Err(FractionError::QuantileArguments);
        }
        values.sort_unstable();
        exact_sample_quantile(&values, &probability)
    }

    fn interquartile_range_function(&mut self) -> Result<BigRational, FractionError> {
        let mut values = self.list_function_arguments()?;
        values.sort_unstable();
        let quartile = |numerator: u8| {
            exact_sample_quantile(&values, &BigRational::new(numerator.into(), 4.into()))
        };
        let spread = quartile(3)? - quartile(1)?;
        ensure_exact_size(&spread)?;
        Ok(spread)
    }

    fn weighted_average_function(&mut self) -> Result<BigRational, FractionError> {
        let mut values = self.list_function_arguments()?.into_iter();
        if !values.len().is_multiple_of(2) {
            return Err(FractionError::WeightedAverageArguments);
        }
        let mut mean = WeightedMean::default();
        while let Some(value) = values.next() {
            let weight = values.next().expect("每个数值都有对应权重");
            if weight.numer().sign() == Sign::Minus {
                return Err(FractionError::WeightedAverageArguments);
            }
            mean.record((value, weight))
                .map_err(|_| FractionError::ExactRange)?;
        }
        mean.value().map_err(|error| match error {
            StatisticsError::ZeroTotalWeight => FractionError::WeightedAverageArguments,
            _ => FractionError::ExactRange,
        })
    }

    fn weighted_quantile_function(
        &mut self,
        percentage: bool,
    ) -> Result<BigRational, FractionError> {
        let mut values = self.list_function_arguments()?.into_iter();
        let mut probability = values.next().expect("函数参数列表至少有一项");
        if percentage {
            probability /= BigInt::from(100);
        }
        if probability.numer().sign() == Sign::Minus || probability.numer() > probability.denom() {
            return Err(FractionError::WeightedQuantileArguments);
        }
        let invalid = FractionError::WeightedQuantileArguments;
        let distribution = Self::weighted_distribution(values, invalid)?;
        weighted_quantile_value(&distribution, probability)
            .cloned()
            .map_err(|error| Self::weighted_statistics_error(error, invalid))
    }

    fn weighted_population_spread(&mut self, root: bool) -> Result<BigRational, FractionError> {
        let invalid = FractionError::WeightedSpreadArguments;
        let values = self.list_function_arguments()?.into_iter();
        let variance = Self::weighted_distribution(values, invalid)?
            .population_variance()
            .map_err(|error| Self::weighted_statistics_error(error, invalid))?;
        if root {
            return exact_rational_root(&variance, 2)
                .map_err(|_| FractionError::RationalStandardDeviation);
        }
        Ok(variance)
    }

    fn weighted_distribution(
        mut values: std::vec::IntoIter<BigRational>,
        invalid: FractionError,
    ) -> Result<WeightedDistribution, FractionError> {
        if values.len() < 2 || !values.len().is_multiple_of(2) {
            return Err(invalid);
        }
        let mut distribution = WeightedDistribution::default();
        while let Some(value) = values.next() {
            let weight = values.next().expect("每个数值都有对应权重");
            distribution
                .record((value, weight))
                .map_err(|error| Self::weighted_statistics_error(error, invalid))?;
        }
        Ok(distribution)
    }

    fn weighted_statistics_error(error: StatisticsError, invalid: FractionError) -> FractionError {
        match error {
            StatisticsError::InvalidWeight | StatisticsError::ZeroTotalWeight => invalid,
            _ => FractionError::ExactRange,
        }
    }

    fn variance_function(&mut self, name: &str) -> Result<BigRational, FractionError> {
        let values = self.list_function_arguments()?;
        let sample = matches!(name, "variance" | "stdev");
        let variance = variance_unweighted(values, sample)
            .map_err(|_| FractionError::ExactRange)?
            .ok_or(FractionError::SampleVarianceCount)?;
        if matches!(name, "stdev" | "pstdev") {
            return exact_rational_root(&variance, 2)
                .map_err(|_| FractionError::RationalStandardDeviation);
        }
        Ok(variance)
    }

    fn is_close_function(&mut self) -> Result<BigRational, FractionError> {
        let values = self.list_function_arguments()?;
        if !(2..=4).contains(&values.len()) {
            return Err(FractionError::ToleranceArguments);
        }
        let mut values = values.into_iter();
        let left = values.next().expect("已检查第一个数存在");
        let right = values.next().expect("已检查第二个数存在");
        let relative = values
            .next()
            .unwrap_or_else(|| BigRational::new(1.into(), 1_000_000_000.into()));
        let absolute = values.next().unwrap_or_default();
        if relative.numer().sign() == Sign::Minus
            || relative.numer() >= relative.denom()
            || absolute.numer().sign() == Sign::Minus
        {
            return Err(FractionError::ToleranceArguments);
        }
        let magnitude = |value: &BigRational| {
            if value.numer().sign() == Sign::Minus {
                -value
            } else {
                value.clone()
            }
        };
        let distance = magnitude(&(&left - &right));
        let close =
            distance <= absolute || distance <= relative * magnitude(&left).max(magnitude(&right));
        Ok(BigRational::from_integer(u8::from(close).into()))
    }

    fn clamp_function(&mut self) -> Result<BigRational, FractionError> {
        let [value, lower, upper]: [BigRational; 3] = self
            .fixed_function_arguments(3)?
            .try_into()
            .expect("clamp 有三个参数");
        if lower > upper {
            return Err(FractionError::ClampBounds);
        }
        Ok(value.clamp(lower, upper))
    }

    fn fixed_function_arguments(
        &mut self,
        count: usize,
    ) -> Result<Vec<BigRational>, FractionError> {
        if !self.consume('(') {
            return Err(self.expected_token());
        }
        let mut values = Vec::with_capacity(count);
        values.push(self.sum()?);
        for _ in 1..count {
            if !self.consume(',') {
                return Err(self.expected_token());
            }
            values.push(self.sum()?);
        }
        if !self.consume(')') {
            return Err(self.expected_token());
        }
        Ok(values)
    }

    fn interpolation_function(&mut self, name: &str) -> Result<BigRational, FractionError> {
        let count = if name == "remap" { 5 } else { 3 };
        let values = self.fixed_function_arguments(count)?;
        match (name, values.as_slice()) {
            ("lerp", [start, end, progress]) => interpolate_fraction(start, end, progress),
            ("invlerp", [start, end, value]) => fraction_progress(start, end, value),
            ("remap", [value, start, end, output_start, output_end]) => {
                let progress = fraction_progress(start, end, value)?;
                interpolate_fraction(output_start, output_end, &progress)
            }
            _ => unreachable!("插值函数参数数量已确定"),
        }
    }

    fn round_function(&mut self) -> Result<BigRational, FractionError> {
        if !self.consume('(') {
            return Err(self.expected_token());
        }
        let value = self.sum()?;
        let places = if self.consume(',') {
            let places = self.sum()?;
            if !places.is_integer() {
                return Err(FractionError::RoundPlaces);
            }
            i32::try_from(places.numer())
                .ok()
                .filter(|places| places.unsigned_abs() <= MAX_DECIMAL_PLACES)
                .ok_or(FractionError::RoundPlaces)?
        } else {
            0
        };
        if !self.consume(')') {
            return Err(self.expected_token());
        }
        rational_round(value, places)
    }

    fn signed_number(&mut self) -> Result<BigRational, FractionError> {
        let input = self.remaining.trim_start();
        let negative = input.starts_with('-');
        self.remaining = input.strip_prefix(['+', '-']).unwrap_or(input);
        let number = self.number()?;
        Ok(if negative { -number } else { number })
    }

    fn number(&mut self) -> Result<BigRational, FractionError> {
        let input = self.remaining.trim_start();
        if let Some((base, digits)) = Base::prefixed(input) {
            let length = digits
                .bytes()
                .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                .count();
            let (digits, rest) = digits.split_at(length);
            self.remaining = rest;
            if digits.is_empty() {
                return Err(self.expected_token());
            }
            let digits = ungroup_digits(digits).ok_or(FractionError::Invalid)?;
            let numerator = BigInt::parse_bytes(digits.as_bytes(), base as u32)
                .ok_or(FractionError::Invalid)?;
            return Ok(BigRational::from_integer(numerator));
        }
        let whole = input.bytes().take_while(u8::is_ascii_digit).count();
        let (fraction, rest) = if let Some(after_dot) = input[whole..].strip_prefix('.') {
            let length = after_dot.bytes().take_while(u8::is_ascii_digit).count();
            after_dot.split_at(length)
        } else {
            ("", &input[whole..])
        };
        if whole == 0 && fraction.is_empty() {
            return Err(self.expected_token());
        }
        self.remaining = rest;
        let exponent = if let Some(exponent) = rest.strip_prefix(['e', 'E']) {
            let digits = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
            let length = digits.bytes().take_while(u8::is_ascii_digit).count();
            if length == 0 {
                self.remaining = digits;
                return Err(self.expected_token());
            }
            let (exponent, rest) = exponent.split_at(exponent.len() - digits.len() + length);
            self.remaining = rest;
            Some(exponent)
        } else {
            None
        };
        let digits = format!("{}{fraction}", &input[..whole]);
        let numerator = BigInt::parse_bytes(digits.as_bytes(), 10).ok_or(FractionError::Invalid)?;
        if numerator.sign() == Sign::NoSign {
            return Ok(BigRational::default());
        }
        let exponent = exponent
            .map_or(Ok(0), str::parse::<i32>)
            .ok()
            .filter(|power| u64::from(power.unsigned_abs()) <= MAX_EXACT_BITS)
            .ok_or(FractionError::ExactRange)?;
        let shift = exponent - fraction.len() as i32;
        let scale = BigInt::from(10u8).pow(shift.unsigned_abs());
        let value = if shift < 0 {
            BigRational::new(numerator, scale)
        } else {
            BigRational::from_integer(numerator * scale)
        };
        ensure_exact_size(&value)?;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_a_secant_expression_that_can_be_evaluated_exactly_again() {
        let line = simplify_fraction("割线式 [1/3,2/3] x**2").unwrap();
        assert_eq!(line, "x - 2/9");
        assert_eq!(
            simplify_fraction(&format!("取值CSV [1/3,2/3] {line}")).as_deref(),
            Ok("x,y\r\n1/3,1/9\r\n2/3,4/9")
        );
        assert_eq!(
            simplify_fraction("secant-line [2,1] 1/7").as_deref(),
            Ok("1/7")
        );
    }

    #[test]
    fn computes_the_exact_average_rate_of_change_in_either_direction() {
        for query in ["平均变化率 [0,1/3] x**2", "secant [1/3,0] x^2"] {
            assert_eq!(simplify_fraction(query).as_deref(), Ok("1/3"));
        }
        assert_eq!(
            simplify_fraction("平均变化率 [0.50,1/2] x"),
            Err(FractionError::EqualSecantPoints)
        );
        assert_eq!(
            simplify_fraction("平均变化率 [1,2,3] x"),
            Err(FractionError::SecantArguments)
        );
        assert_eq!(
            simplify_fraction("平均变化率 [0,1] 1/x"),
            Err(FractionError::ZeroDenominator)
        );
    }

    #[test]
    fn compares_fraction_expressions_exactly_including_factorials_and_unicode_operators() {
        for (query, expected) in [
            ("比较 1/3 < 0.34", "成立"),
            ("比较 2**53+1 > 2**53", "成立"),
            ("compare 5! == 120", "成立"),
            ("比较 ３／２ ≤ １．５", "成立"),
            ("compare 1.50e0 != 3/2", "不成立"),
            ("比较 1/3 < 0.34 <= 1/2", "成立"),
            ("比较 2**53+1 > 2**53 ≥ 2**52", "成立"),
            ("compare 5! == 120 != 119 >= 120", "不成立"),
        ] {
            assert_eq!(simplify_fraction(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            simplify_fraction("比较 1/2 >"),
            Err(FractionError::ComparisonArguments)
        );
        assert_eq!(
            simplify_fraction("比较 2/0 = 1"),
            Err(FractionError::ZeroDenominator)
        );
        assert_eq!(
            simplify_fraction("比较 1/2 > 0 <"),
            Err(FractionError::ComparisonArguments)
        );
        assert_eq!(
            simplify_fraction("比较 1 > 2 < 1/0"),
            Err(FractionError::ZeroDenominator)
        );
    }

    #[test]
    fn reduces_exact_nonnegative_values_to_the_smallest_integer_ratio() {
        for (query, expected) in [
            ("化简比例 0.5,3/4,1", "2:3:4"),
            ("ratio 0,gcd(6,9)/4,1/2", "0:3:2"),
            ("化简比例 2**100,3*2**100,0", "1:3:0"),
        ] {
            assert_eq!(simplify_fraction(query).as_deref(), Ok(expected), "{query}");
        }
        for query in ["ratio 0,0", "ratio 2,-1", "ratio 1"] {
            assert_eq!(
                simplify_fraction(query),
                Err(FractionError::RatioArguments),
                "{query}"
            );
        }
        assert_eq!(
            simplify_fraction("化简比例 1/1e1500,1/(1e1500-1)"),
            Err(FractionError::ExactRange)
        );
    }

    #[test]
    fn converts_mixed_fractions_using_one_sign_for_the_whole_value() {
        for (input, expected) in [
            ("带分数 -7/3", "-2 1/3"),
            ("带分数 (2**80+1)/2", "604462909807314587353088 1/2"),
            ("mixed 3/4", "3/4"),
            ("mixed -8/4", "-2"),
            ("从带分数 -2 1/3", "-7/3"),
            ("from-mixed +2 2/4", "5/2"),
            ("从带分数 -0 1/3", "-1/3"),
            ("从带分数 3/4", "3/4"),
            ("从带分数 2 0.125", "17/8"),
        ] {
            assert_eq!(simplify_fraction(input).as_deref(), Ok(expected), "{input}");
        }
        for input in ["从带分数 -2 -1/3", "从带分数 2 3/2", "从带分数 0.5 1/3"] {
            assert_eq!(
                simplify_fraction(input),
                Err(FractionError::MixedFractionArguments),
                "{input}"
            );
        }
    }

    #[test]
    fn solves_exact_congruences_with_non_coprime_or_redundant_moduli() {
        for (query, expected) in [
            ("同余方程 2,3; 3,5; 2,7", "x ≡ 23 (mod 105)"),
            ("同余方程 3,10; 1,8", "x ≡ 33 (mod 40)"),
            ("同余方程 3,10; 2,8", "无整数解"),
            ("crt -1,6; 5,12", "x ≡ 5 (mod 12)"),
            ("同余方程 10,1; 20,1", "x ≡ 0 (mod 1)"),
            ("同余方程 2,2**60; -1,3", "x ≡ 2 (mod 3458764513820540928)"),
        ] {
            assert_eq!(simplify_fraction(query).as_deref(), Ok(expected), "{query}");
        }
        for query in ["同余方程 2,3", "同余方程 1,0; 1,2", "同余方程 1/2,3; 1,2"] {
            assert_eq!(
                simplify_fraction(query),
                Err(FractionError::CongruenceArguments)
            );
        }
    }

    #[test]
    fn reports_floor_quotient_and_exact_remainder_for_both_divisor_signs() {
        for (query, expected) in [
            ("商和余数 -7,3", "商 -3\n余数 2"),
            ("商和余数 7,-3", "商 -3\n余数 -2"),
            ("divmod 1/2,-1/3", "商 -2\n余数 -1/6"),
            ("商和余数 7,gcd(8,12)", "商 1\n余数 3"),
            ("商和余数 0,-3", "商 0\n余数 0"),
        ] {
            assert_eq!(simplify_fraction(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            simplify_fraction("商和余数 1,0"),
            Err(FractionError::ZeroDenominator)
        );
        assert_eq!(
            simplify_fraction("商和余数 2,3,4"),
            Err(FractionError::QuotientRemainderArguments)
        );
        assert_eq!(simplify_fraction("计算 -7%3").as_deref(), Ok("2"));
    }

    #[test]
    fn counts_and_sums_positive_divisors_of_signed_exact_integers() {
        for (query, expected) in [
            ("因数统计 12", "正因数个数 6\n正因数之和 28"),
            ("因数统计 -360", "正因数个数 24\n正因数之和 1170"),
            ("divisor-stats 1", "正因数个数 1\n正因数之和 1"),
            ("因数统计 104729", "正因数个数 2\n正因数之和 104730"),
        ] {
            assert_eq!(simplify_fraction(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            simplify_fraction("因数统计 0"),
            Err(FractionError::PrimeFactorizationDomain)
        );
    }

    #[test]
    fn factors_nonzero_integers_as_bounded_exact_products() {
        for (query, expected) in [
            ("质因数分解 360", "2**3 * 3**2 * 5"),
            ("质因数分解 -360", "-1 * 2**3 * 3**2 * 5"),
            ("factorize 2**10-1", "3 * 11 * 31"),
            ("质因数分解 1000000000000", "2**12 * 5**12"),
            ("质因数分解 104729", "104729"),
            ("质因数分解 -1", "-1"),
            ("质因数分解 1", "1"),
        ] {
            assert_eq!(simplify_fraction(query).as_deref(), Ok(expected), "{query}");
        }
        for value in ["0", "1/2"] {
            assert_eq!(
                simplify_fraction(&format!("质因数分解 {value}")),
                Err(FractionError::PrimeFactorizationDomain)
            );
        }
        assert_eq!(
            simplify_fraction("质因数分解 -1000000000001"),
            Err(FractionError::PrimeFactorizationRange)
        );
    }

    #[test]
    fn finds_the_closest_fraction_with_a_bounded_denominator_and_stable_ties() {
        for (input, expected) in [
            ("限分母 0.3333 to 100", "1/3"),
            ("limit-denominator 3.1415926535897932 to 1000", "355/113"),
            ("限分母 355/113 to 100", "311/99"),
            ("限分母 1/4 to 2", "0"),
            ("限分母 -3/2 to 1", "-2"),
            ("限分母 3/2 to 1", "1"),
            ("限分母 -355/113 to 113", "-355/113"),
            ("限分母 (2**80+1/3) to 3", "3626777458843887524118529/3"),
        ] {
            assert_eq!(simplify_fraction(input).as_deref(), Ok(expected), "{input}");
        }
        for input in [
            "限分母 1",
            "限分母 1 to 0",
            "限分母 1 to 1000001",
            "限分母 1 to 3/2",
        ] {
            assert_eq!(
                simplify_fraction(input),
                Err(FractionError::DenominatorLimit),
                "{input}"
            );
        }
    }

    #[test]
    fn sums_exact_arithmetic_and_geometric_progressions() {
        for (query, expected) in [
            ("等差求和 1/3,1/6,5", "10/3"),
            ("arithmetic-sum comb(4,2),-1/2,3", "33/2"),
            ("等比求和 1,-1/2,4", "5/8"),
            ("geometric-sum 2,1,128", "256"),
            ("等比求和 2,0,3", "2"),
            ("等比求和 1,-1,128", "0"),
            ("等比求和 1/7,1e2000,1", "1/7"),
        ] {
            assert_eq!(simplify_fraction(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            simplify_fraction("等差求和 1,2,129"),
            Err(FractionError::SequenceArguments),
        );
    }

    #[test]
    fn generates_exact_progressions_in_order_without_computing_an_extra_term() {
        for (query, expected) in [
            ("等差数列 1/3,1/6,5", "1/3\n1/2\n2/3\n5/6\n1"),
            ("arithmetic-sequence comb(4,2),-1/2,3", "6\n11/2\n5"),
            ("等比数列 1,-1/2,4", "1\n-1/2\n1/4\n-1/8"),
            ("geometric-sequence 2,0,3", "2\n0\n0"),
            ("等比数列 1/7,1e2000,1", "1/7"),
        ] {
            assert_eq!(simplify_fraction(query).as_deref(), Ok(expected), "{query}");
        }
        for query in [
            "等差数列 1,2",
            "等比数列 1,2,0",
            "等差数列 1,2,129",
            "等差数列 1,2,3,4",
        ] {
            assert_eq!(
                simplify_fraction(query),
                Err(FractionError::SequenceArguments),
                "{query}"
            );
        }
        assert_eq!(
            simplify_fraction("等比数列 1e1200,1e1200,3"),
            Err(FractionError::ExactRange),
        );
    }

    #[test]
    fn evaluates_a_formula_for_each_exact_table_input() {
        assert_eq!(
            simplify_fraction("取值表 [-2,0,3/2] x**2+1/3").as_deref(),
            Ok("x\ty\n-2\t13/3\n0\t1/3\n3/2\t31/12")
        );
        assert_eq!(
            simplify_fraction("table ［３，２，３］ comb(x,2)-x%2").as_deref(),
            Ok("x\ty\n3\t2\n2\t1\n3\t2")
        );
        for input in [
            "取值表 [] x",
            "取值表 [1,,2] x",
            "取值表 [0,1,2,3,4,5,6,7,8,9,10,11,12] x",
        ] {
            assert_eq!(simplify_fraction(input), Err(FractionError::TableInputs));
        }
        assert_eq!(
            simplify_fraction("取值表 [2,0] 1/x"),
            Err(FractionError::ZeroDenominator)
        );
        assert_eq!(calculate_fraction("x+1"), Err(FractionError::Invalid));
        assert_eq!(
            simplify_fraction("取值表 [2] xx"),
            Err(FractionError::Invalid)
        );
    }

    #[test]
    fn converts_repeating_decimal_notation_to_reduced_fractions() {
        for (input, expected) in [
            ("从循环小数 0.(142857)", "1/7"),
            ("from-repeating-decimal -2.1(6)", "-13/6"),
            ("从百分比 33.(3)%", "1/3"),
            ("from-percent １２．５％", "1/8"),
            ("从百分比 -7/3%", "-7/300"),
            ("０．０８（３）", "1/12"),
            ("+.0(09)", "1/110"),
            ("0.(9)", "1"),
            ("-0.(0)", "0"),
        ] {
            assert_eq!(simplify_fraction(input).as_deref(), Ok(expected), "{input}");
        }
        for input in ["0.1()", "1(3)", "0.(3)0", "0.(a)"] {
            assert_eq!(
                simplify_fraction(input),
                Err(FractionError::RepeatingDecimalSyntax)
            );
        }
        let overlong = format!("0.({})", "1".repeat(MAX_REPEATING_DECIMAL_PLACES + 1));
        assert_eq!(
            simplify_fraction(&overlong),
            Err(FractionError::RepeatingDecimalLength)
        );
    }

    #[test]
    fn returns_exact_continued_fraction_convergents_in_expansion_order() {
        for (query, expected) in [
            ("收敛分数 355/113", "3\n22/7\n355/113"),
            ("convergents -7/3", "-3\n-2\n-7/3"),
            ("收敛分数 1/7", "0\n1/7"),
            ("收敛分数 -2", "-2"),
            ("收敛分数 0", "0"),
        ] {
            assert_eq!(simplify_fraction(query).as_deref(), Ok(expected));
        }
    }

    #[test]
    fn round_trips_finite_continued_fractions_exactly() {
        for (value, terms) in [
            ("355/113", "[3,7,16]"),
            ("-7/3", "[-3,1,2]"),
            ("4/7", "[0,1,1,3]"),
            ("-5", "[-5]"),
            ("0", "[0]"),
        ] {
            assert_eq!(
                simplify_fraction(&format!("连分数 {value}")).as_deref(),
                Ok(terms)
            );
            assert_eq!(
                simplify_fraction(&format!("从连分数 {terms}")).as_deref(),
                Ok(value)
            );
        }
        assert_eq!(
            simplify_fraction("from-continued-fraction [1,2,1]").as_deref(),
            Ok("4/3")
        );
        for terms in ["[]", "[1,0]", "[1,-2]", "[1,,2]", "[1,2.1]"] {
            assert_eq!(
                simplify_fraction(&format!("从连分数 {terms}")),
                Err(FractionError::ContinuedFractionArguments)
            );
        }
        assert_eq!(
            simplify_fraction(&format!(
                "从连分数 [0{}]",
                ",1".repeat(MAX_CONTINUED_FRACTION_TERMS)
            )),
            Err(FractionError::ContinuedFractionLength)
        );
    }

    #[test]
    fn evaluates_exact_power_functions_with_python_modulus_sign() {
        for (input, expected) in [
            ("pow(1/2,-3)", "8"),
            ("pow(2,10,1000)", "24"),
            ("pow(3,-1,11)", "4"),
            ("pow(3,-1,-11)", "-7"),
            ("pow(-3,3,11)", "6"),
            ("pow(3,2,-11)", "-2"),
            ("pow(0,-1,1)", "0"),
            ("pow(3,-1,11)+pow(2,10)", "1028"),
        ] {
            assert_eq!(
                calculate_fraction(input).unwrap().to_string(),
                expected,
                "{input}"
            );
        }
        assert_eq!(
            calculate_fraction("pow(6,-1,9)"),
            Err(FractionError::ModularPowerInverse)
        );
        assert_eq!(
            calculate_fraction("pow(2,3,0)"),
            Err(FractionError::ModularPowerArguments)
        );
        assert_eq!(
            calculate_fraction("pow(1/2,3,5)"),
            Err(FractionError::ModularPowerArguments)
        );
        assert_eq!(
            calculate_fraction("pow(2,2**1024,5)"),
            Err(FractionError::ModularPowerRange)
        );
    }

    #[test]
    fn expands_exact_fractions_with_minimal_repeating_digits() {
        for (query, expected) in [
            ("循环小数 1/7", "0.(142857)"),
            ("循环小数 -7/6", "-1.1(6)"),
            ("repeating-decimal 1/90", "0.0(1)"),
            ("百分比 1/8", "12.5%"),
            ("百分比 1/3", "33.(3)%"),
            ("percent -7/30", "-23.(3)%"),
            ("百分比 0", "0%"),
            ("循环小数 1/8", "0.125"),
            ("循环小数 3-3", "0"),
        ] {
            assert_eq!(simplify_fraction(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            simplify_fraction("循环小数 1/2**513"),
            Err(FractionError::RepeatingDecimalLength)
        );
    }
    #[test]
    fn reads_scientific_notation_without_rounding_exact_operands() {
        for (query, expected) in [
            ("计算 2.5e-3 * 4e3", "10"),
            ("计算 9.007199254740993e15 - 9007199254740992", "1"),
            ("计算 .125E+1", "5/4"),
            ("计算 2e2^2", "40000"),
            ("计算 -0.000e-99999999999999999999", "0"),
            ("小数 125e-4 to 4", "0.0125"),
        ] {
            assert_eq!(simplify_fraction(query).as_deref(), Ok(expected), "{query}");
        }
        for (input, error) in [
            ("1e", FractionError::Incomplete),
            ("1e+", FractionError::Incomplete),
            ("1e+x", FractionError::Invalid),
            ("1e99999999999999999999", FractionError::ExactRange),
            ("1e-8192", FractionError::ExactRange),
        ] {
            assert_eq!(calculate_fraction(input), Err(error), "{input}");
        }
        let padding = " ".repeat(QUERY_LIMIT);
        let padded = format!("{padding}75e-1{padding}");
        assert_eq!(
            parse_fraction_literal(&padded),
            Ok(BigRational::new(15.into(), 2.into()))
        );
    }

    #[test]
    fn computes_relative_change_from_exact_nested_expressions() {
        for (query, expected) in [
            ("relchange(1/3,1/2)", "1/2"),
            ("relchange(2**80,3*2**79)*100", "50"),
            ("RELCHANGE(200,150)", "-1/4"),
            ("relchange(-2,-3)", "1/2"),
            ("relchange(10,0)", "-1"),
        ] {
            assert_eq!(
                calculate_fraction(query).unwrap().to_string(),
                expected,
                "{query}"
            );
        }
        assert_eq!(
            calculate_fraction("relchange(0,1)"),
            Err(FractionError::ZeroDenominator)
        );
        assert!(calculate_fraction("relchange(1,2,3)").is_err());
    }

    #[test]
    fn rounds_exact_fractions_to_integers_in_the_requested_direction() {
        for (query, expected) in [
            ("计算 floor(-7/3)", "-3"),
            ("计算 ceil(-7/3)", "-2"),
            ("计算 trunc(-7/3)", "-2"),
            ("计算 floor(7/3)", "2"),
            ("计算 ceil(7/3)", "3"),
            ("计算 trunc(7/3)", "2"),
            ("计算 floor(9007199254740993.9)", "9007199254740993"),
            ("计算 CEIL(-9007199254740993.9)", "-9007199254740993"),
            ("计算 trunc(-9007199254740993.9)", "-9007199254740993"),
            ("计算 trunc(-2)*floor(1/3)", "0"),
        ] {
            assert_eq!(simplify_fraction(query).as_deref(), Ok(expected), "{query}");
        }
    }

    #[test]
    fn computes_exact_weighted_averages_inside_calculator_expressions() {
        for (query, expected) in [
            ("wavg(1/3,2,1/6,1)*18", "5"),
            ("wavg(80,2,90,3)", "86"),
            (
                "wavg(9007199254740993,1,9007199254740992,1)",
                "18014398509481985/2",
            ),
            ("wavg(20,0,1/7,2)", "1/7"),
        ] {
            assert_eq!(
                calculate_fraction(query).unwrap().to_string(),
                expected,
                "{query}"
            );
        }
        for query in ["wavg(1)", "wavg(1,-1,2,3)", "wavg(1,0,2,0)"] {
            assert_eq!(
                calculate_fraction(query),
                Err(FractionError::WeightedAverageArguments),
                "{query}"
            );
        }
    }

    #[test]
    fn computes_exact_weighted_quantiles_inside_calculator_expressions() {
        for (query, expected) in [
            ("wquantile(3/4,1,1,2,1,4,2)", "4"),
            ("wquantile(1/2,4,2,1,1,2,1)", "2"),
            ("wquantile(0,-1,0,3/8,1)", "3/8"),
            ("wquantile(1,6,0,4,1,1,1)", "4"),
            ("WQUANTILE(2/3,2,1/3,4,1/3,2,1/3)*10", "20"),
            ("wpercentile(25,1/2,0,5/4,3,7/3,1)", "5/4"),
        ] {
            assert_eq!(
                calculate_fraction(query).unwrap().to_string(),
                expected,
                "{query}"
            );
        }
        for query in [
            "wquantile(0)",
            "wquantile(1/2,1,1,2)",
            "wquantile(-1/2,1,1)",
            "wquantile(3/2,1,1)",
            "wquantile(1/2,1,-1,2,2)",
            "wquantile(1/2,1,0,2,0)",
            "wpercentile(101,1,1)",
        ] {
            assert_eq!(
                calculate_fraction(query),
                Err(FractionError::WeightedQuantileArguments),
                "{query}"
            );
        }
    }

    #[test]
    fn computes_exact_absolute_deviations_inside_calculator_expressions() {
        for (query, expected) in [
            ("mad(0,1/2,1,10)", "1/2"),
            ("aad(0,1/2,1,10)*16", "57"),
            ("MAD(-3/2,0,3/2)", "3/2"),
            ("aad(9007199254740993,9007199254740992)", "1/2"),
            ("mad(2**80)+aad(1/3)", "0"),
        ] {
            assert_eq!(
                calculate_fraction(query).unwrap().to_string(),
                expected,
                "{query}"
            );
        }
        let at_length_limit = format!("mad({}1)", "1,".repeat(125));
        assert_eq!(
            calculate_fraction(&at_length_limit).unwrap().to_string(),
            "0"
        );
    }

    #[test]
    fn computes_exact_variance_and_standard_deviation_in_expressions() {
        for (query, expected) in [
            ("variance(1/3,1,5/3)", "4/9"),
            ("stdev(1/3,1,5/3)*3", "2"),
            ("pvariance(1/3,5/3)", "4/9"),
            ("PSTDEV(9007199254740993,9007199254740991)", "1"),
            ("pstdev(5/3)", "0"),
        ] {
            assert_eq!(
                calculate_fraction(query).unwrap().to_string(),
                expected,
                "{query}"
            );
        }
        for query in ["variance(1)", "stdev(1)"] {
            assert_eq!(
                calculate_fraction(query),
                Err(FractionError::SampleVarianceCount)
            );
        }
        assert_eq!(
            calculate_fraction("stdev(1,2)"),
            Err(FractionError::RationalStandardDeviation)
        );
    }

    #[test]
    fn sums_rational_arguments_to_fsum_without_averaging() {
        for (query, expected) in [("fsum(1/3,1/6)", "1/2"), ("FSUM(2**80,1/7,-2**80)*7", "1")] {
            assert_eq!(calculate_fraction(query).unwrap().to_string(), expected);
        }
    }

    #[test]
    fn compares_exact_values_with_relative_and_absolute_tolerances() {
        for (query, expected) in [
            ("isclose(1/3,0.33,0,1/300)", "1"),
            ("isclose(1/3,0.33,0,1/301)", "0"),
            ("isclose(2**80,2**80+1)", "1"),
            ("isclose(2**80,2**80+1,0)", "0"),
            ("isclose(0,0.01,0,1/100)*7", "7"),
        ] {
            assert_eq!(calculate_fraction(query).unwrap().to_string(), expected);
        }
        for query in ["isclose(1)", "isclose(1,1,1)", "isclose(1,1,0,-1)"] {
            assert_eq!(
                calculate_fraction(query),
                Err(FractionError::ToleranceArguments)
            );
        }
    }

    #[test]
    fn computes_exact_squares_and_rational_root_mean_square() {
        for (query, expected) in [
            ("sumsq(-1/3,2/3)", "5/9"),
            ("SUMSQ(-3,2+2)*2", "50"),
            ("sumsq(0)", "0"),
            ("meansquare(1/3,2/3)", "5/18"),
            ("meansquare(3,4)", "25/2"),
            ("rms(-1/3,7/3)", "5/3"),
            ("RMS(1/3,1/3)*3", "1"),
            ("rms(0,0)", "0"),
        ] {
            assert_eq!(
                calculate_fraction(query).unwrap().to_string(),
                expected,
                "{query}"
            );
        }
        assert_eq!(
            calculate_fraction("rms(3,4)"),
            Err(FractionError::RationalRootMeanSquare)
        );
        assert!(calculate_fraction("sumsq()").is_err());
        assert_eq!(
            calculate_fraction("sumsq(1e2000)"),
            Err(FractionError::ExactRange)
        );
    }

    #[test]
    fn computes_weighted_population_variance_inside_exact_expressions() {
        for (query, expected) in [
            ("wpvariance(1/3,2,7/3,1)", "8/9"),
            ("wpstdev(-3,1,3,1)*2", "6"),
            ("wpvariance(90,0,2,1)", "0"),
            ("wpvariance(0,1/2,4,3/2)", "3"),
        ] {
            assert_eq!(calculate_fraction(query).unwrap().to_string(), expected);
        }
        assert_eq!(
            calculate_fraction("wpstdev(0,1,4,3)"),
            Err(FractionError::RationalStandardDeviation)
        );
        for query in ["wpstdev(2,0)", "wpvariance(1,-1,2,2)", "wpvariance(1,1,2)"] {
            assert_eq!(
                calculate_fraction(query),
                Err(FractionError::WeightedSpreadArguments)
            );
        }
    }

    #[test]
    fn interpolates_exact_quantiles_inside_calculator_expressions() {
        for (query, expected) in [
            ("quantile(1/4,3/2,1/2)", "3/4"),
            ("percentile(25,4,1,3,2)", "7/4"),
            (
                "quantile(1/2,9007199254740993,9007199254740992)",
                "18014398509481985/2",
            ),
            ("quantile(0,-1/2,3)+percentile(100,-1/2,3)", "5/2"),
            ("quantile(2/3,1/7)", "1/7"),
        ] {
            assert_eq!(
                calculate_fraction(query).unwrap().to_string(),
                expected,
                "{query}"
            );
        }
        for query in [
            "quantile(1/2)",
            "quantile(-0.01,1,2)",
            "percentile(101,1,2)",
        ] {
            assert_eq!(
                calculate_fraction(query),
                Err(FractionError::QuantileArguments),
                "{query}"
            );
        }
    }

    #[test]
    fn combines_exact_sums_averages_and_medians_in_calculator_expressions() {
        for (query, expected) in [
            ("sum(1e-3,1/4,3/4)", "1001/1000"),
            ("avg(1/3,1/6)*2", "1/2"),
            ("MEAN(1/3,max(2,5/3))", "7/6"),
            ("median(0.5,3,1/3,1/8)", "5/12"),
            (
                "median(9007199254740993,9007199254740992,9007199254740994)",
                "9007199254740993",
            ),
            ("sum(-7/3)+avg(7/3)", "0"),
        ] {
            assert_eq!(
                calculate_fraction(query).unwrap().to_string(),
                expected,
                "{query}"
            );
        }
        for query in ["sum()", "avg(1,)", "median(1,,2)"] {
            assert!(calculate_fraction(query).is_err(), "{query}");
        }
        assert_eq!(
            calculate_fraction("sum(1e2000,1e-2000)"),
            Err(FractionError::ExactRange)
        );
    }

    #[test]
    fn combines_exact_products_and_harmonic_means_in_calculator_expressions() {
        for (query, expected) in [
            ("prod(1/3,1/2,5/7)", "5/42"),
            ("PRODUCT(9007199254740993,2,1/3)", "6004799503160662"),
            ("prod(-2,0,5)", "0"),
            ("prod(-2)", "-2"),
            ("hmean(1/3,1/6)*9", "2"),
            ("hmean(9007199254740993)", "9007199254740993"),
        ] {
            assert_eq!(
                calculate_fraction(query).unwrap().to_string(),
                expected,
                "{query}"
            );
        }
        for query in ["hmean(1,0)", "hmean(-1/2,3)"] {
            assert_eq!(
                calculate_fraction(query),
                Err(FractionError::PositiveMeanArguments)
            );
        }
        assert!(calculate_fraction("prod()").is_err());
        assert_eq!(
            calculate_fraction("prod(1e2000,1e2000)"),
            Err(FractionError::ExactRange),
        );
    }

    #[test]
    fn combines_exact_geometric_means_in_calculator_expressions() {
        for (query, expected) in [
            ("gmean(1/4,9/16)", "3/8"),
            ("gmean(1/8,1/8,1)*4", "1"),
            ("gmean(1e2000,1e2000)/1e2000", "1"),
            ("GMEAN(1e2000,1e-2000)", "1"),
            ("gmean(9007199254740993)", "9007199254740993"),
        ] {
            assert_eq!(
                calculate_fraction(query).unwrap().to_string(),
                expected,
                "{query}"
            );
        }
        for query in ["gmean(81,0)", "gmean(-1,-1)"] {
            assert_eq!(
                calculate_fraction(query),
                Err(FractionError::PositiveMeanArguments)
            );
        }
        assert_eq!(
            calculate_fraction("gmean(1,2)"),
            Err(FractionError::RationalGeometricMean),
        );
    }

    #[test]
    fn combines_exact_interquartile_ranges_inside_calculator_expressions() {
        for (query, expected) in [
            ("iqr(7/3,2/3,5/3,1/3)", "5/4"),
            ("2*iqr(4,1,3,2)", "3"),
            ("iqr(9007199254740993,9007199254740992)", "1/2"),
            ("IQR(1/3)", "0"),
            ("iqr(-2,-2,-2)", "0"),
        ] {
            assert_eq!(
                calculate_fraction(query).unwrap().to_string(),
                expected,
                "{query}"
            );
        }
        assert!(calculate_fraction("iqr()").is_err());
    }

    #[test]
    fn compares_exact_fractions_and_clamps_without_rounding() {
        for (query, expected) in [
            ("计算 min(1/3,0.3334,-1/7)", "-1/7"),
            (
                "计算 max(9007199254740992,9007199254740993)",
                "9007199254740993",
            ),
            ("计算 max(-7/3)", "-7/3"),
            ("计算 clamp(0.334,0,1/3)", "1/3"),
            ("计算 clamp(-1,-1/3,1/3)", "-1/3"),
            ("计算 clamp(1/7,0,min(2/7,1/3))", "1/7"),
            ("计算 clamp(0,1/7,1/7)", "1/7"),
        ] {
            assert_eq!(simplify_fraction(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            calculate_fraction("clamp(1,2,0)"),
            Err(FractionError::ClampBounds)
        );
        for query in ["min()", "max(1,)", "clamp(0,1)", "clamp(0,1,2,3)"] {
            assert!(calculate_fraction(query).is_err(), "{query}");
        }
    }

    #[test]
    fn interpolates_progress_and_remaps_intervals_using_exact_fractions() {
        for (query, expected) in [
            ("计算 lerp(0,10,1/3)", "10/3"),
            ("计算 LERP(2,2,1/2)", "2"),
            ("计算 invlerp(10,0,7)", "3/10"),
            ("计算 remap(1/3,0,1,10,20)", "40/3"),
            ("计算 remap(2,3,1,4,0)", "2"),
            (
                "计算 remap(9007199254740993,9007199254740992,9007199254740995,0,1)",
                "1/3",
            ),
        ] {
            assert_eq!(simplify_fraction(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            ("invlerp(0,0,0)", FractionError::InterpolationInterval),
            ("lerp(0,1,101/100)", FractionError::InterpolationRange),
            ("remap(2,0,1,0,100)", FractionError::InterpolationRange),
            ("invlerp(0,1,-1)", FractionError::InterpolationRange),
        ] {
            assert_eq!(calculate_fraction(query), Err(error), "{query}");
        }
    }

    #[test]
    fn aligns_exact_fractions_to_positive_step_multiples_using_even_ties() {
        for (query, expected) in [
            ("就近对齐 2.625 to 0.05", "13/5"),
            ("round-step 2.675 to 0.05", "27/10"),
            ("就近对齐 -2.625 to 0.05", "-13/5"),
            ("向下对齐 -2.675 to 0.05", "-27/10"),
            ("向上对齐 -2.675 to 0.05", "-53/20"),
            ("就近对齐 11/7 to 1/3", "5/3"),
            ("floor-step 5/6 to 1/6", "5/6"),
            ("ceil-step 0 to 1/8", "0"),
        ] {
            assert_eq!(simplify_fraction(query).as_deref(), Ok(expected), "{query}");
        }
        for query in ["就近对齐 1", "就近对齐 1 to 0", "floor-step 1 to -0.05"] {
            assert_eq!(
                simplify_fraction(query),
                Err(FractionError::AlignmentArguments),
                "{query}"
            );
        }
    }

    #[test]
    fn calculates_exact_fractions_with_prefixed_integer_literals() {
        for (input, expected) in [
            ("计算 0x2000_0000_0000_01 + 0b10", "9007199254740995"),
            ("计算 ０Ｘ１ＦＦ / 0o10", "511/8"),
            ("计算 -0O10**2 + 0B1", "-63"),
            ("计算 gcd(0x1e,0b10010)", "6"),
        ] {
            assert_eq!(simplify_fraction(input).as_deref(), Ok(expected), "{input}");
        }
        assert_eq!(
            parse_fraction_literal("-0x21/0b10"),
            Ok(BigRational::new((-33).into(), 2.into()))
        );
        for (input, error) in [
            ("0x", FractionError::Incomplete),
            ("0x+1", FractionError::Invalid),
            ("0o8", FractionError::Invalid),
            ("0b1__1", FractionError::Invalid),
            ("0x_ff", FractionError::Invalid),
            ("0xff_", FractionError::Invalid),
            ("0x10.5", FractionError::Invalid),
        ] {
            assert_eq!(calculate_fraction(input), Err(error), "{input}");
        }
    }

    #[test]
    fn calculates_exact_combinations_and_permutations_with_expression_arguments() {
        for (input, expected) in [
            ("计算 comb(100,50)", "100891344545564193334812497256"),
            ("计算 comb(100,50)/COMB(99,49)", "2"),
            ("计算 comb(4+2,4/2)!", "1307674368000"),
            ("计算 comb(3,4)", "0"),
            ("计算 comb(0,0)", "1"),
            ("计算 perm(24,4)", "255024"),
            ("计算 perm(25)", "15511210043330985984000000"),
            ("计算 perm(3,4)", "0"),
            ("计算 perm(0)", "1"),
        ] {
            assert_eq!(simplify_fraction(input), Ok(expected.into()), "{input}");
        }
        for (input, expected) in [
            ("计算 comb(4,1/2)", FractionError::CombinatorialDomain),
            ("计算 perm(-1,1)", FractionError::CombinatorialDomain),
            ("计算 comb(4097,1)", FractionError::CombinatorialRange),
            ("计算 perm(4096,4096)", FractionError::CombinatorialRange),
            ("计算 comb(10,", FractionError::Incomplete),
            ("计算 comb(10)", FractionError::Invalid),
            ("计算 perm(10,)", FractionError::Invalid),
            ("计算 comb(8,2,1)", FractionError::Invalid),
        ] {
            assert_eq!(simplify_fraction(input), Err(expected), "{input}");
        }
    }

    #[test]
    fn evaluates_exact_square_roots_and_absolute_values_inside_expressions() {
        for (expression, expected) in [
            ("sqrt(81/16)+abs(-1/2)", "11/4"),
            ("SQRT(abs(-9/4))", "3/2"),
            ("abs(-2)**3", "8"),
            ("sqrt(0)", "0"),
        ] {
            assert_eq!(
                calculate_fraction(expression).unwrap().to_string(),
                expected
            );
        }
        for expression in ["sqrt(2)", "sqrt(-9/4)", "sqrt(1/2)"] {
            assert_eq!(
                calculate_fraction(expression),
                Err(FractionError::RationalSquareRoot)
            );
        }
        assert_eq!(calculate_fraction("abs(1,2)"), Err(FractionError::Invalid));
    }

    #[test]
    fn evaluates_bitwise_functions_on_exact_integers_wider_than_float_precision() {
        for (query, expected) in [
            ("计算 shl(1,100)", "1267650600228229401496703205376"),
            ("计算 bitxor(0x2000_0000_0000_01,0b1)", "9007199254740992"),
            ("计算 bitor(2**80,3)", "1208925819614629174706179"),
            ("计算 bitand(2**80+7,3)", "3"),
            ("计算 popcount(2**80 + 3)", "3"),
            ("计算 shr(2**100+3,100)", "1"),
            ("计算 shr(1,2**100)", "0"),
            ("计算 shl(0,2**100)", "0"),
        ] {
            assert_eq!(simplify_fraction(query).as_deref(), Ok(expected), "{query}");
        }
        for query in ["bitand(-1,3)", "bitor(1/2,3)", "shl(1,-1)", "popcount(-1)"] {
            assert_eq!(
                calculate_fraction(query),
                Err(FractionError::BitwiseArguments),
                "{query}"
            );
        }
        assert_eq!(
            calculate_fraction("shl(1,8191)").unwrap().numer().bits(),
            MAX_EXACT_BITS
        );
        assert_eq!(
            calculate_fraction("shl(1,8192)"),
            Err(FractionError::ExactRange)
        );
    }

    #[test]
    fn evaluates_integer_root_and_divisibility_functions_beyond_float_precision() {
        for (expression, expected) in [
            ("isqrt(2**100 - 1)", "1125899906842623"),
            ("ISQRT(0)", "0"),
            ("gcd(0, -3 * 2**80, 5 * 2**80)", "1208925819614629174706176"),
            ("lcm(-3 * 2**80, 5 * 2**80)", "18133887294219437620592640"),
            ("gcd() + lcm()", "1"),
            ("lcm(1e2000, 1e2000 - 1, 0)", "0"),
        ] {
            assert_eq!(
                calculate_fraction(expression).unwrap().to_string(),
                expected
            );
        }
        for (expression, expected) in [
            ("isqrt(-1)", FractionError::IntegerSquareRoot),
            ("isqrt(1/2)", FractionError::IntegerSquareRoot),
            ("gcd(1, 1/2)", FractionError::IntegerAggregate),
            ("lcm(1e2000, 1e2000 - 1)", FractionError::ExactRange),
            ("lcm(1,)", FractionError::Invalid),
        ] {
            assert_eq!(calculate_fraction(expression), Err(expected));
        }
    }

    #[test]
    fn calculates_exact_fraction_roots_without_rounding() {
        for (expression, expected) in [
            ("root(-125/343,3)", "-5/7"),
            ("ROOT(1/81,2+2)", "1/3"),
            ("root(2**1024,256)", "16"),
            ("root(-1/2,1)", "-1/2"),
            ("root(0,1024)", "0"),
            ("root(256/81,4) + root(1/8,3)", "11/6"),
        ] {
            assert_eq!(
                calculate_fraction(expression).unwrap().to_string(),
                expected,
                "{expression}"
            );
        }
        for (expression, expected) in [
            ("root(2,3)", FractionError::RationalRoot),
            ("root(1/2,3)", FractionError::RationalRoot),
            ("root(-16/81,4)", FractionError::RationalRoot),
            ("root(4,0)", FractionError::RootDegree),
            ("root(4,-1)", FractionError::RootDegree),
            ("root(4,3/2)", FractionError::RootDegree),
            ("root(4,1025)", FractionError::RootDegree),
        ] {
            assert_eq!(
                calculate_fraction(expression),
                Err(expected),
                "{expression}"
            );
        }
    }

    #[test]
    fn rounds_inside_exact_expressions_to_even_and_accepts_negative_places() {
        for (expression, expected) in [
            ("round(1/3,2)*3", "99/100"),
            ("ROUND(2.675,1+1)", "67/25"),
            ("round(1250,-2)", "1200"),
            ("round(-1350,-2)", "-1400"),
            ("round(5/2)", "2"),
            ("round(-3/2)", "-2"),
            ("round(-1/3)", "0"),
        ] {
            assert_eq!(
                calculate_fraction(expression).unwrap().to_string(),
                expected
            );
        }
        for expression in ["round(1,101)", "round(1,-101)", "round(1,1/2)"] {
            assert_eq!(
                calculate_fraction(expression),
                Err(FractionError::RoundPlaces)
            );
        }
    }

    #[test]
    fn rounds_exact_expressions_to_decimal_places_with_ties_to_even() {
        for (query, expected) in [
            ("小数 1/3 to 8", "0.33333333"),
            ("小数 1/8 转 ４位小数", "0.1250"),
            ("decimal 1/2 to 0", "0"),
            ("小数 -0.005 to 2", "0.00"),
            ("小数 -2.675 to 2", "-2.68"),
            ("小数 2.685 to 2", "2.68"),
            ("小数 9.995 to 2", "10.00"),
            (
                "小数 100000000000000000001/3 to 2",
                "33333333333333333333.67",
            ),
        ] {
            assert_eq!(simplify_fraction(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            ("小数 1/3 to", FractionError::Incomplete),
            ("小数 1/3 to 101", FractionError::DecimalPlaces),
            ("小数 1/3 to -1", FractionError::DecimalPlaces),
            ("小数 1/3 to 2.5", FractionError::DecimalPlaces),
            ("小数 1/0 to 2", FractionError::ZeroDenominator),
        ] {
            assert_eq!(simplify_fraction(query), Err(error), "{query}");
        }
    }

    #[test]
    fn formats_engineering_notation_after_rounding_exact_significant_digits() {
        for (query, expected) in [
            ("工程计数法 12345 to 4", "12.34e3"),
            ("工程计数法 0.000012345 转 ４位", "12.34e-6"),
            ("工程计数法 9.995 to 3", "10.0e0"),
            ("engineering-notation -999.5 to 3", "-1.00e3"),
            ("工程计数法 120.01 to 2", "120e0"),
            ("工程计数法 0 to 3", "0.00e0"),
            ("工程计数法 10**1000 to 3", "10.0e999"),
            ("工程计数法 1/10**1000 to 3", "100e-1002"),
        ] {
            assert_eq!(simplify_fraction(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            simplify_fraction("工程计数法 1 to 101"),
            Err(FractionError::SignificantDigits),
        );
    }

    #[test]
    fn rounds_exact_expressions_to_significant_digits_and_renormalizes_carry() {
        for (query, expected) in [
            ("有效数字 1/3 to 4", "3.333e-1"),
            ("有效数字 0.0012345 转 ４位", "1.234e-3"),
            ("significant-digits -1.255 to 3", "-1.26e0"),
            ("有效数字 12345 to 3", "1.23e4"),
            ("有效数字 9.995 to 3", "1.00e1"),
            ("有效数字 1/8 to 1", "1e-1"),
            ("有效数字 99/10 to 1", "1e1"),
            ("有效数字 0 to 3", "0.00e0"),
            ("有效数字 10**1000 to 2", "1.0e1000"),
            ("有效数字 1/10**1000 to 2", "1.0e-1000"),
        ] {
            assert_eq!(simplify_fraction(query).as_deref(), Ok(expected), "{query}");
        }
        for query in ["有效数字 2 to 0", "有效数字 2 to 101", "有效数字 2 to 2.5"] {
            assert_eq!(
                simplify_fraction(query),
                Err(FractionError::SignificantDigits)
            );
        }
    }

    #[test]
    fn floors_division_and_uses_the_divisor_sign_for_exact_remainders() {
        for (query, expected) in [
            ("计算 2**80//3", "402975273204876391568725"),
            ("计算 2**80%3", "1"),
            ("计算 -7.5//2", "-4"),
            ("计算 7.5//-2", "-4"),
            ("计算 -7.5%2", "1/2"),
            ("计算 7.5%-2", "-1/2"),
            ("计算 (7/3) % (1/2)", "1/3"),
            ("计算 3+9//2*2", "11"),
            ("计算 2^8 ％ ３", "1"),
            ("计算 0%2", "0"),
        ] {
            assert_eq!(simplify_fraction(query).as_deref(), Ok(expected), "{query}");
        }
        for query in ["计算 1//0", "计算 1%(1-1)"] {
            assert_eq!(
                simplify_fraction(query),
                Err(FractionError::ZeroDenominator),
                "{query}"
            );
        }
        assert_eq!(
            simplify_fraction("计算 1 / /2"),
            Err(FractionError::Invalid)
        );
        assert_eq!(
            simplify_fraction("计算 1//"),
            Err(FractionError::Incomplete)
        );
    }

    #[test]
    fn calculates_exact_factorials_before_powers_and_unary_signs() {
        for (query, expected) in [
            ("计算 0!", "1"),
            ("计算 5.0!", "120"),
            ("计算 (2+3)!/2**3", "15"),
            ("计算 2**3!", "64"),
            ("计算 3!**2", "36"),
            ("计算 -3！", "-6"),
            (
                "计算 50!",
                "30414093201713378043612608166064768844377641568960512000000000000",
            ),
        ] {
            assert_eq!(simplify_fraction(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            ("计算 (-3)!", FractionError::FactorialDomain),
            ("计算 (1/2)!", FractionError::FactorialDomain),
            ("计算 3!!", FractionError::Invalid),
            ("计算 1024!", FractionError::FactorialRange),
            ("计算 1025!", FractionError::FactorialRange),
        ] {
            assert_eq!(simplify_fraction(query), Err(error), "{query}");
        }
    }

    #[test]
    fn converts_finite_decimals_and_reduces_integer_fractions_exactly() {
        for (query, expected) in [
            ("42/56", "3/4"),
            ("－４２／５６", "-3/4"),
            ("-42 / -56", "3/4"),
            ("0/-3", "0"),
            ("25/5", "5"),
            ("0.125", "1/8"),
            ("-.500", "-1/2"),
            ("12.50", "25/2"),
            (
                "100000000000000000000000000000000000000.5",
                "200000000000000000000000000000000000001/2",
            ),
            (
                "340282366920938463463374607431768211455/3",
                "113427455640312821154458202477256070485",
            ),
        ] {
            assert_eq!(simplify_fraction(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            ("", FractionError::Incomplete),
            ("12/", FractionError::Incomplete),
            ("1/0", FractionError::ZeroDenominator),
            ("1/2/3", FractionError::Invalid),
            (
                "340282366920938463463374607431768211456/7",
                FractionError::OutOfRange,
            ),
        ] {
            assert_eq!(simplify_fraction(query), Err(error), "{query}");
        }
    }

    #[test]
    fn calculates_rational_expressions_without_floating_point_rounding() {
        for (query, expected) in [
            ("计算 (1/2 + 1/3) * 6", "5"),
            ("CALC 0.1 + 0.2", "3/10"),
            ("计算 -(1/3+1/6)÷(2.5-1)", "-1/3"),
            ("计算 1/2/3", "1/6"),
            ("计算 10 - 2×3", "4"),
            ("计算 1--.5", "3/2"),
            (
                "计算 340282366920938463463374607431768211456/2",
                "170141183460469231731687303715884105728",
            ),
        ] {
            assert_eq!(simplify_fraction(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, expected) in [
            ("计算", FractionError::Incomplete),
            ("计算 (1+2", FractionError::Incomplete),
            ("计算 (1+2]", FractionError::Invalid),
            ("计算 2/(1-1)", FractionError::ZeroDenominator),
            ("计算 1.2.3", FractionError::Invalid),
        ] {
            assert_eq!(simplify_fraction(query), Err(expected), "{query}");
        }
        assert_eq!(
            simplify_fraction(&format!("计算 {}", "9".repeat(QUERY_LIMIT + 1))),
            Err(FractionError::TooLong)
        );
    }

    #[test]
    fn calculates_integer_powers_with_right_associativity_and_checked_size() {
        for (query, expected) in [
            ("计算 2**80", "1208925819614629174706176"),
            ("计算 2**3^2", "512"),
            ("计算 -2**2", "-4"),
            ("计算 (-2)^2", "4"),
            ("计算 2**-3^2", "1/512"),
            ("计算 (2/3)^-2", "9/4"),
            ("计算 3*2^2+1", "13"),
            ("计算 0^0", "1"),
        ] {
            assert_eq!(simplify_fraction(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, expected) in [
            ("计算 2**", FractionError::Incomplete),
            ("计算 0**-1", FractionError::ZeroDenominator),
            ("计算 2**0.5", FractionError::NonIntegerExponent),
            ("计算 2**1025", FractionError::ExactRange),
            ("计算 2**(2**20)", FractionError::ExactRange),
            ("计算 10^1024*10^1024*10^1024", FractionError::ExactRange),
        ] {
            assert_eq!(simplify_fraction(query), Err(expected), "{query}");
        }
    }
}
