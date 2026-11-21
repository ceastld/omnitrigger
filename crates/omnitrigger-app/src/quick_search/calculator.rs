pub(super) const QUERY_LIMIT: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CalculationError {
    Incomplete,
    Invalid,
    DivisionByZero,
    OutOfRange,
    TooLong,
}

impl CalculationError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "算式未完成",
            Self::Invalid => "无法识别算式",
            Self::DivisionByZero => "不能除以零",
            Self::OutOfRange => "结果超出计算范围",
            Self::TooLong => "算式过长",
        }
    }
}

pub(super) fn calculate(query: &str) -> Result<String, CalculationError> {
    if query.chars().take(QUERY_LIMIT + 1).count() > QUERY_LIMIT {
        return Err(CalculationError::TooLong);
    }

    let normalized = normalize(query);
    let input = normalized.trim();
    let mut parser = Parser(input.strip_prefix('=').unwrap_or(input));
    let value = parser.expression(0)?;
    if !parser.0.trim().is_empty() {
        return Err(CalculationError::Invalid);
    }
    format_result(value)
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
            Some('(') => {
                self.0 = &self.0[1..];
                let value = self.expression(0)?;
                match self.peek() {
                    Some(')') => self.0 = &self.0[1..],
                    None => return Err(CalculationError::Incomplete),
                    _ => return Err(CalculationError::Invalid),
                }
                value
            }
            Some('0'..='9' | '.') => self.number()?,
            Some(_) => return Err(CalculationError::Invalid),
            None => return Err(CalculationError::Incomplete),
        };

        while let Some(op) = self.peek() {
            let (op, width) = if self.0.starts_with("**") {
                ('^', 2)
            } else {
                (op, op.len_utf8())
            };
            let (left, right) = match op {
                '+' | '-' => (1, 2),
                '*' | '×' | '/' => (3, 4),
                '^' => (6, 5),
                _ => break,
            };
            if left < min_binding {
                break;
            }
            self.0 = &self.0[width..];
            let rhs = self.expression(right)?;
            lhs = match op {
                '+' => lhs + rhs,
                '-' => lhs - rhs,
                '*' | '×' => lhs * rhs,
                '/' if rhs == 0.0 => return Err(CalculationError::DivisionByZero),
                '/' => lhs / rhs,
                _ => lhs.powf(rhs),
            };
            finite(lhs)?;
        }
        Ok(lhs)
    }

    fn peek(&mut self) -> Option<char> {
        self.0 = self.0.trim_start();
        self.0.chars().next()
    }

    fn number(&mut self) -> Result<f64, CalculationError> {
        let start = self.0;
        self.0 = self.0.trim_start_matches(|ch: char| ch.is_ascii_digit());
        if let Some(rest) = self.0.strip_prefix('.') {
            self.0 = rest.trim_start_matches(|ch: char| ch.is_ascii_digit());
        }
        if &start[..start.len() - self.0.len()] == "." {
            return Err(if self.0.is_empty() {
                CalculationError::Incomplete
            } else {
                CalculationError::Invalid
            });
        }
        if let Some(rest) = self.0.strip_prefix(['e', 'E']) {
            let digits = rest.strip_prefix(['+', '-']).unwrap_or(rest);
            self.0 = digits.trim_start_matches(|ch: char| ch.is_ascii_digit());
            if self.0.len() == digits.len() {
                return Err(if digits.is_empty() {
                    CalculationError::Incomplete
                } else {
                    CalculationError::Invalid
                });
            }
        }

        let value = start[..start.len() - self.0.len()]
            .parse::<f64>()
            .map_err(|_| CalculationError::Invalid)?;
        finite(value)
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
            ("0.1 + 0.2", "0.3"),
            ("2^53 - 1", "9007199254740991"),
            ("1e-8", "1e-8"),
            ("-0", "0"),
        ] {
            assert_eq!(calculate(query).as_deref(), Ok(expected), "{query}");
        }
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
