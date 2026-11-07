//! 既有安装worker的两段升级叶；不创建线程、动作队列或GUI。
#[path = "legacy_upgrade_files.rs"]
mod files;
#[path = "legacy_upgrade_recovery.rs"]
pub(crate) mod recovery;
use crate::legacy_plugin_work::{LegacyArtifactSource, VerifiedLegacyInstall};
use omnitrigger_actions::{ActionError, CancellationToken, NativeHost};
use serde_json::{json, Value};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::mpsc::SyncSender,
    time::{Duration, Instant},
};

const MARKER: &str = "legacy-upgrade-in-progress.json";
const INDEX_LOCK: &str = "plugins.legacy-write.lock";
pub(crate) struct Request {
    pub source: crate::plugin_install::LegacyInstallSource,
    pub expected_old_manifest: String,
    pub source_root: PathBuf,
    pub source_database_sha256: String,
}
#[derive(Debug)]
pub(crate) struct Prepared {
    config: PathBuf,
    stage: PathBuf,
    source_root: PathBuf,
    destination_root: PathBuf,
    old_digest: String,
    new_digest: String,
    installed: VerifiedLegacyInstall,
    index_guard: IndexGuard,
    _old_images: Vec<File>,
    marker_bytes: Vec<u8>,
}
#[derive(Debug)]
pub(crate) struct IndexGuard {
    config: PathBuf,
    _file: File,
}
fn fault(code: &str, message: impl Into<String>) -> ActionError {
    // 可能已下载或复制自有副本；不以not_started描述整个文件事务。
    ActionError::new(code, message)
}
fn uncertain(code: &str, message: impl Into<String>) -> ActionError {
    let mut error = fault(code, message);
    error.effect_state = Some("unknown".into());
    error
}
fn local(path: &Path) -> Result<(), ActionError> {
    crate::script_runtime::check_local_path(path)
        .map_err(|e| fault("legacy_upgrade_path_invalid", e))
}
fn profile(config: &Path) -> Result<PathBuf, ActionError> {
    if !config.is_absolute() {
        return Err(fault(
            "legacy_upgrade_path_invalid",
            "配置必须是明确绝对路径。",
        ));
    }
    local(config)?;
    config
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| fault("legacy_upgrade_path_invalid", "配置没有父目录。"))
}
pub(crate) fn block_config(config: &Path) -> Result<(), ActionError> {
    block_profile(&profile(config)?)
}
pub(crate) fn block_package(package: &Path) -> Result<(), ActionError> {
    let root = package
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| fault("legacy_upgrade_path_invalid", "包路径无效。"))?;
    block_profile(root)
}
fn block_profile(root: &Path) -> Result<(), ActionError> {
    let marker = root.join(MARKER);
    local(&marker)?;
    if marker
        .try_exists()
        .map_err(|e| fault("legacy_upgrade_state_unreadable", e.to_string()))?
    {
        return Err(fault(
            "legacy_upgrade_recovery_required",
            "存在未结束的升级记录，未准备或运行宿主。",
        ));
    }
    Ok(())
}
impl IndexGuard {
    // 全部Plugins写入共用这个短路径锁；Windows共享失败立即返回，不等待或轮询。
    pub(crate) fn acquire(config: &Path) -> Result<Self, ActionError> {
        let parent = profile(config)?;
        fs::create_dir_all(&parent).map_err(|e| fault("legacy_upgrade_io", e.to_string()))?;
        let path = parent.join(INDEX_LOCK);
        local(&path)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0);
        }
        #[cfg(not(windows))]
        {
            return Err(fault(
                "legacy_upgrade_platform_unsupported",
                "本索引写锁只支持Windows。",
            ));
        }
        #[cfg(windows)]
        {
            let file = options.open(path).map_err(|_| {
                fault(
                    "legacy_upgrade_index_busy",
                    "插件索引正在写入或升级，未覆盖。",
                )
            })?;
            Ok(Self {
                config: config.to_path_buf(),
                _file: file,
            })
        }
    }
    pub(crate) fn matches(&self, config: &Path) -> bool {
        self.config == config
    }
}
fn capture(exe: &Path, args: Vec<String>, timeout: u64) -> Result<Value, ActionError> {
    let host = crate::process_capture_host::WithProcessCapture::new(
        crate::legacy_prepare_bridge::NoTargetHost,
    );
    let program = exe
        .to_str()
        .ok_or_else(|| fault("legacy_upgrade_path_invalid", "程序路径无法表示。"))?;
    let output = host.call_with_deadline(
        "process.capture",
        json!({"program":program,"args":args,
        "timeout_ms":timeout,"allow_non_zero":true}),
        &CancellationToken::new(),
        Instant::now() + Duration::from_millis(timeout + 1000),
    )?;
    if output.get("code").and_then(Value::as_i64) != Some(0)
        || output.get("stderr").and_then(Value::as_str) != Some("")
    {
        return Err(uncertain(
            "legacy_upgrade_control_unconfirmed",
            "控制命令没有有效成功回执，不重试。",
        ));
    }
    serde_json::from_str(output.get("stdout").and_then(Value::as_str).unwrap_or("")).map_err(|_| {
        uncertain(
            "legacy_upgrade_control_unconfirmed",
            "控制回执无法解析，不重试。",
        )
    })
}
fn require_absent(value: &Value) -> Result<(), ActionError> {
    if value.as_object().map(|v| v.len()) == Some(5)
        && value["schema_version"] == 1
        && value["scope"] == "host_status"
        && value["state"] == "absent"
        && value["can_start"] == true
        && value["error_code"] == "endpoint_missing"
    {
        return Ok(());
    }
    Err(fault(
        "legacy_upgrade_stop_required",
        "旧宿主仍在运行或状态未知；请先正常停止，再显式重试升级。",
    ))
}
fn retain_image_guards(package: &Path) -> Result<Vec<File>, ActionError> {
    let manifest_path = package.join("manifest.json");
    local(&manifest_path)?;
    let file = File::open(manifest_path)
        .map_err(|e| fault("legacy_upgrade_stop_required", e.to_string()))?;
    let mut bytes = Vec::new();
    file.take(256 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| fault("legacy_upgrade_stop_required", e.to_string()))?;
    if bytes.len() > 256 * 1024 {
        return Err(fault("legacy_upgrade_stop_required", "旧清单超限。"));
    }
    let manifest: Value = serde_json::from_slice(&bytes)
        .map_err(|e| fault("legacy_upgrade_stop_required", e.to_string()))?;
    let rows = manifest["files"]
        .as_array()
        .filter(|v| v.len() <= 1023)
        .ok_or_else(|| fault("legacy_upgrade_stop_required", "旧清单无效。"))?;
    let mut held = Vec::new();
    for row in rows {
        let relative = row["path"]
            .as_str()
            .ok_or_else(|| fault("legacy_upgrade_stop_required", "旧清单路径无效。"))?;
        if !relative.to_ascii_lowercase().ends_with(".exe")
            && !relative.to_ascii_lowercase().ends_with(".dll")
        {
            continue;
        }
        if Path::new(relative).is_absolute()
            || relative.contains('\\')
            || relative
                .split('/')
                .any(|p| p.is_empty() || p == ".." || p == ".")
        {
            return Err(fault("legacy_upgrade_stop_required", "旧代码路径无效。"));
        }
        let path = package.join(relative);
        local(&path)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true);
        // 只申请写访问，不写入任何字节。Windows已有映像/写入占用会阻止获得访问；
        // 保持句柄到索引发布，防止旧代码重新映射；只逐文件移动，不移动含打开子文件的父目录。
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(7);
        }
        #[cfg(not(windows))]
        {
            return Err(fault(
                "legacy_upgrade_platform_unsupported",
                "仅支持Windows映像保护。",
            ));
        }
        held.push(options.open(path).map_err(|_| {
            fault(
                "legacy_upgrade_stop_required",
                "旧代码仍被映射/占用或无法取得保护访问；未结束任何进程。",
            )
        })?);
    }
    if held.is_empty() {
        return Err(fault("legacy_upgrade_stop_required", "旧代码保护集为空。"));
    }
    Ok(held)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), ActionError> {
    local(path)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| fault("legacy_upgrade_io", e.to_string()))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| fault("legacy_upgrade_io", e.to_string()))
}
fn bounded_read(path: &Path, limit: u64) -> Result<Vec<u8>, ActionError> {
    local(path)?;
    let file = File::open(path).map_err(|e| fault("legacy_upgrade_io", e.to_string()))?;
    if file
        .metadata()
        .map_err(|e| fault("legacy_upgrade_io", e.to_string()))?
        .len()
        > limit
    {
        return Err(fault("legacy_upgrade_read_limit", "升级记录或索引超限。"));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| fault("legacy_upgrade_io", e.to_string()))?;
    if bytes.len() as u64 > limit {
        return Err(fault("legacy_upgrade_read_limit", "读取期间文件超限。"));
    }
    Ok(bytes)
}
pub(crate) fn prepare(
    config: &Path,
    request: Request,
    events: &SyncSender<crate::plugin_install::InstallEvent>,
) -> Result<Prepared, ActionError> {
    let parent = profile(config)?;
    block_config(config)?;
    request.source.validate()?;
    if request.expected_old_manifest.len() != 64
        || !request
            .expected_old_manifest
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
        || request.expected_old_manifest == request.source.manifest_sha256
        || request.source_database_sha256.len() != 64
        || !request
            .source_database_sha256
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
        || !request.source_root.is_absolute()
    {
        return Err(fault(
            "legacy_upgrade_request_invalid",
            "需要不同的新清单、明确旧根与数据库摘要。",
        ));
    }
    local(&request.source_root)?;
    let destination = parent.join("legacy-host-data");
    local(&destination)?;
    // 在任何暂存或迁移前排除源根与包目录重叠；canonicalize解析本机已有路径别名。
    let packages = crate::action_packages::root(config);
    local(&packages)?;
    let source_scope = fs::canonicalize(&request.source_root)
        .map_err(|e| fault("legacy_upgrade_path_invalid", e.to_string()))?;
    let package_scope = fs::canonicalize(&packages)
        .map_err(|e| fault("legacy_upgrade_path_invalid", e.to_string()))?;
    let destination_scope = fs::canonicalize(&parent)
        .map_err(|e| fault("legacy_upgrade_path_invalid", e.to_string()))?
        .join("legacy-host-data");
    if source_scope.starts_with(&package_scope)
        || package_scope.starts_with(&source_scope)
        || source_scope.starts_with(&destination_scope)
        || destination_scope.starts_with(&source_scope)
    {
        return Err(fault(
            "legacy_upgrade_path_invalid",
            "旧数据根不能与包目录或目标根重叠。",
        ));
    }
    if destination
        .try_exists()
        .map_err(|e| fault("legacy_upgrade_io", e.to_string()))?
    {
        return Err(fault(
            "legacy_upgrade_destination_exists",
            "新根已存在，未删除、合并或当作迁移成功。",
        ));
    }
    // 先完整下载/解包/哈希/合同校验新包；不先停止、改索引或移动旧包。
    let (stage_config, installed) =
        crate::plugin_install::stage_legacy_upgrade(config, &request.source, events)?;
    let stage = stage_config
        .parent()
        .ok_or_else(|| fault("legacy_upgrade_path_invalid", "暂存目录无效。"))?
        .to_path_buf();

    let old_source =
        crate::legacy_local_source::LocalLegacySource::new(config, &request.expected_old_manifest)?;
    let old = old_source
        .verify(config, &CancellationToken::new())
        .map_err(|_| {
            fault(
                "legacy_upgrade_stop_required",
                "旧包/协议不可核验，未猜测旧根或旧进程。",
            )
        })?;
    crate::legacy_local_source::require_explicit_host_contract(
        &old.prepare_action.package_root,
        &CancellationToken::new(),
    )?;
    let status = capture(
        &old.executable,
        vec![
            "status".into(),
            "--data-root".into(),
            request.source_root.to_string_lossy().into(),
        ],
        5000,
    )
    .map_err(|mut error| {
        error.message = format!(
            "旧宿主状态采集未确认，未迁移或换版，不自动重试。{}",
            error.message
        );
        error
    })?;
    require_absent(&status)?;
    let old_images = retain_image_guards(&old.prepare_action.package_root)?;
    let index_guard = IndexGuard::acquire(config)?;
    block_config(config)?;
    let plugins = crate::plugins::Plugins::load(config)
        .map_err(|e| fault("legacy_upgrade_index_invalid", e.to_string()))?;
    if plugins.legacy_manifest_sha256() != Some(request.expected_old_manifest.as_str()) {
        return Err(fault(
            "legacy_upgrade_index_changed",
            "旧安装索引已变化，未换版。",
        ));
    }
    let index = parent.join("plugins.json");
    local(&index)?;
    let original = bounded_read(&index, 128 * 1024)?;
    write_new(&stage.join("plugins.before.json"), &original)?;
    let marker = json!({"schema_version":1,"scope":"legacy_upgrade_pending","stage":stage,
        "old_manifest_sha256":request.expected_old_manifest,"new_manifest_sha256":request.source.manifest_sha256,
        "source_root":request.source_root,"destination_root":destination,"host_started":false});
    let marker_bytes =
        serde_json::to_vec(&marker).map_err(|e| fault("legacy_upgrade_io", e.to_string()))?;
    write_new(&parent.join(MARKER), &marker_bytes)?;
    // 从此任何失败都保留marker/快照/新根，正常prepare与索引写入明确阻断，不能静默重试。
    let reply = capture(
        &installed.executable,
        vec![
            "migrate-root".into(),
            "--source-root".into(),
            request.source_root.to_string_lossy().into(),
            "--destination-root".into(),
            destination.to_string_lossy().into(),
            "--source-kind".into(),
            "standalone-v1".into(),
            "--expected-database-sha256".into(),
            request.source_database_sha256.clone(),
        ],
        60000,
    )
    .map_err(|e| {
        uncertain(
            "legacy_upgrade_migration_unconfirmed",
            format!("{}；保留升级记录供恢复。", e.message),
        )
    })?;
    if reply["schema_version"] != 1
        || reply["scope"] != "legacy_data_root_copy"
        || reply["status"] != "copied_verified_not_started"
        || reply["destination_activated"] != true
        || reply["source_modified"] != false
        || reply["host_started"] != false
        || reply["bindings_modified"] != false
        || !destination.is_dir()
    {
        return Err(uncertain(
            "legacy_upgrade_migration_unconfirmed",
            "迁移回执无效，未发布新索引或启动。",
        ));
    }
    let receipt = reply["receipt"]
        .as_str()
        .map(PathBuf::from)
        .ok_or_else(|| uncertain("legacy_upgrade_migration_unconfirmed", "迁移缺少回执。"))?;
    if receipt.parent() != Some(destination.as_path()) {
        return Err(uncertain(
            "legacy_upgrade_migration_unconfirmed",
            "回执不属于新根。",
        ));
    }
    local(&receipt)?;
    let details: Value = serde_json::from_slice(
        &bounded_read(&receipt, 4 * 1024 * 1024)
            .map_err(|e| uncertain("legacy_upgrade_migration_unconfirmed", e.message))?,
    )
    .map_err(|e| uncertain("legacy_upgrade_migration_unconfirmed", e.to_string()))?;
    if details["source"] != request.source_root.to_string_lossy().as_ref()
        || details["destination"] != destination.to_string_lossy().as_ref()
        || details["source_database_sha256"] != request.source_database_sha256
        || details["source_modified"] != false
    {
        return Err(uncertain(
            "legacy_upgrade_migration_unconfirmed",
            "迁移根或源摘要不一致。",
        ));
    }
    Ok(Prepared {
        config: config.to_path_buf(),
        stage,
        source_root: request.source_root,
        destination_root: destination,
        old_digest: request.expected_old_manifest,
        new_digest: request.source.manifest_sha256,
        installed,
        index_guard,
        _old_images: old_images,
        marker_bytes,
    })
}

// 逐文件事务在files叶；生产传入已核验的新旧pin，下面仅为既有合同的fixture桥。
#[cfg(test)]
fn switch_and_publish(
    final_path: &Path,
    new_path: &Path,
    backup: &Path,
    publish: impl FnOnce() -> Result<(), ActionError>,
) -> Result<(), ActionError> {
    // 原合同仍调用同一生产叶；pin来自该次自有fixture，缺新路径由生产前置检查拒绝。
    let old = files::manifest_digest(final_path)?;
    let new = files::manifest_digest(new_path).unwrap_or_else(|_| "0".repeat(64));
    files::switch_and_publish(final_path, new_path, backup, (&old, &new), publish)
}
impl Prepared {
    pub(crate) fn commit(
        self,
        plugins: &mut crate::plugins::Plugins,
    ) -> Result<VerifiedLegacyInstall, ActionError> {
        let parent = profile(&self.config)?;
        let marker = parent.join(MARKER);
        local(&marker)?;
        if bounded_read(&marker, 64 * 1024)
            .map_err(|e| uncertain("legacy_upgrade_recovery_required", e.message))?
            != self.marker_bytes
        {
            return Err(uncertain(
                "legacy_upgrade_recovery_required",
                "升级记录已变，未切换。",
            ));
        }
        let mut latest = crate::plugins::Plugins::load(&self.config)
            .map_err(|e| fault("legacy_upgrade_index_invalid", e.to_string()))?;
        if latest.legacy_manifest_sha256() != Some(self.old_digest.as_str()) {
            return Err(fault(
                "legacy_upgrade_index_changed",
                "旧安装索引已变，未切换。",
            ));
        }
        let final_path = crate::action_packages::root(&self.config).join("legacy-host-entry");
        let new_path = self.installed.prepare_action.package_root.clone();
        let backup = self.stage.join("old-package");
        local(&final_path)?;
        local(&new_path)?;
        local(&backup)?;
        latest.pin_legacy_manifest(self.new_digest.clone());
        files::switch_and_publish(
            &final_path,
            &new_path,
            &backup,
            (&self.old_digest, &self.new_digest),
            || {
                latest
                    .save_for_legacy_upgrade(&self.config, &self.index_guard)
                    .map_err(|e| fault("legacy_upgrade_index_publish_failed", e.to_string()))
            },
        )?;
        *plugins = latest; // 最新enabled不变；旧index原字节与旧包仍在stage供显式回退。
        let mut installed = self.installed;
        installed.executable = final_path.join("host/OmniTrigger.LegacyHost.exe");
        installed.prepare_action.package_root = final_path.clone();
        installed.prepare_action.entry_path = final_path.join("prepare-host.mjs");
        // 只有索引提交后解除恢复门。失败则保持门关闭，不能把“索引已发布”当成“宿主已启动”。
        // 文件事务可能持续较久；开始时的归属核对不能授权删除后来替换的marker。
        if bounded_read(&marker, 64 * 1024)
            .map_err(|e| uncertain("legacy_upgrade_committed_but_blocked", e.message))?
            != self.marker_bytes
        {
            return Err(uncertain(
                "legacy_upgrade_committed_but_blocked",
                "索引已发布，但升级记录归属已变化；保留记录，不解除恢复门。",
            ));
        }
        fs::remove_file(&marker)
            .map_err(|e| uncertain("legacy_upgrade_committed_but_blocked", e.to_string()))?;
        let _retained_roots = (&self.source_root, &self.destination_root);
        Ok(installed)
    }
}

#[cfg(test)]
fn contract_root(label: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "omnitrigger-{label}-contract-{}-{stamp}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    local(&root).unwrap();
    fs::create_dir(&root).unwrap();
    root
}

#[cfg(test)]
mod contracts {
    use super::*;
    fn fixture_manifest(root: &Path, paths: &[&str]) {
        use sha2::{Digest, Sha256};
        let records=paths.iter().map(|relative| {
            let bytes=fs::read(root.join(relative)).unwrap();
            json!({"path":relative,"bytes":bytes.len(),"sha256":format!("{:x}",Sha256::digest(&bytes))})
        }).collect::<Vec<_>>();
        let value = json!({"schema_version":1,"package_version":"fixture","architecture":"windows-x64","files":records});
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
    }
    fn fixture() -> PathBuf {
        let root = super::contract_root("upgrade");
        fs::create_dir(root.join("old")).unwrap();
        fs::write(root.join("old/identity"), b"old").unwrap();
        fs::create_dir(root.join("new")).unwrap();
        fs::write(root.join("new/identity"), b"new").unwrap();
        fixture_manifest(&root.join("old"), &["identity"]);
        fixture_manifest(&root.join("new"), &["identity"]);
        root // 自有证据全部保留，不递归清理。
    }
    #[test]
    fn running_stopping_starting_failed_unknown_and_extra_fields_require_stop() {
        for state in ["ready", "stopping", "starting", "failed", "unknown"] {
            assert_eq!(require_absent(&json!({"schema_version":1,"scope":"host_status","state":state,"can_start":false,"error_code":null})).unwrap_err().code,"legacy_upgrade_stop_required");
        }
        let mut absent = json!({"schema_version":1,"scope":"host_status","state":"absent","can_start":true,"error_code":"endpoint_missing"});
        assert!(require_absent(&absent).is_ok());
        absent["extra"] = true.into();
        assert!(require_absent(&absent).is_err());
    }
    #[test]
    fn index_publish_failure_restores_old_package_and_retains_new() {
        let root = fixture();
        let index = root.join("index");
        fs::write(&index, b"old-index").unwrap();
        let result = switch_and_publish(
            &root.join("old"),
            &root.join("new"),
            &root.join("backup"),
            || Err(fault("injected_index_failure", "未写入")),
        );
        assert!(result.is_err());
        assert_eq!(fs::read(root.join("old/identity")).unwrap(), b"old");
        assert_eq!(fs::read(root.join("new/identity")).unwrap(), b"new");
        assert_eq!(fs::read(index).unwrap(), b"old-index");
    }
    #[test]
    fn missing_new_package_restores_old_without_publishing_index() {
        let root = fixture();
        let missing = root.join("missing");
        let result = switch_and_publish(&root.join("old"), &missing, &root.join("backup"), || {
            panic!("不得发布索引")
        });
        assert!(result.is_err());
        assert_eq!(fs::read(root.join("old/identity")).unwrap(), b"old");
    }
    #[test]
    fn success_keeps_old_backup_and_publishes_after_switch() {
        let root = fixture();
        let index = root.join("index");
        fs::write(&index, b"old-index").unwrap();
        switch_and_publish(
            &root.join("old"),
            &root.join("new"),
            &root.join("backup"),
            || {
                assert_eq!(fs::read(root.join("old/identity")).unwrap(), b"new");
                crate::settings::atomic_write(&index, b"new-index").map_err(|e| fault("index", e))
            },
        )
        .unwrap();
        assert_eq!(fs::read(root.join("backup/identity")).unwrap(), b"old");
        assert_eq!(fs::read(index).unwrap(), b"new-index");
    }
    #[cfg(windows)]
    #[test]
    fn held_real_image_guards_allow_old_backup_and_new_switch() {
        let root = fixture();
        let final_path = root.join("old");
        let stage = root.join(".install-legacy-upgrade-owned");
        let new_path = stage.join("actions/legacy-host-entry");
        fs::create_dir_all(new_path.parent().unwrap()).unwrap();
        fs::rename(root.join("new"), &new_path).unwrap();
        let backup = stage.join("old-package");
        // 不执行这些文件；通过真实manifest路径进入生产retain_image_guards。
        let images = [
            "host/guarded.exe",
            "host/guarded.dll",
            "runtimes/win-x64/native/guarded.dll",
        ];
        let bytes = b"owned non-executable image guard fixture";
        for relative in images {
            let path = final_path.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        fixture_manifest(&final_path, &["identity", images[0], images[1], images[2]]);
        let guards = retain_image_guards(&final_path).unwrap();
        assert_eq!(guards.len(), images.len());
        let published = std::cell::Cell::new(false);
        // 与无句柄的原成功合同不同：真实保护句柄在两个rename和发布回调内都存活。
        switch_and_publish(&final_path, &new_path, &backup, || {
            assert_eq!(guards.len(), images.len());
            for guard in &guards {
                assert_eq!(guard.metadata().unwrap().len(), bytes.len() as u64);
            }
            assert_eq!(fs::read(backup.join("identity")).unwrap(), b"old");
            assert_eq!(fs::read(final_path.join("identity")).unwrap(), b"new");
            published.set(true);
            Ok(())
        })
        .unwrap();
        assert!(published.get());
        for relative in images {
            assert_eq!(fs::read(backup.join(relative)).unwrap(), bytes);
        }
        assert_eq!(guards.len(), images.len());
        drop(guards); // 只在完整切换/断言之后释放，生产保护策略没有被提前绕过。
    }

    #[cfg(windows)]
    #[test]
    fn held_real_image_guards_allow_one_file_backup_and_reverse() {
        use std::io::{Read, Seek, SeekFrom};
        let root = fixture();
        let old = root.join("old");
        let stage = root.join(".install-legacy-upgrade-owned-file");
        let images = [
            "host/guarded.exe",
            "host/guarded.dll",
            "runtimes/win-x64/native/guarded.dll",
        ];
        let bytes = b"owned non-executable guarded file move";
        for relative in images {
            let path = old.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        let manifest = json!({"files":images.iter().map(|relative|json!({"path":relative})).collect::<Vec<_>>()});
        fs::write(
            old.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let guards = retain_image_guards(&old).unwrap();
        assert_eq!(guards.len(), images.len());
        let source = old.join(images[0]);
        let backup = stage.join("old-package").join(images[0]);
        fs::create_dir_all(backup.parent().unwrap()).unwrap();
        assert!(source.is_file());
        assert!(!backup.exists());
        // 只移动一个文件；不移动含打开子文件的old/host或old父目录。
        fs::rename(&source, &backup).expect("guarded_file_to_backup");
        assert!(!source.exists());
        assert_eq!(fs::read(&backup).unwrap(), bytes);
        let mut through_guard = guards[0].try_clone().unwrap();
        through_guard.seek(SeekFrom::Start(0)).unwrap();
        let mut observed = Vec::new();
        through_guard.read_to_end(&mut observed).unwrap();
        assert_eq!(observed, bytes);
        assert_eq!(guards.len(), images.len());
        for guard in &guards {
            assert_eq!(guard.metadata().unwrap().len(), bytes.len() as u64);
        }
        assert!(!source.exists());
        // 同一组原guard与其clone仍存活，反向恢复单文件；没有先drop保护再重试。
        fs::rename(&backup, &source).expect("guarded_file_restore");
        assert!(!backup.exists());
        assert_eq!(fs::read(&source).unwrap(), bytes);
        through_guard.seek(SeekFrom::Start(0)).unwrap();
        observed.clear();
        through_guard.read_to_end(&mut observed).unwrap();
        assert_eq!(observed, bytes);
        for relative in images {
            assert_eq!(fs::read(old.join(relative)).unwrap(), bytes);
        }
        assert_eq!(fs::read(old.join("identity")).unwrap(), b"old");
        assert_eq!(fs::read(root.join("new/identity")).unwrap(), b"new");
        assert_eq!(guards.len(), images.len());
        drop(through_guard);
        drop(guards);
    }

    #[cfg(windows)]
    #[test]
    fn held_real_guards_mid_file_collision_reverses_completed_only_without_overwrite() {
        let root = fixture();
        let old = root.join("old");
        let stage = root.join(".install-legacy-upgrade-mid-file");
        let new = stage.join("actions/legacy-host-entry");
        fs::create_dir_all(new.parent().unwrap()).unwrap();
        fs::rename(root.join("new"), &new).unwrap();
        let backup = stage.join("old-package");
        let images = ["host/a.dll", "host/b.exe", "runtimes/win-x64/native/c.dll"];
        let bytes = b"owned guarded transaction payload";
        for relative in images {
            let path = old.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        fixture_manifest(&old, &["identity", images[0], images[1], images[2]]);
        let old_manifest = fs::read(old.join("manifest.json")).unwrap();
        let old_pin = files::manifest_digest(&old).unwrap();
        let new_pin = files::manifest_digest(&new).unwrap();
        let guards = retain_image_guards(&old).unwrap();
        assert_eq!(guards.len(), images.len());
        let published = std::cell::Cell::new(false);
        let result = files::switch_with_probe(
            &old,
            &new,
            &backup,
            (&old_pin, &new_pin),
            || {
                published.set(true);
                Ok(())
            },
            |index, target| {
                if index == 1 {
                    assert_eq!(fs::read(backup.join(images[0])).unwrap(), bytes);
                    fs::write(target, b"owned collision sentinel").unwrap(); // 真实目标占用，不伪造返回错误。
                }
                Ok(())
            },
        );
        assert_eq!(result.unwrap_err().code, "legacy_upgrade_switch_failed");
        assert!(!published.get());
        assert_eq!(
            fs::read(backup.join(images[1])).unwrap(),
            b"owned collision sentinel"
        );
        assert!(!backup.join(images[0]).exists());
        for relative in images {
            assert_eq!(fs::read(old.join(relative)).unwrap(), bytes);
        }
        assert_eq!(fs::read(old.join("manifest.json")).unwrap(), old_manifest);
        assert_eq!(fs::read(old.join("identity")).unwrap(), b"old");
        assert_eq!(fs::read(new.join("identity")).unwrap(), b"new");
        let log = fs::read_to_string(stage.join("swap-journal.jsonl")).unwrap();
        let records = log
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            records
                .iter()
                .filter(|r| r["op"] == "old_file_to_backup" && r["phase"] == "done")
                .count(),
            1
        );
        assert_eq!(
            records
                .iter()
                .filter(|r| r["op"] == "restore_old_file" && r["phase"] == "done")
                .count(),
            1
        );
        for guard in &guards {
            assert_eq!(guard.metadata().unwrap().len(), bytes.len() as u64);
        }
        drop(guards); // 整个失败与反向恢复期间保持全部真实保护。
    }

    #[test]
    fn existing_backup_never_overwritten() {
        let root = fixture();
        fs::create_dir(root.join("backup")).unwrap();
        assert!(switch_and_publish(
            &root.join("old"),
            &root.join("new"),
            &root.join("backup"),
            || panic!("不得写索引")
        )
        .is_err());
        assert_eq!(fs::read(root.join("old/identity")).unwrap(), b"old");
    }
    #[test]
    fn pending_marker_blocks_normal_prepare_and_cli_without_reading_payload() {
        let root = fixture();
        let config = root.join("bindings.json");
        write_new(&root.join(MARKER), b"owned pending evidence").unwrap();
        assert_eq!(
            block_config(&config).unwrap_err().code,
            "legacy_upgrade_recovery_required"
        );
        assert_eq!(
            fs::read(root.join(MARKER)).unwrap(),
            b"owned pending evidence"
        );
    }
    #[cfg(windows)]
    #[test]
    fn existing_index_writer_is_rejected_and_crash_free_drop_releases_lock() {
        let root = fixture();
        let config = root.join("bindings.json");
        let first = IndexGuard::acquire(&config).unwrap();
        assert_eq!(
            IndexGuard::acquire(&config).unwrap_err().code,
            "legacy_upgrade_index_busy"
        );
        drop(first);
        let _second = IndexGuard::acquire(&config).unwrap();
    }
    #[test]
    fn owned_input_read_is_bounded_without_truncating_file() {
        let root = fixture();
        let path = root.join("owned.bin");
        fs::write(&path, b"12345").unwrap();
        assert_eq!(
            bounded_read(&path, 4).unwrap_err().code,
            "legacy_upgrade_read_limit"
        );
        assert_eq!(fs::read(&path).unwrap(), b"12345");
    }
    #[cfg(windows)]
    #[test]
    fn running_test_image_cannot_obtain_upgrade_write_access() {
        use std::os::windows::fs::OpenOptionsExt;
        let executable = std::env::current_exe().unwrap();
        // 仅尝试打开本测试进程自身映像，不扫描或操作任何外部PID/窗口，不写字节。
        assert!(OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(7)
            .open(executable)
            .is_err());
    }
    #[cfg(windows)]
    #[test]
    fn latest_disabled_intent_and_other_plugin_settings_survive_upgrade_pin() {
        let root = fixture();
        let config = root.join("bindings.json");
        let mut original = crate::plugins::Plugins::default();
        original.pin_legacy_manifest("a".repeat(64));
        original.configure_legacy_enabled(false);
        original.save(&config).unwrap();
        let before: Value =
            serde_json::from_slice(&fs::read(root.join("plugins.json")).unwrap()).unwrap();
        let guard = IndexGuard::acquire(&config).unwrap();
        write_new(&root.join(MARKER), b"owned pending").unwrap();
        let mut latest = crate::plugins::Plugins::load(&config).unwrap();
        latest.pin_legacy_manifest("b".repeat(64));
        latest.save_for_legacy_upgrade(&config, &guard).unwrap();
        let after: Value =
            serde_json::from_slice(&fs::read(root.join("plugins.json")).unwrap()).unwrap();
        assert!(!latest.legacy_enabled());
        assert_eq!(after["legacy"]["enabled"], false);
        assert_eq!(before["raishot"], after["raishot"]);
        assert_eq!(after["legacy"]["manifest_sha256"], "b".repeat(64));
    }
    #[cfg(windows)]
    #[test]
    fn pending_marker_rejects_normal_plugin_save_and_preserves_original_bytes() {
        let root = fixture();
        let config = root.join("bindings.json");
        let original = crate::plugins::Plugins::default();
        original.save(&config).unwrap();
        let before = fs::read(root.join("plugins.json")).unwrap();
        write_new(&root.join(MARKER), b"owned pending").unwrap();
        assert!(original.save(&config).is_err());
        assert_eq!(fs::read(root.join("plugins.json")).unwrap(), before);
    }
    #[cfg(windows)]
    #[test]
    fn real_locked_index_failure_rolls_package_back_without_rewriting_index() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = fixture();
        let index = root.join("index");
        fs::write(&index, b"old-index").unwrap();
        let held = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&index)
            .unwrap();
        let result = switch_and_publish(
            &root.join("old"),
            &root.join("new"),
            &root.join("backup"),
            || {
                crate::settings::atomic_write(&index, b"new-index")
                    .map_err(|e| fault("index_locked", e))
            },
        );
        assert!(result.is_err());
        drop(held);
        assert_eq!(fs::read(index).unwrap(), b"old-index");
        assert_eq!(fs::read(root.join("old/identity")).unwrap(), b"old");
        assert_eq!(fs::read(root.join("new/identity")).unwrap(), b"new");
    }
    #[test]
    fn rollback_obstruction_is_recovery_required_and_retains_both_packages() {
        let root = fixture();
        let result = switch_and_publish(
            &root.join("old"),
            &root.join("new"),
            &root.join("backup"),
            || {
                fs::create_dir(root.join("new")).unwrap();
                fs::write(root.join("new/owned-blocker"), b"owned obstruction").unwrap();
                Err(fault("injected", "fail"))
            },
        );
        assert_eq!(result.unwrap_err().code, "legacy_upgrade_recovery_required");
        assert_eq!(fs::read(root.join("backup/identity")).unwrap(), b"old");
        assert_eq!(fs::read(root.join("old/identity")).unwrap(), b"new");
        assert_eq!(
            fs::read(root.join("new/owned-blocker")).unwrap(),
            b"owned obstruction"
        );
    }
    #[test]
    fn explicit_upgrade_cli_without_config_rejects_before_worker_or_default_profile() {
        let root = fixture();
        let descriptor = root.join("owned-source.json");
        let value = json!({"archive":{"kind":"https","url":"https://github.com/"},
            "archive_bytes":1,"archive_sha256":"a".repeat(64),"manifest_sha256":"b".repeat(64),
            "upgrade":{"expected_old_manifest":"c".repeat(64),"source_root":root.join("owned-source"),
                "source_database_sha256":"d".repeat(64)}});
        fs::write(&descriptor, serde_json::to_vec(&value).unwrap()).unwrap();
        let args = vec![
            "owned-test".into(),
            "--install-legacy-package".into(),
            descriptor.to_string_lossy().into_owned(),
        ];
        let result = crate::legacy_local_source::cli(&args).unwrap();
        assert_eq!(result.unwrap_err().code, "invalid_cli_arguments");
        assert!(!root.join("actions").exists()); // 此合同不能访问URL、启动worker或默认个人配置。
    }
}
