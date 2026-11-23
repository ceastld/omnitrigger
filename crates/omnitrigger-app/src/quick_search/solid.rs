use num_bigint::{BigInt, Sign};
use num_rational::BigRational;

use super::calculator::normalize;
use super::circle::multiple_of_pi;
use super::fraction::{
    calculate_fraction, ensure_exact_size, exact_abs_square_root, format_square_root, FractionError,
};
use super::operators::split_top_level_commas;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SolidError {
    Arguments,
    NonPositiveDimension,
    InvalidInnerRadius,
    Evaluation(FractionError),
}

impl SolidError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Arguments => "填入形状和尺寸，例如 圆柱 3,5。按 F1 查看各形状的参数",
            Self::NonPositiveDimension => "每项尺寸须大于零",
            Self::InvalidInnerRadius => "内半径须小于外半径",
            Self::Evaluation(error) => error.message(),
        }
    }
}

pub(super) fn calculate_solid(query: &str) -> Result<String, SolidError> {
    let input = normalize(query);
    let (shape, dimensions) = input
        .trim()
        .split_once(char::is_whitespace)
        .ok_or(SolidError::Arguments)?;
    let dimensions: Vec<_> = split_top_level_commas(dimensions).take(4).collect();
    let (surface, volume, with_pi) =
        match (shape.to_ascii_lowercase().as_str(), dimensions.as_slice()) {
            ("圆柱" | "cylinder", [radius, height]) => {
                let radius = positive_dimension(radius)?;
                let height = positive_dimension(height)?;
                let surface = &radius * (&radius + &height) * BigInt::from(2);
                let volume = &radius * &radius * height;
                (surface, volume, true)
            }
            ("圆锥" | "cone", [radius, height]) => return cone(radius, height),
            ("圆台" | "圆锥台" | "frustum", [first, second, height]) => {
                return conical_frustum(first, second, height);
            }
            ("圆管" | "tube" | "hollow-cylinder", [outer, inner, height]) => {
                let outer = positive_dimension(outer)?;
                let inner = positive_dimension(inner)?;
                let height = positive_dimension(height)?;
                if inner >= outer {
                    return Err(SolidError::InvalidInnerRadius);
                }
                let radii_sum = &outer + &inner;
                let annulus = (&outer - &inner) * &radii_sum;
                let surface = (&radii_sum * &height + &annulus) * BigInt::from(2);
                let volume = annulus * height;
                (surface, volume, true)
            }
            ("球" | "球体" | "sphere", [radius]) => {
                let radius = positive_dimension(radius)?;
                let surface = &radius * &radius * BigInt::from(4);
                let volume = &surface * radius / BigInt::from(3);
                (surface, volume, true)
            }
            ("立方体" | "cube", [side]) => {
                let side = positive_dimension(side)?;
                let area = &side * &side;
                let volume = &area * side;
                (area * BigInt::from(6), volume, false)
            }
            ("长方体" | "cuboid", [length, width, height]) => {
                let length = positive_dimension(length)?;
                let width = positive_dimension(width)?;
                let height = positive_dimension(height)?;
                let surface =
                    (&length * &width + &width * &height + &height * &length) * BigInt::from(2);
                let volume = length * width * height;
                (surface, volume, false)
            }
            _ => return Err(SolidError::Arguments),
        };
    ensure_exact_size(&surface).map_err(SolidError::Evaluation)?;
    ensure_exact_size(&volume).map_err(SolidError::Evaluation)?;
    let (surface, volume) = if with_pi {
        (multiple_of_pi(&surface), multiple_of_pi(&volume))
    } else {
        (surface.to_string(), volume.to_string())
    };
    Ok(format!("表面积 {surface} · 体积 {volume}"))
}

fn cone(radius: &str, height: &str) -> Result<String, SolidError> {
    let radius = positive_dimension(radius)?;
    let height = positive_dimension(height)?;
    let base = &radius * &radius;
    let lateral_square = &base * (&base + &height * &height);
    let volume = &base * height / BigInt::from(3);
    format_circular_solid(base, lateral_square, volume)
}

fn conical_frustum(first: &str, second: &str, height: &str) -> Result<String, SolidError> {
    let first = positive_dimension(first)?;
    let second = positive_dimension(second)?;
    let height = positive_dimension(height)?;
    let base = &first * &first + &second * &second;
    let radius_sum = &first + &second;
    let radius_difference = &first - &second;
    let slant_square = &radius_difference * &radius_difference + &height * &height;
    let lateral_square = &radius_sum * &radius_sum * slant_square;
    let volume = (&base + first * second) * height / BigInt::from(3);
    format_circular_solid(base, lateral_square, volume)
}

fn format_circular_solid(
    base: BigRational,
    lateral_square: BigRational,
    volume: BigRational,
) -> Result<String, SolidError> {
    for value in [&base, &lateral_square, &volume] {
        ensure_exact_size(value).map_err(SolidError::Evaluation)?;
    }
    let surface = if let Some(lateral) = exact_abs_square_root(&lateral_square) {
        let coefficient = base + lateral;
        ensure_exact_size(&coefficient).map_err(SolidError::Evaluation)?;
        multiple_of_pi(&coefficient)
    } else {
        format!("π({base} + {})", format_square_root(&lateral_square))
    };
    Ok(format!(
        "表面积 {surface} · 体积 {}",
        multiple_of_pi(&volume)
    ))
}

fn positive_dimension(query: &str) -> Result<BigRational, SolidError> {
    let dimension = calculate_fraction(query.trim()).map_err(SolidError::Evaluation)?;
    if dimension.numer().sign() != Sign::Plus {
        return Err(SolidError::NonPositiveDimension);
    }
    Ok(dimension)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculates_exact_surfaces_and_volumes() {
        for (query, expected) in [
            ("圆柱 3,5", "表面积 48π · 体积 45π"),
            ("圆锥 3,4", "表面积 24π · 体积 12π"),
            ("圆锥 1,1", "表面积 π(1 + √2) · 体积 π/3"),
            ("CONE 3/2,2", "表面积 6π · 体积 3π/2"),
            ("圆台 2,5,4", "表面积 64π · 体积 52π"),
            ("FRUSTUM 5,2,4", "表面积 64π · 体积 52π"),
            ("圆台 2,1,1", "表面积 π(5 + √18) · 体积 7π/3"),
            ("圆台 2,2,3", "表面积 20π · 体积 12π"),
            ("圆管 3,2,5", "表面积 60π · 体积 25π"),
            ("HOLLOW-CYLINDER 3/2,1/2,1/2", "表面积 6π · 体积 π"),
            ("ＣＹＬＩＮＤＥＲ １／２，３", "表面积 7π/2 · 体积 3π/4"),
            ("球 1/2", "表面积 π · 体积 π/6"),
            ("sphere 3", "表面积 36π · 体积 36π"),
            ("cube 1/2", "表面积 3/2 · 体积 1/8"),
            ("长方体 2,3,4", "表面积 52 · 体积 24"),
            ("长方体 1+1,3,(2+2)", "表面积 52 · 体积 24"),
        ] {
            assert_eq!(calculate_solid(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            calculate_solid("球 -1"),
            Err(SolidError::NonPositiveDimension)
        );
        assert_eq!(
            calculate_solid("圆柱 1,0"),
            Err(SolidError::NonPositiveDimension)
        );
        assert_eq!(
            calculate_solid("圆锥 0,4"),
            Err(SolidError::NonPositiveDimension)
        );
        assert_eq!(
            calculate_solid("圆台 2,-1,4"),
            Err(SolidError::NonPositiveDimension)
        );
        for query in ["圆管 2,2,3", "圆管 2,3,3"] {
            assert_eq!(calculate_solid(query), Err(SolidError::InvalidInnerRadius));
        }
        assert_eq!(calculate_solid("长方体 1,2"), Err(SolidError::Arguments));
        assert_eq!(calculate_solid("圆柱 1,2,3"), Err(SolidError::Arguments));
    }
}
