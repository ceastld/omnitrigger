use std::{fs::Metadata, path::Path};

pub(crate) fn is_cloud_path(path: &Path) -> bool {
    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy().to_ascii_lowercase();
        name.starts_with("onedrive") || name == "dropbox" || name.starts_with("icloud")
    })
}

/// 调用方提供不跟随链接取得的元数据。
pub(crate) fn is_plain(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows::Win32::Storage::FileSystem::{
            FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS,
            FILE_ATTRIBUTE_RECALL_ON_OPEN, FILE_ATTRIBUTE_REPARSE_POINT,
        };
        let unavailable = FILE_ATTRIBUTE_REPARSE_POINT
            | FILE_ATTRIBUTE_OFFLINE
            | FILE_ATTRIBUTE_RECALL_ON_OPEN
            | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS;
        metadata.file_attributes() & unavailable.0 == 0
    }
    #[cfg(not(windows))]
    {
        !metadata.file_type().is_symlink()
    }
}

pub(crate) fn is_plain_directory(path: &Path) -> bool {
    if is_cloud_path(path) {
        return false;
    }
    // 先检查祖先，再接触其中的子路径。
    for ancestor in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
        let Ok(metadata) = ancestor.symlink_metadata() else {
            return false;
        };
        if !metadata.is_dir() || !is_plain(&metadata) {
            return false;
        }
    }
    true
}
