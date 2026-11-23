use num_bigint::{BigInt, BigUint, Sign};
use num_rational::BigRational;

use super::{
    calculator::{format_result, normalize},
    fraction::{
        calculate_fraction, ensure_exact_size, exact_abs_square_root, format_square_root,
        FractionError,
    },
    operators::{split_top_level_commas, visit_additive_terms},
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
            Self::Incomplete => "填写复数，两组复数用 @ 隔开",
            Self::Operation => "可用加法、减法、乘法、除法、模、共轭、乘方、平方根和极坐标",
            Self::Components => "填写 a,b 或 a+bi 形式的复数，例如 3,4 或 3+4i",
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
            return complex_modulus(&components(input)?);
        }
        "极坐标" | "polar" => return complex_polar(components(input)?),
        "共轭" | "conjugate" => {
            let [real, imaginary] = components(input)?;
            return format_complex([real, -imaginary]);
        }
        "平方根" | "sqrt" | "square-roots" => return square_roots(components(input)?),
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
    right: &[BigRational; 2],
) -> Result<[BigRational; 2], ComplexError> {
    let [c, d] = right;
    let denominator = squared_modulus(right);
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

fn squared_modulus([real, imaginary]: &[BigRational; 2]) -> BigRational {
    real * real + imaginary * imaginary
}

fn complex_modulus(value: &[BigRational; 2]) -> Result<String, ComplexError> {
    let squared = squared_modulus(value);
    ensure_exact_size(&squared).map_err(ComplexError::Evaluation)?;
    Ok(format_square_root(&squared))
}

fn complex_polar(value: [BigRational; 2]) -> Result<String, ComplexError> {
    let [real, imaginary] = &value;
    let real_sign = real.numer().sign();
    let imaginary_sign = imaginary.numer().sign();
    if real_sign == Sign::NoSign && imaginary_sign == Sign::NoSign {
        return Ok("模 0 · 辐角未定义".to_owned());
    }
    let modulus = complex_modulus(&value)?;
    let [real_size, imaginary_size] = [real, imaginary].map(|coordinate| {
        if coordinate.numer().sign() == Sign::Minus {
            -coordinate
        } else {
            (*coordinate).clone()
        }
    });
    let direction = if imaginary_sign == Sign::Minus { -1 } else { 1 };
    let angle = if imaginary_sign == Sign::NoSign {
        if real_sign == Sign::Minus { "180" } else { "0" }.to_owned()
    } else if real_sign == Sign::NoSign {
        (90 * direction).to_string()
    } else if real_size == imaginary_size {
        let angle = if real_sign == Sign::Minus { 135 } else { 45 };
        (angle * direction).to_string()
    } else {
        let scale = real_size.max(imaginary_size);
        let x = relative_coordinate(real, &scale);
        let y = relative_coordinate(imaginary, &scale);
        let degrees = format_result(y.atan2(x).to_degrees())
            .map_err(|_| ComplexError::Evaluation(FractionError::ExactRange))?;
        format!("≈ {degrees}")
    };
    Ok(format!("模 {modulus} · 辐角 {angle}°"))
}

fn relative_coordinate(value: &BigRational, scale: &BigRational) -> f64 {
    // 先在有理数内归一化，再分别取分子和分母的高位进入浮点计算。
    let ratio = value / scale;
    let leading_bits = |value: &BigUint| {
        let shift = value.bits().saturating_sub(53);
        let bits = u64::try_from(value >> shift as usize).expect("尾数至多为 53 位");
        (bits as f64, shift as i32)
    };
    let (numerator, numerator_shift) = leading_bits(ratio.numer().magnitude());
    let (denominator, denominator_shift) = leading_bits(ratio.denom().magnitude());
    let exponent = numerator_shift - denominator_shift;
    let mantissa = numerator / denominator;
    let magnitude = if exponent < f64::MIN_EXP - 1 {
        (mantissa * 2f64.powi(exponent - (f64::MIN_EXP - 1))) * f64::MIN_POSITIVE
    } else {
        mantissa * 2f64.powi(exponent)
    };
    if value.numer().sign() == Sign::Minus {
        -magnitude
    } else {
        magnitude
    }
}

fn square_roots(value: [BigRational; 2]) -> Result<String, ComplexError> {
    let modulus_squared = squared_modulus(&value);
    let [real, imaginary] = value;
    ensure_exact_size(&modulus_squared).map_err(ComplexError::Evaluation)?;
    if modulus_squared.numer().sign() == Sign::NoSign {
        return Ok("0".to_owned());
    }
    // 平方后比较实部与模，可得 u² = (|z| + a)/2，v² = (|z| - a)/2。
    let (root_real, root_imaginary) = if let Some(modulus) = exact_abs_square_root(&modulus_squared)
    {
        let two = BigInt::from(2u8);
        let [real_squared, imaginary_squared] =
            bounded_components([(&modulus + &real) / &two, (modulus - real) / two])?;
        let real = format_square_root(&real_squared);
        let imaginary = exact_abs_square_root(&imaginary_squared).map_or_else(
            || format!("{}i", format_square_root(&imaginary_squared)),
            |root| imaginary_term(&root),
        );
        (real, imaginary)
    } else {
        let magnitude = format_square_root(&modulus_squared);
        let (absolute, [add, subtract]) = if real.numer().sign() == Sign::Minus {
            (-real, ['-', '+'])
        } else {
            (real, ['+', '-'])
        };
        (
            format!("√(({magnitude} {add} {absolute})/2)"),
            format!("√(({magnitude} {subtract} {absolute})/2)i"),
        )
    };
    let root = format_complex_parts(
        root_real,
        root_imaginary,
        imaginary.numer().sign() == Sign::Minus,
    );
    // ± 同时作用于实部与虚部，保留两个互为相反数的根。
    Ok(if root.contains(' ') {
        format!("±({root})")
    } else {
        format!("±{root}")
    })
}

fn components(input: &str) -> Result<[BigRational; 2], ComplexError> {
    let mut terms = split_top_level_commas(input);
    match (terms.next(), terms.next(), terms.next()) {
        (Some(real), Some(imaginary), None) => Ok([
            calculate_fraction(real).map_err(ComplexError::Evaluation)?,
            calculate_fraction(imaginary).map_err(ComplexError::Evaluation)?,
        ]),
        (Some(expression), None, None) if !expression.trim().is_empty() => {
            parse_complex_expression(expression)
        }
        _ => Err(ComplexError::Components),
    }
}

fn parse_complex_expression(input: &str) -> Result<[BigRational; 2], ComplexError> {
    let [mut real, mut imaginary] = [BigRational::default(), BigRational::default()];
    visit_additive_terms(input, ComplexError::Components, |term| {
        let term = term.trim();
        let (value, amount) = if let Some(coefficient) = term.strip_suffix(['i', 'I']) {
            (&mut imaginary, parse_imaginary_coefficient(coefficient)?)
        } else {
            (
                &mut real,
                calculate_fraction(term).map_err(ComplexError::Evaluation)?,
            )
        };
        *value += amount;
        ensure_exact_size(value).map_err(ComplexError::Evaluation)
    })?;
    Ok([real, imaginary])
}

fn parse_imaginary_coefficient(input: &str) -> Result<BigRational, ComplexError> {
    let input = input.trim();
    match input {
        "" | "+" => Ok(BigRational::from_integer(1.into())),
        "-" => Ok(BigRational::from_integer((-1).into())),
        _ => {
            let scalar = input.strip_suffix(['*', '×']).unwrap_or(input);
            calculate_fraction(scalar).map_err(ComplexError::Evaluation)
        }
    }
}

fn bounded_components(values: [BigRational; 2]) -> Result<[BigRational; 2], ComplexError> {
    for value in &values {
        ensure_exact_size(value).map_err(ComplexError::Evaluation)?;
    }
    Ok(values)
}

fn format_complex(values: [BigRational; 2]) -> Result<String, ComplexError> {
    let [real, imaginary] = bounded_components(values)?;
    let negative = imaginary.numer().sign() == Sign::Minus;
    let absolute = if negative { -imaginary } else { imaginary };
    Ok(format_complex_parts(
        real.to_string(),
        imaginary_term(&absolute),
        negative,
    ))
}

fn imaginary_term(absolute: &BigRational) -> String {
    if absolute.numer().sign() == Sign::NoSign {
        String::new()
    } else if absolute.numer() == absolute.denom() {
        "i".to_owned()
    } else if absolute.is_integer() {
        format!("{absolute}i")
    } else {
        format!("({absolute})i")
    }
}

fn format_complex_parts(real: String, imaginary: String, negative: bool) -> String {
    if imaginary.is_empty() {
        return real;
    }
    match (real == "0", negative) {
        (true, true) => format!("-{imaginary}"),
        (true, false) => imaginary,
        (false, true) => format!("{real} - {imaginary}"),
        (false, false) => format!("{real} + {imaginary}"),
    }
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
    fn reports_exact_polar_axes_and_approximates_other_arguments_after_scaling() {
        for (query, expected) in [
            ("极坐标 1+i", "模 √2 · 辐角 45°"),
            ("极坐标 -1-i", "模 √2 · 辐角 -135°"),
            ("极坐标 -9", "模 9 · 辐角 180°"),
            ("polar 0,-1/2", "模 1/2 · 辐角 -90°"),
            ("极坐标 0", "模 0 · 辐角未定义"),
        ] {
            assert_eq!(calculate_complex(query).as_deref(), Ok(expected), "{query}");
        }
        let usual = calculate_complex("极坐标 3+4i").unwrap();
        assert!(usual.starts_with("模 5 · 辐角 ≈ 53.1301023"), "{usual}");
        let huge = calculate_complex("极坐标 3e1000,4e1000").unwrap();
        assert_eq!(
            huge.split_once(" · 辐角 ").map(|(_, angle)| angle),
            usual.split_once(" · 辐角 ").map(|(_, angle)| angle),
        );
        let near_axis = calculate_complex("极坐标 1,1e-100").unwrap();
        assert!(near_axis.contains("辐角 ≈ 5.729577") && near_axis.ends_with("e-99°"));
        let subnormal = calculate_complex("极坐标 1,1e-320").unwrap();
        assert!(subnormal.ends_with("e-319°"), "{subnormal}");
        let near_cut = calculate_complex("极坐标 -1,-1e-100").unwrap();
        assert!(near_cut.ends_with("辐角 ≈ -180°"), "{near_cut}");
    }

    #[test]
    fn accepts_cartesian_complex_input_and_can_reuse_formatted_rational_results() {
        for (query, expected) in [
            ("平方根 3 + 4i", "±(2 + i)"),
            ("乘法 (1/2)i @ 1 - i", "1/2 + (1/2)i"),
            ("共轭 1e-3 - 2e-4i", "1/1000 + (1/5000)i"),
            ("共轭 0x1e+4i", "30 - 4i"),
            ("加法 0x1e+4i @ 1.2e-3+3e-4i", "75003/2500 + (40003/10000)i"),
            ("加法 1 + i - i @ 1/2", "3/2"),
            ("模 -2*I", "2"),
            ("乘方 3+4i @ 2", "-7 + 24i"),
        ] {
            assert_eq!(calculate_complex(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            calculate_complex("模 (3 + 4i"),
            Err(ComplexError::Components)
        );
    }

    #[test]
    fn computes_both_complex_square_roots_in_exact_form() {
        for (query, expected) in [
            ("平方根 3,4", "±(2 + i)"),
            ("平方根 3,-4", "±(2 - i)"),
            ("sqrt 5/36,1/3", "±(1/2 + (1/3)i)"),
            ("平方根 -4,0", "±2i"),
            ("平方根 4,0", "±2"),
            ("平方根 0,0", "0"),
            ("平方根 0,1", "±(√(1/2) + √(1/2)i)"),
            ("平方根 1,1", "±(√((√2 + 1)/2) + √((√2 - 1)/2)i)"),
            ("平方根 -1,-1", "±(√((√2 - 1)/2) - √((√2 + 1)/2)i)"),
        ] {
            assert_eq!(calculate_complex(query).as_deref(), Ok(expected), "{query}");
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
