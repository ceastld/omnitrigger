use std::cmp::Ordering;

use num_bigint::{BigInt, Sign};
use num_rational::BigRational;

use super::calculator::{normalize, QUERY_LIMIT};
use super::fraction::{
    calculate_fraction, ensure_exact_size, exact_abs_square_root, format_square_root, FractionError,
};
use super::operators::{split_top_level_commas, NumberComparison};
use super::plane::{parse_point, PointError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EquationError {
    Coefficients,
    RootArguments,
    IntersectionArguments,
    ThreePointArguments,
    VertexFormArguments,
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
            Self::RootArguments => "填写两个有理数根，可再填非零的二次项系数",
            Self::IntersectionArguments => {
                "分别填写两条曲线的 a, b, c 三个系数，两组用分号或换行隔开"
            }
            Self::ThreePointArguments => {
                "填写三个横坐标互不相同的点，每个点写 x, y，各点用分号或换行隔开"
            }
            Self::VertexFormArguments => "按顶点横坐标、顶点纵坐标、二次项系数的顺序填写",
            Self::NonQuadratic => "二次项系数 a 须非零",
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
            "顶点" | "vertex" => return describe_quadratic(arguments.trim_start(), false),
            "判别式" | "discriminant" => return quadratic_discriminant(arguments.trim_start()),
            "韦达" | "vieta" => return quadratic_root_sum_and_product(arguments.trim_start()),
            "由根" | "from-roots" => return quadratic_from_roots(arguments.trim_start()),
            "交点" | "intersections" => return polynomial_intersections(arguments.trim_start()),
            "过三点" | "through-points" => {
                return quadratic_through_points(arguments.trim_start())
            }
            "由顶点" | "from-vertex" => return quadratic_from_vertex(arguments.trim_start()),
            "配方" | "complete-the-square" => {
                return describe_quadratic(arguments.trim_start(), true);
            }
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

fn quadratic_coefficients(input: &str) -> Result<[BigRational; 3], EquationError> {
    let coefficients = coefficient_row(input)?;
    if coefficients[0].numer().sign() == Sign::NoSign {
        return Err(EquationError::NonQuadratic);
    }
    Ok(coefficients)
}

fn quadratic_discriminant(input: &str) -> Result<String, EquationError> {
    let [a, b, c] = quadratic_coefficients(input)?;
    let discriminant = &b * &b - a * c * BigInt::from(4u8);
    ensure_exact_size(&discriminant).map_err(EquationError::Evaluation)?;
    Ok(discriminant.to_string())
}

fn quadratic_root_sum_and_product(input: &str) -> Result<String, EquationError> {
    let [a, b, c] = quadratic_coefficients(input)?;
    let sum = -b / &a;
    let product = c / a;
    for value in [&sum, &product] {
        ensure_exact_size(value).map_err(EquationError::Evaluation)?;
    }
    Ok(format!("根和 {sum}\n根积 {product}"))
}

fn quadratic_from_roots(input: &str) -> Result<String, EquationError> {
    let values: Vec<_> = split_top_level_commas(input).collect();
    let (first, second, leading) = match values.as_slice() {
        [first, second] => (*first, *second, "1"),
        [first, second, leading] => (*first, *second, *leading),
        _ => return Err(EquationError::RootArguments),
    };
    let parse = |value: &str| {
        calculate_fraction(value).map_err(|error| match error {
            FractionError::Invalid | FractionError::Incomplete => EquationError::RootArguments,
            _ => EquationError::Evaluation(error),
        })
    };
    let first = parse(first)?;
    let second = parse(second)?;
    let a = parse(leading)?;
    if a.numer().sign() == Sign::NoSign {
        return Err(EquationError::NonQuadratic);
    }
    let b = -&a * (&first + &second);
    let c = &a * first * second;
    format_quadratic_coefficients(a, b, c)
}

fn quadratic_through_points(input: &str) -> Result<String, EquationError> {
    let mut points = input.split([';', '；', '\n']);
    let (Some(first), Some(second), Some(third), None) =
        (points.next(), points.next(), points.next(), points.next())
    else {
        return Err(EquationError::ThreePointArguments);
    };
    let parse = |point| {
        parse_point(point).map_err(|error| match error {
            PointError::Arguments
            | PointError::Evaluation(FractionError::Invalid | FractionError::Incomplete) => {
                EquationError::ThreePointArguments
            }
            PointError::Evaluation(error) => EquationError::Evaluation(error),
        })
    };
    let [x0, y0] = parse(first)?;
    let [x1, y1] = parse(second)?;
    let [x2, y2] = parse(third)?;
    if x0 == x1 || x1 == x2 || x0 == x2 {
        return Err(EquationError::ThreePointArguments);
    }
    let first_slope = (&y1 - &y0) / (&x1 - &x0);
    let second_slope = (y2 - y1) / (&x2 - &x1);
    let a = (second_slope - &first_slope) / (x2 - &x0);
    let b = first_slope - &a * (&x0 + x1);
    let c = y0 - (&a * &x0 + &b) * x0;
    format_quadratic_coefficients(a, b, c)
}

fn quadratic_from_vertex(input: &str) -> Result<String, EquationError> {
    let [h, k, a] = coefficient_row(input).map_err(|error| match error {
        EquationError::Coefficients
        | EquationError::Evaluation(FractionError::Invalid | FractionError::Incomplete) => {
            EquationError::VertexFormArguments
        }
        _ => error,
    })?;
    if a.numer().sign() == Sign::NoSign {
        return Err(EquationError::NonQuadratic);
    }
    let b = -&a * &h * BigInt::from(2u8);
    let c = &a * &h * h + k;
    format_quadratic_coefficients(a, b, c)
}

fn format_quadratic_coefficients(
    a: BigRational,
    b: BigRational,
    c: BigRational,
) -> Result<String, EquationError> {
    for value in [&a, &b, &c] {
        ensure_exact_size(value).map_err(EquationError::Evaluation)?;
    }
    Ok(format!("{a},{b},{c}"))
}

fn polynomial_intersections(input: &str) -> Result<String, EquationError> {
    let (first, second) = input
        .split_once([';', '；', '\n'])
        .ok_or(EquationError::IntersectionArguments)?;
    let parse = |coefficients| match coefficient_row(coefficients) {
        Err(
            EquationError::Coefficients
            | EquationError::Evaluation(FractionError::Invalid | FractionError::Incomplete),
        ) => Err(EquationError::IntersectionArguments),
        result => result,
    };
    let [a, b, c] = parse(first)?;
    let [other_a, other_b, other_c] = parse(second)?;
    let (diff_a, diff_b, diff_c) = (&a - other_a, &b - other_b, &c - other_c);
    let point = |x: &BigRational| {
        let y = (&a * x + &b) * x + &c;
        for value in [x, &y] {
            ensure_exact_size(value).map_err(EquationError::Evaluation)?;
        }
        Ok(format!("({x}, {y})"))
    };
    if diff_a.numer().sign() == Sign::NoSign {
        if diff_b.numer().sign() == Sign::NoSign {
            return Ok(if diff_c.numer().sign() == Sign::NoSign {
                "两条曲线重合"
            } else {
                "没有实数交点"
            }
            .to_owned());
        }
        return point(&(-diff_c / diff_b));
    }
    let (x, dx_squared) = quadratic_center(&diff_a, diff_b.clone(), diff_c.clone());
    match dx_squared.numer().sign() {
        Sign::Minus => return Ok("没有实数交点".to_owned()),
        Sign::NoSign => return point(&x),
        Sign::Plus => {}
    }
    // 在交点处用差式化去 x²，纵坐标可按 x 的一次式计算。
    let slope = &b - &a * diff_b / &diff_a;
    let y = &slope * &x + &c - &a * diff_c / &diff_a;
    let dy_squared = &slope * &slope * &dx_squared;
    for value in [&x, &dx_squared, &y, &dy_squared] {
        ensure_exact_size(value).map_err(EquationError::Evaluation)?;
    }
    let [left_x, right_x] =
        shifted_real_roots(&x, &dx_squared).map_err(EquationError::Evaluation)?;
    let mut ys = shifted_real_roots(&y, &dy_squared).map_err(EquationError::Evaluation)?;
    if slope.numer().sign() == Sign::Minus {
        ys.reverse();
    }
    let [left_y, right_y] = ys;
    Ok(format!("({left_x}, {left_y})\n({right_x}, {right_y})"))
}

fn describe_quadratic(input: &str, complete_square: bool) -> Result<String, EquationError> {
    let [a, b, c] = quadratic_coefficients(input)?;
    let extremum = if a.numer().sign() == Sign::Plus {
        "最小值"
    } else {
        "最大值"
    };
    let (x, distance_squared) = quadratic_center(&a, b, c);
    let y = -&a * distance_squared;
    for value in [&x, &y] {
        ensure_exact_size(value).map_err(EquationError::Evaluation)?;
    }
    if !complete_square {
        return Ok(format!("x = {x} · y = {y}（{extremum}）"));
    }
    let square = if x.numer().sign() == Sign::NoSign {
        "x²".to_owned()
    } else {
        format!("(x{})²", signed_offset(&-x))
    };
    let unit = BigRational::from_integer(BigInt::from(1u8));
    let factor = if a == unit {
        String::new()
    } else if a == -unit {
        "-".to_owned()
    } else {
        format!("{a}·")
    };
    Ok(format!("{factor}{square}{}", signed_offset(&y)))
}

fn signed_offset(value: &BigRational) -> String {
    match value.numer().sign() {
        Sign::Plus => format!(" + {value}"),
        Sign::Minus => format!(" - {}", -value),
        Sign::NoSign => String::new(),
    }
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
            let [left, right] =
                shifted_real_roots(&center, &squared).map_err(EquationError::Evaluation)?;
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

fn shifted_real_roots(
    center: &BigRational,
    squared: &BigRational,
) -> Result<[String; 2], FractionError> {
    if let Some(distance) = exact_abs_square_root(squared) {
        return exact_real_roots(center, &distance);
    }
    let distance = format_square_root(squared);
    Ok(if center.numer().sign() == Sign::NoSign {
        [format!("-{distance}"), distance]
    } else {
        [
            format!("{center} - {distance}"),
            format!("{center} + {distance}"),
        ]
    })
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
    fn recovers_quadratic_coefficients_from_a_fractional_vertex_and_curvature() {
        let coefficients = solve_equation("由顶点 3/2,-1/4,2").unwrap();
        assert_eq!(coefficients, "2,-6,17/4");
        assert_eq!(
            solve_equation(&format!("顶点 {coefficients}")).as_deref(),
            Ok("x = 3/2 · y = -1/4（最小值）"),
        );
        assert_eq!(
            solve_equation("from-vertex 1,2,0"),
            Err(EquationError::NonQuadratic)
        );
        assert_eq!(
            solve_equation("由顶点 1,2"),
            Err(EquationError::VertexFormArguments)
        );
    }

    #[test]
    fn recovers_a_polynomial_through_three_distinct_abscissas_exactly() {
        assert_eq!(
            solve_equation("过三点 0,3/4 ; 1/2,3/8 ; 2,3/4").as_deref(),
            Ok("1/2,-1,3/4"),
        );
        assert_eq!(
            solve_equation("through-points 3,7 ; 1,3 ; 2,5").as_deref(),
            Ok("0,2,1"),
        );
        assert_eq!(
            solve_equation("过三点 1,2 ; 1,3 ; 2,4"),
            Err(EquationError::ThreePointArguments),
        );
    }

    #[test]
    fn intersects_quadratic_and_linear_graphs_with_exact_coordinates() {
        for (input, expected) in [
            ("交点 1,0,0 ; 0,-1,2", "(-2, 4)\n(1, 1)"),
            ("交点 1,0,0 ; 0,0,2", "(-√2, 2)\n(√2, 2)"),
            ("交点 0,2,1 ; 0,-1,2", "(1/3, 5/3)"),
            ("交点 1,0,0 ; 0,0,0", "(0, 0)"),
            ("交点 1,0,0 ; 0,0,-2", "没有实数交点"),
            ("交点 1,0,0 ; 1,0,0", "两条曲线重合"),
        ] {
            assert_eq!(solve_equation(input).as_deref(), Ok(expected), "{input}");
        }
        assert_eq!(
            solve_equation("交点 1,0,0 ; 0,1"),
            Err(EquationError::IntersectionArguments)
        );
    }

    #[test]
    fn builds_exact_quadratic_coefficients_from_two_roots() {
        assert_eq!(
            solve_equation("由根 1/2,3/2,-2").as_deref(),
            Ok("-2,4,-3/2")
        );
        assert_eq!(solve_equation("from-roots -2,-2").as_deref(), Ok("1,4,4"));
        assert_eq!(
            solve_equation("由根 1,2,0"),
            Err(EquationError::NonQuadratic)
        );
        assert_eq!(
            solve_equation("由根 1,2,3,4"),
            Err(EquationError::RootArguments)
        );
        assert_eq!(
            solve_equation("由根 1,无效"),
            Err(EquationError::RootArguments)
        );
    }

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
    fn rewrites_a_quadratic_in_exact_completed_square_form() {
        for (query, expected) in [
            ("配方 2,-3,1", "2·(x - 3/4)² - 1/8"),
            ("complete-the-square -1,0,0", "-x²"),
            ("配方 1,2,1", "(x + 1)²"),
            ("配方 1,0,5", "x² + 5"),
            ("配方 -1/2,3/2,0", "-1/2·(x - 3/2)² + 9/8"),
        ] {
            assert_eq!(solve_equation(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            solve_equation("配方 0,2,1"),
            Err(EquationError::NonQuadratic)
        );
    }

    #[test]
    fn calculates_the_discriminant_and_vieta_values_exactly() {
        for (query, expected) in [
            ("判别式 2,-3,1", "1"),
            ("discriminant 1/2,1/2,1/8", "0"),
            ("判别式 1,0,1", "-4"),
            ("韦达 2,-3,1", "根和 3/2\n根积 1/2"),
            ("vieta 1,0,1", "根和 0\n根积 1"),
            ("韦达 -1/2,3/2,1/2", "根和 3\n根积 -1"),
        ] {
            assert_eq!(solve_equation(query).as_deref(), Ok(expected), "{query}");
        }
        for query in ["判别式 0,1,2", "韦达 0,1,2"] {
            assert_eq!(solve_equation(query), Err(EquationError::NonQuadratic));
        }
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
