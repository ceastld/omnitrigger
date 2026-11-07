//! 文件列表写入参数与CF_HDROP编码；不查询路径或打开文件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardFilesError {
    pub code: &'static str,
    pub effect_state: &'static str,
}
impl ClipboardFilesError {
    pub fn before(code: &'static str) -> Self {
        Self {
            code,
            effect_state: "not_started",
        }
    }
    pub fn unknown(mut self) -> Self {
        self.effect_state = "unknown";
        self
    }
}

fn absolute_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    if value
        .chars()
        .any(|c| c < '\u{20}' || matches!(c, '<' | '>' | '"' | '|' | '?' | '*'))
    {
        return false;
    }
    let tail = if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
    {
        &value[3..]
    } else if value.starts_with("\\\\") && !value.starts_with("\\\\.\\") {
        let mut parts = value[2..].split(['\\', '/']);
        if parts.next().is_none_or(str::is_empty) || parts.next().is_none_or(str::is_empty) {
            return false;
        }
        &value[2..]
    } else {
        return false;
    };
    !tail.contains(':')
        && !tail
            .split(['\\', '/'])
            .any(|part| matches!(part, "." | ".."))
}

pub fn validate_clipboard_file_paths(paths: &[String]) -> Result<(), ClipboardFilesError> {
    if paths.is_empty() || paths.len() > 64 {
        return Err(ClipboardFilesError::before("invalid_input"));
    }
    let mut total = 0usize;
    for path in paths {
        let units = path.encode_utf16().count();
        if units == 0 || units > 32767 || !absolute_path(path) {
            return Err(ClipboardFilesError::before("invalid_input"));
        }
        total = total
            .checked_add(units)
            .ok_or_else(|| ClipboardFilesError::before("invalid_input"))?;
        if total > 65536 {
            return Err(ClipboardFilesError::before("invalid_input"));
        }
    }
    Ok(())
}

#[cfg(any(windows, test))]
pub(crate) fn encode_paths(paths: &[String]) -> Result<Vec<u16>, ClipboardFilesError> {
    validate_clipboard_file_paths(paths)?;
    // DROPFILES固定20字节：pFiles=20、pt=(0,0)、fNC=0、fWide=1。
    let mut units = vec![20, 0, 0, 0, 0, 0, 0, 0, 1, 0];
    for path in paths {
        units.extend(path.encode_utf16());
        units.push(0);
    }
    units.push(0);
    Ok(units)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_drop_header_unicode_order_duplicates_and_double_terminator() {
        let paths = vec![
            r"C:\合成😀.txt".into(),
            r"\\server\share\file.txt".into(),
            r"C:\合成😀.txt".into(),
        ];
        let units = encode_paths(&paths).unwrap();
        assert_eq!(&units[..10], &[20, 0, 0, 0, 0, 0, 0, 0, 1, 0]);
        let actual = units[10..units.len() - 1]
            .split(|u| *u == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf16(s).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(actual, paths);
        assert_eq!(&units[units.len() - 2..], &[0, 0]);
    }
    #[test]
    fn invalid_entries_reject_entire_list_before_native_work() {
        for bad in [
            "",
            "relative.txt",
            r"C:relative",
            r"\\?\C:\x",
            r"\\.\pipe\x",
            r"\\server",
            r"C:\a\..\x",
            r"C:\x:stream",
            "C:\\nul\0x",
        ] {
            assert!(
                encode_paths(&[r"C:\ok".into(), bad.into()]).is_err(),
                "{bad:?}"
            );
        }
        assert!(encode_paths(&[]).is_err());
        assert!(encode_paths(&vec![r"C:\ok".into(); 65]).is_err());
        assert!(encode_paths(&[format!("C:\\{}", "😀".repeat(16382))]).is_ok());
        assert!(encode_paths(&[format!("C:\\{}", "😀".repeat(16383))]).is_err());
        assert!(encode_paths(&vec![format!("C:\\{}", "x".repeat(16382)); 4]).is_err());
    }
}
