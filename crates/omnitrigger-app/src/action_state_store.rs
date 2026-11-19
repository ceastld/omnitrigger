//! 应用所有者显式挂载的状态适配；构造不读写文件，不访问旧配置。
use omnitrigger_actions::state::{
    valid_state_values, ActionStateScope, DurableStateSnapshot, DurableStateStorage,
    StateStorageError, MAX_STATE_DOCUMENT_BYTES,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions, TryLockError},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const FILE_LIMIT: usize = MAX_STATE_DOCUMENT_BYTES + 4096;
type Result<T> = std::result::Result<T, StateStorageError>;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema_version: u32,
    revision: u64,
    scope: ActionStateScope,
    values: BTreeMap<String, Value>,
}

impl Document {
    fn decode(bytes: &[u8]) -> Result<Self> {
        let doc: Self = serde_json::from_slice(bytes).map_err(|_| StateStorageError::Corrupt)?;
        if doc.schema_version != 1 || doc.revision == 0 || !valid_state_values(&doc.values) {
            return Err(StateStorageError::Corrupt);
        }
        Ok(doc)
    }
}

pub struct FileActionStateStorage {
    root: PathBuf,
    #[cfg(test)]
    fail_before_replace: std::sync::atomic::AtomicBool,
}

fn local(path: &Path) -> Result<()> {
    for part in path.ancestors() {
        let name = part
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        if name.starts_with("onedrive") || name == "dropbox" || name.starts_with("icloud") {
            return Err(StateStorageError::Unavailable);
        }
        match fs::symlink_metadata(part) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(StateStorageError::Unavailable);
                }
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if metadata.file_attributes() & 0x441400 != 0 {
                        return Err(StateStorageError::Unavailable);
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(StateStorageError::Unavailable),
        }
    }
    Ok(())
}
fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

impl FileActionStateStorage {
    pub fn new(root: PathBuf) -> Result<Self> {
        if !root.is_absolute() || root.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(StateStorageError::Unavailable);
        }
        // 仅词法检查；真正的文件属性复验在显式load/commit执行。
        Ok(Self {
            root,
            #[cfg(test)]
            fail_before_replace: std::sync::atomic::AtomicBool::new(false),
        })
    }
    fn document_name(scope: &ActionStateScope) -> Result<String> {
        if !valid_id(&scope.package_id) || !valid_id(&scope.action_id) {
            return Err(StateStorageError::Unavailable);
        }
        let identity = serde_json::to_vec(scope).map_err(|_| StateStorageError::Unavailable)?;
        // 用户key不参与路径；文档内也复核真实scope，不能仅依赖文件名hash。
        Ok(format!("{}.json", digest(&identity)))
    }
    fn path(&self, scope: &ActionStateScope) -> Result<PathBuf> {
        Ok(self.root.join(Self::document_name(scope)?))
    }
    fn bytes(path: &Path) -> Result<Option<Vec<u8>>> {
        local(path)?;
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(StateStorageError::Io),
        };
        if !file
            .metadata()
            .map_err(|_| StateStorageError::Io)?
            .is_file()
        {
            return Err(StateStorageError::Corrupt);
        }
        let mut bytes = Vec::new();
        file.take(FILE_LIMIT as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| StateStorageError::Io)?;
        if bytes.len() > FILE_LIMIT {
            return Err(StateStorageError::Corrupt);
        }
        Ok(Some(bytes))
    }

    /// 只把已经验证过的原始字节写进待发布的配置档，避免再次打开可能已被替换的来源。
    pub(super) fn restore_document(source: &Path, destination_dir: &Path) -> std::io::Result<()> {
        let (name, bytes) = Self::read_restore_document(source)?;
        File::create_new(destination_dir.join(name))?.write_all(&bytes)
    }

    pub(super) fn read_restore_document(source: &Path) -> std::io::Result<(String, Vec<u8>)> {
        let invalid = |reason: &str| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("无法还原动作状态文档 {}：{reason}", source.display()),
            )
        };
        let bytes = Self::bytes(source)
            .map_err(|error| match error {
                StateStorageError::Corrupt => invalid("文件类型或大小超出支持范围"),
                _ => invalid("无法读取本机文件"),
            })?
            .ok_or_else(|| invalid("文件已不存在"))?;
        let doc = Document::decode(&bytes).map_err(|_| invalid("内容损坏或版本不受支持"))?;
        let name = Self::document_name(&doc.scope).map_err(|_| invalid("动作标识无效"))?;
        if !source
            .file_name()
            .is_some_and(|file| file.eq_ignore_ascii_case(&name))
        {
            return Err(invalid("文件名与记录的动作标识不符"));
        }
        Ok((name, bytes))
    }
    fn version(bytes: Option<&[u8]>) -> String {
        bytes.map_or_else(|| "missing".into(), digest)
    }
    fn atomic_write(
        &self,
        path: &Path,
        bytes: &[u8],
        before_commit: &dyn Fn() -> Result<()>,
    ) -> Result<()> {
        // 沿settings::atomic_write：同目录排他临时文件、完整写入、sync_all后rename。
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let name = path
            .file_name()
            .ok_or(StateStorageError::Unavailable)?
            .to_string_lossy();
        let temporary = path.with_file_name(format!(
            "{name}.{}.{}.tmp",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        local(&temporary)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|_| StateStorageError::Io)?;
        let result = (|| {
            file.write_all(bytes).map_err(|_| StateStorageError::Io)?;
            file.sync_all().map_err(|_| StateStorageError::Io)?;
            drop(file);
            before_commit()?;
            #[cfg(test)]
            if self.fail_before_replace.load(Ordering::Acquire) {
                return Err(StateStorageError::Io);
            }
            local(path)?;
            fs::rename(&temporary, path).map_err(|_| StateStorageError::Io)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

impl DurableStateStorage for FileActionStateStorage {
    fn load(&self, scope: &ActionStateScope) -> Result<DurableStateSnapshot> {
        let path = self.path(scope)?;
        let bytes = Self::bytes(&path)?;
        let values = match bytes.as_deref() {
            Some(bytes) => {
                let doc = Document::decode(bytes)?;
                if &doc.scope != scope {
                    return Err(StateStorageError::Corrupt);
                }
                doc.values
            }
            None => BTreeMap::new(),
        };
        Ok(DurableStateSnapshot {
            values,
            version: Self::version(bytes.as_deref()),
        })
    }
    fn commit(
        &self,
        scope: &ActionStateScope,
        expected_version: &str,
        values: &BTreeMap<String, Value>,
        before_commit: &dyn Fn() -> Result<()>,
    ) -> Result<String> {
        if !valid_state_values(values) {
            return Err(StateStorageError::Corrupt);
        }
        before_commit()?;
        let path = self.path(scope)?;
        local(&path)?;
        fs::create_dir_all(&self.root).map_err(|_| StateStorageError::Io)?;
        local(&path)?;
        let lock_path = path.with_extension("lock");
        local(&lock_path)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|_| StateStorageError::Io)?;
        lock.try_lock().map_err(|e| match e {
            TryLockError::WouldBlock => StateStorageError::Busy,
            _ => StateStorageError::Io,
        })?;
        let current = Self::bytes(&path)?;
        if Self::version(current.as_deref()) != expected_version {
            return Err(StateStorageError::Changed);
        }
        let revision = match current.as_deref() {
            Some(bytes) => {
                let previous = Document::decode(bytes)?;
                if previous.scope != *scope {
                    return Err(StateStorageError::Corrupt);
                }
                previous
                    .revision
                    .checked_add(1)
                    .ok_or(StateStorageError::Limit)?
            }
            None => 1,
        };
        let doc = Document {
            schema_version: 1,
            revision,
            scope: scope.clone(),
            values: values.clone(),
        };
        let bytes = serde_json::to_vec(&doc).map_err(|_| StateStorageError::Corrupt)?;
        if bytes.len() > FILE_LIMIT {
            return Err(StateStorageError::Corrupt);
        }
        self.atomic_write(&path, &bytes, before_commit)?;
        // 锁文件句柄在替换和返回期间保持；键不参与路径，也不覆盖bindings。
        Ok(digest(&bytes))
    }
}

#[cfg(test)]
#[path = "action_state_store_tests.rs"]
mod tests;
