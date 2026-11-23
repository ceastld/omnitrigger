use serde_json::{Map, Number, Value};

const MAX_TOML_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TomlError {
    Incomplete,
    UnknownOperation,
    Invalid { line: usize, column: usize },
    NonFiniteFloat,
    Serialization,
    TooLarge,
}

impl TomlError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入 TOML 内容，例如 TOML: 校验 port = 8000",
            Self::UnknownOperation => "用 校验 或 转JSON 处理 TOML",
            Self::Invalid { .. } => "请检查 TOML 语法",
            Self::NonFiniteFloat => "JSON 无法表示 TOML 中的 inf 或 nan",
            Self::Serialization => "无法生成 JSON",
            Self::TooLarge => "TOML 输入或 JSON 结果超过 1 MiB",
        }
    }
}

pub(super) fn transform_toml(input: &str) -> Result<String, TomlError> {
    let input = input.trim_start();
    let end = input.find(char::is_whitespace).unwrap_or(input.len());
    let (operation, rest) = input.split_at(end);
    let to_json = match operation.to_ascii_lowercase().as_str() {
        "校验" | "validate" => false,
        "转json" | "to-json" => true,
        "" => return Err(TomlError::Incomplete),
        _ => return Err(TomlError::UnknownOperation),
    };
    let source = rest
        .strip_prefix("\r\n")
        .or_else(|| rest.strip_prefix(char::is_whitespace))
        .filter(|source| !source.trim().is_empty())
        .ok_or(TomlError::Incomplete)?;
    if source.len() > MAX_TOML_BYTES {
        return Err(TomlError::TooLarge);
    }

    let table = source.parse::<toml::Table>().map_err(|error| {
        let prefix = error
            .span()
            .and_then(|span| source.get(..span.start))
            .unwrap_or("");
        TomlError::Invalid {
            line: prefix.bytes().filter(|&byte| byte == b'\n').count() + 1,
            column: prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1,
        }
    })?;
    if !to_json {
        return Ok(format!("TOML 有效 · {} 个顶层键", table.len()));
    }

    let json = serde_json::to_string(&json_value(toml::Value::Table(table))?)
        .map_err(|_| TomlError::Serialization)?;
    if json.len() > MAX_TOML_BYTES {
        return Err(TomlError::TooLarge);
    }
    Ok(json)
}

fn json_value(value: toml::Value) -> Result<Value, TomlError> {
    Ok(match value {
        toml::Value::String(text) => Value::String(text),
        toml::Value::Integer(number) => Value::Number(number.into()),
        toml::Value::Float(number) => {
            Value::Number(Number::from_f64(number).ok_or(TomlError::NonFiniteFloat)?)
        }
        toml::Value::Boolean(flag) => Value::Bool(flag),
        toml::Value::Datetime(date) => Value::String(date.to_string()),
        toml::Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(json_value)
                .collect::<Result<_, _>>()?,
        ),
        toml::Value::Table(table) => Value::Object(
            table
                .into_iter()
                .map(|(key, value)| json_value(value).map(|value| (key, value)))
                .collect::<Result<Map<_, _>, _>>()?,
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_toml_and_preserves_source_line_and_column_in_errors() {
        assert_eq!(
            transform_toml("校验\r\nname = '茶'\n[server]\nport = 8000").as_deref(),
            Ok("TOML 有效 · 2 个顶层键"),
        );
        assert_eq!(
            transform_toml("validate weight = inf").as_deref(),
            Ok("TOML 有效 · 1 个顶层键"),
        );
        assert!(matches!(
            transform_toml("校验\r\nname = '茶'\r\nname = '咖啡'"),
            Err(TomlError::Invalid { line: 2, column: 1 })
        ));
        assert_eq!(transform_toml("校验"), Err(TomlError::Incomplete));
    }

    #[test]
    fn converts_toml_tables_arrays_dates_and_large_integers_to_json() {
        let result = transform_toml(concat!(
            "转JSON date = 2024-02-29\n",
            "[[items]]\nname = '茶'\nprice = 1.5\nnumber = 9_007_199_254_740_993\n",
            "[[items]]\nname = '咖啡'\nenabled = true"
        ))
        .unwrap();
        assert!(result.contains("9007199254740993"));
        let json: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "date": "2024-02-29",
                "items": [
                    {"name":"茶","price":1.5,"number":9007199254740993_i64},
                    {"name":"咖啡","enabled":true}
                ]
            })
        );
        assert_eq!(
            transform_toml("to-json values = [1, nan]"),
            Err(TomlError::NonFiniteFloat),
        );
    }
}
