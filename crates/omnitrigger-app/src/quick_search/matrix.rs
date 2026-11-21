use num_bigint::Sign;
use num_rational::BigRational;

use super::calculator::normalize;
use super::fraction::{calculate_fraction, ensure_exact_size, FractionError};
use super::operators::split_top_level_commas;

const MAX_DIMENSION: usize = 8;
type Matrix = Vec<Vec<BigRational>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MatrixError {
    Incomplete,
    UnknownOperation,
    UnevenRows,
    TooLarge,
    SquareRequired,
    ProductDimensions,
    Evaluation(FractionError),
}

impl MatrixError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "用逗号隔开列，用分号或换行隔开行",
            Self::UnknownOperation => "可用 行列式、乘法 或 转置",
            Self::UnevenRows => "每行的列数须一致",
            Self::TooLarge => "矩阵最多 8 行 8 列",
            Self::SquareRequired => "行列式需要方阵",
            Self::ProductDimensions => "左矩阵的列数须等于右矩阵的行数",
            Self::Evaluation(FractionError::Invalid | FractionError::Incomplete) => {
                "矩阵元素须为整数、有限小数、分数或精确算式"
            }
            Self::Evaluation(error) => error.message(),
        }
    }
}

pub(super) fn calculate_matrix(query: &str) -> Result<String, MatrixError> {
    let normalized = normalize(query);
    let (operation, input) = normalized
        .trim()
        .split_once(char::is_whitespace)
        .ok_or(MatrixError::Incomplete)?;
    match operation.to_ascii_lowercase().as_str() {
        "行列式" | "det" => determinant(parse_matrix(input)?).map(|value| value.to_string()),
        "乘法" | "乘" | "multiply" => {
            let (left, right) = input.split_once('@').ok_or(MatrixError::Incomplete)?;
            multiply(&parse_matrix(left)?, &parse_matrix(right)?).map(format_matrix)
        }
        "转置" | "transpose" => Ok(format_matrix(transpose(parse_matrix(input)?))),
        _ => Err(MatrixError::UnknownOperation),
    }
}

fn parse_matrix(input: &str) -> Result<Matrix, MatrixError> {
    let rows: Vec<_> = input
        .trim()
        .split([';', '\n'])
        .take(MAX_DIMENSION + 1)
        .collect();
    if rows.len() > MAX_DIMENSION {
        return Err(MatrixError::TooLarge);
    }
    let mut values: Matrix = Vec::with_capacity(rows.len());
    for row in rows {
        let terms: Vec<_> = split_top_level_commas(row)
            .take(MAX_DIMENSION + 1)
            .collect();
        if terms.len() > MAX_DIMENSION {
            return Err(MatrixError::TooLarge);
        }
        if values
            .first()
            .is_some_and(|first| first.len() != terms.len())
        {
            return Err(MatrixError::UnevenRows);
        }
        let cells = terms
            .into_iter()
            .map(|term| calculate_fraction(term).map_err(MatrixError::Evaluation))
            .collect::<Result<_, _>>()?;
        values.push(cells);
    }
    Ok(values)
}

fn determinant(mut matrix: Matrix) -> Result<BigRational, MatrixError> {
    let dimension = matrix.len();
    if matrix[0].len() != dimension {
        return Err(MatrixError::SquareRequired);
    }
    let mut result = BigRational::from_integer(1.into());
    for column in 0..dimension {
        let Some(pivot_row) =
            (column..dimension).find(|&row| matrix[row][column].numer().sign() != Sign::NoSign)
        else {
            return Ok(BigRational::default());
        };
        if pivot_row != column {
            matrix.swap(column, pivot_row);
            result = -result;
        }
        let (leading, remaining) = matrix.split_at_mut(column + 1);
        let pivot = &leading[column];
        result *= &pivot[column];
        ensure_exact_size(&result).map_err(MatrixError::Evaluation)?;
        for row in remaining {
            let factor = &row[column] / &pivot[column];
            ensure_exact_size(&factor).map_err(MatrixError::Evaluation)?;
            // 左侧列已完成，只更新剩余的待消元列。
            for (target, source) in row.iter_mut().zip(pivot).skip(column + 1) {
                *target -= &factor * source;
                ensure_exact_size(target).map_err(MatrixError::Evaluation)?;
            }
        }
    }
    Ok(result)
}

fn multiply(left: &[Vec<BigRational>], right: &[Vec<BigRational>]) -> Result<Matrix, MatrixError> {
    if left[0].len() != right.len() {
        return Err(MatrixError::ProductDimensions);
    }
    left.iter()
        .map(|row| {
            (0..right[0].len())
                .map(|column| {
                    let mut sum = BigRational::default();
                    for (a, other_row) in row.iter().zip(right) {
                        sum += a * &other_row[column];
                        ensure_exact_size(&sum).map_err(MatrixError::Evaluation)?;
                    }
                    Ok(sum)
                })
                .collect()
        })
        .collect()
}

fn transpose(matrix: Matrix) -> Matrix {
    let mut result = vec![Vec::new(); matrix[0].len()];
    for row in matrix {
        for (column, value) in result.iter_mut().zip(row) {
            column.push(value);
        }
    }
    result
}

fn format_matrix(matrix: Matrix) -> String {
    matrix
        .into_iter()
        .map(|row| {
            row.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculates_exact_determinants_products_and_transposes() {
        for (query, expected) in [
            ("行列式 0,2; 3,4", "-6"),
            ("det 1,2,3; 2,4,6; 1,0,1", "0"),
            ("det 1/2,0,0; 0,1/3,0; 0,0,1/4", "1/24"),
            ("det 1,2,3;0,4,5;1,0,6", "22"),
            ("det 9007199254740993", "9007199254740993"),
            ("乘法 1,2;3,4 @ 5,6;7,8", "19, 22\n43, 50"),
            ("multiply 1/2, 3 @ 2\n1/3", "2"),
            ("转置 １，２，３；４，５，６", "1, 4\n2, 5\n3, 6"),
            ("transpose comb(3,2),0.1", "3\n1/10"),
        ] {
            assert_eq!(calculate_matrix(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            ("det 1,2", MatrixError::SquareRequired),
            ("det 1,2; 3", MatrixError::UnevenRows),
            ("multiply 1,2 @ 3,4", MatrixError::ProductDimensions),
            ("multiply 1,2", MatrixError::Incomplete),
        ] {
            assert_eq!(calculate_matrix(query), Err(error), "{query}");
        }
        assert_eq!(
            calculate_matrix(&format!("transpose {}1", "1,".repeat(MAX_DIMENSION))),
            Err(MatrixError::TooLarge)
        );
    }
}
