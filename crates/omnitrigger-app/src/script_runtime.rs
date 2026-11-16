//! 独立脚本宿主的本机开发安装与只读定位，不写触发绑定，不安装常驻进程。
use omnitrigger_actions::{
    ActionCall, ActionDefinition, ActionRuntime, CancellationToken, InvocationContext,
    ResolvedAction, ScriptHostRuntime,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{Read, Seek, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const EXTRACT: &str = include_str!("script_runtime_extract.ps1");
const FILE_LIMIT: u64 = 16 * 1024 * 1024;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PackageFile {
    path: String,
    size: u64,
    sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    product: String,
    version: String,
    engine: String,
    target: String,
    api_version: u32,
    protocol_schema: u32,
    distribution: String,
    source_revision: String,
    source_dirty: bool,
    files: Vec<PackageFile>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Pointer {
    schema_version: u32,
    archive_sha256: String,
    manifest_sha256: String,
}

#[derive(Serialize, Deserialize)]
pub struct Installed {
    pub program: PathBuf,
    pub integrity: Option<(String, u64)>,
}
impl Installed {
    pub fn runtime(self) -> ScriptHostRuntime {
        let runtime = ScriptHostRuntime::new(self.program);
        match self.integrity {
            Some((hash, size)) => runtime.with_integrity(hash, size),
            None => runtime,
        }
    }
}
fn sha(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}
fn valid_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
pub fn root(config: &Path) -> Result<PathBuf, String> {
    Ok(std::path::absolute(config)
        .map_err(|e| e.to_string())?
        .with_file_name("runtimes")
        .join("script"))
}
pub(super) fn check_local_path(path: &Path) -> Result<(), String> {
    let absolute = std::path::absolute(path).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    if let Some(std::path::Component::Prefix(prefix)) = absolute.components().next() {
        if !matches!(
            prefix.kind(),
            std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)
        ) {
            return Err("仅支持普通本地磁盘路径。".into());
        }
    }
    let mut ancestors = absolute.ancestors().collect::<Vec<_>>();
    ancestors.reverse();
    for ancestor in ancestors {
        if ancestor.file_name().is_some_and(|s| {
            let s = s.to_string_lossy().to_ascii_lowercase();
            s.starts_with("onedrive") || s == "dropbox" || s.starts_with("icloud")
        }) {
            return Err("脚本路径不能位于云盘目录。".into());
        }
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) => {
                #[cfg(windows)]
                let unavailable = {
                    use std::os::windows::fs::MetadataExt;
                    metadata.file_attributes() & 0x441400 != 0
                };
                #[cfg(not(windows))]
                let unavailable = metadata.file_type().is_symlink();
                if unavailable {
                    return Err("脚本路径包含链接、联接点或占位文件。".into());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}
fn read(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    check_local_path(path)?;
    let file = File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("需要普通文件。".into());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("宿主文件超过大小限制。".into());
    }
    Ok(bytes)
}
pub(super) fn relative(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 240
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_./+-".contains(&b))
        && path.split('/').all(|p| {
            !p.is_empty()
                && p != "."
                && p != ".."
                && !p.ends_with('.')
                && !{
                    let stem = p.split('.').next().unwrap().to_ascii_uppercase();
                    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                        || (stem.len() == 4
                            && (stem.starts_with("COM") || stem.starts_with("LPT"))
                            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
                }
        })
}
fn manifest(path: &Path) -> Result<(Manifest, String), String> {
    let bytes = read(path, 256 * 1024)?;
    let parsed: Manifest =
        serde_json::from_slice(&bytes).map_err(|e| format!("宿主清单无效：{e}"))?;
    if parsed.schema_version != 1
        || parsed.product != "omnitrigger-script-host"
        || parsed.target != "windows-x86_64"
        || parsed.api_version != 1
        || parsed.protocol_schema != 1
        || parsed.distribution != "development"
        || !matches!(parsed.engine.as_str(), "boa" | "quickjs")
        || parsed.version.is_empty()
        || parsed.version.len() > 64
        || !parsed
            .version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-+".contains(&b))
        || parsed.source_revision.len() != 40
        || !parsed
            .source_revision
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
        || parsed.files.len() > 1023
    {
        return Err("宿主清单身份、版本或执行协议不兼容。".into());
    }
    let mut names = BTreeSet::new();
    let mut total = 0u64;
    for file in &parsed.files {
        if !relative(&file.path)
            || !valid_hash(&file.sha256)
            || file.size > FILE_LIMIT
            || !names.insert(file.path.to_ascii_lowercase())
            || !(matches!(
                file.path.as_str(),
                "host.exe" | "LICENSE" | "README.txt" | "THIRD_PARTY_NOTICES.md" | "sdk/api.d.ts"
            ) || file.path.starts_with("licenses/"))
        {
            return Err("宿主文件清单包含非法、重复或不支持的项目。".into());
        }
        total = total.saturating_add(file.size);
    }
    if total > 64 * 1024 * 1024
        || [
            "host.exe",
            "license",
            "readme.txt",
            "third_party_notices.md",
            "sdk/api.d.ts",
            "licenses/dependencies.json",
        ]
        .iter()
        .any(|name| !names.contains(*name))
    {
        return Err("宿主包缺少程序、SDK 或许可清单。".into());
    }
    Ok((parsed, sha(&bytes)))
}
struct VerifiedPackage {
    manifest: Manifest,
    /// 相对包根的目录，父目录先于子目录，包含 ZIP 中显式创建的空目录。
    directories: Vec<PathBuf>,
}

fn package_directories(
    directory: &Path,
    parsed: &Manifest,
    require_complete: bool,
) -> Result<Vec<PathBuf>, String> {
    check_local_path(directory)?;
    let expected = parsed
        .files
        .iter()
        .map(|f| f.path.clone())
        .chain(std::iter::once("manifest.json".into()))
        .collect::<BTreeSet<_>>();
    let mut pending = vec![PathBuf::new()];
    let mut directories = Vec::new();
    let mut found = BTreeSet::new();
    let mut entries = 0;
    while let Some(folder) = pending.pop() {
        for item in fs::read_dir(directory.join(&folder)).map_err(|e| e.to_string())? {
            let item = item.map_err(|e| e.to_string())?;
            entries += 1;
            if entries > 2048 {
                return Err("宿主目录项目数量超限。".into());
            }
            let relative = folder.join(item.file_name());
            let path = item.path();
            check_local_path(&path)?;
            let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if metadata.is_dir() {
                pending.push(relative);
            } else {
                let name = relative.to_string_lossy().replace('\\', "/");
                if !expected.contains(&name) || !found.insert(name) {
                    return Err("宿主目录包含清单外文件。".into());
                }
            }
        }
        directories.push(folder);
    }
    if require_complete && found != expected {
        return Err("宿主目录缺少清单文件。".into());
    }
    Ok(directories)
}

fn file_matches(path: &Path, file: &PackageFile) -> Result<bool, String> {
    check_local_path(path)?;
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("{}：{error}", path.display())),
        Ok(metadata) if !metadata.is_file() => {
            return Err(format!("需要普通文件：{}", path.display()))
        }
        Ok(metadata) if metadata.len() != file.size => return Ok(false),
        Ok(_) => {}
    }
    Ok(sha(&read(path, file.size)?).eq_ignore_ascii_case(&file.sha256))
}

fn verify_package(directory: &Path, expected_manifest: &str) -> Result<VerifiedPackage, String> {
    let (parsed, hash) = manifest(&directory.join("manifest.json"))?;
    if hash != expected_manifest {
        return Err("已安装宿主的清单摘要不符。".into());
    }
    let directories = package_directories(directory, &parsed, true)?;
    for file in &parsed.files {
        if !file_matches(&directory.join(&file.path), file)? {
            return Err(format!("宿主文件校验失败：{}", file.path));
        }
    }
    Ok(VerifiedPackage {
        manifest: parsed,
        directories,
    })
}

/// 先核对整个目标目录，再逐个发布已验证的文件。中断后可用同一包继续。
fn repair_installation(
    payload: &Path,
    destination: &Path,
    parsed: &Manifest,
    digest: &str,
) -> Result<usize, String> {
    package_directories(destination, parsed, false)?;
    let record = PackageFile {
        path: "manifest.json".into(),
        size: fs::metadata(payload.join("manifest.json"))
            .map_err(|error| error.to_string())?
            .len(),
        sha256: digest.into(),
    };
    let mut repairs = Vec::new();
    for file in parsed.files.iter().chain(std::iter::once(&record)) {
        if !file_matches(&destination.join(&file.path), file)? {
            repairs.push(file);
        }
    }
    for file in &repairs {
        let from = payload.join(&file.path);
        let to = destination.join(&file.path);
        check_local_path(&from)?;
        check_local_path(&to)?;
        fs::create_dir_all(to.parent().expect("包内文件应有父目录"))
            .map_err(|error| error.to_string())?;
        if let Err(error) = fs::rename(&from, &to) {
            // 并行安装可能已经修复该文件；仍不匹配时保留现场供重试。
            if !file_matches(&to, file)? {
                return Err(format!("无法修复宿主文件 {}：{error}", file.path));
            }
        }
    }
    if !repairs.is_empty() {
        verify_package(destination, digest)?;
    }
    Ok(repairs.len())
}

pub fn locate(config: &Path) -> Result<Option<Installed>, String> {
    environment_host().map_or_else(|| locate_in_profile(config), |result| result.map(Some))
}

pub(super) fn environment_host() -> Option<Result<Installed, String>> {
    let path = std::env::var_os("OMNITRIGGER_SCRIPT_HOST").filter(|path| !path.is_empty())?;
    let program = PathBuf::from(path);
    Some(check_local_path(&program).and_then(|()| {
        if program.is_absolute() && program.is_file() {
            Ok(Installed {
                program,
                integrity: None,
            })
        } else {
            Err("OMNITRIGGER_SCRIPT_HOST 需要存在的完整本地宿主路径。".into())
        }
    }))
}

pub(super) fn locate_in_profile(config: &Path) -> Result<Option<Installed>, String> {
    let own_root = root(config)?;
    if let Some(host) = locate_managed(&own_root)? {
        return Ok(Some(host));
    }
    let program = own_root.join(if cfg!(windows) { "host.exe" } else { "host" });
    check_local_path(&program)?;
    if program.is_file() {
        return Ok(Some(Installed {
            program,
            integrity: None,
        }));
    }
    for source in crate::profiles::copied_profile_sources(config) {
        if let Some(host) = locate_managed(&root(&source.join("bindings.json"))?)? {
            return Ok(Some(host));
        }
    }
    Ok(None)
}

fn active_pointer(root: &Path) -> Result<Option<Pointer>, String> {
    let current = root.join("current.json");
    check_local_path(&current)?;
    if !current.try_exists().map_err(|error| error.to_string())? {
        return Ok(None);
    }
    let pointer: Pointer =
        serde_json::from_slice(&read(&current, 4096)?).map_err(|e| e.to_string())?;
    if pointer.schema_version != 1
        || !valid_hash(&pointer.archive_sha256)
        || !valid_hash(&pointer.manifest_sha256)
    {
        return Err("宿主激活指针无效，未回退到旧解释器。".into());
    }
    Ok(Some(pointer))
}

fn locate_managed(root: &Path) -> Result<Option<Installed>, String> {
    let Some(pointer) = active_pointer(root)? else {
        return Ok(None);
    };
    let package = root.join("packages").join(&pointer.archive_sha256);
    check_local_path(&package)?;
    let (parsed, digest) = manifest(&package.join("manifest.json"))?;
    if digest != pointer.manifest_sha256 {
        return Err("宿主激活清单摘要不符，未回退到旧解释器。".into());
    }
    let image = parsed
        .files
        .into_iter()
        .find(|file| file.path == "host.exe")
        .ok_or("缺少宿主映像。")?;
    let program = package.join("host.exe");
    check_local_path(&program)?;
    if !program.is_file() {
        return Err("已安装宿主映像缺失。".into());
    }
    Ok(Some(Installed {
        program,
        integrity: Some((image.sha256, image.size)),
    }))
}

/// 重置数据时只沿用本档已登记的活动包。旧版本和手工放入的宿主留在原档备份。
pub(super) fn preserve_on_reset(previous: &Path, staging: &Path) -> Result<Option<String>, String> {
    let original = previous.join("runtimes/script");
    let verified: Result<_, String> = (|| {
        let Some(pointer) = active_pointer(&original)? else {
            return Ok(None);
        };
        let package = original.join("packages").join(&pointer.archive_sha256);
        let manifest = verify_package(&package, &pointer.manifest_sha256)?.manifest;
        Ok(Some((pointer, package, manifest)))
    })();
    let (pointer, package, manifest) = match verified {
        Ok(Some(active)) => active,
        Ok(None) => return Ok(None),
        Err(reason) => {
            return Ok(Some(format!(
                "本档脚本宿主未通过校验：{reason}。原安装保存在重置备份中。"
            )))
        }
    };
    let runtime = staging.join("runtimes/script");
    let dest = runtime.join("packages").join(&pointer.archive_sha256);
    for name in manifest
        .files
        .iter()
        .map(|f| f.path.as_str())
        .chain(["manifest.json"])
    {
        let bytes = read(&package.join(name), FILE_LIMIT)?;
        let target = dest.join(name);
        check_local_path(&target)?;
        fs::create_dir_all(target.parent().ok_or("宿主包目录无效。")?)
            .map_err(|e| e.to_string())?;
        fs::write(target, bytes).map_err(|e| e.to_string())?;
    }
    // 原文件即使在复制期间被外部修改，暂存档也只能登记经过完整校验的字节。
    verify_package(&dest, &pointer.manifest_sha256)?;
    crate::settings::atomic_write(
        &runtime.join("current.json"),
        &serde_json::to_vec(&pointer).map_err(|e| e.to_string())?,
    )?;
    Ok(None)
}

struct Process(std::process::Child);
impl Drop for Process {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}
fn hidden(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
}
fn description(program: &Path, stage: &Path) -> Result<Value, String> {
    let path = stage.join("description.json");
    let mut output = File::options()
        .create_new(true)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    let mut command = Command::new(program);
    command
        .arg("--describe-runtime")
        .current_dir(stage)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(output.try_clone().map_err(|e| e.to_string())?);
    hidden(&mut command);
    let mut child = Process(
        command
            .spawn()
            .map_err(|e| format!("宿主能力探测无法启动：{e}"))?,
    );
    let began = Instant::now();
    loop {
        if began.elapsed() > Duration::from_secs(10)
            || output.metadata().map_err(|e| e.to_string())?.len() > 4096
        {
            return Err("宿主能力探测超时或输出过大。".into());
        }
        if let Some(status) = child.0.try_wait().map_err(|e| e.to_string())? {
            if !status.success() {
                return Err("宿主能力探测失败。".into());
            }
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    output.rewind().map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    output
        .take(4097)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 4096 {
        return Err("宿主描述输出过大。".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}
/// 只清理这次成功安装创建并校验过的暂存内容。对目录仅尝试删除空目录。
fn finish_install_stage(stage: &Path, duplicate: Option<VerifiedPackage>) -> Result<(), String> {
    let mut files = vec![
        stage.join("extract.ps1"),
        stage.join("description.json"),
        stage.join("probe/main.js"),
    ];
    let mut directories = vec![stage.to_owned(), stage.join("probe")];
    if let Some(package) = duplicate {
        let payload = stage.join("payload");
        files.push(payload.join("manifest.json"));
        files.extend(
            package
                .manifest
                .files
                .iter()
                .map(|file| payload.join(&file.path)),
        );
        directories.extend(package.directories.iter().map(|dir| payload.join(dir)));
    }
    for (path, directory) in files
        .into_iter()
        .map(|path| (path, false))
        .chain(directories.into_iter().rev().map(|path| (path, true)))
    {
        check_local_path(&path)?;
        let removed = if directory {
            fs::remove_dir(&path)
        } else {
            fs::remove_file(&path)
        };
        if let Err(error) = removed {
            if error.kind() != std::io::ErrorKind::NotFound {
                return Err(format!("{}：{error}", path.display()));
            }
        }
    }
    Ok(())
}

pub fn install_archive(config: &Path, archive: &Path, expected: &str) -> Result<Value, String> {
    if !valid_hash(expected) {
        return Err("本机开发安装需要完整 SHA-256。".into());
    }
    check_local_path(archive)?;
    let metadata = fs::metadata(archive).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > FILE_LIMIT {
        return Err("开发包不是普通 ZIP 或大小超限。".into());
    }
    let mut options = File::options();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    let mut source = options.open(archive).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    Read::by_ref(&mut source)
        .take(FILE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let archive_hash = sha(&bytes);
    if bytes.len() as u64 != metadata.len() || !archive_hash.eq_ignore_ascii_case(expected) {
        return Err("开发包 SHA-256 不匹配，未解包或激活。".into());
    }
    drop(bytes);
    let root = root(config)?;
    check_local_path(&root)?;
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let nonce = format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos()
    );
    let stage = root.join(format!(".install-{nonce}"));
    fs::create_dir(&stage).map_err(|e| e.to_string())?;
    let payload = stage.join("payload");
    let script = stage.join("extract.ps1");
    fs::write(&script, format!("\u{feff}{EXTRACT}")).map_err(|e| e.to_string())?;
    let result = (|| {
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
            .arg(&script)
            .env(
                "OMNITRIGGER_SCRIPT_ARCHIVE",
                std::path::absolute(archive).map_err(|e| e.to_string())?,
            )
            .env("OMNITRIGGER_SCRIPT_PAYLOAD", &payload)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        hidden(&mut command);
        let mut child = Process(command.spawn().map_err(|e| e.to_string())?);
        let began = Instant::now();
        loop {
            if began.elapsed() > Duration::from_secs(30) {
                return Err("开发包解压超时。".into());
            }
            if let Some(status) = child.0.try_wait().map_err(|e| e.to_string())? {
                if !status.success() {
                    return Err("开发包解压校验失败，未激活。".into());
                }
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        drop(child);
        drop(source);
        let (_, digest) = manifest(&payload.join("manifest.json"))?;
        let manifest = verify_package(&payload, &digest)?.manifest;
        let image = manifest
            .files
            .iter()
            .find(|f| f.path == "host.exe")
            .ok_or("缺少宿主。")?;
        let described = description(&payload.join("host.exe"), &stage)?;
        if described["schema_version"] != 1
            || described["product"] != manifest.product
            || described["engine"] != manifest.engine
            || described["version"] != manifest.version
            || described["api_version"] != 1
            || described["protocol_schema"] != 1
        {
            return Err("宿主自述与包清单不兼容，未激活。".into());
        }
        let probe = stage.join("probe");
        fs::create_dir(&probe).map_err(|e| e.to_string())?;
        fs::write(
            probe.join("main.js"),
            "function main(ctx){return {version:ctx.apiVersion,value:ctx.input.value};}",
        )
        .map_err(|e| e.to_string())?;
        let action = ResolvedAction {
            package_root: probe.clone(),
            entry_path: probe.join("main.js"),
            call: ActionCall {
                package_id: "install-probe".into(),
                action_id: "run".into(),
                action_version: 1,
                input: json!({"value":"安装验证"}),
            },
            definition: ActionDefinition {
                id: "run".into(),
                title: "安装验证".into(),
                version: 1,
                runtime: "script".into(),
                entry: "main.js".into(),
                single_instance: false,
            },
        };
        let output = ScriptHostRuntime::new(payload.join("host.exe"))
            .with_integrity(image.sha256.clone(), image.size)
            .with_timeout(Duration::from_secs(10))
            .execute(
                &action,
                &InvocationContext {
                    invocation_id: 0,
                    binding_id: 0,
                },
                &CancellationToken::new(),
            )
            .map_err(|e| e.to_string())?;
        if output != json!({"version":1,"value":"安装验证"}) {
            return Err("宿主实际 JS 调用失败，未激活。".into());
        }
        let verified = verify_package(&payload, &digest)?;
        let packages = root.join("packages");
        check_local_path(&packages)?;
        fs::create_dir_all(&packages).map_err(|e| e.to_string())?;
        let destination = packages.join(&archive_hash);
        check_local_path(&destination)?;
        let mut reused = destination.try_exists().map_err(|e| e.to_string())?;
        if !reused {
            let resolved_root = fs::canonicalize(&root).map_err(|e| e.to_string())?;
            if !fs::canonicalize(&payload)
                .map_err(|e| e.to_string())?
                .starts_with(&resolved_root)
                || !fs::canonicalize(&packages)
                    .map_err(|e| e.to_string())?
                    .starts_with(&resolved_root)
            {
                return Err("暂存目录超出安装根目录。".into());
            }
            if let Err(error) = fs::rename(&payload, &destination) {
                if !destination.try_exists().map_err(|e| e.to_string())? {
                    return Err(error.to_string());
                }
                // 另一个安装进程可能刚发布了同一摘要，两边都须核对实际内容。
                reused = true;
            }
        }
        let repaired = if reused {
            repair_installation(&payload, &destination, &verified.manifest, &digest)?
        } else {
            0
        };
        let pointer = Pointer {
            schema_version: 1,
            archive_sha256: archive_hash.clone(),
            manifest_sha256: digest,
        };
        let current = root.join("current.json");
        check_local_path(&current)?;
        let next = root.join(format!(".current-{nonce}.json"));
        let mut file = File::options()
            .create_new(true)
            .write(true)
            .open(&next)
            .map_err(|e| e.to_string())?;
        file.write_all(&serde_json::to_vec(&pointer).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        fs::rename(&next, &current).map_err(|e| e.to_string())?;
        let mut receipt = json!({"schema_version":1,"status":"installed","program":destination.join("host.exe"),"engine":manifest.engine,"version":manifest.version,"source_revision":manifest.source_revision,"source_dirty":manifest.source_dirty,"distribution":manifest.distribution,"archive_sha256":archive_hash,"protocol_and_JS_verified":true});
        if repaired != 0 {
            receipt["repaired_files"] = json!(repaired);
        }
        Ok((receipt, reused.then_some(verified)))
    })();
    let (mut receipt, duplicate) =
        result.map_err(|e: String| format!("{e} 暂存位置：{}", stage.display()))?;
    if let Err(error) = finish_install_stage(&stage, duplicate) {
        receipt["cleanup_warning"] = format!("宿主已安装，暂存目录未能清理完毕：{error}").into();
    }
    Ok(receipt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_cleanup_preserves_files_created_by_another_operation() {
        let stage = std::env::temp_dir().join(format!(
            "omnitrigger-install-cleanup-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&stage).unwrap();
        let probe = stage.join("probe");
        fs::create_dir(&probe).unwrap();
        fs::write(probe.join("main.js"), "probe").unwrap();
        let other = probe.join("diagnostic.txt");
        fs::write(&other, "keep").unwrap();

        assert!(finish_install_stage(&stage, None)
            .unwrap_err()
            .contains(&probe.display().to_string()));
        assert_eq!(fs::read_to_string(&other).unwrap(), "keep");
        assert!(!probe.join("main.js").exists());
        fs::remove_file(other).unwrap();
        finish_install_stage(&stage, None).unwrap();
        assert!(!stage.exists());
    }
}
