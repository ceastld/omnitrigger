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
    const TARGETS: [(Self, &[&str]); 4] = [
        (Self::Decimal, &["dec", "decimal", "十进制"]),
        (Self::Binary, &["bin", "binary", "二进制"]),
        (Self::Octal, &["oct", "octal", "八进制"]),
        (Self::Hexadecimal, &["hex", "hexadecimal", "十六进制"]),
    ];

    pub(super) fn parse(target: &str) -> Option<Self> {
        let target = target.trim().to_ascii_lowercase();
        Self::TARGETS
            .into_iter()
            .find_map(|(base, aliases)| aliases.contains(&target.as_str()).then_some(base))
    }

    pub(super) fn matching_targets(prefix: &str) -> impl Iterator<Item = (&'static str, Self)> {
        let prefix = prefix.to_ascii_lowercase();
        Self::TARGETS
            .into_iter()
            .filter_map(move |(base, aliases)| {
                aliases
                    .iter()
                    .any(|alias| alias.starts_with(&prefix))
                    .then_some((aliases[0], base))
            })
    }

    pub(super) fn prefixed(input: &str) -> Option<(Self, &str)> {
        let base = match input.as_bytes() {
            [b'0', b'b' | b'B', ..] => Self::Binary,
            [b'0', b'o' | b'O', ..] => Self::Octal,
            [b'0', b'x' | b'X', ..] => Self::Hexadecimal,
            _ => return None,
        };
        Some((base, &input[2..]))
    }

    pub(super) fn parse_digits(self, digits: &str) -> Result<u128, ConversionError> {
        if !digits
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            || digits.split('_').any(str::is_empty)
        {
            return Err(ConversionError::InvalidInteger);
        }
        u128::from_str_radix(&digits.replace('_', ""), self as u32).map_err(|error| {
            if *error.kind() == IntErrorKind::PosOverflow {
                ConversionError::OutOfRange
            } else {
                ConversionError::InvalidInteger
            }
        })
    }

    pub(super) fn convert(self, source: &str) -> Result<String, ConversionError> {
        let negative = source.starts_with('-');
        let magnitude = source.strip_prefix(['-', '+']).unwrap_or(source);
        let (base, digits) = Self::prefixed(magnitude).unwrap_or((Self::Decimal, magnitude));
        let value = base.parse_digits(digits)?;
        let sign = if negative && value != 0 { "-" } else { "" };
        Ok(match self {
            Self::Binary => format!("{sign}0b{value:b}"),
            Self::Octal => format!("{sign}0o{value:o}"),
            Self::Decimal => format!("{sign}{value}"),
            Self::Hexadecimal => format!("{sign}0x{value:x}"),
        })
    }
}
