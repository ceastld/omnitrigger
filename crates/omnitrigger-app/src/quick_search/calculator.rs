use super::operators::Operator;
use super::percentage::percentage_operands;
use super::scientific::{named_value, split_name, NamedValue};

pub(super) const QUERY_LIMIT: usize = 256;

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
    if query.starts_with(|ch| matches!(ch, '0'..='9' | '=' | '+' | '-' | '.' | '(')) {
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
    if query.chars().take(QUERY_LIMIT + 1).count() > QUERY_LIMIT {
        return Err(CalculationError::TooLong);
    }

    let normalized = normalize(query);
    let input = normalized.trim();
    evaluate_expression(input.strip_prefix('=').unwrap_or(input).trim())
}

fn evaluate_expression(input: &str) -> Result<f64, CalculationError> {
    if let Some((mode, base, rate)) = percentage_operands(input) {
        let rate = evaluate_expression(rate)?;
        return finite(mode.apply(evaluate_expression(base)?, rate));
    }
    let mut parser = Parser(input);
    let value = parser.expression(0)?;
    if !parser.0.trim().is_empty() {
        return Err(CalculationError::Invalid);
    }
    Ok(value)
}

pub(super) fn normalize(query: &str) -> String {
    query
        .chars()
        .map(|ch| match ch {
            '÷' => '/',
            '−' => '-',
            '！'..='～' => char::from_u32(u32::from(ch) - 0xfee0).unwrap_or(ch),
            _ => ch,
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

struct Parser<'a>(&'a str);

impl Parser<'_> {
    fn expression(&mut self, min_binding: u8) -> Result<f64, CalculationError> {
        let mut lhs = match self.peek() {
            Some('+') => {
                self.0 = &self.0[1..];
                self.expression(5)?
            }
            Some('-') => {
                self.0 = &self.0[1..];
                -self.expression(5)?
            }
            Some('(') => self.parenthesized()?,
            Some('0'..='9' | '.') => self.number()?,
            Some('a'..='z' | 'A'..='Z' | 'π' | 'Π') => self.named()?,
            Some(_) => return Err(CalculationError::Invalid),
            None => return Err(CalculationError::Incomplete),
        };

        while let Some((operator, width)) = Operator::prefix(self.0.trim_start()) {
            let (left, right) = operator.binding_power();
            if left < min_binding {
                break;
            }
            self.0 = &self.0.trim_start()[width..];
            let rhs = self.expression(right)?;
            lhs = finite(operator.apply(lhs, rhs)?)?;
        }
        Ok(lhs)
    }

    fn peek(&mut self) -> Option<char> {
        self.0 = self.0.trim_start();
        self.0.chars().next()
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
            self.0 = &self.0[1..];
            values.push(self.expression(0)?);
        }
        self.expect(')')?;
        Ok(values)
    }

    fn expect(&mut self, expected: char) -> Result<(), CalculationError> {
        match self.peek() {
            Some(ch) if ch == expected => {
                self.0 = &self.0[ch.len_utf8()..];
                Ok(())
            }
            None => Err(CalculationError::Incomplete),
            Some(_) => Err(CalculationError::Invalid),
        }
    }

    fn named(&mut self) -> Result<f64, CalculationError> {
        let (name, rest) = split_name(self.0);
        self.0 = rest;
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
        let start = self.0;
        let has_integer = self.number_digits()?;
        let has_fraction = if let Some(rest) = self.0.strip_prefix('.') {
            self.0 = rest;
            self.number_digits()?
        } else {
            false
        };
        if !has_integer && !has_fraction {
            return Err(if self.0.is_empty() {
                CalculationError::Incomplete
            } else {
                CalculationError::Invalid
            });
        }
        if let Some(rest) = self.0.strip_prefix(['e', 'E']) {
            self.0 = rest.strip_prefix(['+', '-']).unwrap_or(rest);
            if !self.number_digits()? {
                return Err(if self.0.is_empty() {
                    CalculationError::Incomplete
                } else {
                    CalculationError::Invalid
                });
            }
        }

        let literal = &start[..start.len() - self.0.len()];
        let value = if literal.contains('_') {
            literal.replace('_', "").parse::<f64>()
        } else {
            literal.parse::<f64>()
        }
        .map_err(|_| CalculationError::Invalid)?;
        finite(value)
    }

    fn number_digits(&mut self) -> Result<bool, CalculationError> {
        if !self.0.as_bytes().first().is_some_and(u8::is_ascii_digit) {
            return Ok(false);
        }
        let end = self
            .0
            .find(|ch: char| !ch.is_ascii_digit() && ch != '_')
            .unwrap_or(self.0.len());
        let digits = &self.0[..end];
        if digits.contains("__") {
            return Err(CalculationError::Invalid);
        }
        if digits.ends_with('_') {
            return Err(if end == self.0.len() {
                CalculationError::Incomplete
            } else {
                CalculationError::Invalid
            });
        }
        self.0 = &self.0[end..];
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
            ("hypot(3,4,12)", "13"),
            ("hypot(1e308,1e308)", "1.4142135623731e308"),
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
        for query in ["min()", "max(,2)", "sum(2,)", "median()", "hypot()"] {
            assert_eq!(calculate(query), Err(CalculationError::Invalid), "{query}");
        }
        for query in [
            "atan2(1)",
            "atan2(1,2,3)",
            "log(2,0)",
            "log(2,1)",
            "log(2,-1)",
            "log(1,2,3)",
            "pow(2)",
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
