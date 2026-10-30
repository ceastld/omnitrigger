use crate::model::{valid_id, ActionCall, ActionDefinition, ActionError, PackageManifest};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, Metadata},
    io::Read,
    path::{Component, Path, PathBuf},
};

const MAX_MANIFEST_BYTES: usize = 256 * 1024;
const MAX_PACKAGES: usize = 128;
const MAX_DIRECTORY_ENTRIES: usize = 512;
const MAX_ACTIONS_PER_PACKAGE: usize = 128;

#[derive(Clone, Debug)]
pub struct CatalogAction {
    pub package_id: String,
    pub package_name: String,
    pub definition: ActionDefinition,
}

#[derive(Clone, Debug)]
pub struct ResolvedAction {
    pub call: ActionCall,
    pub definition: ActionDefinition,
    pub package_root: PathBuf,
    pub entry_path: PathBuf,
}

#[derive(Clone, Debug)]
struct CatalogEntry {
    definition: ActionDefinition,
    package_root: PathBuf,
    entry_path: PathBuf,
}

/// 单次加载的只读快照。查询不会访问文件系统，也不会导入或运行脚本。
#[derive(Clone, Debug, Default)]
pub struct PackageCatalog {
    pub actions: Vec<CatalogAction>,
    pub diagnostics: Vec<String>,
    entries: BTreeMap<(String, String), CatalogEntry>,
    package_ids: BTreeSet<String>,
}

impl PackageCatalog {
    pub fn load(root: &Path) -> Self {
        let mut catalog = Self::default();
        if let Err(error) = catalog.load_inner(root) {
            catalog.diagnostics.push(error.to_string());
        }
        catalog
    }

    fn load_inner(&mut self, root: &Path) -> Result<(), ActionError> {
        let metadata = match fs::symlink_metadata(root) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(io_error("读取动作目录失败", error)),
        };
        check_local(root, &metadata)?;
        check_ancestors(root)?;
        if !metadata.is_dir() {
            return Err(ActionError::new(
                "invalid_catalog",
                "动作目录路径不是文件夹。",
            ));
        }
        let canonical_root =
            fs::canonicalize(root).map_err(|error| io_error("定位动作目录失败", error))?;
        let directory =
            fs::read_dir(&canonical_root).map_err(|error| io_error("枚举动作目录失败", error))?;
        let mut folders = Vec::new();
        for (index, entry) in directory.enumerate() {
            if index == MAX_DIRECTORY_ENTRIES {
                return Err(ActionError::new(
                    "catalog_limit",
                    "动作目录超过 512 个项目，请减少目录内容后重新加载。",
                ));
            }
            let entry = entry.map_err(|error| io_error("读取动作目录项目失败", error))?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| io_error("读取动作包属性失败", error))?;
            if let Err(error) = check_local(&path, &metadata) {
                self.diagnostics
                    .push(format!("{}：{error}", path.display()));
                continue;
            }
            if metadata.is_dir() {
                folders.push(path);
                if folders.len() > MAX_PACKAGES {
                    return Err(ActionError::new(
                        "catalog_limit",
                        "动作目录最多加载 128 个动作包。",
                    ));
                }
            }
        }
        folders.sort();
        let mut packages = Vec::new();
        let mut identity_counts = BTreeMap::<String, usize>::new();
        for folder in folders {
            match read_manifest(&folder) {
                Ok(manifest) => {
                    *identity_counts.entry(manifest.id.clone()).or_default() += 1;
                    packages.push((folder, manifest));
                }
                Err(error) => self
                    .diagnostics
                    .push(format!("{}：{error}", folder.display())),
            }
        }
        for (folder, manifest) in packages {
            if identity_counts[&manifest.id] > 1 {
                self.diagnostics.push(format!(
                    "动作包 {} 存在重复标识，已拒绝 {}。",
                    manifest.id,
                    folder.display()
                ));
                continue;
            }
            let loaded = (|| {
                validate_manifest(&manifest)?;
                let package_root =
                    fs::canonicalize(&folder).map_err(|error| io_error("定位动作包失败", error))?;
                if !package_root.starts_with(&canonical_root) {
                    return Err(ActionError::new("invalid_path", "动作包路径超出动作目录。"));
                }
                let mut entries = Vec::new();
                for definition in &manifest.actions {
                    let entry_path = checked_entry(&package_root, &definition.entry)?;
                    entries.push(CatalogEntry {
                        definition: definition.clone(),
                        package_root: package_root.clone(),
                        entry_path,
                    });
                }
                Ok(entries)
            })();
            match loaded {
                Ok(entries) => {
                    self.package_ids.insert(manifest.id.clone());
                    for entry in entries {
                        self.actions.push(CatalogAction {
                            package_id: manifest.id.clone(),
                            package_name: manifest.name.clone(),
                            definition: entry.definition.clone(),
                        });
                        self.entries
                            .insert((manifest.id.clone(), entry.definition.id.clone()), entry);
                    }
                }
                Err(error) => self
                    .diagnostics
                    .push(format!("动作包 {}：{error}", manifest.id)),
            }
        }
        Ok(())
    }

    pub fn resolve(&self, call: &ActionCall) -> Result<ResolvedAction, ActionError> {
        call.validate()?;
        if !self.package_ids.contains(&call.package_id) {
            return Err(ActionError::new(
                "package_missing",
                format!("动作包 {} 不可用，原调用已保留。", call.package_id),
            ));
        }
        let entry = self
            .entries
            .get(&(call.package_id.clone(), call.action_id.clone()))
            .ok_or_else(|| {
                ActionError::new(
                    "action_missing",
                    format!("动作包 {} 未提供动作 {}。", call.package_id, call.action_id),
                )
            })?;
        if entry.definition.version != call.action_version {
            return Err(ActionError::new(
                "incompatible_version",
                "动作契约版本不兼容，原调用已保留。",
            ));
        }
        Ok(ResolvedAction {
            call: call.clone(),
            definition: entry.definition.clone(),
            package_root: entry.package_root.clone(),
            entry_path: entry.entry_path.clone(),
        })
    }
}

fn read_manifest(folder: &Path) -> Result<PackageManifest, ActionError> {
    let path = folder.join("action.json");
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| io_error("读取 action.json 属性失败", error))?;
    check_local(&path, &metadata)?;
    if !metadata.is_file() || metadata.len() > MAX_MANIFEST_BYTES as u64 {
        return Err(ActionError::new(
            "invalid_manifest",
            "action.json 必须是至多 256 KiB 的本地文件。",
        ));
    }
    let mut bytes = Vec::new();
    fs::File::open(&path)
        .map_err(|error| io_error("打开 action.json 失败", error))?
        .take((MAX_MANIFEST_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| io_error("读取 action.json 失败", error))?;
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(ActionError::new(
            "manifest_too_large",
            "action.json 超过 256 KiB。",
        ));
    }
    serde_json::from_slice(&bytes).map_err(|error| {
        ActionError::new("invalid_manifest", format!("action.json 格式有误：{error}"))
    })
}

fn validate_manifest(manifest: &PackageManifest) -> Result<(), ActionError> {
    if manifest.schema_version != 1 {
        return Err(ActionError::new(
            "incompatible_manifest",
            "仅支持动作包清单版本 1。",
        ));
    }
    if !valid_id(&manifest.id)
        || !valid_label(&manifest.name)
        || manifest.version.trim().is_empty()
        || manifest.version.len() > 100
        || manifest.version.chars().any(char::is_control)
    {
        return Err(ActionError::new(
            "invalid_manifest",
            "动作包标识、名称或版本无效。",
        ));
    }
    if manifest.actions.is_empty() || manifest.actions.len() > MAX_ACTIONS_PER_PACKAGE {
        return Err(ActionError::new(
            "invalid_manifest",
            "每个动作包需要 1–128 个动作。",
        ));
    }
    let mut identities = BTreeSet::new();
    for action in &manifest.actions {
        if !valid_id(&action.id)
            || !valid_label(&action.title)
            || action.version == 0
            || !valid_id(&action.runtime)
        {
            return Err(ActionError::new(
                "invalid_manifest",
                "动作标识、名称、版本或运行时标识无效。",
            ));
        }
        if !identities.insert(&action.id) {
            return Err(ActionError::new(
                "duplicate_action",
                format!("动作标识 {} 重复。", action.id),
            ));
        }
    }
    Ok(())
}

fn valid_label(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 1024 && !value.contains('\0')
}

fn checked_entry(root: &Path, entry: &str) -> Result<PathBuf, ActionError> {
    if entry.is_empty() || entry.len() > 4096 || entry.contains(['\0', ':']) {
        return Err(ActionError::new(
            "invalid_entry",
            "入口必须是动作包内的相对文件路径。",
        ));
    }
    let mut path = root.to_path_buf();
    for component in Path::new(entry).components() {
        let Component::Normal(name) = component else {
            return Err(ActionError::new(
                "invalid_entry",
                "入口不能使用绝对路径或父目录。",
            ));
        };
        path.push(name);
        let metadata =
            fs::symlink_metadata(&path).map_err(|error| io_error("读取动作入口属性失败", error))?;
        check_local(&path, &metadata)?;
    }
    if !path.is_file() {
        return Err(ActionError::new("invalid_entry", "动作入口不是文件。"));
    }
    let canonical = fs::canonicalize(path).map_err(|error| io_error("定位动作入口失败", error))?;
    if !canonical.starts_with(root) {
        return Err(ActionError::new(
            "invalid_entry",
            "动作入口超出动作包目录。",
        ));
    }
    Ok(canonical)
}

fn check_local(path: &Path, metadata: &Metadata) -> Result<(), ActionError> {
    let cloud = path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy().to_ascii_lowercase();
        name.starts_with("onedrive") || name == "dropbox" || name.starts_with("icloud")
    });
    if cloud || metadata.file_type().is_symlink() || unavailable_attributes(metadata) {
        return Err(ActionError::new(
            "nonlocal_path",
            "跳过链接、云盘目录或云端占位文件。",
        ));
    }
    Ok(())
}

/// 只读取明确路径的祖先属性，避免调用者把目录放在链接或联接点下。
fn check_ancestors(path: &Path) -> Result<(), ActionError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| io_error("读取当前目录失败", error))?
            .join(path)
    };
    for ancestor in absolute.ancestors().skip(1) {
        let metadata = fs::symlink_metadata(ancestor)
            .map_err(|error| io_error("读取动作目录祖先属性失败", error))?;
        check_local(ancestor, &metadata)?;
    }
    Ok(())
}

#[cfg(windows)]
fn unavailable_attributes(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    // REPARSE_POINT、OFFLINE、RECALL_ON_OPEN、RECALL_ON_DATA_ACCESS。
    metadata.file_attributes() & (0x400 | 0x1000 | 0x40000 | 0x400000) != 0
}

#[cfg(not(windows))]
fn unavailable_attributes(_: &Metadata) -> bool {
    false
}

fn io_error(context: &str, error: std::io::Error) -> ActionError {
    ActionError::new("catalog_io", format!("{context}：{error}"))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(1);
    pub(crate) struct Fixture(pub PathBuf);
    impl Fixture {
        pub(crate) fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "omnitrigger-actions-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }
        pub(crate) fn package(&self, folder: &str, id: &str, runtime: &str) -> PathBuf {
            let root = self.0.join(folder);
            fs::create_dir_all(&root).unwrap();
            fs::write(
                root.join("main.mjs"),
                "throw new Error('目录加载不得执行脚本');",
            )
            .unwrap();
            fs::write(root.join("action.json"), serde_json::to_vec(&json!({"schema_version":1,"id":id,"name":"示例包","version":"1.0.0","actions":[{"id":"run","title":"运行","version":1,"runtime":runtime,"entry":"main.mjs"}]})).unwrap()).unwrap();
            root
        }
        pub(crate) fn call() -> ActionCall {
            ActionCall {
                package_id: "example".into(),
                action_id: "run".into(),
                action_version: 1,
                input: json!({}),
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn loading_is_declarative_and_resolving_uses_a_snapshot() {
        let fixture = Fixture::new();
        let root = fixture.package("sample", "example", "future-runtime");
        let catalog = PackageCatalog::load(&fixture.0);
        assert!(catalog.diagnostics.is_empty(), "{:?}", catalog.diagnostics);
        assert_eq!(catalog.actions.len(), 1);
        fs::remove_file(root.join("main.mjs")).unwrap();
        assert_eq!(
            catalog
                .resolve(&Fixture::call())
                .unwrap()
                .definition
                .runtime,
            "future-runtime"
        );
        let mut call = Fixture::call();
        call.action_version = 2;
        assert_eq!(
            catalog.resolve(&call).unwrap_err().code,
            "incompatible_version"
        );
        call.action_id = "other".into();
        assert_eq!(catalog.resolve(&call).unwrap_err().code, "action_missing");
        call.package_id = "absent".into();
        assert_eq!(catalog.resolve(&call).unwrap_err().code, "package_missing");
    }

    #[test]
    fn duplicate_package_id_disables_every_copy() {
        let fixture = Fixture::new();
        fixture.package("a", "example", "javascript");
        fixture.package("b", "example", "javascript");
        let catalog = PackageCatalog::load(&fixture.0);
        assert!(catalog.actions.is_empty());
        assert_eq!(catalog.diagnostics.len(), 2);
        assert_eq!(
            catalog.resolve(&Fixture::call()).unwrap_err().code,
            "package_missing"
        );
    }

    #[test]
    fn rejects_escape_duplicate_actions_large_manifests_and_cloud_folders() {
        let fixture = Fixture::new();
        let root = fixture.package("sample", "example", "javascript");
        let original = fs::read(root.join("action.json")).unwrap();
        let mut manifest: Value = serde_json::from_slice(&original).unwrap();
        manifest["actions"][0]["entry"] = "../outside.mjs".into();
        fs::write(
            root.join("action.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(PackageCatalog::load(&fixture.0).actions.is_empty());
        manifest = serde_json::from_slice(&original).unwrap();
        let duplicate = manifest["actions"][0].clone();
        manifest["actions"].as_array_mut().unwrap().push(duplicate);
        fs::write(
            root.join("action.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(PackageCatalog::load(&fixture.0).actions.is_empty());
        fs::write(root.join("action.json"), vec![b' '; MAX_MANIFEST_BYTES + 1]).unwrap();
        assert!(PackageCatalog::load(&fixture.0).actions.is_empty());
        let cloud = fixture.0.join("OneDrive-test");
        fs::create_dir(&cloud).unwrap();
        let catalog = PackageCatalog::load(&cloud);
        assert!(catalog.actions.is_empty());
        assert!(catalog.diagnostics[0].contains("nonlocal_path"));
        let missing = PackageCatalog::load(&fixture.0.join("missing"));
        assert!(missing.actions.is_empty() && missing.diagnostics.is_empty());
    }

    #[test]
    fn manifest_versions_unknown_fields_and_action_limits_are_checked() {
        let fixture = Fixture::new();
        let root = fixture.package("sample", "example", "node");
        let mut manifest: PackageManifest =
            serde_json::from_slice(&fs::read(root.join("action.json")).unwrap()).unwrap();
        manifest.version = "1.2.3+build.4".into();
        validate_manifest(&manifest).unwrap();
        manifest.schema_version = 2;
        assert_eq!(
            validate_manifest(&manifest).unwrap_err().code,
            "incompatible_manifest"
        );
        manifest.schema_version = 1;
        let definition = manifest.actions[0].clone();
        manifest.actions = (0..MAX_ACTIONS_PER_PACKAGE)
            .map(|index| ActionDefinition {
                id: format!("action-{index}"),
                ..definition.clone()
            })
            .collect();
        validate_manifest(&manifest).unwrap();
        manifest.actions.push(definition);
        assert_eq!(
            validate_manifest(&manifest).unwrap_err().code,
            "invalid_manifest"
        );
        let mut raw = serde_json::to_value(&manifest).unwrap();
        raw["unrecognized"] = true.into();
        assert!(serde_json::from_value::<PackageManifest>(raw).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn links_in_package_entries_or_catalog_ancestors_are_skipped() {
        use std::os::unix::fs::{symlink, symlink as symlink_dir};
        let fixture = Fixture::new();
        let root = fixture.package("sample", "example", "node");
        let outside = fixture.0.join("outside.mjs");
        fs::write(&outside, "throw new Error('不可加载');").unwrap();
        fs::remove_file(root.join("main.mjs")).unwrap();
        symlink(&outside, root.join("main.mjs")).unwrap();
        assert!(PackageCatalog::load(&fixture.0).actions.is_empty());
        let link = fixture.0.join("linked");
        symlink_dir(&root, &link).unwrap();
        let nested = root.join("nested");
        fs::create_dir(&nested).unwrap();
        let catalog = PackageCatalog::load(&link.join("nested"));
        assert!(catalog.actions.is_empty());
        assert!(catalog
            .diagnostics
            .iter()
            .any(|message| message.contains("nonlocal_path")));
    }
}
