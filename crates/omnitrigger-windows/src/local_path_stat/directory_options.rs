//! 显式目录投影；默认行为仍交给原有句柄枚举。
use super::{
    checkpoint, directory_fault, read_local_directory, validate_local_path, LocalDirectoryEntry,
    LocalStatError, MAX_DIRECTORY_ENTRIES,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DirectoryOrder {
    #[default]
    Name,
    Native,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DirectoryEntryType {
    #[default]
    All,
    File,
    Directory,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalDirectoryOptions {
    pub max_entries: usize,
    pub order: DirectoryOrder,
    pub metadata: bool,
    pub entry_type: DirectoryEntryType,
    /// 仅单个名称组件；明确采用 Windows FindFirstFileW 匹配，不冒充现代.NET托管匹配。
    pub search_pattern: Option<String>,
}
impl LocalDirectoryOptions {
    pub fn validate(&self) -> Result<(), LocalStatError> {
        if !(1..=MAX_DIRECTORY_ENTRIES).contains(&self.max_entries) {
            return Err(directory_fault("invalid_input"));
        }
        if let Some(pattern) = &self.search_pattern {
            normalize_pattern(pattern)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalDirectoryMetadata {
    pub ordinal: usize,
    pub attributes: u32,
    pub creation_utc_100ns: u64,
    pub access_utc_100ns: u64,
    pub write_utc_100ns: u64,
    pub creation_local_ticks: u64,
    pub access_local_ticks: u64,
    pub write_local_ticks: u64,
    pub time_zone_key: String,
}

pub fn read_local_directory_with_options(
    path: &str,
    options: &LocalDirectoryOptions,
    check: impl Fn() -> Result<(), LocalStatError>,
) -> Result<Vec<LocalDirectoryEntry>, LocalStatError> {
    checkpoint(&check)?;
    validate_local_path(path)?;
    options.validate()?;
    if options.order == DirectoryOrder::Name
        && !options.metadata
        && options.search_pattern.is_none()
        && options.entry_type == DirectoryEntryType::All
    {
        return read_local_directory(path, options.max_entries, check);
    }
    #[cfg(windows)]
    {
        super::native::read_options(path, options, &check)
    }
    #[cfg(not(windows))]
    {
        Err(directory_fault("fs_unavailable"))
    }
}

/// 不接受目录组件、设备前缀或控制字符；不会把参数重解释成另一个目录。
/// 采用Framework参考实现的尾空格与单点规范化；空结果明确拒绝而不改成全匹配。
pub(super) fn normalize_pattern(pattern: &str) -> Result<String, LocalStatError> {
    if pattern.encode_utf16().count() > 256
        || pattern.chars().any(|c| {
            c < ' '
                || matches!(
                    c,
                    '\u{7f}'..='\u{9f}' | '\\' | '/' | ':' | '<' | '>' | '"' | '|'
                )
        })
    {
        return Err(directory_fault("invalid_input"));
    }
    let pattern = pattern.trim_end_matches(' ');
    if pattern.is_empty() || pattern.ends_with("..") {
        return Err(directory_fault("invalid_input"));
    }
    Ok(if pattern == "." { "*" } else { pattern }.into())
}

/// .NET DateTime比较只使用公历本地wall-clock ticks，不包含Kind/歧义DST标志。
/// Windows转换只返回毫秒；额外保留原FILETIME的100ns余数，避免排序精度损失。
#[cfg(any(windows, test))]
pub(super) fn datetime_ticks(parts: [u16; 7], sub_millisecond: u64) -> Result<u64, LocalStatError> {
    let [year, month, day, hour, minute, second, millisecond] = parts;
    if !(1..=9999).contains(&year)
        || !(1..=12).contains(&month)
        || hour > 23
        || minute > 59
        || second > 59
        || millisecond > 999
        || sub_millisecond > 9999
    {
        return Err(directory_fault("fs_error"));
    }
    let year = u64::from(year);
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let lengths = [
        31u16,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if day == 0 || day > lengths[usize::from(month - 1)] {
        return Err(directory_fault("fs_error"));
    }
    let prior = year - 1;
    let days = prior * 365 + prior / 4 - prior / 100
        + prior / 400
        + lengths[..usize::from(month - 1)]
            .iter()
            .map(|d| u64::from(*d))
            .sum::<u64>()
        + u64::from(day - 1);
    Ok(
        (((days * 24 + u64::from(hour)) * 60 + u64::from(minute)) * 60 + u64::from(second))
            * 10_000_000
            + u64::from(millisecond) * 10_000
            + sub_millisecond,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pattern_restricts_scope_without_guessing_regex() {
        assert_eq!(normalize_pattern("*.xls").unwrap(), "*.xls");
        assert_eq!(normalize_pattern(". ").unwrap(), "*");
        for pattern in ["", " ", "..", "sub\\*.txt", "C:*.txt", "a\0b", "<.txt"] {
            assert!(normalize_pattern(pattern).is_err());
        }
    }
    #[test]
    fn local_datetime_keys_preserve_100ns_and_calendar_boundaries() {
        assert_eq!(datetime_ticks([1, 1, 1, 0, 0, 0, 0], 0).unwrap(), 0);
        assert_eq!(
            datetime_ticks([1601, 1, 1, 0, 0, 0, 0], 0).unwrap(),
            504_911_232_000_000_000
        );
        let a = datetime_ticks([2026, 11, 1, 1, 30, 0, 123], 4567).unwrap();
        assert_eq!(
            datetime_ticks([2026, 11, 1, 1, 30, 0, 123], 4568).unwrap() - a,
            1
        );
        assert_eq!(a % 10_000_000, 1_234_567);
        assert!(datetime_ticks([1900, 2, 29, 0, 0, 0, 0], 0).is_err());
        assert!(datetime_ticks([2000, 2, 29, 0, 0, 0, 0], 0).is_ok());
    }
}
