use std::cmp::Ordering;

use num_bigint::{BigInt, Sign};
use num_rational::BigRational;

use super::calculator::{
    evaluate, evaluate_at_t, evaluate_at_x, format_result, format_round_trip as format_coordinate,
    normalize, sample_at_t, sample_at_x, CalculationError, QUERY_LIMIT,
};
use super::csv_tools::append_bounded_csv_row;
use super::fraction::{
    calculate_fraction, ensure_exact_size, exact_abs_square_root, format_exact_multiple,
    format_square_root, FractionError,
};
use super::matrix::{general_solution, MatrixError, MAX_DIMENSION};
use super::operators::{
    json_value_and_tail, split_top_level_commas, visit_additive_terms, NumberComparison,
};
use super::plane::{parse_point, PointError};
use super::scatter_plot::{SampledSeries, ScatterPlot};
use super::scientific::{interpolate_between, named_value, split_name};

const MAX_ROOT_CANDIDATES: usize = 64;
const FUNCTION_PLOT_INTERVALS: u32 = 32;
const MAX_FUNCTION_POINTS: u32 = 129;
const MAX_FUNCTION_INPUTS: usize = 64;
const APPROACH_SAMPLE_LAYERS: usize = 6;
const MAX_COMPARED_FUNCTIONS: usize = 4;
const BISECTION_ITERATIONS: usize = 128;
const BISECTION_RESIDUAL_RATIO: f64 = 1e-8;
const MAX_SAMPLED_BISECTIONS: usize = 8;
const MAX_INTEGRAL_INTERVALS: u32 = 128;
const MAX_EXTREMUM_REFINEMENT_STEPS: usize = 96;

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
    FunctionSampleHasNoValues,
    FunctionValuesArguments,
    FunctionApproachArguments,
    FunctionApproachStep,
    FunctionComparisonArguments,
    FunctionComparisonPlotArguments,
    ParametricSampleArguments,
    ParametricPlotHasNoPoints,
    PolarSampleArguments,
    NumericPolarAreaArguments,
    BisectionArguments,
    BisectionSameSign,
    BisectionResidual,
    TooManySampledBrackets,
    NumericIntegralArguments,
    NumericIntegralComparisonArguments,
    NumericIntervalMeanArguments,
    NumericIntegralIntervals,
    IntegralSampleArguments,
    NumericAreaArguments,
    NumericArcLengthArguments,
    NumericDerivativeArguments,
    AverageRateArguments,
    NumericDerivativeStep,
    DerivativeSampleArguments,
    NumericExtremumArguments,
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
            Self::FunctionSampleHasNoValues => "该区间没有有限的函数值",
            Self::FunctionValuesArguments => {
                "填写带 x 的算式，用 @ 隔开 1 至 64 个以逗号分隔的横坐标"
            }
            Self::FunctionApproachArguments => {
                "填写函数、中心横坐标和可选正步长，例如 sin(x)/x @ 0; 0.1"
            }
            Self::FunctionApproachStep => "该步长下无法取到中心两侧的有限横坐标",
            Self::FunctionComparisonArguments => {
                "填写 2 至 4 个算式的 JSON 数组，@ 后接横坐标，例如 [\"sin(x)\",\"cos(x)\"] @ 0,pi/2"
            }
            Self::FunctionComparisonPlotArguments => {
                "填写 2 至 4 个算式的 JSON 数组，@ 后接范围，例如 [\"sin(x)\",\"cos(x)\"] @ 0..2*pi; 33"
            }
            Self::ParametricSampleArguments => {
                "按 x、y 的顺序填写两个带 t 的算式，再填两个不同的端点和可选的 2 至 129 个采样点"
            }
            Self::ParametricPlotHasNoPoints => "该区间没有横坐标和纵坐标均有限的采样点",
            Self::PolarSampleArguments => {
                "填写以 t 为弧度角的半径算式，再填两个不同的端点和可选的 2 至 129 个采样点"
            }
            Self::NumericPolarAreaArguments => {
                "填写以 t 为弧度角的半径算式，再填不同的端点和可选的 2 至 128 的偶数子区间数"
            }
            Self::BisectionArguments => {
                "填写要求零点的算式或等式和两个不同的端点，例如 cos(x)=x @ 0 到 1"
            }
            Self::BisectionSameSign => "两端的函数值须异号，或者一端的函数值等于零",
            Self::BisectionResidual => "区间已缩小，函数值仍未接近零。请检查函数是否连续或缩小区间",
            Self::TooManySampledBrackets => "异号区间超过 8 个，请缩小范围或先用找异号区间",
            Self::NumericIntegralArguments => {
                "填写被积算式和两个不同的端点，分号后可填 2 至 128 之间的偶数"
            }
            Self::NumericIntegralComparisonArguments => {
                "填写被积算式和两个不同的端点，分号后可填 4 至 128 之间的 4 的倍数"
            }
            Self::NumericIntervalMeanArguments => {
                "填写要取区间平均值的算式、两个不同的端点和可选偶数子区间数"
            }
            Self::NumericIntegralIntervals => "当前精度无法区分积分采样点，请减少子区间数",
            Self::IntegralSampleArguments => {
                "填写被积算式和不同的区间端点，分号后可填 2 至 129 的采样点数"
            }
            Self::NumericAreaArguments => {
                "用数组填写两条算式，再填两个不同端点和可选的偶数子区间数"
            }
            Self::NumericArcLengthArguments => {
                "填写函数算式和不同的区间端点，分号后可填 2 至 128 的偶数子区间数"
            }
            Self::NumericDerivativeArguments => {
                "填写要估算导数的算式和横坐标，可在分号后填写正步长"
            }
            Self::AverageRateArguments => {
                "填写函数算式和两个不同的横坐标，例如 x² @ 1..3"
            }
            Self::NumericDerivativeStep => "该步长下无法取到两个不同的有限横坐标",
            Self::DerivativeSampleArguments => {
                "填写函数算式和不同的区间端点，分号后可填 2 至 129 的采样点数"
            }
            Self::NumericExtremumArguments => "填写函数算式和不同的区间端点，例如 (x-1)^2 @ 0..3",
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

pub(super) enum EquationOutput {
    Text(String),
    Table(String),
}

pub(super) fn solve_equation(query: &str) -> Result<String, EquationError> {
    solve_equation_output(query).map(|output| match output {
        EquationOutput::Text(text) | EquationOutput::Table(text) => text,
    })
}

pub(super) fn solve_equation_output(query: &str) -> Result<EquationOutput, EquationError> {
    let input = normalize_equation(query)?;
    let command = input
        .trim()
        .split_once(char::is_whitespace)
        .map(|(operation, arguments)| (operation.to_ascii_lowercase(), arguments));
    let command = command
        .as_ref()
        .map(|(operation, arguments)| (operation.as_str(), *arguments));
    if let Some(table) =
        command.and_then(|(operation, arguments)| solve_equation_table(operation, arguments))
    {
        return table.map(EquationOutput::Table);
    }
    solve_equation_text(&input, command).map(EquationOutput::Text)
}

fn solve_equation_table(operation: &str, arguments: &str) -> Option<Result<String, EquationError>> {
    Some(match operation {
        "函数采样" | "sample-function" => function_sample_points(arguments)
            .and_then(|samples| format_function_samples(&samples, "近似值")),
        "函数值" | "function-values" => function_samples_for_inputs(arguments)
            .and_then(|samples| format_function_samples(&samples, "近似值")),
        "两侧采样" | "approach-samples" => function_approach_samples(arguments),
        "函数对照" | "compare-functions" => compare_function_values(arguments),
        "参数采样" | "sample-parametric" => {
            parametric_samples(arguments).and_then(|samples| format_parametric_samples(&samples))
        }
        "极坐标采样" | "sample-polar" => {
            polar_samples(arguments).and_then(|samples| format_polar_samples(&samples))
        }
        "积分采样" | "sample-integral" => integral_samples(arguments)
            .and_then(|samples| format_function_samples(&samples, "累计积分")),
        "导数采样" | "sample-derivative" => derivative_samples(arguments)
            .and_then(|samples| format_function_samples(&samples, "近似导数")),
        _ => return None,
    })
}

fn solve_equation_text(
    input: &str,
    command: Option<(&str, &str)>,
) -> Result<String, EquationError> {
    if let Some((operation, arguments)) = command {
        match operation {
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
            "采样图" | "sample-plot" => {
                return format_function_samples(&function_sample_points(arguments)?, "近似值");
            }
            "采样极值" | "sample-extrema" => return sampled_function_extrema(arguments),
            "数值极小" | "numeric-minimum" => {
                return numeric_function_extremum(arguments, false)
            }
            "数值极大" | "numeric-maximum" => {
                return numeric_function_extremum(arguments, true)
            }
            "找异号区间" | "sign-change-intervals" => return sampled_sign_changes(arguments),
            "搜索零点" | "search-roots" => return refine_sampled_roots(arguments),
            "函数对照图" | "compare-function-plot" => {
                return sample_function_comparison(arguments)?.csv();
            }
            "参数图" | "parametric-plot" => {
                return format_parametric_samples(&parametric_samples(arguments)?);
            }
            "极坐标图" | "polar-plot" => {
                return format_polar_samples(&polar_samples(arguments)?);
            }
            "二分求根" | "bisect-root" => return bisect_function_root(arguments),
            "数值积分" | "numeric-integral" => return numeric_integral(arguments),
            "积分对照" | "compare-integrals" => return compare_numeric_integrals(arguments),
            "数值平均" | "numeric-mean" => return numeric_function_interval_mean(arguments),
            "数值面积" | "numeric-area" => return numeric_function_area(arguments),
            "数值弧长" | "numeric-arc-length" => return numeric_function_arc_length(arguments),
            "极坐标面积" | "polar-area" => return numeric_polar_area(arguments),
            "积分采样图" | "integral-plot" => {
                return format_function_samples(&integral_samples(arguments)?, "累计积分");
            }
            "数值导数" | "numeric-derivative" => return numeric_derivative(arguments),
            "平均变化率" | "average-rate" => return function_average_rate(arguments),
            "数值二阶导数" | "numeric-second-derivative" => {
                return numeric_second_derivative(arguments)
            }
            "导数采样图" | "derivative-plot" => {
                return format_function_samples(&derivative_samples(arguments)?, "近似导数");
            }
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
        expanded_equation_coefficients(input)?
    } else {
        coefficient_row(input)?
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

pub(super) fn equation_chart(input: &str) -> Option<Result<(String, ScatterPlot), EquationError>> {
    let (command, arguments) = input.trim().split_once(char::is_whitespace)?;
    match command.to_ascii_lowercase().as_str() {
        "函数图" | "function-plot" => Some(plot_quadratic_function(arguments)),
        "采样图" | "sample-plot" => Some(plot_sampled_function(arguments)),
        "导数采样图" | "derivative-plot" => Some(plot_sampled_derivative(arguments)),
        "积分采样图" | "integral-plot" => Some(plot_sampled_integral(arguments)),
        "函数对照图" | "compare-function-plot" => Some(plot_function_comparison(arguments)),
        "参数图" | "parametric-plot" => Some(plot_parametric_samples(arguments)),
        "极坐标图" | "polar-plot" => Some(plot_polar_samples(arguments)),
        _ => None,
    }
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

fn plot_sampled_function(input: &str) -> Result<(String, ScatterPlot), EquationError> {
    let input = normalize_equation(input)?;
    let samples = function_sample_points(&input)?;
    let plot = plot_samples(&samples)?;
    Ok((format_function_samples(&samples, "近似值")?, plot))
}

fn plot_sampled_derivative(input: &str) -> Result<(String, ScatterPlot), EquationError> {
    let input = normalize_equation(input)?;
    let samples = derivative_samples(&input)?;
    let plot = plot_samples(&samples)?.with_axis_names(["x".to_owned(), "导数".to_owned()]);
    Ok((format_function_samples(&samples, "近似导数")?, plot))
}

fn plot_sampled_integral(input: &str) -> Result<(String, ScatterPlot), EquationError> {
    let input = normalize_equation(input)?;
    let samples = integral_samples(&input)?;
    let plot = plot_samples(&samples)?.with_axis_names(["x".to_owned(), "积分".to_owned()]);
    Ok((format_function_samples(&samples, "累计积分")?, plot))
}

fn plot_samples(samples: &[(f64, Option<f64>)]) -> Result<ScatterPlot, EquationError> {
    let valid: Vec<_> = samples
        .iter()
        .filter_map(|&(x, y)| y.map(|y| (x, y)))
        .collect();
    if valid.is_empty() {
        return Err(EquationError::FunctionSampleHasNoValues);
    }
    let points = valid
        .iter()
        .map(|&(x, y)| Ok((sample_rational(x)?, sample_rational(y)?)))
        .collect::<Result<Vec<_>, EquationError>>()?;
    let first = samples.first().expect("至少有两个采样点").0;
    let last = samples.last().expect("至少有两个采样点").0;
    let x = [sample_rational(first)?, sample_rational(last)?];
    let ys = valid.iter().map(|&(_, y)| y);
    let y_min = ys
        .clone()
        .min_by(f64::total_cmp)
        .expect("至少有一个有效函数值");
    let y_max = ys.max_by(f64::total_cmp).expect("至少有一个有效函数值");
    let display = |value| format_result(value).map_err(EquationError::Approximation);
    let plot = ScatterPlot::from_sampled_function(&points, [&x[0], &x[1]], samples.len())
        .with_range_labels(
            [display(first)?, display(last)?],
            [display(y_min)?, display(y_max)?],
        );
    Ok(plot)
}

fn plot_function_comparison(input: &str) -> Result<(String, ScatterPlot), EquationError> {
    let input = normalize_equation(input)?;
    let comparison = sample_function_comparison(&input)?;
    Ok((comparison.csv()?, comparison.plot()?))
}

fn plot_parametric_samples(input: &str) -> Result<(String, ScatterPlot), EquationError> {
    let input = normalize_equation(input)?;
    let samples = parametric_samples(&input)?;
    let points = samples.iter().filter_map(|sample| sample.x.zip(sample.y));
    let plot = plot_sampled_coordinates(points, samples.len())?;
    Ok((format_parametric_samples(&samples)?, plot))
}

fn plot_polar_samples(input: &str) -> Result<(String, ScatterPlot), EquationError> {
    let input = normalize_equation(input)?;
    let samples = polar_samples(&input)?;
    let points = samples
        .iter()
        .filter_map(|sample| sample.radius_and_point.map(|[_, x, y]| (x, y)));
    let plot = plot_sampled_coordinates(points, samples.len())?;
    Ok((format_polar_samples(&samples)?, plot))
}

fn plot_sampled_coordinates(
    points: impl Iterator<Item = (f64, f64)>,
    requested: usize,
) -> Result<ScatterPlot, EquationError> {
    let points = points
        .map(|(x, y)| Ok((sample_rational(x)?, sample_rational(y)?)))
        .collect::<Result<Vec<_>, EquationError>>()?;
    if points.is_empty() {
        return Err(EquationError::ParametricPlotHasNoPoints);
    }
    Ok(ScatterPlot::from_parametric_samples(&points, requested))
}

fn sample_rational(value: f64) -> Result<BigRational, EquationError> {
    BigRational::from_float(value).ok_or(EquationError::Approximation(CalculationError::OutOfRange))
}

fn split_function_arguments(
    input: &str,
    invalid: EquationError,
) -> Result<(&str, &str), EquationError> {
    let (expression, arguments) = input.split_once('@').ok_or(invalid)?;
    let expression = match expression.split_once('=') {
        Some((label, formula)) if has_x_function_label(label) => formula.trim(),
        _ => expression.trim(),
    };
    if expression.is_empty() {
        return Err(invalid);
    }
    Ok((expression, arguments))
}

fn has_x_function_label(label: &str) -> bool {
    let label = label.trim();
    if label.eq_ignore_ascii_case("y") {
        return true;
    }
    let (name, rest) = split_name(label);
    name.starts_with(|character: char| character.is_ascii_alphabetic() || character == '_')
        && named_value(name).is_none()
        && rest
            .trim_start()
            .strip_prefix('(')
            .and_then(|arguments| arguments.strip_suffix(')'))
            .is_some_and(|argument| argument.trim().eq_ignore_ascii_case("x"))
}

fn function_sample_points(input: &str) -> Result<Vec<(f64, Option<f64>)>, EquationError> {
    let invalid = EquationError::FunctionSampleArguments;
    let (expression, bounds) = split_function_arguments(input, invalid)?;
    collect_function_samples(expression, bounds, invalid)
}

fn collect_function_samples(
    expression: &str,
    bounds: &str,
    invalid: EquationError,
) -> Result<Vec<(f64, Option<f64>)>, EquationError> {
    sampled_coordinates(bounds, invalid)?
        .map(|x| Ok((x, function_sample_value(expression, x)?)))
        .collect()
}

fn sampled_coordinates(
    bounds: &str,
    invalid: EquationError,
) -> Result<impl Iterator<Item = f64>, EquationError> {
    let (bounds, count) = match bounds.split_once(';') {
        Some((bounds, count)) => {
            let count = count
                .trim()
                .parse::<u32>()
                .ok()
                .filter(|count| (2..=MAX_FUNCTION_POINTS).contains(count))
                .ok_or(invalid)?;
            (bounds, count)
        }
        None => (bounds, 33),
    };
    let [start, end] = function_interval(bounds, invalid)?;
    let last = count - 1;
    Ok((0..count).map(move |index| sample_grid_coordinate(start, end, index, last)))
}

fn sample_grid_coordinate(start: f64, end: f64, index: u32, last: u32) -> f64 {
    let intervals = f64::from(last);
    if index == 0 || index == last || start.is_sign_negative() == end.is_sign_negative() {
        return interpolate_between(start, end, f64::from(index) / intervals);
    }

    // 用二进制可精确表示的权重，避免跨零时丢失两端贡献的抵消。
    let scale = f64::from(last.next_power_of_two());
    let left_weight = f64::from(last - index) / scale;
    let right_weight = f64::from(index) / scale;
    let left = start * left_weight;
    let right = end * right_weight;
    let error = start.mul_add(left_weight, -left) + end.mul_add(right_weight, -right);
    (((left + right) + error) * (scale / intervals)).clamp(start.min(end), start.max(end))
}

fn derivative_samples(input: &str) -> Result<Vec<(f64, Option<f64>)>, EquationError> {
    let invalid = EquationError::DerivativeSampleArguments;
    let (expression, bounds) = split_function_arguments(input, invalid)?;
    sampled_coordinates(bounds, invalid)?
        .map(|x| {
            let value = match estimate_derivative(expression, x, derivative_step(x)) {
                Ok(value) => Some(value),
                Err(
                    EquationError::NumericDerivativeStep
                    | EquationError::Approximation(
                        CalculationError::DivisionByZero
                        | CalculationError::FunctionDomain
                        | CalculationError::OutOfRange,
                    ),
                ) => None,
                Err(error) => return Err(error),
            };
            Ok((x, value))
        })
        .collect()
}

fn integral_samples(input: &str) -> Result<Vec<(f64, Option<f64>)>, EquationError> {
    let invalid = EquationError::IntegralSampleArguments;
    let (expression, bounds) = split_function_arguments(input, invalid)?;
    let mut coordinates = sampled_coordinates(bounds, invalid)?;
    let start = coordinates.next().expect("至少有两个采样点");
    let mut previous = function_sample_value(expression, start)?.map(|value| (start, value, 0.0));
    let mut samples = vec![(start, previous.map(|(_, _, total)| total))];
    for x in coordinates {
        if let Some((left, first, total)) = previous {
            let midpoint = interpolate_between(left, x, 0.5);
            let middle = function_sample_value(expression, midpoint)?;
            let last = function_sample_value(expression, x)?;
            previous = middle.zip(last).and_then(|(middle, last)| {
                let sum = total + simpson_integral(&[first, middle, last], left, x);
                sum.is_finite().then_some((x, last, sum))
            });
        }
        samples.push((x, previous.map(|(_, _, total)| total)));
    }
    Ok(samples)
}

fn function_samples_for_inputs(input: &str) -> Result<Vec<(f64, Option<f64>)>, EquationError> {
    let invalid = EquationError::FunctionValuesArguments;
    let (expression, inputs) = split_function_arguments(input, invalid)?;
    function_input_coordinates(inputs, invalid)
        .map(|x| {
            let x = x?;
            Ok((x, function_sample_value(expression, x)?))
        })
        .collect()
}

fn function_input_coordinates(
    inputs: &str,
    invalid: EquationError,
) -> impl Iterator<Item = Result<f64, EquationError>> + '_ {
    split_top_level_commas(inputs)
        .enumerate()
        .map(move |(index, input)| {
            if input.trim().is_empty() || index >= MAX_FUNCTION_INPUTS {
                return Err(invalid);
            }
            evaluate(input).map_err(EquationError::Approximation)
        })
}

fn compare_function_values(input: &str) -> Result<String, EquationError> {
    let invalid = EquationError::FunctionComparisonArguments;
    let (expressions, inputs) = function_comparison_arguments(input, invalid)?;
    FunctionComparison::evaluate(expressions, function_input_coordinates(inputs, invalid))?.csv()
}

fn sample_function_comparison(input: &str) -> Result<FunctionComparison, EquationError> {
    let invalid = EquationError::FunctionComparisonPlotArguments;
    let (expressions, bounds) = function_comparison_arguments(input, invalid)?;
    FunctionComparison::evaluate(expressions, sampled_coordinates(bounds, invalid)?.map(Ok))
}

fn function_comparison_arguments(
    input: &str,
    invalid: EquationError,
) -> Result<(Vec<String>, &str), EquationError> {
    let (expressions, tail) = json_value_and_tail::<Vec<String>>(input).ok_or(invalid)?;
    let inputs = tail.trim_start().strip_prefix('@').ok_or(invalid)?;
    if !(2..=MAX_COMPARED_FUNCTIONS).contains(&expressions.len())
        || expressions
            .iter()
            .any(|expression| expression.trim().is_empty())
    {
        return Err(invalid);
    }
    Ok((expressions, inputs))
}

struct FunctionComparison {
    expressions: Vec<String>,
    rows: Vec<(f64, Vec<Option<f64>>)>,
}

impl FunctionComparison {
    fn evaluate(
        expressions: Vec<String>,
        coordinates: impl Iterator<Item = Result<f64, EquationError>>,
    ) -> Result<Self, EquationError> {
        let mut rows = Vec::new();
        for x in coordinates {
            let x = x?;
            let ys = expressions
                .iter()
                .map(|expression| function_sample_value(expression, x))
                .collect::<Result<Vec<_>, _>>()?;
            rows.push((x, ys));
        }
        Ok(Self { expressions, rows })
    }

    fn csv(&self) -> Result<String, EquationError> {
        let invalid = EquationError::FunctionComparisonArguments;
        let display = |value| format_result(value).map_err(EquationError::Approximation);
        let mut csv = String::new();
        let labels =
            std::iter::once("x").chain(self.expressions.iter().map(|expression| expression.trim()));
        append_bounded_csv_row(&mut csv, labels, invalid)?;
        for (x, ys) in &self.rows {
            let mut row = vec![display(*x)?];
            for y in ys {
                row.push(y.map(display).transpose()?.unwrap_or_default());
            }
            append_bounded_csv_row(&mut csv, row.iter().map(String::as_str), invalid)?;
        }
        Ok(csv)
    }

    fn plot(&self) -> Result<ScatterPlot, EquationError> {
        let ys = self
            .rows
            .iter()
            .flat_map(|(_, ys)| ys.iter().copied().flatten());
        let y_min = ys
            .clone()
            .min_by(f64::total_cmp)
            .ok_or(EquationError::FunctionSampleHasNoValues)?;
        let y_max = ys.max_by(f64::total_cmp).expect("至少有一个有效函数值");
        let first = self.rows.first().expect("至少有两个采样点").0;
        let last = self.rows.last().expect("至少有两个采样点").0;
        let x = [sample_rational(first)?, sample_rational(last)?];
        let series = self
            .expressions
            .iter()
            .enumerate()
            .map(|(index, expression)| {
                let points = self
                    .rows
                    .iter()
                    .filter_map(|(x, ys)| ys[index].map(|y| (*x, y)))
                    .map(|(x, y)| Ok((sample_rational(x)?, sample_rational(y)?)))
                    .collect::<Result<Vec<_>, EquationError>>()?;
                Ok(SampledSeries {
                    label: expression.trim().to_owned(),
                    points,
                })
            })
            .collect::<Result<Vec<_>, EquationError>>()?;
        let display = |value| format_result(value).map_err(EquationError::Approximation);
        Ok(ScatterPlot::from_sampled_series(
            series,
            [&x[0], &x[1]],
            self.rows.len() * self.expressions.len(),
        )
        .with_range_labels(
            [display(first)?, display(last)?],
            [display(y_min)?, display(y_max)?],
        ))
    }
}

struct ParametricSample {
    t: f64,
    x: Option<f64>,
    y: Option<f64>,
}

fn parametric_samples(input: &str) -> Result<Vec<ParametricSample>, EquationError> {
    let invalid = EquationError::ParametricSampleArguments;
    let (expressions, bounds) = function_comparison_arguments(input, invalid)?;
    let [x, y] = expressions.as_slice() else {
        return Err(invalid);
    };
    sampled_coordinates(bounds, invalid)?
        .map(|t| {
            Ok(ParametricSample {
                t,
                x: sample_at_t(x, t).map_err(EquationError::Approximation)?,
                y: sample_at_t(y, t).map_err(EquationError::Approximation)?,
            })
        })
        .collect()
}

fn format_parametric_samples(samples: &[ParametricSample]) -> Result<String, EquationError> {
    let mut csv = String::from("t,x,y");
    for sample in samples {
        let row = [
            format_coordinate(sample.t),
            format_optional_sample_value(sample.x)?,
            format_optional_sample_value(sample.y)?,
        ];
        append_bounded_csv_row(
            &mut csv,
            row.iter().map(String::as_str),
            EquationError::ParametricSampleArguments,
        )?;
    }
    Ok(csv)
}

struct PolarSample {
    t: f64,
    radius_and_point: Option<[f64; 3]>,
}

fn polar_samples(input: &str) -> Result<Vec<PolarSample>, EquationError> {
    let invalid = EquationError::PolarSampleArguments;
    let (radius, bounds) = split_polar_arguments(input, invalid)?;
    sampled_coordinates(bounds, invalid)?
        .map(|t| {
            let radius = sample_at_t(radius, t).map_err(EquationError::Approximation)?;
            let radius_and_point = radius.map(|radius| {
                let (sin, cos) = t.sin_cos();
                [radius, radius * cos, radius * sin]
            });
            Ok(PolarSample {
                t,
                radius_and_point,
            })
        })
        .collect()
}

fn split_polar_arguments(
    input: &str,
    invalid: EquationError,
) -> Result<(&str, &str), EquationError> {
    let (radius, bounds) = input.split_once('@').ok_or(invalid)?;
    let radius = radius.trim();
    if radius.is_empty() {
        return Err(invalid);
    }
    Ok((radius, bounds))
}

fn format_polar_samples(samples: &[PolarSample]) -> Result<String, EquationError> {
    let mut csv = String::from("t,r,x,y");
    for sample in samples {
        let [radius, x, y] = sample
            .radius_and_point
            .map_or([None; 3], |values| values.map(Some));
        let row = [
            format_coordinate(sample.t),
            format_optional_sample_value(radius)?,
            format_optional_sample_value(x)?,
            format_optional_sample_value(y)?,
        ];
        append_bounded_csv_row(
            &mut csv,
            row.iter().map(String::as_str),
            EquationError::PolarSampleArguments,
        )?;
    }
    Ok(csv)
}

fn format_optional_sample_value(value: Option<f64>) -> Result<String, EquationError> {
    value
        .map(format_result)
        .transpose()
        .map(Option::unwrap_or_default)
        .map_err(EquationError::Approximation)
}

fn function_sample_value(expression: &str, x: f64) -> Result<Option<f64>, EquationError> {
    sample_at_x(expression, x).map_err(EquationError::Approximation)
}

fn function_approach_samples(input: &str) -> Result<String, EquationError> {
    let invalid = EquationError::FunctionApproachArguments;
    let (expression, arguments) = split_function_arguments(input, invalid)?;
    let (center, step) = arguments
        .split_once(';')
        .map_or((arguments, None), |(center, step)| (center, Some(step)));
    let x = evaluate(center).map_err(EquationError::Approximation)?;
    let mut step = match step {
        Some(step) => evaluate(step).map_err(EquationError::Approximation)?,
        None => x.abs().max(1.0) * 0.1,
    };
    if step <= 0.0 {
        return Err(invalid);
    }
    let header = "步长,左横坐标,左值,右横坐标,右值";
    let mut csv = String::from(header);
    let display = |value| format_result(value).map_err(EquationError::Approximation);
    for _ in 0..APPROACH_SAMPLE_LAYERS {
        let [left, right] = [x - step, x + step];
        if !left.is_finite() || !right.is_finite() || left == x || right == x {
            break;
        }
        let left_value = function_sample_value(expression, left)?
            .map(display)
            .transpose()?
            .unwrap_or_default();
        let right_value = function_sample_value(expression, right)?
            .map(display)
            .transpose()?
            .unwrap_or_default();
        let row = [
            display(step)?,
            format_coordinate(left),
            left_value,
            format_coordinate(right),
            right_value,
        ];
        append_bounded_csv_row(&mut csv, row.iter().map(String::as_str), invalid)?;
        step /= 10.0;
    }
    if csv == header {
        return Err(EquationError::FunctionApproachStep);
    }
    Ok(csv)
}

enum SampledRootCandidate {
    Zero(f64),
    Bracket([(f64, f64); 2]),
}

const NO_SAMPLED_ROOT_CANDIDATES: &str = "采样点中未发现零值或相邻异号值";

fn sampled_root_candidates(
    samples: Vec<(f64, Option<f64>)>,
) -> Result<Vec<SampledRootCandidate>, EquationError> {
    if samples.iter().all(|(_, y)| y.is_none()) {
        return Err(EquationError::FunctionSampleHasNoValues);
    }
    let mut previous: Option<(f64, f64)> = None;
    let mut candidates = Vec::new();
    for (x, y) in samples {
        let Some(y) = y else {
            previous = None;
            continue;
        };
        if previous.is_some_and(|(previous_x, _)| x == previous_x) {
            continue;
        }
        if y == 0.0 {
            candidates.push(SampledRootCandidate::Zero(x));
        } else if let Some(left) = previous.filter(|&(_, previous_y)| {
            previous_y != 0.0 && previous_y.is_sign_negative() != y.is_sign_negative()
        }) {
            candidates.push(SampledRootCandidate::Bracket([left, (x, y)]));
        }
        previous = Some((x, y));
    }
    Ok(candidates)
}

fn sampled_sign_changes(input: &str) -> Result<String, EquationError> {
    let candidates = sampled_root_candidates(function_sample_points(input)?)?;
    if candidates.is_empty() {
        return Ok(NO_SAMPLED_ROOT_CANDIDATES.to_owned());
    }
    let mut rows = vec!["类型,左端点,右端点".to_owned()];
    for candidate in candidates {
        rows.push(match candidate {
            SampledRootCandidate::Zero(x) => {
                let zero = format_coordinate(x);
                format!("采样值为零,{zero},{zero}")
            }
            SampledRootCandidate::Bracket([(left, _), (right, _)]) => {
                format!(
                    "相邻样值异号,{},{}",
                    format_coordinate(left),
                    format_coordinate(right)
                )
            }
        });
    }
    Ok(rows.join("\r\n"))
}

fn refine_sampled_roots(input: &str) -> Result<String, EquationError> {
    let invalid = EquationError::FunctionSampleArguments;
    let (expression, bounds) = split_function_arguments(input, invalid)?;
    let candidates =
        sampled_root_candidates(collect_function_samples(expression, bounds, invalid)?)?;
    if candidates.is_empty() {
        return Ok(NO_SAMPLED_ROOT_CANDIDATES.to_owned());
    }
    let brackets = candidates
        .iter()
        .filter(|candidate| matches!(candidate, SampledRootCandidate::Bracket(_)))
        .count();
    if brackets > MAX_SAMPLED_BISECTIONS {
        return Err(EquationError::TooManySampledBrackets);
    }
    let value_at = |x| evaluate_at_x(expression, x).map_err(EquationError::Approximation);
    let mut csv = String::from("横坐标,残差,左端点,右端点,状态");
    for candidate in candidates {
        let ([left, right], root, status) = match candidate {
            SampledRootCandidate::Zero(x) => ([x, x], Some((x, 0.0)), "采样值为零"),
            SampledRootCandidate::Bracket(bounds) => {
                let range = [bounds[0].0, bounds[1].0];
                match bisect_root(bounds, value_at) {
                    Ok(root) => (range, Some(root), "近似根"),
                    Err(EquationError::BisectionResidual) => (range, None, "残差未收敛"),
                    Err(EquationError::Approximation(
                        error @ (CalculationError::DivisionByZero
                        | CalculationError::FunctionDomain
                        | CalculationError::OutOfRange),
                    )) => (range, None, error.message()),
                    Err(error) => return Err(error),
                }
            }
        };
        let (x, residual) = match root {
            Some((x, residual)) => (
                format_coordinate(x),
                format_result(residual).map_err(EquationError::Approximation)?,
            ),
            None => (String::new(), String::new()),
        };
        let left = format_coordinate(left);
        let right = format_coordinate(right);
        append_bounded_csv_row(
            &mut csv,
            [x.as_str(), residual.as_str(), &left, &right, status].into_iter(),
            invalid,
        )?;
    }
    Ok(csv)
}

fn sampled_function_extrema(input: &str) -> Result<String, EquationError> {
    let samples = function_sample_points(input)?;
    let mut finite = samples.iter().filter_map(|&(x, y)| y.map(|y| (x, y)));
    let first = finite
        .next()
        .ok_or(EquationError::FunctionSampleHasNoValues)?;
    let mut minimum = first;
    let mut maximum = first;
    let mut count = 1;
    for (x, y) in finite {
        count += 1;
        if y < minimum.1 {
            minimum = (x, y);
        }
        if y > maximum.1 {
            maximum = (x, y);
        }
    }
    let display = |value| format_result(value).map_err(EquationError::Approximation);
    Ok(format!(
        "最小样值 {} · x = {}\r\n最大样值 {} · x = {}\r\n有效样点 {count}/{}",
        display(minimum.1)?,
        display(minimum.0)?,
        display(maximum.1)?,
        display(maximum.0)?,
        samples.len(),
    ))
}

fn numeric_function_extremum(input: &str, maximum: bool) -> Result<String, EquationError> {
    let invalid = EquationError::NumericExtremumArguments;
    let (expression, bounds) = split_function_arguments(input, invalid)?;
    let [mut left, mut right] = function_interval(bounds, invalid)?;
    let evaluate_at = |x| evaluate_at_x(expression, x).map_err(EquationError::Approximation);
    let better = |first, second| {
        if maximum {
            first > second
        } else {
            first < second
        }
    };

    let mut best = (left, evaluate_at(left)?);
    let mut candidate = |x| {
        let y = evaluate_at(x)?;
        if better(y, best.1) || (y == best.1 && x < best.0) {
            best = (x, y);
        }
        Ok::<_, EquationError>(y)
    };
    candidate(right)?;
    for _ in 0..MAX_EXTREMUM_REFINEMENT_STEPS {
        let first = interpolate_between(left, right, 1. / 3.);
        let second = interpolate_between(left, right, 2. / 3.);
        if !(left < first && first < second && second < right) {
            break;
        }
        let first_value = candidate(first)?;
        let second_value = candidate(second)?;
        // 在单谷或单峰区间舍弃较差的一侧。
        if better(first_value, second_value) {
            right = second;
        } else if better(second_value, first_value) {
            left = first;
        } else {
            left = first;
            right = second;
        }
    }
    let (x, value) = best;
    let x = format_coordinate(x);
    let value = format_result(value).map_err(EquationError::Approximation)?;
    let label = if maximum { "极大值" } else { "极小值" };
    Ok(format!("x ≈ {x}\r\n{label} ≈ {value}"))
}

fn format_function_samples(
    samples: &[(f64, Option<f64>)],
    heading: &str,
) -> Result<String, EquationError> {
    let display = |value| format_result(value).map_err(EquationError::Approximation);
    let mut csv = format!("x,{heading}");
    for &(x, y) in samples {
        let x = display(x)?;
        let y = y.map(display).transpose()?.unwrap_or_default();
        csv.push_str(&format!("\r\n{x},{y}"));
    }
    Ok(csv)
}

fn function_interval(input: &str, invalid: EquationError) -> Result<[f64; 2], EquationError> {
    let [start, end] = function_endpoints(input, invalid)?;
    Ok([start.min(end), start.max(end)])
}

fn function_endpoints(input: &str, invalid: EquationError) -> Result<[f64; 2], EquationError> {
    let (start, end) = split_equation_interval(input).ok_or(invalid)?;
    let start = evaluate(start).map_err(EquationError::Approximation)?;
    let end = evaluate(end).map_err(EquationError::Approximation)?;
    if start == end {
        return Err(invalid);
    }
    Ok([start, end])
}

fn split_equation_interval(input: &str) -> Option<(&str, &str)> {
    if let Some(bounds) = input
        .split_once(['到', '至'])
        .or_else(|| input.split_once(".."))
    {
        return Some(bounds);
    }
    input
        .match_indices(char::is_whitespace)
        .find_map(|(offset, separator)| {
            let tail = input[offset + separator.len()..].trim_start();
            let token_end = tail.find(char::is_whitespace).unwrap_or(tail.len());
            tail[..token_end]
                .eq_ignore_ascii_case("to")
                .then_some((&input[..offset], &tail[token_end..]))
        })
}

fn bisect_function_root(input: &str) -> Result<String, EquationError> {
    let invalid = EquationError::BisectionArguments;
    let (expression, interval) = split_function_arguments(input, invalid)?;
    let [left, right] = function_interval(interval, invalid)?;
    let expression = expression
        .strip_prefix('=')
        .unwrap_or(expression)
        .trim_start();
    let (first, second) = match expression
        .split_once("==")
        .or_else(|| expression.split_once('='))
    {
        Some((first, second)) => {
            if first.contains('=') || second.contains('=') {
                return Err(invalid);
            }
            (first, Some(second))
        }
        None => (expression, None),
    };
    let residual_name = if second.is_some() {
        "|左式 − 右式|"
    } else {
        "|f(x)|"
    };
    let value_at = |x| {
        let first = evaluate_at_x(first, x).map_err(EquationError::Approximation)?;
        let second = second
            .map(|expression| evaluate_at_x(expression, x))
            .transpose()
            .map_err(EquationError::Approximation)?
            .unwrap_or_default();
        let residual = first - second;
        if !residual.is_finite() {
            return Err(EquationError::Approximation(CalculationError::OutOfRange));
        }
        Ok(residual)
    };
    let bounds = [(left, value_at(left)?), (right, value_at(right)?)];
    let (x, residual) = bisect_root(bounds, value_at)?;
    format_bisection_result(x, residual, residual_name)
}

fn bisect_root(
    bounds: [(f64, f64); 2],
    value_at: impl Fn(f64) -> Result<f64, EquationError>,
) -> Result<(f64, f64), EquationError> {
    let [(mut left, mut left_value), (mut right, mut right_value)] = bounds;
    if left_value == 0.0 {
        return Ok((left, 0.0));
    }
    if right_value == 0.0 {
        return Ok((right, 0.0));
    }
    if left_value.is_sign_negative() == right_value.is_sign_negative() {
        return Err(EquationError::BisectionSameSign);
    }
    let initial_residual = left_value.abs().min(right_value.abs());
    for _ in 0..BISECTION_ITERATIONS {
        let middle = left.midpoint(right);
        if middle == left || middle == right {
            break;
        }
        let value = value_at(middle)?;
        if value == 0.0 {
            return Ok((middle, 0.0));
        }
        if value.is_sign_negative() == left_value.is_sign_negative() {
            (left, left_value) = (middle, value);
        } else {
            (right, right_value) = (middle, value);
        }
    }
    let (x, residual) = if left_value.abs() <= right_value.abs() {
        (left, left_value.abs())
    } else {
        (right, right_value.abs())
    };
    if residual / initial_residual > BISECTION_RESIDUAL_RATIO {
        return Err(EquationError::BisectionResidual);
    }
    Ok((x, residual))
}

fn format_bisection_result(
    x: f64,
    residual: f64,
    residual_name: &str,
) -> Result<String, EquationError> {
    let x = format_coordinate(x);
    let residual = format_result(residual).map_err(EquationError::Approximation)?;
    Ok(format!("x ≈ {x}\r\n残差 {residual_name} ≈ {residual}"))
}

fn function_average_rate(input: &str) -> Result<String, EquationError> {
    let invalid = EquationError::AverageRateArguments;
    let (expression, arguments) = split_function_arguments(input, invalid)?;
    let [first_x, last_x] = function_endpoints(arguments, invalid)?;
    let value_at = |x| evaluate_at_x(expression, x).map_err(EquationError::Approximation);
    let first_y = value_at(first_x)?;
    let last_y = value_at(last_x)?;
    let slope = difference_quotient(first_x, last_x, first_y, last_y);
    let display = |value| format_result(value).map_err(EquationError::Approximation);
    Ok(format!(
        "平均变化率 ≈ {}\r\nx = {} → {}\r\nf(x) = {} → {}",
        display(slope)?,
        format_coordinate(first_x),
        format_coordinate(last_x),
        display(first_y)?,
        display(last_y)?
    ))
}

fn numeric_derivative(input: &str) -> Result<String, EquationError> {
    let (expression, x, step) = derivative_input(input, derivative_step)?;
    let value = estimate_derivative(expression, x, step)?;
    derivative_report("中心差分", value, x, step)
}

fn numeric_second_derivative(input: &str) -> Result<String, EquationError> {
    let default_step = |x: f64| f64::EPSILON.sqrt().sqrt() * x.abs().max(1.0);
    let (expression, x, step) = derivative_input(input, default_step)?;
    let [lower, upper] = derivative_coordinates(x, step)?;
    let value_at =
        |position| evaluate_at_x(expression, position).map_err(EquationError::Approximation);
    let center = value_at(x)?;
    let left = difference_quotient(lower, x, value_at(lower)?, center);
    let right = difference_quotient(x, upper, center, value_at(upper)?);
    let value = 2.0 * difference_quotient(lower, upper, left, right);
    derivative_report("二阶中心差分", value, x, step)
}

fn derivative_input(
    input: &str,
    default_step: impl Fn(f64) -> f64,
) -> Result<(&str, f64, f64), EquationError> {
    let invalid = EquationError::NumericDerivativeArguments;
    let (expression, arguments) = split_function_arguments(input, invalid)?;
    let (center, step) = arguments
        .split_once(';')
        .map_or((arguments, None), |(center, step)| (center, Some(step)));
    let x = evaluate(center).map_err(EquationError::Approximation)?;
    let step = match step {
        Some(step) => evaluate(step).map_err(EquationError::Approximation)?,
        None => default_step(x),
    };
    if step <= 0.0 {
        return Err(invalid);
    }
    Ok((expression, x, step))
}

fn derivative_report(label: &str, value: f64, x: f64, step: f64) -> Result<String, EquationError> {
    let display = |value| format_result(value).map_err(EquationError::Approximation);
    Ok(format!(
        "{label} ≈ {}\r\nx = {} · 步长 {}",
        display(value)?,
        display(x)?,
        display(step)?
    ))
}

fn derivative_step(x: f64) -> f64 {
    f64::EPSILON.cbrt() * x.abs().max(1.0)
}

fn derivative_coordinates(x: f64, step: f64) -> Result<[f64; 2], EquationError> {
    let lower = x - step;
    let upper = x + step;
    if lower == x || upper == x || !lower.is_finite() || !upper.is_finite() {
        return Err(EquationError::NumericDerivativeStep);
    }
    Ok([lower, upper])
}

fn estimate_derivative(expression: &str, x: f64, step: f64) -> Result<f64, EquationError> {
    let [lower, upper] = derivative_coordinates(x, step)?;
    let value_at =
        |position| evaluate_at_x(expression, position).map_err(EquationError::Approximation);
    value_at(x)?;
    let derivative = difference_quotient(lower, upper, value_at(lower)?, value_at(upper)?);
    if derivative.is_finite() {
        Ok(derivative)
    } else {
        Err(EquationError::Approximation(CalculationError::OutOfRange))
    }
}

fn difference_quotient(start: f64, end: f64, first: f64, second: f64) -> f64 {
    let delta_y = second - first;
    let delta_x = end - start;
    if delta_y.is_finite() && delta_x.is_finite() {
        delta_y / delta_x
    } else {
        (second * 0.5 - first * 0.5) / (end * 0.5 - start * 0.5)
    }
}

fn numeric_integral(input: &str) -> Result<String, EquationError> {
    let invalid = EquationError::NumericIntegralArguments;
    let (expression, arguments) = split_function_arguments(input, invalid)?;
    let (bounds, intervals) = numeric_integral_range(arguments, invalid)?;
    let value = integrate_samples(bounds, intervals, |x| {
        evaluate_at_x(expression, x).map_err(EquationError::Approximation)
    })?;
    display_integral("积分", value, intervals)
}

fn numeric_function_interval_mean(input: &str) -> Result<String, EquationError> {
    let invalid = EquationError::NumericIntervalMeanArguments;
    let (expression, arguments) = split_function_arguments(input, invalid)?;
    let ([left, right], intervals) = numeric_integral_range(arguments, invalid)?;
    let values = collect_integral_values([left.min(right), left.max(right)], intervals, |x| {
        evaluate_at_x(expression, x).map_err(EquationError::Approximation)
    })?;
    let (mean, scale) = scaled_simpson_mean(&values);
    display_integral("区间平均值", mean * scale, intervals)
}

fn compare_numeric_integrals(input: &str) -> Result<String, EquationError> {
    let invalid = EquationError::NumericIntegralComparisonArguments;
    let (expression, arguments) = split_function_arguments(input, invalid)?;
    let ([start, end], fine_count) = numeric_integral_range(arguments, invalid)?;
    if !fine_count.is_multiple_of(4) {
        return Err(invalid);
    }
    let fine_values = collect_integral_values([start, end], fine_count, |x| {
        evaluate_at_x(expression, x).map_err(EquationError::Approximation)
    })?;
    let coarse_values: Vec<_> = fine_values.iter().step_by(2).copied().collect();
    let fine = simpson_integral(&fine_values, start, end);
    let coarse = simpson_integral(&coarse_values, start, end);
    let display = |value| format_result(value).map_err(EquationError::Approximation);
    Ok(format!(
        "复合辛普森法\r\n{fine_count} 个子区间 ≈ {}\r\n{} 个子区间 ≈ {}\r\n两次结果差 ≈ {}",
        display(fine)?,
        fine_count / 2,
        display(coarse)?,
        display((fine - coarse).abs())?
    ))
}

fn numeric_function_area(input: &str) -> Result<String, EquationError> {
    let invalid = EquationError::NumericAreaArguments;
    let (expressions, bounds) = function_comparison_arguments(input, invalid)?;
    let [first, second] = expressions.as_slice() else {
        return Err(invalid);
    };
    let ([left, right], intervals) = numeric_integral_range(bounds, invalid)?;
    let value = integrate_samples([left.min(right), left.max(right)], intervals, |x| {
        let value_at =
            |expression| evaluate_at_x(expression, x).map_err(EquationError::Approximation);
        let distance = (value_at(first)? - value_at(second)?).abs();
        if distance.is_finite() {
            Ok(distance)
        } else {
            Err(EquationError::Approximation(CalculationError::OutOfRange))
        }
    })?;
    display_integral("面积", value, intervals)
}

fn numeric_function_arc_length(input: &str) -> Result<String, EquationError> {
    let invalid = EquationError::NumericArcLengthArguments;
    let (expression, bounds) = split_function_arguments(input, invalid)?;
    let ([left, right], intervals) = numeric_integral_range(bounds, invalid)?;
    let value = integrate_samples([left.min(right), left.max(right)], intervals, |x| {
        estimate_derivative(expression, x, derivative_step(x)).map(|slope| slope.hypot(1.0))
    })?;
    display_integral("弧长", value, intervals)
}

fn numeric_polar_area(input: &str) -> Result<String, EquationError> {
    let invalid = EquationError::NumericPolarAreaArguments;
    let (radius, bounds) = split_polar_arguments(input, invalid)?;
    let ([left, right], intervals) = numeric_integral_range(bounds, invalid)?;
    let value = integrate_samples([left.min(right), left.max(right)], intervals, |t| {
        let radius = evaluate_at_t(radius, t).map_err(EquationError::Approximation)?;
        let sector = (radius * 0.5) * radius;
        if sector.is_finite() {
            Ok(sector)
        } else {
            Err(EquationError::Approximation(CalculationError::OutOfRange))
        }
    })?;
    display_integral("极坐标面积", value, intervals)
}

fn numeric_integral_range(
    arguments: &str,
    invalid: EquationError,
) -> Result<([f64; 2], u32), EquationError> {
    let (bounds, intervals) = match arguments.split_once(';') {
        Some((bounds, intervals)) => {
            let intervals = intervals
                .trim()
                .parse::<u32>()
                .ok()
                .filter(|count| {
                    (2..=MAX_INTEGRAL_INTERVALS).contains(count) && count.is_multiple_of(2)
                })
                .ok_or(invalid)?;
            (bounds, intervals)
        }
        None => (arguments, 64),
    };
    Ok((function_endpoints(bounds, invalid)?, intervals))
}

fn integrate_samples(
    bounds: [f64; 2],
    intervals: u32,
    value_at: impl Fn(f64) -> Result<f64, EquationError>,
) -> Result<f64, EquationError> {
    let values = collect_integral_values(bounds, intervals, value_at)?;
    Ok(simpson_integral(&values, bounds[0], bounds[1]))
}

fn collect_integral_values(
    [start, end]: [f64; 2],
    intervals: u32,
    value_at: impl Fn(f64) -> Result<f64, EquationError>,
) -> Result<Vec<f64>, EquationError> {
    let mut values = Vec::with_capacity(intervals as usize + 1);
    let mut previous = None;
    for index in 0..=intervals {
        let x = sample_grid_coordinate(start, end, index, intervals);
        if previous == Some(x) {
            return Err(EquationError::NumericIntegralIntervals);
        }
        previous = Some(x);
        values.push(value_at(x)?);
    }
    Ok(values)
}

fn display_integral(label: &str, value: f64, intervals: u32) -> Result<String, EquationError> {
    let display = format_result(value).map_err(EquationError::Approximation)?;
    Ok(format!(
        "{label} ≈ {display}\r\n复合辛普森法 · {intervals} 个子区间"
    ))
}

fn simpson_integral(values: &[f64], start: f64, end: f64) -> f64 {
    let (mean, scale) = scaled_simpson_mean(values);
    if scale == 0.0 {
        return 0.0;
    }
    let span = end - start;
    let (width, width_scale) = if span.is_finite() {
        (span, 1.0)
    } else {
        // 区间横跨很大的正负端点时，先乘半宽再乘二。
        (end * 0.5 - start * 0.5, 2.0)
    };
    let integral = if width.abs() <= 1.0 || scale <= 1.0 {
        mean * (width * scale)
    } else if width.abs() <= scale {
        (mean * width) * scale
    } else {
        (mean * scale) * width
    };
    integral * width_scale
}

fn scaled_simpson_mean(values: &[f64]) -> (f64, f64) {
    let intervals = values.len() - 1;
    let scale = values.iter().map(|value| value.abs()).fold(0.0, f64::max);
    if scale == 0.0 {
        return (0.0, 0.0);
    }
    let weighted_sum: f64 = values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let weight = if index == 0 || index == intervals {
                1.0
            } else if index.is_multiple_of(2) {
                2.0
            } else {
                4.0
            };
            weight * (value / scale)
        })
        .sum();
    (
        (weighted_sum / (3.0 * intervals as f64)).clamp(-1.0, 1.0),
        scale,
    )
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
    let (start, end) = split_equation_interval(bounds).ok_or(invalid)?;
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
    fn preserves_the_zero_grid_point_and_distinguishes_nearby_nonzero_coordinates() {
        let samples = function_sample_points("sqrt(x) @ -1..2; 13").unwrap();
        assert_eq!(samples[4], (0.0, Some(0.0)));
        assert!(sample_grid_coordinate(-1.0, 2.0000000000000004, 4, 12) > 0.0);
        assert_eq!(
            sample_grid_coordinate(-f64::MAX / 2.0, f64::MAX, 4, 12),
            0.0
        );
        assert_eq!(sample_grid_coordinate(2.0, -1.0, 8, 12), 0.0);
    }

    #[test]
    fn keeps_rounded_samples_inside_very_narrow_large_intervals() {
        let start = 1.7976931348623155e308;
        let end = f64::MAX;
        let samples =
            function_sample_points("x @ 1.7976931348623155e308 到 1.7976931348623157e308; 10")
                .unwrap();
        assert_eq!(samples.len(), 10);
        assert_eq!(samples.first().unwrap().0, start);
        assert_eq!(samples.last().unwrap().0, end);
        assert!(samples
            .iter()
            .all(|&(x, y)| (start..=end).contains(&x) && y == Some(x)));
        assert!(samples.windows(2).all(|pair| pair[0].0 <= pair[1].0));
    }

    #[test]
    fn plots_only_finite_calculator_samples_and_preserves_blank_csv_values() {
        let (csv, plot) = equation_chart("采样图 sqrt(x) @ -1 到 1; 3")
            .unwrap()
            .unwrap();
        assert_eq!(csv, "x,近似值\r\n-1,\r\n0,0\r\n1,1");
        assert_eq!(plot.point_count(), 2);
        assert!(!plot.has_curve());
        assert!(matches!(
            equation_chart("sample-plot sqrt(x) @ -2 到 -1; 2"),
            Some(Err(EquationError::FunctionSampleHasNoValues))
        ));
        assert_eq!(
            solve_equation("采样图 1/x @ -1 到 1; 3").as_deref(),
            Ok("x,近似值\r\n-1,-1\r\n0,\r\n1,1")
        );
    }

    #[test]
    fn accepts_dot_ranges_and_english_separators_in_equation_intervals() {
        assert_eq!(
            solve_equation("函数采样 x**2 @ -1.5..0.5; 3").as_deref(),
            Ok("x,近似值\r\n-1.5,2.25\r\n-0.5,0.25\r\n0.5,0.25")
        );
        assert_eq!(solve_equation("定积分 x² @ 0..2").as_deref(), Ok("8/3"));
        assert_eq!(solve_equation("integral x² @ 0 TO 2").as_deref(), Ok("8/3"));
        assert_eq!(
            solve_equation("numeric-integral x @ 2 to 0; 4").as_deref(),
            Ok("积分 ≈ -2\r\n复合辛普森法 · 4 个子区间")
        );
        assert_eq!(
            solve_equation("函数采样 x @ 0to1; 3"),
            Err(EquationError::FunctionSampleArguments)
        );
    }

    #[test]
    fn accepts_named_one_variable_function_labels_in_calculator_equations() {
        assert_eq!(
            solve_equation("函数采样 f(x) = x²+1 @ 0 到 2; 3").as_deref(),
            Ok("x,近似值\r\n0,1\r\n1,2\r\n2,5")
        );
        assert_eq!(
            solve_equation("函数值 h_1 ( X ) = cosd(x) @ 180,0,90").as_deref(),
            Ok("x,近似值\r\n180,-1\r\n0,1\r\n90,0")
        );
        for query in ["函数值 f(y) = x @ 1", "函数值 cos(x) = x @ 1"] {
            assert_eq!(
                solve_equation(query),
                Err(EquationError::Approximation(CalculationError::Invalid))
            );
        }
    }

    #[test]
    fn samples_integrals_from_the_lower_endpoint_without_crossing_undefined_spans() {
        let input = "f(x)=x @ 2..0; 3";
        let expected = "x,累计积分\r\n0,0\r\n1,0.5\r\n2,2";
        assert_eq!(
            solve_equation(&format!("积分采样 {input}")).as_deref(),
            Ok(expected)
        );
        let (csv, plot) = plot_sampled_integral(input).unwrap();
        assert_eq!(csv, expected);
        assert_eq!(plot.point_count(), 3);
        assert_eq!(
            solve_equation("sample-integral 1/(x-1) @ 0..4; 3").as_deref(),
            Ok("x,累计积分\r\n0,0\r\n2,\r\n4,")
        );
        assert_eq!(
            solve_equation("积分采样 sqrt(x) @ -1..1; 3").as_deref(),
            Ok("x,累计积分\r\n-1,\r\n0,\r\n1,")
        );
    }

    #[test]
    fn samples_derivatives_skips_domain_gaps_and_keeps_chart_data() {
        assert_eq!(
            solve_equation("导数采样 x @ -1..1; 3").as_deref(),
            Ok("x,近似导数\r\n-1,1\r\n0,1\r\n1,1")
        );
        let input = "f(x)=sqrt(x) @ -1..1; 5";
        let expected = solve_equation(&format!("导数采样 {input}")).unwrap();
        assert!(expected.starts_with("x,近似导数\r\n-1,\r\n-0.5,\r\n0,\r\n0.5,"));
        let (csv, plot) = plot_sampled_derivative(input).unwrap();
        assert_eq!(csv, expected);
        assert_eq!(plot.point_count(), 2);
        assert_eq!(
            solve_equation("sample-derivative unknown(x) @ 0..1; 2"),
            Err(EquationError::Approximation(CalculationError::Invalid))
        );
    }

    #[test]
    fn estimates_second_derivatives_and_preserves_the_point_and_step() {
        assert_eq!(
            solve_equation("数值二阶导数 x^2 @ 2; 0.5").as_deref(),
            Ok("二阶中心差分 ≈ 2\r\nx = 2 · 步长 0.5")
        );
        let default = solve_equation("numeric-second-derivative x³ @ 2").unwrap();
        let (value, context) = default.split_once("\r\n").unwrap();
        let value = value
            .strip_prefix("二阶中心差分 ≈ ")
            .unwrap()
            .parse::<f64>()
            .unwrap();
        assert!((value - 12.0).abs() < 1e-6);
        assert_eq!(context, "x = 2 · 步长 0.000244140625");
        assert_eq!(
            solve_equation("数值二阶导数 sqrt(x) @ 0; 0.01"),
            Err(EquationError::Approximation(
                CalculationError::FunctionDomain
            ))
        );
    }

    #[test]
    fn estimates_derivatives_from_distinct_finite_samples() {
        let result = solve_equation("数值导数 x³ @ 2").unwrap();
        let derivative: f64 = result
            .lines()
            .next()
            .unwrap()
            .strip_prefix("中心差分 ≈ ")
            .unwrap()
            .parse()
            .unwrap();
        assert!((derivative - 12.0).abs() < 1e-7);
        assert_eq!(
            solve_equation("numeric-derivative 1e308*x @ 0; 1").as_deref(),
            Ok("中心差分 ≈ 1e308\r\nx = 0 · 步长 1")
        );
        assert_eq!(
            solve_equation("数值导数 1/x @ 0; 0.01"),
            Err(EquationError::Approximation(
                CalculationError::DivisionByZero
            ))
        );
        assert_eq!(
            solve_equation("数值导数 x @ 1e30; 1"),
            Err(EquationError::NumericDerivativeStep)
        );
    }

    #[test]
    fn finds_the_secant_slope_in_either_endpoint_order() {
        assert_eq!(
            solve_equation("平均变化率 f(x)=x² @ 3..1").as_deref(),
            Ok("平均变化率 ≈ 4\r\nx = 3 → 1\r\nf(x) = 9 → 1")
        );
        assert_eq!(
            solve_equation("average-rate x @ -1e308..1e308").as_deref(),
            Ok("平均变化率 ≈ 1\r\nx = -1e308 → 1e308\r\nf(x) = -1e308 → 1e308")
        );
        assert_eq!(
            solve_equation("平均变化率 x @ 1..1"),
            Err(EquationError::AverageRateArguments)
        );
        assert_eq!(
            solve_equation("平均变化率 sqrt(x) @ -1..1"),
            Err(EquationError::Approximation(
                CalculationError::FunctionDomain
            ))
        );
    }

    #[test]
    fn integrates_the_area_swept_by_polar_radii_with_unordered_endpoints() {
        assert_eq!(
            solve_equation("极坐标面积 1 @ 2*pi..0; 8").as_deref(),
            Ok("极坐标面积 ≈ 3.14159265358979\r\n复合辛普森法 · 8 个子区间")
        );
        assert_eq!(
            solve_equation("polar-area -2 @ 0..pi; 8").as_deref(),
            Ok("极坐标面积 ≈ 6.28318530717959\r\n复合辛普森法 · 8 个子区间")
        );
        assert_eq!(
            solve_equation("极坐标面积 1/t @ -1..1; 4"),
            Err(EquationError::Approximation(
                CalculationError::DivisionByZero
            ))
        );
        assert_eq!(
            solve_equation("极坐标面积 1 @ 0..pi; 3"),
            Err(EquationError::NumericPolarAreaArguments)
        );
    }

    #[test]
    fn samples_polar_curves_with_radian_angles_and_signed_radii() {
        let command = "极坐标图 1+cos(t) @ 0..2*pi; 65";
        let (csv, plot) = equation_chart(command).unwrap().unwrap();
        assert_eq!(plot.point_count(), 65);
        assert!(csv.starts_with("t,r,x,y\r\n0,2,2,0\r\n"));
        assert_eq!(csv, solve_equation(command).unwrap());
        let samples = solve_equation("极坐标采样 sqrt(t) @ -1..1; 3").unwrap();
        assert!(samples.starts_with("t,r,x,y\r\n-1,\"\",\"\",\"\"\r\n0,0,0,0\r\n"));
        let signed = solve_equation("sample-polar -2 @ 0..1; 2").unwrap();
        assert!(signed.starts_with("t,r,x,y\r\n0,-2,-2,0\r\n"));
        assert_eq!(
            solve_equation("极坐标采样 @ 0..1"),
            Err(EquationError::PolarSampleArguments)
        );
    }

    #[test]
    fn parametric_chart_only_draws_complete_points_and_keeps_all_csv_rows() {
        let command = r#"参数图 ["sqrt(t)","t²"] @ -1..1; 3"#;
        let (csv, plot) = equation_chart(command).unwrap().unwrap();
        assert_eq!(plot.point_count(), 2);
        assert_eq!(csv, solve_equation(command).unwrap());
        assert!(csv.contains("-1,\"\",1"));
        assert!(matches!(
            equation_chart(r#"parametric-plot ["sqrt(t)","t"] @ -2..-1; 3"#),
            Some(Err(EquationError::ParametricPlotHasNoPoints))
        ));
    }

    #[test]
    fn samples_parametric_coordinates_on_the_same_parameter_grid() {
        assert_eq!(
            solve_equation(r#"参数采样 ["cosd(t)","sind(T)"] @ 360..0; 5"#).as_deref(),
            Ok("t,x,y\r\n0,1,0\r\n90,0,1\r\n180,-1,0\r\n270,0,-1\r\n360,1,0")
        );
        assert_eq!(
            solve_equation(r#"sample-parametric ["sqrt(t)","t²"] @ -1..1; 3"#).as_deref(),
            Ok("t,x,y\r\n-1,\"\",1\r\n0,0,0\r\n1,1,1")
        );
        assert_eq!(
            solve_equation(r#"参数采样 ["t"] @ 0..1"#),
            Err(EquationError::ParametricSampleArguments)
        );
        assert_eq!(
            solve_equation(r#"参数采样 ["x","t"] @ 0..1"#),
            Err(EquationError::Approximation(CalculationError::Invalid))
        );
    }

    #[test]
    fn estimates_arc_length_for_linear_functions_and_reports_unavailable_derivatives() {
        assert_eq!(
            solve_equation("数值弧长 f(x)=x @ 1..0; 8").as_deref(),
            Ok("弧长 ≈ 1.4142135623731\r\n复合辛普森法 · 8 个子区间")
        );
        assert_eq!(
            solve_equation("numeric-arc-length 4 @ -2 to 2").as_deref(),
            Ok("弧长 ≈ 4\r\n复合辛普森法 · 64 个子区间")
        );
        assert_eq!(
            solve_equation("数值弧长 sqrt(x) @ 0..2"),
            Err(EquationError::Approximation(
                CalculationError::FunctionDomain
            ))
        );
        assert_eq!(
            solve_equation("数值弧长 x @ 0..2; 3"),
            Err(EquationError::NumericArcLengthArguments)
        );
    }

    #[test]
    fn takes_the_simpson_average_without_forming_an_overflowing_integral() {
        assert_eq!(
            solve_equation("数值平均 x @ 2..0; 4").as_deref(),
            Ok("区间平均值 ≈ 1\r\n复合辛普森法 · 4 个子区间")
        );
        assert_eq!(
            solve_equation("numeric-mean 1e308 @ -1e308..1e308; 4").as_deref(),
            Ok("区间平均值 ≈ 1e308\r\n复合辛普森法 · 4 个子区间")
        );
        assert_eq!(
            solve_equation("数值平均 0 @ -1..1").as_deref(),
            Ok("区间平均值 ≈ 0\r\n复合辛普森法 · 64 个子区间")
        );
        assert_eq!(
            solve_equation("数值平均 1/x @ -1..1; 4"),
            Err(EquationError::Approximation(
                CalculationError::DivisionByZero
            ))
        );
    }

    #[test]
    fn compares_simpson_estimates_at_two_nested_sampling_resolutions() {
        let report = solve_equation("积分对照 x^4 @ 1..0; 4").unwrap();
        let mut lines = report.lines();
        assert_eq!(lines.next(), Some("复合辛普森法"));
        assert_eq!(lines.next(), Some("4 个子区间 ≈ -0.200520833333333"));
        assert_eq!(lines.next(), Some("2 个子区间 ≈ -0.208333333333333"));
        let difference: f64 = lines
            .next()
            .unwrap()
            .strip_prefix("两次结果差 ≈ ")
            .unwrap()
            .parse()
            .unwrap();
        assert!((difference - 0.0078125).abs() < 1e-14);
        assert_eq!(
            solve_equation("compare-integrals 4 @ 0..1").as_deref(),
            Ok("复合辛普森法\r\n64 个子区间 ≈ 4\r\n32 个子区间 ≈ 4\r\n两次结果差 ≈ 0")
        );
        assert_eq!(
            solve_equation("积分对照 x @ 0..1; 6"),
            Err(EquationError::NumericIntegralComparisonArguments)
        );
        assert_eq!(
            solve_equation("积分对照 1/x @ -1..1; 4"),
            Err(EquationError::Approximation(
                CalculationError::DivisionByZero
            ))
        );
    }

    #[test]
    fn estimates_the_area_between_crossing_curves_in_either_endpoint_order() {
        assert_eq!(
            solve_equation(r#"数值面积 ["x","0"] @ 1..-1; 8"#).as_deref(),
            Ok("面积 ≈ 1\r\n复合辛普森法 · 8 个子区间")
        );
        assert_eq!(
            solve_equation(r#"numeric-area ["x²","x²"] @ -2..2"#).as_deref(),
            Ok("面积 ≈ 0\r\n复合辛普森法 · 64 个子区间")
        );
        assert_eq!(
            solve_equation(r#"数值面积 ["sqrt(x)","0"] @ -1..1"#),
            Err(EquationError::Approximation(
                CalculationError::FunctionDomain
            ))
        );
        assert_eq!(
            solve_equation(r#"数值面积 ["x","0","1"] @ 0..2"#),
            Err(EquationError::NumericAreaArguments)
        );
    }

    #[test]
    fn integrates_calculator_functions_and_preserves_interval_direction() {
        assert_eq!(
            solve_equation("数值积分 x**3 @ 0 到 2; 8").as_deref(),
            Ok("积分 ≈ 4\r\n复合辛普森法 · 8 个子区间")
        );
        assert_eq!(
            solve_equation("numeric-integral x**3 @ 2 到 0; 8").as_deref(),
            Ok("积分 ≈ -4\r\n复合辛普森法 · 8 个子区间")
        );
        assert_eq!(
            solve_equation("数值积分 x @ 0 到 2; 3"),
            Err(EquationError::NumericIntegralArguments)
        );
        assert_eq!(
            solve_equation("数值积分 1/x @ -1 到 1"),
            Err(EquationError::Approximation(
                CalculationError::DivisionByZero
            ))
        );
        assert_eq!(
            solve_equation("数值积分 x @ 1.0000000000000002 到 1; 4"),
            Err(EquationError::NumericIntegralIntervals)
        );
        assert!(
            (simpson_integral(&[1e-308; 3], -f64::MAX, f64::MAX) - 3.59538626972463).abs() < 1e-14
        );
    }

    #[test]
    fn bisects_two_expression_equalities_and_reports_their_residual() {
        let single = solve_equation("二分求根 cos(x)-x @ 0 到 1").unwrap();
        let equation = solve_equation("二分求根 cos(x)=x @ 0 到 1").unwrap();
        assert_eq!(equation.lines().next(), single.lines().next());
        assert!(equation.contains("残差 |左式 − 右式| ≈ "));
        assert_eq!(
            solve_equation("二分求根 x**3 == 8 @ 0..4").as_deref(),
            Ok("x ≈ 2\r\n残差 |左式 − 右式| ≈ 0")
        );
        assert_eq!(
            solve_equation("二分求根 2=x=2 @ 0..4"),
            Err(EquationError::BisectionArguments)
        );
    }

    #[test]
    fn bisects_sign_change_intervals_and_rejects_jumps_without_a_small_residual() {
        let result = solve_equation("二分求根 cos(x)-x @ 1 到 0").unwrap();
        let value: f64 = result
            .lines()
            .next()
            .unwrap()
            .strip_prefix("x ≈ ")
            .unwrap()
            .parse()
            .unwrap();
        assert!((value - 0.7390851332151607).abs() < 1e-14);
        assert!(result.contains("残差 |f(x)| ≈ "));
        assert_eq!(
            solve_equation("bisect-root x^2 @ 0 到 2").as_deref(),
            Ok("x ≈ 0\r\n残差 |f(x)| ≈ 0")
        );
        assert_eq!(
            solve_equation("二分求根 x^2+1 @ -1 到 1"),
            Err(EquationError::BisectionSameSign)
        );
        assert_eq!(
            solve_equation("二分求根 floor(x)-0.5 @ 0 到 2"),
            Err(EquationError::BisectionResidual)
        );
        assert_eq!(
            solve_equation("二分求根 1/x @ -1 到 1"),
            Err(EquationError::Approximation(
                CalculationError::DivisionByZero
            ))
        );
    }

    #[test]
    fn plots_multiple_functions_and_keeps_every_sampling_row_in_the_csv() {
        let query = r#"函数对照图 ["sqrt(x)","sqrt(-x)"] @ -1..1; 3"#;
        let (csv, plot) = equation_chart(query).unwrap().unwrap();
        assert_eq!(csv, "x,sqrt(x),sqrt(-x)\r\n-1,\"\",1\r\n0,0,0\r\n1,1,\"\"");
        assert_eq!(solve_equation(query).as_deref(), Ok(csv.as_str()));
        assert_eq!(plot.point_count(), 4);
        assert!(!plot.has_curve());
        assert!(matches!(
            equation_chart(r#"compare-function-plot ["sqrt(x)","ln(x)"] @ -3..-1; 3"#),
            Some(Err(EquationError::FunctionSampleHasNoValues))
        ));
    }

    #[test]
    fn compares_multiple_functions_in_a_single_csv_and_quotes_expression_headers() {
        assert_eq!(
            solve_equation(r#"函数对照 ["sqrt(x)","max(x,0)"] @ -1,0,4"#).as_deref(),
            Ok("x,sqrt(x),\"max(x,0)\"\r\n-1,\"\",0\r\n0,0,0\r\n4,2,4")
        );
        assert_eq!(
            solve_equation(r#"compare-functions ["cosd(x)","sind(x)"] @ 90,0,90"#).as_deref(),
            Ok("x,cosd(x),sind(x)\r\n90,0,1\r\n0,1,0\r\n90,0,1")
        );
        assert_eq!(
            solve_equation(r#"函数对照 ["x"] @ 1"#),
            Err(EquationError::FunctionComparisonArguments)
        );
        assert_eq!(
            solve_equation(r#"函数对照 ["x","x**2"] @ 1,"#),
            Err(EquationError::FunctionComparisonArguments)
        );
    }

    #[test]
    fn samples_both_sides_of_a_coordinate_at_successively_smaller_steps() {
        let csv = solve_equation("两侧采样 1/x @ 0; 0.1").unwrap();
        assert_eq!(csv.lines().count(), 7);
        assert_eq!(csv.lines().nth(1), Some("0.1,-0.1,-10,0.1,10"));
        let left_missing = solve_equation("approach-samples sqrt(x) @ 0; 1").unwrap();
        assert_eq!(left_missing.lines().nth(1), Some("1,-1,\"\",1,1"));
        assert_eq!(
            solve_equation("两侧采样 x @ 1e30; 1"),
            Err(EquationError::FunctionApproachStep)
        );
        assert_eq!(
            solve_equation("两侧采样 x @ 0; -1"),
            Err(EquationError::FunctionApproachArguments)
        );
    }

    #[test]
    fn calculates_values_at_requested_coordinates_in_input_order() {
        assert_eq!(
            solve_equation("函数值 y = sqrt(x) @ 9, -1, max(1,4), 9").as_deref(),
            Ok("x,近似值\r\n9,3\r\n-1,\r\n4,2\r\n9,3")
        );
        assert_eq!(
            solve_equation("function-values 2**x @ 1/2").as_deref(),
            Ok("x,近似值\r\n0.5,1.4142135623731")
        );
        assert_eq!(
            solve_equation("函数值 x @ 1,,2"),
            Err(EquationError::FunctionValuesArguments)
        );
        let over_limit = format!("函数值 x @ 1{}", ",1".repeat(64));
        assert_eq!(
            solve_equation(&over_limit),
            Err(EquationError::FunctionValuesArguments)
        );
    }

    #[test]
    fn refines_several_sampled_root_intervals_and_reports_unresolved_jumps() {
        let text = solve_equation("搜索零点 x²-2 @ -2..2; 5").unwrap();
        let roots: Vec<_> = text
            .lines()
            .skip(1)
            .map(|line| {
                assert!(line.ends_with(",近似根"));
                line.split_once(',').unwrap().0.parse::<f64>().unwrap()
            })
            .collect();
        assert_eq!(roots.len(), 2);
        assert!((roots[0] + 2.0_f64.sqrt()).abs() < 1e-14);
        assert!((roots[1] - 2.0_f64.sqrt()).abs() < 1e-14);
        assert_eq!(
            solve_equation("search-roots x @ -1..1; 3").as_deref(),
            Ok("横坐标,残差,左端点,右端点,状态\r\n0,0,0,0,采样值为零")
        );
        assert_eq!(
            solve_equation("搜索零点 floor(x)-0.5 @ 0..2; 5").as_deref(),
            Ok("横坐标,残差,左端点,右端点,状态\r\n\"\",\"\",0.5,1,残差未收敛")
        );
        assert_eq!(
            solve_equation("搜索零点 cosd(180*x) @ 0..9; 10"),
            Err(EquationError::TooManySampledBrackets)
        );
    }

    #[test]
    fn finds_sampled_zeros_and_sign_changes_without_crossing_undefined_rows() {
        assert_eq!(
            solve_equation("找异号区间 x²-2 @ 0..2; 3").as_deref(),
            Ok("类型,左端点,右端点\r\n相邻样值异号,1,2")
        );
        assert_eq!(
            solve_equation("sign-change-intervals x³-x @ -2..2; 9").as_deref(),
            Ok("类型,左端点,右端点\r\n采样值为零,-1,-1\r\n采样值为零,0,0\r\n采样值为零,1,1")
        );
        assert_eq!(
            solve_equation("找异号区间 1/x @ -1..1; 5").as_deref(),
            Ok("采样点中未发现零值或相邻异号值")
        );
        assert_eq!(
            solve_equation("找异号区间 0 @ 1..1.0000000000000002; 4").as_deref(),
            Ok("类型,左端点,右端点\r\n采样值为零,1,1\r\n采样值为零,1.0000000000000002,1.0000000000000002")
        );
    }

    #[test]
    fn refines_an_interior_extremum_checks_the_endpoints_and_reports_domain_errors() {
        let result = solve_equation("数值极小 f(x)=(x-3/2)^2 @ 3 to 0").unwrap();
        let (x, y) = result.split_once("\r\n").unwrap();
        let x = x.strip_prefix("x ≈ ").unwrap().parse::<f64>().unwrap();
        let y = y.strip_prefix("极小值 ≈ ").unwrap().parse::<f64>().unwrap();
        assert!((x - 1.5).abs() < 1e-10);
        assert!(y.abs() < 1e-20);
        assert_eq!(
            solve_equation("numeric-maximum cosd(x) @ 180..0").as_deref(),
            Ok("x ≈ 0\r\n极大值 ≈ 1")
        );
        assert_eq!(
            solve_equation("数值极小 sqrt(x) @ -1..2"),
            Err(EquationError::Approximation(
                CalculationError::FunctionDomain
            ))
        );
        assert_eq!(
            solve_equation("数值极大 x @ 1..1"),
            Err(EquationError::NumericExtremumArguments)
        );
    }

    #[test]
    fn reports_sampled_extrema_and_uses_the_first_coordinate_when_values_tie() {
        assert_eq!(
            solve_equation("采样极值 cosd(x) @ 360..0; 5").as_deref(),
            Ok("最小样值 -1 · x = 180\r\n最大样值 1 · x = 0\r\n有效样点 5/5")
        );
        assert_eq!(
            solve_equation("sample-extrema sqrt(x) @ -1..1; 3").as_deref(),
            Ok("最小样值 0 · x = 0\r\n最大样值 1 · x = 1\r\n有效样点 2/3")
        );
        assert_eq!(
            solve_equation("采样极值 sqrt(x) @ -3..-1; 3"),
            Err(EquationError::FunctionSampleHasNoValues)
        );
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
