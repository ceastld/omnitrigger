//! 按登记的身份恢复已准备好的新副本，或在重置中断时恢复原目录。
use super::{lock_catalog, open_profile_use_file, read_catalog, Entry};
use crate::local_files::{is_plain, is_plain_directory};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub(super) fn directory_present(path: &Path) -> io::Result<bool> {
    match path.symlink_metadata() {
        Ok(meta) if meta.is_dir() && is_plain(&meta) => Ok(true),
        Ok(_) => Err(io::Error::other(format!(
            "此位置需要一个普通本机目录：{}。请检查占用此位置的文件或目录后重试。",
            path.display()
        ))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// 发布使用不覆盖目标的改名。两个目录同时存在时，保留现场供用户核对。
pub(super) fn live_profile_present(directory: &Path) -> io::Result<bool> {
    if !directory_present(directory)? {
        return Ok(false);
    }
    let creating = directory.with_extension("creating");
    if creating.symlink_metadata().is_ok()
        && identity(&creating).is_ok_and(|staged| staged.is_test_at(directory))
    {
        return Err(io::Error::other(format!(
            "配置档目录及其待发布的副本同时存在，请核对 {} 和 {}。",
            directory.display(),
            creating.display()
        )));
    }
    // 总登记决定选中哪个档，目录里现存的身份须属于同一个 ID。
    // 旧备份只有总登记、缺少自述文件时，沿用原来的打开方式。
    let path = directory.join("profile.json");
    match identity(directory) {
        Ok(entry) => entry.check_directory_id(directory)?,
        Err(error) if error.kind() != io::ErrorKind::NotFound => {
            return Err(io::Error::new(
                error.kind(),
                format!("无法核对 {}：{error}", path.display()),
            ));
        }
        _ => {}
    }
    Ok(true)
}

pub(super) fn identity(directory: &Path) -> io::Result<Entry> {
    if !is_plain_directory(directory) {
        return Err(io::Error::other(format!(
            "配置档不是普通本机目录：{}",
            directory.display()
        )));
    }
    let path = directory.join("profile.json");
    let meta = path.symlink_metadata()?;
    if !meta.is_file() || !is_plain(&meta) {
        return Err(io::Error::other(format!(
            "配置档身份文件不可读：{}",
            path.display()
        )));
    }
    crate::local_files::parse_config_json(&fs::read_to_string(path)?).map_err(io::Error::other)
}

fn reset_tag(name: &str) -> Option<(&str, &str)> {
    let tag = name.strip_prefix('.')?.strip_suffix(".tmp")?;
    let (prefix, nonce) = tag.rsplit_once("-t")?;
    let (id, time) = prefix.rsplit_once('-')?;
    (nonce.len() == 12
        && nonce.bytes().all(|c| c.is_ascii_hexdigit())
        && !time.is_empty()
        && time.bytes().all(|c| c.is_ascii_digit()))
    .then_some((id, tag))
}

/// 只识别原目录缺失、同一轮重置的两份身份相符的现场。枚举本身不恢复目录。
pub(super) fn interrupted_resets(
    root: &Path,
    include: impl Fn(&str) -> bool,
) -> io::Result<Vec<(Entry, PathBuf)>> {
    let backups = root.join("backups/profile-reset");
    let mut pending = Vec::new();
    if !directory_present(&root.join("backups"))? || !directory_present(&backups)? {
        return Ok(pending);
    }
    if !is_plain_directory(&backups) {
        return Err(io::Error::other("重置备份不是普通本机目录"));
    }
    for item in fs::read_dir(&backups)? {
        let item = item?;
        let name = item.file_name();
        let Some((id, tag)) = name.to_str().and_then(reset_tag) else {
            continue;
        };
        if !include(id) {
            continue;
        }
        let original = backups.join(tag);
        if directory_present(&super::profile_dir(root, id))? || !directory_present(&original)? {
            continue;
        }
        let entry = identity(&original)?;
        if entry.kind != "test" || entry.id != id || identity(&item.path())? != entry {
            return Err(io::Error::other(format!(
                "备份与暂存目录的身份不符：{}",
                original.display()
            )));
        }
        pending.push((entry, original));
    }
    Ok(pending)
}

/// 调用者持有目录登记锁及本档独占锁。只发布相符的新副本，或从成对的重置现场恢复。
pub(super) fn restore_missing(directory: &Path, entry: &Entry) -> Result<bool, String> {
    let root = directory
        .parent()
        .and_then(Path::parent)
        .ok_or("找不到测试配置档的数据根。")?;
    let backups = root.join("backups/profile-reset");
    let restore = || -> io::Result<bool> {
        if live_profile_present(directory)? {
            return Ok(true);
        }
        if !entry.is_test_at(directory) || !is_plain_directory(directory.parent().unwrap()) {
            return Err(io::Error::other("测试档身份或目录无效"));
        }
        let pending = interrupted_resets(root, |id| id == entry.id)?;
        let creating = directory.with_extension("creating");
        if directory_present(&creating)? {
            if !pending.is_empty() || identity(&creating)? != *entry {
                return Err(io::Error::other(format!(
                    "创建现场与登记或重置备份不符：{}",
                    creating.display()
                )));
            }
            super::move_profile_directory(&creating, directory)?;
            return Ok(true);
        }
        let original = match pending.as_slice() {
            [] => return Ok(false),
            [(found, path)] if found == entry => path,
            [(_, path)] => {
                return Err(io::Error::other(format!(
                    "重置备份的身份与总登记不符：{}",
                    path.display()
                )))
            }
            _ => return Err(io::Error::other("存在多组未完成的重置备份")),
        };
        super::move_profile_directory(original, directory)?;
        Ok(true)
    };
    restore().map_err(|e| {
        format!(
            "无法恢复测试档 {}。请检查 {}。{e}",
            entry.alias,
            backups.display()
        )
    })
}

pub(super) fn hold_for_use(directory: &Path) -> Result<fs::File, String> {
    let file = open_profile_use_file(directory)?;
    loop {
        file.lock_shared()
            .map_err(|e| format!("无法打开测试配置档：{e}"))?;
        if live_profile_present(directory).map_err(|e| e.to_string())? {
            return Ok(file);
        }
        file.unlock().map_err(|e| e.to_string())?;
        let root = directory
            .parent()
            .and_then(Path::parent)
            .ok_or("找不到测试配置档的数据根。")?;
        let _catalog = lock_catalog(root)?;
        // 其他打开者可能已经完成恢复并持有共享锁，此时无需再抢占独占锁。
        if live_profile_present(directory).map_err(|e| e.to_string())? {
            continue;
        }
        file.lock()
            .map_err(|e| format!("无法锁定待恢复的测试档：{e}"))?;
        let catalog = read_catalog(root)?;
        let entry = catalog
            .profiles
            .iter()
            .find(|entry| entry.is_test_at(directory))
            .ok_or("找不到待恢复测试档的登记。")?;
        if !restore_missing(directory, entry)? {
            return Err(format!("测试档 {} 的目录缺失，找不到成对的重置备份或已登记的创建副本。请检查 {}，或用 --profile-reset {} 从种子重建。", entry.alias, root.join("backups/profile-reset").display(), entry.alias));
        }
        file.unlock().map_err(|e| e.to_string())?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::{
        catalog_path, create_profile, new_id, prepare_root_access, profile_dir, reset_profile,
        testing_workspace, tests::temp_root, write_json, Catalog, ProfileCommandError, RootAccess,
        Seed,
    };
    use std::path::PathBuf;

    struct Fixture {
        root: PathBuf,
        dir: PathBuf,
        backup: PathBuf,
        stage: PathBuf,
        entry: Entry,
    }

    impl Fixture {
        fn interrupt() -> Self {
            let root = temp_root("reset-recovery");
            let entry = create_profile(&root, "testing", &Seed::Empty, None).unwrap();
            let dir = profile_dir(&root, &entry.id);
            let backups = root.join("backups/profile-reset");
            let tag = format!("{}-12-{}", entry.id, new_id());
            let stage = backups.join(format!(".{tag}.tmp"));
            let backup = backups.join(tag);
            fs::create_dir_all(&stage).unwrap();
            fs::copy(dir.join("profile.json"), stage.join("profile.json")).unwrap();
            fs::write(stage.join("bindings.json"), "prepared").unwrap();
            fs::create_dir_all(dir.join("actions/one")).unwrap();
            fs::write(dir.join("bindings.json"), "my hotkeys").unwrap();
            fs::write(dir.join("wheels.json"), "my wheels").unwrap();
            fs::write(dir.join("actions/one/main.mjs"), "my action").unwrap();
            fs::rename(&dir, &backup).unwrap();
            Self {
                root,
                dir,
                backup,
                stage,
                entry,
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn restored_directory_of_another_profile_is_not_opened_copied_or_reset() {
        let root = temp_root("restored-wrong-id");
        let original = create_profile(&root, "first", &Seed::Empty, None).unwrap();
        let other = create_profile(&root, "second", &Seed::Empty, None).unwrap();
        let directory = profile_dir(&root, &original.id);
        let identity_path = directory.join("profile.json");
        let config = directory.join("bindings.json");
        let action = directory.join("actions/demo/settings.json");
        fs::create_dir_all(action.parent().unwrap()).unwrap();
        fs::write(&action, b"restored settings").unwrap();
        write_json(&identity_path, &other).unwrap();
        for access in [RootAccess::Inspect, RootAccess::Open] {
            assert!(prepare_root_access(&config, access).is_err());
        }
        let seed = Seed::Copy(original.id.clone());
        let error = create_profile(&root, "fork", &seed, None).unwrap_err();
        assert!(error.contains("profile.json") && error.contains(&other.id));
        assert!(reset_profile(&root, "first").is_err());
        assert!(read_catalog(&root).unwrap().find("fork").is_none());
        assert_eq!(fs::read(&action).unwrap(), b"restored settings");
        assert_eq!(identity(&directory).unwrap(), other);
        assert!(!root.join("backups/profile-reset").exists());

        // 显式数据根入口也不能把身份冲突的目录当作普通日常档。
        let catalog = catalog_path(&root);
        let saved = fs::read(&catalog).unwrap();
        fs::write(&catalog, "{").unwrap();
        assert!(prepare_root_access(&config, RootAccess::Open).is_err());
        fs::remove_file(&catalog).unwrap();
        assert!(prepare_root_access(&config, RootAccess::Inspect).is_err());
        fs::write(&catalog, saved).unwrap();

        // 总登记中的显示名可以比备份新，稳定 ID 相同仍可使用。
        let mut restored = original;
        restored.name = "备份里的旧名称".into();
        write_json(&identity_path, &restored).unwrap();
        let copy = create_profile(&root, "fork", &seed, None).unwrap();
        fs::write(
            profile_dir(&root, &copy.id).join("actions/demo/settings.json"),
            b"edited",
        )
        .unwrap();
        assert_eq!(fs::read(&action).unwrap(), b"restored settings");
        fs::remove_file(identity_path).unwrap();
        assert!(prepare_root_access(&config, RootAccess::Inspect).is_ok());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_catalog_lists_an_interrupted_reset_and_the_shortcut_reopens_its_original_id() {
        let f = Fixture::interrupt();
        let catalog = catalog_path(&f.root);
        fs::remove_file(&catalog).unwrap();

        let listed = read_catalog(&f.root).unwrap();
        assert_eq!(listed.find("testing"), Some(&f.entry));
        let check = prepare_root_access(&f.dir.join("bindings.json"), RootAccess::Inspect)
            .unwrap_err()
            .response();
        assert!(check.contains("测试档目录缺失"), "{check}");
        assert!(!catalog.exists() && !f.dir.exists());
        assert_eq!(
            fs::read(f.backup.join("bindings.json")).unwrap(),
            b"my hotkeys"
        );
        let opened = testing_workspace(&f.root, false).unwrap();
        assert_eq!(opened, f.entry);
        let _guard = hold_for_use(&f.dir).unwrap();
        assert_eq!(fs::read(f.dir.join("wheels.json")).unwrap(), b"my wheels");
        assert_eq!(
            fs::read(f.dir.join("actions/one/main.mjs")).unwrap(),
            b"my action"
        );
        assert_eq!(
            read_catalog(&f.root).unwrap().profiles,
            vec![f.entry.clone()]
        );
    }

    #[test]
    fn missing_catalog_preserves_conflicting_pairs_and_ignores_an_ordinary_backup() {
        let f = Fixture::interrupt();
        let catalog = catalog_path(&f.root);
        fs::remove_file(&catalog).unwrap();
        let mut wrong = f.entry.clone();
        wrong.alias = "other".into();
        write_json(&f.stage.join("profile.json"), &wrong).unwrap();
        for error in [
            testing_workspace(&f.root, false).unwrap_err(),
            prepare_root_access(&f.dir.join("bindings.json"), RootAccess::Open)
                .unwrap_err()
                .response(),
        ] {
            assert!(error.contains("身份"), "{error}");
        }
        assert!(!catalog.exists() && !f.dir.exists());
        assert_eq!(
            fs::read(f.backup.join("wheels.json")).unwrap(),
            b"my wheels"
        );
        fs::remove_dir_all(&f.stage).unwrap();
        assert!(read_catalog(&f.root).unwrap().profiles.is_empty());
        assert!(f.backup.join("bindings.json").is_file());
        assert!(!catalog.exists());
    }

    #[test]
    fn reset_persists_a_recovered_catalog_before_replacing_the_live_directory() {
        let f = Fixture::interrupt();
        fs::rename(&f.backup, &f.dir).unwrap();
        let catalog = catalog_path(&f.root);
        fs::remove_file(&catalog).unwrap();
        let blocked = catalog.with_extension("json.tmp");
        fs::create_dir(&blocked).unwrap();
        assert!(reset_profile(&f.root, "testing").is_err());
        assert_eq!(fs::read(f.dir.join("wheels.json")).unwrap(), b"my wheels");
        assert!(!catalog.exists());
        fs::remove_dir(blocked).unwrap();
        let reset = reset_profile(&f.root, "testing").unwrap();
        let entry = reset.entry;
        let backup = reset.backup;
        let saved: Catalog = serde_json::from_slice(&fs::read(catalog).unwrap()).unwrap();
        assert_eq!(entry, f.entry);
        assert_eq!(saved.profiles, vec![entry]);
        assert_eq!(
            fs::read(backup.unwrap().join("wheels.json")).unwrap(),
            b"my wheels"
        );
    }

    #[test]
    fn utf8_bom_preserves_registration_across_reset_and_catalog_recovery() {
        let f = Fixture::interrupt();
        let catalog = catalog_path(&f.root);
        for path in [
            &catalog,
            &f.backup.join("profile.json"),
            &f.stage.join("profile.json"),
        ] {
            let original = fs::read_to_string(path).unwrap();
            fs::write(path, format!("\u{feff}{original}")).unwrap();
        }
        let original_catalog = fs::read(&catalog).unwrap();
        let original_identity = fs::read(f.backup.join("profile.json")).unwrap();
        let guard = hold_for_use(&f.dir).unwrap();
        assert_eq!(fs::read(&catalog).unwrap(), original_catalog);
        assert_eq!(
            fs::read(f.dir.join("profile.json")).unwrap(),
            original_identity
        );
        drop(guard);
        fs::remove_file(&catalog).unwrap();
        assert_eq!(
            read_catalog(&f.root).unwrap().find("testing"),
            Some(&f.entry)
        );
        assert!(!catalog.exists());
    }

    #[test]
    fn simultaneous_opens_restore_the_whole_original_and_hold_it_until_release() {
        let f = Fixture::interrupt();
        let guards = std::thread::scope(|s| {
            let threads: Vec<_> = (0..4)
                .map(|_| {
                    s.spawn(|| {
                        let guard = hold_for_use(&f.dir).unwrap();
                        assert_eq!(
                            fs::read(f.dir.join("bindings.json")).unwrap(),
                            b"my hotkeys"
                        );
                        guard
                    })
                })
                .collect();
            threads
                .into_iter()
                .map(|t| t.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(fs::read(f.dir.join("wheels.json")).unwrap(), b"my wheels");
        assert_eq!(
            fs::read(f.dir.join("actions/one/main.mjs")).unwrap(),
            b"my action"
        );
        assert!(matches!(
            reset_profile(&f.root, "testing"),
            Err(ProfileCommandError::InUse(_))
        ));
        drop(guards);
        assert!(!f.backup.exists());
        assert_eq!(
            fs::read(f.stage.join("bindings.json")).unwrap(),
            b"prepared"
        );
        let backup = reset_profile(&f.root, "testing").unwrap().backup;
        assert_eq!(
            fs::read(backup.unwrap().join("bindings.json")).unwrap(),
            b"my hotkeys"
        );
    }

    #[test]
    fn missing_directories_without_a_unique_matching_pair_are_not_replaced() {
        let f = Fixture::interrupt();
        let backup_root = f.backup.parent().unwrap();
        let other_stage = backup_root.join(format!(".{}-13-t123456789abc.tmp", f.entry.id));
        let other_backup = backup_root.join(format!("{}-13-t123456789abc", f.entry.id));
        for path in [&other_stage, &other_backup] {
            fs::create_dir(path).unwrap();
            fs::copy(f.stage.join("profile.json"), path.join("profile.json")).unwrap();
        }
        assert!(hold_for_use(&f.dir).unwrap_err().contains("多组"));
        let registry = catalog_path(&f.root);
        let registered = fs::read(&registry).unwrap();
        fs::remove_file(&registry).unwrap();
        assert!(testing_workspace(&f.root, false)
            .unwrap_err()
            .contains("重复"));
        assert!(!registry.exists() && !f.dir.exists());
        fs::write(registry, registered).unwrap();
        fs::remove_dir_all(other_backup).unwrap();
        let mut wrong = f.entry.clone();
        wrong.alias = "another".into();
        fs::write(
            f.stage.join("profile.json"),
            serde_json::to_vec(&wrong).unwrap(),
        )
        .unwrap();
        assert!(hold_for_use(&f.dir).unwrap_err().contains("身份"));
        fs::remove_dir_all(&f.stage).unwrap();
        assert!(hold_for_use(&f.dir).unwrap_err().contains("找不到成对"));
        assert!(!f.dir.exists());
        assert_eq!(
            fs::read(f.backup.join("bindings.json")).unwrap(),
            b"my hotkeys"
        );
    }

    #[test]
    fn file_at_live_path_leaves_the_reset_pair_available_for_a_later_open() {
        let f = Fixture::interrupt();
        let catalog = catalog_path(&f.root);
        let registration = fs::read(&catalog).unwrap();
        fs::write(&f.dir, b"unrelated file").unwrap();

        let reset = reset_profile(&f.root, "testing").unwrap_err();
        let ProfileCommandError::Other(reset_error) = reset else {
            panic!("目录被文件占用时应报告路径错误")
        };
        let copy_error =
            create_profile(&f.root, "child", &Seed::Copy("testing".into()), None).unwrap_err();
        for error in [reset_error, copy_error, hold_for_use(&f.dir).unwrap_err()] {
            assert!(error.contains(f.dir.to_str().unwrap()), "{error}");
        }
        assert_eq!(fs::read(&f.dir).unwrap(), b"unrelated file");
        assert_eq!(fs::read(&catalog).unwrap(), registration);
        assert_eq!(
            fs::read(f.backup.join("wheels.json")).unwrap(),
            b"my wheels"
        );
        assert_eq!(
            fs::read(f.stage.join("bindings.json")).unwrap(),
            b"prepared"
        );

        fs::rename(&f.dir, f.root.join("saved-file")).unwrap();
        let _guard = hold_for_use(&f.dir).unwrap();
        assert_eq!(fs::read(f.dir.join("wheels.json")).unwrap(), b"my wheels");
        assert_eq!(
            fs::read(f.dir.join("actions/one/main.mjs")).unwrap(),
            b"my action"
        );
        assert!(!f.backup.exists());
    }

    #[test]
    fn copies_and_explicit_resets_recover_the_missing_source_first() {
        let f = Fixture::interrupt();
        let copy = create_profile(&f.root, "child", &Seed::Copy("testing".into()), None).unwrap();
        assert_eq!(
            fs::read(profile_dir(&f.root, &copy.id).join("bindings.json")).unwrap(),
            b"my hotkeys"
        );
        fs::rename(&f.dir, &f.backup).unwrap();
        let original = reset_profile(&f.root, "testing").unwrap().backup;
        assert_eq!(
            fs::read(original.unwrap().join("wheels.json")).unwrap(),
            b"my wheels"
        );
        assert!(!f.dir.join("bindings.json").exists());
    }
}
