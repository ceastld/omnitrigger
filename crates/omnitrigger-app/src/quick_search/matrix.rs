use num_bigint::{BigInt, Sign};
use num_rational::BigRational;

use super::calculator::normalize;
use super::equation::format_centered_roots;
use super::fraction::{
    calculate_fraction, ensure_exact_size, format_exact_multiple, format_square_root, FractionError,
};
use super::operators::split_top_level_commas;

pub(super) const MAX_DIMENSION: usize = 8;
const MAX_POWER: u32 = 64;
pub(super) type Matrix = Vec<Vec<BigRational>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MatrixError {
    Incomplete,
    UnknownOperation,
    UnevenRows,
    TooLarge,
    SquareRequired,
    SymmetricRequired,
    EigenvalueDimensions,
    ElementwiseDimensions,
    ProductDimensions,
    PowerExponent,
    ConditionNorm,
    SolveDimensions,
    GeneralSolutionDimensions,
    Singular,
    LeastSquaresRank,
    Evaluation(FractionError),
}

impl MatrixError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "用逗号隔开列，用分号或换行隔开行",
            Self::UnknownOperation => {
                "可用加法、减法、逐项乘法、乘法、乘方、数乘、F范数、1范数、无穷范数、条件数、正定性、行列式、迹、特征值、转置、求解、通解、最小二乘、求逆、秩、行最简形、零空间、行空间和列空间"
            }
            Self::UnevenRows => "每行的列数须一致",
            Self::TooLarge => "矩阵最多 8 行 8 列",
            Self::SquareRequired => "行列式、迹、乘方、求解、求逆和条件数需要方阵",
            Self::SymmetricRequired => "正定性检查需要实对称方阵",
            Self::EigenvalueDimensions => "请输入 2 行 2 列的矩阵求特征值",
            Self::ElementwiseDimensions => "加减法和逐项乘法要求两个矩阵的行数和列数分别相同",
            Self::ProductDimensions => "左矩阵的列数须等于右矩阵的行数",
            Self::PowerExponent => "矩阵乘方的指数须为 -64 至 64 的整数",
            Self::ConditionNorm => "填写 1 或 ∞，用 @ 隔开矩阵，例如 条件数 1 @ 2,0; 0,1/3",
            Self::SolveDimensions => "等号右侧的常数列或矩阵须与系数矩阵行数一致",
            Self::GeneralSolutionDimensions => "@ 右侧须为一列常数，行数与系数矩阵一致",
            Self::Singular => "系数矩阵不可逆，无法得到唯一解",
            Self::LeastSquaresRank => "系数矩阵的列线性相关，最小二乘解不唯一",
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
        "特征值" | "eigenvalues" => eigenvalues(&parse_matrix(input)?),
        "秩" | "rank" => matrix_rank(parse_matrix(input)?).map(|value| value.to_string()),
        "行最简形" | "行最简" | "rref" => {
            reduced_row_echelon(parse_matrix(input)?).map(|(matrix, _)| format_matrix(matrix))
        }
        "零空间" | "核空间" | "nullspace" => nullspace(parse_matrix(input)?),
        "行空间" | "row-space" => {
            let (mut rows, pivots) = reduced_row_echelon(parse_matrix(input)?)?;
            rows.truncate(pivots.len());
            Ok(format_basis("行空间", rows))
        }
        "列空间" | "column-space" => {
            let matrix = parse_matrix(input)?;
            let (_, pivots) = reduced_row_echelon(matrix.clone())?;
            let columns = pivots
                .into_iter()
                .map(|column| matrix.iter().map(|row| row[column].clone()).collect())
                .collect();
            Ok(format_basis("列空间", columns))
        }
        "加法" | "add" => elementwise(input, |left, right| *left += right),
        "减法" | "subtract" => elementwise(input, |left, right| *left -= right),
        "逐项乘法" | "hadamard" => elementwise(input, |left, right| *left *= right),
        "f范数" | "frobenius" => frobenius_norm(parse_matrix(input)?),
        "1范数" | "one-norm" => {
            max_absolute_row_sum(transpose(parse_matrix(input)?)).map(|norm| norm.to_string())
        }
        "无穷范数" | "∞范数" | "infinity-norm" => {
            max_absolute_row_sum(parse_matrix(input)?).map(|norm| norm.to_string())
        }
        "条件数" | "condition-number" => matrix_condition_number(input),
        "正定性" | "正定" | "positive-definite" => {
            matrix_positive_definiteness(parse_matrix(input)?)
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
        "通解" | "general-solution" => {
            let (left, right) = parse_pair(input)?;
            general_solution(left, right, None)
        }
        "最小二乘" | "least-squares" => {
            let (left, right) = parse_pair(input)?;
            least_squares(left, right).map(format_matrix)
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

pub(super) fn parse_matrix(input: &str) -> Result<Matrix, MatrixError> {
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

fn eigenvalues(matrix: &[Vec<BigRational>]) -> Result<String, MatrixError> {
    let [first, second] = matrix else {
        return Err(MatrixError::EigenvalueDimensions);
    };
    let ([a, b], [c, d]) = (first.as_slice(), second.as_slice()) else {
        return Err(MatrixError::EigenvalueDimensions);
    };
    let two = BigInt::from(2);
    let center = (a + d) / &two;
    let half_difference = (a - d) / two;
    let distance_squared = &half_difference * &half_difference + b * c;
    format_centered_roots(center, distance_squared, "λ").map_err(MatrixError::Evaluation)
}

fn scale_matrix(mut matrix: Matrix, factor: &BigRational) -> Result<Matrix, MatrixError> {
    for element in matrix.iter_mut().flatten() {
        *element *= factor;
        ensure_exact_size(element).map_err(MatrixError::Evaluation)?;
    }
    Ok(matrix)
}

fn matrix_positive_definiteness(mut matrix: Matrix) -> Result<String, MatrixError> {
    let dimension = matrix.len();
    if matrix[0].len() != dimension
        || matrix.iter().enumerate().any(|(row, values)| {
            values[..row]
                .iter()
                .enumerate()
                .any(|(column, value)| value != &matrix[column][row])
        })
    {
        return Err(MatrixError::SymmetricRequired);
    }
    let mut minor = BigRational::from_integer(1.into());
    let mut minors = Vec::with_capacity(dimension);
    for column in 0..dimension {
        let (leading, remaining) = matrix.split_at_mut(column + 1);
        let pivot = &leading[column];
        // 顺序主子式等于当前主元和之前各主元的乘积。
        minor *= &pivot[column];
        ensure_exact_size(&minor).map_err(MatrixError::Evaluation)?;
        if minor.numer().sign() != Sign::Plus {
            return Ok(format!(
                "正定性 · 否\n第 {} 阶顺序主子式 {minor}",
                column + 1
            ));
        }
        minors.push(minor.to_string());
        eliminate_below(pivot, remaining, column)?;
    }
    Ok(format!("正定性 · 是\n顺序主子式 {}", minors.join(", ")))
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

fn reduced_row_echelon(mut matrix: Matrix) -> Result<(Matrix, Vec<usize>), MatrixError> {
    let mut pivot_columns = Vec::new();
    for column in 0..matrix[0].len() {
        let row = pivot_columns.len();
        let Some(pivot) = find_pivot(&matrix, row, column) else {
            continue;
        };
        matrix.swap(row, pivot);
        reduce_pivot_column(&mut matrix, row, column)?;
        pivot_columns.push(column);
        if pivot_columns.len() == matrix.len() {
            break;
        }
    }
    Ok((matrix, pivot_columns))
}

fn nullspace(matrix: Matrix) -> Result<String, MatrixError> {
    let (matrix, pivot_columns) = reduced_row_echelon(matrix)?;
    let width = matrix[0].len();
    let basis = (0..width)
        .filter(|column| !pivot_columns.contains(column))
        .map(|free| {
            let mut vector = vec![BigRational::default(); width];
            vector[free] = BigRational::from_integer(1.into());
            for (&pivot, equation) in pivot_columns.iter().zip(&matrix) {
                vector[pivot] = -&equation[free];
            }
            vector
        })
        .collect::<Matrix>();
    Ok(format_basis("零空间", basis))
}

fn format_basis(space: &str, basis: Matrix) -> String {
    if basis.is_empty() {
        format!("{space}维数 0")
    } else {
        format!("{space}维数 {}\n{}", basis.len(), format_matrix(basis))
    }
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

pub(super) fn general_solution(
    mut coefficients: Matrix,
    constants: Matrix,
    names: Option<&[&str]>,
) -> Result<String, MatrixError> {
    if coefficients.len() != constants.len() || constants[0].len() != 1 {
        return Err(MatrixError::GeneralSolutionDimensions);
    }
    let width = coefficients[0].len();
    for (row, rhs) in coefficients.iter_mut().zip(constants) {
        row.extend(rhs);
    }
    let (reduced, pivots) = reduced_row_echelon(coefficients)?;
    // 增广列是主元列时，至少有一行要求 0 等于非零常数。
    if pivots.last() == Some(&width) {
        return Ok("无解".to_owned());
    }

    const SUBSCRIPTS: [char; MAX_DIMENSION] = ['₁', '₂', '₃', '₄', '₅', '₆', '₇', '₈'];
    let free_variables: Vec<_> = (0..width)
        .filter(|column| !pivots.contains(column))
        .enumerate()
        .map(|(index, column)| (column, format!("t{}", SUBSCRIPTS[index])))
        .collect();
    let mut expressions = vec![String::new(); width];
    for (&pivot, row) in pivots.iter().zip(&reduced) {
        let expression = &mut expressions[pivot];
        if row[width].numer().sign() != Sign::NoSign {
            expression.push_str(&row[width].to_string());
        }
        for (free, parameter) in &free_variables {
            append_parameter(expression, -&row[*free], parameter);
        }
        if expression.is_empty() {
            expression.push('0');
        }
    }
    for (free, parameter) in &free_variables {
        expressions[*free].clone_from(parameter);
    }
    let mut lines: Vec<_> = expressions
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            if let Some(name) = names.and_then(|names| names.get(index)) {
                format!("{name} = {value}")
            } else {
                format!("x{} = {value}", SUBSCRIPTS[index])
            }
        })
        .collect();
    if !free_variables.is_empty() {
        let parameters: Vec<_> = free_variables
            .into_iter()
            .map(|(_, parameter)| parameter)
            .collect();
        lines.push(format!("{} 为任意实数", parameters.join("、")));
    }
    Ok(lines.join("\n"))
}

fn append_parameter(expression: &mut String, coefficient: BigRational, parameter: &str) {
    let sign = coefficient.numer().sign();
    if sign == Sign::NoSign {
        return;
    }
    let negative = sign == Sign::Minus;
    let prefix = match (expression.is_empty(), negative) {
        (true, true) => "-",
        (true, false) => "",
        (false, true) => " - ",
        (false, false) => " + ",
    };
    expression.push_str(prefix);
    let magnitude = if negative { -coefficient } else { coefficient };
    expression.push_str(&format_exact_multiple(&magnitude, parameter));
}

fn least_squares(left: Matrix, right: Matrix) -> Result<Matrix, MatrixError> {
    if left.len() != right.len() {
        return Err(MatrixError::SolveDimensions);
    }
    if left[0].len() > left.len() {
        return Err(MatrixError::LeastSquaresRank);
    }
    let transposed = transpose(left.clone());
    let normal = multiply(&transposed, &left)?;
    let projected = multiply(&transposed, &right)?;
    solve(normal, projected).map_err(|error| match error {
        MatrixError::Singular => MatrixError::LeastSquaresRank,
        error => error,
    })
}

fn elementwise(
    input: &str,
    operation: impl Fn(&mut BigRational, BigRational),
) -> Result<String, MatrixError> {
    let (mut left, right) = parse_pair(input)?;
    if left.len() != right.len() || left[0].len() != right[0].len() {
        return Err(MatrixError::ElementwiseDimensions);
    }
    for (left_row, right_row) in left.iter_mut().zip(right) {
        for (left, right) in left_row.iter_mut().zip(right_row) {
            operation(left, right);
            ensure_exact_size(left).map_err(MatrixError::Evaluation)?;
        }
    }
    Ok(format_matrix(left))
}

fn frobenius_norm(matrix: Matrix) -> Result<String, MatrixError> {
    let mut squared_norm = BigRational::default();
    for value in matrix.into_iter().flatten() {
        squared_norm += &value * &value;
        ensure_exact_size(&squared_norm).map_err(MatrixError::Evaluation)?;
    }
    Ok(format_square_root(&squared_norm))
}

fn matrix_condition_number(input: &str) -> Result<String, MatrixError> {
    let (norm, input) = input.split_once('@').ok_or(MatrixError::ConditionNorm)?;
    let one_norm = match norm.trim().to_ascii_lowercase().as_str() {
        "1" => true,
        "∞" | "无穷" | "inf" | "infinity" => false,
        _ => return Err(MatrixError::ConditionNorm),
    };
    let matrix = parse_matrix(input)?;
    let inverse = match solve(matrix.clone(), identity(matrix.len())) {
        Ok(inverse) => inverse,
        Err(MatrixError::Singular) => return Ok("∞".to_owned()),
        Err(error) => return Err(error),
    };
    let (matrix, inverse) = if one_norm {
        (transpose(matrix), transpose(inverse))
    } else {
        (matrix, inverse)
    };
    let result = max_absolute_row_sum(matrix)? * max_absolute_row_sum(inverse)?;
    ensure_exact_size(&result).map_err(MatrixError::Evaluation)?;
    Ok(result.to_string())
}

fn max_absolute_row_sum(matrix: Matrix) -> Result<BigRational, MatrixError> {
    let mut maximum = BigRational::default();
    for row in matrix {
        let mut sum = BigRational::default();
        for value in row {
            sum += if value.numer().sign() == Sign::Minus {
                -value
            } else {
                value
            };
            ensure_exact_size(&sum).map_err(MatrixError::Evaluation)?;
        }
        maximum = maximum.max(sum);
    }
    Ok(maximum)
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

pub(super) fn format_matrix(matrix: Matrix) -> String {
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
    fn calculates_real_complex_and_repeated_eigenvalues_exactly() {
        for (query, expected) in [
            ("特征值 2,1; 1,2", "λ₁ = 1 · λ₂ = 3"),
            ("eigenvalues 0,-1; 1,0", "λ = ±i"),
            ("特征值 0,1; 2,0", "λ = ±√2"),
            ("特征值 3/2,1/4; 0,3/2", "λ = 3/2（重根）"),
            ("特征值 1/3,0; 0,2/3", "λ₁ = 1/3 · λ₂ = 2/3"),
        ] {
            assert_eq!(calculate_matrix(query).as_deref(), Ok(expected));
        }
        for query in ["特征值 1", "特征值 1,2; 3,4; 5,6", "特征值 1,2,3; 4,5,6"] {
            assert_eq!(
                calculate_matrix(query),
                Err(MatrixError::EigenvalueDimensions)
            );
        }
    }

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
    fn finds_an_exact_basis_for_each_free_column() {
        for (query, expected) in [
            ("零空间 0,2,3; 0,4,6", "零空间维数 2\n1, 0, 0\n0, -3/2, 1"),
            ("nullspace 1,2,-1; 0,0,1", "零空间维数 1\n-2, 1, 0"),
            ("核空间 0,0; 0,0", "零空间维数 2\n1, 0\n0, 1"),
            ("零空间 1,2; 3,4; 5,6", "零空间维数 0"),
        ] {
            assert_eq!(calculate_matrix(query).as_deref(), Ok(expected), "{query}");
        }
    }

    #[test]
    fn finds_row_bases_and_selects_pivot_columns_from_the_original_matrix() {
        for (query, expected) in [
            (
                "行空间 1,2,3; 2,4,6; 0,1,1",
                "行空间维数 2\n1, 0, 1\n0, 1, 1",
            ),
            (
                "column-space 1,2,3; 2,4,6; 0,1,1",
                "列空间维数 2\n1, 2, 0\n2, 4, 1",
            ),
            ("列空间 0,1/2,1; 0,1,2", "列空间维数 1\n1/2, 1"),
            ("row-space 0,0; 0,0", "行空间维数 0"),
            ("列空间 0,0,0", "列空间维数 0"),
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
            assert_eq!(
                calculate_matrix(query),
                Err(MatrixError::ElementwiseDimensions)
            );
        }
        assert_eq!(calculate_matrix("加法 1,2"), Err(MatrixError::Incomplete));
    }

    #[test]
    fn checks_positive_definiteness_using_exact_leading_principal_minors() {
        for (query, expected) in [
            ("正定性 2,1; 1,2", "正定性 · 是\n顺序主子式 2, 3"),
            (
                "positive-definite 1/2,0.5; 1/2,1",
                "正定性 · 是\n顺序主子式 1/2, 1/4",
            ),
            ("正定性 1,2; 2,1", "正定性 · 否\n第 2 阶顺序主子式 -3"),
            ("正定 1,1; 1,1", "正定性 · 否\n第 2 阶顺序主子式 0"),
            (
                "正定性 1,0,0;0,1,2;0,2,1",
                "正定性 · 否\n第 3 阶顺序主子式 -3",
            ),
            ("正定性 -1", "正定性 · 否\n第 1 阶顺序主子式 -1"),
        ] {
            assert_eq!(calculate_matrix(query).as_deref(), Ok(expected), "{query}");
        }
        for query in ["正定性 1,2", "正定性 1,2; 0,1"] {
            assert_eq!(calculate_matrix(query), Err(MatrixError::SymmetricRequired));
        }
    }

    #[test]
    fn calculates_exact_condition_numbers_and_detects_singular_matrices() {
        for (query, expected) in [
            ("条件数 1 @ 2,0; 0,1/3", "6"),
            ("条件数 ∞ @ 1,1; 0,2", "3"),
            ("condition-number INF @ -2", "1"),
            ("条件数 1 @ 1,2; 2,4", "∞"),
            ("条件数 ∞ @ 0", "∞"),
        ] {
            assert_eq!(calculate_matrix(query).as_deref(), Ok(expected));
        }
        for (query, error) in [
            ("条件数 2 @ 1", MatrixError::ConditionNorm),
            ("条件数 1 @ 1,2", MatrixError::SquareRequired),
        ] {
            assert_eq!(calculate_matrix(query), Err(error));
        }
    }

    #[test]
    fn calculates_exact_induced_one_and_infinity_norms_of_rectangular_matrices() {
        for (query, expected) in [
            ("1范数 1/2,-2;3,1", "7/2"),
            ("无穷范数 1/2,-2;3,1", "4"),
            ("one-norm -1,0,5", "5"),
            ("infinity-norm -1,0,5", "6"),
            ("∞范数 0", "0"),
        ] {
            assert_eq!(calculate_matrix(query).as_deref(), Ok(expected));
        }
    }

    #[test]
    fn calculates_elementwise_products_and_frobenius_norms_exactly() {
        for (query, expected) in [
            ("逐项乘法 1/2,2; 3,4 @ 3,1/3; 0,-1", "3/2, 2/3\n0, -4"),
            ("hadamard 1,2,3 @ -4,5,6", "-4, 10, 18"),
            ("F范数 1,2; 2,4", "5"),
            ("frobenius 1/2,1/2", "√(1/2)"),
            ("F范数 0,0,0; 0,0,0", "0"),
        ] {
            assert_eq!(calculate_matrix(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            calculate_matrix("逐项乘法 1,2 @ 3;4"),
            Err(MatrixError::ElementwiseDimensions),
        );
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
    fn finds_exact_least_squares_coefficients_for_one_or_more_response_columns() {
        assert_eq!(
            calculate_matrix("最小二乘 1,0;1,1;1,2 @ 1;2;2").as_deref(),
            Ok("7/6\n1/2")
        );
        assert_eq!(
            calculate_matrix("least-squares 1,0;1,1;1,2 @ 1,1;2,3;2,5").as_deref(),
            Ok("7/6, 1\n1/2, 2")
        );
        for (query, expected) in [
            (
                "最小二乘 1,1;2,2;3,3 @ 1;2;3",
                MatrixError::LeastSquaresRank,
            ),
            ("最小二乘 1,2 @ 1", MatrixError::LeastSquaresRank),
            ("最小二乘 1;2 @ 1", MatrixError::SolveDimensions),
        ] {
            assert_eq!(calculate_matrix(query), Err(expected));
        }
    }

    #[test]
    fn finds_exact_general_solutions_for_rectangular_or_singular_systems() {
        for (input, expected) in [
            (
                "通解 1,0,1; 0,1,0 @ 2; 1",
                "x₁ = 2 - t₁\nx₂ = 1\nx₃ = t₁\nt₁ 为任意实数",
            ),
            (
                "通解 1,2,3 @ 0",
                "x₁ = -2t₁ - 3t₂\nx₂ = t₁\nx₃ = t₂\nt₁、t₂ 为任意实数",
            ),
            (
                "通解 0,0; 0,0 @ 0; 0",
                "x₁ = t₁\nx₂ = t₂\nt₁、t₂ 为任意实数",
            ),
            (
                "通解 0,1; 0,2; 0,3 @ 1; 2; 3",
                "x₁ = t₁\nx₂ = 1\nt₁ 为任意实数",
            ),
            ("general-solution 1/2,0; 0,3 @ 1/6; 2", "x₁ = 1/3\nx₂ = 2/3"),
            ("通解 2,4; 1,2 @ 6; 4", "无解"),
        ] {
            assert_eq!(calculate_matrix(input).as_deref(), Ok(expected), "{input}");
        }
        assert_eq!(
            calculate_matrix("通解 1,0; 0,1 @ 1,2; 3,4"),
            Err(MatrixError::GeneralSolutionDimensions)
        );
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
