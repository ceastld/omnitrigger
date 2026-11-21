use num_rational::BigRational;

use super::calculator::normalize;
use super::fraction::{calculate_fraction, ensure_exact_size, format_square_root, FractionError};
use super::operators::split_top_level_commas;

const MAX_DIMENSIONS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum VectorError {
    Incomplete,
    UnknownOperation,
    Dimensions,
    CrossDimensions,
    TooManyDimensions,
    Evaluation(FractionError),
}

impl VectorError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "用逗号隔开分量，用分号隔开两个向量",
            Self::UnknownOperation => "可用 点积、叉积、长度 或 距离",
            Self::Dimensions => "两个向量的分量数须一致",
            Self::CrossDimensions => "叉积需要两个三维向量",
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
        "点积" | "dot" | "叉积" | "cross" | "距离" | "distance"
    ) {
        return Err(VectorError::UnknownOperation);
    }
    let (left, right) = input.split_once(';').ok_or(VectorError::Incomplete)?;
    let left = components(left)?;
    let right = components(right)?;
    match operation.as_str() {
        "叉积" | "cross" => cross_product(&left, &right),
        _ if left.len() != right.len() => Err(VectorError::Dimensions),
        "距离" | "distance" => {
            let differences: Vec<_> = left.iter().zip(&right).map(|(a, b)| a - b).collect();
            for value in &differences {
                ensure_exact_size(value).map_err(VectorError::Evaluation)?;
            }
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
        ] {
            assert_eq!(calculate_vector(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            ("点积 1,2", VectorError::Incomplete),
            ("未知 1", VectorError::UnknownOperation),
            ("点积 1;1,2", VectorError::Dimensions),
            ("叉积 1,2;3,4", VectorError::CrossDimensions),
        ] {
            assert_eq!(calculate_vector(query), Err(error), "{query}");
        }
        assert_eq!(
            calculate_vector(&format!("长度 {}1", "1,".repeat(MAX_DIMENSIONS))),
            Err(VectorError::TooManyDimensions)
        );
    }
}
