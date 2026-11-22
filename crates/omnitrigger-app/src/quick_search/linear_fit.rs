use num_bigint::{BigInt, Sign};
use num_rational::BigRational;

use super::calculator::normalize;
use super::fraction::{
    calculate_fraction, ensure_exact_size, format_signed_square_root, FractionError,
};
use super::operators::split_top_level_commas;

const MAX_POINTS: usize = 64;
const MAX_PREDICTIONS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LinearFitError {
    Incomplete,
    InvalidPoint,
    TooManyPoints,
    TooManyPredictions,
    ConstantX,
    ConstantCoordinate,
    Evaluation(FractionError),
}

impl LinearFitError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入至少两组 x,y，例如 0,1; 1,3; 2,5",
            Self::InvalidPoint => "每组须包含 x 和 y，用逗号隔开",
            Self::TooManyPoints => "最多可输入 64 对数值",
            Self::TooManyPredictions => "每次最多可预测 64 个 x 值",
            Self::ConstantX => "所有 x 相同，无法求拟合斜率",
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
}

pub(super) fn calculate_linear_fit(input: &str) -> Result<String, LinearFitError> {
    let normalized = normalize(input);
    let input = normalized.trim();
    let (points, output) = match input.split_once(char::is_whitespace) {
        Some((operation, rest))
            if operation == "预测" || operation.eq_ignore_ascii_case("predict") =>
        {
            let (x, points) = rest.split_once('@').ok_or(LinearFitError::Incomplete)?;
            if x.trim().is_empty() {
                return Err(LinearFitError::Incomplete);
            }
            let terms = split_top_level_commas(x)
                .take(MAX_PREDICTIONS + 1)
                .collect::<Vec<_>>();
            if terms.len() > MAX_PREDICTIONS {
                return Err(LinearFitError::TooManyPredictions);
            }
            let predictions = terms
                .into_iter()
                .map(evaluate)
                .collect::<Result<Vec<_>, _>>()?;
            (points.trim(), FitOutput::Predictions(predictions))
        }
        Some((operation, rest))
            if operation == "残差" || operation.eq_ignore_ascii_case("residuals") =>
        {
            (rest.trim(), FitOutput::Residuals)
        }
        _ => (input, FitOutput::Summary),
    };
    let sums = paired_sums(points, matches!(output, FitOutput::Residuals))?;
    let products = sums.centered_products()?;
    if products.square_x.numer().sign() == Sign::NoSign {
        return Err(LinearFitError::ConstantX);
    }
    let slope = &products.cross / &products.square_x;
    let intercept = (sums.y - &slope * sums.x) / sums.count;
    check_size([&slope, &intercept])?;
    let predict_y = |x: &BigRational| {
        let y = &slope * x + &intercept;
        check_size([&y])?;
        Ok::<_, LinearFitError>(y)
    };
    match output {
        FitOutput::Summary => {}
        FitOutput::Predictions(xs) => {
            if let [x] = xs.as_slice() {
                return predict_y(x).map(|value| value.to_string());
            }
            let mut rows = vec!["x,预测值".to_owned()];
            for x in &xs {
                rows.push(format!("{x},{}", predict_y(x)?));
            }
            return Ok(rows.join("\r\n"));
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
    let r_squared = if products.square_y.numer().sign() == Sign::NoSign {
        "未定义".to_owned()
    } else {
        products.squared_correlation()?.to_string()
    };
    Ok(format!("斜率 {slope} · 截距 {intercept}\nR² {r_squared}"))
}

pub(super) fn calculate_covariance(points: &str, sample: bool) -> Result<String, LinearFitError> {
    let sums = paired_sums(points.trim(), false)?;
    let degrees_of_freedom = &sums.count - BigInt::from(u8::from(sample));
    let covariance = sums.centered_cross_product() / (&sums.count * degrees_of_freedom);
    check_size([&covariance])?;
    Ok(covariance.to_string())
}

pub(super) fn calculate_correlation(points: &str) -> Result<String, LinearFitError> {
    let products = paired_sums(points.trim(), false)?.centered_products()?;
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

struct PairedSums {
    count: BigInt,
    x: BigRational,
    y: BigRational,
    xx: BigRational,
    xy: BigRational,
    yy: BigRational,
    points: Vec<(BigRational, BigRational)>,
}

impl PairedSums {
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

fn paired_sums(points: &str, retain_points: bool) -> Result<PairedSums, LinearFitError> {
    let points = points.split([';', '\n']);
    let count = points.clone().count();
    if count < 2 {
        return Err(LinearFitError::Incomplete);
    }
    if count > MAX_POINTS {
        return Err(LinearFitError::TooManyPoints);
    }
    let [mut sx, mut sy, mut sxx, mut sxy, mut syy]: [BigRational; 5] = Default::default();
    let mut observations = Vec::new();
    for point in points {
        let mut coordinates = split_top_level_commas(point);
        let (Some(x), Some(y), None) = (coordinates.next(), coordinates.next(), coordinates.next())
        else {
            return Err(LinearFitError::InvalidPoint);
        };
        if x.trim().is_empty() || y.trim().is_empty() {
            return Err(LinearFitError::InvalidPoint);
        }
        let (x, y) = (evaluate(x)?, evaluate(y)?);
        sxx += &x * &x;
        sxy += &x * &y;
        syy += &y * &y;
        sx += &x;
        sy += &y;
        check_size([&sx, &sy, &sxx, &sxy, &syy])?;
        if retain_points {
            observations.push((x, y));
        }
    }

    Ok(PairedSums {
        count: BigInt::from(count),
        x: sx,
        y: sy,
        xx: sxx,
        xy: sxy,
        yy: syy,
        points: observations,
    })
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
