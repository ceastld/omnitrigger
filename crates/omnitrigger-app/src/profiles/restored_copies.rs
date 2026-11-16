//! 读取当前配置直接恢复出的测试档。原版本清理后，副本仍可按登记身份打开。
use super::{
    backup_source, catalog_path, lock_catalog, open_test_profile, profile_dir, read_catalog,
    relative_age, source_dir, Catalog, Entry, Seed, DEFAULT_ID,
};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub(crate) struct RestoredCopy {
    root: PathBuf,
    source_id: String,
    id: String,
    pub(crate) alias: String,
    pub(crate) name: String,
    created_ms: u64,
    bindings: Option<u64>,
    wheels: Option<u64>,
}

impl RestoredCopy {
    pub(super) fn from_entry(
        root: &Path,
        source_id: &str,
        entry: &Entry,
        bindings: Option<u64>,
        wheels: Option<u64>,
    ) -> Self {
        Self {
            root: root.into(),
            source_id: source_id.into(),
            id: entry.id.clone(),
            alias: entry.alias.clone(),
            name: entry.name.clone(),
            created_ms: entry.created_ms,
            bindings,
            wheels,
        }
    }

    pub(crate) fn summary(&self) -> String {
        let version =
            |n: Option<u64>| n.map_or_else(|| "创建时版本".into(), |n| format!("第 {n} 份"));
        format!(
            "{}创建 · 热键 {} · 轮盘 {}",
            relative_age(self.created_ms),
            version(self.bindings),
            version(self.wheels)
        )
    }

    /// 列表可能已经过时。重读登记，以原 ID 唤回复用窗口；中断的目录交换沿用启动恢复。
    fn prepare_to_open(&self) -> Result<(), String> {
        if !crate::local_files::is_plain_directory(&self.root) {
            return Err("恢复副本的数据根不可用。请重新查看已有副本。".into());
        }
        crate::script_runtime::check_local_path(&catalog_path(&self.root))?;
        let _lock = lock_catalog(&self.root)?;
        let catalog = read_catalog(&self.root)?;
        let alias = source_alias(&catalog, &self.source_id)?;
        let entry = catalog
            .source_profile(&self.id)
            .filter(|entry| {
                entry.id == self.id
                    && entry.created_ms == self.created_ms
                    && revisions_for_source(entry, &self.source_id, alias).is_some()
            })
            .ok_or("这个恢复副本的登记已变更。请重新查看已有副本。")?;
        source_dir(&self.root, &catalog, &entry.id)?;
        Ok(())
    }

    pub(crate) fn open(&self) -> Result<(), String> {
        self.prepare_to_open()?;
        open_test_profile(&self.root, &self.id)
    }
}

fn source_alias<'a>(catalog: &'a Catalog, id: &str) -> Result<&'a str, String> {
    if id == DEFAULT_ID {
        return Ok(DEFAULT_ID);
    }
    catalog
        .source_profile(id)
        .filter(|parent| parent.kind == "test" && parent.id == id)
        .map(|parent| parent.alias.as_str())
        .ok_or_else(|| format!("找不到恢复副本的来源档 {id}。"))
}

fn revisions_for_source(
    entry: &Entry,
    id: &str,
    alias: &str,
) -> Option<(Option<u64>, Option<u64>)> {
    if entry.kind != "test" || !super::generated_profile_id(&entry.id) {
        return None;
    }
    let Seed::Backup {
        source,
        bindings,
        wheels,
    } = Seed::parse(&entry.seed).ok()?
    else {
        return None;
    };
    (source == id || source == alias).then_some((bindings, wheels))
}

fn copies_in(root: &Path, source_id: &str) -> Result<Vec<RestoredCopy>, String> {
    crate::script_runtime::check_local_path(&catalog_path(root))?;
    let catalog = read_catalog(root)?;
    let alias = source_alias(&catalog, source_id)?;
    Ok(catalog
        .profiles
        .iter()
        .filter_map(|entry| {
            let (bindings, wheels) = revisions_for_source(entry, source_id, alias)?;
            Some(RestoredCopy::from_entry(
                root, source_id, entry, bindings, wheels,
            ))
        })
        .collect())
}

pub(crate) struct SavedCopies {
    pub(crate) items: Vec<RestoredCopy>,
    pub(crate) notice: Option<String>,
}

pub(crate) fn saved_backup_copies(config: &Path) -> Result<SavedCopies, String> {
    let (root, source_id) = backup_source(config)?;
    let mut copies = Vec::new();
    let mut errors = Vec::new();
    // 以测试档目录作为 --data-root 建档时，backup:default 指向这一档本身。
    let nested = (source_id != DEFAULT_ID).then(|| (profile_dir(&root, &source_id), DEFAULT_ID));
    for (directory, id) in std::iter::once((root, source_id.as_str())).chain(nested) {
        match copies_in(&directory, id) {
            Ok(found) => copies.extend(found),
            Err(error) => errors.push(error),
        }
    }
    copies.sort_by(|a, b| {
        b.created_ms
            .cmp(&a.created_ms)
            .then_with(|| a.alias.cmp(&b.alias))
            .then_with(|| a.id.cmp(&b.id))
    });
    let notice = (!errors.is_empty()).then(|| errors.join("\n"));
    if copies.is_empty() {
        if let Some(error) = notice {
            return Err(error);
        }
    }
    Ok(SavedCopies {
        items: copies,
        notice,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::{create_backup_copy, create_profile, tests::temp_root, write_json};
    use std::fs;

    fn add_history(directory: &Path) {
        let history = directory.join("backups/edits/bindings.json");
        fs::create_dir_all(&history).unwrap();
        fs::write(
            history.join("edit-0000000000000001.json"),
            br#"{"version":24,"bindings":[]}"#,
        )
        .unwrap();
    }

    #[test]
    fn saved_copies_belong_to_the_open_source_and_survive_history_cleanup() {
        let root = temp_root("list-restored");
        add_history(&root);
        let daily = root.join("bindings.json");
        let topic = create_profile(&root, "notes", &Seed::Empty, None).unwrap();
        let notes_dir = profile_dir(&root, &topic.id);
        let notes = notes_dir.join("bindings.json");
        add_history(&notes_dir);
        let first = create_backup_copy(&daily, "one", Some(1), None).unwrap();
        create_backup_copy(&notes, "by-id", Some(1), None).unwrap();
        create_profile(
            &root,
            "by-alias",
            &Seed::parse("backup:notes:1:current").unwrap(),
            Some("轮盘实验"),
        )
        .unwrap();
        create_profile(
            &notes_dir,
            "inner",
            &Seed::parse("backup:default:1:current").unwrap(),
            None,
        )
        .unwrap();
        create_profile(&root, "plain", &Seed::Copy(DEFAULT_ID.into()), None).unwrap();
        fs::remove_dir_all(root.join("backups/edits")).unwrap();
        fs::remove_dir_all(notes_dir.join("backups/edits")).unwrap();
        let saved_file = profile_dir(&root, &first.id).join("bindings.json");
        fs::write(&saved_file, b"edits after restore").unwrap();
        let registry = fs::read(catalog_path(&root)).unwrap();
        let copies = saved_backup_copies(&daily).unwrap().items;
        assert_eq!(copies.len(), 1);
        assert_eq!(copies[0].id, first.id);
        copies[0].prepare_to_open().unwrap();
        assert_eq!(fs::read(saved_file).unwrap(), b"edits after restore");
        let owned = saved_backup_copies(&notes).unwrap().items;
        assert!(owned.windows(2).all(|p| p[0].created_ms >= p[1].created_ms));
        let mut aliases: Vec<_> = owned.iter().map(|v| v.alias.as_str()).collect();
        aliases.sort_unstable();
        assert_eq!(aliases, ["by-alias", "by-id", "inner"]);
        for item in owned {
            item.prepare_to_open().unwrap();
        }
        fs::write(catalog_path(&notes_dir), "damaged nested catalog").unwrap();
        let partial = saved_backup_copies(&notes).unwrap();
        assert_eq!(partial.items.len(), 2);
        assert!(partial.notice.unwrap().contains("profiles.json"));
        assert_eq!(
            fs::read(catalog_path(&notes_dir)).unwrap(),
            b"damaged nested catalog"
        );
        assert_eq!(fs::read(catalog_path(&root)).unwrap(), registry);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn only_open_recovers_an_interrupted_copy_and_a_stale_selection_is_rejected() {
        let root = temp_root("reopen-interrupted");
        add_history(&root);
        let config = root.join("bindings.json");
        let item = create_backup_copy(&config, "saved", Some(1), None).unwrap();
        let folder = profile_dir(&root, &item.id);
        let staging = folder.with_extension("creating");
        let registry = catalog_path(&root);
        let persisted = fs::read(&registry).unwrap();
        fs::remove_file(&registry).unwrap();
        assert_eq!(saved_backup_copies(&config).unwrap().items[0].id, item.id);
        assert!(!registry.exists());
        fs::write(&registry, &persisted).unwrap();
        fs::rename(&folder, &staging).unwrap();
        let list = saved_backup_copies(&config).unwrap().items;
        assert_eq!(list.len(), 1);
        assert!(!folder.exists() && staging.is_dir());
        assert_eq!(fs::read(&registry).unwrap(), persisted);
        list[0].prepare_to_open().unwrap();
        assert!(folder.is_dir() && !staging.exists());
        let mut catalog = read_catalog(&root).unwrap();
        catalog.profiles[0].seed = "copy:default".into();
        write_json(&registry, &catalog).unwrap();
        assert!(item.prepare_to_open().unwrap_err().contains("登记已变更"));
        assert!(saved_backup_copies(&config).unwrap().items.is_empty());
        fs::remove_dir_all(root).unwrap();
    }
}
