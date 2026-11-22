use num_rational::BigRational;

use super::{
    fraction::{calculate_fraction, FractionError},
    operators::split_top_level_commas,
};

pub(super) type Point2 = [BigRational; 2];

pub(super) enum PointError {
    Arguments,
    Evaluation(FractionError),
}

pub(super) fn parse_point(input: &str) -> Result<Point2, PointError> {
    let mut axes = split_top_level_commas(input);
    let (Some(x), Some(y), None) = (axes.next(), axes.next(), axes.next()) else {
        return Err(PointError::Arguments);
    };
    if x.trim().is_empty() || y.trim().is_empty() {
        return Err(PointError::Arguments);
    }
    Ok([
        calculate_fraction(x).map_err(PointError::Evaluation)?,
        calculate_fraction(y).map_err(PointError::Evaluation)?,
    ])
}
