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
    ByteOrderArguments,
    ByteOrderInteger,
    ByteOrderWidthExceeded,
    SignedByteOrderInteger,
    SignedByteOrderWidthExceeded,
    ReadByteOrderArguments,
    ReadByteOrderBytes,
    BitRangeArguments,
}

impl RadixError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入整数、原进制和目标进制，例如 ff 从 16 到 10",
            Self::Base => "原进制和目标进制须为 2 至 36",
            Self::Digits => "输入包含原进制不支持的字符",
            Self::OutOfRange => "整数的绝对值最多支持 8192 个二进制位",
            Self::ByteOrderArguments => {
                "填写位宽和整数，例如 字节序 32 @ 0x12345678。位宽为 8 至 128 的 8 倍数"
            }
            Self::ByteOrderInteger => "填写非负整数，可用十进制或 0x、0b、0o 前缀",
            Self::ByteOrderWidthExceeded => "整数超出所填位宽",
            Self::SignedByteOrderInteger => "填写有符号整数，可用十进制或 0x、0b、0o 前缀",
            Self::SignedByteOrderWidthExceeded => "有符号整数超出所填位宽",
            Self::ReadByteOrderArguments => "填写大端或小端，例如 读字节序 小端 @ 78 56 34 12",
            Self::ReadByteOrderBytes => "填写 1 至 16 个两位十六进制字节，字节之间用空格分开",
            Self::BitRangeArguments => {
                "填写位号范围和非负整数，例如 取位 8-15 @ 0x12345678。位号为 0 至 8191"
            }
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
    let (operation, arguments) = input
        .trim()
        .split_once(char::is_whitespace)
        .unwrap_or((input.trim(), ""));
    if operation == "字节序" || operation.eq_ignore_ascii_case("byte-order") {
        return byte_order(arguments, false);
    }
    if operation == "补码" || operation.eq_ignore_ascii_case("twos-complement") {
        return byte_order(arguments, true);
    }
    if operation == "读字节序" || operation.eq_ignore_ascii_case("read-byte-order") {
        return read_byte_order(arguments, false);
    }
    if operation == "读补码" || operation.eq_ignore_ascii_case("read-twos-complement") {
        return read_byte_order(arguments, true);
    }
    if operation == "位统计" || operation.eq_ignore_ascii_case("bit-statistics") {
        return bit_statistics(arguments);
    }
    if operation == "取位" || operation.eq_ignore_ascii_case("bit-range") {
        return bit_range(arguments);
    }
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
    let value = bounded_magnitude(digits, source)?;
    let mut text = value.to_str_radix(target);
    if negative && value.bits() > 0 {
        text.insert(0, '-');
    }
    Ok(text)
}

fn bit_statistics(input: &str) -> Result<String, RadixError> {
    let input = input.trim();
    let input = input.strip_prefix(['+', '-']).unwrap_or(input);
    let (base, digits) = Base::prefixed(input).unwrap_or((Base::Decimal, input));
    let value = bounded_magnitude(digits, base as u32)?;
    Ok(format!(
        "二进制位数 {}\n置位数 {}",
        value.bits(),
        value.count_ones()
    ))
}

fn bit_range(input: &str) -> Result<String, RadixError> {
    let (range, number) = input.split_once('@').ok_or(RadixError::BitRangeArguments)?;
    let (start, end) = range.split_once('-').unwrap_or((range, range));
    let position = |text: &str| {
        text.trim()
            .parse::<u64>()
            .ok()
            .filter(|&bit| bit < MAX_INTEGER_BITS)
            .ok_or(RadixError::BitRangeArguments)
    };
    let (start, end) = (position(start)?, position(end)?);
    let number = number.trim();
    if start > end || number.starts_with('-') {
        return Err(RadixError::BitRangeArguments);
    }
    let number = number.strip_prefix('+').unwrap_or(number);
    let (base, digits) = Base::prefixed(number).unwrap_or((Base::Decimal, number));
    let value = bounded_magnitude(digits, base as u32)?;
    let width = end - start + 1;
    let mask = (BigUint::from(1_u8) << width) - 1_u8;
    let field = (value >> start) & mask;
    let hex_digits = width.div_ceil(4) as usize;
    Ok(format!("十六进制 0x{field:0>hex_digits$X}\n十进制 {field}"))
}

fn bounded_magnitude(digits: &str, base: u32) -> Result<BigUint, RadixError> {
    let digits = ungroup_digits(digits).ok_or(RadixError::Digits)?;
    let value = BigUint::parse_bytes(digits.as_bytes(), base).ok_or(RadixError::Digits)?;
    if value.bits() > MAX_INTEGER_BITS {
        return Err(RadixError::OutOfRange);
    }
    Ok(value)
}

fn byte_order(input: &str, signed: bool) -> Result<String, RadixError> {
    let (bits, number) = input
        .split_once('@')
        .ok_or(RadixError::ByteOrderArguments)?;
    let bits = bits
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|bits| (8..=128).contains(bits) && bits.is_multiple_of(8))
        .ok_or(RadixError::ByteOrderArguments)?;
    let number = number.trim();
    let negative = number.starts_with('-');
    if negative && !signed {
        return Err(RadixError::ByteOrderInteger);
    }
    let (integer_error, width_error) = if signed {
        (
            RadixError::SignedByteOrderInteger,
            RadixError::SignedByteOrderWidthExceeded,
        )
    } else {
        (
            RadixError::ByteOrderInteger,
            RadixError::ByteOrderWidthExceeded,
        )
    };
    let number = number.strip_prefix(['+', '-']).unwrap_or(number);
    let (base, digits) = Base::prefixed(number).unwrap_or((Base::Decimal, number));
    let value = base.parse_digits(digits).map_err(|error| match error {
        ConversionError::OutOfRange => width_error,
        _ => integer_error,
    })?;
    let fits = if signed {
        let sign_bit = 1_u128 << (bits - 1);
        let limit = if negative { sign_bit } else { sign_bit - 1 };
        value <= limit
    } else {
        u128::BITS - value.leading_zeros() <= bits
    };
    if !fits {
        return Err(width_error);
    }
    let value = if negative {
        0_u128.wrapping_sub(value)
    } else {
        value
    };
    let bytes = value.to_be_bytes();
    let bytes = &bytes[bytes.len() - (bits / 8) as usize..];
    let big_endian = bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>();
    let little_endian = big_endian
        .iter()
        .rev()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" ");
    Ok(format!(
        "大端 {}\n小端 {little_endian}",
        big_endian.join(" ")
    ))
}

fn read_byte_order(input: &str, signed: bool) -> Result<String, RadixError> {
    let (order, bytes) = input
        .split_once('@')
        .ok_or(RadixError::ReadByteOrderArguments)?;
    let little_endian = match order.trim().to_ascii_lowercase().as_str() {
        "小端" | "le" => true,
        "大端" | "be" => false,
        _ => return Err(RadixError::ReadByteOrderArguments),
    };
    let bytes: Vec<_> = bytes.split_whitespace().take(17).collect();
    if !(1..=16).contains(&bytes.len()) {
        return Err(RadixError::ReadByteOrderBytes);
    }
    let mut value = 0_u128;
    for (index, token) in bytes.iter().enumerate() {
        if token.len() != 2 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(RadixError::ReadByteOrderBytes);
        }
        let byte = u8::from_str_radix(token, 16).map_err(|_| RadixError::ReadByteOrderBytes)?;
        let offset = if little_endian {
            index
        } else {
            bytes.len() - 1 - index
        };
        value |= u128::from(byte) << (8 * offset);
    }
    let width = bytes.len() * 2;
    let decimal = if signed {
        // 将最高有效位移到 i128 的符号位，再以算术右移补齐符号位。
        let shift = (16 - bytes.len()) * 8;
        (((value << shift) as i128) >> shift).to_string()
    } else {
        value.to_string()
    };
    Ok(format!("十六进制 0x{value:0width$X}\n十进制 {decimal}"))
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
    fn extracts_inclusive_integer_bit_fields_and_pads_to_hex_width() {
        for (query, expected) in [
            ("取位 8-15 @ 0x12345678", "十六进制 0x56\n十进制 86"),
            ("取位 0-11 @ 0b11", "十六进制 0x003\n十进制 3"),
            ("bit-range 4 @ +16", "十六进制 0x1\n十进制 1"),
            ("取位 １４－１５ ＠ １", "十六进制 0x0\n十进制 0"),
        ] {
            assert_eq!(convert_arbitrary_radix(query).as_deref(), Ok(expected));
        }
        let one_at_top = format!("取位 8191 @ 0x8{}", "0".repeat(2047));
        assert_eq!(
            convert_arbitrary_radix(&one_at_top).as_deref(),
            Ok("十六进制 0x1\n十进制 1")
        );
        for query in [
            "取位 8-3 @ 0x41",
            "取位 -1 @ 1",
            "取位 8192 @ 1",
            "取位 3 @ -1",
        ] {
            assert_eq!(
                convert_arbitrary_radix(query),
                Err(RadixError::BitRangeArguments)
            );
        }
    }

    #[test]
    fn counts_absolute_integer_bits_and_enforces_the_existing_radix_limit() {
        for (query, expected) in [
            ("位统计 -0xFF00", "二进制位数 16\n置位数 8"),
            ("BIT-STATISTICS +0b10_1101", "二进制位数 6\n置位数 4"),
            ("位统计 0", "二进制位数 0\n置位数 0"),
        ] {
            assert_eq!(
                convert_arbitrary_radix(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        assert_eq!(
            convert_arbitrary_radix(&format!("位统计 0x{}", "f".repeat(2048))).as_deref(),
            Ok("二进制位数 8192\n置位数 8192"),
        );
        assert_eq!(
            convert_arbitrary_radix(&format!("位统计 0x{}", "f".repeat(2049))),
            Err(RadixError::OutOfRange),
        );
        assert_eq!(
            convert_arbitrary_radix("位统计 0b2"),
            Err(RadixError::Digits)
        );
    }

    #[test]
    fn encodes_signed_twos_complement_without_overflow_or_truncation() {
        for (query, expected) in [
            ("补码 16 @ -257", "大端 FE FF\n小端 FF FE"),
            ("TWOS-COMPLEMENT 8 @ -0x80", "大端 80\n小端 80"),
            ("补码 8 @ +127", "大端 7F\n小端 7F"),
            ("补码 8 @ -0", "大端 00\n小端 00"),
        ] {
            assert_eq!(
                convert_arbitrary_radix(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        for query in [
            "补码 8 @ -129",
            "补码 8 @ 128",
            "补码 128 @ 0x8000_0000_0000_0000_0000_0000_0000_0000",
        ] {
            assert_eq!(
                convert_arbitrary_radix(query),
                Err(RadixError::SignedByteOrderWidthExceeded),
                "{query}"
            );
        }
        let minimum = convert_arbitrary_radix(&format!("补码 128 @ {}", i128::MIN)).unwrap();
        assert_eq!(
            minimum,
            format!("大端 80{}\n小端{} 80", " 00".repeat(15), " 00".repeat(15))
        );
    }

    #[test]
    fn shows_both_byte_orders_with_exact_width_and_padding() {
        for (query, expected) in [
            (
                "字节序 32 @ 0x12345678",
                "大端 12 34 56 78\n小端 78 56 34 12",
            ),
            ("BYTE-ORDER 24 @ 258", "大端 00 01 02\n小端 02 01 00"),
            ("字节序 ８ ＠ ０ｂ１", "大端 01\n小端 01"),
        ] {
            assert_eq!(
                convert_arbitrary_radix(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        for (query, error) in [
            ("字节序 8 @ 256", RadixError::ByteOrderWidthExceeded),
            ("字节序 9 @ 1", RadixError::ByteOrderArguments),
            ("字节序 16 @ -1", RadixError::ByteOrderInteger),
            (
                "字节序 128 @ 0x1_00000000_00000000_00000000_00000000",
                RadixError::ByteOrderWidthExceeded,
            ),
        ] {
            assert_eq!(convert_arbitrary_radix(query), Err(error), "{query}");
        }
    }

    #[test]
    fn decodes_signed_twos_complement_using_the_number_of_supplied_bytes() {
        for (query, expected) in [
            ("读补码 小端 @ FF FE", "十六进制 0xFEFF\n十进制 -257"),
            ("read-twos-complement BE @ 80", "十六进制 0x80\n十进制 -128"),
            ("读补码 大端 @ 00 80", "十六进制 0x0080\n十进制 128"),
            ("读补码 大端 @ FF", "十六进制 0xFF\n十进制 -1"),
            ("读字节序 小端 @ FF FE", "十六进制 0xFEFF\n十进制 65279"),
        ] {
            assert_eq!(
                convert_arbitrary_radix(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        let query = format!("读补码 大端 @ 80{}", " 00".repeat(15));
        let expected = format!("十六进制 0x80{}\n十进制 {}", "00".repeat(15), i128::MIN);
        assert_eq!(
            convert_arbitrary_radix(&query).as_deref(),
            Ok(expected.as_str())
        );
    }

    #[test]
    fn reads_bytes_in_either_order_and_preserves_hexadecimal_width() {
        for (query, expected) in [
            ("读字节序 小端 @ 78 56 34 12", "十六进制 0x12345678\n十进制 305419896"),
            ("read-byte-order BE @ 00 ab Cd", "十六进制 0x00ABCD\n十进制 43981"),
            ("读字节序 大端 ＠ ＦＦ", "十六进制 0xFF\n十进制 255"),
            ("读字节序 小端 @ FF FF FF FF FF FF FF FF FF FF FF FF FF FF FF FF",
             "十六进制 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF\n十进制 340282366920938463463374607431768211455"),
        ] {
            assert_eq!(convert_arbitrary_radix(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            ("读字节序 @ 10", RadixError::ReadByteOrderArguments),
            ("读字节序 大端 @", RadixError::ReadByteOrderBytes),
            ("读字节序 大端 @ 1", RadixError::ReadByteOrderBytes),
            ("读字节序 大端 @ +F", RadixError::ReadByteOrderBytes),
            ("读字节序 大端 @ GG", RadixError::ReadByteOrderBytes),
        ] {
            assert_eq!(convert_arbitrary_radix(query), Err(error), "{query}");
        }
        assert_eq!(
            convert_arbitrary_radix(&format!("读字节序 小端 @ {}", " 00".repeat(17))),
            Err(RadixError::ReadByteOrderBytes),
        );
    }

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
