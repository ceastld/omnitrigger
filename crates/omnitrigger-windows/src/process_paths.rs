//! 只读查询：运行中进程的映像路径与当前用户的“文档”目录。不注入、不终止进程。

use std::path::PathBuf;

/// 返回映像文件名（忽略大小写）属于 `names` 的运行中进程的完整路径。
/// 无权访问的进程直接跳过，结果按发现顺序去重。
pub fn running_process_paths(names: &[&str]) -> Vec<PathBuf> {
    use windows::{
        core::PWSTR,
        Win32::{
            Foundation::CloseHandle,
            System::{
                Diagnostics::ToolHelp::{
                    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
                    TH32CS_SNAPPROCESS,
                },
                Threading::{
                    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
                    PROCESS_QUERY_LIMITED_INFORMATION,
                },
            },
        },
    };
    let mut paths = Vec::new();
    let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
        return paths;
    };
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut next = unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok();
    while next {
        let length = entry
            .szExeFile
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(entry.szExeFile.len());
        let name = String::from_utf16_lossy(&entry.szExeFile[..length]);
        if names
            .iter()
            .any(|wanted| wanted.eq_ignore_ascii_case(&name))
        {
            if let Ok(process) = unsafe {
                OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION,
                    false,
                    entry.th32ProcessID,
                )
            } {
                let mut buffer = vec![0u16; 32_768];
                let mut size = buffer.len() as u32;
                let queried = unsafe {
                    QueryFullProcessImageNameW(
                        process,
                        PROCESS_NAME_WIN32,
                        PWSTR(buffer.as_mut_ptr()),
                        &mut size,
                    )
                };
                let _ = unsafe { CloseHandle(process) };
                if queried.is_ok() {
                    let path = PathBuf::from(String::from_utf16_lossy(&buffer[..size as usize]));
                    if !paths.contains(&path) {
                        paths.push(path);
                    }
                }
            }
        }
        next = unsafe { Process32NextW(snapshot, &mut entry) }.is_ok();
    }
    let _ = unsafe { CloseHandle(snapshot) };
    paths
}

/// 当前用户的“文档”目录；用户迁移过文档位置时也返回实际位置。
pub fn documents_dir() -> Option<PathBuf> {
    use windows::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{FOLDERID_Documents, SHGetKnownFolderPath, KF_FLAG_DEFAULT},
    };
    let path = unsafe { SHGetKnownFolderPath(&FOLDERID_Documents, KF_FLAG_DEFAULT, None) }.ok()?;
    let result = unsafe { path.to_string() }.ok().map(PathBuf::from);
    unsafe { CoTaskMemFree(Some(path.0 as *const _)) };
    result
}
