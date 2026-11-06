//! 只读本地固定卷元数据及有界直接目录枚举；不打开内容、不追随已发现的 reparse。
//!
//! 各次盘符/祖先检查不是原子快照。它们不能防止检查之间的名称空间变化，
//! 也不构成沙盒；后续打开文件必须重新检查。check 不能抢占已进入的系统调用。

const MAX_PATH_UNITS: usize = 32_767;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalPathKind {
    File,
    Directory,
    Missing,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalStatError {
    pub code: &'static str,
    pub message: String,
}

impl std::fmt::Display for LocalStatError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for LocalStatError {}

fn fault(code: &'static str) -> LocalStatError {
    let message = match code {
        "invalid_input" => "需要符合规则的普通盘符绝对路径。",
        "path_not_local" => "只支持直接映射到本地固定卷的盘符。",
        "path_unavailable" => "此路径包含不允许查询的目录或文件属性。",
        "access_denied" => "系统拒绝读取文件元数据。",
        "fs_unavailable" => "当前平台不提供本地文件元数据查询。",
        "cancelled" => "文件元数据查询已取消。",
        "timeout" => "文件元数据查询已超过截止时限。",
        _ => "系统无法读取文件元数据。",
    };
    LocalStatError {
        code,
        message: message.into(),
    }
}

fn checkpoint(check: &impl Fn() -> Result<(), LocalStatError>) -> Result<(), LocalStatError> {
    // 外层回调仅提供错误类别，避免其附带的 OS 文本或路径进入公开错误正文。
    check().map_err(|error| fault(error.code))
}

fn separator(character: char) -> bool {
    matches!(character, '\\' | '/')
}

fn reserved_component(component: &str) -> bool {
    // Win32 设备别名也忽略扩展名前的 ASCII 尾空格；不修剪实际传入的路径。
    let base = component
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end_matches(' ');
    let upper = base.to_ascii_uppercase();
    if matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    // Windows 同样把 ISO-8859-1 上标 ¹/²/³ 当作 COM/LPT 设备号。
    upper
        .strip_prefix("COM")
        .or_else(|| upper.strip_prefix("LPT"))
        .is_some_and(|suffix| {
            matches!(
                suffix,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
}

fn validate_local_path(path: &str) -> Result<(), LocalStatError> {
    // &str 本身保证有效 Unicode；UTF-16 编码不会产生孤立代理项。
    if path.encode_utf16().take(MAX_PATH_UNITS + 1).count() > MAX_PATH_UNITS {
        return Err(fault("invalid_input"));
    }
    let bytes = path.as_bytes();
    if bytes.len() < 3
        || !bytes[0].is_ascii_alphabetic()
        || bytes[1] != b':'
        || !matches!(bytes[2], b'\\' | b'/')
        || path.chars().any(char::is_control)
    {
        return Err(fault("invalid_input"));
    }
    let mut remainder = &path[3..];
    if remainder.is_empty() {
        return Ok(());
    }
    if remainder.ends_with(separator) {
        remainder = &remainder[..remainder.len() - 1];
    }
    // 根已经带分隔符；额外的根分隔符和空中间组件都不能被归一化吞掉。
    for component in remainder.split(separator) {
        if component.is_empty()
            || matches!(component, "." | "..")
            || component.ends_with([' ', '.'])
            || component
                .chars()
                .any(|character| matches!(character, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
            || reserved_component(component)
        {
            return Err(fault("invalid_input"));
        }
    }
    // 先完成整条路径的词法检查，避免云目录前缀掩盖其它非法组件。
    if remainder.split(separator).any(|component| {
        let lower = component.to_ascii_lowercase();
        lower.starts_with("onedrive") || lower.starts_with("icloud") || lower == "dropbox"
    }) {
        return Err(fault("path_unavailable"));
    }
    Ok(())
}

/// 查询普通盘符绝对路径的元数据。错误正文不包含目标路径或 OS 原始文本。
///
/// 在任何元数据调用之前独立复验词法，并验证固定盘类型和 DOS 当前映射。
/// 每个系统调用前后执行 check；调用进行中不能硬性抢占，且各次结果不原子。
pub fn stat_local_path(
    path: &str,
    check: impl Fn() -> Result<(), LocalStatError>,
) -> Result<LocalPathKind, LocalStatError> {
    #[cfg(windows)]
    {
        stat_with(path, &check, &mut NativeMetadata)
    }
    #[cfg(not(windows))]
    {
        checkpoint(&check)?;
        validate_local_path(path)?;
        Err(fault("fs_unavailable"))
    }
}

#[cfg(any(windows, test))]
const FIXED_DRIVE: u32 = 3;
#[cfg(any(windows, test))]
const ATTRIBUTE_DIRECTORY: u32 = 0x10;
#[cfg(any(windows, test))]
const ATTRIBUTE_DEVICE: u32 = 0x40;
#[cfg(any(windows, test))]
const FORBIDDEN_ATTRIBUTES: u32 = 0x400 | 0x1000 | 0x40000 | 0x400000;

#[cfg(any(windows, test))]
trait Metadata {
    fn drive_type(&mut self, root: &str) -> u32;
    fn current_device(&mut self, drive: &str) -> Result<String, u32>;
    fn attributes(&mut self, path: &str) -> Result<u32, u32>;
}

#[cfg(any(windows, test))]
fn system_step<T>(
    check: &impl Fn() -> Result<(), LocalStatError>,
    operation: impl FnOnce() -> T,
) -> Result<T, LocalStatError> {
    checkpoint(check)?;
    let result = operation();
    // 即使 OS 返回 missing/拒绝访问，期间发生的取消/超时也先于结果返回。
    checkpoint(check)?;
    Ok(result)
}

#[cfg(any(windows, test))]
fn physical_volume_device(device: &str) -> bool {
    // 只接受当前直接卷映射；\??\、SUBST、MUP、重定向器与附带路径都不接受。
    const PREFIX: &str = "\\Device\\HarddiskVolume";
    device
        .get(..PREFIX.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(PREFIX))
        && device.get(PREFIX.len()..).is_some_and(|suffix| {
            !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
        })
}

#[cfg(any(windows, test))]
fn error_for_os(code: u32) -> LocalStatError {
    if matches!(code, 5 | 1314) {
        fault("access_denied")
    } else {
        fault("fs_error")
    }
}

#[cfg(any(windows, test))]
fn classify_attributes(result: Result<u32, u32>) -> Result<LocalPathKind, LocalStatError> {
    let attributes = match result {
        Ok(value) => value,
        Err(2 | 3) => return Ok(LocalPathKind::Missing),
        Err(code) => return Err(error_for_os(code)),
    };
    if attributes & FORBIDDEN_ATTRIBUTES != 0 {
        return Err(fault("path_unavailable"));
    }
    if attributes & ATTRIBUTE_DEVICE != 0 {
        Ok(LocalPathKind::Other)
    } else if attributes & ATTRIBUTE_DIRECTORY != 0 {
        Ok(LocalPathKind::Directory)
    } else {
        Ok(LocalPathKind::File)
    }
}

#[cfg(any(windows, test))]
fn stat_with(
    path: &str,
    check: &impl Fn() -> Result<(), LocalStatError>,
    metadata: &mut impl Metadata,
) -> Result<LocalPathKind, LocalStatError> {
    checkpoint(check)?;
    validate_local_path(path)?;
    let root = format!("{}:\\", char::from(path.as_bytes()[0].to_ascii_uppercase()));
    if system_step(check, || metadata.drive_type(&root))? != FIXED_DRIVE {
        return Err(fault("path_not_local"));
    }
    let device = system_step(check, || metadata.current_device(&root[..2]))?.map_err(|code| {
        if matches!(code, 2 | 3 | 15) {
            fault("path_not_local")
        } else {
            error_for_os(code)
        }
    })?;
    if !physical_volume_device(&device) {
        return Err(fault("path_not_local"));
    }
    let mut current = root;
    let mut kind = classify_attributes(system_step(check, || metadata.attributes(&current))?)?;
    let remainder = &path[3..];
    if remainder.is_empty() {
        return Ok(kind);
    }
    let trailing_separator = remainder.ends_with(separator);
    let remainder = if trailing_separator {
        &remainder[..remainder.len() - 1]
    } else {
        remainder
    };
    let mut components = remainder.split(separator).peekable();
    while let Some(component) = components.next() {
        if kind == LocalPathKind::Missing {
            return Ok(LocalPathKind::Missing);
        }
        if kind != LocalPathKind::Directory {
            return Err(fault("path_unavailable"));
        }
        if !current.ends_with('\\') {
            current.push('\\');
        }
        current.push_str(component);
        if trailing_separator && components.peek().is_none() {
            current.push('\\');
        }
        kind = classify_attributes(system_step(check, || metadata.attributes(&current))?)?;
    }
    Ok(kind)
}

#[cfg(windows)]
struct NativeMetadata;

#[cfg(windows)]
impl Metadata for NativeMetadata {
    fn drive_type(&mut self, root: &str) -> u32 {
        use windows::{core::PCWSTR, Win32::Storage::FileSystem::GetDriveTypeW};
        let wide: Vec<u16> = root.encode_utf16().chain(Some(0)).collect();
        // SAFETY: 有效的 NUL 结尾盘符根，缓冲区存活至调用结束。
        unsafe { GetDriveTypeW(PCWSTR(wide.as_ptr())) }
    }

    fn current_device(&mut self, drive: &str) -> Result<String, u32> {
        use windows::{
            core::PCWSTR,
            Win32::{Foundation::GetLastError, Storage::FileSystem::QueryDosDeviceW},
        };
        let wide: Vec<u16> = drive.encode_utf16().chain(Some(0)).collect();
        let mut target = vec![0u16; 32_768];
        // SAFETY: 指定单个盘符，不传 NULL（不枚举设备）；切片提供有界输出区。
        let count = unsafe { QueryDosDeviceW(PCWSTR(wide.as_ptr()), Some(&mut target)) };
        if count == 0 {
            // 必须在外层 checkpoint 之前捕获 LastError，不能依赖回调保留线程错误槽。
            return Err(unsafe { GetLastError() }.0);
        }
        let Some(written) = target.get(..count as usize) else {
            return Err(13);
        };
        // 文档定义首个 NUL 结尾字符串为当前映射；后续是尚未清除的旧映射。
        let Some(end) = written.iter().position(|unit| *unit == 0) else {
            return Err(13);
        };
        String::from_utf16(&written[..end]).map_err(|_| 13)
    }

    fn attributes(&mut self, path: &str) -> Result<u32, u32> {
        use windows::{
            core::PCWSTR,
            Win32::{
                Foundation::GetLastError,
                Storage::FileSystem::{GetFileAttributesW, INVALID_FILE_ATTRIBUTES},
            },
        };
        // 输入扩展/设备路径已被拒绝；只给经过独立词法验证的盘符路径加内部前缀，
        // 避免进程未选择 longPathAware 时把合法长路径误判成不存在。
        let wide: Vec<u16> = "\\\\?\\"
            .encode_utf16()
            .chain(path.encode_utf16())
            .chain(Some(0))
            .collect();
        // SAFETY: 缓冲区是完整有效的 NUL 结尾 UTF-16；仅查询当前末级属性。
        let attributes = unsafe { GetFileAttributesW(PCWSTR(wide.as_ptr())) };
        if attributes == INVALID_FILE_ATTRIBUTES {
            Err(unsafe { GetLastError() }.0)
        } else {
            Ok(attributes)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, collections::BTreeMap};

    struct FakeMetadata {
        drive_type: u32,
        device: Result<String, u32>,
        attributes: BTreeMap<String, Result<u32, u32>>,
        calls: Vec<String>,
    }

    impl Default for FakeMetadata {
        fn default() -> Self {
            Self {
                drive_type: FIXED_DRIVE,
                device: Ok("\\Device\\HarddiskVolume3".into()),
                attributes: BTreeMap::from([
                    ("C:\\".into(), Ok(ATTRIBUTE_DIRECTORY)),
                    ("C:\\parent".into(), Ok(ATTRIBUTE_DIRECTORY)),
                    ("C:\\parent\\file".into(), Ok(0x80)),
                ]),
                calls: Vec::new(),
            }
        }
    }

    impl Metadata for FakeMetadata {
        fn drive_type(&mut self, root: &str) -> u32 {
            self.calls.push(format!("drive:{root}"));
            self.drive_type
        }

        fn current_device(&mut self, drive: &str) -> Result<String, u32> {
            self.calls.push(format!("device:{drive}"));
            self.device.clone()
        }

        fn attributes(&mut self, path: &str) -> Result<u32, u32> {
            self.calls.push(format!("attributes:{path}"));
            self.attributes.get(path).copied().unwrap_or(Err(2))
        }
    }

    #[test]
    fn local_stat_lexical_rejections_do_not_query_the_os_or_echo_the_path() {
        for path in [
            "",
            "relative",
            "C:relative",
            "\\rooted",
            "\\\\server\\share",
            "\\\\?\\C:\\x",
            "\\\\.\\C:\\x",
            "//server/share",
            "1:\\x",
            "é:\\x",
            "C:\\\\",
            "C:\\a\\\\b",
            "C:\\a//b",
            "C:\\.\\x",
            "C:\\..\\x",
            "C:\\a\\..",
            "C:\\file:stream",
            "C:\\a*",
            "C:\\a?",
            "C:\\a\"",
            "C:\\a<",
            "C:\\a>",
            "C:\\a|",
            "C:\\a.",
            "C:\\a ",
            "C:\\CON",
            "C:\\nul.txt",
            "C:\\Aux.tar.gz",
            "C:\\PRN\\x",
            "C:\\com1",
            "C:\\LPT9.log",
            "C:/CON .txt",
            "C:/com1 .log",
            "C:/LPT9 .txt",
            "C:/COM¹.txt",
            "C:/COM²",
            "C:/COM³.log",
            "C:/LPT¹",
            "C:/LPT².txt",
            "C:/LPT³ .txt",
            "C:\\x\0y",
            "C:\\x\u{7f}y",
            "C:\\x\u{85}y",
        ] {
            let mut metadata = FakeMetadata::default();
            let error = stat_with(path, &|| Ok(()), &mut metadata).unwrap_err();
            assert_eq!(error.code, "invalid_input");
            assert!(metadata.calls.is_empty());
            if !path.is_empty() {
                assert!(!error.message.contains(path));
            }
        }
    }

    #[test]
    fn local_stat_utf16_limit_roots_and_trailing_separator_are_preserved() {
        assert!(validate_local_path("C:\\").is_ok());
        assert!(validate_local_path("c:/").is_ok());
        assert!(validate_local_path("C:/中文/😀/").is_ok());
        assert!(validate_local_path("C:\\COM0\\LPT10\\console\\.hidden").is_ok());
        let at_limit = format!("C:\\{}", "😀".repeat((MAX_PATH_UNITS - 3) / 2));
        assert_eq!(at_limit.encode_utf16().count(), MAX_PATH_UNITS);
        assert!(validate_local_path(&at_limit).is_ok());
        assert_eq!(
            validate_local_path(&(at_limit + "x")).unwrap_err().code,
            "invalid_input"
        );
        let mut metadata = FakeMetadata::default();
        metadata
            .attributes
            .insert("C:\\parent\\".into(), Ok(ATTRIBUTE_DIRECTORY));
        assert_eq!(
            stat_with("c:/parent/", &|| Ok(()), &mut metadata),
            Ok(LocalPathKind::Directory)
        );
        assert_eq!(metadata.calls.last().unwrap(), "attributes:C:\\parent\\");
    }

    #[test]
    fn local_stat_cloud_components_are_distinct_from_invalid_lexical_paths() {
        for path in [
            "C:\\OneDrive\\a",
            "C:/oNeDrIvE-company/a",
            "C:\\iCLOUDDrive\\a",
            "C:\\DROPBOX\\a",
        ] {
            let mut metadata = FakeMetadata::default();
            assert_eq!(
                stat_with(path, &|| Ok(()), &mut metadata).unwrap_err().code,
                "path_unavailable"
            );
            assert!(metadata.calls.is_empty());
        }
        for path in [
            "C:\\xOneDrive\\a",
            "C:\\DropboxBackup\\a",
            "C:\\OneDrivefile.txt",
        ] {
            let result = validate_local_path(path);
            assert_eq!(result.is_ok(), !path.contains("OneDrivefile"));
        }
        assert_eq!(
            validate_local_path("C:\\OneDrive\\..\\a").unwrap_err().code,
            "invalid_input"
        );
    }

    #[test]
    fn local_stat_rejects_nonfixed_drives_and_subst_before_any_attributes() {
        for drive_type in [0, 1, 2, 4, 5, 6] {
            let mut metadata = FakeMetadata {
                drive_type,
                ..Default::default()
            };
            assert_eq!(
                stat_with("C:\\parent\\file", &|| Ok(()), &mut metadata)
                    .unwrap_err()
                    .code,
                "path_not_local"
            );
            assert_eq!(metadata.calls.len(), 1);
        }
        for device in [
            "\\??\\D:\\folder",
            "\\Device\\Mup\\server\\share",
            "\\Device\\LanmanRedirector\\x",
            "\\Device\\HarddiskVolume3\\folder",
            "\\Device\\HarddiskVolumeShadowCopy1",
            "\\Device\\HarddiskVolume",
            "\\Device\\HarddiskVolume3x",
            "",
        ] {
            let mut metadata = FakeMetadata {
                device: Ok(device.into()),
                ..Default::default()
            };
            assert_eq!(
                stat_with("C:\\parent\\file", &|| Ok(()), &mut metadata)
                    .unwrap_err()
                    .code,
                "path_not_local"
            );
            assert_eq!(metadata.calls.len(), 2);
        }
        assert!(physical_volume_device("\\device\\harddiskvolume12"));
    }

    #[test]
    fn local_stat_checks_ancestors_in_order_and_stops_at_unsafe_attributes() {
        for forbidden in [0x400, 0x1000, 0x40000, 0x400000, FORBIDDEN_ATTRIBUTES] {
            let mut metadata = FakeMetadata::default();
            metadata
                .attributes
                .insert("C:\\parent".into(), Ok(ATTRIBUTE_DIRECTORY | forbidden));
            assert_eq!(
                stat_with("C:\\parent\\file", &|| Ok(()), &mut metadata)
                    .unwrap_err()
                    .code,
                "path_unavailable"
            );
            assert_eq!(
                metadata.calls,
                [
                    "drive:C:\\",
                    "device:C:",
                    "attributes:C:\\",
                    "attributes:C:\\parent"
                ]
            );
        }
        let mut metadata = FakeMetadata::default();
        assert_eq!(
            stat_with("C:\\parent\\file", &|| Ok(()), &mut metadata),
            Ok(LocalPathKind::File)
        );
        assert_eq!(
            metadata.calls.last().unwrap(),
            "attributes:C:\\parent\\file"
        );
        metadata.attributes.insert("C:\\parent".into(), Ok(0x80));
        metadata.calls.clear();
        assert_eq!(
            stat_with("C:\\parent\\file", &|| Ok(()), &mut metadata)
                .unwrap_err()
                .code,
            "path_unavailable"
        );
        assert_eq!(metadata.calls.len(), 4);
    }

    #[test]
    fn local_stat_missing_access_denied_and_other_failures_are_not_conflated() {
        for (code, expected) in [
            (2, None),
            (3, None),
            (5, Some("access_denied")),
            (1314, Some("access_denied")),
            (32, Some("fs_error")),
            (123, Some("fs_error")),
        ] {
            for failed_path in ["C:\\", "C:\\parent", "C:\\parent\\file"] {
                let mut metadata = FakeMetadata::default();
                metadata.attributes.insert(failed_path.into(), Err(code));
                let result = stat_with("C:\\parent\\file", &|| Ok(()), &mut metadata);
                if let Some(expected) = expected {
                    assert_eq!(result.unwrap_err().code, expected);
                } else {
                    assert_eq!(result, Ok(LocalPathKind::Missing));
                }
                assert_eq!(
                    metadata.calls.last().unwrap(),
                    &format!("attributes:{failed_path}")
                );
            }
        }
        assert_eq!(
            classify_attributes(Ok(ATTRIBUTE_DEVICE)),
            Ok(LocalPathKind::Other)
        );
        assert_eq!(
            classify_attributes(Ok(ATTRIBUTE_DIRECTORY)),
            Ok(LocalPathKind::Directory)
        );
        assert_eq!(classify_attributes(Ok(0x80)), Ok(LocalPathKind::File));
    }

    #[test]
    fn local_stat_dos_mapping_errors_are_not_reported_as_missing_paths() {
        for (code, expected) in [
            (2, "path_not_local"),
            (3, "path_not_local"),
            (15, "path_not_local"),
            (5, "access_denied"),
            (1314, "access_denied"),
            (122, "fs_error"),
        ] {
            let mut metadata = FakeMetadata {
                device: Err(code),
                ..Default::default()
            };
            assert_eq!(
                stat_with("C:\\parent\\file", &|| Ok(()), &mut metadata)
                    .unwrap_err()
                    .code,
                expected
            );
            assert_eq!(metadata.calls.len(), 2);
        }
    }

    #[test]
    fn local_stat_checkpoints_cover_before_and_after_every_os_step() {
        // 此路径有 drive、DOS 映射、根、父目录、文件五个 OS 步骤，另有入口检查。
        for code in ["cancelled", "timeout"] {
            for fail_at in 1..=11 {
                let count = Cell::new(0);
                let check = || {
                    count.set(count.get() + 1);
                    if count.get() == fail_at {
                        Err(LocalStatError {
                            code,
                            message: "C:\\private-target 不应泄露".into(),
                        })
                    } else {
                        Ok(())
                    }
                };
                let mut metadata = FakeMetadata::default();
                let error = stat_with("C:\\parent\\file", &check, &mut metadata).unwrap_err();
                assert_eq!(error.code, code);
                assert!(!error.message.contains("private-target"));
                assert_eq!(metadata.calls.len(), (fail_at - 1) / 2);
                assert_eq!(count.get(), fail_at);
            }
        }
    }

    #[test]
    fn local_stat_after_call_cancellation_wins_over_missing_or_os_failure() {
        for os_error in [2, 3, 5, 32] {
            let mut metadata = FakeMetadata::default();
            metadata.attributes.insert("C:\\".into(), Err(os_error));
            let count = Cell::new(0);
            let check = || {
                count.set(count.get() + 1);
                if count.get() == 7 {
                    Err(fault("timeout"))
                } else {
                    Ok(())
                }
            };
            assert_eq!(
                stat_with("C:\\parent\\file", &check, &mut metadata)
                    .unwrap_err()
                    .code,
                "timeout"
            );
            assert_eq!(metadata.calls.len(), 3);
        }
    }

    #[cfg(windows)]
    mod real_metadata {
        use super::*;
        use std::{
            path::PathBuf,
            sync::atomic::{AtomicU64, Ordering},
        };

        static NEXT: AtomicU64 = AtomicU64::new(1);

        struct TemporaryFixture {
            root: PathBuf,
        }

        impl TemporaryFixture {
            fn new() -> Self {
                let base = std::env::temp_dir();
                assert_eq!(
                    stat_local_path(base.to_str().expect("临时目录需要有效Unicode"), || Ok(()))
                        .expect("临时目录必须位于受支持的本地固定卷"),
                    LocalPathKind::Directory
                );
                for _ in 0..16 {
                    let root = base.join(format!(
                        "omnitrigger-local-stat-{}-{}",
                        std::process::id(),
                        NEXT.fetch_add(1, Ordering::Relaxed)
                    ));
                    match std::fs::create_dir(&root) {
                        Ok(()) => return Self { root },
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                        Err(_) => panic!("无法创建隔离临时测试目录"),
                    }
                }
                panic!("无法分配隔离临时测试目录");
            }
        }

        impl Drop for TemporaryFixture {
            fn drop(&mut self) {
                // 所有路径均由本测试构造；只删已知对象，不枚举或递归处理父目录。
                let _ = std::fs::remove_dir(self.root.join("link"));
                let _ = std::fs::remove_file(self.root.join("empty-file"));
                let _ = std::fs::remove_dir(self.root.join("directory"));
                let _ = std::fs::remove_dir(&self.root);
            }
        }

        #[test]
        fn local_stat_real_temporary_file_directory_and_missing() {
            let fixture = TemporaryFixture::new();
            drop(
                std::fs::File::create_new(fixture.root.join("empty-file")).expect("创建空测试文件"),
            );
            std::fs::create_dir(fixture.root.join("directory")).expect("创建测试子目录");
            for (suffix, expected) in [
                ("empty-file", LocalPathKind::File),
                ("directory", LocalPathKind::Directory),
                ("missing", LocalPathKind::Missing),
                ("missing/child", LocalPathKind::Missing),
            ] {
                let path = fixture.root.join(suffix);
                assert_eq!(
                    stat_local_path(path.to_str().unwrap(), || Ok(())),
                    Ok(expected)
                );
            }
        }

        #[test]
        fn local_stat_real_symlink_is_rejected_before_following_descendants() {
            let fixture = TemporaryFixture::new();
            std::fs::create_dir(fixture.root.join("directory")).expect("创建测试子目录");
            match std::os::windows::fs::symlink_dir(
                fixture.root.join("directory"),
                fixture.root.join("link"),
            ) {
                Ok(()) => {}
                Err(error) if error.raw_os_error() == Some(1314) => return,
                Err(_) => panic!("无法创建隔离测试链接"),
            }
            for suffix in ["link", "link/missing-child"] {
                let path = fixture.root.join(suffix);
                let error = stat_local_path(path.to_str().unwrap(), || Ok(())).unwrap_err();
                assert_eq!(error.code, "path_unavailable");
            }
        }

        #[test]
        fn local_stat_attribute_constants_match_the_pinned_windows_bindings() {
            use windows::Win32::Storage::FileSystem::{
                FILE_ATTRIBUTE_DEVICE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_OFFLINE,
                FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS, FILE_ATTRIBUTE_RECALL_ON_OPEN,
                FILE_ATTRIBUTE_REPARSE_POINT,
            };
            assert_eq!(ATTRIBUTE_DEVICE, FILE_ATTRIBUTE_DEVICE.0);
            assert_eq!(ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_DIRECTORY.0);
            assert_eq!(
                FORBIDDEN_ATTRIBUTES,
                FILE_ATTRIBUTE_REPARSE_POINT.0
                    | FILE_ATTRIBUTE_OFFLINE.0
                    | FILE_ATTRIBUTE_RECALL_ON_OPEN.0
                    | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS.0
            );
        }
    }
}

mod directory;
pub use directory::{
    read_local_directory, LocalDirectoryEntry, MAX_DIRECTORY_BYTES, MAX_DIRECTORY_ENTRIES,
};
