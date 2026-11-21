use std::cmp::Ordering;

use num_bigint::{BigInt, Sign};

use super::calculator::normalize;
use super::fraction::{calculate_fraction, ensure_exact_size, format_square_root, FractionError};
use super::operators::split_top_level_commas;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TriangleError {
    Dimensions,
    NonPositive,
    Degenerate,
    Evaluation(FractionError),
}

impl TriangleError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Dimensions
            | Self::Evaluation(FractionError::Invalid | FractionError::Incomplete) => {
                "用逗号隔开三条边，示例 3,4,5"
            }
            Self::NonPositive => "三条边的长度须大于零",
            Self::Degenerate => "较短两边之和须大于最长边",
            Self::Evaluation(error) => error.message(),
        }
    }
}

pub(super) fn calculate_triangle(query: &str) -> Result<String, TriangleError> {
    let input = normalize(query);
    let mut terms = split_top_level_commas(&input);
    let (Some(a), Some(b), Some(c), None) =
        (terms.next(), terms.next(), terms.next(), terms.next())
    else {
        return Err(TriangleError::Dimensions);
    };
    let side = |value| calculate_fraction(value).map_err(TriangleError::Evaluation);
    let mut sides = [side(a)?, side(b)?, side(c)?];
    if sides.iter().any(|value| value.numer().sign() != Sign::Plus) {
        return Err(TriangleError::NonPositive);
    }
    sides.sort();
    let [a, b, c] = sides;
    let shorter = &a + &b;
    if shorter <= c {
        return Err(TriangleError::Degenerate);
    }

    let perimeter = shorter + &c;
    let semiperimeter = &perimeter / BigInt::from(2);
    let area_squared =
        &semiperimeter * (&semiperimeter - &a) * (&semiperimeter - &b) * (&semiperimeter - &c);
    for value in [&perimeter, &area_squared] {
        ensure_exact_size(value).map_err(TriangleError::Evaluation)?;
    }
    let area = format_square_root(&area_squared);
    let shape = if a == c {
        "等边三角形"
    } else {
        let right_limit = &a * &a + &b * &b;
        let largest_square = &c * &c;
        match (a == b || b == c, largest_square.cmp(&right_limit)) {
            (true, Ordering::Less) => "等腰锐角三角形",
            (true, Ordering::Equal) => "等腰直角三角形",
            (true, Ordering::Greater) => "等腰钝角三角形",
            (false, Ordering::Less) => "锐角三角形",
            (false, Ordering::Equal) => "直角三角形",
            (false, Ordering::Greater) => "钝角三角形",
        }
    };
    Ok(format!("面积 {area} · 周长 {perimeter}\n{shape}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculates_triangle_geometry_with_exact_sides() {
        for (input, expected) in [
            ("３，４，５", "面积 6 · 周长 12\n直角三角形"),
            ("5,3,4", "面积 6 · 周长 12\n直角三角形"),
            ("2,2,2", "面积 √3 · 周长 6\n等边三角形"),
            ("5,5,6", "面积 12 · 周长 16\n等腰锐角三角形"),
            ("1,1,3/2", "面积 √(63/256) · 周长 7/2\n等腰钝角三角形"),
            ("3/4,1,1.25", "面积 3/8 · 周长 3\n直角三角形"),
            ("comb(3,2), 4, 5", "面积 6 · 周长 12\n直角三角形"),
        ] {
            assert_eq!(
                calculate_triangle(input).as_deref(),
                Ok(expected),
                "{input}"
            );
        }
        for (input, error) in [
            ("1,1", TriangleError::Dimensions),
            ("0,1,1", TriangleError::NonPositive),
            ("1,1,2", TriangleError::Degenerate),
            ("1,1,2.0000000000000000000001", TriangleError::Degenerate),
        ] {
            assert_eq!(calculate_triangle(input), Err(error), "{input}");
        }
    }
}
