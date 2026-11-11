use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

fn scope() -> ActionStateScope {
    ActionStateScope {
        package_id: "test.package".into(),
        action_id: "test-action".into(),
    }
}
fn call(store: &ActionStateStore, method: &str, params: Value) -> Result<Value, ActionError> {
    store.call(
        scope(),
        method,
        params,
        &|| false,
        Instant::now() + Duration::from_secs(5),
    )
}
fn read(store: &ActionStateStore, area: &str, key: &str) -> Value {
    call(store, "state.read", json!({"area":area,"key":key})).unwrap()
}
fn cas(
    store: &ActionStateStore,
    area: &str,
    key: &str,
    snapshot: &Value,
    next: Value,
) -> Result<Value, ActionError> {
    call(
        store,
        "state.compare_and_swap",
        json!({"area":area,"key":key,"expectedRevision":snapshot["revision"],"next":next}),
    )
}

type MemoryDocuments = BTreeMap<ActionStateScope, (String, BTreeMap<String, Value>)>;
#[derive(Default)]
struct MemoryDisk {
    current: Mutex<MemoryDocuments>,
    fail: AtomicBool,
}
impl DurableStateStorage for MemoryDisk {
    fn load(&self, scope: &ActionStateScope) -> Result<DurableStateSnapshot, StateStorageError> {
        let d = self.current.lock().unwrap();
        let (version, values) = d
            .get(scope)
            .cloned()
            .unwrap_or(("missing".into(), BTreeMap::new()));
        Ok(DurableStateSnapshot { version, values })
    }
    fn commit(
        &self,
        scope: &ActionStateScope,
        expected: &str,
        values: &BTreeMap<String, Value>,
        before: &dyn Fn() -> Result<(), StateStorageError>,
    ) -> Result<String, StateStorageError> {
        let mut d = self.current.lock().unwrap();
        let version = d.get(scope).map(|v| v.0.as_str()).unwrap_or("missing");
        if version != expected {
            return Err(StateStorageError::Changed);
        }
        before()?;
        if self.fail.load(Ordering::Acquire) {
            return Err(StateStorageError::Io);
        }
        let version = format!("{:x}", Sha256::digest(serde_json::to_vec(values).unwrap()));
        d.insert(scope.clone(), (version.clone(), values.clone()));
        Ok(version)
    }
}

#[test]
fn missing_null_zero_and_delete_are_distinct_and_delete_prevents_aba() {
    let store = ActionStateStore::new(None).unwrap();
    let missing = read(&store, "session", "key");
    assert_eq!(missing["entry"], json!({"present":false}));
    let null = cas(
        &store,
        "session",
        "key",
        &missing,
        json!({"present":true,"value":null}),
    )
    .unwrap()["current"]
        .clone();
    assert_eq!(null["entry"], json!({"present":true,"value":null}));
    let zero = cas(
        &store,
        "session",
        "key",
        &null,
        json!({"present":true,"value":0}),
    )
    .unwrap()["current"]
        .clone();
    assert_eq!(zero["entry"]["value"], 0);
    let deleted =
        cas(&store, "session", "key", &zero, json!({"present":false})).unwrap()["current"].clone();
    assert_eq!(deleted["entry"], missing["entry"]);
    assert_ne!(deleted["revision"], missing["revision"]);
    assert_eq!(
        cas(
            &store,
            "session",
            "key",
            &missing,
            json!({"present":true,"value":1})
        )
        .unwrap()["status"],
        "conflict"
    );
}

#[test]
fn revisions_are_bound_to_area_key_scope_and_generation() {
    let disk = Arc::new(MemoryDisk::default());
    let store = ActionStateStore::new(Some(disk.clone())).unwrap();
    let old = read(&store, "session", "key");
    assert_ne!(
        old["revision"],
        read(&store, "session", "other")["revision"]
    );
    assert_ne!(old["revision"], read(&store, "durable", "key")["revision"]);
    let mut other = scope();
    other.action_id = "other".into();
    let value = store
        .call(
            other,
            "state.read",
            json!({"area":"session","key":"key"}),
            &|| false,
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap();
    assert_ne!(old["revision"], value["revision"]);
    let durable = read(&store, "durable", "key");
    cas(
        &store,
        "durable",
        "key",
        &durable,
        json!({"present":true,"value":"kept"}),
    )
    .unwrap();
    let restarted = ActionStateStore::new(Some(disk)).unwrap();
    assert_ne!(
        old["revision"],
        read(&restarted, "session", "key")["revision"]
    );
    assert_eq!(
        read(&restarted, "session", "key")["entry"]["present"],
        false
    );
    assert_eq!(read(&restarted, "durable", "key")["entry"]["value"], "kept");
    assert_eq!(
        cas(
            &restarted,
            "durable",
            "key",
            &durable,
            json!({"present":false})
        )
        .unwrap()["status"],
        "conflict"
    );
}

#[test]
fn durable_failure_does_not_publish_cache_or_revision_and_can_be_retried_explicitly() {
    let disk = Arc::new(MemoryDisk::default());
    let store = ActionStateStore::new(Some(disk.clone())).unwrap();
    let before = read(&store, "durable", "key");
    disk.fail.store(true, Ordering::Release);
    let e = cas(
        &store,
        "durable",
        "key",
        &before,
        json!({"present":true,"value":"new"}),
    )
    .unwrap_err();
    assert_eq!(e.code, "state_io");
    assert_eq!(read(&store, "durable", "key"), before);
    assert!(disk.current.lock().unwrap().is_empty());
    disk.fail.store(false, Ordering::Release);
    assert_eq!(
        cas(
            &store,
            "durable",
            "key",
            &before,
            json!({"present":true,"value":"new"})
        )
        .unwrap()["status"],
        "committed"
    );
}

#[test]
fn invalid_requests_and_bounds_do_not_mutate_existing_state() {
    let store = ActionStateStore::new(None).unwrap();
    let before = read(&store, "session", "key");
    for next in [
        json!({"present":false,"value":0}),
        json!({"present":true}),
        json!({"present":true,"value":"x".repeat(MAX_STATE_VALUE_BYTES+1)}),
        json!({"present":true,"value":9_007_199_254_740_992u64}),
    ] {
        assert_eq!(
            cas(&store, "session", "key", &before, next)
                .unwrap_err()
                .code,
            "invalid_input"
        );
        assert_eq!(read(&store, "session", "key"), before);
    }
    let mut deep = Value::Null;
    for _ in 0..=MAX_STATE_DEPTH {
        deep = json!([deep]);
    }
    assert!(!valid_state_value(&deep));
    assert_eq!(
        call(
            &store,
            "state.read",
            json!({"area":"session","key":"key","actionId":"other"})
        )
        .unwrap_err()
        .code,
        "invalid_input"
    );
    for i in 0..MAX_STATE_KEYS {
        let key = format!("k{i}");
        let s = read(&store, "session", &key);
        cas(&store, "session", &key, &s, json!({"present":false})).unwrap();
    }
    let s = read(&store, "session", "extra");
    assert_eq!(
        cas(
            &store,
            "session",
            "extra",
            &s,
            json!({"present":true,"value":0})
        )
        .unwrap_err()
        .code,
        "state_limit"
    );
}

#[test]
fn missing_durable_backend_cancellation_and_timeout_are_explicit() {
    let store = ActionStateStore::new(None).unwrap();
    assert_eq!(
        call(&store, "state.read", json!({"area":"durable","key":"key"}))
            .unwrap_err()
            .code,
        "state_unavailable"
    );
    assert_eq!(
        store
            .call(
                scope(),
                "state.read",
                json!({"area":"session","key":"key"}),
                &|| true,
                Instant::now() + Duration::from_secs(1)
            )
            .unwrap_err()
            .code,
        "cancelled"
    );
    assert_eq!(
        store
            .call(
                scope(),
                "state.read",
                json!({"area":"session","key":"key"}),
                &|| false,
                Instant::now()
            )
            .unwrap_err()
            .code,
        "timeout"
    );
}

#[test]
fn stale_external_storage_invalidates_cache_and_old_epoch() {
    let disk = Arc::new(MemoryDisk::default());
    let store = ActionStateStore::new(Some(disk.clone())).unwrap();
    let old = read(&store, "durable", "key");
    disk.current.lock().unwrap().insert(
        scope(),
        (
            "external".into(),
            BTreeMap::from([("key".into(), json!(7))]),
        ),
    );
    assert_eq!(
        cas(
            &store,
            "durable",
            "key",
            &old,
            json!({"present":true,"value":8})
        )
        .unwrap_err()
        .code,
        "state_storage_changed"
    );
    let fresh = read(&store, "durable", "key");
    assert_eq!(fresh["entry"]["value"], 7);
    assert_ne!(fresh["revision"], old["revision"]);
}
