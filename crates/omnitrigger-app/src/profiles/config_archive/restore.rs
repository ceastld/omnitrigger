//! 只在新建的暂存档中还原 ZIP，检查登记与内容后由调用方发布。
use super::{digest, metadata, Inventory, Result};
use crate::{action_state_store::FileActionStateStorage, profiles, script_runtime};
use chrono::{Local, TimeZone};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{self, Read, Seek},
    path::{Component, Path, PathBuf},
};
use zip::{DateTime, ZipArchive};

fn open_archive(path: &Path) -> Result<File> {
    if !path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
        || !metadata(path)?.is_some_and(|m| m.is_file() && crate::local_files::is_plain(&m))
    {
        return Err(format!("请选择本机的配置备份 ZIP 文件：{}", path.display()).into());
    }
    let mut options = File::options();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    Ok(options.open(path)?)
}

pub(in crate::profiles) fn archive_digest(path: &Path) -> std::result::Result<String, String> {
    open_archive(path)
        .and_then(|file| Ok(digest(file)?))
        .map_err(|e| e.to_string())
}

// 在任何平台都按 Windows 安装版的普通文件名解释备份。保留含中文和空格的动作资源名。
fn member_path(name: &str) -> Result<PathBuf> {
    let name = name.strip_suffix('/').unwrap_or(name);
    for part in name.split('/') {
        let stem = part
            .split('.')
            .next()
            .unwrap_or("")
            .trim_end_matches(' ')
            .to_ascii_uppercase();
        let device = matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        ) || ["COM", "LPT"].iter().any(|p| {
            stem.strip_prefix(p).is_some_and(|n| {
                matches!(
                    n,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        });
        if part.is_empty()
            || part.ends_with(['.', ' '])
            || device
            || part
                .chars()
                .any(|c| c.is_control() || r#"\:*?"<>|"#.contains(c))
            || !matches!(
                Path::new(part).components().next(),
                Some(Component::Normal(_))
            )
        {
            return Err(format!("ZIP 成员不是可还原的本机路径：{name}").into());
        }
    }
    Ok(PathBuf::from(name))
}

#[cfg(windows)]
fn check_space(target: &Path, size: u128) -> Result<()> {
    use windows::{core::HSTRING, Win32::Storage::FileSystem::GetDiskFreeSpaceExW};
    let mut available = 0;
    // 暂存目录已存在，系统只写入调用方可用的字节数。
    unsafe {
        GetDiskFreeSpaceExW(
            &HSTRING::from(target.as_os_str()),
            Some(&mut available),
            None,
            None,
        )
    }?;
    if size > u128::from(available) {
        return Err("配置档所在磁盘的可用空间不足。".into());
    }
    Ok(())
}

fn restore_time(file: &File, date: DateTime) -> Result<()> {
    if let Some(local) = Local
        .with_ymd_and_hms(
            i32::from(date.year()),
            date.month().into(),
            date.day().into(),
            date.hour().into(),
            date.minute().into(),
            date.second().into(),
        )
        .earliest()
    {
        file.set_modified(local.into())?;
    }
    Ok(())
}

fn unpack(path: &Path, expected: &str, target: &Path) -> Result<()> {
    script_runtime::check_local_path(target)?;
    if fs::read_dir(target)?.next().is_some() {
        return Err("还原配置需要空的暂存目录。".into());
    }
    let mut file = open_archive(path)?;
    if digest(&mut file)? != expected {
        return Err("所选 ZIP 与创建测试副本时的文件不同，请选择原备份或新建副本。".into());
    }
    file.rewind()?;
    let mut zip = ZipArchive::new(file)?;
    #[cfg(windows)]
    {
        // 流式写入的 ZIP 可以含有数据描述符，最终大小在中央目录中。
        let mut total = 0u128;
        for index in 0..zip.len() {
            total += u128::from(zip.by_index_raw(index)?.size());
        }
        check_space(target, total)?;
    }
    let mut files = BTreeSet::new();
    let mut folders = BTreeSet::new();
    let mut names = BTreeSet::new();
    for index in 0..zip.len() {
        let mut member = zip.by_index(index)?;
        let relative = member_path(member.name())?;
        let name = member.name().trim_end_matches('/').to_owned();
        if !names.insert(name.to_lowercase()) {
            return Err(format!("ZIP 内路径重复：{name}").into());
        }
        let dest = target.join(&relative);
        script_runtime::check_local_path(&dest)?;
        let mode = member.unix_mode().unwrap_or(0) & 0o170000;
        let expected_mode = if member.is_dir() { 0o040000 } else { 0o100000 };
        if member.is_symlink() || (mode != 0 && mode != expected_mode) {
            return Err(format!("ZIP 成员须为普通文件或目录：{name}").into());
        }
        if member.is_dir() {
            if member.size() != 0 {
                return Err(format!("ZIP 目录含文件内容：{name}").into());
            }
            fs::create_dir_all(dest)?;
            folders.insert(relative);
        } else {
            fs::create_dir_all(dest.parent().ok_or("ZIP 文件路径无效。")?)?;
            let mut output = File::create_new(&dest)?;
            let size = member.size();
            if io::copy(
                &mut member.by_ref().take(size.saturating_add(1)),
                &mut output,
            )? != size
            {
                return Err(format!("ZIP 文件长度不符：{name}").into());
            }
            if let Some(time) = member.last_modified() {
                restore_time(&output, time)?;
            }
            files.insert(name);
        }
    }
    validate_contents(target, &files, &folders)?;
    let mut file = zip.into_inner();
    file.rewind()?;
    if digest(file)? != expected {
        return Err("还原期间 ZIP 已变化，请重试。".into());
    }
    // 顶层成为新登记的测试档；子档保留各自的原身份和来源记录。
    for record in [
        "backups/seed-source.json",
        "backups/restored-revisions.json",
    ] {
        if files.contains(record) {
            fs::remove_file(target.join(record))?;
        }
    }
    Ok(())
}

fn validate_contents(
    target: &Path,
    files: &BTreeSet<String>,
    folders: &BTreeSet<PathBuf>,
) -> Result<()> {
    let inventory = Inventory::collect_profile_files(target, false)?;
    if !files.iter().eq(inventory.files.keys()) {
        return Err("ZIP 的文件与所含配置档登记或运行时清单不一致。".into());
    }
    let allowed: BTreeSet<_> = inventory
        .folders
        .iter()
        .chain(files.iter())
        .flat_map(|name| Path::new(name).ancestors().map(Path::to_owned))
        .collect();
    if !folders.is_subset(&allowed) {
        return Err("ZIP 含有配置备份范围以外的目录。".into());
    }
    profiles::restored_root_identity(target)?;
    for directory in &inventory.profiles {
        for entry in profiles::read_catalog(directory)?.profiles {
            let child = profiles::checked_profile_dir(directory, &entry)?;
            if !entry.is_test_at(&child) {
                return Err(format!("子档登记的测试身份无效：{}", entry.alias).into());
            }
            if child.join("profile.json").try_exists()? {
                let identity = profiles::reset_recovery::identity(&child)?;
                if !identity.is_test_at(&child) {
                    return Err(format!("子档身份与登记不符：{}", entry.alias).into());
                }
            }
        }
        crate::check_config(&directory.join("bindings.json"), true)?;
        let states = directory.join("action-state");
        if states.try_exists()? {
            for item in fs::read_dir(states)? {
                let path = item?.path();
                if profiles::is_action_state_document(&path) {
                    FileActionStateStorage::read_restore_document(&path)?;
                }
            }
        }
    }
    Ok(())
}

pub(in crate::profiles) fn restore_archive(
    path: &Path,
    digest: &str,
    target: &Path,
) -> std::result::Result<(), String> {
    unpack(path, digest, target).map_err(|e| format!("无法还原配置 ZIP：{e}"))
}
