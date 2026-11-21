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
    TransparentContrast,
}

impl ColorError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入 HEX、RGB、HSL、HWB 或 CSS 颜色名",
            Self::IncompleteContrast => "输入两个颜色，例如 对比度: #fff vs #000",
            Self::Invalid => "检查颜色名、HEX 位数或颜色通道范围",
            Self::InvalidFormat => "目标颜色格式可用 HEX、RGB、HSL 或 HWB",
            Self::TransparentContrast => "计算对比度需要两个不透明颜色",
        }
    }
}

enum ColorFormat {
    Hex,
    Rgb,
    Hsl,
    Hwb,
}

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
    let (channels, default) = if let Some((front, backdrop)) = color_pair(input, "叠加", " over ")
    {
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
    let [r, g, b] = channels.rgb;
    let output = match target.unwrap_or(default) {
        ColorFormat::Hex => {
            if let Some(alpha) = channels.alpha {
                format!("#{r:02X}{g:02X}{b:02X}{alpha:02X}")
            } else {
                format!("#{r:02X}{g:02X}{b:02X}")
            }
        }
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

fn relative_luminance(rgb: [u8; 3]) -> f64 {
    let [r, g, b] = rgb.map(|value| {
        let channel = f64::from(value) / 255.;
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    });
    0.2126 * r + 0.7152 * g + 0.0722 * b
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
    let ratio = (first_luminance.max(second_luminance) + 0.05)
        / (first_luminance.min(second_luminance) + 0.05);
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
