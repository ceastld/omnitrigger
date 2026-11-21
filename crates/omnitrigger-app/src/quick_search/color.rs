use std::f64::consts::TAU;

use super::calculator::normalize;
use super::conversion::split_conversion;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Color {
    pub(super) swatch: u32,
    pub(super) output: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ColorError {
    Incomplete,
    Invalid,
    InvalidFormat,
}

impl ColorError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入 HEX、RGB 或 HSL 颜色",
            Self::Invalid => "检查 HEX 位数或颜色通道范围",
            Self::InvalidFormat => "目标颜色格式可用 HEX、RGB 或 HSL",
        }
    }
}

enum ColorFormat {
    Hex,
    Rgb,
    Hsl,
}

struct Channels {
    rgb: [u8; 3],
    alpha: Option<u8>,
}

impl Channels {
    fn into_color(self, output: String) -> Color {
        Color {
            swatch: u32::from_be_bytes([
                self.rgb[0],
                self.rgb[1],
                self.rgb[2],
                self.alpha.unwrap_or(255),
            ]),
            output,
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
        let hsl = match name.to_ascii_lowercase().as_str() {
            "rgb" | "rgba" => false,
            "hsl" | "hsla" => true,
            _ => return Err(ColorError::Invalid),
        };
        let terms = terms.strip_suffix(')').ok_or(ColorError::Incomplete)?;
        let (values, alpha) = if terms.contains(',') {
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
        let rgb = if hsl {
            rgb_from_hsl(first, second, third)?
        } else {
            [
                component(first, 255.)?,
                component(second, 255.)?,
                component(third, 255.)?,
            ]
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
    let middle = chroma * (1. - (sector % 2. - 1.).abs());
    let floor = lightness - chroma / 2.;
    let rgb = match sector as u8 {
        0 => [chroma, middle, 0.],
        1 => [middle, chroma, 0.],
        2 => [0., chroma, middle],
        3 => [0., middle, chroma],
        4 => [middle, 0., chroma],
        _ => [chroma, 0., middle],
    };
    Ok(rgb.map(|value| ((value + floor) * 255.).round() as u8))
}

fn css_number(value: f64) -> String {
    format!("{value:.3}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}

fn format_hsl([r, g, b]: [u8; 3], alpha: Option<u8>) -> String {
    let (r, g, b) = (f64::from(r), f64::from(g), f64::from(b));
    let peak = r.max(g).max(b);
    let low = r.min(g).min(b);
    let chroma = peak - low;
    let sum = peak + low;
    let lightness = sum * 100. / 510.;
    // CSS Color 4 的 sRGB 转 HSL 公式，通道以 0–255 表示。
    let (hue, saturation) = if chroma == 0. {
        (0., 0.)
    } else {
        let sector = if peak == r {
            (g - b) / chroma
        } else if peak == g {
            2. + (b - r) / chroma
        } else {
            4. + (r - g) / chroma
        };
        (
            sector.rem_euclid(6.) * 60.,
            chroma * 100. / (255. - (sum - 255.).abs()),
        )
    };
    let hue = css_number(hue);
    let saturation = css_number(saturation);
    let lightness = css_number(lightness);
    if let Some(alpha) = alpha {
        let alpha = css_number(f64::from(alpha) / 255.);
        format!("hsl({hue} {saturation}% {lightness}% / {alpha})")
    } else {
        format!("hsl({hue} {saturation}% {lightness}%)")
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
            "" => return Err(ColorError::Incomplete),
            _ => return Err(ColorError::InvalidFormat),
        };
        (source, Some(target))
    } else {
        (input, None)
    };
    let (channels, default) = if let Some(digits) = input.strip_prefix('#') {
        (Channels::hex(digits)?, ColorFormat::Rgb)
    } else {
        (Channels::function(input)?, ColorFormat::Hex)
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
    };
    Ok(channels.into_color(output))
}

#[cfg(test)]
mod tests {
    use super::*;

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
