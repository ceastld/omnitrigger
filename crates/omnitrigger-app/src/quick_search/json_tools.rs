use serde::de::IgnoredAny;

const MAX_PRETTY_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum JsonError {
    Incomplete,
    UnknownOperation,
    Invalid,
    LayoutTooLarge,
}

impl JsonError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入操作和 JSON 文本",
            Self::UnknownOperation => "可用的操作有 格式化 和 压缩",
            Self::Invalid => "JSON 语法无效",
            Self::LayoutTooLarge => "JSON 格式化结果超过 1 MiB",
        }
    }
}

pub(super) fn transform_json(query: &str) -> Result<String, JsonError> {
    let (operation, source) = query
        .split_once(char::is_whitespace)
        .ok_or(JsonError::Incomplete)?;
    let pretty = match operation.to_ascii_lowercase().as_str() {
        "格式化" | "format" | "pretty" => true,
        "压缩" | "compact" | "minify" => false,
        _ => return Err(JsonError::UnknownOperation),
    };
    let source = source.trim_start();
    if source.is_empty() {
        return Err(JsonError::Incomplete);
    }
    // 通过解析验证语法，再调整原文空白，保留键顺序、数字和字符串拼写。
    serde_json::from_str::<IgnoredAny>(source).map_err(|error| {
        if error.is_eof() {
            JsonError::Incomplete
        } else {
            JsonError::Invalid
        }
    })?;

    let mut result = String::with_capacity(source.len());
    let mut depth = 0;
    let mut previous = ' ';
    let mut quoted = false;
    let mut escaped = false;
    for ch in source.chars() {
        if quoted {
            result.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                quoted = false;
            }
            continue;
        }
        if matches!(ch, ' ' | '\t' | '\n' | '\r') {
            continue;
        }
        if pretty {
            let line_break = if matches!(ch, '}' | ']') {
                depth -= 1;
                !matches!(previous, '{' | '[')
            } else {
                matches!(previous, '{' | '[' | ',')
            };
            if line_break {
                let indent = depth * 2;
                if result.len() + indent + 1 > MAX_PRETTY_BYTES {
                    return Err(JsonError::LayoutTooLarge);
                }
                result.push('\n');
                result.extend(std::iter::repeat_n(' ', indent));
            } else if previous == ':' {
                result.push(' ');
            }
            if matches!(ch, '{' | '[') {
                depth += 1;
            }
        }
        result.push(ch);
        quoted = ch == '"';
        previous = ch;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adjusts_json_whitespace_without_rounding_or_reordering() {
        let source = r#"{"z":[" 北京 \"字\" ",{}],"n":9007199254740993,"n":1e999}"#;
        let expected = r#"{
  "z": [
    " 北京 \"字\" ",
    {}
  ],
  "n": 9007199254740993,
  "n": 1e999
}"#;
        assert_eq!(
            transform_json(&format!("格式化 {source}")).as_deref(),
            Ok(expected)
        );
        assert_eq!(
            transform_json(&format!("minify {expected}")).as_deref(),
            Ok(source)
        );
        assert_eq!(
            transform_json("pretty [[],{}]").unwrap(),
            "[\n  [],\n  {}\n]"
        );
        assert_eq!(
            transform_json("pretty -0.001200e+02").unwrap(),
            "-0.001200e+02"
        );
    }

    #[test]
    fn distinguishes_incomplete_invalid_and_large_layouts() {
        assert_eq!(transform_json("格式化 {"), Err(JsonError::Incomplete));
        assert_eq!(transform_json("压缩 [1,]"), Err(JsonError::Invalid));
        assert_eq!(transform_json("压缩 true false"), Err(JsonError::Invalid));
        assert_eq!(transform_json("未知 {}"), Err(JsonError::UnknownOperation));
        let nested = format!("{}0{}", "[".repeat(1024), "]".repeat(1024));
        assert_eq!(
            transform_json(&format!("格式化 {nested}")),
            Err(JsonError::LayoutTooLarge)
        );
        assert_eq!(transform_json(&format!("压缩 {nested}")).unwrap(), nested);
    }
}
