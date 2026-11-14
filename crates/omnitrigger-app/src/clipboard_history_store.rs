//! 显式私有路径中的有界纯文本历史。不监听系统剪贴板，不从用户配置推断路径。
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

pub const FILE_NAME: &str = "clipboard-history.json";
pub const MAX_ITEMS: usize = 200;
pub const MAX_TEXT_UNITS: usize = 65_536;
pub const MAX_TOTAL_UNITS: usize = 1_048_576;
pub const MAX_STORAGE_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HistoryError {
    pub code: &'static str,
    pub message: &'static str,
    pub effect_state: &'static str,
}
impl HistoryError {
    pub fn new(code: &'static str) -> Self {
        let message = match code {
            "invalid_input" => "历史参数无效。",
            "history_item_missing" => "历史项目不存在。",
            "history_busy" => "历史存储正在使用，请稍后明确发起新请求。",
            "history_corrupt" => "历史文件版本或数据无效，原文件已保留。",
            "history_conflict" => "历史文件已发生外部变化，未覆盖现有文件。",
            "history_id_exhausted" => "历史项目标识已达到上限。",
            "history_revision_exhausted" => "历史修订号已达到上限。",
            "cancelled" => "历史操作已取消。",
            "timeout" => "历史操作超过运行时限。",
            _ => "历史存储不可用。",
        };
        Self {
            code,
            message,
            effect_state: "not_started",
        }
    }
    fn unknown(mut self) -> Self {
        self.effect_state = "unknown";
        self
    }
}
type Result<T> = std::result::Result<T, HistoryError>;

fn decimal(value: &str, allow_zero: bool) -> Option<u64> {
    if value.is_empty()
        || value.len() > 20
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return None;
    }
    value
        .parse::<u64>()
        .ok()
        .filter(|&number| allow_zero || number != 0)
}
fn text_units(text: &str) -> Result<usize> {
    let units = text.encode_utf16().take(MAX_TEXT_UNITS + 1).count();
    if units > MAX_TEXT_UNITS || text.contains('\0') {
        return Err(HistoryError::new("invalid_input"));
    }
    Ok(units)
}

/// 单行摘要按 Unicode 标量截取，因此不会切开 UTF-16 代理对。
pub fn preview(text: &str) -> String {
    let mut result = String::new();
    let mut units = 0;
    for character in text.chars() {
        let character = if character.is_control() || matches!(character, '\u{2028}' | '\u{2029}') {
            ' '
        } else {
            character
        };
        if units + character.len_utf16() > 64 {
            break;
        }
        result.push(character);
        units += character.len_utf16();
    }
    result
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Entry {
    id: String,
    text: String,
    created_at: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Document {
    version: u32,
    last_id: String,
    revision: String,
    items: Vec<Entry>, // 最旧在前；时间戳不参与身份或淘汰顺序。
}
impl Default for Document {
    fn default() -> Self {
        Self {
            version: 1,
            last_id: "0".into(),
            revision: "0".into(),
            items: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistorySummary {
    pub id: String,
    pub preview: String,
    pub utf16_units: usize,
    pub created_at: String,
    // 仅供本机窗口核对已显示的条目，外部动作与历史文件均不暴露此摘要。
    #[serde(skip)]
    pub(super) text_sha256: [u8; 32],
}
impl HistorySummary {
    pub(super) fn from_text(id: &str, created_at: &str, text: &str) -> Self {
        Self {
            id: id.to_owned(),
            preview: preview(text),
            utf16_units: text.encode_utf16().count(),
            created_at: created_at.to_owned(),
            text_sha256: Sha256::digest(text.as_bytes()).into(),
        }
    }

    pub(super) fn matches_item(&self, id: &str, created_at: &str, text: &str) -> bool {
        id == self.id
            && created_at == self.created_at
            && text.encode_utf16().count() == self.utf16_units
            && preview(text) == self.preview
            && <[u8; 32]>::from(Sha256::digest(text.as_bytes())) == self.text_sha256
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HistoryList {
    pub items: Vec<HistorySummary>,
    pub revision: String,
}
/// 仅供本应用历史页：全部规范摘要与同一版本下的正文匹配ID，不携带正文。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct HistorySearch {
    pub list: HistoryList,
    pub matching_ids: Vec<String>,
}

/// 词项由有界查询归一化；与摘要筛选共用Unicode小写后的字面AND匹配。
pub(super) fn text_matches_terms(text: &str, terms: &[String]) -> bool {
    if terms.is_empty() {
        return true;
    }
    let lower = text.to_lowercase();
    terms.iter().all(|term| lower.contains(term))
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryItem {
    pub id: String,
    pub text: String,
    pub created_at: String,
    pub revision: String,
}
impl std::fmt::Debug for HistoryItem {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HistoryItem")
            .field("id", &self.id)
            .field("utf16_units", &self.text.encode_utf16().count())
            .field("created_at", &self.created_at)
            .field("revision", &self.revision)
            .finish()
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AppendResult {
    pub id: String,
    pub revision: String,
    pub inserted: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RemoveResult {
    pub removed: bool,
    pub revision: String,
}

impl Document {
    fn validate(&self) -> Result<()> {
        let corrupt = || HistoryError::new("history_corrupt");
        let last = decimal(&self.last_id, true).ok_or_else(corrupt)?;
        let revision = decimal(&self.revision, true).ok_or_else(corrupt)?;
        if self.version != 1 || self.items.len() > MAX_ITEMS || revision < last {
            return Err(corrupt());
        }
        let mut previous = 0;
        let mut total = 0usize;
        let mut ids = BTreeSet::new();
        for item in &self.items {
            let id = decimal(&item.id, false).ok_or_else(corrupt)?;
            if id <= previous
                || id > last
                || !ids.insert(id)
                || decimal(&item.created_at, true).is_none()
            {
                return Err(corrupt());
            }
            previous = id;
            total += text_units(&item.text).map_err(|_| corrupt())?;
            if total > MAX_TOTAL_UNITS {
                return Err(corrupt());
            }
        }
        Ok(())
    }
    fn list(&self, limit: usize) -> Result<HistoryList> {
        if !(1..=MAX_ITEMS).contains(&limit) {
            return Err(HistoryError::new("invalid_input"));
        }
        Ok(HistoryList {
            revision: self.revision.clone(),
            items: self
                .items
                .iter()
                .rev()
                .take(limit)
                .map(|item| HistorySummary::from_text(&item.id, &item.created_at, &item.text))
                .collect(),
        })
    }
    fn get(&self, id: &str) -> Result<HistoryItem> {
        decimal(id, false).ok_or_else(|| HistoryError::new("invalid_input"))?;
        let entry = self
            .items
            .iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| HistoryError::new("history_item_missing"))?;
        Ok(HistoryItem {
            id: entry.id.clone(),
            text: entry.text.clone(),
            created_at: entry.created_at.clone(),
            revision: self.revision.clone(),
        })
    }
    fn append(&mut self, text: &str, created_at_ms: u64) -> Result<AppendResult> {
        let units = text_units(text)?;
        if let Some(last) = self.items.last().filter(|entry| entry.text == text) {
            return Ok(AppendResult {
                id: last.id.clone(),
                revision: self.revision.clone(),
                inserted: false,
            });
        }
        let id = decimal(&self.last_id, true)
            .ok_or_else(|| HistoryError::new("history_corrupt"))?
            .checked_add(1)
            .ok_or_else(|| HistoryError::new("history_id_exhausted"))?;
        let revision = decimal(&self.revision, true)
            .ok_or_else(|| HistoryError::new("history_corrupt"))?
            .checked_add(1)
            .ok_or_else(|| HistoryError::new("history_revision_exhausted"))?;
        let mut total = self
            .items
            .iter()
            .map(|item| item.text.encode_utf16().count())
            .sum::<usize>()
            + units;
        self.items.push(Entry {
            id: id.to_string(),
            text: text.into(),
            created_at: created_at_ms.to_string(),
        });
        let mut evicted = 0;
        while self.items.len() - evicted > MAX_ITEMS || total > MAX_TOTAL_UNITS {
            total -= self.items[evicted].text.encode_utf16().count();
            evicted += 1;
        }
        drop(self.items.drain(..evicted));
        self.last_id = id.to_string();
        self.revision = revision.to_string();
        Ok(AppendResult {
            id: id.to_string(),
            revision: self.revision.clone(),
            inserted: true,
        })
    }
    fn remove(&mut self, id: &str) -> Result<RemoveResult> {
        decimal(id, false).ok_or_else(|| HistoryError::new("invalid_input"))?;
        let Some(index) = self.items.iter().position(|item| item.id == id) else {
            return Ok(RemoveResult {
                removed: false,
                revision: self.revision.clone(),
            });
        };
        let revision = decimal(&self.revision, true)
            .ok_or_else(|| HistoryError::new("history_corrupt"))?
            .checked_add(1)
            .ok_or_else(|| HistoryError::new("history_revision_exhausted"))?;
        self.items.remove(index);
        self.revision = revision.to_string();
        Ok(RemoveResult {
            removed: true,
            revision: self.revision.clone(),
        })
    }
}

const LOCK_MAGIC: &[u8] = b"OmniTrigger.ClipboardHistory.Lock.v1\n";
static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(1);

/// 所有实例遵守同一个永久 sidecar 文件锁；没有陈旧正文缓存，也不按脚本参数选路径。
#[derive(Clone, Debug)]
pub struct ClipboardHistoryStore {
    path: PathBuf,
}

struct Lease {
    file: File,
    initialized: bool,
}
impl Lease {
    fn initialize(&mut self) -> Result<()> {
        if self.initialized {
            return Ok(());
        }
        self.file
            .seek(SeekFrom::Start(LOCK_MAGIC.len() as u64))
            .map_err(|_| unavailable())?;
        self.file
            .write_all(b"1")
            .and_then(|()| self.file.sync_all())
            .map_err(|_| unavailable())?;
        self.initialized = true;
        Ok(())
    }
}
// 不显式 unlock 或删除 lock 文件：最后一个句柄关闭时系统释放锁，崩溃不留下占用。

struct Temporary {
    path: PathBuf,
}
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
fn unavailable() -> HistoryError {
    HistoryError::new("history_unavailable")
}
fn check_path(path: &Path) -> Result<()> {
    crate::script_runtime::check_local_path(path).map_err(|_| unavailable())
}
fn check_file(file: &File) -> Result<()> {
    let metadata = file.metadata().map_err(|_| unavailable())?;
    if !metadata.is_file() {
        return Err(unavailable());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x441400 != 0 {
            return Err(unavailable());
        }
    }
    Ok(())
}
fn safe_open(options: &mut OpenOptions) {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // 检查最终对象本身，不跟随 reparse；锁文件不可在持有句柄时被改名/删除。
        options.custom_flags(0x00200000).share_mode(0x00000003);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
}

fn lock_open_error(error: std::io::Error) -> HistoryError {
    // 首次建立标记时 Windows 使用独占共享模式，其他实例此刻应得到 busy。
    #[cfg(windows)]
    if matches!(error.raw_os_error(), Some(32 | 33)) {
        return HistoryError::new("history_busy");
    }
    let _ = error;
    unavailable()
}

#[cfg(windows)]
fn commit(temporary: &Path, destination: &Path, replace: bool) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        #[link_name = "MoveFileExW"]
        fn move_file_ex_w(existing: *const u16, destination: *const u16, flags: u32) -> i32;
    }
    let wide = |path: &Path| -> Result<Vec<u16>> {
        let mut path = path.as_os_str().encode_wide().collect::<Vec<_>>();
        if path.contains(&0) {
            return Err(unavailable());
        }
        path.push(0);
        Ok(path)
    };
    let source = wide(temporary)?;
    let target = wide(destination)?;
    // 同目录移动。首次创建不允许覆盖；替换仅在持有 sidecar 锁且比较原文后进行。
    let flags = 0x8 | u32::from(replace); // WRITE_THROUGH | 可选 REPLACE_EXISTING
    if unsafe { move_file_ex_w(source.as_ptr(), target.as_ptr(), flags) } != 0 {
        return Ok(());
    }
    let code = std::io::Error::last_os_error().raw_os_error();
    if !replace && matches!(code, Some(80 | 183)) {
        return Err(HistoryError::new("history_conflict"));
    }
    Err(unavailable().unknown())
}

#[cfg(not(windows))]
fn commit(temporary: &Path, destination: &Path, replace: bool) -> Result<()> {
    if replace {
        return fs::rename(temporary, destination).map_err(|_| unavailable().unknown());
    }
    fs::hard_link(temporary, destination).map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            HistoryError::new("history_conflict")
        } else {
            unavailable()
        }
    })
}

impl ClipboardHistoryStore {
    /// 路径来自应用组合根，不来自脚本。构造不读写磁盘，不启动监听器。
    pub fn new(path: PathBuf) -> Result<Self> {
        if !path.is_absolute()
            || path.file_name().is_none()
            || path
                .components()
                .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
        {
            return Err(unavailable());
        }
        Ok(Self { path })
    }

    /// 外部导入修改原始文件时，与历史窗口和动作包使用同一把锁。
    #[cfg(windows)]
    pub(crate) fn hold_for_import(&self) -> Result<File> {
        self.lease().map(|lease| lease.file)
    }

    fn lock_path(&self) -> Result<PathBuf> {
        let mut name = self
            .path
            .file_name()
            .ok_or_else(unavailable)?
            .to_os_string();
        name.push(".lock");
        Ok(self.path.with_file_name(name))
    }
    fn lease(&self) -> Result<Lease> {
        check_path(&self.path)?;
        let parent = self.path.parent().ok_or_else(unavailable)?;
        if !fs::metadata(parent).map_err(|_| unavailable())?.is_dir() {
            return Err(unavailable());
        }
        let lock = self.lock_path()?;
        check_path(&lock)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        safe_open(&mut options);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // 标记写完并关闭句柄前禁止其他实例打开，避免读取半初始化 lock。
            options.share_mode(0);
        }
        let (mut file, created) = match options.open(&lock) {
            Ok(file) => (file, true),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let mut options = OpenOptions::new();
                options.read(true).write(true);
                safe_open(&mut options);
                (options.open(&lock).map_err(lock_open_error)?, false)
            }
            Err(error) => return Err(lock_open_error(error)),
        };
        check_file(&file)?;
        file.try_lock().map_err(|error| match error {
            TryLockError::WouldBlock => HistoryError::new("history_busy"),
            TryLockError::Error(_) => unavailable(),
        })?;
        if created {
            file.write_all(LOCK_MAGIC)
                .and_then(|()| file.write_all(b"0\n"))
                .and_then(|()| file.sync_all())
                .map_err(|_| unavailable())?;
        }
        file.rewind().map_err(|_| unavailable())?;
        let mut bytes = Vec::new();
        (&mut file)
            .take((LOCK_MAGIC.len() + 3) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| unavailable())?;
        let mut expected = LOCK_MAGIC.to_vec();
        expected.extend_from_slice(b"0\n");
        let initialized = if bytes == expected {
            false
        } else {
            expected[LOCK_MAGIC.len()] = b'1';
            if bytes != expected {
                return Err(HistoryError::new("history_corrupt"));
            }
            true
        };
        Ok(Lease { file, initialized })
    }

    fn raw(&self) -> Result<Option<Vec<u8>>> {
        check_path(&self.path)?;
        let mut options = OpenOptions::new();
        options.read(true);
        safe_open(&mut options);
        let file = match options.open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(unavailable()),
        };
        check_file(&file)?;
        if file.metadata().map_err(|_| unavailable())?.len() > MAX_STORAGE_BYTES {
            return Err(HistoryError::new("history_corrupt"));
        }
        let mut bytes = Vec::new();
        file.take(MAX_STORAGE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| unavailable())?;
        if bytes.len() as u64 > MAX_STORAGE_BYTES {
            return Err(HistoryError::new("history_corrupt"));
        }
        Ok(Some(bytes))
    }

    fn load(&self, lease: &mut Lease) -> Result<(Document, Option<Vec<u8>>)> {
        let raw = self.raw()?;
        let document = if let Some(bytes) = &raw {
            let document: Document =
                serde_json::from_slice(bytes).map_err(|_| HistoryError::new("history_corrupt"))?;
            document.validate()?;
            lease.initialize()?; // 之后文件消失不能被当作新历史，避免复用已返回的 ID。
            document
        } else {
            if lease.initialized {
                return Err(HistoryError::new("history_conflict"));
            }
            Document::default()
        };
        Ok((document, raw))
    }

    fn temporary(&self, bytes: &[u8]) -> Result<Temporary> {
        for _ in 0..32 {
            let mut name = self
                .path
                .file_name()
                .ok_or_else(unavailable)?
                .to_os_string();
            let nonce = NEXT_TEMPORARY
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                    value.checked_add(1)
                })
                .map_err(|_| unavailable())?;
            name.push(format!(".{}.{nonce}.tmp", std::process::id()));
            let path = self.path.with_file_name(name);
            check_path(&path)?;
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            safe_open(&mut options);
            match options.open(&path) {
                Ok(mut file) => {
                    let temporary = Temporary { path };
                    let written = check_file(&file).and_then(|()| {
                        file.write_all(bytes)
                            .and_then(|()| file.sync_all())
                            .map_err(|_| unavailable())
                    });
                    drop(file); // Windows 句柄不共享 DELETE；先关闭再走错误/取消清理。
                    written?;
                    return Ok(temporary);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(_) => return Err(unavailable()),
            }
        }
        Err(unavailable())
    }

    fn read<T>(
        &self,
        mut check: impl FnMut() -> Result<()>,
        operation: impl FnOnce(&Document) -> Result<T>,
    ) -> Result<T> {
        check()?;
        let mut lease = self.lease()?;
        check()?;
        let (document, _) = self.load(&mut lease)?;
        check()?;
        let result = operation(&document)?;
        check()?;
        Ok(result)
    }

    fn update<T>(
        &self,
        mut check: impl FnMut() -> Result<()>,
        operation: impl FnOnce(&mut Document) -> Result<(T, bool)>,
    ) -> Result<T> {
        check()?;
        let mut lease = self.lease()?;
        check()?;
        let (mut document, original) = self.load(&mut lease)?;
        check()?;
        let (result, changed) = operation(&mut document)?;
        check()?;
        if !changed {
            return Ok(result);
        }
        document.validate()?;
        let bytes = serde_json::to_vec(&document).map_err(|_| unavailable())?;
        if bytes.len() as u64 > MAX_STORAGE_BYTES {
            return Err(unavailable());
        }
        check()?;
        let temporary = self.temporary(&bytes)?;
        check()?; // 最后一个可取消点在比较前；比较至提交之间不再执行外部回调。
        if self.raw()? != original {
            return Err(HistoryError::new("history_conflict"));
        }
        check_path(&self.path)?;
        commit(&temporary.path, &self.path, original.is_some())?;
        // 从提交开始，后续失败/取消都不能谎称未开始；不自动回滚或重试。
        lease.initialize().map_err(HistoryError::unknown)?;
        check().map_err(HistoryError::unknown)?;
        Ok(result)
    }

    pub fn list(&self, limit: usize, check: impl FnMut() -> Result<()>) -> Result<HistoryList> {
        if !(1..=MAX_ITEMS).contains(&limit) {
            return Err(HistoryError::new("invalid_input"));
        }
        self.read(check, |document| document.list(limit))
    }
    /// 显式正文检索；一次私有只读事务返回同一revision的摘要与ID，扫描最多MAX_ITEMS项。
    pub(super) fn search_full_text(
        &self,
        terms: &[String],
        mut check: impl FnMut() -> Result<()>,
    ) -> Result<HistorySearch> {
        // UI查询最多256字符；归一化允许Unicode小写扩展，但仍给内部参数设硬上限。
        if terms.len() > 256
            || terms
                .iter()
                .any(|term| term.is_empty() || term.len() > 4_096)
            || terms.iter().map(String::len).sum::<usize>() > 4_096
        {
            return Err(HistoryError::new("invalid_input"));
        }
        let (list, texts) = self.read(&mut check, |document| {
            let list = document.list(MAX_ITEMS)?;
            let texts = document
                .items
                .iter()
                .rev()
                .map(|item| (item.id.clone(), item.text.clone()))
                .collect::<Vec<_>>();
            Ok((list, texts))
        })?;
        // 快照已取得且lease已释放；CPU匹配不继续占着采集写入的存储槽。
        check()?;
        let matching_ids = texts
            .into_iter()
            .filter(|(_, text)| text_matches_terms(text, terms))
            .map(|(id, _)| id)
            .collect();
        check()?;
        Ok(HistorySearch { list, matching_ids })
    }

    pub fn get(&self, id: &str, check: impl FnMut() -> Result<()>) -> Result<HistoryItem> {
        decimal(id, false).ok_or_else(|| HistoryError::new("invalid_input"))?;
        self.read(check, |document| document.get(id))
    }
    pub fn append_at(
        &self,
        text: &str,
        created_at_ms: u64,
        check: impl FnMut() -> Result<()>,
    ) -> Result<AppendResult> {
        text_units(text)?;
        self.update(check, |document| {
            let result = document.append(text, created_at_ms)?;
            let changed = result.inserted;
            Ok((result, changed))
        })
    }
    pub fn append(&self, text: &str, check: impl FnMut() -> Result<()>) -> Result<AppendResult> {
        let milliseconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| unavailable())?
            .as_millis();
        let milliseconds = u64::try_from(milliseconds).map_err(|_| unavailable())?;
        self.append_at(text, milliseconds, check)
    }
    pub fn remove(&self, id: &str, check: impl FnMut() -> Result<()>) -> Result<RemoveResult> {
        decimal(id, false).ok_or_else(|| HistoryError::new("invalid_input"))?;
        self.update(check, |document| {
            let result = document.remove(id)?;
            let changed = result.removed;
            Ok((result, changed))
        })
    }

    pub(super) fn remove_selected(
        &self,
        selected: &HistorySummary,
        check: impl FnMut() -> Result<()>,
    ) -> Result<RemoveResult> {
        self.update(check, |document| {
            let current = document.items.iter().find(|item| item.id == selected.id);
            if !current
                .is_some_and(|item| selected.matches_item(&item.id, &item.created_at, &item.text))
            {
                return Err(HistoryError::new("history_conflict"));
            }
            document.remove(&selected.id).map(|result| (result, true))
        })
    }
}

#[cfg(test)]
mod persistence_tests {
    use super::*;
    struct Fixture {
        directory: PathBuf,
        store: ClipboardHistoryStore,
    }
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(1);
            let directory = loop {
                let candidate = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
                    ".history-store-test-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                match fs::create_dir(&candidate) {
                    Ok(()) => break candidate,
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("无法创建合成测试目录：{error}"),
                }
            };
            let store = ClipboardHistoryStore::new(directory.join("history.json")).unwrap();
            Self { directory, store }
        }
        fn bytes(&self) -> Vec<u8> {
            fs::read(&self.store.path).unwrap()
        }
        fn temporary_exists(&self) -> bool {
            fs::read_dir(&self.directory).unwrap().any(|entry| {
                entry
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "tmp")
            })
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            assert!(self.directory.starts_with(env!("CARGO_MANIFEST_DIR")));
            assert!(self
                .directory
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".history-store-test-"));
            // 独占的合成测试目录，只删普通文件，不递归、不跟随链接或联接点。
            if let Ok(entries) = fs::read_dir(&self.directory) {
                for entry in entries.flatten() {
                    if entry
                        .file_type()
                        .is_ok_and(|kind| kind.is_file() && !kind.is_symlink())
                    {
                        let _ = fs::remove_file(entry.path());
                    }
                }
            }
            let _ = fs::remove_dir(&self.directory);
        }
    }

    #[test]
    fn full_text_search_finds_tail_beyond_preview_without_body_output_or_writes() {
        let fixture = Fixture::new();
        let text = format!("{} Needle 中文\r\n保留空白  ", "前".repeat(80));
        let saved = fixture.store.append_at(&text, 1, || Ok(())).unwrap();
        fixture.store.append_at("其他摘要", 2, || Ok(())).unwrap();
        let bytes = fixture.bytes();
        let query = crate::clipboard_history_filter::SummaryQuery::new("NEEDLE 中文").unwrap();
        let result = fixture
            .store
            .search_full_text(query.terms(), || Ok(()))
            .unwrap();
        assert_eq!(result.matching_ids, vec![saved.id.clone()]);
        assert_eq!(result.list.items.len(), 2);
        assert!(!result.list.items[1].preview.contains("Needle"));
        assert!(!format!("{result:?}").contains("Needle"));
        assert_eq!(fixture.bytes(), bytes);
        assert_eq!(fixture.store.get(&saved.id, || Ok(())).unwrap().text, text);
        assert_eq!(
            fixture.store.list(MAX_ITEMS, || Ok(())).unwrap().revision,
            result.list.revision
        );
    }
    #[test]
    fn full_text_search_uses_all_terms_and_empty_query_keeps_stored_order() {
        let fixture = Fixture::new();
        let first = fixture
            .store
            .append_at("Alpha 中文\nBeta", 9, || Ok(()))
            .unwrap();
        let last = fixture.store.append_at("只含 ALPHA", 1, || Ok(())).unwrap();
        let query = crate::clipboard_history_filter::SummaryQuery::new("alpha BETA 中文").unwrap();
        assert_eq!(
            fixture
                .store
                .search_full_text(query.terms(), || Ok(()))
                .unwrap()
                .matching_ids,
            vec![first.id.clone()]
        );
        let result = fixture.store.search_full_text(&[], || Ok(())).unwrap();
        assert_eq!(result.matching_ids, vec![last.id, first.id]);
        assert_eq!(
            result.matching_ids,
            result
                .list
                .items
                .iter()
                .map(|item| item.id.clone())
                .collect::<Vec<_>>()
        );
    }
    #[test]
    fn full_text_search_rejects_unbounded_terms_and_cancel_before_disk() {
        let fixture = Fixture::new();
        let error = fixture
            .store
            .search_full_text(&vec!["x".into(); 257], || Ok(()))
            .unwrap_err();
        assert_eq!(error.code, "invalid_input");
        assert!(!fixture.store.path.exists());
        let error = fixture
            .store
            .search_full_text(&["x".repeat(4_097)], || Ok(()))
            .unwrap_err();
        assert_eq!(error.code, "invalid_input");
        assert!(!fixture.store.path.exists());
        let error = fixture
            .store
            .search_full_text(&[], || Err(HistoryError::new("cancelled")))
            .unwrap_err();
        assert_eq!(error.code, "cancelled");
        assert!(!fixture.store.path.exists());
    }
    #[test]
    fn full_text_search_cancel_after_scan_returns_no_partial_receipt_or_mutation() {
        let fixture = Fixture::new();
        fixture
            .store
            .append_at("合成 needle", 1, || Ok(()))
            .unwrap();
        let before = fixture.bytes();
        let mut checks = 0;
        let result = fixture.store.search_full_text(&["needle".into()], || {
            checks += 1;
            if checks == 6 {
                Err(HistoryError::new("cancelled"))
            } else {
                Ok(())
            }
        });
        assert_eq!(result.unwrap_err().code, "cancelled");
        assert_eq!(fixture.bytes(), before);
    }

    #[test]
    fn full_text_search_accepts_unicode_lowercase_expansion_of_a_valid_query() {
        let fixture = Fixture::new();
        let text = "İ".repeat(256);
        let saved = fixture.store.append_at(&text, 1, || Ok(())).unwrap();
        let query = crate::clipboard_history_filter::SummaryQuery::new(&text).unwrap();
        assert_eq!(
            fixture
                .store
                .search_full_text(query.terms(), || Ok(()))
                .unwrap()
                .matching_ids,
            vec![saved.id]
        );
    }

    #[test]
    fn full_text_search_releases_lease_before_matching_and_keeps_snapshot_identity() {
        let fixture = Fixture::new();
        let first = fixture
            .store
            .append_at("合成 needle", 1, || Ok(()))
            .unwrap();
        let mut checks = 0;
        let mut wrote = false;
        let result = fixture
            .store
            .search_full_text(&["needle".into()], || {
                checks += 1;
                if checks == 5 {
                    fixture
                        .store
                        .append_at("另一条 needle 采集", 2, || Ok(()))?;
                    wrote = true;
                }
                Ok(())
            })
            .unwrap();
        assert!(wrote);
        assert_eq!(result.matching_ids, vec![first.id]);
        assert_eq!(result.list.items.len(), 1);
        let latest = fixture.store.list(MAX_ITEMS, || Ok(())).unwrap();
        assert_eq!(latest.items.len(), 2);
        assert_ne!(result.list.revision, latest.revision);
    }

    #[test]
    fn fresh_and_stale_instances_read_current_revision_and_do_not_lose_updates() {
        let fixture = Fixture::new();
        let other = ClipboardHistoryStore::new(fixture.store.path.clone()).unwrap();
        assert!(fixture.store.list(100, || Ok(())).unwrap().items.is_empty());
        assert_eq!(
            fixture.store.append_at("one", 1, || Ok(())).unwrap().id,
            "1"
        );
        fixture.store.list(100, || Ok(())).unwrap();
        assert_eq!(other.append_at("two", 2, || Ok(())).unwrap().id, "2");
        assert_eq!(
            fixture.store.append_at("three", 0, || Ok(())).unwrap().id,
            "3"
        );
        let list = other.list(200, || Ok(())).unwrap();
        assert_eq!(list.revision, "3");
        assert_eq!(
            list.items
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            ["3", "2", "1"]
        );
        assert_eq!(other.get("1", || Ok(())).unwrap().text, "one");
    }

    #[test]
    fn lock_contention_is_explicit_and_drop_releases_it_for_another_handle() {
        let fixture = Fixture::new();
        fixture.store.append_at("one", 0, || Ok(())).unwrap();
        let original = fixture.bytes();
        let lease = fixture.store.lease().unwrap();
        let other = fixture.store.clone();
        let error = std::thread::spawn(move || other.append_at("blocked", 0, || Ok(())))
            .join()
            .unwrap()
            .unwrap_err();
        assert_eq!(error.code, "history_busy");
        assert_eq!(error.effect_state, "not_started");
        assert_eq!(fixture.bytes(), original);
        drop(lease);
        assert_eq!(
            fixture
                .store
                .clone()
                .append_at("allowed", 0, || Ok(()))
                .unwrap()
                .id,
            "2"
        );
    }

    #[test]
    #[cfg(windows)]
    fn first_lock_initialization_is_busy_until_its_marker_is_complete() {
        use std::os::windows::fs::OpenOptionsExt;
        let fixture = Fixture::new();
        let lock = fixture.store.lock_path().unwrap();
        let mut held = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .share_mode(0)
            .open(&lock)
            .unwrap();
        let error = fixture
            .store
            .append_at("blocked", 0, || Ok(()))
            .unwrap_err();
        assert_eq!(error.code, "history_busy");
        assert_eq!(error.effect_state, "not_started");
        assert!(!fixture.store.path.exists());
        held.write_all(LOCK_MAGIC).unwrap();
        held.write_all(b"0\n").unwrap();
        held.sync_all().unwrap();
        drop(held);
        assert_eq!(
            fixture.store.append_at("first", 0, || Ok(())).unwrap().id,
            "1"
        );
    }

    #[test]
    fn cancel_before_or_after_preparation_keeps_original_bytes_and_removes_temp() {
        let fixture = Fixture::new();
        fixture.store.append_at("original", 0, || Ok(())).unwrap();
        let original = fixture.bytes();
        assert_eq!(
            fixture
                .store
                .append_at("new", 0, || Err(HistoryError::new("cancelled")))
                .unwrap_err()
                .effect_state,
            "not_started"
        );
        let error = fixture
            .store
            .append_at("new", 0, || {
                if fixture.temporary_exists() {
                    Err(HistoryError::new("cancelled"))
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
        assert_eq!(error.code, "cancelled");
        assert_eq!(error.effect_state, "not_started");
        assert_eq!(fixture.bytes(), original);
        assert!(!fixture.temporary_exists());
        assert_eq!(
            fixture.store.append_at("next", 0, || Ok(())).unwrap().id,
            "2"
        );
    }

    #[test]
    fn cancellation_after_commit_reports_unknown_and_does_not_repeat_mutation() {
        let fixture = Fixture::new();
        fixture.store.append_at("original", 0, || Ok(())).unwrap();
        let original = fixture.bytes();
        let error = fixture
            .store
            .append_at("committed", 0, || {
                if fixture.bytes() != original {
                    Err(HistoryError::new("cancelled"))
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
        assert_eq!(error.code, "cancelled");
        assert_eq!(error.effect_state, "unknown");
        assert_eq!(fixture.store.get("2", || Ok(())).unwrap().text, "committed");
        assert_eq!(fixture.store.list(100, || Ok(())).unwrap().revision, "2");
        assert!(!fixture.temporary_exists());
    }

    #[test]
    fn outside_write_detected_before_commit_is_not_overwritten() {
        let fixture = Fixture::new();
        fixture.store.append_at("original", 0, || Ok(())).unwrap();
        let mut foreign = Document::default();
        foreign.append("foreign", 0).unwrap();
        foreign.append("foreign-new", 0).unwrap();
        let foreign = serde_json::to_vec(&foreign).unwrap();
        let error = fixture
            .store
            .append_at("ours", 0, || {
                if fixture.temporary_exists() {
                    fs::write(&fixture.store.path, &foreign).unwrap();
                }
                Ok(())
            })
            .unwrap_err();
        assert_eq!(error.code, "history_conflict");
        assert_eq!(error.effect_state, "not_started");
        assert_eq!(fixture.bytes(), foreign);
        assert!(!fixture.temporary_exists());
    }

    #[test]
    fn corrupt_unknown_and_oversized_documents_are_preserved() {
        for contents in [
            b"not json synthetic-private-body".as_slice(),
            b"{\"version\":2,\"lastId\":\"0\",\"revision\":\"0\",\"items\":[]}".as_slice(),
            b"{\"version\":1,\"lastId\":\"0\",\"revision\":\"0\",\"items\":[],\"unknown\":true}"
                .as_slice(),
        ] {
            let fixture = Fixture::new();
            fs::write(&fixture.store.path, contents).unwrap();
            let error = fixture.store.append_at("new", 0, || Ok(())).unwrap_err();
            assert_eq!(error.code, "history_corrupt");
            assert!(!error.message.contains("synthetic-private-body"));
            assert_eq!(fixture.bytes(), contents);
        }
        let fixture = Fixture::new();
        File::create(&fixture.store.path)
            .unwrap()
            .set_len(MAX_STORAGE_BYTES + 1)
            .unwrap();
        assert_eq!(
            fixture.store.list(100, || Ok(())).unwrap_err().code,
            "history_corrupt"
        );
        assert_eq!(
            fs::metadata(&fixture.store.path).unwrap().len(),
            MAX_STORAGE_BYTES + 1
        );
    }

    #[test]
    fn deleted_initialized_file_and_unknown_lock_cannot_silently_reset_history() {
        let fixture = Fixture::new();
        fixture.store.append_at("one", 0, || Ok(())).unwrap();
        fs::remove_file(&fixture.store.path).unwrap();
        let error = fixture
            .store
            .clone()
            .append_at("reused-id", 0, || Ok(()))
            .unwrap_err();
        assert_eq!(error.code, "history_conflict");
        assert!(!fixture.store.path.exists());
        let fixture = Fixture::new();
        let lock = fixture.store.lock_path().unwrap();
        fs::write(&lock, b"unrecognized lock bytes").unwrap();
        assert_eq!(
            fixture.store.append_at("x", 0, || Ok(())).unwrap_err().code,
            "history_corrupt"
        );
        assert_eq!(fs::read(lock).unwrap(), b"unrecognized lock bytes");
        assert!(!fixture.store.path.exists());
    }

    #[test]
    fn no_change_preserves_revision_timestamps_and_original_formatting() {
        let fixture = Fixture::new();
        fixture
            .store
            .append_at("  exact  ", 123, || Ok(()))
            .unwrap();
        let original = fixture.bytes();
        let spaced = [b" \r\n".as_slice(), original.as_slice(), b"\r\n".as_slice()].concat();
        fs::write(&fixture.store.path, &spaced).unwrap();
        assert!(
            !fixture
                .store
                .append_at("  exact  ", 456, || Ok(()))
                .unwrap()
                .inserted
        );
        assert!(!fixture.store.remove("2", || Ok(())).unwrap().removed);
        assert_eq!(fixture.bytes(), spaced);
        assert_eq!(fixture.store.get("1", || Ok(())).unwrap().created_at, "123");
        assert_eq!(fixture.store.list(100, || Ok(())).unwrap().revision, "1");
    }

    #[test]
    fn escaped_capacity_is_persisted_and_reopened_without_truncation() {
        let fixture = Fixture::new();
        let mut document = Document::default();
        for index in 0..16 {
            document
                .append(
                    &format!("{index:02}{}", "\u{1}".repeat(MAX_TEXT_UNITS - 2)),
                    index,
                )
                .unwrap();
        }
        fs::write(&fixture.store.path, serde_json::to_vec(&document).unwrap()).unwrap();
        let text = format!("17{}", "\u{1}".repeat(MAX_TEXT_UNITS - 2));
        let result = fixture.store.append_at(&text, 0, || Ok(())).unwrap();
        assert_eq!(result.id, "17");
        assert!(fixture.bytes().len() as u64 <= MAX_STORAGE_BYTES);
        let reopened = ClipboardHistoryStore::new(fixture.store.path.clone()).unwrap();
        let list = reopened.list(200, || Ok(())).unwrap();
        assert_eq!(list.items.len(), 16);
        assert_eq!(list.items.last().unwrap().id, "2");
        assert_eq!(
            list.items
                .iter()
                .map(|item| item.utf16_units)
                .sum::<usize>(),
            MAX_TOTAL_UNITS
        );
        assert_eq!(reopened.get("17", || Ok(())).unwrap().text, text);
        assert_eq!(
            reopened.get("1", || Ok(())).unwrap_err().code,
            "history_item_missing"
        );
    }

    #[test]
    fn counters_are_decimal_strings_and_u64_max_never_wraps_or_reuses() {
        let fixture = Fixture::new();
        let document = Document {
            last_id: (u64::MAX - 1).to_string(),
            revision: (u64::MAX - 1).to_string(),
            ..Document::default()
        };
        fs::write(&fixture.store.path, serde_json::to_vec(&document).unwrap()).unwrap();
        let result = fixture.store.append_at("last", 0, || Ok(())).unwrap();
        assert_eq!(result.id, u64::MAX.to_string());
        assert_eq!(result.revision, u64::MAX.to_string());
        let original = fixture.bytes();
        assert_eq!(
            fixture
                .store
                .append_at("overflow", 0, || Ok(()))
                .unwrap_err()
                .code,
            "history_id_exhausted"
        );
        assert_eq!(
            fixture
                .store
                .remove(&result.id, || Ok(()))
                .unwrap_err()
                .code,
            "history_revision_exhausted"
        );
        assert_eq!(fixture.bytes(), original);
        assert_eq!(
            fixture.store.get(&result.id, || Ok(())).unwrap().text,
            "last"
        );
    }

    #[test]
    fn invalid_arguments_are_rejected_without_creating_storage_or_parents() {
        let fixture = Fixture::new();
        let parent = fixture.directory.join("not-created");
        let store = ClipboardHistoryStore::new(parent.join("history.json")).unwrap();
        assert_eq!(
            store.append_at("a\0b", 0, || Ok(())).unwrap_err().code,
            "invalid_input"
        );
        assert_eq!(store.list(0, || Ok(())).unwrap_err().code, "invalid_input");
        assert_eq!(store.get("0", || Ok(())).unwrap_err().code, "invalid_input");
        assert_eq!(
            store.remove("01", || Ok(())).unwrap_err().code,
            "invalid_input"
        );
        assert!(!parent.exists());
        assert!(ClipboardHistoryStore::new(PathBuf::from("relative.json")).is_err());
    }

    #[test]
    fn result_types_have_exact_fields_and_get_debug_does_not_print_text() {
        let fixture = Fixture::new();
        let text = "A".repeat(80) + "synthetic-private-body";
        let appended =
            serde_json::to_value(fixture.store.append(&text, || Ok(())).unwrap()).unwrap();
        assert_eq!(
            appended,
            serde_json::json!({"id":"1","revision":"1","inserted":true})
        );
        let list = serde_json::to_value(fixture.store.list(100, || Ok(())).unwrap()).unwrap();
        assert_eq!(list.as_object().unwrap().len(), 2);
        assert_eq!(list["items"][0].as_object().unwrap().len(), 4);
        assert!(list["items"][0].get("text").is_none());
        assert_eq!(list["items"][0]["preview"], "A".repeat(64));
        let item = serde_json::to_value(fixture.store.get("1", || Ok(())).unwrap()).unwrap();
        assert_eq!(item.as_object().unwrap().len(), 4);
        assert_eq!(item["text"], text);
        assert!(!format!("{:?}", fixture.store.get("1", || Ok(())).unwrap())
            .contains("synthetic-private-body"));
        assert_eq!(
            serde_json::to_value(fixture.store.remove("1", || Ok(())).unwrap()).unwrap(),
            serde_json::json!({"removed":true,"revision":"2"})
        );
        assert_eq!(
            serde_json::to_value(fixture.store.remove("1", || Ok(())).unwrap()).unwrap(),
            serde_json::json!({"removed":false,"revision":"2"})
        );
        assert_eq!(
            fixture.store.get("1", || Ok(())).unwrap_err().code,
            "history_item_missing"
        );
    }

    #[test]
    fn initial_commit_cannot_overwrite_a_file_created_after_preparation() {
        let fixture = Fixture::new();
        let temporary = fixture.store.temporary(b"ours").unwrap();
        fs::write(&fixture.store.path, b"foreign").unwrap();
        let error = commit(&temporary.path, &fixture.store.path, false).unwrap_err();
        assert_eq!(error.code, "history_conflict");
        assert_eq!(error.effect_state, "not_started");
        assert_eq!(fixture.bytes(), b"foreign");
    }

    #[test]
    #[ignore = "仅由同模块父测试以合成目录显式启动"]
    fn child_lock_probe() {
        let Some(directory) = std::env::var_os("OMNITRIGGER_SYNTHETIC_HISTORY_TEST_DIRECTORY")
        else {
            return;
        };
        let directory = PathBuf::from(directory);
        assert!(directory.starts_with(env!("CARGO_MANIFEST_DIR")));
        assert!(directory
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(".history-store-test-"));
        let store = ClipboardHistoryStore::new(directory.join("history.json")).unwrap();
        let result = store.append_at("child", 0, || Ok(()));
        if std::env::var_os("OMNITRIGGER_SYNTHETIC_HISTORY_EXPECT_BUSY").is_some() {
            assert_eq!(result.unwrap_err().code, "history_busy");
        } else {
            assert_eq!(result.unwrap().id, "2");
        }
    }

    #[test]
    fn another_process_observes_the_same_lock_and_then_current_ids() {
        let fixture = Fixture::new();
        fixture.store.append_at("parent", 0, || Ok(())).unwrap();
        let original = fixture.bytes();
        let child = |busy: bool| {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "clipboard_history_store::persistence_tests::child_lock_probe",
                    "--ignored",
                    "--test-threads=1",
                ])
                .env(
                    "OMNITRIGGER_SYNTHETIC_HISTORY_TEST_DIRECTORY",
                    &fixture.directory,
                )
                .env_remove("OMNITRIGGER_SYNTHETIC_HISTORY_EXPECT_BUSY");
            if busy {
                command.env("OMNITRIGGER_SYNTHETIC_HISTORY_EXPECT_BUSY", "1");
            }
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(0x08000000);
            }
            assert!(command.output().unwrap().status.success());
        };
        let lease = fixture.store.lease().unwrap();
        child(true);
        assert_eq!(fixture.bytes(), original);
        drop(lease);
        child(false);
        assert_eq!(fixture.store.get("2", || Ok(())).unwrap().text, "child");
    }

    #[test]
    #[cfg(windows)]
    fn replacement_failure_keeps_old_file_and_cleans_prepared_content() {
        use std::os::windows::fs::OpenOptionsExt;
        let fixture = Fixture::new();
        fixture.store.append_at("original", 0, || Ok(())).unwrap();
        let original = fixture.bytes();
        let held = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&fixture.store.path)
            .unwrap();
        let error = fixture.store.append_at("new", 0, || Ok(())).unwrap_err();
        assert_eq!(error.effect_state, "unknown");
        assert_eq!(fixture.bytes(), original);
        assert!(!fixture.temporary_exists());
        drop(held);
        fixture.store.append_at("new", 0, || Ok(())).unwrap();
    }
}

#[cfg(test)]
mod model_tests {
    use super::*;
    #[test]
    fn whitespace_empty_dedup_and_ids_survive_remove() {
        let mut document = Document::default();
        assert_eq!(document.append("", 0).unwrap().id, "1");
        assert!(!document.append("", 1).unwrap().inserted);
        assert_eq!(document.append("  x\r\n", 2).unwrap().id, "2");
        assert_eq!(document.get("2").unwrap().text, "  x\r\n");
        assert_eq!(document.list(1).unwrap().items[0].preview, "  x  ");
        assert!(!document.remove("9").unwrap().removed);
        assert_eq!(document.revision, "2");
        assert!(document.remove("2").unwrap().removed);
        assert_eq!(document.append("next", 3).unwrap().id, "3");
        document.validate().unwrap();
    }
    #[test]
    fn preview_cannot_split_surrogates_and_never_returns_body_in_list() {
        assert_eq!(preview(&("x".repeat(63) + "😀")), "x".repeat(63));
        assert_eq!(preview(&"😀".repeat(33)).encode_utf16().count(), 64);
        assert!(!preview("a\r\n\t\u{2028}\u{2029}b")
            .contains(['\r', '\n', '\t', '\u{2028}', '\u{2029}']));
        let mut document = Document::default();
        document.append(&"x".repeat(200), 0).unwrap();
        let value = serde_json::to_value(document.list(100).unwrap()).unwrap();
        assert!(value["items"][0].get("text").is_none());
        assert_eq!(value["items"][0]["utf16Units"], 200);
    }
    #[test]
    fn oldest_entries_are_evicted_without_truncation() {
        let mut document = Document::default();
        for index in 0..201 {
            document.append(&format!("text-{index}"), index).unwrap();
        }
        assert_eq!(document.items.len(), MAX_ITEMS);
        assert_eq!(document.items[0].id, "2");
        let mut document = Document::default();
        for index in 0..17 {
            document
                .append(
                    &format!("{index:02}{}", "x".repeat(MAX_TEXT_UNITS - 2)),
                    index,
                )
                .unwrap();
        }
        assert_eq!(document.items.len(), 16);
        assert!(document
            .items
            .iter()
            .all(|item| item.text.encode_utf16().count() == MAX_TEXT_UNITS));
        document.validate().unwrap();
    }
    #[test]
    fn invalid_input_and_counter_exhaustion_do_not_mutate() {
        let mut document = Document::default();
        for text in ["a\0b".to_string(), "😀".repeat(32769)] {
            assert!(document.append(&text, 0).is_err());
        }
        for id in ["0", "01", "-1", "18446744073709551616", ""] {
            assert!(document.remove(id).is_err());
        }
        document.last_id = u64::MAX.to_string();
        document.revision = u64::MAX.to_string();
        let original = serde_json::to_vec(&document).unwrap();
        assert_eq!(
            document.append("x", 0).unwrap_err().code,
            "history_id_exhausted"
        );
        assert_eq!(serde_json::to_vec(&document).unwrap(), original);
        document.last_id = "0".into();
        assert_eq!(
            document.append("x", 0).unwrap_err().code,
            "history_revision_exhausted"
        );
    }
    #[test]
    fn strict_documents_preserve_unknown_or_corrupt_input() {
        for value in [
            serde_json::json!({"version":2,"lastId":"0","revision":"0","items":[]}),
            serde_json::json!({"version":1,"lastId":"0","revision":"0","items":[],"extra":1}),
            serde_json::json!({"version":1,"lastId":"1","revision":"0","items":[]}),
            serde_json::json!({"version":1,"lastId":"1","revision":"1","items":[{"id":"1","text":"x","createdAt":"0"},{"id":"1","text":"y","createdAt":"0"}]}),
        ] {
            let original = value.to_string();
            let result = serde_json::from_str::<Document>(&original);
            assert!(result.is_err() || result.unwrap().validate().is_err());
            assert_eq!(value.to_string(), original);
        }
    }
}
