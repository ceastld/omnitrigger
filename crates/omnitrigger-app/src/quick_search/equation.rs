use num_bigint::{BigInt, Sign};
use num_rational::BigRational;

use super::calculator::{normalize, QUERY_LIMIT};
use super::fraction::{calculate_fraction, ensure_exact_size, FractionError};
use super::operators::split_top_level_commas;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EquationError {
    Coefficients,
    Evaluation(FractionError),
}

impl EquationError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Coefficients | Self::Evaluation(FractionError::Invalid) => {
                "按 a, b, c 输入 ax² + bx + c = 0 的三个系数"
            }
            Self::Evaluation(error) => error.message(),
        }
    }
}

pub(super) fn solve_equation(query: &str) -> Result<String, EquationError> {
    if query.chars().take(QUERY_LIMIT + 1).count() > QUERY_LIMIT {
        return Err(EquationError::Evaluation(FractionError::TooLong));
    }
    let input = normalize(query);
    let mut terms = split_top_level_commas(&input);
    let (Some(a), Some(b), Some(c), None) =
        (terms.next(), terms.next(), terms.next(), terms.next())
    else {
        return Err(EquationError::Coefficients);
    };
    if [a, b, c].into_iter().any(|term| term.trim().is_empty()) {
        return Err(EquationError::Coefficients);
    }
    let coefficient = |term| calculate_fraction(term).map_err(EquationError::Evaluation);
    let (a, b, c) = (coefficient(a)?, coefficient(b)?, coefficient(c)?);
    if a.numer().sign() == Sign::NoSign {
        if b.numer().sign() != Sign::NoSign {
            let root = -c / b;
            ensure_exact_size(&root).map_err(EquationError::Evaluation)?;
            return Ok(format!("x = {root}"));
        }
        return Ok(if c.numer().sign() == Sign::NoSign {
            "任意 x 都是解".to_owned()
        } else {
            "无解".to_owned()
        });
    }

    // 配方后直接计算有理数和根式，有限小数系数也能精确判定重根。
    let center = -b / (&a * BigInt::from(2));
    let distance_squared = &center * &center - c / a;
    for value in [&center, &distance_squared] {
        ensure_exact_size(value).map_err(EquationError::Evaluation)?;
    }
    let sign = distance_squared.numer().sign();
    if sign == Sign::NoSign {
        return Ok(format!("x = {center}（重根）"));
    }
    let numerator = distance_squared.numer().magnitude();
    let denominator = distance_squared.denom().magnitude();
    let top = numerator.sqrt();
    let bottom = denominator.sqrt();
    let exact = &top * &top == *numerator && &bottom * &bottom == *denominator;
    if sign == Sign::Plus && exact {
        let distance = BigRational::new(top.into(), bottom.into());
        let first = &center - &distance;
        let second = center + distance;
        for root in [&first, &second] {
            ensure_exact_size(root).map_err(EquationError::Evaluation)?;
        }
        return Ok(format!("x₁ = {first} · x₂ = {second}"));
    }

    let distance = if exact {
        let root = BigRational::new(top.into(), bottom.into());
        if root == BigRational::from_integer(BigInt::from(1)) {
            "i".to_owned()
        } else if root.is_integer() {
            format!("{root}i")
        } else {
            format!("({root})i")
        }
    } else {
        let distance = distance_squared.to_string();
        let distance = distance.trim_start_matches('-');
        let unit = if sign == Sign::Minus { "i" } else { "" };
        if distance_squared.is_integer() {
            format!("{unit}√{distance}")
        } else {
            format!("{unit}√({distance})")
        }
    };
    if center.numer().sign() == Sign::NoSign {
        Ok(format!("x = ±{distance}"))
    } else {
        Ok(format!("x = {center} ± {distance}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solves_linear_and_quadratic_equations_with_exact_coefficients() {
        for (query, expected) in [
            ("1, -3, 2", "x₁ = 1 · x₂ = 2"),
            ("１，−２，−１", "x = 1 ± √2"),
            ("1/2, -5/4, 3/4", "x₁ = 1 · x₂ = 3/2"),
            ("comb(3, 2), -5, 2", "x₁ = 2/3 · x₂ = 1"),
            ("0.01, 0.2, 1", "x = -10（重根）"),
            ("1, 0, 1", "x = ±i"),
            ("1, 0, 1/4", "x = ±(1/2)i"),
            ("1, -2, 3", "x = 1 ± i√2"),
            ("2, 0, -1", "x = ±√(1/2)"),
            ("0, 3, -2", "x = 2/3"),
            ("0, 0, 1", "无解"),
            ("0, 0, 0", "任意 x 都是解"),
        ] {
            assert_eq!(solve_equation(query).as_deref(), Ok(expected), "{query}");
        }
        for query in ["1, 2", "1, , 2", "1, 2, 3, 4"] {
            assert_eq!(solve_equation(query), Err(EquationError::Coefficients));
        }
        assert_eq!(
            solve_equation("1, 1/0, 2"),
            Err(EquationError::Evaluation(FractionError::ZeroDenominator))
        );
    }
}
