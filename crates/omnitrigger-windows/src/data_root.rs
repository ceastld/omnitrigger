//! 数据根的系统适配：探针取得目录的最终路径、经系统外壳脱离打包上下文重启、阻断提示。
//!
//! 打包应用（MSIX）启动的子进程对 `%LOCALAPPDATA%` 的写入会被重定向到该应用私有目录，
//! 进程自身无法察觉。这里只报告事实，是否重启、如何提示由应用层决定。

use std::path::{Path, PathBuf};

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn strip_verbatim(path: String) -> PathBuf {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = path.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        PathBuf::from(path)
    }
}

/// 在目录中创建一个关闭即删除的探针文件，返回探针所在目录的最终路径。
/// 只创建探针，不读取或修改目录中的其他文件。
pub fn probe_final_directory(directory: &Path) -> Result<PathBuf, String> {
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE},
            Storage::FileSystem::{
                CreateFileW, GetFinalPathNameByHandleW, CREATE_NEW, FILE_ATTRIBUTE_HIDDEN,
                FILE_ATTRIBUTE_TEMPORARY, FILE_FLAG_DELETE_ON_CLOSE, FILE_NAME_NORMALIZED,
                FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
            },
        },
    };
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    let probe = directory.join(format!(".root-probe-{}-{nanos:x}", std::process::id()));
    let name = wide(&probe.to_string_lossy());
    // SAFETY: 路径以 NUL 结尾；成功返回的句柄由下方唯一关闭。
    let handle = unsafe {
        CreateFileW(
            PCWSTR(name.as_ptr()),
            (GENERIC_READ | GENERIC_WRITE).0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            CREATE_NEW,
            FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_TEMPORARY | FILE_FLAG_DELETE_ON_CLOSE,
            None,
        )
    }
    .map_err(|error| format!("无法在 {} 创建探针：{error}", directory.display()))?;
    let mut buffer = vec![0u16; 32_768];
    // SAFETY: 句柄有效，缓冲区长度固定。
    let length =
        unsafe { GetFinalPathNameByHandleW(handle, &mut buffer, FILE_NAME_NORMALIZED) } as usize;
    // SAFETY: 句柄只在此关闭一次；关闭即删除探针。
    let _ = unsafe { CloseHandle(handle) };
    if length == 0 || length >= buffer.len() {
        return Err(format!("无法取得 {} 的最终路径。", directory.display()));
    }
    let final_path = strip_verbatim(String::from_utf16_lossy(&buffer[..length]));
    final_path
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("探针路径无效：{}", final_path.display()))
}

/// 取得已存在文件的最终路径；用于把重定向上下文中的程序路径换成系统外壳可见的真实路径。
pub fn final_file_path(path: &Path) -> Result<PathBuf, String> {
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::CloseHandle,
            Storage::FileSystem::{
                CreateFileW, GetFinalPathNameByHandleW, FILE_FLAGS_AND_ATTRIBUTES,
                FILE_NAME_NORMALIZED, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
                FILE_SHARE_WRITE, OPEN_EXISTING,
            },
        },
    };
    let name = wide(&path.to_string_lossy());
    // SAFETY: 路径以 NUL 结尾；成功返回的句柄由下方唯一关闭。
    let handle = unsafe {
        CreateFileW(
            PCWSTR(name.as_ptr()),
            FILE_READ_ATTRIBUTES.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
    }
    .map_err(|error| format!("无法打开 {}：{error}", path.display()))?;
    let mut buffer = vec![0u16; 32_768];
    // SAFETY: 句柄有效，缓冲区长度固定。
    let length =
        unsafe { GetFinalPathNameByHandleW(handle, &mut buffer, FILE_NAME_NORMALIZED) } as usize;
    // SAFETY: 句柄只在此关闭一次。
    let _ = unsafe { CloseHandle(handle) };
    if length == 0 || length >= buffer.len() {
        return Err(format!("无法取得 {} 的最终路径。", path.display()));
    }
    Ok(strip_verbatim(String::from_utf16_lossy(&buffer[..length])))
}

/// 经系统外壳重新启动程序：写一个快捷方式并交给 `explorer.exe` 打开，
/// 新进程由外壳创建，不继承打包应用的重定向上下文。`arguments` 须已按命令行规则转义。
/// 快捷方式写到 `link`，由新进程启动后删除。
pub fn relaunch_via_shell(
    program: &Path,
    arguments: &str,
    working_directory: &Path,
    link: &Path,
) -> Result<(), String> {
    use windows::{
        core::{Interface, PCWSTR},
        Win32::{
            System::Com::{
                CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile,
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
            },
            UI::Shell::{IShellLinkW, ShellLink},
        },
    };
    // SAFETY: 本线程初始化 COM，下方成功时对称反初始化。
    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
    let result = (|| -> windows::core::Result<()> {
        // SAFETY: 标准外壳快捷方式对象；所有字符串以 NUL 结尾且在调用期间有效。
        unsafe {
            let shell_link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            let program = wide(&program.to_string_lossy());
            let arguments = wide(arguments);
            let directory = wide(&working_directory.to_string_lossy());
            shell_link.SetPath(PCWSTR(program.as_ptr()))?;
            shell_link.SetArguments(PCWSTR(arguments.as_ptr()))?;
            shell_link.SetWorkingDirectory(PCWSTR(directory.as_ptr()))?;
            let file: IPersistFile = shell_link.cast()?;
            let link = wide(&link.to_string_lossy());
            file.Save(PCWSTR(link.as_ptr()), true)?;
        }
        Ok(())
    })();
    if initialized {
        // SAFETY: 与上方成功的 CoInitializeEx 对称。
        unsafe { CoUninitialize() };
    }
    result.map_err(|error| format!("无法写入重启快捷方式：{error}"))?;
    let explorer = std::env::var_os("WINDIR")
        .map(|root| PathBuf::from(root).join("explorer.exe"))
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows\explorer.exe"));
    std::process::Command::new(explorer)
        .arg(link)
        .spawn()
        .map(drop)
        .map_err(|error| format!("无法经系统外壳重启：{error}"))
}

/// 阻断式错误提示。
pub fn show_error_dialog(title: &str, text: &str) {
    use windows::{
        core::PCWSTR,
        Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK},
    };
    let title = wide(title);
    let text = wide(text);
    // SAFETY: 字符串以 NUL 结尾且在调用期间有效；无属主窗口。
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONERROR,
        )
    };
}

/// 三选一提示：`Some(true)` 为“是”，`Some(false)` 为“否”，`None` 为取消或关闭。
pub fn ask_yes_no_cancel(title: &str, text: &str) -> Option<bool> {
    use windows::{
        core::PCWSTR,
        Win32::UI::WindowsAndMessaging::{
            MessageBoxW, IDNO, IDYES, MB_DEFBUTTON1, MB_ICONQUESTION, MB_YESNOCANCEL,
        },
    };
    let title = wide(title);
    let text = wide(text);
    // SAFETY: 字符串以 NUL 结尾且在调用期间有效；无属主窗口。
    let answer = unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_YESNOCANCEL | MB_ICONQUESTION | MB_DEFBUTTON1,
        )
    };
    match answer {
        IDYES => Some(true),
        IDNO => Some(false),
        _ => None,
    }
}
