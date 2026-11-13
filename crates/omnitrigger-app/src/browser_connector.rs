//! 浏览器连接器的可选安装与本机生命周期；所有进程、磁盘和网络工作均在后台。
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fmt, fs,
    io::Read,
    path::{Path, PathBuf},
    sync::mpsc::Receiver,
};
#[cfg(windows)]
use std::{
    io::Write,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

pub const CONNECTOR_VERSION: &str = "0.1.0";
pub const RELEASE_MANIFEST_URL: &str =
    "https://github.com/ceastld/omnitrigger-browser/releases/download/v0.1.0/release-manifest.json";
#[cfg(windows)]
const SCRIPT: &str = include_str!("browser_connector_install.ps1");
#[cfg(windows)]
static BUSY: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Snapshot {
    pub enabled: bool,
    pub extension_id: String,
    pub version: String,
    pub cli_path: PathBuf,
    pub installed: bool,
    pub registered: bool,
    pub bridge_pid: Option<u32>,
    pub bridge_started_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorError {
    pub code: String,
    pub message: String,
}
impl ConnectorError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}
impl fmt::Display for ConnectorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}
impl std::error::Error for ConnectorError {}

#[derive(Clone, Debug)]
pub enum Operation {
    Install,
    Register { extension_id: String },
    Start,
    Stop,
    Refresh,
    Uninstall,
}

#[derive(Clone, Debug)]
pub enum ConnectorEvent {
    Progress(String),
    Finished {
        snapshot: Snapshot,
        profiles: Vec<Value>,
        message: String,
    },
    Failed(ConnectorError),
}

pub fn state_path(config_path: &Path) -> PathBuf {
    config_path.with_file_name("browser-connector.json")
}
pub fn runtime_root(config_path: &Path) -> PathBuf {
    config_path
        .with_file_name("plugins")
        .join("browser-connector")
        .join("runtime")
}

pub(crate) fn ensure_plain(path: &Path) -> Result<(), ConnectorError> {
    for ancestor in path.ancestors() {
        let metadata = match fs::symlink_metadata(ancestor) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(ConnectorError::new("path_unavailable", error.to_string())),
        };
        #[cfg(windows)]
        let forbidden = {
            use std::os::windows::fs::MetadataExt;
            metadata.file_attributes() & (0x400 | 0x1000 | 0x40000 | 0x400000) != 0
        };
        #[cfg(not(windows))]
        let forbidden = metadata.file_type().is_symlink();
        if forbidden {
            return Err(ConnectorError::new(
                "unsafe_path",
                "连接器目录不能包含链接、云盘占位或离线文件。",
            ));
        }
    }
    if path.components().any(|part| {
        let value = part.as_os_str().to_string_lossy().to_ascii_lowercase();
        value.starts_with("onedrive") || value == "dropbox" || value.starts_with("icloud")
    }) {
        return Err(ConnectorError::new(
            "unsafe_path",
            "连接器请安装在普通本地目录。",
        ));
    }
    Ok(())
}

fn absolute(path: &Path) -> Result<PathBuf, ConnectorError> {
    let path = std::path::absolute(path)
        .map_err(|error| ConnectorError::new("path_unavailable", error.to_string()))?;
    #[cfg(windows)]
    if disk_path_key(&path).is_none() {
        return Err(ConnectorError::new(
            "unsafe_path",
            "连接器仅支持本地磁盘路径。",
        ));
    }
    Ok(path)
}

#[cfg(windows)]
fn disk_path_key(path: &Path) -> Option<String> {
    use std::path::{Component, Prefix};
    let mut parts = path.components();
    let Component::Prefix(prefix) = parts.next()? else {
        return None;
    };
    let (Prefix::Disk(drive) | Prefix::VerbatimDisk(drive)) = prefix.kind() else {
        return None;
    };
    if parts.next() != Some(Component::RootDir) {
        return None;
    }
    let mut key = format!("{}:", char::from(drive));
    for component in parts {
        let Component::Normal(name) = component else {
            return None;
        };
        key.push('\\');
        key.push_str(name.to_str()?);
    }
    Some(key)
}

fn same_cli_path(left: &Path, right: &Path) -> bool {
    #[cfg(windows)]
    {
        disk_path_key(left)
            .zip(disk_path_key(right))
            .is_some_and(|(a, b)| a.eq_ignore_ascii_case(&b))
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn expected_cli(config_path: &Path) -> PathBuf {
    runtime_root(config_path)
        .join(CONNECTOR_VERSION)
        .join("omnitrigger-browser.exe")
}

#[derive(Default)]
struct SavedState {
    snapshot: Snapshot,
    digest: Option<String>,
    inherited: bool,
}

pub fn load(config_path: &Path) -> Result<Snapshot, ConnectorError> {
    read_state(config_path).map(|saved| saved.snapshot)
}

fn read_state(config_path: &Path) -> Result<SavedState, ConnectorError> {
    let config_path = absolute(config_path)?;
    let path = state_path(&config_path);
    ensure_plain(&path)?;
    let bytes = match fs::File::open(&path) {
        Ok(file) => {
            let mut bytes = Vec::new();
            file.take(64 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| ConnectorError::new("settings_read", error.to_string()))?;
            bytes
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(SavedState::default())
        }
        Err(error) => return Err(ConnectorError::new("settings_read", error.to_string())),
    };
    if bytes.len() > 64 * 1024 {
        return Err(ConnectorError::new(
            "settings_invalid",
            "浏览器连接器配置超过大小限制，原文件已保留。",
        ));
    }
    let mut state: Snapshot = serde_json::from_slice(&bytes).map_err(|error| {
        ConnectorError::new(
            "settings_invalid",
            format!("浏览器连接器配置有误，原文件已保留：{error}"),
        )
    })?;
    if !state.version.is_empty() && state.version != CONNECTOR_VERSION {
        return Err(ConnectorError::new(
            "version_incompatible",
            "已保存的连接器版本不兼容，配置已保留。",
        ));
    }
    if !state.extension_id.is_empty() && !valid_extension_id(&state.extension_id) {
        return Err(ConnectorError::new(
            "settings_invalid",
            "已保存的 Chrome 扩展 ID 无效。",
        ));
    }
    let expected = expected_cli(&config_path);
    let digest = Some(format!("{:x}", Sha256::digest(&bytes)));
    if !state.cli_path.as_os_str().is_empty() && !same_cli_path(&state.cli_path, &expected) {
        let inherited = state.version == CONNECTOR_VERSION
            && crate::profiles::seed_sources_for_config(&config_path)
                .iter()
                .any(|source| {
                    same_cli_path(
                        &state.cli_path,
                        &expected_cli(&source.join("bindings.json")),
                    )
                });
        if inherited {
            return Ok(SavedState {
                snapshot: Snapshot::default(),
                digest,
                inherited: true,
            });
        }
        return Err(ConnectorError::new(
            "settings_invalid",
            "连接器程序路径不在版本安装目录内。",
        ));
    }
    ensure_plain(&expected)?;
    ensure_plain(&expected.with_file_name("omnitrigger-browser-host.exe"))?;
    state.installed = !state.cli_path.as_os_str().is_empty()
        && expected.is_file()
        && expected
            .with_file_name("omnitrigger-browser-host.exe")
            .is_file();
    if !state.cli_path.as_os_str().is_empty() {
        state.cli_path = expected;
    }
    Ok(SavedState {
        snapshot: state,
        digest,
        inherited: false,
    })
}

pub fn valid_extension_id(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|byte| (b'a'..=b'p').contains(&byte))
}

/// 同时只接纳一个连接器管理操作。返回值供应用事件循环非阻塞读取。
pub fn start(
    config_path: PathBuf,
    operation: Operation,
) -> Result<Receiver<ConnectorEvent>, ConnectorError> {
    if let Operation::Register { extension_id } = &operation {
        if !valid_extension_id(extension_id) {
            return Err(ConnectorError::new(
                "invalid_extension_id",
                "Chrome 扩展 ID 应为 32 个 a 到 p 的小写字母。",
            ));
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (config_path, operation);
        Err(ConnectorError::new(
            "unsupported_platform",
            "浏览器连接器目前仅支持 Windows。",
        ))
    }
    #[cfg(windows)]
    {
        if BUSY.swap(true, Ordering::AcqRel) {
            return Err(ConnectorError::new(
                "busy",
                "浏览器连接器正在处理上一次操作。",
            ));
        }
        let (sender, receiver) = mpsc::sync_channel(16);
        let spawn = thread::Builder::new()
            .name("omnitrigger-browser-connector".into())
            .spawn(move || {
                struct Guard;
                impl Drop for Guard {
                    fn drop(&mut self) {
                        BUSY.store(false, Ordering::Release);
                    }
                }
                let _guard = Guard;
                let result = run(&config_path, operation, |event| {
                    if matches!(event, ConnectorEvent::Progress(_)) {
                        let _ = sender.try_send(event);
                    } else {
                        let _ = sender.send(event);
                    }
                });
                if let Err(error) = result {
                    let _ = sender.send(ConnectorEvent::Failed(error));
                }
            });
        if let Err(error) = spawn {
            BUSY.store(false, Ordering::Release);
            return Err(ConnectorError::new("worker_start", error.to_string()));
        }
        Ok(receiver)
    }
}

#[cfg(any(windows, test))]
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ScriptEvent {
    Progress {
        message: String,
    },
    Finished {
        snapshot: Snapshot,
        profiles: Vec<Value>,
        message: String,
    },
    Failed {
        code: String,
        message: String,
    },
}

#[cfg(windows)]
fn run(
    config_path: &Path,
    operation: Operation,
    mut event: impl FnMut(ConnectorEvent),
) -> Result<(), ConnectorError> {
    use std::os::windows::process::CommandExt;
    let config_path = absolute(config_path)?;
    let root = runtime_root(&config_path);
    ensure_plain(&root)?;
    let state_file = state_path(&config_path);
    ensure_plain(&state_file.with_file_name("browser-connector.json.lock"))?;
    let _lock = crate::settings::lock_config(&state_file, "浏览器连接器")
        .map_err(|message| ConnectorError::new("state_busy", message))?;
    let saved = read_state(&config_path)?;
    fs::create_dir_all(&root)
        .map_err(|error| ConnectorError::new("install_directory", error.to_string()))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| ConnectorError::new("install_directory", error.to_string()))?
        .as_nanos();
    let stage = root.join(format!(".install-{}-{stamp}", std::process::id()));
    fs::create_dir(&stage)
        .map_err(|error| ConnectorError::new("install_directory", error.to_string()))?;
    let script_path = stage.join("connector.ps1");
    fs::File::create(&script_path)
        .and_then(|mut file| file.write_all(format!("\u{feff}{SCRIPT}").as_bytes()))
        .map_err(|error| ConnectorError::new("installer_prepare", error.to_string()))?;
    let mode = match &operation {
        Operation::Install => "install",
        Operation::Register { .. } => "register",
        Operation::Start => "start",
        Operation::Stop => "stop",
        Operation::Refresh => "refresh",
        Operation::Uninstall => "uninstall",
    };
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
        .arg(&script_path)
        .env("OMNITRIGGER_BROWSER_OPERATION", mode)
        .env("OMNITRIGGER_BROWSER_ROOT", &root)
        .env("OMNITRIGGER_BROWSER_STAGE", &stage)
        .env("OMNITRIGGER_BROWSER_STATE", &state_file)
        .env(
            "OMNITRIGGER_BROWSER_STATE_HASH",
            saved.digest.as_deref().unwrap_or("absent"),
        )
        .env(
            "OMNITRIGGER_BROWSER_INHERITED",
            if saved.inherited { "1" } else { "0" },
        )
        .env("OMNITRIGGER_BROWSER_VERSION", CONNECTOR_VERSION)
        .env("OMNITRIGGER_BROWSER_RELEASE_MANIFEST", RELEASE_MANIFEST_URL)
        .env_remove("OMNITRIGGER_BROWSER_EXTENSION_ID")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(0x0800_0000);
    if let Operation::Register { extension_id } = operation {
        command.env("OMNITRIGGER_BROWSER_EXTENSION_ID", extension_id);
    }
    let mut child = command
        .spawn()
        .map_err(|error| ConnectorError::new("installer_start", error.to_string()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ConnectorError::new("installer_io", "安装器输出不可用。"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| ConnectorError::new("installer_io", "安装器错误输出不可用。"))?;
    let (output_tx, output_rx) = mpsc::channel();
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = output_tx.send(result);
    });
    thread::spawn(move || {
        let _ = std::io::copy(&mut stderr.take(64 * 1024), &mut std::io::sink());
    });
    let began = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => return Err(ConnectorError::new("installer_wait", error.to_string())),
        }
        if began.elapsed() > Duration::from_secs(240) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ConnectorError::new(
                "installer_timeout",
                "连接器操作超时，未自动重试。",
            ));
        }
        thread::sleep(Duration::from_millis(30));
    };
    let bytes = output_rx
        .recv_timeout(Duration::from_secs(3))
        .map_err(|_| ConnectorError::new("installer_io", "无法读取安装回执。"))?
        .map_err(|error| ConnectorError::new("installer_io", error.to_string()))?;
    if bytes.len() > 1024 * 1024 {
        return Err(ConnectorError::new("installer_io", "安装器返回过大回执。"));
    }
    let output = String::from_utf8(bytes)
        .map_err(|_| ConnectorError::new("installer_io", "安装器回执编码错误。"))?;
    let mut finished = false;
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        let decoded: ScriptEvent = serde_json::from_str(line.trim_start_matches('\u{feff}'))
            .map_err(|_| ConnectorError::new("installer_io", "安装器返回了无效回执。"))?;
        match decoded {
            ScriptEvent::Progress { message } => event(ConnectorEvent::Progress(message)),
            ScriptEvent::Finished {
                snapshot,
                profiles,
                message,
            } => {
                finished = true;
                event(ConnectorEvent::Finished {
                    snapshot,
                    profiles,
                    message,
                });
            }
            ScriptEvent::Failed { code, message } => {
                return Err(ConnectorError::new(&code, message))
            }
        }
    }
    if !status.success() || !finished {
        return Err(ConnectorError::new(
            "installer_failed",
            "连接器操作未完成，原有配置已保留。",
        ));
    }
    // 暂存路径明确归属本次操作。只删除脚本和下载临时包，不递归碰用户目录。
    for name in ["connector.ps1", "release-manifest.json", "connector.zip"] {
        let _ = fs::remove_file(stage.join(name));
    }
    let _ = fs::remove_dir(&stage);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_extension_identity_without_rewriting() {
        assert!(valid_extension_id("abcdefghijklmnopabcdefghijklmnop"));
        assert!(!valid_extension_id("ABCDEFGHIJKLMNOPABCDEFGHIJKLMNOP"));
        assert!(!valid_extension_id("abcdefghijklmnopabcdefghijklmnox"));
        assert!(!valid_extension_id("../extension"));
    }
    #[test]
    fn state_is_absent_by_default_and_invalid_content_is_preserved() {
        let root = std::env::temp_dir().join(format!(
            "omnitrigger-browser-state-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let config = root.join("bindings.json");
        assert!(!load(&config).unwrap().installed);
        let state = state_path(&config);
        fs::write(&state, "{invalid}").unwrap();
        assert_eq!(load(&config).unwrap_err().code, "settings_invalid");
        assert_eq!(fs::read_to_string(&state).unwrap(), "{invalid}");
        fs::remove_file(state).unwrap();
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn event_preserves_structured_error_and_unicode() {
        let decoded: ScriptEvent = serde_json::from_str(
            r#"{"kind":"failed","code":"checksum_mismatch","message":"校验失败"}"#,
        )
        .unwrap();
        let ScriptEvent::Failed { code, message } = decoded else {
            panic!("应为错误事件");
        };
        assert_eq!(code, "checksum_mismatch");
        assert_eq!(message, "校验失败");
    }

    #[test]
    fn copied_connector_state_discards_source_identity_but_keeps_a_local_install() {
        let root = std::env::temp_dir().join(format!(
            "omnitrigger-connector-copy-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let dest = root.join("profiles").join("t000000000001");
        fs::create_dir_all(&dest).unwrap();
        let config = dest.join("bindings.json");
        let state_file = state_path(&config);
        let fixture = root.join("missing-fixture");
        let catalog = serde_json::json!({"version":1,"profiles":[{"id":"t000000000001","kind":"test","alias":"qa","name":"QA",
            "seed": format!("fixture:{}", fixture.display()),"created_ms":1}]});
        fs::write(root.join("profiles.json"), catalog.to_string()).unwrap();
        let mut state = Snapshot {
            cli_path: expected_cli(&fixture.join("bindings.json")),
            installed: true,
            enabled: true,
            registered: true,
            version: CONNECTOR_VERSION.into(),
            extension_id: "a".repeat(32),
            bridge_pid: Some(4512),
            bridge_started_at: "source".into(),
        };
        let original = serde_json::to_vec(&state).unwrap();
        fs::write(&state_file, &original).unwrap();
        let local = expected_cli(&config);
        fs::create_dir_all(local.parent().unwrap()).unwrap();
        fs::write(&local, b"").unwrap();
        fs::write(local.with_file_name("omnitrigger-browser-host.exe"), b"").unwrap();
        let inherited = read_state(&config).unwrap();
        assert!(inherited.inherited);
        assert_eq!(
            inherited.digest.unwrap(),
            format!("{:x}", Sha256::digest(&original))
        );
        assert_eq!(
            serde_json::to_value(inherited.snapshot).unwrap(),
            serde_json::to_value(Snapshot::default()).unwrap()
        );
        assert_eq!(fs::read(&state_file).unwrap(), original);
        assert!(!fixture.exists());

        state.cli_path = expected_cli(&root.join("unrelated").join("bindings.json"));
        fs::write(&state_file, serde_json::to_vec(&state).unwrap()).unwrap();
        assert_eq!(load(&config).unwrap_err().code, "settings_invalid");
        state.cli_path = local.clone();
        fs::write(&state_file, serde_json::to_vec(&state).unwrap()).unwrap();
        let own = read_state(&config).unwrap();
        assert!(!own.inherited && own.snapshot.installed);
        assert_eq!(own.snapshot.cli_path, local);
        assert_eq!(own.snapshot.bridge_pid, Some(4512));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn owned_cli_accepts_only_equivalent_local_drive_paths() {
        let own = Path::new(r"C:\work\qa\cli.exe");
        assert!(same_cli_path(Path::new(r"\\?\c:\WORK\QA\CLI.exe"), own));
        for other in [
            r"c:work\qa\cli.exe",
            r"C:\work\qa\..\qa\cli.exe",
            r"\\server\share\work\qa\cli.exe",
            r"\\?\UNC\server\share\work\qa\cli.exe",
            r"\\.\C:\work\qa\cli.exe",
        ] {
            assert!(!same_cli_path(Path::new(other), own));
        }
    }
}
