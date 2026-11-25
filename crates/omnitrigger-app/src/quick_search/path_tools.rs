use std::path::{Component, Path, PathBuf, Prefix, MAIN_SEPARATOR_STR};

use url::Url;

use super::csv_tools::append_bounded_csv_row;
use super::text_tools::logical_newline_pattern;

const INVALID_PATH_TEXT: [char; 5] = ['\0', '"', '\r', '\n', '\t'];
const MAX_FILENAME_UNITS: usize = 255;
const MAX_PATH_REPORT_ROWS: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PathError {
    Incomplete,
    UnknownOperation,
    InvalidPath,
    AbsolutePathRequired,
    InvalidFileUrl,
    JoinArguments,
    RelativePartRequired,
    RelativeArguments,
    CommonPathArguments,
    ReportArguments,
    LineReportArguments,
    ReportTooLarge,
    NormalizationPath,
    AbovePathRoot,
    DifferentRoot,
    ExtensionArguments,
    InvalidExtension,
    FilenameTooLong,
    FilenameCharacters,
    FilenameEnding,
    ReservedFilename,
    MissingPart,
}

impl PathError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "在操作名后粘贴路径",
            Self::UnknownOperation => "输入 ? 路径 查看可用操作",
            Self::InvalidPath => "输入单条路径，可用一对双引号包住",
            Self::AbsolutePathRequired => "请输入可以转换为文件网址的绝对路径",
            Self::InvalidFileUrl => "请填写能转换为绝对路径的 file:// 网址，省略查询串和片段",
            Self::JoinArguments => "用 JSON 字符串数组填写起始路径和至少一条相对路径",
            Self::RelativePartRequired => "起始路径后的每一条路径须为相对路径",
            Self::RelativeArguments => {
                "用 JSON 数组填写基准目录和至少一条绝对目标路径，路径里省略 .."
            }
            Self::CommonPathArguments => {
                "用 JSON 数组填写至少两个普通盘符或共享目录下的绝对路径，路径里省略 .."
            }
            Self::ReportArguments => "用 JSON 字符串数组填写 1 至 128 条路径",
            Self::LineReportArguments => "粘贴 1 至 128 条路径，每行一条，可用双引号包住",
            Self::ReportTooLarge => "路径部件表超过 1 MiB",
            Self::NormalizationPath => "请输入普通盘符或共享目录下的绝对路径",
            Self::AbovePathRoot => ".. 超出了盘符或共享根目录",
            Self::DifferentRoot => "输入路径须在同一盘符或同一共享根目录下",
            Self::ExtensionArguments => "用 JSON 字符串数组填写路径和新扩展名",
            Self::InvalidExtension => "扩展名须为文件名后缀，可用空字符串删除",
            Self::FilenameTooLong => "文件名最多支持 255 个 UTF-16 单元",
            Self::FilenameCharacters => "文件名含有 Windows 保留字符",
            Self::FilenameEnding => "文件名末尾有空格或句点",
            Self::ReservedFilename => "这是 Windows 的保留设备名",
            Self::MissingPart => "这条路径没有所选部分",
        }
    }
}

pub(super) fn path_csv_report(query: &str) -> Option<Result<String, PathError>> {
    let (operation, input) = query.split_once(char::is_whitespace)?;
    match operation.to_ascii_lowercase().as_str() {
        "部件csv" | "components-csv" => Some(json_path_components_csv(input)),
        "逐行部件csv" | "line-components-csv" => Some(line_path_components_csv(input)),
        _ => None,
    }
}

pub(super) fn path_component(query: &str) -> Result<String, PathError> {
    if let Some(report) = path_csv_report(query) {
        return report;
    }
    let (operation, input) = query
        .split_once(char::is_whitespace)
        .ok_or(PathError::Incomplete)?;
    let operation = operation.to_ascii_lowercase();
    match operation.as_str() {
        "换扩展名" | "replace-extension" => return replace_extension(input.trim()),
        "拼接" | "join" => return join_paths(input.trim()),
        "相对路径" | "relative" => return relative_path(input.trim()),
        "共同路径" | "common-path" => return common_path(input.trim()),
        "合法文件名" | "safe-filename" => return safe_filename(input.trim()),
        "校验文件名" | "validate-filename" => return validate_filename(input.trim()),
        _ => {}
    }
    let input = strip_path_quotes(input.trim());
    if input.is_empty() {
        return Err(PathError::Incomplete);
    }
    if input.contains(INVALID_PATH_TEXT) {
        return Err(PathError::InvalidPath);
    }
    let path = Path::new(input);
    let component = match operation.as_str() {
        "转文件网址" | "to-file-url" => {
            return file_url(path, input.ends_with(['/', '\\']));
        }
        "从文件网址" | "from-file-url" => return path_from_file_url(input),
        "规范化" | "normalize" => return normalize_absolute_path(path),
        "文件名" | "name" | "filename" => path.file_name().and_then(|part| part.to_str()),
        "主名" | "stem" => path.file_stem().and_then(|part| part.to_str()),
        "扩展名" | "ext" | "extension" => path.extension().and_then(|part| part.to_str()),
        "上一级" | "parent" => path.parent().and_then(Path::to_str),
        _ => return Err(PathError::UnknownOperation),
    };
    component
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .ok_or(PathError::MissingPart)
}

fn json_path_components_csv(input: &str) -> Result<String, PathError> {
    let paths =
        serde_json::from_str::<Vec<String>>(input).map_err(|_| PathError::ReportArguments)?;
    path_components_csv(paths.iter().map(String::as_str), PathError::ReportArguments)
}

fn line_path_components_csv(input: &str) -> Result<String, PathError> {
    let paths: Vec<_> = logical_newline_pattern()
        .split(input)
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(strip_path_quotes)
        .take(MAX_PATH_REPORT_ROWS + 1)
        .collect();
    path_components_csv(paths.into_iter(), PathError::LineReportArguments)
}

fn path_components_csv<'a>(
    paths: impl ExactSizeIterator<Item = &'a str>,
    invalid_count: PathError,
) -> Result<String, PathError> {
    if !(1..=MAX_PATH_REPORT_ROWS).contains(&paths.len()) {
        return Err(invalid_count);
    }
    let mut csv = String::from("路径,上一级,文件名,主名,扩展名");
    for text in paths {
        if text.is_empty() || text.contains(INVALID_PATH_TEXT) {
            return Err(PathError::InvalidPath);
        }
        let path = Path::new(text);
        let parent = path.parent().and_then(Path::to_str).unwrap_or("");
        let name = path
            .file_name()
            .and_then(|part| part.to_str())
            .unwrap_or("");
        let stem = path
            .file_stem()
            .and_then(|part| part.to_str())
            .unwrap_or("");
        let extension = path
            .extension()
            .and_then(|part| part.to_str())
            .unwrap_or("");
        append_bounded_csv_row(
            &mut csv,
            [text, parent, name, stem, extension].into_iter(),
            PathError::ReportTooLarge,
        )?;
    }
    Ok(csv)
}

fn safe_filename(input: &str) -> Result<String, PathError> {
    let mut name = filename_text(input)?.replace(invalid_filename_character, "_");
    name.truncate(name.trim_end_matches([' ', '.']).len());
    if name.is_empty() || reserved_filename(&name) {
        name.insert(0, '_');
    }
    if name.encode_utf16().take(MAX_FILENAME_UNITS + 1).count() > MAX_FILENAME_UNITS {
        return Err(PathError::FilenameTooLong);
    }
    Ok(name)
}

fn validate_filename(input: &str) -> Result<String, PathError> {
    let name = filename_text(input)?;
    if name.chars().any(invalid_filename_character) {
        return Err(PathError::FilenameCharacters);
    }
    if name.ends_with([' ', '.']) {
        return Err(PathError::FilenameEnding);
    }
    if reserved_filename(name) {
        return Err(PathError::ReservedFilename);
    }
    let units = name.encode_utf16().take(MAX_FILENAME_UNITS + 1).count();
    if units > MAX_FILENAME_UNITS {
        return Err(PathError::FilenameTooLong);
    }
    Ok(format!("文件名有效 · {units} 个 UTF-16 单元"))
}

fn filename_text(input: &str) -> Result<&str, PathError> {
    let input = input
        .strip_prefix('“')
        .and_then(|quoted| quoted.strip_suffix('”'))
        .unwrap_or_else(|| strip_path_quotes(input));
    (!input.is_empty())
        .then_some(input)
        .ok_or(PathError::Incomplete)
}

fn invalid_filename_character(character: char) -> bool {
    character < ' '
        || matches!(
            character,
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
        )
}

fn reserved_filename(name: &str) -> bool {
    let stem = name
        .split_once('.')
        .map_or(name, |(stem, _)| stem)
        .trim_matches(' ')
        .to_ascii_uppercase();
    let numbered_device = stem
        .strip_prefix("COM")
        .or_else(|| stem.strip_prefix("LPT"))
        .is_some_and(|number| {
            matches!(
                number,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        });
    numbered_device
        || matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        )
}

pub(super) fn file_url(path: &Path, is_directory: bool) -> Result<String, PathError> {
    let url = if is_directory {
        Url::from_directory_path(path)
    } else {
        Url::from_file_path(path)
    };
    url.map(String::from)
        .map_err(|_| PathError::AbsolutePathRequired)
}

fn path_from_file_url(input: &str) -> Result<String, PathError> {
    let url = Url::parse(input).map_err(|_| PathError::InvalidFileUrl)?;
    if url.scheme() != "file" || url.query().is_some() || url.fragment().is_some() {
        return Err(PathError::InvalidFileUrl);
    }
    let path = url.to_file_path().map_err(|_| PathError::InvalidFileUrl)?;
    path.to_str()
        .filter(|text| path.is_absolute() && !text.contains(INVALID_PATH_TEXT))
        .map(str::to_owned)
        .ok_or(PathError::InvalidFileUrl)
}

fn join_paths(input: &str) -> Result<String, PathError> {
    let parts = serde_json::from_str::<Vec<String>>(input).map_err(|error| {
        if error.is_eof() {
            PathError::Incomplete
        } else {
            PathError::JoinArguments
        }
    })?;
    let [base, first, rest @ ..] = parts.as_slice() else {
        return Err(PathError::JoinArguments);
    };
    if parts
        .iter()
        .any(|part| part.is_empty() || part.contains(INVALID_PATH_TEXT))
    {
        return Err(PathError::JoinArguments);
    }
    let mut path = PathBuf::from(base.as_str());
    for part in std::iter::once(first).chain(rest) {
        let part = Path::new(part);
        if part.has_root() || matches!(part.components().next(), Some(Component::Prefix(_))) {
            return Err(PathError::RelativePartRequired);
        }
        path.push(part);
    }
    path.into_os_string()
        .into_string()
        .map_err(|_| PathError::InvalidPath)
}

fn relative_path(input: &str) -> Result<String, PathError> {
    let paths = serde_json::from_str::<Vec<String>>(input).map_err(|error| {
        if error.is_eof() {
            PathError::Incomplete
        } else {
            PathError::RelativeArguments
        }
    })?;
    let [base, targets @ ..] = paths.as_slice() else {
        return Err(PathError::RelativeArguments);
    };
    if targets.is_empty() {
        return Err(PathError::RelativeArguments);
    }
    let base = absolute_path(base).ok_or(PathError::RelativeArguments)?;
    let results = targets
        .iter()
        .map(|target| {
            let target = absolute_path(target).ok_or(PathError::RelativeArguments)?;
            relative_path_from(base, target)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(results.join("\n"))
}

fn relative_path_from(base: &Path, target: &Path) -> Result<String, PathError> {
    let mut base = base.components().peekable();
    let mut target = target.components().peekable();
    if base.next() != target.next() {
        return Err(PathError::DifferentRoot);
    }
    while base.peek().is_some() && base.peek() == target.peek() {
        base.next();
        target.next();
    }
    let mut parts = vec![".."; base.count()];
    for part in target {
        let Component::Normal(name) = part else {
            return Err(PathError::RelativeArguments);
        };
        parts.push(name.to_str().ok_or(PathError::InvalidPath)?);
    }
    if parts.is_empty() {
        return Ok(".".to_owned());
    }
    let result = parts.join(MAIN_SEPARATOR_STR);
    if matches!(
        Path::new(&result).components().next(),
        Some(Component::Prefix(_))
    ) {
        return Ok(format!(".{MAIN_SEPARATOR_STR}{result}"));
    }
    Ok(result)
}

fn replace_extension(input: &str) -> Result<String, PathError> {
    let [source, extension] = serde_json::from_str::<[String; 2]>(input).map_err(|error| {
        if error.is_eof() {
            PathError::Incomplete
        } else {
            PathError::ExtensionArguments
        }
    })?;
    if source.contains(INVALID_PATH_TEXT) {
        return Err(PathError::InvalidPath);
    }
    let last = source.rsplit(['/', '\\']).next().unwrap_or_default();
    let path = Path::new(&source);
    if matches!(last, "" | "." | "..") || path.file_name().is_none() {
        return Err(PathError::MissingPart);
    }
    if extension.ends_with(['.', ' '])
        || extension.chars().any(|character| {
            character.is_control()
                || matches!(
                    character,
                    '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
                )
        })
    {
        return Err(PathError::InvalidExtension);
    }
    let extension = extension.strip_prefix('.').unwrap_or(&extension);
    path.with_extension(extension)
        .to_str()
        .map(str::to_owned)
        .ok_or(PathError::InvalidPath)
}

fn has_ordinary_root(path: &Path) -> bool {
    path.is_absolute()
        && match path.components().next() {
            Some(Component::Prefix(prefix)) => {
                matches!(prefix.kind(), Prefix::Disk(_) | Prefix::UNC(_, _))
            }
            _ => true,
        }
}

fn normalize_absolute_path(path: &Path) -> Result<String, PathError> {
    if !has_ordinary_root(path) {
        return Err(PathError::NormalizationPath);
    }
    let mut root = String::new();
    let mut segments = Vec::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => {
                let prefix = prefix.as_os_str().to_str().ok_or(PathError::InvalidPath)?;
                root.push_str(&prefix.replace('/', MAIN_SEPARATOR_STR));
            }
            Component::RootDir => root.push_str(MAIN_SEPARATOR_STR),
            Component::CurDir => {}
            Component::ParentDir => {
                segments.pop().ok_or(PathError::AbovePathRoot)?;
            }
            Component::Normal(segment) => {
                segments.push(segment.to_str().ok_or(PathError::InvalidPath)?);
            }
        }
    }
    root.push_str(&segments.join(MAIN_SEPARATOR_STR));
    Ok(root)
}

pub(super) fn absolute_path(input: &str) -> Option<&Path> {
    let path = Path::new(input);
    (!input.contains(INVALID_PATH_TEXT)
        && has_ordinary_root(path)
        && !path.components().any(|part| part == Component::ParentDir))
    .then_some(path)
}

fn common_path(input: &str) -> Result<String, PathError> {
    let paths = serde_json::from_str::<Vec<String>>(input).map_err(|error| {
        if error.is_eof() {
            PathError::Incomplete
        } else {
            PathError::CommonPathArguments
        }
    })?;
    let [first, second, remaining @ ..] = paths.as_slice() else {
        return Err(PathError::CommonPathArguments);
    };
    let mut common = absolute_path(first).ok_or(PathError::CommonPathArguments)?;
    for path in std::iter::once(second).chain(remaining) {
        let path = absolute_path(path).ok_or(PathError::CommonPathArguments)?;
        while !path.starts_with(common) {
            common = common.parent().ok_or(PathError::DifferentRoot)?;
        }
    }
    common
        .to_str()
        .map(str::to_owned)
        .ok_or(PathError::InvalidPath)
}

pub(super) fn strip_path_quotes(input: &str) -> &str {
    input
        .strip_prefix('"')
        .and_then(|input| input.strip_suffix('"'))
        .unwrap_or(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn produces_a_single_windows_filename_and_handles_reserved_names() {
        for (query, expected) in [
            ("合法文件名 CON.报告?.txt", "_CON.报告_.txt"),
            ("合法文件名 NUL .txt", "_NUL .txt"),
            ("safe-filename COM².txt", "_COM².txt"),
            ("safe-filename conout$.TXT", "_conout$.TXT"),
            ("合法文件名 .temp", ".temp"),
            ("合法文件名 \".. \"", "_"),
            ("合法文件名 \"a:b\\c<d>|e*.\t\"", "a_b_c_d__e_._"),
            ("合法文件名 “COM10.包.”", "COM10.包"),
        ] {
            assert_eq!(path_component(query).as_deref(), Ok(expected), "{query}");
        }
        let fits = format!("{}a", "😀".repeat(127));
        assert_eq!(path_component(&format!("合法文件名 {fits}")), Ok(fits));
        assert_eq!(
            path_component(&format!("合法文件名 {}", "😀".repeat(128))),
            Err(PathError::FilenameTooLong)
        );
    }

    #[test]
    fn validates_single_windows_filenames_and_reports_why_a_name_fails() {
        for (query, expected) in [
            ("校验文件名 报告集.txt", Ok("文件名有效 · 7 个 UTF-16 单元")),
            ("validate-filename .😀", Ok("文件名有效 · 3 个 UTF-16 单元")),
            ("校验文件名 CON.txt", Err(PathError::ReservedFilename)),
            (
                "validate-filename LPT³.png",
                Err(PathError::ReservedFilename),
            ),
            ("校验文件名 “报告集.txt ”", Err(PathError::FilenameEnding)),
            ("校验文件名 ..", Err(PathError::FilenameEnding)),
            ("校验文件名 原稿?.txt", Err(PathError::FilenameCharacters)),
            (
                "校验文件名 \"条\t目.txt\"",
                Err(PathError::FilenameCharacters),
            ),
            ("校验文件名 \"\"", Err(PathError::Incomplete)),
        ] {
            assert_eq!(
                path_component(query),
                expected.map(str::to_owned),
                "{query}"
            );
        }
        assert_eq!(
            path_component(&format!("校验文件名 {}", "😀".repeat(128))),
            Err(PathError::FilenameTooLong),
        );
    }

    #[cfg(windows)]
    #[test]
    fn normalizes_drive_and_unc_paths_lexically() {
        for (query, expected) in [
            (
                r#"规范化 "C:/工作//发布/./临时/../报告 集.txt""#,
                r"C:\工作\发布\报告 集.txt",
            ),
            (r"normalize \\Host\Share\目录\..", "\\\\Host\\Share\\"),
            (r"规范化 C:\资料\..\.", "C:\\"),
        ] {
            assert_eq!(path_component(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(
            path_component(r"规范化 C:\..\资料"),
            Err(PathError::AbovePathRoot)
        );
        for query in [
            r"规范化 目录\..\文件",
            r"规范化 C:相对",
            r"规范化 \\?\C:\文件",
        ] {
            assert_eq!(
                path_component(query),
                Err(PathError::NormalizationPath),
                "{query}"
            );
        }
    }

    #[test]
    #[cfg(windows)]
    fn finds_the_longest_common_path_by_complete_components() {
        for (input, expected) in [
            (
                r#"共同路径 ["C:\\工作\\发布\\A.txt","C:/工作/草稿/B.txt","C:\\工作\\设计图.png"]"#,
                r"C:\工作",
            ),
            (r#"common-path ["C:\\work","C:\\worker"]"#, "C:\\"),
            (
                r#"common-path ["\\\\server\\share\\x","\\\\server\\share\\y"]"#,
                r"\\server\share\",
            ),
            (
                r#"common-path ["C:\\work\\a.txt","C:\\work\\a.txt"]"#,
                r"C:\work\a.txt",
            ),
        ] {
            assert_eq!(path_component(input).as_deref(), Ok(expected), "{input}");
        }
        for (input, expected) in [
            (
                r#"common-path ["C:\\work"]"#,
                PathError::CommonPathArguments,
            ),
            (
                r#"common-path ["C:\\work","C:\\..\\x"]"#,
                PathError::CommonPathArguments,
            ),
            (
                r#"common-path ["C:\\work","D:\\work"]"#,
                PathError::DifferentRoot,
            ),
        ] {
            assert_eq!(path_component(input), Err(expected), "{input}");
        }
    }

    #[test]
    fn requires_absolute_paths_without_parent_components_for_relative_calculation() {
        for input in [r#"相对路径 ["工作","结果.txt"]"#, r#"相对路径 ["工作"]"#] {
            assert_eq!(path_component(input), Err(PathError::RelativeArguments));
        }
    }

    #[cfg(windows)]
    #[test]
    fn finds_relative_paths_within_one_drive_or_unc_share() {
        for (input, expected) in [
            (
                r#"相对路径 ["C:\\工作\\资料","C:\\工作\\发布\\2026.txt"]"#,
                r"..\发布\2026.txt",
            ),
            (
                r#"relative ["C:/工作","C:\\工作\\发布\\2026.txt"]"#,
                r"发布\2026.txt",
            ),
            (r#"relative ["C:\\docs","C:\\docs"]"#, "."),
            (
                r#"relative ["\\\\server\\share\\a","\\\\server\\share\\b.txt"]"#,
                r"..\b.txt",
            ),
        ] {
            assert_eq!(path_component(input).as_deref(), Ok(expected), "{input}");
        }
        for (input, expected) in [
            (
                r#"relative ["C:\\docs","D:\\docs"]"#,
                PathError::DifferentRoot,
            ),
            (
                r#"relative ["\\\\server\\a","\\\\server\\b"]"#,
                PathError::DifferentRoot,
            ),
            (
                r#"relative ["C:\\docs","C:\\docs\\..\\a"]"#,
                PathError::RelativeArguments,
            ),
            (
                r#"relative ["\\\\?\\C:\\docs","\\\\?\\C:\\a"]"#,
                PathError::RelativeArguments,
            ),
        ] {
            assert_eq!(path_component(input), Err(expected), "{input}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn lists_relative_paths_for_multiple_targets_in_order() {
        let query = r#"相对路径 ["C:\\工作\\资料","C:\\工作\\资料\\表.csv","C:\\工作\\发布\\报告.txt","C:\\工作\\资料"]"#;
        assert_eq!(
            path_component(query).as_deref(),
            Ok("表.csv\n..\\发布\\报告.txt\n.")
        );
        assert_eq!(
            path_component(r#"relative ["C:\\工作","C:\\工作\\报告.txt","D:\\工作\\报告.txt"]"#),
            Err(PathError::DifferentRoot)
        );
        assert_eq!(
            path_component(r#"relative ["C:\\工作","C:\\工作\\报告.txt","报告.txt"]"#),
            Err(PathError::RelativeArguments)
        );
    }

    #[test]
    fn rejects_relative_paths_and_file_urls_that_do_not_identify_plain_paths() {
        assert_eq!(
            path_component("转文件网址 ./report.txt"),
            Err(PathError::AbsolutePathRequired)
        );
        for url in [
            "https://example.invalid/report.txt",
            "file:///C:/report.txt?download=1",
            "file:///C:/report.txt#part",
            "file:///C:/a%00b.txt",
            "file:///C:/a%0Ab.txt",
        ] {
            assert_eq!(
                path_component(&format!("从文件网址 {url}")),
                Err(PathError::InvalidFileUrl),
                "{url}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn round_trips_windows_disk_and_unc_paths_with_escaped_file_names() {
        for (path, url) in [
            (
                r"C:\工作\报告 #.txt",
                "file:///C:/%E5%B7%A5%E4%BD%9C/%E6%8A%A5%E5%91%8A%20%23.txt",
            ),
            (r"\\server\share\a b.txt", "file://server/share/a%20b.txt"),
        ] {
            assert_eq!(
                path_component(&format!("转文件网址 {path}")).as_deref(),
                Ok(url)
            );
            assert_eq!(
                path_component(&format!("from-file-url {url}")).as_deref(),
                Ok(path)
            );
        }
        assert_eq!(
            path_component(r"转文件网址 C:\docs\").as_deref(),
            Ok("file:///C:/docs/")
        );
    }

    #[test]
    fn joins_multiple_paths_and_requires_relative_parts_after_the_base() {
        let expected = PathBuf::from("base").join("目录 集").join("2026.txt");
        assert_eq!(
            path_component(r#"join ["base","目录 集","2026.txt"]"#).as_deref(),
            Ok(expected.to_str().unwrap())
        );
        assert_eq!(
            path_component(r#"拼接 ["base","/absolute"]"#),
            Err(PathError::RelativePartRequired)
        );
        assert_eq!(
            path_component(r#"拼接 ["base",""]"#),
            Err(PathError::JoinArguments)
        );
        #[cfg(windows)]
        {
            assert_eq!(
                path_component(r#"拼接 ["C:\\工作","报告 集","2026.txt"]"#).as_deref(),
                Ok(r"C:\工作\报告 集\2026.txt")
            );
            for query in [r#"拼接 ["base","C:child"]"#, r#"拼接 ["base","\\root"]"#] {
                assert_eq!(path_component(query), Err(PathError::RelativePartRequired));
            }
        }
    }

    #[test]
    fn changes_the_last_path_extension_and_preserves_the_directory() {
        for (query, expected) in [
            (
                r#"换扩展名 ["/项目/发布 包.tar.gz",".zip"]"#,
                "/项目/发布 包.tar.zip",
            ),
            (
                r#"replace-extension ["./archive/.gitignore","bak"]"#,
                "./archive/.gitignore.bak",
            ),
            (r#"换扩展名 ["/项目/资料.tar.gz",""]"#, "/项目/资料.tar"),
        ] {
            assert_eq!(path_component(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            (r#"换扩展名 ["report"]"#, PathError::ExtensionArguments),
            (r#"换扩展名 ["report","a/b"]"#, PathError::InvalidExtension),
            (r#"换扩展名 ["report","a\\b"]"#, PathError::InvalidExtension),
            (r#"换扩展名 ["report","zip."]"#, PathError::InvalidExtension),
            (r#"换扩展名 ["/项目/","zip"]"#, PathError::MissingPart),
            (r#"换扩展名 ["/项目/.","zip"]"#, PathError::MissingPart),
        ] {
            assert_eq!(path_component(query), Err(error), "{query}");
        }
        #[cfg(windows)]
        assert_eq!(
            path_component(r#"换扩展名 ["C:\\工作\\发布 包.tar.gz",".zip"]"#).as_deref(),
            Ok(r"C:\工作\发布 包.tar.zip")
        );
    }

    #[test]
    fn inspects_the_supplied_path_without_accessing_it() {
        for (query, expected) in [
            (r#"文件名 "/项目/归档 2026.tar.gz""#, "归档 2026.tar.gz"),
            ("主名 /项目/归档 2026.tar.gz", "归档 2026.tar"),
            ("扩展名 /项目/归档 2026.tar.gz", "gz"),
            ("上一级 /项目/归档 2026.tar.gz", "/项目"),
            ("stem ./src/.gitignore", ".gitignore"),
            ("parent ./archive/../missing.txt", "./archive/.."),
        ] {
            assert_eq!(path_component(query).as_deref(), Ok(expected), "{query}");
        }
        for (query, error) in [
            ("文件名", PathError::Incomplete),
            ("主名 \"\"", PathError::Incomplete),
            ("未知 x", PathError::UnknownOperation),
            ("文件名 \"x", PathError::InvalidPath),
            ("文件名 x\ny", PathError::InvalidPath),
            ("文件名 x\0y", PathError::InvalidPath),
            ("扩展名 .gitignore", PathError::MissingPart),
            ("扩展名 trailing.", PathError::MissingPart),
            ("上一级 one.txt", PathError::MissingPart),
            ("上一级 /", PathError::MissingPart),
        ] {
            assert_eq!(path_component(query), Err(error), "{query}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn extracts_windows_drive_and_unc_components() {
        for (query, expected) in [
            (r#"文件名 "C:\工作\发布 包.zip""#, "发布 包.zip"),
            (r"上一级 C:\工作\发布 包.zip", r"C:\工作"),
            (r"上一级 C:\abc.txt", r"C:\"),
            (r"扩展名 \\server\share\x.tar.gz", "gz"),
            (r"上一级 \\server\share\目录\abc", r"\\server\share\目录"),
        ] {
            assert_eq!(path_component(query).as_deref(), Ok(expected), "{query}");
        }
        assert_eq!(path_component(r"文件名 C:\"), Err(PathError::MissingPart));
    }
}
