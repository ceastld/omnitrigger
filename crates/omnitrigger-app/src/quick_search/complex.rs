use num_bigint::Sign;
use num_rational::BigRational;

use super::{
    calculator::normalize,
    fraction::{calculate_fraction, ensure_exact_size, format_square_root, FractionError},
    operators::split_top_level_commas,
};

const MAX_POWER: u32 = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ComplexError {
    Incomplete,
    Operation,
    Components,
    Exponent,
    ZeroDivisor,
    Evaluation(FractionError),
}

impl ComplexError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "填写实部和虚部，两组复数用 @ 隔开",
            Self::Operation => "可用加法、减法、乘法、除法、模、共轭和乘方",
            Self::Components => "每个复数按实部、虚部填写，两个部分用逗号隔开",
            Self::Exponent => "在 @ 后填写 −1024 至 1024 的整数指数",
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
            return format_complex([real, -imaginary]);
        }
        "乘方" | "pow" | "power" => {
            let (base, exponent) = input.split_once('@').ok_or(ComplexError::Incomplete)?;
            let exponent = exponent
                .trim()
                .parse::<i32>()
                .ok()
                .filter(|value| value.unsigned_abs() <= MAX_POWER)
                .ok_or(ComplexError::Exponent)?;
            return integer_power(components(base)?, exponent).and_then(format_complex);
        }
        _ => return Err(ComplexError::Operation),
    };
    let (left, right) = input.split_once('@').ok_or(ComplexError::Incomplete)?;
    let left = components(left)?;
    let right = components(right)?;
    let [a, b] = &left;
    let [c, d] = &right;
    let result = match operation {
        BinaryOperation::Add => [a + c, b + d],
        BinaryOperation::Subtract => [a - c, b - d],
        BinaryOperation::Multiply => multiply(&left, &right),
        BinaryOperation::Divide => divide(&left, &right)?,
    };
    format_complex(result)
}

fn multiply([a, b]: &[BigRational; 2], [c, d]: &[BigRational; 2]) -> [BigRational; 2] {
    [a * c - b * d, a * d + b * c]
}

fn divide(
    [a, b]: &[BigRational; 2],
    [c, d]: &[BigRational; 2],
) -> Result<[BigRational; 2], ComplexError> {
    let denominator = c * c + d * d;
    if denominator.numer().sign() == Sign::NoSign {
        return Err(ComplexError::ZeroDivisor);
    }
    Ok([
        (a * c + b * d) / &denominator,
        (b * c - a * d) / denominator,
    ])
}

fn integer_power(base: [BigRational; 2], exponent: i32) -> Result<[BigRational; 2], ComplexError> {
    let mut result = [
        BigRational::from_integer(1.into()),
        BigRational::from_integer(0.into()),
    ];
    let mut base = if exponent < 0 {
        bounded_components(divide(&result, &base)?)?
    } else {
        base
    };
    let mut exponent = exponent.unsigned_abs();
    while exponent > 0 {
        if exponent & 1 != 0 {
            result = bounded_components(multiply(&result, &base))?;
        }
        exponent >>= 1;
        if exponent > 0 {
            base = bounded_components(multiply(&base, &base))?;
        }
    }
    Ok(result)
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

fn bounded_components(values: [BigRational; 2]) -> Result<[BigRational; 2], ComplexError> {
    for value in &values {
        ensure_exact_size(value).map_err(ComplexError::Evaluation)?;
    }
    Ok(values)
}

fn format_complex(values: [BigRational; 2]) -> Result<String, ComplexError> {
    let [real, imaginary] = bounded_components(values)?;
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

    #[test]
    fn raises_complex_numbers_to_bounded_signed_integer_powers() {
        for (query, expected) in [
            ("乘方 1,1 @ 5", "-4 - 4i"),
            ("pow 1,1 @ -2", "-(1/2)i"),
            ("乘方 0,0 @ 0", "1"),
            ("乘方 0,1 @ 1024", "1"),
            ("乘方 0,1 @ -1023", "i"),
        ] {
            assert_eq!(calculate_complex(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            ("乘方 0,0 @ -1", ComplexError::ZeroDivisor),
            ("乘方 1,1 @ 1025", ComplexError::Exponent),
            ("乘方 1,1 @ 1.5", ComplexError::Exponent),
            (
                "乘方 2**1024,0 @ 8",
                ComplexError::Evaluation(FractionError::ExactRange),
            ),
        ] {
            assert_eq!(calculate_complex(query), Err(error), "{query}");
        }
    }
}
