use std::collections::BTreeMap;

use serde_json::{Map, Number, Value};

use super::{
    csv_tools::append_bounded_csv_row,
    input_line_column, is_validation_operation,
    json_tools::{
        append_pointer_key, json_array_index, json_pointer_tokens, pointer_and_source,
        read_json_array, read_unique_object_entries, selected_pointers, JsonError,
    },
    InputValidationReport,
};

const MAX_TOML_BYTES: usize = 1024 * 1024;
const MAX_JSON_DEPTH: usize = 128;
const TOML_LEAF_TYPES: [&str; 7] = [
    "array", "boolean", "datetime", "float", "integer", "string", "table",
];

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
    RegexPattern,
    TypeSearchArguments,
    ComparisonArguments,
    ComparisonInvalid {
        after: bool,
        line: usize,
        column: usize,
    },
    MissingPointer,
    Serialization,
    Json(JsonError),
    JsonNull,
    JsonIntegerRange,
    JsonFloatRange,
    JsonDepth,
    TomlSerialization,
    TooLarge,
    CsvNullCharacter,
}

impl TomlError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入 TOML 内容，例如 TOML: 校验 port = 8000",
            Self::UnknownOperation => {
                "用 F1 查看 TOML 操作，例如 校验、取值 或 叶子CSV"
            }
            Self::Invalid { .. } => "请检查 TOML 语法",
            Self::NonFiniteFloat => "JSON 无法表示 TOML 中的 inf 或 nan",
            Self::InvalidPointer => "填写有效路径，例如 /server/port，空路径可写成 \"\"",
            Self::RegexPattern => "用双引号括起有效的正则表达式，再输入 TOML",
            Self::TypeSearchArguments => {
                "输入字符串或数组指定类型，可用 string、integer、float、boolean、datetime、array、table"
            }
            Self::ComparisonArguments => "输入两个 TOML 字符串组成的 JSON 数组",
            Self::ComparisonInvalid { after: false, .. } => "原 TOML 配置语法有误",
            Self::ComparisonInvalid { after: true, .. } => "新 TOML 配置语法有误",
            Self::MissingPointer => "TOML 没有该路径，数组下标从 0 开始",
            Self::Serialization => "无法生成 JSON",
            Self::Json(error) => error.message(),
            Self::JsonNull => "TOML 无法表示 JSON 的 null，请先提供明确的值",
            Self::JsonIntegerRange => "JSON 整数超出 TOML 支持的有符号 64 位范围",
            Self::JsonFloatRange => "JSON 小数超出双精度浮点数可表示的范围",
            Self::JsonDepth => "JSON 嵌套超过 128 层",
            Self::TomlSerialization => "无法生成 TOML",
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
    FromJson,
    LeavesCsv,
    RegexLeavesCsv,
    RegexStringsCsv,
    FindTypesCsv,
    DiffCsv,
    TypeCountsCsv,
    Select,
    SelectPaths,
}

impl TomlOperation {
    fn parse(operation: &str) -> Result<Self, TomlError> {
        if is_validation_operation(operation) {
            return Ok(Self::Validate);
        }
        match operation.to_ascii_lowercase().as_str() {
            "转json" | "to-json" => Ok(Self::Json),
            "从json" | "from-json" => Ok(Self::FromJson),
            "叶子csv" | "leaves-csv" => Ok(Self::LeavesCsv),
            "正则叶子csv" | "regex-leaves-csv" => Ok(Self::RegexLeavesCsv),
            "正则字符串csv" | "regex-strings-csv" => Ok(Self::RegexStringsCsv),
            "找类型csv" | "find-type-csv" => Ok(Self::FindTypesCsv),
            "对比csv" | "diff-csv" => Ok(Self::DiffCsv),
            "类型统计csv" | "type-counts-csv" => Ok(Self::TypeCountsCsv),
            "取值" | "get" => Ok(Self::Select),
            "取多路径" | "get-paths" => Ok(Self::SelectPaths),
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
    run_toml_operation(TomlOperation::parse(operation)?, source)
}

pub(super) fn produces_json(input: &str) -> bool {
    let (operation, _) = split_toml_input(input);
    matches!(
        TomlOperation::parse(operation),
        Ok(TomlOperation::Json | TomlOperation::Select | TomlOperation::SelectPaths)
    )
}

pub(super) fn produces_toml(input: &str) -> bool {
    let (operation, _) = split_toml_input(input);
    matches!(TomlOperation::parse(operation), Ok(TomlOperation::FromJson))
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
    matches!(
        operation,
        TomlOperation::LeavesCsv
            | TomlOperation::RegexLeavesCsv
            | TomlOperation::RegexStringsCsv
            | TomlOperation::FindTypesCsv
            | TomlOperation::DiffCsv
            | TomlOperation::TypeCountsCsv
    )
    .then(|| run_toml_operation(operation, source))
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
    match operation {
        TomlOperation::Validate => {
            let table = parse_toml_table(source)?;
            Ok(format!("TOML 有效 · {} 个顶层键", table.len()))
        }
        TomlOperation::LeavesCsv => toml_leaves_csv(parse_toml_table(source)?, |_, _, _| true),
        TomlOperation::RegexLeavesCsv | TomlOperation::RegexStringsCsv => {
            let (pattern, source) = super::operators::json_argument_and_tail::<String>(source)
                .ok_or(TomlError::RegexPattern)?;
            let pattern =
                super::text_tools::compile_regex(&pattern).map_err(|_| TomlError::RegexPattern)?;
            let match_path = matches!(operation, TomlOperation::RegexLeavesCsv);
            toml_leaves_csv(parse_toml_table(source)?, |path, kind, value| {
                if match_path {
                    pattern.is_match(path)
                } else {
                    kind == "string" && pattern.is_match(value)
                }
            })
        }
        TomlOperation::FindTypesCsv => {
            let (argument, source) =
                super::operators::json_argument_and_tail::<TypeArgument>(source)
                    .ok_or(TomlError::TypeSearchArguments)?;
            let types = match argument {
                TypeArgument::One(kind) => vec![kind],
                TypeArgument::Many(kinds) => kinds,
            };
            if !(1..=TOML_LEAF_TYPES.len()).contains(&types.len())
                || types
                    .iter()
                    .any(|kind| !TOML_LEAF_TYPES.contains(&kind.as_str()))
            {
                return Err(TomlError::TypeSearchArguments);
            }
            toml_leaves_csv(parse_toml_table(source)?, |_, kind, _| {
                types.iter().any(|item| item == kind)
            })
        }
        TomlOperation::DiffCsv => compare_toml_leaves(source),
        TomlOperation::TypeCountsCsv => count_toml_leaf_types(parse_toml_table(source)?),
        TomlOperation::Json => write_toml_json(&toml::Value::Table(parse_toml_table(source)?)),
        TomlOperation::Select => select_toml_value(source),
        TomlOperation::SelectPaths => select_toml_paths(source),
        TomlOperation::FromJson => {
            let table = json_table(source.trim_start(), 0)?;
            let output =
                toml::to_string_pretty(&table).map_err(|_| TomlError::TomlSerialization)?;
            if output.len() > MAX_TOML_BYTES {
                return Err(TomlError::TooLarge);
            }
            Ok(output)
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum TypeArgument {
    One(String),
    Many(Vec<String>),
}

fn json_table(input: &str, depth: usize) -> Result<toml::Table, TomlError> {
    read_unique_object_entries(input)
        .map_err(TomlError::Json)?
        .into_iter()
        .map(|(key, value)| Ok((key, value_from_json(value.get(), depth + 1)?)))
        .collect()
}

fn value_from_json(input: &str, depth: usize) -> Result<toml::Value, TomlError> {
    use toml::Value as TomlValue;
    if depth > MAX_JSON_DEPTH {
        return Err(TomlError::JsonDepth);
    }
    match input.as_bytes().first() {
        Some(b'{') => json_table(input, depth).map(TomlValue::Table),
        Some(b'[') => read_json_array(input)
            .map_err(TomlError::Json)?
            .into_iter()
            .map(|value| value_from_json(value.get(), depth + 1))
            .collect::<Result<_, _>>()
            .map(TomlValue::Array),
        Some(b'"') => serde_json::from_str(input)
            .map(TomlValue::String)
            .map_err(|_| TomlError::Json(JsonError::Invalid)),
        Some(b't') => Ok(TomlValue::Boolean(true)),
        Some(b'f') => Ok(TomlValue::Boolean(false)),
        Some(b'n') => Err(TomlError::JsonNull),
        _ if input.contains(['.', 'e', 'E']) => {
            let value = input
                .parse::<f64>()
                .map_err(|_| TomlError::JsonFloatRange)?;
            let mantissa = input.split(['e', 'E']).next().unwrap_or(input);
            let underflowed =
                value == 0. && mantissa.bytes().any(|digit| matches!(digit, b'1'..=b'9'));
            if !value.is_finite() || underflowed {
                return Err(TomlError::JsonFloatRange);
            }
            Ok(TomlValue::Float(value))
        }
        _ => input
            .parse()
            .map(TomlValue::Integer)
            .map_err(|_| TomlError::JsonIntegerRange),
    }
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
    let root = toml::Value::Table(parse_toml_table(source)?);
    write_toml_json(toml_path_value(&root, &keys)?)
}

fn select_toml_paths(input: &str) -> Result<String, TomlError> {
    let (pointers, source) = selected_pointers(input).map_err(|error| match error {
        JsonError::Incomplete => TomlError::Incomplete,
        _ => TomlError::Json(error),
    })?;
    let root = toml::Value::Table(parse_toml_table(source)?);
    let mut json = String::from("{");
    for (index, pointer) in pointers.into_iter().enumerate() {
        let keys = json_pointer_tokens(&pointer).map_err(|_| TomlError::InvalidPointer)?;
        let key = serde_json::to_string(&pointer).map_err(|_| TomlError::Serialization)?;
        let value = write_toml_json(toml_path_value(&root, &keys)?)?;
        let separator = usize::from(index > 0);
        if json.len() + separator + key.len() + value.len() + 2 > MAX_TOML_BYTES {
            return Err(TomlError::TooLarge);
        }
        if index > 0 {
            json.push(',');
        }
        json.push_str(&key);
        json.push(':');
        json.push_str(&value);
    }
    json.push('}');
    Ok(json)
}

fn toml_path_value<'a>(
    root: &'a toml::Value,
    keys: &[String],
) -> Result<&'a toml::Value, TomlError> {
    let mut value = root;
    for key in keys {
        value = match value {
            toml::Value::Table(fields) => fields.get(key),
            toml::Value::Array(items) => json_array_index(key)
                .ok()
                .and_then(|index| items.get(index)),
            _ => None,
        }
        .ok_or(TomlError::MissingPointer)?;
    }
    Ok(value)
}

fn write_toml_json(value: &toml::Value) -> Result<String, TomlError> {
    let json = serde_json::to_string(&json_value(value)?).map_err(|_| TomlError::Serialization)?;
    if json.len() > MAX_TOML_BYTES {
        return Err(TomlError::TooLarge);
    }
    Ok(json)
}

fn toml_leaves_csv(
    table: toml::Table,
    include: impl Fn(&str, &str, &str) -> bool,
) -> Result<String, TomlError> {
    let mut output = String::from("路径,类型,值");
    visit_toml_leaves(table, |path, kind, value| {
        if !include(path, kind, &value) {
            return Ok(());
        }
        if path.contains('\0') || value.contains('\0') {
            return Err(TomlError::CsvNullCharacter);
        }
        append_bounded_csv_row(
            &mut output,
            [path, kind, value.as_str()].into_iter(),
            TomlError::TooLarge,
        )
    })?;
    Ok(output)
}

fn visit_toml_leaves(
    table: toml::Table,
    mut visit: impl FnMut(&str, &'static str, String) -> Result<(), TomlError>,
) -> Result<(), TomlError> {
    use toml::Value as TomlValue;

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
        visit(&path, kind, value)?;
    }
    Ok(())
}

fn count_toml_leaf_types(table: toml::Table) -> Result<String, TomlError> {
    let mut counts = BTreeMap::<&str, usize>::new();
    visit_toml_leaves(table, |path, kind, _| {
        if !path.is_empty() {
            *counts.entry(kind).or_default() += 1;
        }
        Ok(())
    })?;
    let mut output = String::from("类型,数量");
    for (kind, count) in counts {
        append_bounded_csv_row(
            &mut output,
            [kind, &count.to_string()].into_iter(),
            TomlError::TooLarge,
        )?;
    }
    Ok(output)
}

type TomlLeafCell = (&'static str, String);

fn compare_toml_leaves(source: &str) -> Result<String, TomlError> {
    let documents =
        serde_json::from_str::<[String; 2]>(source).map_err(|_| TomlError::ComparisonArguments)?;
    let mut changes = BTreeMap::<String, [Option<TomlLeafCell>; 2]>::new();
    let mut leaf_bytes = 0;
    for (side, document) in documents.into_iter().enumerate() {
        let table = parse_toml_table(&document).map_err(|error| match error {
            TomlError::Invalid { line, column, .. } => TomlError::ComparisonInvalid {
                after: side == 1,
                line,
                column,
            },
            _ => error,
        })?;
        visit_toml_leaves(table, |path, kind, value| {
            if path.is_empty() {
                return Ok(());
            }
            leaf_bytes += path.len() + kind.len() + value.len();
            if leaf_bytes > MAX_TOML_BYTES {
                return Err(TomlError::TooLarge);
            }
            changes.entry(path.to_owned()).or_default()[side] = Some((kind, value));
            Ok(())
        })?;
    }
    let mut output = String::from("路径,变更,原类型,原值,新类型,新值");
    for (path, [before, after]) in changes {
        if before == after {
            continue;
        }
        let change = match (&before, &after) {
            (None, _) => "新增",
            (_, None) => "删除",
            _ => "修改",
        };
        let (old_kind, old_value) = before.unwrap_or_default();
        let (new_kind, new_value) = after.unwrap_or_default();
        if [&path, &old_value, &new_value]
            .iter()
            .any(|value| value.contains('\0'))
        {
            return Err(TomlError::CsvNullCharacter);
        }
        append_bounded_csv_row(
            &mut output,
            [
                path.as_str(),
                change,
                old_kind,
                old_value.as_str(),
                new_kind,
                new_value.as_str(),
            ]
            .into_iter(),
            TomlError::TooLarge,
        )?;
    }
    Ok(output)
}

fn json_value(value: &toml::Value) -> Result<Value, TomlError> {
    Ok(match value {
        toml::Value::String(text) => Value::String(text.clone()),
        toml::Value::Integer(number) => Value::Number((*number).into()),
        toml::Value::Float(number) => {
            Value::Number(Number::from_f64(*number).ok_or(TomlError::NonFiniteFloat)?)
        }
        toml::Value::Boolean(flag) => Value::Bool(*flag),
        toml::Value::Datetime(date) => Value::String(date.to_string()),
        toml::Value::Array(items) => {
            Value::Array(items.iter().map(json_value).collect::<Result<_, _>>()?)
        }
        toml::Value::Table(table) => Value::Object(
            table
                .iter()
                .map(|(key, value)| json_value(value).map(|value| (key.clone(), value)))
                .collect::<Result<Map<_, _>, _>>()?,
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_json_objects_to_toml_with_exact_integer_bounds() {
        let source = r#"{"server":{"名称":"北","ports":[8000,8001]},"id":9007199254740993,"min":-9223372036854775808,"max":9223372036854775807,"array":[true,-0.0,"2024-02-29",{"k":"hello\nworld"}]}"#;
        let text = transform_toml(&format!("从JSON {source}")).unwrap();
        let table = text.parse::<toml::Table>().unwrap();
        assert_eq!(table["id"].as_integer(), Some(9007199254740993));
        assert_eq!(table["min"].as_integer(), Some(i64::MIN));
        assert_eq!(table["max"].as_integer(), Some(i64::MAX));
        let items = table["array"].as_array().unwrap();
        assert_eq!(items[1].as_float().unwrap().to_bits(), (-0.0_f64).to_bits());
        assert_eq!(items[2].as_str(), Some("2024-02-29"));
        let json = transform_toml(&format!("转JSON {text}")).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&json).unwrap(),
            serde_json::from_str::<Value>(source).unwrap()
        );
        for (source, expected) in [
            ("[]", TomlError::Json(JsonError::ExpectedObject)),
            (r#"{"x":[null]}"#, TomlError::JsonNull),
            (
                r#"{"x":{"a":1,"a":2}}"#,
                TomlError::Json(JsonError::DuplicateKey),
            ),
            (r#"{"x":9223372036854775808}"#, TomlError::JsonIntegerRange),
            (
                r#"{"x":-18446744073709551616}"#,
                TomlError::JsonIntegerRange,
            ),
            (r#"{"x":1e400}"#, TomlError::JsonFloatRange),
            (r#"{"x":1e-400}"#, TomlError::JsonFloatRange),
        ] {
            assert_eq!(
                transform_toml(&format!("from-json {source}")),
                Err(expected)
            );
        }
    }

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
    fn selects_multiple_toml_paths_in_request_order_as_json() {
        let data = "\ndate = 2024-02-29\nweight = nan\n[server]\nports = [8000, 9007199254740993]\n\"a/b~\" = true";
        let query = format!(r#"取多路径 ["/server/ports/1","/date","/server/a~1b~0"]{data}"#);
        assert_eq!(
            transform_toml(&query).as_deref(),
            Ok(
                r#"{"/server/ports/1":9007199254740993,"/date":"2024-02-29","/server/a~1b~0":true}"#
            )
        );
        assert!(produces_json(&query));
        for (paths, error) in [
            (
                r#"["/date","/date"]"#,
                TomlError::Json(JsonError::PointerSelection),
            ),
            (r#"["/server/~2"]"#, TomlError::InvalidPointer),
            (r#"["/date","/server/missing"]"#, TomlError::MissingPointer),
            (r#"["/weight"]"#, TomlError::NonFiniteFloat),
        ] {
            assert_eq!(
                transform_toml(&format!("get-paths {paths}{data}")),
                Err(error)
            );
        }
    }

    #[test]
    fn finds_toml_leaves_by_escaped_path_including_array_indices() {
        let query = concat!(
            "正则叶子CSV \"^/server/(port|ports/[0-9]+)$\"\r\n",
            "[server]\nname = 'api'\nports = [8001, 8002]\nport = 8000",
        );
        let expected = concat!(
            "路径,类型,值\r\n/server/port,integer,8000",
            "\r\n/server/ports/0,integer,8001\r\n/server/ports/1,integer,8002",
        );
        assert_eq!(transform_toml(query).as_deref(), Ok(expected));
        assert_eq!(toml_csv_report(query).unwrap().as_deref(), Ok(expected));
        let escaped = "regex-leaves-csv \"^/a~1b/x~0y$\"\n[\"a/b\"]\n\"x~y\" = true";
        assert_eq!(
            transform_toml(escaped).as_deref(),
            Ok("路径,类型,值\r\n/a~1b/x~0y,boolean,true")
        );
        assert_eq!(
            transform_toml("正则叶子CSV \"[\" port = 8000"),
            Err(TomlError::RegexPattern)
        );
    }

    #[test]
    fn finds_only_decoded_toml_string_values_by_regular_expression() {
        let query = concat!(
            "正则字符串CSV \"^(北|8000|2024)\"\n",
            "count = 8000\n",
            "date = 2024-02-29\n",
            "labels = [\"\\u5317京\", '南昌', '8000']\n",
            "[server]\n",
            "name = '北京'\n",
        );
        let expected = concat!(
            "路径,类型,值\r\n/labels/0,string,北京",
            "\r\n/labels/2,string,8000\r\n/server/name,string,北京",
        );
        assert_eq!(transform_toml(query).as_deref(), Ok(expected));
        assert_eq!(toml_csv_report(query).unwrap().as_deref(), Ok(expected));
        assert_eq!(
            transform_toml("regex-strings-csv \"^other$\" name = '北京'").as_deref(),
            Ok("路径,类型,值")
        );
    }

    #[test]
    fn finds_toml_leaves_by_native_types_in_report_order() {
        let source = concat!(
            "first = [8000, '8001', [8002]]\n",
            "enabled = true\n",
            "date = 2024-02-29\n",
            "ratio = 2.5\n",
            "empty = []\n",
            "[group]\n",
        );
        let query = format!(r#"找类型CSV ["integer","datetime","array"] {source}"#);
        let expected = concat!(
            "路径,类型,值\r\n/date,datetime,2024-02-29\r\n/empty,array,[]",
            "\r\n/first/0,integer,8000\r\n/first/2/0,integer,8002",
        );
        assert_eq!(transform_toml(&query).as_deref(), Ok(expected));
        assert_eq!(toml_csv_report(&query).unwrap().as_deref(), Ok(expected));
        assert_eq!(
            transform_toml(&format!(r#"find-type-csv "table" {source}"#)).as_deref(),
            Ok("路径,类型,值\r\n/group,table,{}")
        );
        for types in ["[]", "123", r#"["number"]"#, r#"["integer",null]"#] {
            assert_eq!(
                transform_toml(&format!("找类型CSV {types} {source}")),
                Err(TomlError::TypeSearchArguments)
            );
        }
    }

    #[test]
    fn compares_toml_values_and_types_by_escaped_leaf_path() {
        let before = "gone = '旧,值'\nsame = 1.0\nswitch = 2\n[\"a/b\"]\n\"x~y\" = true";
        let after = "new = 2024-02-29\nsame = 1e0\nswitch = '2'\n[\"a/b\"]\n\"x~y\" = false";
        let source = serde_json::to_string(&[before, after]).unwrap();
        let query = format!("对比CSV {source}");
        let expected = concat!(
            "路径,变更,原类型,原值,新类型,新值",
            "\r\n/a~1b/x~0y,修改,boolean,true,boolean,false",
            "\r\n/gone,删除,string,\"旧,值\",\"\",\"\"",
            "\r\n/new,新增,\"\",\"\",datetime,2024-02-29",
            "\r\n/switch,修改,integer,2,string,2",
        );
        assert_eq!(transform_toml(&query).as_deref(), Ok(expected));
        assert_eq!(toml_csv_report(&query).unwrap().as_deref(), Ok(expected));
        assert_eq!(
            transform_toml(r#"diff-csv ["","a = 1"]"#).as_deref(),
            Ok("路径,变更,原类型,原值,新类型,新值\r\n/a,新增,\"\",\"\",integer,1"),
        );
        assert_eq!(
            transform_toml(r#"对比CSV ["a = 1","a ="]"#),
            Err(TomlError::ComparisonInvalid {
                after: true,
                line: 1,
                column: 4
            }),
        );
    }

    #[test]
    fn counts_toml_leaf_types_without_counting_populated_containers() {
        let query = concat!(
            "类型统计CSV\n",
            "port = 8000\nports = [8001, 8002]\nlabels = ['甲', '乙']\n",
            "empty_array = []\n",
            "[zone]\nactive = true\n",
            "[empty_table]\n",
        );
        let expected = concat!(
            "类型,数量\r\narray,1\r\nboolean,1\r\ninteger,3",
            "\r\nstring,2\r\ntable,1",
        );
        assert_eq!(transform_toml(query).as_deref(), Ok(expected));
        assert_eq!(toml_csv_report(query).unwrap().as_deref(), Ok(expected));
        assert_eq!(
            transform_toml("type-counts-csv # only a comment").as_deref(),
            Ok("类型,数量")
        );
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
