//! 查找本机已有的 RaiShot。只读取 RaiShot 自带的 manifest.json 与文件大小，
//! 不计算哈希、不运行程序，也不遍历用户目录；整个查找在毫秒级完成。
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

/// 宿主依赖 `--capture` 命令行接口：0.4.15 起提供，0.5 起视为可能不兼容。
const MIN_VERSION: Version = (0, 4, 15);
const MAX_VERSION_EXCLUSIVE: Version = (0, 5, 0);
const MAX_VERSIONS_PER_ROOT: usize = 64;
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
/// 运行必需的程序与运行库；说明和许可文件缺失不影响运行，不作为接入条件。
const REQUIRED_FILES: &[&str] = &[
    "raishot.exe",
    "raishot-gpui.exe",
    "raishot-cli.exe",
    "vcruntime140.dll",
    "vcruntime140_1.dll",
    "msvcp140.dll",
    "msvcp140_1.dll",
    "onnxruntime.dll",
    "onnxruntime_providers_shared.dll",
];
/// Quicker 中 RaiShot 的包标识；每个版本位于其下的独立子目录。
const QUICKER_PACKAGE: &str = "cea.rustshot";

type Version = (u32, u32, u32);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub cli: PathBuf,
    pub version: String,
    order: Version,
}

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
}

pub(super) fn is_cloud_path(path: &Path) -> bool {
    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy().to_ascii_lowercase();
        name.starts_with("onedrive") || name == "dropbox" || name.starts_with("icloud")
    })
}

fn parse_version(text: &str) -> Option<Version> {
    let mut parts = text.split('.').map(|part| part.parse::<u32>().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

/// 检查一个目录是否为完整、兼容的 RaiShot 安装。依据 RaiShot 自带清单的版本、
/// 架构与文件大小判断；目录或其祖先含链接、联接点、云端占位时拒绝。
pub fn inspect(directory: &Path) -> Option<Installed> {
    if !directory.is_absolute()
        || directory.to_string_lossy().starts_with(r"\\")
        || super::ensure_plain_ancestors(directory).is_err()
    {
        return None;
    }
    let manifest_path = directory.join("manifest.json");
    let metadata = fs::symlink_metadata(&manifest_path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_MANIFEST_BYTES {
        return None;
    }
    let manifest: Manifest = serde_json::from_slice(&fs::read(&manifest_path).ok()?).ok()?;
    let order = parse_version(&manifest.package_version)?;
    if manifest.schema_version != 1
        || manifest.architecture != "windows-x64"
        || order < MIN_VERSION
        || order >= MAX_VERSION_EXCLUSIVE
    {
        return None;
    }
    for name in REQUIRED_FILES {
        let record = manifest.files.iter().find(|record| record.path == *name)?;
        let metadata = fs::symlink_metadata(directory.join(name)).ok()?;
        if !metadata.is_file() || metadata.len() != record.bytes {
            return None;
        }
    }
    Some(Installed {
        cli: directory.join("raishot-cli.exe"),
        version: manifest.package_version,
        order,
    })
}

/// 依次考虑：正在运行的 RaiShot、已配置路径、各安装根目录及其一级版本子目录。
/// 正在运行的副本已被系统检查过、启动最快，优先使用；否则取最高兼容版本，
/// 同版本保留更靠前的位置（已配置路径优先）。
fn find_in(running: &[PathBuf], configured: Option<&Path>, roots: &[PathBuf]) -> Option<Installed> {
    let best = |directories: &mut dyn Iterator<Item = PathBuf>| {
        directories
            .filter_map(|directory| inspect(&directory))
            .fold(None::<Installed>, |best, found| match best {
                Some(best) if best.order >= found.order => Some(best),
                _ => Some(found),
            })
    };
    if let Some(found) = best(
        &mut running
            .iter()
            .filter_map(|path| path.parent().map(Path::to_path_buf)),
    ) {
        return Some(found);
    }
    let mut directories: Vec<PathBuf> = configured
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .into_iter()
        .collect();
    for root in roots {
        directories.push(root.clone());
        if !root.is_absolute() || super::ensure_plain_ancestors(root).is_err() {
            continue;
        }
        let Ok(reader) = fs::read_dir(root) else {
            continue;
        };
        let mut children: Vec<_> = reader
            .filter_map(Result::ok)
            .filter(|item| item.file_type().is_ok_and(|kind| kind.is_dir()))
            .map(|item| item.path())
            .take(MAX_VERSIONS_PER_ROOT)
            .collect();
        children.sort();
        directories.extend(children);
    }
    best(&mut directories.into_iter())
}

/// 本机常见的 RaiShot 安装根目录。`shared` 为 OmniTrigger 下载时使用的共享位置，
/// `legacy` 为早期版本安装在 OmniTrigger 配置目录下的副本，保留以继续复用。
pub fn find(configured: Option<&Path>, shared: Option<&Path>, legacy: &Path) -> Option<Installed> {
    let mut roots: Vec<PathBuf> = shared.map(Path::to_path_buf).into_iter().collect();
    let quicker = |base: PathBuf| {
        ["_packages", "packages"]
            .map(|folder| base.join("Quicker").join(folder).join(QUICKER_PACKAGE))
    };
    #[cfg(windows)]
    if let Some(documents) = omnitrigger_windows::documents_dir() {
        roots.extend(quicker(documents));
    }
    for variable in ["LOCALAPPDATA", "APPDATA"] {
        if let Some(base) = std::env::var_os(variable).map(PathBuf::from) {
            roots.extend(quicker(base));
        }
    }
    if let Some(base) = std::env::var_os("ProgramFiles").map(PathBuf::from) {
        roots.push(base.join("RaiShot"));
    }
    roots.push(legacy.to_path_buf());
    #[cfg(windows)]
    let running = omnitrigger_windows::running_process_paths(&["raishot.exe", "raishot-gpui.exe"]);
    #[cfg(not(windows))]
    let running = Vec::new();
    find_in(&running, configured, &roots)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "omnitrigger-discovery-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        /// 写入一份最小安装：每个必需文件内容为其名称，清单记录对应大小。
        fn install(&self, relative: &str, version: &str) -> PathBuf {
            let directory = self.0.join(relative);
            fs::create_dir_all(&directory).unwrap();
            let files: Vec<_> = REQUIRED_FILES
                .iter()
                .map(|name| {
                    fs::write(directory.join(name), name).unwrap();
                    serde_json::json!({"path": name, "bytes": name.len(), "sha256": "0"})
                })
                .collect();
            let manifest = serde_json::json!({
                "schema_version": 1,
                "package_version": version,
                "architecture": "windows-x64",
                "files": files,
            });
            fs::write(directory.join("manifest.json"), manifest.to_string()).unwrap();
            directory
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            if self.0.parent() == Some(std::env::temp_dir().as_path()) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
    }

    #[test]
    fn picks_highest_compatible_version_under_a_package_root() {
        let fixture = Fixture::new();
        for version in ["0.4.9", "0.4.15", "0.4.16", "0.5.0"] {
            fixture.install(&format!("cea.rustshot/{version}"), version);
        }
        let found = find_in(&[], None, &[fixture.0.join("cea.rustshot")]).unwrap();
        assert_eq!(found.version, "0.4.16");
        assert_eq!(
            found.cli,
            fixture.0.join("cea.rustshot/0.4.16/raishot-cli.exe")
        );
    }

    #[test]
    fn running_copy_wins_over_newer_installed_versions() {
        let fixture = Fixture::new();
        let running = fixture.install("running", "0.4.15");
        fixture.install("packages/0.4.16", "0.4.16");
        let found = find_in(
            &[running.join("raishot.exe")],
            None,
            &[fixture.0.join("packages")],
        )
        .unwrap();
        assert_eq!(found.cli, running.join("raishot-cli.exe"));
    }

    #[test]
    fn configured_path_is_kept_when_version_ties() {
        let fixture = Fixture::new();
        let configured = fixture.install("configured", "0.4.16");
        fixture.install("packages/0.4.16", "0.4.16");
        let found = find_in(
            &[],
            Some(&configured.join("raishot-cli.exe")),
            &[fixture.0.join("packages")],
        )
        .unwrap();
        assert_eq!(found.cli, configured.join("raishot-cli.exe"));
    }

    #[test]
    fn rejects_incomplete_resized_or_cloud_installs() {
        let fixture = Fixture::new();
        let missing = fixture.install("missing", "0.4.16");
        fs::remove_file(missing.join("vcruntime140.dll")).unwrap();
        let resized = fixture.install("resized", "0.4.16");
        fs::write(resized.join("raishot-cli.exe"), "changed size").unwrap();
        let cloud = fixture.install("OneDrive - personal/raishot", "0.4.16");
        assert!(inspect(&missing).is_none());
        assert!(inspect(&resized).is_none());
        assert!(inspect(&cloud).is_none());
        assert!(inspect(&fixture.install("ok", "0.4.15")).is_some());
    }

    #[cfg(windows)]
    #[test]
    fn does_not_follow_junctions() {
        use std::os::windows::process::CommandExt;
        let fixture = Fixture::new();
        fixture.install("outside/0.4.16", "0.4.16");
        let packages = fixture.0.join("packages");
        fs::create_dir(&packages).unwrap();
        let output = std::process::Command::new("powershell.exe")
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "$ErrorActionPreference = 'Stop'; New-Item -ItemType Junction -Path $env:OMNITRIGGER_TEST_LINK -Target $env:OMNITRIGGER_TEST_TARGET | Out-Null",
            ])
            .env("OMNITRIGGER_TEST_LINK", packages.join("linked"))
            .env("OMNITRIGGER_TEST_TARGET", fixture.0.join("outside"))
            .creation_flags(0x0800_0000)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(find_in(&[], None, &[packages]).is_none());
    }

    #[test]
    fn version_parsing_requires_three_numbers() {
        assert_eq!(parse_version("0.4.16"), Some((0, 4, 16)));
        assert_eq!(parse_version("0.4"), None);
        assert_eq!(parse_version("0.4.16.0"), None);
        assert_eq!(parse_version("v0.4.16"), None);
    }
}
