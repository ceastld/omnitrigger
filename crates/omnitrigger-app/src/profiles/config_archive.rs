//! 从当前数据根导出配置和已登记的子档，不在来源目录执行扩展。
use super::{checked_profile_dir, is_action_state_document, read_catalog};
use crate::{plugins::RAISHOT_MANIFEST_PATH, script_runtime, settings};
use chrono::{Datelike, Local};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, Metadata},
    io::{self, Read, Seek, Write},
    path::{Component, Path, PathBuf},
    time::SystemTime,
};
use zip::{write::SimpleFileOptions, CompressionMethod, DateTime, ZipArchive, ZipWriter};
mod restore;
pub(super) use restore::{archive_digest, restore_archive};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Debug, PartialEq, Eq)]
struct SourceFile {
    path: PathBuf,
    size: u64,
    modified: SystemTime,
    hash: String,
}

#[derive(Default, PartialEq, Eq)]
struct Inventory {
    profiles: BTreeSet<PathBuf>,
    inherited_sources: BTreeSet<PathBuf>,
    folders: BTreeSet<String>,
    files: BTreeMap<String, SourceFile>,
    catalog_identity: Option<Vec<u8>>,
}

// 旧测试档可能只在父档登记。独立备份需要能在原目录之外识别这份配置。
fn catalog_identity(root: &Path) -> Result<Option<Vec<u8>>> {
    if !super::is_profile_directory(root)
        || !root
            .file_name()
            .and_then(|id| id.to_str())
            .is_some_and(super::generated_profile_id)
    {
        return Ok(None);
    }
    let parent = root
        .parent()
        .and_then(Path::parent)
        .ok_or("无法定位测试档的父档。")?;
    if metadata(&super::catalog_path(parent))?.is_none() {
        return Ok(None);
    }
    read_catalog(parent)?
        .profiles
        .into_iter()
        .find(|entry| entry.is_test_at(root))
        .map(|entry| Ok(serde_json::to_vec_pretty(&entry)?))
        .transpose()
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
        self.add_named(name(root, path)?, path, meta, expected_hash)
    }

    fn add_named(
        &mut self,
        key: String,
        path: &Path,
        meta: Metadata,
        expected_hash: Option<String>,
    ) -> Result<()> {
        if !meta.is_file() || !crate::local_files::is_plain(&meta) {
            return Err(format!("需要普通本机文件：{}", path.display()).into());
        }
        let hash = digest(File::open(path)?)?;
        if expected_hash.is_some_and(|expected| !expected.eq_ignore_ascii_case(&hash)) {
            return Err(format!("宿主文件与登记的摘要不符：{key}").into());
        }
        let file = SourceFile {
            path: path.to_owned(),
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
        Self::collect_profile_files(root, true)
    }

    fn collect_profile_files(root: &Path, include_inherited: bool) -> Result<Self> {
        if !crate::local_files::is_plain_directory(root) {
            return Err(format!("配置档目录不可用：{}", root.display()).into());
        }
        let canonical = root.canonicalize()?;
        let root = canonical.as_path();
        let mut inventory = Self::default();
        let mut own_hosts = BTreeSet::new();
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
            inventory.profiles.insert(directory.clone());
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
            let runtime_files = script_runtime::backup_files(&directory)?;
            if !runtime_files.is_empty() {
                own_hosts.insert(directory.clone());
            }
            for (path, hash) in runtime_files {
                let meta =
                    metadata(&path)?.ok_or_else(|| format!("缺少宿主文件：{}", path.display()))?;
                inventory.add(root, &path, meta, hash)?;
            }
            for entry in read_catalog(&directory)?.profiles {
                pending.push(checked_profile_dir(&directory, &entry)?);
            }
        }
        if include_inherited {
            if !inventory.files.contains_key("profile.json") {
                inventory.catalog_identity = catalog_identity(root)?;
            }
            for (destination, source) in
                script_runtime::inherited_backup_sources(&inventory.profiles, &own_hosts)?
            {
                for (path, hash) in script_runtime::inherited_backup_files(&source)? {
                    let member = destination.join(path.strip_prefix(&source)?);
                    let meta = metadata(&path)?.ok_or("来源档的脚本宿主文件已变更。")?;
                    inventory.add_named(name(root, &member)?, &path, meta, hash)?;
                }
                inventory.inherited_sources.insert(source);
            }
        }
        if inventory.files.is_empty() && inventory.catalog_identity.is_none() {
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

fn write_verified(inventory: &Inventory, archive: &mut File) -> Result<()> {
    let mut zip = ZipWriter::new(&mut *archive);
    if let Some(identity) = &inventory.catalog_identity {
        zip.start_file(
            "profile.json",
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
        )?;
        zip.write_all(identity)?;
    }
    for folder in &inventory.folders {
        zip.add_directory(format!("{folder}/"), SimpleFileOptions::default())?;
    }
    for (name, entry) in &inventory.files {
        let mut source = entry.open(&entry.path)?;
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .large_file(entry.size >= u64::from(u32::MAX) / 2)
            .last_modified_time(zip_time(entry.modified));
        zip.start_file(name.as_str(), options)?;
        if io::copy(&mut source, &mut zip)? != entry.size {
            return Err(format!("备份期间文件长度有改动：{name}").into());
        }
        entry.verify(&source, &entry.path)?;
    }
    zip.finish()?.sync_all()?;
    archive.rewind()?;
    let mut zip = ZipArchive::new(archive)?;
    if let Some(identity) = &inventory.catalog_identity {
        if digest(zip.by_name("profile.json")?)? != digest(identity.as_slice())? {
            return Err("备份校验未通过：profile.json".into());
        }
    }
    for (name, entry) in &inventory.files {
        if digest(zip.by_name(name)?)? != entry.hash {
            return Err(format!("备份校验未通过：{name}").into());
        }
    }
    Ok(())
}

#[derive(serde::Serialize)]
pub(crate) struct ExportedArchive {
    #[serde(rename = "archive")]
    pub path: PathBuf,
    pub profiles: usize,
    pub files: usize,
    pub bytes: u64,
}

pub(crate) fn export_config_archive(config: &Path) -> std::result::Result<ExportedArchive, String> {
    export(config, None).map_err(|e| format!("无法导出配置备份：{e}"))
}

pub(super) fn export_to(
    config: &Path,
    output: &Path,
) -> std::result::Result<ExportedArchive, String> {
    export(config, Some(output)).map_err(|e| format!("无法导出配置备份：{e}"))
}

fn archive_target(output: &Path, inventory: &Inventory) -> Result<PathBuf> {
    let output = std::path::absolute(output)?;
    script_runtime::check_local_path(&output)?;
    if !output
        .extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("zip"))
    {
        return Err("备份文件名须以 .zip 结尾。".into());
    }
    if output.try_exists()? {
        return Err(format!("备份文件已存在：{}", output.display()).into());
    }
    let directory = super::config_directory(&output).ok_or("无法定位备份目标目录。")?;
    if directory.components().any(|c| c == Component::ParentDir) {
        return Err("请指定可确定的备份目标目录。".into());
    }
    if inventory
        .profiles
        .iter()
        .chain(&inventory.inherited_sources)
        .any(|profile| {
            directory
                .strip_prefix(profile)
                .ok()
                .and_then(|relative| relative.components().next())
                .is_some_and(|first| {
                    ["actions", "action-state", "runtimes"]
                        .iter()
                        .any(|protected| first.as_os_str().eq_ignore_ascii_case(protected))
                })
        })
    {
        return Err("请选择动作包、扩展状态或运行时目录以外的位置保存备份。".into());
    }
    Ok(output)
}

fn export(config: &Path, output: Option<&Path>) -> Result<ExportedArchive> {
    if !config
        .file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case("bindings.json"))
    {
        return Err("仅支持从标准配置档导出备份。".into());
    }
    let config = std::path::absolute(config)?;
    let root = config.parent().ok_or("无法定位当前配置档。")?;
    let inventory = Inventory::collect(root)?;
    let output = output.map(|p| archive_target(p, &inventory)).transpose()?;
    let destination = output
        .as_deref()
        .and_then(Path::parent)
        .map(Path::to_owned)
        .unwrap_or_else(|| root.join("backups"));
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
    write_verified(&inventory, stage.as_file_mut())?;
    if inventory != Inventory::collect(root)? {
        return Err("备份期间配置档有改动，请重试。".into());
    }
    let path = output.unwrap_or_else(|| stage.path().with_extension("zip"));
    let bytes = stage.as_file().metadata()?.len();
    script_runtime::check_local_path(&path)?;
    stage.persist_noclobber(&path)?;
    Ok(ExportedArchive {
        path,
        profiles: inventory.profiles.len(),
        files: inventory.files.len() + usize::from(inventory.catalog_identity.is_some()),
        bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::{create_profile, profile_dir, Seed};

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
        let mut zip = ZipArchive::new(File::open(isolated.path).unwrap()).unwrap();
        assert!(!zip
            .file_names()
            .any(|n| n.starts_with("profiles/") || n == revision));
        assert!(zip.file_names().any(|n| n == state));
        let detached = tempfile::tempdir().unwrap();
        zip.extract(detached.path()).unwrap();
        let config = detached.path().join("bindings.json");
        use crate::profiles::{prepare_root_access, ProfileCommandError, RootAccess};
        for access in [RootAccess::Inspect, RootAccess::Open] {
            let (startup, locks) = prepare_root_access(&config, access).unwrap();
            assert_eq!(startup.test_name.as_deref(), Some(child.name.as_str()));
            assert!(startup.test_data && !startup.listen && locks.is_empty());
        }
        assert!(matches!(
            prepare_root_access(&config, RootAccess::Tray),
            Err(ProfileCommandError::TestInTray)
        ));
    }

    #[test]
    fn standalone_legacy_test_exports_its_catalog_identity_without_changing_the_source() {
        let base = tempfile::tempdir().unwrap();
        let parent = base.path();
        let entry = create_profile(parent, "old-test", &Seed::Empty, Some("历史工作区")).unwrap();
        let root = profile_dir(parent, &entry.id);
        let identity = root.join("profile.json");
        fs::remove_file(&identity).unwrap();
        let exported = export_config_archive(&root.join("bindings.json")).unwrap();
        assert_eq!(exported.files, 1);
        let detached = tempfile::tempdir().unwrap();
        ZipArchive::new(File::open(exported.path).unwrap())
            .unwrap()
            .extract(detached.path())
            .unwrap();
        assert_eq!(
            super::super::restored_root_identity(detached.path()).unwrap(),
            Some(entry.clone())
        );
        assert!(!identity.exists());
        assert!(!detached.path().join("profiles.json").exists());

        let full = Inventory::collect(parent).unwrap();
        assert!(full.catalog_identity.is_none());
        assert!(!full
            .files
            .contains_key(&format!("profiles/{}/profile.json", entry.id)));
        let other = profile_dir(parent, "t000000000000");
        put(&other, "bindings.json", b"{}");
        assert!(Inventory::collect(&other)
            .unwrap()
            .catalog_identity
            .is_none());

        let mut raw = serde_json::to_vec(&entry).unwrap();
        raw.extend_from_slice(b"\r\n");
        fs::write(&identity, &raw).unwrap();
        let exported = export_config_archive(&root.join("bindings.json")).unwrap();
        ZipArchive::new(File::open(exported.path).unwrap())
            .unwrap()
            .extract(detached.path())
            .unwrap();
        assert_eq!(fs::read(detached.path().join("profile.json")).unwrap(), raw);
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
        assert!(write_verified(&original, &mut stage).is_err());
        assert!(original != Inventory::collect(root).unwrap());
    }

    fn put_managed_host(root: &Path) -> PathBuf {
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
        package
    }

    #[test]
    fn zip_seed_restores_nested_profiles_state_history_and_the_archived_runtime() {
        use crate::profiles::{read_catalog, reset_profile};
        let source = tempfile::tempdir().unwrap();
        let src = source.path();
        let config = br#"{"version":24,"bindings":[]}"#;
        put(src, "bindings.json", config);
        let child = create_profile(src, "child", &Seed::Copy("default".into()), None).unwrap();
        let state_name = format!(
            "action-state/{}.json",
            digest(br#"{"package_id":"demo","action_id":"echo"}"#.as_slice()).unwrap()
        );
        let state = br#"{"schema_version":1,"revision":2,"scope":{"package_id":"demo","action_id":"echo"},"values":{"example":42}}"#;
        put(&profile_dir(src, &child.id), &state_name, state);
        let edit = "backups/edits/bindings.json/edit-0000000000000001.json";
        put(src, edit, b"previous");
        let edit_time = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        File::options()
            .write(true)
            .open(src.join(edit))
            .unwrap()
            .set_modified(edit_time)
            .unwrap();
        let host = put_managed_host(src)
            .join("host.exe")
            .strip_prefix(src)
            .unwrap()
            .to_owned();
        let zip = export_config_archive(&src.join("bindings.json"))
            .unwrap()
            .path;
        let destination = tempfile::tempdir().unwrap();
        let root = destination.path();
        put(root, "bindings.json", b"original");
        let seed = Seed::Zip {
            path: zip.clone(),
            digest: None,
        };
        let entry = create_profile(root, "restored", &seed, None).unwrap();
        let imported = profile_dir(root, &entry.id);
        assert_eq!(fs::read(root.join("bindings.json")).unwrap(), b"original");
        assert_eq!(
            read_catalog(&imported).unwrap().profiles.as_slice(),
            std::slice::from_ref(&child)
        );
        assert_eq!(
            fs::read(profile_dir(&imported, &child.id).join(state_name)).unwrap(),
            state
        );
        assert_eq!(fs::read(imported.join(edit)).unwrap(), b"previous");
        assert_eq!(
            fs::metadata(imported.join(edit))
                .unwrap()
                .modified()
                .unwrap(),
            edit_time
        );
        assert_eq!(fs::read(imported.join(&host)).unwrap(), b"fixture");
        assert!(entry.seed.starts_with("zip-sha256:"));
        put(&imported, "bindings.json", b"edited");
        let previous = reset_profile(root, "restored").unwrap().backup.unwrap();
        assert_eq!(fs::read(previous.join("bindings.json")).unwrap(), b"edited");
        assert_eq!(fs::read(imported.join("bindings.json")).unwrap(), config);
        assert_eq!(fs::read(imported.join(host)).unwrap(), b"fixture");
        put(&imported, "bindings.json", b"still editing");
        fs::write(&zip, b"changed archive").unwrap();
        assert!(reset_profile(root, "restored").is_err());
        assert_eq!(
            fs::read(imported.join("bindings.json")).unwrap(),
            b"still editing"
        );
    }

    #[test]
    fn invalid_zip_seed_never_publishes_a_partial_profile() {
        let source = tempfile::tempdir().unwrap();
        let src = source.path();
        let root = tempfile::tempdir().unwrap();
        let state_path = format!("action-state/{}.json", "a".repeat(64));
        for (file, bytes) in [
            ("bindings.json", b"bad config".as_slice()),
            (state_path.as_str(), b"bad state"),
        ] {
            put(src, file, bytes);
            let zip = export_config_archive(&src.join("bindings.json"))
                .unwrap()
                .path;
            let error = create_profile(
                root.path(),
                "new",
                &Seed::Zip {
                    path: zip,
                    digest: None,
                },
                None,
            )
            .unwrap_err();
            assert!(error.contains("无法还原配置 ZIP"), "{error}");
            assert!(read_catalog(root.path()).unwrap().profiles.is_empty());
            assert_eq!(
                fs::read_dir(root.path().join("profiles")).unwrap().count(),
                0
            );
            fs::remove_file(src.join(file)).unwrap();
        }
    }

    #[test]
    fn backs_up_only_verified_files_of_the_active_runtime() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let package = put_managed_host(root);
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

    #[test]
    fn detached_copies_retain_their_effective_host_and_internal_inheritance() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        put(root, "bindings.json", br#"{"version":24,"bindings":[]}"#);
        let package = put_managed_host(root);
        let copied = |root: &Path, alias: &str, seed: &str| {
            let entry = create_profile(root, alias, &Seed::Copy(seed.into()), None).unwrap();
            profile_dir(root, &entry.id)
        };
        let parent = copied(root, "trial", "default");
        let child = copied(&parent, "child", "default");
        let sibling = copied(&parent, "other", "child");
        let child_relative = child.strip_prefix(&parent).unwrap();
        let sibling_relative = sibling.strip_prefix(&parent).unwrap();
        let pointer = Path::new("runtimes/script/current.json");
        let legacy =
            Path::new("runtimes/script").join(if cfg!(windows) { "host.exe" } else { "host" });
        for has_legacy in [false, true] {
            if has_legacy {
                fs::create_dir_all(parent.join("runtimes/script")).unwrap();
                fs::write(parent.join(&legacy), b"legacy").unwrap();
            }
            let result = export_config_archive(&parent.join("bindings.json")).unwrap();
            let copy = tempfile::tempdir().unwrap();
            ZipArchive::new(File::open(result.path).unwrap())
                .unwrap()
                .extract(copy.path())
                .unwrap();
            assert!(!parent.join(pointer).exists() && !child.join(pointer).exists());
            assert_eq!(copy.path().join(pointer).is_file(), !has_legacy);
            assert_eq!(
                copy.path().join(child_relative).join(pointer).is_file(),
                has_legacy
            );
            assert!(!copy.path().join(sibling_relative).join(pointer).exists());
            for dir in [Path::new(""), child_relative, sibling_relative] {
                let restored =
                    script_runtime::locate_in_profile(&copy.path().join(dir).join("bindings.json"))
                        .unwrap()
                        .unwrap();
                assert!(restored
                    .program
                    .canonicalize()
                    .unwrap()
                    .starts_with(copy.path().canonicalize().unwrap()));
                let expected = if has_legacy && dir == Path::new("") {
                    b"legacy".as_slice()
                } else {
                    b"fixture"
                };
                assert_eq!(fs::read(restored.program).unwrap(), expected);
            }
        }
        fs::write(package.join("host.exe"), b"changed").unwrap();
        assert!(export_config_archive(&parent.join("bindings.json"))
            .err()
            .unwrap()
            .contains("摘要"));
    }

    #[test]
    fn custom_output_preserves_existing_files_and_avoids_action_directories() {
        let root = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        let config = root.path().join("bindings.json");
        fs::write(&config, b"[]").unwrap();
        let output = dest.path().join("new/group.zip");
        let result = export_to(&config, &output).unwrap();
        assert_eq!(result.path, output);
        let original = fs::read(&output).unwrap();
        assert!(export_to(&config, &output).is_err());
        assert_eq!(fs::read(output).unwrap(), original);
        let child = create_profile(root.path(), "trial", &Seed::Empty, None).unwrap();
        let protected = profile_dir(root.path(), &child.id).join("actions");
        assert!(export_to(&config, &protected.join("out.zip")).is_err());
        assert!(!protected.exists());
    }
}
