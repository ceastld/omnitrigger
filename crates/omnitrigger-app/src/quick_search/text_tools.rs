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
    InvalidUtf8,
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
            Self::InvalidUtf8 => "解码结果不是 UTF-8 文本",
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
    if matches!(operation.as_str(), "替换" | "replace") {
        return replace_text(text);
    }
    let text = text_payload(text)?;
    match operation.as_str() {
        "大写" | "upper" | "uppercase" => Ok(text.to_uppercase()),
        "小写" | "lower" | "lowercase" => Ok(text.to_lowercase()),
        "合并空白" | "collapse" => Ok(text.split_whitespace().collect::<Vec<_>>().join(" ")),
        "倒序" | "reverse" => Ok(text.graphemes(true).rev().collect()),
        "sha256" | "sha-256" => Ok(format!("{:x}", Sha256::digest(text.as_bytes()))),
        "sha512" | "sha-512" => Ok(format!("{:x}", Sha512::digest(text.as_bytes()))),
        "base64编码" | "base64-encode" => Ok(STANDARD.encode(text)),
        "base64解码" | "base64-decode" => decode_base64(text, &STANDARD, &STANDARD_NO_PAD),
        "base64url编码" | "base64url-encode" => Ok(URL_SAFE_NO_PAD.encode(text)),
        "base64url解码" | "base64url-decode" => decode_base64(text, &URL_SAFE, &URL_SAFE_NO_PAD),
        _ => Err(TextError::UnknownOperation),
    }
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
