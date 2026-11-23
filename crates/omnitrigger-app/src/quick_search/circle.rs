use std::cmp::Ordering;

use num_bigint::{BigInt, Sign};
use num_rational::BigRational;

use super::calculator::normalize;
use super::fraction::{
    calculate_fraction, ensure_exact_size, exact_abs_square_root, format_exact_multiple,
    format_square_root, FractionError,
};
use super::plane::{parse_point, Point2, PointError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CircleError {
    Incomplete,
    NonPositiveRadius,
    AngleOutOfRange,
    PointArguments,
    TwoCircleArguments,
    LineCircleArguments,
    TangentPointArguments,
    CoincidentLinePoints,
    PointCircleArguments,
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
            Self::TwoCircleArguments => "分别填写两个圆心和半径，例如 0,0 @ 1; 2,0 @ 1",
            Self::LineCircleArguments => {
                "依次填写圆心与半径、直线上的两个点，例如 0,0 @ 1; -1,0; 1,0"
            }
            Self::TangentPointArguments => "填写圆心与半径，再填引出切线的点，例如 0,0 @ 1; 2,0",
            Self::CoincidentLinePoints => "用于确定直线的两个点须互不重合",
            Self::PointCircleArguments => "填写待测点、圆心和半径，例如 包含 3/5,4/5 @ 0,0 @ 1",
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
    if operation == "两圆关系" || operation.eq_ignore_ascii_case("relation") {
        return compare_circles(arguments);
    }
    if operation == "两圆交点" || operation.eq_ignore_ascii_case("intersections") {
        return circle_intersections(arguments);
    }
    if operation == "直线交点" || operation.eq_ignore_ascii_case("line-intersections") {
        return line_circle_intersections(arguments);
    }
    if operation == "切点" || operation.eq_ignore_ascii_case("tangent-points") {
        return circle_tangent_points(arguments);
    }
    if operation == "包含" || operation.eq_ignore_ascii_case("contains") {
        return circle_contains(arguments);
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
    let radius = parse_positive_length(length)?;
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

fn parse_positive_length(input: &str) -> Result<BigRational, CircleError> {
    let length = calculate_fraction(input).map_err(CircleError::Evaluation)?;
    if length.numer().sign() != Sign::Plus {
        return Err(CircleError::NonPositiveRadius);
    }
    Ok(length)
}

struct CenteredCircle {
    center: Point2,
    radius: BigRational,
}

fn circle_center_and_radius(input: &str) -> Result<CenteredCircle, CircleError> {
    let (center, radius) = input
        .split_once('@')
        .ok_or(CircleError::TwoCircleArguments)?;
    if center.trim().is_empty() || radius.trim().is_empty() {
        return Err(CircleError::TwoCircleArguments);
    }
    Ok(CenteredCircle {
        center: circle_point(center)?,
        radius: parse_positive_length(radius)?,
    })
}

fn parse_two_circles(input: &str) -> Result<[CenteredCircle; 2], CircleError> {
    let mut circles = input.split(';');
    let (Some(first), Some(second), None) = (circles.next(), circles.next(), circles.next()) else {
        return Err(CircleError::TwoCircleArguments);
    };
    Ok([
        circle_center_and_radius(first)?,
        circle_center_and_radius(second)?,
    ])
}

fn compare_circles(input: &str) -> Result<String, CircleError> {
    let [first, second] = parse_two_circles(input)?;
    let distance_squared = squared_point_distance(&first.center, &second.center);
    let sum = &first.radius + &second.radius;
    let difference = first.radius - second.radius;
    let outer_limit = &sum * &sum;
    let inner_limit = &difference * &difference;
    for value in [&distance_squared, &outer_limit, &inner_limit] {
        ensure_exact_size(value).map_err(CircleError::Evaluation)?;
    }
    let relation = match distance_squared.cmp(&outer_limit) {
        Ordering::Greater => "外离 · 0 个交点",
        Ordering::Equal => "外切 · 1 个交点",
        Ordering::Less => match distance_squared.cmp(&inner_limit) {
            Ordering::Greater => "相交 · 2 个交点",
            Ordering::Equal if distance_squared.numer().sign() == Sign::NoSign => {
                "重合 · 无穷多个交点"
            }
            Ordering::Equal => "内切 · 1 个交点",
            Ordering::Less => "内含 · 0 个交点",
        },
    };
    Ok(format!(
        "{relation}\n圆心距 {}",
        format_square_root(&distance_squared)
    ))
}

fn squared_point_distance(first: &Point2, second: &Point2) -> BigRational {
    let (dx, dy) = (&first[0] - &second[0], &first[1] - &second[1]);
    &dx * &dx + &dy * &dy
}

fn circle_contains(input: &str) -> Result<String, CircleError> {
    let (point, circle) = input
        .split_once('@')
        .ok_or(CircleError::PointCircleArguments)?;
    let point = circle_point(point)?;
    let circle = circle_center_and_radius(circle).map_err(|error| match error {
        CircleError::TwoCircleArguments => CircleError::PointCircleArguments,
        error => error,
    })?;
    let distance_squared = squared_point_distance(&point, &circle.center);
    let radius_squared = &circle.radius * &circle.radius;
    for value in [&distance_squared, &radius_squared] {
        ensure_exact_size(value).map_err(CircleError::Evaluation)?;
    }
    Ok(match distance_squared.cmp(&radius_squared) {
        Ordering::Less => "圆内",
        Ordering::Equal => "圆周上",
        Ordering::Greater => "圆外",
    }
    .to_owned())
}

fn circle_intersections(input: &str) -> Result<String, CircleError> {
    let [first, second] = parse_two_circles(input)?;
    let [x1, y1] = &first.center;
    let [x2, y2] = &second.center;
    let (dx, dy) = (x2 - x1, y2 - y1);
    let dx_squared = &dx * &dx;
    let dy_squared = &dy * &dy;
    let distance_squared = &dx_squared + &dy_squared;
    if distance_squared.numer().sign() == Sign::NoSign {
        return Ok(if first.radius == second.radius {
            "无穷多个交点".to_owned()
        } else {
            "无交点".to_owned()
        });
    }
    let first_radius_squared = &first.radius * &first.radius;
    let second_radius_squared = &second.radius * &second.radius;
    let projection = (&distance_squared + &first_radius_squared - second_radius_squared)
        / (&distance_squared * BigInt::from(2_u8));
    let offset_squared = first_radius_squared / distance_squared - &projection * &projection;
    let offset_sign = offset_squared.numer().sign();
    if offset_sign == Sign::Minus {
        return Ok("无交点".to_owned());
    }
    let middle_x = x1 + &projection * &dx;
    let middle_y = y1 + projection * &dy;
    let shift_x_squared = dy_squared * &offset_squared;
    let shift_y_squared = dx_squared * offset_squared;
    render_symmetric_intersections(
        [middle_x, middle_y],
        [shift_x_squared, shift_y_squared],
        [
            dy.numer().sign() == Sign::Plus,
            dx.numer().sign() == Sign::Minus,
        ],
        offset_sign == Sign::NoSign,
        "交点",
    )
}

fn line_circle_intersections(input: &str) -> Result<String, CircleError> {
    let invalid = CircleError::LineCircleArguments;
    let mut parts = input.split(';');
    let (Some(circle), Some(first), Some(second), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(invalid);
    };
    let circle = circle_center_and_radius(circle).map_err(|error| match error {
        CircleError::TwoCircleArguments => invalid,
        error => error,
    })?;
    let [x1, y1] = circle_point(first)?;
    let [x2, y2] = circle_point(second)?;
    let (dx, dy) = (x2 - &x1, y2 - &y1);
    let dx_squared = &dx * &dx;
    let dy_squared = &dy * &dy;
    let length_squared = &dx_squared + &dy_squared;
    if length_squared.numer().sign() == Sign::NoSign {
        return Err(CircleError::CoincidentLinePoints);
    }
    ensure_exact_size(&length_squared).map_err(CircleError::Evaluation)?;
    let projection =
        ((&circle.center[0] - &x1) * &dx + (&circle.center[1] - &y1) * &dy) / &length_squared;
    let center = [x1 + &projection * &dx, y1 + projection * &dy];
    let remainder =
        &circle.radius * &circle.radius - squared_point_distance(&center, &circle.center);
    ensure_exact_size(&remainder).map_err(CircleError::Evaluation)?;
    let offset_sign = remainder.numer().sign();
    if offset_sign == Sign::Minus {
        return Ok("无交点".to_owned());
    }
    let offset_squared = remainder / length_squared;
    render_symmetric_intersections(
        center,
        [dx_squared * &offset_squared, dy_squared * offset_squared],
        [
            dx.numer().sign() == Sign::Minus,
            dy.numer().sign() == Sign::Minus,
        ],
        offset_sign == Sign::NoSign,
        "交点",
    )
}

fn circle_tangent_points(input: &str) -> Result<String, CircleError> {
    let invalid = CircleError::TangentPointArguments;
    let (circle, point) = input.split_once(';').ok_or(invalid)?;
    let circle = circle_center_and_radius(circle).map_err(|error| match error {
        CircleError::TwoCircleArguments => invalid,
        error => error,
    })?;
    let [px, py] = circle_point(point)?;
    let [cx, cy] = circle.center;
    let (dx, dy) = (&px - &cx, &py - &cy);
    let dx_squared = &dx * &dx;
    let dy_squared = &dy * &dy;
    let distance_squared = &dx_squared + &dy_squared;
    let radius_squared = &circle.radius * &circle.radius;
    for value in [&distance_squared, &radius_squared] {
        ensure_exact_size(value).map_err(CircleError::Evaluation)?;
    }
    match distance_squared.cmp(&radius_squared) {
        Ordering::Less => return Ok("无切点".to_owned()),
        Ordering::Equal => return Ok(format!("切点 ({px}, {py})")),
        Ordering::Greater => {}
    }
    // 切点半径垂直于切线，切点在圆心与给定点连线上的投影比例为 r²/d²。
    let fraction = radius_squared / distance_squared;
    let center = [cx + &fraction * &dx, cy + &fraction * &dy];
    let one = BigRational::from_integer(1.into());
    let offset_squared = &fraction * (one - &fraction);
    render_symmetric_intersections(
        center,
        [dy_squared * &offset_squared, dx_squared * offset_squared],
        [
            dy.numer().sign() == Sign::Plus,
            dx.numer().sign() == Sign::Minus,
        ],
        false,
        "切点",
    )
}

fn render_symmetric_intersections(
    [middle_x, middle_y]: Point2,
    [shift_x_squared, shift_y_squared]: Point2,
    [subtract_x, subtract_y]: [bool; 2],
    tangent: bool,
    label: &str,
) -> Result<String, CircleError> {
    let render_point = |opposite| {
        let x = intersection_axis(&middle_x, &shift_x_squared, subtract_x ^ opposite)?;
        let y = intersection_axis(&middle_y, &shift_y_squared, subtract_y ^ opposite)?;
        Ok::<_, CircleError>(format!("{label} ({x}, {y})"))
    };
    let first = render_point(false)?;
    if tangent {
        return Ok(first);
    }
    Ok(format!("{first}\n{}", render_point(true)?))
}

fn intersection_axis(
    center: &BigRational,
    shift_squared: &BigRational,
    subtract: bool,
) -> Result<String, CircleError> {
    ensure_exact_size(shift_squared).map_err(CircleError::Evaluation)?;
    if let Some(shift) = exact_abs_square_root(shift_squared) {
        let coordinate = if subtract {
            center - shift
        } else {
            center + shift
        };
        ensure_exact_size(&coordinate).map_err(CircleError::Evaluation)?;
        return Ok(coordinate.to_string());
    }
    ensure_exact_size(center).map_err(CircleError::Evaluation)?;
    let shift = format_square_root(shift_squared);
    let operator = if subtract { '-' } else { '+' };
    if center.numer().sign() == Sign::NoSign {
        return Ok(if subtract { format!("-{shift}") } else { shift });
    }
    Ok(format!("{center} {operator} {shift}"))
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

pub(super) fn multiple_of_pi(value: &BigRational) -> String {
    format_exact_multiple(value, "π")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_points_inside_on_or_outside_an_exact_circle() {
        for (query, expected) in [
            ("包含 3/5,4/5 @ 0,0 @ 1", "圆周上"),
            ("contains 2/3,-3/4 @ 1/6,-3/4 @ 1/2", "圆周上"),
            ("包含 -1/2,1/2 @ 0,0 @ 1", "圆内"),
            ("包含 １，１ ＠ ０，０ ＠ １", "圆外"),
        ] {
            assert_eq!(calculate_circle(query).as_deref(), Ok(expected));
        }
        assert_eq!(
            calculate_circle("包含 1,1 @ 0,0"),
            Err(CircleError::PointCircleArguments)
        );
        assert_eq!(
            calculate_circle("包含 1,1 @ 0,0 @ 0"),
            Err(CircleError::NonPositiveRadius)
        );
    }

    #[test]
    fn finds_exact_tangent_points_from_exterior_boundary_and_interior_points() {
        for (query, expected) in [
            (
                "切点 0,0 @ 1; 2,0",
                "切点 (1/2, √(3/4))\n切点 (1/2, -√(3/4))",
            ),
            ("tangent-points 0,0 @ 1; 3/5,4/5", "切点 (3/5, 4/5)"),
            ("切点 0,0 @ 1; 1/2,0", "无切点"),
            ("切点 0,0 @ 1; 0,0", "无切点"),
        ] {
            assert_eq!(calculate_circle(query).as_deref(), Ok(expected), "{query}");
        }
    }

    #[test]
    fn intersects_a_circle_and_an_infinite_line_with_exact_coordinates() {
        for (query, expected) in [
            (
                "直线交点 0,0 @ 1; -1,1/2; 1,1/2",
                "交点 (√(3/4), 1/2)\n交点 (-√(3/4), 1/2)",
            ),
            (
                "line-intersections 0,0 @ 1; 3/5,10; 3/5,11",
                "交点 (3/5, 4/5)\n交点 (3/5, -4/5)",
            ),
            ("直线交点 0,0 @ 1; -1,1; 1,1", "交点 (0, 1)"),
            ("直线交点 0,0 @ 1; -1,2; 1,2", "无交点"),
        ] {
            assert_eq!(calculate_circle(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            calculate_circle("直线交点 0,0 @ 1; 2,2; 2,2"),
            Err(CircleError::CoincidentLinePoints)
        );
    }

    #[test]
    fn finds_exact_circle_intersection_points_or_reports_their_absence() {
        for (query, expected) in [
            ("两圆交点 -3,0 @ 5; 3,0 @ 5", "交点 (0, 4)\n交点 (0, -4)"),
            ("两圆交点 0,0 @ 2; 2,2 @ 2", "交点 (0, 2)\n交点 (2, 0)"),
            (
                "两圆交点 0,0 @ 1; 1,0 @ 1",
                "交点 (1/2, √(3/4))\n交点 (1/2, -√(3/4))",
            ),
            ("intersections 0,0 @ 1/2; 1,0 @ 1/2", "交点 (1/2, 0)"),
            ("两圆交点 0,0 @ 3; 2,0 @ 1", "交点 (3, 0)"),
            ("两圆交点 0,0 @ 3; 1,0 @ 1", "无交点"),
            ("两圆交点 0,0 @ 1; 3,0 @ 1", "无交点"),
            ("两圆交点 0,0 @ 1; 0,0 @ 2", "无交点"),
            ("两圆交点 0,0 @ 1/2; 0,0 @ 0.5", "无穷多个交点"),
        ] {
            assert_eq!(calculate_circle(query).as_deref(), Ok(expected), "{query}");
        }
    }

    #[test]
    fn classifies_two_circles_using_precise_distances_and_radii() {
        for (query, expected) in [
            ("两圆关系 0,0 @ 1; 3,0 @ 1", "外离 · 0 个交点\n圆心距 3"),
            ("两圆关系 0,0 @ 1/2; 1,0 @ 1/2", "外切 · 1 个交点\n圆心距 1"),
            ("两圆关系 0,0 @ 1; 1,1 @ 1", "相交 · 2 个交点\n圆心距 √2"),
            ("relation 0,0 @ 3; 2,0 @ 1", "内切 · 1 个交点\n圆心距 2"),
            ("两圆关系 0,0 @ 3; 1/2,0 @ 1", "内含 · 0 个交点\n圆心距 1/2"),
            ("两圆关系 0,0 @ 3; 0,0 @ 1", "内含 · 0 个交点\n圆心距 0"),
            (
                "两圆关系 0,0 @ 1/2; 0,0 @ 0.5",
                "重合 · 无穷多个交点\n圆心距 0",
            ),
        ] {
            assert_eq!(calculate_circle(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            calculate_circle("两圆关系 0,0 @ 0; 1,0 @ 1"),
            Err(CircleError::NonPositiveRadius)
        );
        assert_eq!(
            calculate_circle("两圆关系 0,0 @ 1; 1,0"),
            Err(CircleError::TwoCircleArguments)
        );
    }

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
