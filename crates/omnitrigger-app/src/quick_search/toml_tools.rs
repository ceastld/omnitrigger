use serde_json::{Map, Number, Value};

use super::{
    csv_tools::append_bounded_csv_row,
    input_line_column, is_validation_operation,
    json_tools::{
        append_pointer_key, json_array_index, json_pointer_tokens, pointer_and_source, JsonError,
    },
    InputValidationReport,
};

const MAX_TOML_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TomlError {
    Incomplete,
    UnknownOperation,
    Invalid {
        line: usize,
        column: usize,
        span: Option<(usize, usize)>,
    },
    NonFiniteFloat,
    InvalidPointer,
    MissingPointer,
    Serialization,
    TooLarge,
    CsvNullCharacter,
}

impl TomlError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入 TOML 内容，例如 TOML: 校验 port = 8000",
            Self::UnknownOperation => "用 校验、取值、转JSON 或 叶子CSV 处理 TOML",
            Self::Invalid { .. } => "请检查 TOML 语法",
            Self::NonFiniteFloat => "JSON 无法表示 TOML 中的 inf 或 nan",
            Self::InvalidPointer => "填写有效路径，例如 /server/port，空路径可写成 \"\"",
            Self::MissingPointer => "TOML 没有该路径，数组下标从 0 开始",
            Self::Serialization => "无法生成 JSON",
            Self::TooLarge => "TOML 输入或转换结果超过 1 MiB",
            Self::CsvNullCharacter => "CSV 路径和单元格不能包含空字符 U+0000",
        }
    }
}

pub(super) fn syntax_error_message(line: usize, column: usize) -> String {
    format!("TOML 语法有误 · 第 {line} 行第 {column} 列")
}

enum TomlOperation {
    Validate,
    Json,
    LeavesCsv,
}

impl TomlOperation {
    fn parse(operation: &str) -> Result<Self, TomlError> {
        if is_validation_operation(operation) {
            return Ok(Self::Validate);
        }
        match operation.to_ascii_lowercase().as_str() {
            "转json" | "to-json" => Ok(Self::Json),
            "叶子csv" | "leaves-csv" => Ok(Self::LeavesCsv),
            "" => Err(TomlError::Incomplete),
            _ => Err(TomlError::UnknownOperation),
        }
    }
}

fn split_toml_input(input: &str) -> (&str, &str) {
    let input = input.trim_start();
    let end = input.find(char::is_whitespace).unwrap_or(input.len());
    input.split_at(end)
}

pub(super) fn transform_toml(input: &str) -> Result<String, TomlError> {
    let (operation, source) = split_toml_input(input);
    if is_selection(operation) {
        return select_toml_value(toml_source(source)?);
    }
    run_toml_operation(TomlOperation::parse(operation)?, source)
}

fn is_selection(operation: &str) -> bool {
    operation == "取值" || operation.eq_ignore_ascii_case("get")
}

pub(super) fn produces_json(input: &str) -> bool {
    let (operation, _) = split_toml_input(input);
    is_selection(operation) || matches!(TomlOperation::parse(operation), Ok(TomlOperation::Json))
}

pub(super) fn toml_validation_report(
    input: &str,
) -> Option<Result<InputValidationReport, TomlError>> {
    let (operation, rest) = split_toml_input(input);
    if !is_validation_operation(operation) {
        return None;
    }
    Some(toml_source(rest).and_then(|source| {
        match run_toml_source(TomlOperation::Validate, source) {
            Ok(text) => Ok(InputValidationReport {
                text,
                error_span: None,
            }),
            Err(TomlError::Invalid {
                line,
                column,
                span: Some((start, end)),
            }) => Ok(InputValidationReport {
                text: syntax_error_message(line, column),
                error_span: Some(start..end),
            }
            .offset_error_span(input.len() - source.len())),
            Err(error) => Err(error),
        }
    }))
}

pub(super) fn toml_csv_report(input: &str) -> Option<Result<String, TomlError>> {
    let (operation, source) = split_toml_input(input);
    let operation = TomlOperation::parse(operation).ok()?;
    matches!(operation, TomlOperation::LeavesCsv).then(|| run_toml_operation(operation, source))
}

fn toml_source(rest: &str) -> Result<&str, TomlError> {
    let source = rest
        .strip_prefix("\r\n")
        .or_else(|| rest.strip_prefix(char::is_whitespace))
        .filter(|source| !source.trim().is_empty())
        .ok_or(TomlError::Incomplete)?;
    if source.len() > MAX_TOML_BYTES {
        return Err(TomlError::TooLarge);
    }
    Ok(source)
}

fn run_toml_operation(operation: TomlOperation, rest: &str) -> Result<String, TomlError> {
    run_toml_source(operation, toml_source(rest)?)
}

fn parse_toml_table(source: &str) -> Result<toml::Table, TomlError> {
    source.parse::<toml::Table>().map_err(|error| {
        let span = error.span().and_then(|span| {
            source.get(span.clone())?;
            Some((span.start, span.end))
        });
        let prefix = span.map_or("", |(start, _)| &source[..start]);
        let (line, column) = input_line_column(prefix);
        TomlError::Invalid { line, column, span }
    })
}

fn run_toml_source(operation: TomlOperation, source: &str) -> Result<String, TomlError> {
    let table = parse_toml_table(source)?;
    match operation {
        TomlOperation::Validate => return Ok(format!("TOML 有效 · {} 个顶层键", table.len())),
        TomlOperation::LeavesCsv => return toml_leaves_csv(table),
        TomlOperation::Json => {}
    }

    write_toml_json(toml::Value::Table(table))
}

fn select_toml_value(input: &str) -> Result<String, TomlError> {
    let (pointer, source) = pointer_and_source(input).map_err(|error| match error {
        JsonError::Incomplete => TomlError::Incomplete,
        _ => TomlError::InvalidPointer,
    })?;
    if source.is_empty() {
        return Err(TomlError::Incomplete);
    }
    let keys = json_pointer_tokens(&pointer).map_err(|_| TomlError::InvalidPointer)?;
    let mut value = toml::Value::Table(parse_toml_table(source)?);
    for key in keys {
        value = match value {
            toml::Value::Table(mut fields) => fields.remove(&key),
            toml::Value::Array(mut items) => json_array_index(&key)
                .ok()
                .filter(|&index| index < items.len())
                .map(|index| items.swap_remove(index)),
            _ => None,
        }
        .ok_or(TomlError::MissingPointer)?;
    }
    write_toml_json(value)
}

fn write_toml_json(value: toml::Value) -> Result<String, TomlError> {
    let json = serde_json::to_string(&json_value(value)?).map_err(|_| TomlError::Serialization)?;
    if json.len() > MAX_TOML_BYTES {
        return Err(TomlError::TooLarge);
    }
    Ok(json)
}

fn toml_leaves_csv(table: toml::Table) -> Result<String, TomlError> {
    use toml::Value as TomlValue;

    let mut output = String::from("路径,类型,值");
    let mut path = String::new();
    let mut pending = vec![(TomlValue::Table(table), None::<(usize, String)>)];
    while let Some((value, key)) = pending.pop() {
        if let Some((parent_length, key)) = key {
            path.truncate(parent_length);
            append_pointer_key(&mut path, &key);
        }
        let (kind, value) = match value {
            TomlValue::Table(table) if !table.is_empty() => {
                let mut children: Vec<_> = table.into_iter().collect();
                children.sort_unstable_by(|left, right| left.0.cmp(&right.0));
                pending.extend(
                    children
                        .into_iter()
                        .rev()
                        .map(|(key, value)| (value, Some((path.len(), key)))),
                );
                continue;
            }
            TomlValue::Array(values) if !values.is_empty() => {
                pending.extend(
                    values
                        .into_iter()
                        .enumerate()
                        .rev()
                        .map(|(index, value)| (value, Some((path.len(), index.to_string())))),
                );
                continue;
            }
            TomlValue::String(value) => ("string", value),
            TomlValue::Integer(value) => ("integer", value.to_string()),
            TomlValue::Float(value) if value.is_nan() => ("float", "nan".into()),
            TomlValue::Float(value) => ("float", value.to_string()),
            TomlValue::Boolean(value) => ("boolean", value.to_string()),
            TomlValue::Datetime(value) => ("datetime", value.to_string()),
            TomlValue::Array(_) => ("array", "[]".into()),
            TomlValue::Table(_) => ("table", "{}".into()),
        };
        if path.contains('\0') || value.contains('\0') {
            return Err(TomlError::CsvNullCharacter);
        }
        append_bounded_csv_row(
            &mut output,
            [path.as_str(), kind, value.as_str()].into_iter(),
            TomlError::TooLarge,
        )?;
    }
    Ok(output)
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
            Err(TomlError::Invalid {
                line: 2,
                column: 1,
                ..
            })
        ));
        assert_eq!(transform_toml("校验"), Err(TomlError::Incomplete));
    }

    #[test]
    fn locates_duplicate_invalid_unicode_and_unfinished_toml_tokens() {
        for (query, selected, location) in [
            (
                "校验\r\nname = '茶'\r\nname = '咖啡'",
                "name",
                "第 2 行第 1 列",
            ),
            ("VALIDATE [项]", "项", "第 1 行第 2 列"),
            ("校验 list = [1,2", "", "第 1 行第 12 列"),
        ] {
            let report = toml_validation_report(query).unwrap().unwrap();
            let span = report.error_span.unwrap();
            assert_eq!(&query[span.clone()], selected);
            if selected.is_empty() {
                assert_eq!(span, query.len()..query.len());
            }
            assert!(report.text.contains(location));
        }
        let success = toml_validation_report("校验 x = 1").unwrap().unwrap();
        assert_eq!(success.text, "TOML 有效 · 1 个顶层键");
        assert!(success.error_span.is_none());
        assert!(toml_validation_report("转JSON x = 1").is_none());
    }

    #[test]
    fn selects_one_toml_value_without_converting_unrelated_nonfinite_values() {
        let data = "\nweight = inf\n[server]\nports = [8000, 9007199254740993]\n\"region key\" = \"北\"\n\"a/b~\" = 2024-02-29";
        for (pointer, expected) in [
            ("/server/ports/1", "9007199254740993"),
            ("\"/server/region key\"", "\"北\""),
            ("/server/a~1b~0", "\"2024-02-29\""),
            ("/server/ports", "[8000,9007199254740993]"),
        ] {
            assert_eq!(
                transform_toml(&format!("取值 {pointer}{data}")).as_deref(),
                Ok(expected)
            );
        }
        for pointer in ["/server/ports/01", "/server/ports/-1", "/server/missing"] {
            assert_eq!(
                transform_toml(&format!("GET {pointer}{data}")),
                Err(TomlError::MissingPointer)
            );
        }
        assert_eq!(
            transform_toml(&format!("get /server/~2{data}")),
            Err(TomlError::InvalidPointer)
        );
        assert_eq!(
            transform_toml(&format!("get /weight{data}")),
            Err(TomlError::NonFiniteFloat)
        );
        assert_eq!(transform_toml("get \"\" x = 1").as_deref(), Ok("{\"x\":1}"));
        assert_eq!(transform_toml("get /missing"), Err(TomlError::Incomplete));
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
