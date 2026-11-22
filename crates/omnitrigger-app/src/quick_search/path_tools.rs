use std::path::Path;

const INVALID_PATH_TEXT: [char; 5] = ['\0', '"', '\r', '\n', '\t'];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PathError {
    Incomplete,
    UnknownOperation,
    InvalidPath,
    ExtensionArguments,
    InvalidExtension,
    MissingPart,
}

impl PathError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "在操作名后粘贴路径",
            Self::UnknownOperation => "可用操作为文件名、主名、扩展名、换扩展名和上一级",
            Self::InvalidPath => "输入单条路径，可用一对双引号包住",
            Self::ExtensionArguments => "用 JSON 字符串数组填写路径和新扩展名",
            Self::InvalidExtension => "扩展名须为文件名后缀，可用空字符串删除",
            Self::MissingPart => "这条路径没有所选部分",
        }
    }
}

pub(super) fn path_component(query: &str) -> Result<String, PathError> {
    let (operation, input) = query
        .split_once(char::is_whitespace)
        .ok_or(PathError::Incomplete)?;
    let operation = operation.to_ascii_lowercase();
    if matches!(operation.as_str(), "换扩展名" | "replace-extension") {
        return replace_extension(input.trim());
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
