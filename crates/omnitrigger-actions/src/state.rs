//! 当前真实动作的有界状态；条件、默认值、时间与业务语义均由脚本解释。
use crate::{model::valid_id, ActionError};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, MutexGuard, TryLockError},
    time::{Duration, Instant},
};

pub const MAX_STATE_KEYS: usize = 64;
pub const MAX_STATE_VALUE_BYTES: usize = 16 * 1024;
pub const MAX_STATE_DOCUMENT_BYTES: usize = 256 * 1024;
pub const MAX_STATE_DEPTH: usize = 8;
const MAX_NODES: usize = 2048;
const MAX_ACTIONS: usize = 128;
const LOCK_WAIT: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionStateScope {
    pub package_id: String,
    pub action_id: String,
}

/// 适配器必须有界读取并验证文件；version用于拒绝外部变化后的陈旧覆盖。
pub struct DurableStateSnapshot {
    pub values: BTreeMap<String, Value>,
    pub version: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StateStorageError {
    Unavailable,
    Busy,
    Corrupt,
    Changed,
    Io,
    Cancelled,
    Timeout,
    Limit,
}

pub trait DurableStateStorage: Send + Sync {
    fn load(&self, scope: &ActionStateScope) -> Result<DurableStateSnapshot, StateStorageError>;
    /// Err必须表示未提交；成功后不能再因取消伪报回滚。
    fn commit(
        &self,
        scope: &ActionStateScope,
        expected_version: &str,
        values: &BTreeMap<String, Value>,
        before_commit: &dyn Fn() -> Result<(), StateStorageError>,
    ) -> Result<String, StateStorageError>;
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Area {
    Session,
    Durable,
}
impl Area {
    fn name(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Durable => "durable",
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadRequest {
    area: Area,
    key: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CasRequest {
    area: Area,
    key: String,
    expected_revision: String,
    next: Value,
}

#[derive(Clone)]
struct Cell {
    // None是missing，Some(Value::Null)是显式null。删除保留修订，防止ABA。
    value: Option<Value>,
    revision: u64,
}
struct DurableCache {
    cells: BTreeMap<String, Cell>,
    version: String,
    epoch: u64,
}
#[derive(Default)]
struct ScopedState {
    session: BTreeMap<String, Cell>,
    durable: Option<DurableCache>,
    next: u64,
}

/// 每个应用所有者一个Arc；reload和每次Node/Script调用复用，默认服务不创建它。
pub struct ActionStateStore {
    generation: [u8; 32],
    actions: Mutex<BTreeMap<ActionStateScope, Arc<Mutex<ScopedState>>>>,
    durable: Option<Arc<dyn DurableStateStorage>>,
}

pub(crate) fn fault(code: &str, method: &str) -> ActionError {
    let mut error = ActionError::new(code, "动作状态操作未提交；请检查错误代码，不会自动重试。");
    error.method = Some(method.into());
    error.effect_state = Some("not_started".into());
    error
}
fn storage_fault(error: StateStorageError, method: &str) -> ActionError {
    fault(
        match error {
            StateStorageError::Unavailable => "state_unavailable",
            StateStorageError::Busy => "state_busy",
            StateStorageError::Corrupt => "state_corrupt",
            StateStorageError::Changed => "state_storage_changed",
            StateStorageError::Io => "state_io",
            StateStorageError::Cancelled => "cancelled",
            StateStorageError::Timeout => "timeout",
            StateStorageError::Limit => "state_limit",
        },
        method,
    )
}
pub fn valid_state_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= 64 && !key.chars().any(char::is_control)
}
pub fn valid_state_value(value: &Value) -> bool {
    fn visit(value: &Value, depth: usize, remaining: &mut usize) -> bool {
        if depth > MAX_STATE_DEPTH || *remaining == 0 {
            return false;
        }
        *remaining -= 1;
        match value {
            Value::Number(n) => n.as_f64().is_some_and(|n| {
                n.is_finite() && (n.fract() != 0.0 || n.abs() <= 9_007_199_254_740_991.0)
            }),
            Value::Array(items) => items.iter().all(|v| visit(v, depth + 1, remaining)),
            Value::Object(items) => items.values().all(|v| visit(v, depth + 1, remaining)),
            _ => true,
        }
    }
    let mut remaining = MAX_NODES;
    visit(value, 0, &mut remaining)
        && serde_json::to_vec(value).is_ok_and(|v| v.len() <= MAX_STATE_VALUE_BYTES)
}
pub fn valid_state_values(values: &BTreeMap<String, Value>) -> bool {
    values.len() <= MAX_STATE_KEYS
        && values
            .iter()
            .all(|(k, v)| valid_state_key(k) && valid_state_value(v))
        && serde_json::to_vec(values).is_ok_and(|v| v.len() <= MAX_STATE_DOCUMENT_BYTES)
}
fn next_entry(value: &Value) -> Option<Option<Value>> {
    let fields = value.as_object()?;
    match fields.get("present")?.as_bool()? {
        false if fields.len() == 1 => Some(None),
        true if fields.len() == 2 => fields
            .get("value")
            .filter(|v| valid_state_value(v))
            .cloned()
            .map(Some),
        _ => None,
    }
}
fn check(cancelled: &dyn Fn() -> bool, deadline: Instant, method: &str) -> Result<(), ActionError> {
    if cancelled() {
        Err(fault("cancelled", method))
    } else if Instant::now() >= deadline {
        Err(fault("timeout", method))
    } else {
        Ok(())
    }
}

impl ActionStateStore {
    pub fn new(durable: Option<Arc<dyn DurableStateStorage>>) -> Result<Self, StateStorageError> {
        let mut generation = [0; 32];
        getrandom::fill(&mut generation).map_err(|_| StateStorageError::Unavailable)?;
        Ok(Self {
            generation,
            actions: Mutex::new(BTreeMap::new()),
            durable,
        })
    }

    fn lock<'a>(
        mutex: &'a Mutex<ScopedState>,
        cancelled: &dyn Fn() -> bool,
        deadline: Instant,
        method: &str,
    ) -> Result<MutexGuard<'a, ScopedState>, ActionError> {
        let waiting = Instant::now() + LOCK_WAIT;
        loop {
            check(cancelled, deadline, method)?;
            match mutex.try_lock() {
                Ok(guard) => return Ok(guard),
                Err(TryLockError::Poisoned(_)) => return Err(fault("state_unavailable", method)),
                Err(TryLockError::WouldBlock) if Instant::now() >= waiting => {
                    return Err(fault("state_busy", method));
                }
                Err(TryLockError::WouldBlock) => std::thread::sleep(Duration::from_millis(1)),
            }
        }
    }

    fn snapshot(
        &self,
        scope: &ActionStateScope,
        area: Area,
        epoch: u64,
        key: &str,
        cell: Option<&Cell>,
    ) -> Value {
        let mut hash = Sha256::new();
        hash.update(self.generation);
        for part in [
            scope.package_id.as_str(),
            scope.action_id.as_str(),
            area.name(),
            key,
        ] {
            hash.update((part.len() as u64).to_le_bytes());
            hash.update(part.as_bytes());
        }
        hash.update(epoch.to_le_bytes());
        hash.update(cell.map_or(0, |v| v.revision).to_le_bytes());
        let revision = format!("{:x}", hash.finalize());
        let entry = match cell.and_then(|v| v.value.as_ref()) {
            Some(value) => json!({"present":true,"value":value}),
            None => json!({"present":false}),
        };
        json!({"revision":revision,"entry":entry})
    }

    pub(crate) fn call(
        &self,
        scope: ActionStateScope,
        method: &str,
        params: Value,
        cancelled: &dyn Fn() -> bool,
        deadline: Instant,
    ) -> Result<Value, ActionError> {
        let invalid = || fault("invalid_input", method);
        if !valid_id(&scope.package_id) || !valid_id(&scope.action_id) {
            return Err(invalid());
        }
        let (area, key, proposal) = match method {
            "state.read" => {
                let r: ReadRequest = serde_json::from_value(params).map_err(|_| invalid())?;
                (r.area, r.key, None)
            }
            "state.compare_and_swap" => {
                let r: CasRequest = serde_json::from_value(params).map_err(|_| invalid())?;
                if r.expected_revision.len() != 64
                    || !r
                        .expected_revision
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                {
                    return Err(invalid());
                }
                let next = next_entry(&r.next).ok_or_else(invalid)?;
                (r.area, r.key, Some((r.expected_revision, next)))
            }
            _ => return Err(invalid()),
        };
        if !valid_state_key(&key) {
            return Err(invalid());
        }
        check(cancelled, deadline, method)?;
        let action = {
            let mut actions = self
                .actions
                .lock()
                .map_err(|_| fault("state_unavailable", method))?;
            if !actions.contains_key(&scope) && actions.len() >= MAX_ACTIONS {
                return Err(fault("state_limit", method));
            }
            actions.entry(scope.clone()).or_default().clone()
        };
        let mut state = Self::lock(&action, cancelled, deadline, method)?;
        if matches!(area, Area::Durable) && state.durable.is_none() {
            let storage = self
                .durable
                .as_ref()
                .ok_or_else(|| fault("state_unavailable", method))?;
            let loaded = storage.load(&scope).map_err(|e| storage_fault(e, method))?;
            if !valid_state_values(&loaded.values)
                || loaded.version.is_empty()
                || loaded.version.len() > 128
            {
                return Err(fault("state_corrupt", method));
            }
            state.next = state
                .next
                .checked_add(1)
                .ok_or_else(|| fault("state_limit", method))?;
            state.durable = Some(DurableCache {
                cells: loaded
                    .values
                    .into_iter()
                    .map(|(k, value)| {
                        (
                            k,
                            Cell {
                                value: Some(value),
                                revision: 0,
                            },
                        )
                    })
                    .collect(),
                version: loaded.version,
                epoch: state.next,
            });
        }
        check(cancelled, deadline, method)?;
        let (cells, epoch) = match area {
            Area::Session => (&state.session, 0),
            Area::Durable => {
                let durable = state.durable.as_ref().expect("已加载持久状态");
                (&durable.cells, durable.epoch)
            }
        };
        let current = self.snapshot(&scope, area, epoch, &key, cells.get(&key));
        let Some((expected, next)) = proposal else {
            return Ok(current);
        };
        if current["revision"].as_str() != Some(expected.as_str()) {
            return Ok(json!({"status":"conflict","current":current}));
        }
        if !cells.contains_key(&key) && cells.len() >= MAX_STATE_KEYS {
            return Err(fault("state_limit", method));
        }
        let revision = state
            .next
            .checked_add(1)
            .ok_or_else(|| fault("state_limit", method))?;
        let mut changed = cells.clone();
        changed.insert(
            key.clone(),
            Cell {
                value: next,
                revision,
            },
        );
        let values: BTreeMap<_, _> = changed
            .iter()
            .filter_map(|(k, c)| c.value.clone().map(|v| (k.clone(), v)))
            .collect();
        if !valid_state_values(&values) {
            return Err(fault("state_limit", method));
        }
        check(cancelled, deadline, method)?;
        match area {
            Area::Session => state.session = changed,
            Area::Durable => {
                let durable = state.durable.as_ref().expect("已加载持久状态");
                let storage = self.durable.as_ref().expect("已验证持久适配器");
                let version = match storage.commit(&scope, &durable.version, &values, &|| {
                    if cancelled() {
                        Err(StateStorageError::Cancelled)
                    } else if Instant::now() >= deadline {
                        Err(StateStorageError::Timeout)
                    } else {
                        Ok(())
                    }
                }) {
                    Ok(version) => version,
                    Err(error) => {
                        if error == StateStorageError::Changed {
                            state.durable = None;
                        }
                        return Err(storage_fault(error, method));
                    }
                };
                state.durable = Some(DurableCache {
                    cells: changed,
                    version,
                    epoch,
                });
            }
        }
        // 提交点之后不再次检查取消/时限，不能把已提交状态声称为未发生。
        state.next = revision;
        let cell = match area {
            Area::Session => state.session.get(&key),
            Area::Durable => state.durable.as_ref().and_then(|s| s.cells.get(&key)),
        };
        Ok(json!({"status":"committed","current":self.snapshot(&scope, area, epoch, &key, cell)}))
    }
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
