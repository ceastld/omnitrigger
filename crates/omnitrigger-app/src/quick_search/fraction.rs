use num_bigint::{BigInt, BigUint, Sign};
use num_rational::BigRational;

use super::calculator::{normalize, QUERY_LIMIT};
use super::conversion::split_conversion;
use super::operators::Operator;

const MAX_POWER_EXPONENT: u32 = 1024;
const MAX_FACTORIAL_ARGUMENT: u32 = 1024;
const MAX_COMBINATORIAL_ARGUMENT: u32 = 4096;
const MAX_EXACT_BITS: u64 = 8192;
const MAX_DECIMAL_PLACES: u32 = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FractionError {
    Incomplete,
    Invalid,
    ZeroDenominator,
    OutOfRange,
    TooLong,
    NonIntegerExponent,
    FactorialDomain,
    FactorialRange,
    CombinatorialDomain,
    CombinatorialRange,
    DecimalPlaces,
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
            Self::FactorialDomain => "阶乘的参数必须是非负整数",
            Self::FactorialRange => "阶乘参数最多 1024，结果最多 8192 个二进制位",
            Self::CombinatorialDomain => "组合数和排列数的参数必须是非负整数",
            Self::CombinatorialRange => "组合数和排列数的参数最多 4096，结果最多 8192 个二进制位",
            Self::DecimalPlaces => "输入 0 至 100 的小数位数，例如 分数: 小数 1/3 to 8",
            Self::ExactRange => "指数绝对值最多 1024，分子和分母各最多 8192 个二进制位",
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

pub(super) fn calculate_fraction(input: &str) -> Result<BigRational, FractionError> {
    if input.chars().take(QUERY_LIMIT + 1).count() > QUERY_LIMIT {
        return Err(FractionError::TooLong);
    }
    let mut parser = FractionParser { remaining: input };
    let value = parser.sum()?;
    if !parser.remaining.trim().is_empty() {
        return Err(FractionError::Invalid);
    }
    Ok(value)
}

fn rounded_decimal(value: BigRational, places: u32) -> String {
    let unit = BigUint::from(10u8).pow(places);
    let numerator = value.numer().magnitude() * unit;
    let denominator = value.denom().magnitude();
    let mut rounded = &numerator / denominator;
    let twice_remainder = (numerator - &rounded * denominator) << 1usize;
    if &twice_remainder > denominator || (&twice_remainder == denominator && rounded.bit(0)) {
        rounded += 1u8;
    }
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

pub(super) fn exact_abs_square_root(value: &BigRational) -> Option<BigRational> {
    let numerator = value.numer().magnitude();
    let denominator = value.denom().magnitude();
    let top = numerator.sqrt();
    let bottom = denominator.sqrt();
    (&top * &top == *numerator && &bottom * &bottom == *denominator)
        .then(|| BigRational::new(top.into(), bottom.into()))
}

pub(super) fn ensure_exact_size(value: &BigRational) -> Result<(), FractionError> {
    if value.numer().bits().max(value.denom().bits()) > MAX_EXACT_BITS {
        return Err(FractionError::ExactRange);
    }
    Ok(())
}

fn rational_power(base: BigRational, exponent: BigRational) -> Result<BigRational, FractionError> {
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
        let mut result = BigInt::from(1u8);
        match self {
            Self::Combination => {
                let k = k.min(n - k);
                // C(n-k+i, i) 在每一步都是整数。
                for index in 1..=k {
                    result *= n - k + index;
                    result /= index;
                }
            }
            Self::Permutation => {
                for factor in n - k + 1..=n {
                    result *= factor;
                    if result.bits() > MAX_EXACT_BITS {
                        return Err(FractionError::CombinatorialRange);
                    }
                }
            }
        }
        let result = BigRational::from_integer(result);
        ensure_exact_size(&result).map_err(|_| FractionError::CombinatorialRange)?;
        Ok(result)
    }
}

struct FractionParser<'a> {
    remaining: &'a str,
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
            self.combinatorial()?
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

    fn combinatorial(&mut self) -> Result<BigRational, FractionError> {
        let input = self.remaining.trim_start();
        let length = input.bytes().take_while(u8::is_ascii_alphabetic).count();
        let (name, rest) = input.split_at(length);
        let function = if name.eq_ignore_ascii_case("comb") {
            CombinatorialFunction::Combination
        } else if name.eq_ignore_ascii_case("perm") {
            CombinatorialFunction::Permutation
        } else {
            return Err(FractionError::Invalid);
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
        let digits = format!("{}{fraction}", &input[..whole]);
        let numerator = BigInt::parse_bytes(digits.as_bytes(), 10).ok_or(FractionError::Invalid)?;
        let denominator = BigInt::from(10u8).pow(fraction.len() as u32);
        Ok(BigRational::new(numerator, denominator))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
