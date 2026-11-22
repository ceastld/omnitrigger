use std::{
    borrow::Cow,
    cmp::Ordering,
    collections::{HashMap, HashSet},
    fmt,
};

use num_bigint::BigInt;
use serde::de::{IgnoredAny, MapAccess, Visitor};
use serde_json::value::RawValue;

use super::csv_tools::csv_cell;

const MAX_PRETTY_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum JsonError {
    Incomplete,
    UnknownOperation,
    Invalid,
    LayoutTooLarge,
    InvalidPointer,
    InvalidPointerSegment,
    PointerListTooLarge,
    FlattenedObjectTooLarge,
    ExpectedObject,
    ExpectedCollection,
    ExpectedArray,
    ExpectedObjectItem,
    InvalidLine,
    MissingValue,
    DuplicateKey,
    KeyArguments,
    MissingKey,
    RenameConflict,
    CsvRows,
    CsvDuplicateKey,
    CsvNullCharacter,
    SelectionKeys,
    SelectionConflict,
    FilterValue,
    ExpectedScalar,
    ExpectedNumber,
}

impl JsonError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "输入操作和 JSON 文本",
            Self::UnknownOperation => "输入 ? JSON 查看可用操作",
            Self::Invalid => "JSON 语法无效",
            Self::LayoutTooLarge => "JSON 格式化结果超过 1 MiB",
            Self::InvalidPointer => "JSON 路径无效，示例 /商品/0/价格",
            Self::InvalidPointerSegment => "指针分段须为字符串或非负整数",
            Self::PointerListTooLarge => "指针列表超过 1 MiB",
            Self::FlattenedObjectTooLarge => "JSON 展平结果超过 1 MiB",
            Self::ExpectedObject => "请输入 JSON 对象",
            Self::ExpectedCollection => "请输入 JSON 数组或对象以统计顶层条目数",
            Self::ExpectedArray => "请输入 JSON 数组",
            Self::ExpectedObjectItem => "数组中的每一项都应是 JSON 对象",
            Self::InvalidLine => "每个非空行需要一个完整的 JSON 值",
            Self::MissingValue => "JSON 路径没有对应的值",
            Self::DuplicateKey => "JSON 路径遇到了重复的键",
            Self::KeyArguments => "用 JSON 字符串数组填写键名，改键需要两个键名",
            Self::MissingKey => "顶层对象中找不到要改名的键",
            Self::RenameConflict => "改键要求原键唯一且新键名可用",
            Self::CsvRows => "输入等宽且有数据列的数组，或至少有一个键的对象列表",
            Self::CsvDuplicateKey => "同一对象的 CSV 列名需要唯一",
            Self::CsvNullCharacter => "CSV 单元格不能包含空字符 U+0000",
            Self::SelectionKeys => "选键需要至少一个互不重复的键名",
            Self::SelectionConflict => "待选键在同一对象中重复出现",
            Self::FilterValue => "路径后请输入 JSON 字符串、数字、布尔值或 null，再粘贴数组",
            Self::ExpectedScalar => "指定路径的值须为字符串、数字、布尔值或 null",
            Self::ExpectedNumber => "数值排序需要每一项的指定路径都对应 JSON 数字",
        }
    }
}

pub(super) fn transform_json(query: &str) -> Result<String, JsonError> {
    let (operation, source) = query
        .split_once(char::is_whitespace)
        .ok_or(JsonError::Incomplete)?;
    let source = source.trim_start();
    let pretty = match operation.to_ascii_lowercase().as_str() {
        "格式化" | "format" | "pretty" => true,
        "压缩" | "compact" | "minify" => false,
        "键排序" | "sort-keys" => return sort_json_keys(source),
        "取值" | "get" | "pointer" => return select_json_value(source),
        "取列" | "column" => return select_json_column(source),
        "有路径" | "has-pointer" => return filter_json_by_pointer(source, true),
        "缺路径" | "missing-pointer" => return filter_json_by_pointer(source, false),
        "筛选" | "filter" => return filter_json_by_scalar(source, true),
        "排除" | "exclude" => return filter_json_by_scalar(source, false),
        "按值去重" | "unique-by-pointer" => return deduplicate_json_by_scalar(source),
        "数值排序" | "sort-numeric" => return sort_json_numbers(source, false),
        "数值降序" | "sort-numeric-desc" => return sort_json_numbers(source, true),
        "键名" | "keys" => return json_keys(source),
        "长度" | "length" => return json_length(source),
        "展开数组" | "to-jsonl" => return json_array_to_lines(source),
        "合并行" | "from-jsonl" => return json_lines_to_array(source),
        "合并对象" | "merge-objects" => return merge_json_objects(source),
        "生成指针" | "build-pointer" => return build_json_pointer(source),
        "解析指针" | "parse-pointer" => return parse_json_pointer(source),
        "列指针" | "列路径" | "list-pointers" => return write_pointer_leaves(source, false),
        "展平" | "flatten" => return write_pointer_leaves(source, true),
        "改键" | "rename-key" => return rename_json_key(source),
        "去键" | "remove-keys" => return remove_json_keys(source),
        "转csv" | "to-csv" => return json_to_csv(source),
        "选键" | "pick-keys" => return pick_json_keys(source),
        _ => return Err(JsonError::UnknownOperation),
    };
    if source.is_empty() {
        return Err(JsonError::Incomplete);
    }
    // 通过解析验证语法，再调整原文空白，保留键顺序、数字和字符串拼写。
    serde_json::from_str::<IgnoredAny>(source).map_err(json_parse_error)?;
    layout_json(source, pretty)
}

fn layout_json(source: &str, pretty: bool) -> Result<String, JsonError> {
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
    let items = read_json_array(source)?;
    let keys = pointer_keys(&pointer)?.collect::<Result<Vec<_>, _>>()?;
    let values = items
        .into_iter()
        .map(|item| select_pointer_keys(item, &keys))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(serde_json::to_string(&values).expect("有效的原始 JSON 值可以直接序列化"))
}

fn filter_json_by_pointer(query: &str, keep_present: bool) -> Result<String, JsonError> {
    let (pointer, source) = pointer_and_source(query)?;
    filter_json_items(&pointer, source, keep_present, |value| Ok(value.is_some()))
}

fn filter_json_by_scalar(query: &str, keep_match: bool) -> Result<String, JsonError> {
    let (pointer, query) = pointer_and_source(query)?;
    let mut values = serde_json::Deserializer::from_str(query).into_iter::<&RawValue>();
    let value = values
        .next()
        .ok_or(JsonError::Incomplete)?
        .map_err(|error| {
            if error.is_eof() {
                JsonError::Incomplete
            } else {
                JsonError::FilterValue
            }
        })?;
    let source = &query[values.byte_offset()..];
    if source.is_empty() {
        return Err(JsonError::Incomplete);
    }
    if !source.starts_with(char::is_whitespace) {
        return Err(JsonError::FilterValue);
    }
    let expected = json_scalar(value)?.ok_or(JsonError::FilterValue)?;
    filter_json_items(&pointer, source.trim_start(), keep_match, |value| {
        Ok(match value {
            Some(value) => json_scalar(value)?.is_some_and(|scalar| scalar == expected),
            None => false,
        })
    })
}

fn deduplicate_json_by_scalar(query: &str) -> Result<String, JsonError> {
    let (pointer, source) = pointer_and_source(query)?;
    let mut seen = HashSet::new();
    filter_json_items(&pointer, source, true, |value| {
        let value = value.ok_or(JsonError::MissingValue)?;
        let scalar = json_scalar(value)?.ok_or(JsonError::ExpectedScalar)?;
        Ok(seen.insert(scalar))
    })
}

fn sort_json_numbers(query: &str, descending: bool) -> Result<String, JsonError> {
    let (pointer, source) = pointer_and_source(query)?;
    let keys = pointer_keys(&pointer)?.collect::<Result<Vec<_>, _>>()?;
    let mut items = read_json_array(source)?
        .into_iter()
        .map(|item| {
            let value = select_pointer_keys(item, &keys)?;
            let Some(JsonScalar::Number {
                negative,
                digits,
                exponent,
            }) = json_scalar(value)?
            else {
                return Err(JsonError::ExpectedNumber);
            };
            let sign = if digits.is_empty() {
                0
            } else if negative {
                -1
            } else {
                1
            };
            let decimal_position = exponent + BigInt::from(digits.len());
            Ok((
                JsonNumericSortKey {
                    sign,
                    decimal_position,
                    digits,
                },
                item,
            ))
        })
        .collect::<Result<Vec<_>, _>>()?;
    items.sort_by(|(left, _), (right, _)| {
        if descending {
            right.compare(left)
        } else {
            left.compare(right)
        }
    });
    let values: Vec<_> = items.into_iter().map(|(_, item)| item).collect();
    Ok(serde_json::to_string(&values).expect("有效的原始 JSON 值可以直接序列化"))
}

struct JsonNumericSortKey {
    sign: i8,
    decimal_position: BigInt,
    digits: String,
}

impl JsonNumericSortKey {
    fn compare(&self, other: &Self) -> Ordering {
        self.sign.cmp(&other.sign).then_with(|| {
            // 相同位置的有效数位可直接比较，末尾的零已在规范化时移除。
            let magnitude = self
                .decimal_position
                .cmp(&other.decimal_position)
                .then_with(|| self.digits.cmp(&other.digits));
            if self.sign < 0 {
                magnitude.reverse()
            } else {
                magnitude
            }
        })
    }
}

fn filter_json_items(
    pointer: &str,
    source: &str,
    keep_match: bool,
    mut matches_value: impl FnMut(Option<&RawValue>) -> Result<bool, JsonError>,
) -> Result<String, JsonError> {
    let items = read_json_array(source)?;
    let keys = pointer_keys(pointer)?.collect::<Result<Vec<_>, _>>()?;
    let mut retained = Vec::new();
    for item in items {
        let value = match select_pointer_keys(item, &keys) {
            Ok(value) => Some(value),
            Err(JsonError::MissingValue) => None,
            Err(error) => return Err(error),
        };
        if matches_value(value)? == keep_match {
            retained.push(item);
        }
    }
    Ok(serde_json::to_string(&retained).expect("有效的原始 JSON 值可以直接序列化"))
}

#[derive(PartialEq, Eq, Hash)]
enum JsonScalar {
    String(String),
    Number {
        negative: bool,
        digits: String,
        exponent: BigInt,
    },
    Boolean(bool),
    Null,
}

fn json_scalar(value: &RawValue) -> Result<Option<JsonScalar>, JsonError> {
    let raw = value.get();
    let scalar = match raw.as_bytes()[0] {
        b'{' | b'[' => return Ok(None),
        b'"' => JsonScalar::String(serde_json::from_str(raw).map_err(json_parse_error)?),
        b't' | b'f' => JsonScalar::Boolean(raw == "true"),
        b'n' => JsonScalar::Null,
        _ => {
            let (mantissa, exponent) = raw.split_once(['e', 'E']).unwrap_or((raw, "0"));
            let negative = mantissa.starts_with('-');
            let mantissa = mantissa.strip_prefix('-').unwrap_or(mantissa);
            let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
            let all_digits = format!("{whole}{fraction}");
            let without_trailing_zeros = all_digits.trim_end_matches('0');
            let digits = without_trailing_zeros.trim_start_matches('0');
            let exponent = if digits.is_empty() {
                BigInt::default()
            } else {
                // 将十进制位置留在指数中，避免展开很长的零串。
                let exponent: BigInt = exponent.parse().map_err(|_| JsonError::Invalid)?;
                exponent + BigInt::from(all_digits.len() - without_trailing_zeros.len())
                    - BigInt::from(fraction.len())
            };
            JsonScalar::Number {
                negative: negative && !digits.is_empty(),
                digits: digits.to_owned(),
                exponent,
            }
        }
    };
    Ok(Some(scalar))
}

fn select_pointer_keys<'a>(item: &'a RawValue, keys: &[String]) -> Result<&'a RawValue, JsonError> {
    keys.iter()
        .try_fold(item, |value, key| select_member(value, key))
}

fn read_json_array(source: &str) -> Result<Vec<&RawValue>, JsonError> {
    if !source.starts_with('[') {
        serde_json::from_str::<IgnoredAny>(source).map_err(json_parse_error)?;
        return Err(JsonError::ExpectedArray);
    }
    serde_json::from_str(source).map_err(json_parse_error)
}

fn json_array_to_lines(source: &str) -> Result<String, JsonError> {
    let lines = read_json_array(source)?
        .into_iter()
        .map(|value| layout_json(value.get(), false))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(lines.join("\n"))
}

fn json_lines_to_array(source: &str) -> Result<String, JsonError> {
    if source.is_empty() {
        return Err(JsonError::Incomplete);
    }
    let values = source
        .lines()
        .filter(|line| !line.trim_matches([' ', '\t', '\r']).is_empty())
        .map(|line| {
            let value =
                serde_json::from_str::<&RawValue>(line).map_err(|_| JsonError::InvalidLine)?;
            layout_json(value.get(), false)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(format!("[{}]", values.join(",")))
}

fn merge_json_objects(source: &str) -> Result<String, JsonError> {
    let mut members = HashMap::new();
    for object in read_json_array(source)? {
        let raw = object.get();
        if !raw.starts_with('{') {
            return Err(JsonError::ExpectedObjectItem);
        }
        for (key, value) in read_object_entries(raw)? {
            let next_position = members.len();
            members
                .entry(key)
                .and_modify(|(_, previous)| *previous = value)
                .or_insert((next_position, value));
        }
    }
    let mut ordered: Vec<_> = members.into_iter().collect();
    ordered.sort_unstable_by_key(|(_, (position, _))| *position);
    write_object_entries(ordered.into_iter().map(|(key, (_, value))| (key, value)))
}

fn read_object_entries(source: &str) -> Result<Vec<(String, &RawValue)>, JsonError> {
    if !source.starts_with('{') {
        serde_json::from_str::<IgnoredAny>(source).map_err(json_parse_error)?;
        return Err(JsonError::ExpectedObject);
    }
    let mut parser = serde_json::Deserializer::from_str(source);
    let entries = serde::Deserializer::deserialize_map(&mut parser, ObjectEntries)
        .map_err(json_parse_error)?;
    parser.end().map_err(json_parse_error)?;
    Ok(entries)
}

fn sort_json_keys(source: &str) -> Result<String, JsonError> {
    enum Part<'a> {
        Value(&'a RawValue),
        Key(String),
        Symbol(char),
    }

    let root = serde_json::from_str::<&RawValue>(source).map_err(json_parse_error)?;
    let mut pending = vec![Part::Value(root)];
    let mut result = String::new();
    while let Some(part) = pending.pop() {
        match part {
            Part::Symbol(symbol) => result.push(symbol),
            Part::Key(key) => {
                let key = serde_json::to_string(&key).expect("JSON 键名可以序列化");
                result.push_str(&key);
            }
            Part::Value(value) => {
                let raw = value.get();
                match raw.as_bytes()[0] {
                    b'{' => {
                        let mut entries = read_object_entries(raw)?;
                        entries.sort_unstable_by(|left, right| left.0.cmp(&right.0));
                        if entries.windows(2).any(|pair| pair[0].0 == pair[1].0) {
                            return Err(JsonError::DuplicateKey);
                        }
                        result.push('{');
                        pending.push(Part::Symbol('}'));
                        for (index, (key, child)) in entries.into_iter().enumerate().rev() {
                            pending.push(Part::Value(child));
                            pending.push(Part::Symbol(':'));
                            pending.push(Part::Key(key));
                            if index != 0 {
                                pending.push(Part::Symbol(','));
                            }
                        }
                    }
                    b'[' => {
                        let elements = read_json_array(raw)?;
                        result.push('[');
                        pending.push(Part::Symbol(']'));
                        for (index, child) in elements.into_iter().enumerate().rev() {
                            pending.push(Part::Value(child));
                            if index != 0 {
                                pending.push(Part::Symbol(','));
                            }
                        }
                    }
                    _ => result.push_str(raw),
                }
            }
        }
        if result.len() > MAX_PRETTY_BYTES {
            return Err(JsonError::LayoutTooLarge);
        }
    }
    Ok(result)
}

fn write_object_entries<'a>(
    entries: impl IntoIterator<Item = (String, &'a RawValue)>,
) -> Result<String, JsonError> {
    let entries = entries
        .into_iter()
        .map(|(key, value)| {
            let key = serde_json::to_string(&key).expect("JSON 键名可以序列化");
            let value = layout_json(value.get(), false)?;
            Ok(format!("{key}:{value}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(format!("{{{}}}", entries.join(",")))
}

fn key_names_and_source(query: &str) -> Result<(Vec<String>, &str), JsonError> {
    let mut arguments = serde_json::Deserializer::from_str(query).into_iter::<Vec<String>>();
    let names = arguments
        .next()
        .ok_or(JsonError::Incomplete)?
        .map_err(|error| {
            if error.is_eof() {
                JsonError::Incomplete
            } else {
                JsonError::KeyArguments
            }
        })?;
    let source = query[arguments.byte_offset()..].trim_start();
    if source.is_empty() {
        return Err(JsonError::Incomplete);
    }
    Ok((names, source))
}

fn rename_json_key(query: &str) -> Result<String, JsonError> {
    let (names, source) = key_names_and_source(query)?;
    let [original, replacement] = names.as_slice() else {
        return Err(JsonError::KeyArguments);
    };
    let mut entries = read_object_entries(source)?;
    let mut matches = entries
        .iter()
        .enumerate()
        .filter(|(_, (key, _))| key == original);
    let (position, _) = matches.next().ok_or(JsonError::MissingKey)?;
    if matches.next().is_some()
        || (original != replacement && entries.iter().any(|(key, _)| key == replacement))
    {
        return Err(JsonError::RenameConflict);
    }
    entries[position].0.clone_from(replacement);
    write_object_entries(entries)
}

fn remove_json_keys(query: &str) -> Result<String, JsonError> {
    let (names, source) = key_names_and_source(query)?;
    if names.is_empty() {
        return Err(JsonError::KeyArguments);
    }
    let names: HashSet<_> = names.into_iter().collect();
    write_object_entries(
        read_object_entries(source)?
            .into_iter()
            .filter(|(key, _)| !names.contains(key)),
    )
}

fn json_to_csv(source: &str) -> Result<String, JsonError> {
    let rows = read_json_array(source)?;
    match rows.first().and_then(|row| row.get().chars().next()) {
        None => Ok(String::new()),
        Some('[') => json_array_rows_to_csv(rows),
        Some('{') => json_object_rows_to_csv(rows),
        _ => Err(JsonError::CsvRows),
    }
}

fn pick_json_keys(query: &str) -> Result<String, JsonError> {
    let (names, source) = key_names_and_source(query)?;
    let wanted: HashSet<_> = names.iter().map(String::as_str).collect();
    if names.is_empty() || names.len() != wanted.len() {
        return Err(JsonError::SelectionKeys);
    }
    let project = |raw: &str| {
        let mut selected = HashMap::new();
        for (key, value) in read_object_entries(raw)? {
            if wanted.contains(key.as_str()) && selected.insert(key, value).is_some() {
                return Err(JsonError::SelectionConflict);
            }
        }
        write_object_entries(names.iter().filter_map(|key| selected.remove_entry(key)))
    };
    if !source.starts_with('[') {
        return project(source);
    }
    let items = read_json_array(source)?
        .into_iter()
        .map(|item| {
            let raw = item.get();
            if raw.starts_with('{') {
                project(raw)
            } else {
                Err(JsonError::ExpectedObjectItem)
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(format!("[{}]", items.join(",")))
}

fn json_array_rows_to_csv(rows: Vec<&RawValue>) -> Result<String, JsonError> {
    let mut width = None;
    let mut output = Vec::with_capacity(rows.len());
    for row in rows {
        if !row.get().starts_with('[') {
            return Err(JsonError::CsvRows);
        }
        let cells = read_json_array(row.get())?;
        if cells.is_empty()
            || width
                .replace(cells.len())
                .is_some_and(|previous| previous != cells.len())
        {
            return Err(JsonError::CsvRows);
        }
        let record = cells
            .into_iter()
            .map(json_csv_cell)
            .collect::<Result<Vec<_>, _>>()?;
        output.push(record.join(","));
    }
    Ok(output.join("\r\n"))
}

fn json_object_rows_to_csv(rows: Vec<&RawValue>) -> Result<String, JsonError> {
    let mut order = Vec::new();
    let mut positions = HashMap::new();
    let mut entries = Vec::with_capacity(rows.len());
    for row in rows {
        if !row.get().starts_with('{') {
            return Err(JsonError::CsvRows);
        }
        let mut fields = HashMap::new();
        for (name, value) in read_object_entries(row.get())? {
            let next = positions.len();
            let position = *positions.entry(name.clone()).or_insert_with(|| {
                order.push(name);
                next
            });
            if fields.insert(position, value).is_some() {
                return Err(JsonError::CsvDuplicateKey);
            }
        }
        entries.push(fields);
    }
    if order.is_empty() {
        return Err(JsonError::CsvRows);
    }

    let mut output = Vec::with_capacity(entries.len() + 1);
    let headers = order
        .iter()
        .map(|name| encode_csv_cell(name))
        .collect::<Result<Vec<_>, _>>()?;
    output.push(headers.join(","));
    for fields in entries {
        let record = (0..order.len())
            .map(|index| match fields.get(&index) {
                Some(value) => json_csv_cell(value),
                None => encode_csv_cell(""),
            })
            .collect::<Result<Vec<_>, _>>()?;
        output.push(record.join(","));
    }
    Ok(output.join("\r\n"))
}

fn json_csv_cell(value: &RawValue) -> Result<String, JsonError> {
    let raw = value.get();
    if raw.starts_with('"') {
        let decoded = serde_json::from_str::<String>(raw).map_err(json_parse_error)?;
        return encode_csv_cell(&decoded);
    }
    if raw.starts_with(['{', '[']) {
        return encode_csv_cell(&layout_json(raw, false)?);
    }
    encode_csv_cell(if raw == "null" { "" } else { raw })
}

fn encode_csv_cell(value: &str) -> Result<String, JsonError> {
    if value.contains('\0') {
        return Err(JsonError::CsvNullCharacter);
    }
    Ok(csv_cell(value).into_owned())
}

fn build_json_pointer(source: &str) -> Result<String, JsonError> {
    let mut pointer = String::new();
    for segment in read_json_array(source)? {
        let raw = segment.get();
        let token = if raw.starts_with('"') {
            Cow::Owned(serde_json::from_str::<String>(raw).map_err(json_parse_error)?)
        } else if raw.bytes().all(|byte| byte.is_ascii_digit()) {
            Cow::Borrowed(raw)
        } else {
            return Err(JsonError::InvalidPointerSegment);
        };
        append_pointer_key(&mut pointer, &token);
    }
    if pointer.is_empty() || pointer.contains(|ch: char| ch.is_whitespace() || ch.is_control()) {
        serde_json::to_string(&pointer).map_err(json_parse_error)
    } else {
        Ok(pointer)
    }
}

fn append_pointer_key(pointer: &mut String, key: &str) {
    pointer.push('/');
    for ch in key.chars() {
        match ch {
            '~' => pointer.push_str("~0"),
            '/' => pointer.push_str("~1"),
            _ => pointer.push(ch),
        }
    }
}

fn write_pointer_leaves(source: &str, include_values: bool) -> Result<String, JsonError> {
    let (opening, closing, size_error) = if include_values {
        ('{', '}', JsonError::FlattenedObjectTooLarge)
    } else {
        ('[', ']', JsonError::PointerListTooLarge)
    };
    let root = serde_json::from_str::<&RawValue>(source).map_err(json_parse_error)?;
    let mut pending = vec![(root, None::<(usize, String)>)];
    let mut path = String::new();
    let mut result = String::from(opening);
    while let Some((value, key)) = pending.pop() {
        if let Some((parent_length, key)) = key {
            path.truncate(parent_length);
            append_pointer_key(&mut path, &key);
        }
        let raw = value.get();
        let children = match raw.as_bytes()[0] {
            b'{' => {
                let entries = read_object_entries(raw)?;
                let mut seen = HashSet::with_capacity(entries.len());
                if entries.iter().any(|(key, _)| !seen.insert(key)) {
                    return Err(JsonError::DuplicateKey);
                }
                entries
            }
            b'[' => read_json_array(raw)?
                .into_iter()
                .enumerate()
                .map(|(index, child)| (index.to_string(), child))
                .collect(),
            _ => Vec::new(),
        };
        if !children.is_empty() {
            for (key, child) in children.into_iter().rev() {
                pending.push((child, Some((path.len(), key))));
            }
            continue;
        }
        let encoded = serde_json::to_string(&path).map_err(json_parse_error)?;
        let has_previous = result.len() > 1;
        let value_length = if include_values { 1 + raw.len() } else { 0 };
        if result.len() + encoded.len() + value_length + usize::from(has_previous) + 1
            > MAX_PRETTY_BYTES
        {
            return Err(size_error);
        }
        if has_previous {
            result.push(',');
        }
        result.push_str(&encoded);
        if include_values {
            result.push(':');
            result.push_str(raw);
        }
    }
    result.push(closing);
    Ok(result)
}

fn parse_json_pointer(source: &str) -> Result<String, JsonError> {
    if source.is_empty() {
        return Err(JsonError::Incomplete);
    }
    let pointer = if source.starts_with('"') {
        Cow::Owned(serde_json::from_str::<String>(source).map_err(pointer_parse_error)?)
    } else {
        Cow::Borrowed(source)
    };
    let tokens = pointer_keys(&pointer)?.collect::<Result<Vec<_>, _>>()?;
    serde_json::to_string(&tokens).map_err(json_parse_error)
}

fn pointer_parse_error(error: serde_json::Error) -> JsonError {
    if error.is_eof() {
        JsonError::Incomplete
    } else {
        JsonError::InvalidPointer
    }
}

fn pointer_and_source(query: &str) -> Result<(String, &str), JsonError> {
    let (pointer, source) = if query.starts_with('"') {
        let mut strings = serde_json::Deserializer::from_str(query).into_iter::<String>();
        let pointer = strings
            .next()
            .ok_or(JsonError::Incomplete)?
            .map_err(pointer_parse_error)?;
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

struct ObjectEntries;

impl<'de> Visitor<'de> for ObjectEntries {
    type Value = Vec<(String, &'de RawValue)>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON object")
    }

    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
        let mut entries = Vec::new();
        while let Some(entry) = map.next_entry()? {
            entries.push(entry);
        }
        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_object_keys_through_arrays_and_preserves_raw_literal_values() {
        let input = r#"键排序 {"z":[{"b":9007199254740993,"a":1.20e2},{}],"\u0061":{"t":"\u0062","b":null},"A":[]}"#;
        assert_eq!(
            transform_json(input).as_deref(),
            Ok(
                r#"{"A":[],"a":{"b":null,"t":"\u0062"},"z":[{"a":1.20e2,"b":9007199254740993},{}]}"#
            )
        );
        assert_eq!(
            transform_json(r#"sort-keys [{"a":1,"\u0061":2}]"#),
            Err(JsonError::DuplicateKey)
        );
        let query = format!(
            "sort-keys {}{{\"z\":1,\"a\":2}}{}",
            "[".repeat(256),
            "]".repeat(256)
        );
        assert_eq!(
            transform_json(&query).unwrap(),
            format!("{}{{\"a\":2,\"z\":1}}{}", "[".repeat(256), "]".repeat(256))
        );
    }

    #[test]
    fn enumerates_resolvable_leaf_pointers_in_document_order() {
        let source = r#"{"商品":[{"a/b":9007199254740993},{"~":null},[]],"":"", "a\nb":{}}"#;
        let listed = write_pointer_leaves(source, false).unwrap();
        assert_eq!(
            listed,
            r#"["/商品/0/a~1b","/商品/1/~0","/商品/2","/","/a\nb"]"#
        );
        let paths: Vec<String> = serde_json::from_str(&listed).unwrap();
        for (path, expected) in paths
            .iter()
            .zip(["9007199254740993", "null", "[]", r#""""#, "{}"])
        {
            let path = serde_json::to_string(path).unwrap();
            assert_eq!(
                select_json_value(&format!("{path} {source}")).as_deref(),
                Ok(expected)
            );
        }
        for source in ["[]", "{}", "23", "null"] {
            assert_eq!(
                write_pointer_leaves(source, false).as_deref(),
                Ok(r#"[""]"#)
            );
        }
        assert_eq!(
            write_pointer_leaves(r#"{"a":{"x":1,"\u0078":2}}"#, false),
            Err(JsonError::DuplicateKey)
        );
        assert_eq!(write_pointer_leaves("[1,]", false), Err(JsonError::Invalid));
    }

    #[test]
    fn flattens_values_under_escaped_pointers_and_keeps_raw_literals() {
        let json = r#"{"商品":[{"a/b":9007199254740993},{"~":1.20e+2},[]],"":"\u0061","a\nb":{}}"#;
        assert_eq!(
            transform_json(&format!("展平 {json}")).as_deref(),
            Ok(
                r#"{"/商品/0/a~1b":9007199254740993,"/商品/1/~0":1.20e+2,"/商品/2":[],"/":"\u0061","/a\nb":{}}"#
            )
        );
        for (source, expected) in [
            ("12.30", r#"{"":12.30}"#),
            ("[]", r#"{"":[]}"#),
            ("{}", r#"{"":{}}"#),
        ] {
            assert_eq!(
                transform_json(&format!("flatten {source}")).as_deref(),
                Ok(expected)
            );
        }
        assert_eq!(
            transform_json(r#"展平 {"a":{"x":1,"\u0078":2}}"#),
            Err(JsonError::DuplicateKey)
        );
    }

    #[test]
    fn filters_json_items_by_pointer_presence_and_keeps_original_values() {
        let source = r#"[{"id":"001","a":{"b":null},"n":9007199254740993},{"id":"002","a": {}},{"id":"003","a":{"b":false}},null]"#;
        for (operation, expected) in [
            (
                "有路径",
                r#"[{"id":"001","a":{"b":null},"n":9007199254740993},{"id":"003","a":{"b":false}}]"#,
            ),
            ("missing-pointer", r#"[{"id":"002","a": {}},null]"#),
        ] {
            assert_eq!(
                transform_json(&format!("{operation} /a/b {source}")).as_deref(),
                Ok(expected)
            );
        }
        assert_eq!(
            transform_json(r#"has-pointer /0 [[null],[],[false]]"#).as_deref(),
            Ok("[[null],[false]]")
        );
        assert_eq!(
            transform_json(r#"有路径 "" [3,{},null]"#).as_deref(),
            Ok("[3,{},null]")
        );
        assert_eq!(
            transform_json("有路径 /a~2b []"),
            Err(JsonError::InvalidPointer)
        );
        assert_eq!(
            transform_json(r#"缺路径 /a [{"a":1,"a":2}]"#),
            Err(JsonError::DuplicateKey)
        );
    }

    #[test]
    fn filters_json_items_by_exact_scalar_value() {
        let source = r#"[{"id":"001","a/b":"完成"},{"id":"002","a/b":"\u5b8c\u6210"},{"id":"003"},{"id":"004","a/b":null},{"id":"005","a/b":["完成"]}]"#;
        assert_eq!(
            transform_json(&format!(r#"筛选 /a~1b "完成" {source}"#)).as_deref(),
            Ok(r#"[{"id":"001","a/b":"完成"},{"id":"002","a/b":"\u5b8c\u6210"}]"#)
        );
        assert_eq!(
            transform_json(&format!(r#"exclude /a~1b "完成" {source}"#)).as_deref(),
            Ok(r#"[{"id":"003"},{"id":"004","a/b":null},{"id":"005","a/b":["完成"]}]"#)
        );
        for (query, expected) in [
            (
                r#"filter "" 9007199254740993 [9007199254740992,"9007199254740993",9007199254740993.0,90071992547409930e-1]"#,
                "[9007199254740993.0,90071992547409930e-1]",
            ),
            (
                r#"filter "" 1e+99999999999999999999 [10e99999999999999999998,1e99999999999999999998]"#,
                "[10e99999999999999999998]",
            ),
            (
                r#"filter "" 0 [-0.00e-99999999999999999999,0,0.01,false]"#,
                "[-0.00e-99999999999999999999,0]",
            ),
            (
                r#"筛选 "/a b" null [{"a b":false},{"a b":null},{}]"#,
                r#"[{"a b":null}]"#,
            ),
            (r#"filter "" true [1,"true",true,false,null]"#, "[true]"),
        ] {
            assert_eq!(transform_json(query).as_deref(), Ok(expected));
        }
        for (query, error) in [
            ("筛选 /a {} []", JsonError::FilterValue),
            (r#"筛选 /a "完成"[]"#, JsonError::FilterValue),
            ("筛选 /a true", JsonError::Incomplete),
            ("筛选 /a null {}", JsonError::ExpectedArray),
            (r#"筛选 /a 1 [{"a":1,"a":2}]"#, JsonError::DuplicateKey),
        ] {
            assert_eq!(transform_json(query), Err(error), "{query}");
        }
    }

    #[test]
    fn keeps_the_first_json_item_for_each_exact_scalar_value() {
        let source = r#"[{"键":9007199254740993,"序":"001"},{"键":90071992547409930e-1,"序":"002"},{"键":9007199254740992,"序":"003"},{"键":"9007199254740993","序":"004"},{"键":null,"序":"005"},{"键":null,"序":"006"}]"#;
        assert_eq!(
            transform_json(&format!("按值去重 /键 {source}")).as_deref(),
            Ok(
                r#"[{"键":9007199254740993,"序":"001"},{"键":9007199254740992,"序":"003"},{"键":"9007199254740993","序":"004"},{"键":null,"序":"005"}]"#
            )
        );
        assert_eq!(
            transform_json(r#"unique-by-pointer "" ["茶","\u8336",0,-0.0,false,false]"#).as_deref(),
            Ok(r#"["茶",0,false]"#)
        );
        for (query, error) in [
            ("按值去重 /键 [{}]", JsonError::MissingValue),
            (r#"按值去重 /键 [{"键":[]}]"#, JsonError::ExpectedScalar),
            (r#"按值去重 /键 [{"键":1,"键":2}]"#, JsonError::DuplicateKey),
        ] {
            assert_eq!(transform_json(query), Err(error), "{query}");
        }
    }

    #[test]
    fn sorts_json_numbers_by_exact_magnitude_and_keeps_tied_items_in_order() {
        let values = "[0,-2.50,3e10000000000000000000,1.2,1e10000000000000000000,-1E-4,9007199254740993,9007199254740992,1.200,-0.0,1e-10000000000000000000]";
        for (operation, expected) in [
            ("数值排序", "[-2.50,-1E-4,0,-0.0,1e-10000000000000000000,1.2,1.200,9007199254740992,9007199254740993,1e10000000000000000000,3e10000000000000000000]"),
            ("sort-numeric-desc", "[3e10000000000000000000,1e10000000000000000000,9007199254740993,9007199254740992,1.2,1.200,1e-10000000000000000000,0,-0.0,-1E-4,-2.50]"),
        ] {
            assert_eq!(
                transform_json(&format!(r#"{operation} "" {values}"#)).as_deref(),
                Ok(expected)
            );
        }
        assert_eq!(
            transform_json(r#"数值排序 /n [{"n":1.001,"id":"A"},{"n":1,"id":"B"}]"#).as_deref(),
            Ok(r#"[{"n":1,"id":"B"},{"n":1.001,"id":"A"}]"#)
        );
        for (query, error) in [
            (r#"数值排序 /n [{"n":"1"}]"#, JsonError::ExpectedNumber),
            (r#"数值排序 /n [{}]"#, JsonError::MissingValue),
            (r#"数值降序 /n [{"n":1,"n":2}]"#, JsonError::DuplicateKey),
        ] {
            assert_eq!(transform_json(query), Err(error), "{query}");
        }
    }

    #[test]
    fn lists_deep_pointers_iteratively_and_bounds_the_serialized_result() {
        let depth = 256;
        let nested = format!("{}null{}", "[".repeat(depth), "]".repeat(depth));
        assert_eq!(
            write_pointer_leaves(&nested, false).unwrap(),
            format!(r#"["{}"]"#, "/0".repeat(depth))
        );
        let wide = format!(r#"{{"{}":[{}0]}}"#, "x".repeat(1500), "0,".repeat(849));
        assert_eq!(
            write_pointer_leaves(&wide, false),
            Err(JsonError::PointerListTooLarge)
        );
        assert_eq!(
            write_pointer_leaves(&wide, true),
            Err(JsonError::FlattenedObjectTooLarge)
        );
    }

    #[test]
    fn merges_object_arrays_in_first_key_order_with_the_last_raw_value() {
        let input = r#"合并对象 [{"z":9007199254740993,"a":[ 1 ]},{"a":1e3,"b":"\u0058","内层":{"x":1}},{"内层":{ "y":2 }}]"#;
        assert_eq!(
            transform_json(input).as_deref(),
            Ok(r#"{"z":9007199254740993,"a":1e3,"b":"\u0058","内层":{"y":2}}"#),
        );
        assert_eq!(
            transform_json(r#"merge-objects [{"\u0061":1,"a":2},{"a":3}]"#).as_deref(),
            Ok(r#"{"a":3}"#),
        );
        assert_eq!(transform_json("合并对象 []").as_deref(), Ok("{}"));
        assert_eq!(
            transform_json(r#"合并对象 [{"x":1},2]"#),
            Err(JsonError::ExpectedObjectItem),
        );
        assert_eq!(
            transform_json(r#"合并对象 {"x":1}"#),
            Err(JsonError::ExpectedArray),
        );
        assert_eq!(transform_json("合并对象 ["), Err(JsonError::Incomplete));
    }

    #[test]
    fn converts_json_lines_without_changing_values_or_splitting_strings() {
        let array = r#"[{"a":"\u5317\n京","a":9.90},9007199254740993,null]"#;
        let lines = "{\"a\":\"\\u5317\\n京\",\"a\":9.90}\n9007199254740993\nnull";
        assert_eq!(
            transform_json(&format!("展开数组 {array}")).as_deref(),
            Ok(lines)
        );
        assert_eq!(
            transform_json(&format!("合并行 {}", lines.replace('\n', "\r\n \t\r\n"))).as_deref(),
            Ok(array)
        );
        assert_eq!(
            transform_json("to-jsonl [ {\n \"k\": \"a b\" \n}, [1,2] ]").as_deref(),
            Ok("{\"k\":\"a b\"}\n[1,2]")
        );
        assert_eq!(
            transform_json("from-jsonl 1 2\n3"),
            Err(JsonError::InvalidLine)
        );
        assert_eq!(
            transform_json("合并行 {\n\"x\":1\n}"),
            Err(JsonError::InvalidLine)
        );
        assert_eq!(transform_json("展开数组 []").as_deref(), Ok(""));
        assert_eq!(transform_json("展开数组 {}"), Err(JsonError::ExpectedArray));
    }

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
    fn projects_one_object_or_all_object_rows_in_requested_key_order() {
        for (query, expected) in [
            (
                r#"选键 ["名","值"] {"值":2e2,"名":"\u4f60","调试":true}"#,
                r#"{"名":"\u4f60","值":2e2}"#,
            ),
            (
                r#"选键 ["编号","状态"] [{"状态":true,"编号":9007199254740993},{"编号":"002","备注":"茶"}]"#,
                r#"[{"编号":9007199254740993,"状态":true},{"编号":"002"}]"#,
            ),
            (r#"选键 ["编号"] [{"名称":"茶","名称":"咖啡"}]"#, "[{}]"),
            (r#"选键 ["空"] []"#, "[]"),
        ] {
            assert_eq!(transform_json(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            (r#"选键 ["a","a"] {}"#, JsonError::SelectionKeys),
            (r#"选键 [] {}"#, JsonError::SelectionKeys),
            (r#"选键 ["a"] {"a":1,"a":2}"#, JsonError::SelectionConflict),
            (r#"选键 ["a"] [{},2]"#, JsonError::ExpectedObjectItem),
        ] {
            assert_eq!(transform_json(query), Err(error), "{query}");
        }
    }

    #[test]
    fn converts_json_arrays_and_objects_to_csv_with_original_number_spellings() {
        assert_eq!(
            transform_json(r#"转CSV [["编号","备注"],["001","茶,咖啡"],["002","他说\"好\"\n下一行"],[9007199254740993,1.20]]"#).as_deref(),
            Ok("编号,备注\r\n001,\"茶,咖啡\"\r\n002,\"他说\"\"好\"\"\n下一行\"\r\n9007199254740993,1.20")
        );
        assert_eq!(
            transform_json(r#"转CSV [{"编号":"001","数量":1.20},{"编号":"002","备注":"甲,乙"},{"编号":"003","数量":null,"备注":{ "x":2 }}]"#).as_deref(),
            Ok("编号,数量,备注\r\n001,1.20,\"\"\r\n002,\"\",\"甲,乙\"\r\n003,\"\",\"{\"\"x\"\":2}\"")
        );
        assert_eq!(transform_json("转CSV []").as_deref(), Ok(""));
    }

    #[test]
    fn rejects_json_rows_that_cannot_be_represented_as_a_csv_table() {
        for (query, error) in [
            ("转CSV [1,2]", JsonError::CsvRows),
            ("转CSV [[1],[2,3]]", JsonError::CsvRows),
            ("转CSV [[]]", JsonError::CsvRows),
            ("转CSV [{}]", JsonError::CsvRows),
            (r#"转CSV [[],{"a":1}]"#, JsonError::CsvRows),
            (r#"转CSV [{"a":1,"a":2}]"#, JsonError::CsvDuplicateKey),
            (r#"转CSV [["\u0000"]]"#, JsonError::CsvNullCharacter),
            (r#"转CSV [{"\u0000":1}]"#, JsonError::CsvNullCharacter),
        ] {
            assert_eq!(transform_json(query), Err(error), "{query}");
        }
    }

    #[test]
    fn renames_and_removes_top_level_json_keys_without_reencoding_values() {
        for (query, expected) in [
            (
                r#"改键 ["旧","新"] {"前":1e2,"旧":9007199254740993,"后":"\u4f60"}"#,
                r#"{"前":1e2,"新":9007199254740993,"后":"\u4f60"}"#,
            ),
            (
                r#"改键 ["","含 空格"]{"":{ "金额": 1.20 }, "其他":0,"其他":1}"#,
                r#"{"含 空格":{"金额":1.20},"其他":0,"其他":1}"#,
            ),
            (r#"改键 ["同","同"] {"同":1}"#, r#"{"同":1}"#),
            (
                r#"去键 ["调试","备注"] {"调试":0,"调试":1,"数据":{"调试":1},"备注":"x"}"#,
                r#"{"数据":{"调试":1}}"#,
            ),
            (r#"去键 ["已删除"] {"值":3.0}"#, r#"{"值":3.0}"#),
            (r#"去键 ["值"] {"值":3.0}"#, "{}"),
        ] {
            assert_eq!(transform_json(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            (r#"改键 ["旧","新"] {"新":2}"#, JsonError::MissingKey),
            (
                r#"改键 ["旧","新"] {"旧":1,"新":2}"#,
                JsonError::RenameConflict,
            ),
            (
                r#"改键 ["旧","新"] {"旧":1,"旧":2}"#,
                JsonError::RenameConflict,
            ),
            (r#"改键 ["旧"] {"旧":1}"#, JsonError::KeyArguments),
            (r#"去键 [1] {"值":1}"#, JsonError::KeyArguments),
            (r#"去键 [] {}"#, JsonError::KeyArguments),
            (r#"去键 ["值"] []"#, JsonError::ExpectedObject),
        ] {
            assert_eq!(transform_json(query), Err(error), "{query}");
        }
    }

    #[test]
    fn converts_between_json_pointer_tokens_and_query_ready_pointers() {
        for (query, expected) in [
            (
                r#"生成指针 ["商品",0,"a/b","x~y",""]"#,
                "/商品/0/a~1b/x~0y/",
            ),
            (r#"生成指针 [9007199254740993]"#, "/9007199254740993"),
            (r#"生成指针 ["a\u0000","b c"]"#, r#""/a\u0000/b c""#),
            ("生成指针 []", "\"\""),
            (
                "解析指针 /商品/0/a~1b/x~0y/",
                r#"["商品","0","a/b","x~y",""]"#,
            ),
            ("解析指针 /a~01", r#"["a~1"]"#),
            (r#"解析指针 """#, "[]"),
            (r#"解析指针 "/a\u0000/b c""#, r#"["a\u0000","b c"]"#),
        ] {
            assert_eq!(transform_json(query).as_deref(), Ok(expected), "{query}");
        }
        let pointer = transform_json(r#"生成指针 ["a b",0]"#).unwrap();
        let query = format!(r#"取值 {pointer} {{"a b":[42]}}"#);
        assert_eq!(transform_json(&query).as_deref(), Ok("42"));
        for (query, error) in [
            ("生成指针 [true]", JsonError::InvalidPointerSegment),
            ("生成指针 [-1]", JsonError::InvalidPointerSegment),
            ("解析指针 /a~2b", JsonError::InvalidPointer),
            ("解析指针 没有斜线", JsonError::InvalidPointer),
        ] {
            assert_eq!(transform_json(query), Err(error), "{query}");
        }
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
