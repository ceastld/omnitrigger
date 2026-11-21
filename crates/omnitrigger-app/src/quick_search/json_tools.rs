use std::fmt;

use serde::de::{IgnoredAny, MapAccess, Visitor};
use serde_json::value::RawValue;

const MAX_PRETTY_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum JsonError {
    Incomplete,
    UnknownOperation,
    Invalid,
    LayoutTooLarge,
    InvalidPointer,
    ExpectedObject,
    ExpectedCollection,
    ExpectedArray,
    MissingValue,
    DuplicateKey,
}

impl JsonError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入操作和 JSON 文本",
            Self::UnknownOperation => "可用的操作有 格式化、压缩、取值、取列、键名 和 长度",
            Self::Invalid => "JSON 语法无效",
            Self::LayoutTooLarge => "JSON 格式化结果超过 1 MiB",
            Self::InvalidPointer => "JSON 路径无效，示例 /商品/0/价格",
            Self::ExpectedObject => "请输入 JSON 对象以查看顶层键名",
            Self::ExpectedCollection => "请输入 JSON 数组或对象以统计顶层条目数",
            Self::ExpectedArray => "请输入 JSON 数组以提取每项中的值",
            Self::MissingValue => "JSON 路径没有对应的值",
            Self::DuplicateKey => "JSON 路径遇到了重复的键",
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
        "取值" | "get" | "pointer" => return select_json_value(source.trim_start()),
        "取列" | "column" => return select_json_column(source.trim_start()),
        "键名" | "keys" => return json_keys(source.trim_start()),
        "长度" | "length" => return json_length(source.trim_start()),
        _ => return Err(JsonError::UnknownOperation),
    };
    let source = source.trim_start();
    if source.is_empty() {
        return Err(JsonError::Incomplete);
    }
    // 通过解析验证语法，再调整原文空白，保留键顺序、数字和字符串拼写。
    serde_json::from_str::<IgnoredAny>(source).map_err(json_parse_error)?;

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

fn json_parse_error(error: serde_json::Error) -> JsonError {
    if error.is_eof() {
        JsonError::Incomplete
    } else {
        JsonError::Invalid
    }
}

fn select_json_value(query: &str) -> Result<String, JsonError> {
    let (pointer, source) = pointer_and_source(query)?;
    let root = serde_json::from_str::<&RawValue>(source).map_err(json_parse_error)?;
    let value = pointer_keys(&pointer)?.try_fold(root, |value, key| select_member(value, &key?))?;
    Ok(value.get().to_owned())
}

fn select_json_column(query: &str) -> Result<String, JsonError> {
    let (pointer, source) = pointer_and_source(query)?;
    if !source.starts_with('[') {
        serde_json::from_str::<IgnoredAny>(source).map_err(json_parse_error)?;
        return Err(JsonError::ExpectedArray);
    }
    let items = serde_json::from_str::<Vec<&RawValue>>(source).map_err(json_parse_error)?;
    let keys = pointer_keys(&pointer)?.collect::<Result<Vec<_>, _>>()?;
    let values = items
        .into_iter()
        .map(|item| {
            keys.iter()
                .try_fold(item, |value, key| select_member(value, key))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(serde_json::to_string(&values).expect("有效的原始 JSON 值可以直接序列化"))
}

fn pointer_and_source(query: &str) -> Result<(String, &str), JsonError> {
    let (pointer, source) = if query.starts_with('"') {
        let mut strings = serde_json::Deserializer::from_str(query).into_iter::<String>();
        let pointer = strings
            .next()
            .ok_or(JsonError::Incomplete)?
            .map_err(|error| {
                if error.is_eof() {
                    JsonError::Incomplete
                } else {
                    JsonError::InvalidPointer
                }
            })?;
        let rest = &query[strings.byte_offset()..];
        if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
            return Err(JsonError::InvalidPointer);
        }
        (pointer, rest)
    } else {
        let (pointer, rest) = query
            .split_once(char::is_whitespace)
            .ok_or(JsonError::Incomplete)?;
        (pointer.to_owned(), rest)
    };
    Ok((pointer, source.trim_start()))
}

fn pointer_keys(
    pointer: &str,
) -> Result<impl Iterator<Item = Result<String, JsonError>> + '_, JsonError> {
    let path = if pointer.is_empty() {
        None
    } else {
        Some(pointer.strip_prefix('/').ok_or(JsonError::InvalidPointer)?)
    };
    Ok(path
        .into_iter()
        .flat_map(|path| path.split('/'))
        .map(decode_pointer_key))
}

fn select_member<'a>(value: &'a RawValue, key: &str) -> Result<&'a RawValue, JsonError> {
    let raw = value.get();
    match raw.as_bytes()[0] {
        b'{' => serde::Deserializer::deserialize_map(
            &mut serde_json::Deserializer::from_str(raw),
            ObjectMember(key),
        )
        .map_err(json_parse_error)?,
        b'[' => {
            if (key.len() > 1 && key.starts_with('0'))
                || !key.bytes().all(|byte| byte.is_ascii_digit())
            {
                return Err(JsonError::MissingValue);
            }
            let index = key.parse::<usize>().map_err(|_| JsonError::MissingValue)?;
            let items = serde_json::from_str::<Vec<&RawValue>>(raw).map_err(json_parse_error)?;
            items.get(index).copied().ok_or(JsonError::MissingValue)
        }
        _ => Err(JsonError::MissingValue),
    }
}

fn json_keys(source: &str) -> Result<String, JsonError> {
    if !source.starts_with('{') {
        serde_json::from_str::<IgnoredAny>(source).map_err(json_parse_error)?;
        return Err(JsonError::ExpectedObject);
    }
    let keys = read_object_keys(source)?;
    Ok(serde_json::to_string(&keys).expect("键名列表可以序列化为 JSON"))
}

fn read_object_keys(source: &str) -> Result<Vec<String>, JsonError> {
    let mut parser = serde_json::Deserializer::from_str(source);
    let keys =
        serde::Deserializer::deserialize_map(&mut parser, ObjectKeys).map_err(json_parse_error)?;
    parser.end().map_err(json_parse_error)?;
    Ok(keys)
}

fn json_length(source: &str) -> Result<String, JsonError> {
    let length = match source.as_bytes().first().copied() {
        Some(b'{') => read_object_keys(source)?.len(),
        Some(b'[') => serde_json::from_str::<Vec<IgnoredAny>>(source)
            .map_err(json_parse_error)?
            .len(),
        _ => {
            serde_json::from_str::<IgnoredAny>(source).map_err(json_parse_error)?;
            return Err(JsonError::ExpectedCollection);
        }
    };
    Ok(length.to_string())
}

fn decode_pointer_key(token: &str) -> Result<String, JsonError> {
    let mut decoded = String::with_capacity(token.len());
    let mut chars = token.chars();
    while let Some(ch) = chars.next() {
        decoded.push(match ch {
            '~' => match chars.next() {
                Some('0') => '~',
                Some('1') => '/',
                _ => return Err(JsonError::InvalidPointer),
            },
            ch => ch,
        });
    }
    Ok(decoded)
}

struct ObjectKeys;

impl<'de> Visitor<'de> for ObjectKeys {
    type Value = Vec<String>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON object")
    }

    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
        let mut keys = Vec::new();
        while let Some((key, _)) = map.next_entry::<String, IgnoredAny>()? {
            keys.push(key);
        }
        Ok(keys)
    }
}

struct ObjectMember<'a>(&'a str);

impl<'de> Visitor<'de> for ObjectMember<'_> {
    type Value = Result<&'de RawValue, JsonError>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON object")
    }

    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
        let mut found = Ok(None);
        while let Some((key, value)) = map.next_entry::<String, &RawValue>()? {
            if key == self.0 {
                found = match found {
                    Ok(None) => Ok(Some(value)),
                    _ => Err(JsonError::DuplicateKey),
                };
            }
        }
        Ok(found.and_then(|value| value.ok_or(JsonError::MissingValue)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_a_json_pointer_from_each_array_entry_and_preserves_raw_values() {
        for (input, expected) in [
            (r#"取列 /价格 [{"价格":9.90},{"价格":2e2}]"#, "[9.90,2e2]"),
            (
                r#"column "/a b/0" [{"a b":[9007199254740993]},{"a b":[null]}]"#,
                "[9007199254740993,null]",
            ),
            (r#"取列 "" [1,"北京"]"#, r#"[1,"北京"]"#),
            (r#"取列 /状态 []"#, "[]"),
        ] {
            assert_eq!(transform_json(input).as_deref(), Ok(expected), "{input}");
        }
        for (input, error) in [
            (r#"取列 /x [{"x":1},{}]"#, JsonError::MissingValue),
            (r#"取列 /x [{"x":1,"x":2}]"#, JsonError::DuplicateKey),
            (r#"取列 /~2 []"#, JsonError::InvalidPointer),
            (r#"取列 /x {"x":1}"#, JsonError::ExpectedArray),
        ] {
            assert_eq!(transform_json(input), Err(error), "{input}");
        }
    }

    #[test]
    fn counts_only_top_level_array_elements_or_object_entries() {
        for (query, expected) in [
            (r#"长度 [null, {"x":[1,2]}, 1e999]"#, "3"),
            (r#"length {"x":1,"x":2,"y":{}}"#, "3"),
            ("长度 []", "0"),
            ("长度 {}", "0"),
        ] {
            assert_eq!(transform_json(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            ("长度 [1,", JsonError::Incomplete),
            ("长度 [1,]", JsonError::Invalid),
            ("长度 {} []", JsonError::Invalid),
            ("长度 true", JsonError::ExpectedCollection),
        ] {
            assert_eq!(transform_json(query), Err(error), "{query}");
        }
    }

    #[test]
    fn lists_top_level_object_keys_in_order_including_duplicate_names() {
        for (query, expected) in [
            (
                r#"键名 {"Z":{"nested":1},"a":1e999,"\u0061":null}"#,
                r#"["Z","a","a"]"#,
            ),
            (
                r#"keys {"\u5317\u4EAC":1,"控制\n符":null,"":0}"#,
                r#"["北京","控制\n符",""]"#,
            ),
            ("键名 {}", "[]"),
        ] {
            assert_eq!(transform_json(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            ("keys [1,2]", JsonError::ExpectedObject),
            ("keys \"北京\"", JsonError::ExpectedObject),
            ("keys {", JsonError::Incomplete),
            ("keys {\"a\":1,}", JsonError::Invalid),
            ("keys {} []", JsonError::Invalid),
            ("keys [,", JsonError::Invalid),
        ] {
            assert_eq!(transform_json(query), Err(error), "{query}");
        }
    }

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

    #[test]
    fn resolves_json_pointer_without_rewriting_the_selected_value() {
        assert_eq!(
            transform_json(r#"取值 /商品/0/价格 {"商品":[{"价格":9007199254740993}]}"#).as_deref(),
            Ok("9007199254740993")
        );
        assert_eq!(
            transform_json(r#"get "/a~1b/~01/ " {"a/b":{"~1":{" ":1e999}}}"#).as_deref(),
            Ok("1e999")
        );
        assert_eq!(transform_json(r#"get / {"":null}"#).as_deref(), Ok("null"));
        assert_eq!(
            transform_json(r#"get "" [ 1 , "\u5317" ]"#).as_deref(),
            Ok(r#"[ 1 , "\u5317" ]"#)
        );
        assert_eq!(
            transform_json(r#"get /a {"a":[ 1 , 2 ],"b":0,"b":1}"#).as_deref(),
            Ok("[ 1 , 2 ]")
        );
    }

    #[test]
    fn distinguishes_bad_pointers_missing_values_and_duplicate_keys() {
        for query in ["get /a~2 {}", "get a {}", r#"get "/a"{}"#] {
            assert_eq!(transform_json(query), Err(JsonError::InvalidPointer));
        }
        for query in ["get /a {}", "get /- [1]", "get /01 [1,2]", "get /0 null"] {
            assert_eq!(transform_json(query), Err(JsonError::MissingValue));
        }
        assert_eq!(
            transform_json(r#"get /a {"a":1,"\u0061":2}"#),
            Err(JsonError::DuplicateKey)
        );
        assert_eq!(transform_json("get /a {"), Err(JsonError::Incomplete));
    }
}
