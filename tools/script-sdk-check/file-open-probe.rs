#![windows_subsystem = "windows"]

// 文件启动验收专用：没有窗口、控制台、注册表、剪贴板或个人文件访问。
use std::{env, fs::OpenOptions, io::Write, process};

fn write_marker() -> std::io::Result<()> {
    let executable = env::current_exe()?;
    let directory = executable
        .parent()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "无法定位验收目录"))?;
    // 每次验收必须使用新目录；绝不覆盖旧 marker 或跟随已存在的同名文件。
    let mut marker = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join(".opened.json"))?;
    writeln!(
        marker,
        "{{\"opened\":true,\"process_id\":{}}}",
        process::id()
    )?;
    marker.sync_all()
}

fn main() {
    if write_marker().is_err() {
        process::exit(1);
    }
}
