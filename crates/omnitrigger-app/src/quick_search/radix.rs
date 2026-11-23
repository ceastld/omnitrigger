use std::num::IntErrorKind;

use num_bigint::{BigInt, BigUint, Sign};
use num_rational::BigRational;

use super::calculator::normalize;
use super::conversion::ConversionError;
use super::fraction::{repeating_radix, MAX_REPEATING_DECIMAL_PLACES};

const MAX_INTEGER_BITS: u64 = 8192;
const MAX_SHEET_COLUMN: u32 = 16_384;
const MAX_ROMAN_VALUE: u16 = 3999;
const ROMAN_DIGITS: [(u16, &str); 13] = [
    (1000, "M"),
    (900, "CM"),
    (500, "D"),
    (400, "CD"),
    (100, "C"),
    (90, "XC"),
    (50, "L"),
    (40, "XL"),
    (10, "X"),
    (9, "IX"),
    (5, "V"),
    (4, "IV"),
    (1, "I"),
];

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
    WriteBitsArguments,
    WriteBitsWidthExceeded,
    RotateBitsArguments,
    RotateBitsWidthExceeded,
    ReverseBitsArguments,
    ReverseBytesArguments,
    ReverseWidthExceeded,
    SheetColumnArguments,
    RomanArguments,
    RomanRange,
    FractionalArguments,
    FractionalLength,
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
            Self::ByteOrderWidthExceeded
            | Self::RotateBitsWidthExceeded
            | Self::ReverseWidthExceeded => "整数超出所填位宽",
            Self::SignedByteOrderInteger => "填写有符号整数，可用十进制或 0x、0b、0o 前缀",
            Self::SignedByteOrderWidthExceeded => "有符号整数超出所填位宽",
            Self::ReadByteOrderArguments => "填写大端或小端，例如 读字节序 小端 @ 78 56 34 12",
            Self::ReadByteOrderBytes => "填写 1 至 16 个两位十六进制字节，字节之间用空格分开",
            Self::BitRangeArguments => {
                "填写位号范围和非负整数，例如 取位 8-15 @ 0x12345678。位号为 0 至 8191"
            }
            Self::WriteBitsArguments => {
                "填写位号范围、原整数和新值，例如 写位 8-15 @ 0x12345678 @ 0xAB。位号为 0 至 8191"
            }
            Self::WriteBitsWidthExceeded => "新值超出所选位宽",
            Self::RotateBitsArguments => "填写位宽、移动位数和非负整数，例如 循环左移 8,3 @ 0xA5",
            Self::ReverseBitsArguments => "填写 1 至 8192 的位宽和非负整数，例如 位倒序 8 @ 0x06",
            Self::ReverseBytesArguments => {
                "填写 8 至 8192 的 8 倍数位宽和非负整数，例如 字节倒序 32 @ 0x12345678"
            }
            Self::SheetColumnArguments => "填写 A 至 XFD 的列字母，或 1 至 16384 的整数列号",
            Self::RomanArguments => "填写常用写法的罗马数字，例如 IV、CDXCIX 或 MMXXVI",
            Self::RomanRange => "填写 1 至 3999 的十进制整数，例如 2026",
            Self::FractionalArguments => "填写小数、原进制和目标进制，例如 10.625 从 10 到 2",
            Self::FractionalLength => "原小数或结果超过 512 位，请缩短输入或选择其他目标进制",
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
    if operation == "写位" || operation.eq_ignore_ascii_case("write-bits") {
        return write_bits(arguments);
    }
    if operation == "循环左移" || operation.eq_ignore_ascii_case("rotate-left") {
        return rotate_bits(arguments, false);
    }
    if operation == "循环右移" || operation.eq_ignore_ascii_case("rotate-right") {
        return rotate_bits(arguments, true);
    }
    if operation == "位倒序" || operation.eq_ignore_ascii_case("reverse-bits") {
        return reverse_bits_or_bytes(arguments, false);
    }
    if operation == "字节倒序" || operation.eq_ignore_ascii_case("reverse-bytes") {
        return reverse_bits_or_bytes(arguments, true);
    }
    if operation == "表格列号" || operation.eq_ignore_ascii_case("sheet-column") {
        return convert_sheet_column(arguments);
    }
    if operation == "罗马数字" || operation.eq_ignore_ascii_case("roman") {
        return convert_roman(arguments);
    }
    let fractional = operation == "小数" || operation.eq_ignore_ascii_case("fractional");
    let (expression, missing) = if fractional {
        (arguments, RadixError::FractionalArguments)
    } else {
        (input.as_str(), RadixError::Incomplete)
    };
    let tokens: Vec<_> = expression.split_whitespace().take(6).collect();
    let [number, from, source, to, target] = tokens.as_slice() else {
        return Err(missing);
    };
    if !((*from == "从" || from.eq_ignore_ascii_case("from"))
        && (*to == "到" || to.eq_ignore_ascii_case("to")))
    {
        return Err(missing);
    }
    let source = arbitrary_base(source).ok_or(RadixError::Base)?;
    let target = arbitrary_base(target).ok_or(RadixError::Base)?;
    if fractional {
        convert_radix_fraction(number, source, target)
    } else {
        convert_radix_integer(number, source, target)
    }
}

fn convert_radix_fraction(number: &str, source: u32, target: u32) -> Result<String, RadixError> {
    let sign = if number.starts_with('-') {
        Sign::Minus
    } else {
        Sign::Plus
    };
    let number = number.strip_prefix(['+', '-']).unwrap_or(number);
    let (whole, fractional) = number
        .split_once('.')
        .ok_or(RadixError::FractionalArguments)?;
    let whole = if whole.is_empty() {
        BigUint::default()
    } else {
        bounded_magnitude(whole, source)?
    };
    let (fractional, repeating) = if let Some((fixed, repeating)) = fractional.split_once('(') {
        let repeating = repeating.strip_suffix(')').ok_or(RadixError::Digits)?;
        let repeating = ungroup_digits(repeating).ok_or(RadixError::Digits)?;
        (fixed, Some(repeating))
    } else {
        (fractional, None)
    };
    let fractional = if fractional.is_empty() && repeating.is_some() {
        String::new()
    } else {
        ungroup_digits(fractional).ok_or(RadixError::Digits)?
    };
    let places = fractional.len() + repeating.as_ref().map_or(0, String::len);
    if places > MAX_REPEATING_DECIMAL_PLACES {
        return Err(RadixError::FractionalLength);
    }
    let divisor = BigUint::from(source).pow(fractional.len() as u32);
    let remainder = if fractional.is_empty() {
        BigUint::default()
    } else {
        BigUint::parse_bytes(fractional.as_bytes(), source).ok_or(RadixError::Digits)?
    };
    let whole = whole * &divisor + remainder;
    let (numerator, divisor) = if let Some(repeating) = repeating {
        let cycle_divisor = BigUint::from(source).pow(repeating.len() as u32) - 1_u8;
        let cycle = BigUint::parse_bytes(repeating.as_bytes(), source).ok_or(RadixError::Digits)?;
        (whole * &cycle_divisor + cycle, divisor * cycle_divisor)
    } else {
        (whole, divisor)
    };
    let numerator = BigInt::from_biguint(sign, numerator);
    let value = BigRational::new(numerator, divisor.into());
    repeating_radix(&value, target).map_err(|_| RadixError::FractionalLength)
}

fn convert_roman(input: &str) -> Result<String, RadixError> {
    let input = input.trim();
    let decimal_digits = input.strip_prefix(['+', '-']).unwrap_or(input);
    if !decimal_digits.is_empty() && decimal_digits.bytes().all(|byte| byte.is_ascii_digit()) {
        let value = input
            .parse::<u16>()
            .ok()
            .filter(|value| (1..=MAX_ROMAN_VALUE).contains(value))
            .ok_or(RadixError::RomanRange)?;
        return Ok(format_roman(value));
    }
    if input.is_empty() || input.len() > 15 {
        return Err(RadixError::RomanArguments);
    }
    let roman = input.to_ascii_uppercase();
    let mut rest = roman.as_str();
    let mut value = 0;
    for (digit, symbol) in ROMAN_DIGITS {
        while let Some(suffix) = rest.strip_prefix(symbol) {
            value += digit;
            rest = suffix;
        }
    }
    if rest.is_empty() && value <= MAX_ROMAN_VALUE && format_roman(value) == roman {
        return Ok(value.to_string());
    }
    Err(RadixError::RomanArguments)
}

fn format_roman(mut value: u16) -> String {
    let mut roman = String::new();
    for (digit, symbol) in ROMAN_DIGITS {
        while value >= digit {
            roman.push_str(symbol);
            value -= digit;
        }
    }
    roman
}

fn convert_sheet_column(input: &str) -> Result<String, RadixError> {
    let input = input.trim();
    if (1..=3).contains(&input.len()) && input.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        let column = input.bytes().fold(0_u32, |column, letter| {
            column * 26 + u32::from(letter.to_ascii_uppercase() - b'A' + 1)
        });
        return (column <= MAX_SHEET_COLUMN)
            .then(|| column.to_string())
            .ok_or(RadixError::SheetColumnArguments);
    }
    let mut column = input
        .parse::<u32>()
        .ok()
        .filter(|column| (1..=MAX_SHEET_COLUMN).contains(column))
        .ok_or(RadixError::SheetColumnArguments)?;
    let mut letters = Vec::with_capacity(3);
    while column > 0 {
        column -= 1;
        letters.push(char::from(b'A' + (column % 26) as u8));
        column /= 26;
    }
    Ok(letters.into_iter().rev().collect())
}

pub(super) fn convert_radix_integer(
    integer: &str,
    source: u32,
    target: u32,
) -> Result<String, RadixError> {
    if [source, target].iter().any(|base| !(2..=36).contains(base)) {
        return Err(RadixError::Base);
    }
    let integer = integer.trim();
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
    let (start, width) = bit_field_range(range, RadixError::BitRangeArguments)?;
    let value = unsigned_bit_value(number, RadixError::BitRangeArguments)?;
    let field = selected_bits(&value, start, width);
    Ok(format_padded_bit_value(&field, width))
}

fn write_bits(input: &str) -> Result<String, RadixError> {
    let error = RadixError::WriteBitsArguments;
    let (range, values) = input.split_once('@').ok_or(error)?;
    let (number, replacement) = values.split_once('@').ok_or(error)?;
    let (start, width) = bit_field_range(range, error)?;
    let value = unsigned_bit_value(number, error)?;
    let replacement = unsigned_bit_value(replacement, error)?;
    if replacement.bits() > width {
        return Err(RadixError::WriteBitsWidthExceeded);
    }
    let field = selected_bits(&value, start, width);
    let cleared = value - (field << start);
    let result = cleared | (replacement << start);
    Ok(format!("十六进制 0x{result:X}\n十进制 {result}"))
}

fn rotate_bits(input: &str, right: bool) -> Result<String, RadixError> {
    let error = RadixError::RotateBitsArguments;
    let (settings, number) = input.split_once('@').ok_or(error)?;
    let (width, count) = settings.split_once(',').ok_or(error)?;
    let width = bit_width(width, error)?;
    let shift = count.trim().parse::<u64>().map_err(|_| error)? % width;
    let value = unsigned_bit_value(number, error)?;
    if value.bits() > width {
        return Err(RadixError::RotateBitsWidthExceeded);
    }
    let shift = if right { width - shift } else { shift };
    let rotated = ((&value << shift) | (&value >> (width - shift))) & bit_mask(width);
    Ok(format_padded_bit_value(&rotated, width))
}

fn reverse_bits_or_bytes(input: &str, by_byte: bool) -> Result<String, RadixError> {
    let error = if by_byte {
        RadixError::ReverseBytesArguments
    } else {
        RadixError::ReverseBitsArguments
    };
    let (width, number) = input.split_once('@').ok_or(error)?;
    let width = bit_width(width, error)?;
    if by_byte && !width.is_multiple_of(8) {
        return Err(error);
    }
    let value = unsigned_bit_value(number, error)?;
    if value.bits() > width {
        return Err(RadixError::ReverseWidthExceeded);
    }
    let bytes = width.div_ceil(8) as usize;
    let mut data = value.to_bytes_le();
    data.resize(bytes, 0);
    if !by_byte {
        for byte in &mut data {
            *byte = byte.reverse_bits();
        }
    }
    // 小端字节按大端读取，逐位反转时移除落到低位的补齐位。
    let reversed = BigUint::from_bytes_be(&data) >> (bytes * 8 - width as usize);
    Ok(format_padded_bit_value(&reversed, width))
}

fn bit_width(input: &str, error: RadixError) -> Result<u64, RadixError> {
    input
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|width| (1..=MAX_INTEGER_BITS).contains(width))
        .ok_or(error)
}

fn format_padded_bit_value(value: &BigUint, width: u64) -> String {
    let hex_digits = width.div_ceil(4) as usize;
    format!("十六进制 0x{value:0>hex_digits$X}\n十进制 {value}")
}

fn bit_field_range(range: &str, error: RadixError) -> Result<(u64, u64), RadixError> {
    let (start, end) = range.split_once('-').unwrap_or((range, range));
    let position = |text: &str| {
        text.trim()
            .parse::<u64>()
            .ok()
            .filter(|&bit| bit < MAX_INTEGER_BITS)
            .ok_or(error)
    };
    let (start, end) = (position(start)?, position(end)?);
    if start > end {
        return Err(error);
    }
    Ok((start, end - start + 1))
}

fn unsigned_bit_value(number: &str, error: RadixError) -> Result<BigUint, RadixError> {
    let number = number.trim();
    if number.starts_with('-') {
        return Err(error);
    }
    let number = number.strip_prefix('+').unwrap_or(number);
    let (base, digits) = Base::prefixed(number).unwrap_or((Base::Decimal, number));
    bounded_magnitude(digits, base as u32)
}

fn selected_bits(value: &BigUint, start: u64, width: u64) -> BigUint {
    (value >> start) & bit_mask(width)
}

fn bit_mask(width: u64) -> BigUint {
    (BigUint::from(1_u8) << width) - 1_u8
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
    fn reverses_bits_or_bytes_within_the_explicit_width() {
        for (query, expected) in [
            ("位倒序 8 @ 0x06", "十六进制 0x60\n十进制 96"),
            ("reverse-bits 5 @ 0b00110", "十六进制 0x0C\n十进制 12"),
            ("位倒序 5 @ 0x0C", "十六进制 0x06\n十进制 6"),
            ("位倒序 1 @ 1", "十六进制 0x1\n十进制 1"),
            (
                "字节倒序 32 @ 0x12345678",
                "十六进制 0x78563412\n十进制 2018915346",
            ),
            ("reverse-bytes 16 @ 1", "十六进制 0x0100\n十进制 256"),
        ] {
            assert_eq!(
                convert_arbitrary_radix(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        for (query, error) in [
            ("位倒序 4 @ 16", RadixError::ReverseWidthExceeded),
            ("字节倒序 8 @ 256", RadixError::ReverseWidthExceeded),
            ("字节倒序 7 @ 1", RadixError::ReverseBytesArguments),
            ("位倒序 0 @ 0", RadixError::ReverseBitsArguments),
            ("位倒序 8193 @ 0", RadixError::ReverseBitsArguments),
            ("位倒序 8 @ -1", RadixError::ReverseBitsArguments),
        ] {
            assert_eq!(convert_arbitrary_radix(query), Err(error), "{query}");
        }
    }

    #[test]
    fn rotates_within_the_requested_width_and_reduces_the_shift_count() {
        for (query, expected) in [
            ("循环左移 8,3 @ 0xA5", "十六进制 0x2D\n十进制 45"),
            ("循环右移 8,3 @ 0xA5", "十六进制 0xB4\n十进制 180"),
            ("rotate-right 8,9 @ 1", "十六进制 0x80\n十进制 128"),
            ("rotate-left 12,12 @ 1", "十六进制 0x001\n十进制 1"),
            ("循环右移 1,100 @ 1", "十六进制 0x1\n十进制 1"),
        ] {
            assert_eq!(convert_arbitrary_radix(query).as_deref(), Ok(expected));
        }
        assert_eq!(
            convert_arbitrary_radix("循环左移 5,1 @ 32"),
            Err(RadixError::RotateBitsWidthExceeded)
        );
        for query in [
            "循环左移 0,1 @ 0",
            "循环左移 8,-1 @ 0",
            "循环右移 8193,1 @ 0",
        ] {
            assert_eq!(
                convert_arbitrary_radix(query),
                Err(RadixError::RotateBitsArguments)
            );
        }
    }

    #[test]
    fn writes_bit_ranges_without_changing_surrounding_bits() {
        for (query, expected) in [
            (
                "写位 8-15 @ 0x12345678 @ 0xAB",
                "十六进制 0x1234AB78\n十进制 305441656",
            ),
            (
                "write-bits 2-5 @ 0b11100011 @ 0",
                "十六进制 0xC3\n十进制 195",
            ),
            ("写位 4 @ 0b11111 @ 0", "十六进制 0xF\n十进制 15"),
            ("写位 0 @ 0 @ 1", "十六进制 0x1\n十进制 1"),
        ] {
            assert_eq!(convert_arbitrary_radix(query).as_deref(), Ok(expected));
        }
        assert_eq!(
            convert_arbitrary_radix("写位 8-15 @ 0x12345678 @ 256"),
            Err(RadixError::WriteBitsWidthExceeded)
        );
        for query in [
            "写位 2-1 @ 0 @ 0",
            "写位 0 @ -1 @ 0",
            "写位 0 @ 0 @ -1",
            "写位 0 @ 0",
        ] {
            assert_eq!(
                convert_arbitrary_radix(query),
                Err(RadixError::WriteBitsArguments)
            );
        }
    }

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
    fn converts_finite_and_repeating_fractional_values_across_radices() {
        for (query, expected) in [
            ("小数 10.625 从 10 到 2", "1010.101"),
            ("fractional -A.A from hex to decimal", "-10.625"),
            ("小数 0.1 从 10 到 2", "0.0(0011)"),
            ("小数 1.1 从 3 到 10", "1.(3)"),
            ("小数 .01_1 从 2 到 10", "0.375"),
            ("小数 0.z 从 36 到 36", "0.z"),
            ("小数 -0.0 从 10 到 2", "0"),
            ("小数 0.0(0011) 从 2 到 10", "0.1"),
            ("小数 0.(3) 从 10 到 3", "0.1"),
            ("小数 0.(9) 从 10 到 2", "1"),
            ("小数 -A.(8) 从 16 到 10", "-10.5(3)"),
            ("小数 .(01_01) 从 2 到 10", "0.(3)"),
        ] {
            assert_eq!(
                convert_arbitrary_radix(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        for (query, error) in [
            ("小数 1 从 10 到 2", RadixError::FractionalArguments),
            ("小数 0.2 从 2 到 10", RadixError::Digits),
            ("小数 0..1 从 10 到 2", RadixError::Digits),
            ("小数 0.00001 从 10 到 2", RadixError::FractionalLength),
            ("小数 0.1() 从 10 到 2", RadixError::Digits),
            ("小数 0.(2) 从 2 到 10", RadixError::Digits),
            ("小数 0.(01 从 2 到 10", RadixError::Digits),
        ] {
            assert_eq!(convert_arbitrary_radix(query), Err(error), "{query}");
        }
    }

    #[test]
    fn converts_classic_roman_numerals_in_both_directions() {
        for (query, expected) in [
            ("罗马数字 1", "I"),
            ("罗马数字 2026", "MMXXVI"),
            ("ROMAN cdxCix", "499"),
            ("roman 3999", "MMMCMXCIX"),
            ("roman MMMDCCCLXXXVIII", "3888"),
        ] {
            assert_eq!(convert_arbitrary_radix(query).unwrap(), expected);
        }
        for value in 1..=MAX_ROMAN_VALUE {
            let roman = format_roman(value);
            assert_eq!(convert_roman(&roman).unwrap(), value.to_string());
        }
        for invalid in ["", "IIII", "IC", "VIV", "MMMM", "IV IV", "VX", "Ⅱ"] {
            assert_eq!(convert_roman(invalid), Err(RadixError::RomanArguments));
        }
        for invalid in ["0", "-1", "4000", "70000"] {
            assert_eq!(convert_roman(invalid), Err(RadixError::RomanRange));
        }
    }

    #[test]
    fn converts_sheet_column_labels_and_numbers_at_carry_boundaries() {
        for (query, expected) in [
            ("表格列号 A", "1"),
            ("表格列号 26", "Z"),
            ("表格列号 27", "AA"),
            ("表格列号 ｚｚ", "702"),
            ("SHEET-COLUMN 703", "AAA"),
            ("表格列号 xfd", "16384"),
            ("表格列号 16384", "XFD"),
        ] {
            assert_eq!(
                convert_arbitrary_radix(query).as_deref(),
                Ok(expected),
                "{query}"
            );
        }
        for input in ["", "0", "-1", "16385", "XFE", "AB1", "AAAA", "a b"] {
            assert_eq!(
                convert_arbitrary_radix(&format!("表格列号 {input}")),
                Err(RadixError::SheetColumnArguments),
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
