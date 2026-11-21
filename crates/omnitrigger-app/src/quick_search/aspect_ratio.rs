use super::{calculator::normalize, conversion::split_conversion, fraction::common_divisor};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AspectRatioError {
    Incomplete,
    InvalidDimension,
    InvalidTarget,
    OutOfRange,
}

impl AspectRatioError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入宽和高，例如 1920×1080",
            Self::InvalidDimension => "宽和高必须是正整数",
            Self::InvalidTarget => "指定目标宽或高，例如 按宽2560 或 按高720",
            Self::OutOfRange => "缩放后的宽或高超出像素范围",
        }
    }
}

fn dimension(input: &str) -> Result<u32, AspectRatioError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(AspectRatioError::Incomplete);
    }
    input
        .parse::<u32>()
        .ok()
        .filter(|&value| value != 0)
        .ok_or(AspectRatioError::InvalidDimension)
}

pub(super) fn pixel_dimensions(input: &str) -> Result<(u32, u32), AspectRatioError> {
    let (width, height) = input
        .split_once(['×', 'x', 'X', ':'])
        .ok_or(AspectRatioError::Incomplete)?;
    Ok((dimension(width)?, dimension(height)?))
}

fn scale_dimension(value: u32, original: u32, target: u32) -> Result<u32, AspectRatioError> {
    let original = u64::from(original);
    let rounded = (u64::from(value) * u64::from(target) + original / 2) / original;
    u32::try_from(rounded)
        .ok()
        .filter(|&value| value != 0)
        .ok_or(AspectRatioError::OutOfRange)
}

pub(super) fn calculate_aspect_ratio(query: &str) -> Result<String, AspectRatioError> {
    let input = normalize(query);
    let (size, target) = input
        .split_once('按')
        .or_else(|| split_conversion(&input))
        .map_or((input.as_str(), None), |(size, target)| {
            (size, Some(target.trim()))
        });
    let (width, height) = pixel_dimensions(size)?;
    let Some(target) = target else {
        let (width, height) = (u128::from(width), u128::from(height));
        let divisor = common_divisor(width, height);
        return Ok(format!("{}:{}", width / divisor, height / divisor));
    };
    if target.is_empty() {
        return Err(AspectRatioError::Incomplete);
    }
    let (width, height) = if let Some(target) = target
        .strip_prefix('宽')
        .or_else(|| target.strip_suffix(['w', 'W']))
    {
        let target = dimension(target)?;
        (target, scale_dimension(height, width, target)?)
    } else if let Some(target) = target
        .strip_prefix('高')
        .or_else(|| target.strip_suffix(['h', 'H']))
    {
        let target = dimension(target)?;
        (scale_dimension(width, height, target)?, target)
    } else {
        return Err(AspectRatioError::InvalidTarget);
    };
    Ok(format!("{width} × {height}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduces_pixel_dimensions_and_scales_one_side_to_the_nearest_pixel() {
        for (query, expected) in [
            ("1920×1080", "16:9"),
            ("1080 X 1920", "9:16"),
            ("３８４０：２１６０", "16:9"),
            ("1920x1080 按宽2560", "2560 × 1440"),
            ("1920x1080 按高720", "1280 × 720"),
            ("4:3 TO 10W", "10 × 8"),
            ("1920x1080 to 777h", "1381 × 777"),
            ("4294967295x4294967295", "1:1"),
        ] {
            assert_eq!(
                calculate_aspect_ratio(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        for (query, expected) in [
            ("1920×", AspectRatioError::Incomplete),
            ("1920x1080 按宽", AspectRatioError::Incomplete),
            ("0x1080", AspectRatioError::InvalidDimension),
            ("1920x-1", AspectRatioError::InvalidDimension),
            ("1920x1080 按 999", AspectRatioError::InvalidTarget),
            ("4294967295x1 按高4294967295", AspectRatioError::OutOfRange),
            ("4294967295x1 按宽1", AspectRatioError::OutOfRange),
        ] {
            assert_eq!(calculate_aspect_ratio(query), Err(expected), "{query}");
        }
    }
}
