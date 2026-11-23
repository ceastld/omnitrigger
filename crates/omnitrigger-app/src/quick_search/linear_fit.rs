use std::cmp::Ordering;

use num_bigint::{BigInt, Sign};
use num_rational::BigRational;

use super::calculator::normalize;
use super::fraction::{
    calculate_fraction, ensure_exact_size, format_signed_square_root, format_square_root,
    FractionError,
};
use super::matrix::{least_squares_with_weights, MatrixError};
use super::operators::split_top_level_commas;
use super::scatter_plot::ScatterPlot;
use super::statistics::median_sorted_values;

pub(super) const MAX_POINTS: usize = 64;
const MAX_PREDICTIONS: usize = 64;
const MAX_INTERPOLATION_POINTS: usize = 16;
const MAX_REGRESSION_DEGREE: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LinearFitError {
    Incomplete,
    InvalidPoint,
    TooManyPoints,
    TooManyPredictions,
    TooManyInterpolationPoints,
    RepeatedInterpolationX,
    PolynomialFitArguments,
    PolynomialFitRank,
    WeightedPoint,
    NegativeWeight,
    WeightedFitRank,
    ConstantX,
    ZeroX,
    ConstantCoordinate,
    Evaluation(FractionError),
}

impl LinearFitError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入至少两组 x,y，例如 0,1; 1,3; 2,5",
            Self::InvalidPoint => "每组须包含 x 和 y，用逗号或制表符隔开",
            Self::TooManyPoints => "最多可输入 64 对数值",
            Self::TooManyPredictions => "每次最多可预测 64 个 x 值",
            Self::TooManyInterpolationPoints => "多项式插值最多可输入 16 对数值",
            Self::RepeatedInterpolationX => "多项式插值中的数据点须有不同的横坐标",
            Self::PolynomialFitArguments => "填写 0 至 5 的整数次数，用 @ 隔开数据点",
            Self::PolynomialFitRank => "不同横坐标的个数须大于多项式次数",
            Self::WeightedPoint => "输入至少两组 x,y,权重，用分号或换行隔开",
            Self::NegativeWeight => "权重须大于或等于零",
            Self::WeightedFitRank => "权重大于零的不同横坐标个数须大于多项式次数",
            Self::ConstantX => "所有 x 相同，无法求拟合斜率",
            Self::ZeroX => "过原点拟合至少需要一个非零 x",
            Self::ConstantCoordinate => "x 和 y 各自至少需要两个不同的值才能计算相关系数",
            Self::Evaluation(FractionError::Invalid | FractionError::Incomplete) => {
                "坐标和预测值须为整数、有限小数、分数或精确算式"
            }
            Self::Evaluation(error) => error.message(),
        }
    }
}

enum FitOutput {
    Summary,
    Predictions(Vec<BigRational>),
    Residuals,
    Errors,
}

pub(super) fn calculate_linear_fit(input: &str) -> Result<String, LinearFitError> {
    let normalized = normalize(input);
    let input = normalized.trim();
    if let Some(plot) = plot_request(input) {
        return match plot {
            FitPlotRequest::Scatter(points) => {
                scatter_points(points).map(|points| scatter_csv(&points))
            }
            FitPlotRequest::Polynomial(arguments, _) => polynomial_residuals(arguments),
        };
    }
    let (points, output) = match input.split_once(char::is_whitespace) {
        Some((operation, rest))
            if operation == "过原点" || operation.eq_ignore_ascii_case("through-origin") =>
        {
            return fit_through_origin(rest.trim());
        }
        Some((operation, rest))
            if operation == "预测" || operation.eq_ignore_ascii_case("predict") =>
        {
            let (xs, points) = prediction_arguments(rest)?;
            (points, FitOutput::Predictions(xs))
        }
        Some((operation, rest))
            if operation == "插值" || operation.eq_ignore_ascii_case("interpolate") =>
        {
            let (xs, points) = prediction_arguments(rest)?;
            return interpolate(&xs, points);
        }
        Some((operation, rest))
            if operation == "插值系数"
                || operation.eq_ignore_ascii_case("interpolation-coefficients") =>
        {
            return interpolation_coefficients(rest.trim());
        }
        Some((operation, rest))
            if operation == "插值式"
                || operation.eq_ignore_ascii_case("interpolation-expression") =>
        {
            return interpolation_expression(rest.trim());
        }
        Some((operation, rest))
            if operation == "多项式" || operation.eq_ignore_ascii_case("polynomial-fit") =>
        {
            return fit_polynomial(rest.trim())
                .map(|fit| format_power_expression(fit.coefficients));
        }
        Some((operation, rest))
            if operation == "多项式残差"
                || operation.eq_ignore_ascii_case("polynomial-residuals") =>
        {
            return polynomial_residuals(rest.trim());
        }
        Some((operation, rest))
            if operation == "多项式误差" || operation.eq_ignore_ascii_case("polynomial-errors") =>
        {
            return polynomial_errors(rest.trim());
        }
        Some((operation, rest))
            if operation == "多项式预测"
                || operation.eq_ignore_ascii_case("polynomial-predict") =>
        {
            return polynomial_prediction(rest.trim());
        }
        Some((operation, rest))
            if operation == "多项式摘要"
                || operation.eq_ignore_ascii_case("polynomial-summary") =>
        {
            return polynomial_summary(rest.trim());
        }
        Some((operation, rest))
            if operation == "加权多项式"
                || operation.eq_ignore_ascii_case("weighted-polynomial") =>
        {
            return weighted_polynomial(rest.trim());
        }
        Some((operation, rest))
            if operation == "稳健直线" || operation.eq_ignore_ascii_case("theil-sen") =>
        {
            return robust_line(rest.trim());
        }
        Some((operation, rest))
            if operation == "残差" || operation.eq_ignore_ascii_case("residuals") =>
        {
            (rest.trim(), FitOutput::Residuals)
        }
        Some((operation, rest))
            if operation == "误差" || operation.eq_ignore_ascii_case("errors") =>
        {
            (rest.trim(), FitOutput::Errors)
        }
        _ => (input, FitOutput::Summary),
    };
    let sums = paired_sums(points, matches!(output, FitOutput::Residuals))?;
    let fit = sums.fit_line()?;
    let FittedLine {
        slope,
        intercept,
        products,
    } = &fit;
    let predict_y = |x: &BigRational| fit.predict(x);
    match output {
        FitOutput::Summary => {}
        FitOutput::Errors => {
            // 中心化平方和与乘积和已乘以样本数量。
            let residual_sum = (&products.square_y - &products.cross * slope) / &sums.count;
            return format_fit_errors(residual_sum, sums.count);
        }
        FitOutput::Predictions(xs) => {
            return format_prediction_values(&xs, "预测值", predict_y);
        }
        FitOutput::Residuals => {
            let mut rows = vec!["x,实际值,拟合值,残差".to_owned()];
            for (x, actual) in sums.points {
                let fitted = predict_y(&x)?;
                let residual = &actual - &fitted;
                check_size([&residual])?;
                rows.push(format!("{x},{actual},{fitted},{residual}"));
            }
            return Ok(rows.join("\r\n"));
        }
    }
    let r_squared = fit
        .r_squared()?
        .map_or_else(|| "未定义".to_owned(), |value| value.to_string());
    Ok(format!("斜率 {slope} · 截距 {intercept}\nR² {r_squared}"))
}

pub(super) enum FitPlotRequest<'a> {
    Scatter(&'a str),
    Polynomial(&'a str, FitPlotKind),
}

pub(super) enum FitPlotKind {
    Curve,
    Residuals,
}

pub(super) fn plot_request(input: &str) -> Option<FitPlotRequest<'_>> {
    let (operation, points) = input.trim().split_once(char::is_whitespace)?;
    let points = points.trim();
    if operation == "散点图" || operation.eq_ignore_ascii_case("scatter") {
        Some(FitPlotRequest::Scatter(points))
    } else if operation == "曲线图" || operation.eq_ignore_ascii_case("fit-plot") {
        Some(FitPlotRequest::Polynomial(points, FitPlotKind::Curve))
    } else if operation == "残差图" || operation.eq_ignore_ascii_case("residual-plot") {
        Some(FitPlotRequest::Polynomial(points, FitPlotKind::Residuals))
    } else {
        None
    }
}

pub(super) fn calculate_plot(
    request: FitPlotRequest<'_>,
) -> Result<(String, ScatterPlot), LinearFitError> {
    match request {
        FitPlotRequest::Scatter(input) => {
            let points = scatter_points(input)?;
            Ok((scatter_csv(&points), ScatterPlot::from_points(&points)))
        }
        FitPlotRequest::Polynomial(input, kind) => {
            let input = normalize(input);
            let fit = fit_polynomial(&input)?;
            let rows = polynomial_residual_rows(&fit)?;
            let plot = match kind {
                FitPlotKind::Curve => {
                    let curve = sample_polynomial_curve(&fit)?;
                    ScatterPlot::from_series(&fit.points, &curve)
                }
                FitPlotKind::Residuals => {
                    let points = rows
                        .iter()
                        .map(|row| (row.x.clone(), row.residual.clone()))
                        .collect::<Vec<_>>();
                    ScatterPlot::from_residuals(&points)
                }
            };
            Ok((polynomial_residual_csv(&rows), plot))
        }
    }
}

fn scatter_points(input: &str) -> Result<Vec<Point>, LinearFitError> {
    let input = normalize(input);
    let points = parse_paired_points(input.trim())?;
    points.collect()
}

fn scatter_csv(points: &[Point]) -> String {
    let mut rows = vec!["x,y".to_owned()];
    rows.extend(points.iter().map(|(x, y)| format!("{x},{y}")));
    rows.join("\r\n")
}

fn fit_through_origin(points: &str) -> Result<String, LinearFitError> {
    let sums = paired_sums(points, false)?;
    if sums.xx.numer().sign() == Sign::NoSign {
        return Err(LinearFitError::ZeroX);
    }
    let slope = &sums.xy / &sums.xx;
    let residual_sum = &sums.yy - &sums.xy * &slope;
    check_size([&slope, &residual_sum])?;
    let r_squared = if sums.yy.numer().sign() == Sign::NoSign {
        "未定义".to_owned()
    } else {
        let r_squared = (&sums.yy - &residual_sum) / &sums.yy;
        check_size([&r_squared])?;
        r_squared.to_string()
    };
    Ok(format!(
        "斜率 {slope} · 截距 0\n残差平方和 {residual_sum}\n未中心化 R² {r_squared}"
    ))
}

fn prediction_arguments(input: &str) -> Result<(Vec<BigRational>, &str), LinearFitError> {
    let (x, points) = input.split_once('@').ok_or(LinearFitError::Incomplete)?;
    if x.trim().is_empty() {
        return Err(LinearFitError::Incomplete);
    }
    let terms = split_top_level_commas(x)
        .take(MAX_PREDICTIONS + 1)
        .collect::<Vec<_>>();
    if terms.len() > MAX_PREDICTIONS {
        return Err(LinearFitError::TooManyPredictions);
    }
    let xs = terms
        .into_iter()
        .map(evaluate)
        .collect::<Result<Vec<_>, _>>()?;
    Ok((xs, points.trim()))
}

fn format_prediction_values(
    xs: &[BigRational],
    label: &str,
    evaluate_y: impl Fn(&BigRational) -> Result<BigRational, LinearFitError>,
) -> Result<String, LinearFitError> {
    if let [x] = xs {
        return evaluate_y(x).map(|value| value.to_string());
    }
    let mut rows = vec![format!("x,{label}")];
    for x in xs {
        rows.push(format!("{x},{}", evaluate_y(x)?));
    }
    Ok(rows.join("\r\n"))
}

struct Interpolation {
    nodes: Vec<BigRational>,
    differences: Vec<BigRational>,
}

fn interpolation(points: &str) -> Result<Interpolation, LinearFitError> {
    if points
        .split([';', '\n'])
        .nth(MAX_INTERPOLATION_POINTS)
        .is_some()
    {
        return Err(LinearFitError::TooManyInterpolationPoints);
    }
    let mut points = parse_paired_points(points)?.collect::<Result<Vec<_>, _>>()?;
    points.sort_unstable_by(|(x, _), (other, _)| x.cmp(other));
    if points.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(LinearFitError::RepeatedInterpolationX);
    }
    let (nodes, mut differences): (Vec<_>, Vec<_>) = points.into_iter().unzip();
    // 倒序更新差商，所需的上一阶系数在本轮计算前仍保持原值。
    for order in 1..nodes.len() {
        for index in (order..nodes.len()).rev() {
            let distance = &nodes[index] - &nodes[index - order];
            differences[index] = (&differences[index] - &differences[index - 1]) / distance;
            check_size([&differences[index]])?;
        }
    }
    Ok(Interpolation { nodes, differences })
}

fn interpolate(xs: &[BigRational], points: &str) -> Result<String, LinearFitError> {
    let Interpolation { nodes, differences } = interpolation(points)?;
    format_prediction_values(xs, "插值", |x| {
        let mut value = differences[differences.len() - 1].clone();
        for (node, difference) in nodes.iter().zip(&differences).rev().skip(1) {
            value = difference + (x - node) * value;
            check_size([&value])?;
        }
        Ok(value)
    })
}

fn interpolation_power_coefficients(points: &str) -> Result<Vec<BigRational>, LinearFitError> {
    let Interpolation { nodes, differences } = interpolation(points)?;
    let mut coefficients = vec![differences[differences.len() - 1].clone()];
    // 将牛顿形式逐层乘以 x - node，并把下一阶的差商加到常数项。
    for (node, difference) in nodes.iter().zip(&differences).rev().skip(1) {
        coefficients.push(BigRational::default());
        for degree in (1..coefficients.len()).rev() {
            coefficients[degree] = &coefficients[degree - 1] - node * &coefficients[degree];
            check_size([&coefficients[degree]])?;
        }
        coefficients[0] = difference - node * &coefficients[0];
        check_size([&coefficients[0]])?;
    }
    let degree = coefficients
        .iter()
        .rposition(|value| value.numer().sign() != Sign::NoSign)
        .unwrap_or(0);
    coefficients.truncate(degree + 1);
    Ok(coefficients)
}

fn interpolation_coefficients(points: &str) -> Result<String, LinearFitError> {
    let coefficients = interpolation_power_coefficients(points)?;
    let mut rows = vec!["次数,系数".to_owned()];
    for (degree, value) in coefficients.iter().enumerate().rev() {
        rows.push(format!("{degree},{value}"));
    }
    Ok(rows.join("\r\n"))
}

fn interpolation_expression(points: &str) -> Result<String, LinearFitError> {
    interpolation_power_coefficients(points).map(format_power_expression)
}

struct PolynomialFit {
    coefficients: Vec<BigRational>,
    points: Vec<Point>,
}

fn polynomial_fit_arguments(input: &str) -> Result<(usize, &str), LinearFitError> {
    let invalid = LinearFitError::PolynomialFitArguments;
    let (degree, points) = input.split_once('@').ok_or(invalid)?;
    let degree: usize = degree.trim().parse().map_err(|_| invalid)?;
    if degree > MAX_REGRESSION_DEGREE {
        return Err(invalid);
    }
    Ok((degree, points.trim()))
}

fn fit_polynomial(input: &str) -> Result<PolynomialFit, LinearFitError> {
    let (degree, points) = polynomial_fit_arguments(input)?;
    fit_polynomial_points(degree, points)
}

fn polynomial_prediction(input: &str) -> Result<String, LinearFitError> {
    let (degree, input) = polynomial_fit_arguments(input)?;
    let (xs, points) = prediction_arguments(input)?;
    let fit = fit_polynomial_points(degree, points)?;
    format_prediction_values(&xs, "预测值", |x| {
        polynomial_fit_value(&fit.coefficients, x)
    })
}

fn fit_polynomial_points(degree: usize, points: &str) -> Result<PolynomialFit, LinearFitError> {
    let points = parse_paired_points(points.trim())?.collect::<Result<Vec<_>, _>>()?;
    let coefficients = fit_polynomial_coefficients(degree, &points, None)?;
    Ok(PolynomialFit {
        coefficients,
        points,
    })
}

fn weighted_polynomial(input: &str) -> Result<String, LinearFitError> {
    let (degree, input) = polynomial_fit_arguments(input)?;
    let rows = point_rows(input, LinearFitError::WeightedPoint)?;
    let mut points = Vec::new();
    let mut weights = Vec::new();
    for row in rows {
        let mut fields: Vec<_> = split_top_level_commas(row).take(4).collect();
        if fields.len() == 1 {
            fields = row.split('\t').take(4).collect();
        }
        let [x, y, weight] = fields.as_slice() else {
            return Err(LinearFitError::WeightedPoint);
        };
        if fields.iter().any(|field| field.trim().is_empty()) {
            return Err(LinearFitError::WeightedPoint);
        }
        let weight = evaluate(weight)?;
        if weight.numer().sign() == Sign::Minus {
            return Err(LinearFitError::NegativeWeight);
        }
        let point = (evaluate(x)?, evaluate(y)?);
        if weight.numer().sign() != Sign::NoSign {
            points.push(point);
            weights.push(weight);
        }
    }
    if points.is_empty() {
        return Err(LinearFitError::WeightedFitRank);
    }
    fit_polynomial_coefficients(degree, &points, Some(&weights)).map(format_power_expression)
}

fn robust_line(input: &str) -> Result<String, LinearFitError> {
    let points = parse_paired_points(input)?.collect::<Result<Vec<_>, _>>()?;
    let mut slopes = Vec::new();
    for (index, (x, y)) in points.iter().enumerate() {
        for (other_x, other_y) in &points[index + 1..] {
            if x != other_x {
                let slope = (other_y - y) / (other_x - x);
                check_size([&slope])?;
                slopes.push(slope);
            }
        }
    }
    if slopes.is_empty() {
        return Err(LinearFitError::ConstantX);
    }
    slopes.sort_unstable();
    let slope = median_sorted_values(&slopes).map_err(LinearFitError::Evaluation)?;
    let mut intercepts = Vec::with_capacity(points.len());
    for (x, y) in points {
        let intercept = y - &slope * x;
        check_size([&intercept])?;
        intercepts.push(intercept);
    }
    intercepts.sort_unstable();
    let intercept = median_sorted_values(&intercepts).map_err(LinearFitError::Evaluation)?;
    Ok(format_power_expression(vec![intercept, slope]))
}

fn fit_polynomial_coefficients(
    degree: usize,
    points: &[Point],
    weights: Option<&[BigRational]>,
) -> Result<Vec<BigRational>, LinearFitError> {
    let mut design = Vec::with_capacity(points.len());
    let mut observations = Vec::with_capacity(points.len());
    for (x, y) in points {
        let mut row = vec![BigRational::from_integer(1.into())];
        for _ in 0..degree {
            let power = row.last().expect("初始行含常数项") * x;
            check_size([&power])?;
            row.push(power);
        }
        design.push(row);
        observations.push(vec![y.clone()]);
    }
    let coefficients =
        least_squares_with_weights(design, observations, weights).map_err(|error| match error {
            MatrixError::LeastSquaresRank if weights.is_some() => LinearFitError::WeightedFitRank,
            MatrixError::LeastSquaresRank => LinearFitError::PolynomialFitRank,
            MatrixError::Evaluation(cause) => LinearFitError::Evaluation(cause),
            _ => LinearFitError::PolynomialFitArguments,
        })?;
    Ok(coefficients
        .into_iter()
        .map(|row| row.into_iter().next().expect("每项只有一个响应系数"))
        .collect())
}

fn polynomial_residuals(input: &str) -> Result<String, LinearFitError> {
    let fit = fit_polynomial(input)?;
    polynomial_residual_rows(&fit).map(|rows| polynomial_residual_csv(&rows))
}

struct PolynomialResidual<'a> {
    x: &'a BigRational,
    actual: &'a BigRational,
    predicted: BigRational,
    residual: BigRational,
}

fn polynomial_residual_rows(
    fit: &PolynomialFit,
) -> Result<Vec<PolynomialResidual<'_>>, LinearFitError> {
    fit.points
        .iter()
        .map(|(x, actual)| {
            let predicted = polynomial_fit_value(&fit.coefficients, x)?;
            let residual = actual - &predicted;
            check_size([&residual])?;
            Ok(PolynomialResidual {
                x,
                actual,
                predicted,
                residual,
            })
        })
        .collect()
}

fn polynomial_residual_csv(rows: &[PolynomialResidual<'_>]) -> String {
    let mut lines = vec!["x,实际值,拟合值,残差".to_owned()];
    lines.extend(rows.iter().map(|row| {
        format!(
            "{},{},{},{}",
            row.x, row.actual, row.predicted, row.residual
        )
    }));
    lines.join("\r\n")
}

fn sample_polynomial_curve(fit: &PolynomialFit) -> Result<Vec<Point>, LinearFitError> {
    const SEGMENTS: u32 = 48;
    let xs = fit.points.iter().map(|(x, _)| x);
    let min = xs.clone().min().expect("拟合至少含两个数据点");
    let max = xs.max().expect("拟合至少含两个数据点");
    let segments = if min == max { 0 } else { SEGMENTS };
    let step = (max - min) / BigInt::from(SEGMENTS);
    (0..=segments)
        .map(|index| {
            let x = min + &step * BigInt::from(index);
            let y = polynomial_fit_value(&fit.coefficients, &x)?;
            Ok((x, y))
        })
        .collect()
}

fn polynomial_fit_value(
    coefficients: &[BigRational],
    x: &BigRational,
) -> Result<BigRational, LinearFitError> {
    let mut value = BigRational::default();
    for coefficient in coefficients.iter().rev() {
        value = value * x + coefficient;
        check_size([&value])?;
    }
    Ok(value)
}

fn polynomial_errors(input: &str) -> Result<String, LinearFitError> {
    let fit = fit_polynomial(input)?;
    let count = BigInt::from(fit.points.len());
    format_fit_errors(polynomial_squared_error(&fit)?, count)
}

fn polynomial_squared_error(fit: &PolynomialFit) -> Result<BigRational, LinearFitError> {
    let mut squared_sum = BigRational::default();
    for (x, actual) in &fit.points {
        let residual = actual - polynomial_fit_value(&fit.coefficients, x)?;
        check_size([&residual])?;
        squared_sum += &residual * &residual;
        check_size([&squared_sum])?;
    }
    Ok(squared_sum)
}

fn polynomial_summary(input: &str) -> Result<String, LinearFitError> {
    let fit = fit_polynomial(input)?;
    let residual_sum = polynomial_squared_error(&fit)?;
    let mut observation_sum = BigRational::default();
    let mut squared_sum = BigRational::default();
    for (_, actual) in &fit.points {
        observation_sum += actual;
        squared_sum += actual * actual;
        check_size([&observation_sum, &squared_sum])?;
    }
    let count = BigInt::from(fit.points.len());
    let total = squared_sum - &observation_sum * &observation_sum / count;
    check_size([&total])?;
    let r_squared = if total.numer().sign() == Sign::NoSign {
        "未定义".to_owned()
    } else {
        let value = BigRational::from_integer(1.into()) - &residual_sum / total;
        check_size([&value])?;
        value.to_string()
    };
    let expression = format_power_expression(fit.coefficients);
    Ok(format!(
        "拟合式 {expression}\nR² {r_squared}\n残差平方和 {residual_sum}"
    ))
}

fn format_fit_errors(residual_sum: BigRational, count: BigInt) -> Result<String, LinearFitError> {
    let mean_squared = &residual_sum / count;
    check_size([&residual_sum, &mean_squared])?;
    let root_mean_squared = format_square_root(&mean_squared);
    Ok(format!(
        "残差平方和 {residual_sum}\n均方误差 {mean_squared}\n均方根误差 {root_mean_squared}"
    ))
}

fn format_power_expression(coefficients: Vec<BigRational>) -> String {
    let one = BigInt::from(1u8);
    let mut expression = String::new();
    for (degree, coefficient) in coefficients.into_iter().enumerate().rev() {
        let sign = coefficient.numer().sign();
        if sign == Sign::NoSign {
            continue;
        }
        let negative = sign == Sign::Minus;
        let prefix = match (expression.is_empty(), negative) {
            (true, true) => "-",
            (true, false) => "",
            (false, true) => " - ",
            (false, false) => " + ",
        };
        expression.push_str(prefix);
        let magnitude = if negative { -coefficient } else { coefficient };
        if degree == 0 {
            expression.push_str(&magnitude.to_string());
            continue;
        }
        if magnitude.numer() != &one {
            expression.push_str(&magnitude.numer().to_string());
            expression.push('*');
        }
        expression.push('x');
        if degree > 1 {
            expression.push_str("**");
            expression.push_str(&degree.to_string());
        }
        if !magnitude.is_integer() {
            expression.push('/');
            expression.push_str(&magnitude.denom().to_string());
        }
    }
    if expression.is_empty() {
        expression.push('0');
    }
    expression
}

pub(super) fn calculate_covariance(points: &str, sample: bool) -> Result<String, LinearFitError> {
    paired_sums(points.trim(), false)?
        .covariance(sample)
        .map(|value| value.to_string())
}

pub(super) fn calculate_correlation(points: &str) -> Result<String, LinearFitError> {
    paired_sums(points.trim(), false)?.correlation()
}

pub(super) fn calculate_rank_correlation(points: &str) -> Result<String, LinearFitError> {
    let pairs = parse_paired_points(points.trim())?;
    let (x, y): (Vec<_>, Vec<_>) = pairs.collect::<Result<Vec<_>, _>>()?.into_iter().unzip();
    let ranks = doubled_midranks(&x)
        .into_iter()
        .zip(doubled_midranks(&y))
        .map(Ok);
    summarize_pairs(ranks, false)?.correlation()
}

pub(super) fn calculate_kendall_correlation(points: &str) -> Result<String, LinearFitError> {
    let pairs = parse_paired_points(points.trim())?.collect::<Result<Vec<_>, _>>()?;
    let (mut distinct_x, mut distinct_y, mut score) = (0_u32, 0_u32, 0_i32);
    for (index, (x, y)) in pairs.iter().enumerate() {
        for (other_x, other_y) in &pairs[index + 1..] {
            let (x_order, y_order) = (x.cmp(other_x), y.cmp(other_y));
            distinct_x += u32::from(x_order != Ordering::Equal);
            distinct_y += u32::from(y_order != Ordering::Equal);
            score += match (x_order, y_order) {
                (Ordering::Equal, _) | (_, Ordering::Equal) => 0,
                (left, right) if left == right => 1,
                _ => -1,
            };
        }
    }
    if distinct_x == 0 || distinct_y == 0 {
        return Err(LinearFitError::ConstantCoordinate);
    }
    let score = BigInt::from(score);
    let squared = BigRational::new(&score * &score, BigInt::from(distinct_x) * distinct_y);
    Ok(format_signed_square_root(&squared, score.sign()))
}

fn doubled_midranks(values: &[BigRational]) -> Vec<BigRational> {
    let mut indices = (0..values.len()).collect::<Vec<_>>();
    indices.sort_unstable_by(|&a, &b| values[a].cmp(&values[b]));
    let mut ranks = vec![BigRational::default(); values.len()];
    let mut first = 0;
    for tied in indices.chunk_by(|&a, &b| values[a] == values[b]) {
        let end = first + tied.len();
        // 名次乘以二后仍是整数，相关系数不受共同缩放影响。
        let rank = BigRational::from_integer(BigInt::from(first + end + 1));
        for &index in tied {
            ranks[index] = rank.clone();
        }
        first = end;
    }
    ranks
}

fn format_correlation(products: CenteredProducts) -> Result<String, LinearFitError> {
    if products.square_x.numer().sign() == Sign::NoSign
        || products.square_y.numer().sign() == Sign::NoSign
    {
        return Err(LinearFitError::ConstantCoordinate);
    }
    Ok(format_signed_square_root(
        &products.squared_correlation()?,
        products.cross.numer().sign(),
    ))
}

#[derive(Default)]
pub(super) struct PairedSums {
    count: BigInt,
    x: BigRational,
    y: BigRational,
    xx: BigRational,
    xy: BigRational,
    yy: BigRational,
    points: Vec<Point>,
}

impl PairedSums {
    pub(super) fn covariance(&self, sample: bool) -> Result<BigRational, LinearFitError> {
        let degrees_of_freedom = &self.count - BigInt::from(u8::from(sample));
        let covariance = self.centered_cross_product() / (&self.count * degrees_of_freedom);
        check_size([&covariance])?;
        Ok(covariance)
    }

    pub(super) fn correlation(&self) -> Result<String, LinearFitError> {
        format_correlation(self.centered_products()?)
    }

    pub(super) fn count(&self) -> &BigInt {
        &self.count
    }

    pub(super) fn record(
        &mut self,
        x: &BigRational,
        y: &BigRational,
    ) -> Result<(), LinearFitError> {
        self.count += 1u8;
        self.xx += x * x;
        self.xy += x * y;
        self.yy += y * y;
        self.x += x;
        self.y += y;
        check_size([&self.x, &self.y, &self.xx, &self.xy, &self.yy])
    }

    pub(super) fn fit_line(&self) -> Result<FittedLine, LinearFitError> {
        let products = self.centered_products()?;
        if products.square_x.numer().sign() == Sign::NoSign {
            return Err(LinearFitError::ConstantX);
        }
        let slope = &products.cross / &products.square_x;
        let intercept = (&self.y - &slope * &self.x) / &self.count;
        check_size([&slope, &intercept])?;
        Ok(FittedLine {
            slope,
            intercept,
            products,
        })
    }

    fn centered_products(&self) -> Result<CenteredProducts, LinearFitError> {
        // 每项乘以样本数，保留精确的中心化平方和与乘积和。
        let products = CenteredProducts {
            square_x: &self.xx * &self.count - &self.x * &self.x,
            cross: self.centered_cross_product(),
            square_y: &self.yy * &self.count - &self.y * &self.y,
        };
        check_size([&products.square_x, &products.cross, &products.square_y])?;
        Ok(products)
    }

    fn centered_cross_product(&self) -> BigRational {
        &self.xy * &self.count - &self.x * &self.y
    }
}

pub(super) struct FittedLine {
    pub(super) slope: BigRational,
    pub(super) intercept: BigRational,
    products: CenteredProducts,
}

impl FittedLine {
    pub(super) fn predict(&self, x: &BigRational) -> Result<BigRational, LinearFitError> {
        let y = &self.slope * x + &self.intercept;
        check_size([&y])?;
        Ok(y)
    }

    pub(super) fn r_squared(&self) -> Result<Option<BigRational>, LinearFitError> {
        if self.products.square_y.numer().sign() == Sign::NoSign {
            return Ok(None);
        }
        self.products.squared_correlation().map(Some)
    }
}

struct CenteredProducts {
    square_x: BigRational,
    cross: BigRational,
    square_y: BigRational,
}

impl CenteredProducts {
    fn squared_correlation(&self) -> Result<BigRational, LinearFitError> {
        let value = (&self.cross * &self.cross) / (&self.square_x * &self.square_y);
        check_size([&value])?;
        Ok(value)
    }
}

pub(super) type Point = (BigRational, BigRational);

fn paired_sums(points: &str, retain_points: bool) -> Result<PairedSums, LinearFitError> {
    summarize_pairs(parse_paired_points(points)?, retain_points)
}

fn point_rows(
    points: &str,
    incomplete: LinearFitError,
) -> Result<impl Iterator<Item = &str>, LinearFitError> {
    let points = points.split([';', '\n']);
    let count = points.clone().count();
    if count < 2 {
        return Err(incomplete);
    }
    if count > MAX_POINTS {
        return Err(LinearFitError::TooManyPoints);
    }
    Ok(points)
}

fn parse_paired_points(
    points: &str,
) -> Result<impl Iterator<Item = Result<Point, LinearFitError>> + '_, LinearFitError> {
    let points = point_rows(points, LinearFitError::Incomplete)?;
    let pairs = points.map(|point| {
        let mut coordinates = split_top_level_commas(point);
        let (x, y) = match (coordinates.next(), coordinates.next(), coordinates.next()) {
            (Some(x), Some(y), None) => (x, y),
            (Some(row), None, None) => row
                .split_once('\t')
                .filter(|(_, y)| !y.contains('\t'))
                .ok_or(LinearFitError::InvalidPoint)?,
            _ => return Err(LinearFitError::InvalidPoint),
        };
        if x.trim().is_empty() || y.trim().is_empty() {
            return Err(LinearFitError::InvalidPoint);
        }
        Ok((evaluate(x)?, evaluate(y)?))
    });
    Ok(pairs)
}

fn summarize_pairs(
    pairs: impl Iterator<Item = Result<Point, LinearFitError>>,
    retain_points: bool,
) -> Result<PairedSums, LinearFitError> {
    let mut sums = PairedSums::default();
    for pair in pairs {
        let (x, y) = pair?;
        sums.record(&x, &y)?;
        if retain_points {
            sums.points.push((x, y));
        }
    }
    Ok(sums)
}

fn evaluate(input: &str) -> Result<BigRational, LinearFitError> {
    calculate_fraction(input).map_err(LinearFitError::Evaluation)
}

fn check_size<'a>(values: impl IntoIterator<Item = &'a BigRational>) -> Result<(), LinearFitError> {
    values
        .into_iter()
        .try_for_each(|value| ensure_exact_size(value).map_err(LinearFitError::Evaluation))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_a_theil_sen_line_with_outliers_duplicate_x_and_exact_medians() {
        assert_eq!(
            calculate_linear_fit("稳健直线 0,1; 1,3; 2,5; 3,7; 4,100").as_deref(),
            Ok("2*x + 1")
        );
        assert_eq!(
            calculate_linear_fit("THEIL-SEN 0,0; 0,1; 2,2").as_deref(),
            Ok("3*x/4 + 1/2")
        );
        assert_eq!(
            calculate_linear_fit("稳健直线 1,0; 1,1"),
            Err(LinearFitError::ConstantX)
        );
    }

    #[test]
    fn fits_polynomials_with_exact_nonnegative_weights() {
        assert_eq!(
            calculate_linear_fit("加权多项式 1 @ 0,0,3; 0,4,1; 1,3,2; 2,-99,0").as_deref(),
            Ok("2*x + 1")
        );
        assert_eq!(
            calculate_linear_fit("weighted-polynomial 0 @ 0\t1\t1/2\n1\t3\t3/2").as_deref(),
            Ok("5/2")
        );
        assert_eq!(
            calculate_linear_fit("加权多项式 1 @ 0,0,1; 0,4,1; 1,3,1"),
            calculate_linear_fit("多项式 1 @ 0,0; 0,4; 1,3")
        );
        assert_eq!(
            calculate_linear_fit("加权多项式 1 @ 0,0,1; 1,1,0"),
            Err(LinearFitError::WeightedFitRank)
        );
        assert_eq!(
            calculate_linear_fit("加权多项式 0 @ 0,0,0; 1,3,0"),
            Err(LinearFitError::WeightedFitRank)
        );
        assert_eq!(
            calculate_linear_fit("加权多项式 0 @ 0,99,0; 1,3,1").as_deref(),
            Ok("3")
        );
        assert_eq!(
            calculate_linear_fit("加权多项式 1 @ 0,0,-1; 1,1,1"),
            Err(LinearFitError::NegativeWeight)
        );
        assert_eq!(
            calculate_linear_fit("加权多项式 1 @ 0,0; 1,1,1"),
            Err(LinearFitError::WeightedPoint)
        );
    }

    #[test]
    fn summarizes_polynomial_fit_and_handles_constant_observations() {
        assert_eq!(
            calculate_linear_fit("多项式摘要 2 @ -1,2; 0,0; 1,2; 0,2").as_deref(),
            Ok("拟合式 x**2 + 1\nR² 1/3\n残差平方和 2")
        );
        assert_eq!(
            calculate_linear_fit("polynomial-summary 2 @ 0,2; 1,2; 2,2").as_deref(),
            Ok("拟合式 2\nR² 未定义\n残差平方和 0")
        );
    }

    #[test]
    fn predicts_with_one_polynomial_fit_and_preserves_query_order() {
        assert_eq!(
            calculate_linear_fit("多项式预测 2 @ 1/2,2,0 @ -1,2; 0,0; 1,2; 0,2").as_deref(),
            Ok("x,预测值\r\n1/2,5/4\r\n2,5\r\n0,1")
        );
        assert_eq!(
            calculate_linear_fit("polynomial-predict 0 @ 9 @ 0,1; 1,2").as_deref(),
            Ok("3/2")
        );
        assert_eq!(
            calculate_linear_fit("多项式预测 1 @ 1/2,2 @ 0,1; 1,3; 2,5"),
            calculate_linear_fit("预测 1/2,2 @ 0,1; 1,3; 2,5"),
        );
        assert_eq!(
            calculate_linear_fit("多项式预测 2 @ 1 @ 0,1; 0,2; 0,3"),
            Err(LinearFitError::PolynomialFitRank)
        );
    }

    #[test]
    fn reports_exact_polynomial_fit_errors_with_the_same_denominator_as_linear_fit() {
        assert_eq!(
            calculate_linear_fit("多项式误差 2 @ -1,2; 0,0; 1,2; 0,2").as_deref(),
            Ok("残差平方和 2\n均方误差 1/2\n均方根误差 √(1/2)")
        );
        assert_eq!(
            calculate_linear_fit("polynomial-errors 1 @ 1,1; 2,1; 0,0"),
            calculate_linear_fit("误差 1,1; 2,1; 0,0"),
        );
    }

    #[test]
    fn reports_polynomial_fit_residuals_in_the_original_row_order() {
        assert_eq!(
            calculate_linear_fit("多项式残差 2 @ -1,2; 0,0; 1,2; 0,2").as_deref(),
            Ok("x,实际值,拟合值,残差\r\n-1,2,2,0\r\n0,0,1,-1\r\n1,2,2,0\r\n0,2,1,1")
        );
        assert_eq!(
            calculate_linear_fit("多项式残差 1 @ 1,1; 2,1; 0,0"),
            calculate_linear_fit("残差 1,1; 2,1; 0,0"),
        );
    }

    #[test]
    fn fits_exact_polynomials_to_more_points_than_the_degree_requires() {
        for (input, expected) in [
            ("多项式 2 @ -1,2; 0,0; 1,2; 0,2", "x**2 + 1"),
            ("polynomial-fit 0 @ 0,1; 1,2", "3/2"),
            ("多项式 1 @ 0,1; 1,3; 2,5", "2*x + 1"),
        ] {
            assert_eq!(
                calculate_linear_fit(input).as_deref(),
                Ok(expected),
                "{input}"
            );
        }
        assert_eq!(
            calculate_linear_fit("多项式 2 @ 0,1; 0,2; 0,3"),
            Err(LinearFitError::PolynomialFitRank)
        );
        assert_eq!(
            calculate_linear_fit("多项式 6 @ 0,1; 1,2"),
            Err(LinearFitError::PolynomialFitArguments)
        );
    }

    #[test]
    fn renders_an_exact_expression_that_the_fraction_calculator_can_reuse() {
        for (input, expected) in [
            ("插值式 0,1; 1,2; 2,5", "x**2 + 1"),
            ("插值式 0,-1; 2,2", "3*x/2 - 1"),
            ("interpolation-expression 0,0; 1,0", "0"),
            ("插值式 0,0; 1,-1; 2,-8; 3,-27", "-x**3"),
        ] {
            assert_eq!(
                calculate_linear_fit(input).as_deref(),
                Ok(expected),
                "{input}"
            );
        }
        let expression = calculate_linear_fit("插值式 0,1; 1,2; 2,5").unwrap();
        assert_eq!(
            super::super::fraction::simplify_fraction(&format!("取值表 [0,1,2] {expression}"))
                .as_deref(),
            Ok("x\ty\n0\t1\n1\t2\n2\t5")
        );
    }

    #[test]
    fn writes_the_actual_degree_and_exact_power_coefficients_of_the_interpolant() {
        for (input, expected) in [
            ("插值系数 0,1; 1,2; 2,5", "次数,系数\r\n2,1\r\n1,0\r\n0,1"),
            ("插值系数 3,1/4; 1,1/4; 2,1/4", "次数,系数\r\n0,1/4"),
            (
                "interpolation-coefficients 0,1; 2,2",
                "次数,系数\r\n1,1/2\r\n0,1",
            ),
            ("插值系数 0,0; 1,0", "次数,系数\r\n0,0"),
            (
                "插值系数 0,0; 1,1; 2,8; 3,27",
                "次数,系数\r\n3,1\r\n2,0\r\n1,0\r\n0,0",
            ),
        ] {
            assert_eq!(
                calculate_linear_fit(input).as_deref(),
                Ok(expected),
                "{input}"
            );
        }
        assert_eq!(
            calculate_linear_fit("插值系数 1,1; 1.0,2"),
            Err(LinearFitError::RepeatedInterpolationX)
        );
    }

    #[test]
    fn evaluates_exact_interpolation_for_one_or_many_queries() {
        for (input, expected) in [
            ("插值 3 @ 0,1; 1,2; 2,5", "10"),
            (
                "interpolate 1/2,2,-1 @ 2,5; 0,1; 1,2",
                "x,插值\r\n1/2,5/4\r\n2,5\r\n-1,2",
            ),
            ("插值 1/2 @ 0\t1\r\n1\t2", "3/2"),
            ("插值 1/2 @ 0,0; 1,1; 2,8; 3,27", "1/8"),
        ] {
            assert_eq!(
                calculate_linear_fit(input).as_deref(),
                Ok(expected),
                "{input}"
            );
        }
        assert_eq!(
            calculate_linear_fit("插值 3 @ 1,2; 0,1; 1.0,8"),
            Err(LinearFitError::RepeatedInterpolationX)
        );
        let many = (0..=MAX_INTERPOLATION_POINTS)
            .map(|x| format!("{x},{x}"))
            .collect::<Vec<_>>();
        assert_eq!(
            calculate_linear_fit(&format!("插值 3 @ {}", many.join(";"))),
            Err(LinearFitError::TooManyInterpolationPoints)
        );
    }

    #[test]
    fn accepts_two_spreadsheet_columns_for_fit_and_covariance() {
        assert_eq!(
            calculate_linear_fit("0\t1\r\n1\t3\r\ncomb(2,2)\t3\r\n2\t5").as_deref(),
            Ok("斜率 2 · 截距 1\nR² 1")
        );
        assert_eq!(
            calculate_covariance("0\t0\r\n1\t1\r\n2\t1", true).as_deref(),
            Ok("1/2")
        );
        assert_eq!(
            calculate_linear_fit("0\t1\t2\r\n1\t3\t4"),
            Err(LinearFitError::InvalidPoint)
        );
    }

    #[test]
    fn reports_an_exact_fitted_value_and_residual_for_each_original_point() {
        assert_eq!(
            calculate_linear_fit("残差 2,1; 0,0; 1,1").as_deref(),
            Ok("x,实际值,拟合值,残差\r\n2,1,7/6,-1/6\r\n0,0,1/6,-1/6\r\n1,1,2/3,1/3")
        );
        assert_eq!(
            calculate_linear_fit("residuals 0,1/2; 0,1"),
            Err(LinearFitError::ConstantX)
        );
    }

    #[test]
    fn summarizes_exact_fit_errors_with_observation_count_as_the_divisor() {
        assert_eq!(
            calculate_linear_fit("误差 0,0; 1,1; 2,1").as_deref(),
            Ok("残差平方和 1/6\n均方误差 1/18\n均方根误差 √(1/18)")
        );
        assert_eq!(
            calculate_linear_fit("errors 0,2/3; 5,2/3").as_deref(),
            Ok("残差平方和 0\n均方误差 0\n均方根误差 0")
        );
        assert_eq!(
            calculate_linear_fit("误差 0,0; 0,2"),
            Err(LinearFitError::ConstantX)
        );
    }

    #[test]
    fn predicts_several_exact_values_in_input_order_without_refitting() {
        assert_eq!(
            calculate_linear_fit("预测 1/2, 0, comb(4,2), 1/2 @ 0,1; 1,3; 2,5").as_deref(),
            Ok("x,预测值\r\n1/2,2\r\n0,1\r\n6,13\r\n1/2,2")
        );
        assert_eq!(
            calculate_linear_fit("predict comb(4,2) @ 0,1; 1,3").as_deref(),
            Ok("13")
        );
        let xs = std::iter::repeat_n("2", MAX_PREDICTIONS + 1)
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(
            calculate_linear_fit(&format!("预测 {xs} @ 0,1; 1,3")),
            Err(LinearFitError::TooManyPredictions)
        );
    }

    #[test]
    fn fits_exact_no_intercept_models_and_uses_zero_as_the_r_squared_baseline() {
        for (query, expected) in [
            (
                "过原点 1,2; 2,5; 3,7",
                "斜率 33/14 · 截距 0\n残差平方和 3/14\n未中心化 R² 363/364",
            ),
            (
                "through-origin 1/2,-1; 1/2,-3",
                "斜率 -4 · 截距 0\n残差平方和 2\n未中心化 R² 4/5",
            ),
            (
                "过原点 1,0; 2,0",
                "斜率 0 · 截距 0\n残差平方和 0\n未中心化 R² 未定义",
            ),
        ] {
            assert_eq!(
                calculate_linear_fit(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert_eq!(
            calculate_linear_fit("过原点 0,2; 0,3"),
            Err(LinearFitError::ZeroX)
        );
    }

    #[test]
    fn fits_and_predicts_with_exact_rational_arithmetic() {
        for (input, expected) in [
            ("0,1; 1,3; 2,5", "斜率 2 · 截距 1\nR² 1"),
            ("０，０；１，１；２，１", "斜率 1/2 · 截距 1/6\nR² 3/4"),
            ("0,5\r\n1,5", "斜率 0 · 截距 5\nR² 未定义"),
            (
                "9007199254740992,1; 9007199254740993,3",
                "斜率 2 · 截距 -18014398509481983\nR² 1",
            ),
            ("0,1/3; comb(2,1),-1/3", "斜率 -1/3 · 截距 1/3\nR² 1"),
            ("预测 10 @ 0,1; 1,3; 2,5", "21"),
            ("Predict 1/2 @ 0,0; 1,1; 2,1", "5/12"),
        ] {
            assert_eq!(
                calculate_linear_fit(input).as_deref(),
                Ok(expected),
                "{input}"
            );
        }
    }

    #[test]
    fn rejects_invalid_or_degenerate_regression_inputs() {
        for (input, error) in [
            ("0,1", LinearFitError::Incomplete),
            ("1,1; 1,2", LinearFitError::ConstantX),
            ("0,1,2; 2,3", LinearFitError::InvalidPoint),
            ("0,1; 2,", LinearFitError::InvalidPoint),
            (
                "0,1; 2,1/0",
                LinearFitError::Evaluation(FractionError::ZeroDenominator),
            ),
        ] {
            assert_eq!(calculate_linear_fit(input), Err(error), "{input}");
        }
        let excessive = format!("{}1,2", "0,1;".repeat(MAX_POINTS));
        assert_eq!(
            calculate_linear_fit(&excessive),
            Err(LinearFitError::TooManyPoints)
        );
    }
}
