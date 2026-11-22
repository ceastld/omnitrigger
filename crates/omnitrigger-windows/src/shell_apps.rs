//! 读取 Windows 的 Applications 虚拟目录，并在工作线程上打开 Shell 条目。

use std::{ffi::c_void, mem::size_of};

use windows::core::GUID;
use windows::Win32::{
    System::Com::CoTaskMemFree,
    UI::{
        Shell::{
            BHID_EnumItems, FOLDERID_AppsFolder, FOLDERID_Desktop, FOLDERID_Documents,
            FOLDERID_Downloads, FOLDERID_Music, FOLDERID_Pictures, FOLDERID_Videos,
            IEnumShellItems, ILGetSize, IShellItem, SHCreateItemFromIDList, SHGetIDListFromObject,
            SHGetKnownFolderIDList, SHGetKnownFolderPath, ShellExecuteExW, KF_FLAG_DONT_VERIFY,
            SEE_MASK_FLAG_NO_UI, SEE_MASK_INVOKEIDLIST, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, SIGDN,
            SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING,
        },
        WindowsAndMessaging::SW_SHOWNORMAL,
    },
};

use super::automation_basics::ShellApartment;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellApp {
    title: String,
    identifier: String,
    pidl: Vec<u8>,
}

impl ShellApp {
    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    /// 按 Windows 返回的条目 ID 调用该应用的默认操作。
    pub fn open(mut self) -> Result<(), String> {
        let _com = ShellApartment::enter()?;
        execute_shell_item(self.pidl.as_mut_ptr().cast(), 0)
            .map_err(|error| format!("系统未能启动应用（{error}）"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellFolder {
    Desktop,
    Downloads,
    Documents,
    Pictures,
    Music,
    Videos,
}

impl ShellFolder {
    fn id(self) -> GUID {
        match self {
            Self::Desktop => FOLDERID_Desktop,
            Self::Downloads => FOLDERID_Downloads,
            Self::Documents => FOLDERID_Documents,
            Self::Pictures => FOLDERID_Pictures,
            Self::Music => FOLDERID_Music,
            Self::Videos => FOLDERID_Videos,
        }
    }

    /// 读取当前配置的文件夹路径，路径存在性由使用方检查。
    pub fn filesystem_path(self) -> Result<String, String> {
        let path = unsafe { SHGetKnownFolderPath(&self.id(), KF_FLAG_DONT_VERIFY, None) }
            .map_err(|error| format!("系统未能读取文件夹路径（{error}）"))?;
        let _memory = ShellMemory(path.0.cast());
        if path.is_null() {
            return Err("系统未能读取文件夹路径".into());
        }
        unsafe { path.to_string() }
            .ok()
            .filter(|path| !path.is_empty())
            .ok_or_else(|| "系统未返回可用的文件夹路径".to_owned())
    }

    pub fn open(self) -> Result<(), String> {
        let _com = ShellApartment::enter()?;
        let pidl = unsafe { SHGetKnownFolderIDList(&self.id(), 0, None) }
            .map_err(|error| format!("系统未能找到文件夹（{error}）"))?;
        if pidl.is_null() {
            return Err("系统未能找到文件夹".into());
        }
        let _memory = ShellMemory(pidl.cast());
        execute_shell_item(pidl.cast(), SEE_MASK_NOASYNC)
            .map_err(|error| format!("系统未能打开文件夹（{error}）"))
    }
}

fn execute_shell_item(pidl: *mut c_void, flags: u32) -> windows::core::Result<()> {
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_FLAG_NO_UI | SEE_MASK_INVOKEIDLIST | flags,
        lpIDList: pidl,
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // 调用方在整个 Shell 调用期间都持有有效的 ID 列表与 COM 初始化。
    unsafe { ShellExecuteExW(&mut info) }
}

struct ShellMemory(*mut c_void);

impl Drop for ShellMemory {
    fn drop(&mut self) {
        unsafe { CoTaskMemFree(Some(self.0)) };
    }
}

fn display_name(item: &IShellItem, form: SIGDN) -> Option<String> {
    let name = unsafe { item.GetDisplayName(form) }.ok()?;
    let _memory = ShellMemory(name.0.cast());
    if name.is_null() {
        return None;
    }
    unsafe { name.to_string() }
        .ok()
        .filter(|value| !value.is_empty())
}

fn read_app(item: &IShellItem) -> Option<ShellApp> {
    let title = display_name(item, SIGDN_NORMALDISPLAY)?;
    let identifier = display_name(item, SIGDN_PARENTRELATIVEPARSING).unwrap_or_default();
    let pidl = unsafe { SHGetIDListFromObject(item) }.ok()?;
    let _memory = ShellMemory(pidl.cast());
    if pidl.is_null() {
        return None;
    }
    let length = unsafe { ILGetSize(Some(pidl)) } as usize;
    if length < size_of::<u16>() {
        return None;
    }
    // 保留 Shell 返回的完整 ID 列表及终止项，条目可以在加载和启动线程之间传递。
    let pidl = unsafe { std::slice::from_raw_parts(pidl.cast::<u8>(), length) }.to_vec();
    Some(ShellApp {
        title,
        identifier,
        pidl,
    })
}

fn enumerate_apps() -> windows::core::Result<Vec<ShellApp>> {
    let folder_pidl = unsafe { SHGetKnownFolderIDList(&FOLDERID_AppsFolder, 0, None) }?;
    let _memory = ShellMemory(folder_pidl.cast());
    let folder: IShellItem = unsafe { SHCreateItemFromIDList(folder_pidl) }?;
    let items: IEnumShellItems = unsafe { folder.BindToHandler(None, &BHID_EnumItems) }?;
    let mut apps = Vec::new();
    loop {
        let mut entry = [None];
        unsafe { items.Next(&mut entry, None) }?;
        let [Some(item)] = entry else { break };
        if let Some(app) = read_app(&item) {
            apps.push(app);
        }
    }
    Ok(apps)
}

pub fn list_shell_apps() -> Result<Vec<ShellApp>, String> {
    let _com = ShellApartment::enter()?;
    enumerate_apps().map_err(|error| format!("无法读取 Windows 应用目录（{error}）"))
}
