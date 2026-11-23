use std::cmp::Ordering;

use num_bigint::{BigInt, Sign};
use num_rational::BigRational;

use super::calculator::{
    evaluate, evaluate_at_x, format_result, normalize, CalculationError, QUERY_LIMIT,
};
use super::fraction::{
    calculate_fraction, ensure_exact_size, exact_abs_square_root, format_exact_multiple,
    format_square_root, FractionError,
};
use super::matrix::{general_solution, MatrixError, MAX_DIMENSION};
use super::operators::{split_top_level_commas, visit_additive_terms, NumberComparison};
use super::plane::{parse_point, PointError};
use super::scatter_plot::ScatterPlot;

const MAX_ROOT_CANDIDATES: usize = 64;
const FUNCTION_PLOT_INTERVALS: u32 = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EquationError {
    Coefficients,
    ExpressionArguments,
    ProductArguments,
    ProductDegree,
    VerifyRootsArguments,
    RootArguments,
    CurvePairArguments,
    ThreePointArguments,
    VertexFormArguments,
    TangentArguments,
    NormalLineArguments,
    FunctionPlotArguments,
    FunctionSampleArguments,
    IntegralArguments,
    ExtremaArguments,
    InequalityArguments,
    NonQuadratic,
    SystemRows,
    ThreeVariableRows,
    Evaluation(FractionError),
    Approximation(CalculationError),
}

impl EquationError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Coefficients | Self::Evaluation(FractionError::Invalid) => {
                "按 a, b, c 输入 ax² + bx + c = 0 的三个系数"
            }
            Self::ProductArguments => "填写 x 的因式乘积，例如 (x - 1)(x - 2)",
            Self::ProductDegree => "展开结果的次数须不超过二次",
            Self::VerifyRootsArguments => "填写等式或函数，用 @ 隔开 1 至 64 个候选值",
            Self::ExpressionArguments => {
                "输入由常数、x 和 x² 项组成的式子，例如 x**2 - 3*x + 2 = 0"
            }
            Self::InequalityArguments => {
                "输入 x² < 3x - 2，或先填写 >、>=、<、<= 中的一个，再填写 a, b, c"
            }
            Self::RootArguments => "填写两个有理数根，可再填非零的二次项系数",
            Self::CurvePairArguments => "填写两条函数，用分号或换行隔开，例如 x²; 1",
            Self::ThreePointArguments => {
                "填写三个横坐标互不相同的点，每个点写 x, y，各点用分号或换行隔开"
            }
            Self::VertexFormArguments => "按顶点横坐标、顶点纵坐标、二次项系数的顺序填写",
            Self::TangentArguments => "填写函数系数和切点横坐标，例如 1,0,0 @ 2",
            Self::NormalLineArguments => "填写函数和法线经过的横坐标，例如 x² @ 2",
            Self::FunctionPlotArguments => "填写函数和两个不同的横坐标，例如 x² @ -1 到 2",
            Self::FunctionSampleArguments => {
                "填写带 x 的算式以及两个不同的端点，可在分号后填写 2 至 129 个采样点"
            }
            Self::IntegralArguments => "填写函数系数和积分起止点，例如 1,0,0 @ 0 到 1",
            Self::ExtremaArguments => "填写函数系数和闭区间的两个端点，例如 1,0,0 @ -1 到 1",
            Self::NonQuadratic => "二次项系数 a 须非零",
            Self::SystemRows => "输入 x + y = 3; x - y = 1，或每行按 x 系数、y 系数、右侧常数填写",
            Self::Evaluation(error) => error.message(),
            Self::Approximation(error) => error.message(),
            Self::ThreeVariableRows => "填写最多八条含 x、y、z 的展开式方程，用分号或换行隔开",
        }
    }
}

pub(super) fn solve_equation(query: &str) -> Result<String, EquationError> {
    let input = normalize_equation(query)?;
    if let Some((operation, arguments)) = input.trim().split_once(char::is_whitespace) {
        match operation.to_ascii_lowercase().as_str() {
            "不等式" | "inequality" => return solve_inequality(arguments.trim_start()),
            "顶点" | "vertex" => return describe_quadratic(arguments.trim_start(), false),
            "摘要" | "summary" => return quadratic_summary(arguments.trim_start()),
            "判别式" | "discriminant" => return quadratic_discriminant(arguments.trim_start()),
            "韦达" | "vieta" => return quadratic_root_sum_and_product(arguments.trim_start()),
            "因式分解" | "factor" => return factor_polynomial(arguments.trim_start()),
            "展开" | "expand" => return expand_polynomial(arguments.trim_start()),
            "求根" | "roots" => {
                return solve_equation_coefficients(equation_input_coefficients(arguments)?);
            }
            "验根" | "verify-roots" => return verify_roots(arguments.trim_start()),
            "取系数" | "coefficients" => {
                let [a, b, c] = expanded_equation_coefficients(arguments)?;
                return format_quadratic_coefficients(a, b, c);
            }
            "由根" | "from-roots" => return quadratic_from_roots(arguments.trim_start()),
            "交点" | "intersections" => return polynomial_intersections(arguments.trim_start()),
            "围成面积" | "enclosed-area" => {
                return polynomial_enclosed_area(arguments.trim_start())
            }
            "标准式" | "standard-form" => {
                return polynomial_standard_form(arguments.trim_start())
            }
            "过三点" | "through-points" => {
                return quadratic_through_points(arguments.trim_start())
            }
            "由顶点" | "from-vertex" => return quadratic_from_vertex(arguments.trim_start()),
            "切线" | "tangent" => return polynomial_tangent(arguments.trim_start()),
            "法线" | "normal-line" => return polynomial_normal_line(arguments.trim_start()),
            "函数图" | "function-plot" => {
                return quadratic_curve_points(arguments).map(|points| curve_csv(&points));
            }
            "函数采样" | "sample-function" => return sampled_function_csv(arguments),
            "定积分" | "integral" => return polynomial_integral(arguments.trim_start()),
            "求导" | "differentiate" => return polynomial_derivative(arguments.trim_start()),
            "原函数" | "antiderivative" => {
                return polynomial_antiderivative(arguments.trim_start())
            }
            "区间极值" | "interval-extrema" => {
                return polynomial_interval_extrema(arguments.trim_start())
            }
            "配方" | "complete-the-square" => {
                return describe_quadratic(arguments.trim_start(), true);
            }
            _ => {}
        }
    }
    let coefficients = if input.contains('=') {
        expanded_equation_coefficients(&input)?
    } else {
        coefficient_row(&input)?
    };
    solve_equation_coefficients(coefficients)
}

fn normalize_equation(query: &str) -> Result<String, EquationError> {
    if query.chars().take(QUERY_LIMIT + 1).count() > QUERY_LIMIT {
        return Err(EquationError::Evaluation(FractionError::TooLong));
    }
    Ok(normalize(query).replace('·', "*"))
}

fn equation_input_coefficients(input: &str) -> Result<[BigRational; 3], EquationError> {
    if input
        .split_once('=')
        .is_some_and(|(left, _)| !left.trim().eq_ignore_ascii_case("y"))
    {
        expanded_equation_coefficients(input)
    } else {
        function_coefficients(input)
    }
}

fn verify_roots(input: &str) -> Result<String, EquationError> {
    let invalid = EquationError::VerifyRootsArguments;
    let (expression, candidates) = input.split_once('@').ok_or(invalid)?;
    let candidates: Vec<_> = split_top_level_commas(candidates)
        .take(MAX_ROOT_CANDIDATES + 1)
        .collect();
    if candidates.len() > MAX_ROOT_CANDIDATES {
        return Err(invalid);
    }
    let [a, b, c] = equation_input_coefficients(expression)?;
    let mut lines = vec!["x,代入值,是否为根".to_owned()];
    for candidate in candidates {
        let x = equation_value(candidate, invalid)?;
        let value = polynomial_value(&a, &b, &c, &x)?;
        let is_root = if value.numer().sign() == Sign::NoSign {
            "是"
        } else {
            "否"
        };
        lines.push(format!("{x},{value},{is_root}"));
    }
    Ok(lines.join("\r\n"))
}

fn solve_equation_coefficients([a, b, c]: [BigRational; 3]) -> Result<String, EquationError> {
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

fn expanded_equation_coefficients(input: &str) -> Result<[BigRational; 3], EquationError> {
    let Some((left, right)) = input.split_once('=') else {
        return polynomial_expression(input, PolynomialForm::Quadratic);
    };
    move_polynomial_terms(left, right, PolynomialForm::Quadratic)
}

#[derive(Clone, Copy)]
enum PolynomialForm {
    Quadratic,
    TwoVariableLinear,
    ThreeVariableLinear,
}

fn move_polynomial_terms(
    left: &str,
    right: &str,
    form: PolynomialForm,
) -> Result<[BigRational; 3], EquationError> {
    let [a, b, c] = polynomial_expression(left, form)?;
    let [right_a, right_b, right_c] = polynomial_expression(right, form)?;
    let coefficients = [a - right_a, b - right_b, c - right_c];
    for value in &coefficients {
        ensure_exact_size(value).map_err(EquationError::Evaluation)?;
    }
    Ok(coefficients)
}

fn polynomial_expression(
    input: &str,
    form: PolynomialForm,
) -> Result<[BigRational; 3], EquationError> {
    parse_polynomial(input, form, matches!(form, PolynomialForm::Quadratic))
}

fn parse_polynomial<const N: usize>(
    input: &str,
    form: PolynomialForm,
    allow_product_terms: bool,
) -> Result<[BigRational; N], EquationError> {
    let mut coefficients = std::array::from_fn::<_, N, _>(|_| BigRational::default());
    let add_term = |term: &str| -> Result<(), EquationError> {
        let term = term.trim();
        match expanded_polynomial_term(term, form) {
            Ok((index, coefficient)) => {
                let value = coefficients
                    .get_mut(index)
                    .ok_or(EquationError::ExpressionArguments)?;
                *value += coefficient;
                ensure_exact_size(value).map_err(EquationError::Evaluation)
            }
            Err(EquationError::ExpressionArguments) if allow_product_terms => {
                let product = factored_polynomial(term).map_err(|error| match error {
                    EquationError::ProductArguments => EquationError::ExpressionArguments,
                    _ => error,
                })?;
                for (value, coefficient) in coefficients.iter_mut().zip(product) {
                    *value += coefficient;
                    ensure_exact_size(value).map_err(EquationError::Evaluation)?;
                }
                Ok(())
            }
            Err(error) => Err(error),
        }
    };
    visit_additive_terms(input, EquationError::ExpressionArguments, add_term)?;
    Ok(coefficients)
}

fn expanded_polynomial_term(
    term: &str,
    form: PolynomialForm,
) -> Result<(usize, BigRational), EquationError> {
    let invalid = EquationError::ExpressionArguments;
    let variable = term
        .rmatch_indices(['x', 'X', 'y', 'Y', 'z', 'Z'])
        .find(|(index, _)| {
            let tail = term[index + 1..].trim_start();
            tail.is_empty() || tail.starts_with(['*', '×', '/', '^', '²'])
        });
    let constant_index = if matches!(form, PolynomialForm::ThreeVariableLinear) {
        3
    } else {
        2
    };
    let Some((index, variable)) = variable else {
        return equation_value(term, invalid).map(|constant| (constant_index, constant));
    };
    let prefix = term[..index].trim_end();
    let coefficient = if let Some(coefficient) = prefix.strip_suffix(['*', '×']) {
        let coefficient = coefficient.trim();
        if matches!(coefficient, "" | "+" | "-") {
            return Err(invalid);
        }
        coefficient
    } else {
        prefix
    };
    let value = match coefficient {
        "" | "+" => BigRational::from_integer(1.into()),
        "-" => BigRational::from_integer((-1).into()),
        coefficient => equation_value(coefficient, invalid)?,
    };
    let tail = term[index + 1..].trim_start();
    let (degree, tail) = small_power_suffix(tail).ok_or(invalid)?;
    let index = match (form, variable, degree) {
        (PolynomialForm::Quadratic, "x" | "X", _) => 2 - degree,
        (PolynomialForm::TwoVariableLinear, _, 0) => 2,
        (PolynomialForm::TwoVariableLinear, "x" | "X", 1) => 0,
        (PolynomialForm::TwoVariableLinear, "y" | "Y", 1) => 1,
        (PolynomialForm::ThreeVariableLinear, _, 0) => 3,
        (PolynomialForm::ThreeVariableLinear, "x" | "X", 1) => 0,
        (PolynomialForm::ThreeVariableLinear, "y" | "Y", 1) => 1,
        (PolynomialForm::ThreeVariableLinear, "z" | "Z", 1) => 2,
        _ => return Err(invalid),
    };
    let tail = tail.trim_start();
    if tail.is_empty() {
        return Ok((index, value));
    }
    if tail.starts_with("**") {
        return Err(invalid);
    }
    let mut suffix = tail.chars();
    let operator = suffix.next().ok_or(invalid)?;
    let scalar = suffix.as_str().trim();
    let scalar = if let Some(inner) = scalar
        .strip_prefix('(')
        .and_then(|inner| inner.strip_suffix(')'))
    {
        // The outer parentheses must span the entire operand.
        let mut depth = 0i32;
        for character in inner.chars() {
            match character {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            if depth < 0 {
                return Err(invalid);
            }
        }
        equation_value(scalar, invalid)?
    } else {
        if scalar.contains('/') {
            return Err(invalid);
        }
        super::fraction::parse_fraction_literal(scalar).map_err(|error| match error {
            FractionError::Invalid | FractionError::Incomplete => invalid,
            _ => EquationError::Evaluation(error),
        })?
    };
    match operator {
        '*' | '×' => Ok((index, value * scalar)),
        '/' if scalar.numer().sign() != Sign::NoSign => Ok((index, value / scalar)),
        '/' => Err(EquationError::Evaluation(FractionError::ZeroDenominator)),
        _ => Err(invalid),
    }
}

fn small_power_suffix(input: &str) -> Option<(usize, &str)> {
    if let Some(rest) = input.strip_prefix('²') {
        return Some((2, rest));
    }
    let Some(exponent) = input.strip_prefix("**").or_else(|| input.strip_prefix('^')) else {
        return Some((1, input));
    };
    let (digit, rest) = exponent.trim_start().split_at_checked(1)?;
    let degree = match digit {
        "0" => 0,
        "1" => 1,
        "2" => 2,
        _ => return None,
    };
    Some((degree, rest))
}

fn quadratic_center(a: &BigRational, b: BigRational, c: BigRational) -> (BigRational, BigRational) {
    // 配方后直接计算有理数和根式，有限小数系数也能精确判定重根。
    let center = -b / (a * BigInt::from(2));
    let distance_squared = &center * &center - c / a;
    (center, distance_squared)
}

fn quadratic_coefficients(input: &str) -> Result<[BigRational; 3], EquationError> {
    let coefficients = function_coefficients(input)?;
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

fn polynomial_tangent(input: &str) -> Result<String, EquationError> {
    let ([a, b, c], x) = function_and_x(input, EquationError::TangentArguments)?;
    let ax = &a * &x;
    let slope = &ax * BigInt::from(2u8) + b;
    let intercept = c - ax * x;
    format_quadratic_coefficients(BigRational::default(), slope, intercept)
}

fn polynomial_normal_line(input: &str) -> Result<String, EquationError> {
    let ([a, b, c], x) = function_and_x(input, EquationError::NormalLineArguments)?;
    let derivative = &a * &x * BigInt::from(2u8) + &b;
    if derivative.numer().sign() == Sign::NoSign {
        return Ok(format!("x = {x}"));
    }
    let slope = -derivative.recip();
    let intercept = polynomial_value(&a, &b, &c, &x)? - &slope * x;
    let expression =
        format_polynomial_terms([BigRational::default(), slope, intercept], ["x²", "x", ""])?;
    Ok(format!("y = {expression}"))
}

fn function_and_x(
    input: &str,
    invalid: EquationError,
) -> Result<([BigRational; 3], BigRational), EquationError> {
    let (function, point) = input.split_once('@').ok_or(invalid)?;
    let coefficients = function_coefficients(function)?;
    let x = equation_value(point, invalid)?;
    Ok((coefficients, x))
}

pub(super) fn function_plot_arguments(input: &str) -> Option<&str> {
    let (command, arguments) = input.trim().split_once(char::is_whitespace)?;
    (command == "函数图" || command.eq_ignore_ascii_case("function-plot")).then_some(arguments)
}

pub(super) fn plot_quadratic_function(input: &str) -> Result<(String, ScatterPlot), EquationError> {
    let input = normalize_equation(input)?;
    let points = quadratic_curve_points(&input)?;
    Ok((
        curve_csv(&points),
        ScatterPlot::from_function_curve(&points),
    ))
}

fn quadratic_curve_points(input: &str) -> Result<Vec<(BigRational, BigRational)>, EquationError> {
    let invalid = EquationError::FunctionPlotArguments;
    let ([a, b, c], [mut start, mut end]) = polynomial_bounds(input, invalid)?;
    match start.cmp(&end) {
        Ordering::Greater => std::mem::swap(&mut start, &mut end),
        Ordering::Equal => return Err(invalid),
        Ordering::Less => {}
    }
    let step = (&end - &start) / BigInt::from(FUNCTION_PLOT_INTERVALS);
    let mut xs: Vec<_> = (0..=FUNCTION_PLOT_INTERVALS)
        .map(|index| &start + &step * BigInt::from(index))
        .collect();
    if a.numer().sign() != Sign::NoSign {
        let vertex = -&b / (&a * BigInt::from(2u8));
        match xs.binary_search(&vertex) {
            Err(index) if (1..xs.len()).contains(&index) => xs.insert(index, vertex),
            _ => {}
        }
    }
    xs.into_iter()
        .map(|x| {
            ensure_exact_size(&x).map_err(EquationError::Evaluation)?;
            let y = polynomial_value(&a, &b, &c, &x)?;
            Ok((x, y))
        })
        .collect()
}

fn curve_csv(points: &[(BigRational, BigRational)]) -> String {
    std::iter::once("x,y".to_owned())
        .chain(points.iter().map(|(x, y)| format!("{x},{y}")))
        .collect::<Vec<_>>()
        .join("\r\n")
}

fn sampled_function_csv(input: &str) -> Result<String, EquationError> {
    let invalid = EquationError::FunctionSampleArguments;
    let (expression, bounds) = input.split_once('@').ok_or(invalid)?;
    let expression = match expression.split_once('=') {
        Some((name, formula)) if name.trim().eq_ignore_ascii_case("y") => formula.trim(),
        _ => expression.trim(),
    };
    if expression.is_empty() {
        return Err(invalid);
    }
    let (bounds, count) = match bounds.split_once(';') {
        Some((bounds, count)) => {
            let count = count
                .trim()
                .parse::<u32>()
                .ok()
                .filter(|count| (2..=129).contains(count))
                .ok_or(invalid)?;
            (bounds, count)
        }
        None => (bounds, 33),
    };
    let (start, end) = bounds.split_once(['到', '至']).ok_or(invalid)?;
    let mut start = evaluate(start).map_err(EquationError::Approximation)?;
    let mut end = evaluate(end).map_err(EquationError::Approximation)?;
    if start == end {
        return Err(invalid);
    }
    if start > end {
        std::mem::swap(&mut start, &mut end);
    }
    let last = count - 1;
    let mut csv = String::from("x,近似值");
    for index in 0..count {
        let fraction = f64::from(index) / f64::from(last);
        let x = match index {
            0 => start,
            index if index == last => end,
            _ => start * (1.0 - fraction) + end * fraction,
        };
        let y = match evaluate_at_x(expression, x) {
            Ok(value) => format_result(value).map_err(EquationError::Approximation)?,
            Err(
                CalculationError::DivisionByZero
                | CalculationError::FunctionDomain
                | CalculationError::OutOfRange,
            ) => String::new(),
            Err(error) => return Err(EquationError::Approximation(error)),
        };
        let x = format_result(x).map_err(EquationError::Approximation)?;
        csv.push_str(&format!("\r\n{x},{y}"));
    }
    Ok(csv)
}

fn polynomial_integral(input: &str) -> Result<String, EquationError> {
    let ([a, b, c], [start, end]) = polynomial_bounds(input, EquationError::IntegralArguments)?;
    let cubic = a / BigInt::from(3u8);
    let quadratic = b / BigInt::from(2u8);
    let antiderivative = |x: BigRational| ((&cubic * &x + &quadratic) * &x + &c) * x;
    let value = antiderivative(end) - antiderivative(start);
    ensure_exact_size(&value).map_err(EquationError::Evaluation)?;
    Ok(value.to_string())
}

fn polynomial_interval_extrema(input: &str) -> Result<String, EquationError> {
    let ([a, b, c], [mut start, mut end]) =
        polynomial_bounds(input, EquationError::ExtremaArguments)?;
    if start > end {
        std::mem::swap(&mut start, &mut end);
    }
    if a.numer().sign() == Sign::NoSign && b.numer().sign() == Sign::NoSign && start < end {
        return Ok(format!("最小值 {c} · 最大值 {c}（区间内任意 x）"));
    }
    let mut points = vec![start.clone()];
    if start < end {
        if a.numer().sign() != Sign::NoSign {
            let vertex = -&b / (&a * BigInt::from(2u8));
            if start < vertex && vertex < end {
                points.push(vertex);
            }
        }
        points.push(end);
    }
    let values = points
        .into_iter()
        .map(|x| {
            let y = polynomial_value(&a, &b, &c, &x)?;
            ensure_exact_size(&x).map_err(EquationError::Evaluation)?;
            Ok((x, y))
        })
        .collect::<Result<Vec<_>, EquationError>>()?;
    let extrema = |label, bound| {
        let positions = values
            .iter()
            .filter(|(_, y)| y == bound)
            .map(|(x, _)| x.to_string())
            .collect::<Vec<_>>()
            .join("、");
        format!("{label} {bound}（x = {positions}）")
    };
    let minimum = values
        .iter()
        .map(|(_, y)| y)
        .min()
        .expect("至少计算一个端点");
    let maximum = values
        .iter()
        .map(|(_, y)| y)
        .max()
        .expect("至少计算一个端点");
    Ok(format!(
        "{}\n{}",
        extrema("最小值", minimum),
        extrema("最大值", maximum)
    ))
}

fn polynomial_value(
    a: &BigRational,
    b: &BigRational,
    c: &BigRational,
    x: &BigRational,
) -> Result<BigRational, EquationError> {
    let y = (a * x + b) * x + c;
    ensure_exact_size(&y).map_err(EquationError::Evaluation)?;
    Ok(y)
}

fn polynomial_bounds(
    input: &str,
    invalid: EquationError,
) -> Result<([BigRational; 3], [BigRational; 2]), EquationError> {
    let (coefficients, bounds) = input.split_once('@').ok_or(invalid)?;
    let (start, end) = bounds.split_once(['到', '至']).ok_or(invalid)?;
    Ok((
        function_coefficients(coefficients)?,
        [
            equation_value(start, invalid)?,
            equation_value(end, invalid)?,
        ],
    ))
}

fn equation_value(input: &str, invalid: EquationError) -> Result<BigRational, EquationError> {
    calculate_fraction(input).map_err(|error| match error {
        FractionError::Invalid | FractionError::Incomplete => invalid,
        _ => EquationError::Evaluation(error),
    })
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

fn polynomial_pair(input: &str) -> Result<[[BigRational; 3]; 2], EquationError> {
    let (first, second) = input
        .split_once([';', '；', '\n'])
        .ok_or(EquationError::CurvePairArguments)?;
    let parse = |coefficients: &str| match function_coefficients(coefficients) {
        Err(
            EquationError::Coefficients
            | EquationError::ExpressionArguments
            | EquationError::Evaluation(FractionError::Invalid | FractionError::Incomplete),
        ) => Err(EquationError::CurvePairArguments),
        result => result,
    };
    Ok([parse(first)?, parse(second)?])
}

fn polynomial_enclosed_area(input: &str) -> Result<String, EquationError> {
    let [[a, b, c], [other_a, other_b, other_c]] = polynomial_pair(input)?;
    let (diff_a, diff_b, diff_c) = (a - other_a, b - other_b, c - other_c);
    if diff_a.numer().sign() == Sign::NoSign {
        return Ok("没有有界区域".to_owned());
    }
    let (_, squared) = quadratic_center(&diff_a, diff_b, diff_c);
    if squared.numer().sign() != Sign::Plus {
        return Ok("没有有界区域".to_owned());
    }
    ensure_exact_size(&squared).map_err(EquationError::Evaluation)?;
    let magnitude = if diff_a.numer().sign() == Sign::Minus {
        -diff_a
    } else {
        diff_a
    };
    // 在两个交点之间积分 |a|(r² - (x-h)²)，得到 (4/3)|a|r³。
    let factor = BigRational::new(4.into(), 3.into()) * magnitude * &squared;
    if let Some(radius) = exact_abs_square_root(&squared) {
        let area = factor * radius;
        ensure_exact_size(&area).map_err(EquationError::Evaluation)?;
        Ok(area.to_string())
    } else {
        ensure_exact_size(&factor).map_err(EquationError::Evaluation)?;
        Ok(format_exact_multiple(
            &factor,
            &format_square_root(&squared),
        ))
    }
}

fn polynomial_intersections(input: &str) -> Result<String, EquationError> {
    let [[a, b, c], [other_a, other_b, other_c]] = polynomial_pair(input)?;
    let (diff_a, diff_b, diff_c) = (&a - other_a, &b - other_b, &c - other_c);
    let point = |x: &BigRational| {
        let y = polynomial_value(&a, &b, &c, x)?;
        ensure_exact_size(x).map_err(EquationError::Evaluation)?;
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

fn expand_polynomial(input: &str) -> Result<String, EquationError> {
    let coefficients = function_coefficients(input).map_err(|error| match error {
        EquationError::ExpressionArguments => EquationError::ProductArguments,
        _ => error,
    })?;
    let expression = format_polynomial_terms(coefficients, ["x²", "x", ""])?;
    Ok(format!("y = {expression}"))
}

fn factored_polynomial(input: &str) -> Result<[BigRational; 3], EquationError> {
    let invalid = EquationError::ProductArguments;
    let input = input.trim();
    let (negative, input) = if let Some(input) = input.strip_prefix('-') {
        (true, input)
    } else {
        (false, input.strip_prefix('+').unwrap_or(input))
    };
    let input = input.trim_start();
    let first_factor = input.find(['x', 'X', '(']).ok_or(invalid)?;
    let (scale, mut rest) = input.split_at(first_factor);
    let scale = if scale.is_empty() {
        BigRational::from_integer(1.into())
    } else {
        let scale = scale.trim_end();
        let scale = scale.strip_suffix(['*', '×']).unwrap_or(scale);
        super::fraction::parse_fraction_literal(scale).map_err(|error| match error {
            FractionError::Invalid | FractionError::Incomplete => invalid,
            _ => EquationError::Evaluation(error),
        })?
    };
    let mut coefficients = [
        BigRational::default(),
        BigRational::default(),
        if negative { -scale } else { scale },
    ];
    loop {
        let (factor, remainder) = product_factor(rest)?;
        let (power, remainder) = small_power_suffix(remainder.trim_start()).ok_or(invalid)?;
        for _ in 0..power {
            coefficients = multiply_polynomial(&coefficients, &factor)?;
        }
        rest = remainder.trim_start();
        if rest.is_empty() {
            return Ok(coefficients);
        }
        if let Some(tail) = rest.strip_prefix(['*', '×']) {
            rest = tail.trim_start();
        }
    }
}

fn product_factor(input: &str) -> Result<([BigRational; 3], &str), EquationError> {
    let invalid = EquationError::ProductArguments;
    if let Some(rest) = input.strip_prefix(['x', 'X']) {
        let zero = BigRational::default();
        return Ok((
            [zero.clone(), BigRational::from_integer(1.into()), zero],
            rest,
        ));
    }
    let body = input.strip_prefix('(').ok_or(invalid)?;
    let mut depth = 1usize;
    let close = body
        .char_indices()
        .find_map(|(index, ch)| match ch {
            '(' => {
                depth += 1;
                None
            }
            ')' => {
                depth -= 1;
                (depth == 0).then_some(index)
            }
            _ => None,
        })
        .ok_or(invalid)?;
    let factor =
        parse_polynomial(&body[..close], PolynomialForm::Quadratic, false).map_err(|error| {
            if error == EquationError::ExpressionArguments {
                invalid
            } else {
                error
            }
        })?;
    Ok((factor, &body[close + 1..]))
}

fn multiply_polynomial(
    [a, b, c]: &[BigRational; 3],
    [d, e, f]: &[BigRational; 3],
) -> Result<[BigRational; 3], EquationError> {
    let nonzero = |value: &BigRational| value.numer().sign() != Sign::NoSign;
    if (nonzero(a) && nonzero(e)) || (nonzero(d) && (nonzero(a) || nonzero(b))) {
        return Err(EquationError::ProductDegree);
    }
    let coefficients = [a * f + b * e + c * d, b * f + c * e, c * f];
    for value in &coefficients {
        ensure_exact_size(value).map_err(EquationError::Evaluation)?;
    }
    Ok(coefficients)
}

fn factor_polynomial(input: &str) -> Result<String, EquationError> {
    let [a, b, c] = function_coefficients(input)?;
    let (leading, factors) = if a.numer().sign() == Sign::NoSign {
        if b.numer().sign() == Sign::NoSign {
            return Ok(c.to_string());
        }
        let root = -c / &b;
        ensure_exact_size(&root).map_err(EquationError::Evaluation)?;
        (b / root.denom(), integer_linear_factor(&root))
    } else {
        let (center, squared) = quadratic_center(&a, b, c);
        ensure_exact_size(&squared).map_err(EquationError::Evaluation)?;
        let Some(distance) =
            exact_abs_square_root(&squared).filter(|_| squared.numer().sign() != Sign::Minus)
        else {
            return Ok("在有理数范围内无一次因式".to_owned());
        };
        let [first, second] =
            exact_real_roots(&center, &distance).map_err(EquationError::Evaluation)?;
        let leading = a / (first.denom() * second.denom());
        let left = integer_linear_factor(&first);
        let factors = if first == second {
            format!("{left}²")
        } else {
            format!("{left}{}", integer_linear_factor(&second))
        };
        (leading, factors)
    };
    ensure_exact_size(&leading).map_err(EquationError::Evaluation)?;
    Ok(if leading.is_integer() {
        match leading.numer().to_string().as_str() {
            "1" => factors,
            "-1" => format!("-{factors}"),
            integer => format!("{integer}{factors}"),
        }
    } else {
        format!("({leading}){factors}")
    })
}

fn integer_linear_factor(root: &BigRational) -> String {
    if root.numer().sign() == Sign::NoSign {
        return "x".to_owned();
    }
    let term = format_exact_multiple(&BigRational::from_integer(root.denom().clone()), "x");
    let sign = if root.numer().sign() == Sign::Minus {
        '+'
    } else {
        '-'
    };
    let constant = root.numer().magnitude();
    format!("({term} {sign} {constant})")
}

fn polynomial_standard_form(input: &str) -> Result<String, EquationError> {
    let coefficients = function_coefficients(input)?;
    let expression = format_polynomial_terms(coefficients, ["x²", "x", ""])?;
    Ok(format!("y = {expression}"))
}

fn polynomial_derivative(input: &str) -> Result<String, EquationError> {
    let [a, b, _] = function_coefficients(input)?;
    format_polynomial_terms(
        [BigRational::default(), a * BigInt::from(2u8), b],
        ["x²", "x", ""],
    )
}

fn polynomial_antiderivative(input: &str) -> Result<String, EquationError> {
    let [a, b, c] = function_coefficients(input)?;
    let expression = format_polynomial_terms(
        [a / BigInt::from(3u8), b / BigInt::from(2u8), c],
        ["x³", "x²", "x"],
    )?;
    Ok(if expression == "0" {
        "C".to_owned()
    } else {
        format!("{expression} + C")
    })
}

fn format_polynomial_terms(
    coefficients: [BigRational; 3],
    variables: [&str; 3],
) -> Result<String, EquationError> {
    let mut expression = String::new();
    for (coefficient, variable) in coefficients.into_iter().zip(variables) {
        ensure_exact_size(&coefficient).map_err(EquationError::Evaluation)?;
        let sign = coefficient.numer().sign();
        if sign == Sign::NoSign {
            continue;
        }
        let negative = sign == Sign::Minus;
        if expression.is_empty() {
            if negative {
                expression.push('-');
            }
        } else {
            expression.push_str(if negative { " - " } else { " + " });
        }
        let magnitude = if negative { -coefficient } else { coefficient };
        let term = if variable.is_empty() {
            magnitude.to_string()
        } else {
            format_exact_multiple(&magnitude, variable)
        };
        expression.push_str(&term);
    }
    if expression.is_empty() {
        expression.push('0');
    }
    Ok(expression)
}

fn quadratic_summary(input: &str) -> Result<String, EquationError> {
    let [a, b, c] = quadratic_coefficients(input)?;
    let (x, distance_squared) = quadratic_center(&a, b, c);
    let y = -&a * &distance_squared;
    for value in [&x, &y, &distance_squared] {
        ensure_exact_size(value).map_err(EquationError::Evaluation)?;
    }
    let left = format!("(-∞, {x}]");
    let right = format!("[{x}, ∞)");
    let (opening, range, increasing, decreasing) = if a.numer().sign() == Sign::Plus {
        ("向上", format!("[{y}, ∞)"), right, left)
    } else {
        ("向下", format!("(-∞, {y}]"), left, right)
    };
    let roots = if distance_squared.numer().sign() == Sign::Minus {
        "无实数零点".to_owned()
    } else {
        format_centered_roots(x.clone(), distance_squared, "x")
            .map_err(EquationError::Evaluation)?
    };
    Ok(format!(
        "顶点 ({x}, {y}) · 对称轴 x = {x}\n开口{opening} · 值域 {range}\n递增 {increasing}\n递减 {decreasing}\n{roots}"
    ))
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

fn inequality_arguments(
    input: &str,
) -> Result<(NumberComparison, [BigRational; 3]), EquationError> {
    let invalid = EquationError::InequalityArguments;
    let (index, symbol) = input
        .char_indices()
        .find(|(_, symbol)| matches!(symbol, '<' | '>' | '≤' | '≥'))
        .ok_or(invalid)?;
    let mut end = index + symbol.len_utf8();
    if matches!(symbol, '<' | '>') && input[end..].starts_with('=') {
        end += 1;
    }
    let comparison = NumberComparison::parse(&input[index..end]).ok_or(invalid)?;
    let right = &input[end..];
    let coefficients = if index == 0 {
        coefficient_row(right).map_err(|error| match error {
            EquationError::Coefficients => invalid,
            _ => error,
        })?
    } else {
        move_polynomial_terms(&input[..index], right, PolynomialForm::Quadratic)?
    };
    Ok((comparison, coefficients))
}

fn solve_inequality(input: &str) -> Result<String, EquationError> {
    let (comparison, [a, b, c]) = inequality_arguments(input)?;
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
        return exact_real_roots(center, &distance).map(|roots| roots.map(|root| root.to_string()));
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
) -> Result<[BigRational; 2], FractionError> {
    let left = center - distance;
    let right = center + distance;
    for root in [&left, &right] {
        ensure_exact_size(root)?;
    }
    Ok([left, right])
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

fn function_coefficients(input: &str) -> Result<[BigRational; 3], EquationError> {
    if let Some((output, expression)) = input.split_once('=') {
        if !output.trim().eq_ignore_ascii_case("y") {
            return Err(EquationError::ExpressionArguments);
        }
        return polynomial_expression(expression, PolynomialForm::Quadratic);
    }
    if split_top_level_commas(input).nth(1).is_some() {
        return coefficient_row(input);
    }
    polynomial_expression(input, PolynomialForm::Quadratic)
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
    let input = normalize(query).replace('·', "*");
    let command = input.trim_start().split_once(char::is_whitespace);
    if let Some((_, arguments)) = command.filter(|(operation, _)| {
        *operation == "三元" || operation.eq_ignore_ascii_case("three-variables")
    }) {
        return solve_three_variable_system(arguments);
    }
    let (parametric, input) = command
        .filter(|(operation, _)| *operation == "通解" || operation.eq_ignore_ascii_case("general"))
        .map_or((false, input.as_ref()), |(_, rows)| (true, rows));
    let (first, second) = input
        .split_once([';', '\n'])
        .ok_or(EquationError::SystemRows)?;
    let row = |text: &str| match linear_system_row(text) {
        Err(
            EquationError::Coefficients
            | EquationError::ExpressionArguments
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
        if inconsistent {
            return Ok("无解".to_owned());
        }
        if !parametric {
            return Ok("无穷多解".to_owned());
        }
        let (a, b, c) = if is_zero(&a) && is_zero(&b) {
            (d, e, f)
        } else {
            (a, b, c)
        };
        if is_zero(&a) && is_zero(&b) {
            return Ok("x = s · y = t（s、t 为任意实数）".to_owned());
        }
        if is_zero(&a) {
            let y = c / b;
            ensure_exact_size(&y).map_err(EquationError::Evaluation)?;
            return Ok(format!("x = t · y = {y}（t 为任意实数）"));
        }
        let expression = parameter_expression(c / &a, -b / a)?;
        return Ok(format!("x = {expression} · y = t（t 为任意实数）"));
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

fn solve_three_variable_system(input: &str) -> Result<String, EquationError> {
    let invalid = EquationError::ThreeVariableRows;
    let rows: Vec<_> = input
        .trim()
        .split([';', '\n'])
        .take(MAX_DIMENSION + 1)
        .collect();
    if rows.len() > MAX_DIMENSION {
        return Err(invalid);
    }
    let mut coefficients = Vec::with_capacity(rows.len());
    let mut constants = Vec::with_capacity(rows.len());
    let parse = |input: &str| {
        parse_polynomial::<4>(input, PolynomialForm::ThreeVariableLinear, false).map_err(|error| {
            match error {
                EquationError::ExpressionArguments => invalid,
                _ => error,
            }
        })
    };
    for row in rows {
        let (left, right) = row.split_once('=').ok_or(invalid)?;
        let left = parse(left)?;
        let right = parse(right)?;
        let [x, y, z, constant] = std::array::from_fn(|index| &left[index] - &right[index]);
        for value in [&x, &y, &z, &constant] {
            ensure_exact_size(value).map_err(EquationError::Evaluation)?;
        }
        coefficients.push(vec![x, y, z]);
        constants.push(vec![-constant]);
    }
    general_solution(coefficients, constants, Some(&["x", "y", "z"])).map_err(|error| match error {
        MatrixError::Evaluation(error) => EquationError::Evaluation(error),
        _ => invalid,
    })
}

fn linear_system_row(input: &str) -> Result<[BigRational; 3], EquationError> {
    if let Some((left, right)) = input.split_once('=') {
        let [x, y, constant] =
            move_polynomial_terms(left, right, PolynomialForm::TwoVariableLinear)?;
        Ok([x, y, -constant])
    } else {
        coefficient_row(input)
    }
}

fn parameter_expression(
    constant: BigRational,
    slope: BigRational,
) -> Result<String, EquationError> {
    for value in [&constant, &slope] {
        ensure_exact_size(value).map_err(EquationError::Evaluation)?;
    }
    if slope.numer().sign() == Sign::NoSign {
        return Ok(constant.to_string());
    }
    let negative = slope.numer().sign() == Sign::Minus;
    let magnitude = if negative { -slope } else { slope };
    let term = format_exact_multiple(&magnitude, "t");
    Ok(if constant.numer().sign() == Sign::NoSign {
        if negative {
            format!("-{term}")
        } else {
            term
        }
    } else {
        let sign = if negative { '-' } else { '+' };
        format!("{constant} {sign} {term}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarizes_the_vertex_range_monotonicity_and_real_roots() {
        assert_eq!(
            solve_equation("摘要 y = x² - 2x - 1").as_deref(),
            Ok("顶点 (1, -2) · 对称轴 x = 1\n开口向上 · 值域 [-2, ∞)\n递增 [1, ∞)\n递减 (-∞, 1]\nx = 1 ± √2"),
        );
        assert_eq!(
            solve_equation("summary -2,4,-3").as_deref(),
            Ok("顶点 (1, -1) · 对称轴 x = 1\n开口向下 · 值域 (-∞, -1]\n递增 (-∞, 1]\n递减 [1, ∞)\n无实数零点"),
        );
        assert_eq!(
            solve_equation("摘要 y = x"),
            Err(EquationError::NonQuadratic)
        );
    }

    #[test]
    fn factors_polynomials_with_rational_coefficients_and_repeated_roots() {
        for (input, expected) in [
            ("因式分解 2,-5,2", "(2x - 1)(x - 2)"),
            ("factor 1/2*x² - x + 1/2", "(1/2)(x - 1)²"),
            ("因式分解 -x²", "-x²"),
            ("因式分解 y = x² + 2x", "(x + 2)x"),
            ("因式分解 3x/2 + 1", "(1/2)(3x + 2)"),
            ("因式分解 3/4", "3/4"),
            ("因式分解 x² - 2", "在有理数范围内无一次因式"),
            ("因式分解 x² + 1", "在有理数范围内无一次因式"),
        ] {
            assert_eq!(solve_equation(input).as_deref(), Ok(expected), "{input}");
        }
    }

    #[test]
    fn solves_three_named_variables_and_reports_free_parameters() {
        for (input, expected) in [
            ("三元 x+y+z=6; x-y=0; z=2", "x = 2\ny = 2\nz = 2"),
            (
                "三元 x+z=2; y=1",
                "x = 2 - t₁\ny = 1\nz = t₁\nt₁ 为任意实数",
            ),
            ("三元 z=1; z=2", "无解"),
            (
                "three-variables x+y+z=1; Y+Z=2/3; X+Z=2/3",
                "x = 1/3\ny = 1/3\nz = 1/3",
            ),
        ] {
            assert_eq!(
                solve_linear_system(input).as_deref(),
                Ok(expected),
                "{input}"
            );
        }
        assert_eq!(
            solve_linear_system("三元 x+y+z²=1"),
            Err(EquationError::ThreeVariableRows)
        );
    }

    #[test]
    fn verifies_multiple_candidate_roots_with_exact_residuals() {
        assert_eq!(
            solve_equation("验根 (2x-1)(x-2) = 0 @ 1/2,1,2").as_deref(),
            Ok("x,代入值,是否为根\r\n1/2,0,是\r\n1,-1,否\r\n2,0,是")
        );
        assert_eq!(
            solve_equation("verify-roots y = x - 1/3 @ 1/3,0.33").as_deref(),
            Ok("x,代入值,是否为根\r\n1/3,0,是\r\n33/100,-1/300,否")
        );
        assert_eq!(
            solve_equation("验根 x = 1 @"),
            Err(EquationError::VerifyRootsArguments)
        );
    }

    #[test]
    fn combines_factor_products_and_ordinary_terms_in_one_polynomial() {
        for (input, expected) in [
            ("展开 2·(x - 3/4)² - 1/8", "y = 2x² - 3x + 1"),
            ("求根 (x - 1)**2 - 4", "x₁ = -1 · x₂ = 3"),
            ("1 - (x+1)^2 = 0", "x₁ = -2 · x₂ = 0"),
            ("定积分 (x-1)**2 + 1 @ 0 到 2", "8/3"),
            ("取系数 2*(x-1)^2 + 3*(x+1) - 1", "2,-1,4"),
        ] {
            assert_eq!(solve_equation(input).as_deref(), Ok(expected), "{input}");
        }
        let completed_square = solve_equation("配方 2,-3,1").unwrap();
        assert_eq!(
            solve_equation(&format!("求根 {completed_square}")).as_deref(),
            Ok("x₁ = 1/2 · x₂ = 1"),
        );
    }

    #[test]
    fn solves_roots_from_direct_function_inputs_and_standard_form_output() {
        for (input, expected) in [
            ("求根 y = 2x² - 5x + 2", "x₁ = 1/2 · x₂ = 2"),
            ("roots 2x² - 5x + 2", "x₁ = 1/2 · x₂ = 2"),
            ("求根 2x = 3", "x = 3/2"),
            ("求根 2,-5,2", "x₁ = 1/2 · x₂ = 2"),
        ] {
            assert_eq!(solve_equation(input).as_deref(), Ok(expected), "{input}");
        }
        let expanded = solve_equation("展开 (2x - 1)(x - 2)").unwrap();
        assert_eq!(
            solve_equation(&format!("求根 {expanded}")).as_deref(),
            Ok("x₁ = 1/2 · x₂ = 2"),
        );
    }

    #[test]
    fn accepts_factored_inputs_in_equations_inequalities_and_function_tools() {
        for (input, expected) in [
            ("(2x - 1)(x - 2) = 0", "x₁ = 1/2 · x₂ = 2"),
            ("不等式 (x-1)(x-2) < 0", "(1, 2)"),
            ("定积分 y = (x - 1)^2 @ 0 到 2", "2/3"),
            ("求导 (x+1)**2", "2x + 2"),
            ("取系数 (x + 1)(x - 1) = x", "1,-1,-1"),
        ] {
            assert_eq!(solve_equation(input).as_deref(), Ok(expected), "{input}");
        }
        assert_eq!(
            solve_equation("(x²+1)(x+1)=0"),
            Err(EquationError::ProductDegree)
        );
    }

    #[test]
    fn expands_products_and_rational_factors_without_losing_exactness() {
        for (input, expected) in [
            ("展开 (x - 1)(x - 2)", "y = x² - 3x + 2"),
            ("expand (1/2)(x - 1)²", "y = x²/2 - x + 1/2"),
            ("展开 -x(x + 2)", "y = -x² - 2x"),
            ("展开 (x+1)**2", "y = x² + 2x + 1"),
            ("展开 y = 2 × (x - 1) × (x+1)", "y = 2x² - 2"),
            ("展开 1/2, -1, 1/2", "y = x²/2 - x + 1/2"),
        ] {
            assert_eq!(solve_equation(input).as_deref(), Ok(expected), "{input}");
        }
        assert_eq!(
            solve_equation("展开 (x²+1)(x+1)"),
            Err(EquationError::ProductDegree)
        );
        assert_eq!(
            solve_equation("展开 (x+1)*"),
            Err(EquationError::ProductArguments)
        );
        let factors = solve_equation("因式分解 2,-5,2").unwrap();
        assert_eq!(
            solve_equation(&format!("展开 {factors}")).as_deref(),
            Ok("y = 2x² - 5x + 2")
        );
    }

    #[test]
    fn finds_exact_normal_lines_including_vertical_normals() {
        for (query, expected) in [
            ("法线 x² @ 2", "y = -x/4 + 9/2"),
            ("法线 y = (x-1/2)**2 @ 1/2", "x = 1/2"),
            ("normal-line 0,-2,3 @ 1", "y = x/2 + 1/2"),
            ("法线 3 @ -5/2", "x = -5/2"),
        ] {
            assert_eq!(solve_equation(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            solve_equation("法线 x²"),
            Err(EquationError::NormalLineArguments)
        );
    }

    #[test]
    fn formats_exact_derivatives_and_antiderivatives() {
        for (input, expected) in [
            ("求导 y = x²/2 - x + 3/4", "x - 1"),
            ("differentiate 0,0,3", "0"),
            ("原函数 y = x²/2 - x + 3/4", "x³/6 - x²/2 + 3x/4 + C"),
            ("antiderivative 0,0,-1/2", "-x/2 + C"),
            ("原函数 0,0,0", "C"),
        ] {
            assert_eq!(solve_equation(input).as_deref(), Ok(expected), "{input}");
        }
        let first = solve_equation("求导 x² - 2x + 1").unwrap();
        assert_eq!(solve_equation(&format!("求导 {first}")).as_deref(), Ok("2"));
    }

    #[test]
    fn samples_calculator_functions_with_x_and_leaves_undefined_values_empty() {
        for (query, expected) in [
            (
                "函数采样 y = cosd(x) @ 180 到 0; 3",
                "x,近似值\r\n0,1\r\n90,0\r\n180,-1",
            ),
            (
                "函数采样 1/x @ -1 到 1; 3",
                "x,近似值\r\n-1,-1\r\n0,\r\n1,1",
            ),
            (
                "函数采样 sqrt(X) @ -1 至 1; 3",
                "x,近似值\r\n-1,\r\n0,0\r\n1,1",
            ),
            (
                "sample-function 0x1e + x**2 @ 0 到 2; 2",
                "x,近似值\r\n0,30\r\n2,34",
            ),
        ] {
            assert_eq!(solve_equation(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            solve_equation("函数采样 x @ 0 到 1")
                .unwrap()
                .lines()
                .count(),
            34
        );
        assert_eq!(
            solve_equation("函数采样 unknown(x) @ 0 到 1; 2"),
            Err(EquationError::Approximation(CalculationError::Invalid))
        );
        for query in ["函数采样 x @ 2 到 2", "函数采样 x @ 0 到 1; 130"] {
            assert_eq!(
                solve_equation(query),
                Err(EquationError::FunctionSampleArguments)
            );
        }
        assert_eq!(evaluate("x"), Err(CalculationError::Invalid));
    }

    #[test]
    fn uses_function_expressions_in_the_exact_polynomial_tools() {
        for (input, expected) in [
            ("切线 x²/2 - x + 3/4 @ 2", "0,1,-5/4"),
            ("定积分 y = x**2 @ 0 到 2", "8/3"),
            (
                "区间极值 y = -x² + 1 @ -1 到 1",
                "最小值 0（x = -1、1）\n最大值 1（x = 0）",
            ),
            ("围成面积 y=x² ; 1", "4/3"),
            ("标准式 Y = 2 + x - 1 + x", "y = 2x + 1"),
            ("判别式 y = x**2 - 3*x + 2", "1"),
        ] {
            assert_eq!(solve_equation(input).as_deref(), Ok(expected), "{input}");
        }
        assert_eq!(
            solve_equation("交点 y = x² ; y = 1"),
            solve_equation("交点 1,0,0 ; 0,0,1"),
        );
        assert_eq!(
            solve_equation("定积分 x = 1 @ 0 到 2"),
            Err(EquationError::ExpressionArguments),
        );
    }

    #[test]
    fn solves_two_variable_systems_written_as_equations() {
        for (input, expected) in [
            ("x + y = 3; x - y = 1", "x = 2 · y = 1"),
            ("3x + Y = x + 5; X - y = 1", "x = 2 · y = 1"),
            ("x/2 + Y = 2\nx - y = 1", "x = 2 · y = 1"),
            ("x + y = 3; 1,-1,1", "x = 2 · y = 1"),
            (
                "通解 x + 2y = 3; 2x + 4y = 6",
                "x = 3 - 2t · y = t（t 为任意实数）",
            ),
            ("x = 1; x = 2", "无解"),
        ] {
            assert_eq!(
                solve_linear_system(input).as_deref(),
                Ok(expected),
                "{input}"
            );
        }
        for input in ["xy = 2; x = 1", "x² + y = 1; x = 1", "x + y = 1; x = y = 3"] {
            assert_eq!(solve_linear_system(input), Err(EquationError::SystemRows));
        }
    }

    #[test]
    fn describes_free_parameters_only_when_the_two_equations_are_consistent() {
        for (input, expected) in [
            ("通解 1,2,3; 2,4,6", "x = 3 - 2t · y = t（t 为任意实数）"),
            ("general 0,0,0; 2,1,0", "x = -t/2 · y = t（t 为任意实数）"),
            ("通解 0,2,4; 0,1,2", "x = t · y = 2（t 为任意实数）"),
            ("通解 0,0,0; 0,0,0", "x = s · y = t（s、t 为任意实数）"),
            ("通解 0,0,1; 1,2,3", "无解"),
            ("通解 2,1,5; 1,-1,1", "x = 2 · y = 1"),
        ] {
            assert_eq!(
                solve_linear_system(input).as_deref(),
                Ok(expected),
                "{input}"
            );
        }
        assert_eq!(
            solve_linear_system("1,2,3; 2,4,6").as_deref(),
            Ok("无穷多解")
        );
    }

    #[test]
    fn finds_interval_extrema_and_lists_both_positions_of_a_tied_bound() {
        for (input, expected) in [
            (
                "区间极值 1/2,-1,3/4 @ 0 到 2",
                "最小值 1/4（x = 1）\n最大值 3/4（x = 0、2）",
            ),
            (
                "interval-extrema -1,0,0 @ 1 到 -1",
                "最小值 -1（x = -1、1）\n最大值 0（x = 0）",
            ),
            (
                "区间极值 0,0,3/4 @ -1 到 2",
                "最小值 3/4 · 最大值 3/4（区间内任意 x）",
            ),
            (
                "区间极值 0,1,0 @ 2 到 2",
                "最小值 2（x = 2）\n最大值 2（x = 2）",
            ),
        ] {
            assert_eq!(solve_equation(input).as_deref(), Ok(expected), "{input}");
        }
        assert_eq!(
            solve_equation("区间极值 1,0,0 @ 1"),
            Err(EquationError::ExtremaArguments)
        );
    }

    #[test]
    fn calculates_exact_tangent_coefficients_and_signed_integrals() {
        for (input, expected) in [
            ("切线 1/2,-1,3/4 @ 2", "0,1,-5/4"),
            ("tangent 1/2,-1,3/4 @ 1", "0,0,1/4"),
            ("定积分 1/2,-1,3/4 @ 0 到 2", "5/6"),
            ("integral 1/2,-1,3/4 @ 2 至 0", "-5/6"),
            ("定积分 1,0,0 @ 1/2 到 1/2", "0"),
        ] {
            assert_eq!(solve_equation(input).as_deref(), Ok(expected), "{input}");
        }
        assert_eq!(
            solve_equation("定积分 1,0,0 @ 1"),
            Err(EquationError::IntegralArguments)
        );
    }

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
    fn reads_expanded_equations_from_both_sides_with_exact_coefficients() {
        for (input, expected) in [
            ("x ** 2 - 3*x + 2 = 0", "x₁ = 1 · x₂ = 2"),
            ("x² = 3*x - 2", "x₁ = 1 · x₂ = 2"),
            ("x² = 3×x - 2", "x₁ = 1 · x₂ = 2"),
            ("(x + 1)*(x - 1) = 0", "x₁ = -1 · x₂ = 1"),
            ("x² = x×-3 - 2", "x₁ = -2 · x₂ = -1"),
            ("x²×(1/2) = 2", "x₁ = -2 · x₂ = 2"),
            ("X/2 + 1 = X", "x = 2"),
            ("x/(2*3) = 1", "x = 6"),
            ("取系数 (1/3+1/6)*x^2 - x + 3/4", "1/2,-1,3/4"),
            ("coefficients 1e-3*x**2 + x*2 - 1", "1/1000,2,-1"),
            ("coefficients 0x1e+x=0", "0,1,30"),
            ("取系数 2*x + 1 = x^1 + 2", "0,1,-1"),
        ] {
            assert_eq!(solve_equation(input).as_deref(), Ok(expected), "{input}");
        }
        for invalid in [
            "x^3 = 1",
            "x² = 0 = 2",
            "x² =",
            "x/2*3 = 1",
            "x/(2)*(3) = 1",
            "x/2/3 = 1",
            "x²**2 = 1",
            "*x = 1",
            "2/x = 1",
        ] {
            assert_eq!(
                solve_equation(invalid),
                Err(EquationError::ExpressionArguments)
            );
        }
    }

    #[test]
    fn writes_polynomial_terms_with_canonical_signs_and_fractional_coefficients() {
        for (input, expected) in [
            ("标准式 -2,4,-3/2", "y = -2x² + 4x - 3/2"),
            ("standard-form 1/2,-1,3/4", "y = x²/2 - x + 3/4"),
            ("标准式 0,-1,0", "y = -x"),
            ("标准式 0,0,-1/2", "y = -1/2"),
            ("标准式 0,0,0", "y = 0"),
        ] {
            assert_eq!(solve_equation(input).as_deref(), Ok(expected), "{input}");
        }
    }

    #[test]
    fn integrates_the_finite_area_enclosed_by_two_polynomials() {
        for (input, expected) in [
            ("围成面积 1,0,0 ; 0,0,1", "4/3"),
            ("enclosed-area 0,0,2 ; 1,0,0", "8√2/3"),
            ("围成面积 1,0,0 ; 0,0,0", "没有有界区域"),
            ("围成面积 0,2,1 ; 0,-1,2", "没有有界区域"),
        ] {
            assert_eq!(solve_equation(input).as_deref(), Ok(expected), "{input}");
        }
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
            Err(EquationError::CurvePairArguments)
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
            ("不等式 x² < 3x - 2", "(1, 2)"),
            ("不等式 x**2 ≥ 2", "(-∞, -√2] ∪ [√2, ∞)"),
            ("不等式 X/2 >= 3/4", "[3/2, ∞)"),
            ("不等式 x² - 1 > x² + 2", "∅"),
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
        for query in ["不等式 x² < x < 3", "不等式 x² <= ", "不等式 x² << 2"] {
            assert_eq!(
                solve_equation(query),
                Err(EquationError::ExpressionArguments)
            );
        }
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
