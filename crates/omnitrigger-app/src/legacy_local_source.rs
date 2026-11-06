//! 本机确定制品适配。复用现有插件清单形状/路径检查/SHA-256，不运行宿主或创建新收据协议。
use crate::{
    legacy_load_gate::LegacyKey,
    legacy_plugin_work::{LegacyArtifactSource, VerifiedLegacyInstall, PACKAGE_ID},
    plugins::Plugins,
};
use omnitrigger_actions::{
    ActionCall, ActionDefinition, ActionError, CancellationToken, PackageManifest, ResolvedAction,
};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

const FOLDER: &str = "legacy-host-entry";
const MAX_FILES: usize = 1023;
const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 1024 * 1024 * 1024;

// 与plugin_discovery.rs / plugin_install.ps1已有形状相同；不是ScriptRuntime的product/engine清单。
#[derive(Deserialize)]
struct Manifest {
    schema_version: u32,
    package_version: String,
    architecture: String,
    files: Vec<Record>,
}
#[derive(Deserialize)]
struct Record {
    path: String,
    bytes: u64,
    sha256: String,
}

#[derive(Debug)]
pub(crate) struct LocalLegacySource {
    root: PathBuf,
    digest: String,
}
fn fault(code: &str, message: impl Into<String>) -> ActionError {
    let mut error = ActionError::new(code, message);
    error.effect_state = Some("not_started".into());
    error
}
fn valid_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
fn checked(cancel: &CancellationToken) -> Result<(), ActionError> {
    if cancel.is_cancelled() {
        Err(fault("cancelled", "本机宿主校验已取消。"))
    } else {
        Ok(())
    }
}
fn local(path: &Path) -> Result<(), ActionError> {
    crate::script_runtime::check_local_path(path)
        .map_err(|message| fault("legacy_invalid_path", message))
}
fn read(path: &Path, limit: u64, cancel: &CancellationToken) -> Result<Vec<u8>, ActionError> {
    checked(cancel)?;
    local(path)?;
    let file =
        File::open(path).map_err(|error| fault("legacy_package_missing", error.to_string()))?;
    let metadata = file
        .metadata()
        .map_err(|error| fault("legacy_package_unreadable", error.to_string()))?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(fault(
            "legacy_package_invalid",
            "宿主包文件类型或长度无效。",
        ));
    }
    let mut data = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut data)
        .map_err(|error| fault("legacy_package_unreadable", error.to_string()))?;
    checked(cancel)?;
    if data.len() as u64 > limit {
        return Err(fault("legacy_package_invalid", "宿主包文件长度超限。"));
    }
    Ok(data)
}
impl LocalLegacySource {
    pub(crate) fn new(config: &Path, digest: &str) -> Result<Self, ActionError> {
        if !valid_hash(digest) {
            return Err(fault(
                "legacy_invalid_manifest_digest",
                "宿主清单摘要无效。",
            ));
        }
        let config = std::path::absolute(config)
            .map_err(|error| fault("legacy_invalid_path", error.to_string()))?;
        Ok(Self {
            root: crate::action_packages::root(&config).join(FOLDER),
            digest: digest.to_ascii_lowercase(),
        })
    }
}

/// 启动/安装完成时只组装既有定位信息；完整读取与哈希在原plugin worker中进行。
pub(crate) fn locate(
    config: &Path,
    plugins: &Plugins,
) -> Result<Option<Arc<dyn LegacyArtifactSource>>, ActionError> {
    plugins
        .legacy_manifest_sha256()
        .map(|digest| {
            LocalLegacySource::new(config, digest)
                .map(|source| Arc::new(source) as Arc<dyn LegacyArtifactSource>)
        })
        .transpose()
}

impl LegacyArtifactSource for LocalLegacySource {
    fn key(&self) -> LegacyKey {
        LegacyKey {
            package_id: PACKAGE_ID.into(),
            artifact_fingerprint: self.digest.clone(),
            fixed_scope: "default".into(),
        }
    }
    fn verify(
        &self,
        config: &Path,
        cancel: &CancellationToken,
    ) -> Result<VerifiedLegacyInstall, ActionError> {
        checked(cancel)?;
        if Self::new(config, &self.digest)?.root != self.root {
            return Err(fault(
                "legacy_package_scope_mismatch",
                "宿主包不属于本次配置的动作目录。",
            ));
        }
        let bytes = read(&self.root.join("manifest.json"), 256 * 1024, cancel)?;
        if format!("{:x}", Sha256::digest(&bytes)) != self.digest {
            return Err(fault("legacy_manifest_changed", "宿主包清单摘要不符。"));
        }
        let manifest: Manifest = serde_json::from_slice(&bytes)
            .map_err(|error| fault("legacy_manifest_invalid", error.to_string()))?;
        if manifest.schema_version != 1
            || manifest.architecture != "windows-x64"
            || manifest.package_version.is_empty()
            || manifest.package_version.len() > 64
            || manifest.files.is_empty()
            || manifest.files.len() > MAX_FILES
        {
            return Err(fault(
                "legacy_manifest_invalid",
                "宿主包版本、平台或文件数量无效。",
            ));
        }
        let mut expected = BTreeSet::new();
        let mut folded = BTreeSet::new();
        let mut total = 0u64;
        for record in &manifest.files {
            if !crate::script_runtime::relative(&record.path)
                || record.path.eq_ignore_ascii_case("manifest.json")
                || !valid_hash(&record.sha256)
                || record.bytes > MAX_FILE_BYTES
                || !folded.insert(record.path.to_ascii_lowercase())
            {
                return Err(fault(
                    "legacy_manifest_invalid",
                    "宿主包清单含非法、重复或过大的文件。",
                ));
            }
            total = total
                .checked_add(record.bytes)
                .ok_or_else(|| fault("legacy_manifest_invalid", "宿主包总长度溢出。"))?;
            expected.insert(record.path.clone());
        }
        if total > MAX_TOTAL_BYTES {
            return Err(fault("legacy_manifest_invalid", "宿主包总长度超限。"));
        }
        for required in [
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
            if !expected.contains(required) {
                return Err(fault(
                    "legacy_package_incomplete",
                    format!("宿主包缺少必要文件：{required}"),
                ));
            }
        }
        // 沿现有runtime完整性模式核对闭集；不跟随链接，不扫描动作根以外目录。
        expected.insert("manifest.json".into());
        let mut pending = vec![self.root.clone()];
        let mut found = BTreeSet::new();
        let mut entries = 0usize;
        while let Some(folder) = pending.pop() {
            checked(cancel)?;
            local(&folder)?;
            for entry in fs::read_dir(folder)
                .map_err(|error| fault("legacy_package_unreadable", error.to_string()))?
            {
                let entry =
                    entry.map_err(|error| fault("legacy_package_unreadable", error.to_string()))?;
                entries += 1;
                if entries > 2048 {
                    return Err(fault("legacy_package_invalid", "宿主包目录项目数量超限。"));
                }
                let path = entry.path();
                local(&path)?;
                let metadata = fs::symlink_metadata(&path)
                    .map_err(|error| fault("legacy_package_unreadable", error.to_string()))?;
                if metadata.is_dir() {
                    pending.push(path);
                } else {
                    let name = path
                        .strip_prefix(&self.root)
                        .map_err(|_| fault("legacy_invalid_path", "宿主包路径越界。"))?
                        .to_string_lossy()
                        .replace('\\', "/");
                    if !metadata.is_file() || !expected.contains(&name) || !found.insert(name) {
                        return Err(fault("legacy_package_changed", "宿主包包含清单外文件。"));
                    }
                }
            }
        }
        if found != expected {
            return Err(fault("legacy_package_incomplete", "宿主包文件索引不完整。"));
        }
        for record in &manifest.files {
            checked(cancel)?;
            let path = self.root.join(&record.path);
            local(&path)?;
            let mut file = File::open(&path)
                .map_err(|error| fault("legacy_package_unreadable", error.to_string()))?;
            let metadata = file
                .metadata()
                .map_err(|error| fault("legacy_package_unreadable", error.to_string()))?;
            if !metadata.is_file() || metadata.len() != record.bytes {
                return Err(fault(
                    "legacy_package_changed",
                    format!("宿主文件长度不符：{}", record.path),
                ));
            }
            let mut hash = Sha256::new();
            let mut count = 0u64;
            let mut buffer = [0u8; 65536];
            loop {
                checked(cancel)?;
                let length = file
                    .read(&mut buffer)
                    .map_err(|error| fault("legacy_package_unreadable", error.to_string()))?;
                if length == 0 {
                    break;
                }
                count += length as u64;
                if count > record.bytes {
                    return Err(fault("legacy_package_changed", "宿主文件在读取中变长。"));
                }
                hash.update(&buffer[..length]);
            }
            if count != record.bytes
                || !format!("{:x}", hash.finalize()).eq_ignore_ascii_case(&record.sha256)
            {
                return Err(fault(
                    "legacy_package_changed",
                    format!("宿主文件校验失败：{}", record.path),
                ));
            }
        }
        let catalog: PackageManifest =
            serde_json::from_slice(&read(&self.root.join("action.json"), 256 * 1024, cancel)?)
                .map_err(|error| fault("legacy_action_manifest_invalid", error.to_string()))?;
        if catalog.schema_version != 1
            || catalog.id != PACKAGE_ID
            || catalog.version != manifest.package_version
            || catalog.actions.len() != 2
            || ![
                ("clip-toggle", "clip-toggle.mjs"),
                ("search-toggle", "search-toggle.mjs"),
            ]
            .iter()
            .all(|(id, entry)| {
                catalog.actions.iter().any(|action| {
                    action.id == *id
                        && action.entry == *entry
                        && action.version == 1
                        && action.runtime == "script"
                        && (*id != "clip-toggle" || action.single_instance)
                })
            })
        {
            return Err(fault(
                "legacy_action_manifest_invalid",
                "安装包与两个正式Script入口不一致。",
            ));
        }
        Ok(VerifiedLegacyInstall {
            key: self.key(),
            executable: self.root.join("host/OmniTrigger.LegacyHost.exe"),
            prepare_action: ResolvedAction {
                call: ActionCall {
                    package_id: PACKAGE_ID.into(),
                    action_id: "__prepare".into(),
                    action_version: 1,
                    input: json!({}),
                },
                definition: ActionDefinition {
                    id: "__prepare".into(),
                    title: "内部宿主准备".into(),
                    version: 1,
                    runtime: "script".into(),
                    entry: "prepare-host.mjs".into(),
                    single_instance: true,
                },
                package_root: self.root.clone(),
                entry_path: self.root.join("prepare-host.mjs"),
            },
        })
    }
}

pub(crate) fn verify_cli_action(config: &Path, call: &ActionCall) -> Result<(), ActionError> {
    if call.package_id != PACKAGE_ID {
        return Ok(());
    }
    let plugins = Plugins::load(config)
        .map_err(|error| fault("legacy_settings_invalid", error.to_string()))?;
    verify_enabled_cli(config, &plugins)
}
fn verify_enabled_cli(config: &Path, plugins: &Plugins) -> Result<(), ActionError> {
    if !plugins.legacy_enabled() {
        return Err(fault("legacy_disabled", "历史与聚合窗口插件已停用。"));
    }
    let source = locate(config, plugins)?
        .ok_or_else(|| fault("legacy_package_missing", "宿主安装尚未登记。"))?;
    source.verify(config, &CancellationToken::new())?;
    Ok(())
}

/// 开发登记入口：制品已由本机打包入口放到固定actions目录；完整校验后只写原plugins.json摘要。
pub(crate) fn register(config: &Path, digest: &str) -> Result<String, ActionError> {
    let source = LocalLegacySource::new(config, digest)?;
    let installed = source.verify(config, &CancellationToken::new())?;
    let mut plugins = Plugins::load(config)
        .map_err(|error| fault("legacy_settings_invalid", error.to_string()))?;
    if plugins
        .legacy_manifest_sha256()
        .is_some_and(|old| !old.eq_ignore_ascii_case(&source.digest))
    {
        return Err(fault(
            "legacy_update_requires_installer",
            "开发登记入口不替换已登记版本；请沿既有停用、退出确认和安装切换流程更新。",
        ));
    }
    let enabled = plugins.legacy_enabled();
    plugins.pin_legacy_manifest(source.digest.clone());
    plugins
        .save(config)
        .map_err(|error| fault("legacy_settings_write_failed", error.to_string()))?;
    Ok(json!({"status":"registered","package_id":PACKAGE_ID,"package_root":installed.prepare_action.package_root,
        "manifest_sha256":source.digest,"enabled":enabled,"host_started":false}).to_string())
}

/// 只允许登记命令和可选--config；先验证完整参数，再访问配置或制品，不与其他模式混用。
pub(crate) fn cli(arguments: &[String]) -> Option<Result<String, ActionError>> {
    if !arguments
        .iter()
        .any(|value| value == "--register-legacy-package")
    {
        return None;
    }
    Some((|| {
        let mut digest = None;
        let mut config = None;
        let mut index = 1;
        while index < arguments.len() {
            let value = arguments
                .get(index + 1)
                .ok_or_else(|| fault("invalid_cli_arguments", "登记参数缺少值。"))?;
            match arguments[index].as_str() {
                "--register-legacy-package" if digest.is_none() && valid_hash(value) => {
                    digest = Some(value.as_str())
                }
                "--config" if config.is_none() && !value.starts_with("--") => config = Some(value),
                _ => {
                    return Err(fault(
                        "invalid_cli_arguments",
                        "登记命令不可与其他命令混用或重复。",
                    ))
                }
            }
            index += 2;
        }
        let digest =
            digest.ok_or_else(|| fault("invalid_cli_arguments", "缺少有效的清单摘要。"))?;
        if let Some(config) = config {
            crate::settings::use_path(config.into())
                .map_err(|message| fault("invalid_cli_arguments", message))?;
        }
        register(&crate::settings::path(), digest)
    })())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        root: PathBuf,
        config: PathBuf,
        package: PathBuf,
        digest: String,
    }
    impl Fixture {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "omnitrigger-legacy-local-source-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            local(&root).unwrap();
            fs::create_dir(&root).unwrap();
            let config = root.join("bindings.json");
            let package = crate::action_packages::root(&config).join(FOLDER);
            fs::create_dir_all(package.join("host")).unwrap();
            let catalog = json!({"schema_version":1,"id":PACKAGE_ID,"name":"不执行的本地夹具","version":"fixture","actions":[
                {"id":"clip-toggle","title":"fixture","version":1,"runtime":"script","entry":"clip-toggle.mjs","single_instance":true},
                {"id":"search-toggle","title":"fixture","version":1,"runtime":"script","entry":"search-toggle.mjs"}]});
            fs::write(package.join("action.json"), catalog.to_string()).unwrap();
            for name in [
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
                fs::write(package.join(name), b"non-executable synthetic fixture").unwrap();
            }
            let mut files = vec![];
            let mut folders = vec![package.clone()];
            while let Some(folder) = folders.pop() {
                for entry in fs::read_dir(folder).unwrap() {
                    let path = entry.unwrap().path();
                    if path.is_dir() {
                        folders.push(path);
                    } else {
                        let bytes = fs::read(&path).unwrap();
                        files.push(json!({"path":path.strip_prefix(&package).unwrap().to_str().unwrap().replace('\\',"/"),"bytes":bytes.len(),"sha256":format!("{:x}",Sha256::digest(bytes))}));
                    }
                }
            }
            let manifest=json!({"schema_version":1,"package_version":"fixture","architecture":"windows-x64","files":files}).to_string();
            fs::write(package.join("manifest.json"), &manifest).unwrap();
            Self {
                root,
                config,
                package,
                digest: format!("{:x}", Sha256::digest(manifest.as_bytes())),
            }
        }
        fn source(&self) -> LocalLegacySource {
            LocalLegacySource::new(&self.config, &self.digest).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            if self.root.parent() == Some(std::env::temp_dir().as_path())
                && self
                    .root
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("omnitrigger-legacy-local-source-")
                && local(&self.root).is_ok()
            {
                let _ = fs::remove_dir_all(&self.root);
            }
        }
    }
    #[test]
    fn complete_local_index_resolves_internal_prepare_without_running_host() {
        let f = Fixture::new();
        let installed = f
            .source()
            .verify(&f.config, &CancellationToken::new())
            .unwrap();
        assert_eq!(installed.prepare_action.call.action_id, "__prepare");
        assert_eq!(
            installed.executable,
            f.package.join("host/OmniTrigger.LegacyHost.exe")
        );
    }
    #[test]
    fn unlisted_file_and_changed_image_are_rejected() {
        let f = Fixture::new();
        fs::write(f.package.join("extra.txt"), b"fixture").unwrap();
        assert_eq!(
            f.source()
                .verify(&f.config, &CancellationToken::new())
                .unwrap_err()
                .code,
            "legacy_package_changed"
        );
        fs::remove_file(f.package.join("extra.txt")).unwrap();
        fs::write(
            f.package.join("host/OmniTrigger.LegacyHost.exe"),
            b"changed",
        )
        .unwrap();
        assert_eq!(
            f.source()
                .verify(&f.config, &CancellationToken::new())
                .unwrap_err()
                .code,
            "legacy_package_changed"
        );
    }
    #[test]
    fn cancellation_precedes_package_access() {
        let source = LocalLegacySource::new(
            &std::env::temp_dir().join("not-created-legacy-config.json"),
            &"a".repeat(64),
        )
        .unwrap();
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert_eq!(
            source
                .verify(
                    &std::env::temp_dir().join("not-created-legacy-config.json"),
                    &cancellation
                )
                .unwrap_err()
                .code,
            "cancelled"
        );
    }
    #[test]
    fn registration_rejects_mixed_mode_before_setting_config_or_reading_package() {
        let args = vec![
            "fixture".into(),
            "--register-legacy-package".into(),
            "a".repeat(64),
            "--run-action".into(),
            "fixture.json".into(),
        ];
        assert_eq!(
            cli(&args).unwrap().unwrap_err().code,
            "invalid_cli_arguments"
        );
    }
    #[test]
    fn disabled_cli_with_a_pin_never_reads_the_package() {
        let mut plugins = Plugins::default();
        plugins.pin_legacy_manifest("a".repeat(64));
        let absent = std::env::temp_dir().join("never-created-legacy-config.json");
        assert_eq!(
            verify_enabled_cli(&absent, &plugins).unwrap_err().code,
            "legacy_disabled"
        );
    }
    #[test]
    fn unrelated_cli_package_does_not_load_legacy_configuration() {
        let absent = std::env::temp_dir().join("never-created-other-config.json");
        let call = ActionCall {
            package_id: "examples.unchanged".into(),
            action_id: "run".into(),
            action_version: 1,
            input: json!({}),
        };
        assert!(verify_cli_action(&absent, &call).is_ok());
    }
}
