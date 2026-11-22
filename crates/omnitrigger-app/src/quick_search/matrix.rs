use num_bigint::Sign;
use num_rational::BigRational;

use super::calculator::normalize;
use super::fraction::{calculate_fraction, ensure_exact_size, FractionError};
use super::operators::split_top_level_commas;

const MAX_DIMENSION: usize = 8;
const MAX_POWER: u32 = 64;
type Matrix = Vec<Vec<BigRational>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MatrixError {
    Incomplete,
    UnknownOperation,
    UnevenRows,
    TooLarge,
    SquareRequired,
    SumDimensions,
    ProductDimensions,
    PowerExponent,
    SolveDimensions,
    Singular,
    Evaluation(FractionError),
}

impl MatrixError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "用逗号隔开列，用分号或换行隔开行",
            Self::UnknownOperation => {
                "可用加法、减法、乘法、乘方、数乘、行列式、迹、转置、求解、求逆、秩和行最简形"
            }
            Self::UnevenRows => "每行的列数须一致",
            Self::TooLarge => "矩阵最多 8 行 8 列",
            Self::SquareRequired => "行列式、迹、乘方、求解和求逆需要方阵",
            Self::SumDimensions => "加减法要求两个矩阵的行数和列数分别相同",
            Self::ProductDimensions => "左矩阵的列数须等于右矩阵的行数",
            Self::PowerExponent => "矩阵乘方的指数须为 -64 至 64 的整数",
            Self::SolveDimensions => "等号右侧的常数列或矩阵须与系数矩阵行数一致",
            Self::Singular => "系数矩阵不可逆，无法得到唯一解",
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
    let operation = operation.to_ascii_lowercase();
    match operation.as_str() {
        "行列式" | "det" => determinant(parse_matrix(input)?).map(|value| value.to_string()),
        "迹" | "trace" => trace(&parse_matrix(input)?).map(|value| value.to_string()),
        "秩" | "rank" => matrix_rank(parse_matrix(input)?).map(|value| value.to_string()),
        "行最简形" | "行最简" | "rref" => {
            reduced_row_echelon(parse_matrix(input)?).map(format_matrix)
        }
        "加法" | "add" | "减法" | "subtract" => {
            let (left, right) = parse_pair(input)?;
            let subtract = matches!(operation.as_str(), "减法" | "subtract");
            add_or_subtract(left, right, subtract).map(format_matrix)
        }
        "乘法" | "乘" | "multiply" => {
            let (left, right) = parse_pair(input)?;
            multiply(&left, &right).map(format_matrix)
        }
        "乘方" | "幂" | "power" => {
            let (matrix, exponent) = input.split_once('@').ok_or(MatrixError::Incomplete)?;
            let exponent = exponent
                .trim()
                .parse::<i32>()
                .ok()
                .filter(|exponent| exponent.unsigned_abs() <= MAX_POWER)
                .ok_or(MatrixError::PowerExponent)?;
            integer_power(parse_matrix(matrix)?, exponent).map(format_matrix)
        }
        "数乘" | "scale" => {
            let (factor, matrix) = input.split_once('@').ok_or(MatrixError::Incomplete)?;
            let factor = calculate_fraction(factor).map_err(MatrixError::Evaluation)?;
            scale_matrix(parse_matrix(matrix)?, &factor).map(format_matrix)
        }
        "转置" | "transpose" => Ok(format_matrix(transpose(parse_matrix(input)?))),
        "求解" | "solve" => {
            let (left, right) = parse_pair(input)?;
            solve(left, right).map(format_matrix)
        }
        "求逆" | "inverse" => {
            let matrix = parse_matrix(input)?;
            let dimension = matrix.len();
            solve(matrix, identity(dimension)).map(format_matrix)
        }
        _ => Err(MatrixError::UnknownOperation),
    }
}

fn parse_pair(input: &str) -> Result<(Matrix, Matrix), MatrixError> {
    let (left, right) = input.split_once('@').ok_or(MatrixError::Incomplete)?;
    Ok((parse_matrix(left)?, parse_matrix(right)?))
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

fn identity(dimension: usize) -> Matrix {
    (0..dimension)
        .map(|row| {
            (0..dimension)
                .map(|column| BigRational::from_integer(u8::from(row == column).into()))
                .collect()
        })
        .collect()
}

fn integer_power(matrix: Matrix, exponent: i32) -> Result<Matrix, MatrixError> {
    let dimension = matrix.len();
    if matrix[0].len() != dimension {
        return Err(MatrixError::SquareRequired);
    }
    let mut base = if exponent < 0 {
        solve(matrix, identity(dimension))?
    } else {
        matrix
    };
    let mut result = identity(dimension);
    let mut exponent = exponent.unsigned_abs();
    while exponent > 0 {
        if exponent & 1 != 0 {
            result = multiply(&result, &base)?;
        }
        exponent >>= 1;
        if exponent > 0 {
            base = multiply(&base, &base)?;
        }
    }
    Ok(result)
}

fn trace(matrix: &[Vec<BigRational>]) -> Result<BigRational, MatrixError> {
    if matrix.len() != matrix[0].len() {
        return Err(MatrixError::SquareRequired);
    }
    let mut sum = BigRational::default();
    for (index, row) in matrix.iter().enumerate() {
        sum += &row[index];
        ensure_exact_size(&sum).map_err(MatrixError::Evaluation)?;
    }
    Ok(sum)
}

fn scale_matrix(mut matrix: Matrix, factor: &BigRational) -> Result<Matrix, MatrixError> {
    for element in matrix.iter_mut().flatten() {
        *element *= factor;
        ensure_exact_size(element).map_err(MatrixError::Evaluation)?;
    }
    Ok(matrix)
}

fn determinant(mut matrix: Matrix) -> Result<BigRational, MatrixError> {
    let dimension = matrix.len();
    if matrix[0].len() != dimension {
        return Err(MatrixError::SquareRequired);
    }
    let mut result = BigRational::from_integer(1.into());
    for column in 0..dimension {
        let Some(pivot_row) = find_pivot(&matrix, column, column) else {
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
        eliminate_below(pivot, remaining, column)?;
    }
    Ok(result)
}

fn matrix_rank(mut matrix: Matrix) -> Result<usize, MatrixError> {
    let mut rank = 0;
    for column in 0..matrix[0].len() {
        let Some(pivot) = find_pivot(&matrix, rank, column) else {
            continue;
        };
        matrix.swap(rank, pivot);
        let (leading, remaining) = matrix.split_at_mut(rank + 1);
        eliminate_below(&leading[rank], remaining, column)?;
        rank += 1;
        if rank == matrix.len() {
            break;
        }
    }
    Ok(rank)
}

fn find_pivot(matrix: &[Vec<BigRational>], start_row: usize, column: usize) -> Option<usize> {
    (start_row..matrix.len()).find(|&row| matrix[row][column].numer().sign() != Sign::NoSign)
}

fn eliminate_below(
    pivot: &[BigRational],
    remaining: &mut [Vec<BigRational>],
    column: usize,
) -> Result<(), MatrixError> {
    for row in remaining {
        let factor = &row[column] / &pivot[column];
        ensure_exact_size(&factor).map_err(MatrixError::Evaluation)?;
        // 左侧列已完成，只更新剩余的待消元列。
        for (target, source) in row.iter_mut().zip(pivot).skip(column + 1) {
            *target -= &factor * source;
            ensure_exact_size(target).map_err(MatrixError::Evaluation)?;
        }
    }
    Ok(())
}

fn reduced_row_echelon(mut matrix: Matrix) -> Result<Matrix, MatrixError> {
    let mut row = 0;
    for column in 0..matrix[0].len() {
        let Some(pivot) = find_pivot(&matrix, row, column) else {
            continue;
        };
        matrix.swap(row, pivot);
        reduce_pivot_column(&mut matrix, row, column)?;
        row += 1;
        if row == matrix.len() {
            break;
        }
    }
    Ok(matrix)
}

fn reduce_pivot_column(
    matrix: &mut [Vec<BigRational>],
    row: usize,
    column: usize,
) -> Result<(), MatrixError> {
    let (before, remaining) = matrix.split_at_mut(row);
    let (pivot, after) = remaining.split_first_mut().expect("主元行存在");
    let scale = pivot[column].clone();
    for cell in &mut pivot[column..] {
        *cell /= &scale;
        ensure_exact_size(cell).map_err(MatrixError::Evaluation)?;
    }
    for row in before.iter_mut().chain(after) {
        let factor = std::mem::take(&mut row[column]);
        if factor.numer().sign() == Sign::NoSign {
            continue;
        }
        for (target, source) in row.iter_mut().zip(pivot.iter()).skip(column + 1) {
            *target -= &factor * source;
            ensure_exact_size(target).map_err(MatrixError::Evaluation)?;
        }
    }
    Ok(())
}

fn solve(mut left: Matrix, right: Matrix) -> Result<Matrix, MatrixError> {
    let dimension = left.len();
    if left[0].len() != dimension {
        return Err(MatrixError::SquareRequired);
    }
    if right.len() != dimension {
        return Err(MatrixError::SolveDimensions);
    }
    for (row, right_row) in left.iter_mut().zip(right) {
        row.extend(right_row);
    }
    for column in 0..dimension {
        let pivot = find_pivot(&left, column, column).ok_or(MatrixError::Singular)?;
        left.swap(column, pivot);
        reduce_pivot_column(&mut left, column, column)?;
    }
    Ok(left
        .into_iter()
        .map(|mut row| row.split_off(dimension))
        .collect())
}

fn add_or_subtract(mut left: Matrix, right: Matrix, subtract: bool) -> Result<Matrix, MatrixError> {
    if left.len() != right.len() || left[0].len() != right[0].len() {
        return Err(MatrixError::SumDimensions);
    }
    for (left_row, right_row) in left.iter_mut().zip(right) {
        for (left, right) in left_row.iter_mut().zip(right_row) {
            if subtract {
                *left -= right;
            } else {
                *left += right;
            }
            ensure_exact_size(left).map_err(MatrixError::Evaluation)?;
        }
    }
    Ok(left)
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
    fn reduces_rectangular_and_singular_matrices_with_exact_pivot_columns() {
        for (query, expected) in [
            (
                "行最简形 0,2,3; 1,4,5; 2,8,10",
                "1, 0, -1\n0, 1, 3/2\n0, 0, 0",
            ),
            ("rref 0,2,3; 0,4,6", "0, 1, 3/2\n0, 0, 0"),
            ("行最简形 0,0,-3", "0, 0, 1"),
            ("行最简形 0,0; 0,0", "0, 0\n0, 0"),
            ("rref 1/3; -2/3; 5", "1\n0\n0"),
        ] {
            assert_eq!(calculate_matrix(query).as_deref(), Ok(expected), "{query}");
        }
    }

    #[test]
    fn raises_square_matrices_to_positive_zero_and_negative_integer_powers() {
        for (query, expected) in [
            ("乘方 1,1;1,0 @ 5", "8, 5\n5, 3"),
            ("power 1,2;2,4 @ 0", "1, 0\n0, 1"),
            ("幂 1,1;1,0 @ -2", "1, -1\n-1, 2"),
            ("乘方 1/2,0;0,3 @ -1", "2, 0\n0, 1/3"),
        ] {
            assert_eq!(calculate_matrix(query).as_deref(), Ok(expected));
        }
        for (query, error) in [
            ("乘方 1,2;2,4 @ -1", MatrixError::Singular),
            ("乘方 1,2 @ 0", MatrixError::SquareRequired),
            ("乘方 1 @ 65", MatrixError::PowerExponent),
            ("乘方 1 @ 1/3", MatrixError::PowerExponent),
        ] {
            assert_eq!(calculate_matrix(query), Err(error));
        }
    }

    #[test]
    fn calculates_exact_matrix_trace_and_scales_rectangular_matrices() {
        for (query, expected) in [
            ("迹 1/2,20; 3,-1/3", "1/6"),
            ("trace 0", "0"),
            ("数乘 -3/2 @ 1/3,2; -1,0", "-1/2, -3\n3/2, 0"),
            ("scale 0 @ 1,-2,3", "0, 0, 0"),
        ] {
            assert_eq!(calculate_matrix(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            calculate_matrix("迹 1,2; 3,4; 5,6"),
            Err(MatrixError::SquareRequired)
        );
    }

    #[test]
    fn adds_and_subtracts_equal_shape_matrices_exactly() {
        for (query, expected) in [
            ("加法 1/2,2; 3,4 @ 1/3,5; -3,1", "5/6, 7\n0, 5"),
            ("减法 1/2,2; 3,4 @ 1/3,5; -3,1", "1/6, -3\n6, 3"),
            ("add 1/500,0.04 @ 3e-3,1/100", "1/200, 1/20"),
            ("subtract 9007199254740993 @ 9007199254740992", "1"),
        ] {
            assert_eq!(calculate_matrix(query).as_deref(), Ok(expected), "{query}");
        }
        for query in ["加法 1,2 @ 1", "减法 1;2 @ 1", "add 1,2 @ 1;2"] {
            assert_eq!(calculate_matrix(query), Err(MatrixError::SumDimensions));
        }
        assert_eq!(calculate_matrix("加法 1,2"), Err(MatrixError::Incomplete));
    }

    #[test]
    fn finds_exact_matrix_rank_with_skipped_columns_and_dependent_rows() {
        for (query, expected) in [
            ("秩 1,2,3; 2,4,6", "1"),
            ("rank 0,0,1; 0,0,2; 0,1,0", "2"),
            ("秩 0,0; 0,0", "0"),
            ("秩 1; 2; 3", "1"),
            ("秩 1/2,1/3; 3/2,1; 0,0", "1"),
            (
                "秩 9007199254740993,9007199254740992; 9007199254740992,9007199254740992",
                "2",
            ),
        ] {
            assert_eq!(calculate_matrix(query).as_deref(), Ok(expected), "{query}");
        }
    }

    #[test]
    fn solves_exact_square_systems_and_inverts_matrices() {
        for (query, expected) in [
            ("求解 0,2; 3,4 @ 4; 11", "1\n2"),
            ("求解 1/2,0; 0,3 @ 1/6; 2", "1/3\n2/3"),
            ("solve 2,0; 0,4 @ 1,2; 3,4", "1/2, 1\n3/4, 1"),
            ("求逆 1,2; 3,4", "-2, 1\n3/2, -1/2"),
            ("inverse -3/2", "-2/3"),
        ] {
            assert_eq!(calculate_matrix(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            ("求逆 1,2; 2,4", MatrixError::Singular),
            ("求解 1,2; 2,4 @ 3; 5", MatrixError::Singular),
            ("求解 1,2; 2,4 @ 3; 6", MatrixError::Singular),
            ("求逆 1,2,3; 4,5,6", MatrixError::SquareRequired),
            ("求解 1,0; 0,1 @ 1", MatrixError::SolveDimensions),
        ] {
            assert_eq!(calculate_matrix(query), Err(error), "{query}");
        }
    }

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
