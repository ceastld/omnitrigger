use std::f64::consts::{E, PI};

pub(super) enum NamedValue {
    Constant(f64),
    Function(fn(f64) -> f64),
    Aggregate(fn(&[f64]) -> f64),
}

pub(super) fn split_name(input: &str) -> (&str, &str) {
    let end = input
        .find(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, 'π' | 'Π' | '_')))
        .unwrap_or(input.len());
    input.split_at(end)
}

pub(super) fn named_value(name: &str) -> Option<NamedValue> {
    let function: fn(f64) -> f64 = match name.to_ascii_lowercase().as_str() {
        "pi" | "π" | "Π" => return Some(NamedValue::Constant(PI)),
        "e" => return Some(NamedValue::Constant(E)),
        "sum" => return Some(NamedValue::Aggregate(|values| values.iter().sum())),
        "avg" | "mean" => return Some(NamedValue::Aggregate(average)),
        "median" => return Some(NamedValue::Aggregate(median)),
        "hypot" => {
            return Some(NamedValue::Aggregate(|values| {
                values.iter().copied().fold(0., f64::hypot)
            }));
        }
        "atan2" => {
            return Some(NamedValue::Aggregate(|values| match values {
                [y, x] => y.atan2(*x),
                _ => f64::NAN,
            }));
        }
        "min" => {
            return Some(NamedValue::Aggregate(|values| {
                values.iter().copied().fold(f64::INFINITY, f64::min)
            }));
        }
        "max" => {
            return Some(NamedValue::Aggregate(|values| {
                values.iter().copied().fold(f64::NEG_INFINITY, f64::max)
            }));
        }
        "abs" => f64::abs,
        "sqrt" => f64::sqrt,
        "cbrt" => f64::cbrt,
        "floor" => f64::floor,
        "ceil" => f64::ceil,
        "round" => f64::round_ties_even,
        "trunc" => f64::trunc,
        "sin" => f64::sin,
        "cos" => f64::cos,
        "tan" => f64::tan,
        "asin" => f64::asin,
        "acos" => f64::acos,
        "atan" => f64::atan,
        "ln" | "log" => f64::ln,
        "log2" => f64::log2,
        "log10" => f64::log10,
        "exp" => f64::exp,
        "radians" => f64::to_radians,
        "degrees" => f64::to_degrees,
        "percent" => |value| value / 100.,
        _ => return None,
    };
    Some(NamedValue::Function(function))
}

fn average(values: &[f64]) -> f64 {
    let count = values.len() as f64;
    let total: f64 = values.iter().sum();
    if total.is_finite() {
        return total / count;
    }

    // 中间和溢出时，缩放参数后再计算均值。
    let scale = values.iter().map(|value| value.abs()).fold(0., f64::max);
    scale * (values.iter().map(|value| value / scale).sum::<f64>() / count)
}

fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_unstable_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        average(&sorted[middle - 1..=middle])
    } else {
        sorted[middle]
    }
}
