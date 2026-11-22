use std::{collections::HashMap, fmt::Write as _};

use num_bigint::{BigInt, BigUint, Sign};
use num_rational::BigRational;

use super::calculator::{normalize, QUERY_LIMIT};
use super::conversion::split_conversion;
use super::operators::{split_top_level_commas, NumberComparison, Operator};

const MAX_POWER_EXPONENT: u32 = 1024;
const MAX_MODULAR_POWER_BITS: u64 = 1024;
const MAX_FACTORIAL_ARGUMENT: u32 = 1024;
pub(super) const MAX_COMBINATORIAL_ARGUMENT: u32 = 4096;
const MAX_EXACT_BITS: u64 = 8192;
const MAX_DECIMAL_PLACES: u32 = 100;
const MAX_REPEATING_DECIMAL_PLACES: usize = 512;
const MAX_TABLE_INPUTS: usize = 12;
const MAX_RATIO_VALUES: usize = 16;
const MAX_CONTINUED_FRACTION_TERMS: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FractionError {
    Incomplete,
    Invalid,
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
    RoundPlaces,
    RationalSquareRoot,
    IntegerSquareRoot,
    IntegerAggregate,
    RepeatingDecimalLength,
    RepeatingDecimalSyntax,
    TableInputs,
    ContinuedFractionArguments,
    ContinuedFractionLength,
    ComparisonArguments,
    RatioArguments,
    ExactRange,
}

impl FractionError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入分数或有限小数，例如 42/56 或 0.125",
            Self::Invalid => "请输入分数、有限小数或用 计算 开头的算式",
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
            Self::RoundPlaces => "round 的位数须为 -100 至 100 的整数",
            Self::RationalSquareRoot => "精确开方需要非负数，且结果能表示为分数",
            Self::IntegerSquareRoot => "isqrt 需要非负整数",
            Self::IntegerAggregate => "gcd 和 lcm 的参数须为整数",
            Self::RepeatingDecimalLength => "循环小数最多展开 512 个小数位",
            Self::RepeatingDecimalSyntax => "用括号填写循环节，例如 0.(142857) 或 -2.1(6)",
            Self::TableInputs => {
                "用方括号填写 1 至 12 个数值，例如 分数: 取值表 [-2,0,3/2] x**2+1/3"
            }
            Self::ContinuedFractionArguments => {
                "用方括号填写连分数整数项，例如 [3,7,16]。第一项之后须为正整数"
            }
            Self::ContinuedFractionLength => "连分数最多支持 512 项",
            Self::ComparisonArguments => "输入两个算式和一个比较符，例如 分数: 比较 1/3 < 0.34",
            Self::RatioArguments => "输入 2 至 16 个非负数，至少一个大于零，例如 0.5,3/4,1",
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
    let normalized = normalize(input);
    let input = normalized.trim();
    let (verb, expression) = input.split_once(char::is_whitespace).unwrap_or((input, ""));
    if verb == "计算" || verb.eq_ignore_ascii_case("calc") {
        return calculate_fraction(expression.trim()).map(|value| value.to_string());
    }
    if verb == "循环小数" || verb.eq_ignore_ascii_case("repeating-decimal") {
        return repeating_decimal(&calculate_fraction(expression.trim())?);
    }
    if verb == "从循环小数" || verb.eq_ignore_ascii_case("from-repeating-decimal") {
        return from_repeating_decimal(expression.trim()).map(|value| value.to_string());
    }
    if verb == "取值表" || verb.eq_ignore_ascii_case("table") {
        return fraction_table(expression.trim());
    }
    if verb == "连分数" || verb.eq_ignore_ascii_case("continued-fraction") {
        return continued_fraction(calculate_fraction(expression.trim())?);
    }
    if verb == "从连分数" || verb.eq_ignore_ascii_case("from-continued-fraction") {
        return from_continued_fraction(expression.trim()).map(|value| value.to_string());
    }
    if verb == "比较" || verb.eq_ignore_ascii_case("compare") {
        return compare_fraction_expressions(expression);
    }
    if verb == "化简比例" || verb.eq_ignore_ascii_case("ratio") {
        return simplify_ratio(expression);
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
        return calculate_fraction(source).map(|value| rounded_decimal(value, places));
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
    let (start, length, comparison) = input
        .char_indices()
        .find_map(|(index, character)| {
            let remainder = &input[index..];
            [2, character.len_utf8()].into_iter().find_map(|length| {
                let operator = remainder.get(..length)?;
                NumberComparison::parse(operator).map(|comparison| (index, length, comparison))
            })
        })
        .ok_or(FractionError::ComparisonArguments)?;
    let left = input[..start].trim();
    let right = input[start + length..].trim();
    if left.is_empty() || right.is_empty() {
        return Err(FractionError::ComparisonArguments);
    }
    let order = calculate_fraction(left)?.cmp(&calculate_fraction(right)?);
    Ok(if comparison.accepts(order) {
        "成立"
    } else {
        "不成立"
    }
    .to_owned())
}

pub(super) fn calculate_fraction(input: &str) -> Result<BigRational, FractionError> {
    calculate_fraction_with_variables(input, None, None)
}

pub(super) fn calculate_fraction_with_variables(
    input: &str,
    x: Option<&BigRational>,
    y: Option<&BigRational>,
) -> Result<BigRational, FractionError> {
    if input.chars().take(QUERY_LIMIT + 1).count() > QUERY_LIMIT {
        return Err(FractionError::TooLong);
    }
    let mut parser = FractionParser {
        remaining: input,
        x,
        y,
    };
    let value = parser.sum()?;
    if !parser.remaining.trim().is_empty() {
        return Err(FractionError::Invalid);
    }
    Ok(value)
}

fn fraction_table(input: &str) -> Result<String, FractionError> {
    let (points, expression) = input
        .strip_prefix('[')
        .and_then(|input| input.split_once(']'))
        .ok_or(FractionError::TableInputs)?;
    let points: Vec<_> = points.split(',').take(MAX_TABLE_INPUTS + 1).collect();
    if points.len() > MAX_TABLE_INPUTS || points.iter().any(|point| point.trim().is_empty()) {
        return Err(FractionError::TableInputs);
    }
    let mut rows = vec!["x\ty".to_owned()];
    for point in points {
        let x = parse_fraction_literal(point)?;
        let y = calculate_fraction_with_variables(expression.trim(), Some(&x), None)?;
        rows.push(format!("{x}\t{y}"));
    }
    Ok(rows.join("\n"))
}

fn continued_fraction(mut value: BigRational) -> Result<String, FractionError> {
    let mut terms = Vec::new();
    for _ in 0..MAX_CONTINUED_FRACTION_TERMS {
        let whole = value.floor();
        value -= &whole;
        terms.push(whole.to_string());
        if value.numer().sign() == Sign::NoSign {
            return Ok(format!("[{}]", terms.join(",")));
        }
        value = value.recip();
    }
    Err(FractionError::ContinuedFractionLength)
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
    let input = input.trim();
    if input.chars().take(QUERY_LIMIT + 1).count() > QUERY_LIMIT {
        return Err(FractionError::TooLong);
    }
    let mut parser = FractionParser {
        remaining: input,
        x: None,
        y: None,
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

fn repeating_decimal(value: &BigRational) -> Result<String, FractionError> {
    let numerator = value.numer().magnitude();
    let denominator = value.denom().magnitude();
    let mut text = (numerator / denominator).to_string();
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
        let scaled = remainder * 10_u8;
        write!(text, "{}", &scaled / denominator).expect("可将一位数字写入字符串");
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

fn rounded_decimal(value: BigRational, places: u32) -> String {
    let unit = BigUint::from(10u8).pow(places);
    let numerator = value.numer().magnitude() * unit;
    let rounded = round_magnitude_to_even(&numerator, value.denom().magnitude());
    let sign = if value.numer().sign() == Sign::Minus && rounded.bits() != 0 {
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
    } else if value.is_integer() {
        format!("√{value}")
    } else {
        format!("√({value})")
    }
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

fn exact_gcd(mut left: BigUint, mut right: BigUint) -> BigUint {
    while right.bits() != 0 {
        let remainder = left % &right;
        left = right;
        right = remainder;
    }
    left
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
                Operator::Remainder => {
                    let quotient = (&value / &right).floor();
                    value - right * quotient
                }
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
        let length = input.bytes().take_while(u8::is_ascii_alphabetic).count();
        let (name, rest) = input.split_at(length);
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
        if matches!(name.as_str(), "gcd" | "lcm") {
            self.remaining = rest;
            return self.integer_aggregate(name == "lcm");
        }
        if matches!(name.as_str(), "sqrt" | "abs" | "isqrt") {
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
    fn compares_fraction_expressions_exactly_including_factorials_and_unicode_operators() {
        for (query, expected) in [
            ("比较 1/3 < 0.34", "成立"),
            ("比较 2**53+1 > 2**53", "成立"),
            ("compare 5! == 120", "成立"),
            ("比较 ３／２ ≤ １．５", "成立"),
            ("compare 1.50e0 != 3/2", "不成立"),
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
