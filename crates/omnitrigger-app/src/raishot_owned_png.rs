//! 仅保管宿主新建的唯一会话及一个 PNG；不提供任意路径读写或递归清理。
#![cfg(windows)]
use super::protocol::Image;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    path::{Path, PathBuf},
};

#[repr(C)]
struct FileInfo {
    attributes: u32,
    _times: [u32; 6],
    _volume: u32,
    _size_high: u32,
    _size_low: u32,
    links: u32,
    _index_high: u32,
    _index_low: u32,
}
#[link(name = "kernel32")]
unsafe extern "system" {
    #[link_name = "GetFileInformationByHandle"]
    fn get_file_information_by_handle(handle: *mut std::ffi::c_void, info: *mut FileInfo) -> i32;
}
fn inspect(file: &File, directory: bool) -> Result<(), &'static str> {
    let mut info = std::mem::MaybeUninit::<FileInfo>::uninit();
    // SAFETY: 有效 File 句柄及完整可写 Win32 结构；成功后才读取。
    if unsafe { get_file_information_by_handle(file.as_raw_handle(), info.as_mut_ptr()) } == 0 {
        return Err("capture_owned_file_unavailable");
    }
    // SAFETY: 上面的 Win32 调用已成功填满该结构。
    let info = unsafe { info.assume_init() };
    if info.attributes & 0x441400 != 0
        || (info.attributes & 0x10 != 0) != directory
        || (!directory && info.links != 1)
    {
        return Err("capture_owned_file_unavailable");
    }
    Ok(())
}
fn pin_directory(path: &Path) -> Result<File, &'static str> {
    // 不共享 DELETE：保管期间禁止替换/重命名该层路径；允许普通子文件创建。
    let file = OpenOptions::new()
        .access_mode(0x80)
        .share_mode(3)
        .custom_flags(0x02000000 | 0x00200000)
        .open(path)
        .map_err(|_| "capture_owned_file_unavailable")?;
    inspect(&file, true)?;
    Ok(file)
}
fn image_file(path: &Path) -> Result<File, &'static str> {
    // 只申请 READ 并共享 READ，核验/保管期间禁止写入与替换，兼容普通只读消费者。
    let file = OpenOptions::new()
        .access_mode(0x80000000)
        .share_mode(1)
        .custom_flags(0x00200000)
        .open(path)
        .map_err(|_| "capture_owned_file_unavailable")?;
    inspect(&file, false)?;
    Ok(file)
}
pub fn pin_program(path: &Path) -> Result<Vec<File>, &'static str> {
    crate::plugin_install::ensure_plain_ancestors(path).map_err(|_| "raishot_unavailable")?;
    let parent = path.parent().ok_or("raishot_unavailable")?;
    let mut held = Vec::new();
    for ancestor in parent.ancestors().collect::<Vec<_>>().into_iter().rev() {
        held.push(pin_directory(ancestor)?);
    }
    let executable = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .custom_flags(0x00200000)
        .open(path)
        .map_err(|_| "raishot_unavailable")?;
    inspect(&executable, false)?;
    held.push(executable);
    Ok(held)
}
pub struct Session {
    pub directory: PathBuf,
    _directories: Vec<File>,
    image: Option<File>,
}
impl Session {
    pub fn create(root: &Path, id: &str) -> Result<Self, &'static str> {
        if !super::protocol::hex(id, 32) {
            return Err("invalid_input");
        }
        let directory = root.join(format!("capture-{id}"));
        let path = directory.to_str().ok_or("capture_owned_file_unavailable")?;
        if path.len() > 2048
            || path.as_bytes().get(1) != Some(&b':')
            || !path.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
            || !matches!(path.as_bytes().get(2), Some(b'\\' | b'/'))
        {
            return Err("capture_owned_file_unavailable");
        }
        crate::plugin_install::ensure_plain_ancestors(&directory)
            .map_err(|_| "capture_owned_file_unavailable")?;
        let parent = root.parent().ok_or("capture_owned_file_unavailable")?;
        let mut held = Vec::new();
        for ancestor in parent.ancestors().collect::<Vec<_>>().into_iter().rev() {
            held.push(pin_directory(ancestor)?);
        }
        match fs::create_dir(root) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err("capture_owned_file_unavailable"),
        }
        held.push(pin_directory(root)?);
        // create_dir 是唯一占用点；绝不复用已有目录或导入脚本提供的目录。
        fs::create_dir(&directory).map_err(|_| "capture_owned_file_unavailable")?;
        held.push(pin_directory(&directory)?);
        if fs::read_dir(&directory)
            .map_err(|_| "capture_owned_file_unavailable")?
            .next()
            .is_some()
        {
            return Err("capture_owned_file_unavailable");
        }
        Ok(Self {
            directory,
            _directories: held,
            image: None,
        })
    }
    pub fn path(&self) -> PathBuf {
        self.directory.join("image.png")
    }
    pub fn verify(
        &mut self,
        image: &Image,
        check: &mut dyn FnMut() -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        check()?;
        // 路径只由新建会话 + 固定相对文件名组合，不采用进程回传的路径。
        let mut file = image_file(&self.path())?;
        let expected = image.bytes().ok_or("capture_result_unconfirmed")?;
        if file
            .metadata()
            .map_err(|_| "capture_result_unconfirmed")?
            .len()
            != expected
        {
            return Err("capture_result_unconfirmed");
        }
        let mut header = [0u8; 33];
        file.read_exact(&mut header)
            .map_err(|_| "capture_result_unconfirmed")?;
        if header[..8] != *b"\x89PNG\r\n\x1a\n"
            || header[8..16] != *b"\0\0\0\rIHDR"
            || u32::from_be_bytes(
                header[16..20]
                    .try_into()
                    .map_err(|_| "capture_result_unconfirmed")?,
            ) != image.width
            || u32::from_be_bytes(
                header[20..24]
                    .try_into()
                    .map_err(|_| "capture_result_unconfirmed")?,
            ) != image.height
            || header[24..29] != [8, 6, 0, 0, 0]
        {
            return Err("capture_result_unconfirmed");
        }
        file.seek(SeekFrom::Start(0))
            .map_err(|_| "capture_result_unconfirmed")?;
        let mut hash = Sha256::new();
        let mut count = 0u64;
        let mut buffer = [0u8; 65536];
        loop {
            check()?;
            let n = file
                .read(&mut buffer)
                .map_err(|_| "capture_result_unconfirmed")?;
            if n == 0 {
                break;
            }
            count = count
                .checked_add(n as u64)
                .ok_or("capture_result_unconfirmed")?;
            if count > expected {
                return Err("capture_result_unconfirmed");
            }
            hash.update(&buffer[..n]);
        }
        check()?;
        if count != expected || format!("{:x}", hash.finalize()) != image.sha256 {
            return Err("capture_result_unconfirmed");
        }
        self.image = Some(file);
        Ok(())
    }
    pub fn release_image(
        &mut self,
        check: &mut dyn FnMut() -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        check()?;
        if self.image.is_none() {
            match fs::symlink_metadata(self.path()) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(_) => return Err("capture_release_unconfirmed"),
                Ok(_) => self.image = Some(image_file(&self.path())?),
            }
        }
        let file = self.image.as_ref().ok_or("capture_release_unconfirmed")?;
        inspect(file, false)?;
        check()?;
        // 释放读锁后，只 unlink 自有且仍被逐层保管的目录中的固定 PNG 项。
        // remove_file 不递归、不跟随文件符号链接删除其目标；不按回执路径删除。
        // 外部只读消费者未共享 DELETE 时可明确失败，由调用者日后显式重试释放。
        self.image.take();
        fs::remove_file(self.path()).map_err(|_| "capture_release_unconfirmed")?;
        // 保留会话设置目录；不枚举、递归删除或接触该目录以外的用户文件。
        Ok(())
    }
}
