//! 导入动作时沿用已安装包的位置。目录名由用户选择，热键和轮盘通过清单标识查找动作。
use super::{metadata, move_new};
use omnitrigger_actions::catalog::{is_legacy_stage, read_package_manifest, validate_manifest};
use std::{collections::BTreeMap, ffi::OsString, fs, io, path::Path};

fn package_folders(actions: &Path) -> io::Result<BTreeMap<String, Vec<OsString>>> {
    let mut packages = BTreeMap::<String, Vec<OsString>>::new();
    if metadata(actions)?.is_none() {
        return Ok(packages);
    }
    for entry in fs::read_dir(actions)? {
        let entry = entry?;
        let name = entry.file_name();
        let folder = entry.path();
        if is_legacy_stage(&name)
            || !metadata(&folder)?.is_some_and(|meta| meta.is_dir())
            || metadata(&folder.join("action.json"))?.is_none()
        {
            continue;
        }
        let manifest = match read_package_manifest(&folder) {
            Ok(manifest) => manifest,
            // 未完成的动作工程仍可作为普通文件备份和导入。
            Err(error) if error.code == "invalid_manifest" => continue,
            Err(error) => return Err(io::Error::other(format!("{}：{error}", folder.display()))),
        };
        packages.entry(manifest.id).or_default().push(name);
    }
    Ok(packages)
}

fn unique<'a>(id: &str, names: &'a [OsString], location: &str) -> io::Result<&'a OsString> {
    if let [name] = names {
        return Ok(name);
    }
    let names = names
        .iter()
        .map(|name| name.to_string_lossy())
        .collect::<Vec<_>>()
        .join("、");
    Err(io::Error::other(format!(
        "{location}的动作包 {id} 有多个目录（{names}），请确认要保留的版本后重新导入。"
    )))
}

pub(super) fn align(pending: &Path, backup: &Path, root: &Path) -> io::Result<()> {
    let actions = pending.join("incoming/actions");
    if metadata(&actions)?.is_none() {
        return Ok(());
    }
    let incoming = package_folders(&actions)?;
    let installed = package_folders(&backup.join("actions"))?;
    let daily = root.join("actions");
    if metadata(&daily)?.is_some_and(|meta| !meta.is_dir()) {
        return Err(super::invalid(&daily));
    }
    let holding = pending.join("packages");
    fs::create_dir(&holding)?;
    // 先移走暂存包，来源和目标的目录名即使互换也可以完整保留两份内容。
    let mut packages = Vec::new();
    for (id, names) in &incoming {
        let name = unique(id, names, "外部副本")?;
        let temporary = holding.join(packages.len().to_string());
        move_new(&actions.join(name), &temporary)?;
        packages.push((id, name, temporary));
    }
    // 无法解读身份的旧清单可以原样复制，但不能覆盖当前可识别的包。
    for name in installed.values().flatten() {
        let folder = actions.join(name);
        if metadata(&folder)?.is_some_and(|meta| meta.is_dir())
            && metadata(&folder.join("action.json"))?.is_some()
        {
            return Err(io::Error::other(format!(
                "外部副本的 {} 清单无法识别，无法替换同目录的日常动作包。",
                name.to_string_lossy()
            )));
        }
    }
    let mut suffix = 0;
    for (id, original, temporary) in packages {
        let existing = installed
            .get(id)
            .map(|names| unique(id, names, "日常档"))
            .transpose()?;
        let mut name = existing.unwrap_or(original).clone();
        if existing.is_some() {
            let manifest = read_package_manifest(&temporary).map_err(io::Error::other)?;
            validate_manifest(&manifest).map_err(io::Error::other)?;
        } else {
            // 文件系统决定名称是否相同，覆盖 Windows 大小写和非 ASCII 名称。
            while metadata(&daily.join(&name))?.is_some()
                || metadata(&actions.join(&name))?.is_some()
            {
                if id == crate::legacy_plugin_work::PACKAGE_ID {
                    return Err(io::Error::other(format!(
                        "兼容宿主的安装目录 {} 已被占用。",
                        name.to_string_lossy()
                    )));
                }
                suffix += 1;
                name = format!("imported-{suffix}").into();
            }
        }
        move_new(&temporary, &actions.join(&name)).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("无法导入动作包 {id} 到 {}：{error}", name.to_string_lossy()),
            )
        })?;
    }
    Ok(())
}
