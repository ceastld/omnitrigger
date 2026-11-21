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
        "wavg" => return Some(NamedValue::Aggregate(weighted_average)),
        "gmean" => return Some(NamedValue::Aggregate(geometric_mean)),
        "hmean" => return Some(NamedValue::Aggregate(harmonic_mean)),
        "median" => return Some(NamedValue::Aggregate(median)),
        "quantile" => return Some(NamedValue::Aggregate(|values| quantile(values, 1.))),
        "percentile" => return Some(NamedValue::Aggregate(|values| quantile(values, 100.))),
        "iqr" => return Some(NamedValue::Aggregate(interquartile_range)),
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
        "round" => return Some(NamedValue::Aggregate(round_to_places)),
        "bitand" => {
            return Some(NamedValue::Aggregate(|values| {
                bitwise(values, |a, b| (a & b) as f64)
            }));
        }
        "bitor" => {
            return Some(NamedValue::Aggregate(|values| {
                bitwise(values, |a, b| (a | b) as f64)
            }));
        }
        "bitxor" => {
            return Some(NamedValue::Aggregate(|values| {
                bitwise(values, |a, b| (a ^ b) as f64)
            }));
        }
        "shl" => return Some(NamedValue::Aggregate(|values| bitwise(values, shift_left))),
        "shr" => return Some(NamedValue::Aggregate(|values| bitwise(values, shift_right))),
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
        "lerp" => return Some(NamedValue::Aggregate(linear_interpolation)),
        "root" => return Some(NamedValue::Aggregate(nth_root)),
        "invlerp" => return Some(NamedValue::Aggregate(interpolation_progress)),
        "clamp" => {
            return Some(NamedValue::Aggregate(|values| match values {
                [value, min, max] if min <= max => value.clamp(*min, *max),
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
        "trunc" => f64::trunc,
        "factorial" | "fact" => |n| permutations(&[n, n]),
        "popcount" | "bit_count" => {
            |n| nonnegative_integer(n).map_or(f64::NAN, |value| f64::from(value.count_ones()))
        }
        "sin" => f64::sin,
        "cos" => f64::cos,
        "tan" => f64::tan,
        "sind" => sin_degrees,
        "cosd" => cos_degrees,
        "tand" => tan_degrees,
        "asin" => f64::asin,
        "acos" => f64::acos,
        "atan" => f64::atan,
        "ln" => f64::ln,
        "log1p" => f64::ln_1p,
        "log2" => f64::log2,
        "log10" => f64::log10,
        "exp" => f64::exp,
        "expm1" => f64::exp_m1,
        "radians" => f64::to_radians,
        "degrees" => f64::to_degrees,
        "percent" => |value| value / 100.,
        _ => return None,
    };
    Some(NamedValue::Function(function))
}

fn sin_degrees(angle: f64) -> f64 {
    match angle % 360. {
        0. | 180. | -180. => 0.,
        90. | -270. => 1.,
        -90. | 270. => -1.,
        reduced => reduced.to_radians().sin(),
    }
}

fn cos_degrees(angle: f64) -> f64 {
    match angle % 360. {
        0. => 1.,
        180. | -180. => -1.,
        90. | -90. | 270. | -270. => 0.,
        reduced => reduced.to_radians().cos(),
    }
}

fn tan_degrees(angle: f64) -> f64 {
    match angle % 180. {
        90. | -90. => f64::NAN,
        reduced => reduced.to_radians().tan(),
    }
}

fn nth_root(values: &[f64]) -> f64 {
    let &[value, degree] = values else {
        return f64::NAN;
    };
    let Some(degree) = nonnegative_integer(degree).filter(|&degree| degree != 0) else {
        return f64::NAN;
    };
    if value < 0. && degree.is_multiple_of(2) {
        return f64::NAN;
    }
    match degree {
        1 => value,
        2 => value.sqrt(),
        3 => value.cbrt(),
        _ => value.abs().powf(1. / degree as f64).copysign(value),
    }
}

fn linear_interpolation(values: &[f64]) -> f64 {
    let &[start, end, progress] = values else {
        return f64::NAN;
    };
    if !(0. ..=1.).contains(&progress) {
        return f64::NAN;
    }
    if progress == 0. {
        return start;
    }
    if progress == 1. {
        return end;
    }
    if start.is_sign_negative() != end.is_sign_negative() {
        // 异号端点的差可能溢出，直接计算两端的加权和。
        start * (1. - progress) + end * progress
    } else {
        progress
            .mul_add(end - start, start)
            .clamp(start.min(end), start.max(end))
    }
}

fn interpolation_progress(values: &[f64]) -> f64 {
    let &[start, end, value] = values else {
        return f64::NAN;
    };
    if start == end || !(start.min(end)..=start.max(end)).contains(&value) {
        return f64::NAN;
    }
    let span = end - start;
    if span.is_infinite() {
        (value * 0.5 - start * 0.5) / (end * 0.5 - start * 0.5)
    } else {
        (value - start) / span
    }
}

fn round_to_places(values: &[f64]) -> f64 {
    let (value, digits) = match *values {
        [value] | [value, 0.] => return value.round_ties_even(),
        [value, digits] if digits.fract() == 0. => (value, digits),
        _ => return f64::NAN,
    };
    if digits > 323. {
        return value;
    }
    if digits < -308. {
        return 0.0_f64.copysign(value);
    }
    if digits < 0. {
        return round_left_of_decimal(value, -digits as i32);
    }
    let precision = digits as usize;
    format!("{value:.precision$}")
        .parse()
        .expect("格式化的数字可解析")
}

fn round_left_of_decimal(value: f64, places: i32) -> f64 {
    let scientific = format!("{value:e}");
    let (_, exponent) = scientific.split_once('e').expect("科学计数法含有指数");
    let exponent: i32 = exponent.parse().expect("科学计数法指数为整数");
    if exponent >= places {
        let precision = (exponent - places) as usize;
        return format!("{value:.precision$e}")
            .parse()
            .expect("格式化的数字可解析");
    }
    if exponent < places - 1 {
        return 0.0_f64.copysign(value);
    }

    let midpoint = format!("5{}", "0".repeat((places - 1) as usize));
    let cutoff: f64 = midpoint.parse().expect("中点可表示为有限浮点数");
    let magnitude = value.abs();
    // 大的十进制中点可能在解析时舍入。浮点值相同时比较其精确整数。
    let above = magnitude > cutoff
        || (magnitude == cutoff && places >= 23 && format!("{magnitude:.0}") > midpoint);
    let rounded = if above {
        format!("1e{places}")
            .parse::<f64>()
            .expect("十进制幂可解析")
    } else {
        0.
    };
    rounded.copysign(value)
}

fn integer_pair(values: &[f64]) -> Option<[u128; 2]> {
    let &[left, right] = values else {
        return None;
    };
    Some([
        nonnegative_integer(left.abs())?,
        nonnegative_integer(right.abs())?,
    ])
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
    let &[left, right] = values else {
        return None;
    };
    Some([nonnegative_integer(left)?, nonnegative_integer(right)?])
}

fn nonnegative_integer(value: f64) -> Option<u128> {
    (value >= 0. && value <= MAX_SAFE_INTEGER as f64 && value.fract() == 0.)
        .then_some(value as u128)
}

fn bitwise(values: &[f64], operation: fn(u128, u128) -> f64) -> f64 {
    nonnegative_pair(values).map_or(f64::NAN, |[left, right]| operation(left, right))
}

fn shift_left(value: u128, bits: u128) -> f64 {
    if value == 0 {
        return 0.;
    }
    if bits >= u128::from(MAX_SAFE_INTEGER.count_ones()) || value > MAX_SAFE_INTEGER >> bits {
        return f64::INFINITY;
    }
    (value << bits) as f64
}

fn shift_right(value: u128, bits: u128) -> f64 {
    if bits >= u128::from(MAX_SAFE_INTEGER.count_ones()) {
        0.
    } else {
        (value >> bits) as f64
    }
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

fn weighted_average(values: &[f64]) -> f64 {
    let pairs = values.chunks_exact(2);
    if !pairs.remainder().is_empty() || pairs.clone().any(|pair| pair[1] < 0.) {
        return f64::NAN;
    }
    let active = pairs.filter(|pair| pair[1] > 0.);
    let max_weight = active.clone().map(|pair| pair[1]).fold(0., f64::max);
    if max_weight == 0. {
        return f64::NAN;
    }
    let total_weight: f64 = active.clone().map(|pair| pair[1] / max_weight).sum();
    let mut minimum = f64::INFINITY;
    let mut maximum = f64::NEG_INFINITY;
    let mut mean = 0.;
    for pair in active {
        let value = pair[0];
        let weight = pair[1];
        minimum = minimum.min(value);
        maximum = maximum.max(value);
        let scaled_weight = weight / max_weight;
        let fraction = scaled_weight / total_weight;
        mean += if fraction != 0. {
            value * fraction
        } else if scaled_weight != 0. {
            value * scaled_weight / total_weight
        } else {
            // 权重比可能下溢，但先缩放数值仍可保留这个加权项。
            value / max_weight * weight / total_weight
        };
    }
    mean.clamp(minimum, maximum)
}

fn positive_bounds(values: &[f64]) -> Option<(f64, f64)> {
    let (&first, rest) = values.split_first()?;
    let (minimum, maximum) = rest
        .iter()
        .fold((first, first), |(minimum, maximum), &value| {
            (minimum.min(value), maximum.max(value))
        });
    (minimum > 0.).then_some((minimum, maximum))
}

fn geometric_mean(values: &[f64]) -> f64 {
    let Some((minimum, maximum)) = positive_bounds(values) else {
        return f64::NAN;
    };
    let logarithm = values.iter().map(|value| value.ln()).sum::<f64>() / values.len() as f64;
    logarithm.exp().clamp(minimum, maximum)
}

fn harmonic_mean(values: &[f64]) -> f64 {
    let Some((minimum, maximum)) = positive_bounds(values) else {
        return f64::NAN;
    };
    let scaled = values.iter().map(|value| minimum / value).sum::<f64>() / values.len() as f64;
    (minimum / scaled).clamp(minimum, maximum)
}

fn median(values: &[f64]) -> f64 {
    let sorted = sorted_samples(values);
    let middle = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        average(&sorted[middle - 1..=middle])
    } else {
        sorted[middle]
    }
}

fn sorted_samples(values: &[f64]) -> Vec<f64> {
    let mut sorted = values.to_vec();
    sorted.sort_unstable_by(f64::total_cmp);
    sorted
}

fn quantile(arguments: &[f64], maximum: f64) -> f64 {
    let [percent, samples @ ..] = arguments else {
        return f64::NAN;
    };
    if samples.is_empty() || !(0. ..=maximum).contains(percent) {
        return f64::NAN;
    }
    interpolated_quantile(&sorted_samples(samples), percent / maximum)
}

fn interquartile_range(samples: &[f64]) -> f64 {
    let sorted = sorted_samples(samples);
    interpolated_quantile(&sorted, 0.75) - interpolated_quantile(&sorted, 0.25)
}

fn interpolated_quantile(sorted: &[f64], probability: f64) -> f64 {
    let last = sorted.len() - 1;
    let rank = last as f64 * probability;
    let index = rank as usize;
    let lower = sorted[index];
    if index == last {
        return lower;
    }
    let upper = sorted[index + 1];
    let weight = rank.fract();
    if lower.is_sign_negative() == upper.is_sign_negative() {
        lower + (upper - lower) * weight
    } else {
        // 异号时分开加权，避免相减超出浮点数范围。
        lower * (1. - weight) + upper * weight
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
