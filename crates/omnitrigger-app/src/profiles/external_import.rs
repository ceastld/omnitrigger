//! 日常配置导入。先读取整份副本，逐个替换文件；中断后用原文件收尾。
use super::{copy_data, is_data_file, lock_catalog, new_id, now_ms, RAISHOT_MANIFEST_PATH};
use crate::local_files::{is_cloud_path, is_plain, is_plain_directory};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, ErrorKind},
    os::windows::ffi::OsStrExt,
    path::{Component, Path, PathBuf},
};
use windows::{core::PCWSTR, Win32::Storage::FileSystem::MoveFileW};

const PENDING: &str = "external-import.pending";
const PLAN: &str = "plan.json";

#[derive(Serialize, Deserialize)]
struct Plan {
    backup: String,
    files: Vec<IncomingFile>,
    directories: Vec<PathBuf>,
}

#[derive(Serialize, Deserialize)]
struct IncomingFile {
    path: PathBuf,
    sha256: [u8; 32],
}

fn digest(path: &Path) -> io::Result<[u8; 32]> {
    let mut hash = Sha256::new();
    io::copy(&mut fs::File::open(path)?, &mut hash)?;
    Ok(hash.finalize().into())
}

fn invalid(path: &Path) -> io::Error {
    io::Error::other(format!("导入路径或文件类型异常：{}", path.display()))
}

/// None 表示不存在，链接和云端占位不能当作普通文件或目录访问。
fn metadata(path: &Path) -> io::Result<Option<fs::Metadata>> {
    match path.symlink_metadata() {
        Ok(meta) if is_plain(&meta) && !is_cloud_path(path) => Ok(Some(meta)),
        Ok(_) => Err(invalid(path)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn ensure_directory(path: &Path) -> io::Result<()> {
    match metadata(path)? {
        Some(meta) if meta.is_dir() => Ok(()),
        None => fs::create_dir(path),
        _ => Err(invalid(path)),
    }
}

fn check_parents(base: &Path, path: &Path) -> io::Result<()> {
    let mut directory = base.to_owned();
    for part in path.parent().unwrap_or(Path::new("")).components() {
        directory.push(part);
        if metadata(&directory)?.is_some_and(|m| !m.is_dir()) {
            return Err(invalid(&directory));
        }
    }
    Ok(())
}

/// 原路径若在导入期间被其他程序占用或新建，不能用 rename 的覆盖语义。
fn move_new(from: &Path, to: &Path) -> io::Result<()> {
    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain([0])
            .collect::<Vec<_>>()
    };
    let (from, to) = (wide(from), wide(to));
    unsafe { MoveFileW(PCWSTR(from.as_ptr()), PCWSTR(to.as_ptr())) }
        .map_err(|error| io::Error::from_raw_os_error(error.code().0 & 0xffff))
}

fn collect(staged: &Path, root: &Path, relative: &Path, plan: &mut Plan) -> io::Result<()> {
    let mut entries = fs::read_dir(staged.join(relative))?.collect::<io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| {
        (
            entry.file_type().is_ok_and(|kind| kind.is_dir()),
            entry.file_name(),
        )
    });
    for entry in entries {
        let path = relative.join(entry.file_name());
        let target = root.join(&path);
        let existing = metadata(&target)?;
        if entry.file_type()?.is_dir() {
            if let Some(meta) = existing {
                if !meta.is_dir() {
                    return Err(invalid(&target));
                }
            } else {
                plan.directories.push(path.clone());
            }
            collect(staged, root, &path, plan)?;
        } else {
            if existing.is_some_and(|meta| !meta.is_file()) {
                return Err(invalid(&target));
            }
            plan.files.push(IncomingFile {
                path,
                sha256: digest(&entry.path())?,
            });
        }
    }
    Ok(())
}

fn validate(plan: &Plan) -> io::Result<()> {
    let backup = Path::new(&plan.backup);
    if !plan.backup.starts_with("before-import-") || backup.file_name() != Some(backup.as_os_str())
    {
        return Err(invalid(backup));
    }
    for (path, directories) in plan
        .directories
        .iter()
        .map(|p| (p, true))
        .chain(plan.files.iter().map(|f| (&f.path, false)))
    {
        let actions = Path::new("actions");
        let manifest = Path::new(RAISHOT_MANIFEST_PATH);
        let allowed = if directories {
            path.starts_with(actions) || (manifest.starts_with(path) && path != manifest)
        } else {
            (path.starts_with(actions) && path != actions)
                || path == manifest
                || (path.file_name() == Some(path.as_os_str())
                    && is_data_file(&path.to_string_lossy())
                    && !matches!(
                        path.to_string_lossy().to_ascii_lowercase().as_str(),
                        "actions" | "plugins" | "profiles" | "backups" | "device"
                    ))
        };
        if !allowed
            || path.as_os_str().is_empty()
            || is_cloud_path(path)
            || !path.components().all(|c| {
                matches!(c, Component::Normal(name) if {
                    let name = name.to_string_lossy();
                    !name.contains(':') && !name.ends_with(['.', ' '])
                })
            })
        {
            return Err(invalid(path));
        }
    }
    Ok(())
}

fn apply(root: &Path, pending: &Path, plan: &Plan) -> io::Result<()> {
    let staged = pending.join("incoming");
    let originals = pending.join("originals");
    for (i, directory) in plan.directories.iter().enumerate() {
        check_parents(root, directory)?;
        move_new(
            &pending.join("directories").join(i.to_string()),
            &root.join(directory),
        )?;
    }
    for file in &plan.files {
        let path = &file.path;
        check_parents(root, path)?;
        let target = root.join(path);
        if metadata(&target)?.is_some() {
            let original = originals.join(path);
            fs::create_dir_all(original.parent().unwrap())?;
            move_new(&target, &original).map_err(|error| {
                io::Error::new(error.kind(), format!("{}：{error}", target.display()))
            })?;
        }
        move_new(&staged.join(path), &target).map_err(|error| {
            io::Error::new(error.kind(), format!("{}：{error}", target.display()))
        })?;
    }
    Ok(())
}

fn rollback(root: &Path, pending: &Path, plan: &Plan) -> io::Result<()> {
    for file in plan.files.iter().rev() {
        let path = &file.path;
        let target = root.join(path);
        let staged = pending.join("incoming").join(path);
        let original = pending.join("originals").join(path);
        // 目录在中断期间可能被替换；此时保留记录供重试，不沿新链接恢复文件。
        check_parents(root, path)?;
        check_parents(&pending.join("incoming"), path)?;
        check_parents(&pending.join("originals"), path)?;
        // 已移出的暂存文件代表该项已安装。移回暂存也让第二次恢复能从中断处继续。
        if metadata(&staged)?.is_none() {
            if !metadata(&target)?.is_some_and(|m| m.is_file()) || digest(&target)? != file.sha256 {
                return Err(io::Error::other(format!(
                    "{} 在导入后有其他修改，已保留现场。",
                    target.display()
                )));
            }
            move_new(&target, &staged)?;
        }
        if metadata(&original)?.is_some() {
            move_new(&original, &target)?;
        }
    }
    for (i, dir) in plan.directories.iter().enumerate().rev() {
        // 原先为空的暂存目录只有移动成功后才消失，避免误删同一时刻由其他程序创建的目录。
        if metadata(&pending.join("directories").join(i.to_string()))?.is_some() {
            continue;
        }
        check_parents(root, dir)?;
        if metadata(&root.join(dir))?.is_some_and(|m| !m.is_dir()) {
            return Err(invalid(&root.join(dir)));
        }
        match fs::remove_dir(root.join(dir)) {
            Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::DirectoryNotEmpty) => {}
            result => result?,
        }
    }
    Ok(())
}

fn finish(pending: &Path) -> io::Result<()> {
    // 单次重命名确认完成，清理临时文件时被中断也不会把成功的导入误判为待回滚。
    let retired = pending.with_file_name(format!("external-import-finished-{}", new_id()));
    fs::rename(pending, &retired)?;
    let _ = fs::remove_dir_all(retired);
    Ok(())
}

fn recover_locked(root: &Path, pending: &Path) -> io::Result<()> {
    if metadata(pending)?.is_none() {
        return Ok(());
    }
    let file = pending.join(PLAN);
    if metadata(&file)?.is_some() {
        if !is_plain_directory(&pending.join("incoming"))
            || metadata(&pending.join("originals"))?.is_some_and(|m| !m.is_dir())
        {
            return Err(invalid(pending));
        }
        let plan: Plan = serde_json::from_slice(&fs::read(&file)?)?;
        validate(&plan)?;
        if !plan.directories.is_empty() && !is_plain_directory(&pending.join("directories")) {
            return Err(invalid(&pending.join("directories")));
        }
        rollback(root, pending, &plan).map_err(|e| {
            io::Error::other(format!(
                "恢复导入前的数据失败：{e}。备份位于 {}",
                root.join("backups").join(&plan.backup).display()
            ))
        })?;
    }
    finish(pending)
}

pub(crate) fn recover(root: &Path) -> Result<(), String> {
    let backups = root.join("backups");
    if !metadata(&backups)
        .map_err(|e| e.to_string())?
        .is_some_and(|m| m.is_dir())
    {
        return Ok(());
    }
    let pending = backups.join(PENDING);
    if metadata(&pending).map_err(|e| e.to_string())?.is_some() {
        let _lock = lock_catalog(root)?;
        recover_locked(root, &pending).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub(super) fn import(root: &Path, source: &Path) -> Result<(), String> {
    let _lock = lock_catalog(root)?;
    let backups = root.join("backups");
    ensure_directory(&backups).map_err(|e| e.to_string())?;
    let pending = backups.join(PENDING);
    recover_locked(root, &pending).map_err(|e| e.to_string())?;
    let backup = format!("before-import-{}-{}", now_ms(), new_id());
    let backup_dir = backups.join(&backup);
    let mut plan = Plan {
        backup,
        files: Vec::new(),
        directories: Vec::new(),
    };
    if !is_plain_directory(source) || !is_plain_directory(root) {
        return Err("导入需要普通本机配置目录。".into());
    }
    fs::create_dir(&pending).map_err(|e| e.to_string())?;
    let staged = pending.join("incoming");
    let result = (|| -> io::Result<()> {
        let snapshot = pending.join("backup");
        copy_data(root, &snapshot, is_data_file).map_err(io::Error::other)?;
        fs::rename(&snapshot, &backup_dir)?;
        copy_data(source, &staged, is_data_file).map_err(io::Error::other)?;
        collect(&staged, root, Path::new(""), &mut plan)?;
        validate(&plan)?;
        let directories = pending.join("directories");
        fs::create_dir(&directories)?;
        for i in 0..plan.directories.len() {
            fs::create_dir(directories.join(i.to_string()))?;
        }
        let bytes = serde_json::to_vec(&plan)?;
        crate::settings::atomic_write(&pending.join(PLAN), &bytes).map_err(io::Error::other)?;
        apply(root, &pending, &plan)?;
        finish(&pending)
    })();
    if let Err(error) = result {
        recover_locked(root, &pending).map_err(|recovery| format!("{error}\n{recovery}"))?;
        return if backup_dir.exists() {
            Err(format!(
                "{error}\n\n导入前的备份位于：\n{}",
                backup_dir.display()
            ))
        } else {
            Err(format!("准备导入备份失败：{error}"))
        };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::fs::OpenOptionsExt;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("omnitrigger-import-{}", new_id()));
            fs::create_dir(&dir).unwrap();
            Self(dir)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn write(root: &Path, path: &str, value: &str) {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, value).unwrap();
    }

    #[test]
    fn locked_source_or_target_preserves_data_and_retry_imports_the_overlay() {
        let action = "actions/example/entry.mjs";
        for lock_source in [true, false] {
            let fixture = Fixture::new();
            let root = fixture.0.join("daily");
            let source = fixture.0.join("other");
            for path in [
                "bindings.json",
                "wheels.json",
                action,
                RAISHOT_MANIFEST_PATH,
            ] {
                write(&root, path, "old");
                write(&source, path, "new");
            }
            write(&root, "actions/local-only/entry.mjs", "local");
            write(&root, "profiles.json", "local catalog");
            write(&source, "profiles.json", "other catalog");
            write(&source, "added.json", "new");
            write(&source, "actions/added/file.mjs", "new");
            let locked = fs::File::options()
                .read(true)
                .share_mode(if lock_source { 0 } else { 1 })
                .open(if lock_source { &source } else { &root }.join(action))
                .unwrap();
            assert!(import(&root, &source).is_err());
            for path in [
                "bindings.json",
                "wheels.json",
                action,
                RAISHOT_MANIFEST_PATH,
            ] {
                assert_eq!(fs::read(root.join(path)).unwrap(), b"old");
            }
            assert!(!root.join("added.json").exists() && !root.join("actions/added").exists());
            let backups = fs::read_dir(root.join("backups"))
                .unwrap()
                .map(|e| e.unwrap().path())
                .collect::<Vec<_>>();
            assert_eq!(backups.len(), 1);
            assert_eq!(fs::read(backups[0].join(action)).unwrap(), b"old");
            drop(locked);
            import(&root, &source).unwrap();
            for path in [
                "bindings.json",
                "wheels.json",
                action,
                RAISHOT_MANIFEST_PATH,
                "added.json",
                "actions/added/file.mjs",
            ] {
                assert_eq!(fs::read(root.join(path)).unwrap(), b"new");
                assert_eq!(fs::read(source.join(path)).unwrap(), b"new");
            }
            assert_eq!(
                fs::read(root.join("actions/local-only/entry.mjs")).unwrap(),
                b"local"
            );
            assert_eq!(
                fs::read(root.join("profiles.json")).unwrap(),
                b"local catalog"
            );
            assert!(!root.join("backups").join(PENDING).exists());
        }
    }

    #[test]
    fn incomplete_commit_recovers_on_start_and_preserves_intervening_edits() {
        let fixture = Fixture::new();
        let root = &fixture.0;
        let pending = root.join("backups").join(PENDING);
        let staged = pending.join("incoming");
        for name in ["bindings.json", "new.json", "wheels.json"] {
            write(&staged, name, "import");
        }
        for name in ["bindings.json", "wheels.json"] {
            write(root, name, "old");
        }
        let mut plan = Plan {
            backup: "before-import-fixture".into(),
            files: Vec::new(),
            directories: Vec::new(),
        };
        collect(&staged, root, Path::new(""), &mut plan).unwrap();
        fs::write(pending.join(PLAN), serde_json::to_vec(&plan).unwrap()).unwrap();
        let originals = pending.join("originals");
        fs::create_dir(&originals).unwrap();
        for name in ["bindings.json", "wheels.json"] {
            move_new(&root.join(name), &originals.join(name)).unwrap();
        }
        for name in ["bindings.json", "new.json"] {
            move_new(&staged.join(name), &root.join(name)).unwrap();
        }
        write(root, "bindings.json", "later edit");
        assert!(recover(root).is_err());
        assert_eq!(fs::read(root.join("bindings.json")).unwrap(), b"later edit");
        assert_eq!(fs::read(originals.join("bindings.json")).unwrap(), b"old");
        write(root, "bindings.json", "import");
        recover(root).unwrap();
        recover(root).unwrap();
        for name in ["bindings.json", "wheels.json"] {
            assert_eq!(fs::read(root.join(name)).unwrap(), b"old");
        }
        assert!(!root.join("new.json").exists() && !pending.exists());
    }
}
