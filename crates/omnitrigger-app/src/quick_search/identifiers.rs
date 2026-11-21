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
    InvalidUlid,
    InvalidOperation,
    MissingUuidTime,
    TimeRange,
}

impl IdentifierError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "粘贴标识符，或输入操作名查看示例",
            Self::InvalidUuid => "输入有效的 UUID，可用连字符、紧凑、花括号或 URN 格式",
            Self::InvalidUlid => "ULID 须为 26 位 Crockford Base32，首位为 0 至 7",
            Self::InvalidOperation => "可用 格式 或 时间，UUID 还支持 紧凑 和 URN",
            Self::MissingUuidTime => "请输入带时间字段的 UUIDv1、UUIDv6 或 UUIDv7",
            Self::TimeRange => "标识符的时间超出可显示范围",
        }
    }
}

pub(super) fn transform_uuid(query: &str) -> Result<String, IdentifierError> {
    let input = normalize(query).to_ascii_lowercase();
    let (operation, value) = identifier_input(&input)?;
    let uuid = Uuid::try_parse(value).map_err(|_| IdentifierError::InvalidUuid)?;
    match operation {
        "" | "格式" | "normalize" => Ok(uuid.hyphenated().to_string()),
        "紧凑" | "compact" => Ok(uuid.simple().to_string()),
        "urn" => Ok(uuid.urn().to_string()),
        "时间" | "time" => uuid_time(uuid),
        _ => Err(IdentifierError::InvalidOperation),
    }
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
