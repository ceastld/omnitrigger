//! 重置的两次目录替换之间退出时，匹配同一事务的备份与暂存档，恢复用户原来的目录。
use super::{lock_catalog, open_profile_use_file, read_catalog, Entry};
use crate::local_files::{is_plain, is_plain_directory};
use std::{fs, io, path::Path};

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
    serde_json::from_slice(&fs::read(path)?).map_err(io::Error::other)
}

/// 调用者持有目录登记锁及本档独占锁。无成对现场时不从普通历史备份猜测。
pub(super) fn restore_missing(directory: &Path, entry: &Entry) -> Result<bool, String> {
    let root = directory
        .parent()
        .and_then(Path::parent)
        .ok_or("找不到测试配置档的数据根。")?;
    let backups = root.join("backups/profile-reset");
    let restore = || -> io::Result<bool> {
        if directory_present(directory)? {
            return Ok(true);
        }
        if !entry.is_test_at(directory) || !is_plain_directory(directory.parent().unwrap()) {
            return Err(io::Error::other("测试档身份或目录无效"));
        }
        if !directory_present(&root.join("backups"))? {
            return Ok(false);
        }
        if !directory_present(&backups)? {
            return Ok(false);
        }
        if !is_plain_directory(&backups) {
            return Err(io::Error::other("重置备份不是普通本机目录"));
        }
        let mut backup = None;
        let prefix = format!("{}-", entry.id);
        for item in fs::read_dir(&backups)? {
            let item = item?;
            let name = item.file_name();
            let Some(tag) = name
                .to_str()
                .and_then(|s| s.strip_prefix('.'))
                .and_then(|s| s.strip_suffix(".tmp"))
            else {
                continue;
            };
            let Some((time, nonce)) = tag.strip_prefix(&prefix).and_then(|s| s.split_once("-t"))
            else {
                continue;
            };
            if time.is_empty()
                || !time.bytes().all(|c| c.is_ascii_digit())
                || nonce.len() != 12
                || !nonce.bytes().all(|c| c.is_ascii_hexdigit())
            {
                continue;
            }
            let original = backups.join(tag);
            if !directory_present(&original)? {
                continue;
            }
            if identity(&item.path())? != *entry || identity(&original)? != *entry {
                return Err(io::Error::other(format!(
                    "备份或暂存目录的身份与总登记不符：{}",
                    original.display()
                )));
            }
            if backup.replace(original).is_some() {
                return Err(io::Error::other("存在多组未完成的重置备份"));
            }
        }
        let Some(original) = backup else {
            return Ok(false);
        };
        super::move_new(&original, directory)?;
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
        if directory_present(directory).map_err(|e| e.to_string())? {
            return Ok(file);
        }
        file.unlock().map_err(|e| e.to_string())?;
        let root = directory
            .parent()
            .and_then(Path::parent)
            .ok_or("找不到测试配置档的数据根。")?;
        let _catalog = lock_catalog(root)?;
        // 其他打开者可能已经完成恢复并持有共享锁，此时无需再抢占独占锁。
        if directory_present(directory).map_err(|e| e.to_string())? {
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
            return Err(format!("测试档 {} 的目录缺失，找不到成对的重置备份。请检查 {}，或用 --profile-reset {} 从种子重建。", entry.alias, root.join("backups/profile-reset").display(), entry.alias));
        }
        file.unlock().map_err(|e| e.to_string())?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::{
        create_profile, new_id, profile_dir, reset_profile, tests::temp_root, ProfileCommandError,
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
        let (_, backup) = reset_profile(&f.root, "testing").unwrap();
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
        let catalog = super::super::catalog_path(&f.root);
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
        let (_, original) = reset_profile(&f.root, "testing").unwrap();
        assert_eq!(
            fs::read(original.unwrap().join("wheels.json")).unwrap(),
            b"my wheels"
        );
        assert!(!f.dir.join("bindings.json").exists());
    }
}
