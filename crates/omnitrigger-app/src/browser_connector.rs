//! 浏览器连接器的可选安装与本机生命周期；所有进程、磁盘和网络工作均在后台。
use serde::{Deserialize, Serialize};
use serde_json::Value;
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
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir()
            .map(|current| current.join(path))
            .map_err(|error| ConnectorError::new("path_unavailable", error.to_string()))
    }
}

pub fn load(config_path: &Path) -> Result<Snapshot, ConnectorError> {
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
            return Ok(Snapshot::default())
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
    let expected = runtime_root(&config_path)
        .join(CONNECTOR_VERSION)
        .join("omnitrigger-browser.exe");
    if !state.cli_path.as_os_str().is_empty() && state.cli_path != expected {
        return Err(ConnectorError::new(
            "settings_invalid",
            "连接器程序路径不在版本安装目录内。",
        ));
    }
    if !state.extension_id.is_empty() && !valid_extension_id(&state.extension_id) {
        return Err(ConnectorError::new(
            "settings_invalid",
            "已保存的 Chrome 扩展 ID 无效。",
        ));
    }
    ensure_plain(&expected)?;
    ensure_plain(&expected.with_file_name("omnitrigger-browser-host.exe"))?;
    state.installed = !state.cli_path.as_os_str().is_empty()
        && state.cli_path.is_file()
        && state
            .cli_path
            .with_file_name("omnitrigger-browser-host.exe")
            .is_file();
    Ok(state)
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
        .env("OMNITRIGGER_BROWSER_STATE", state_path(&config_path))
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
}
