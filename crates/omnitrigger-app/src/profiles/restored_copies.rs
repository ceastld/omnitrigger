//! 读取当前配置直接恢复出的测试档。原版本清理后，副本仍可按登记身份打开。
use super::{
    backup_source, catalog_path, open_test_profile, profile_dir, read_catalog, relative_age,
    saved_profiles::prepare_test_open, Catalog, Entry, Seed, DEFAULT_ID,
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
        prepare_test_open(&self.root, &self.id, |catalog, entry| {
            let alias = source_alias(catalog, &self.source_id)?;
            Ok(entry.created_ms == self.created_ms
                && copy_for_source(&self.root, entry, &self.source_id, alias)?.is_some())
        })
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

fn copy_for_source(
    root: &Path,
    entry: &Entry,
    id: &str,
    alias: &str,
) -> Result<Option<RestoredCopy>, String> {
    if entry.kind != "test" || !super::generated_profile_id(&entry.id) {
        return Ok(None);
    }
    let Ok(Seed::Backup {
        source,
        bindings,
        wheels,
    }) = Seed::parse(&entry.seed)
    else {
        return Ok(None);
    };
    let same = if id == DEFAULT_ID || source == DEFAULT_ID {
        source == id
    } else {
        super::seed_source::recorded_id(&profile_dir(root, &entry.id), entry)?.map_or_else(
            || source == id || (!super::generated_profile_id(&source) && source == alias),
            |recorded| recorded == id,
        )
    };
    Ok(same.then(|| RestoredCopy::from_entry(root, id, entry, bindings, wheels)))
}

fn copies_in(
    root: &Path,
    source_id: &str,
    errors: &mut Vec<String>,
) -> Result<Vec<RestoredCopy>, String> {
    crate::script_runtime::check_local_path(&catalog_path(root))?;
    let catalog = read_catalog(root)?;
    let alias = source_alias(&catalog, source_id)?;
    Ok(catalog
        .profiles
        .iter()
        .filter_map(
            |entry| match copy_for_source(root, entry, source_id, alias) {
                Ok(result) => result,
                Err(error) => {
                    errors.push(error);
                    None
                }
            },
        )
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
        match copies_in(&directory, id, &mut errors) {
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
    fn resetting_an_old_backup_seed_keeps_it_with_its_source_after_alias_reuse() {
        let root = temp_root("legacy-backup-source");
        let mut source = create_profile(&root, "notes", &Seed::Empty, None).unwrap();
        let source_dir = profile_dir(&root, &source.id);
        add_history(&source_dir);
        let seed = Seed::parse("backup:notes:1:current").unwrap();
        let mut copy = create_profile(&root, "restored", &seed, None).unwrap();
        let dir = profile_dir(&root, &copy.id);
        // 构造旧版实际保存的登记：只有别名，没有独立的来源 ID 记录。
        copy.seed = seed.record();
        fs::remove_file(dir.join("backups/seed-source.json")).unwrap();
        write_json(&dir.join("profile.json"), &copy).unwrap();
        let mut catalog = read_catalog(&root).unwrap();
        *catalog
            .profiles
            .iter_mut()
            .find(|entry| entry.id == copy.id)
            .unwrap() = copy.clone();
        write_json(&catalog_path(&root), &catalog).unwrap();
        crate::profiles::reset_profile(&root, &copy.id).unwrap();
        source.alias = "recovered".into();
        write_json(&source_dir.join("profile.json"), &source).unwrap();
        fs::remove_file(catalog_path(&root)).unwrap();
        let replacement = create_profile(&root, "notes", &Seed::Empty, None).unwrap();
        let other = profile_dir(&root, &replacement.id);
        assert!(saved_backup_copies(&other.join("bindings.json"))
            .unwrap()
            .items
            .is_empty());
        let mine = saved_backup_copies(&source_dir.join("bindings.json")).unwrap();
        assert_eq!(mine.items.len(), 1);
        mine.items[0].prepare_to_open().unwrap();
        crate::profiles::reset_profile(&root, &copy.id).unwrap();
        assert_eq!(
            fs::read(dir.join("bindings.json")).unwrap(),
            br#"{"version":24,"bindings":[]}"#
        );
        fs::remove_dir_all(root).unwrap();
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
