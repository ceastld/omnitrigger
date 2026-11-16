//! 核对复制来源的稳定 ID。创建时的选择另存一份，用于中断后按原命令发布暂存档。
use super::{read_copy_origin, write_json, Catalog, Entry};
use serde::{Deserialize, Serialize};
use std::path::Path;

const RECORD: &str = "backups/seed-source.json";

#[derive(Deserialize, Serialize)]
struct Source {
    profile_id: String,
    seed: String,
    source_id: String,
    requested_seed: String,
}

fn read(profile: &Path, entry: &Entry) -> Result<Option<Source>, String> {
    let path = profile.join(RECORD);
    crate::script_runtime::check_local_path(&path)?;
    let Some(saved) = read_copy_origin::<Source>(&path)? else {
        return Ok(None);
    };
    if saved.profile_id != entry.id || saved.seed != entry.seed || saved.source_id.is_empty() {
        return Err(format!(
            "{} 与测试档 {} 的种子不符，请核对备份。",
            path.display(),
            entry.alias
        ));
    }
    Ok(Some(saved))
}

pub(super) fn recorded_id(profile: &Path, entry: &Entry) -> Result<Option<String>, String> {
    Ok(read(profile, entry)?.map(|record| record.source_id))
}

pub(super) fn requested(profile: &Path, entry: &Entry, seed: &str) -> Result<bool, String> {
    Ok(read(profile, entry)?.is_some_and(|record| record.requested_seed == seed))
}

pub(super) fn resolve<'a>(
    catalog: &'a Catalog,
    previous_profile: Option<&Path>,
    entry: &Entry,
    key: &str,
) -> Result<&'a Entry, String> {
    if let Some(id) = previous_profile
        .map(|dir| recorded_id(dir, entry))
        .transpose()?
        .flatten()
    {
        return catalog
            .source_profile(&id)
            .filter(|source| source.id == id)
            .ok_or_else(|| {
                format!(
                    "找不到测试档 {} 的原来源档 {id}，请从备份恢复该 ID。",
                    entry.alias
                )
            });
    }
    // 只有总登记可用时，种子里的 ID 仍能重建副本。不能把同形的别名当成丢失的 ID。
    let source = catalog
        .source_profile(key)
        .ok_or_else(|| format!("找不到配置档 {key}；可用 --profile-list 查看。"))?;
    if previous_profile.is_some() && super::generated_profile_id(key) && source.id != key {
        return Err(format!("原来源 ID {key} 现在只匹配到另一个档的别名，请恢复原 ID；要使用当前别名可从它新建副本。"));
    }
    Ok(source)
}

pub(super) fn save(
    target: &Path,
    entry: &Entry,
    source: &Entry,
    requested_seed: String,
) -> Result<(), String> {
    std::fs::create_dir_all(target.join("backups")).map_err(|e| e.to_string())?;
    write_json(
        &target.join(RECORD),
        &Source {
            profile_id: entry.id.clone(),
            seed: entry.seed.clone(),
            source_id: source.id.clone(),
            requested_seed,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::{
        catalog_path, copied_profile_sources, create_profile, profile_dir, read_catalog,
        reset_profile, tests::temp_root, Seed,
    };
    use std::fs;

    #[test]
    fn reused_alias_cannot_redirect_a_reset_or_inherited_runtime() {
        let root = temp_root("seed-source-id");
        let source = create_profile(&root, "work", &Seed::Empty, None).unwrap();
        let directory = profile_dir(&root, &source.id);
        fs::write(directory.join("wheels.json"), "original").unwrap();
        let copy = create_profile(&root, "review", &Seed::Copy("work".into()), None).unwrap();
        let child = profile_dir(&root, &copy.id);
        let config = child.join("bindings.json");
        fs::write(child.join("wheels.json"), "my edit").unwrap();
        assert_eq!(copy.seed, format!("copy:{}", source.id));
        let archived = root.join("archive");
        fs::rename(&directory, &archived).unwrap();
        fs::remove_file(catalog_path(&root)).unwrap();
        let other = create_profile(&root, "work", &Seed::Empty, None).unwrap();
        let lookalike = create_profile(&root, &source.id, &Seed::Empty, None).unwrap();
        assert!(copied_profile_sources(&config).is_empty());
        assert!(reset_profile(&root, &copy.alias).is_err());
        assert_eq!(fs::read(child.join("wheels.json")).unwrap(), b"my edit");
        fs::remove_file(child.join(RECORD)).unwrap();
        assert!(copied_profile_sources(&config).is_empty());
        assert!(reset_profile(&root, &copy.alias).is_err());

        let mut catalog = read_catalog(&root).unwrap();
        catalog.profiles.retain(|entry| entry.id != lookalike.id);
        let mut recovered = source;
        recovered.alias = "renamed".into();
        fs::rename(archived, &directory).unwrap();
        write_json(&directory.join("profile.json"), &recovered).unwrap();
        catalog.profiles.push(recovered);
        write_json(&catalog_path(&root), &catalog).unwrap();
        assert_eq!(
            copied_profile_sources(&config),
            [directory.canonicalize().unwrap()]
        );
        reset_profile(&root, &copy.alias).unwrap();
        assert_eq!(fs::read(child.join("wheels.json")).unwrap(), b"original");
        assert_ne!(directory, profile_dir(&root, &other.id));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retry_of_staged_alias_copy_uses_original_selection_even_after_source_disappears() {
        let root = temp_root("seed-source-retry");
        let parent = create_profile(&root, "work", &Seed::Empty, None).unwrap();
        let seed = Seed::Copy("work".into());
        let copy = create_profile(&root, "review", &seed, None).unwrap();
        let dir = profile_dir(&root, &copy.id);
        fs::write(dir.join("wheels.json"), "prepared").unwrap();
        fs::rename(&dir, dir.with_extension("creating")).unwrap();
        let mut catalog = read_catalog(&root).unwrap();
        catalog.profiles.retain(|entry| entry.id != parent.id);
        write_json(&catalog_path(&root), &catalog).unwrap();
        assert!(create_profile(&root, "review", &Seed::Empty, None).is_err());
        assert_eq!(create_profile(&root, "review", &seed, None).unwrap(), copy);
        assert_eq!(fs::read(dir.join("wheels.json")).unwrap(), b"prepared");
        fs::remove_dir_all(root).unwrap();
    }
}
