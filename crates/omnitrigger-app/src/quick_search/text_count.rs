use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TextCountError {
    MissingText,
    UnclosedQuote,
}

impl TextCountError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::MissingText => "输入要统计的文字",
            Self::UnclosedQuote => "补全引号后统计",
        }
    }
}

pub(super) fn count_text(input: &str) -> Result<String, TextCountError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(TextCountError::MissingText);
    }
    let text = match input.chars().next() {
        Some('"') => input[1..]
            .strip_suffix('"')
            .ok_or(TextCountError::UnclosedQuote)?,
        Some('“') => input['“'.len_utf8()..]
            .strip_suffix('”')
            .ok_or(TextCountError::UnclosedQuote)?,
        _ => input,
    };
    let characters = text.graphemes(true).count();
    Ok(format!("{characters} 字符 · {} UTF-8 字节", text.len()))
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
        assert_eq!(count_text(""), Err(TextCountError::MissingText));
        assert_eq!(count_text("“你好"), Err(TextCountError::UnclosedQuote));
    }
}
