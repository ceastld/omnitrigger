use std::collections::HashSet;

use num_bigint::{BigInt, Sign};
use num_rational::BigRational;

use super::{
    calculator::normalize,
    fraction::{ensure_exact_size, FractionError},
    plane::{parse_point, Point2, PointError},
};

const MAX_VERTICES: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PolygonError {
    Incomplete,
    PointArguments,
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

pub(super) fn calculate_polygon(query: &str) -> Result<String, PolygonError> {
    let normalized = normalize(query);
    let input = normalized.trim();
    let (operation, arguments) = input.split_once(char::is_whitespace).unwrap_or((input, ""));
    let centroid = operation == "重心" || operation.eq_ignore_ascii_case("centroid");
    let input = if centroid { arguments.trim() } else { input };
    if input.is_empty() {
        return Err(PolygonError::Incomplete);
    }
    let mut vertices = input
        .split(';')
        .take(MAX_VERTICES + 2)
        .map(|part| {
            parse_point(part).map_err(|error| match error {
                PointError::Arguments => PolygonError::PointArguments,
                PointError::Evaluation(error) => PolygonError::Evaluation(error),
            })
        })
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
        return area_centroid(&vertices, &twice_area);
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
