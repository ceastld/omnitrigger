use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PathError {
    Incomplete,
    UnknownOperation,
    InvalidPath,
    MissingPart,
}

impl PathError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "在操作名后粘贴路径",
            Self::UnknownOperation => "可用 文件名、主名、扩展名 或 上一级",
            Self::InvalidPath => "输入单条路径，可用一对双引号包住",
            Self::MissingPart => "这条路径没有所选部分",
        }
    }
}

pub(super) fn path_component(query: &str) -> Result<String, PathError> {
    let (operation, input) = query
        .split_once(char::is_whitespace)
        .ok_or(PathError::Incomplete)?;
    let input = strip_path_quotes(input.trim());
    if input.is_empty() {
        return Err(PathError::Incomplete);
    }
    if input.contains(['\0', '"', '\r', '\n', '\t']) {
        return Err(PathError::InvalidPath);
    }
    let path = Path::new(input);
    let component = match operation.to_ascii_lowercase().as_str() {
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
