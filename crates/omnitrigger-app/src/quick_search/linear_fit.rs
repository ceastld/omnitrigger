use num_bigint::{BigInt, Sign};
use num_rational::BigRational;

use super::calculator::normalize;
use super::fraction::{calculate_fraction, ensure_exact_size, format_square_root, FractionError};
use super::operators::split_top_level_commas;

const MAX_POINTS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LinearFitError {
    Incomplete,
    InvalidPoint,
    TooManyPoints,
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
            Self::ConstantX => "所有 x 相同，无法求拟合斜率",
            Self::ConstantCoordinate => "x 和 y 各自至少需要两个不同的值才能计算相关系数",
            Self::Evaluation(FractionError::Invalid | FractionError::Incomplete) => {
                "坐标和预测值须为整数、有限小数、分数或精确算式"
            }
            Self::Evaluation(error) => error.message(),
        }
    }
}

pub(super) fn calculate_linear_fit(input: &str) -> Result<String, LinearFitError> {
    let normalized = normalize(input);
    let input = normalized.trim();
    let (points, predict) = match input.split_once(char::is_whitespace) {
        Some((operation, rest))
            if operation == "预测" || operation.eq_ignore_ascii_case("predict") =>
        {
            let (x, points) = rest.split_once('@').ok_or(LinearFitError::Incomplete)?;
            if x.trim().is_empty() {
                return Err(LinearFitError::Incomplete);
            }
            (points.trim(), Some(evaluate(x)?))
        }
        _ => (input, None),
    };
    let sums = paired_sums(points)?;
    let products = sums.centered_products()?;
    if products.square_x.numer().sign() == Sign::NoSign {
        return Err(LinearFitError::ConstantX);
    }
    let slope = &products.cross / &products.square_x;
    let intercept = (sums.y - &slope * sums.x) / sums.count;
    check_size([&slope, &intercept])?;
    if let Some(x) = predict {
        let y = slope * x + intercept;
        check_size([&y])?;
        return Ok(y.to_string());
    }
    let r_squared = if products.square_y.numer().sign() == Sign::NoSign {
        "未定义".to_owned()
    } else {
        products.squared_correlation()?.to_string()
    };
    Ok(format!("斜率 {slope} · 截距 {intercept}\nR² {r_squared}"))
}

pub(super) fn calculate_covariance(points: &str, sample: bool) -> Result<String, LinearFitError> {
    let sums = paired_sums(points.trim())?;
    let degrees_of_freedom = &sums.count - BigInt::from(u8::from(sample));
    let covariance = sums.centered_cross_product() / (&sums.count * degrees_of_freedom);
    check_size([&covariance])?;
    Ok(covariance.to_string())
}

pub(super) fn calculate_correlation(points: &str) -> Result<String, LinearFitError> {
    let products = paired_sums(points.trim())?.centered_products()?;
    if products.square_x.numer().sign() == Sign::NoSign
        || products.square_y.numer().sign() == Sign::NoSign
    {
        return Err(LinearFitError::ConstantCoordinate);
    }
    let root = format_square_root(&products.squared_correlation()?);
    if products.cross.numer().sign() == Sign::Minus {
        Ok(format!("-{root}"))
    } else {
        Ok(root)
    }
}

struct PairedSums {
    count: BigInt,
    x: BigRational,
    y: BigRational,
    xx: BigRational,
    xy: BigRational,
    yy: BigRational,
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

fn paired_sums(points: &str) -> Result<PairedSums, LinearFitError> {
    let points = points.split([';', '\n']);
    let count = points.clone().count();
    if count < 2 {
        return Err(LinearFitError::Incomplete);
    }
    if count > MAX_POINTS {
        return Err(LinearFitError::TooManyPoints);
    }
    let [mut sx, mut sy, mut sxx, mut sxy, mut syy]: [BigRational; 5] = Default::default();
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
        sx += x;
        sy += y;
        check_size([&sx, &sy, &sxx, &sxy, &syy])?;
    }

    Ok(PairedSums {
        count: BigInt::from(count),
        x: sx,
        y: sy,
        xx: sxx,
        xy: sxy,
        yy: syy,
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
