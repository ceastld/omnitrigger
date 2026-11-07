//! 开发CLI显式恢复叶；没有后台恢复、GUI或隐式个人配置入口。
use super::{
    bounded_read, capture, fault, files, local, profile, require_absent, uncertain, IndexGuard,
    MARKER,
};
use crate::legacy_local_source::Record;
use omnitrigger_actions::ActionError;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Inspect,
    RestoreOld,
    FinishNew,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Marker {
    schema_version: u32,
    scope: String,
    stage: PathBuf,
    old_manifest_sha256: String,
    new_manifest_sha256: String,
    source_root: PathBuf,
    destination_root: PathBuf,
    host_started: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    schema_version: u32,
    scope: String,
    final_path: PathBuf,
    new_path: PathBuf,
    backup: PathBuf,
    old: Value,
    new: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Row {
    seq: usize,
    op: String,
    phase: String,
    relative: Option<String>,
}
struct State {
    marker: Marker,
    marker_bytes: Vec<u8>,
    marker_path: PathBuf,
    final_path: PathBuf,
    new_path: PathBuf,
    backup: PathBuf,
    old: files::Snapshot,
    new: files::Snapshot,
    old_in_backup: Vec<usize>,
    new_in_final: bool,
    index_new: bool,
    journal_records: usize,
    journal_partial_tail: bool,
}
fn refused(message: impl Into<String>) -> ActionError {
    fault("legacy_upgrade_recovery_conflict", message)
}
fn valid_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
fn strict_path(path: &Path) -> Result<(), ActionError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(refused("恢复路径必须是无相对分量的本地绝对路径。"));
    }
    local(path)
}
fn json_read(path: &Path, limit: u64) -> Result<Value, ActionError> {
    serde_json::from_slice(&bounded_read(path, limit)?).map_err(|e| refused(e.to_string()))
}
fn manifest_at(root: &Path, expected: &str) -> Result<Option<Vec<u8>>, ActionError> {
    let path = root.join("manifest.json");
    if !files::exists(&path)? {
        return Ok(None);
    }
    let bytes = bounded_read(&path, 256 * 1024)?;
    Ok(files::digest(&bytes)
        .eq_ignore_ascii_case(expected)
        .then_some(bytes))
}
fn validate_journal(bytes: &[u8], old: &files::Snapshot) -> Result<(usize, bool), ActionError> {
    if bytes.len() > 8 * 1024 * 1024 {
        return Err(refused("原事务日志超限。"));
    }
    let names: BTreeSet<_> = old.files.iter().map(|r| r.path.as_str()).collect();
    let mut records = 0;
    let mut start = 0;
    for (at, &byte) in bytes.iter().enumerate() {
        if byte != b'\n' {
            continue;
        }
        let line = &bytes[start..at];
        if line.is_empty() || line.len() + 1 > 1024 || records >= 4 * old.files.len() + 32 {
            return Err(refused("原事务日志记录超限或为空。"));
        }
        let row: Row =
            serde_json::from_slice(line).map_err(|_| refused("原事务日志中间记录损坏。"))?;
        let per_file = matches!(row.op.as_str(), "old_file_to_backup" | "restore_old_file");
        let known = per_file
            || matches!(
                row.op.as_str(),
                "remove_empty_old_dirs"
                    | "new_to_final"
                    | "index_publish"
                    | "new_to_stage"
                    | "rollback"
            );
        if row.seq != records
            || !known
            || !matches!(row.phase.as_str(), "intent" | "done")
            || (per_file && !row.relative.as_deref().is_some_and(|s| names.contains(s)))
            || (!per_file && row.relative.is_some())
        {
            return Err(refused("原事务日志序号、操作或相对路径无效。"));
        }
        records += 1;
        start = at + 1;
    }
    // 只容许有界的最后一条未落完记录；保留原字节，不截断、不把它当成完成证据。
    if bytes.len() - start >= 1024 {
        return Err(refused("原事务日志尾记录超限。"));
    }
    Ok((records, start != bytes.len()))
}
impl State {
    fn load(config: &Path, marker_pin: &str) -> Result<Self, ActionError> {
        strict_path(config)?;
        if config.is_dir() || !valid_hash(marker_pin) {
            return Err(refused("需要明确配置文件与marker摘要。"));
        }
        let root = profile(config)?;
        let marker_path = root.join(MARKER);
        let marker_bytes = bounded_read(&marker_path, 64 * 1024)?;
        if !files::digest(&marker_bytes).eq_ignore_ascii_case(marker_pin) {
            return Err(refused("marker已变化，未恢复。"));
        }
        let marker: Marker =
            serde_json::from_slice(&marker_bytes).map_err(|e| refused(e.to_string()))?;
        let actions = crate::action_packages::root(config);
        let name = marker
            .stage
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("");
        let suffix = name.strip_prefix(".install-legacy-upgrade-").unwrap_or("");
        let parts: Vec<_> = suffix.split('-').collect();
        if marker.schema_version != 1
            || marker.scope != "legacy_upgrade_pending"
            || marker.host_started
            || !valid_hash(&marker.old_manifest_sha256)
            || !valid_hash(&marker.new_manifest_sha256)
            || marker
                .old_manifest_sha256
                .eq_ignore_ascii_case(&marker.new_manifest_sha256)
            || marker.stage.parent() != Some(actions.as_path())
            || parts.len() != 3
            || parts
                .iter()
                .any(|s| s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()))
            || marker.destination_root != root.join("legacy-host-data")
        {
            return Err(refused("不属于已支持的升级marker或固定事务目录。"));
        }
        for path in [&marker.stage, &marker.source_root, &marker.destination_root] {
            strict_path(path)?;
        }
        if marker.source_root.starts_with(&marker.destination_root)
            || marker.destination_root.starts_with(&marker.source_root)
            || marker.source_root.starts_with(&actions)
            || actions.starts_with(&marker.source_root)
        {
            return Err(refused("旧数据根不能与目标根或包目录重叠。"));
        }
        let final_path = actions.join("legacy-host-entry");
        let new_path = marker.stage.join("actions/legacy-host-entry");
        let backup = marker.stage.join("old-package");
        for path in [&final_path, &new_path, &backup] {
            strict_path(path)?;
        }
        // 普通事务只会使新完整包位于stage或final之一；不猜测脏/部分新包的状态。
        let staged_manifest = manifest_at(&new_path, &marker.new_manifest_sha256)?;
        let final_manifest = manifest_at(&final_path, &marker.new_manifest_sha256)?;
        if staged_manifest.is_some() == final_manifest.is_some() {
            return Err(refused("新包缺失或重复，不能自动选择。"));
        }
        let new_in_final = final_manifest.is_some();
        if new_in_final && files::exists(&new_path)? {
            return Err(refused("新包已在final，但stage仍存在其他包内容。"));
        }
        let new = files::snapshot(
            if new_in_final { &final_path } else { &new_path },
            &marker.new_manifest_sha256,
        )?;
        let a = if new_in_final {
            None
        } else {
            manifest_at(&final_path, &marker.old_manifest_sha256)?
        };
        let b = manifest_at(&backup, &marker.old_manifest_sha256)?;
        let old_bytes = match (a, b) {
            (Some(a), None) => a,
            (None, Some(b)) => b,
            _ => return Err(refused("旧清单缺失或重复。")),
        };
        let old = files::descriptor(&old_bytes, &marker.old_manifest_sha256)?;
        let mut at_final = BTreeSet::new();
        let mut at_backup = BTreeSet::new();
        let mut old_in_backup = Vec::new();
        for (i, row) in old.files.iter().enumerate() {
            let a = !new_in_final && files::exists(&final_path.join(&row.path))?;
            let b = files::exists(&backup.join(&row.path))?;
            if a == b {
                return Err(refused(format!("旧文件缺失或重复：{}", row.path)));
            }
            let parent = if b { &backup } else { &final_path };
            files::hash_file(&parent.join(&row.path), row)?;
            if b {
                at_backup.insert(row.path.clone());
                old_in_backup.push(i);
            } else {
                at_final.insert(row.path.clone());
            }
        }
        let dirs = old.dirs.iter().cloned().collect();
        if !new_in_final {
            files::inventory_partial(&final_path, &at_final, &dirs, false)?;
        }
        files::inventory_partial(&backup, &at_backup, &dirs, false)?;
        let plan_path = marker.stage.join("swap-plan.json");
        if files::exists(&plan_path)? {
            let plan: Plan = serde_json::from_slice(&bounded_read(&plan_path, 2 * 1024 * 1024)?)
                .map_err(|e| refused(e.to_string()))?;
            if plan.schema_version != 1
                || plan.scope != "legacy_upgrade_file_swap"
                || plan.final_path != final_path
                || plan.new_path != new_path
                || plan.backup != backup
                || plan.old != json!(&old)
                || plan.new != json!(&new)
            {
                return Err(refused("原事务计划与固定路径或已钉住清单不符。"));
            }
        } else if new_in_final || !old_in_backup.is_empty() {
            return Err(refused("换版已经开始但缺少原计划；未猜测分片。"));
        }
        let log_path = marker.stage.join("swap-journal.jsonl");
        let (journal_records, journal_partial_tail) = if files::exists(&log_path)? {
            validate_journal(&bounded_read(&log_path, 8 * 1024 * 1024)?, &old)?
        } else if new_in_final || !old_in_backup.is_empty() {
            return Err(refused("换版已开始但原日志缺失。"));
        } else {
            (0, false)
        };
        let plugins = crate::plugins::Plugins::load(config).map_err(|e| refused(e.to_string()))?;
        let pin = plugins.legacy_manifest_sha256();
        let index_new = pin == Some(marker.new_manifest_sha256.as_str());
        if !index_new && pin != Some(marker.old_manifest_sha256.as_str()) {
            return Err(refused("插件索引既不是原pin也不是目标pin。"));
        }
        if index_new && !new_in_final {
            return Err(refused("索引已发布但新包不在final。"));
        }
        Ok(Self {
            marker,
            marker_bytes,
            marker_path,
            final_path,
            new_path,
            backup,
            old,
            new,
            old_in_backup,
            new_in_final,
            index_new,
            journal_records,
            journal_partial_tail,
        })
    }
    fn report(&self, status: &str) -> Value {
        json!({"schema_version":1,"scope":"legacy_upgrade_explicit_recovery","status":status,
            "marker_sha256":files::digest(&self.marker_bytes),"new_package_in_final":self.new_in_final,
            "index_is_new":self.index_new,"old_files_in_backup":self.old_in_backup.len(),
            "old_files_total":self.old.files.len(),"journal_complete_records":self.journal_records,
            "journal_partial_tail_preserved":self.journal_partial_tail,"host_started":false,
            "data_root_rolled_back":false,"bindings_modified":false})
    }
    fn unchanged_marker(&self) -> Result<(), ActionError> {
        if bounded_read(&self.marker_path, 64 * 1024)? != self.marker_bytes {
            return Err(refused("marker已变化；保留恢复门。"));
        }
        Ok(())
    }
}

fn migration_ready(state: &State) -> Result<(), ActionError> {
    let destination = &state.marker.destination_root;
    if bounded_read(&destination.join("omnitrigger-legacy-host.marker"), 128)?
        != b"OmniTrigger.LegacyHost.v1"
    {
        return Err(refused("新数据根owner无效。"));
    }
    let database = destination.join("userdata.db");
    local(&database)?;
    let metadata = fs::symlink_metadata(&database).map_err(|e| refused(e.to_string()))?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(refused("新数据根数据库缺失或为空。"));
    }
    let mut receipts = Vec::new();
    for (count, entry) in fs::read_dir(destination)
        .map_err(|e| refused(e.to_string()))?
        .enumerate()
    {
        if count >= 4096 {
            return Err(refused("新数据根顶层项目超限。"));
        }
        let path = entry.map_err(|e| refused(e.to_string()))?.path();
        local(&path)?;
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if name.starts_with("omnitrigger-data-import-") && name.ends_with(".json") {
            receipts.push(path);
        }
    }
    if receipts.len() != 1 {
        return Err(refused("迁移回执缺失或不唯一。"));
    }
    let receipt = json_read(&receipts[0], 4 * 1024 * 1024)?;
    let source = &state.marker.source_root;
    if receipt["schema_version"] != 1
        || receipt["scope"] != "legacy_data_root_copy"
        || receipt["source"] != source.to_string_lossy().as_ref()
        || receipt["destination"] != destination.to_string_lossy().as_ref()
        || receipt["source_kind"] != "standalone-v1"
        || receipt["source_modified"] != false
        || receipt["old_root_retained"] != true
        || receipt["format_translation_performed"] != false
        || receipt["bindings_modified"] != false
        || receipt["external_asset_paths_rewritten"] != false
    {
        return Err(refused("迁移回执与已知根或保留约定不符。"));
    }
    let digest = receipt["source_database_sha256"]
        .as_str()
        .filter(|v| valid_hash(v))
        .ok_or_else(|| refused("迁移源摘要缺失。"))?;
    let source_db = source.join("userdata.db");
    local(&source_db)?;
    let size = fs::metadata(&source_db)
        .map_err(|e| refused(e.to_string()))?
        .len();
    if size == 0 || size > 1024 * 1024 * 1024 {
        return Err(refused("迁移源数据库大小无效。"));
    }
    files::hash_file(
        &source_db,
        &Record {
            path: "userdata.db".into(),
            bytes: size,
            sha256: digest.into(),
        },
    )?;
    // 不打开SQLite、不重做迁移，不声称此处重新验证了新根逻辑行/外部绝对路径。
    Ok(())
}
fn hold_images(state: &State, include_new: bool) -> Result<Vec<File>, ActionError> {
    let mut held = Vec::new();
    for (snapshot, new) in [(&state.old, false), (&state.new, true)] {
        if new && !include_new {
            continue;
        }
        for (i, row) in snapshot.files.iter().enumerate() {
            let lower = row.path.to_ascii_lowercase();
            if !lower.ends_with(".exe") && !lower.ends_with(".dll") {
                continue;
            }
            let root = if new {
                &state.final_path
            } else if state.old_in_backup.contains(&i) {
                &state.backup
            } else {
                &state.final_path
            };
            let path = root.join(&row.path);
            local(&path)?;
            let mut options = OpenOptions::new();
            options.read(true).write(true);
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                options.share_mode(7);
            }
            held.push(options.open(&path).map_err(|_| {
                fault(
                    "legacy_upgrade_stop_required",
                    "代码映像仍被占用，未结束任何进程。",
                )
            })?);
            files::hash_file(&path, row)?;
        }
    }
    if held.is_empty() {
        return Err(refused("映像保护集为空。"));
    }
    Ok(held)
}
fn recovery_archive(state: &State) -> Result<PathBuf, ActionError> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| refused(e.to_string()))?
        .as_nanos();
    let path = state.marker.stage.join(format!(
        "recovery-{}-{stamp}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    local(&path)?;
    fs::create_dir(&path).map_err(|e| refused(e.to_string()))?;
    Ok(path)
}
fn restore_old(state: &State, journal: &mut files::Journal) -> Result<(), ActionError> {
    if state.new_in_final || state.index_new {
        return Err(refused(
            "新包已在final；本入口不提供撤回已切入新包或已发布索引。",
        ));
    }
    files::create_dirs(&state.final_path, &state.old.dirs)?;
    for &i in state.old_in_backup.iter().rev() {
        let row = &state.old.files[i];
        files::hash_file(&state.backup.join(&row.path), row)?;
        journal.record("restore_old_file", "intent", Some(&row.path))?;
        files::move_no_replace(
            &state.backup.join(&row.path),
            &state.final_path.join(&row.path),
            "explicit_restore_old_file",
        )?;
        journal.record("restore_old_file", "done", Some(&row.path))?;
    }
    files::snapshot(&state.final_path, &state.old.digest)?;
    Ok(())
}
fn archive_empty_transaction(state: &State, archive: &Path) -> Result<(), ActionError> {
    // 必须已恢复全旧包。仅删除已知空目录；保留原计划、日志和未落完的尾字节。
    files::snapshot(&state.final_path, &state.old.digest)?;
    if files::exists(&state.backup)? {
        files::inventory_partial(
            &state.backup,
            &BTreeSet::new(),
            &state.old.dirs.iter().cloned().collect(),
            false,
        )?;
        for dir in state.old.dirs.iter().rev() {
            let path = state.backup.join(dir);
            if files::exists(&path)? {
                fs::remove_dir(&path).map_err(|e| refused(e.to_string()))?;
            }
        }
        fs::remove_dir(&state.backup).map_err(|e| refused(e.to_string()))?;
    }
    for name in ["swap-plan.json", "swap-journal.jsonl"] {
        let path = state.marker.stage.join(name);
        if files::exists(&path)? {
            files::move_no_replace(&path, &archive.join(name), "archive_original_transaction")?;
        }
    }
    Ok(())
}
fn run(config: &Path, pin: &str, mode: Mode) -> Result<Value, ActionError> {
    run_observed(config, pin, mode, observe_status)
}
fn observe_status(state: &State) -> Result<(), ActionError> {
    let exe = if state.new_in_final {
        &state.final_path
    } else {
        &state.new_path
    }
    .join("host/OmniTrigger.LegacyHost.exe");
    for root in [&state.marker.source_root, &state.marker.destination_root] {
        require_absent(&capture(
            &exe,
            vec![
                "status".into(),
                "--data-root".into(),
                root.to_string_lossy().into(),
            ],
            5000,
        )?)?;
    }
    Ok(())
}
// 唯一测试接缝为status观察；生产入口只能传上面的真实观察函数，没有JSON免验证选项。
fn run_observed(
    config: &Path,
    pin: &str,
    mode: Mode,
    observe: impl FnOnce(&State) -> Result<(), ActionError>,
) -> Result<Value, ActionError> {
    #[cfg(not(windows))]
    if mode != Mode::Inspect {
        return Err(refused("仅支持Windows恢复写入。"));
    }
    // inspect不创建索引锁或日志。写模式沿用既有索引锁，拒绝并行升级/恢复。
    strict_path(config)?;
    if mode != Mode::Inspect
        && !files::digest(&bounded_read(&profile(config)?.join(MARKER), 64 * 1024)?)
            .eq_ignore_ascii_case(pin)
    {
        return Err(refused("marker摘要不符，未创建恢复写锁。"));
    }
    let guard = if mode == Mode::Inspect {
        None
    } else {
        Some(IndexGuard::acquire(config)?)
    };
    let state = State::load(config, pin)?;
    if mode == Mode::Inspect {
        return Ok(state.report("inspected_no_changes"));
    }
    if mode == Mode::RestoreOld && state.new_in_final {
        return Err(refused(
            "新包已在final，使用finish-new核验收尾；不提供通用版本撤回。",
        ));
    }
    if mode == Mode::FinishNew {
        migration_ready(&state)?;
    }
    // 先使用完整新包的既有status协议核验两个明确根，再取得写映像guard；没有自动重试。
    observe(&state)?;
    let _images = hold_images(&state, state.new_in_final)?;
    // status期间可能有文件变动；在任何文件恢复或索引写入前重建完整分片事实。
    let refreshed = State::load(config, pin)?;
    if refreshed.old_in_backup != state.old_in_backup
        || refreshed.new_in_final != state.new_in_final
        || refreshed.index_new != state.index_new
    {
        return Err(refused("status期间事务位置已变化。"));
    }
    if mode == Mode::FinishNew {
        migration_ready(&refreshed)?;
    }
    state.unchanged_marker()?;
    let archive = recovery_archive(&state)?;
    super::write_new(&archive.join("observed-marker.json"), &state.marker_bytes)?;
    let mut journal = files::Journal::create(
        &archive.join("recovery-journal.jsonl"),
        state.old.files.len(),
    )?;
    if mode == Mode::RestoreOld {
        restore_old(&state, &mut journal)?;
        state.unchanged_marker()?;
        let mut report = state.report("old_package_restored_gate_retained");
        report["old_files_in_backup"] = 0.into();
        report["evidence"] = json!(archive);
        return Ok(report);
    }
    let guard = guard
        .as_ref()
        .expect("write mode holds existing index guard");
    let mut latest = crate::plugins::Plugins::load(config).map_err(|e| refused(e.to_string()))?;
    if latest.legacy_manifest_sha256()
        != Some(if state.index_new {
            state.marker.new_manifest_sha256.as_str()
        } else {
            state.marker.old_manifest_sha256.as_str()
        })
    {
        return Err(refused("恢复前插件索引pin已变化。"));
    }
    if !state.new_in_final {
        restore_old(&state, &mut journal)?;
        archive_empty_transaction(&state, &archive)?;
        state.unchanged_marker()?;
        latest.pin_legacy_manifest(state.marker.new_manifest_sha256.clone());
        files::switch_and_publish(
            &state.final_path,
            &state.new_path,
            &state.backup,
            (&state.old.digest, &state.new.digest),
            || {
                latest
                    .save_for_legacy_upgrade(config, guard)
                    .map_err(|e| uncertain("legacy_upgrade_index_publish_failed", e.to_string()))
            },
        )?;
    } else if !state.index_new {
        state.unchanged_marker()?;
        files::snapshot(&state.final_path, &state.new.digest)?;
        files::snapshot(&state.backup, &state.old.digest)?;
        latest.pin_legacy_manifest(state.marker.new_manifest_sha256.clone());
        journal.record("index_publish", "intent", None)?;
        latest
            .save_for_legacy_upgrade(config, guard)
            .map_err(|e| uncertain("legacy_upgrade_index_publish_failed", e.to_string()))?;
        journal
            .record("index_publish", "done", None)
            .map_err(|e| uncertain("legacy_upgrade_committed_but_blocked", e.message))?;
    }
    // 索引已经发布时从不撤回包。最终再次核验；任何失败保留marker，后续仍须显式入口。
    files::snapshot(&state.final_path, &state.new.digest)?;
    files::snapshot(&state.backup, &state.old.digest)?;
    journal
        .record("finish_recovery", "done", None)
        .map_err(|e| uncertain("legacy_upgrade_committed_but_blocked", e.message))?;
    // 收尾日志之后、按路径删除之前再次核对归属；这不是原子的compare-and-delete。
    state
        .unchanged_marker()
        .map_err(|e| uncertain("legacy_upgrade_committed_but_blocked", e.message))?;
    fs::remove_file(&state.marker_path)
        .map_err(|e| uncertain("legacy_upgrade_committed_but_blocked", e.to_string()))?;
    let mut report = state.report("new_index_committed_not_started");
    report["new_package_in_final"] = true.into();
    report["index_is_new"] = true.into();
    report["old_files_in_backup"] = state.old.files.len().into();
    report["marker_removed"] = true.into();
    report["evidence"] = json!(archive);
    Ok(report)
}

pub(crate) fn cli(arguments: &[String]) -> Option<Result<String, ActionError>> {
    if !arguments
        .iter()
        .any(|v| matches!(v.as_str(), "--recover-legacy-upgrade" | "--recovery-mode"))
    {
        return None;
    }
    Some((|| {
        let mut config = None;
        let mut pin = None;
        let mut mode = None;
        let mut index = 1;
        while index < arguments.len() {
            let value = arguments
                .get(index + 1)
                .ok_or_else(|| refused("恢复参数缺少值。"))?;
            match arguments[index].as_str() {
                "--recover-legacy-upgrade" if pin.is_none() && valid_hash(value) => {
                    pin = Some(value.as_str())
                }
                "--config" if config.is_none() => {
                    strict_path(Path::new(value))?;
                    config = Some(Path::new(value));
                }
                "--recovery-mode" if mode.is_none() => {
                    mode = Some(match value.as_str() {
                        "inspect" => Mode::Inspect,
                        "restore-old" => Mode::RestoreOld,
                        "finish-new" => Mode::FinishNew,
                        _ => return Err(refused("未知恢复模式。")),
                    })
                }
                _ => return Err(refused("恢复命令不接受重复、混用或未知参数。")),
            }
            index += 2;
        }
        let result = run(
            config.ok_or_else(|| refused("恢复必须明确--config。"))?,
            pin.ok_or_else(|| refused("缺少marker摘要。"))?,
            mode.ok_or_else(|| refused("缺少--recovery-mode。"))?,
        )?;
        serde_json::to_string(&result).map_err(|e| refused(e.to_string()))
    })())
}

#[cfg(all(test, windows))]
mod contracts {
    use super::*;
    struct Fixture {
        root: PathBuf,
        config: PathBuf,
        marker_pin: String,
    }
    impl Fixture {
        fn new() -> Self {
            let root = super::super::contract_root("recovery");
            let profile = root.join("profile");
            let config = profile.join("bindings.json");
            let final_path = profile.join("actions/legacy-host-entry");
            let stage = profile.join("actions/.install-legacy-upgrade-1-2-3");
            let new_path = stage.join("actions/legacy-host-entry");
            for (path, value) in [(&final_path, "old"), (&new_path, "new")] {
                fs::create_dir_all(path.join("host")).unwrap();
                let mut rows = Vec::new();
                for name in ["identity", "host/code.dll"] {
                    let bytes = format!("nonexecutable-synthetic-{value}-{name}").into_bytes();
                    fs::write(path.join(name), &bytes).unwrap();
                    rows.push(
                        json!({"path":name,"bytes":bytes.len(),"sha256":files::digest(&bytes)}),
                    );
                }
                fs::write(path.join("manifest.json"), serde_json::to_vec(&json!({"schema_version":1,"architecture":"windows-x64","package_version":value,"files":rows})).unwrap()).unwrap();
            }
            let old = files::snapshot(&final_path, &files::manifest_digest(&final_path).unwrap())
                .unwrap();
            let new =
                files::snapshot(&new_path, &files::manifest_digest(&new_path).unwrap()).unwrap();
            let mut plugins = crate::plugins::Plugins::default();
            plugins.pin_legacy_manifest(old.digest.clone());
            plugins.configure_legacy_enabled(false);
            plugins.save(&config).unwrap();
            // 明确删除此全新fixture自己创建的空锁文件，验证inspect不重新创建它。
            fs::remove_file(profile.join(super::super::INDEX_LOCK)).unwrap();
            let source = root.join("source");
            let destination = profile.join("legacy-host-data");
            fs::create_dir(&source).unwrap();
            fs::create_dir(&destination).unwrap();
            fs::write(
                source.join("userdata.db"),
                b"synthetic-source-database-bytes",
            )
            .unwrap();
            fs::write(
                destination.join("userdata.db"),
                b"synthetic-target-database-bytes",
            )
            .unwrap();
            fs::write(
                destination.join("omnitrigger-legacy-host.marker"),
                b"OmniTrigger.LegacyHost.v1",
            )
            .unwrap();
            let receipt = json!({"schema_version":1,"scope":"legacy_data_root_copy","source":source,"destination":destination,
                "source_kind":"standalone-v1","source_modified":false,"old_root_retained":true,"format_translation_performed":false,
                "bindings_modified":false,"external_asset_paths_rewritten":false,"source_database_sha256":files::digest(b"synthetic-source-database-bytes")});
            fs::write(
                destination.join("omnitrigger-data-import-fixture.json"),
                serde_json::to_vec(&receipt).unwrap(),
            )
            .unwrap();
            let marker = json!({"schema_version":1,"scope":"legacy_upgrade_pending","stage":stage,"old_manifest_sha256":old.digest,
                "new_manifest_sha256":new.digest,"source_root":source,"destination_root":destination,"host_started":false});
            let bytes = serde_json::to_vec(&marker).unwrap();
            fs::write(profile.join(MARKER), &bytes).unwrap();
            let fixture = Self {
                root,
                config,
                marker_pin: files::digest(&bytes),
            };
            let state = fixture.state();
            fs::write(state.marker.stage.join("swap-plan.json"), serde_json::to_vec(&json!({"schema_version":1,"scope":"legacy_upgrade_file_swap",
                "final_path":state.final_path,"new_path":state.new_path,"backup":state.backup,"old":state.old,"new":state.new})).unwrap()).unwrap();
            fs::write(state.marker.stage.join("swap-journal.jsonl"), b"").unwrap();
            fixture
        }
        fn state(&self) -> State {
            State::load(&self.config, &self.marker_pin).unwrap()
        }
        fn move_old(&self, count: usize, final_new: bool) {
            let state = self.state();
            files::create_dirs(&state.backup, &state.old.dirs).unwrap();
            let mut log = files::Journal::create(
                &state.marker.stage.join("fixture-move-evidence.jsonl"),
                state.old.files.len(),
            )
            .unwrap();
            for row in state.old.files.iter().take(count) {
                log.record("old_file_to_backup", "intent", Some(&row.path))
                    .unwrap();
                files::move_no_replace(
                    &state.final_path.join(&row.path),
                    &state.backup.join(&row.path),
                    "fixture_move",
                )
                .unwrap();
                // 故意不写done：验证实际路径与摘要负责判定，不依赖done的存在。
            }
            fs::copy(
                state.marker.stage.join("fixture-move-evidence.jsonl"),
                state.marker.stage.join("swap-journal.jsonl"),
            )
            .unwrap();
            if final_new {
                assert_eq!(count, state.old.files.len());
                for dir in state.old.dirs.iter().rev() {
                    fs::remove_dir(state.final_path.join(dir)).unwrap();
                }
                fs::remove_dir(&state.final_path).unwrap();
                files::move_no_replace(&state.new_path, &state.final_path, "fixture_new_cut_in")
                    .unwrap();
            }
        }
        fn finish(&self) -> Value {
            run_observed(&self.config, &self.marker_pin, Mode::FinishNew, |_| Ok(())).unwrap()
        }
    }
    #[test]
    fn inspect_intent_only_split_is_readonly_and_restore_keeps_gate() {
        let fixture = Fixture::new();
        fixture.move_old(1, false);
        let state = fixture.state();
        let index = fs::read(fixture.config.parent().unwrap().join("plugins.json")).unwrap();
        let log_path = state.marker.stage.join("swap-journal.jsonl");
        let mut log = fs::read(&log_path).unwrap();
        log.extend_from_slice(b"{\"seq\":1,");
        fs::write(&log_path, &log).unwrap();
        let report = run_observed(&fixture.config, &fixture.marker_pin, Mode::Inspect, |_| {
            panic!("inspect不得观察Host")
        })
        .unwrap();
        assert_eq!(report["old_files_in_backup"], 1);
        assert_eq!(report["journal_partial_tail_preserved"], true);
        assert!(!fixture
            .config
            .parent()
            .unwrap()
            .join(super::super::INDEX_LOCK)
            .exists());
        let restored = run_observed(
            &fixture.config,
            &fixture.marker_pin,
            Mode::RestoreOld,
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(restored["status"], "old_package_restored_gate_retained");
        assert_eq!(fs::read(&log_path).unwrap(), log);
        assert_eq!(
            fs::read(fixture.config.parent().unwrap().join("plugins.json")).unwrap(),
            index
        );
        assert!(state.marker_path.exists());
        assert!(super::super::block_config(&fixture.config).is_err());
        files::snapshot(&state.final_path, &state.old.digest).unwrap();
        files::snapshot(&state.new_path, &state.new.digest).unwrap();
    }
    #[test]
    fn finish_split_restores_then_reuses_transaction_and_preserves_original_log() {
        let fixture = Fixture::new();
        fixture.move_old(1, false);
        let state = fixture.state();
        let original = fs::read(state.marker.stage.join("swap-journal.jsonl")).unwrap();
        let source_before = fs::read(fixture.root.join("source/userdata.db")).unwrap();
        let report = fixture.finish();
        assert_eq!(report["status"], "new_index_committed_not_started");
        assert!(!state.marker_path.exists());
        let evidence = PathBuf::from(report["evidence"].as_str().unwrap());
        assert_eq!(
            fs::read(evidence.join("swap-journal.jsonl")).unwrap(),
            original
        );
        files::snapshot(&state.final_path, &state.new.digest).unwrap();
        files::snapshot(&state.backup, &state.old.digest).unwrap();
        let plugins = crate::plugins::Plugins::load(&fixture.config).unwrap();
        assert_eq!(
            plugins.legacy_manifest_sha256(),
            Some(state.new.digest.as_str())
        );
        assert!(!plugins.legacy_enabled());
        assert_eq!(
            fs::read(fixture.root.join("source/userdata.db")).unwrap(),
            source_before
        );
    }
    #[test]
    fn new_cut_in_finishes_old_index_and_already_published_index() {
        for published in [false, true] {
            let fixture = Fixture::new();
            let count = fixture.state().old.files.len();
            fixture.move_old(count, true);
            let state = fixture.state();
            if published {
                let guard = IndexGuard::acquire(&fixture.config).unwrap();
                let mut plugins = crate::plugins::Plugins::load(&fixture.config).unwrap();
                plugins.pin_legacy_manifest(state.new.digest.clone());
                plugins
                    .save_for_legacy_upgrade(&fixture.config, &guard)
                    .unwrap();
            }
            let before = fs::read(fixture.config.parent().unwrap().join("plugins.json")).unwrap();
            assert_eq!(fixture.finish()["marker_removed"], true);
            files::snapshot(&state.final_path, &state.new.digest).unwrap();
            files::snapshot(&state.backup, &state.old.digest).unwrap();
            if published {
                assert_eq!(
                    fs::read(fixture.config.parent().unwrap().join("plugins.json")).unwrap(),
                    before
                );
            }
        }
    }
    #[test]
    fn duplicate_or_changed_old_file_and_bad_plan_refuse_before_observation() {
        for damage in ["duplicate", "changed", "plan"] {
            let fixture = Fixture::new();
            fixture.move_old(1, false);
            let state = fixture.state();
            let row = &state.old.files[0];
            match damage {
                "duplicate" => {
                    fs::copy(
                        state.backup.join(&row.path),
                        state.final_path.join(&row.path),
                    )
                    .unwrap();
                }
                "changed" => fs::write(state.backup.join(&row.path), b"changed").unwrap(),
                _ => {
                    let path = state.marker.stage.join("swap-plan.json");
                    let mut value = json_read(&path, 2 * 1024 * 1024).unwrap();
                    value["backup"] = json!(fixture.root.join("outside"));
                    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
                }
            }
            assert!(run_observed(
                &fixture.config,
                &fixture.marker_pin,
                Mode::RestoreOld,
                |_| panic!("损坏必须先于status拒绝")
            )
            .is_err());
            assert!(state.marker_path.exists());
        }
    }
    #[test]
    fn observation_failure_and_marker_pin_change_leave_split_untouched() {
        let fixture = Fixture::new();
        fixture.move_old(1, false);
        let state = fixture.state();
        let error = run_observed(
            &fixture.config,
            &fixture.marker_pin,
            Mode::RestoreOld,
            |_| Err(uncertain("process_timeout", "合成观察超时")),
        )
        .unwrap_err();
        assert_eq!(error.code, "process_timeout");
        assert_eq!(fixture.state().old_in_backup, state.old_in_backup);
        assert!(State::load(&fixture.config, &"0".repeat(64)).is_err());
        assert!(state.marker_path.exists());
    }
    #[test]
    fn missing_migration_receipt_refuses_finish_but_restore_remains_explicit() {
        let fixture = Fixture::new();
        fixture.move_old(1, false);
        let state = fixture.state();
        fs::remove_file(
            state
                .marker
                .destination_root
                .join("omnitrigger-data-import-fixture.json"),
        )
        .unwrap();
        assert!(run_observed(
            &fixture.config,
            &fixture.marker_pin,
            Mode::FinishNew,
            |_| panic!("回执不成立不得观察Host")
        )
        .is_err());
        assert_eq!(
            run_observed(
                &fixture.config,
                &fixture.marker_pin,
                Mode::RestoreOld,
                |_| Ok(())
            )
            .unwrap()["old_files_in_backup"],
            0
        );
        assert!(state.marker_path.exists());
    }
    #[test]
    fn cli_requires_explicit_mode_config_and_pin_without_personal_defaults() {
        let pin = "a".repeat(64);
        for args in [
            vec!["app", "--recover-legacy-upgrade"],
            vec!["app", "--recover-legacy-upgrade", "bad"],
            vec![
                "app",
                "--recover-legacy-upgrade",
                pin.as_str(),
                "--recovery-mode",
                "inspect",
            ],
        ] {
            let values = args.iter().map(|s| s.to_string()).collect::<Vec<_>>();
            assert!(cli(&values).unwrap().is_err());
        }
        assert!(cli(&["app".into()]).is_none());
    }
}
