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
    ZeroDirection,
    ZeroAngleVector,
    TooManyDimensions,
    Evaluation(FractionError),
}

impl VectorError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "用逗号隔开分量，用分号隔开两个向量",
            Self::UnknownOperation => "输入 ? 向量 查看可用操作",
            Self::Dimensions => "两个向量的分量数须一致",
            Self::CrossDimensions => "叉积需要两个三维向量",
            Self::ZeroDirection => "投影方向须为非零向量",
            Self::ZeroAngleVector => "计算夹角余弦需要两个非零向量",
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
    if matches!(operation.as_str(), "长度" | "norm" | "length") {
        let values = components(input)?;
        return Ok(format_square_root(&dot_product(&values, &values)?));
    }
    if !matches!(
        operation.as_str(),
        "点积"
            | "dot"
            | "叉积"
            | "cross"
            | "距离"
            | "distance"
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
        "叉积" | "cross" => cross_product(&left, &right),
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
        "距离" | "distance" => {
            let differences = vector_difference(&left, &right)?;
            Ok(format_square_root(&dot_product(
                &differences,
                &differences,
            )?))
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

fn format_vector(values: &[BigRational]) -> String {
    values
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
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

fn cross_product(left: &[BigRational], right: &[BigRational]) -> Result<String, VectorError> {
    let ([ax, ay, az], [bx, by, bz]) = (left, right) else {
        return Err(VectorError::CrossDimensions);
    };
    let [x, y, z] = [ay * bz - az * by, az * bx - ax * bz, ax * by - ay * bx];
    for value in [&x, &y, &z] {
        ensure_exact_size(value).map_err(VectorError::Evaluation)?;
    }
    Ok(format!("{x}, {y}, {z}"))
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
