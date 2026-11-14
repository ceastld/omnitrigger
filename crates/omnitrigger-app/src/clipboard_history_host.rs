//! 历史动作的独立宿主。只使用应用传入的存储，不捕获前台、读系统剪贴板或启动监听。
use crate::clipboard_history_store::{
    AppendResult, ClipboardHistoryStore, HistoryError, HistoryItem, HistoryList, RemoveResult,
    MAX_ITEMS, MAX_TEXT_UNITS, MAX_TOTAL_UNITS,
};
use omnitrigger_actions::{ActionError, CancellationToken, NativeHost};
use serde::Deserialize;
use serde_json::Value;
use std::{fmt, sync::Arc, time::Instant};

type StoreResult<T> = Result<T, HistoryError>;
type Check<'a> = &'a mut dyn FnMut() -> StoreResult<()>;

/// 用于合成测试和应用组合；真实实现直接复用 ClipboardHistoryStore 的检查与提交边界。
pub trait HistoryBackend: Send + Sync {
    fn list(&self, limit: usize, check: Check<'_>) -> StoreResult<HistoryList>;
    fn get(&self, id: &str, check: Check<'_>) -> StoreResult<HistoryItem>;
    fn append(&self, text: &str, check: Check<'_>) -> StoreResult<AppendResult>;
    fn remove(&self, id: &str, check: Check<'_>) -> StoreResult<RemoveResult>;
}

impl HistoryBackend for ClipboardHistoryStore {
    fn list(&self, limit: usize, check: Check<'_>) -> StoreResult<HistoryList> {
        ClipboardHistoryStore::list(self, limit, check)
    }
    fn get(&self, id: &str, check: Check<'_>) -> StoreResult<HistoryItem> {
        ClipboardHistoryStore::get(self, id, check)
    }
    fn append(&self, text: &str, check: Check<'_>) -> StoreResult<AppendResult> {
        ClipboardHistoryStore::append(self, text, check)
    }
    fn remove(&self, id: &str, check: Check<'_>) -> StoreResult<RemoveResult> {
        ClipboardHistoryStore::remove(self, id, check)
    }
}

impl<T: HistoryBackend + ?Sized> HistoryBackend for Arc<T> {
    fn list(&self, limit: usize, check: Check<'_>) -> StoreResult<HistoryList> {
        self.as_ref().list(limit, check)
    }
    fn get(&self, id: &str, check: Check<'_>) -> StoreResult<HistoryItem> {
        self.as_ref().get(id, check)
    }
    fn append(&self, text: &str, check: Check<'_>) -> StoreResult<AppendResult> {
        self.as_ref().append(text, check)
    }
    fn remove(&self, id: &str, check: Check<'_>) -> StoreResult<RemoveResult> {
        self.as_ref().remove(id, check)
    }
}

pub fn is_history_method(method: &str) -> bool {
    matches!(
        method,
        "history.list" | "history.get" | "history.append" | "history.remove"
    )
}

pub struct ClipboardHistoryHost<B = ClipboardHistoryStore> {
    backend: B,
}

impl<B> ClipboardHistoryHost<B> {
    /// 构造不读写存储；路径与共享生命周期由应用组合根明确提供。
    pub fn new(backend: B) -> Self {
        Self { backend }
    }
}

impl<B> fmt::Debug for ClipboardHistoryHost<B> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 不格式化 backend：其 Debug 可能包含私有路径或文本。
        formatter.write_str("ClipboardHistoryHost { .. }")
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListArgs {
    #[serde(default = "default_limit")]
    limit: usize,
}
fn default_limit() -> usize {
    100
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdArgs {
    id: String,
}

// 正文 DTO 不派生 Debug，避免解析/诊断路径把正文写入日志。
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AppendArgs {
    text: String,
}

enum Request {
    List(ListArgs),
    Get(IdArgs),
    Append(AppendArgs),
    Remove(IdArgs),
}

fn valid_id(value: &str, allow_zero: bool) -> bool {
    !value.is_empty()
        && value.len() <= 20
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && (value.len() == 1 || !value.starts_with('0'))
        && value
            .parse::<u64>()
            .is_ok_and(|number| allow_zero || number != 0)
}

fn valid_text(text: &str) -> bool {
    !text.contains('\0') && text.encode_utf16().take(MAX_TEXT_UNITS + 1).count() <= MAX_TEXT_UNITS
}

fn parse(method: &str, params: Value) -> Result<Request, ActionError> {
    // 必须先验参，再访问取消/目标/存储；serde 结构体的序列输入不属于此 wire 契约。
    let known_method = is_history_method(method).then_some(method);
    let invalid = || host_error(known_method, "invalid_input", "not_started");
    if !params.is_object() {
        return Err(invalid());
    }
    match method {
        "history.list" => {
            let args: ListArgs = serde_json::from_value(params).map_err(|_| invalid())?;
            if !(1..=MAX_ITEMS).contains(&args.limit) {
                return Err(invalid());
            }
            Ok(Request::List(args))
        }
        "history.get" | "history.remove" => {
            let args: IdArgs = serde_json::from_value(params).map_err(|_| invalid())?;
            if !valid_id(&args.id, false) {
                return Err(invalid());
            }
            Ok(if method == "history.get" {
                Request::Get(args)
            } else {
                Request::Remove(args)
            })
        }
        "history.append" => {
            let args: AppendArgs = serde_json::from_value(params).map_err(|_| invalid())?;
            if !valid_text(&args.text) {
                return Err(invalid());
            }
            Ok(Request::Append(args))
        }
        _ => Err(invalid()),
    }
}

fn code(code: &str) -> &'static str {
    match code {
        "invalid_input" => "invalid_input",
        "invalid_response" => "invalid_response",
        "history_unavailable" => "history_unavailable",
        "history_item_missing" => "history_item_missing",
        "history_busy" => "history_busy",
        "history_corrupt" => "history_corrupt",
        "history_conflict" => "history_conflict",
        "history_io" => "history_io",
        "history_id_exhausted" => "history_id_exhausted",
        "history_revision_exhausted" => "history_revision_exhausted",
        "cancelled" => "cancelled",
        "timeout" => "timeout",
        _ => "history_failed",
    }
}

fn host_error(method: Option<&str>, reported_code: &str, effect: &str) -> ActionError {
    let mut error = ActionError::new(code(reported_code), "剪贴板历史调用失败；请检查错误代码。");
    error.method = method
        .filter(|method| is_history_method(method))
        .map(str::to_string);
    let write = matches!(method, Some("history.append" | "history.remove"));
    error.effect_state = Some(
        if write && effect != "not_started" {
            "unknown"
        } else {
            "not_started"
        }
        .into(),
    );
    // 不拷贝底层 message、路径、权限字段或序列化错误。
    error
}

fn check(cancellation: &CancellationToken, deadline: Option<Instant>) -> StoreResult<()> {
    if cancellation.is_cancelled() {
        return Err(HistoryError::new("cancelled"));
    }
    if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
        return Err(HistoryError::new("timeout"));
    }
    Ok(())
}

fn valid_list(list: &HistoryList, limit: usize) -> bool {
    let mut ids = std::collections::BTreeSet::new();
    let mut total = 0usize;
    valid_id(&list.revision, true)
        && list.items.len() <= limit
        && list.items.iter().all(|item| {
            total = total.saturating_add(item.utf16_units);
            valid_id(&item.id, false)
                && ids.insert(item.id.as_str())
                && item.utf16_units <= MAX_TEXT_UNITS
                && total <= MAX_TOTAL_UNITS
                && item.preview.encode_utf16().count() <= 64
                && item.preview.encode_utf16().count() <= item.utf16_units
                && !item.preview.chars().any(|character| {
                    matches!(
                        character,
                        '\0' | '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{85}' | '\u{2028}' | '\u{2029}'
                    )
                })
                && !item.created_at.contains('\0')
        })
}

impl<B: HistoryBackend> ClipboardHistoryHost<B> {
    fn execute(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
        deadline: Option<Instant>,
    ) -> Result<Value, ActionError> {
        let request = parse(method, params)?;
        let map = |error: HistoryError| host_error(Some(method), error.code, error.effect_state);
        let invalid = || host_error(Some(method), "invalid_response", "unknown");
        let mut guard = || check(cancellation, deadline);
        guard().map_err(map)?;
        let (result, changed) = match request {
            Request::List(args) => {
                let result = self.backend.list(args.limit, &mut guard).map_err(map)?;
                if !valid_list(&result, args.limit) {
                    return Err(invalid());
                }
                (serde_json::to_value(result), false)
            }
            Request::Get(args) => {
                let result = self.backend.get(&args.id, &mut guard).map_err(map)?;
                if result.id != args.id
                    || !valid_id(&result.id, false)
                    || !valid_id(&result.revision, true)
                    || !valid_text(&result.text)
                    || result.created_at.contains('\0')
                {
                    return Err(invalid());
                }
                (serde_json::to_value(result), false)
            }
            Request::Append(args) => {
                let result = self.backend.append(&args.text, &mut guard).map_err(map)?;
                if !valid_id(&result.id, false) || !valid_id(&result.revision, true) {
                    return Err(invalid());
                }
                let changed = result.inserted;
                (serde_json::to_value(result), changed)
            }
            Request::Remove(args) => {
                let result = self.backend.remove(&args.id, &mut guard).map_err(map)?;
                if !valid_id(&result.revision, true) {
                    return Err(invalid());
                }
                let changed = result.removed;
                (serde_json::to_value(result), changed)
            }
        };
        let effect = if changed { "unknown" } else { "not_started" };
        let value = result.map_err(|_| host_error(Some(method), "history_failed", effect))?;
        // 复验编码结束前发生的取消；无变化回执不能被标记成已变更，提交后不回滚。
        guard().map_err(|error| host_error(Some(method), error.code, effect))?;
        Ok(value)
    }
}

impl<B: HistoryBackend> NativeHost for ClipboardHistoryHost<B> {
    fn call(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
    ) -> Result<Value, ActionError> {
        self.execute(method, params, cancellation, None)
    }
    fn call_with_deadline(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<Value, ActionError> {
        self.execute(method, params, cancellation, Some(deadline))
    }
}

/// 在既有宿主外显式增加历史服务；缺少 store 时仍严格验参并明确不可用。
/// 其他方法原样委托，包括无 deadline 的 call，不为它们悄悄增加新时限。
pub struct WithClipboardHistory<H, B = ClipboardHistoryStore> {
    inner: H,
    history: Option<ClipboardHistoryHost<B>>,
}
impl<H, B> WithClipboardHistory<H, B> {
    pub fn new(inner: H, backend: Option<B>) -> Self {
        Self {
            inner,
            history: backend.map(ClipboardHistoryHost::new),
        }
    }
}
impl<H: NativeHost, B: HistoryBackend> WithClipboardHistory<H, B> {
    fn history_call(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
        deadline: Option<Instant>,
    ) -> Result<Value, ActionError> {
        if let Some(history) = &self.history {
            return history.execute(method, params, cancellation, deadline);
        }
        parse(method, params)?;
        check(cancellation, deadline)
            .map_err(|error| host_error(Some(method), error.code, "not_started"))?;
        Err(host_error(
            Some(method),
            "history_unavailable",
            "not_started",
        ))
    }
}
impl<H: NativeHost, B: HistoryBackend> NativeHost for WithClipboardHistory<H, B> {
    fn call(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
    ) -> Result<Value, ActionError> {
        if is_history_method(method) {
            self.history_call(method, params, cancellation, None)
        } else {
            self.inner.call(method, params, cancellation)
        }
    }
    fn call_with_deadline(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<Value, ActionError> {
        if is_history_method(method) {
            self.history_call(method, params, cancellation, Some(deadline))
        } else {
            self.inner
                .call_with_deadline(method, params, cancellation, deadline)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard_history_store::HistorySummary;
    use serde_json::json;
    use std::sync::Mutex;

    const TEXT: &str = "synthetic_private_history_body_合成😀";
    #[derive(Clone, Copy)]
    enum Mode {
        Normal,
        Fail(&'static str, &'static str),
        CancelBefore,
        CancelReturned(bool),
        BadGet,
        BadList,
    }
    struct Fake {
        mode: Mode,
        token: CancellationToken,
        calls: Mutex<Vec<&'static str>>,
        units: Mutex<Vec<usize>>,
    }
    impl Fake {
        fn enter(&self, method: &'static str, check: Check<'_>) -> StoreResult<()> {
            self.calls.lock().unwrap().push(method);
            check()?;
            if matches!(self.mode, Mode::CancelBefore) {
                self.token.cancel();
                check()?;
            }
            if let Mode::Fail(code, effect) = self.mode {
                return Err(HistoryError {
                    code,
                    message: TEXT,
                    effect_state: effect,
                });
            }
            Ok(())
        }
    }
    impl HistoryBackend for Fake {
        fn list(&self, limit: usize, check: Check<'_>) -> StoreResult<HistoryList> {
            self.enter("history.list", check)?;
            self.units.lock().unwrap().push(limit);
            Ok(HistoryList {
                items: vec![HistorySummary {
                    id: "1".into(),
                    preview: if matches!(self.mode, Mode::BadList) {
                        TEXT.repeat(8)
                    } else {
                        "合成😀".into()
                    },
                    utf16_units: TEXT.encode_utf16().count(),
                    created_at: "0".into(),
                    text_sha256: [0; 32],
                }],
                revision: "1".into(),
            })
        }
        fn get(&self, id: &str, check: Check<'_>) -> StoreResult<HistoryItem> {
            self.enter("history.get", check)?;
            Ok(HistoryItem {
                id: if matches!(self.mode, Mode::BadGet) {
                    "2".into()
                } else {
                    id.into()
                },
                text: TEXT.into(),
                created_at: "0".into(),
                revision: "1".into(),
            })
        }
        fn append(&self, text: &str, check: Check<'_>) -> StoreResult<AppendResult> {
            self.enter("history.append", check)?;
            self.units.lock().unwrap().push(text.encode_utf16().count());
            let inserted = if let Mode::CancelReturned(changed) = self.mode {
                self.token.cancel();
                changed
            } else {
                true
            };
            Ok(AppendResult {
                id: "9007199254740993".into(),
                revision: "1".into(),
                inserted,
            })
        }
        fn remove(&self, _: &str, check: Check<'_>) -> StoreResult<RemoveResult> {
            self.enter("history.remove", check)?;
            Ok(RemoveResult {
                removed: false,
                revision: "1".into(),
            })
        }
    }
    fn fixture(
        mode: Mode,
    ) -> (
        ClipboardHistoryHost<Arc<Fake>>,
        Arc<Fake>,
        CancellationToken,
    ) {
        let token = CancellationToken::new();
        let backend = Arc::new(Fake {
            mode,
            token: token.clone(),
            calls: Mutex::default(),
            units: Mutex::default(),
        });
        (ClipboardHistoryHost::new(backend.clone()), backend, token)
    }
    fn assert_error(error: ActionError, method: &str, code: &str, effect: &str) {
        assert_eq!(error.code, code);
        assert_eq!(error.method.as_deref(), Some(method));
        assert_eq!(error.effect_state.as_deref(), Some(effect));
        assert!(error.required_permissions.is_empty());
        assert!(!format!("{error:?}").contains(TEXT));
        assert!(!serde_json::to_string(&error).unwrap().contains(TEXT));
    }

    #[test]
    fn invalid_dtos_never_reach_backend_and_have_no_body_in_errors() {
        let (host, backend, token) = fixture(Mode::Normal);
        for (method, values) in [
            (
                "history.list",
                vec![
                    json!(null),
                    json!([]),
                    json!({"limit":null}),
                    json!({"limit":0}),
                    json!({"limit":201}),
                    json!({"limit":1.5}),
                    json!({"path":TEXT}),
                ],
            ),
            (
                "history.get",
                vec![
                    json!({}),
                    json!({"id":1}),
                    json!({"id":"0"}),
                    json!({"id":"01"}),
                    json!({"id":"18446744073709551616"}),
                    json!({"id":"1","text":TEXT}),
                ],
            ),
            (
                "history.append",
                vec![
                    json!({}),
                    json!({"text":null}),
                    json!({"text":"a\0b"}),
                    json!({"text":"😀".repeat(32769)}),
                    json!({"text":TEXT,"path":TEXT}),
                ],
            ),
            (
                "history.remove",
                vec![
                    json!([]),
                    json!({"id":"-1"}),
                    json!({"id":"1","extra":true}),
                ],
            ),
        ] {
            for params in values {
                assert_error(
                    host.call(method, params, &token).unwrap_err(),
                    method,
                    "invalid_input",
                    "not_started",
                );
            }
        }
        assert!(backend.calls.lock().unwrap().is_empty());
        let error = host.call(TEXT, json!({}), &token).unwrap_err();
        assert!(error.method.is_none());
        assert!(!serde_json::to_string(&error).unwrap().contains(TEXT));
    }

    #[test]
    fn all_routes_preserve_schema_and_text_is_only_in_get() {
        let (host, backend, token) = fixture(Mode::Normal);
        let list = host.call("history.list", json!({}), &token).unwrap();
        assert_eq!(backend.units.lock().unwrap()[0], 100);
        assert!(list["items"][0].get("text").is_none());
        assert_eq!(list["items"][0]["createdAt"], "0");
        let get = host.call("history.get", json!({"id":"1"}), &token).unwrap();
        assert_eq!(get["text"], TEXT);
        for text in [String::new(), " \r\n ".into(), "😀".repeat(32768)] {
            let append = host
                .call("history.append", json!({"text":text}), &token)
                .unwrap();
            assert_eq!(append["id"], "9007199254740993");
            assert!(append.get("text").is_none());
        }
        let remove = host
            .call(
                "history.remove",
                json!({"id":"18446744073709551615"}),
                &token,
            )
            .unwrap();
        assert_eq!(remove, json!({"removed":false,"revision":"1"}));
        assert!(!format!("{host:?}").contains(TEXT));
    }

    #[test]
    fn cancellation_and_expired_deadline_stop_before_storage() {
        let (host, backend, token) = fixture(Mode::Normal);
        token.cancel();
        assert_error(
            host.call("history.append", json!({"text":TEXT}), &token)
                .unwrap_err(),
            "history.append",
            "cancelled",
            "not_started",
        );
        let token = CancellationToken::new();
        assert_error(
            host.call_with_deadline("history.get", json!({"id":"1"}), &token, Instant::now())
                .unwrap_err(),
            "history.get",
            "timeout",
            "not_started",
        );
        assert!(backend.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn store_check_and_post_return_cancellation_keep_commit_stage() {
        for (mode, effect) in [
            (Mode::CancelBefore, "not_started"),
            (Mode::CancelReturned(false), "not_started"),
            (Mode::CancelReturned(true), "unknown"),
        ] {
            let (host, _, token) = fixture(mode);
            assert_error(
                host.call("history.append", json!({"text":TEXT}), &token)
                    .unwrap_err(),
                "history.append",
                "cancelled",
                effect,
            );
        }
        for effect in ["not_started", "unknown", "malformed"] {
            let (host, _, token) = fixture(Mode::Fail("history_conflict", effect));
            assert_error(
                host.call("history.append", json!({"text":TEXT}), &token)
                    .unwrap_err(),
                "history.append",
                "history_conflict",
                if effect == "not_started" {
                    "not_started"
                } else {
                    "unknown"
                },
            );
            assert_error(
                host.call("history.get", json!({"id":"1"}), &token)
                    .unwrap_err(),
                "history.get",
                "history_conflict",
                "not_started",
            );
        }
    }

    #[test]
    fn malformed_backend_data_and_error_codes_are_redacted() {
        for (mode, method, params) in [
            (Mode::BadGet, "history.get", json!({"id":"1"})),
            (Mode::BadList, "history.list", json!({})),
        ] {
            let (host, _, token) = fixture(mode);
            assert_error(
                host.call(method, params, &token).unwrap_err(),
                method,
                "invalid_response",
                "not_started",
            );
        }
        let (host, _, token) = fixture(Mode::Fail(TEXT, "unknown"));
        assert_error(
            host.call("history.remove", json!({"id":"1"}), &token)
                .unwrap_err(),
            "history.remove",
            "history_failed",
            "unknown",
        );
    }

    #[test]
    fn composition_routes_history_without_desktop_and_preserves_original_call_deadlines() {
        type Calls = Arc<Mutex<Vec<(String, Option<Instant>)>>>;
        struct Original(Calls);
        impl NativeHost for Original {
            fn call(
                &self,
                method: &str,
                _: Value,
                _: &CancellationToken,
            ) -> Result<Value, ActionError> {
                self.0.lock().unwrap().push((method.into(), None));
                Ok(Value::Null)
            }
            fn call_with_deadline(
                &self,
                method: &str,
                _: Value,
                _: &CancellationToken,
                deadline: Instant,
            ) -> Result<Value, ActionError> {
                self.0.lock().unwrap().push((method.into(), Some(deadline)));
                Ok(Value::Null)
            }
        }
        let (_, backend, token) = fixture(Mode::Normal);
        let calls: Calls = Arc::default();
        let host = WithClipboardHistory::new(Original(calls.clone()), Some(backend));
        host.call("history.list", json!({}), &token).unwrap();
        assert!(calls.lock().unwrap().is_empty());
        let deadline = Instant::now();
        host.call("target.info", json!({}), &token).unwrap();
        host.call_with_deadline("ui.choose", json!({}), &token, deadline)
            .unwrap();
        assert_eq!(
            *calls.lock().unwrap(),
            vec![
                ("target.info".into(), None),
                ("ui.choose".into(), Some(deadline))
            ]
        );
        let missing =
            WithClipboardHistory::<_, ClipboardHistoryStore>::new(Original(calls.clone()), None);
        assert_error(
            missing
                .call("history.get", json!({"id":"1"}), &token)
                .unwrap_err(),
            "history.get",
            "history_unavailable",
            "not_started",
        );
        assert_error(
            missing
                .call("history.append", json!({"text":TEXT,"path":TEXT}), &token)
                .unwrap_err(),
            "history.append",
            "invalid_input",
            "not_started",
        );
        assert_eq!(calls.lock().unwrap().len(), 2);
    }
}
