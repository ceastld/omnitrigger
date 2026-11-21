use super::calculator::normalize;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Color {
    pub(super) swatch: u32,
    pub(super) output: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ColorError {
    Incomplete,
    Invalid,
}

impl ColorError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入 HEX 或 RGB 颜色",
            Self::Invalid => "检查 HEX 位数或 RGB 通道范围",
        }
    }
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
        if !name.eq_ignore_ascii_case("rgb") && !name.eq_ignore_ascii_case("rgba") {
            return Err(ColorError::Invalid);
        }
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
        let &[r, g, b] = values.as_slice() else {
            return Err(ColorError::Invalid);
        };
        Ok(Self {
            rgb: [
                component(r, 255.)?,
                component(g, 255.)?,
                component(b, 255.)?,
            ],
            alpha: alpha.map(|value| component(value, 1.)).transpose()?,
        })
    }
}

fn component(text: &str, max: f64) -> Result<u8, ColorError> {
    let text = text.trim();
    let (text, max) = text
        .strip_suffix('%')
        .map_or((text, max), |number| (number, 100.));
    let value = text.parse::<f64>().map_err(|_| ColorError::Invalid)?;
    if !(0.0..=max).contains(&value) {
        return Err(ColorError::Invalid);
    }
    Ok(if max == 255. {
        value
    } else {
        value * 255. / max
    }
    .round() as u8)
}

pub(super) fn convert_color(input: &str) -> Result<Color, ColorError> {
    let normalized = normalize(input);
    let input = normalized.trim();
    if input.is_empty() {
        return Err(ColorError::Incomplete);
    }
    if let Some(digits) = input.strip_prefix('#') {
        let channels = Channels::hex(digits)?;
        let [r, g, b] = channels.rgb;
        let output = if let Some(alpha) = channels.alpha {
            format!("rgba({r}, {g}, {b}, {:.3})", f64::from(alpha) / 255.)
        } else {
            format!("rgb({r}, {g}, {b})")
        };
        return Ok(channels.into_color(output));
    }
    let channels = Channels::function(input)?;
    let [r, g, b] = channels.rgb;
    let mut output = format!("#{r:02X}{g:02X}{b:02X}");
    if let Some(alpha) = channels.alpha {
        use std::fmt::Write;
        write!(output, "{alpha:02X}").expect("写入字符串");
    }
    Ok(channels.into_color(output))
}

#[cfg(test)]
mod tests {
    use super::*;

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
            "hsl(0 0% 0%)",
        ] {
            assert_eq!(convert_color(input), Err(ColorError::Invalid), "{input}");
        }
    }
}
