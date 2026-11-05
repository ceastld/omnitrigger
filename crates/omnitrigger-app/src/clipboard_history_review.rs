//! 仅合成目录的后台历史组合回归；由应用测试组合根显式声明，不启动原生监听器。

use crate::clipboard_history_background::{
    BackgroundError, BackgroundListener, BackgroundStore, Cancellation, ClipboardHistoryWriter,
    HistoryBackground, ListenerFactory, ListenerPoll, ObservedText,
};
use crate::clipboard_history_controller::{
    Command, CommitDisposition, DropReason, Event, HistoryControl, InitialCapturePolicy,
    Observation, SessionKey, Status, StoreFailure,
};
use crate::clipboard_history_store::{ClipboardHistoryStore, HistoryError};
use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    time::Duration,
};

struct Fixture {
    directory: PathBuf,
    path: PathBuf,
    store: Arc<ClipboardHistoryStore>,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let parent = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        crate::script_runtime::check_local_path(&parent).unwrap();
        let directory = loop {
            let candidate = parent.join(format!(
                ".history-background-review-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&candidate) {
                Ok(()) => break candidate,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(_) => panic!("无法创建合成历史测试目录"),
            }
        };
        let path = directory.join("history.json");
        let store = Arc::new(ClipboardHistoryStore::new(path.clone()).unwrap());
        Self {
            directory,
            path,
            store,
        }
    }
    fn revision(&self) -> String {
        self.store.list(200, || Ok(())).unwrap().revision
    }
    fn rows_oldest_first(&self) -> Vec<(String, String)> {
        self.store
            .list(200, || Ok(()))
            .unwrap()
            .items
            .into_iter()
            .rev()
            .map(|summary| {
                let item = self.store.get(&summary.id, || Ok(())).unwrap();
                (item.id, item.text)
            })
            .collect()
    }
    fn body_exists(&self) -> bool {
        history_bytes(&self.path).is_some()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        assert_eq!(
            self.directory.parent(),
            Some(Path::new(env!("CARGO_MANIFEST_DIR")))
        );
        assert!(self
            .directory
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(".history-background-review-"));
        if crate::script_runtime::check_local_path(&self.directory).is_err() {
            return;
        }
        if let Ok(entries) = fs::read_dir(&self.directory) {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name();
                let name = name.to_string_lossy();
                let owned = name == "history.json"
                    || name == "history.json.lock"
                    || name.starts_with(&format!("history.json.{}.", std::process::id()))
                        && name.ends_with(".tmp");
                if owned
                    && crate::script_runtime::check_local_path(&path).is_ok()
                    && fs::symlink_metadata(&path).is_ok_and(|metadata| {
                        metadata.is_file() && !metadata.file_type().is_symlink()
                    })
                {
                    let _ = fs::remove_file(path);
                }
            }
        }
        let _ = fs::remove_dir(&self.directory);
    }
}
fn history_bytes(path: &Path) -> Option<Vec<u8>> {
    crate::script_runtime::check_local_path(path).unwrap();
    match fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => panic!("合成历史文件读取失败"),
    }
}

enum Injection {
    Before(&'static str),
    After(&'static str),
    TokenAfter(Cancellation),
}
#[derive(Default)]
struct StoreAudit {
    calls: usize,
    next: Option<Injection>,
    errors: Vec<(&'static str, &'static str)>,
}
struct ProbeStore {
    real: Arc<ClipboardHistoryStore>,
    writer: ClipboardHistoryWriter,
    path: PathBuf,
    audit: Arc<Mutex<StoreAudit>>,
}
impl BackgroundStore for ProbeStore {
    fn append(&mut self, text: &str, cancel: &Cancellation) -> Result<(), StoreFailure> {
        let injection = {
            let mut audit = self.audit.lock().unwrap();
            audit.calls += 1;
            audit.next.take()
        };
        let Some(injection) = injection else {
            return self.writer.append(text, cancel);
        };
        let (code, before, token_after) = match injection {
            Injection::Before(code) => (code, true, None),
            Injection::After(code) => (code, false, None),
            Injection::TokenAfter(token) => ("cancelled", false, Some(token)),
        };
        assert!(matches!(code, "cancelled" | "timeout"));
        let original = history_bytes(&self.path);
        // 仅在真实Store公开事务检查点注入错误，不复刻ID/去重/落盘模型。
        let error = self
            .real
            .append_at(text, 0, || {
                if cancel.is_cancelled() {
                    return Err(HistoryError::new("cancelled"));
                }
                if before || history_bytes(&self.path) != original {
                    if let Some(token) = &token_after {
                        token.cancel();
                    }
                    return Err(HistoryError::new(code));
                }
                Ok(())
            })
            .expect_err("真实事务必须经过指定故障检查点");
        assert_eq!(error.code, code);
        let expected_effect = if before { "not_started" } else { "unknown" };
        assert_eq!(error.effect_state, expected_effect);
        self.audit
            .lock()
            .unwrap()
            .errors
            .push((error.code, error.effect_state));
        // 故障路径观察真实Store阶段；生产Writer错误翻译由其独立模块测试覆盖。
        Err(if before {
            StoreFailure::Cancelled
        } else {
            StoreFailure::Unknown
        })
    }
}

#[derive(Default)]
struct ListenerAudit {
    starts: usize,
    stops: usize,
    polls: usize,
}
struct Feed {
    session: u64,
    baseline: u32,
    records: VecDeque<ListenerPoll>,
    stop_failures: usize,
    cancel_on_next: Option<Cancellation>,
}
struct Factory {
    feeds: VecDeque<Arc<Mutex<Feed>>>,
    audit: Arc<Mutex<ListenerAudit>>,
}
struct Listener {
    feed: Arc<Mutex<Feed>>,
    audit: Arc<Mutex<ListenerAudit>>,
}
impl ListenerFactory for Factory {
    fn start(
        &mut self,
        initial: InitialCapturePolicy,
        _: &Cancellation,
    ) -> Result<Box<dyn BackgroundListener>, BackgroundError> {
        assert_eq!(initial, InitialCapturePolicy::FutureNotificationsOnly);
        self.audit.lock().unwrap().starts += 1;
        let feed = self.feeds.pop_front().expect("不能自动创建未授权会话");
        Ok(Box::new(Listener {
            feed,
            audit: self.audit.clone(),
        }))
    }
}
impl BackgroundListener for Listener {
    fn session(&self) -> u64 {
        self.feed.lock().unwrap().session
    }
    fn baseline_sequence(&self) -> u32 {
        self.feed.lock().unwrap().baseline
    }
    fn try_next(&mut self) -> Result<ListenerPoll, BackgroundError> {
        self.audit.lock().unwrap().polls += 1;
        let mut feed = self.feed.lock().unwrap();
        if let Some(cancel) = feed.cancel_on_next.take() {
            cancel.cancel();
        }
        Ok(feed.records.pop_front().unwrap_or(ListenerPoll::Empty))
    }
    fn stop(&mut self) -> Result<(), BackgroundError> {
        self.audit.lock().unwrap().stops += 1;
        let mut feed = self.feed.lock().unwrap();
        if feed.stop_failures > 0 {
            feed.stop_failures -= 1;
            return Err(BackgroundError::CleanupFailed);
        }
        Ok(())
    }
}
fn text(sequence: u32, text: &str) -> ListenerPoll {
    ListenerPoll::Text(ObservedText {
        sequence,
        text: text.into(),
    })
}
fn feed(records: Vec<ListenerPoll>) -> Arc<Mutex<Feed>> {
    Arc::new(Mutex::new(Feed {
        session: 7,
        baseline: 10,
        records: records.into(),
        stop_failures: 0,
        cancel_on_next: None,
    }))
}
type Driver = HistoryBackground<Factory, ProbeStore>;
fn driver(
    fixture: &Fixture,
    feeds: Vec<Arc<Mutex<Feed>>>,
) -> (Driver, Arc<Mutex<ListenerAudit>>, Arc<Mutex<StoreAudit>>) {
    let listener_audit = Arc::new(Mutex::new(ListenerAudit::default()));
    let store_audit = Arc::new(Mutex::new(StoreAudit::default()));
    let store = ProbeStore {
        real: fixture.store.clone(),
        writer: ClipboardHistoryWriter::new(fixture.store.clone()),
        path: fixture.path.clone(),
        audit: store_audit.clone(),
    };
    (
        HistoryBackground::new(
            Factory {
                feeds: feeds.into(),
                audit: listener_audit.clone(),
            },
            store,
        ),
        listener_audit,
        store_audit,
    )
}
fn running(control: &HistoryControl) -> SessionKey {
    assert!(matches!(
        control.apply(Event::Enable).unwrap().commands.as_slice(),
        [Command::Start { generation: 1, .. }]
    ));
    control
        .apply(Event::Started {
            generation: 1,
            listener_session: 7,
            baseline_sequence: 10,
        })
        .unwrap();
    SessionKey {
        generation: 1,
        listener_session: 7,
    }
}
fn observed(key: SessionKey, sequence: u32, text: &str) -> Event {
    Event::Observed(Observation {
        key,
        sequence,
        text: text.into(),
    })
}

#[test]
fn disable_confirmation_prevents_all_later_queued_old_writes() {
    let fixture = Fixture::new();
    let old = feed(vec![text(11, "合成旧记录")]);
    let (mut driver, listeners, calls) = driver(&fixture, vec![old.clone()]);
    driver.enable(Cancellation::default()).unwrap();
    assert_eq!(driver.disable().unwrap().status, Status::Stopped);
    old.lock()
        .unwrap()
        .records
        .push_back(text(12, "合成停用后记录"));
    driver.poll_once().unwrap();
    assert_eq!(calls.lock().unwrap().calls, 0);
    assert!(!fixture.body_exists());
    assert_eq!(listeners.lock().unwrap().stops, 1);
}

#[test]
fn reused_native_session_does_not_reuse_the_old_receiver_or_old_ids() {
    let fixture = Fixture::new();
    let old = feed(vec![text(11, "合成旧队列")]);
    let current = feed(vec![text(12, "合成新队列")]);
    let (mut driver, listeners, _) = driver(&fixture, vec![old.clone(), current]);
    let first = driver.enable(Cancellation::default()).unwrap().status;
    driver.disable().unwrap();
    let second = driver.enable(Cancellation::default()).unwrap().status;
    assert_ne!(first, second);
    old.lock()
        .unwrap()
        .records
        .push_back(text(13, "合成迟到旧记录"));
    assert_eq!(
        driver.poll_once().unwrap().commit,
        Some(CommitDisposition::Stored)
    );
    assert_eq!(
        fixture.rows_oldest_first(),
        [("1".to_owned(), "合成新队列".to_owned())]
    );
    assert_eq!(fixture.revision(), "1");
    assert_eq!(listeners.lock().unwrap().starts, 2);
}

#[test]
fn cancellation_between_poll_and_commit_never_enters_store() {
    let fixture = Fixture::new();
    let input = feed(vec![text(11, "合成竞态取消")]);
    let token = Cancellation::default();
    input.lock().unwrap().cancel_on_next = Some(token.clone());
    let (mut driver, listeners, calls) = driver(&fixture, vec![input]);
    driver.enable(token).unwrap();
    assert_eq!(
        driver.poll_once().unwrap().commit,
        Some(CommitDisposition::Failed(StoreFailure::Cancelled))
    );
    assert_eq!(calls.lock().unwrap().calls, 0);
    assert!(!fixture.body_exists());
    assert_eq!(driver.poll_once().unwrap().status, Status::Stopped);
    assert_eq!(listeners.lock().unwrap().stops, 1);
}

#[test]
fn cancelled_or_timeout_unknown_keeps_one_real_commit_and_never_retries_ticket() {
    for code in ["cancelled", "timeout"] {
        let fixture = Fixture::new();
        let input = feed(vec![
            text(11, "合成同文"),
            text(11, "合成同文"),
            text(12, "合成同文"),
        ]);
        let (mut driver, _, calls) = driver(&fixture, vec![input]);
        calls.lock().unwrap().next = Some(Injection::After(code));
        driver.enable(Cancellation::default()).unwrap();
        assert_eq!(
            driver.poll_once().unwrap().commit,
            Some(CommitDisposition::Failed(StoreFailure::Unknown))
        );
        assert_eq!(fixture.revision(), "1");
        assert_eq!(
            driver.poll_once().unwrap().dropped,
            Some(DropReason::DuplicateSequence)
        );
        assert_eq!(calls.lock().unwrap().calls, 1);
        let original = history_bytes(&fixture.path);
        assert_eq!(
            driver.poll_once().unwrap().commit,
            Some(CommitDisposition::Stored)
        );
        assert_eq!(history_bytes(&fixture.path), original);
        assert_eq!(
            fixture.rows_oldest_first(),
            [("1".to_owned(), "合成同文".to_owned())]
        );
        assert_eq!(calls.lock().unwrap().calls, 2);
        assert_eq!(calls.lock().unwrap().errors, [(code, "unknown")]);
        driver.disable().unwrap();
        driver.poll_once().unwrap();
        assert_eq!(calls.lock().unwrap().calls, 2);
    }
}

#[test]
fn before_commit_cancel_or_timeout_does_not_consume_a_persisted_id() {
    for code in ["cancelled", "timeout"] {
        let fixture = Fixture::new();
        let (mut driver, _, calls) = driver(
            &fixture,
            vec![feed(vec![
                text(11, "合成取消前"),
                text(11, "合成重复"),
                text(12, "合成后续"),
            ])],
        );
        calls.lock().unwrap().next = Some(Injection::Before(code));
        driver.enable(Cancellation::default()).unwrap();
        assert_eq!(
            driver.poll_once().unwrap().commit,
            Some(CommitDisposition::Failed(StoreFailure::Cancelled))
        );
        assert!(!fixture.body_exists());
        assert_eq!(
            driver.poll_once().unwrap().dropped,
            Some(DropReason::DuplicateSequence)
        );
        driver.poll_once().unwrap();
        assert_eq!(
            fixture.rows_oldest_first(),
            [("1".to_owned(), "合成后续".to_owned())]
        );
        assert_eq!(fixture.revision(), "1");
        assert_eq!(calls.lock().unwrap().errors, [(code, "not_started")]);
    }
}

#[test]
fn actual_token_cancellation_after_real_commit_is_unknown_and_disable_does_not_repeat() {
    let fixture = Fixture::new();
    let token = Cancellation::default();
    let (mut driver, _, calls) = driver(
        &fixture,
        vec![feed(vec![
            text(11, "合成已提交"),
            text(12, "合成不能再提交"),
        ])],
    );
    calls.lock().unwrap().next = Some(Injection::TokenAfter(token.clone()));
    driver.enable(token).unwrap();
    assert_eq!(
        driver.poll_once().unwrap().commit,
        Some(CommitDisposition::Failed(StoreFailure::Unknown))
    );
    assert_eq!(driver.disable().unwrap().status, Status::Stopped);
    driver.poll_once().unwrap();
    assert_eq!(
        fixture.rows_oldest_first(),
        [("1".to_owned(), "合成已提交".to_owned())]
    );
    assert_eq!(fixture.revision(), "1");
    assert_eq!(calls.lock().unwrap().calls, 1);
}

#[test]
fn empty_whitespace_and_exact_duplicates_keep_original_text_and_contiguous_ids() {
    let fixture = Fixture::new();
    let (mut driver, _, calls) = driver(
        &fixture,
        vec![feed(vec![
            text(11, ""),
            text(12, ""),
            text(13, "  "),
            text(14, "\r\n\t "),
            text(14, "合成不应转发"),
            text(16, "\r\n\t "),
        ])],
    );
    driver.enable(Cancellation::default()).unwrap();
    for _ in 0..6 {
        driver.poll_once().unwrap();
    }
    assert_eq!(
        fixture.rows_oldest_first(),
        [
            ("1".to_owned(), "".to_owned()),
            ("2".to_owned(), "  ".to_owned()),
            ("3".to_owned(), "\r\n\t ".to_owned())
        ]
    );
    assert_eq!(fixture.revision(), "3");
    assert_eq!(calls.lock().unwrap().calls, 5);
    assert_eq!(
        fixture
            .store
            .list(200, || Ok(()))
            .unwrap()
            .items
            .into_iter()
            .map(|item| item.id)
            .collect::<Vec<_>>(),
        ["3", "2", "1"]
    );
}

#[test]
fn failed_stop_blocks_new_session_and_store_until_explicit_cleanup() {
    let fixture = Fixture::new();
    let old = feed(vec![text(11, "合成第一项"), text(12, "合成旧队列")]);
    old.lock().unwrap().stop_failures = 1;
    let current = feed(vec![text(13, "合成第二项")]);
    let (mut driver, listeners, calls) = driver(&fixture, vec![old, current]);
    driver.enable(Cancellation::default()).unwrap();
    driver.poll_once().unwrap();
    assert!(matches!(
        driver.disable().unwrap().status,
        Status::CleanupFailed { .. }
    ));
    assert_eq!(
        driver.enable(Cancellation::default()).err(),
        Some(BackgroundError::InvalidState)
    );
    driver.poll_once().unwrap();
    assert_eq!(calls.lock().unwrap().calls, 1);
    assert_eq!(driver.retry_cleanup().unwrap().status, Status::Stopped);
    assert_eq!(listeners.lock().unwrap().starts, 1);
    driver.enable(Cancellation::default()).unwrap();
    driver.poll_once().unwrap();
    assert_eq!(
        fixture.rows_oldest_first(),
        [
            ("1".to_owned(), "合成第一项".to_owned()),
            ("2".to_owned(), "合成第二项".to_owned())
        ]
    );
    assert_eq!(fixture.revision(), "2");
}

#[test]
fn disable_ack_waits_for_real_unknown_commit_and_revokes_later_old_requests() {
    let fixture = Fixture::new();
    let control = Arc::new(HistoryControl::default());
    let key = running(&control);
    let request = control
        .apply(observed(key, 11, "合成在途提交"))
        .unwrap()
        .append
        .unwrap();
    let retry = request.clone();
    let store = fixture.store.clone();
    let path = fixture.path.clone();
    let (entered, ready) = mpsc::sync_channel(1);
    let (release, released) = mpsc::sync_channel(1);
    let writing = control.clone();
    let writer = std::thread::spawn(move || {
        writing.commit(&request, |text| {
            let error = store
                .append_at(text, 0, || {
                    if history_bytes(&path).is_some() {
                        entered.send(()).unwrap();
                        released.recv_timeout(Duration::from_secs(5)).unwrap();
                        return Err(HistoryError::new("timeout"));
                    }
                    Ok(())
                })
                .expect_err("真实提交后的检查点应注入超时");
            assert_eq!((error.code, error.effect_state), ("timeout", "unknown"));
            Err(StoreFailure::Unknown)
        })
    });
    ready.recv_timeout(Duration::from_secs(5)).unwrap();
    let (requested, request_seen) = mpsc::sync_channel(1);
    let (ack, ack_seen) = mpsc::sync_channel(1);
    let disabling = control.clone();
    let disable = std::thread::spawn(move || {
        requested.send(()).unwrap();
        ack.send(disabling.apply(Event::Disable).unwrap()).unwrap();
    });
    request_seen.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(matches!(
        ack_seen.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    release.send(()).unwrap();
    assert_eq!(
        writer.join().unwrap().unwrap(),
        CommitDisposition::Failed(StoreFailure::Unknown)
    );
    assert_eq!(
        ack_seen
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .commands,
        [Command::Stop { key }]
    );
    disable.join().unwrap();
    assert_eq!(
        control
            .commit(&retry, |_| panic!("停用确认后不得写旧项"))
            .unwrap(),
        CommitDisposition::Discarded
    );
    assert_eq!(
        control
            .apply(observed(key, 12, "合成迟到"))
            .unwrap()
            .dropped,
        Some(DropReason::InactiveSession)
    );
    assert_eq!(
        fixture.rows_oldest_first(),
        [("1".to_owned(), "合成在途提交".to_owned())]
    );
    assert_eq!(fixture.revision(), "1");
}

#[test]
fn same_controller_new_generation_rejects_old_ticket_without_consuming_current_pending() {
    let fixture = Fixture::new();
    let control = HistoryControl::default();
    let old = running(&control);
    let request = control
        .apply(observed(old, 11, "合成旧代"))
        .unwrap()
        .append
        .unwrap();
    control.apply(Event::Disable).unwrap();
    control.apply(Event::Enable).unwrap();
    control.apply(Event::Stopped { key: old }).unwrap();
    control
        .apply(Event::Started {
            generation: 2,
            listener_session: 7,
            baseline_sequence: 10,
        })
        .unwrap();
    let current = SessionKey {
        generation: 2,
        listener_session: 7,
    };
    let own = control
        .apply(observed(current, 11, "合成新代"))
        .unwrap()
        .append
        .unwrap();
    assert_eq!(
        control
            .commit(&request, |_| panic!("旧代不可进存储"))
            .unwrap(),
        CommitDisposition::Discarded
    );
    let mut writer = ClipboardHistoryWriter::new(fixture.store.clone());
    assert_eq!(
        control
            .commit(&own, |text| writer.append(text, &Cancellation::default()))
            .unwrap(),
        CommitDisposition::Stored
    );
    assert_eq!(
        fixture.rows_oldest_first(),
        [("1".to_owned(), "合成新代".to_owned())]
    );
}

#[test]
fn foreign_controller_ticket_cannot_write_or_consume_recreated_owner_pending() {
    let fixture = Fixture::new();
    let old = HistoryControl::default();
    let key = running(&old);
    let stale = old
        .apply(observed(key, 11, "合成旧owner正文"))
        .unwrap()
        .append
        .unwrap();
    old.apply(Event::Disable).unwrap();
    old.apply(Event::Stopped { key }).unwrap();
    drop(old);
    let current = HistoryControl::default();
    let key = running(&current);
    let own = current
        .apply(observed(key, 11, "合成新owner正文"))
        .unwrap()
        .append
        .unwrap();
    assert_eq!(
        current
            .commit(&stale, |_| panic!("外来许可不可进存储"))
            .unwrap(),
        CommitDisposition::Discarded
    );
    assert!(!fixture.body_exists());
    let mut writer = ClipboardHistoryWriter::new(fixture.store.clone());
    assert_eq!(
        current
            .commit(&own, |text| writer.append(text, &Cancellation::default()))
            .unwrap(),
        CommitDisposition::Stored
    );
    assert_eq!(
        fixture.rows_oldest_first(),
        [("1".to_owned(), "合成新owner正文".to_owned())]
    );
    assert_eq!(fixture.revision(), "1");
}

#[path = "clipboard_history_ui_lifecycle_review.rs"]
mod ui_lifecycle_review;

#[path = "clipboard_history_host_stage_review.rs"]
mod host_stage_review;
