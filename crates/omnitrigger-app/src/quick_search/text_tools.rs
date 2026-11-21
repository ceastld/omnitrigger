use std::{
    cmp::Reverse,
    collections::{HashMap, HashSet},
    fmt::Write as _,
};

use base64::{
    engine::general_purpose::{
        GeneralPurpose, STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD,
    },
    Engine,
};
use sha2::{Digest, Sha256, Sha512};
use unicode_segmentation::UnicodeSegmentation;

const MAX_REPLACEMENT_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TextError {
    MissingText,
    UnclosedQuote,
    IncompleteTransform,
    UnknownOperation,
    IncompleteReplacement,
    InvalidReplacementArguments,
    EmptySearchText,
    ReplacementTooLarge,
    NullCharacter,
    InvalidBase64,
    InvalidHexBytes,
    InvalidUtf8,
    InvalidCodePoint,
    InvalidJsonString,
}

impl TextError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::MissingText => "输入要统计的文字",
            Self::UnclosedQuote => "补全引号后继续",
            Self::IncompleteTransform => "输入操作和文本，例如 文本: 大写 hello",
            Self::UnknownOperation => "输入 ? 文本 查看可用操作",
            Self::IncompleteReplacement => "输入查找内容、替换内容和原文",
            Self::InvalidReplacementArguments => "查找内容和替换内容要加引号，并用空格分隔",
            Self::EmptySearchText => "请输入要查找的文字",
            Self::ReplacementTooLarge => "替换结果超过 1 MiB",
            Self::NullCharacter => "结果含空字符，无法完整复制",
            Self::InvalidBase64 => "Base64 编码无效",
            Self::InvalidHexBytes => "输入成对的十六进制数字，例如 E5 8C 97",
            Self::InvalidUtf8 => "解码结果不是 UTF-8 文本",
            Self::InvalidCodePoint => "输入以空格分隔的 Unicode 码位，例如 U+5317 U+4EAC",
            Self::InvalidJsonString => "请输入带双引号的 JSON 字符串",
        }
    }
}

fn text_payload(input: &str) -> Result<&str, TextError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(TextError::MissingText);
    }
    match input.chars().next() {
        Some('"') => input[1..].strip_suffix('"').ok_or(TextError::UnclosedQuote),
        Some('“') => input['“'.len_utf8()..]
            .strip_suffix('”')
            .ok_or(TextError::UnclosedQuote),
        _ => Ok(input),
    }
}

pub(super) fn count_text(input: &str) -> Result<String, TextError> {
    let text = text_payload(input)?;
    let characters = text.graphemes(true).count();
    Ok(format!("{characters} 字符 · {} UTF-8 字节", text.len()))
}

pub(super) fn transform_text(input: &str) -> Result<String, TextError> {
    let (operation, text) = input
        .split_once(char::is_whitespace)
        .ok_or(TextError::IncompleteTransform)?;
    let operation = operation.to_ascii_lowercase();
    match operation.as_str() {
        "替换" | "replace" => return replace_text(text),
        "json还原" | "json-unquote" => return decode_json_string(text),
        _ => {}
    }
    let text = text_payload(text)?;
    match operation.as_str() {
        "大写" | "upper" | "uppercase" => Ok(text.to_uppercase()),
        "小写" | "lower" | "lowercase" => Ok(text.to_lowercase()),
        "合并空白" | "collapse" => Ok(text.split_whitespace().collect::<Vec<_>>().join(" ")),
        "倒序" | "reverse" => Ok(text.graphemes(true).rev().collect()),
        "按行去重" | "unique-lines" => {
            let mut seen = HashSet::new();
            Ok(text
                .lines()
                .filter(|&line| seen.insert(line))
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "按行排序" | "sort-lines" => {
            let mut lines: Vec<_> = text.lines().collect();
            lines.sort_unstable();
            Ok(lines.join("\n"))
        }
        "去空行" | "remove-empty-lines" => Ok(text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n")),
        "按行编号" | "number-lines" => Ok(text
            .lines()
            .enumerate()
            .map(|(index, line)| format!("{}\t{line}", index + 1))
            .collect::<Vec<_>>()
            .join("\n")),
        "按行计数" | "line-counts" => Ok(count_lines(text)),
        "sha256" | "sha-256" => Ok(format!("{:x}", Sha256::digest(text.as_bytes()))),
        "sha512" | "sha-512" => Ok(format!("{:x}", Sha512::digest(text.as_bytes()))),
        "base64编码" | "base64-encode" => Ok(STANDARD.encode(text)),
        "base64解码" | "base64-decode" => decode_base64(text, &STANDARD, &STANDARD_NO_PAD),
        "base64url编码" | "base64url-encode" => Ok(URL_SAFE_NO_PAD.encode(text)),
        "base64url解码" | "base64url-decode" => decode_base64(text, &URL_SAFE, &URL_SAFE_NO_PAD),
        "utf8编码" | "utf8-hex" => Ok(encode_utf8_bytes(text)),
        "utf8解码" | "utf8-hex-decode" => decode_utf8_bytes(text),
        "码位" | "codepoints" => Ok(encode_code_points(text)),
        "码位解码" | "codepoints-decode" => decode_code_points(text),
        "json转义" | "json-quote" => {
            Ok(serde_json::to_string(text).expect("字符串可以序列化为 JSON"))
        }
        _ => Err(TextError::UnknownOperation),
    }
}

fn count_lines(text: &str) -> String {
    let mut indices = HashMap::new();
    let mut counts = Vec::new();
    for line in text.lines() {
        let index = *indices.entry(line).or_insert_with(|| {
            counts.push((line, 0usize));
            counts.len() - 1
        });
        counts[index].1 += 1;
    }
    counts.sort_by_key(|&(_, count)| Reverse(count));
    counts
        .into_iter()
        .map(|(line, count)| format!("{count}\t{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn encode_utf8_bytes(text: &str) -> String {
    let mut encoded = String::new();
    for byte in text.bytes() {
        if !encoded.is_empty() {
            encoded.push(' ');
        }
        write!(encoded, "{byte:02X}").expect("写入 String 不会出错");
    }
    encoded
}

fn decode_utf8_bytes(text: &str) -> Result<String, TextError> {
    let mut bytes = Vec::with_capacity(text.len() / 2);
    for group in text.split_whitespace() {
        let digits = group
            .strip_prefix("0x")
            .or_else(|| group.strip_prefix("0X"))
            .unwrap_or(group);
        if digits.is_empty()
            || !digits.len().is_multiple_of(2)
            || !digits.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(TextError::InvalidHexBytes);
        }
        for pair in digits.as_bytes().chunks_exact(2) {
            let pair = std::str::from_utf8(pair).expect("十六进制数字为 ASCII");
            bytes.push(u8::from_str_radix(pair, 16).expect("每两个十六进制数字为一个字节"));
        }
    }
    decoded_utf8(bytes)
}

fn encode_code_points(text: &str) -> String {
    let mut encoded = String::new();
    for character in text.chars() {
        if !encoded.is_empty() {
            encoded.push(' ');
        }
        write!(encoded, "U+{:04X}", u32::from(character)).expect("写入 String 不会出错");
    }
    encoded
}

fn decode_json_string(text: &str) -> Result<String, TextError> {
    let decoded: String =
        serde_json::from_str(text.trim()).map_err(|_| TextError::InvalidJsonString)?;
    if decoded.contains('\0') {
        return Err(TextError::NullCharacter);
    }
    Ok(decoded)
}

fn decode_code_points(text: &str) -> Result<String, TextError> {
    text.split_whitespace()
        .map(|code| {
            let digits = code
                .strip_prefix("U+")
                .or_else(|| code.strip_prefix("u+"))
                .ok_or(TextError::InvalidCodePoint)?;
            if !(1..=6).contains(&digits.len())
                || !digits.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(TextError::InvalidCodePoint);
            }
            let character = u32::from_str_radix(digits, 16)
                .ok()
                .and_then(char::from_u32)
                .ok_or(TextError::InvalidCodePoint)?;
            if character == '\0' {
                return Err(TextError::NullCharacter);
            }
            Ok(character)
        })
        .collect()
}

fn decode_base64(
    text: &str,
    padded: &GeneralPurpose,
    unpadded: &GeneralPurpose,
) -> Result<String, TextError> {
    let encoded: Vec<_> = text
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    let decoder = if encoded.ends_with(b"=") {
        padded
    } else {
        unpadded
    };
    let bytes = decoder
        .decode(encoded)
        .map_err(|_| TextError::InvalidBase64)?;
    decoded_utf8(bytes)
}

fn decoded_utf8(bytes: Vec<u8>) -> Result<String, TextError> {
    let result = String::from_utf8(bytes).map_err(|_| TextError::InvalidUtf8)?;
    if result.contains('\0') {
        return Err(TextError::NullCharacter);
    }
    Ok(result)
}

fn replacement_argument(input: &str) -> Result<(String, &str), TextError> {
    let input = input.trim_start();
    let (operand, rest) = if let Some(quoted) = input.strip_prefix('“') {
        let (operand, rest) = quoted
            .split_once('”')
            .ok_or(TextError::IncompleteReplacement)?;
        (operand.to_owned(), rest)
    } else {
        let mut values = serde_json::Deserializer::from_str(input).into_iter::<String>();
        let operand = values
            .next()
            .ok_or(TextError::IncompleteReplacement)?
            .map_err(|error| {
                if error.is_eof() {
                    TextError::IncompleteReplacement
                } else {
                    TextError::InvalidReplacementArguments
                }
            })?;
        (operand, &input[values.byte_offset()..])
    };
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return Err(TextError::InvalidReplacementArguments);
    }
    Ok((operand, rest))
}

fn replace_text(input: &str) -> Result<String, TextError> {
    let (needle, rest) = replacement_argument(input)?;
    let (replacement, rest) = replacement_argument(rest)?;
    let text = text_payload(rest).map_err(|error| match error {
        TextError::MissingText => TextError::IncompleteReplacement,
        _ => error,
    })?;
    if needle.is_empty() {
        return Err(TextError::EmptySearchText);
    }
    let matches = text.matches(&needle).count();
    let kept_bytes = text.len() - matches * needle.len();
    let result_bytes = kept_bytes.saturating_add(matches.saturating_mul(replacement.len()));
    if result_bytes > MAX_REPLACEMENT_BYTES {
        return Err(TextError::ReplacementTooLarge);
    }
    if matches != 0 && replacement.contains('\0') {
        return Err(TextError::NullCharacter);
    }
    Ok(text.replace(&needle, &replacement))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_utf8_bytes_in_hex_and_validates_decoded_text() {
        for (query, expected) in [
            ("UTF8编码 北京", "E5 8C 97 E4 BA AC"),
            ("UTF8编码 “ A ”", "20 41 20"),
            ("UTF8解码 e58c97 E4BAAC", "北京"),
            ("utf8-hex-decode 0xf0 0X9F\r\n98 80", "😀"),
            ("UTF8解码 20 41 20", " A "),
            ("UTF8编码 \"\"", ""),
            ("UTF8解码 \"\"", ""),
        ] {
            assert_eq!(transform_text(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            ("UTF8解码 4 1", TextError::InvalidHexBytes),
            ("UTF8解码 0x", TextError::InvalidHexBytes),
            ("UTF8解码 +A", TextError::InvalidHexBytes),
            ("UTF8解码 FF", TextError::InvalidUtf8),
            ("UTF8解码 E5 8C", TextError::InvalidUtf8),
            ("UTF8解码 00", TextError::NullCharacter),
        ] {
            assert_eq!(transform_text(query), Err(error), "{query}");
        }
    }

    #[test]
    fn edits_pasted_lines_without_changing_text_on_each_line() {
        for (query, expected) in [
            ("按行去重 菠萝\r\n苹果\n菠萝\n苹果 ", "菠萝\n苹果"),
            ("按行排序 B\r\nA\nb\nA", "A\nA\nB\nb"),
            ("去空行 \" 甲\r\n  \t\r\n乙 \n\n丙\"", " 甲\n乙 \n丙"),
            ("按行去重 \"A\nA \nA\"", "A\nA "),
            ("去空行 \"\n\t\n \"", ""),
            ("按行编号 \" 甲\r\n\r\n乙 \"", "1\t 甲\n2\t\n3\t乙 "),
            ("number-lines \"A\n\"", "1\tA"),
            (
                "按行计数 梨\n苹果\n梨\n苹果\n苹果\n桃",
                "3\t苹果\n2\t梨\n1\t桃",
            ),
            (
                "line-counts \"B\r\nA\nB\nA\nA \n\n\"",
                "2\tB\n2\tA\n1\tA \n1\t",
            ),
            ("按行计数 \"\"", ""),
        ] {
            assert_eq!(transform_text(query).as_deref(), Ok(expected), "{query}");
        }
    }

    #[test]
    fn base64_handles_utf8_both_alphabets_and_wrapped_input() {
        for (input, expected) in [
            ("Base64编码 北京", "5YyX5Lqs"),
            ("Base64解码 5YyX\r\n5Lqs", "北京"),
            ("Base64编码 😀", "8J+YgA=="),
            ("Base64解码 8J+YgA", "😀"),
            ("Base64URL编码 😀", "8J-YgA"),
            ("Base64URL解码 8J-YgA==", "😀"),
            (r#"base64-encode " a ""#, "IGEg"),
            (r#"base64-decode """#, ""),
        ] {
            assert_eq!(transform_text(input).as_deref(), Ok(expected), "{input}");
        }
        for input in [
            "Base64解码 8J-YgA",
            "Base64URL解码 8J+YgA",
            "Base64解码 AB==",
            "Base64解码 AA=",
        ] {
            assert_eq!(
                transform_text(input),
                Err(TextError::InvalidBase64),
                "{input}"
            );
        }
        assert_eq!(
            transform_text("Base64解码 /w=="),
            Err(TextError::InvalidUtf8)
        );
        assert_eq!(
            transform_text("Base64解码 AA=="),
            Err(TextError::NullCharacter)
        );
    }

    #[test]
    fn quotes_and_decodes_json_strings_with_unicode_and_control_characters() {
        for (query, expected) in [
            (r#"JSON转义 他写下 "你好""#, r#""他写下 \"你好\"""#),
            ("JSON转义 “ 甲\r\n乙\t ”", r#"" 甲\r\n乙\t ""#),
            (r#"JSON还原 "\u4F60\n好""#, "你\n好"),
            (r#"json-unquote "He said \"hi\"""#, r#"He said "hi""#),
            ("json-quote \"\"", "\"\""),
            ("JSON还原 \"\"", ""),
        ] {
            assert_eq!(transform_text(query).as_deref(), Ok(expected), "{query}");
        }
        for query in [
            "JSON还原 123",
            r#"JSON还原 ["你好"]"#,
            r#"JSON还原 "\uD800""#,
            r#"JSON还原 "A" "B""#,
        ] {
            assert_eq!(
                transform_text(query),
                Err(TextError::InvalidJsonString),
                "{query}"
            );
        }
        assert_eq!(
            transform_text(r#"JSON还原 "\u0000""#),
            Err(TextError::NullCharacter)
        );
    }

    #[test]
    fn converts_unicode_code_points_without_combining_or_losing_scalars() {
        for (query, expected) in [
            ("码位 你👩‍💻", "U+4F60 U+1F469 U+200D U+1F4BB"),
            ("码位 “ e\u{301} ”", "U+0020 U+0065 U+0301 U+0020"),
            ("码位解码 u+4f60 U+1F469\nU+200D U+1F4BB", "你👩‍💻"),
            ("codepoints-decode U+000A U+9", "\n\t"),
            ("codepoints \"\"", ""),
        ] {
            assert_eq!(transform_text(query).as_deref(), Ok(expected), "{query}");
        }
        for code in ["U+", "U++0041", "U+D800", "U+110000", "U+0000041", "0041"] {
            assert_eq!(
                transform_text(&format!("码位解码 {code}")),
                Err(TextError::InvalidCodePoint),
                "{code}"
            );
        }
        assert_eq!(
            transform_text("码位解码 U+0061 U+0000"),
            Err(TextError::NullCharacter)
        );
    }

    #[test]
    fn counts_graphemes_and_utf8_bytes_including_quoted_spaces() {
        for (input, expected) in [
            ("你好👩‍💻", "3 字符 · 17 UTF-8 字节"),
            ("e\u{301}", "1 字符 · 3 UTF-8 字节"),
            ("\" a \"", "3 字符 · 3 UTF-8 字节"),
            ("“ 你 ”", "3 字符 · 5 UTF-8 字节"),
            ("\"\"", "0 字符 · 0 UTF-8 字节"),
        ] {
            assert_eq!(count_text(input).as_deref(), Ok(expected), "{input}");
        }
        assert_eq!(count_text(""), Err(TextError::MissingText));
        assert_eq!(count_text("“你好"), Err(TextError::UnclosedQuote));
    }

    #[test]
    fn replaces_literal_text_with_quoted_operands_and_reverses_graphemes() {
        for (input, expected) in [
            (r#"替换 "A B" "X" A B | A B"#, "X | X"),
            (r#"replace "," "\n" a,b,c"#, "a\nb\nc"),
            (r#"替换 "." "" a.b.c"#, "abc"),
            ("替换 “甲” “乙” “ 甲 甲 ”", " 乙 乙 "),
            ("倒序 A👩‍💻中", "中👩‍💻A"),
        ] {
            assert_eq!(transform_text(input).as_deref(), Ok(expected), "{input}");
        }
        for (input, expected) in [
            (r#"替换 "a""b" a"#, TextError::InvalidReplacementArguments),
            (r#"替换 "a" "#, TextError::IncompleteReplacement),
            (r#"替换 "" "x" a"#, TextError::EmptySearchText),
            (r#"替换 "a" "\u0000" a"#, TextError::NullCharacter),
        ] {
            assert_eq!(transform_text(input), Err(expected), "{input}");
        }
        let large = format!(r#"替换 "x" "{}" {}"#, "Y".repeat(1500), "x".repeat(1500));
        assert_eq!(transform_text(&large), Err(TextError::ReplacementTooLarge));
    }

    #[test]
    fn hashes_the_exact_utf8_text() {
        for (input, expected) in [
            (
                "SHA-256 北京",
                "68c77e155132565e1cbbace574552fedbb9e769c147d0e9e5ed64701f0cdf82d",
            ),
            (
                "sha512 abc",
                concat!(
                    "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a",
                    "2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
                ),
            ),
            (
                "SHA256 \"\"",
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            ),
        ] {
            assert_eq!(transform_text(input).as_deref(), Ok(expected), "{input}");
        }
    }

    #[test]
    fn changes_unicode_case_and_collapses_whitespace() {
        for (input, expected) in [
            ("大写 Straße", "STRASSE"),
            ("LOWER “ ÉTÉ ”", " été "),
            ("小写 你好ABC", "你好abc"),
            ("合并空白 A　 B\tC", "A B C"),
            ("collapse \"  A   B  \"", "A B"),
        ] {
            assert_eq!(transform_text(input).as_deref(), Ok(expected), "{input}");
        }
        for (input, error) in [
            ("大写", TextError::IncompleteTransform),
            ("小写 “ABC", TextError::UnclosedQuote),
            ("缩写 ABC", TextError::UnknownOperation),
        ] {
            assert_eq!(transform_text(input), Err(error), "{input}");
        }
    }
}
