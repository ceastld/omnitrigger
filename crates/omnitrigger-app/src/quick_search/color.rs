use std::f64::consts::TAU;

use super::calculator::{format_result, normalize};
use super::conversion::split_conversion;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Color {
    pub(super) swatch: u32,
    pub(super) output: String,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct ColorContrast {
    pub(super) output: String,
    pub(super) swatches: [u32; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ColorError {
    Incomplete,
    IncompleteContrast,
    Invalid,
    InvalidFormat,
    InvalidMix,
    InvalidPalette,
    MixColorSpace,
    TransparentContrast,
    TransparentTextBackground,
}

impl ColorError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入 HEX、RGB、HSL、HWB 或 CSS 颜色名",
            Self::IncompleteContrast => "输入两个颜色，例如 对比度: #fff vs #000",
            Self::Invalid => "检查颜色名、HEX 位数或颜色通道范围",
            Self::InvalidFormat => "目标颜色格式可用 HEX、RGB、HSL 或 HWB",
            Self::InvalidMix => "输入两个颜色，权重范围为 0% 到 100%，总权重需大于 0%",
            Self::InvalidPalette => {
                "填写起始颜色、结束颜色和 2 至 32 个色值，例如 #000 到 #fff @ 5"
            }
            Self::MixColorSpace => "颜色混合支持 in srgb 或 in srgb-linear",
            Self::TransparentContrast => "计算对比度需要两个不透明颜色",
            Self::TransparentTextBackground => "需要不透明底色，也可先叠加到实际背景上",
        }
    }
}

enum ColorFormat {
    Hex,
    Rgb,
    Hsl,
    Hwb,
}

#[derive(Clone, Copy)]
pub(super) enum ColorMixSpace {
    Srgb,
    LinearSrgb,
}

impl ColorMixSpace {
    fn interpolate_channel(self, left: u8, right: u8, first: f64, second: f64, total: f64) -> u8 {
        match self {
            Self::Srgb => {
                ((f64::from(left) * first + f64::from(right) * second) / total).round() as u8
            }
            Self::LinearSrgb => {
                let linear = (srgb_channel_to_linear(left) * first
                    + srgb_channel_to_linear(right) * second)
                    / total;
                let encoded = if linear <= 0.0031308 {
                    linear * 12.92
                } else {
                    1.055 * linear.powf(1. / 2.4) - 0.055
                };
                (encoded * 255.).round() as u8
            }
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
            std::array::from_fn(|index| {
                space.interpolate_channel(
                    self.rgb[index],
                    other.rgb[index],
                    first,
                    second,
                    combined,
                )
            })
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
            _ => return Err(ColorError::Invalid),
        };
        let terms = terms.strip_suffix(')').ok_or(ColorError::Incomplete)?;
        let (values, alpha) = if terms.contains(',') {
            if matches!(format, ColorFormat::Hwb) {
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
            ColorFormat::Rgb => [
                component(first, 255.)?,
                component(second, 255.)?,
                component(third, 255.)?,
            ],
            ColorFormat::Hex => unreachable!("HEX 不使用函数形式"),
        };
        Ok(Self {
            rgb,
            alpha: alpha.map(|value| component(value, 1.)).transpose()?,
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
    } else if let Some((front, backdrop)) = color_pair(input, "叠加", " over ") {
        (
            Channels::parse(front.trim())?.over(Channels::parse(backdrop.trim())?),
            ColorFormat::Hex,
        )
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

pub(super) fn calculate_contrast(input: &str) -> Result<ColorContrast, ColorError> {
    let input = normalize(input);
    let (first, second) =
        color_pair(&input, "对比", " vs ").ok_or(ColorError::IncompleteContrast)?;
    let first = Channels::parse(first.trim())?;
    let second = Channels::parse(second.trim())?;
    if first.alpha.unwrap_or(255) != 255 || second.alpha.unwrap_or(255) != 255 {
        return Err(ColorError::TransparentContrast);
    }
    let first_luminance = relative_luminance(first.rgb);
    let second_luminance = relative_luminance(second.rgb);
    let ratio = contrast_ratio(first_luminance, second_luminance);
    let ratio = format_result(ratio).map_err(|_| ColorError::Invalid)?;
    Ok(ColorContrast {
        output: format!("{ratio}:1"),
        swatches: [first.swatch(), second.swatch()],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
            convert_color("color-mix(in oklab, white, black)"),
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
                Ok(ColorContrast {
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
