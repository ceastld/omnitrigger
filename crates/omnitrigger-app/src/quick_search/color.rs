use std::f64::consts::TAU;

use super::calculator::{format_result, normalize};
use super::conversion::split_conversion;

// Oklab 的线性 sRGB 转换矩阵由 Björn Ottosson 发布。
const LINEAR_SRGB_TO_LMS: [[f64; 3]; 3] = [
    [0.4122214708, 0.5363325363, 0.0514459929],
    [0.2119034982, 0.6806995451, 0.1073969566],
    [0.0883024619, 0.2817188376, 0.6299787005],
];
const LMS_ROOT_TO_OKLAB: [[f64; 3]; 3] = [
    [0.2104542553, 0.7936177850, -0.0040720468],
    [1.9779984951, -2.4285922050, 0.4505937099],
    [0.0259040371, 0.7827717662, -0.8086757660],
];
const OKLAB_TO_LMS_ROOT: [[f64; 3]; 3] = [
    [1., 0.3963377774, 0.2158037573],
    [1., -0.1055613458, -0.0638541728],
    [1., -0.0894841775, -1.2914855480],
];
const LMS_TO_LINEAR_SRGB: [[f64; 3]; 3] = [
    [4.0767416621, -3.3077115913, 0.2309699292],
    [-1.2684380046, 2.6097574011, -0.3413193965],
    [-0.0041960863, -0.7034186147, 1.7076147010],
];
const LINEAR_GAMUT_TOLERANCE: f64 = 0.00001;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Color {
    pub(super) swatch: u32,
    pub(super) output: String,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct ColorComparison {
    pub(super) output: String,
    pub(super) swatches: [u32; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ColorError {
    Incomplete,
    IncompleteContrast,
    IncompleteDifference,
    Invalid,
    InvalidFormat,
    OutsideSrgbGamut,
    InvalidMix,
    InvalidPalette,
    MixColorSpace,
    TransparentContrast,
    TransparentDifference,
    TransparentTextBackground,
}

impl ColorError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入 HEX、RGB、HSL、HWB、OKLab、OKLCH 或 CSS 颜色名",
            Self::IncompleteContrast => "输入两个颜色，例如 对比度: #fff vs #000",
            Self::IncompleteDifference => "输入两个颜色，例如 颜色: 色差 red vs lime",
            Self::Invalid => "检查颜色名、HEX 位数或颜色通道范围",
            Self::InvalidFormat => "目标颜色格式可用 HEX、RGB、HSL、HWB、OKLab 或 OKLCH",
            Self::OutsideSrgbGamut => "这个颜色超出 sRGB 色域",
            Self::InvalidMix => "输入两个颜色，权重范围为 0% 到 100%，总权重需大于 0%",
            Self::InvalidPalette => {
                "填写起始颜色、结束颜色和 2 至 32 个色值，例如 #000 到 #fff @ 5"
            }
            Self::MixColorSpace => "颜色混合支持 in srgb、in srgb-linear 或 in oklab",
            Self::TransparentContrast => "计算对比度需要两个不透明颜色",
            Self::TransparentDifference => "计算色差需要两个不透明颜色",
            Self::TransparentTextBackground => "需要不透明底色，也可先叠加到实际背景上",
        }
    }
}

enum ColorFormat {
    Hex,
    Rgb,
    Hsl,
    Hwb,
    Oklab,
    Oklch,
}

#[derive(Clone, Copy)]
pub(super) enum ColorMixSpace {
    Srgb,
    LinearSrgb,
    Oklab,
}

impl ColorMixSpace {
    fn interpolate(
        self,
        left: [u8; 3],
        right: [u8; 3],
        first: f64,
        second: f64,
        total: f64,
    ) -> Result<[u8; 3], ColorError> {
        let blend = |left: [f64; 3], right: [f64; 3]| {
            std::array::from_fn(|index| (left[index] * first + right[index] * second) / total)
        };
        match self {
            Self::Srgb => Ok(blend(left.map(f64::from), right.map(f64::from))
                .map(|channel| channel.round() as u8)),
            Self::LinearSrgb => Ok(blend(
                left.map(srgb_channel_to_linear),
                right.map(srgb_channel_to_linear),
            )
            .map(linear_to_srgb_channel)),
            Self::Oklab => srgb_from_oklab(blend(oklab_from_srgb(left), oklab_from_srgb(right))),
        }
    }
}

#[derive(Clone, Copy)]
struct Channels {
    rgb: [u8; 3],
    alpha: Option<u8>,
}

impl Channels {
    fn parse(input: &str) -> Result<Self, ColorError> {
        if let Some(digits) = input.strip_prefix('#') {
            Self::hex(digits)
        } else if input.is_empty() {
            Err(ColorError::Incomplete)
        } else if input.bytes().all(|byte| byte.is_ascii_alphabetic()) {
            Self::named(input)
        } else {
            Self::function(input)
        }
    }

    fn named(input: &str) -> Result<Self, ColorError> {
        let name = input.to_ascii_lowercase();
        let color = if name == "rebeccapurple" {
            svgtypes::Color::new_rgb(102, 51, 153)
        } else {
            name.parse::<svgtypes::Color>()
                .map_err(|_| ColorError::Invalid)?
        };
        Ok(Self {
            rgb: [color.red, color.green, color.blue],
            alpha: (color.alpha != 255).then_some(color.alpha),
        })
    }

    fn swatch(&self) -> u32 {
        let [r, g, b] = self.rgb;
        u32::from_be_bytes([r, g, b, self.alpha.unwrap_or(255)])
    }

    fn format_hex(&self) -> String {
        let [r, g, b] = self.rgb;
        if let Some(alpha) = self.alpha {
            format!("#{r:02X}{g:02X}{b:02X}{alpha:02X}")
        } else {
            format!("#{r:02X}{g:02X}{b:02X}")
        }
    }

    fn over(self, backdrop: Self) -> Self {
        let alpha = u32::from(self.alpha.unwrap_or(255));
        // Porter-Duff source-over，两层权重都以 255² 为分母。
        let front_weight = alpha * 255;
        let back_weight = u32::from(backdrop.alpha.unwrap_or(255)) * (255 - alpha);
        let total = front_weight + back_weight;
        let rgb = if total == 0 {
            [0; 3]
        } else {
            std::array::from_fn(|index| {
                let front = u32::from(self.rgb[index]) * front_weight;
                let back = u32::from(backdrop.rgb[index]) * back_weight;
                ((front + back + total / 2) / total) as u8
            })
        };
        let alpha = ((total + 127) / 255) as u8;
        Self {
            rgb,
            alpha: (alpha != 255).then_some(alpha),
        }
    }

    fn mix(
        self,
        other: Self,
        first: f64,
        second: f64,
        space: ColorMixSpace,
    ) -> Result<Self, ColorError> {
        let total = first + second;
        if total == 0. {
            return Err(ColorError::InvalidMix);
        }

        // 用预乘 Alpha 的通道插值。总权重不足 100% 时保留剩余透明度。
        let first = first * f64::from(self.alpha.unwrap_or(255));
        let second = second * f64::from(other.alpha.unwrap_or(255));
        let combined = first + second;
        let rgb = if combined == 0. {
            [0; 3]
        } else {
            space.interpolate(self.rgb, other.rgb, first, second, combined)?
        };
        let alpha = (combined / total.max(100.)).round() as u8;
        Ok(Self {
            rgb,
            alpha: (alpha != 255).then_some(alpha),
        })
    }

    fn hex(digits: &str) -> Result<Self, ColorError> {
        let short = matches!(digits.len(), 3 | 4);
        if digits.is_empty() {
            return Err(ColorError::Incomplete);
        }
        if !short && !matches!(digits.len(), 6 | 8) {
            return Err(ColorError::Invalid);
        }
        let value = u32::from_str_radix(digits, 16).map_err(|_| ColorError::Invalid)?;
        if !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ColorError::Invalid);
        }
        let width = if short { 4 } else { 8 };
        let mask = if short { 0xf } else { 0xff };
        let alpha = matches!(digits.len(), 4 | 8);
        let component = |index: usize| {
            let shift = width * (usize::from(alpha) + 2 - index);
            let value = ((value >> shift) & mask) as u8;
            if short {
                value * 17
            } else {
                value
            }
        };
        Ok(Self {
            rgb: [component(0), component(1), component(2)],
            alpha: alpha.then(|| component(3)),
        })
    }

    fn function(input: &str) -> Result<Self, ColorError> {
        let (name, terms) = input.split_once('(').ok_or(ColorError::Invalid)?;
        let format = match name.to_ascii_lowercase().as_str() {
            "rgb" | "rgba" => ColorFormat::Rgb,
            "hsl" | "hsla" => ColorFormat::Hsl,
            "hwb" => ColorFormat::Hwb,
            "oklab" => ColorFormat::Oklab,
            "oklch" => ColorFormat::Oklch,
            _ => return Err(ColorError::Invalid),
        };
        let terms = terms.strip_suffix(')').ok_or(ColorError::Incomplete)?;
        let (values, alpha) = if terms.contains(',') {
            if matches!(
                format,
                ColorFormat::Hwb | ColorFormat::Oklab | ColorFormat::Oklch
            ) {
                return Err(ColorError::Invalid);
            }
            let mut parts: Vec<_> = terms.split(',').collect();
            let alpha = (parts.len() == 4).then(|| parts.pop().expect("有四项"));
            (parts, alpha)
        } else {
            let (values, alpha) = terms
                .split_once('/')
                .map_or((terms, None), |(values, alpha)| (values, Some(alpha)));
            (values.split_whitespace().collect(), alpha)
        };
        let &[first, second, third] = values.as_slice() else {
            return Err(ColorError::Invalid);
        };
        let rgb = match format {
            ColorFormat::Hsl => rgb_from_hsl(first, second, third)?,
            ColorFormat::Hwb => rgb_from_hwb(first, second, third)?,
            ColorFormat::Oklab => rgb_from_oklab(first, second, third)?,
            ColorFormat::Oklch => rgb_from_oklch(first, second, third)?,
            ColorFormat::Rgb => [
                component(first, 255.)?,
                component(second, 255.)?,
                component(third, 255.)?,
            ],
            ColorFormat::Hex => unreachable!("HEX 不使用函数形式"),
        };
        Ok(Self {
            rgb,
            alpha: alpha
                .map(|value| {
                    if matches!(format, ColorFormat::Oklab)
                        && value.trim().eq_ignore_ascii_case("none")
                    {
                        Ok(0)
                    } else {
                        component(value, 1.)
                    }
                })
                .transpose()?,
        })
    }
}

fn bounded_number(text: &str, max: f64) -> Result<f64, ColorError> {
    let value = text.parse::<f64>().map_err(|_| ColorError::Invalid)?;
    if !(0.0..=max).contains(&value) {
        return Err(ColorError::Invalid);
    }
    Ok(value)
}

fn component(text: &str, max: f64) -> Result<u8, ColorError> {
    let text = text.trim();
    let (text, max) = text
        .strip_suffix('%')
        .map_or((text, max), |number| (number, 100.));
    let value = bounded_number(text, max)?;
    Ok(if max == 255. {
        value
    } else {
        value * 255. / max
    }
    .round() as u8)
}

fn hue_sector(input: &str) -> Result<f64, ColorError> {
    let lower = input.trim().to_ascii_lowercase();
    let (number, period) = [("deg", 360.), ("grad", 400.), ("rad", TAU), ("turn", 1.)]
        .into_iter()
        .find_map(|(unit, period)| lower.strip_suffix(unit).map(|number| (number, period)))
        .unwrap_or((&lower, 360.));
    let angle = number.parse::<f64>().map_err(|_| ColorError::Invalid)?;
    if !angle.is_finite() {
        return Err(ColorError::Invalid);
    }
    Ok((angle.rem_euclid(period) / period * 6.).rem_euclid(6.))
}

fn rgb_from_hsl(hue: &str, saturation: &str, lightness: &str) -> Result<[u8; 3], ColorError> {
    let percentage = |text: &str| {
        let number = text.trim().strip_suffix('%').ok_or(ColorError::Invalid)?;
        bounded_number(number, 100.).map(|value| value / 100.)
    };
    let sector = hue_sector(hue)?;
    let saturation = percentage(saturation)?;
    let lightness = percentage(lightness)?;
    let chroma = (1. - (2. * lightness - 1.).abs()) * saturation;
    let floor = lightness - chroma / 2.;
    Ok(hue_channels(sector, chroma).map(|value| ((value + floor) * 255.).round() as u8))
}

fn rgb_from_hwb(hue: &str, white: &str, black: &str) -> Result<[u8; 3], ColorError> {
    let sector = hue_sector(hue)?;
    let percentage = |text: &str| {
        let text = text.trim();
        bounded_number(text.strip_suffix('%').unwrap_or(text), 100.).map(|value| value / 100.)
    };
    let white = percentage(white)?;
    let black = percentage(black)?;
    let total = white + black;
    if total >= 1. {
        return Ok([(white / total * 255.).round() as u8; 3]);
    }
    Ok(hue_channels(sector, 1. - total).map(|value| ((value + white) * 255.).round() as u8))
}

fn hue_channels(sector: f64, chroma: f64) -> [f64; 3] {
    let middle = chroma * (1. - (sector % 2. - 1.).abs());
    match sector as u8 {
        0 => [chroma, middle, 0.],
        1 => [middle, chroma, 0.],
        2 => [0., chroma, middle],
        3 => [0., middle, chroma],
        4 => [middle, 0., chroma],
        _ => [chroma, 0., middle],
    }
}

fn apply_color_matrix(matrix: [[f64; 3]; 3], components: [f64; 3]) -> [f64; 3] {
    matrix.map(|row| {
        row.into_iter()
            .zip(components)
            .map(|(weight, value)| weight * value)
            .sum()
    })
}

fn oklab_lightness(input: &str) -> Result<f64, ColorError> {
    if let Some(percent) = input.strip_suffix('%') {
        Ok(bounded_number(percent, 100.)? / 100.)
    } else {
        bounded_number(input, 1.)
    }
}

fn oklab_axis(input: &str) -> Result<f64, ColorError> {
    if input.eq_ignore_ascii_case("none") {
        return Ok(0.);
    }
    let (number, factor) = input
        .strip_suffix('%')
        .map_or((input, 1.), |value| (value, 0.004));
    let value = number.parse::<f64>().map_err(|_| ColorError::Invalid)?;
    value
        .is_finite()
        .then_some(value * factor)
        .ok_or(ColorError::Invalid)
}

fn rgb_from_oklab(lightness: &str, a: &str, b: &str) -> Result<[u8; 3], ColorError> {
    let lightness = if lightness.eq_ignore_ascii_case("none") {
        0.
    } else {
        oklab_lightness(lightness)?
    };
    srgb_from_oklab([lightness, oklab_axis(a)?, oklab_axis(b)?])
}

fn rgb_from_oklch(lightness: &str, chroma: &str, hue: &str) -> Result<[u8; 3], ColorError> {
    let lightness = oklab_lightness(lightness)?;
    let chroma = if let Some(percent) = chroma.strip_suffix('%') {
        bounded_number(percent, f64::MAX)? * 0.004
    } else {
        bounded_number(chroma, f64::MAX)?
    };
    let angle = hue_sector(hue)? / 6. * TAU;
    let (sin, cos) = angle.sin_cos();
    srgb_from_oklab([lightness, chroma * cos, chroma * sin])
}

fn srgb_from_oklab(lab: [f64; 3]) -> Result<[u8; 3], ColorError> {
    let lms = apply_color_matrix(OKLAB_TO_LMS_ROOT, lab).map(|channel| channel.powi(3));
    let rgb = apply_color_matrix(LMS_TO_LINEAR_SRGB, lms);
    // 容差覆盖 OKLab 和 OKLCH 输出小数舍入引起的边界误差。
    let gamut = -LINEAR_GAMUT_TOLERANCE..=1. + LINEAR_GAMUT_TOLERANCE;
    if rgb.iter().any(|channel| !gamut.contains(channel)) {
        return Err(ColorError::OutsideSrgbGamut);
    }
    Ok(rgb.map(|channel| linear_to_srgb_channel(channel.clamp(0., 1.))))
}

fn oklab_from_srgb(rgb: [u8; 3]) -> [f64; 3] {
    let lms = apply_color_matrix(LINEAR_SRGB_TO_LMS, rgb.map(srgb_channel_to_linear));
    apply_color_matrix(LMS_ROOT_TO_OKLAB, lms.map(f64::cbrt))
}

fn format_oklab(rgb: [u8; 3], alpha: Option<u8>) -> String {
    let [lightness, a, b] =
        oklab_from_srgb(rgb).map(|value| if value.abs() < 0.0000005 { 0. } else { value });
    format_ok_color(format!("oklab({lightness:.6} {a:.6} {b:.6}"), alpha)
}

fn format_oklch(rgb: [u8; 3], alpha: Option<u8>) -> String {
    let [lightness, a, b] = oklab_from_srgb(rgb);
    let chroma = a.hypot(b);
    let (chroma, hue) = if chroma <= 0.000004 {
        (0., 0.)
    } else {
        (chroma, b.atan2(a).to_degrees().rem_euclid(360.))
    };
    format_ok_color(format!("oklch({lightness:.6} {chroma:.6} {hue:.4}"), alpha)
}

fn format_ok_color(base: String, alpha: Option<u8>) -> String {
    match alpha {
        Some(alpha) => format!("{base} / {})", css_number(f64::from(alpha) / 255.)),
        None => format!("{base})"),
    }
}

fn css_number(value: f64) -> String {
    format!("{value:.3}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}

fn hue_and_extrema([r, g, b]: [u8; 3]) -> (f64, f64, f64) {
    let (r, g, b) = (f64::from(r), f64::from(g), f64::from(b));
    let peak = r.max(g).max(b);
    let low = r.min(g).min(b);
    let chroma = peak - low;
    let hue = if chroma == 0. {
        0.
    } else {
        let sector = if peak == r {
            (g - b) / chroma
        } else if peak == g {
            2. + (b - r) / chroma
        } else {
            4. + (r - g) / chroma
        };
        sector.rem_euclid(6.) * 60.
    };
    (hue, low, peak)
}

fn format_hsl(rgb: [u8; 3], alpha: Option<u8>) -> String {
    let (hue, low, peak) = hue_and_extrema(rgb);
    let chroma = peak - low;
    let sum = peak + low;
    let saturation = if chroma == 0. {
        0.
    } else {
        chroma * 100. / (255. - (sum - 255.).abs())
    };
    format_hue_color("hsl", [hue, saturation, sum * 100. / 510.], alpha)
}

fn format_hwb(rgb: [u8; 3], alpha: Option<u8>) -> String {
    let (hue, low, peak) = hue_and_extrema(rgb);
    format_hue_color(
        "hwb",
        [hue, low * 100. / 255., (255. - peak) * 100. / 255.],
        alpha,
    )
}

fn format_hue_color(name: &str, values: [f64; 3], alpha: Option<u8>) -> String {
    let [hue, first, second] = values.map(css_number);
    if let Some(alpha) = alpha {
        let alpha = css_number(f64::from(alpha) / 255.);
        format!("{name}({hue} {first}% {second}% / {alpha})")
    } else {
        format!("{name}({hue} {first}% {second}%)")
    }
}

fn mix_color(input: &str) -> Result<Channels, ColorError> {
    let input = input.strip_suffix(')').ok_or(ColorError::Incomplete)?;
    let (space, stops) = input.split_once(',').ok_or(ColorError::InvalidMix)?;
    let space = space.to_ascii_lowercase();
    let mut words = space.split_whitespace();
    let space = match (words.next(), words.next(), words.next()) {
        (Some("in"), Some("srgb"), None) => ColorMixSpace::Srgb,
        (Some("in"), Some("srgb-linear"), None) => ColorMixSpace::LinearSrgb,
        (Some("in"), Some("oklab"), None) => ColorMixSpace::Oklab,
        _ => return Err(ColorError::MixColorSpace),
    };

    let mut depth = 0usize;
    let mut separator = None;
    for (index, character) in stops.char_indices() {
        match (character, depth) {
            ('(', _) => depth += 1,
            (')', _) => depth = depth.checked_sub(1).ok_or(ColorError::InvalidMix)?,
            (',', 0) if separator.is_none() => separator = Some(index),
            (',', 0) => return Err(ColorError::InvalidMix),
            _ => {}
        }
    }
    let index = separator
        .filter(|_| depth == 0)
        .ok_or(ColorError::InvalidMix)?;
    let (first, left) = mix_stop(stops[..index].trim())?;
    let (second, right) = mix_stop(stops[index + 1..].trim())?;
    let (left, right) = match (left, right) {
        (Some(left), Some(right)) => (left, right),
        (Some(left), None) => (left, 100. - left),
        (None, Some(right)) => (100. - right, right),
        (None, None) => (50., 50.),
    };
    first.mix(second, left, right, space)
}

fn mix_stop(input: &str) -> Result<(Channels, Option<f64>), ColorError> {
    let weighted = input
        .split_once(char::is_whitespace)
        .and_then(|(first, rest)| first.strip_suffix('%').map(|weight| (rest, weight)))
        .or_else(|| {
            input
                .rsplit_once(char::is_whitespace)
                .and_then(|(rest, last)| last.strip_suffix('%').map(|weight| (rest, weight)))
        });
    let Some((color, weight)) = weighted else {
        return Ok((Channels::parse(input)?, None));
    };
    let weight = bounded_number(weight, 100.).map_err(|_| ColorError::InvalidMix)?;
    Ok((Channels::parse(color.trim())?, Some(weight)))
}

pub(super) fn make_color_palette(input: &str, space: ColorMixSpace) -> Result<String, ColorError> {
    let input = normalize(input);
    let (range, count) = input.rsplit_once('@').ok_or(ColorError::InvalidPalette)?;
    let count = count
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|count| (2..=32).contains(count))
        .ok_or(ColorError::InvalidPalette)?;
    let (first, last) = color_pair(range, "到", " to ").ok_or(ColorError::InvalidPalette)?;
    let first = Channels::parse(first.trim())?;
    let last = Channels::parse(last.trim())?;
    let mut colors = Vec::with_capacity(count as usize);
    colors.push(first.format_hex());
    for index in 1..count - 1 {
        let weight = f64::from(index) / f64::from(count - 1) * 100.;
        colors.push(first.mix(last, 100. - weight, weight, space)?.format_hex());
    }
    colors.push(last.format_hex());
    Ok(colors.join("\n"))
}

pub(super) fn convert_color_to_hex(input: &str) -> Result<String, ColorError> {
    let normalized = normalize(input);
    Ok(Channels::parse(normalized.trim())?.format_hex())
}

pub(super) fn hex_for_swatch(swatch: u32) -> String {
    let [red, green, blue, alpha] = swatch.to_be_bytes();
    Channels {
        rgb: [red, green, blue],
        alpha: (alpha != 255).then_some(alpha),
    }
    .format_hex()
}

pub(super) fn convert_color(input: &str) -> Result<Color, ColorError> {
    let normalized = normalize(input);
    let input = normalized.trim();
    if input.is_empty() {
        return Err(ColorError::Incomplete);
    }
    let (input, target) = if let Some((source, target)) = split_conversion(input) {
        let target = match target.trim().to_ascii_lowercase().as_str() {
            "hex" => ColorFormat::Hex,
            "rgb" | "rgba" => ColorFormat::Rgb,
            "hsl" | "hsla" => ColorFormat::Hsl,
            "hwb" => ColorFormat::Hwb,
            "oklab" => ColorFormat::Oklab,
            "oklch" => ColorFormat::Oklch,
            "" => return Err(ColorError::Incomplete),
            _ => return Err(ColorError::InvalidFormat),
        };
        (source, Some(target))
    } else {
        (input, None)
    };
    let (first, rest) = input.split_once(char::is_whitespace).unwrap_or((input, ""));
    let choose_text = first == "文字色" || first.eq_ignore_ascii_case("text-color");
    let input = if choose_text { rest.trim() } else { input };
    let mix_prefix = "color-mix(";
    let (channels, default) = if input
        .get(..mix_prefix.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(mix_prefix))
    {
        (mix_color(&input[mix_prefix.len()..])?, ColorFormat::Hex)
    } else if let Some(channels) = composed_srgb(input) {
        (channels?, ColorFormat::Hex)
    } else {
        let default = if input.starts_with('#') {
            ColorFormat::Rgb
        } else {
            ColorFormat::Hex
        };
        (Channels::parse(input)?, default)
    };
    let (channels, default) = if choose_text {
        (pick_text_color(channels)?, ColorFormat::Hex)
    } else {
        (channels, default)
    };
    let [r, g, b] = channels.rgb;
    let output = match target.unwrap_or(default) {
        ColorFormat::Hex => channels.format_hex(),
        ColorFormat::Rgb => {
            if let Some(alpha) = channels.alpha {
                format!("rgba({r}, {g}, {b}, {:.3})", f64::from(alpha) / 255.)
            } else {
                format!("rgb({r}, {g}, {b})")
            }
        }
        ColorFormat::Hsl => format_hsl(channels.rgb, channels.alpha),
        ColorFormat::Hwb => format_hwb(channels.rgb, channels.alpha),
        ColorFormat::Oklab => format_oklab(channels.rgb, channels.alpha),
        ColorFormat::Oklch => format_oklch(channels.rgb, channels.alpha),
    };
    Ok(Color {
        swatch: channels.swatch(),
        output,
    })
}

fn srgb_channel_to_linear(value: u8) -> f64 {
    let channel = f64::from(value) / 255.;
    if channel <= 0.04045 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb_channel(linear: f64) -> u8 {
    let encoded = if linear <= 0.0031308 {
        linear * 12.92
    } else {
        1.055 * linear.powf(1. / 2.4) - 0.055
    };
    (encoded * 255.).round() as u8
}

fn relative_luminance(rgb: [u8; 3]) -> f64 {
    let [r, g, b] = rgb.map(srgb_channel_to_linear);
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

fn contrast_ratio(first: f64, second: f64) -> f64 {
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

fn pick_text_color(background: Channels) -> Result<Channels, ColorError> {
    if background.alpha.unwrap_or(255) != 255 {
        return Err(ColorError::TransparentTextBackground);
    }
    let luminance = relative_luminance(background.rgb);
    let channel = if contrast_ratio(luminance, 0.) >= contrast_ratio(luminance, 1.) {
        0
    } else {
        255
    };
    Ok(Channels {
        rgb: [channel; 3],
        alpha: None,
    })
}

fn color_pair<'a>(input: &'a str, local: &str, english: &str) -> Option<(&'a str, &'a str)> {
    input.split_once(local).or_else(|| {
        let start = input.to_ascii_lowercase().find(english)?;
        Some((&input[..start], &input[start + english.len()..]))
    })
}

fn composed_srgb(input: &str) -> Option<Result<Channels, ColorError>> {
    let (front, backdrop) = color_pair(input, "叠加", " over ")?;
    Some(Channels::parse(front.trim()).and_then(|front| {
        Channels::parse(backdrop.trim()).map(|background| front.over(background))
    }))
}

fn opaque_color_pair(
    input: &str,
    incomplete: ColorError,
    transparent: ColorError,
) -> Result<[Channels; 2], ColorError> {
    let input = normalize(input);
    let (first, second) = color_pair(&input, "对比", " vs ").ok_or(incomplete)?;
    let parse = |input: &str| {
        let input = input.trim();
        composed_srgb(input).unwrap_or_else(|| Channels::parse(input))
    };
    let first = parse(first)?;
    let second = parse(second)?;
    if first.alpha.unwrap_or(255) != 255 || second.alpha.unwrap_or(255) != 255 {
        return Err(transparent);
    }
    Ok([first, second])
}

pub(super) fn calculate_contrast(input: &str) -> Result<ColorComparison, ColorError> {
    let [first, second] = opaque_color_pair(
        input,
        ColorError::IncompleteContrast,
        ColorError::TransparentContrast,
    )?;
    let first_luminance = relative_luminance(first.rgb);
    let second_luminance = relative_luminance(second.rgb);
    let ratio = contrast_ratio(first_luminance, second_luminance);
    let ratio = format_result(ratio).map_err(|_| ColorError::Invalid)?;
    Ok(ColorComparison {
        output: format!("{ratio}:1"),
        swatches: [first.swatch(), second.swatch()],
    })
}

pub(super) fn calculate_color_difference(input: &str) -> Result<ColorComparison, ColorError> {
    let [first, second] = opaque_color_pair(
        input,
        ColorError::IncompleteDifference,
        ColorError::TransparentDifference,
    )?;
    let [l1, a1, b1] = oklab_from_srgb(first.rgb);
    let [l2, a2, b2] = oklab_from_srgb(second.rgb);
    let delta = ((l1 - l2).powi(2) + (a1 - a2).powi(2) + (b1 - b2).powi(2)).sqrt();
    Ok(ColorComparison {
        output: format_result(delta).map_err(|_| ColorError::Invalid)?,
        swatches: [first.swatch(), second.swatch()],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composites_each_color_before_measuring_contrast_or_difference() {
        let contrast = calculate_contrast("#0008 叠加 #fff vs #fff").unwrap();
        let ratio = contrast
            .output
            .strip_suffix(":1")
            .unwrap()
            .parse::<f64>()
            .unwrap();
        assert!((ratio - 4.478089453577214).abs() < 1e-12);
        assert_eq!(contrast.swatches, [0x777777ff, 0xffffffff]);

        let comparison = calculate_color_difference("#777 vs #0008 over white").unwrap();
        assert_eq!(comparison.output, "0");
        assert_eq!(comparison.swatches, [0x777777ff; 2]);
        assert_eq!(
            calculate_contrast("#0008 over #fff8 vs white"),
            Err(ColorError::TransparentContrast),
        );
    }

    #[test]
    fn compares_opaque_srgb_colors_by_oklab_distance() {
        let comparison = calculate_color_difference("red vs lime").unwrap();
        let distance = comparison.output.parse::<f64>().unwrap();
        assert!((distance - 0.519814).abs() < 0.000002);
        assert_eq!(comparison.swatches, [0xff0000ff, 0x00ff00ff]);
        assert_eq!(
            calculate_color_difference("lime 对比 red").unwrap().output,
            comparison.output,
        );
        assert_eq!(
            calculate_color_difference("#fff vs white").unwrap().output,
            "0"
        );
        assert_eq!(
            calculate_color_difference("red vs #fffa"),
            Err(ColorError::TransparentDifference),
        );
        assert_eq!(
            calculate_color_difference("red"),
            Err(ColorError::IncompleteDifference),
        );
    }

    #[test]
    fn picks_the_higher_contrast_of_black_or_white_for_an_opaque_background() {
        for (input, output, swatch) in [
            ("文字色 #3A86FF", "#000000", 0x000000ff),
            ("文字色 #757575", "#FFFFFF", 0xffffffff),
            ("文字色 #767676", "#000000", 0x000000ff),
            ("text-color #123 to rgb", "rgb(255, 255, 255)", 0xffffffff),
            ("文字色 #fff8 over #000", "#000000", 0x000000ff),
        ] {
            assert_eq!(
                convert_color(input),
                Ok(Color {
                    output: output.to_owned(),
                    swatch,
                }),
                "{input}",
            );
        }
        assert_eq!(
            convert_color("文字色 transparent"),
            Err(ColorError::TransparentTextBackground),
        );
        assert_eq!(convert_color("文字色"), Err(ColorError::Incomplete));
    }

    #[test]
    fn produces_equally_spaced_srgb_hex_colors_including_both_endpoints() {
        assert_eq!(
            make_color_palette("#000 到 #fff @ 5", ColorMixSpace::Srgb).as_deref(),
            Ok("#000000\n#404040\n#808080\n#BFBFBF\n#FFFFFF")
        );
        assert_eq!(
            make_color_palette("#F000 TO rgba(0, 0, 255, 0.5) @ 3", ColorMixSpace::Srgb).as_deref(),
            Ok("#FF000000\n#0000FF40\n#0000FF80")
        );
        assert_eq!(
            make_color_palette("rebeccapurple 到 #0080ff @ 2", ColorMixSpace::Srgb).as_deref(),
            Ok("#663399\n#0080FF")
        );
        assert_eq!(
            make_color_palette("#000 到 #fff @ 33", ColorMixSpace::Srgb),
            Err(ColorError::InvalidPalette)
        );
    }

    #[test]
    fn produces_linear_light_steps_with_premultiplied_alpha() {
        assert_eq!(
            make_color_palette("#000 到 #fff @ 3", ColorMixSpace::LinearSrgb).as_deref(),
            Ok("#000000\n#BCBCBC\n#FFFFFF"),
        );
        assert_eq!(
            make_color_palette(
                "#F000 TO rgba(0, 0, 255, 0.5) @ 3",
                ColorMixSpace::LinearSrgb
            )
            .as_deref(),
            Ok("#FF000000\n#0000FF40\n#0000FF80"),
        );
    }

    #[test]
    fn mixes_two_srgb_colors_with_premultiplied_alpha_and_relative_weights() {
        for (input, output, swatch) in [
            ("color-mix(in srgb, white, black)", "#808080", 0x808080ff),
            ("COLOR-MIX(IN SRGB, 75% #000, #fff)", "#404040", 0x404040ff),
            (
                "color-mix(in srgb, rgba(255, 0, 0, 0.5), blue)",
                "#5500AAC0",
                0x5500aac0,
            ),
            (
                "color-mix(in srgb, #f00 20%, #00f 30%)",
                "#66009980",
                0x66009980,
            ),
            (
                "color-mix(in srgb, #fff 80%, #000 80%)",
                "#808080",
                0x808080ff,
            ),
            (
                "color-mix(in srgb, transparent, #f00) to rgb",
                "rgba(255, 0, 0, 0.502)",
                0xff000080,
            ),
        ] {
            assert_eq!(
                convert_color(input),
                Ok(Color {
                    output: output.to_owned(),
                    swatch,
                }),
                "{input}",
            );
        }

        for input in [
            "color-mix(in srgb, white 0%, black 0%)",
            "color-mix(in srgb, white, black -3%)",
            "color-mix(in srgb, white 101%, black)",
            "color-mix(in srgb, white, black, red)",
            "color-mix(in srgb, rgba(1, 2, 3, 1, blue)",
        ] {
            assert_eq!(convert_color(input), Err(ColorError::InvalidMix), "{input}");
        }
        assert_eq!(
            convert_color("color-mix(in lab, white, black)"),
            Err(ColorError::MixColorSpace),
        );
        assert_eq!(
            convert_color("color-mix(in srgb, white, black"),
            Err(ColorError::Incomplete),
        );
    }

    #[test]
    fn mixes_in_linear_light_and_retains_the_same_alpha_weighting() {
        for (input, output, swatch) in [
            (
                "color-mix(in srgb-linear, white, black)",
                "#BCBCBC",
                0xbcbcbcff,
            ),
            (
                "color-mix(IN SRGB-LINEAR, rgba(255, 0, 0, 0.5), blue)",
                "#9C00D5C0",
                0x9c00d5c0,
            ),
            (
                "color-mix(in srgb-linear, #f00 20%, #00f 30%)",
                "#AA00CB80",
                0xaa00cb80,
            ),
            (
                "color-mix(in srgb-linear, transparent, transparent)",
                "#00000000",
                0,
            ),
        ] {
            assert_eq!(
                convert_color(input),
                Ok(Color {
                    output: output.to_owned(),
                    swatch
                }),
                "{input}"
            );
        }
    }

    #[test]
    fn mixes_in_oklab_with_premultiplied_alpha_and_checks_the_srgb_gamut() {
        for (input, output, swatch) in [
            ("color-mix(in oklab, #000, #fff)", "#636363", 0x636363ff),
            (
                "color-mix(in OKLAB, #f00, transparent)",
                "#FF000080",
                0xff000080,
            ),
            (
                "color-mix(in oklab, #0f0 100%, #00f)",
                "#00FF00",
                0x00ff00ff,
            ),
        ] {
            assert_eq!(
                convert_color(input),
                Ok(Color {
                    output: output.to_owned(),
                    swatch,
                }),
                "{input}"
            );
        }
        assert_eq!(
            convert_color("color-mix(in oklab, #0f0, #00f)"),
            Err(ColorError::OutsideSrgbGamut)
        );
        assert_eq!(
            make_color_palette("#000 到 #fff @ 3", ColorMixSpace::Oklab).as_deref(),
            Ok("#000000\n#636363\n#FFFFFF")
        );
    }

    #[test]
    fn composites_foreground_over_background_using_both_alpha_channels() {
        for (input, output, swatch) in [
            ("#0080FF80 叠加 #fff", "#7FBFFF", 0x7fbfffff),
            ("#FF000080 OVER #0000FF80", "#AA0055C0", 0xaa0055c0),
            ("#1230 over #4450", "#00000000", 0x00000000),
            ("#123 over #456 to rgb", "rgb(17, 34, 51)", 0x112233ff),
        ] {
            assert_eq!(
                convert_color(input),
                Ok(Color {
                    output: output.to_owned(),
                    swatch
                }),
                "{input}",
            );
        }
    }

    #[test]
    fn calculates_contrast_in_either_order_and_preserves_both_swatches() {
        for (input, ratio, swatches) in [
            ("#fff vs #000", "21:1", [0xffffffff, 0x000000ff]),
            ("#000 对比 #fff", "21:1", [0x000000ff, 0xffffffff]),
            ("#f00 VS rgb(0 0 0)", "5.252:1", [0xff0000ff, 0x000000ff]),
            ("#888 vs #888f", "1:1", [0x888888ff, 0x888888ff]),
            ("hsl(0 0% 100%) vs #000", "21:1", [0xffffffff, 0x000000ff]),
        ] {
            assert_eq!(
                calculate_contrast(input),
                Ok(ColorComparison {
                    output: ratio.to_owned(),
                    swatches
                }),
                "{input}",
            );
        }
        assert_eq!(
            calculate_contrast("#fff"),
            Err(ColorError::IncompleteContrast)
        );
        assert_eq!(
            calculate_contrast("#fff8 vs #000"),
            Err(ColorError::TransparentContrast)
        );
    }

    #[test]
    fn converts_to_explicit_formats_and_round_trips_hsl_channels() {
        for (input, output, swatch) in [
            (
                "#FF800080 to hsl",
                "hsl(30.118 100% 50% / 0.502)",
                0xff800080,
            ),
            ("#808080 转 HSL", "hsl(0 0% 50.196%)", 0x808080ff),
            ("rgb(0 255 0) to HSL", "hsl(120 100% 50%)", 0x00ff00ff),
            ("#f0f0 -> hsl", "hsl(300 100% 50% / 0)", 0xff00ff00),
            ("hsl(0 100% 50%) to rgb", "rgb(255, 0, 0)", 0xff0000ff),
            ("#abc to HEX", "#AABBCC", 0xaabbccff),
        ] {
            let result = convert_color(input).unwrap();
            assert_eq!(result.output, output, "{input}");
            assert_eq!(result.swatch, swatch, "{input}");
            assert_eq!(convert_color(output).unwrap().swatch, swatch, "{output}");
        }
        assert_eq!(convert_color("#abc to"), Err(ColorError::Incomplete));
        assert_eq!(convert_color("#abc to xyz"), Err(ColorError::InvalidFormat));
    }

    #[test]
    fn converts_srgb_and_oklch_while_retaining_alpha_and_rejecting_outside_gamut() {
        assert_eq!(
            convert_color("oklch(70% 25% 180 / 50%)").unwrap().output,
            "#4BB3A180",
        );
        assert_eq!(convert_color("oklch(0.5 0 0)").unwrap().output, "#636363");
        assert_eq!(
            convert_color("#f00 to oklch").unwrap().output,
            "oklch(0.627955 0.257683 29.2339)",
        );
        for hex in [
            "#f00",
            "#0f0",
            "#00f",
            "#fff",
            "#000",
            "#abcdef80",
            "#80c0ff7f",
        ] {
            let oklch = convert_color(&format!("{hex} to oklch")).unwrap();
            assert_eq!(
                convert_color(&oklch.output).unwrap().swatch,
                oklch.swatch,
                "{hex}"
            );
        }
        assert_eq!(
            convert_color("oklch(0.7 0.4 180)"),
            Err(ColorError::OutsideSrgbGamut),
        );
        assert_eq!(
            convert_color("oklch(0.7, 0.1, 180)"),
            Err(ColorError::Invalid)
        );
        assert_eq!(
            convert_color("oklch(0.7 -0.1 180)"),
            Err(ColorError::Invalid)
        );
    }

    #[test]
    fn converts_srgb_and_oklab_using_signed_axes_and_optional_alpha() {
        for (query, expected) in [
            ("#f00 to oklab", "oklab(0.627955 0.224863 0.125846)"),
            ("oklab(62.7955% 56.21575% 31.4615% / 50%)", "#FF000080"),
            ("OKLAB(50% none none / none)", "#63636300"),
            ("oklab(0.5 0 0)", "#636363"),
        ] {
            assert_eq!(convert_color(query).unwrap().output, expected, "{query}");
        }
        for hex in ["#f00", "#0f0", "#00f", "#fff", "#000", "#1234567F"] {
            let lab = convert_color(&format!("{hex} to oklab")).unwrap();
            assert_eq!(
                convert_color(&lab.output).unwrap().swatch,
                lab.swatch,
                "{hex}"
            );
        }
        for query in ["oklab(0.5, 0, 0)", "oklab(0.5 NaN 0)", "oklab(101% 0 0)"] {
            assert_eq!(convert_color(query), Err(ColorError::Invalid), "{query}");
        }
        assert_eq!(
            convert_color("oklab(0.7 -0.4 0.4)"),
            Err(ColorError::OutsideSrgbGamut)
        );
    }

    #[test]
    fn converts_hwb_colors_with_whiteness_blackness_and_alpha() {
        for (input, output) in [
            ("hwb(210 10% 20%)", "#1A73CC"),
            ("hwb(30deg 80% 40% / 50%)", "#AAAAAA80"),
            ("hwb(.5turn 0 0)", "#00FFFF"),
            ("#FF0000 to hwb", "hwb(0 0% 0%)"),
            ("#00000080 to hwb", "hwb(0 0% 100% / 0.502)"),
            ("navy to hwb", "hwb(240 0% 49.804%)"),
        ] {
            assert_eq!(convert_color(input).unwrap().output, output, "{input}");
        }
        assert_eq!(
            calculate_contrast("hwb(0 100% 0%) vs black")
                .unwrap()
                .output,
            "21:1"
        );
        for input in ["hwb(0, 10%, 20%)", "hwb(0 -1% 10%)", "hwb(0 10%)"] {
            assert!(
                matches!(convert_color(input), Err(ColorError::Invalid)),
                "{input}"
            );
        }
    }

    #[test]
    fn converts_hsl_colors_with_wrapped_hues_and_optional_alpha() {
        for (input, output, swatch) in [
            ("hsl(210 100% 50%)", "#0080FF", 0x0080ffff),
            ("HSLA(-120deg, 100%, 50%, .5)", "#0000FF80", 0x0000ff80),
            ("hsl(0.25turn 0% 50% / 25%)", "#80808040", 0x80808040),
            ("hsl(400grad 100% 50%)", "#FF0000", 0xff0000ff),
            ("hsl(0 0% 0%)", "#000000", 0x000000ff),
        ] {
            assert_eq!(
                convert_color(input),
                Ok(Color {
                    swatch,
                    output: output.to_owned()
                }),
                "{input}",
            );
        }
    }

    #[test]
    fn named_colors_work_in_conversion_compositing_and_contrast() {
        for (input, output) in [
            ("Navy", "#000080"),
            ("RebeccaPurple", "#663399"),
            ("PaleGoldenRod to rgb", "rgb(238, 232, 170)"),
            ("transparent", "#00000000"),
            ("transparent over tomato", "#FF6347"),
        ] {
            assert_eq!(convert_color(input).unwrap().output, output, "{input}");
        }
        assert_eq!(calculate_contrast("White vs BLACK").unwrap().output, "21:1");
        assert!(matches!(
            calculate_contrast("transparent vs white"),
            Err(ColorError::TransparentContrast)
        ));
        assert!(matches!(
            convert_color("currentColor"),
            Err(ColorError::Invalid)
        ));
    }

    #[test]
    fn converts_hex_and_rgb_with_optional_alpha() {
        for (input, output, swatch) in [
            ("#3a86fF", "rgb(58, 134, 255)", 0x3a86ffff),
            ("#B34", "rgb(187, 51, 68)", 0xbb3344ff),
            ("#B340", "rgba(187, 51, 68, 0.000)", 0xbb334400),
            ("#3a86ff80", "rgba(58, 134, 255, 0.502)", 0x3a86ff80),
            ("RGB（58， 134， 255）", "#3A86FF", 0x3a86ffff),
            ("rgba(58, 134, 255, 0.502)", "#3A86FF80", 0x3a86ff80),
            ("rgb(100% 50% 0 / 50%)", "#FF800080", 0xff800080),
        ] {
            assert_eq!(
                convert_color(input),
                Ok(Color {
                    swatch,
                    output: output.to_owned()
                }),
                "{input}",
            );
        }
    }

    #[test]
    fn requires_valid_digits_and_channel_ranges() {
        assert_eq!(convert_color(""), Err(ColorError::Incomplete));
        assert_eq!(convert_color("rgb("), Err(ColorError::Incomplete));
        for input in [
            "#+12",
            "#12345",
            "#12z",
            "rgb(256,0,0)",
            "rgb(-1 0 0)",
            "rgb(0 0 0 / 101%)",
            "rgb(0,0,0,NaN)",
            "rgb(0 0)",
            "hsl(120 101% 50%)",
            "hsl(NaN 100% 50%)",
            "hsl(120 50 50%)",
        ] {
            assert_eq!(convert_color(input), Err(ColorError::Invalid), "{input}");
        }
    }
}
