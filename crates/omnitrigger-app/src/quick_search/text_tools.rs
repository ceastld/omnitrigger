use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TextError {
    MissingText,
    UnclosedQuote,
    IncompleteTransform,
    UnknownOperation,
}

impl TextError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::MissingText => "输入要统计的文字",
            Self::UnclosedQuote => "补全引号后继续",
            Self::IncompleteTransform => "输入操作和文本，例如 文本: 大写 hello",
            Self::UnknownOperation => "可用的操作有 大写、小写 和 合并空白",
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
    let text = text_payload(text)?;
    match operation.to_ascii_lowercase().as_str() {
        "大写" | "upper" | "uppercase" => Ok(text.to_uppercase()),
        "小写" | "lower" | "lowercase" => Ok(text.to_lowercase()),
        "合并空白" | "collapse" => Ok(text.split_whitespace().collect::<Vec<_>>().join(" ")),
        _ => Err(TextError::UnknownOperation),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
