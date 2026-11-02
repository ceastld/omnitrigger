//! 版本检查优先统一 Cloudflare 清单；下载和安装需用户操作，耗时工作均在后台执行。
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc::{self, Receiver, SyncSender},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const CHECK_INTERVAL: Duration = Duration::from_secs(5 * 60 * 60);
const UPDATE_API: &str = "https://update.alinko.cc/omnitrigger/version.json";
const TRANSPORT: &str = include_str!("update_transport.ps1");
const APPLY: &str = include_str!("update_apply.ps1");
const COMMON: &str = include_str!("update_common.ps1");

#[derive(Default)]
pub(super) struct CheckSchedule {
    next: Option<Instant>,
}
impl CheckSchedule {
    pub fn claim_due(&mut self, now: Instant, ready: bool) -> bool {
        if !ready || self.next.is_some_and(|next| now < next) {
            return false;
        }
        self.checked_at(now);
        true
    }
    pub fn checked_at(&mut self, now: Instant) {
        self.next = Some(now + CHECK_INTERVAL);
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
    #[serde(default)]
    digest: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    draft: bool,
    assets: Vec<Asset>,
    #[serde(default)]
    sha256: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiManifest {
    product: String,
    release: String,
    version: String,
    download_url: String,
    sha256_sums_url: String,
    sha256: String,
    size: u64,
    notes: Option<String>,
}

impl ApiManifest {
    fn into_release(self) -> Result<Release, String> {
        let tag = format!("v{}", self.release);
        let version = parse_version(&tag).ok_or("版本 API 返回的版本无效。")?;
        let display_version = if version.pre.is_empty() {
            format!("{version}.0")
        } else {
            version.to_string()
        };
        if self.product != "omnitrigger" || self.version != display_version {
            return Err("版本 API 返回的产品或版本不匹配。".into());
        }
        let release = Release {
            tag_name: tag,
            body: self.notes,
            draft: false,
            sha256: Some(self.sha256),
            assets: vec![
                Asset {
                    name: format!("OmniTrigger-{version}-windows-x86_64.zip"),
                    browser_download_url: self.download_url,
                    size: self.size,
                    digest: None,
                },
                Asset {
                    name: "SHA256SUMS.txt".into(),
                    browser_download_url: self.sha256_sums_url,
                    size: 0,
                    digest: None,
                },
            ],
        };
        release.assets()?;
        release.archive_hash()?;
        Ok(release)
    }
}

impl Release {
    fn assets(&self) -> Result<(&Asset, &Asset), String> {
        let version = parse_version(&self.tag_name).ok_or("发布版本无效。")?;
        let archive_name = format!("OmniTrigger-{version}-windows-x86_64.zip");
        let find = |name: &str, limit| {
            let mut matches = self.assets.iter().filter(|asset| asset.name == name);
            let asset = matches.next().ok_or("发布包尚未准备完整，请稍后重试。")?;
            let expected = format!(
                "https://github.com/ceastld/omnitrigger/releases/download/{}/{name}",
                self.tag_name
            );
            if matches.next().is_some()
                || asset.browser_download_url != expected
                || (asset.size == 0 && name != "SHA256SUMS.txt")
                || asset.size > limit
            {
                return Err("发布资产的地址、大小或名称无效。".to_string());
            }
            Ok(asset)
        };
        Ok((
            find(&archive_name, 512 * 1024 * 1024)?,
            find("SHA256SUMS.txt", 64 * 1024)?,
        ))
    }

    fn archive_hash(&self) -> Result<&str, String> {
        let archive = self.assets()?.0;
        let hash = self
            .sha256
            .as_deref()
            .or_else(|| archive.digest.as_deref()?.strip_prefix("sha256:"))
            .ok_or("发行包缺少 SHA-256 摘要。")?;
        if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("发行包 SHA-256 摘要无效。".into());
        }
        Ok(hash)
    }
}

fn parse_version(tag: &str) -> Option<Version> {
    // 不接受可编码路径的标签；资产 URL 必须与仓库现有打包约定完全一致。
    let version = tag.strip_prefix('v')?;
    if !version
        .bytes()
        .all(|c| c.is_ascii_alphanumeric() || b".-".contains(&c))
    {
        return None;
    }
    Version::parse(version).ok()
}

fn select_release(releases: Vec<Release>, current: &str) -> Result<Option<Release>, String> {
    let current = Version::parse(current).map_err(|e| format!("当前版本无效：{e}"))?;
    let mut newest = releases
        .into_iter()
        .filter(|release| !release.draft)
        .filter_map(|release| parse_version(&release.tag_name).map(|version| (version, release)))
        .filter(|(version, _)| version > &current)
        .max_by(|(a, _), (b, _)| a.cmp(b))
        .map(|(_, release)| release);
    if let Some(release) = &mut newest {
        release.sha256 = Some(release.archive_hash()?.to_owned());
    }
    Ok(newest)
}

#[derive(Debug)]
pub enum Event {
    Progress(String),
    Checked(Option<Release>),
    Restart(RestartPermit),
    Failed(String),
}

#[derive(Debug)]
pub struct RestartPermit {
    stage: PathBuf,
    token: String,
}
impl RestartPermit {
    pub fn authorize(self) -> Result<(), String> {
        use std::io::Write;
        plain_path(&self.stage)?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.stage.join("authorize"))
            .map_err(|error| format!("无法授权更新重启：{error}"))?;
        file.write_all(self.token.as_bytes())
            .map_err(|error| error.to_string())
    }
}

pub enum Operation {
    Check,
    Install(Release),
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum WireEvent {
    Progress { message: String },
    Releases { releases: Vec<Release> },
    Manifest { manifest: ApiManifest },
    Prepared,
    Failed { message: String },
}

enum Response {
    Releases(Vec<Release>),
    Manifest(ApiManifest),
    Prepared,
}

/// 每次请求独立解析完整结果，网络及清单身份失败都能进入下一版本源。
fn transport(stage: &Path, mode: &str, sender: &SyncSender<Event>) -> Result<Response, String> {
    let mut child = powershell(&stage.join("transport.ps1"))
        .env("OMNITRIGGER_UPDATE_STAGE", stage)
        .env("OMNITRIGGER_UPDATE_MODE", mode)
        .env("OMNITRIGGER_UPDATE_API", UPDATE_API)
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("无法启动更新下载器：{e}"))?;
    let result = (|| {
        let stdout = child.stdout.take().ok_or("无法读取更新状态。")?;
        let mut terminal = None;
        for line in BufReader::new(stdout).lines() {
            let line = line.map_err(|e| e.to_string())?;
            if line.len() > 4 * 1024 * 1024 {
                return Err("更新响应过大。".into());
            }
            match serde_json::from_str::<WireEvent>(line.trim_start_matches('\u{feff}'))
                .map_err(|e| format!("更新响应无效：{e}"))?
            {
                WireEvent::Progress { message } => {
                    let _ = sender.try_send(Event::Progress(message));
                }
                WireEvent::Releases { releases } if mode == "fallback" => {
                    terminal = Some(Response::Releases(releases))
                }
                WireEvent::Manifest { manifest } if mode == "check" => {
                    terminal = Some(Response::Manifest(manifest))
                }
                WireEvent::Prepared if mode == "install" => terminal = Some(Response::Prepared),
                WireEvent::Failed { message } => return Err(message),
                _ => return Err("更新器返回了意外状态。".into()),
            }
        }
        if !child.wait().map_err(|e| e.to_string())?.success() {
            return Err("更新未完成，请重试。".into());
        }
        terminal.ok_or("更新器未返回结果。".into())
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

fn checked_response(response: Response, current: &str) -> Result<Option<Release>, String> {
    match response {
        Response::Manifest(manifest) => select_release(vec![manifest.into_release()?], current),
        Response::Releases(releases) => select_release(releases, current),
        Response::Prepared => Err("版本源返回了意外状态。".into()),
    }
}

fn with_fallback(
    primary: impl FnOnce() -> Result<Option<Release>, String>,
    fallback: impl FnOnce() -> Result<Option<Release>, String>,
) -> Result<Option<Release>, String> {
    primary().or_else(|primary_error| {
        fallback().map_err(|fallback_error| {
            format!("版本 API：{primary_error}；GitHub 回退：{fallback_error}")
        })
    })
}

#[derive(Serialize)]
struct Plan<'a> {
    executable: &'a Path,
    config: &'a Path,
    parent_pid: u32,
    release: &'a Release,
    token: &'a str,
}

pub fn start(operation: Operation, config: PathBuf) -> Result<Receiver<Event>, String> {
    let (sender, receiver) = mpsc::sync_channel(128);
    thread::Builder::new()
        .name("omnitrigger-update".into())
        .spawn(move || {
            if let Err(error) = run(operation, &config, &sender) {
                let _ = sender.send(Event::Failed(error));
            }
        })
        .map_err(|e| format!("无法启动更新线程：{e}"))?;
    Ok(receiver)
}

fn powershell(script: &Path) -> Command {
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
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
}

/// 根到叶检查，不能先访问联接点或云盘内部再检查父目录。
pub(super) fn plain_path(path: &Path) -> Result<(), String> {
    for ancestor in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
        let name = ancestor
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        if name.starts_with("onedrive") || name == "dropbox" || name.starts_with("icloud") {
            return Err("更新目录不能位于云盘同步目录。".into());
        }
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) => {
                #[cfg(windows)]
                let special = {
                    use std::os::windows::fs::MetadataExt;
                    metadata.file_attributes() & (0x400 | 0x1000 | 0x40000 | 0x400000) != 0
                };
                #[cfg(not(windows))]
                let special = metadata.file_type().is_symlink();
                if special {
                    return Err("更新目录包含链接或云端占位文件。".into());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(format!("无法检查更新目录：{e}")),
        }
    }
    Ok(())
}

struct Stage {
    path: PathBuf,
    handed_off: bool,
}
impl Drop for Stage {
    fn drop(&mut self) {
        if !self.handed_off && plain_path(&self.path).is_ok() {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn run(operation: Operation, config: &Path, sender: &SyncSender<Event>) -> Result<(), String> {
    let install = matches!(operation, Operation::Install(_));
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let root = if install {
        executable
            .parent()
            .ok_or("无法确定应用目录。")?
            .to_path_buf()
    } else {
        std::env::temp_dir()
    };
    plain_path(&root)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let mut stage = Stage {
        path: root.join(format!(
            ".omnitrigger-update-{}-{stamp}",
            std::process::id()
        )),
        handed_off: false,
    };
    let token = format!("{}-{stamp}", std::process::id());
    fs::create_dir(&stage.path)
        .map_err(|e| format!("无法创建更新目录，请确认应用目录可写：{e}"))?;
    let script = stage.path.join("transport.ps1");
    fs::write(&script, format!("\u{feff}{TRANSPORT}")).map_err(|e| e.to_string())?;
    fs::write(stage.path.join("common.ps1"), format!("\u{feff}{COMMON}"))
        .map_err(|e| e.to_string())?;
    if !install {
        let checked = with_fallback(
            || checked_response(transport(&stage.path, "check", sender)?, VERSION),
            || {
                let _ =
                    sender.try_send(Event::Progress("版本 API 暂不可用，正在检查备用源…".into()));
                checked_response(transport(&stage.path, "fallback", sender)?, VERSION)
            },
        )?;
        let _ = sender.send(Event::Checked(checked));
        return Ok(());
    }
    if let Operation::Install(release) = &operation {
        release.assets()?;
        release.archive_hash()?;
        let config = std::path::absolute(config).map_err(|e| e.to_string())?;
        let plan = Plan {
            executable: &executable,
            config: &config,
            parent_pid: std::process::id(),
            release,
            token: &token,
        };
        fs::write(
            stage.path.join("plan.json"),
            serde_json::to_vec(&plan).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    }
    transport(&stage.path, "install", sender)?;
    let helper_script = stage.path.join("apply.ps1");
    fs::write(&helper_script, format!("\u{feff}{APPLY}")).map_err(|e| e.to_string())?;
    let mut helper = powershell(&helper_script)
        .env("OMNITRIGGER_UPDATE_STAGE", &stage.path)
        .stdout(Stdio::null())
        .spawn()
        .map_err(|e| format!("无法准备更新重启：{e}"))?;
    let deadline = Instant::now() + Duration::from_secs(15);
    while !stage.path.join("helper-ready").is_file() {
        if helper.try_wait().map_err(|e| e.to_string())?.is_some() || Instant::now() > deadline {
            let _ = helper.kill();
            let _ = helper.wait();
            return Err(fs::read_to_string(stage.path.join("error.txt"))
                .unwrap_or_else(|_| "更新重启器未就绪，当前程序继续运行。".into()));
        }
        thread::sleep(Duration::from_millis(50));
    }
    // 接收方已经退出时撤销交接，不能留下下一次关闭时才偷偷执行的更新。
    if sender
        .send(Event::Restart(RestartPermit {
            stage: stage.path.clone(),
            token,
        }))
        .is_err()
    {
        let _ = helper.kill();
        let _ = helper.wait();
        return Err("应用已关闭，已取消更新。".into());
    }
    stage.handed_off = true;
    Ok(())
}

pub fn mark_healthy() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        if let Some(path) = std::env::var_os("OMNITRIGGER_UPDATE_HEALTH") {
            std::env::remove_var("OMNITRIGGER_UPDATE_HEALTH");
            let path = PathBuf::from(path);
            if path.file_name().is_some_and(|name| name == "healthy") && plain_path(&path).is_ok() {
                let _ = fs::write(path, VERSION);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn release(tag: &str) -> Release {
        let mut release = Release {
            tag_name: tag.into(),
            body: None,
            draft: false,
            assets: vec![],
            sha256: Some("a".repeat(64)),
        };
        for name in [
            format!("OmniTrigger-{}-windows-x86_64.zip", &tag[1..]),
            "SHA256SUMS.txt".into(),
        ] {
            release.assets.push(Asset {
                browser_download_url: format!(
                    "https://github.com/ceastld/omnitrigger/releases/download/{tag}/{name}"
                ),
                name,
                size: 100,
                digest: None,
            });
        }
        release
    }
    #[test]
    fn release_selection_uses_semver_and_includes_preview_channel() {
        let releases = vec![
            release("v0.9.0"),
            release("v0.10.0-beta.2"),
            release("v0.10.0-beta.10"),
        ];
        assert_eq!(
            select_release(releases, "0.9.0").unwrap().unwrap().tag_name,
            "v0.10.0-beta.10"
        );
        assert!(select_release(vec![release("v0.1.0")], "0.1.0")
            .unwrap()
            .is_none());
        assert!(select_release(vec![release("v0.1.0")], "0.2.0")
            .unwrap()
            .is_none());
        let mut draft = release("v9.0.0");
        draft.draft = true;
        assert!(select_release(vec![draft], "0.1.0").unwrap().is_none());
    }
    #[test]
    fn malformed_or_external_assets_cannot_be_installed() {
        let mut candidate = release("v0.2.0");
        candidate.assets[0].browser_download_url = "https://example.com/program.zip".into();
        assert!(select_release(vec![candidate], "0.1.0").is_err());
        let mut missing = release("v0.2.0");
        missing.assets.pop();
        assert!(select_release(vec![missing], "0.1.0").is_err());
        assert!(parse_version("v0.2.0/../x").is_none());
        assert!(parse_version("v0.2.0+metadata").is_none());
    }

    fn manifest(version: &str) -> ApiManifest {
        let version = Version::parse(version).unwrap();
        ApiManifest {
            product: "omnitrigger".into(), release: version.to_string(),
            version: if version.pre.is_empty() { format!("{version}.0") } else { version.to_string() },
            download_url: format!("https://github.com/ceastld/omnitrigger/releases/download/v{version}/OmniTrigger-{version}-windows-x86_64.zip"),
            sha256_sums_url: format!("https://github.com/ceastld/omnitrigger/releases/download/v{version}/SHA256SUMS.txt"),
            sha256: "a".repeat(64), size: 100, notes: Some("更新说明".into()),
        }
    }

    #[test]
    fn cloudflare_manifest_identity_versions_and_hash_are_validated_before_fallback() {
        for version in ["0.2.0", "0.2.0-beta.1"] {
            let release = manifest(version).into_release().unwrap();
            assert_eq!(release.tag_name, format!("v{version}"));
            assert_eq!(release.archive_hash().unwrap(), "a".repeat(64));
        }
        for field in ["product", "version", "download", "checksum", "hash", "size"] {
            let mut bad = manifest("0.2.0");
            match field {
                "product" => bad.product = "eaclip".into(),
                "version" => bad.version = "0.3.0.0".into(),
                "download" => bad.download_url = "https://example.test/update.zip".into(),
                "checksum" => bad.sha256_sums_url = "https://example.test/SHA256SUMS.txt".into(),
                "hash" => bad.sha256 = "invalid".into(),
                _ => bad.size = 0,
            }
            assert!(bad.into_release().is_err(), "{field}");
        }
        let fallback_called = std::cell::Cell::new(false);
        assert!(with_fallback(
            || Ok(None),
            || {
                fallback_called.set(true);
                Err("不应访问备用源".into())
            }
        )
        .unwrap()
        .is_none());
        assert!(!fallback_called.get());
        let selected = with_fallback(
            || {
                checked_response(
                    Response::Manifest(ApiManifest {
                        product: "wrong".into(),
                        ..manifest("0.2.0")
                    }),
                    "0.1.0",
                )
            },
            || checked_response(Response::Releases(vec![release("v0.3.0")]), "0.1.0"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(selected.tag_name, "v0.3.0");
        let error =
            with_fallback(|| Err("API 失败".into()), || Err("备用源失败".into())).unwrap_err();
        assert!(error.contains("API 失败") && error.contains("备用源失败"));
    }

    #[test]
    fn automatic_check_waits_for_readiness_and_coalesces_missed_intervals() {
        let start = Instant::now();
        let mut schedule = CheckSchedule::default();
        assert!(!schedule.claim_due(start, false));
        assert!(schedule.claim_due(start, true));
        assert!(!schedule.claim_due(start + CHECK_INTERVAL - Duration::from_secs(1), true));
        let resumed = start + CHECK_INTERVAL * 4;
        assert!(schedule.claim_due(resumed, true));
        assert!(!schedule.claim_due(resumed, true));
        schedule.checked_at(resumed + Duration::from_secs(10));
        assert!(!schedule.claim_due(resumed + CHECK_INTERVAL, true));
        assert!(schedule.claim_due(resumed + CHECK_INTERVAL + Duration::from_secs(10), true));
    }

    #[cfg(windows)]
    #[test]
    fn windows_update_archive_restart_and_rollback() {
        let root = std::env::temp_dir().join(format!(
            "omnitrigger-update-tests-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        for (name, content) in [
            ("update_tests.ps1", include_str!("update_tests.ps1")),
            ("update_common.ps1", COMMON),
            ("update_apply.ps1", APPLY),
        ] {
            fs::write(root.join(name), format!("\u{feff}{content}")).unwrap();
        }
        let script = root.join("update_tests.ps1");
        let output = powershell(&script)
            .arg("-FixtureRoot")
            .arg(&root)
            .stderr(Stdio::piped())
            .output()
            .unwrap();
        // 失败时保留夹具与 error.txt 供诊断；不清理其他目录。
        assert!(
            output.status.success(),
            "{}\n{}\n夹具：{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
            root.display()
        );
        if plain_path(&root).is_ok() {
            fs::remove_dir_all(root).unwrap();
        }
    }
}
