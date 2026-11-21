use std::f64::consts::{E, PI};

use super::{calculator::MAX_SAFE_INTEGER, fraction::common_divisor};

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
        "stdev" => {
            return Some(NamedValue::Aggregate(|values| {
                standard_deviation(values, 1)
            }))
        }
        "pstdev" => {
            return Some(NamedValue::Aggregate(|values| {
                standard_deviation(values, 0)
            }))
        }
        "gcd" => {
            return Some(NamedValue::Aggregate(|values| {
                integer_pair(values)
                    .map_or(f64::NAN, |[left, right]| common_divisor(left, right) as f64)
            }));
        }
        "lcm" => return Some(NamedValue::Aggregate(least_common_multiple)),
        "comb" => return Some(NamedValue::Aggregate(combinations)),
        "perm" => {
            return Some(NamedValue::Aggregate(|values| match values {
                [n] => permutations(&[*n, *n]),
                _ => permutations(values),
            }));
        }
        "log" => {
            return Some(NamedValue::Aggregate(|values| match values {
                [value] => value.ln(),
                [value, base] if *base > 0. && *base != 1. => value.log(*base),
                _ => f64::NAN,
            }));
        }
        "pow" => {
            return Some(NamedValue::Aggregate(|values| match values {
                [base, exponent] => base.powf(*exponent),
                _ => f64::NAN,
            }));
        }
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
        "factorial" | "fact" => |n| permutations(&[n, n]),
        "sin" => f64::sin,
        "cos" => f64::cos,
        "tan" => f64::tan,
        "asin" => f64::asin,
        "acos" => f64::acos,
        "atan" => f64::atan,
        "ln" => f64::ln,
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

fn integer_pair(values: &[f64]) -> Option<[u128; 2]> {
    let &[left, right] = values else {
        return None;
    };
    let magnitudes = [left.abs(), right.abs()];
    magnitudes
        .iter()
        .all(|&value| value <= MAX_SAFE_INTEGER as f64 && value.fract() == 0.)
        .then(|| magnitudes.map(|value| value as u128))
}

fn least_common_multiple(values: &[f64]) -> f64 {
    let Some([left, right]) = integer_pair(values) else {
        return f64::NAN;
    };
    if left == 0 || right == 0 {
        return 0.;
    }
    let multiple = left / common_divisor(left, right) * right;
    if multiple > MAX_SAFE_INTEGER {
        f64::INFINITY
    } else {
        multiple as f64
    }
}

fn nonnegative_pair(values: &[f64]) -> Option<[u128; 2]> {
    if values.iter().any(|&value| value < 0.) {
        return None;
    }
    integer_pair(values)
}

fn combinations(values: &[f64]) -> f64 {
    let Some([n, k]) = nonnegative_pair(values) else {
        return f64::NAN;
    };
    if k > n {
        return 0.;
    }
    let k = k.min(n - k);
    let mut result = 1;
    // 每次乘法前结果与因子都在安全整数范围内，中间乘积可用 u128 表示。
    for index in 1..=k {
        result = result * (n - k + index) / index;
        if result > MAX_SAFE_INTEGER {
            return f64::INFINITY;
        }
    }
    result as f64
}

fn permutations(values: &[f64]) -> f64 {
    let Some([n, k]) = nonnegative_pair(values) else {
        return f64::NAN;
    };
    if k > n {
        return 0.;
    }
    let mut result = 1;
    for factor in n - k + 1..=n {
        result *= factor;
        if result > MAX_SAFE_INTEGER {
            return f64::INFINITY;
        }
    }
    result as f64
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

fn standard_deviation(values: &[f64], correction: usize) -> f64 {
    let count = values.len();
    if count <= correction {
        return f64::NAN;
    }
    let (low, high) = values
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), &value| {
            (low.min(value), high.max(value))
        });
    // 先平移再缩放，保留大数之间的微小差异，也避免平方溢出。
    let center = low.midpoint(high);
    let scale = (high - center).max(center - low);
    if scale == 0. {
        return 0.;
    }
    let scaled = |value: f64| (value - center) / scale;
    let mean = values.iter().copied().map(scaled).sum::<f64>() / count as f64;
    let squared_deviations: f64 = values
        .iter()
        .map(|&value| (scaled(value) - mean).powi(2))
        .sum();
    (squared_deviations / (count - correction) as f64).sqrt() * scale
}
