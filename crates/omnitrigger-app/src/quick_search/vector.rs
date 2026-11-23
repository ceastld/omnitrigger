use num_bigint::Sign;
use num_rational::BigRational;

use super::calculator::normalize;
use super::fraction::{
    calculate_fraction, ensure_exact_size, format_signed_square_root, format_square_root,
    FractionError,
};
use super::operators::split_top_level_commas;

const MAX_DIMENSIONS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum VectorError {
    Incomplete,
    UnknownOperation,
    Dimensions,
    CrossDimensions,
    LineArguments,
    CoincidentLinePoints,
    PlaneArguments,
    PlaneDimensions,
    CollinearPlanePoints,
    ZeroDirection,
    ZeroAngleVector,
    ZeroUnitVector,
    TooManyDimensions,
    Evaluation(FractionError),
}

impl VectorError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "用逗号隔开分量，用分号隔开两个向量",
            Self::UnknownOperation => "输入 ? 向量 查看可用操作",
            Self::Dimensions => "各向量的分量数须一致",
            Self::CrossDimensions => "叉积需要两个三维向量",
            Self::LineArguments => "用 @ 隔开待测点和两个已知点，两个已知点用分号隔开",
            Self::CoincidentLinePoints => "直线需要两个不同的点",
            Self::PlaneArguments => "用 @ 隔开待测点和三个平面点，平面点用分号隔开",
            Self::PlaneDimensions => "待测点和平面上的三个点都须使用三维坐标",
            Self::CollinearPlanePoints => "平面上的三个点须互不重合且不在同一直线上",
            Self::ZeroDirection => "投影方向须为非零向量",
            Self::ZeroAngleVector => "计算夹角余弦需要两个非零向量",
            Self::ZeroUnitVector => "计算单位向量需要非零向量",
            Self::TooManyDimensions => "每个向量最多 32 个分量",
            Self::Evaluation(FractionError::Invalid | FractionError::Incomplete) => {
                "分量须为整数、有限小数、分数或精确算式"
            }
            Self::Evaluation(error) => error.message(),
        }
    }
}

pub(super) fn calculate_vector(query: &str) -> Result<String, VectorError> {
    let normalized = normalize(query);
    let (operation, input) = normalized
        .trim()
        .split_once(char::is_whitespace)
        .ok_or(VectorError::Incomplete)?;
    let operation = operation.to_ascii_lowercase();
    match operation.as_str() {
        "长度" | "norm" | "length" => {
            let values = components(input)?;
            return Ok(format_square_root(&dot_product(&values, &values)?));
        }
        "单位向量" | "unit-vector" => return unit_vector(&components(input)?),
        "线段距离" | "segment-distance" => return project_onto_line(input, true)?.distance(),
        "直线距离" | "line-distance" => return project_onto_line(input, false)?.distance(),
        "平面距离" | "plane-distance" => return project_onto_plane(input)?.distance(),
        "直线对称" | "line-reflection" => return project_onto_line(input, false)?.reflection(),
        "平面对称" | "plane-reflection" => return project_onto_plane(input)?.reflection(),
        _ => {}
    }
    if !matches!(
        operation.as_str(),
        "点积"
            | "dot"
            | "叉积"
            | "cross"
            | "距离"
            | "distance"
            | "曼哈顿距离"
            | "manhattan-distance"
            | "切比雪夫距离"
            | "chebyshev-distance"
            | "投影"
            | "project"
            | "分解"
            | "decompose"
            | "夹角余弦"
            | "余弦相似度"
            | "cosine"
    ) {
        return Err(VectorError::UnknownOperation);
    }
    let (left, right) = input.split_once(';').ok_or(VectorError::Incomplete)?;
    let left = components(left)?;
    let right = components(right)?;
    match operation.as_str() {
        "叉积" | "cross" => Ok(format_vector(&cross_product(&left, &right)?)),
        _ if left.len() != right.len() => Err(VectorError::Dimensions),
        "投影" | "project" => Ok(format_vector(&project_vector(&left, &right)?)),
        "分解" | "decompose" => {
            let parallel = project_vector(&left, &right)?;
            let perpendicular = vector_difference(&left, &parallel)?;
            Ok(format!(
                "平行 {}\n垂直 {}",
                format_vector(&parallel),
                format_vector(&perpendicular),
            ))
        }
        "夹角余弦" | "余弦相似度" | "cosine" => vector_cosine(&left, &right),
        "距离" | "distance" => Ok(format_square_root(&squared_euclidean_distance(
            &left, &right,
        )?)),
        "曼哈顿距离" | "manhattan-distance" | "切比雪夫距离" | "chebyshev-distance" => {
            let mut differences = vector_difference(&left, &right)?.into_iter().map(|value| {
                if value.numer().sign() == Sign::Minus {
                    -value
                } else {
                    value
                }
            });
            let distance = if matches!(operation.as_str(), "切比雪夫距离" | "chebyshev-distance")
            {
                differences.max().unwrap_or_default()
            } else {
                differences.try_fold(BigRational::default(), |total, value| {
                    let total = total + value;
                    ensure_exact_size(&total).map_err(VectorError::Evaluation)?;
                    Ok(total)
                })?
            };
            Ok(distance.to_string())
        }
        _ => Ok(dot_product(&left, &right)?.to_string()),
    }
}

fn components(input: &str) -> Result<Vec<BigRational>, VectorError> {
    let terms: Vec<_> = split_top_level_commas(input)
        .take(MAX_DIMENSIONS + 1)
        .collect();
    if terms.len() > MAX_DIMENSIONS {
        return Err(VectorError::TooManyDimensions);
    }
    terms
        .into_iter()
        .map(|term| calculate_fraction(term).map_err(VectorError::Evaluation))
        .collect()
}

fn dot_product(left: &[BigRational], right: &[BigRational]) -> Result<BigRational, VectorError> {
    let mut result = BigRational::default();
    for (a, b) in left.iter().zip(right) {
        result += a * b;
        ensure_exact_size(&result).map_err(VectorError::Evaluation)?;
    }
    Ok(result)
}

fn project_vector(
    vector: &[BigRational],
    direction: &[BigRational],
) -> Result<Vec<BigRational>, VectorError> {
    let squared_length = dot_product(direction, direction)?;
    if squared_length == BigRational::default() {
        return Err(VectorError::ZeroDirection);
    }
    let scale = dot_product(vector, direction)? / squared_length;
    ensure_exact_size(&scale).map_err(VectorError::Evaluation)?;
    direction
        .iter()
        .map(|component| {
            let value = component * &scale;
            ensure_exact_size(&value).map_err(VectorError::Evaluation)?;
            Ok(value)
        })
        .collect()
}

fn vector_difference(
    left: &[BigRational],
    right: &[BigRational],
) -> Result<Vec<BigRational>, VectorError> {
    left.iter()
        .zip(right)
        .map(|(a, b)| {
            let value = a - b;
            ensure_exact_size(&value).map_err(VectorError::Evaluation)?;
            Ok(value)
        })
        .collect()
}

pub(super) fn squared_euclidean_distance(
    left: &[BigRational],
    right: &[BigRational],
) -> Result<BigRational, VectorError> {
    if left.len() != right.len() {
        return Err(VectorError::Dimensions);
    }
    let differences = vector_difference(left, right)?;
    dot_product(&differences, &differences)
}

fn format_vector(values: &[BigRational]) -> String {
    values
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

fn unit_vector(values: &[BigRational]) -> Result<String, VectorError> {
    let squared_length = dot_product(values, values)?;
    if squared_length == BigRational::default() {
        return Err(VectorError::ZeroUnitVector);
    }
    let components = values
        .iter()
        .map(|component| {
            let squared = (component * component) / &squared_length;
            ensure_exact_size(&squared).map_err(VectorError::Evaluation)?;
            Ok(format_signed_square_root(
                &squared,
                component.numer().sign(),
            ))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(components.join(", "))
}

fn vector_cosine(left: &[BigRational], right: &[BigRational]) -> Result<String, VectorError> {
    let squared_lengths = dot_product(left, left)? * dot_product(right, right)?;
    if squared_lengths == BigRational::default() {
        return Err(VectorError::ZeroAngleVector);
    }
    let dot = dot_product(left, right)?;
    let squared = (&dot * &dot) / squared_lengths;
    ensure_exact_size(&squared).map_err(VectorError::Evaluation)?;
    Ok(format_signed_square_root(&squared, dot.numer().sign()))
}

struct PointProjection {
    point: Vec<BigRational>,
    closest: Vec<BigRational>,
}

impl PointProjection {
    fn distance(&self) -> Result<String, VectorError> {
        let difference = vector_difference(&self.point, &self.closest)?;
        let distance = format_square_root(&dot_product(&difference, &difference)?);
        Ok(format!(
            "距离 {distance}\n最近点 {}",
            format_vector(&self.closest)
        ))
    }

    fn reflection(&self) -> Result<String, VectorError> {
        let coordinates = self
            .closest
            .iter()
            .zip(&self.point)
            .map(|(foot, point)| {
                let value = foot + (foot - point);
                ensure_exact_size(&value).map_err(VectorError::Evaluation)?;
                Ok(value)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(format_vector(&coordinates))
    }
}

fn project_onto_line(
    input: &str,
    restrict_to_segment: bool,
) -> Result<PointProjection, VectorError> {
    let (point, line) = input.split_once('@').ok_or(VectorError::LineArguments)?;
    let (start, end) = line.split_once(';').ok_or(VectorError::LineArguments)?;
    let (point, start, end) = (components(point)?, components(start)?, components(end)?);
    if point.len() != start.len() || start.len() != end.len() {
        return Err(VectorError::Dimensions);
    }
    let direction = vector_difference(&end, &start)?;
    let squared_length = dot_product(&direction, &direction)?;
    let zero = BigRational::default();
    let closest = if squared_length == zero {
        if !restrict_to_segment {
            return Err(VectorError::CoincidentLinePoints);
        }
        start
    } else {
        let offset = vector_difference(&point, &start)?;
        let weight = dot_product(&offset, &direction)? / squared_length;
        let weight = if restrict_to_segment {
            weight.clamp(zero, BigRational::from_integer(1.into()))
        } else {
            weight
        };
        start
            .into_iter()
            .zip(direction)
            .map(|(origin, step)| {
                let coordinate = origin + step * &weight;
                ensure_exact_size(&coordinate).map_err(VectorError::Evaluation)?;
                Ok(coordinate)
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    Ok(PointProjection { point, closest })
}

fn project_onto_plane(input: &str) -> Result<PointProjection, VectorError> {
    let (point, plane) = input.split_once('@').ok_or(VectorError::PlaneArguments)?;
    let points: Vec<_> = plane.split(';').take(4).collect();
    let [origin, next, last] = points.as_slice() else {
        return Err(VectorError::PlaneArguments);
    };
    let point = components(point)?;
    let (origin, next, last) = (components(origin)?, components(next)?, components(last)?);
    if [&point, &origin, &next, &last]
        .iter()
        .any(|point| point.len() != 3)
    {
        return Err(VectorError::PlaneDimensions);
    }
    let left = vector_difference(&next, &origin)?;
    let right = vector_difference(&last, &origin)?;
    let normal = cross_product(&left, &right)?;
    let from_origin = vector_difference(&point, &origin)?;
    let perpendicular = project_vector(&from_origin, &normal).map_err(|error| match error {
        VectorError::ZeroDirection => VectorError::CollinearPlanePoints,
        _ => error,
    })?;
    let closest = vector_difference(&point, &perpendicular)?;
    Ok(PointProjection { point, closest })
}

fn cross_product(
    left: &[BigRational],
    right: &[BigRational],
) -> Result<[BigRational; 3], VectorError> {
    let ([ax, ay, az], [bx, by, bz]) = (left, right) else {
        return Err(VectorError::CrossDimensions);
    };
    let [x, y, z] = [ay * bz - az * by, az * bx - ax * bz, ax * by - ay * bx];
    for value in [&x, &y, &z] {
        ensure_exact_size(value).map_err(VectorError::Evaluation)?;
    }
    Ok([x, y, z])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decomposes_a_vector_into_exact_parallel_and_perpendicular_components() {
        for (query, expected) in [
            ("分解 2,1; 1,1", "平行 3/2, 3/2\n垂直 1/2, -1/2"),
            ("decompose 1,2,3; 0,-1,0", "平行 0, 2, 0\n垂直 1, 0, 3"),
            ("分解 2,0; -1,0", "平行 2, 0\n垂直 0, 0"),
            ("分解 0,0; 1,0", "平行 0, 0\n垂直 0, 0"),
        ] {
            assert_eq!(calculate_vector(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            calculate_vector("分解 1,2; 0,0"),
            Err(VectorError::ZeroDirection)
        );
        assert_eq!(
            calculate_vector("分解 1; 1,2"),
            Err(VectorError::Dimensions)
        );
    }

    #[test]
    fn returns_exact_unit_vector_components_and_rejects_a_zero_vector() {
        for (query, expected) in [
            ("单位向量 3,4", "3/5, 4/5"),
            ("unit-vector -1,1,0", "-√(1/2), √(1/2), 0"),
            ("单位向量 -1/2", "-1"),
        ] {
            assert_eq!(calculate_vector(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            calculate_vector("单位向量 0,0"),
            Err(VectorError::ZeroUnitVector)
        );
    }

    #[test]
    fn reflects_points_across_a_line_or_plane_with_exact_coordinates() {
        for (query, expected) in [
            ("直线对称 3,4 @ 0,0; 2,0", "3, -4"),
            ("line-reflection 1/2,0 @ 0,0; 1,1", "0, 1/2"),
            ("直线对称 2,1,2 @ 0,1,0; 0,1,1", "-2, 1, 2"),
            ("平面对称 3,4,5 @ 0,0,0; 1,0,0; 0,1,0", "3, 4, -5"),
            (
                "plane-reflection 1,1,1 @ 1,0,0; 0,1,0; 0,0,1",
                "-1/3, -1/3, -1/3",
            ),
            ("平面对称 1/2,-2,0 @ 0,0,0; 1,0,0; 0,1,0", "1/2, -2, 0"),
        ] {
            assert_eq!(calculate_vector(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            ("直线对称 0,0 @ 1,1; 1,1", VectorError::CoincidentLinePoints),
            (
                "平面对称 1,2,3 @ 0,0,0; 1,0,0; 2,0,0",
                VectorError::CollinearPlanePoints,
            ),
        ] {
            assert_eq!(calculate_vector(query), Err(error), "{query}");
        }
    }

    #[test]
    fn projects_onto_the_full_line_beyond_either_reference_point() {
        for (query, expected) in [
            ("直线距离 3,4 @ 0,0; 2,0", "距离 4\n最近点 3, 0"),
            (
                "line-distance -1/3,1/2 @ 2,0; 0,0",
                "距离 1/2\n最近点 -1/3, 0",
            ),
            ("直线距离 1,2,3 @ 1,0,0; 1,1,0", "距离 3\n最近点 1, 2, 0"),
        ] {
            assert_eq!(calculate_vector(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            calculate_vector("直线距离 0,0 @ 3,4; 3,4"),
            Err(VectorError::CoincidentLinePoints)
        );
    }

    #[test]
    fn finds_exact_distance_and_projection_to_a_plane_in_3d() {
        for (query, expected) in [
            (
                "平面距离 3,4,5 @ 0,0,0; 1,0,0; 0,1,0",
                "距离 5\n最近点 3, 4, 0",
            ),
            (
                "PLANE-DISTANCE 1,1,1 @ 1,0,0; 0,1,0; 0,0,1",
                "距离 √(4/3)\n最近点 1/3, 1/3, 1/3",
            ),
            (
                "平面距离 １，１，０ ＠ ０，０，０；１，０，０；０，１，０",
                "距离 0\n最近点 1, 1, 0",
            ),
        ] {
            assert_eq!(calculate_vector(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            ("平面距离 3,4,5 @ 0,0,0; 1,0,0", VectorError::PlaneArguments),
            (
                "平面距离 3,4 @ 0,0,0; 1,0,0; 0,1,0",
                VectorError::PlaneDimensions,
            ),
            (
                "平面距离 3,4,5 @ 0,0,0; 1,0,0; 2,0,0",
                VectorError::CollinearPlanePoints,
            ),
        ] {
            assert_eq!(calculate_vector(query), Err(error), "{query}");
        }
    }

    #[test]
    fn finds_the_exact_closest_point_on_a_finite_segment() {
        for (query, expected) in [
            ("线段距离 3,4 @ 0,0; 2,0", "距离 √17\n最近点 2, 0"),
            ("线段距离 -1,1 @ 0,0; 2,0", "距离 √2\n最近点 0, 0"),
            ("segment-distance 1,1 @ 0,0; 2,0", "距离 1\n最近点 1, 0"),
            ("线段距离 0,0 @ 3,4; 3,4", "距离 5\n最近点 3, 4"),
            ("线段距离 1/2,1/2 @ 0,0; 1,1", "距离 0\n最近点 1/2, 1/2"),
        ] {
            assert_eq!(calculate_vector(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            calculate_vector("线段距离 3,4 @ 0,0; 2,0,1"),
            Err(VectorError::Dimensions)
        );
    }

    #[test]
    fn computes_exact_manhattan_and_chebyshev_distances() {
        for (query, expected) in [
            ("曼哈顿距离 1/2,-2,3; -1,-1/4,3", "13/4"),
            ("切比雪夫距离 1/2,-2,3; -1,-1/4,3", "7/4"),
            ("MANHATTAN-DISTANCE -1,-1/4,3; 1/2,-2,3", "13/4"),
            ("chebyshev-distance -1/2; 1/3", "5/6"),
            ("切比雪夫距离 1/2,3; 0.50,3", "0"),
        ] {
            assert_eq!(calculate_vector(query).as_deref(), Ok(expected), "{query}");
        }
        for query in ["曼哈顿距离 1,2; 1", "切比雪夫距离 1; 1,2"] {
            assert_eq!(calculate_vector(query), Err(VectorError::Dimensions));
        }
    }

    #[test]
    fn computes_exact_vector_products_norms_and_distances() {
        for (query, expected) in [
            ("点积 １，２，３；４，５，６", "32"),
            ("dot 9007199254740993, 1/3; 1, 3", "9007199254740994"),
            ("dot comb(3,2), -2; 2,3", "0"),
            ("叉积 1/2,0,0; 0,1/3,0", "0, 0, 1/6"),
            ("cross 1,2,3; 4,5,6", "-3, 6, -3"),
            ("长度 3/4,1", "5/4"),
            ("norm 1/2,1/2", "√(1/2)"),
            ("距离 0,0; 1,1", "√2"),
            ("distance 1,2; 1,2", "0"),
            ("投影 2,1; 1,1", "3/2, 3/2"),
            ("project 1,2,3; 0,-1,0", "0, 2, 0"),
            ("project 1,2; 2,-1", "0, 0"),
            ("夹角余弦 1,1; -1,0", "-√(1/2)"),
            ("余弦相似度 1/2,0; 2,0", "1"),
            ("cosine 1,0; -2,0", "-1"),
            ("cosine 1,0; 0,1", "0"),
        ] {
            assert_eq!(calculate_vector(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            ("点积 1,2", VectorError::Incomplete),
            ("未知 1", VectorError::UnknownOperation),
            ("点积 1;1,2", VectorError::Dimensions),
            ("叉积 1,2;3,4", VectorError::CrossDimensions),
            ("project 1,2; 0,0", VectorError::ZeroDirection),
            ("project 1; 1,2", VectorError::Dimensions),
            ("cosine 0,0; 1,2", VectorError::ZeroAngleVector),
            ("cosine 1,2; 0,0", VectorError::ZeroAngleVector),
            ("cosine 1; 1,2", VectorError::Dimensions),
        ] {
            assert_eq!(calculate_vector(query), Err(error), "{query}");
        }
        assert_eq!(
            calculate_vector(&format!("长度 {}1", "1,".repeat(MAX_DIMENSIONS))),
            Err(VectorError::TooManyDimensions)
        );
    }
}
