//! 既有plugin_install worker的首次安装字节叶；返回现有验证收据，由UI事件泵登记。
use super::{absolute, ensure_plain_ancestors, InstallEvent, TRANSPORT_SCRIPT};
use crate::{
    legacy_local_source::LocalLegacySource,
    legacy_plugin_work::{LegacyArtifactSource, VerifiedLegacyInstall},
};
use omnitrigger_actions::{ActionError, CancellationToken};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::SyncSender,
    },
    time::{SystemTime, UNIX_EPOCH},
};

const EXTRACT_SCRIPT: &str = include_str!("legacy_package_install.ps1");
const MAX_DESCRIPTOR_BYTES: u64 = 64 * 1024;

/// 普通制品描述，来自调用者；不保存另一份启用/授权/收据状态。
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LegacyInstallSource {
    pub archive: LegacyArchive,
    pub archive_bytes: u64,
    pub archive_sha256: String,
    pub manifest_sha256: String,
    #[serde(default)]
    pub upgrade: Option<UpgradeMigration>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpgradeMigration {
    pub expected_old_manifest: String,
    pub source_root: PathBuf,
    pub source_database_sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum LegacyArchive {
    LocalArchive { path: PathBuf },
    Https { url: String },
}

fn fault(code: &str, message: impl Into<String>) -> ActionError {
    ActionError::new(code, message)
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|ch| ch.is_ascii_digit() || (b'a'..=b'f').contains(&ch))
}

// 与共用Download-Public相同的HTTPS来源子集；重定向仍由原传输逐跳校验。
fn trusted_https_descriptor(value: &str) -> bool {
    if value.len() > 8192
        || value
            .bytes()
            .any(|b| !b.is_ascii_alphanumeric() && !b"-._~:/?[]@!$&'()*+,;=%".contains(&b))
    {
        return false;
    }
    let Some(rest) = value.strip_prefix("https://") else {
        return false;
    };
    let authority = rest.split(['/', '?']).next().unwrap_or("");
    let hostname = authority.strip_suffix(":443").unwrap_or(authority);
    if ![
        "github.com",
        "release-assets.githubusercontent.com",
        "objects.githubusercontent.com",
    ]
    .iter()
    .any(|allowed| hostname.eq_ignore_ascii_case(allowed))
    {
        return false;
    }
    let bytes = value.as_bytes();
    bytes.iter().enumerate().all(|(i, b)| {
        *b != b'%'
            || (bytes.get(i + 1).is_some_and(u8::is_ascii_hexdigit)
                && bytes.get(i + 2).is_some_and(u8::is_ascii_hexdigit))
    })
}

impl LegacyInstallSource {
    pub(crate) fn validate(&self) -> Result<(), ActionError> {
        if self.archive_bytes == 0
            || self.archive_bytes > 1024 * 1024 * 1024
            || !valid_hash(&self.archive_sha256)
            || !valid_hash(&self.manifest_sha256)
        {
            return Err(fault(
                "legacy_install_source_invalid",
                "宿主制品大小或摘要无效。",
            ));
        }
        if let Some(upgrade) = &self.upgrade {
            if !valid_hash(&upgrade.expected_old_manifest)
                || !valid_hash(&upgrade.source_database_sha256)
                || !upgrade.source_root.is_absolute()
            {
                return Err(fault(
                    "legacy_upgrade_request_invalid",
                    "升级必须明确旧索引、旧数据根及数据库摘要。",
                ));
            }
            ensure_plain_ancestors(&upgrade.source_root)
                .map_err(|e| fault("legacy_upgrade_path_invalid", e))?;
        }
        match &self.archive {
            LegacyArchive::LocalArchive { path } => {
                if !path.is_absolute() {
                    return Err(fault(
                        "legacy_install_source_invalid",
                        "本地制品必须是显式绝对路径。",
                    ));
                }
                ensure_plain_ancestors(path)
                    .map_err(|e| fault("legacy_install_source_invalid", e))?;
            }
            LegacyArchive::Https { url } => {
                if !trusted_https_descriptor(url) {
                    return Err(fault(
                        "legacy_install_source_invalid",
                        "宿主下载描述不是现有可信HTTPS来源或URI无效。",
                    ));
                }
            }
        }
        Ok(())
    }
}

/// CLI只读前置校验：坏描述/本地输入在改配置或启动worker之前拒绝。
/// 不搜索文件，不访问URL，不从环境补默认描述。
pub(crate) fn read_descriptor(path: &Path) -> Result<LegacyInstallSource, ActionError> {
    let path = absolute(path).map_err(|e| fault("legacy_install_descriptor_invalid", e))?;
    ensure_plain_ancestors(&path).map_err(|e| fault("legacy_install_descriptor_invalid", e))?;
    let metadata = fs::metadata(&path)
        .map_err(|e| fault("legacy_install_descriptor_invalid", e.to_string()))?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_DESCRIPTOR_BYTES {
        return Err(fault(
            "legacy_install_descriptor_invalid",
            "制品描述必须是非空且不超过64KiB的普通JSON文件。",
        ));
    }
    let mut options = fs::File::options();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    let file = options
        .open(&path)
        .map_err(|e| fault("legacy_install_descriptor_invalid", e.to_string()))?;
    let mut bytes = Vec::new();
    file.take(MAX_DESCRIPTOR_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| fault("legacy_install_descriptor_invalid", e.to_string()))?;
    if bytes.len() as u64 != metadata.len() {
        return Err(fault(
            "legacy_install_descriptor_invalid",
            "读取期间制品描述发生变化或超限。",
        ));
    }
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
    let source: LegacyInstallSource = serde_json::from_slice(bytes)
        .map_err(|e| fault("legacy_install_descriptor_invalid", e.to_string()))?;
    source.validate()?;
    // 本地文件先只读核对，错误大小/摘要也不触发目录、配置或传输副作用。
    if let LegacyArchive::LocalArchive { path } = &source.archive {
        let metadata = fs::metadata(path)
            .map_err(|e| fault("legacy_install_source_invalid", e.to_string()))?;
        if !metadata.is_file() || metadata.len() != source.archive_bytes {
            return Err(fault(
                "legacy_install_source_invalid",
                "本地制品不是普通文件或大小不符。",
            ));
        }
        let mut options = fs::File::options();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1);
        }
        let mut file = options
            .open(path)
            .map_err(|e| fault("legacy_install_source_invalid", e.to_string()))?;
        let mut hasher = Sha256::new();
        let mut total = 0u64;
        let mut buffer = [0u8; 65536];
        loop {
            let count = file
                .read(&mut buffer)
                .map_err(|e| fault("legacy_install_source_invalid", e.to_string()))?;
            if count == 0 {
                break;
            }
            total += count as u64;
            if total > source.archive_bytes {
                return Err(fault(
                    "legacy_install_source_invalid",
                    "本地制品读取大小超限。",
                ));
            }
            hasher.update(&buffer[..count]);
        }
        if total != source.archive_bytes
            || format!("{:x}", hasher.finalize()) != source.archive_sha256
        {
            return Err(fault(
                "legacy_install_source_invalid",
                "本地制品SHA-256或大小不符。",
            ));
        }
    }
    Ok(source)
}

pub(super) fn install(
    config: &Path,
    source: Option<&LegacyInstallSource>,
    events: &SyncSender<InstallEvent>,
) -> Result<VerifiedLegacyInstall, ActionError> {
    // 无来源在任何配置/磁盘访问前结束；不从环境变量拼接URL或选择本机包。
    let source = source
        .ok_or_else(|| fault("legacy_install_unavailable", "暂未提供可用的宿主安装来源。"))?;
    source.validate()?;
    let config = absolute(config).map_err(|e| fault("legacy_install_path_invalid", e))?;
    ensure_plain_ancestors(&config).map_err(|e| fault("legacy_install_path_invalid", e))?;
    crate::legacy_upgrade::block_config(&config)?;
    let plugins = crate::plugins::Plugins::load(&config)
        .map_err(|e| fault("legacy_settings_invalid", e.to_string()))?;
    if plugins
        .legacy_manifest_sha256()
        .is_some_and(|old| !old.eq_ignore_ascii_case(&source.manifest_sha256))
    {
        return Err(fault(
            "legacy_update_requires_installer",
            "此入口只安装首次制品或核验同一版本，不能替换已登记版本。",
        ));
    }
    let root = crate::action_packages::root(&config);
    ensure_plain_ancestors(&root).map_err(|e| fault("legacy_install_path_invalid", e))?;
    let destination = root.join("legacy-host-entry");
    if destination
        .try_exists()
        .map_err(|e| fault("legacy_install_path_invalid", e.to_string()))?
    {
        // 不重新下载、不覆盖既有目录；沿现有完整性与登记收据验证。
        return LocalLegacySource::new(&config, &source.manifest_sha256)?
            .verify(&config, &CancellationToken::new());
    }
    fs::create_dir_all(&root).map_err(|e| fault("legacy_install_io", e.to_string()))?;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| fault("legacy_install_io", e.to_string()))?
        .as_nanos();
    let stage = root.join(format!(
        ".install-legacy-{}-{stamp}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&stage).map_err(|e| fault("legacy_install_io", e.to_string()))?;
    let result = (|| {
        acquire(&stage, source, events)?;
        let staged_config = stage.join("bindings.json");
        // 临时布局沿用既有config→actions定位，无独立收据或弱化验证分支。
        let verified = LocalLegacySource::new(&staged_config, &source.manifest_sha256)?;
        verified.verify(&staged_config, &CancellationToken::new())?;
        ensure_plain_ancestors(&destination)
            .map_err(|e| fault("legacy_install_path_invalid", e))?;
        if destination
            .try_exists()
            .map_err(|e| fault("legacy_install_io", e.to_string()))?
        {
            return Err(fault(
                "legacy_install_destination_exists",
                "安装目标已出现，未覆盖现有目录。",
            ));
        }
        fs::rename(stage.join("actions/legacy-host-entry"), &destination)
            .map_err(|e| fault("legacy_install_io", e.to_string()))?;
        // 最终固定目录再核验；只返回既有收据类型，不在线程中写Plugins或启动Host。
        LocalLegacySource::new(&config, &source.manifest_sha256)?
            .verify(&config, &CancellationToken::new())
    })();
    if stage.parent() == Some(root.as_path()) && ensure_plain_ancestors(&stage).is_ok() {
        let _ = fs::remove_dir_all(&stage);
    }
    result
}

// 升级仅复用既有acquire和完整验证；绝不覆盖固定目录，不写索引，不启动宿主。
pub(crate) fn stage_upgrade(
    config: &Path,
    source: &LegacyInstallSource,
    events: &SyncSender<InstallEvent>,
) -> Result<(PathBuf, VerifiedLegacyInstall), ActionError> {
    source.validate()?;
    let config = absolute(config).map_err(|e| fault("legacy_upgrade_path_invalid", e))?;
    ensure_plain_ancestors(&config).map_err(|e| fault("legacy_upgrade_path_invalid", e))?;
    let root = crate::action_packages::root(&config);
    ensure_plain_ancestors(&root).map_err(|e| fault("legacy_upgrade_path_invalid", e))?;
    fs::create_dir_all(&root).map_err(|e| fault("legacy_upgrade_io", e.to_string()))?;
    static NEXT_UPGRADE: AtomicU64 = AtomicU64::new(0);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| fault("legacy_upgrade_io", e.to_string()))?
        .as_nanos();
    let stage = root.join(format!(
        ".install-legacy-upgrade-{}-{stamp}-{}",
        std::process::id(),
        NEXT_UPGRADE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&stage).map_err(|e| fault("legacy_upgrade_io", e.to_string()))?;
    acquire(&stage, source, events)?;
    let staged_config = stage.join("bindings.json");
    let verified = LocalLegacySource::new(&staged_config, &source.manifest_sha256)?
        .verify(&staged_config, &CancellationToken::new())?;
    crate::legacy_local_source::require_explicit_host_contract(
        &verified.prepare_action.package_root,
        &CancellationToken::new(),
    )?;
    Ok((staged_config, verified)) // 成功/失败现场都保留；没有清理用户数据的分支。
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum AcquisitionEvent {
    Progress { message: String },
    Finished,
    Failed { message: String },
}

// 任何解析/IO提前返回也要结束本次传输子进程，不运行包内程序。
struct AcquisitionProcess(std::process::Child);
impl Drop for AcquisitionProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn acquire(
    stage: &Path,
    source: &LegacyInstallSource,
    events: &SyncSender<InstallEvent>,
) -> Result<(), ActionError> {
    let script = stage.join("installer.ps1");
    fs::write(
        &script,
        format!("\u{feff}{TRANSPORT_SCRIPT}\n{EXTRACT_SCRIPT}"),
    )
    .map_err(|e| fault("legacy_install_io", e.to_string()))?;
    let mut command = Command::new("powershell.exe");
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(script)
        .env("OMNITRIGGER_INSTALL_STAGE", stage)
        .env(
            "OMNITRIGGER_INSTALL_BYTES",
            source.archive_bytes.to_string(),
        )
        .env("OMNITRIGGER_INSTALL_SHA256", &source.archive_sha256)
        .env(
            "OMNITRIGGER_INSTALL_MANIFEST_SHA256",
            &source.manifest_sha256,
        )
        .env_remove("OMNITRIGGER_INSTALL_ARCHIVE")
        .env_remove("OMNITRIGGER_INSTALL_URL")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    match &source.archive {
        LegacyArchive::LocalArchive { path } => {
            command.env("OMNITRIGGER_INSTALL_ARCHIVE", path);
        }
        LegacyArchive::Https { url } => {
            command.env("OMNITRIGGER_INSTALL_URL", url);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = AcquisitionProcess(
        command
            .spawn()
            .map_err(|e| fault("legacy_install_transport", e.to_string()))?,
    );
    let stdout = child
        .0
        .stdout
        .take()
        .ok_or_else(|| fault("legacy_install_transport", "安装器输出不可用。"))?;
    let mut finished = false;
    let mut failure = None;
    for line in BufReader::new(stdout).lines() {
        let line = line.map_err(|e| fault("legacy_install_transport", e.to_string()))?;
        if line.len() > 16 * 1024 {
            return Err(fault("legacy_install_transport", "安装状态超长。"));
        }
        match serde_json::from_str::<AcquisitionEvent>(line.trim_start_matches('\u{feff}')) {
            Ok(AcquisitionEvent::Progress { message }) => {
                let _ = events.send(InstallEvent::LegacyInstallProgress(message));
            }
            Ok(AcquisitionEvent::Finished) if !finished => finished = true,
            Ok(AcquisitionEvent::Failed { message }) => failure = Some(message),
            _ => failure = Some("安装器状态无效。".into()),
        }
    }
    let status = child
        .0
        .wait()
        .map_err(|e| fault("legacy_install_transport", e.to_string()))?;
    if let Some(message) = failure {
        return Err(fault("legacy_archive_install_failed", message));
    }
    if !status.success() || !finished {
        return Err(fault("legacy_archive_install_failed", "安装器未成功完成。"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_descriptor_uses_existing_transport_hosts_and_rejects_invalid_authorities() {
        for value in [
            "https://github.com/",
            "https://github.com:443/",
            "https://objects.githubusercontent.com/?x=%2F",
        ] {
            assert!(trusted_https_descriptor(value));
        }
        for value in [
            "https://",
            "http://github.com/",
            "https://github.com.evil.invalid/",
            "https://github.com@evil.invalid/",
            "https://user@github.com/",
            "https://github.com:444/",
            "https://github.com/%zz",
            "https://github.com/#fragment",
            "https://github.com/has space",
            "https://github.com/\\bad",
        ] {
            assert!(!trusted_https_descriptor(value), "{value}");
        }
    }

    #[test]
    fn absent_source_is_structured_unavailable_before_any_path_access() {
        let (sender, _) = std::sync::mpsc::sync_channel(16);
        let error = install(Path::new("not-an-owned-config"), None, &sender).unwrap_err();
        assert_eq!(error.code, "legacy_install_unavailable");
    }

    #[test]
    fn caller_descriptor_requires_hash_size_and_explicit_transport() {
        let base = serde_json::json!({"archive":{"kind":"https","url":"https://github.com/"},
            "archive_bytes":1,"archive_sha256":"a".repeat(64),"manifest_sha256":"b".repeat(64)});
        let source: LegacyInstallSource = serde_json::from_value(base.clone()).unwrap();
        assert!(source.validate().is_ok());
        for key in ["archive_sha256", "manifest_sha256"] {
            let mut bad = base.clone();
            bad[key] = "bad".into();
            assert!(serde_json::from_value::<LegacyInstallSource>(bad)
                .unwrap()
                .validate()
                .is_err());
        }
        let mut bad = base;
        bad["archive"]["url"] = "http://github.com/".into();
        assert!(serde_json::from_value::<LegacyInstallSource>(bad)
            .unwrap()
            .validate()
            .is_err());
    }

    #[cfg(windows)]
    struct Fixture {
        root: PathBuf,
        config: PathBuf,
        source: LegacyInstallSource,
    }

    #[cfg(windows)]
    impl Fixture {
        fn new(enabled: bool) -> Self {
            use sha2::{Digest, Sha256};
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "omnitrigger-legacy-install-fixture-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            ensure_plain_ancestors(&root).unwrap();
            fs::create_dir(&root).unwrap();
            let input = root.join("input");
            fs::create_dir_all(input.join("host")).unwrap();
            let catalog = serde_json::json!({"schema_version":1,"id":"omnitrigger.legacy","name":"不执行的安装夹具","version":"fixture","actions":[
                {"id":"clip-toggle","title":"fixture","version":1,"runtime":"script","entry":"clip-toggle.mjs","single_instance":true},
                {"id":"search-toggle","title":"fixture","version":1,"runtime":"script","entry":"search-toggle.mjs"}]});
            let mut files = Vec::new();
            for name in [
                "action.json",
                "entry.mjs",
                "ensure-ready.mjs",
                "host-path.mjs",
                "prepare-host.mjs",
                "clip-toggle.mjs",
                "search-toggle.mjs",
                "LICENSE",
                "host/OmniTrigger.LegacyHost.exe",
                "host/OmniTrigger.LegacyHost.deps.json",
                "host/OmniTrigger.LegacyHost.runtimeconfig.json",
            ] {
                let bytes = if name == "action.json" {
                    catalog.to_string().into_bytes()
                } else {
                    b"non-executable synthetic fixture; never run".to_vec()
                };
                fs::write(input.join(name), &bytes).unwrap();
                files.push(serde_json::json!({"path":name,"bytes":bytes.len(),"sha256":format!("{:x}", Sha256::digest(&bytes))}));
            }
            let contract = crate::legacy_local_source::explicit_host_contract()
                .to_string()
                .into_bytes();
            fs::write(input.join("host-contract.json"), &contract).unwrap();
            files.push(serde_json::json!({"path":"host-contract.json","bytes":contract.len(),"sha256":format!("{:x}", Sha256::digest(&contract))}));
            let manifest = serde_json::json!({"schema_version":1,"package_version":"fixture","architecture":"windows-x64","files":files}).to_string();
            fs::write(input.join("manifest.json"), &manifest).unwrap();
            let archive = root.join("fixture.zip");
            let status = Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", r#"Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
$fixtureRoot = $env:FIXTURE_INPUT
$fixtureManifest = [IO.File]::ReadAllText((Join-Path $fixtureRoot 'manifest.json')) | ConvertFrom-Json
$fixtureNames = @($fixtureManifest.files | ForEach-Object { [string]$_.path }) + @('manifest.json')
$fixtureZip = [IO.Compression.ZipFile]::Open($env:FIXTURE_ARCHIVE, [IO.Compression.ZipArchiveMode]::Create)
try {
    foreach ($fixtureName in $fixtureNames) {
        if ($fixtureName.Contains('\') -or $fixtureName.EndsWith('/')) { throw '夹具清单路径必须是canonical文件名。' }
        [void][IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
            $fixtureZip, (Join-Path $fixtureRoot $fixtureName), $fixtureName, [IO.Compression.CompressionLevel]::Optimal)
    }
} finally { $fixtureZip.Dispose() }"#])
                .env("FIXTURE_INPUT", &input).env("FIXTURE_ARCHIVE", &archive).status().unwrap();
            assert!(status.success());
            let bytes = fs::read(&archive).unwrap();
            let config = root.join("bindings.json");
            let mut plugins = crate::plugins::Plugins::load(&config).unwrap();
            plugins.configure_legacy_enabled(enabled);
            plugins.save(&config).unwrap();
            Self {
                root,
                config,
                source: LegacyInstallSource {
                    upgrade: None,
                    archive: LegacyArchive::LocalArchive { path: archive },
                    archive_bytes: bytes.len() as u64,
                    archive_sha256: format!("{:x}", Sha256::digest(bytes)),
                    manifest_sha256: format!("{:x}", Sha256::digest(manifest.as_bytes())),
                },
            }
        }
        fn run(&self) -> Result<(), ActionError> {
            let (sender, _receiver) = std::sync::mpsc::sync_channel(16);
            let installed = install(&self.config, Some(&self.source), &sender)?;
            let mut plugins = crate::plugins::Plugins::load(&self.config).unwrap();
            crate::legacy_local_source::register_verified(&self.config, &installed, &mut plugins)?;
            Ok(())
        }
    }

    #[cfg(windows)]
    impl Drop for Fixture {
        fn drop(&mut self) {
            if self.root.parent() == Some(std::env::temp_dir().as_path())
                && self
                    .root
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("omnitrigger-legacy-install-fixture-")
                && ensure_plain_ancestors(&self.root).is_ok()
            {
                let _ = fs::remove_dir_all(&self.root);
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn upgrade_stage_uses_real_acquire_layout_without_publishing_or_running_host() {
        // 复用既有工厂新建隔离的不可执行合成归档；不重用任何已存在的业务目录。
        let fixture = Fixture::new(false);
        let index = fixture.config.with_file_name("plugins.json");
        let before = fs::read(&index).unwrap();
        let (sender, _receiver) = std::sync::mpsc::sync_channel(16);
        // 直接走真实stage_upgrade -> acquire -> 原PowerShell叶 -> LocalLegacySource校验。
        let (staged_config, verified) =
            stage_upgrade(&fixture.config, &fixture.source, &sender).unwrap();
        let stage = staged_config.parent().unwrap();
        let actions = crate::action_packages::root(&fixture.config);
        assert_eq!(stage.parent(), Some(actions.as_path()));
        assert!(stage
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(".install-legacy-upgrade-"));
        assert!(stage.join("archive.zip").is_file());
        assert_eq!(
            verified.prepare_action.package_root,
            stage.join("actions/legacy-host-entry")
        );
        assert_eq!(
            verified.executable,
            stage.join("actions/legacy-host-entry/host/OmniTrigger.LegacyHost.exe")
        );
        assert_eq!(
            fs::read(&verified.executable).unwrap(),
            b"non-executable synthetic fixture; never run"
        );
        assert!(!actions.join("legacy-host-entry").exists());
        assert!(!fixture
            .config
            .parent()
            .unwrap()
            .join("legacy-host-data")
            .exists());
        assert_eq!(fs::read(index).unwrap(), before);
        assert!(!crate::plugins::Plugins::load(&fixture.config)
            .unwrap()
            .legacy_enabled());
    }

    #[cfg(windows)]
    #[test]
    fn local_install_registers_preserves_enabled_and_rejects_tampering_without_running_host() {
        for enabled in [false, true] {
            let fixture = Fixture::new(enabled);
            fixture.run().unwrap();
            let plugins = crate::plugins::Plugins::load(&fixture.config).unwrap();
            assert_eq!(plugins.legacy_enabled(), enabled);
            assert_eq!(
                plugins.legacy_manifest_sha256(),
                Some(fixture.source.manifest_sha256.as_str())
            );
            fixture.run().unwrap(); // 同一版本只验证，不替换现有目录。
            let settings = fixture.root.join("plugins.json");
            let before = fs::read(&settings).unwrap();
            let image = crate::action_packages::root(&fixture.config)
                .join("legacy-host-entry/host/OmniTrigger.LegacyHost.exe");
            fs::write(&image, b"changed synthetic fixture").unwrap();
            assert!(fixture.run().is_err());
            assert_eq!(fs::read(settings).unwrap(), before);
            assert_eq!(fs::read(image).unwrap(), b"changed synthetic fixture");
        }
    }

    #[cfg(windows)]
    #[test]
    fn different_registered_version_is_refused_before_transport_or_replacement() {
        let fixture = Fixture::new(false);
        let mut plugins = crate::plugins::Plugins::load(&fixture.config).unwrap();
        plugins.pin_legacy_manifest("c".repeat(64));
        plugins.save(&fixture.config).unwrap();
        assert_eq!(
            fixture.run().unwrap_err().code,
            "legacy_update_requires_installer"
        );
        assert!(!crate::action_packages::root(&fixture.config).exists());
    }

    #[cfg(windows)]
    #[test]
    fn first_install_enable_intent_and_later_disable_survive_worker_and_registration() {
        for disable_before_completion in [false, true] {
            let fixture = Fixture::new(false);
            let mut clicked = crate::plugins::Plugins::load(&fixture.config).unwrap();
            assert!(!clicked.legacy_enabled());
            // UI单击安装沿用既有启用字段，写入此刻显式意图。
            clicked.configure_legacy_enabled(true);
            clicked.save(&fixture.config).unwrap();
            let (sender, _receiver) = std::sync::mpsc::sync_channel(16);
            let installed = install(&fixture.config, Some(&fixture.source), &sender).unwrap();
            let after_worker = crate::plugins::Plugins::load(&fixture.config).unwrap();
            assert!(after_worker.legacy_enabled());
            assert!(after_worker.legacy_manifest_sha256().is_none()); // worker不写登记或旧enabled。
            if disable_before_completion {
                let mut stopped = after_worker;
                stopped.configure_legacy_enabled(false);
                stopped.save(&fixture.config).unwrap();
            }
            let mut latest = crate::plugins::Plugins::load(&fixture.config).unwrap();
            crate::legacy_local_source::register_verified(&fixture.config, &installed, &mut latest)
                .unwrap();
            assert_eq!(latest.legacy_enabled(), !disable_before_completion);
            assert_eq!(
                latest.legacy_manifest_sha256(),
                Some(fixture.source.manifest_sha256.as_str())
            );
            assert_eq!(
                crate::plugins::Plugins::load(&fixture.config)
                    .unwrap()
                    .legacy_enabled(),
                !disable_before_completion
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn stale_worker_receipt_cannot_replace_newer_registration_or_disabled_choice() {
        let fixture = Fixture::new(false);
        let (sender, _receiver) = std::sync::mpsc::sync_channel(16);
        let installed = install(&fixture.config, Some(&fixture.source), &sender).unwrap();
        let mut latest = crate::plugins::Plugins::load(&fixture.config).unwrap();
        latest.pin_legacy_manifest("c".repeat(64));
        latest.configure_legacy_enabled(false);
        latest.save(&fixture.config).unwrap();
        let before = fs::read(fixture.root.join("plugins.json")).unwrap();
        assert_eq!(
            crate::legacy_local_source::register_verified(&fixture.config, &installed, &mut latest)
                .unwrap_err()
                .code,
            "legacy_update_requires_installer"
        );
        assert_eq!(fs::read(fixture.root.join("plugins.json")).unwrap(), before);
        assert!(!latest.legacy_enabled());
    }
}
