use num_bigint::Sign;
use num_rational::BigRational;

use super::{
    calculator::normalize,
    fraction::{calculate_fraction, ensure_exact_size, format_square_root, FractionError},
    operators::split_top_level_commas,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ComplexError {
    Incomplete,
    Operation,
    Components,
    ZeroDivisor,
    Evaluation(FractionError),
}

impl ComplexError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "填写实部和虚部，两组复数用 @ 隔开",
            Self::Operation => "可用加法、减法、乘法、除法、模和共轭",
            Self::Components => "每个复数按实部、虚部填写，两个部分用逗号隔开",
            Self::ZeroDivisor => "复数的除数不能为零",
            Self::Evaluation(error) => error.message(),
        }
    }
}

enum BinaryOperation {
    Add,
    Subtract,
    Multiply,
    Divide,
}

pub(super) fn calculate_complex(query: &str) -> Result<String, ComplexError> {
    let input = normalize(query);
    let (operation, input) = input
        .trim()
        .split_once(char::is_whitespace)
        .ok_or(ComplexError::Incomplete)?;
    let operation = match operation.to_ascii_lowercase().as_str() {
        "加法" | "add" => BinaryOperation::Add,
        "减法" | "subtract" => BinaryOperation::Subtract,
        "乘法" | "multiply" => BinaryOperation::Multiply,
        "除法" | "divide" => BinaryOperation::Divide,
        "模" | "modulus" | "abs" => {
            let [real, imaginary] = components(input)?;
            let squared = &real * &real + &imaginary * &imaginary;
            ensure_exact_size(&squared).map_err(ComplexError::Evaluation)?;
            return Ok(format_square_root(&squared));
        }
        "共轭" | "conjugate" => {
            let [real, imaginary] = components(input)?;
            return format_complex(real, -imaginary);
        }
        _ => return Err(ComplexError::Operation),
    };
    let (left, right) = input.split_once('@').ok_or(ComplexError::Incomplete)?;
    let [a, b] = components(left)?;
    let [c, d] = components(right)?;
    let (real, imaginary) = match operation {
        BinaryOperation::Add => (a + c, b + d),
        BinaryOperation::Subtract => (a - c, b - d),
        BinaryOperation::Multiply => (&a * &c - &b * &d, a * d + b * c),
        BinaryOperation::Divide => {
            let denominator = &c * &c + &d * &d;
            if denominator.numer().sign() == Sign::NoSign {
                return Err(ComplexError::ZeroDivisor);
            }
            (
                (&a * &c + &b * &d) / &denominator,
                (b * c - a * d) / denominator,
            )
        }
    };
    format_complex(real, imaginary)
}

fn components(input: &str) -> Result<[BigRational; 2], ComplexError> {
    let mut terms = split_top_level_commas(input);
    let (Some(real), Some(imaginary), None) = (terms.next(), terms.next(), terms.next()) else {
        return Err(ComplexError::Components);
    };
    Ok([
        calculate_fraction(real).map_err(ComplexError::Evaluation)?,
        calculate_fraction(imaginary).map_err(ComplexError::Evaluation)?,
    ])
}

fn format_complex(real: BigRational, imaginary: BigRational) -> Result<String, ComplexError> {
    for value in [&real, &imaginary] {
        ensure_exact_size(value).map_err(ComplexError::Evaluation)?;
    }
    let sign = imaginary.numer().sign();
    if sign == Sign::NoSign {
        return Ok(real.to_string());
    }
    let absolute = if sign == Sign::Minus {
        -imaginary
    } else {
        imaginary
    };
    let imaginary = if absolute.numer() == absolute.denom() {
        "i".to_owned()
    } else if absolute.is_integer() {
        format!("{absolute}i")
    } else {
        format!("({absolute})i")
    };
    Ok(match (real.numer().sign(), sign) {
        (Sign::NoSign, Sign::Minus) => format!("-{imaginary}"),
        (Sign::NoSign, _) => imaginary,
        (_, Sign::Minus) => format!("{real} - {imaginary}"),
        _ => format!("{real} + {imaginary}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculates_exact_complex_numbers_and_formats_real_imaginary_and_radical_results() {
        for (query, expected) in [
            ("乘法 1,2 @ 3,4", "-5 + 10i"),
            ("除法 1,2 @ 3,4", "11/25 + (2/25)i"),
            ("divide 0,1 @ 0,1", "1"),
            ("减法 0,0 @ 0,1", "-i"),
            ("加法 comb(5,2),-1/2 @ -10,0", "-(1/2)i"),
            ("共轭 -2,3/4", "-2 - (3/4)i"),
            ("模 3,4", "5"),
            ("模 1/2,-1/2", "√(1/2)"),
            ("模 0,0", "0"),
            ("乘法 １，２ ＠ ３，４", "-5 + 10i"),
        ] {
            assert_eq!(calculate_complex(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            ("除法 1,2 @ 0,0", ComplexError::ZeroDivisor),
            ("模 1,2,3", ComplexError::Components),
            ("乘法 1,2", ComplexError::Incomplete),
            ("复数", ComplexError::Incomplete),
            ("未知 1,2", ComplexError::Operation),
        ] {
            assert_eq!(calculate_complex(query), Err(error), "{query}");
        }
    }
}
