use num_bigint::{BigInt, Sign};
use num_rational::BigRational;

use super::calculator::{normalize, QUERY_LIMIT};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FractionError {
    Incomplete,
    Invalid,
    ZeroDenominator,
    OutOfRange,
    TooLong,
}

impl FractionError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入分数或有限小数，例如 42/56 或 0.125",
            Self::Invalid => "请输入分数、有限小数或用 计算 开头的算式",
            Self::ZeroDenominator => "分母不能为零",
            Self::OutOfRange => "分子或分母超出整数范围",
            Self::TooLong => "分数算式最多支持 256 个字符",
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
        return calculate_fraction(expression.trim());
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

fn calculate_fraction(input: &str) -> Result<String, FractionError> {
    if input.chars().take(QUERY_LIMIT + 1).count() > QUERY_LIMIT {
        return Err(FractionError::TooLong);
    }
    let mut parser = FractionParser { remaining: input };
    let value = parser.sum()?;
    if !parser.remaining.trim().is_empty() {
        return Err(FractionError::Invalid);
    }
    Ok(value.to_string())
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
        }
    }

    fn product(&mut self) -> Result<BigRational, FractionError> {
        let mut value = self.factor()?;
        loop {
            if self.consume('*') || self.consume('×') {
                value *= self.factor()?;
            } else if self.consume('/') {
                let divisor = self.factor()?;
                if divisor.numer().sign() == Sign::NoSign {
                    return Err(FractionError::ZeroDenominator);
                }
                value /= divisor;
            } else {
                return Ok(value);
            }
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
        let value = if self.consume('(') {
            let value = self.sum()?;
            if !self.consume(')') {
                return Err(self.expected_token());
            }
            value
        } else {
            self.number()?
        };
        Ok(if negative { -value } else { value })
    }

    fn expected_token(&self) -> FractionError {
        if self.remaining.trim().is_empty() {
            FractionError::Incomplete
        } else {
            FractionError::Invalid
        }
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
}
