//! 导入只覆盖来源提供的数据，本机独有的文件不参与相同副本判定。
use super::{digest, invalid, metadata, packages};
use crate::local_files::{is_cloud_path, is_plain, is_plain_directory};
use crate::plugins::{local_manifest_metadata, RAISHOT_MANIFEST_PATH};
use crate::profiles::transferable_file;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, BufRead, BufReader},
    path::Path,
};

fn same_file(source: &Path, source_meta: &fs::Metadata, target: &Path) -> io::Result<bool> {
    if !metadata(target)?.is_some_and(|meta| meta.is_file() && meta.len() == source_meta.len()) {
        return Ok(false);
    }
    let mut left = BufReader::new(fs::File::open(source)?);
    let mut right = BufReader::new(fs::File::open(target)?);
    loop {
        let a = left.fill_buf()?;
        let b = right.fill_buf()?;
        let n = a.len().min(b.len());
        if n == 0 {
            return Ok(a.is_empty() && b.is_empty());
        }
        if a[..n] != b[..n] {
            return Ok(false);
        }
        left.consume(n);
        right.consume(n);
    }
}

fn sorted_entries(path: &Path) -> io::Result<Vec<fs::DirEntry>> {
    let mut entries = fs::read_dir(path)?.collect::<io::Result<Vec<_>>>()?;
    entries.sort_unstable_by_key(|entry| entry.file_name());
    Ok(entries)
}

fn visit_actions(
    root: &Path,
    relative: &Path,
    visit: &mut impl FnMut(&Path, &fs::Metadata) -> io::Result<bool>,
) -> io::Result<bool> {
    for entry in sorted_entries(&root.join(relative))? {
        let path = entry.path();
        let meta = path.symlink_metadata()?;
        if !is_plain(&meta) || is_cloud_path(&path) {
            continue;
        }
        let name = entry.file_name();
        if relative == Path::new("actions")
            && meta.is_dir()
            && omnitrigger_actions::catalog::is_legacy_stage(&name)
        {
            continue;
        }
        let child = relative.join(name);
        if (meta.is_file() || meta.is_dir()) && !visit(&child, &meta)? {
            return Ok(false);
        }
        if meta.is_dir() && !visit_actions(root, &child, visit)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// 两种比较遍历相同的导入范围，先处理顶层配置，再检查扩展。发现差异即可停止。
fn visit_copy(
    source: &Path,
    mut visit: impl FnMut(&Path, &fs::Metadata) -> io::Result<bool>,
) -> io::Result<bool> {
    if !is_plain_directory(source) {
        return Err(invalid(source));
    }
    for entry in sorted_entries(source)? {
        let name = entry.file_name();
        let path = entry.path();
        let meta = path.symlink_metadata()?;
        if meta.is_file()
            && is_plain(&meta)
            && !is_cloud_path(&path)
            && transferable_file(&name.to_string_lossy())
            && !visit(Path::new(&name), &meta)?
        {
            return Ok(false);
        }
    }
    let manifest = Path::new(RAISHOT_MANIFEST_PATH);
    if let Some(meta) = local_manifest_metadata(&source.join(manifest)).map_err(io::Error::other)? {
        if !visit(manifest, &meta)? {
            return Ok(false);
        }
    }
    let actions = Path::new("actions");
    match source.join(actions).symlink_metadata() {
        Ok(meta) if meta.is_dir() && is_plain(&meta) => {
            Ok(visit(actions, &meta)? && visit_actions(source, actions, &mut visit)?)
        }
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(e) => Err(e),
    }
}

pub(in crate::profiles) fn already_present(daily: &Path, source: &Path) -> io::Result<bool> {
    if !is_plain_directory(daily) || !is_plain_directory(source) {
        return Ok(false);
    }
    let mut folders = BTreeMap::new();
    visit_copy(source, |relative, meta| {
        let mut target = daily.join(relative);
        if let Ok(action) = relative.strip_prefix("actions") {
            let mut parts = action.iter();
            if let Some(folder) = parts.next() {
                if let Some(installed) = folders.get(folder) {
                    target = daily.join("actions").join(installed).join(parts.as_path());
                }
            } else if meta.is_dir() {
                let Some(installed) = packages::installed_folders(&source.join(relative), &target)?
                else {
                    return Ok(false);
                };
                folders = installed;
            }
        }
        if meta.is_dir() {
            return Ok(metadata(&target)?.is_some_and(|m| m.is_dir()));
        }
        if relative == Path::new(RAISHOT_MANIFEST_PATH)
            && local_manifest_metadata(&target)
                .map_err(io::Error::other)?
                .is_none()
        {
            return Ok(false);
        }
        same_file(&source.join(relative), meta, &target)
    })
}

/// 按路径、类型和文件内容计算摘要。长度前缀区分路径的边界，空目录也参与计算。
pub(in crate::profiles) fn copy_digest(source: &Path) -> io::Result<[u8; 32]> {
    use std::os::windows::ffi::OsStrExt;
    let mut hash = Sha256::new();
    visit_copy(source, |relative, meta| {
        let name: Vec<_> = relative.as_os_str().encode_wide().collect();
        hash.update((name.len() as u64).to_le_bytes());
        for unit in name {
            hash.update(unit.to_le_bytes());
        }
        hash.update([u8::from(meta.is_file())]);
        if meta.is_file() {
            hash.update(digest(&source.join(relative))?);
        }
        Ok(true)
    })?;
    Ok(hash.finalize().into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::fs::OpenOptionsExt;

    fn save(root: &Path, relative: &str, bytes: &[u8]) {
        let file = root.join(relative);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, bytes).unwrap();
    }

    #[test]
    fn digest_covers_transferable_paths_and_bytes_in_a_stable_order() {
        let root = crate::profiles::tests::temp_root("copy-digest");
        let first = root.join("first");
        let second = root.join("second");
        let files = [
            ("wheels.json", "layout"),
            ("actions/pkg/run.js", "code"),
            (RAISHOT_MANIFEST_PATH, "manifest"),
        ];
        for (name, value) in files {
            save(&first, name, value.as_bytes());
        }
        for (name, value) in files.into_iter().rev() {
            save(&second, name, value.as_bytes());
        }
        save(&first, "browser-connector.json", b"local");
        save(&first, "backups/edits/old.json", b"backup");
        save(&first, "actions/pkg/OneDrive-test/skipped.js", b"excluded");
        let original = copy_digest(&first).unwrap();
        assert_eq!(original, copy_digest(&second).unwrap());
        for (name, value) in files {
            save(&second, name, b"edited");
            assert_ne!(original, copy_digest(&second).unwrap(), "{name}");
            save(&second, name, value.as_bytes());
        }
        fs::rename(
            second.join("actions/pkg/run.js"),
            second.join("actions/pkg/new.js"),
        )
        .unwrap();
        assert_ne!(original, copy_digest(&second).unwrap());
        let lock = fs::File::options()
            .read(true)
            .share_mode(0)
            .open(first.join("wheels.json"))
            .unwrap();
        assert!(copy_digest(&first).is_err());
        drop(lock);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn renamed_packages_and_daily_only_files_do_not_trigger_an_import() {
        let root = crate::profiles::tests::temp_root("compare-contents");
        let daily = root.join("daily");
        let source = root.join("source");
        let manifest = include_bytes!("../../../../../examples/actions/n10-rust/action.json");
        let script = vec![b'x'; 22_000];
        for (folder, name) in [(&source, "old-name"), (&daily, "installed-name")] {
            save(folder, "bindings.json", br#"{"version":24,"bindings":[]}"#);
            save(folder, "wheels.json", br#"{"version":6,"wheels":[]}"#);
            save(folder, RAISHOT_MANIFEST_PATH, b"manifest");
            save(folder, &format!("actions/{name}/action.json"), manifest);
            save(folder, &format!("actions/{name}/nested/run.js"), &script);
        }
        save(&daily, "actions/installed-name/local-only.js", b"custom");
        save(&source, "backups/edits/old.json", b"old");
        save(&source, "browser-connector.json", b"source only");
        save(
            &source,
            "actions/old-name/OneDrive-test/skip.js",
            b"offline",
        );
        assert!(already_present(&daily, &source).unwrap());

        let mut changed = script.clone();
        *changed.last_mut().unwrap() = b'y';
        save(&source, "actions/old-name/nested/run.js", &changed);
        assert!(!already_present(&daily, &source).unwrap());
        save(&source, "actions/old-name/nested/run.js", &script);
        save(&source, RAISHOT_MANIFEST_PATH, b"Manifest");
        assert!(!already_present(&daily, &source).unwrap());
        save(&source, RAISHOT_MANIFEST_PATH, b"manifest");
        save(&source, "clipboard-history.json", b"history");
        assert!(!already_present(&daily, &source).unwrap());
        save(&daily, "clipboard-history.json", b"history");
        let lock = fs::File::options()
            .read(true)
            .share_mode(0)
            .open(daily.join("bindings.json"))
            .unwrap();
        assert!(already_present(&daily, &source).is_err());
        drop(lock);
        assert!(already_present(&daily, &source).unwrap());
        fs::remove_dir_all(root).unwrap();
    }
}
