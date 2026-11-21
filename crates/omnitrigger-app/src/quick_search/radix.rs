use std::num::IntErrorKind;

use super::conversion::ConversionError;

#[derive(Clone, Copy)]
pub(super) enum Base {
    Binary = 2,
    Octal = 8,
    Decimal = 10,
    Hexadecimal = 16,
}

impl Base {
    pub(super) fn parse(target: &str) -> Option<Self> {
        match target.trim().to_ascii_lowercase().as_str() {
            "bin" | "binary" | "二进制" => Some(Self::Binary),
            "oct" | "octal" | "八进制" => Some(Self::Octal),
            "dec" | "decimal" | "十进制" => Some(Self::Decimal),
            "hex" | "hexadecimal" | "十六进制" => Some(Self::Hexadecimal),
            _ => None,
        }
    }

    pub(super) fn convert(self, source: &str) -> Result<String, ConversionError> {
        let source = source.to_ascii_lowercase();
        let negative = source.starts_with('-');
        let magnitude = source.strip_prefix(['-', '+']).unwrap_or(&source);
        let (base, digits) = if let Some(digits) = magnitude.strip_prefix("0b") {
            (Self::Binary, digits)
        } else if let Some(digits) = magnitude.strip_prefix("0o") {
            (Self::Octal, digits)
        } else if let Some(digits) = magnitude.strip_prefix("0x") {
            (Self::Hexadecimal, digits)
        } else {
            (Self::Decimal, magnitude)
        };
        if !digits
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            || digits.split('_').any(str::is_empty)
        {
            return Err(ConversionError::InvalidInteger);
        }
        let value =
            u128::from_str_radix(&digits.replace('_', ""), base as u32).map_err(|error| {
                if *error.kind() == IntErrorKind::PosOverflow {
                    ConversionError::OutOfRange
                } else {
                    ConversionError::InvalidInteger
                }
            })?;
        let sign = if negative && value != 0 { "-" } else { "" };
        Ok(match self {
            Self::Binary => format!("{sign}0b{value:b}"),
            Self::Octal => format!("{sign}0o{value:o}"),
            Self::Decimal => format!("{sign}{value}"),
            Self::Hexadecimal => format!("{sign}0x{value:x}"),
        })
    }
}
