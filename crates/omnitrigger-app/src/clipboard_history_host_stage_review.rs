//! 真实Store与公开Host的阶段回归。
use super::{history_bytes, Fixture};
use crate::clipboard_history_host::{ClipboardHistoryHost, HistoryBackend};
use crate::clipboard_history_store::{
    AppendResult, ClipboardHistoryStore, HistoryError, HistoryItem, HistoryList, RemoveResult,
};
use omnitrigger_actions::{ActionError, CancellationToken, NativeHost};
use serde_json::{json, Value};
use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

type Check<'a> = &'a mut dyn FnMut() -> Result<(), HistoryError>;
#[derive(Clone, Copy)]
enum Fault {
    None,
    Before(&'static str),
    After(&'static str),
    CancelOnReturn,
}
struct Backend {
    real: Arc<ClipboardHistoryStore>,
    path: PathBuf,
    token: CancellationToken,
    fault: Fault,
    calls: Arc<AtomicUsize>,
}
impl Backend {
    fn guard(
        &self,
        check: Check<'_>,
        original: &Option<Vec<u8>>,
        write: bool,
    ) -> Result<(), HistoryError> {
        check()?;
        match self.fault {
            Fault::Before(code) => Err(HistoryError::new(code)),
            Fault::After(code) if write && &history_bytes(&self.path) != original => {
                Err(HistoryError::new(code))
            }
            _ => Ok(()),
        }
    }
    fn returned<T>(&self, value: T) -> T {
        if matches!(self.fault, Fault::CancelOnReturn) {
            self.token.cancel();
        }
        value
    }
    fn entered(&self) -> Option<Vec<u8>> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        history_bytes(&self.path)
    }
}
impl HistoryBackend for Backend {
    fn list(&self, limit: usize, check: Check<'_>) -> Result<HistoryList, HistoryError> {
        let original = self.entered();
        self.real
            .as_ref()
            .list(limit, || self.guard(check, &original, false))
            .map(|value| self.returned(value))
    }
    fn get(&self, id: &str, check: Check<'_>) -> Result<HistoryItem, HistoryError> {
        let original = self.entered();
        self.real
            .as_ref()
            .get(id, || self.guard(check, &original, false))
            .map(|value| self.returned(value))
    }
    fn append(&self, text: &str, check: Check<'_>) -> Result<AppendResult, HistoryError> {
        let original = self.entered();
        self.real
            .as_ref()
            .append_at(text, 0, || self.guard(check, &original, true))
            .map(|value| self.returned(value))
    }
    fn remove(&self, id: &str, check: Check<'_>) -> Result<RemoveResult, HistoryError> {
        let original = self.entered();
        self.real
            .as_ref()
            .remove(id, || self.guard(check, &original, true))
            .map(|value| self.returned(value))
    }
}
fn host(
    fixture: &Fixture,
    token: &CancellationToken,
    fault: Fault,
) -> (ClipboardHistoryHost<Backend>, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    (
        ClipboardHistoryHost::new(Backend {
            real: fixture.store.clone(),
            path: fixture.path.clone(),
            token: token.clone(),
            fault,
            calls: calls.clone(),
        }),
        calls,
    )
}
fn call(
    host: &impl NativeHost,
    method: &str,
    params: Value,
    token: &CancellationToken,
) -> Result<Value, ActionError> {
    host.call_with_deadline(
        method,
        params,
        token,
        Instant::now() + Duration::from_secs(3),
    )
}
fn assert_error(error: &ActionError, code: &str, method: &str, effect: &str) {
    assert_eq!(error.code, code);
    assert_eq!(error.method.as_deref(), Some(method));
    assert_eq!(error.effect_state.as_deref(), Some(effect));
    assert!(!error.message.contains("合成阶段私密"));
}
fn seed(fixture: &Fixture) {
    fixture
        .store
        .append_at("合成阶段私密", 1, || Ok(()))
        .unwrap();
}

#[test]
fn invalid_wire_input_is_rejected_before_cancellation_or_backend_access() {
    let fixture = Fixture::new();
    let token = CancellationToken::new();
    token.cancel();
    let (host, calls) = host(&fixture, &token, Fault::None);
    let error = call(
        &host,
        "history.append",
        json!({"text":"合成阶段私密\0"}),
        &token,
    )
    .unwrap_err();
    assert_error(&error, "invalid_input", "history.append", "not_started");
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    assert!(!fixture.body_exists());
}

#[test]
fn pre_cancelled_or_expired_host_never_calls_real_store() {
    for cancelled in [true, false] {
        let fixture = Fixture::new();
        let token = CancellationToken::new();
        if cancelled {
            token.cancel();
        }
        let (host, calls) = host(&fixture, &token, Fault::None);
        let deadline = if cancelled {
            Instant::now() + Duration::from_secs(3)
        } else {
            Instant::now()
        };
        let error = host
            .call_with_deadline(
                "history.append",
                json!({"text":"合成阶段私密"}),
                &token,
                deadline,
            )
            .unwrap_err();
        assert_error(
            &error,
            if cancelled { "cancelled" } else { "timeout" },
            "history.append",
            "not_started",
        );
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        assert!(!fixture.body_exists());
    }
}

#[test]
fn real_pre_commit_cancel_or_timeout_keeps_not_started_without_retry() {
    for code in ["cancelled", "timeout"] {
        let fixture = Fixture::new();
        let token = CancellationToken::new();
        let (host, calls) = host(&fixture, &token, Fault::Before(code));
        let error = call(
            &host,
            "history.append",
            json!({"text":"合成阶段私密"}),
            &token,
        )
        .unwrap_err();
        assert_error(&error, code, "history.append", "not_started");
        assert!(!fixture.body_exists());
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }
}

#[test]
fn real_post_commit_cancel_or_timeout_keeps_unknown_and_one_persisted_id() {
    for code in ["cancelled", "timeout"] {
        let fixture = Fixture::new();
        let token = CancellationToken::new();
        let (host, calls) = host(&fixture, &token, Fault::After(code));
        let error = call(
            &host,
            "history.append",
            json!({"text":"合成阶段私密"}),
            &token,
        )
        .unwrap_err();
        assert_error(&error, code, "history.append", "unknown");
        assert_eq!(
            fixture.rows_oldest_first(),
            [("1".to_owned(), "合成阶段私密".to_owned())]
        );
        assert_eq!(fixture.revision(), "1");
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }
}

#[test]
fn real_remove_post_commit_timeout_is_unknown_without_rollback_or_retry() {
    let fixture = Fixture::new();
    seed(&fixture);
    fixture.store.append_at("合成第二项", 2, || Ok(())).unwrap();
    let token = CancellationToken::new();
    let (host, calls) = host(&fixture, &token, Fault::After("timeout"));
    let error = call(&host, "history.remove", json!({"id":"1"}), &token).unwrap_err();
    assert_error(&error, "timeout", "history.remove", "unknown");
    assert_eq!(
        fixture.rows_oldest_first(),
        [("2".to_owned(), "合成第二项".to_owned())]
    );
    assert_eq!(fixture.revision(), "3");
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[test]
fn cancellation_after_backend_return_distinguishes_real_noop_from_real_mutation() {
    let cases = [
        (
            "history.append",
            json!({"text":"合成阶段私密"}),
            "not_started",
            "1",
        ),
        ("history.remove", json!({"id":"999"}), "not_started", "1"),
        (
            "history.append",
            json!({"text":"合成第二项"}),
            "unknown",
            "2",
        ),
        ("history.remove", json!({"id":"1"}), "unknown", "2"),
    ];
    for (method, params, effect, revision) in cases {
        let fixture = Fixture::new();
        seed(&fixture);
        let original = history_bytes(&fixture.path);
        let token = CancellationToken::new();
        let (host, calls) = host(&fixture, &token, Fault::CancelOnReturn);
        let error = call(&host, method, params, &token).unwrap_err();
        assert_error(&error, "cancelled", method, effect);
        assert_eq!(fixture.revision(), revision);
        assert_eq!(
            history_bytes(&fixture.path) == original,
            effect == "not_started"
        );
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }
}

#[test]
fn real_corruption_and_counter_exhaustion_keep_known_not_started_and_original_bytes() {
    let documents = [
        (b"synthetic invalid JSON".to_vec(), "history_corrupt"),
        (serde_json::to_vec(&json!({"version":1,"lastId":"18446744073709551615","revision":"18446744073709551615","items":[]})).unwrap(), "history_id_exhausted"),
        (serde_json::to_vec(&json!({"version":1,"lastId":"0","revision":"18446744073709551615","items":[]})).unwrap(), "history_revision_exhausted"),
    ];
    for (document, code) in documents {
        let fixture = Fixture::new();
        crate::script_runtime::check_local_path(&fixture.path).unwrap();
        fs::write(&fixture.path, &document).unwrap();
        let token = CancellationToken::new();
        let (host, calls) = host(&fixture, &token, Fault::None);
        let error = call(
            &host,
            "history.append",
            json!({"text":"合成阶段私密"}),
            &token,
        )
        .unwrap_err();
        assert_error(&error, code, "history.append", "not_started");
        assert_eq!(history_bytes(&fixture.path).unwrap(), document);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }
}

#[test]
fn real_sidecar_contention_keeps_not_started_and_does_not_retry_or_consume_an_id() {
    let fixture = Fixture::new();
    seed(&fixture);
    let path = fixture.path.with_file_name("history.json.lock");
    crate::script_runtime::check_local_path(&path).unwrap();
    let held = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    held.try_lock().unwrap();
    let original = history_bytes(&fixture.path);
    let token = CancellationToken::new();
    let (host, calls) = host(&fixture, &token, Fault::None);
    let error = call(
        &host,
        "history.append",
        json!({"text":"合成第二项"}),
        &token,
    )
    .unwrap_err();
    assert_error(&error, "history_busy", "history.append", "not_started");
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(history_bytes(&fixture.path), original);
    drop(held);
    let next = fixture.store.append_at("合成第二项", 2, || Ok(())).unwrap();
    assert_eq!((next.id.as_str(), next.revision.as_str()), ("2", "2"));
}

#[test]
fn cancellation_after_read_backend_return_stays_not_started_and_redacts_full_text() {
    for (method, params) in [
        ("history.get", json!({"id":"1"})),
        ("history.list", json!({"limit":200})),
    ] {
        let fixture = Fixture::new();
        seed(&fixture);
        let original = history_bytes(&fixture.path);
        let token = CancellationToken::new();
        let (host, calls) = host(&fixture, &token, Fault::CancelOnReturn);
        let error = call(&host, method, params, &token).unwrap_err();
        assert_error(&error, "cancelled", method, "not_started");
        assert_eq!(history_bytes(&fixture.path), original);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }
}
