//! 从当前数据根导出配置和已登记的子档，不在来源目录执行扩展。
use super::{is_action_state_document, profile_dir, read_catalog};
use crate::{plugins::RAISHOT_MANIFEST_PATH, script_runtime, settings};
use chrono::{Datelike, Local};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, Metadata},
    io::{self, Read, Seek},
    path::{Component, Path, PathBuf},
    time::SystemTime,
};
use zip::{write::SimpleFileOptions, CompressionMethod, DateTime, ZipArchive, ZipWriter};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Debug, PartialEq, Eq)]
struct SourceFile {
    size: u64,
    modified: SystemTime,
    hash: String,
}

#[derive(Default, PartialEq, Eq)]
struct Inventory {
    profiles: usize,
    folders: BTreeSet<String>,
    files: BTreeMap<String, SourceFile>,
}

fn metadata(path: &Path) -> Result<Option<Metadata>> {
    script_runtime::check_local_path(path)?;
    match fs::symlink_metadata(path) {
        Ok(meta) => Ok(Some(meta)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("无法读取 {}：{e}", path.display()).into()),
    }
}

fn name(root: &Path, path: &Path) -> Result<String> {
    let relative = path.strip_prefix(root)?;
    let mut parts = Vec::new();
    for part in relative.components() {
        let Component::Normal(part) = part else {
            return Err("备份成员路径无效。".into());
        };
        parts.push(part.to_str().ok_or("配置文件名不是有效的 Unicode。")?);
    }
    Ok(parts.join("/"))
}

impl Inventory {
    fn add(
        &mut self,
        root: &Path,
        path: &Path,
        meta: Metadata,
        expected_hash: Option<String>,
    ) -> Result<()> {
        if !meta.is_file() || !crate::local_files::is_plain(&meta) {
            return Err(format!("需要普通本机文件：{}", path.display()).into());
        }
        let key = name(root, path)?;
        let hash = digest(File::open(path)?)?;
        if expected_hash.is_some_and(|expected| !expected.eq_ignore_ascii_case(&hash)) {
            return Err(format!("宿主文件与登记的摘要不符：{key}").into());
        }
        let file = SourceFile {
            size: meta.len(),
            modified: meta.modified()?,
            hash,
        };
        if self.files.insert(key, file).is_some() {
            return Err(format!("出现重复的备份文件：{}", path.display()).into());
        }
        Ok(())
    }

    fn add_existing(&mut self, root: &Path, path: &Path) -> Result<()> {
        if let Some(meta) = metadata(path)? {
            self.add(root, path, meta, None)?;
        }
        Ok(())
    }

    fn actions(&mut self, root: &Path, actions: &Path) -> Result<()> {
        if metadata(actions)?.is_none() {
            return Ok(());
        }
        let mut pending = vec![actions.to_owned()];
        while let Some(folder) = pending.pop() {
            script_runtime::check_local_path(&folder)?;
            self.folders.insert(name(root, &folder)?);
            for item in fs::read_dir(&folder)? {
                let item = item?;
                if folder == actions
                    && omnitrigger_actions::catalog::is_legacy_stage(&item.file_name())
                {
                    continue;
                }
                let path = item.path();
                let meta = metadata(&path)?.ok_or("备份期间动作目录有改动。")?;
                if meta.is_dir() {
                    pending.push(path);
                } else {
                    self.add(root, &path, meta, None)?;
                }
            }
        }
        Ok(())
    }

    fn selected_files(
        &mut self,
        root: &Path,
        folder: &Path,
        include: impl Fn(&Path) -> bool,
    ) -> Result<()> {
        if metadata(folder)?.is_some() {
            for item in fs::read_dir(folder)? {
                let path = item?.path();
                if include(&path) {
                    let meta = metadata(&path)?.ok_or("备份期间配置有改动。")?;
                    self.add(root, &path, meta, None)?;
                }
            }
        }
        Ok(())
    }

    fn collect(root: &Path) -> Result<Self> {
        let mut inventory = Self::default();
        let mut pending = vec![root.to_owned()];
        while let Some(directory) = pending.pop() {
            if !crate::local_files::is_plain_directory(&directory) {
                return Err(format!("配置档目录不可用：{}", directory.display()).into());
            }
            for marker in [
                crate::legacy_upgrade::MARKER,
                "backups/external-import.pending/plan.json",
            ] {
                if metadata(&directory.join(marker))?.is_some() {
                    return Err(
                        format!("配置档还有未完成的升级或导入：{}", directory.display()).into(),
                    );
                }
            }
            inventory.profiles += 1;
            if directory != root {
                inventory.folders.insert(name(root, &directory)?);
                inventory.add_existing(root, &directory.with_extension("copy-origin.json"))?;
            }
            for file in [
                "profiles.json",
                "profile.json",
                "bindings.json",
                "wheels.json",
                "appearance.json",
                "plugins.json",
                RAISHOT_MANIFEST_PATH,
                "backups/seed-source.json",
                "backups/restored-revisions.json",
            ] {
                inventory.add_existing(root, &directory.join(file))?;
            }
            inventory.selected_files(
                root,
                &directory.join("action-state"),
                is_action_state_document,
            )?;
            for file in ["bindings.json", "wheels.json"] {
                inventory.selected_files(
                    root,
                    &directory.join("backups/edits").join(file),
                    |p| {
                        p.file_name()
                            .and_then(settings::edit_revision_number)
                            .is_some()
                    },
                )?;
            }
            inventory.actions(root, &directory.join("actions"))?;
            for (path, hash) in script_runtime::backup_files(&directory)? {
                let meta =
                    metadata(&path)?.ok_or_else(|| format!("缺少宿主文件：{}", path.display()))?;
                inventory.add(root, &path, meta, hash)?;
            }
            for entry in read_catalog(&directory)?.profiles {
                let child = profile_dir(&directory, &entry.id);
                if child.parent() != Some(directory.join("profiles").as_path())
                    || child.file_name() != Some(entry.id.as_ref())
                {
                    return Err(format!("配置档登记中的目录无效：{}", entry.id).into());
                }
                pending.push(child);
            }
        }
        if inventory.files.is_empty() {
            return Err("本档还没有可导出的配置。".into());
        }
        let mut names = BTreeSet::new();
        for key in inventory.files.keys().chain(&inventory.folders) {
            if !names.insert(key.to_lowercase()) {
                return Err(format!("备份内有同名路径：{key}").into());
            }
        }
        Ok(inventory)
    }
}

fn zip_time(time: SystemTime) -> DateTime {
    let local = chrono::DateTime::<Local>::from(time).naive_local();
    if local.year() > 2107 {
        DateTime::from_date_and_time(2107, 12, 31, 23, 59, 58).expect("有效的 ZIP 日期")
    } else {
        DateTime::try_from(local).unwrap_or_default()
    }
}

impl SourceFile {
    fn open(&self, path: &Path) -> Result<File> {
        script_runtime::check_local_path(path)?;
        let file = File::open(path)?;
        self.verify(&file, path)?;
        Ok(file)
    }
    fn verify(&self, file: &File, path: &Path) -> Result<()> {
        let meta = file.metadata()?;
        if !meta.is_file()
            || !crate::local_files::is_plain(&meta)
            || meta.len() != self.size
            || meta.modified()? != self.modified
        {
            return Err(format!("备份期间文件有改动：{}", path.display()).into());
        }
        Ok(())
    }
}

fn digest(mut input: impl Read) -> io::Result<String> {
    let mut sha = Sha256::new();
    io::copy(&mut input, &mut sha)?;
    Ok(format!("{:x}", sha.finalize()))
}

fn write_verified(root: &Path, inventory: &Inventory, archive: &mut File) -> Result<()> {
    let mut zip = ZipWriter::new(&mut *archive);
    for folder in &inventory.folders {
        zip.add_directory(format!("{folder}/"), SimpleFileOptions::default())?;
    }
    for (name, entry) in &inventory.files {
        let path = root.join(name);
        let mut source = entry.open(&path)?;
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .large_file(entry.size >= u64::from(u32::MAX) / 2)
            .last_modified_time(zip_time(entry.modified));
        zip.start_file(name.as_str(), options)?;
        if io::copy(&mut source, &mut zip)? != entry.size {
            return Err(format!("备份期间文件长度有改动：{name}").into());
        }
        entry.verify(&source, &path)?;
    }
    zip.finish()?.sync_all()?;
    archive.rewind()?;
    let mut zip = ZipArchive::new(archive)?;
    for (name, entry) in &inventory.files {
        if digest(zip.by_name(name)?)? != entry.hash {
            return Err(format!("备份校验未通过：{name}").into());
        }
    }
    Ok(())
}

pub(crate) struct ExportedArchive {
    pub path: PathBuf,
    pub profiles: usize,
    pub files: usize,
}

pub(crate) fn export_config_archive(config: &Path) -> std::result::Result<ExportedArchive, String> {
    export(config).map_err(|e| format!("无法导出配置备份：{e}"))
}

fn export(config: &Path) -> Result<ExportedArchive> {
    if !config
        .file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case("bindings.json"))
    {
        return Err("仅支持从标准配置档导出备份。".into());
    }
    let config = std::path::absolute(config)?;
    let root = config.parent().ok_or("无法定位当前配置档。")?;
    let inventory = Inventory::collect(root)?;
    let destination = root.join("backups");
    script_runtime::check_local_path(&destination)?;
    fs::create_dir_all(&destination)?;
    let prefix = format!(
        "OmniTrigger-config-{}-",
        Local::now().format("%Y%m%d-%H%M%S")
    );
    let mut stage = tempfile::Builder::new()
        .prefix(&prefix)
        .suffix(".part")
        .tempfile_in(&destination)?;
    write_verified(root, &inventory, stage.as_file_mut())?;
    if inventory != Inventory::collect(root)? {
        return Err("备份期间配置档有改动，请重试。".into());
    }
    let path = stage.path().with_extension("zip");
    script_runtime::check_local_path(&path)?;
    stage.persist_noclobber(&path)?;
    Ok(ExportedArchive {
        path,
        profiles: inventory.profiles,
        files: inventory.files.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::{create_profile, Seed};

    fn put(root: &Path, path: &str, content: &[u8]) {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    #[test]
    fn restores_registered_profiles_packages_state_and_edits_without_unrelated_files() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path();
        let child = create_profile(root, "testing", &Seed::Empty, None).unwrap();
        let child_dir = profile_dir(root, &child.id);
        let cfg = br#"{"version":24,"bindings":[]}"#;
        let revision = "backups/edits/bindings.json/edit-0000000000000001.json";
        let state = format!("action-state/{}.json", "a".repeat(64));
        put(root, "bindings.json", cfg);
        put(root, "actions/example/action.json", b"package");
        fs::create_dir(root.join("actions/example/empty")).unwrap();
        put(root, revision, b"old configuration");
        put(&child_dir, "wheels.json", b"wheel");
        put(&child_dir, &state, b"state");
        let excluded = [
            "profiles/unregistered/bindings.json",
            "backups/other.zip",
            "backups/edits/bindings.json/edit-1.json",
            "action-state/cache.json",
            "actions/.install-legacy-1-2-3/incomplete",
            "clipboard/history.json",
        ];
        for file in excluded {
            put(root, file, b"excluded");
        }
        let exported = export_config_archive(&root.join("bindings.json")).unwrap();
        assert_eq!(exported.profiles, 2);
        let restored = tempfile::tempdir().unwrap();
        let mut zip = ZipArchive::new(File::open(&exported.path).unwrap()).unwrap();
        assert_eq!(
            zip.file_names().filter(|n| !n.ends_with('/')).count(),
            exported.files
        );
        zip.extract(restored.path()).unwrap();
        let restored_child = profile_dir(restored.path(), &child.id);
        for (path, bytes) in [
            (restored.path().join("bindings.json"), cfg.as_slice()),
            (restored.path().join(revision), b"old configuration"),
            (restored_child.join("wheels.json"), b"wheel"),
            (restored_child.join(&state), b"state"),
            (
                restored.path().join("actions/example/action.json"),
                b"package",
            ),
        ] {
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
        assert!(restored.path().join("actions/example/empty").is_dir());
        assert_eq!(
            read_catalog(restored.path()).unwrap().profiles[0].id,
            child.id
        );
        assert!(excluded.iter().all(|p| !restored.path().join(p).exists()));
        let isolated = export_config_archive(&child_dir.join("bindings.json")).unwrap();
        assert_eq!(isolated.profiles, 1);
        let zip = ZipArchive::new(File::open(isolated.path).unwrap()).unwrap();
        assert!(!zip
            .file_names()
            .any(|n| n.starts_with("profiles/") || n == revision));
        assert!(zip.file_names().any(|n| n == state));
    }

    #[test]
    fn rejects_changed_bytes_even_when_size_and_timestamp_match() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let path = root.join("bindings.json");
        put(root, "bindings.json", b"before");
        let original = Inventory::collect(root).unwrap();
        let time = original.files["bindings.json"].modified;
        fs::write(&path, b"after!").unwrap();
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(time)
            .unwrap();
        let mut stage = tempfile::tempfile().unwrap();
        assert!(write_verified(root, &original, &mut stage).is_err());
        assert!(original != Inventory::collect(root).unwrap());
    }

    #[test]
    fn backs_up_only_verified_files_of_the_active_runtime() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let hash = "a".repeat(64);
        let package = root.join("runtimes/script/packages").join(&hash);
        let contents = b"fixture";
        let sha = digest(contents.as_slice()).unwrap();
        let mut records = Vec::new();
        for name in [
            "host.exe",
            "LICENSE",
            "README.txt",
            "THIRD_PARTY_NOTICES.md",
            "sdk/api.d.ts",
            "licenses/dependencies.json",
        ] {
            put(&package, name, contents);
            records.push(serde_json::json!({"path":name,"size":contents.len(),"sha256":sha}));
        }
        let manifest = serde_json::to_vec(&serde_json::json!({
            "schema_version":1,"product":"omnitrigger-script-host","version":"1.0",
            "engine":"boa","target":"windows-x86_64","api_version":1,"protocol_schema":1,
            "distribution":"development","source_revision":"a".repeat(40),"source_dirty":false,
            "files":records,
        }))
        .unwrap();
        put(&package, "manifest.json", &manifest);
        put(root, "runtimes/script/current.json", &serde_json::to_vec(&serde_json::json!({
            "schema_version":1, "archive_sha256":hash, "manifest_sha256":digest(manifest.as_slice()).unwrap()
        })).unwrap());
        put(root, "runtimes/script/packages/inactive/host.exe", b"older");
        let config = root.join("bindings.json");
        let result = export_config_archive(&config).unwrap();
        assert_eq!(result.files, 8);
        let zip = ZipArchive::new(File::open(&result.path).unwrap()).unwrap();
        assert!(!zip.file_names().any(|n| n.contains("inactive")));
        for (bad, message) in [(b"short".as_slice(), "长度"), (b"changed", "摘要")] {
            put(&package, "host.exe", bad);
            assert!(export_config_archive(&config)
                .err()
                .unwrap()
                .contains(message));
            assert_eq!(fs::read_dir(root.join("backups")).unwrap().count(), 1);
        }
    }
}
