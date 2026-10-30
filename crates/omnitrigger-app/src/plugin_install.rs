//! 插件依赖的后台发现和安装边界。只校验、下载和解包，不运行能力程序。
#[path = "plugin_discovery.rs"]
mod discovery;

use serde::Deserialize;
use std::{
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

pub const RAISHOT_VERSION: &str = "0.4.15";
pub const RAISHOT_DOWNLOAD: &str =
    "https://github.com/ceastld/raishot/releases/download/v0.4.15/RaiShot.0.4.15.0.zip";
const ARCHIVE_BYTES: u64 = 27_203_273;
const ARCHIVE_SHA256: &str = "78894445b71d7e3fa5e028510973ce4bcf57cbde88628e7509928f64aa7abecf";
const MANIFEST_SHA256: &str = "67edadf2f91e7793547c449f016d26f9fdb8288f54401b2bcd16bf70832d97fb";
const INSTALL_SCRIPT: &str = include_str!("plugin_install.ps1");

#[derive(Debug)]
pub enum InstallEvent {
    Progress(String),
    Finished {
        cli_path: PathBuf,
        version: String,
        /// 本次是否新下载；新程序首次启动时系统会检查，需要提示用户。
        downloaded: bool,
    },
    Failed(String),
}

/// OmniTrigger 下载 RaiShot 时使用的共享位置，与 RaiShot 自己的安装约定一致，
/// 其他程序也能找到并复用，避免每个宿主各存一份副本。
pub fn shared_root() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(|base| PathBuf::from(base).join("Programs").join("RaiShot"))
}

/// 早期版本安装在配置目录下的副本，仍作为已有安装复用。
fn legacy_root(config_path: &Path) -> Option<PathBuf> {
    Some(
        absolute(config_path)
            .ok()?
            .parent()?
            .join("plugins")
            .join("raishot")
            .join("runtime"),
    )
}

/// 同步查找本机已有的兼容 RaiShot；只读清单与文件大小，可在界面线程调用。
pub fn locate(config_path: &Path, configured: Option<&Path>) -> Option<discovery::Installed> {
    let legacy = legacy_root(config_path)?;
    discovery::find(configured, shared_root().as_deref(), &legacy)
}

/// 优先接入本机已有安装，找不到时才下载到共享位置。下载与校验在后台线程。
pub fn start_prepare(
    config_path: PathBuf,
    configured_cli: Option<PathBuf>,
) -> Result<Receiver<InstallEvent>, String> {
    let (events, receiver) = mpsc::sync_channel(16);
    thread::Builder::new()
        .name("omnitrigger-plugin-install".into())
        .spawn(move || {
            if let Some(found) = locate(&config_path, configured_cli.as_deref()) {
                let _ = events.send(InstallEvent::Finished {
                    cli_path: found.cli,
                    version: found.version,
                    downloaded: false,
                });
                return;
            }
            let result = shared_root()
                .ok_or_else(|| "无法确定 RaiShot 安装位置。".to_string())
                .and_then(|root| install(&root, None, &events));
            if let Err(message) = result {
                let _ = events.send(InstallEvent::Failed(message));
            }
        })
        .map_err(|error| format!("无法启动插件安装线程：{error}"))?;
    Ok(receiver)
}

/// 离线包也必须匹配适配中固定的官方版本、大小和 SHA-256。
#[cfg(test)]
pub fn start_install_from_archive(
    root: PathBuf,
    archive_path: PathBuf,
) -> Result<Receiver<InstallEvent>, String> {
    let (events, receiver) = mpsc::sync_channel(16);
    thread::Builder::new()
        .name("omnitrigger-plugin-install".into())
        .spawn(move || {
            if let Err(message) = install(&root, Some(&archive_path), &events) {
                let _ = events.send(InstallEvent::Failed(message));
            }
        })
        .map_err(|error| format!("无法启动插件安装线程：{error}"))?;
    Ok(receiver)
}

fn absolute(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir()
            .map(|directory| directory.join(path))
            .map_err(|error| format!("无法确定安装目录：{error}"))
    }
}

fn ensure_plain_ancestors(path: &Path) -> Result<(), String> {
    if discovery::is_cloud_path(path) {
        return Err("插件目录不能位于云盘同步目录。".into());
    }
    // 从根向下检查，避免先访问链接或云端目录内部的子路径。
    for ancestor in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) => {
                #[cfg(windows)]
                let reparse = {
                    use std::os::windows::fs::MetadataExt;
                    metadata.file_attributes() & (0x400 | 0x1000 | 0x40000 | 0x400000) != 0
                };
                #[cfg(not(windows))]
                let reparse = metadata.file_type().is_symlink();
                if reparse {
                    return Err(format!(
                        "插件目录不能包含链接、联接点或云端占位文件：{}",
                        ancestor.display()
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("无法检查插件安装目录：{error}")),
        }
    }
    Ok(())
}

fn install(
    root: &Path,
    archive_path: Option<&Path>,
    events: &SyncSender<InstallEvent>,
) -> Result<(), String> {
    let root = absolute(root)?;
    ensure_plain_ancestors(&root)?;
    fs::create_dir_all(&root).map_err(|error| format!("无法创建插件安装目录：{error}"))?;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("无法生成安装身份：{error}"))?
        .as_nanos();
    let stage = root.join(format!(
        ".install-{}-{stamp}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&stage).map_err(|error| format!("无法创建插件暂存目录：{error}"))?;
    let result = run_script(&root, &stage, archive_path, events);
    // 只清理本次创建的明确子目录，且再次检查链接；清理失败时保留暂存文件。
    if stage.parent() == Some(root.as_path()) && ensure_plain_ancestors(&stage).is_ok() {
        let _ = fs::remove_dir_all(&stage);
    }
    result
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ScriptEvent {
    Progress { message: String },
    Finished { cli_path: PathBuf, version: String },
    Failed { message: String },
}

fn run_script(
    root: &Path,
    stage: &Path,
    archive_path: Option<&Path>,
    events: &SyncSender<InstallEvent>,
) -> Result<(), String> {
    let script = stage.join("installer.ps1");
    // Windows PowerShell 5.1 需要 BOM 才能准确读取中文脚本。
    fs::write(&script, format!("\u{feff}{INSTALL_SCRIPT}"))
        .map_err(|error| format!("无法准备插件安装器：{error}"))?;
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
        .env("OMNITRIGGER_INSTALL_ROOT", root)
        .env("OMNITRIGGER_INSTALL_STAGE", stage)
        .env("OMNITRIGGER_INSTALL_VERSION", RAISHOT_VERSION)
        .env("OMNITRIGGER_INSTALL_URL", RAISHOT_DOWNLOAD)
        .env("OMNITRIGGER_INSTALL_BYTES", ARCHIVE_BYTES.to_string())
        .env("OMNITRIGGER_INSTALL_SHA256", ARCHIVE_SHA256)
        .env("OMNITRIGGER_INSTALL_MANIFEST_SHA256", MANIFEST_SHA256)
        .env_remove("OMNITRIGGER_INSTALL_ARCHIVE")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(archive) = archive_path {
        command.env("OMNITRIGGER_INSTALL_ARCHIVE", absolute(archive)?);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("无法启动插件安装器：{error}"))?;
    let stdout = child.stdout.take().ok_or("插件安装器输出不可用。")?;
    let mut finished = None;
    let mut failure = None;
    for line in BufReader::new(stdout).lines() {
        let line = line.map_err(|error| format!("读取插件安装进度失败：{error}"))?;
        if line.len() > 16 * 1024 {
            let _ = child.kill();
            let _ = child.wait();
            return Err("插件安装器返回过大的状态。".into());
        }
        match serde_json::from_str::<ScriptEvent>(line.trim_start_matches('\u{feff}')) {
            Ok(ScriptEvent::Progress { message }) => {
                let _ = events.send(InstallEvent::Progress(message));
            }
            Ok(ScriptEvent::Finished { cli_path, version }) => finished = Some((cli_path, version)),
            Ok(ScriptEvent::Failed { message }) => failure = Some(message),
            Err(_) => failure = Some("插件安装器返回了无法识别的状态。".into()),
        }
    }
    let status = child
        .wait()
        .map_err(|error| format!("无法读取插件安装器退出状态：{error}"))?;
    if let Some(message) = failure {
        return Err(message);
    }
    if !status.success() {
        return Err("插件安装器意外退出，原有安装已保留。".into());
    }
    let (cli_path, version) = finished.ok_or("插件安装未返回完成结果。")?;
    let expected = root.join(RAISHOT_VERSION).join("raishot-cli.exe");
    if cli_path != expected
        || version != RAISHOT_VERSION
        || ensure_plain_ancestors(&cli_path).is_err()
        || !cli_path.is_file()
    {
        return Err("插件安装结果无效，未更新程序路径。".into());
    }
    let _ = events.send(InstallEvent::Finished {
        cli_path,
        version,
        downloaded: true,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_events_preserve_chinese_and_paths_with_spaces() {
        let event: ScriptEvent = serde_json::from_str(
            r#"{"kind":"finished","cli_path":"C:\\截图 工具\\raishot-cli.exe","version":"0.4.15"}"#,
        )
        .unwrap();
        let ScriptEvent::Finished { cli_path, version } = event else {
            panic!("应返回安装完成事件");
        };
        assert_eq!(cli_path, PathBuf::from(r"C:\截图 工具\raishot-cli.exe"));
        assert_eq!(version, RAISHOT_VERSION);
    }

    #[cfg(windows)]
    struct Fixture {
        base: PathBuf,
        root: PathBuf,
        stage: PathBuf,
    }

    #[cfg(windows)]
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let base = std::env::temp_dir().join(format!(
                "omnitrigger-plugin-install-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let root = base.join("plugins").join("raishot").join("runtime");
            let stage = root.join(".install-fixture");
            fs::create_dir_all(&stage).unwrap();
            Self { base, root, stage }
        }

        fn run(&self, mode: &str) -> std::process::Output {
            let installer = self.stage.join("installer.ps1");
            fs::write(&installer, format!("\u{feff}{INSTALL_SCRIPT}")).unwrap();
            let driver = self.stage.join("fixture.ps1");
            fs::write(&driver, format!("\u{feff}{FIXTURE_SCRIPT}")).unwrap();
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
                .arg(driver)
                .env("OMNITRIGGER_INSTALL_ROOT", &self.root)
                .env("OMNITRIGGER_INSTALL_STAGE", &self.stage)
                .env("OMNITRIGGER_INSTALL_VERSION", RAISHOT_VERSION)
                .env("OMNITRIGGER_TEST_INSTALLER", installer)
                .env("OMNITRIGGER_TEST_MODE", mode);
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
            command.output().unwrap()
        }
    }

    #[cfg(windows)]
    impl Drop for Fixture {
        fn drop(&mut self) {
            if self.base.parent() == Some(std::env::temp_dir().as_path()) {
                let _ = fs::remove_dir_all(&self.base);
            }
        }
    }

    #[cfg(windows)]
    const FIXTURE_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
$fixtureFiles = @('raishot.exe', 'raishot-gpui.exe', 'raishot-cli.exe',
    'vcruntime140.dll', 'vcruntime140_1.dll', 'msvcp140.dll', 'msvcp140_1.dll',
    'onnxruntime.dll', 'onnxruntime_providers_shared.dll', 'LICENSE-ONNXRuntime.txt',
    'THIRD-PARTY-ONNXRuntime.txt', 'README.txt', 'LICENSE', 'THIRD_PARTY_NOTICES.md')
$fixtureRecords = @()
$fixtureContents = @{}
foreach ($fixtureName in $fixtureFiles) {
    $fixtureBytes = [Text.Encoding]::UTF8.GetBytes('fixture:' + $fixtureName)
    $fixtureHasher = [Security.Cryptography.SHA256]::Create()
    try { $fixtureHash = ([BitConverter]::ToString($fixtureHasher.ComputeHash($fixtureBytes))).Replace('-','').ToLowerInvariant() }
    finally { $fixtureHasher.Dispose() }
    $fixtureContents[$fixtureName] = $fixtureBytes
    $fixtureRecords += [ordered]@{path=$fixtureName;bytes=$fixtureBytes.Length;sha256=$fixtureHash}
}
$fixtureManifest = [ordered]@{schema_version=1;package_version='0.4.15';architecture='windows-x64';files=$fixtureRecords} | ConvertTo-Json -Depth 5
$fixtureManifestBytes = [Text.Encoding]::UTF8.GetBytes($fixtureManifest)
$fixtureContents['manifest.json'] = $fixtureManifestBytes
$fixtureManifestHasher = [Security.Cryptography.SHA256]::Create()
try { $env:OMNITRIGGER_INSTALL_MANIFEST_SHA256 = ([BitConverter]::ToString($fixtureManifestHasher.ComputeHash($fixtureManifestBytes))).Replace('-','').ToLowerInvariant() }
finally { $fixtureManifestHasher.Dispose() }
$fixtureArchive = Join-Path $env:OMNITRIGGER_INSTALL_STAGE 'fixture.zip'
if (Test-Path -LiteralPath $fixtureArchive) { [IO.File]::Delete($fixtureArchive) }
$fixtureZip = [IO.Compression.ZipFile]::Open($fixtureArchive, [IO.Compression.ZipArchiveMode]::Create)
try {
    foreach ($fixtureName in $fixtureContents.Keys) {
        $fixtureEntryName = $fixtureName
        if ($env:OMNITRIGGER_TEST_MODE -ceq 'traversal' -and $fixtureName -ceq 'raishot.exe') { $fixtureEntryName = '../outside.exe' }
        $fixtureEntry = $fixtureZip.CreateEntry($fixtureEntryName)
        $fixtureStream = $fixtureEntry.Open()
        try { $fixtureStream.Write($fixtureContents[$fixtureName], 0, $fixtureContents[$fixtureName].Length) }
        finally { $fixtureStream.Dispose() }
    }
} finally { $fixtureZip.Dispose() }
$env:OMNITRIGGER_INSTALL_ARCHIVE = $fixtureArchive
$env:OMNITRIGGER_INSTALL_BYTES = [string](Get-Item -LiteralPath $fixtureArchive).Length
$fixtureArchiveHasher = [Security.Cryptography.SHA256]::Create()
$fixtureArchiveStream = [IO.File]::OpenRead($fixtureArchive)
try { $env:OMNITRIGGER_INSTALL_SHA256 = ([BitConverter]::ToString($fixtureArchiveHasher.ComputeHash($fixtureArchiveStream))).Replace('-','').ToLowerInvariant() }
finally { $fixtureArchiveHasher.Dispose(); $fixtureArchiveStream.Dispose() }
if ($env:OMNITRIGGER_TEST_MODE -ceq 'hash') { $env:OMNITRIGGER_INSTALL_SHA256 = ('0' * 64) }
$fixtureCopy = Join-Path $env:OMNITRIGGER_INSTALL_STAGE 'RaiShot.0.4.15.0.zip'
if (Test-Path -LiteralPath $fixtureCopy) { [IO.File]::Delete($fixtureCopy) }
& $env:OMNITRIGGER_TEST_INSTALLER
exit $LASTEXITCODE
"#;

    #[cfg(windows)]
    #[test]
    fn rejects_wrong_archive_hash_and_preserves_other_installed_versions() {
        let fixture = Fixture::new();
        let previous = fixture.root.join("0.4.14");
        fs::create_dir(&previous).unwrap();
        fs::write(previous.join("raishot-cli.exe"), "previous-runtime").unwrap();
        let output = fixture.run("hash");
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("SHA-256"));
        assert!(!fixture.root.join(RAISHOT_VERSION).exists());
        assert_eq!(
            fs::read_to_string(previous.join("raishot-cli.exe")).unwrap(),
            "previous-runtime"
        );
    }

    #[cfg(windows)]
    #[test]
    fn rejects_traversal_before_extracting_any_payload() {
        let fixture = Fixture::new();
        let output = fixture.run("traversal");
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("不允许的路径"));
        assert!(!fixture.stage.join("outside.exe").exists());
        assert!(!fixture.root.join(RAISHOT_VERSION).exists());
        assert_eq!(
            fs::read_dir(fixture.stage.join("payload")).unwrap().count(),
            0
        );
    }

    #[cfg(windows)]
    #[test]
    fn installs_complete_runtime_and_rejects_modification_without_overwriting() {
        let fixture = Fixture::new();
        let installed = fixture.root.join(RAISHOT_VERSION);
        let output = fixture.run("valid");
        assert!(
            output.status.success(),
            "stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(fs::read_dir(&installed).unwrap().count(), 15);
        assert!(installed.join("raishot-cli.exe").is_file());
        assert!(installed.join("LICENSE").is_file());
        assert!(installed.join("vcruntime140.dll").is_file());
        let output = fixture.run("valid");
        assert!(
            output.status.success(),
            "stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        fs::write(installed.join("raishot-cli.exe"), "changed").unwrap();
        let output = fixture.run("valid");
        assert!(!output.status.success());
        assert_eq!(
            fs::read_to_string(installed.join("raishot-cli.exe")).unwrap(),
            "changed"
        );
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "显式设置 OMNITRIGGER_RAISHOT_ARCHIVE 后验证真实固定发布包，不联网或运行 EXE"]
    fn installs_pinned_official_archive() {
        let archive = PathBuf::from(
            std::env::var_os("OMNITRIGGER_RAISHOT_ARCHIVE")
                .expect("请设置已下载的 RaiShot.0.4.15.0.zip 路径"),
        );
        let fixture = Fixture::new();
        let events = start_install_from_archive(fixture.root.clone(), archive).unwrap();
        loop {
            match events
                .recv_timeout(std::time::Duration::from_secs(60))
                .unwrap()
            {
                InstallEvent::Progress(_) => {}
                InstallEvent::Failed(message) => panic!("{message}"),
                InstallEvent::Finished {
                    cli_path, version, ..
                } => {
                    assert_eq!(version, RAISHOT_VERSION);
                    let directory = cli_path.parent().unwrap();
                    assert!(directory.join("raishot.exe").is_file());
                    assert!(directory.join("raishot-gpui.exe").is_file());
                    assert!(directory.join("vcruntime140.dll").is_file());
                    assert!(directory.join("onnxruntime.dll").is_file());
                    assert!(directory.join("LICENSE").is_file());
                    assert!(directory.join("THIRD_PARTY_NOTICES.md").is_file());
                    break;
                }
            }
        }
    }
}
