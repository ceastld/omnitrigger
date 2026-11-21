use num_bigint::{BigInt, Sign};
use num_rational::BigRational;

use super::calculator::normalize;
use super::fraction::{calculate_fraction, ensure_exact_size, FractionError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CircleError {
    Incomplete,
    NonPositiveRadius,
    AngleOutOfRange,
    Evaluation(FractionError),
}

impl CircleError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入半径或直径，可加 @ 和圆心角",
            Self::NonPositiveRadius => "半径和直径须大于零",
            Self::AngleOutOfRange => "圆心角须在 0 至 360 度之间",
            Self::Evaluation(FractionError::Invalid | FractionError::Incomplete) => {
                "长度和角度须为整数、有限小数、分数或精确算式"
            }
            Self::Evaluation(error) => error.message(),
        }
    }
}

pub(super) fn calculate_circle(query: &str) -> Result<String, CircleError> {
    let input = normalize(query);
    let input = input.trim();
    let (length, angle) = input
        .split_once('@')
        .map_or((input, None), |(length, angle)| {
            (length.trim(), Some(angle.trim()))
        });
    let (length, is_diameter) = match length.split_once(char::is_whitespace) {
        Some((unit, value)) if unit == "直径" || unit.eq_ignore_ascii_case("diameter") => {
            (value.trim(), true)
        }
        Some((unit, value)) if unit == "半径" || unit.eq_ignore_ascii_case("radius") => {
            (value.trim(), false)
        }
        _ => (length, false),
    };
    if length.is_empty() {
        return Err(CircleError::Incomplete);
    }
    let radius = calculate_fraction(length).map_err(CircleError::Evaluation)?;
    if radius.numer().sign() != Sign::Plus {
        return Err(CircleError::NonPositiveRadius);
    }
    let two = BigInt::from(2);
    let (radius, diameter) = if is_diameter {
        (&radius / &two, radius)
    } else {
        let diameter = &radius * &two;
        (radius, diameter)
    };
    let area = &radius * &radius;
    for value in [&radius, &diameter, &area] {
        ensure_exact_size(value).map_err(CircleError::Evaluation)?;
    }
    let Some(angle) = angle else {
        return Ok(format!(
            "半径 {radius} · 直径 {diameter}\n周长 {} · 面积 {}",
            multiple_of_pi(&diameter),
            multiple_of_pi(&area),
        ));
    };
    let angle = angle.strip_suffix(['°', '度']).unwrap_or(angle).trim();
    if angle.is_empty() {
        return Err(CircleError::Incomplete);
    }
    let angle = calculate_fraction(angle).map_err(CircleError::Evaluation)?;
    let complete = BigInt::from(360);
    if angle.numer().sign() == Sign::Minus || angle.numer() > &(&complete * angle.denom()) {
        return Err(CircleError::AngleOutOfRange);
    }
    let ratio = angle / complete;
    let arc = diameter * &ratio;
    let sector = area * ratio;
    for value in [&arc, &sector] {
        ensure_exact_size(value).map_err(CircleError::Evaluation)?;
    }
    Ok(format!(
        "弧长 {} · 扇形面积 {}",
        multiple_of_pi(&arc),
        multiple_of_pi(&sector),
    ))
}

fn multiple_of_pi(value: &BigRational) -> String {
    if value.numer().sign() == Sign::NoSign {
        return "0".to_owned();
    }
    let numerator = value.numer().to_string();
    let numerator = if numerator == "1" { "" } else { &numerator };
    if value.is_integer() {
        format!("{numerator}π")
    } else {
        format!("{numerator}π/{}", value.denom())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculates_circle_and_sector_measurements_as_exact_multiples_of_pi() {
        for (query, expected) in [
            ("3", "半径 3 · 直径 6\n周长 6π · 面积 9π"),
            ("半径 1/2", "半径 1/2 · 直径 1\n周长 π · 面积 π/4"),
            ("直径 10 @ 90°", "弧长 5π/2 · 扇形面积 25π/4"),
            ("diameter 2 @ 360度", "弧长 2π · 扇形面积 π"),
            ("radius 2 @ 0", "弧长 0 · 扇形面积 0"),
            ("半径 １ @ １／２度", "弧长 π/360 · 扇形面积 π/720"),
        ] {
            assert_eq!(calculate_circle(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            ("0", CircleError::NonPositiveRadius),
            ("半径 -2", CircleError::NonPositiveRadius),
            ("3 @ -1", CircleError::AngleOutOfRange),
            ("3 @ 360.0001", CircleError::AngleOutOfRange),
            ("3 @", CircleError::Incomplete),
        ] {
            assert_eq!(calculate_circle(query), Err(error), "{query}");
        }
    }
}
