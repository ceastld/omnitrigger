//! 包内 ESM 的共同路径、读取和限额规则；不遍历工程目录。
use serde_json::{json, Value};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    rc::Rc,
};

const FILE_LIMIT: u64 = 128 * 1024;
const TOTAL_LIMIT: usize = 1024 * 1024;
const MODULE_LIMIT: usize = 64;

pub fn fault(code: &str, message: impl Into<String>, effect: &str) -> Value {
    json!({"code":code,"message":crate::bounded_diagnostic(message.into()),"effect_state":effect})
}

#[derive(Default)]
struct Sources {
    files: HashMap<PathBuf, String>,
    bytes: usize,
}

pub struct PackageModules {
    root: PathBuf,
    pub entry: PathBuf,
    sources: RefCell<Sources>,
    executing: Cell<bool>,
}

impl PackageModules {
    pub fn new(root: &str, entry: &str) -> Result<Rc<Self>, Value> {
        let root = Path::new(root);
        if !root.is_absolute() {
            return Err(fault(
                "module_path",
                "模块工程需要绝对本地目录。",
                "not_started",
            ));
        }
        check_local_path(root)?;
        if !root.is_dir() {
            return Err(fault("module_path", "模块工程目录不存在。", "not_started"));
        }
        let root = fs::canonicalize(root).map_err(|e| io_fault(root, e))?;
        let mut components = Vec::new();
        append_components(&mut components, entry)?;
        if !entry.ends_with(".mjs") || entry.starts_with('.') {
            return Err(fault(
                "module_entry",
                "模块入口须为包内 .mjs 相对路径（例如 main.mjs）。",
                "not_started",
            ));
        }
        let path = checked_file(&root, &components)?;
        let modules = Rc::new(Self {
            root,
            entry: path,
            sources: RefCell::new(Sources::default()),
            executing: Cell::new(false),
        });
        // 入口也计入模块总数和总字节数，并在执行任何代码前验证 UTF-8。
        modules.source(&modules.entry)?;
        Ok(modules)
    }

    pub fn resolve(&self, importer: &Path, specifier: &str) -> Result<PathBuf, Value> {
        if !specifier.starts_with("./") && !specifier.starts_with("../") {
            return Err(fault(
                "module_specifier",
                format!(
                    "{}：仅支持 ./ 或 ../ 开头的包内导入：{specifier}",
                    importer.display()
                ),
                "not_started",
            ));
        }
        let relative = importer
            .strip_prefix(&self.root)
            .map_err(|_| fault("module_path", "模块引用来源不在动作包内。", "not_started"))?;
        let mut components: Vec<String> = relative
            .parent()
            .unwrap_or(Path::new(""))
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect();
        append_components(&mut components, specifier).map_err(|mut error| {
            error["message"] = crate::bounded_diagnostic(format!(
                "{} → {specifier}：{}",
                importer.display(),
                error["message"].as_str().unwrap_or_default()
            ))
            .into();
            error
        })?;
        checked_file(&self.root, &components).map_err(|mut error| {
            error["message"] = crate::bounded_diagnostic(format!(
                "{} → {specifier}：{}",
                importer.display(),
                error["message"].as_str().unwrap_or_default()
            ))
            .into();
            error
        })
    }

    pub fn source(&self, path: &Path) -> Result<String, Value> {
        let mut sources = self.sources.borrow_mut();
        if let Some(source) = sources.files.get(path) {
            return Ok(source.clone());
        }
        if sources.files.len() >= MODULE_LIMIT {
            return Err(fault(
                "module_limit",
                "每次调用最多加载 64 个模块。",
                "not_started",
            ));
        }
        if !path.starts_with(&self.root) {
            return Err(fault("module_path", "模块文件超出动作包。", "not_started"));
        }
        // 解析与实际读取之间再检查一次，缓存命中则使用本次调用已读到的快照。
        check_local_path(path)?;
        let mut source = String::new();
        fs::File::open(path)
            .and_then(|file| file.take(FILE_LIMIT + 1).read_to_string(&mut source))
            .map_err(|e| io_fault(path, e))?;
        if source.len() as u64 > FILE_LIMIT || sources.bytes + source.len() > TOTAL_LIMIT {
            return Err(fault(
                "module_limit",
                format!(
                    "{}：单文件限 128 KiB，所有模块合计限 1 MiB。",
                    path.display()
                ),
                "not_started",
            ));
        }
        sources.bytes += source.len();
        sources.files.insert(path.to_path_buf(), source.clone());
        Ok(source)
    }

    pub fn begin_execution(&self) {
        self.executing.set(true);
    }

    pub fn execution_fault(&self, mut error: Value) -> Value {
        // 动态导入可以发生在初始化或 main 中，加载失败不代表之前没有副作用。
        if self.executing.get() {
            error["effect_state"] = "unknown".into();
        }
        error
    }
}

fn io_fault(path: &Path, error: std::io::Error) -> Value {
    fault(
        "module_unavailable",
        format!("{}：{error}", path.display()),
        "not_started",
    )
}

fn append_components(parts: &mut Vec<String>, specifier: &str) -> Result<(), Value> {
    if specifier.is_empty()
        || specifier.len() > 2048
        || specifier
            .chars()
            .any(|c| c.is_control() || matches!(c, '\\' | ':' | '?' | '#' | '%'))
        || specifier.starts_with('/')
    {
        return Err(fault(
            "module_specifier",
            "模块路径不能使用绝对路径、URL、转义、反斜杠或查询片段。",
            "not_started",
        ));
    }
    for part in specifier.split('/') {
        match part {
            "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(fault(
                        "module_path",
                        "模块导入超出动作包目录。",
                        "not_started",
                    ));
                }
            }
            "" => {
                return Err(fault(
                    "module_specifier",
                    "模块路径包含空片段。",
                    "not_started",
                ))
            }
            _ => {
                let stem = part
                    .split('.')
                    .next()
                    .unwrap_or_default()
                    .trim_end_matches(' ')
                    .to_ascii_uppercase();
                let serial = stem
                    .strip_prefix("COM")
                    .or_else(|| stem.strip_prefix("LPT"));
                if part.ends_with(['.', ' '])
                    || part
                        .chars()
                        .any(|c| matches!(c, '<' | '>' | '"' | '|' | '*'))
                    || matches!(
                        stem.as_str(),
                        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" | "CLOCK$"
                    )
                    || serial.is_some_and(|number| {
                        matches!(
                            number,
                            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                        )
                    })
                {
                    return Err(fault(
                        "module_specifier",
                        "模块路径包含 Windows 设备名或不明确的尾部字符。",
                        "not_started",
                    ));
                }
                parts.push(part.to_owned());
            }
        }
    }
    Ok(())
}

fn checked_file(root: &Path, parts: &[String]) -> Result<PathBuf, Value> {
    let path = parts
        .iter()
        .fold(root.to_path_buf(), |path, part| path.join(part));
    if !matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("js" | "mjs")
    ) {
        return Err(fault(
            "module_specifier",
            "模块需要显式 .js 或 .mjs 扩展名；不补全目录或扩展名。",
            "not_started",
        ));
    }
    check_local_path(&path)?;
    let canonical = fs::canonicalize(&path).map_err(|e| io_fault(&path, e))?;
    if !canonical.starts_with(root) || !canonical.is_file() || canonical.to_str().is_none() {
        return Err(fault(
            "module_path",
            format!("{}：模块需要包内普通 UTF-8 路径文件。", path.display()),
            "not_started",
        ));
    }
    Ok(canonical)
}

fn check_local_path(path: &Path) -> Result<(), Value> {
    // 检查名称先于任何文件读取，检查父目录先于后代，避免跟随联接点或云占位目录。
    for part in path.components() {
        let name = part.as_os_str().to_string_lossy().to_ascii_lowercase();
        if name.starts_with("onedrive") || name == "dropbox" || name.starts_with("icloud") {
            return Err(fault(
                "module_nonlocal",
                "模块跳过云盘目录。",
                "not_started",
            ));
        }
        if matches!(
            part,
            std::path::Component::ParentDir | std::path::Component::CurDir
        ) {
            return Err(fault(
                "module_path",
                "模块工程根目录不能含 . 或 ..。",
                "not_started",
            ));
        }
        #[cfg(windows)]
        if let std::path::Component::Prefix(prefix) = part {
            if !matches!(
                prefix.kind(),
                std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)
            ) {
                return Err(fault(
                    "module_nonlocal",
                    "模块需要普通本地磁盘目录。",
                    "not_started",
                ));
            }
        }
    }
    let mut ancestors: Vec<_> = path.ancestors().collect();
    ancestors.reverse();
    for ancestor in ancestors {
        let metadata = fs::symlink_metadata(ancestor).map_err(|e| io_fault(ancestor, e))?;
        if restricted(&metadata) {
            return Err(fault(
                "module_nonlocal",
                format!(
                    "{}：模块跳过链接、联接点或云端占位文件。",
                    ancestor.display()
                ),
                "not_started",
            ));
        }
    }
    Ok(())
}

fn restricted(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & (0x400 | 0x1000 | 0x40000 | 0x400000) != 0 {
            return true;
        }
    }
    metadata.file_type().is_symlink()
}

#[cfg(feature = "boa")]
#[path = "modules_boa.rs"]
pub mod boa;
#[cfg(feature = "quickjs")]
#[path = "modules_quickjs.rs"]
pub mod quickjs;
