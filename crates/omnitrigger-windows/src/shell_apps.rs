//! 读取 Windows 的 Applications 虚拟目录，查询和启动均从工作线程调用。

use std::ffi::c_void;

use windows::Win32::{
    System::Com::CoTaskMemFree,
    UI::Shell::{
        BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, SHCreateItemFromIDList,
        SHGetKnownFolderIDList, SIGDN, SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_NORMALDISPLAY,
    },
};

use super::automation_basics::{open_shell_app, ShellApartment};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellApp {
    title: String,
    parsing_name: String,
}

impl ShellApp {
    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn identifier(&self) -> &str {
        self.parsing_name
            .rsplit_once('\\')
            .map_or(&self.parsing_name, |(_, name)| name)
    }

    /// 由 Shell 解释其返回的条目身份，参数留空。
    pub fn open(&self) -> Result<(), String> {
        open_shell_app(&self.parsing_name)
    }
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
        if let (Some(title), Some(parsing_name)) = (
            display_name(&item, SIGDN_NORMALDISPLAY),
            display_name(&item, SIGDN_DESKTOPABSOLUTEPARSING),
        ) {
            apps.push(ShellApp {
                title,
                parsing_name,
            });
        }
    }
    Ok(apps)
}

pub fn list_shell_apps() -> Result<Vec<ShellApp>, String> {
    let _com = ShellApartment::enter()?;
    enumerate_apps().map_err(|error| format!("无法读取 Windows 应用目录（{error}）"))
}
