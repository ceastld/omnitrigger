use num_bigint::{BigInt, Sign};
use num_rational::BigRational;

use super::calculator::normalize;
use super::fraction::{calculate_fraction, ensure_exact_size, format_square_root, FractionError};
use super::plane::{parse_point, Point2, PointError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CircleError {
    Incomplete,
    NonPositiveRadius,
    AngleOutOfRange,
    PointArguments,
    CollinearPoints,
    CoincidentDiameterPoints,
    Evaluation(FractionError),
}

impl CircleError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入半径或直径，可加 @ 和圆心角",
            Self::NonPositiveRadius => "半径和直径须大于零",
            Self::AngleOutOfRange => "圆心角须在 0 至 360 度之间",
            Self::PointArguments => "填写二维坐标，用逗号分隔每点的两个坐标，用分号分隔各点",
            Self::CollinearPoints => "三个点须互不重合且不在同一直线上",
            Self::CoincidentDiameterPoints => "直径的两个端点须互不重合",
            Self::Evaluation(FractionError::Invalid | FractionError::Incomplete) => {
                "长度、角度和坐标须用整数、有限小数、分数或精确算式"
            }
            Self::Evaluation(error) => error.message(),
        }
    }
}

pub(super) fn calculate_circle(query: &str) -> Result<String, CircleError> {
    let input = normalize(query);
    let input = input.trim();
    let (operation, arguments) = input.split_once(char::is_whitespace).unwrap_or((input, ""));
    if operation == "三点" || operation.eq_ignore_ascii_case("three-points") {
        return circle_through_points(arguments);
    }
    if operation == "直径端点" || operation.eq_ignore_ascii_case("diameter-points") {
        return circle_from_diameter_points(arguments);
    }
    let (length, angle) = input
        .split_once('@')
        .map_or((input, None), |(length, angle)| {
            (length.trim(), Some(angle.trim()))
        });
    let (length, is_diameter) = match length.split_once(char::is_whitespace) {
        Some((unit, value)) if unit == "直径" || unit.eq_ignore_ascii_case("diameter") => {
            (value.trim(), true)
        }
        Some((unit, value)) if unit == "半径" || unit.eq_ignore_ascii_case("radius") => {
            (value.trim(), false)
        }
        _ => (length, false),
    };
    if length.is_empty() {
        return Err(CircleError::Incomplete);
    }
    let radius = calculate_fraction(length).map_err(CircleError::Evaluation)?;
    if radius.numer().sign() != Sign::Plus {
        return Err(CircleError::NonPositiveRadius);
    }
    let two = BigInt::from(2);
    let (radius, diameter) = if is_diameter {
        (&radius / &two, radius)
    } else {
        let diameter = &radius * &two;
        (radius, diameter)
    };
    let area = &radius * &radius;
    for value in [&radius, &diameter, &area] {
        ensure_exact_size(value).map_err(CircleError::Evaluation)?;
    }
    let Some(angle) = angle else {
        return Ok(format!(
            "半径 {radius} · 直径 {diameter}\n周长 {} · 面积 {}",
            multiple_of_pi(&diameter),
            multiple_of_pi(&area),
        ));
    };
    let angle = angle.strip_suffix(['°', '度']).unwrap_or(angle).trim();
    if angle.is_empty() {
        return Err(CircleError::Incomplete);
    }
    let angle = calculate_fraction(angle).map_err(CircleError::Evaluation)?;
    let complete = BigInt::from(360);
    if angle.numer().sign() == Sign::Minus || angle.numer() > &(&complete * angle.denom()) {
        return Err(CircleError::AngleOutOfRange);
    }
    let ratio = angle / complete;
    let arc = diameter * &ratio;
    let sector = area * ratio;
    for value in [&arc, &sector] {
        ensure_exact_size(value).map_err(CircleError::Evaluation)?;
    }
    Ok(format!(
        "弧长 {} · 扇形面积 {}",
        multiple_of_pi(&arc),
        multiple_of_pi(&sector),
    ))
}

fn circle_through_points(input: &str) -> Result<String, CircleError> {
    let points: Vec<_> = input.split(';').take(4).collect();
    let [start, next, last] = points.as_slice() else {
        return Err(CircleError::PointArguments);
    };
    let [x, y] = circle_point(start)?;
    let [bx, by] = circle_point(next)?;
    let [cx, cy] = circle_point(last)?;
    let (bx, by, cx, cy) = (bx - &x, by - &y, cx - &x, cy - &y);
    // 用相对第一点的坐标解圆心到三点等距的两个线性方程。
    let divisor = (&bx * &cy - &by * &cx) * BigInt::from(2);
    if divisor.numer().sign() == Sign::NoSign {
        return Err(CircleError::CollinearPoints);
    }
    let squared_b = &bx * &bx + &by * &by;
    let squared_c = &cx * &cx + &cy * &cy;
    let offset_x = (&squared_b * &cy - &squared_c * &by) / &divisor;
    let offset_y = (&bx * &squared_c - &cx * &squared_b) / divisor;
    let center_x = x + &offset_x;
    let center_y = y + &offset_y;
    let radius_squared = &offset_x * &offset_x + &offset_y * &offset_y;
    render_coordinate_circle(&center_x, &center_y, &radius_squared)
}

fn circle_from_diameter_points(input: &str) -> Result<String, CircleError> {
    let mut points = input.split(';');
    let (Some(start), Some(end), None) = (points.next(), points.next(), points.next()) else {
        return Err(CircleError::PointArguments);
    };
    let [x, y] = circle_point(start)?;
    let [end_x, end_y] = circle_point(end)?;
    let two = BigInt::from(2);
    let offset_x = (end_x - &x) / &two;
    let offset_y = (end_y - &y) / &two;
    let center_x = x + &offset_x;
    let center_y = y + &offset_y;
    let radius_squared = &offset_x * &offset_x + &offset_y * &offset_y;
    if radius_squared.numer().sign() == Sign::NoSign {
        return Err(CircleError::CoincidentDiameterPoints);
    }
    render_coordinate_circle(&center_x, &center_y, &radius_squared)
}

fn render_coordinate_circle(
    center_x: &BigRational,
    center_y: &BigRational,
    radius_squared: &BigRational,
) -> Result<String, CircleError> {
    for value in [center_x, center_y, radius_squared] {
        ensure_exact_size(value).map_err(CircleError::Evaluation)?;
    }
    let radius = format_square_root(radius_squared);
    let area = multiple_of_pi(radius_squared);
    Ok(format!(
        "圆心 ({center_x}, {center_y})\n半径 {radius} · 面积 {area}"
    ))
}

fn circle_point(input: &str) -> Result<Point2, CircleError> {
    parse_point(input).map_err(|error| match error {
        PointError::Arguments => CircleError::PointArguments,
        PointError::Evaluation(error) => CircleError::Evaluation(error),
    })
}

fn multiple_of_pi(value: &BigRational) -> String {
    if value.numer().sign() == Sign::NoSign {
        return "0".to_owned();
    }
    let numerator = value.numer().to_string();
    let numerator = if numerator == "1" { "" } else { &numerator };
    if value.is_integer() {
        format!("{numerator}π")
    } else {
        format!("{numerator}π/{}", value.denom())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_the_midpoint_of_a_diameter_and_preserves_exact_coordinates() {
        for (query, expected) in [
            ("直径端点 -1,0; 3,2", "圆心 (1, 1)\n半径 √5 · 面积 5π"),
            (
                "DIAMETER-POINTS 1/2,-1/3; 1/2,5/3",
                "圆心 (1/2, 2/3)\n半径 1 · 面积 π",
            ),
        ] {
            assert_eq!(calculate_circle(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            calculate_circle("直径端点 0,1; 0,1"),
            Err(CircleError::CoincidentDiameterPoints),
        );
        assert_eq!(
            calculate_circle("直径端点 1,0; 3,2; 1,2"),
            Err(CircleError::PointArguments),
        );
    }

    #[test]
    fn finds_the_exact_circle_through_three_non_collinear_points() {
        for (query, expected) in [
            ("三点 1,0; 3,0; 1,2", "圆心 (2, 1)\n半径 √2 · 面积 2π"),
            (
                "THREE-POINTS 0,1/2; 1/2,0; 0,0",
                "圆心 (1/4, 1/4)\n半径 √(1/8) · 面积 π/8",
            ),
            (
                "三点 １，０；０，１；−１，０",
                "圆心 (0, 0)\n半径 1 · 面积 π",
            ),
        ] {
            assert_eq!(calculate_circle(query).as_deref(), Ok(expected), "{query}");
        }
        for query in ["三点 0,0; 1,1; 2,2", "三点 0,0; 0,0; 2,2"] {
            assert_eq!(
                calculate_circle(query),
                Err(CircleError::CollinearPoints),
                "{query}"
            );
        }
        assert_eq!(
            calculate_circle("三点 0,0,0; 1,0; 0,1"),
            Err(CircleError::PointArguments)
        );
    }

    #[test]
    fn calculates_circle_and_sector_measurements_as_exact_multiples_of_pi() {
        for (query, expected) in [
            ("3", "半径 3 · 直径 6\n周长 6π · 面积 9π"),
            ("半径 1/2", "半径 1/2 · 直径 1\n周长 π · 面积 π/4"),
            ("直径 10 @ 90°", "弧长 5π/2 · 扇形面积 25π/4"),
            ("diameter 2 @ 360度", "弧长 2π · 扇形面积 π"),
            ("radius 2 @ 0", "弧长 0 · 扇形面积 0"),
            ("半径 １ @ １／２度", "弧长 π/360 · 扇形面积 π/720"),
        ] {
            assert_eq!(calculate_circle(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            ("0", CircleError::NonPositiveRadius),
            ("半径 -2", CircleError::NonPositiveRadius),
            ("3 @ -1", CircleError::AngleOutOfRange),
            ("3 @ 360.0001", CircleError::AngleOutOfRange),
            ("3 @", CircleError::Incomplete),
        ] {
            assert_eq!(calculate_circle(query), Err(error), "{query}");
        }
    }
}
