use std::f64::consts::{E, PI};

use num_bigint::{BigInt, Sign};

use super::{calculator::MAX_SAFE_INTEGER, fraction::common_divisor};

const FRACTION_BITS: u32 = f64::MANTISSA_DIGITS - 1;
const HIDDEN_BIT: u64 = 1 << FRACTION_BITS;
const FRACTION_MASK: u64 = HIDDEN_BIT - 1;
const EXPONENT_MASK: u64 = 0x7ff;
const SIGN_BIT: u64 = 1 << 63;

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
        "isclose" => return Some(NamedValue::Aggregate(is_close)),
        "remainder" => return Some(NamedValue::Aggregate(nearest_remainder)),
        "nextafter" => return Some(NamedValue::Aggregate(next_after)),
        "ldexp" => return Some(NamedValue::Aggregate(scale_by_power_of_two)),
        "copysign" => {
            return Some(NamedValue::Aggregate(|values| match *values {
                [value, sign] => value.copysign(sign),
                _ => f64::NAN,
            }));
        }
        "fsum" => return Some(NamedValue::Aggregate(exact_float_sum)),
        "logsumexp" => return Some(NamedValue::Aggregate(log_sum_exp)),
        "prod" | "product" => return Some(NamedValue::Aggregate(|values| values.iter().product())),
        "avg" | "mean" => return Some(NamedValue::Aggregate(average)),
        "wavg" => return Some(NamedValue::Aggregate(weighted_average)),
        "wpvariance" => {
            return Some(NamedValue::Aggregate(|values| {
                weighted_population_deviation(values).powi(2)
            }))
        }
        "wpstdev" => return Some(NamedValue::Aggregate(weighted_population_deviation)),
        "gmean" => return Some(NamedValue::Aggregate(geometric_mean)),
        "hmean" => return Some(NamedValue::Aggregate(harmonic_mean)),
        "sumsq" => {
            return Some(NamedValue::Aggregate(|values| {
                let (scale, squares) = scaled_sum_of_squares(values);
                scale * (scale * squares)
            }))
        }
        "rms" => return Some(NamedValue::Aggregate(root_mean_square)),
        "meansquare" => {
            return Some(NamedValue::Aggregate(|values| {
                root_mean_square(values).powi(2)
            }))
        }
        "corr" => return Some(NamedValue::Aggregate(pearson_correlation)),
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
        "variance" => {
            return Some(NamedValue::Aggregate(|values| {
                standard_deviation(values, 1).powi(2)
            }))
        }
        "pvariance" => {
            return Some(NamedValue::Aggregate(|values| {
                standard_deviation(values, 0).powi(2)
            }))
        }
        "gcd" => return Some(NamedValue::Aggregate(greatest_common_divisor)),
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
        "fma" => {
            return Some(NamedValue::Aggregate(|values| match values {
                [left, right, addend] => left.mul_add(*right, *addend),
                _ => f64::NAN,
            }))
        }
        "root" => return Some(NamedValue::Aggregate(nth_root)),
        "invlerp" => return Some(NamedValue::Aggregate(interpolation_progress)),
        "remap" => return Some(NamedValue::Aggregate(remap_range)),
        "relchange" => return Some(NamedValue::Aggregate(relative_change)),
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
        "atan2" => return Some(NamedValue::Aggregate(atan2)),
        "atan2d" => return Some(NamedValue::Aggregate(|values| atan2(values).to_degrees())),
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
        "ulp" => |value| {
            let magnitude = value.abs();
            if magnitude == f64::MAX {
                magnitude - magnitude.next_down()
            } else {
                magnitude.next_up() - magnitude
            }
        },
        "signbit" => |value| f64::from(u8::from(value.is_sign_negative())),
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
        "sinh" => f64::sinh,
        "cosh" => f64::cosh,
        "tanh" => f64::tanh,
        "asinh" => f64::asinh,
        "acosh" => f64::acosh,
        "atanh" => |value| {
            if value.abs() < 1. {
                value.atanh()
            } else {
                f64::NAN
            }
        },
        "sind" => sin_degrees,
        "cosd" => cos_degrees,
        "tand" => tan_degrees,
        "asin" => f64::asin,
        "acos" => f64::acos,
        "atan" => f64::atan,
        "asind" => |value| value.asin().to_degrees(),
        "acosd" => |value| value.acos().to_degrees(),
        "atand" => |value| value.atan().to_degrees(),
        "ln" => f64::ln,
        "log1p" => f64::ln_1p,
        "log2" => f64::log2,
        "log10" => f64::log10,
        "exp" => f64::exp,
        "expm1" => f64::exp_m1,
        "sigmoid" | "expit" => sigmoid,
        "logit" => logit,
        "radians" => f64::to_radians,
        "degrees" => f64::to_degrees,
        "percent" => |value| value / 100.,
        _ => return None,
    };
    Some(NamedValue::Function(function))
}

fn relative_change(values: &[f64]) -> f64 {
    let &[before, after] = values else {
        return f64::NAN;
    };
    if before == 0. {
        return f64::NAN;
    }
    let difference = after - before;
    if difference.is_finite() {
        difference / before
    } else {
        after / before - 1.
    }
}

fn atan2(values: &[f64]) -> f64 {
    match values {
        [y, x] => y.atan2(*x),
        _ => f64::NAN,
    }
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
    interpolate_between(start, end, progress)
}

pub(super) fn interpolate_between(start: f64, end: f64, progress: f64) -> f64 {
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

fn remap_range(values: &[f64]) -> f64 {
    let &[value, source_start, source_end, target_start, target_end] = values else {
        return f64::NAN;
    };
    let progress = interpolation_progress(&[source_start, source_end, value]);
    linear_interpolation(&[target_start, target_end, progress])
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

fn greatest_common_divisor(values: &[f64]) -> f64 {
    values
        .iter()
        .try_fold(0, |divisor, value| {
            Some(common_divisor(divisor, nonnegative_integer(value.abs())?))
        })
        .map_or(f64::NAN, |divisor| divisor as f64)
}

fn least_common_multiple(values: &[f64]) -> f64 {
    let Some(integers) = values
        .iter()
        .map(|value| nonnegative_integer(value.abs()))
        .collect::<Option<Vec<_>>>()
    else {
        return f64::NAN;
    };
    if integers.contains(&0) {
        return 0.;
    }
    integers
        .into_iter()
        .try_fold(1, |multiple, value| {
            let multiple = multiple / common_divisor(multiple, value) * value;
            (multiple <= MAX_SAFE_INTEGER).then_some(multiple)
        })
        .map_or(f64::INFINITY, |multiple| multiple as f64)
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

fn sigmoid(value: f64) -> f64 {
    if value < 0. {
        let exponential = value.exp();
        exponential / (1. + exponential)
    } else {
        1. / (1. + (-value).exp())
    }
}

fn logit(probability: f64) -> f64 {
    if probability <= 0. || probability >= 1. {
        return f64::NAN;
    }
    if (0.25..=0.75).contains(&probability) {
        // 在 0.5 附近用一次 log1p，避免两个接近的对数相减。
        ((2. * probability - 1.) / (1. - probability)).ln_1p()
    } else {
        probability.ln() - (-probability).ln_1p()
    }
}

fn log_sum_exp(values: &[f64]) -> f64 {
    let Some((largest_index, &largest)) = values
        .iter()
        .enumerate()
        .max_by(|(_, left), (_, right)| left.total_cmp(right))
    else {
        return f64::NAN;
    };
    // 提出最大的指数项，单独保留它对应的 1，避免微小的和在取对数前丢失。
    let rest = values
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != largest_index)
        .map(|(_, value)| (value - largest).exp())
        .sum::<f64>();
    largest + rest.ln_1p()
}

fn exact_float_sum(values: &[f64]) -> f64 {
    // 每个有限的二进制浮点数都是 2^-1074 的整数倍。
    let mut total = BigInt::default();
    for &value in values {
        let bits = value.to_bits();
        let exponent = (bits >> FRACTION_BITS) & EXPONENT_MASK;
        let fraction = bits & FRACTION_MASK;
        let mantissa = if exponent == 0 {
            fraction
        } else {
            fraction | HIDDEN_BIT
        };
        let units = BigInt::from(mantissa) << exponent.saturating_sub(1);
        if bits & SIGN_BIT == 0 {
            total += units;
        } else {
            total -= units;
        }
    }

    let sign = if total.sign() == Sign::Minus {
        SIGN_BIT
    } else {
        0
    };
    let magnitude = total.magnitude();
    let width = magnitude.bits();
    let shift = width.saturating_sub(u64::from(f64::MANTISSA_DIGITS));
    let mut mantissa = (magnitude >> shift).iter_u64_digits().next().unwrap_or(0);
    if width <= u64::from(FRACTION_BITS) {
        return f64::from_bits(sign | mantissa);
    }

    // 在压缩到 53 位时只舍入一次，中点取偶数。
    if shift != 0
        && magnitude.bit(shift - 1)
        && (mantissa & 1 != 0
            || magnitude
                .trailing_zeros()
                .is_some_and(|zeros| zeros < shift - 1))
    {
        mantissa += 1;
    }
    let exponent = shift + 1 + (mantissa >> f64::MANTISSA_DIGITS);
    if exponent >= EXPONENT_MASK {
        return f64::from_bits(sign | (EXPONENT_MASK << FRACTION_BITS));
    }
    f64::from_bits(sign | (exponent << FRACTION_BITS) | (mantissa & FRACTION_MASK))
}

fn scale_by_power_of_two(values: &[f64]) -> f64 {
    let [value, power] = *values else {
        return f64::NAN;
    };
    if power.fract() != 0. || power.abs() > MAX_SAFE_INTEGER as f64 {
        return f64::NAN;
    }
    if value == 0. || !value.is_finite() {
        return value;
    }
    let bits = value.to_bits();
    let fraction = bits & FRACTION_MASK;
    let biased = ((bits >> FRACTION_BITS) & EXPONENT_MASK) as i64;
    // 先把次正规数的有效位移到与正规数相同的位置。
    let (significand, biased) = if biased == 0 {
        let shift = fraction.leading_zeros() - (u64::BITS - f64::MANTISSA_DIGITS);
        (fraction << shift, 1 - i64::from(shift))
    } else {
        (fraction | HIDDEN_BIT, biased)
    };
    let scaled_exponent = biased + power as i64;
    let sign = bits & SIGN_BIT;
    if scaled_exponent >= EXPONENT_MASK as i64 {
        return f64::from_bits(sign | (EXPONENT_MASK << FRACTION_BITS));
    }
    if scaled_exponent > 0 {
        return f64::from_bits(
            sign | ((scaled_exponent as u64) << FRACTION_BITS) | (significand & FRACTION_MASK),
        );
    }
    let shift = 1 - scaled_exponent;
    if shift > i64::from(f64::MANTISSA_DIGITS) {
        return f64::from_bits(sign);
    }
    // 结果进入次正规范围时只舍入一次，中点取偶数。
    let half = 1_u64 << (shift - 1);
    let discarded = significand & ((half << 1) - 1);
    let remaining = significand >> shift;
    let round_up = discarded > half || (discarded == half && !remaining.is_multiple_of(2));
    f64::from_bits(sign | (remaining + u64::from(round_up)))
}

fn next_after(values: &[f64]) -> f64 {
    let (start, target, steps) = match *values {
        [start, target] => (start, target, 1),
        [start, target, steps]
            if (0.0..=MAX_SAFE_INTEGER as f64).contains(&steps) && steps.fract() == 0.0 =>
        {
            (start, target, steps as i64)
        }
        _ => return f64::NAN,
    };
    if steps == 0 {
        return start;
    }
    if start == target {
        return target;
    }

    const SIGN_BIT: u64 = 1 << 63;
    // 位模式的绝对值按大小排序，两个有符号零共用一个位置。
    let rank = |value: f64| {
        let magnitude = (value.to_bits() & !SIGN_BIT) as i64;
        if value.is_sign_negative() {
            -magnitude
        } else {
            magnitude
        }
    };
    let origin = rank(start);
    let destination = rank(target);
    if steps as u64 > origin.abs_diff(destination) {
        return target;
    }
    let advanced = origin + if origin < destination { steps } else { -steps };
    let negative = advanced < 0 || (advanced == 0 && start.is_sign_negative());
    f64::from_bits(advanced.unsigned_abs() | (u64::from(negative) << 63))
}

fn nearest_remainder(values: &[f64]) -> f64 {
    let [dividend, divisor] = *values else {
        return f64::NAN;
    };
    let remainder = dividend % divisor;
    if !remainder.is_finite() || divisor.is_infinite() {
        return remainder;
    }

    let magnitude = remainder.abs();
    let divisor = divisor.abs();
    // 与互补余数比较，避免极小的除数在减半时舍入。
    let complement = divisor - magnitude;
    if magnitude < complement {
        return remainder;
    }
    if magnitude == complement {
        let period = divisor + divisor;
        let offset = if period.is_finite() {
            dividend.abs() % period
        } else {
            dividend.abs()
        };
        if offset < divisor {
            return remainder;
        }
    }
    -complement.copysign(dividend)
}

fn is_close(values: &[f64]) -> f64 {
    let (left, right, relative, absolute) = match *values {
        [left, right] => (left, right, 1e-9, 0.),
        [left, right, relative] => (left, right, relative, 0.),
        [left, right, relative, absolute] => (left, right, relative, absolute),
        _ => return f64::NAN,
    };
    if !(0.0..1.0).contains(&relative) || absolute < 0. {
        return f64::NAN;
    }
    let limit = absolute.max(relative * left.abs().max(right.abs()));
    f64::from(u8::from((left - right).abs() <= limit))
}

pub(super) fn average(values: &[f64]) -> f64 {
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

fn weighted_population_deviation(values: &[f64]) -> f64 {
    let pairs = values.chunks_exact(2);
    if !pairs.remainder().is_empty() || pairs.clone().any(|pair| pair[1] < 0.) {
        return f64::NAN;
    }
    let active = pairs.filter(|pair| pair[1] > 0.);
    let max_weight = active.clone().map(|pair| pair[1]).fold(0., f64::max);
    if max_weight == 0. {
        return f64::NAN;
    }
    let (center, scale) = centered_scale(active.clone().map(|pair| pair[0]));
    if scale == 0. {
        return 0.;
    }
    let mut weighted_total = 0.;
    let mut total_weight = 0.;
    let scaled = |value: f64| (value - center) / scale;
    for pair in active.clone() {
        let weight = pair[1] / max_weight;
        weighted_total += weight * scaled(pair[0]);
        total_weight += weight;
    }
    let mean = weighted_total / total_weight;
    let weight_root = max_weight.sqrt();
    // 先对每个权重开方，极小的权重比仍能参与均方根。
    let norm = active.fold(0.0_f64, |norm, pair| {
        let deviation = (scaled(pair[0]) - mean) * (pair[1].sqrt() / weight_root);
        norm.hypot(deviation)
    });
    (norm / total_weight.sqrt()).min(1.) * scale
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

pub(super) fn median(values: &[f64]) -> f64 {
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

fn root_mean_square(values: &[f64]) -> f64 {
    let (scale, squares) = scaled_sum_of_squares(values);
    scale * (squares / values.len().max(1) as f64).sqrt()
}

fn scaled_sum_of_squares(values: &[f64]) -> (f64, f64) {
    let scale = values.iter().map(|value| value.abs()).fold(0., f64::max);
    if scale == 0. {
        return (0., 0.);
    }
    let squares = values.iter().map(|value| (value / scale).powi(2)).sum();
    (scale, squares)
}

fn centered_scale(values: impl Iterator<Item = f64>) -> (f64, f64) {
    let (low, high) = values.fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), value| {
        (low.min(value), high.max(value))
    });
    // 先平移再缩放，保留大数之间的微小差异，也避免平方溢出。
    let center = low.midpoint(high);
    (center, (high - center).max(center - low))
}

fn pearson_correlation(values: &[f64]) -> f64 {
    let pairs = values.chunks_exact(2);
    let count = pairs.len();
    if count < 2 || !pairs.remainder().is_empty() {
        return f64::NAN;
    }
    let (center_x, scale_x) = centered_scale(pairs.clone().map(|pair| pair[0]));
    let (center_y, scale_y) = centered_scale(pairs.clone().map(|pair| pair[1]));
    if scale_x == 0. || scale_y == 0. {
        return f64::NAN;
    }
    let scaled = pairs.map(|pair| {
        (
            (pair[0] - center_x) / scale_x,
            (pair[1] - center_y) / scale_y,
        )
    });
    let (sum_x, sum_y) = scaled
        .clone()
        .fold((0., 0.), |(sum_x, sum_y), (x, y)| (sum_x + x, sum_y + y));
    let (mean_x, mean_y) = (sum_x / count as f64, sum_y / count as f64);
    let (mut squared_x, mut squared_y, mut products) = (0., 0., 0.);
    for (x, y) in scaled {
        let x = x - mean_x;
        let y = y - mean_y;
        squared_x += x * x;
        squared_y += y * y;
        products += x * y;
    }
    (products / (squared_x * squared_y).sqrt()).clamp(-1., 1.)
}

fn standard_deviation(values: &[f64], correction: usize) -> f64 {
    let count = values.len();
    if count <= correction {
        return f64::NAN;
    }
    let (center, scale) = centered_scale(values.iter().copied());
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
