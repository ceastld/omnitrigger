//! 数据根解析、打包重定向处理与持久测试配置档，设计见 `docs/PROFILES.md`。
//!
//! 首阶段的默认配置档就是数据根本身（文件保持原位）；测试配置档位于
//! `<数据根>\profiles\<ID>\`，由 `<数据根>\profiles.json` 登记。选中配置档等价于把
//! 该目录下的 `bindings.json` 作为配置路径，其余数据文件随之切换。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const CATALOG_VERSION: u32 = 1;
const DEFAULT_ID: &str = "default";
const TEST_ALIAS: &str = "testing";
const RESULT_WAIT: std::time::Duration = std::time::Duration::from_secs(60);

/// 启动解析结果，交给界面决定标题、标识与是否安装 Hook。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Startup {
    /// 测试配置档的显示名；日常数据为 `None`。
    pub test_name: Option<String>,
    /// 是否安装输入 Hook。测试配置档默认不安装，`--listen` 显式开启。
    pub listen: bool,
}

impl Startup {
    fn daily() -> Self {
        Self {
            test_name: None,
            listen: true,
        }
    }

    pub fn window_title(&self, base: &str) -> String {
        match &self.test_name {
            Some(name) => format!("{base} — 测试：{name}"),
            None => base.into(),
        }
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
    if actual.len() <= prefix.len() || !actual[..prefix.len()].eq_ignore_ascii_case(&prefix) {
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
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Catalog {
            version: CATALOG_VERSION,
            profiles: Vec::new(),
        }),
        Err(error) => Err(format!("读取 {} 失败：{error}", path.display())),
    }
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
}

/// 参与复制、导入与备份的数据文件：顶层普通文件，排除锁、旧备份与配置档自身的登记文件。
fn is_data_file(name: &str) -> bool {
    !(name.ends_with(".lock")
        || name.ends_with(".bak")
        || name.ends_with(".tmp")
        || name.starts_with(".root-probe-")
        || name.contains(".previous-registration-")
        || name == "profiles.json"
        || name == "profile.json")
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// 复制一份配置档数据（顶层数据文件与托管动作包 `actions\`），返回复制的顶层文件数。
fn copy_data(from: &Path, to: &Path) -> Result<usize, String> {
    let fail = |error: std::io::Error| {
        format!(
            "从 {} 复制数据到 {} 失败：{error}",
            from.display(),
            to.display()
        )
    };
    std::fs::create_dir_all(to).map_err(fail)?;
    let mut copied = 0;
    for entry in std::fs::read_dir(from).map_err(fail)? {
        let entry = entry.map_err(fail)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let kind = entry.file_type().map_err(fail)?;
        if kind.is_file() && is_data_file(&name) {
            std::fs::copy(entry.path(), to.join(&name)).map_err(fail)?;
            copied += 1;
        } else if kind.is_dir() && name == "actions" {
            copy_tree(&entry.path(), &to.join(&name)).map_err(fail)?;
        }
    }
    Ok(copied)
}

/// 目录中数据文件的最后修改时间（毫秒）；没有数据文件时为 `None`。
fn data_modified_ms(directory: &Path) -> Option<u64> {
    std::fs::read_dir(directory)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter(|entry| is_data_file(&entry.file_name().to_string_lossy()))
        .filter_map(|entry| entry.metadata().ok()?.modified().ok())
        .filter_map(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|time| time.as_millis() as u64)
        .max()
}

fn source_dir(root: &Path, catalog: &Catalog, key: &str) -> Result<PathBuf, String> {
    if key == DEFAULT_ID {
        return Ok(root.to_path_buf());
    }
    catalog
        .find(key)
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
            copy_data(path, target).map(drop)
        }
        Seed::Copy(key) => copy_data(&source_dir(root, catalog, key)?, target).map(drop),
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
fn reset_profile(root: &Path, key: &str) -> Result<(Entry, PathBuf), String> {
    let catalog = read_catalog(root)?;
    let entry = catalog
        .find(key)
        .cloned()
        .ok_or_else(|| format!("找不到配置档 {key}；可用 --profile-list 查看。"))?;
    if entry.kind != "test" {
        return Err(format!(
            "只有测试配置档可以重置，{key} 的类型是 {}。",
            entry.kind
        ));
    }
    let seed = Seed::parse(&entry.seed)?;
    let directory = profile_dir(root, &entry.id);
    let backup_root = root.join("backups").join("profile-reset");
    let backup = backup_root.join(format!("{}-{}", entry.id, now_ms()));
    std::fs::create_dir_all(&backup_root)
        .map_err(|error| format!("创建 {} 失败：{error}", backup_root.display()))?;
    if directory.exists() {
        std::fs::rename(&directory, &backup)
            .map_err(|error| format!("备份 {} 失败：{error}", directory.display()))?;
    }
    seed_into(root, &catalog, &seed, &directory)?;
    write_json(&directory.join("profile.json"), &entry)?;
    Ok((entry, backup))
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

fn run_command(root: &Path, source: RootSource, command: &Command) -> Result<String, String> {
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
    serde_json::json!({"status":"rejected","error":{
        "code":"invalid_profile_arguments","message":message,"effect_state":"not_started"
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
    let text = result.unwrap_or_else(|error| rejected(&error));
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
    let has_config = arguments
        .iter()
        .skip(1)
        .any(|argument| argument == "--config");
    if has_config {
        if parsed.any() {
            exit_rejected(
                "--config 是临时配置路径，不能与 --data-root、--profile 或配置档命令同用。",
            );
        }
        return Startup::daily();
    }
    let plain = arguments.len() == 1;
    if parsed.command.is_some() && !plain {
        exit_rejected("配置档命令不能与其他参数混用。");
    }
    let interactive = plain && parsed.command.is_none();
    if !parsed.any() && !interactive {
        // 其他命令行入口保持原有路径解析，不做探针。
        return Startup::daily();
    }
    let local_app_data = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let (root, source) = resolve_root(
        parsed.data_root.as_deref(),
        std::env::var_os("OMNITRIGGER_HOME"),
        local_app_data.clone(),
    )
    .unwrap_or_else(|message| exit_rejected(&message));
    if plain {
        #[cfg(windows)]
        escape_redirect(&root, local_app_data.as_deref(), &parsed, &original);
    }
    if let Some(link) = &parsed.relaunched {
        let _ = std::fs::remove_file(link);
    }
    if let Some(command) = &parsed.command {
        finish_command(
            std::fs::create_dir_all(&root)
                .map_err(|error| format!("创建数据根 {} 失败：{error}", root.display()))
                .and_then(|()| run_command(&root, source, command)),
            parsed.result_file.as_deref(),
        );
    }
    let profile = parsed.profile.as_deref().filter(|key| *key != DEFAULT_ID);
    #[cfg(windows)]
    if interactive && profile.is_none() && source == RootSource::Default {
        if let Some(local_app_data) = &local_app_data {
            offer_external_copy(&root, local_app_data);
        }
    }
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
                exit_rejected(&message)
            });
            let test = entry.kind == "test";
            (
                Some(profile_dir(&root, &entry.id).join("bindings.json")),
                Startup {
                    test_name: test.then(|| entry.name.clone()),
                    listen: !test || parsed.listen,
                },
            )
        }
        None => (
            (source != RootSource::Default).then(|| root.join("bindings.json")),
            Startup::daily(),
        ),
    };
    if let Some(config) = config {
        if let Err(message) = crate::settings::use_path(config) {
            exit_rejected(&message);
        }
    }
    startup
}

#[cfg(windows)]
fn fail_redirect(is_command: bool, message: &str) -> ! {
    if is_command {
        exit_rejected(message);
    }
    omnitrigger_windows::show_error_dialog("OmniTrigger 无法使用真实数据目录", message);
    std::process::exit(1);
}

/// 数据根被打包应用重定向时，经系统外壳重启自身；配置档命令等待新进程转交结果。
#[cfg(windows)]
fn escape_redirect(
    root: &Path,
    local_app_data: Option<&Path>,
    parsed: &Arguments,
    original: &[String],
) {
    let Some(local_app_data) = local_app_data else {
        return;
    };
    let is_command = parsed.command.is_some();
    if let Err(error) = std::fs::create_dir_all(root) {
        fail_redirect(
            is_command,
            &format!("创建数据根 {} 失败：{error}", root.display()),
        );
    }
    let actual = match omnitrigger_windows::probe_final_directory(root) {
        Ok(actual) => actual,
        Err(error) => fail_redirect(is_command, &error),
    };
    let Some(package) = redirected_package(root, &actual, local_app_data) else {
        return;
    };
    if parsed.relaunched.is_some() {
        fail_redirect(is_command, &format!(
            "当前进程仍处在打包应用 {package} 的文件重定向中，读写的是：\n{}\n\n真实数据目录：\n{}\n\n请从开始菜单或资源管理器启动 OmniTrigger，或用 --data-root 显式指定数据目录。",
            actual.display(),
            root.display()
        ));
    }
    let program = std::env::current_exe()
        .and_then(std::path::absolute)
        .map_err(|error| error.to_string())
        .and_then(|path| omnitrigger_windows::final_file_path(&path))
        .unwrap_or_else(|error| fail_redirect(is_command, &format!("无法确定程序路径：{error}")));
    let scratch = std::env::temp_dir().join("OmniTrigger");
    let _ = std::fs::create_dir_all(&scratch);
    let tag = format!("{}-{}", std::process::id(), now_ms());
    let link = scratch.join(format!("relaunch-{tag}.lnk"));
    let result_file = scratch.join(format!("relaunch-{tag}.json"));
    let mut forwarded = parsed.forwarded(original);
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
        fail_redirect(is_command, &error);
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
    let Ok(packages) = std::fs::read_dir(local_app_data.join("Packages")) else {
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
            directory.join("bindings.json").is_file().then(|| {
                let modified = data_modified_ms(&directory)?;
                Some((
                    entry.file_name().to_string_lossy().into_owned(),
                    directory,
                    modified,
                ))
            })?
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
fn offer_external_copy(root: &Path, local_app_data: &Path) {
    let ignored_file = ignored_path(root);
    let mut ignored: BTreeMap<String, u64> = std::fs::read_to_string(&ignored_file)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    let current = data_modified_ms(root);
    let Some((package, directory, modified)) =
        pending_copy(external_copies(local_app_data), current, &ignored)
    else {
        return;
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
            let backup = root
                .join("backups")
                .join(format!("before-import-{}", now_ms()));
            let result = copy_data(root, &backup).and_then(|_| copy_data(&directory, root));
            if let Err(error) = result {
                omnitrigger_windows::show_error_dialog(
                    "OmniTrigger 导入失败",
                    &format!("{error}\n\n当前数据的备份位于：\n{}", backup.display()),
                );
            }
        }
        Some(false) => {
            ignored.insert(normalize(&directory).to_ascii_lowercase(), modified);
            let _ = std::fs::create_dir_all(root.join("device"));
            let _ = write_json(&ignored_file, &ignored);
        }
        None => {}
    }
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
            std::fs::read_to_string(backup.join("bindings.json")).unwrap(),
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
    fn missing_fixture_leaves_no_profile_behind() {
        let root = temp_root("fixture");
        let missing = root.join("no-such-fixture");
        assert!(create_profile(&root, "ui", &Seed::Fixture(missing), None).is_err());
        assert!(read_catalog(&root).unwrap().profiles.is_empty());
        let _ = std::fs::remove_dir_all(&root);
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
    fn startup_title_marks_test_data() {
        let test = Startup {
            test_name: Some("浏览视图".into()),
            listen: false,
        };
        assert_eq!(
            test.window_title("OmniTrigger"),
            "OmniTrigger — 测试：浏览视图"
        );
        assert_eq!(Startup::daily().window_title("OmniTrigger"), "OmniTrigger");
    }
}
