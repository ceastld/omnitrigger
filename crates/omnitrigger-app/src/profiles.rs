//! 数据根解析、打包重定向处理与持久测试配置档，设计见 `docs/PROFILES.md`。
//!
//! 首阶段的默认配置档就是数据根本身（文件保持原位）；测试配置档位于
//! `<数据根>\profiles\<ID>\`，由 `<数据根>\profiles.json` 登记。选中配置档等价于把
//! 该目录下的 `bindings.json` 作为配置路径，其余数据文件随之切换。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::plugins::{local_manifest_metadata, read_local_manifest, RAISHOT_MANIFEST_PATH};
use serde::{Deserialize, Serialize};

#[cfg(windows)]
mod external_import;
#[cfg(windows)]
pub(super) use external_import::recover as recover_external_import;

const CATALOG_VERSION: u32 = 1;
const DEFAULT_ID: &str = "default";
const TEST_ALIAS: &str = "testing";
const RESULT_WAIT: std::time::Duration = std::time::Duration::from_secs(60);

/// 启动解析结果，交给界面决定标题、标识与是否安装 Hook。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Startup {
    /// 测试配置档的显示名；日常数据为 `None`。
    pub test_name: Option<String>,
    /// 动作数据是否属于已登记的测试档，与窗口标题、启动参数及监听状态无关。
    pub test_data: bool,
    /// 是否安装输入 Hook。测试配置档默认不安装，`--listen` 显式开启。
    pub listen: bool,
    external_copy_roots: Option<(PathBuf, PathBuf)>,
}

impl Startup {
    #[cfg(windows)]
    pub fn offer_external_copy(&self) -> Result<(), String> {
        if let Some((root, local_app_data)) = &self.external_copy_roots {
            offer_external_copy(root, local_app_data)?;
        }
        Ok(())
    }

    /// 主窗口和设置页按配置目录与监听模式复用。
    pub fn workspace_key(&self, config: &Path) -> Result<String, String> {
        use sha2::{Digest, Sha256};
        let dir = config.parent().ok_or("找不到配置文件的父目录。")?;
        let dir = if dir.as_os_str().is_empty() {
            Path::new(".")
        } else {
            dir
        };
        std::fs::create_dir_all(dir).map_err(|error| format!("无法打开配置目录：{error}"))?;
        let dir = dir
            .canonicalize()
            .map_err(|error| format!("无法确定配置目录：{error}"))?;
        let id = Sha256::digest(normalize(&dir).to_lowercase().as_bytes());
        Ok(format!("{id:x}-{}", self.listen))
    }

    fn daily() -> Self {
        Self {
            test_name: None,
            test_data: false,
            listen: true,
            external_copy_roots: None,
        }
    }

    fn for_config(config: &Path) -> Self {
        Self {
            test_data: registered_test_data(config),
            ..Self::daily()
        }
    }

    fn for_entry(entry: &Entry, listen: bool) -> Self {
        let test = entry.kind == "test";
        Self {
            test_name: test.then(|| entry.name.clone()),
            test_data: test,
            listen: !test || listen,
            external_copy_roots: None,
        }
    }

    pub fn window_title(&self, base: &str) -> String {
        match &self.test_name {
            Some(name) => format!("{base} — 测试：{name}"),
            None => base.into(),
        }
    }

    /// 测试窗口加载数据前取得共享锁，直到退出。普通窗口、设置及历史入口共用此规则。
    pub fn hold_window_profile(&self, config: &Path) -> Result<Option<std::fs::File>, String> {
        if !self.test_data {
            return Ok(None);
        }
        let directory = profile_directory(config)
            .ok_or_else(|| format!("无法定位测试配置目录：{}", config.display()))?;
        let file = open_profile_use_file(&directory)?;
        file.lock_shared()
            .map_err(|error| format!("无法打开正在重置的测试配置档：{error}"))?;
        Ok(Some(file))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Command {
    Create {
        alias: String,
        seed: Seed,
        name: Option<String>,
    },
    Reset(String),
    List,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Arguments {
    data_root: Option<String>,
    profile: Option<String>,
    test_workspace: bool,
    listen: bool,
    relaunched: Option<String>,
    result_file: Option<String>,
    command: Option<Command>,
}

impl Arguments {
    fn any(&self) -> bool {
        self.data_root.is_some()
            || self.profile.is_some()
            || self.listen
            || self.relaunched.is_some()
            || self.result_file.is_some()
            || self.command.is_some()
    }

    /// 重启时原样转交的配置档参数（不含重启标记本身）。
    fn forwarded(&self, original: &[String]) -> Vec<String> {
        let mut out = Vec::new();
        let mut index = 1;
        while index < original.len() {
            let flag = original[index].as_str();
            match flag {
                "--relaunched" | "--relaunch-result" => index += 2,
                _ => {
                    out.push(original[index].clone());
                    index += 1;
                }
            }
        }
        out
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Seed {
    Empty,
    Fixture(PathBuf),
    Copy(String),
}

impl Seed {
    fn parse(text: &str) -> Result<Self, String> {
        if text == "empty" {
            return Ok(Self::Empty);
        }
        if let Some(path) = text
            .strip_prefix("fixture:")
            .filter(|path| !path.is_empty())
        {
            return Ok(Self::Fixture(PathBuf::from(path)));
        }
        if let Some(source) = text
            .strip_prefix("copy:")
            .filter(|source| !source.is_empty())
        {
            return Ok(Self::Copy(source.into()));
        }
        Err(format!(
            "未知种子 {text}；可用 empty、fixture:<目录> 或 copy:<别名或 ID>。"
        ))
    }

    fn record(&self) -> String {
        match self {
            Self::Empty => "empty".into(),
            Self::Fixture(path) => format!("fixture:{}", path.display()),
            Self::Copy(source) => format!("copy:{source}"),
        }
    }
}

/// 从命令行取出配置档参数，其余参数保持原顺序留给后续入口。
fn take_arguments(arguments: &mut Vec<String>) -> Result<Arguments, String> {
    let mut parsed = Arguments::default();
    let mut create: Option<String> = None;
    let mut kind: Option<String> = None;
    let mut seed: Option<String> = None;
    let mut name: Option<String> = None;
    let mut reset: Option<String> = None;
    let mut list = false;
    let mut kept = Vec::with_capacity(arguments.len());
    let mut iter = arguments.drain(..);
    if let Some(program) = iter.next() {
        kept.push(program);
    }
    fn value(
        flag: &str,
        slot: &mut Option<String>,
        iter: &mut impl Iterator<Item = String>,
    ) -> Result<(), String> {
        if slot.is_some() {
            return Err(format!("{flag} 只能出现一次。"));
        }
        match iter.next() {
            Some(text) if !text.is_empty() && !text.starts_with("--") => {
                *slot = Some(text);
                Ok(())
            }
            _ => Err(format!("{flag} 后需要一个值。")),
        }
    }
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--data-root" => value("--data-root", &mut parsed.data_root, &mut iter)?,
            "--profile" => value("--profile", &mut parsed.profile, &mut iter)?,
            "--test-workspace" if !parsed.test_workspace => parsed.test_workspace = true,
            "--relaunched" => value("--relaunched", &mut parsed.relaunched, &mut iter)?,
            "--relaunch-result" => value("--relaunch-result", &mut parsed.result_file, &mut iter)?,
            "--profile-create" => value("--profile-create", &mut create, &mut iter)?,
            "--profile-reset" => value("--profile-reset", &mut reset, &mut iter)?,
            "--type" if create.is_some() => value("--type", &mut kind, &mut iter)?,
            "--seed" if create.is_some() => value("--seed", &mut seed, &mut iter)?,
            "--name" if create.is_some() => value("--name", &mut name, &mut iter)?,
            "--listen" if !parsed.listen => parsed.listen = true,
            "--profile-list" if !list => list = true,
            "--listen" | "--profile-list" | "--test-workspace" => {
                return Err(format!("{argument} 只能出现一次。"))
            }
            _ => kept.push(argument),
        }
    }
    drop(iter);
    *arguments = kept;
    if parsed.test_workspace {
        if parsed.profile.is_some() {
            return Err("--test-workspace 不能与 --profile 同用。".into());
        }
        parsed.profile = Some(TEST_ALIAS.into());
    }
    let commands = usize::from(create.is_some()) + usize::from(reset.is_some()) + usize::from(list);
    if commands > 1 {
        return Err("--profile-create、--profile-reset 与 --profile-list 只能选一个。".into());
    }
    if create.is_none() && (kind.is_some() || seed.is_some() || name.is_some()) {
        return Err("--type、--seed、--name 只用于 --profile-create。".into());
    }
    if commands == 1 && (parsed.profile.is_some() || parsed.listen) {
        return Err("配置档命令不能与 --profile 或 --listen 同用。".into());
    }
    if parsed.listen && parsed.profile.is_none() {
        return Err("--listen 只用于 --profile 指定的测试配置档。".into());
    }
    if let Some(alias) = create {
        validate_alias(&alias)?;
        if kind.as_deref().unwrap_or("test") != "test" {
            return Err("目前只能创建测试配置档：--type test。".into());
        }
        let seed = Seed::parse(seed.as_deref().unwrap_or("empty"))?;
        parsed.command = Some(Command::Create { alias, seed, name });
    } else if let Some(alias) = reset {
        parsed.command = Some(Command::Reset(alias));
    } else if list {
        parsed.command = Some(Command::List);
    }
    Ok(parsed)
}

fn validate_alias(alias: &str) -> Result<(), String> {
    let mut chars = alias.chars();
    let valid = alias.len() <= 40
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !valid || alias == DEFAULT_ID {
        return Err(format!(
            "别名 {alias} 无效：只能用小写字母、数字和 -，以字母或数字开头，最长 40 个字符，且不能是 default。"
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RootSource {
    Flag,
    Env,
    Default,
}

impl RootSource {
    fn label(self) -> &'static str {
        match self {
            Self::Flag => "--data-root",
            Self::Env => "OMNITRIGGER_HOME",
            Self::Default => "default",
        }
    }
}

fn resolve_root(
    flag: Option<&str>,
    env: Option<std::ffi::OsString>,
    local_app_data: Option<PathBuf>,
) -> Result<(PathBuf, RootSource), String> {
    let absolute = |path: PathBuf| {
        std::path::absolute(&path)
            .map_err(|error| format!("数据根 {} 无效：{error}", path.display()))
    };
    if let Some(flag) = flag {
        return Ok((absolute(PathBuf::from(flag))?, RootSource::Flag));
    }
    if let Some(env) = env.filter(|value| !value.is_empty()) {
        return Ok((absolute(PathBuf::from(env))?, RootSource::Env));
    }
    let base = local_app_data.ok_or("找不到 LOCALAPPDATA，请用 --data-root 指定数据根。")?;
    Ok((base.join("OmniTrigger"), RootSource::Default))
}

fn normalize(path: &Path) -> String {
    let text = path.to_string_lossy();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    text.replace('/', "\\").trim_end_matches('\\').to_string()
}

/// 实际目录落在 `<LOCALAPPDATA>\Packages\<包名>\` 之下且与预期不同时，返回包名。
fn redirected_package(expected: &Path, actual: &Path, local_app_data: &Path) -> Option<String> {
    let actual = normalize(actual);
    if actual.eq_ignore_ascii_case(&normalize(expected)) {
        return None;
    }
    let prefix = format!("{}\\Packages\\", normalize(local_app_data));
    if !actual.get(..prefix.len())?.eq_ignore_ascii_case(&prefix) {
        return None;
    }
    actual[prefix.len()..]
        .split('\\')
        .next()
        .filter(|name| !name.is_empty())
        .map(str::to_string)
}

/// 按 Windows 命令行规则（MSVC 运行库解析方式）转义单个参数。
fn quote_argument(argument: &str) -> String {
    if !argument.is_empty() && !argument.contains([' ', '\t', '\n', '"']) {
        return argument.into();
    }
    let mut out = String::from('"');
    let mut slashes = 0;
    for c in argument.chars() {
        match c {
            '\\' => slashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(slashes * 2 + 1));
                out.push('"');
                slashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(slashes));
                out.push(c);
                slashes = 0;
            }
        }
    }
    out.push_str(&"\\".repeat(slashes * 2));
    out.push('"');
    out
}

fn join_arguments(arguments: &[String]) -> String {
    arguments
        .iter()
        .map(|argument| quote_argument(argument))
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    version: u32,
    profiles: Vec<Entry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    id: String,
    alias: String,
    kind: String,
    name: String,
    seed: String,
    created_ms: u64,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or_default()
}

fn new_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or_default(),
    );
    hasher.write_u32(std::process::id());
    format!("t{:012x}", hasher.finish() & 0xffff_ffff_ffff)
}

fn catalog_path(root: &Path) -> PathBuf {
    root.join("profiles.json")
}

fn profile_dir(root: &Path, id: &str) -> PathBuf {
    root.join("profiles").join(id)
}

/// 只定位配置文件自己的实际目录；新建空白档可能还没有 bindings.json。
fn profile_directory(config_path: &Path) -> Option<PathBuf> {
    let parent = config_path.parent()?;
    let parent = if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    };
    let directory = match parent.canonicalize() {
        Ok(directory) => directory,
        // 重置替换目录的间隙，仍可由父目录和登记的 ID 确定配置档。
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => parent
            .parent()?
            .canonicalize()
            .ok()?
            .join(parent.file_name()?),
        Err(_) => return None,
    };
    directory
        .parent()?
        .file_name()?
        .eq_ignore_ascii_case("profiles")
        .then_some(directory)
}

impl Entry {
    fn is_test_at(&self, directory: &Path) -> bool {
        self.kind == "test"
            && directory.file_name().is_some_and(|id| {
                let expected = std::ffi::OsStr::new(&self.id);
                if cfg!(windows) {
                    id.eq_ignore_ascii_case(expected)
                } else {
                    id == expected
                }
            })
    }
}

/// 旧版 --config 仍从文件所在目录读取动作数据。以实际目录和登记表确认其身份。
fn registered_test_data(config_path: &Path) -> bool {
    let Some(directory) = profile_directory(config_path) else {
        return false;
    };
    let Some(root) = directory.parent().and_then(Path::parent) else {
        return false;
    };
    if let Ok(catalog) = read_catalog(root) {
        if catalog
            .profiles
            .iter()
            .any(|entry| entry.is_test_at(&directory))
        {
            return true;
        }
        // 有效总登记是权威来源；别名、备份和相邻的普通目录都不算已登记的测试档。
        if catalog_path(root).exists() {
            return false;
        }
    }
    // 总登记缺失或损坏时，以原目录自己的身份文件保守隔离动作，保留 --config 启动能力。
    std::fs::read_to_string(directory.join("profile.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Entry>(&text).ok())
        .as_ref()
        .is_some_and(|entry| entry.is_test_at(&directory))
}

/// 按复制顺序向上追溯种子，只读本根总登记。损坏或循环的登记链不能用作安装来源。
pub(crate) fn seed_sources_for_config(config: &Path) -> Vec<PathBuf> {
    seed_paths(config, SeedPaths::All)
}

/// 只有已登记配置档的安装可用于运行复制来的脚本；夹具仅提供动作和测试数据。
pub(crate) fn copied_profile_sources(config: &Path) -> Vec<PathBuf> {
    seed_paths(config, SeedPaths::Profiles)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SeedPaths {
    All,
    Profiles,
}

fn seed_paths(config: &Path, paths: SeedPaths) -> Vec<PathBuf> {
    let Some(directory) = profile_directory(config) else {
        return Vec::new();
    };
    let Some(root) = directory.parent().and_then(Path::parent) else {
        return Vec::new();
    };
    let Ok(catalog) = read_catalog(root) else {
        return Vec::new();
    };
    let Some(entry) = catalog
        .profiles
        .iter()
        .find(|entry| entry.is_test_at(&directory))
    else {
        return Vec::new();
    };
    if paths == SeedPaths::Profiles {
        return trace_seed_sources(root, None, &catalog, entry, paths);
    }
    // TEMP、8.3 短名或目录联接可能让调用者使用另一条数据根路径。只从已经
    // 核对身份的目标档推导这个别名，不打开可能已移走的种子目录。
    let supplied = std::path::absolute(config).ok();
    let root_alias = supplied
        .as_deref()
        .and_then(Path::parent)
        .filter(|dir| entry.is_test_at(dir))
        .and_then(Path::parent)
        .filter(|dir| {
            dir.file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("profiles"))
        })
        .and_then(Path::parent)
        .filter(|alias| !normalize(alias).eq_ignore_ascii_case(&normalize(root)));
    trace_seed_sources(root, root_alias, &catalog, entry, paths)
}

fn trace_seed_sources<'a>(
    root: &Path,
    root_alias: Option<&Path>,
    catalog: &'a Catalog,
    mut entry: &'a Entry,
    paths: SeedPaths,
) -> Vec<PathBuf> {
    let mut seen = BTreeSet::from([normalize(&profile_dir(root, &entry.id)).to_lowercase()]);
    if let Some(alias) = root_alias {
        seen.insert(normalize(&profile_dir(alias, &entry.id)).to_lowercase());
    }
    let mut sources = Vec::new();
    loop {
        let (path, alias, upstream) = match Seed::parse(&entry.seed) {
            Ok(Seed::Empty) => break,
            Ok(Seed::Copy(key)) if key == DEFAULT_ID => {
                (root.to_path_buf(), root_alias.map(Path::to_path_buf), None)
            }
            Ok(Seed::Copy(key)) => {
                let Some(source) = catalog.source_profile(&key) else {
                    return Vec::new();
                };
                (
                    profile_dir(root, &source.id),
                    root_alias.map(|alias| profile_dir(alias, &source.id)),
                    Some(source),
                )
            }
            Ok(Seed::Fixture(_)) if paths == SeedPaths::Profiles => break,
            Ok(Seed::Fixture(path)) if path.is_absolute() => (path, None, None),
            _ => return Vec::new(),
        };
        for candidate in std::iter::once(path).chain(alias) {
            if !seen.insert(normalize(&candidate).to_lowercase()) {
                return Vec::new();
            }
            sources.push(candidate);
        }
        let Some(source) = upstream else { break };
        entry = source;
    }
    sources
}

fn read_catalog(root: &Path) -> Result<Catalog, String> {
    let path = catalog_path(root);
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            let catalog: Catalog = serde_json::from_str(&text)
                .map_err(|error| format!("{} 格式有误：{error}", path.display()))?;
            if catalog.version != CATALOG_VERSION {
                return Err(format!(
                    "{} 的版本 {} 不受支持。",
                    path.display(),
                    catalog.version
                ));
            }
            Ok(catalog)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => recover_catalog(root),
        Err(error) => Err(format!("读取 {} 失败：{error}", path.display())),
    }
}

/// 总登记缺失时，只从本根直属的配置档身份文件恢复，不读取热键、动作或历史。
fn recover_catalog(root: &Path) -> Result<Catalog, String> {
    let mut catalog = Catalog {
        version: CATALOG_VERSION,
        profiles: Vec::new(),
    };
    let directory = root.join("profiles");
    let fail = |error| format!("无法恢复 {} 中的配置档登记：{error}", directory.display());
    match directory.symlink_metadata() {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(catalog),
        Err(error) => return Err(fail(error)),
        Ok(_) if !crate::local_files::is_plain_directory(&directory) => {
            return Err(format!(
                "无法从普通本机目录读取配置档：{}",
                directory.display()
            ));
        }
        Ok(_) => {}
    }
    let mut keys = BTreeSet::new();
    for item in std::fs::read_dir(&directory).map_err(fail)? {
        let item = item.map_err(fail)?;
        let id = item.file_name();
        // 历代测试档均由 new_id 生成。忽略旁置的备份、别名目录及非本机条目。
        let Some(hex) = id.to_str().and_then(|s| s.strip_prefix(['t', 'T'])) else {
            continue;
        };
        if hex.len() != 12 || !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        let metadata = item.metadata().map_err(fail)?;
        if !metadata.is_dir() || !crate::local_files::is_plain(&metadata) {
            continue;
        }
        let path = item.path().join("profile.json");
        let unavailable = |error| format!("无法读取配置档身份文件 {}：{error}", path.display());
        let metadata = path.symlink_metadata().map_err(unavailable)?;
        if !metadata.is_file() || !crate::local_files::is_plain(&metadata) {
            return Err(format!(
                "配置档身份文件须为普通本机文件：{}",
                path.display()
            ));
        }
        let text = std::fs::read_to_string(&path).map_err(unavailable)?;
        let entry: Entry = serde_json::from_str(&text)
            .map_err(|error| format!("{} 格式有误：{error}", path.display()))?;
        if !entry.is_test_at(&item.path()) {
            return Err(format!("配置档身份与目录不符：{}", path.display()));
        }
        validate_alias(&entry.alias)?;
        let key = entry.id.to_ascii_lowercase();
        if (entry.alias != key && !keys.insert(entry.alias.clone())) || !keys.insert(key) {
            return Err(format!("配置档标识或别名重复：{}", path.display()));
        }
        catalog.profiles.push(entry);
    }
    catalog
        .profiles
        .sort_by(|a, b| (a.created_ms, &a.id).cmp(&(b.created_ms, &b.id)));
    Ok(catalog)
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let text = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    let staging = path.with_extension("json.tmp");
    std::fs::write(&staging, text)
        .and_then(|()| std::fs::rename(&staging, path))
        .map_err(|error| format!("写入 {} 失败：{error}", path.display()))
}

impl Catalog {
    fn find(&self, key: &str) -> Option<&Entry> {
        self.profiles
            .iter()
            .find(|entry| entry.alias == key || entry.id == key)
    }

    fn source_profile(&self, key: &str) -> Option<&Entry> {
        self.find(key).filter(|entry| {
            Path::new(&entry.id).file_name() == Some(std::ffi::OsStr::new(&entry.id))
        })
    }
}

/// 参与复制、导入与备份的数据文件：顶层普通文件，排除锁、旧备份与配置档自身的登记文件。
fn is_data_file(name: &str) -> bool {
    #[cfg(windows)]
    let name = name.to_ascii_lowercase();
    !(name.ends_with(".lock")
        || name.ends_with(".bak")
        || name.ends_with(".tmp")
        || name.starts_with(".root-probe-")
        || name.contains(".previous-registration-")
        || name == "profiles.json"
        || name == "profile.json")
}

/// 浏览器连接器的程序路径与桥接进程属于当前安装，只在原配置的备份中保留。
fn transferable_file(name: &str) -> bool {
    is_data_file(name) && !name.eq_ignore_ascii_case("browser-connector.json")
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.path().symlink_metadata()?;
        if !crate::local_files::is_plain(&kind) || crate::local_files::is_cloud_path(&entry.path())
        {
            continue;
        }
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// 复制配置档的顶层数据、托管动作包与 RaiShot 自定义动作清单。
fn copy_data(from: &Path, to: &Path, include_file: impl Fn(&str) -> bool) -> Result<usize, String> {
    let fail = |error: std::io::Error| {
        format!(
            "从 {} 复制数据到 {} 失败：{error}",
            from.display(),
            to.display()
        )
    };
    let source_manifest = from.join(RAISHOT_MANIFEST_PATH);
    let manifest = read_local_manifest(&source_manifest)
        .map_err(|error| format!("读取 {} 失败：{error}", source_manifest.display()))?;
    let destination = to.join(RAISHOT_MANIFEST_PATH);
    if manifest.is_some() {
        local_manifest_metadata(&destination)
            .map_err(|error| format!("检查 {} 失败：{error}", destination.display()))?;
    }
    std::fs::create_dir_all(to).map_err(fail)?;
    let mut copied = 0;
    for entry in std::fs::read_dir(from).map_err(fail)? {
        let entry = entry.map_err(fail)?;
        let name = entry.file_name();
        let kind = entry.path().symlink_metadata().map_err(fail)?;
        if kind.is_file()
            && crate::local_files::is_plain(&kind)
            && !crate::local_files::is_cloud_path(&entry.path())
            && include_file(&name.to_string_lossy())
        {
            std::fs::copy(entry.path(), to.join(name)).map_err(fail)?;
            copied += 1;
        }
    }
    // 与动作目录服务使用同一路径，由文件系统解析大小写；不跟随目录符号链接。
    let actions = from.join("actions");
    match actions.symlink_metadata() {
        Ok(metadata) if metadata.is_dir() && crate::local_files::is_plain(&metadata) => {
            copy_tree(&actions, &to.join("actions")).map_err(fail)?
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(fail(error)),
    }
    if let Some(bytes) = manifest {
        crate::settings::atomic_write(&destination, &bytes)?;
    }
    Ok(copied)
}

/// 配置档数据的最近修改时间。仅读元数据。
fn data_modified_ms(directory: &Path) -> Option<u64> {
    if !crate::local_files::is_plain_directory(directory) {
        return None;
    }
    let mut latest = local_manifest_metadata(&directory.join(RAISHOT_MANIFEST_PATH))
        .ok()
        .flatten()
        .and_then(|metadata| metadata.modified().ok());
    let mut directories = vec![directory.to_owned()];
    while let Some(path) = directories.pop() {
        let Ok(entries) = std::fs::read_dir(&path) else {
            continue;
        };
        let top = path == directory;
        for entry in entries.flatten() {
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if !crate::local_files::is_plain(&metadata) {
                continue;
            }
            let name = entry.file_name();
            if metadata.is_dir() {
                let actions = if cfg!(windows) {
                    name.eq_ignore_ascii_case("actions")
                } else {
                    name == "actions"
                };
                if (!top || actions) && !crate::local_files::is_cloud_path(Path::new(&name)) {
                    directories.push(entry.path());
                }
            } else if metadata.is_file() && (!top || transferable_file(&name.to_string_lossy())) {
                latest = latest.max(metadata.modified().ok());
            }
        }
    }
    Some(
        latest?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_millis() as u64,
    )
}

fn source_dir(root: &Path, catalog: &Catalog, key: &str) -> Result<PathBuf, String> {
    if key == DEFAULT_ID {
        return Ok(root.to_path_buf());
    }
    catalog
        .source_profile(key)
        .map(|entry| profile_dir(root, &entry.id))
        .ok_or_else(|| format!("找不到配置档 {key}；可用 --profile-list 查看。"))
}

fn seed_into(root: &Path, catalog: &Catalog, seed: &Seed, target: &Path) -> Result<(), String> {
    match seed {
        Seed::Empty => std::fs::create_dir_all(target)
            .map_err(|error| format!("创建 {} 失败：{error}", target.display())),
        Seed::Fixture(path) => {
            if !path.is_dir() {
                return Err(format!("夹具目录 {} 不存在。", path.display()));
            }
            copy_data(path, target, transferable_file).map(drop)
        }
        Seed::Copy(key) => copy_data(&source_dir(root, catalog, key)?, target, |name| {
            transferable_file(name)
                && !name.eq_ignore_ascii_case(crate::clipboard_history_store::FILE_NAME)
        })
        .map(drop),
    }
}

fn create_profile(
    root: &Path,
    alias: &str,
    seed: &Seed,
    name: Option<&str>,
) -> Result<Entry, String> {
    let _lock = lock_catalog(root)?;
    let mut catalog = read_catalog(root)?;
    if catalog.find(alias).is_some() {
        return Err(format!(
            "配置档 {alias} 已存在；要恢复种子数据请用 --profile-reset {alias}。"
        ));
    }
    insert_profile(root, &mut catalog, alias, seed, name)
}

fn lock_catalog(root: &Path) -> Result<std::fs::File, String> {
    std::fs::create_dir_all(root).map_err(|error| error.to_string())?;
    let lock = std::fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("profiles.lock"))
        .map_err(|error| format!("打开配置档目录锁失败：{error}"))?;
    lock.lock()
        .map_err(|error| format!("锁定配置档目录失败：{error}"))?;
    Ok(lock)
}

fn testing_workspace(root: &Path) -> Result<Entry, String> {
    let _lock = lock_catalog(root)?;
    let mut catalog = read_catalog(root)?;
    if let Some(entry) = catalog.find(TEST_ALIAS) {
        if entry.kind != "test" {
            return Err(format!("配置档 {TEST_ALIAS} 已用于其他用途。"));
        }
        persist_recovered_catalog(root, &catalog)?;
        return Ok(entry.clone());
    }
    insert_profile(
        root,
        &mut catalog,
        TEST_ALIAS,
        &Seed::Empty,
        Some("本机测试"),
    )
}

fn persist_recovered_catalog(root: &Path, catalog: &Catalog) -> Result<(), String> {
    let path = catalog_path(root);
    if !path.try_exists().map_err(|error| error.to_string())? {
        write_json(&path, catalog)?;
    }
    Ok(())
}

fn daily_test_copy(root: &Path) -> Result<Entry, String> {
    let _lock = lock_catalog(root)?;
    let mut catalog = read_catalog(root)?;
    let seed = Seed::Copy(DEFAULT_ID.into());
    let mut alias = String::from("daily-copy");
    let mut suffix = 2;
    while let Some(entry) = catalog.find(&alias) {
        if entry.alias == alias && entry.kind == "test" && entry.seed == "copy:default" {
            persist_recovered_catalog(root, &catalog)?;
            return Ok(entry.clone());
        }
        // 别名可能已被用户用于其他数据，保留原配置档。
        alias = format!("daily-copy-{suffix}");
        suffix += 1;
    }
    insert_profile(root, &mut catalog, &alias, &seed, Some("日常配置副本"))
}

pub(super) fn open_daily_test_copy(root: &Path) -> Result<(), String> {
    let program = std::env::current_exe().map_err(|error| error.to_string())?;
    let entry = daily_test_copy(root)?;
    let mut launch = std::process::Command::new(program);
    launch
        .arg("--data-root")
        .arg(root)
        .arg("--profile")
        .arg(entry.id);
    launch
        .env_remove("OMNITRIGGER_UPDATE_HEALTH")
        .env_remove("OMNITRIGGER_UPDATE_ERROR");
    launch
        .spawn()
        .map_err(|error| format!("无法启动测试窗口：{error}"))?;
    Ok(())
}

fn insert_profile(
    root: &Path,
    catalog: &mut Catalog,
    alias: &str,
    seed: &Seed,
    name: Option<&str>,
) -> Result<Entry, String> {
    let seed = match seed {
        Seed::Fixture(path) => Seed::Fixture(
            std::path::absolute(path)
                .map_err(|error| format!("夹具目录 {} 无效：{error}", path.display()))?,
        ),
        other => other.clone(),
    };
    let entry = Entry {
        id: new_id(),
        alias: alias.into(),
        kind: "test".into(),
        name: name.unwrap_or(alias).into(),
        seed: seed.record(),
        created_ms: now_ms(),
    };
    let directory = profile_dir(root, &entry.id);
    let result = seed_into(root, catalog, &seed, &directory)
        .and_then(|()| write_json(&directory.join("profile.json"), &entry))
        .and_then(|()| {
            catalog.profiles.push(entry.clone());
            write_json(&catalog_path(root), catalog)
        });
    if let Err(error) = result {
        let _ = std::fs::remove_dir_all(&directory);
        return Err(error);
    }
    Ok(entry)
}

/// 用记录的种子重建测试配置档；原目录整体移入 `backups\profile-reset\`。
fn reset_profile(root: &Path, key: &str) -> Result<(Entry, Option<PathBuf>), ProfileCommandError> {
    let _lock = lock_catalog(root)?;
    let catalog = read_catalog(root)?;
    let entry = catalog
        .find(key)
        .cloned()
        .ok_or_else(|| format!("找不到配置档 {key}；可用 --profile-list 查看。"))?;
    if entry.kind != "test" {
        return Err(format!("只有测试配置档可以重置，{key} 的类型是 {}。", entry.kind).into());
    }
    let seed = Seed::parse(&entry.seed)?;
    let directory = profile_dir(root, &entry.id);
    let use_file = open_profile_use_file(&directory)?;
    use_file.try_lock().map_err(|error| match error {
        std::fs::TryLockError::WouldBlock => ProfileCommandError::InUse(entry.alias.clone()),
        std::fs::TryLockError::Error(error) => format!("无法锁定测试配置档：{error}").into(),
    })?;
    let backup_root = root.join("backups").join("profile-reset");
    let tag = format!("{}-{}-{}", entry.id, now_ms(), new_id());
    let backup = backup_root.join(&tag);
    let staging = backup_root.join(format!(".{tag}.tmp"));
    std::fs::create_dir_all(&backup_root)
        .map_err(|error| format!("创建 {} 失败：{error}", backup_root.display()))?;
    std::fs::create_dir(&staging)
        .map_err(|error| format!("创建 {} 失败：{error}", staging.display()))?;
    let result = (|| {
        seed_into(root, &catalog, &seed, &staging)?;
        write_json(&staging.join("profile.json"), &entry)?;
        std::fs::create_dir_all(root.join("profiles")).map_err(|error| error.to_string())?;
        let backed_up = match std::fs::rename(&directory, &backup) {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(format!("备份 {} 失败：{error}", directory.display())),
        };
        if let Err(error) = std::fs::rename(&staging, &directory) {
            if backed_up {
                std::fs::rename(&backup, &directory).map_err(|restore| {
                    format!(
                        "重置失败：{error}；恢复原配置失败：{restore}。原配置位于 {}",
                        backup.display()
                    )
                })?;
            }
            return Err(format!("替换 {} 失败：{error}", directory.display()));
        }
        Ok(backed_up.then_some(backup))
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    Ok((entry, result?))
}

/// 锁文件与配置档目录并列，两次目录重命名不会改变打开的锁。
fn open_profile_use_file(directory: &Path) -> Result<std::fs::File, String> {
    let path = directory.with_extension("window.lock");
    let parent = path.parent().ok_or("找不到测试配置档的父目录。")?;
    std::fs::create_dir_all(parent)
        .and_then(|()| {
            std::fs::File::options()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&path)
        })
        .map_err(|error| format!("打开测试配置档使用锁 {} 失败：{error}", path.display()))
}

fn list_profiles(root: &Path, source: RootSource) -> Result<serde_json::Value, String> {
    let catalog = read_catalog(root)?;
    let mut profiles = vec![serde_json::json!({
        "id": DEFAULT_ID,
        "alias": DEFAULT_ID,
        "kind": "local",
        "name": "我的配置",
        "directory": root,
        "default": true,
    })];
    profiles.extend(catalog.profiles.iter().map(|entry| {
        serde_json::json!({
            "id": entry.id,
            "alias": entry.alias,
            "kind": entry.kind,
            "name": entry.name,
            "seed": entry.seed,
            "directory": profile_dir(root, &entry.id),
            "default": false,
        })
    }));
    Ok(serde_json::json!({
        "status": "ok",
        "data_root": root,
        "data_root_source": source.label(),
        "profiles": profiles,
    }))
}

#[derive(Debug)]
enum ProfileCommandError {
    InUse(String),
    Other(String),
}

impl From<String> for ProfileCommandError {
    fn from(message: String) -> Self {
        Self::Other(message)
    }
}

impl ProfileCommandError {
    fn response(self) -> String {
        match self {
            Self::InUse(alias) => rejection(
                "profile_in_use",
                &format!("测试配置档 {alias} 正在使用，请关闭使用该配置档的窗口后再重置。"),
            ),
            Self::Other(message) => rejected(&message),
        }
    }
}

fn run_command(
    root: &Path,
    source: RootSource,
    command: &Command,
) -> Result<String, ProfileCommandError> {
    let value = match command {
        Command::Create { alias, seed, name } => {
            let entry = create_profile(root, alias, seed, name.as_deref())?;
            serde_json::json!({
                "status": "created",
                "id": entry.id,
                "alias": entry.alias,
                "seed": entry.seed,
                "directory": profile_dir(root, &entry.id),
            })
        }
        Command::Reset(key) => {
            let (entry, backup) = reset_profile(root, key)?;
            serde_json::json!({
                "status": "reset",
                "id": entry.id,
                "alias": entry.alias,
                "seed": entry.seed,
                "directory": profile_dir(root, &entry.id),
                "backup": backup,
            })
        }
        Command::List => list_profiles(root, source)?,
    };
    Ok(value.to_string())
}

fn rejected(message: &str) -> String {
    rejection("invalid_profile_arguments", message)
}

fn rejection(code: &str, message: &str) -> String {
    serde_json::json!({"status":"rejected","error":{
        "code":code,"message":message,"effect_state":"not_started"
    }})
    .to_string()
}

fn exit_rejected(message: &str) -> ! {
    eprintln!("{}", rejected(message));
    std::process::exit(1);
}

#[derive(Serialize, Deserialize)]
struct Relayed {
    ok: bool,
    text: String,
}

fn finish_command(result: Result<String, String>, result_file: Option<&str>) -> ! {
    let ok = result.is_ok();
    let text = result.unwrap_or_else(|error| error);
    if let Some(file) = result_file {
        let path = Path::new(file);
        let _ = write_json(path, &Relayed { ok, text });
    } else if ok {
        println!("{text}");
    } else {
        eprintln!("{text}");
    }
    std::process::exit(if ok { 0 } else { 1 });
}

/// 解析配置档参数并设置配置路径。配置档命令、脱离重定向的重启在此直接退出进程。
pub fn startup(arguments: &mut Vec<String>) -> Startup {
    let original = arguments.clone();
    let parsed = take_arguments(arguments).unwrap_or_else(|message| exit_rejected(&message));
    let config_index = arguments.iter().position(|argument| argument == "--config");
    if let Some(index) = config_index {
        if parsed.any() {
            exit_rejected(
                "--config 是临时配置路径，不能与 --data-root、--profile 或配置档命令同用。",
            );
        }
        return arguments
            .get(index + 1)
            .filter(|path| !path.starts_with("--"))
            .map_or_else(Startup::daily, |path| Startup::for_config(Path::new(path)));
    }
    let plain = arguments.len() == 1;
    if parsed.command.is_some() && !plain {
        exit_rejected("配置档命令不能与其他参数混用。");
    }
    let interactive =
        arguments.iter().skip(1).all(|arg| arg == "--settings") && parsed.command.is_none();
    let check_config = matches!(&arguments[1..], [arg] if arg == "--check-config");
    let env_root = std::env::var_os("OMNITRIGGER_HOME").filter(|root| !root.is_empty());
    if !parsed.any() && !interactive && !check_config && env_root.is_none() {
        // 未指定数据根的其他命令无需探测桌面或预先设置路径。
        return Startup::daily();
    }
    let local_app_data = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    if check_config && !parsed.any() && env_root.is_none() && local_app_data.is_none() {
        return Startup::daily(); // 沿用 settings::path() 的临时目录回退。
    }
    let (root, source) = resolve_root(
        parsed.data_root.as_deref(),
        env_root,
        local_app_data.clone(),
    )
    .unwrap_or_else(|message| exit_rejected(&message));
    if plain || interactive || check_config {
        #[cfg(windows)]
        escape_redirect(
            &root,
            local_app_data.as_deref(),
            &parsed,
            &original,
            check_config,
        );
    }
    if let Some(link) = &parsed.relaunched {
        let _ = std::fs::remove_file(link);
    }
    if let Some(command) = &parsed.command {
        finish_command(
            std::fs::create_dir_all(&root)
                .map_err(|error| {
                    ProfileCommandError::Other(format!(
                        "创建数据根 {} 失败：{error}",
                        root.display()
                    ))
                })
                .and_then(|()| run_command(&root, source, command))
                .map_err(ProfileCommandError::response),
            parsed.result_file.as_deref(),
        );
    }
    let profile = parsed.profile.as_deref().filter(|key| *key != DEFAULT_ID);
    let (config, startup) = match profile {
        Some(key) => {
            let result = if parsed.test_workspace {
                testing_workspace(&root)
            } else {
                read_catalog(&root).and_then(|catalog| {
                    catalog.find(key).cloned().ok_or_else(|| {
                        format!("找不到配置档 {key}；先用 --profile-create {key} --type test 创建，或用 --profile-list 查看。")
                    })
                })
            };
            let entry = result.unwrap_or_else(|message| {
                #[cfg(windows)]
                if interactive {
                    omnitrigger_windows::show_error_dialog("OmniTrigger", &message);
                }
                finish_command(Err(rejected(&message)), parsed.result_file.as_deref())
            });
            (
                Some(profile_dir(&root, &entry.id).join("bindings.json")),
                Startup::for_entry(&entry, parsed.listen),
            )
        }
        None => {
            let config = (source != RootSource::Default).then(|| root.join("bindings.json"));
            let mut startup = config
                .as_deref()
                .map_or_else(Startup::daily, Startup::for_config);
            if interactive && source == RootSource::Default {
                startup.external_copy_roots = local_app_data.map(|local| (root, local));
            }
            (config, startup)
        }
    };
    if let Some(config) = config {
        if let Err(message) = crate::settings::use_path(config) {
            finish_command(Err(rejected(&message)), parsed.result_file.as_deref());
        }
    }
    if check_config {
        finish_command(crate::check_config(), parsed.result_file.as_deref());
    }
    startup
}

#[cfg(windows)]
fn fail_redirect(is_command: bool, result_file: Option<&str>, message: &str) -> ! {
    if is_command {
        finish_command(Err(rejected(message)), result_file);
    }
    omnitrigger_windows::show_error_dialog("OmniTrigger 无法使用真实数据目录", message);
    std::process::exit(1);
}

/// 只读校验逐个检查将读取的现存文件。MSIX 可以在同一个目录内只重定向部分文件。
#[cfg(windows)]
fn redirected_check_file(
    root: &Path,
    parsed: &Arguments,
    local: &Path,
) -> Result<Option<String>, String> {
    fn package(file: &Path, local: &Path) -> Result<Option<String>, String> {
        if !file
            .try_exists()
            .map_err(|error| format!("检查 {} 失败：{error}", file.display()))?
        {
            return Ok(None);
        }
        let actual = omnitrigger_windows::final_file_path(file)?;
        Ok(redirected_package(file, &actual, local))
    }
    let dir = match parsed.profile.as_deref().filter(|key| *key != DEFAULT_ID) {
        Some(key) => {
            if let Some(name) = package(&catalog_path(root), local)? {
                return Ok(Some(name));
            }
            let catalog = read_catalog(root)?;
            let Some(entry) = catalog.find(key) else {
                if parsed.test_workspace {
                    // 首次创建 testing 会写登记文件，沿用普通启动时的写入探针。
                    std::fs::create_dir_all(root).map_err(|error| error.to_string())?;
                    let actual = omnitrigger_windows::probe_final_directory(root)?;
                    return Ok(redirected_package(root, &actual, local));
                }
                return Ok(None);
            };
            profile_dir(root, &entry.id)
        }
        None => root.to_path_buf(),
    };
    for name in ["bindings.json", "wheels.json"] {
        if let Some(name) = package(&dir.join(name), local)? {
            return Ok(Some(name));
        }
    }
    Ok(None)
}

/// 数据根被打包应用重定向时，经系统外壳重启自身；命令等待新进程转交结果。
#[cfg(windows)]
fn escape_redirect(
    root: &Path,
    local_app_data: Option<&Path>,
    parsed: &Arguments,
    original: &[String],
    check_config: bool,
) {
    let Some(local_app_data) = local_app_data else {
        return;
    };
    let is_command = parsed.command.is_some() || check_config;
    let fail =
        |message: &str| -> ! { fail_redirect(is_command, parsed.result_file.as_deref(), message) };
    let redirected = if check_config {
        redirected_check_file(root, parsed, local_app_data)
    } else {
        std::fs::create_dir_all(root)
            .map_err(|error| format!("创建数据根 {} 失败：{error}", root.display()))
            .and_then(|()| omnitrigger_windows::probe_final_directory(root))
            .map(|actual| redirected_package(root, &actual, local_app_data))
    };
    let Some(package) = redirected.unwrap_or_else(|error| fail(&error)) else {
        return;
    };
    if parsed.relaunched.is_some() {
        fail(&format!(
            "当前进程仍处在打包应用 {package} 的文件重定向中。\n\n预期数据目录：\n{}\n\n请从开始菜单或资源管理器启动 OmniTrigger，或用 --data-root 显式指定数据目录。",
            root.display()
        ));
    }
    let program = std::env::current_exe()
        .and_then(std::path::absolute)
        .map_err(|error| error.to_string())
        .and_then(|path| omnitrigger_windows::final_file_path(&path))
        .unwrap_or_else(|error| fail(&format!("无法确定程序路径：{error}")));
    let scratch = std::env::temp_dir().join("OmniTrigger");
    let _ = std::fs::create_dir_all(&scratch);
    let tag = format!("{}-{}", std::process::id(), now_ms());
    let link = scratch.join(format!("relaunch-{tag}.lnk"));
    let result_file = scratch.join(format!("relaunch-{tag}.json"));
    let mut forwarded = parsed.forwarded(original);
    if check_config
        || parsed.data_root.is_some()
        || std::env::var_os("OMNITRIGGER_HOME").is_some_and(|root| !root.is_empty())
    {
        // 外壳使用自己的工作目录与环境，显式选择的数据根按已解析的绝对路径转交。
        if let Some(index) = forwarded.iter().position(|arg| arg == "--data-root") {
            forwarded[index + 1] = root.to_string_lossy().into_owned();
        } else {
            forwarded.extend(["--data-root".into(), root.to_string_lossy().into_owned()]);
        }
    }
    forwarded.extend(["--relaunched".into(), link.to_string_lossy().into_owned()]);
    if is_command {
        forwarded.extend([
            "--relaunch-result".into(),
            result_file.to_string_lossy().into_owned(),
        ]);
    }
    let working = program
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| scratch.clone());
    if let Err(error) = omnitrigger_windows::relaunch_via_shell(
        &program,
        &join_arguments(&forwarded),
        &working,
        &link,
    ) {
        fail(&error);
    }
    if !is_command {
        std::process::exit(0);
    }
    let deadline = std::time::Instant::now() + RESULT_WAIT;
    while std::time::Instant::now() < deadline {
        if let Some(relayed) = std::fs::read_to_string(&result_file)
            .ok()
            .and_then(|text| serde_json::from_str::<Relayed>(&text).ok())
        {
            let _ = std::fs::remove_file(&result_file);
            if relayed.ok {
                println!("{}", relayed.text);
                std::process::exit(0);
            }
            eprintln!("{}", relayed.text);
            std::process::exit(1);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    exit_rejected("已脱离打包重定向重启，但新进程没有在 60 秒内返回结果。");
}

/// 已忽略的外部副本：目录 → 忽略时的最后修改时间。
fn ignored_path(root: &Path) -> PathBuf {
    root.join("device").join("external-copies.json")
}

fn relative_age(modified_ms: u64) -> String {
    let minutes = now_ms().saturating_sub(modified_ms) / 60_000;
    match minutes {
        0 => "刚刚".into(),
        1..=59 => format!("{minutes} 分钟前"),
        60..=1439 => format!("{} 小时前", minutes / 60),
        _ => format!("{} 天前", minutes / 1440),
    }
}

/// 外部副本：打包应用重定向产生的 `<LOCALAPPDATA>\Packages\<包>\LocalCache\Local\OmniTrigger`。
fn external_copies(local_app_data: &Path) -> Vec<(String, PathBuf, u64)> {
    let packages = local_app_data.join("Packages");
    if !crate::local_files::is_plain_directory(&packages) {
        return Vec::new();
    }
    let Ok(packages) = std::fs::read_dir(packages) else {
        return Vec::new();
    };
    packages
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let directory = entry
                .path()
                .join("LocalCache")
                .join("Local")
                .join("OmniTrigger");
            let modified = data_modified_ms(&directory)?;
            directory
                .join("bindings.json")
                .symlink_metadata()
                .is_ok_and(|metadata| metadata.is_file() && crate::local_files::is_plain(&metadata))
                .then(|| {
                    (
                        entry.file_name().to_string_lossy().into_owned(),
                        directory,
                        modified,
                    )
                })
        })
        .collect()
}

/// 选出比当前数据新、且未在该修改时间被忽略的最新副本。
fn pending_copy(
    copies: Vec<(String, PathBuf, u64)>,
    current_ms: Option<u64>,
    ignored: &BTreeMap<String, u64>,
) -> Option<(String, PathBuf, u64)> {
    copies
        .into_iter()
        .filter(|(_, _, modified)| current_ms.is_none_or(|current| *modified > current))
        .filter(|(_, directory, modified)| {
            ignored.get(&normalize(directory).to_ascii_lowercase()) != Some(modified)
        })
        .max_by_key(|(_, _, modified)| *modified)
}

/// 发现比当前数据更新的外部副本时询问用户：导入（先备份当前数据）、忽略或本次不处理。
#[cfg(windows)]
fn offer_external_copy(root: &Path, local_app_data: &Path) -> Result<(), String> {
    let ignored_file = ignored_path(root);
    let mut ignored: BTreeMap<String, u64> = std::fs::read_to_string(&ignored_file)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    let current = data_modified_ms(root);
    let Some((package, directory, modified)) =
        pending_copy(external_copies(local_app_data), current, &ignored)
    else {
        return Ok(());
    };
    let current_text = current
        .map(|ms| format!("修改于 {}", relative_age(ms)))
        .unwrap_or_else(|| "还没有数据".into());
    let text = format!(
        "发现另一份更新的 OmniTrigger 数据：\n{}\n（来自 {package}，修改于 {}）\n\n当前数据目录：\n{}\n（{current_text}）\n\n这份副本通常是从其他应用的终端启动时被系统重定向保存的。\n\n是：导入这份数据，当前数据先备份到 backups 目录\n否：忽略这份副本，之后有新修改时再提示\n取消：本次不处理",
        directory.display(),
        relative_age(modified),
        root.display(),
    );
    match omnitrigger_windows::ask_yes_no_cancel("OmniTrigger 发现外部数据副本", &text) {
        Some(true) => {
            if let Err(error) = external_import::import(root, &directory) {
                // 收尾受阻时保留恢复记录，避免主窗口载入并保存尚未恢复的配置。
                external_import::recover(root)
                    .map_err(|recovery| format!("{error}\n{recovery}"))?;
                omnitrigger_windows::show_error_dialog("OmniTrigger 导入失败", &error);
            }
        }
        Some(false) => {
            ignored.insert(normalize(&directory).to_ascii_lowercase(), modified);
            let _ = std::fs::create_dir_all(root.join("device"));
            let _ = write_json(&ignored_file, &ignored);
        }
        None => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        std::iter::once("omnitrigger.exe")
            .chain(list.iter().copied())
            .map(String::from)
            .collect()
    }

    fn temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("omnitrigger-profiles-{tag}-{}", new_id()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn profile_arguments_are_taken_and_others_kept_in_order() {
        let mut list = args(&["--settings", "--profile", "ui", "--listen", "--panel"]);
        let parsed = take_arguments(&mut list).unwrap();
        assert_eq!(parsed.profile.as_deref(), Some("ui"));
        assert!(parsed.listen);
        assert_eq!(list, args(&["--settings", "--panel"]));
    }

    #[test]
    fn type_and_seed_belong_to_create_only() {
        let mut list = args(&["--create-script-action", "--type", "node"]);
        let parsed = take_arguments(&mut list).unwrap();
        assert!(!parsed.any());
        assert_eq!(list, args(&["--create-script-action", "--type", "node"]));

        let mut list = args(&[
            "--profile-create",
            "ui",
            "--type",
            "test",
            "--seed",
            "copy:default",
        ]);
        let parsed = take_arguments(&mut list).unwrap();
        assert_eq!(
            parsed.command,
            Some(Command::Create {
                alias: "ui".into(),
                seed: Seed::Copy("default".into()),
                name: None
            })
        );
        assert_eq!(list, args(&[]));
    }

    #[test]
    fn conflicting_profile_arguments_are_rejected() {
        for list in [
            args(&["--profile-list", "--profile-reset", "ui"]),
            args(&["--profile-list", "--profile", "ui"]),
            args(&["--listen"]),
            args(&["--profile", "a", "--profile", "b"]),
            args(&["--profile"]),
            args(&["--profile", "--listen"]),
            args(&["--profile-create", "ui", "--type", "local"]),
            args(&["--profile-create", "UI"]),
            args(&["--profile-create", "default"]),
            args(&["--profile-create", "ui", "--seed", "examples"]),
        ] {
            let mut list = list;
            assert!(take_arguments(&mut list).is_err(), "{list:?}");
        }
    }

    #[test]
    fn root_prefers_flag_then_env_then_local_app_data() {
        let base = PathBuf::from(r"C:\Users\u\AppData\Local");
        let (root, source) =
            resolve_root(Some(r"D:\data"), Some("E:\\env".into()), Some(base.clone())).unwrap();
        assert_eq!(
            (root, source),
            (PathBuf::from(r"D:\data"), RootSource::Flag)
        );
        let (root, source) =
            resolve_root(None, Some("E:\\env".into()), Some(base.clone())).unwrap();
        assert_eq!((root, source), (PathBuf::from(r"E:\env"), RootSource::Env));
        let (root, source) = resolve_root(None, Some("".into()), Some(base.clone())).unwrap();
        assert_eq!(
            (root, source),
            (base.join("OmniTrigger"), RootSource::Default)
        );
        assert!(resolve_root(None, None, None).is_err());
    }

    #[test]
    fn redirect_is_reported_only_for_package_copies() {
        let local = Path::new(r"C:\Users\u\AppData\Local");
        let root = local.join("OmniTrigger");
        assert_eq!(
            redirected_package(
                &root,
                Path::new(r"\\?\C:\Users\u\AppData\Local\OmniTrigger"),
                local
            ),
            None
        );
        assert_eq!(
            redirected_package(
                &root,
                Path::new(r"c:\users\u\appdata\local\omnitrigger\"),
                local
            ),
            None
        );
        assert_eq!(
            redirected_package(
                &root,
                Path::new(r"\\?\C:\Users\u\AppData\Local\Packages\Vendor.App_abc\LocalCache\Local\OmniTrigger"),
                local
            )
            .as_deref(),
            Some("Vendor.App_abc")
        );
        assert_eq!(
            redirected_package(&root, Path::new(r"D:\linked\OmniTrigger"), local),
            None
        );
        let chinese = PathBuf::from(format!("D:\\{}", "配置".repeat(10)));
        assert_eq!(redirected_package(&root, &chinese, local), None);
    }

    #[test]
    fn arguments_are_quoted_for_windows_command_lines() {
        assert_eq!(quote_argument("plain"), "plain");
        assert_eq!(quote_argument(""), "\"\"");
        assert_eq!(quote_argument(r"C:\a b\"), r#""C:\a b\\""#);
        assert_eq!(quote_argument(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(quote_argument(r#"a\"b"#), r#""a\\\"b""#);
        assert_eq!(
            join_arguments(&["--profile".into(), "ui".into(), r"C:\x y".into()]),
            r#"--profile ui "C:\x y""#
        );
    }

    #[test]
    fn forwarded_arguments_drop_relaunch_markers() {
        let original = args(&[
            "--profile",
            "ui",
            "--relaunched",
            "x.lnk",
            "--relaunch-result",
            "r.json",
        ]);
        let parsed = Arguments::default();
        assert_eq!(parsed.forwarded(&original), vec!["--profile", "ui"]);
    }

    #[test]
    fn test_profile_persists_resets_and_copies() {
        let root = temp_root("persist");
        std::fs::write(root.join("bindings.json"), "daily").unwrap();
        std::fs::write(root.join("bindings.json.old.bak"), "skip").unwrap();
        std::fs::create_dir_all(root.join("actions").join("pkg")).unwrap();
        std::fs::write(root.join("actions").join("pkg").join("a.mjs"), "x").unwrap();

        let entry =
            create_profile(&root, "ui", &Seed::Copy("default".into()), Some("浏览视图")).unwrap();
        let directory = profile_dir(&root, &entry.id);
        assert_eq!(
            std::fs::read_to_string(directory.join("bindings.json")).unwrap(),
            "daily"
        );
        assert!(!directory.join("bindings.json.old.bak").exists());
        assert!(directory
            .join("actions")
            .join("pkg")
            .join("a.mjs")
            .is_file());
        assert!(create_profile(&root, "ui", &Seed::Empty, None).is_err());

        // 同一别名再次查找得到同一目录，修改保留。
        std::fs::write(directory.join("bindings.json"), "edited").unwrap();
        let catalog = read_catalog(&root).unwrap();
        assert_eq!(catalog.find("ui").unwrap().id, entry.id);
        assert_eq!(catalog.find(&entry.id).unwrap().name, "浏览视图");
        assert_eq!(
            std::fs::read_to_string(directory.join("bindings.json")).unwrap(),
            "edited"
        );

        let (_, backup) = reset_profile(&root, "ui").unwrap();
        assert_eq!(
            std::fs::read_to_string(directory.join("bindings.json")).unwrap(),
            "daily"
        );
        assert_eq!(
            std::fs::read_to_string(backup.unwrap().join("bindings.json")).unwrap(),
            "edited"
        );
        assert!(reset_profile(&root, "missing").is_err());

        let copy = create_profile(&root, "ui-copy", &Seed::Copy("ui".into()), None).unwrap();
        assert_eq!(
            std::fs::read_to_string(profile_dir(&root, &copy.id).join("bindings.json")).unwrap(),
            "daily"
        );
        let empty = create_profile(&root, "blank", &Seed::Empty, None).unwrap();
        assert!(!profile_dir(&root, &empty.id).join("bindings.json").exists());

        let listed = list_profiles(&root, RootSource::Flag).unwrap();
        let aliases: Vec<_> = listed["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["alias"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(aliases, ["default", "ui", "ui-copy", "blank"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn custom_raishot_actions_follow_profile_copies_resets_and_imports() {
        let root = temp_root("raishot-manifest");
        let manifest = root.join(RAISHOT_MANIFEST_PATH);
        let plugin = manifest.parent().unwrap();
        std::fs::create_dir_all(plugin.join("runtime")).unwrap();
        std::fs::write(plugin.join("runtime/local-only"), "runtime").unwrap();
        let mut custom: serde_json::Value =
            serde_json::from_str(include_str!("../plugins/raishot.json")).unwrap();
        let mut second = custom["actions"][0].clone();
        second["id"] = "custom-capture".into();
        custom["actions"].as_array_mut().unwrap().push(second);
        let bytes = serde_json::to_vec_pretty(&custom).unwrap();
        std::fs::write(&manifest, &bytes).unwrap();

        let entry = create_profile(&root, "ui", &Seed::Copy(DEFAULT_ID.into()), None).unwrap();
        let copied = profile_dir(&root, &entry.id);
        let catalogue = || {
            crate::plugins::Plugins::load(&copied.join("bindings.json"))
                .unwrap()
                .catalogue()
                .unwrap()
        };
        assert!(catalogue()
            .iter()
            .any(|action| action.reference.action_id == "custom-capture"));
        assert!(!copied.join("plugins/raishot/runtime").exists());
        std::fs::write(copied.join(RAISHOT_MANIFEST_PATH), "edited").unwrap();
        let (_, backup) = reset_profile(&root, "ui").unwrap();
        assert_eq!(
            std::fs::read(backup.unwrap().join(RAISHOT_MANIFEST_PATH)).unwrap(),
            b"edited"
        );
        assert_eq!(
            std::fs::read(copied.join(RAISHOT_MANIFEST_PATH)).unwrap(),
            bytes
        );
        assert_eq!(std::fs::read(&manifest).unwrap(), bytes);

        let restore = root.join("before-import");
        copy_data(&copied, &restore, is_data_file).unwrap();
        std::fs::write(copied.join(RAISHOT_MANIFEST_PATH), "before restore").unwrap();
        copy_data(&restore, &copied, is_data_file).unwrap();
        assert_eq!(catalogue().len(), 2);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn copies_the_actions_directory_resolved_by_the_filesystem() {
        let root = temp_root("actions-spelling");
        let fixture = root.join("fixture");
        let package = Path::new("actions/example/action.json");
        let mixed = fixture.join("Actions/example/action.json");
        std::fs::create_dir_all(mixed.parent().unwrap()).unwrap();
        std::fs::write(&mixed, "source").unwrap();
        std::fs::write(fixture.join("PrOfIlEs.JsOn"), "foreign catalog").unwrap();
        let entry = create_profile(&root, "ui", &Seed::Fixture(fixture.clone()), None).unwrap();
        let directory = profile_dir(&root, &entry.id);
        assert_eq!(directory.join("PrOfIlEs.JsOn").exists(), !cfg!(windows));
        if !fixture.join(package).exists() {
            // 在区分大小写的目录中，Actions 并不是动作服务使用的目录。
            assert!(!directory.join("Actions").exists());
            assert!(!directory.join("actions").exists());
        } else {
            assert_eq!(std::fs::read(directory.join(package)).unwrap(), b"source");
            std::fs::write(directory.join(package), "edited").unwrap();
            let (_, previous) = reset_profile(&root, "ui").unwrap();
            assert_eq!(std::fs::read(directory.join(package)).unwrap(), b"source");
            assert_eq!(
                std::fs::read(previous.unwrap().join(package)).unwrap(),
                b"edited"
            );

            // 外部副本导入先备份现有动作；两侧原有目录可使用不同大小写。
            let existing = root.join("ACTIONS/example");
            std::fs::create_dir_all(&existing).unwrap();
            std::fs::write(existing.join("action.json"), "original").unwrap();
            let backup = root.join("backups/before-import");
            let catalog = std::fs::read(catalog_path(&root)).unwrap();
            copy_data(&root, &backup, is_data_file).unwrap();
            copy_data(&fixture, &root, is_data_file).unwrap();
            assert_eq!(std::fs::read(catalog_path(&root)).unwrap(), catalog);
            assert_eq!(std::fs::read(root.join(package)).unwrap(), b"source");
            assert_eq!(std::fs::read(backup.join(package)).unwrap(), b"original");
            copy_data(&backup, &root, is_data_file).unwrap();
            assert_eq!(std::fs::read(root.join(package)).unwrap(), b"original");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn profile_file_exclusions_follow_platform_case_rules() {
        for name in [
            "profiles.json",
            "profile.json",
            "bindings.lock",
            "wheels.old.bak",
            "save.tmp",
            ".root-probe-123",
            "connector.previous-registration-123.json",
        ] {
            assert!(!is_data_file(name));
            assert_eq!(is_data_file(&name.to_ascii_uppercase()), !cfg!(windows));
        }
        assert!(is_data_file("My Actions.JSON"));
    }

    #[test]
    fn test_seeds_skip_connector_registration_and_backups_preserve_it() {
        let root = temp_root("connector-registration");
        let fixture = root.join("fixture");
        std::fs::create_dir(&fixture).unwrap();
        let registration = "browser-connector.json";
        for source in [&root, &fixture] {
            std::fs::write(source.join(registration), "source installation").unwrap();
            std::fs::write(source.join("bindings.json"), "binding data").unwrap();
        }
        for (alias, seed) in [
            ("copy", Seed::Copy(DEFAULT_ID.into())),
            ("fixture", Seed::Fixture(fixture.clone())),
        ] {
            let entry = create_profile(&root, alias, &seed, None).unwrap();
            let directory = profile_dir(&root, &entry.id);
            assert!(!directory.join(registration).exists());
            assert_eq!(
                std::fs::read(directory.join("bindings.json")).unwrap(),
                b"binding data"
            );
            std::fs::write(directory.join(registration), "test installation").unwrap();
            let (_, backup) = reset_profile(&root, alias).unwrap();
            assert!(!directory.join(registration).exists());
            assert_eq!(
                std::fs::read(backup.unwrap().join(registration)).unwrap(),
                b"test installation"
            );
        }
        let backup = root.join("full-backup");
        copy_data(&root, &backup, is_data_file).unwrap();
        assert_eq!(
            std::fs::read(backup.join(registration)).unwrap(),
            b"source installation"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn profile_copies_start_with_empty_history_and_reset_backs_up_the_old_history() {
        let root = temp_root("copy-history");
        let filename = crate::clipboard_history_store::FILE_NAME;
        let history = |dir: &Path| dir.join(filename);
        let daily = history(&root);
        #[cfg(windows)]
        let daily = daily.with_file_name(filename.to_ascii_uppercase());
        std::fs::write(daily, b"daily history").unwrap();

        let copy = create_profile(&root, "copy", &Seed::Copy(DEFAULT_ID.into()), None).unwrap();
        let directory = profile_dir(&root, &copy.id);
        assert!(!history(&directory).exists());
        std::fs::write(history(&directory), b"test history").unwrap();
        let nested = create_profile(&root, "nested", &Seed::Copy(copy.id), None).unwrap();
        assert!(!history(&profile_dir(&root, &nested.id)).exists());

        let (_, backup) = reset_profile(&root, "copy").unwrap();
        let backup = backup.unwrap();
        assert!(!history(&directory).exists());
        assert_eq!(std::fs::read(history(&backup)).unwrap(), b"test history");
        assert_eq!(std::fs::read(history(&root)).unwrap(), b"daily history");

        let fixture = create_profile(&root, "fixture", &Seed::Fixture(backup), None).unwrap();
        assert_eq!(
            std::fs::read(history(&profile_dir(&root, &fixture.id))).unwrap(),
            b"test history"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn seed_sources_follow_registered_copies_without_probing_the_sources() {
        let root = temp_root("seed-source");
        let canonical = root.canonicalize().unwrap();
        let fixture = root.join("fixture");
        std::fs::create_dir(&fixture).unwrap();
        let base = create_profile(&root, "base", &Seed::Copy(DEFAULT_ID.into()), None).unwrap();
        let from_base = create_profile(&root, "second", &Seed::Copy("base".into()), None).unwrap();
        let from_fixture =
            create_profile(&root, "third", &Seed::Fixture(fixture.clone()), None).unwrap();
        let fixture_copy = create_profile(
            &root,
            "fixture-copy",
            &Seed::Copy(from_fixture.id.clone()),
            None,
        )
        .unwrap();
        let empty = create_profile(&root, "empty", &Seed::Empty, None).unwrap();
        let empty_copy =
            create_profile(&root, "empty-copy", &Seed::Copy(empty.id.clone()), None).unwrap();
        let config = |id: &str| profile_dir(&root, id).join("bindings.json");
        let registered = |at: &Path| {
            let relative = at.strip_prefix(&canonical).unwrap();
            let mut paths = vec![at.to_path_buf()];
            if !normalize(&root).eq_ignore_ascii_case(&normalize(&canonical)) {
                paths.push(root.join(relative));
            }
            paths
        };
        assert_eq!(
            seed_sources_for_config(&config(&base.id)),
            registered(&canonical)
        );
        assert!(seed_sources_for_config(&config(&empty.id)).is_empty());
        assert_eq!(
            seed_sources_for_config(&config(&empty_copy.id)),
            registered(&profile_dir(&canonical, &empty.id))
        );
        // 源目录已移动也可识别旧状态中的源路径。
        std::fs::rename(profile_dir(&root, &base.id), root.join("moved-base")).unwrap();
        std::fs::rename(&fixture, root.join("moved-fixture")).unwrap();
        assert_eq!(
            seed_sources_for_config(&config(&from_base.id)),
            [
                registered(&profile_dir(&canonical, &base.id)),
                registered(&canonical)
            ]
            .concat()
        );
        assert_eq!(
            seed_sources_for_config(&config(&fixture_copy.id)),
            [
                registered(&profile_dir(&canonical, &from_fixture.id)),
                vec![fixture]
            ]
            .concat()
        );
        assert!(seed_sources_for_config(&root.join("bindings.json")).is_empty());
        let unregistered = profile_dir(&root, &from_base.alias);
        std::fs::create_dir(&unregistered).unwrap();
        std::fs::copy(
            profile_dir(&root, &from_base.id).join("profile.json"),
            unregistered.join("profile.json"),
        )
        .unwrap();
        assert!(seed_sources_for_config(&config(&from_base.alias)).is_empty());
        std::fs::write(catalog_path(&root), "{").unwrap();
        assert!(seed_sources_for_config(&config(&from_base.id)).is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn broken_or_cyclic_seed_chain_cannot_identify_an_installation() {
        let root = temp_root("cyclic-seed-source");
        let first = create_profile(&root, "first", &Seed::Empty, None).unwrap();
        let second = create_profile(&root, "second", &Seed::Copy(first.id.clone()), None).unwrap();
        let config = profile_dir(&root, &second.id).join("bindings.json");
        let mut catalog = read_catalog(&root).unwrap();
        let alternate = root.with_file_name("alternate-root-name");
        assert_eq!(
            trace_seed_sources(&root, Some(&alternate), &catalog, &second, SeedPaths::All),
            [
                profile_dir(&root, &first.id),
                profile_dir(&alternate, &first.id)
            ]
        );
        catalog.profiles[0].seed = Seed::Fixture(profile_dir(&alternate, &first.id)).record();
        assert!(
            trace_seed_sources(&root, Some(&alternate), &catalog, &second, SeedPaths::All)
                .is_empty()
        );
        for seed in [
            Seed::Copy(first.id.clone()),
            Seed::Copy(second.alias.clone()),
            Seed::Copy("no-longer-registered".into()),
            Seed::Fixture(profile_dir(&root, &first.id)),
        ] {
            catalog.profiles[0].seed = seed.record();
            write_json(&catalog_path(&root), &catalog).unwrap();
            assert!(seed_sources_for_config(&config).is_empty(), "{seed:?}");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_fixture_leaves_no_profile_behind() {
        let root = temp_root("fixture");
        let missing = root.join("no-such-fixture");
        assert!(create_profile(&root, "ui", &Seed::Fixture(missing), None).is_err());
        assert!(read_catalog(&root).unwrap().profiles.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reset_requires_all_windows_using_the_target_profile_to_close() {
        let root = temp_root("live-reset");
        let edited = create_profile(&root, "editing", &Seed::Empty, None).unwrap();
        let other = create_profile(&root, "other", &Seed::Empty, None).unwrap();
        let config = profile_dir(&root, &edited.id).join("bindings.json");
        std::fs::write(&config, b"saved").unwrap();
        let window = Startup::for_entry(&edited, false)
            .hold_window_profile(&config)
            .unwrap()
            .unwrap();
        let explicit = Startup::for_config(&config)
            .hold_window_profile(&config)
            .unwrap()
            .unwrap();
        assert!(Startup::daily()
            .hold_window_profile(&root.join("bindings.json"))
            .unwrap()
            .is_none());
        assert!(matches!(
            reset_profile(&root, &edited.id),
            Err(ProfileCommandError::InUse(_))
        ));
        assert!(!root.join("backups/profile-reset").exists());
        assert!(reset_profile(&root, &other.alias).is_ok());
        drop(window);
        assert!(matches!(
            reset_profile(&root, &edited.alias),
            Err(ProfileCommandError::InUse(_))
        ));
        assert_eq!(std::fs::read(&config).unwrap(), b"saved");
        drop(explicit);
        let (_, backup) = reset_profile(&root, &edited.alias).unwrap();
        assert_eq!(
            std::fs::read(backup.unwrap().join("bindings.json")).unwrap(),
            b"saved"
        );
        assert!(!config.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_reset_preserves_live_files_and_can_be_retried() {
        let root = temp_root("reset-retry");
        let fixture = root.join("fixture");
        std::fs::create_dir(&fixture).unwrap();
        std::fs::write(fixture.join("bindings.json"), "seed").unwrap();
        let entry = create_profile(&root, "ui", &Seed::Fixture(fixture.clone()), None).unwrap();
        let directory = profile_dir(&root, &entry.id);
        std::fs::write(directory.join("bindings.json"), "edited").unwrap();
        std::fs::write(directory.join("wheels.json"), "wheel").unwrap();
        let moved = root.join("moved");
        std::fs::rename(&fixture, &moved).unwrap();
        assert!(reset_profile(&root, "ui").is_err());
        assert_eq!(
            std::fs::read(directory.join("bindings.json")).unwrap(),
            b"edited"
        );
        assert_eq!(
            std::fs::read(directory.join("wheels.json")).unwrap(),
            b"wheel"
        );
        assert!(directory.join("profile.json").is_file());
        assert_eq!(
            std::fs::read_dir(root.join("backups/profile-reset"))
                .unwrap()
                .count(),
            0
        );
        std::fs::rename(moved, fixture).unwrap();
        let (_, backup) = reset_profile(&root, "ui").unwrap();
        assert_eq!(
            std::fs::read(directory.join("bindings.json")).unwrap(),
            b"seed"
        );
        assert_eq!(
            std::fs::read(backup.unwrap().join("wheels.json")).unwrap(),
            b"wheel"
        );
        // 已登记的目录意外缺失时也能从种子恢复，回执只列真实备份。
        std::fs::rename(&directory, root.join("displaced")).unwrap();
        assert!(reset_profile(&root, "ui").unwrap().1.is_none());
        assert_eq!(
            std::fs::read(directory.join("bindings.json")).unwrap(),
            b"seed"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn external_copy_with_a_newer_managed_action_is_offered_again() {
        use std::time::{Duration, UNIX_EPOCH};
        let root = temp_root("action-copy-time");
        let source = root.join("Packages/sample/LocalCache/Local/OmniTrigger");
        let daily = root.join("daily");
        let package = source.join("actions/package/scripts");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::create_dir(&daily).unwrap();
        let write_at = |path: PathBuf, seconds| {
            let file = std::fs::File::create(path).unwrap();
            file.set_modified(UNIX_EPOCH + Duration::from_secs(seconds))
                .unwrap();
        };
        write_at(daily.join("bindings.json"), 1_700_000_020);
        write_at(source.join("bindings.json"), 1_700_000_010);
        write_at(source.join("browser-connector.json"), 1_700_000_090);
        write_at(daily.join("browser-connector.json"), 1_700_000_100);
        write_at(package.join("action.mjs"), 1_700_000_030);
        for unrelated in [source.join("backups/old"), package.join("OneDrive")] {
            std::fs::create_dir_all(&unrelated).unwrap();
            write_at(unrelated.join("ignored.json"), 1_700_000_050);
        }
        let ignored =
            BTreeMap::from([(normalize(&source).to_ascii_lowercase(), 1_700_000_010_000)]);
        let offered = pending_copy(external_copies(&root), data_modified_ms(&daily), &ignored);
        let seen = BTreeMap::from([(normalize(&source).to_ascii_lowercase(), 1_700_000_030_000)]);
        assert!(pending_copy(external_copies(&root), data_modified_ms(&daily), &seen).is_none());
        let local_package = daily.join("actions/local");
        std::fs::create_dir_all(&local_package).unwrap();
        write_at(local_package.join("action.json"), 1_700_000_040);
        assert!(pending_copy(external_copies(&root), data_modified_ms(&daily), &ignored).is_none());
        let manifest = source.join(RAISHOT_MANIFEST_PATH);
        let plugin = manifest.parent().unwrap();
        std::fs::create_dir_all(plugin.join("runtime")).unwrap();
        write_at(plugin.join("runtime/manifest.json"), 1_700_000_100);
        assert!(pending_copy(external_copies(&root), data_modified_ms(&daily), &ignored).is_none());
        write_at(manifest, 1_700_000_050);
        assert_eq!(
            pending_copy(external_copies(&root), data_modified_ms(&daily), &ignored)
                .map(|entry| entry.2),
            Some(1_700_000_050_000)
        );
        std::fs::remove_dir_all(root).unwrap();
        assert_eq!(offered.map(|entry| entry.2), Some(1_700_000_030_000));
    }

    #[test]
    fn only_newer_unignored_copies_are_offered() {
        let a = (
            String::from("A"),
            PathBuf::from(r"C:\L\Packages\A\LocalCache\Local\OmniTrigger"),
            200,
        );
        let b = (
            String::from("B"),
            PathBuf::from(r"C:\L\Packages\B\LocalCache\Local\OmniTrigger"),
            300,
        );
        let none = BTreeMap::new();
        assert_eq!(
            pending_copy(vec![a.clone(), b.clone()], Some(100), &none)
                .unwrap()
                .0,
            "B"
        );
        assert_eq!(
            pending_copy(vec![a.clone(), b.clone()], Some(250), &none)
                .unwrap()
                .0,
            "B"
        );
        assert!(pending_copy(vec![a.clone(), b.clone()], Some(300), &none).is_none());
        assert_eq!(pending_copy(vec![a.clone()], None, &none).unwrap().0, "A");
        let mut ignored = BTreeMap::new();
        ignored.insert(normalize(&b.1).to_ascii_lowercase(), 300);
        assert_eq!(
            pending_copy(vec![a.clone(), b.clone()], Some(100), &ignored)
                .unwrap()
                .0,
            "A"
        );
        ignored.insert(normalize(&b.1).to_ascii_lowercase(), 250);
        assert_eq!(
            pending_copy(vec![a, b], Some(100), &ignored).unwrap().0,
            "B"
        );
    }

    #[test]
    fn shortcut_recovers_all_original_profiles_only_when_the_catalog_is_missing() {
        let root = temp_root("recover-catalog");
        let other = create_profile(
            &root,
            "other",
            &Seed::Copy(DEFAULT_ID.into()),
            Some("轮盘测试"),
        )
        .unwrap();
        let testing = testing_workspace(&root).unwrap();
        let config = profile_dir(&root, &testing.id).join("bindings.json");
        std::fs::write(&config, b"test bindings").unwrap();
        let path = catalog_path(&root);
        std::fs::remove_file(&path).unwrap();
        let archive = root.join("profiles/old-backup");
        std::fs::create_dir(&archive).unwrap();
        std::fs::copy(
            profile_dir(&root, &testing.id).join("profile.json"),
            archive.join("profile.json"),
        )
        .unwrap();
        let recovered = read_catalog(&root).unwrap();
        assert_eq!(recovered.profiles.len(), 2);
        assert_eq!(recovered.find("other"), Some(&other));
        assert_eq!(recovered.find(TEST_ALIAS), Some(&testing));
        assert!(!path.exists());
        assert_eq!(testing_workspace(&root).unwrap(), testing);
        assert_eq!(read_catalog(&root).unwrap(), recovered);
        assert!(path.is_file());
        assert_eq!(std::fs::read(&config).unwrap(), b"test bindings");

        write_json(
            &path,
            &Catalog {
                version: CATALOG_VERSION,
                profiles: vec![testing.clone()],
            },
        )
        .unwrap();
        assert!(read_catalog(&root).unwrap().find("other").is_none());
        std::fs::write(&path, "{").unwrap();
        assert!(testing_workspace(&root).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{");
        std::fs::remove_file(&path).unwrap();
        let mut duplicate = other.clone();
        duplicate.alias = TEST_ALIAS.into();
        write_json(
            &profile_dir(&root, &other.id).join("profile.json"),
            &duplicate,
        )
        .unwrap();
        assert!(testing_workspace(&root).unwrap_err().contains("重复"));
        assert!(!path.exists());
        assert_eq!(std::fs::read(&config).unwrap(), b"test bindings");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn settings_test_copy_reuses_edits_and_preserves_existing_workspaces() {
        let root = temp_root("settings-copy");
        let testing = testing_workspace(&root).unwrap();
        let occupied = create_profile(&root, "daily-copy", &Seed::Empty, None).unwrap();
        let file = |entry: &Entry| profile_dir(&root, &entry.id).join("bindings.json");
        std::fs::write(file(&testing), "testing").unwrap();
        std::fs::write(file(&occupied), "existing").unwrap();
        std::fs::write(root.join("bindings.json"), "daily").unwrap();
        std::fs::write(root.join("clipboard-history.json"), "personal history").unwrap();
        let copy = std::thread::scope(|scope| {
            let first = scope.spawn(|| daily_test_copy(&root).unwrap());
            let second = daily_test_copy(&root).unwrap();
            assert_eq!(first.join().unwrap(), second);
            second
        });
        assert_eq!(copy.alias, "daily-copy-2");
        assert_eq!(std::fs::read(file(&copy)).unwrap(), b"daily");
        assert!(!profile_dir(&root, &copy.id)
            .join("clipboard-history.json")
            .exists());
        std::fs::write(file(&copy), "edited").unwrap();
        std::fs::remove_file(catalog_path(&root)).unwrap();
        assert_eq!(daily_test_copy(&root).unwrap(), copy);
        assert!(catalog_path(&root).is_file());
        assert_eq!(std::fs::read(file(&copy)).unwrap(), b"edited");
        assert_eq!(std::fs::read(file(&occupied)).unwrap(), b"existing");
        assert_eq!(std::fs::read(file(&testing)).unwrap(), b"testing");
        assert_eq!(std::fs::read(root.join("bindings.json")).unwrap(), b"daily");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn shortcut_reuses_one_isolated_test_profile_even_on_concurrent_first_launch() {
        let root = temp_root("testing-shortcut");
        std::fs::write(root.join("bindings.json"), "daily").unwrap();
        let ids = std::thread::scope(|scope| {
            let first = scope.spawn(|| testing_workspace(&root).unwrap().id);
            let second = testing_workspace(&root).unwrap().id;
            (first.join().unwrap(), second)
        });
        assert_eq!(ids.0, ids.1);
        let config = profile_dir(&root, &ids.0).join("bindings.json");
        assert!(!config.exists());
        std::fs::write(&config, "test").unwrap();
        assert_eq!(testing_workspace(&root).unwrap().id, ids.0);
        assert_eq!(std::fs::read_to_string(config).unwrap(), "test");
        assert_eq!(
            std::fs::read_to_string(root.join("bindings.json")).unwrap(),
            "daily"
        );
        let parsed = take_arguments(&mut args(&["--test-workspace"])).unwrap();
        assert_eq!(parsed.profile.as_deref(), Some(TEST_ALIAS));
        assert!(!parsed.listen);
        assert!(take_arguments(&mut args(&["--test-workspace", "--profile", "default"])).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn explicit_config_recognizes_test_action_data_without_changing_launch_mode() {
        let root = temp_root("explicit-action-identity");
        let entry = create_profile(&root, "scene-test", &Seed::Empty, Some("动作测试")).unwrap();
        let directory = profile_dir(&root, &entry.id);
        let config = directory.join("alternate.json");
        assert!(!config.exists());
        let mut arguments = args(&["--config", config.to_str().unwrap(), "--settings"]);
        let original = arguments.clone();
        let explicit = startup(&mut arguments);
        assert_eq!(arguments, original);
        assert!(explicit.test_data && explicit.listen);
        assert_eq!(explicit.test_name, None);
        assert_eq!(explicit.window_title("OmniTrigger"), "OmniTrigger");
        assert!(!registered_test_data(&root.join("bindings.json")));
        assert!(!registered_test_data(
            &directory.join("nested/unknown.json")
        ));

        let alias_dir = profile_dir(&root, &entry.alias);
        std::fs::create_dir(&alias_dir).unwrap();
        std::fs::copy(
            directory.join("profile.json"),
            alias_dir.join("profile.json"),
        )
        .unwrap();
        assert!(!registered_test_data(&alias_dir.join("bindings.json")));
        for listen in [false, true] {
            let selected = Startup::for_entry(&entry, listen);
            assert!(selected.test_data);
            assert_eq!(selected.listen, listen);
            assert_eq!(selected.test_name.as_deref(), Some("动作测试"));
        }

        std::fs::write(catalog_path(&root), "{").unwrap();
        assert!(startup(&mut arguments).test_data);
        assert!(!registered_test_data(&alias_dir.join("bindings.json")));
        std::fs::remove_file(catalog_path(&root)).unwrap();
        assert!(registered_test_data(&config));
        std::fs::write(directory.join("profile.json"), "broken").unwrap();
        assert_eq!(startup(&mut arguments), Startup::daily());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn flag_and_env_roots_share_the_explicit_config_identity() {
        let root = temp_root("root-action-identity");
        let test = create_profile(&root, "testing-root", &Seed::Empty, None).unwrap();
        let directory = profile_dir(&root, &test.id);
        for (flag, env) in [
            (directory.to_str(), None),
            (None, Some(directory.as_os_str().to_owned())),
        ] {
            let (resolved, source) = resolve_root(flag, env, None).unwrap();
            assert_ne!(source, RootSource::Default);
            let startup = Startup::for_config(&resolved.join("bindings.json"));
            assert!(startup.test_data && startup.listen);
            assert!(startup.test_name.is_none());
        }
        assert_eq!(
            Startup::for_config(&root.join("bindings.json")),
            Startup::daily()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn startup_title_marks_test_data() {
        let test = Startup {
            test_name: Some("浏览视图".into()),
            test_data: true,
            listen: false,
            ..Startup::daily()
        };
        assert_eq!(
            test.window_title("OmniTrigger"),
            "OmniTrigger — 测试：浏览视图"
        );
        assert_eq!(Startup::daily().window_title("OmniTrigger"), "OmniTrigger");
    }
}
