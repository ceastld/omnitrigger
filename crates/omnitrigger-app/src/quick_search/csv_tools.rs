use std::collections::HashSet;

use serde::ser::{Serialize, SerializeMap, Serializer};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CsvError {
    MissingInput,
    UnknownOperation,
    UnclosedQuote,
    InvalidSeparator,
    DifferentColumnCount,
    InvalidHeader,
    Serialization,
}

impl CsvError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::MissingInput => "在操作名后粘贴 CSV，可用换行开始数据",
            Self::UnknownOperation => "可用 转JSON 保留行列，或用 对象 将首行用作键名",
            Self::UnclosedQuote => "CSV 字段的双引号尚未闭合",
            Self::InvalidSeparator => "检查 CSV 的引号和分隔符，引号内的双引号用两个双引号表示",
            Self::DifferentColumnCount => "CSV 每行的列数须一致",
            Self::InvalidHeader => "CSV 表头的列名须非空且唯一",
            Self::Serialization => "无法生成 JSON",
        }
    }
}

pub(super) fn transform_csv(query: &str) -> Result<String, CsvError> {
    let (operation, input) = query
        .split_once(char::is_whitespace)
        .ok_or(CsvError::MissingInput)?;
    let with_header = match operation.to_ascii_lowercase().as_str() {
        "转json" | "json" => false,
        "对象" | "表头" | "objects" => true,
        _ => return Err(CsvError::UnknownOperation),
    };
    let input = query[operation.len()..]
        .strip_prefix("\r\n")
        .unwrap_or(input);
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    if input.is_empty() {
        return Err(CsvError::MissingInput);
    }

    let rows = read_rows(input)?;
    let json = if with_header {
        let (header, records) = rows.split_first().ok_or(CsvError::MissingInput)?;
        let mut names = HashSet::new();
        if header
            .iter()
            .any(|name| name.trim().is_empty() || !names.insert(name))
        {
            return Err(CsvError::InvalidHeader);
        }
        let objects: Vec<_> = records
            .iter()
            .map(|fields| CsvObject { header, fields })
            .collect();
        serde_json::to_string(&objects)
    } else {
        serde_json::to_string(&rows)
    };
    json.map_err(|_| CsvError::Serialization)
}

fn read_rows(mut input: &str) -> Result<Vec<Vec<String>>, CsvError> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut fields = Vec::new();
    loop {
        let (field, rest) = read_field(input)?;
        fields.push(field);
        if let Some(next) = rest.strip_prefix(',') {
            input = next;
            continue;
        }
        input = if rest.is_empty() {
            rest
        } else {
            rest.strip_prefix("\r\n")
                .or_else(|| rest.strip_prefix('\n'))
                .ok_or(CsvError::InvalidSeparator)?
        };
        if rows
            .first()
            .is_some_and(|first| first.len() != fields.len())
        {
            return Err(CsvError::DifferentColumnCount);
        }
        rows.push(std::mem::take(&mut fields));
        if input.is_empty() {
            return Ok(rows);
        }
    }
}

fn read_field(input: &str) -> Result<(String, &str), CsvError> {
    if let Some(mut quoted) = input.strip_prefix('"') {
        let mut value = String::new();
        loop {
            let (part, rest) = quoted.split_once('"').ok_or(CsvError::UnclosedQuote)?;
            value.push_str(part);
            let Some(next) = rest.strip_prefix('"') else {
                return Ok((value, rest));
            };
            value.push('"');
            quoted = next;
        }
    }
    let end = input.find([',', '\r', '\n', '"']).unwrap_or(input.len());
    let (field, rest) = input.split_at(end);
    if rest.starts_with('"') {
        return Err(CsvError::InvalidSeparator);
    }
    Ok((field.to_owned(), rest))
}

struct CsvObject<'a> {
    header: &'a [String],
    fields: &'a [String],
}

impl Serialize for CsvObject<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut object = serializer.serialize_map(Some(self.header.len()))?;
        for (key, value) in self.header.iter().zip(self.fields) {
            object.serialize_entry(key, value)?;
        }
        object.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_csv_to_json_without_losing_cell_text() {
        assert_eq!(
            transform_csv("转JSON  编号,名称,备注\r\n001,\"茶,咖啡\",\"他写下\"\"你好\"\"\r\n下一行\"\r\n002,,末尾 "),
            Ok("[[\" 编号\",\"名称\",\"备注\"],[\"001\",\"茶,咖啡\",\"他写下\\\"你好\\\"\\r\\n下一行\"],[\"002\",\"\",\"末尾 \"]]".to_owned())
        );
        assert_eq!(
            transform_csv("JSON\r\n\u{feff}a,b,\nc,d,\n").as_deref(),
            Ok("[[\"a\",\"b\",\"\"],[\"c\",\"d\",\"\"]]")
        );
        assert_eq!(transform_csv("json \"\"").as_deref(), Ok("[[\"\"]]"));
        assert_eq!(
            transform_csv("对象 编号,名称\n001,茶\n002,咖啡").as_deref(),
            Ok("[{\"编号\":\"001\",\"名称\":\"茶\"},{\"编号\":\"002\",\"名称\":\"咖啡\"}]")
        );
        assert_eq!(transform_csv("objects a,b").as_deref(), Ok("[]"));
        for (query, error) in [
            ("json ", CsvError::MissingInput),
            ("json \"a", CsvError::UnclosedQuote),
            ("json \"a\"x", CsvError::InvalidSeparator),
            ("json a\"b", CsvError::InvalidSeparator),
            ("json a,b\n1", CsvError::DifferentColumnCount),
            ("objects a,a\n1,2", CsvError::InvalidHeader),
            ("objects a,\n1,2", CsvError::InvalidHeader),
        ] {
            assert_eq!(transform_csv(query), Err(error), "{query}");
        }
    }
}
