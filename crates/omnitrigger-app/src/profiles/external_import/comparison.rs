//! 导入只覆盖来源提供的数据，本机独有的文件不参与相同副本判定。
use super::{metadata, packages};
use crate::local_files::{is_cloud_path, is_plain, is_plain_directory};
use crate::plugins::{local_manifest_metadata, RAISHOT_MANIFEST_PATH};
use crate::profiles::transferable_file;
use std::{
    collections::BTreeMap,
    ffi::OsString,
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

fn same_actions(
    source: &Path,
    target: &Path,
    packages: Option<&BTreeMap<OsString, OsString>>,
) -> io::Result<bool> {
    if !metadata(target)?.is_some_and(|m| m.is_dir()) {
        return Ok(false);
    }
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let path = entry.path();
        let meta = path.symlink_metadata()?;
        if !is_plain(&meta) || is_cloud_path(&path) {
            continue;
        }
        let name = entry.file_name();
        if packages.is_some()
            && meta.is_dir()
            && omnitrigger_actions::catalog::is_legacy_stage(&name)
        {
            continue;
        }
        let destination = target.join(
            packages
                .and_then(|folders| folders.get(&name))
                .unwrap_or(&name),
        );
        if meta.is_dir() && !same_actions(&path, &destination, None)?
            || meta.is_file() && !same_file(&path, &meta, &destination)?
        {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(in crate::profiles) fn already_present(daily: &Path, source: &Path) -> io::Result<bool> {
    if !is_plain_directory(daily) || !is_plain_directory(source) {
        return Ok(false);
    }
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        let path = entry.path();
        let meta = path.symlink_metadata()?;
        if meta.is_file()
            && is_plain(&meta)
            && !is_cloud_path(&path)
            && transferable_file(&name.to_string_lossy())
            && !same_file(&path, &meta, &daily.join(name))?
        {
            return Ok(false);
        }
    }
    let manifest = source.join(RAISHOT_MANIFEST_PATH);
    if let Some(meta) = local_manifest_metadata(&manifest).map_err(io::Error::other)? {
        let target = daily.join(RAISHOT_MANIFEST_PATH);
        if local_manifest_metadata(&target)
            .map_err(io::Error::other)?
            .is_none()
            || !same_file(&manifest, &meta, &target)?
        {
            return Ok(false);
        }
    }
    let actions = source.join("actions");
    match actions.symlink_metadata() {
        Ok(meta) if meta.is_dir() && is_plain(&meta) => {
            let target = daily.join("actions");
            let Some(packages) = packages::installed_folders(&actions, &target)? else {
                return Ok(false);
            };
            same_actions(&actions, &target, Some(&packages))
        }
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error),
    }
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
