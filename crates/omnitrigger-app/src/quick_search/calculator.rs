use super::conversion::ConversionError;
use super::operators::Operator;
use super::percentage::percentage_operands;
use super::radix::Base;
use super::scientific::{named_value, split_name, NamedValue};

pub(super) const QUERY_LIMIT: usize = 256;
pub(super) const MAX_SAFE_INTEGER: u128 = (1 << 53) - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CalculationError {
    Incomplete,
    Invalid,
    DivisionByZero,
    FunctionDomain,
    OutOfRange,
    TooLong,
}

impl CalculationError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "算式未完成",
            Self::Invalid => "无法识别算式",
            Self::DivisionByZero => "不能除以零",
            Self::FunctionDomain => "参数超出函数定义域",
            Self::OutOfRange => "结果超出计算范围",
            Self::TooLong => "算式过长",
        }
    }
}

pub(super) fn looks_like_calculation(query: &str) -> bool {
    let normalized = normalize(query);
    let query = normalized.trim();
    looks_like_expression(query)
        || percentage_operands(query).is_some_and(|(_, base, rate)| {
            looks_like_expression(base) && (rate.is_empty() || looks_like_expression(rate))
        })
}

fn looks_like_expression(query: &str) -> bool {
    if query.starts_with(|ch| matches!(ch, '0'..='9' | '=' | '+' | '-' | '.' | '(' | '√' | '∛'))
    {
        return true;
    }
    let (name, tail) = split_name(query);
    let tail = tail.trim_start();
    match named_value(name) {
        Some(NamedValue::Constant(_)) => tail.is_empty() || Operator::prefix(tail).is_some(),
        Some(NamedValue::Function(_) | NamedValue::Aggregate(_)) => {
            tail.is_empty() || tail.starts_with('(')
        }
        None => false,
    }
}

pub(super) fn calculate(query: &str) -> Result<String, CalculationError> {
    format_result(evaluate(query)?)
}

pub(super) fn evaluate(query: &str) -> Result<f64, CalculationError> {
    evaluate_with_coordinate(query, None)
}

pub(super) fn sample_at_x(query: &str, x: f64) -> Result<Option<f64>, CalculationError> {
    match evaluate_with_coordinate(query, Some(x)) {
        Ok(value) => Ok(Some(value)),
        Err(
            CalculationError::DivisionByZero
            | CalculationError::FunctionDomain
            | CalculationError::OutOfRange,
        ) => Ok(None),
        Err(error) => Err(error),
    }
}

fn evaluate_with_coordinate(query: &str, coordinate: Option<f64>) -> Result<f64, CalculationError> {
    if query.chars().take(QUERY_LIMIT + 1).count() > QUERY_LIMIT {
        return Err(CalculationError::TooLong);
    }

    let normalized = normalize(query);
    let input = normalized.trim();
    evaluate_expression(input.strip_prefix('=').unwrap_or(input).trim(), coordinate)
}

fn evaluate_expression(input: &str, coordinate: Option<f64>) -> Result<f64, CalculationError> {
    if let Some((mode, base, rate)) = percentage_operands(input) {
        let rate = evaluate_expression(rate, coordinate)?;
        return finite(mode.apply(evaluate_expression(base, coordinate)?, rate));
    }
    if let Some(rate) = percentage_literal(input) {
        return Ok(rate);
    }
    let mut parser = Parser {
        rest: input,
        coordinate,
    };
    let value = parser.expression(0)?;
    if !parser.rest.trim().is_empty() {
        return Err(CalculationError::Invalid);
    }
    Ok(value)
}

fn percentage_literal(input: &str) -> Option<f64> {
    let number = input.strip_suffix('%')?.trim_end();
    let sign = if number.starts_with('-') { -1. } else { 1. };
    let digits = number
        .strip_prefix(['-', '+'])
        .unwrap_or(number)
        .trim_start();
    if Base::prefixed(digits).is_some() {
        return None;
    }
    let mut parser = Parser {
        rest: digits,
        coordinate: None,
    };
    let value = parser.number().ok()?;
    parser.rest.is_empty().then_some(sign * value / 100.)
}

pub(super) const FULLWIDTH_ASCII_OFFSET: u32 = 0xfee0;

pub(super) fn narrow_ascii_graphic(ch: char) -> char {
    match ch {
        '！'..='～' => char::from_u32(u32::from(ch) - FULLWIDTH_ASCII_OFFSET).unwrap_or(ch),
        _ => ch,
    }
}

pub(super) fn normalize(query: &str) -> String {
    query
        .chars()
        .map(|ch| match ch {
            '÷' => '/',
            '−' => '-',
            _ => narrow_ascii_graphic(ch),
        })
        .collect()
}

pub(super) fn format_result(value: f64) -> Result<String, CalculationError> {
    let value = finite(value)?;
    if value == 0.0 {
        return Ok("0".into());
    }
    if value.fract() == 0.0 && value.abs() < 1e16 {
        return Ok(value.to_string());
    }
    // 保留 15 位有效数字，避免 0.1 + 0.2 之类的常见二进制浮点尾差。
    let scientific = format!("{value:.14e}");
    let rounded = scientific.parse::<f64>().unwrap_or(value);
    if !rounded.is_finite() {
        return Ok(format!("{value:e}"));
    }
    if (1e-6..1e15).contains(&rounded.abs()) {
        return Ok(rounded.to_string());
    }
    let (mantissa, exponent) = scientific.split_once('e').expect("科学计数法含有指数");
    Ok(format!(
        "{}e{exponent}",
        mantissa.trim_end_matches('0').trim_end_matches('.')
    ))
}

struct Parser<'a> {
    rest: &'a str,
    coordinate: Option<f64>,
}

impl Parser<'_> {
    fn expression(&mut self, min_binding: u8) -> Result<f64, CalculationError> {
        let mut lhs = match self.peek() {
            Some('+') => {
                self.rest = &self.rest[1..];
                self.expression(5)?
            }
            Some('-') => {
                self.rest = &self.rest[1..];
                -self.expression(5)?
            }
            Some(symbol @ ('√' | '∛')) => {
                self.rest = &self.rest[symbol.len_utf8()..];
                let radicand = self.expression(5)?;
                let root = if symbol == '√' {
                    radicand.sqrt()
                } else {
                    radicand.cbrt()
                };
                if root.is_nan() {
                    return Err(CalculationError::FunctionDomain);
                }
                root
            }
            Some('(') => self.parenthesized()?,
            Some('0'..='9' | '.') => self.number()?,
            Some('a'..='z' | 'A'..='Z' | 'π' | 'Π') => self.named()?,
            Some(_) => return Err(CalculationError::Invalid),
            None => return Err(CalculationError::Incomplete),
        };

        while let Some((operator, width)) = Operator::prefix(self.rest.trim_start()) {
            let (left, right) = operator.binding_power();
            if left < min_binding {
                break;
            }
            self.rest = &self.rest.trim_start()[width..];
            let rhs = self.expression(right)?;
            lhs = finite(operator.apply(lhs, rhs)?)?;
        }
        Ok(lhs)
    }

    fn peek(&mut self) -> Option<char> {
        self.rest = self.rest.trim_start();
        self.rest.chars().next()
    }

    fn parenthesized(&mut self) -> Result<f64, CalculationError> {
        self.expect('(')?;
        let value = self.expression(0)?;
        self.expect(')')?;
        Ok(value)
    }

    fn arguments(&mut self) -> Result<Vec<f64>, CalculationError> {
        self.expect('(')?;
        let mut values = vec![self.expression(0)?];
        while self.peek() == Some(',') {
            self.rest = &self.rest[1..];
            values.push(self.expression(0)?);
        }
        self.expect(')')?;
        Ok(values)
    }

    fn expect(&mut self, expected: char) -> Result<(), CalculationError> {
        match self.peek() {
            Some(ch) if ch == expected => {
                self.rest = &self.rest[ch.len_utf8()..];
                Ok(())
            }
            None => Err(CalculationError::Incomplete),
            Some(_) => Err(CalculationError::Invalid),
        }
    }

    fn named(&mut self) -> Result<f64, CalculationError> {
        let (name, rest) = split_name(self.rest);
        self.rest = rest;
        if name.eq_ignore_ascii_case("x") {
            return self
                .coordinate
                .ok_or(CalculationError::Invalid)
                .and_then(finite);
        }
        let value = match named_value(name) {
            Some(NamedValue::Constant(value)) => return Ok(value),
            Some(NamedValue::Function(function)) => function(self.parenthesized()?),
            Some(NamedValue::Aggregate(function)) => function(&self.arguments()?),
            None => return Err(CalculationError::Invalid),
        };
        if value.is_nan() {
            return Err(CalculationError::FunctionDomain);
        }
        finite(value)
    }

    fn number(&mut self) -> Result<f64, CalculationError> {
        if let Some((base, rest)) = Base::prefixed(self.rest) {
            let rest = rest.strip_prefix('_').unwrap_or(rest);
            let end = rest
                .find(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
                .unwrap_or(rest.len());
            if rest.trim().is_empty() {
                return Err(CalculationError::Incomplete);
            }
            let value = base
                .parse_digits(&rest[..end])
                .map_err(|error| match error {
                    ConversionError::OutOfRange => CalculationError::OutOfRange,
                    _ => CalculationError::Invalid,
                })?;
            if value > MAX_SAFE_INTEGER {
                return Err(CalculationError::OutOfRange);
            }
            self.rest = &rest[end..];
            return Ok(value as f64);
        }
        let start = self.rest;
        let has_integer = self.number_digits()?;
        let has_fraction = if let Some(rest) = self.rest.strip_prefix('.') {
            self.rest = rest;
            self.number_digits()?
        } else {
            false
        };
        if !has_integer && !has_fraction {
            return Err(if self.rest.is_empty() {
                CalculationError::Incomplete
            } else {
                CalculationError::Invalid
            });
        }
        if let Some(rest) = self.rest.strip_prefix(['e', 'E']) {
            self.rest = rest.strip_prefix(['+', '-']).unwrap_or(rest);
            if !self.number_digits()? {
                return Err(if self.rest.is_empty() {
                    CalculationError::Incomplete
                } else {
                    CalculationError::Invalid
                });
            }
        }

        let literal = &start[..start.len() - self.rest.len()];
        let value = if literal.contains('_') {
            literal.replace('_', "").parse::<f64>()
        } else {
            literal.parse::<f64>()
        }
        .map_err(|_| CalculationError::Invalid)?;
        finite(value)
    }

    fn number_digits(&mut self) -> Result<bool, CalculationError> {
        if !self.rest.as_bytes().first().is_some_and(u8::is_ascii_digit) {
            return Ok(false);
        }
        let end = self
            .rest
            .find(|ch: char| !ch.is_ascii_digit() && ch != '_')
            .unwrap_or(self.rest.len());
        let digits = &self.rest[..end];
        if digits.contains("__") {
            return Err(CalculationError::Invalid);
        }
        if digits.ends_with('_') {
            return Err(if end == self.rest.len() {
                CalculationError::Incomplete
            } else {
                CalculationError::Invalid
            });
        }
        self.rest = &self.rest[end..];
        Ok(true)
    }
}

fn finite(value: f64) -> Result<f64, CalculationError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(CalculationError::OutOfRange)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluates_prefixed_integer_literals_inside_arithmetic() {
        for (query, expected) in [
            ("0xFF + 1", "256"),
            ("0b1010 * 0o10", "80"),
            ("-0B100 + 0X_FF", "251"),
            ("0x12_34 + 0", "4660"),
            ("gcd(0x30, 0o22)", "6"),
        ] {
            assert_eq!(calculate(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, expected) in [
            ("0x", CalculationError::Incomplete),
            ("0x__ff", CalculationError::Invalid),
            ("0b102", CalculationError::Invalid),
            ("0x20000000000000", CalculationError::OutOfRange),
        ] {
            assert_eq!(calculate(query), Err(expected), "{query}");
        }
    }

    #[test]
    fn calculation_respects_precedence_and_numeric_notation() {
        for (query, expected) in [
            ("12.5 × (8 - 3)", "62.5"),
            ("8 / 4 / 2", "1"),
            ("-2^2", "-4"),
            ("2^3^2", "512"),
            ("2^-3", "0.125"),
            ("2**10", "1024"),
            ("2**3**2", "512"),
            ("-2**2", "-4"),
            ("2**-3", "0.125"),
            ("3 × 2**3", "24"),
            ("２＊＊１０", "1024"),
            ("＝（１．５＋２．５）÷２", "2"),
            ("1e-3 + .002", "0.003"),
            ("1_000 + 20_000", "21000"),
            ("1_000.25_0", "1000.25"),
            ("1.2_5e1_0", "12500000000"),
            ("0.1 + 0.2", "0.3"),
            ("2^53 - 1", "9007199254740991"),
            ("1e-8", "1e-8"),
            ("-0", "0"),
        ] {
            assert_eq!(calculate(query).as_deref(), Ok(expected), "{query}");
        }
    }

    #[test]
    fn floor_division_and_remainder_follow_python_sign_and_precedence_rules() {
        for (query, expected) in [
            ("7 // 3", "2"),
            ("-7 // 3", "-3"),
            ("7 // -3", "-3"),
            ("-7 % 3", "2"),
            ("7 % -3", "-2"),
            ("1.0 // 0.1", "9"),
            ("-1e-300 // 1e300", "-1"),
            ("2**5 % 3 + 1", "3"),
            ("７％３", "1"),
            ("percent(15) * 200", "30"),
        ] {
            assert_eq!(calculate(query).as_deref(), Ok(expected), "{query}");
        }
        assert!(looks_like_calculation("pi % 2"));
        assert_eq!(calculate("7 //"), Err(CalculationError::Incomplete));
        for query in ["7 // 0", "7 % -0"] {
            assert_eq!(calculate(query), Err(CalculationError::DivisionByZero));
        }
    }

    #[test]
    fn percentage_phrases_support_expressions_and_preserve_remainder() {
        for (query, expected) in [
            ("200 的 15%", "30"),
            ("１５％ OF ２００", "30"),
            ("(200 + 40)的12.5%", "30"),
            ("15 % of (200 + 20)", "33"),
            ("pi 的 50%", "1.5707963267949"),
            ("200 增加 15%", "230"),
            ("200 减少 15%", "170"),
            ("200 increased by 15%", "230"),
            ("200 DECREASED BY 15%", "170"),
            ("(80+20)增加(5+5)%", "110"),
            ("200 增加 10% 减少 10%", "198"),
            ("200打8折", "160"),
            ("200 打 8.5 折", "170"),
            ("(80+20)增加20% 打9折", "108"),
            ("7 % -3", "-2"),
        ] {
            assert!(looks_like_calculation(query), "{query}");
            assert_eq!(calculate(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(calculate("15% of"), Err(CalculationError::Incomplete));
        assert!(!looks_like_calculation("用户的50%"));
    }

    #[test]
    fn calculation_evaluates_constants_and_nested_functions() {
        for (query, expected) in [
            ("sqrt(2**6)", "8"),
            ("SQRT（９） + abs(-5)", "8"),
            ("sin(radians(90))", "1"),
            ("degrees(asin(0.5))", "30"),
            ("degrees(acos(0))", "90"),
            ("degrees(atan(1))", "45"),
            ("degrees(π/2)", "90"),
            ("log(e) + log2(8) + log10(100)", "6"),
            ("floor(-1.2) + ceil(1.2)", "0"),
            ("round(2.5)", "2"),
            ("round(3.5)", "4"),
            ("round(-2.5)", "-2"),
            ("trunc(-1.8)", "-1"),
            ("-sqrt(4)^2", "-4"),
        ] {
            assert!(looks_like_calculation(query), "{query}");
            assert_eq!(calculate(query).as_deref(), Ok(expected), "{query}");
        }
        assert!(!looks_like_calculation("extensions"));
        assert!(!looks_like_calculation("pi 工具"));
        assert_eq!(calculate("sqrt("), Err(CalculationError::Incomplete));
        assert_eq!(calculate("sqrt(-1)"), Err(CalculationError::FunctionDomain));
        assert_eq!(calculate("asin(2)"), Err(CalculationError::FunctionDomain));
        assert_eq!(calculate("sqrt(1,2)"), Err(CalculationError::Invalid));
        assert_eq!(calculate("exp(10000)"), Err(CalculationError::OutOfRange));
    }

    #[test]
    fn accepts_square_and_cube_root_symbols_with_normal_expression_precedence() {
        for (query, expected) in [
            ("√16 + ∛(-27)", "1"),
            ("√(9 + 16)", "5"),
            ("2 * √16 + 1", "9"),
            ("√2^6", "8"),
            ("∛-27", "-3"),
        ] {
            assert!(looks_like_calculation(query), "{query}");
            assert_eq!(calculate(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(calculate("√-1"), Err(CalculationError::FunctionDomain));
        assert_eq!(calculate("√"), Err(CalculationError::Incomplete));
    }

    #[test]
    fn evaluates_bitwise_functions_with_exact_nonnegative_integers() {
        for (query, expected) in [
            ("bitand(0xff, 0x0f)", "15"),
            ("bitor(0b1000, 0b0101)", "13"),
            ("bitxor(0b1010, 0b1100)", "6"),
            ("popcount(0xff)", "8"),
            ("bit_count(9007199254740991)", "53"),
            ("shl(0b101,3)", "40"),
            ("shl(1,52)", "4503599627370496"),
            ("shl(0,1000)", "0"),
            ("shr(0xff,4)", "15"),
            ("shr(1,1000)", "0"),
            ("2^10", "1024"),
        ] {
            assert_eq!(calculate(query).as_deref(), Ok(expected), "{query}");
        }
        for query in [
            "bitand(-1,3)",
            "bitor(1.5,3)",
            "popcount(9007199254740992)",
            "shr(1,-1)",
        ] {
            assert_eq!(
                calculate(query),
                Err(CalculationError::FunctionDomain),
                "{query}"
            );
        }
        assert_eq!(calculate("shl(1,53)"), Err(CalculationError::OutOfRange));
        assert_eq!(
            calculate("shl(9007199254740991,1)"),
            Err(CalculationError::OutOfRange)
        );
    }

    #[test]
    fn rounds_at_decimal_places_and_preserves_precision_for_small_inputs() {
        for (query, expected) in [
            ("round(2.675,2)", "2.67"),
            ("round(1.25,1)", "1.2"),
            ("round(1250,-2)", "1200"),
            ("round(-1350,-2)", "-1400"),
            ("round(15,-1)", "20"),
            ("round(25,-1)", "20"),
            ("round(5e22,-23)", "0"),
            ("round(5.0000000000000004e22,-23)", "1e23"),
            ("round(5e24,-25)", "1e25"),
            ("round(1.7e308,-309)", "0"),
            ("round(1e-300,324)", "1e-300"),
            ("log1p(-1e-20)", "-1e-20"),
            ("expm1(1e-20)", "1e-20"),
        ] {
            assert_eq!(calculate(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            calculate("round(3,0.5)"),
            Err(CalculationError::FunctionDomain)
        );
        assert_eq!(
            calculate("round(1.7e308,-308)"),
            Err(CalculationError::OutOfRange)
        );
    }

    #[test]
    fn evaluates_degree_trigonometry_at_regular_angles_and_axes() {
        for (query, expected) in [
            ("sind(30)", "0.5"),
            ("asind(0.5)", "30"),
            ("asind(-1)", "-90"),
            ("acosd(-1)", "180"),
            ("atand(-1)", "-45"),
            ("atan2d(1,-1)", "135"),
            ("atan2d(-1,-1)", "-135"),
            ("cosd(60)", "0.5"),
            ("tand(45)", "1"),
            ("sind(-270)", "1"),
            ("sind(180)", "0"),
            ("sind(360000000090)", "1"),
            ("cosd(270)", "0"),
            ("cosd(-180)", "-1"),
            ("tand(540)", "0"),
        ] {
            assert_eq!(calculate(query).as_deref(), Ok(expected), "{query}");
        }
        for query in ["tand(90)", "tand(270)", "tand(-90)"] {
            assert_eq!(
                calculate(query),
                Err(CalculationError::FunctionDomain),
                "{query}"
            );
        }
    }

    #[test]
    fn converts_a_standalone_percentage_literal_without_changing_modulo() {
        for (query, expected) in [
            ("15%", "0.15"),
            ("－１２．５ ％", "-0.125"),
            ("+ 2.5e1%", "0.25"),
            (".5%", "0.005"),
            ("1_000%", "10"),
            ("200 的 15%", "30"),
            ("15% of 200", "30"),
            ("15%2", "1"),
            ("15%+2", "1"),
        ] {
            assert_eq!(calculate(query).as_deref(), Ok(expected), "{query}");
        }
        for query in ["2+15%", "0xff%", "15%%"] {
            assert!(calculate(query).is_err(), "{query}");
        }
    }

    #[test]
    fn computes_geometric_and_harmonic_means_for_positive_values() {
        for (query, expected) in [
            ("gmean(1,16)", "4"),
            ("gmean(1e-308,1e308)", "1"),
            ("gmean(1e308,1e308)", "1e308"),
            ("hmean(1,4)", "1.6"),
            ("hmean(1e-308,1e308)", "2e-308"),
            ("hmean(1e-308,1e-308)", "1e-308"),
        ] {
            assert_eq!(calculate(query).as_deref(), Ok(expected), "{query}");
        }
        for query in ["gmean(0,4)", "gmean(1,-1)", "hmean(0,1)", "hmean(-1,1)"] {
            assert_eq!(
                calculate(query),
                Err(CalculationError::FunctionDomain),
                "{query}"
            );
        }
    }

    #[test]
    fn averages_values_by_nonnegative_weights_without_overflow() {
        for (query, expected) in [
            ("wavg(80,2,90,3)", "86"),
            ("wavg(100,0,20,1)", "20"),
            ("wavg(-1e308,10,1e308,10)", "0"),
            ("wavg(1e308,1e308,1e308,1e308)", "1e308"),
            ("wavg(1e308,1e-308,0,1e308)", "1e-308"),
        ] {
            assert_eq!(calculate(query).as_deref(), Ok(expected), "{query}");
        }
        for query in ["wavg(1,2,3)", "wavg(1,-1,2,1)", "wavg(1,0,2,0)"] {
            assert_eq!(
                calculate(query),
                Err(CalculationError::FunctionDomain),
                "{query}"
            );
        }
    }

    #[test]
    fn takes_real_roots_at_positive_integer_degrees() {
        for (query, expected) in [
            ("root(81,4)", "3"),
            ("root(-32,5)", "-2"),
            ("root(-27,3)", "-3"),
            ("root((2+2)**3,3)", "4"),
            ("root(2.5,1)", "2.5"),
            ("root(0,200)", "0"),
        ] {
            assert_eq!(calculate(query).as_deref(), Ok(expected), "{query}");
        }
        let large = evaluate("root(1e300,6)").unwrap();
        assert!((large / 1e50 - 1.).abs() < 1e-13);
        for query in [
            "root(-16,4)",
            "root(2,0)",
            "root(2,-1)",
            "root(9,2.5)",
            "root(2,9007199254740992)",
            "root(81)",
        ] {
            assert_eq!(
                calculate(query),
                Err(CalculationError::FunctionDomain),
                "{query}"
            );
        }
    }

    #[test]
    fn interpolates_and_limits_values_in_ranges() {
        for (query, expected) in [
            ("lerp(10,20,0.25)", "12.5"),
            ("lerp(20,10,percent(25))", "17.5"),
            ("lerp(-1e308,1e308,0.75)", "5e307"),
            ("lerp(1e308,1,1)", "1"),
            ("invlerp(10,20,12.5)", "0.25"),
            ("invlerp(20,10,12.5)", "0.75"),
            ("invlerp(-1e308,1e308,0)", "0.5"),
            ("remap(512,0,1024,0,100)", "50"),
            ("remap(25,0,100,100,0)", "75"),
            ("remap(5,20,0,0,100)", "75"),
            ("remap(0,-1e308,1e308,-1e308,1e308)", "0"),
            ("clamp(18,0,10)", "10"),
            ("clamp(-1,0,10)", "0"),
            ("clamp(5,0,10)", "5"),
        ] {
            assert_eq!(calculate(query).as_deref(), Ok(expected), "{query}");
        }
        for query in [
            "lerp(1,2)",
            "remap(1,0,2,100)",
            "remap(3,0,2,0,100)",
            "remap(1,1,1,0,100)",
            "lerp(1,2,-1)",
            "lerp(1,2,2)",
            "invlerp(1,1,1)",
            "invlerp(1,2,3)",
            "clamp(1,2,0)",
        ] {
            assert_eq!(
                calculate(query),
                Err(CalculationError::FunctionDomain),
                "{query}"
            );
        }
    }

    #[test]
    fn evaluates_factorials_combinations_and_permutations_in_the_safe_integer_range() {
        for (query, expected) in [
            ("factorial(10)", "3628800"),
            ("fact(0)", "1"),
            ("fact(18)", "6402373705728000"),
            ("comb(52,5)", "2598960"),
            ("comb(3,5)", "0"),
            ("comb(9007199254740991,1)", "9007199254740991"),
            ("perm(5,3)", "60"),
            ("perm(5)", "120"),
            ("perm(3,5)", "0"),
        ] {
            assert!(looks_like_calculation(query), "{query}");
            assert_eq!(calculate(query).as_deref(), Ok(expected), "{query}");
        }
        for query in ["factorial(-1)", "comb(4,1.5)", "comb(4)", "perm(3,-1)"] {
            assert_eq!(
                calculate(query),
                Err(CalculationError::FunctionDomain),
                "{query}"
            );
        }
        for query in ["factorial(19)", "comb(100,50)", "perm(100,100)"] {
            assert_eq!(
                calculate(query),
                Err(CalculationError::OutOfRange),
                "{query}"
            );
        }
    }

    #[test]
    fn computes_population_and_sample_deviation_without_squaring_large_inputs() {
        for (query, expected) in [
            ("stdev(1,3,5)", "2"),
            ("pstdev(1,3)", "1"),
            ("pstdev(5)", "0"),
            ("pstdev(1e16,1e16+2)", "1"),
            ("pstdev(-1e308,1e308)", "1e308"),
            ("stdev(-1e308,1e308)", "1.4142135623731e308"),
            ("pstdev(-1e-300,1e-300)", "1e-300"),
        ] {
            assert!(looks_like_calculation(query), "{query}");
            assert_eq!(calculate(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(calculate("stdev(1)"), Err(CalculationError::FunctionDomain));
    }

    #[test]
    fn aggregate_functions_accept_expressions_and_require_arguments() {
        for (query, expected) in [
            ("avg(12,18,30)", "20"),
            ("mean（１，２，３）", "2"),
            ("sum(1,sqrt(4),min(3,4))", "6"),
            ("max(-9,-4^2,-12)", "-9"),
            ("min(4)", "4"),
            ("avg(1e308,1e308)", "1e308"),
            ("median(9,1,4)", "4"),
            ("median(4,1,8,2)", "3"),
            ("median(1e308,1.6e308)", "1.3e308"),
            ("quantile(0.25,4,1,3,2)", "1.75"),
            ("percentile(90,10,20,30,40)", "37"),
            ("percentile(0,4,1,3,2) + percentile(100,4,1,3,2)", "5"),
            ("quantile(0.75,42)", "42"),
            ("quantile(0.5,-1e308,1e308)", "0"),
            ("quantile(0.5,1e308,1.6e308)", "1.3e308"),
            ("iqr(4,1,3,2)", "1.5"),
            ("gcd(2^6,-48)", "16"),
            ("gcd(0,0)", "0"),
            ("gcd(9007199254740991,1)", "1"),
            ("gcd(24,-36,60)", "12"),
            ("gcd(-8)", "8"),
            ("lcm(-12,18)", "36"),
            ("lcm(5,0)", "0"),
            ("lcm(4,6,10)", "60"),
            ("lcm(-8)", "8"),
            ("lcm(9007199254740991,2,0)", "0"),
            ("hypot(3,4,12)", "13"),
            ("hypot(1e308,1e308)", "1.4142135623731e308"),
            ("rms(0,6,0,-8)", "5"),
            ("rms(0,0)", "0"),
            ("rms(1e308,-1e308)", "1e308"),
            ("rms(1e-200,-1e-200)", "1e-200"),
            ("corr(1,2,2,4,3,6)", "1"),
            ("corr(1,4,2,3,3,2)", "-1"),
            ("corr(1,1,2,-2,3,1)", "0"),
            ("corr(1e308,1e-308,1.00000000000001e308,2e-308)", "1"),
            ("degrees(atan2(1,-1))", "135"),
            ("log(1024,2)", "10"),
            ("log(e)", "1"),
            ("log(0.25,0.5)", "2"),
            ("pow(-3,2)", "9"),
            ("pow(2,-3)", "0.125"),
        ] {
            assert!(looks_like_calculation(query), "{query}");
            assert_eq!(calculate(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(calculate("avg(1,"), Err(CalculationError::Incomplete));
        assert_eq!(evaluate("rms(1e-310,-1e-310)"), Ok(1e-310));
        for query in [
            "min()", "max(,2)", "sum(2,)", "median()", "hypot()", "iqr()",
        ] {
            assert_eq!(calculate(query), Err(CalculationError::Invalid), "{query}");
        }
        for query in [
            "atan2(1)",
            "atan2(1,2,3)",
            "atan2d(1)",
            "atan2d(1,2,3)",
            "asind(2)",
            "acosd(-2)",
            "log(2,0)",
            "log(2,1)",
            "log(2,-1)",
            "log(1,2,3)",
            "pow(2)",
            "gcd(2.5,1)",
            "gcd(9007199254740992,2)",
            "gcd(8,6,0.5)",
            "lcm(0,2,2.5)",
            "corr(1,2)",
            "corr(1,2,3,4,5)",
            "corr(1,2,1,3)",
            "corr(2,1,3,1)",
            "quantile(0.25)",
            "quantile(1.01,1,2)",
            "percentile(-5e-324,1,2)",
            "percentile(101,1,2)",
        ] {
            assert_eq!(
                calculate(query),
                Err(CalculationError::FunctionDomain),
                "{query}"
            );
        }
        assert_eq!(
            calculate("sum(1e308,1e308)"),
            Err(CalculationError::OutOfRange)
        );
        assert_eq!(
            calculate("lcm(9007199254740991,2)"),
            Err(CalculationError::OutOfRange)
        );
        assert_eq!(
            calculate("lcm(1,9007199254740991,2)"),
            Err(CalculationError::OutOfRange)
        );
    }

    #[test]
    fn calculation_reports_incomplete_invalid_and_unrepresentable_input() {
        for (query, expected) in [
            ("(1+2", CalculationError::Incomplete),
            ("2 ^ -", CalculationError::Incomplete),
            ("2**", CalculationError::Incomplete),
            ("2***3", CalculationError::Invalid),
            ("2 * * 3", CalculationError::Invalid),
            ("2×*3", CalculationError::Invalid),
            ("1e-", CalculationError::Incomplete),
            ("1_", CalculationError::Incomplete),
            ("1__2", CalculationError::Invalid),
            ("1_.2", CalculationError::Invalid),
            ("1._2", CalculationError::Invalid),
            ("1e_2", CalculationError::Invalid),
            ("12 34", CalculationError::Invalid),
            ("2+3)", CalculationError::Invalid),
            ("1/(2-2)", CalculationError::DivisionByZero),
            ("1e309", CalculationError::OutOfRange),
            ("(-1)^0.5", CalculationError::OutOfRange),
        ] {
            assert_eq!(calculate(query), Err(expected), "{query}");
        }
        assert_eq!(
            calculate(&"-".repeat(QUERY_LIMIT + 1)),
            Err(CalculationError::TooLong)
        );
    }
}
