//! 日常设置中的测试档选择器。列表只读，用户选定后才恢复该档并启动窗口。
use super::{
    catalog_path, generated_profile_id, lock_catalog, profile_dir, read_catalog, source_dir,
    spawn_test_profile, Catalog, Entry,
};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub(crate) struct SavedTestProfile {
    root: PathBuf,
    entry: Entry,
}

pub(crate) fn saved_test_profiles(root: &Path) -> Result<Vec<SavedTestProfile>, String> {
    let root = std::path::absolute(root).map_err(|e| e.to_string())?;
    crate::script_runtime::check_local_path(&catalog_path(&root))?;
    let mut entries = read_catalog(&root)?.profiles;
    entries.retain(|entry| entry.kind == "test" && generated_profile_id(&entry.id));
    entries.sort_by(|a, b| (&a.name, &a.alias, &a.id).cmp(&(&b.name, &b.alias, &b.id)));
    Ok(entries
        .into_iter()
        .map(|entry| SavedTestProfile {
            root: root.clone(),
            entry,
        })
        .collect())
}

/// 点击前重新读取登记，避免用旧列表打开同名新档。目录恢复与其他启动入口共用。
pub(super) fn prepare_test_open(
    root: &Path,
    id: &str,
    matches: impl FnOnce(&Catalog, &Entry) -> Result<bool, String>,
) -> Result<(), String> {
    if !crate::local_files::is_plain_directory(root) {
        return Err("测试档的数据根不可用。请重新查看已有配置档。".into());
    }
    crate::script_runtime::check_local_path(&catalog_path(root))?;
    let _lock = lock_catalog(root)?;
    let catalog = read_catalog(root)?;
    let changed = "这个测试档的登记已变更。请重新查看已有配置档。";
    let entry = catalog
        .source_profile(id)
        .filter(|entry| entry.kind == "test" && entry.id == id && generated_profile_id(id))
        .ok_or(changed)?;
    if !matches(&catalog, entry)? {
        return Err(changed.into());
    }
    crate::script_runtime::check_local_path(&profile_dir(root, id))?;
    source_dir(root, &catalog, id)?;
    Ok(())
}

impl SavedTestProfile {
    pub(crate) fn labels(&self) -> (&str, &str) {
        (&self.entry.name, &self.entry.alias)
    }

    fn prepare_to_open(&self) -> Result<(), String> {
        prepare_test_open(&self.root, &self.entry.id, |_, entry| {
            Ok(entry == &self.entry)
        })
    }

    pub(crate) fn open(&self) -> Result<(), String> {
        let program = std::env::current_exe().map_err(|error| error.to_string())?;
        self.spawn(&program).map(drop)
    }

    fn spawn(&self, program: &Path) -> Result<std::process::Child, String> {
        self.prepare_to_open()?;
        spawn_test_profile(program, &self.root, &self.entry.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::{create_profile, tests::temp_root, write_json, Seed, DEFAULT_ID};
    use std::fs;

    #[test]
    fn listing_keeps_existing_edits_and_scopes_names_to_the_daily_root() {
        let root = temp_root("named-test-list");
        assert!(saved_test_profiles(&root).unwrap().is_empty());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        fs::write(root.join("wheels.json"), "daily wheels").unwrap();
        let other = create_profile(&root, "b", &Seed::Empty, Some("同名")).unwrap();
        let selected =
            create_profile(&root, "a", &Seed::Copy(DEFAULT_ID.into()), Some("同名")).unwrap();
        let directory = profile_dir(&root, &selected.id);
        fs::write(directory.join("wheels.json"), "edited wheels").unwrap();
        create_profile(&directory, "nested", &Seed::Empty, None).unwrap();
        fs::remove_file(catalog_path(&root)).unwrap();

        let list = saved_test_profiles(&root).unwrap();
        assert_eq!(
            list.iter()
                .map(SavedTestProfile::labels)
                .collect::<Vec<_>>(),
            [("同名", "a"), ("同名", "b")]
        );
        assert_eq!(list[0].entry.id, selected.id);
        assert_eq!(list[1].entry.id, other.id);
        assert!(!catalog_path(&root).exists());
        list[0].prepare_to_open().unwrap();
        assert_eq!(
            fs::read(directory.join("wheels.json")).unwrap(),
            b"edited wheels"
        );
        assert_eq!(fs::read(root.join("wheels.json")).unwrap(), b"daily wheels");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn opening_checks_the_listed_identity_and_recovers_only_on_selection() {
        let root = temp_root("named-test-open");
        let entry = create_profile(&root, "wheel", &Seed::Empty, None).unwrap();
        let directory = profile_dir(&root, &entry.id);
        fs::write(directory.join("bindings.json"), "previous edits").unwrap();
        let staging = directory.with_extension("creating");
        fs::rename(&directory, &staging).unwrap();
        let item = saved_test_profiles(&root).unwrap().pop().unwrap();
        assert!(staging.is_dir() && !directory.exists());
        item.prepare_to_open().unwrap();
        assert_eq!(
            fs::read(directory.join("bindings.json")).unwrap(),
            b"previous edits"
        );
        let mut catalog = read_catalog(&root).unwrap();
        catalog.profiles[0].id = super::super::new_id();
        write_json(&catalog_path(&root), &catalog).unwrap();
        assert!(item.prepare_to_open().unwrap_err().contains("登记已变更"));
        assert!(!profile_dir(&root, &catalog.profiles[0].id).exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "在 Windows 安装包重装验收中打开已登记的测试档窗口"]
    fn reopen_installed_profile_from_settings_selection() {
        use std::time::{Duration, Instant};
        use windows::{
            core::HSTRING,
            Win32::UI::WindowsAndMessaging::{
                FindWindowW, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CLOSE,
            },
        };

        let root = PathBuf::from(std::env::var_os("OMNITRIGGER_QA_PROFILE_ROOT").unwrap());
        let exe = PathBuf::from(std::env::var_os("OMNITRIGGER_QA_INSTALLED_EXE").unwrap());
        let id = std::env::var("OMNITRIGGER_QA_NAMED_ID").unwrap();
        let list = saved_test_profiles(&root).unwrap();
        let item = list.iter().find(|p| p.entry.alias == "daily-copy").unwrap();
        assert_eq!(item.entry.id, id);
        assert!(list
            .iter()
            .any(|p| p.entry.name == item.entry.name && p.entry.id != id));
        let title = HSTRING::from(format!("OmniTrigger — 测试：{}", item.entry.name));
        let mut child = item.spawn(&exe).unwrap();
        let result = (|| -> Result<(), String> {
            let until = Instant::now() + Duration::from_secs(45);
            loop {
                if child.try_wait().map_err(|e| e.to_string())?.is_some() || Instant::now() >= until
                {
                    return Err("安装版未打开所选测试档的窗口".into());
                }
                if let Ok(hwnd) = unsafe { FindWindowW(None, &title) } {
                    let mut owner = 0;
                    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut owner)) };
                    if owner == child.id() && unsafe { IsWindowVisible(hwnd).as_bool() } {
                        let lock = fs::File::options()
                            .read(true)
                            .write(true)
                            .open(profile_dir(&root, &id).with_extension("window.lock"))
                            .map_err(|e| e.to_string())?;
                        if !matches!(lock.try_lock(), Err(std::fs::TryLockError::WouldBlock)) {
                            return Err("窗口没有持有所选测试档的使用锁".into());
                        }
                        unsafe {
                            PostMessageW(
                                Some(hwnd),
                                WM_CLOSE,
                                Default::default(),
                                Default::default(),
                            )
                        }
                        .map_err(|e| e.to_string())?;
                        break;
                    }
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            let until = Instant::now() + Duration::from_secs(20);
            while Instant::now() < until {
                if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                    return if status.success() {
                        Ok(())
                    } else {
                        Err(format!("测试窗口退出失败：{status}"))
                    };
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err("关闭所选测试窗口后进程仍在运行".into())
        })();
        if result.is_err() {
            let _ = child.kill();
            let _ = child.wait();
        }
        result.unwrap();
    }
}
