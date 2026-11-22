use std::num::IntErrorKind;

use num_bigint::BigUint;

use super::calculator::normalize;
use super::conversion::ConversionError;

const MAX_INTEGER_BITS: u64 = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RadixError {
    Incomplete,
    Base,
    Digits,
    OutOfRange,
}

impl RadixError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入整数、原进制和目标进制，例如 ff 从 16 到 10",
            Self::Base => "原进制和目标进制须为 2 至 36",
            Self::Digits => "输入包含原进制不支持的字符",
            Self::OutOfRange => "整数的绝对值最多支持 8192 个二进制位",
        }
    }
}

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
        let digits = ungroup_digits(digits).ok_or(ConversionError::InvalidInteger)?;
        u128::from_str_radix(&digits, self as u32).map_err(|error| {
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

pub(super) fn convert_arbitrary_radix(input: &str) -> Result<String, RadixError> {
    let input = normalize(input);
    let tokens: Vec<_> = input.split_whitespace().take(6).collect();
    let [integer, from, source, to, target] = tokens.as_slice() else {
        return Err(RadixError::Incomplete);
    };
    if !((*from == "从" || from.eq_ignore_ascii_case("from"))
        && (*to == "到" || to.eq_ignore_ascii_case("to")))
    {
        return Err(RadixError::Incomplete);
    }
    let source = arbitrary_base(source).ok_or(RadixError::Base)?;
    let target = arbitrary_base(target).ok_or(RadixError::Base)?;
    let negative = integer.starts_with('-');
    let digits = integer.strip_prefix(['+', '-']).unwrap_or(integer);
    let digits = ungroup_digits(digits).ok_or(RadixError::Digits)?;
    let value = BigUint::parse_bytes(digits.as_bytes(), source).ok_or(RadixError::Digits)?;
    if value.bits() > MAX_INTEGER_BITS {
        return Err(RadixError::OutOfRange);
    }
    let mut text = value.to_str_radix(target);
    if negative && value.bits() > 0 {
        text.insert(0, '-');
    }
    Ok(text)
}

fn arbitrary_base(token: &str) -> Option<u32> {
    Base::parse(token)
        .map(|base| base as u32)
        .or_else(|| token.parse().ok().filter(|base| (2..=36).contains(base)))
}

pub(super) fn ungroup_digits(digits: &str) -> Option<String> {
    (digits
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        && !digits.split('_').any(str::is_empty))
    .then(|| digits.replace('_', ""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_signed_and_large_integers_between_arbitrary_bases() {
        for (input, expected) in [
            ("ff 从 16 到 10", "255"),
            ("2_147_483_647 从 10 到 36", "zik0zj"),
            ("ZIK0ZJ FROM 36 TO decimal", "2147483647"),
            ("−１_０１ 从 2 到 10", "-5"),
            ("-0000 from oct to bin", "0"),
            ("10201 from 3 to 7", "202"),
            (
                "340282366920938463463374607431768211456 from 10 to hex",
                "100000000000000000000000000000000",
            ),
        ] {
            assert_eq!(
                convert_arbitrary_radix(input).as_deref(),
                Ok(expected),
                "{input}"
            );
        }
    }

    #[test]
    fn rejects_out_of_range_bases_and_invalid_digits() {
        for (input, error) in [
            ("10 2 到 10", RadixError::Incomplete),
            ("1 from 1 to 10", RadixError::Base),
            ("1 from 2 to 37", RadixError::Base),
            ("2 from 2 to 10", RadixError::Digits),
            ("1__0 from 3 to 7", RadixError::Digits),
            ("+_1 from 10 to 36", RadixError::Digits),
            ("+ from 10 to 36", RadixError::Digits),
        ] {
            assert_eq!(convert_arbitrary_radix(input), Err(error), "{input}");
        }
        let excessive = format!("{} from 36 to 2", "z".repeat(1600));
        assert_eq!(
            convert_arbitrary_radix(&excessive),
            Err(RadixError::OutOfRange)
        );
    }
}
