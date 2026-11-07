//! 既有升级叶的有界文件事务。旧文件逐个移动；不移动含旧guard的父目录。
use super::{bounded_read, fault, local, uncertain, write_new};
use crate::legacy_local_source::{Manifest, Record, MAX_FILES, MAX_FILE_BYTES, MAX_TOTAL_BYTES};
use omnitrigger_actions::ActionError;
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

const MAX_ENTRIES: usize = 2048; // 与既有LocalLegacySource闭集枚举限额一致。
const MAX_PLAN: usize = 2 * 1024 * 1024;
const MAX_RECORD: usize = 1024;
const MAX_JOURNAL: usize = 8 * 1024 * 1024;
#[derive(Serialize)]
pub(super) struct Snapshot {
    pub(super) digest: String,
    pub(super) files: Vec<Record>,
    pub(super) dirs: Vec<String>,
}

pub(super) fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
#[cfg(test)]
pub(super) fn manifest_digest(root: &Path) -> Result<String, ActionError> {
    Ok(digest(&bounded_read(
        &root.join("manifest.json"),
        256 * 1024,
    )?))
}
pub(super) fn hash_file(path: &Path, expected: &Record) -> Result<(), ActionError> {
    local(path)?;
    let mut file =
        File::open(path).map_err(|e| fault("legacy_upgrade_package_changed", e.to_string()))?;
    let metadata = file
        .metadata()
        .map_err(|e| fault("legacy_upgrade_package_changed", e.to_string()))?;
    if !metadata.is_file() || metadata.len() != expected.bytes {
        return Err(fault(
            "legacy_upgrade_package_changed",
            format!("文件类型或长度已变：{}", expected.path),
        ));
    }
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|e| fault("legacy_upgrade_package_changed", e.to_string()))?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > expected.bytes {
            return Err(fault("legacy_upgrade_package_changed", "读取时文件变长。"));
        }
        hash.update(&buffer[..count]);
    }
    if total != expected.bytes
        || !format!("{:x}", hash.finalize()).eq_ignore_ascii_case(&expected.sha256)
    {
        return Err(fault(
            "legacy_upgrade_package_changed",
            format!("文件摘要已变：{}", expected.path),
        ));
    }
    Ok(())
}
fn inventory(
    root: &Path,
    expected_files: &BTreeSet<String>,
    expected_dirs: &BTreeSet<String>,
) -> Result<(), ActionError> {
    inventory_partial(root, expected_files, expected_dirs, true)
}
pub(super) fn inventory_partial(
    root: &Path,
    expected_files: &BTreeSet<String>,
    expected_dirs: &BTreeSet<String>,
    all_dirs: bool,
) -> Result<(), ActionError> {
    local(root)?;
    if !exists(root)? && !all_dirs && expected_files.is_empty() {
        return Ok(());
    }
    let mut pending = vec![root.to_path_buf()];
    let mut files = BTreeSet::new();
    let mut dirs = BTreeSet::new();
    let mut entries = 0;
    while let Some(folder) = pending.pop() {
        local(&folder)?;
        for entry in fs::read_dir(folder)
            .map_err(|e| fault("legacy_upgrade_inventory_changed", e.to_string()))?
        {
            let path = entry
                .map_err(|e| fault("legacy_upgrade_inventory_changed", e.to_string()))?
                .path();
            entries += 1;
            if entries > MAX_ENTRIES {
                return Err(fault(
                    "legacy_upgrade_inventory_limit",
                    "事务目录项目超限。",
                ));
            }
            local(&path)?;
            let metadata = fs::symlink_metadata(&path)
                .map_err(|e| fault("legacy_upgrade_inventory_changed", e.to_string()))?;
            let name = path
                .strip_prefix(root)
                .map_err(|_| fault("legacy_upgrade_path_invalid", "事务路径越界。"))?
                .to_string_lossy()
                .replace('\\', "/");
            if metadata.is_dir() {
                if !expected_dirs.contains(&name) || !dirs.insert(name) {
                    return Err(fault(
                        "legacy_upgrade_inventory_changed",
                        "存在计划外目录。",
                    ));
                }
                pending.push(path);
            } else if !metadata.is_file() || !expected_files.contains(&name) || !files.insert(name)
            {
                return Err(fault(
                    "legacy_upgrade_inventory_changed",
                    "存在清单外文件。",
                ));
            }
        }
    }
    if files != *expected_files || (all_dirs && dirs != *expected_dirs) {
        return Err(fault(
            "legacy_upgrade_inventory_changed",
            "事务闭集不完整。",
        ));
    }
    Ok(())
}
pub(super) fn snapshot(root: &Path, expected_digest: &str) -> Result<Snapshot, ActionError> {
    let bytes = bounded_read(&root.join("manifest.json"), 256 * 1024)?;
    let result = descriptor(&bytes, expected_digest)?;
    let names = result.files.iter().map(|r| r.path.clone()).collect();
    let dirs = result.dirs.iter().cloned().collect();
    inventory(root, &names, &dirs)?;
    for row in &result.files {
        hash_file(&root.join(&row.path), row)?;
    }
    Ok(result)
}
pub(super) fn descriptor(bytes: &[u8], expected_digest: &str) -> Result<Snapshot, ActionError> {
    let actual = digest(bytes);
    if !actual.eq_ignore_ascii_case(expected_digest) {
        return Err(fault(
            "legacy_upgrade_manifest_changed",
            "清单与已核验索引pin不符。",
        ));
    }
    // 复用既有清单类型、路径函数及数量/体积限额，不另解读包清单格式。
    let manifest: Manifest = serde_json::from_slice(bytes)
        .map_err(|e| fault("legacy_upgrade_manifest_invalid", e.to_string()))?;
    if manifest.schema_version != 1
        || manifest.architecture != "windows-x64"
        || manifest.package_version.is_empty()
        || manifest.package_version.len() > 64
        || manifest.files.is_empty()
        || manifest.files.len() > MAX_FILES
    {
        return Err(fault(
            "legacy_upgrade_manifest_invalid",
            "清单头或文件数量无效。",
        ));
    }
    let mut names = BTreeSet::new();
    let mut folded = BTreeSet::new();
    let mut dirs = BTreeSet::new();
    let mut total = 0u64;
    for row in &manifest.files {
        if !crate::script_runtime::relative(&row.path)
            || row.path.eq_ignore_ascii_case("manifest.json")
            || row.bytes > MAX_FILE_BYTES
            || row.sha256.len() != 64
            || !row.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            || !folded.insert(row.path.to_ascii_lowercase())
            || !names.insert(row.path.clone())
        {
            return Err(fault(
                "legacy_upgrade_manifest_invalid",
                "清单路径、摘要或大小无效。",
            ));
        }
        total = total
            .checked_add(row.bytes)
            .ok_or_else(|| fault("legacy_upgrade_manifest_invalid", "文件总量溢出。"))?;
        if total > MAX_TOTAL_BYTES {
            return Err(fault("legacy_upgrade_manifest_invalid", "文件总量超限。"));
        }
        let mut parent = Path::new(&row.path).parent();
        while let Some(path) = parent.filter(|p| !p.as_os_str().is_empty()) {
            dirs.insert(path.to_string_lossy().replace('\\', "/"));
            parent = path.parent();
        }
    }
    let mut files = manifest.files;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files.push(Record {
        path: "manifest.json".into(),
        bytes: bytes.len() as u64,
        sha256: actual.clone(),
    }); // 最后移动清单。
    Ok(Snapshot {
        digest: actual,
        files,
        dirs: dirs.into_iter().collect(),
    })
}
pub(super) fn exists(path: &Path) -> Result<bool, ActionError> {
    local(path)?;
    path.try_exists()
        .map_err(|e| fault("legacy_upgrade_io", e.to_string()))
}
pub(super) fn move_no_replace(from: &Path, to: &Path, operation: &str) -> Result<(), ActionError> {
    local(from)?;
    local(to)?;
    omnitrigger_windows::move_owned_path_no_replace(from, to)
        .map_err(|e| fault("legacy_upgrade_switch_failed", format!("{operation}: {e}")))
}
pub(super) struct Journal {
    file: File,
    records: usize,
    bytes: usize,
    max_records: usize,
}
impl Journal {
    pub(super) fn create(path: &Path, count: usize) -> Result<Self, ActionError> {
        local(path)?;
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| fault("legacy_upgrade_journal_failed", e.to_string()))?;
        Ok(Self {
            file,
            records: 0,
            bytes: 0,
            max_records: 4 * count + 32,
        })
    }
    pub(super) fn record(
        &mut self,
        op: &str,
        phase: &str,
        relative: Option<&str>,
    ) -> Result<(), ActionError> {
        let mut line = serde_json::to_vec(
            &json!({"seq":self.records,"op":op,"phase":phase,"relative":relative}),
        )
        .map_err(|e| fault("legacy_upgrade_journal_failed", e.to_string()))?;
        line.push(b'\n');
        if line.len() > MAX_RECORD
            || self.records >= self.max_records
            || self.bytes + line.len() > MAX_JOURNAL
        {
            return Err(fault("legacy_upgrade_journal_limit", "事务日志超限。"));
        }
        self.file
            .write_all(&line)
            .and_then(|_| self.file.sync_all())
            .map_err(|e| fault("legacy_upgrade_journal_failed", e.to_string()))?;
        self.records += 1;
        self.bytes += line.len();
        Ok(())
    }
}
pub(super) fn create_dirs(root: &Path, dirs: &[String]) -> Result<(), ActionError> {
    local(root)?;
    fs::create_dir_all(root).map_err(|e| fault("legacy_upgrade_io", e.to_string()))?;
    for dir in dirs {
        let path = root.join(dir);
        local(&path)?;
        fs::create_dir_all(path).map_err(|e| fault("legacy_upgrade_io", e.to_string()))?;
    }
    Ok(())
}
fn reverse_files(
    old: &Snapshot,
    final_path: &Path,
    backup: &Path,
    completed: &[usize],
    journal: &mut Journal,
) -> Result<(), ActionError> {
    create_dirs(final_path, &old.dirs)?;
    for &index in completed.iter().rev() {
        let row = &old.files[index];
        let source = backup.join(&row.path);
        let target = final_path.join(&row.path);
        hash_file(&source, row)?;
        journal.record("restore_old_file", "intent", Some(&row.path))?;
        move_no_replace(&source, &target, &format!("restore_old_file[{}]", row.path))?;
        journal.record("restore_old_file", "done", Some(&row.path))?;
    }
    snapshot(final_path, &old.digest)?;
    Ok(())
}
pub(super) fn switch_and_publish(
    final_path: &Path,
    new_path: &Path,
    backup: &Path,
    expected: (&str, &str),
    publish: impl FnOnce() -> Result<(), ActionError>,
) -> Result<(), ActionError> {
    switch_with_probe(final_path, new_path, backup, expected, publish, |_, _| {
        Ok(())
    })
}
// 唯一窄测试接缝：测试可在某一步制造真实目标碰撞；生产为no-op，无全局hook/新队列。
pub(super) fn switch_with_probe(
    final_path: &Path,
    new_path: &Path,
    backup: &Path,
    expected: (&str, &str),
    publish: impl FnOnce() -> Result<(), ActionError>,
    mut before_old_file: impl FnMut(usize, &Path) -> Result<(), ActionError>,
) -> Result<(), ActionError> {
    local(final_path)?;
    local(new_path)?;
    local(backup)?;
    if exists(backup)? {
        return Err(fault(
            "legacy_upgrade_backup_exists",
            "旧包备份位置已存在。",
        ));
    }
    if !final_path.is_dir() || !new_path.is_dir() {
        return Err(fault(
            "legacy_upgrade_switch_failed",
            "旧包或新暂存包不存在。",
        ));
    }
    if final_path.starts_with(new_path)
        || new_path.starts_with(final_path)
        || backup.starts_with(final_path)
        || final_path.starts_with(backup)
        || backup.starts_with(new_path)
        || new_path.starts_with(backup)
    {
        return Err(fault("legacy_upgrade_path_invalid", "事务根不能重叠。"));
    }
    let old = snapshot(final_path, expected.0)?;
    let new = snapshot(new_path, expected.1)?;
    let stage = backup
        .parent()
        .ok_or_else(|| fault("legacy_upgrade_path_invalid", "备份没有本stage父目录。"))?;
    let plan_path = stage.join("swap-plan.json");
    let log_path = stage.join("swap-journal.jsonl");
    let plan = serde_json::to_vec(
        &json!({"schema_version":1,"scope":"legacy_upgrade_file_swap","final_path":final_path,
        "new_path":new_path,"backup":backup,"old":&old,"new":&new}),
    )
    .map_err(|e| fault("legacy_upgrade_journal_failed", e.to_string()))?;
    if plan.len() > MAX_PLAN {
        return Err(fault("legacy_upgrade_journal_limit", "事务计划超限。"));
    }
    write_new(&plan_path, &plan)?;
    let mut journal = Journal::create(&log_path, old.files.len())?;
    fs::create_dir(backup).map_err(|e| fault("legacy_upgrade_io", e.to_string()))?;
    create_dirs(backup, &old.dirs)?;
    let mut completed = Vec::with_capacity(old.files.len());
    let mut new_in_final = false;
    let mut index_published = false;
    let result: Result<(), ActionError> = (|| {
        for (index, row) in old.files.iter().enumerate() {
            let from = final_path.join(&row.path);
            let to = backup.join(&row.path);
            hash_file(&from, row)?;
            journal.record("old_file_to_backup", "intent", Some(&row.path))?;
            before_old_file(index, &to)?;
            move_no_replace(&from, &to, &format!("old_file_to_backup[{}]", row.path))?;
            completed.push(index); // 必须先登记内存完成项；done写盘失败也能反向处理。
            journal.record("old_file_to_backup", "done", Some(&row.path))?;
        }
        snapshot(backup, &old.digest)?;
        let dirs: BTreeSet<_> = old.dirs.iter().cloned().collect();
        inventory(final_path, &BTreeSet::new(), &dirs)?;
        journal.record("remove_empty_old_dirs", "intent", None)?;
        for dir in old.dirs.iter().rev() {
            let path = final_path.join(dir);
            local(&path)?;
            fs::remove_dir(path).map_err(|e| {
                fault(
                    "legacy_upgrade_switch_failed",
                    format!("remove_empty_old_dir: {e}"),
                )
            })?;
        }
        fs::remove_dir(final_path).map_err(|e| {
            fault(
                "legacy_upgrade_switch_failed",
                format!("remove_empty_old_root: {e}"),
            )
        })?;
        journal.record("remove_empty_old_dirs", "done", None)?;
        journal.record("new_to_final", "intent", None)?;
        move_no_replace(new_path, final_path, "new_to_final")?;
        new_in_final = true;
        journal.record("new_to_final", "done", None)?;
        snapshot(final_path, &new.digest)?;
        journal.record("index_publish", "intent", None)?;
        publish()?;
        index_published = true;
        journal.record("index_publish", "done", None)?;
        Ok(())
    })();
    if let Err(error) = result {
        if index_published {
            return Err(uncertain(
                "legacy_upgrade_committed_but_blocked",
                format!("索引已发布，日志收尾未确认，不回退：{}", error.message),
            ));
        }
        let rollback: Result<(), ActionError> = (|| {
            if new_in_final {
                snapshot(final_path, &new.digest)?; // 回退也不移动后来出现的未知/已改写新包内容。
                journal.record("new_to_stage", "intent", None)?;
                move_no_replace(final_path, new_path, "new_to_stage")?;
                journal.record("new_to_stage", "done", None)?;
            }
            reverse_files(&old, final_path, backup, &completed, &mut journal)?;
            journal.record("rollback", "done", None)?;
            Ok(())
        })();
        return match rollback {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(uncertain(
                "legacy_upgrade_recovery_required",
                format!(
                    "原错误={}；反向恢复未确认：{}",
                    error.code, rollback_error.message
                ),
            )),
        };
    }
    Ok(())
}
