//! 仅供已核验本机事务使用的普通同卷、不覆盖移动；不执行程序或强制POSIX重命名。
use std::{io, path::Path};

#[cfg(windows)]
pub fn move_owned_path_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::MetadataExt;
    use windows::{
        core::PCWSTR,
        Win32::Storage::FileSystem::{MoveFileExW, MOVE_FILE_FLAGS},
    };
    fn wide(path: &Path) -> io::Result<Vec<u16>> {
        use std::path::{Component, Prefix};
        if !path.is_absolute()
            || path
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "必须是无相对组件的本机绝对路径",
            ));
        }
        let verbatim = match path.components().next() {
            Some(Component::Prefix(prefix)) => match prefix.kind() {
                Prefix::Disk(_) => false,
                Prefix::VerbatimDisk(_) => true,
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "不接受UNC或设备路径",
                    ))
                }
            },
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "路径缺少磁盘前缀",
                ))
            }
        };
        // Win32扩展磁盘前缀：双反斜线、问号、反斜线；不依赖系统长路径开关。
        let mut value: Vec<u16> = if verbatim {
            Vec::new()
        } else {
            vec![92, 92, 63, 92]
        };
        value.extend(path.as_os_str().encode_wide().map(|c| {
            if c == b'/' as u16 {
                b'\\' as u16
            } else {
                c
            }
        }));
        if value.contains(&0) || value.len() >= 32767 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "路径包含NUL或超长",
            ));
        }
        value.push(0);
        Ok(value)
    }
    let from = wide(source)?;
    let to = wide(destination)?;
    let metadata = std::fs::symlink_metadata(source)?;
    if metadata.file_attributes() & 0x441400 != 0 || (!metadata.is_file() && !metadata.is_dir()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "源不是可移动的普通本机路径",
        ));
    }
    // SAFETY: 两个UTF-16缓冲区在调用期间存活且以NUL结束。flags=0不允许覆盖或跨卷复制，
    // 不使用延后重启、POSIX或解除句柄；调用者仍负责已核验根/祖先与事务归属。
    unsafe {
        MoveFileExW(
            PCWSTR(from.as_ptr()),
            PCWSTR(to.as_ptr()),
            MOVE_FILE_FLAGS(0),
        )
    }
    .map_err(|error| io::Error::from_raw_os_error(error.code().0 & 0xffff))
}

#[cfg(not(windows))]
pub fn move_owned_path_no_replace(_: &Path, _: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "仅支持Windows同卷事务",
    ))
}
