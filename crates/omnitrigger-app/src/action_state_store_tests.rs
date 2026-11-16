//! 仅使用新建合成目录，验证文件状态存储的失败、冲突与重启边界。
use super::*;
use serde_json::json;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "omnitrigger-state-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn storage(&self) -> FileActionStateStorage {
        FileActionStateStorage::new(self.0.join("action-state")).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // 只清理由本fixture排他创建且位于临时根中的合成目录。
        if self.0.parent() == Some(std::env::temp_dir().as_path()) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
fn scope() -> ActionStateScope {
    ActionStateScope {
        package_id: "synthetic.package".into(),
        action_id: "action".into(),
    }
}

#[test]
fn construction_and_missing_load_do_not_create_state_files_or_directories() {
    let f = Fixture::new();
    let storage = f.storage();
    assert!(!storage.root.exists());
    let s = storage.load(&scope()).unwrap();
    assert!(s.values.is_empty());
    assert_eq!(s.version, "missing");
    assert!(!storage.root.exists());
}

#[test]
fn failed_atomic_replace_keeps_original_bytes_and_removes_temporary_file() {
    let f = Fixture::new();
    let storage = f.storage();
    let values = BTreeMap::from([("key".into(), json!(0))]);
    let initial = storage.load(&scope()).unwrap();
    let revision = storage
        .commit(&scope(), &initial.version, &values, &|| Ok(()))
        .unwrap();
    let path = storage.path(&scope()).unwrap();
    let bytes = fs::read(&path).unwrap();
    storage.fail_before_replace.store(true, Ordering::Release);
    assert_eq!(
        storage
            .commit(
                &scope(),
                &revision,
                &BTreeMap::from([("key".into(), json!(1))]),
                &|| Ok(())
            )
            .unwrap_err(),
        StateStorageError::Io
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(storage.load(&scope()).unwrap().version, revision);
    assert!(fs::read_dir(&storage.root).unwrap().all(|p| !p
        .unwrap()
        .file_name()
        .to_string_lossy()
        .ends_with(".tmp")));
}

#[test]
fn cancellation_at_replace_boundary_preserves_original_and_initial_failure_keeps_missing() {
    let f = Fixture::new();
    let storage = f.storage();
    storage.fail_before_replace.store(true, Ordering::Release);
    let values = BTreeMap::from([("key".into(), Value::Null)]);
    assert_eq!(
        storage
            .commit(&scope(), "missing", &values, &|| Ok(()))
            .unwrap_err(),
        StateStorageError::Io
    );
    assert!(!storage.path(&scope()).unwrap().exists());
    storage.fail_before_replace.store(false, Ordering::Release);
    let revision = storage
        .commit(&scope(), "missing", &values, &|| Ok(()))
        .unwrap();
    let old = fs::read(storage.path(&scope()).unwrap()).unwrap();
    let calls = std::cell::Cell::new(0);
    let before = || {
        calls.set(calls.get() + 1);
        if calls.get() == 2 {
            Err(StateStorageError::Cancelled)
        } else {
            Ok(())
        }
    };
    assert_eq!(
        storage
            .commit(&scope(), &revision, &BTreeMap::new(), &before)
            .unwrap_err(),
        StateStorageError::Cancelled
    );
    assert_eq!(fs::read(storage.path(&scope()).unwrap()).unwrap(), old);
}

#[test]
fn external_change_and_second_writer_lock_never_overwrite_newer_file() {
    let f = Fixture::new();
    let a = f.storage();
    let b = f.storage();
    let values = BTreeMap::from([("key".into(), json!(1))]);
    let version = a.commit(&scope(), "missing", &values, &|| Ok(())).unwrap();
    let old = b.load(&scope()).unwrap();
    assert_eq!(old.version, version);
    a.commit(
        &scope(),
        &version,
        &BTreeMap::from([("key".into(), json!(2))]),
        &|| Ok(()),
    )
    .unwrap();
    let path = a.path(&scope()).unwrap();
    let bytes = fs::read(&path).unwrap();
    assert_eq!(
        b.commit(&scope(), &old.version, &values, &|| Ok(()))
            .unwrap_err(),
        StateStorageError::Changed
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path.with_extension("lock"))
        .unwrap();
    lock.try_lock().unwrap();
    assert_eq!(
        a.commit(
            &scope(),
            &a.load(&scope()).unwrap().version,
            &values,
            &|| Ok(())
        )
        .unwrap_err(),
        StateStorageError::Busy
    );
}

#[test]
fn corrupt_wrong_scope_and_oversize_documents_remain_unchanged() {
    let f = Fixture::new();
    let storage = f.storage();
    fs::create_dir(&storage.root).unwrap();
    let path = storage.path(&scope()).unwrap();
    let wrong = ActionStateScope {
        package_id: "another".into(),
        action_id: "action".into(),
    };
    for bytes in [
        b"broken".to_vec(),
        serde_json::to_vec(&Document {
            schema_version: 2,
            revision: 1,
            scope: scope(),
            values: BTreeMap::new(),
        })
        .unwrap(),
        serde_json::to_vec(&Document {
            schema_version: 1,
            revision: 1,
            scope: wrong,
            values: BTreeMap::new(),
        })
        .unwrap(),
        vec![b' '; FILE_LIMIT + 1],
    ] {
        fs::write(&path, &bytes).unwrap();
        assert!(matches!(
            storage.load(&scope()),
            Err(StateStorageError::Corrupt)
        ));
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn key_never_selects_a_path_and_bindings_are_untouched_across_adapter_restart() {
    let f = Fixture::new();
    let bindings = f.0.join("bindings.json");
    fs::write(&bindings, b"untouched-binding-sentinel").unwrap();
    let storage = f.storage();
    let values = BTreeMap::from([
        ("../../bindings.json".into(), Value::Null),
        ("zero".into(), json!(0)),
    ]);
    storage
        .commit(&scope(), "missing", &values, &|| Ok(()))
        .unwrap();
    let restarted = f.storage();
    assert_eq!(restarted.load(&scope()).unwrap().values, values);
    assert_eq!(fs::read(&bindings).unwrap(), b"untouched-binding-sentinel");
    let name = storage.path(&scope()).unwrap();
    assert_eq!(name.parent(), Some(storage.root.as_path()));
    assert_eq!(name.file_stem().unwrap().to_string_lossy().len(), 64);
}

#[test]
fn same_value_commits_advance_storage_revision_and_reject_external_aba() {
    let f = Fixture::new();
    let a = f.storage();
    let b = f.storage();
    let values = BTreeMap::from([("key".into(), json!(1))]);
    let first = a.commit(&scope(), "missing", &values, &|| Ok(())).unwrap();
    let second = b.commit(&scope(), &first, &values, &|| Ok(())).unwrap();
    assert_ne!(first, second);
    let third = b
        .commit(
            &scope(),
            &second,
            &BTreeMap::from([("key".into(), json!(2))]),
            &|| Ok(()),
        )
        .unwrap();
    let fourth = b.commit(&scope(), &third, &values, &|| Ok(())).unwrap();
    assert_ne!(first, fourth);
    assert_eq!(
        a.commit(&scope(), &first, &values, &|| Ok(())).unwrap_err(),
        StateStorageError::Changed
    );
}
