use std::cmp::Ordering;

use num_bigint::{BigInt, Sign};
use num_rational::BigRational;

use super::calculator::{normalize, QUERY_LIMIT};
use super::fraction::{
    calculate_fraction, ensure_exact_size, exact_abs_square_root, format_square_root, FractionError,
};
use super::operators::{split_top_level_commas, NumberComparison};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EquationError {
    Coefficients,
    InequalityArguments,
    NonQuadratic,
    SystemRows,
    Evaluation(FractionError),
}

impl EquationError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Coefficients | Self::Evaluation(FractionError::Invalid) => {
                "按 a, b, c 输入 ax² + bx + c = 0 的三个系数"
            }
            Self::InequalityArguments => "先填写 >、>=、< 或 <=，再填写 a, b, c",
            Self::NonQuadratic => "求抛物线顶点时，二次项系数 a 须非零",
            Self::SystemRows => "每行按 x 系数、y 系数、右侧常数输入，两行用分号隔开",
            Self::Evaluation(error) => error.message(),
        }
    }
}

pub(super) fn solve_equation(query: &str) -> Result<String, EquationError> {
    if query.chars().take(QUERY_LIMIT + 1).count() > QUERY_LIMIT {
        return Err(EquationError::Evaluation(FractionError::TooLong));
    }
    let input = normalize(query);
    if let Some((operation, arguments)) = input.trim().split_once(char::is_whitespace) {
        match operation.to_ascii_lowercase().as_str() {
            "不等式" | "inequality" => return solve_inequality(arguments.trim_start()),
            "顶点" | "vertex" => return quadratic_vertex(arguments.trim_start()),
            _ => {}
        }
    }
    let [a, b, c] = coefficient_row(&input)?;
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

    let (center, distance_squared) = quadratic_center(&a, b, c);
    format_centered_roots(center, distance_squared, "x").map_err(EquationError::Evaluation)
}

fn quadratic_center(a: &BigRational, b: BigRational, c: BigRational) -> (BigRational, BigRational) {
    // 配方后直接计算有理数和根式，有限小数系数也能精确判定重根。
    let center = -b / (a * BigInt::from(2));
    let distance_squared = &center * &center - c / a;
    (center, distance_squared)
}

fn quadratic_vertex(input: &str) -> Result<String, EquationError> {
    let [a, b, c] = coefficient_row(input)?;
    let extremum = match a.numer().sign() {
        Sign::Plus => "最小值",
        Sign::Minus => "最大值",
        Sign::NoSign => return Err(EquationError::NonQuadratic),
    };
    let (x, distance_squared) = quadratic_center(&a, b, c);
    let y = -a * distance_squared;
    for value in [&x, &y] {
        ensure_exact_size(value).map_err(EquationError::Evaluation)?;
    }
    Ok(format!("x = {x} · y = {y}（{extremum}）"))
}

fn solve_inequality(input: &str) -> Result<String, EquationError> {
    let (operator, coefficients) = input
        .split_once(char::is_whitespace)
        .ok_or(EquationError::InequalityArguments)?;
    let comparison = match NumberComparison::parse(operator) {
        Some(
            comparison @ (NumberComparison::Less
            | NumberComparison::LessOrEqual
            | NumberComparison::Greater
            | NumberComparison::GreaterOrEqual),
        ) => comparison,
        _ => return Err(EquationError::InequalityArguments),
    };
    let [a, b, c] = coefficient_row(coefficients).map_err(|error| match error {
        EquationError::Coefficients => EquationError::InequalityArguments,
        _ => error,
    })?;
    let zero = BigRational::default();
    let inclusive = comparison.accepts(Ordering::Equal);
    let (opening, closing) = if inclusive { ('[', ']') } else { ('(', ')') };
    if a == zero {
        if b == zero {
            return Ok(real_or_empty(comparison.accepts(c.cmp(&zero))));
        }
        let right_half = comparison.accepts(b.cmp(&zero));
        let root = -c / b;
        ensure_exact_size(&root).map_err(EquationError::Evaluation)?;
        return Ok(if right_half {
            format!("{opening}{root}, ∞)")
        } else {
            format!("(-∞, {root}{closing}")
        });
    }
    let outside = comparison.accepts(a.cmp(&zero));
    let (center, squared) = quadratic_center(&a, b, c);
    for value in [&center, &squared] {
        ensure_exact_size(value).map_err(EquationError::Evaluation)?;
    }
    match squared.numer().sign() {
        Sign::Minus => Ok(real_or_empty(outside)),
        Sign::NoSign => match (outside, inclusive) {
            (true, true) => Ok(real_or_empty(true)),
            (true, false) => Ok(format!("(-∞, {center}) ∪ ({center}, ∞)")),
            (false, true) => Ok(format!("{{{center}}}")),
            (false, false) => Ok(real_or_empty(false)),
        },
        Sign::Plus => {
            let [left, right] = if let Some(distance) = exact_abs_square_root(&squared) {
                exact_real_roots(&center, &distance).map_err(EquationError::Evaluation)?
            } else {
                let distance = format_square_root(&squared);
                if center == zero {
                    [format!("-{distance}"), distance]
                } else {
                    [
                        format!("{center} - {distance}"),
                        format!("{center} + {distance}"),
                    ]
                }
            };
            Ok(if outside {
                format!("(-∞, {left}{closing} ∪ {opening}{right}, ∞)")
            } else {
                format!("{opening}{left}, {right}{closing}")
            })
        }
    }
}

fn real_or_empty(all: bool) -> String {
    let solution = if all { "ℝ" } else { "∅" };
    solution.to_owned()
}

fn exact_real_roots(
    center: &BigRational,
    distance: &BigRational,
) -> Result<[String; 2], FractionError> {
    let left = center - distance;
    let right = center + distance;
    for root in [&left, &right] {
        ensure_exact_size(root)?;
    }
    Ok([left.to_string(), right.to_string()])
}

pub(super) fn format_centered_roots(
    center: BigRational,
    distance_squared: BigRational,
    variable: &str,
) -> Result<String, FractionError> {
    for value in [&center, &distance_squared] {
        ensure_exact_size(value)?;
    }
    let sign = distance_squared.numer().sign();
    if sign == Sign::NoSign {
        return Ok(format!("{variable} = {center}（重根）"));
    }
    let exact_distance = exact_abs_square_root(&distance_squared);
    if let (Sign::Plus, Some(distance)) = (sign, &exact_distance) {
        let [first, second] = exact_real_roots(&center, distance)?;
        return Ok(format!("{variable}₁ = {first} · {variable}₂ = {second}"));
    }

    let distance = if let Some(root) = exact_distance {
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
        Ok(format!("{variable} = ±{distance}"))
    } else {
        Ok(format!("{variable} = {center} ± {distance}"))
    }
}

fn coefficient_row(input: &str) -> Result<[BigRational; 3], EquationError> {
    let mut terms = split_top_level_commas(input);
    let (Some(a), Some(b), Some(c), None) =
        (terms.next(), terms.next(), terms.next(), terms.next())
    else {
        return Err(EquationError::Coefficients);
    };
    if [a, b, c].into_iter().any(|term| term.trim().is_empty()) {
        return Err(EquationError::Coefficients);
    }
    let coefficient = |term| calculate_fraction(term).map_err(EquationError::Evaluation);
    Ok([coefficient(a)?, coefficient(b)?, coefficient(c)?])
}

pub(super) fn solve_linear_system(query: &str) -> Result<String, EquationError> {
    if query.chars().take(QUERY_LIMIT + 1).count() > QUERY_LIMIT {
        return Err(EquationError::Evaluation(FractionError::TooLong));
    }
    let input = normalize(query);
    let (first, second) = input
        .split_once([';', '\n'])
        .ok_or(EquationError::SystemRows)?;
    let row = |text| match coefficient_row(text) {
        Err(
            EquationError::Coefficients
            | EquationError::Evaluation(FractionError::Invalid | FractionError::Incomplete),
        ) => Err(EquationError::SystemRows),
        result => result,
    };
    let [a, b, c] = row(first)?;
    let [d, e, f] = row(second)?;
    let det = &a * &e - &b * &d;
    let numer_x = &c * &e - &b * &f;
    let numer_y = &a * &f - &c * &d;
    let is_zero = |value: &BigRational| value.numer().sign() == Sign::NoSign;
    if is_zero(&det) {
        // 系数全零的行须单独检查，右侧常数在该行不会通过二阶行列式体现。
        let inconsistent_row = [(&a, &b, &c), (&d, &e, &f)]
            .into_iter()
            .any(|(x, y, rhs)| is_zero(x) && is_zero(y) && !is_zero(rhs));
        let inconsistent = inconsistent_row || !is_zero(&numer_x) || !is_zero(&numer_y);
        return Ok(if inconsistent {
            "无解"
        } else {
            "无穷多解"
        }
        .to_owned());
    }
    for value in [&det, &numer_x, &numer_y] {
        ensure_exact_size(value).map_err(EquationError::Evaluation)?;
    }
    let x = numer_x / &det;
    let y = numer_y / det;
    for value in [&x, &y] {
        ensure_exact_size(value).map_err(EquationError::Evaluation)?;
    }
    Ok(format!("x = {x} · y = {y}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solves_exact_linear_systems_including_singular_cases() {
        for (query, expected) in [
            ("2,1,5; 1,-1,1", "x = 2 · y = 1"),
            ("1/2,1,3\r\n1,-1,0", "x = 2 · y = 2"),
            ("２，１，５；１，−１，１", "x = 2 · y = 1"),
            ("comb(2,1),1,5; 1,-1,1", "x = 2 · y = 1"),
            ("1,0,0; 0,1,-2", "x = 0 · y = -2"),
            ("1,1,2; 2,2,4", "无穷多解"),
            ("0,0,0; 0,0,0", "无穷多解"),
            ("1,1,2; 2,2,5", "无解"),
            ("0,0,0; 0,0,1", "无解"),
        ] {
            assert_eq!(
                solve_linear_system(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert_eq!(solve_linear_system("1,2,3"), Err(EquationError::SystemRows));
        assert_eq!(
            solve_linear_system("1,2; 4,5,6"),
            Err(EquationError::SystemRows)
        );
    }

    #[test]
    fn finds_the_exact_vertex_and_extremum_of_a_quadratic_function() {
        for (query, expected) in [
            ("顶点 2,-4,3", "x = 1 · y = 1（最小值）"),
            ("VERTEX -1/2,3/2,0", "x = 3/2 · y = 9/8（最大值）"),
            ("顶点 1,0,-5", "x = 0 · y = -5（最小值）"),
        ] {
            assert_eq!(solve_equation(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            solve_equation("顶点 0,1,2"),
            Err(EquationError::NonQuadratic)
        );
    }

    #[test]
    fn solves_exact_quadratic_linear_and_constant_inequalities() {
        for (query, expected) in [
            ("不等式 >= 1,-3,2", "(-∞, 1] ∪ [2, ∞)"),
            ("inequality < 1,-3,2", "(1, 2)"),
            ("不等式 ≤ -1,3,-2", "(-∞, 1] ∪ [2, ∞)"),
            ("不等式 ≥ 1,0,-2", "(-∞, -√2] ∪ [√2, ∞)"),
            ("不等式 < 1,-2,-1", "(1 - √2, 1 + √2)"),
            ("不等式 > 1,-2,1", "(-∞, 1) ∪ (1, ∞)"),
            ("不等式 <= 1,-2,1", "{1}"),
            ("不等式 < 1,-2,1", "∅"),
            ("不等式 >= 1,-2,1", "ℝ"),
            ("不等式 > 1,0,1", "ℝ"),
            ("不等式 >= -1,0,-1", "∅"),
            ("不等式 ≤ 0,-2,1", "[1/2, ∞)"),
            ("不等式 < 0,3,-1", "(-∞, 1/3)"),
            ("不等式 ≥ 0,0,0", "ℝ"),
            ("不等式 > 0,0,0", "∅"),
        ] {
            assert_eq!(solve_equation(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            solve_equation("不等式 = 1,-3,2"),
            Err(EquationError::InequalityArguments)
        );
    }

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
