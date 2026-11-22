use std::collections::{BTreeMap, HashSet};

use num_bigint::{BigInt, Sign};
use num_rational::BigRational;

use super::{
    calculator::normalize,
    fraction::{ensure_exact_size, exact_abs_square_root, format_square_root, FractionError},
    plane::{parse_point, Point2, PointError},
};

const MAX_VERTICES: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PolygonError {
    Incomplete,
    PointArguments,
    ContainmentArguments,
    VertexCount,
    DuplicateVertex,
    IntersectingEdges,
    Evaluation(FractionError),
}

impl PolygonError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete | Self::PointArguments => {
                "按轮廓顺序输入二维坐标，点内用逗号，各点用分号"
            }
            Self::ContainmentArguments => "先填待检查的点，加 @ 后按轮廓顺序填写多边形顶点",
            Self::VertexCount => "请输入 3 至 32 个顶点，末尾可再填一次起点",
            Self::DuplicateVertex => "除末尾可重复起点外，各顶点须互不重合",
            Self::IntersectingEdges => "多边形的边相交、接触或重叠，请检查顶点顺序",
            Self::Evaluation(FractionError::Invalid | FractionError::Incomplete) => {
                "坐标须为整数、有限小数、分数或精确算式"
            }
            Self::Evaluation(error) => error.message(),
        }
    }
}

enum PolygonOperation {
    Area,
    Centroid,
    Perimeter,
    Bounds,
    Convexity,
    Contains(Point2),
}

pub(super) fn calculate_polygon(query: &str) -> Result<String, PolygonError> {
    let normalized = normalize(query);
    let input = normalized.trim();
    let (operation, arguments) = input.split_once(char::is_whitespace).unwrap_or((input, ""));
    let parse_coordinate = |part: &str| {
        parse_point(part).map_err(|error| match error {
            PointError::Arguments => PolygonError::PointArguments,
            PointError::Evaluation(error) => PolygonError::Evaluation(error),
        })
    };
    let arguments = arguments.trim();
    let (input, operation) = match operation.to_ascii_lowercase().as_str() {
        "重心" | "centroid" => (arguments, PolygonOperation::Centroid),
        "周长" | "perimeter" => (arguments, PolygonOperation::Perimeter),
        "边界框" | "bounds" => (arguments, PolygonOperation::Bounds),
        "凸性" | "convexity" => (arguments, PolygonOperation::Convexity),
        "包含" | "contains" => {
            let (point, vertices) = arguments
                .split_once('@')
                .ok_or(PolygonError::ContainmentArguments)?;
            if point.trim().is_empty() || vertices.trim().is_empty() {
                return Err(PolygonError::ContainmentArguments);
            }
            (
                vertices.trim(),
                PolygonOperation::Contains(parse_coordinate(point)?),
            )
        }
        _ => (input, PolygonOperation::Area),
    };
    if input.is_empty() {
        return Err(PolygonError::Incomplete);
    }
    let mut vertices = input
        .split(';')
        .take(MAX_VERTICES + 2)
        .map(parse_coordinate)
        .collect::<Result<Vec<_>, _>>()?;
    if vertices.len() > 1 && vertices.first() == vertices.last() {
        vertices.pop();
    }
    if !(3..=MAX_VERTICES).contains(&vertices.len()) {
        return Err(PolygonError::VertexCount);
    }
    let mut seen = HashSet::with_capacity(vertices.len());
    if vertices.iter().any(|point| !seen.insert(point)) {
        return Err(PolygonError::DuplicateVertex);
    }
    check_edges(&vertices)?;
    match operation {
        PolygonOperation::Area => polygon_area(&vertices, false),
        PolygonOperation::Centroid => polygon_area(&vertices, true),
        PolygonOperation::Perimeter => polygon_perimeter(&vertices),
        PolygonOperation::Bounds => polygon_bounds(&vertices),
        PolygonOperation::Convexity => polygon_convexity(&vertices).map(str::to_owned),
        PolygonOperation::Contains(point) => point_position(&point, &vertices).map(str::to_owned),
    }
}

fn polygon_area(vertices: &[Point2], centroid: bool) -> Result<String, PolygonError> {
    let origin = &vertices[0];
    let mut twice_area = BigRational::default();
    for pair in vertices[1..].windows(2) {
        twice_area += twice_triangle_area(origin, &pair[0], &pair[1])?;
        ensure_exact_size(&twice_area).map_err(PolygonError::Evaluation)?;
    }
    let sign = twice_area.numer().sign();
    let winding = match sign {
        Sign::Plus => "逆时针",
        Sign::Minus => "顺时针",
        Sign::NoSign => return Err(PolygonError::IntersectingEdges),
    };
    if centroid {
        return area_centroid(vertices, &twice_area);
    }
    let magnitude = if sign == Sign::Minus {
        -twice_area
    } else {
        twice_area
    };
    let area = magnitude / BigInt::from(2);
    ensure_exact_size(&area).map_err(PolygonError::Evaluation)?;
    Ok(format!(
        "面积 {area} · 顶点 {}\n顶点顺序 {winding}",
        vertices.len()
    ))
}

fn polygon_convexity(vertices: &[Point2]) -> Result<&'static str, PolygonError> {
    let count = vertices.len();
    let mut direction = Sign::NoSign;
    for (index, current) in vertices.iter().enumerate() {
        let turn = orientation(
            current,
            &vertices[(index + 1) % count],
            &vertices[(index + 2) % count],
        )?;
        if opposite_signs(direction, turn) {
            return Ok("凹多边形");
        }
        if turn != Sign::NoSign {
            direction = turn;
        }
    }
    Ok("凸多边形")
}

fn polygon_bounds(vertices: &[Point2]) -> Result<String, PolygonError> {
    let [x, y] = &vertices[0];
    let (mut min_x, mut max_x) = (x, x);
    let (mut min_y, mut max_y) = (y, y);
    for [x, y] in &vertices[1..] {
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }
    let width = max_x - min_x;
    let height = max_y - min_y;
    ensure_exact_size(&width).map_err(PolygonError::Evaluation)?;
    ensure_exact_size(&height).map_err(PolygonError::Evaluation)?;
    Ok(format!(
        "X [{min_x},{max_x}] · Y [{min_y},{max_y}]\n宽 {width} · 高 {height}"
    ))
}

fn polygon_perimeter(vertices: &[Point2]) -> Result<String, PolygonError> {
    let mut rational = BigRational::default();
    let mut radicals = BTreeMap::<BigRational, usize>::new();
    for (a, b) in vertices.iter().zip(vertices.iter().cycle().skip(1)) {
        let dx = &a[0] - &b[0];
        let dy = &a[1] - &b[1];
        let squared_length = &dx * &dx + &dy * &dy;
        ensure_exact_size(&squared_length).map_err(PolygonError::Evaluation)?;
        if let Some(length) = exact_abs_square_root(&squared_length) {
            rational += length;
            ensure_exact_size(&rational).map_err(PolygonError::Evaluation)?;
        } else {
            *radicals.entry(squared_length).or_default() += 1;
        }
    }
    let mut terms = Vec::new();
    if rational.numer().sign() != Sign::NoSign {
        terms.push(rational.to_string());
    }
    for (squared_length, count) in radicals {
        let radical = format_square_root(&squared_length);
        terms.push(if count == 1 {
            radical
        } else {
            format!("{count}{radical}")
        });
    }
    Ok(format!("周长 {}", terms.join(" + ")))
}

fn point_position(point: &Point2, vertices: &[Point2]) -> Result<&'static str, PolygonError> {
    let mut inside = false;
    for (a, b) in vertices.iter().zip(vertices.iter().cycle().skip(1)) {
        let turn = orientation(a, b, point)?;
        if turn == Sign::NoSign && in_edge_bounds(point, a, b) {
            return Ok("点在多边形边界上");
        }
        // 半开高度范围使经过顶点的水平射线不会把同一次穿越计两次。
        let crosses_height = (a[1] > point[1]) != (b[1] > point[1]);
        let rightward_turn = if a[1] < b[1] { Sign::Plus } else { Sign::Minus };
        if crosses_height && turn == rightward_turn {
            inside = !inside;
        }
    }
    Ok(if inside {
        "点在多边形内部"
    } else {
        "点在多边形外部"
    })
}

fn area_centroid(vertices: &[Point2], twice_area: &BigRational) -> Result<String, PolygonError> {
    let origin = &vertices[0];
    let mut moments = [BigRational::default(), BigRational::default()];
    for pair in vertices[1..].windows(2) {
        let weight = twice_triangle_area(origin, &pair[0], &pair[1])?;
        for (axis, moment) in moments.iter_mut().enumerate() {
            let axis_offset_sum =
                (&pair[0][axis] - &origin[axis]) + (&pair[1][axis] - &origin[axis]);
            *moment += &weight * axis_offset_sum;
            ensure_exact_size(moment).map_err(PolygonError::Evaluation)?;
        }
    }
    let [x, y] = moments;
    let divisor = twice_area * BigInt::from(3);
    let x = &origin[0] + x / &divisor;
    let y = &origin[1] + y / divisor;
    ensure_exact_size(&x).map_err(PolygonError::Evaluation)?;
    ensure_exact_size(&y).map_err(PolygonError::Evaluation)?;
    Ok(format!("重心 ({x}, {y})"))
}

fn check_edges(vertices: &[Point2]) -> Result<(), PolygonError> {
    let count = vertices.len();
    for (i, a) in vertices.iter().enumerate() {
        let b = &vertices[(i + 1) % count];
        let next = &vertices[(i + 2) % count];
        if orientation(a, b, next)? == Sign::NoSign
            && (in_edge_bounds(a, b, next) || in_edge_bounds(next, a, b))
        {
            return Err(PolygonError::IntersectingEdges);
        }
        let end = if i == 0 { count - 1 } else { count };
        for j in i + 2..end {
            let c = &vertices[j];
            let d = &vertices[(j + 1) % count];
            if edges_meet(a, b, c, d)? {
                return Err(PolygonError::IntersectingEdges);
            }
        }
    }
    Ok(())
}

fn twice_triangle_area(a: &Point2, b: &Point2, c: &Point2) -> Result<BigRational, PolygonError> {
    let area = (&b[0] - &a[0]) * (&c[1] - &a[1]) - (&b[1] - &a[1]) * (&c[0] - &a[0]);
    ensure_exact_size(&area).map_err(PolygonError::Evaluation)?;
    Ok(area)
}

fn orientation(a: &Point2, b: &Point2, c: &Point2) -> Result<Sign, PolygonError> {
    twice_triangle_area(a, b, c).map(|area| area.numer().sign())
}

fn in_edge_bounds(point: &Point2, a: &Point2, b: &Point2) -> bool {
    point
        .iter()
        .zip(a)
        .zip(b)
        .all(|((p, a), b)| (a.min(b)..=a.max(b)).contains(&p))
}

fn opposite_signs(a: Sign, b: Sign) -> bool {
    matches!(
        (a, b),
        (Sign::Plus, Sign::Minus) | (Sign::Minus, Sign::Plus)
    )
}

fn edges_meet(a: &Point2, b: &Point2, c: &Point2, d: &Point2) -> Result<bool, PolygonError> {
    let turn_c = orientation(a, b, c)?;
    let turn_d = orientation(a, b, d)?;
    let turn_a = orientation(c, d, a)?;
    let turn_b = orientation(c, d, b)?;
    Ok(
        (opposite_signs(turn_c, turn_d) && opposite_signs(turn_a, turn_b))
            || (turn_c == Sign::NoSign && in_edge_bounds(c, a, b))
            || (turn_d == Sign::NoSign && in_edge_bounds(d, a, b))
            || (turn_a == Sign::NoSign && in_edge_bounds(a, c, d))
            || (turn_b == Sign::NoSign && in_edge_bounds(b, c, d)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_convex_and_concave_polygons_with_collinear_boundary_points() {
        for (query, expected) in [
            ("凸性 0,0;4,0;2,2;4,4;0,4", "凹多边形"),
            ("凸性 0,0;0,4;4,4;2,2;4,0", "凹多边形"),
            ("凸性 0,0;1/2,0;1,0;1,1;0,1;0,0", "凸多边形"),
            ("convexity 0,0;0,1;1,1;1,0", "凸多边形"),
        ] {
            assert_eq!(calculate_polygon(query).as_deref(), Ok(expected));
        }
        assert_eq!(
            calculate_polygon("凸性 0,0;2,2;2,0;0,2"),
            Err(PolygonError::IntersectingEdges)
        );
    }

    #[test]
    fn finds_exact_area_and_winding_for_simple_polygons() {
        for (query, expected) in [
            ("0,0;4,0;2,2;4,4;0,4", "面积 12 · 顶点 5\n顶点顺序 逆时针"),
            ("0,0;0,4;4,4;2,2;4,0", "面积 12 · 顶点 5\n顶点顺序 顺时针"),
            (
                "０，０；1/2,0;1/2,1/4;0,1/4;0.00,0",
                "面积 1/8 · 顶点 4\n顶点顺序 逆时针",
            ),
            ("0,0;2,0;4,0;4,2;0,2", "面积 8 · 顶点 5\n顶点顺序 逆时针"),
        ] {
            assert_eq!(calculate_polygon(query).as_deref(), Ok(expected), "{query}");
        }
    }

    #[test]
    fn finds_exact_area_centroids_for_concave_and_translated_polygons() {
        for (query, expected) in [
            ("重心 0,0;3,0;3,1;1,1;1,3;0,3", "重心 (11/10, 11/10)"),
            ("centroid 0,0;0,3;1,3;1,1;3,1;3,0", "重心 (11/10, 11/10)"),
            (
                "CENTROID 1/2,-1/4;5/2,-1/4;5/2,3/4;1/2,3/4",
                "重心 (3/2, 1/4)",
            ),
        ] {
            assert_eq!(calculate_polygon(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            calculate_polygon("重心 0,0;5,4;0,4;4,0"),
            Err(PolygonError::IntersectingEdges)
        );
        assert_eq!(calculate_polygon("重心"), Err(PolygonError::Incomplete));
    }

    #[test]
    fn reports_axis_aligned_extrema_and_exact_width_and_height() {
        assert_eq!(
            calculate_polygon("边界框 -1/2,-2;2,-2;1,1;-1/2,1/3").as_deref(),
            Ok("X [-1/2,2] · Y [-2,1]\n宽 5/2 · 高 3")
        );
        assert_eq!(
            calculate_polygon("BOUNDS 0,0;0,4;4,4;2,2;4,0").as_deref(),
            Ok("X [0,4] · Y [0,4]\n宽 4 · 高 4")
        );
        assert_eq!(
            calculate_polygon("边界框 0,0;4,4;0,4;4,0"),
            Err(PolygonError::IntersectingEdges)
        );
    }

    #[test]
    fn sums_exact_rational_and_radical_edge_lengths_including_the_last_edge() {
        for (query, expected) in [
            ("周长 0,0;3,0;3,4", "周长 12"),
            ("周长 0,0;1,1;2,0", "周长 2 + 2√2"),
            ("perimeter 2,0;1,1;0,0;2,0", "周长 2 + 2√2"),
            ("周长 0,0;1/2,0;1/2,1/4;0,1/4", "周长 3/2"),
            ("周长 0,0;1/2,1/2;1,0", "周长 1 + 2√(1/2)"),
        ] {
            assert_eq!(calculate_polygon(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            calculate_polygon("周长 0,0;4,4;0,4;4,0"),
            Err(PolygonError::IntersectingEdges)
        );
    }

    #[test]
    fn classifies_exact_points_on_edges_vertices_and_both_sides_of_concave_polygons() {
        for (query, expected) in [
            ("包含 1/2,3/2 @ 0,0;4,0;2,2;4,4;0,4", "点在多边形内部"),
            ("包含 3,2 @ 0,0;4,0;2,2;4,4;0,4", "点在多边形外部"),
            ("包含 1,2 @ 0,0;4,0;2,2;4,4;0,4", "点在多边形内部"),
            ("包含 5/2,3/2 @ 0,0;4,0;2,2;4,4;0,4", "点在多边形边界上"),
            ("包含 3/2,0 @ 0,0;4,0;2,2;4,4;0,4", "点在多边形边界上"),
            ("CONTAINS 2,2 @ 0,0;0,4;4,4;2,2;4,0;0,0", "点在多边形边界上"),
            ("包含 5/2,0 @ 0,0;2,0;2,2;0,2", "点在多边形外部"),
        ] {
            assert_eq!(calculate_polygon(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            calculate_polygon("包含 0,0"),
            Err(PolygonError::ContainmentArguments)
        );
        assert_eq!(
            calculate_polygon("包含 1,1 @ 0,0;3,3;0,3;4,0"),
            Err(PolygonError::IntersectingEdges)
        );
    }

    #[test]
    fn rejects_crossings_touches_backtracking_and_out_of_range_counts() {
        for query in [
            "0,0;5,4;0,4;4,0",
            "0,0;4,0;4,4;2,0;0,4",
            "0,0;4,0;2,0;2,4;0,4",
            "0,0;1,0;2,0",
        ] {
            assert_eq!(
                calculate_polygon(query),
                Err(PolygonError::IntersectingEdges),
                "{query}"
            );
        }
        assert_eq!(
            calculate_polygon("0,0;4,0;4,4;4,0;0,4"),
            Err(PolygonError::DuplicateVertex)
        );
        let long = ["0,0"; MAX_VERTICES + 2].join(";");
        assert_eq!(calculate_polygon(&long), Err(PolygonError::VertexCount));
        assert_eq!(calculate_polygon("0,0;1,1"), Err(PolygonError::VertexCount));
        assert_eq!(
            calculate_polygon("0,0;1;0,1"),
            Err(PolygonError::PointArguments)
        );
    }
}
