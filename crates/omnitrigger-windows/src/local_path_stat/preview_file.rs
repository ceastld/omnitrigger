use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    os::windows::{
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
};

use windows::Win32::{
    Foundation::HANDLE,
    Storage::FileSystem::{
        FILE_FLAG_OPEN_NO_RECALL, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
    },
};

use super::{
    classify_attributes, error_for_os, stat_local_path, system_step, verify_opened_path,
    LocalPathKind, LocalStatError,
};

/// 读取本地普通文件的有界字节前缀。路径及其祖先须符合本地元数据查询规则。
/// 查询与打开之间不构成原子快照；读取前核对实际打开的句柄。
pub fn read_local_preview_file(
    path: &str,
    byte_limit: usize,
    check: impl Fn() -> Result<(), LocalStatError>,
) -> Result<Vec<u8>, LocalStatError> {
    with_local_preview_file(path, check, |file| {
        let mut bytes = Vec::new();
        file.take(byte_limit as u64).read_to_end(&mut bytes)?;
        Ok(bytes)
    })
}

#[derive(Debug, PartialEq, Eq)]
pub struct LocalPreviewFileTail {
    pub offset: u64,
    pub prefix: Vec<u8>,
    pub bytes: Vec<u8>,
}

/// 读取本地文件头部最多三个字节及有界末尾，以便调用方识别编码和字符边界。
pub fn read_local_preview_file_tail(
    path: &str,
    byte_limit: usize,
    check: impl Fn() -> Result<(), LocalStatError>,
) -> Result<LocalPreviewFileTail, LocalStatError> {
    with_local_preview_file(path, check, |mut file| {
        let offset = file.metadata()?.len().saturating_sub(byte_limit as u64);
        let mut prefix = Vec::new();
        (&mut file).take(3).read_to_end(&mut prefix)?;
        file.seek(SeekFrom::Start(offset))?;
        let mut bytes = Vec::new();
        file.take(byte_limit as u64).read_to_end(&mut bytes)?;
        Ok(LocalPreviewFileTail {
            offset,
            prefix,
            bytes,
        })
    })
}

fn with_local_preview_file<T>(
    path: &str,
    check: impl Fn() -> Result<(), LocalStatError>,
    read: impl FnOnce(&File) -> std::io::Result<T>,
) -> Result<T, LocalStatError> {
    require_file(stat_local_path(path, &check)?)?;
    let native_path = format!("\\\\?\\{}", path.replace('/', "\\"));
    let file = system_step(&check, || {
        OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ.0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0 | FILE_FLAG_OPEN_NO_RECALL.0)
            .open(native_path)
            .map_err(read_error)
    })??;
    let handle = HANDLE(file.as_raw_handle());
    let attributes = system_step(&check, || file.metadata().map_err(read_error))?
        .map(|meta| meta.file_attributes())?;
    require_file(classify_attributes(Ok(attributes))?)?;
    verify_opened_path(handle, path, &check)?;
    require_file(stat_local_path(path, &check)?)?;

    let preview = system_step(&check, || read(&file).map_err(read_error))??;
    verify_opened_path(handle, path, &check)?;
    require_file(stat_local_path(path, &check)?)?;
    Ok(preview)
}

fn read_error(error: std::io::Error) -> LocalStatError {
    let mut error = error_for_os(error.raw_os_error().unwrap_or(0) as u32);
    error.message = if error.code == "access_denied" {
        "系统拒绝读取文件内容。".into()
    } else {
        "无法读取文件内容。".into()
    };
    error
}

fn require_file(kind: LocalPathKind) -> Result<(), LocalStatError> {
    let (code, message) = match kind {
        LocalPathKind::File => return Ok(()),
        LocalPathKind::Missing => ("path_missing", "文件已移走或删除。"),
        _ => ("not_file", "请选择一个普通文本文件。"),
    };
    Err(LocalStatError {
        code,
        message: message.into(),
    })
}
