//! 从编辑备份恢复时记录实际复制的字节。来源目录清理后，编号可能再次使用。
use super::{write_json, DataCopy};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, io, path::Path};

const RECORD: &str = "backups/restored-revisions.json";

#[derive(Serialize, Deserialize, PartialEq, Eq)]
struct Revision {
    number: u64,
    sha256: String,
}

type Revisions = BTreeMap<String, Revision>;

fn read_previous(profile: &Path) -> Result<Option<Revisions>, String> {
    let path = profile.join(RECORD);
    crate::script_runtime::check_local_path(&path)?;
    let read_error = |e: io::Error| format!("无法读取备份校验记录 {}：{e}", path.display());
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.is_file() && crate::local_files::is_plain(&meta) => {}
        Ok(_) => {
            return Err(format!(
                "备份校验记录必须是普通本机文件：{}",
                path.display()
            ))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(read_error(e)),
    }
    let bytes = fs::read(&path).map_err(read_error)?;
    crate::local_files::parse_config_json_bytes(&bytes)
        .map(Some)
        .map_err(|e| format!("无法解析备份校验记录 {}：{e}", path.display()))
}

pub(super) fn restore(
    source: &Path,
    staging: &Path,
    previous_profile: Option<&Path>,
    bindings: Option<u64>,
    wheels: Option<u64>,
) -> Result<(), String> {
    let previous = previous_profile.map(read_previous).transpose()?.flatten();
    let mut revisions = Revisions::new();
    for (name, label, number) in [
        ("bindings.json", "热键", bindings),
        ("wheels.json", "轮盘", wheels),
    ] {
        let Some(number) = number else { continue };
        let backup =
            crate::settings::edit_backup_file(&source.join("bindings.json"), name, number)?;
        let destination = staging.join(name);
        DataCopy::Profile
            .copy_file(&backup, &destination)
            .map_err(|e| format!("复制 {name} 的第 {number} 份备份失败：{e}"))?;
        let mut hash = Sha256::new();
        fs::File::open(&destination)
            .and_then(|mut file| io::copy(&mut file, &mut hash))
            .map_err(|e| format!("校验 {label} 备份失败：{e}"))?;
        let revision = Revision {
            number,
            sha256: format!("{:x}", hash.finalize()),
        };
        if previous
            .as_ref()
            .is_some_and(|record| record.get(name) != Some(&revision))
        {
            return Err(format!("{label}的第 {number} 份备份内容已变化，原测试档的编辑已保留。要试用这份备份，请新建测试副本。"));
        }
        revisions.insert(name.into(), revision);
    }
    if previous.is_some_and(|record| record.len() != revisions.len()) {
        return Err("备份校验记录与配置档登记的版本不一致，原测试档已保留。".into());
    }
    // 记录与暂存目录一起发布。复制、配置校验或替换失败时，旧档和旧记录仍在一起。
    fs::create_dir_all(staging.join("backups"))
        .map_err(|e| format!("创建备份校验目录失败：{e}"))?;
    write_json(&staging.join(RECORD), &revisions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::{create_profile, profile_dir, reset_profile, tests::temp_root, Seed};
    use crate::settings::{edit_backup_file, save_editor_file};

    const ORIGINAL: &str = include_str!("../../../../examples/literal-text.json");
    const OTHER: &str = include_str!("../../../../examples/application-scopes.json");
    const WHEELS: &str = r#"{"version":1,"wheels":[]}"#;

    #[test]
    fn reused_revision_numbers_cannot_replace_test_edits() {
        let root = temp_root("reused-backup-number");
        let bindings = root.join("bindings.json");
        let wheels = root.join("wheels.json");
        save_editor_file(&bindings, Some(ORIGINAL), OTHER.as_bytes()).unwrap();
        save_editor_file(&wheels, Some(WHEELS), format!("{WHEELS}\n").as_bytes()).unwrap();
        let seed = Seed::parse("backup:default:1:1").unwrap();
        let entry = create_profile(&root, "trial", &seed, None).unwrap();
        let profile = profile_dir(&root, &entry.id);
        let test_bindings = profile.join("bindings.json");
        fs::write(&test_bindings, b"test edit").unwrap();
        let record = fs::read(profile.join(RECORD)).unwrap();
        let hotkey_backup = edit_backup_file(&bindings, "bindings.json", 1).unwrap();
        let wheel_backup = edit_backup_file(&bindings, "wheels.json", 1).unwrap();
        // 清掉来源的第一份后，再次用正常保存入口写入，编号会从 1 重新开始。
        fs::remove_file(&hotkey_backup).unwrap();
        save_editor_file(&bindings, Some(OTHER), ORIGINAL.as_bytes()).unwrap();
        let assert_refused = |expected: &str| {
            let error = reset_profile(&root, "trial").unwrap_err().response();
            assert!(error.contains(expected), "{error}");
            assert_eq!(fs::read(&test_bindings).unwrap(), b"test edit");
            assert_eq!(fs::read(profile.join(RECORD)).unwrap(), record);
            assert_eq!(
                fs::read_dir(root.join("backups/profile-reset"))
                    .unwrap()
                    .count(),
                0
            );
        };
        assert_refused("热键的第 1 份备份内容已变化");
        fs::write(&hotkey_backup, ORIGINAL).unwrap();
        fs::remove_file(&wheel_backup).unwrap();
        save_editor_file(&wheels, Some(&format!("{WHEELS}\n")), WHEELS.as_bytes()).unwrap();
        assert_refused("轮盘的第 1 份备份内容已变化");
        fs::write(&wheel_backup, WHEELS).unwrap();
        let backup = reset_profile(&root, "trial").unwrap().backup;
        assert_eq!(
            fs::read(backup.unwrap().join("bindings.json")).unwrap(),
            b"test edit"
        );
        assert_eq!(fs::read_to_string(&test_bindings).unwrap(), ORIGINAL);
        assert_eq!(
            fs::read_to_string(profile.join("wheels.json")).unwrap(),
            WHEELS
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_reset_records_only_numbered_revisions_and_preserves_a_damaged_record() {
        let root = temp_root("legacy-backup-record");
        let config = root.join("bindings.json");
        save_editor_file(&config, Some(ORIGINAL), OTHER.as_bytes()).unwrap();
        let seed = Seed::parse("backup:default:1:current").unwrap();
        let entry = create_profile(&root, "trial", &seed, None).unwrap();
        let profile = profile_dir(&root, &entry.id);
        let record = profile.join(RECORD);
        fs::remove_file(&record).unwrap();
        fs::write(root.join("wheels.json"), b"broken").unwrap();
        fs::write(profile.join("bindings.json"), b"test edit").unwrap();
        assert!(reset_profile(&root, "trial").is_err());
        assert!(!record.exists());
        assert_eq!(
            fs::read(profile.join("bindings.json")).unwrap(),
            b"test edit"
        );
        fs::write(root.join("wheels.json"), WHEELS).unwrap();
        reset_profile(&root, "trial").unwrap();
        let recorded = read_previous(&profile).unwrap().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded["bindings.json"].number, 1);
        let newer = format!("{WHEELS}\n");
        fs::write(root.join("wheels.json"), &newer).unwrap();
        reset_profile(&root, "trial").unwrap();
        assert_eq!(
            fs::read_to_string(profile.join("wheels.json")).unwrap(),
            newer
        );
        fs::write(&record, b"broken").unwrap();
        fs::write(profile.join("bindings.json"), b"edited").unwrap();
        assert!(reset_profile(&root, "trial")
            .unwrap_err()
            .response()
            .contains("无法解析备份校验记录"));
        assert_eq!(fs::read(&record).unwrap(), b"broken");
        assert_eq!(fs::read(profile.join("bindings.json")).unwrap(), b"edited");
        fs::remove_dir_all(root).unwrap();
    }
}
