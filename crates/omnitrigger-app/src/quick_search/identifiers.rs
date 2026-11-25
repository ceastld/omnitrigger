use std::borrow::Cow;

use chrono::{DateTime, SecondsFormat, Utc};
use uuid::{Uuid, Variant};

use super::calculator::normalize;

const GREGORIAN_TO_UNIX_TICKS: i64 = 122_192_928_000_000_000;
const TICKS_PER_SECOND: i64 = 10_000_000;
const ULID_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum IdentifierError {
    Incomplete,
    InvalidUuid,
    UuidV5Arguments,
    InvalidUuidNamespace,
    InvalidUuidName,
    InvalidUlid,
    InvalidIsbn,
    IsbnChecksum,
    IsbnTenUnavailable,
    InvalidOperation,
    MissingUuidTime,
    TimeRange,
}

impl IdentifierError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "粘贴标识符，或输入操作名查看示例",
            Self::InvalidUuid => "输入有效的 UUID，可用连字符、紧凑、花括号或 URN 格式",
            Self::UuidV5Arguments => "输入命名空间和名称，例如 UUID: v5 DNS www.example.com",
            Self::InvalidUuidNamespace => "命名空间可用 DNS、URL、OID、X500 或 UUID",
            Self::InvalidUuidName => "用有效的 JSON 字符串引用名称，例如 \" 名称 \"",
            Self::InvalidUlid => "ULID 须为 26 位 Crockford Base32，首位为 0 至 7",
            Self::InvalidIsbn => "输入 10 位或 13 位 ISBN，10 位编号末位可为 X",
            Self::IsbnChecksum => "ISBN 校验位错误",
            Self::IsbnTenUnavailable => "以 979 开头的 ISBN 只有 13 位格式",
            Self::InvalidOperation => "可用 格式 或 时间，UUID 还支持 紧凑、URN 和 v5",
            Self::MissingUuidTime => "请输入带时间字段的 UUIDv1、UUIDv6 或 UUIDv7",
            Self::TimeRange => "标识符的时间超出可显示范围",
        }
    }
}

pub(super) fn transform_uuid(query: &str) -> Result<String, IdentifierError> {
    let (operation, value) = identifier_input(query)?;
    let operation = normalize(operation).to_ascii_lowercase();
    if matches!(operation.as_str(), "v5" | "名称生成") {
        return generate_uuid_v5(value);
    }
    let value = normalize(value).to_ascii_lowercase();
    let uuid = Uuid::try_parse(&value).map_err(|_| IdentifierError::InvalidUuid)?;
    match operation.as_str() {
        "" | "格式" | "normalize" => Ok(uuid.hyphenated().to_string()),
        "紧凑" | "compact" => Ok(uuid.simple().to_string()),
        "urn" => Ok(uuid.urn().to_string()),
        "时间" | "time" => uuid_time(uuid),
        _ => Err(IdentifierError::InvalidOperation),
    }
}

fn generate_uuid_v5(input: &str) -> Result<String, IdentifierError> {
    let (namespace, name) = input
        .split_once(char::is_whitespace)
        .ok_or(IdentifierError::UuidV5Arguments)?;
    let name = name.trim();
    if name.is_empty() {
        return Err(IdentifierError::UuidV5Arguments);
    }
    let namespace = normalize(namespace).to_ascii_lowercase();
    let namespace = match namespace.as_str() {
        "dns" => Uuid::NAMESPACE_DNS,
        "url" => Uuid::NAMESPACE_URL,
        "oid" => Uuid::NAMESPACE_OID,
        "x500" => Uuid::NAMESPACE_X500,
        _ => Uuid::try_parse(&namespace).map_err(|_| IdentifierError::InvalidUuidNamespace)?,
    };
    let name = if name.starts_with('"') {
        Cow::Owned(
            serde_json::from_str::<String>(name).map_err(|_| IdentifierError::InvalidUuidName)?,
        )
    } else {
        Cow::Borrowed(name)
    };
    Ok(Uuid::new_v5(&namespace, name.as_bytes())
        .hyphenated()
        .to_string())
}

fn uuid_time(uuid: Uuid) -> Result<String, IdentifierError> {
    if uuid.get_variant() != Variant::RFC4122 {
        return Err(IdentifierError::MissingUuidTime);
    }
    let timestamp = uuid
        .get_timestamp()
        .ok_or(IdentifierError::MissingUuidTime)?;
    let (seconds, nanos) = if matches!(uuid.get_version_num(), 1 | 6) {
        // 先取回 Gregorian ticks，再换成有符号的 Unix 时间，保留 1970 年以前的日期。
        let ticks = i64::try_from(timestamp.to_gregorian().0)
            .map_err(|_| IdentifierError::TimeRange)?
            - GREGORIAN_TO_UNIX_TICKS;
        (
            ticks.div_euclid(TICKS_PER_SECOND),
            (ticks.rem_euclid(TICKS_PER_SECOND) * 100) as u32,
        )
    } else {
        let (seconds, nanos) = timestamp.to_unix();
        (
            i64::try_from(seconds).map_err(|_| IdentifierError::TimeRange)?,
            nanos,
        )
    };
    format_time(DateTime::from_timestamp(seconds, nanos))
}

pub(super) fn transform_ulid(query: &str) -> Result<String, IdentifierError> {
    let input = normalize(query).to_ascii_lowercase();
    let (operation, value) = identifier_input(&input)?;
    let ulid = value.to_ascii_uppercase();
    if ulid.len() != 26 {
        return Err(IdentifierError::InvalidUlid);
    }
    let mut millis = 0i64;
    for (index, byte) in ulid.bytes().enumerate() {
        let digit = ULID_ALPHABET
            .iter()
            .position(|&symbol| symbol == byte)
            .ok_or(IdentifierError::InvalidUlid)?;
        if index == 0 && digit > 7 {
            return Err(IdentifierError::InvalidUlid);
        }
        if index < 10 {
            millis = millis * 32 + digit as i64;
        }
    }
    match operation {
        "" | "格式" | "normalize" => Ok(ulid),
        "时间" | "time" => format_time(DateTime::from_timestamp_millis(millis)),
        _ => Err(IdentifierError::InvalidOperation),
    }
}

pub(super) fn transform_isbn(query: &str) -> Result<String, IdentifierError> {
    enum IsbnOperation {
        Normalize,
        Validate,
        ToTen,
        ToThirteen,
    }
    let query = normalize(query);
    let query = query.trim();
    let (operation, rest) = query.split_once(char::is_whitespace).unwrap_or((query, ""));
    let (operation, source) = match operation.to_ascii_lowercase().as_str() {
        "格式" | "normalize" => (IsbnOperation::Normalize, rest),
        "校验" | "validate" => (IsbnOperation::Validate, rest),
        "转10位" | "to-10" => (IsbnOperation::ToTen, rest),
        "转13位" | "to-13" => (IsbnOperation::ToThirteen, rest),
        _ => (IsbnOperation::Normalize, query),
    };
    let isbn: String = source
        .chars()
        .filter(|&ch| !ch.is_whitespace() && !matches!(ch, '-' | '\u{2010}' | '\u{2011}'))
        .map(|ch| ch.to_ascii_uppercase())
        .collect();
    if isbn.is_empty() {
        return Err(IdentifierError::Incomplete);
    }
    let short = isbn.len() == 10;
    let book_prefix =
        isbn.starts_with("978") || (isbn.starts_with("979") && !isbn.starts_with("9790"));
    let valid_shape = short || (isbn.len() == 13 && book_prefix);
    if !isbn.is_ascii() || !valid_shape {
        return Err(IdentifierError::InvalidIsbn);
    }
    let (digits, provided) = isbn.split_at(isbn.len() - 1);
    if !digits.bytes().all(|digit| digit.is_ascii_digit())
        || !(provided.as_bytes()[0].is_ascii_digit() || (short && provided == "X"))
    {
        return Err(IdentifierError::InvalidIsbn);
    }
    if provided.as_bytes()[0] != isbn_check_digit(digits, short) {
        return Err(IdentifierError::IsbnChecksum);
    }
    match operation {
        IsbnOperation::Normalize => Ok(isbn),
        IsbnOperation::Validate => Ok(format!("ISBN-{} 校验位正确", isbn.len())),
        IsbnOperation::ToThirteen if short => {
            let mut result = format!("978{digits}");
            result.push(char::from(isbn_check_digit(&result, false)));
            Ok(result)
        }
        IsbnOperation::ToTen if !short => {
            let mut result = digits
                .strip_prefix("978")
                .ok_or(IdentifierError::IsbnTenUnavailable)?
                .to_owned();
            result.push(char::from(isbn_check_digit(&result, true)));
            Ok(result)
        }
        IsbnOperation::ToTen | IsbnOperation::ToThirteen => Ok(isbn),
    }
}

fn isbn_check_digit(digits: &str, short: bool) -> u8 {
    let sum: u32 = digits
        .bytes()
        .enumerate()
        .map(|(index, digit)| {
            let weight = if short {
                10 - index as u32
            } else {
                1 + 2 * (index % 2) as u32
            };
            u32::from(digit - b'0') * weight
        })
        .sum();
    let modulus = if short { 11 } else { 10 };
    match (modulus - sum % modulus) % modulus {
        10 => b'X',
        check => b'0' + check as u8,
    }
}

fn identifier_input(query: &str) -> Result<(&str, &str), IdentifierError> {
    let query = query.trim();
    if query.is_empty() {
        return Err(IdentifierError::Incomplete);
    }
    let (operation, identifier) = query.split_once(char::is_whitespace).unwrap_or(("", query));
    Ok((operation, identifier.trim()))
}

fn format_time(time: Option<DateTime<Utc>>) -> Result<String, IdentifierError> {
    let time = time.ok_or(IdentifierError::TimeRange)?;
    Ok(time.to_rfc3339_opts(SecondsFormat::AutoSi, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_and_converts_book_numbers_with_fullwidth_and_hyphenated_input() {
        for (input, output) in [
            ("０‑８０４４‑２９５７‑ｘ", "080442957X"),
            ("0 8044 2957 X", "080442957X"),
            ("校验 978-0-11-000222-4", "ISBN-13 校验位正确"),
            ("转13位 0-8044-2957-X", "9780804429573"),
            ("TO-10 9780804429573", "080442957X"),
            ("转13位 9791090636071", "9791090636071"),
        ] {
            assert_eq!(transform_isbn(input).as_deref(), Ok(output));
        }
        assert_eq!(
            transform_isbn("校验 9780110002225"),
            Err(IdentifierError::IsbnChecksum)
        );
        assert_eq!(
            transform_isbn("校验 9790000000001"),
            Err(IdentifierError::InvalidIsbn)
        );
        assert_eq!(
            transform_isbn("转10位 9791090636071"),
            Err(IdentifierError::IsbnTenUnavailable)
        );
        assert_eq!(transform_isbn("校验"), Err(IdentifierError::Incomplete));
    }

    #[test]
    fn generates_rfc_uuid_v5_values_and_preserves_quoted_utf8_names() {
        for (query, expected) in [
            (
                "v5 DNS www.example.com",
                "2ed6657d-e927-568b-95e1-2665a8aea6a2",
            ),
            (
                "Ｖ５ 6ba7b810-9dad-11d1-80b4-00c04fd430c8 www.example.com",
                "2ed6657d-e927-568b-95e1-2665a8aea6a2",
            ),
            (
                "名称生成 URL https://example.invalid/a",
                "c43494ec-b689-5a23-84de-5af91d379181",
            ),
            (
                r#"v5 OID " 茶 Ａ ""#,
                "b876fcb8-a9b5-5ab0-99da-0bc6a5cd6334",
            ),
            (r#"v5 X500 """#, "b4bdf874-8c03-5bd8-8fd7-5e409dfd82c0"),
        ] {
            assert_eq!(transform_uuid(query).as_deref(), Ok(expected), "{query}");
        }
        assert_ne!(
            transform_uuid("v5 DNS www.example.com"),
            transform_uuid("v5 DNS www.EXAMPLE.com")
        );
        assert_eq!(
            transform_uuid("v5 DNS"),
            Err(IdentifierError::UuidV5Arguments)
        );
        assert_eq!(
            transform_uuid("v5 custom 茶"),
            Err(IdentifierError::InvalidUuidNamespace)
        );
        assert_eq!(
            transform_uuid("v5 DNS \"茶"),
            Err(IdentifierError::InvalidUuidName)
        );
    }

    #[test]
    fn normalizes_uuid_forms_and_reads_rfc_timestamps() {
        for (query, expected) in [
            (
                "{550E8400-E29B-41D4-A716-446655440000}",
                "550e8400-e29b-41d4-a716-446655440000",
            ),
            (
                "紧凑 URN:UUID:550E8400-E29B-41D4-A716-446655440000",
                "550e8400e29b41d4a716446655440000",
            ),
            (
                "URN 550e8400e29b41d4a716446655440000",
                "urn:uuid:550e8400-e29b-41d4-a716-446655440000",
            ),
            (
                "时间 c232ab00-9414-11ec-b3c8-9f6bdeced846",
                "2022-02-22T19:22:22Z",
            ),
            (
                "时间 1ec9414c-232a-6b00-b3c8-9f6bdeced846",
                "2022-02-22T19:22:22Z",
            ),
            (
                "时间 017f22e2-79b0-7cc3-98c4-dc0c0c07398f",
                "2022-02-22T19:22:22Z",
            ),
            (
                "时间 13813fff-1dd2-11b2-8000-000000000000",
                "1969-12-31T23:59:59.999999900Z",
            ),
            (
                "时间 1b21dd21-3813-6fff-8000-000000000000",
                "1969-12-31T23:59:59.999999900Z",
            ),
            (
                "时间 ffffffff-ffff-7000-8000-000000000000",
                "+10889-08-02T05:31:50.655Z",
            ),
        ] {
            assert_eq!(transform_uuid(query).as_deref(), Ok(expected), "{query}");
        }
        for query in [
            "时间 550e8400-e29b-41d4-a716-446655440000",
            "时间 017f22e2-79b0-7cc3-c8c4-dc0c0c07398f",
        ] {
            assert_eq!(transform_uuid(query), Err(IdentifierError::MissingUuidTime));
        }
        assert_eq!(
            transform_uuid("550e8400"),
            Err(IdentifierError::InvalidUuid)
        );
    }

    #[test]
    fn validates_all_ulid_symbols_and_extracts_the_timestamp() {
        for (query, expected) in [
            ("01arz3ndektsv4rrffq69g5fav", "01ARZ3NDEKTSV4RRFFQ69G5FAV"),
            (
                "时间 01ARZ3NDEKTSV4RRFFQ69G5FAV",
                "2016-07-30T23:54:10.259Z",
            ),
            ("时间 00000000000000000000000000", "1970-01-01T00:00:00Z"),
            (
                "时间 7ZZZZZZZZZZZZZZZZZZZZZZZZZ",
                "+10889-08-02T05:31:50.655Z",
            ),
        ] {
            assert_eq!(transform_ulid(query).as_deref(), Ok(expected), "{query}");
        }
        for query in [
            "80000000000000000000000000",
            "01ARZ3NDEKTSV4RRFFQ69G5FAI",
            "01ARZ3NDEKTSV4RRFFQ69G5FA",
        ] {
            assert_eq!(transform_ulid(query), Err(IdentifierError::InvalidUlid));
        }
    }
}
