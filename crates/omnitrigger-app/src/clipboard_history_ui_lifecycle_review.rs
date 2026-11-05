//! 真实临时存储与历史页面状态的生命周期回归，不访问桌面或系统剪贴板。
use super::{driver, feed, history_bytes, text, Driver, Fixture, Injection};
use crate::clipboard_history_background::Cancellation;
use crate::clipboard_history_controller::{CommitDisposition, DropReason, Status, StoreFailure};
use crate::clipboard_history_ui_state::{Control, HistoryUiState, Phase};
use std::{fs, fs::File};

fn enabled(ui: &mut HistoryUiState, background: &mut Driver, token: Cancellation) -> u64 {
    let (id, command) = ui.primary().unwrap();
    assert_eq!(command, Control::Enable);
    let step = background.enable(token).unwrap();
    ui.acknowledge(id, step.status, None);
    assert_eq!(ui.phase, Phase::On);
    id
}
fn seed(fixture: &Fixture) {
    fixture.store.append_at("合成一", 1, || Ok(())).unwrap();
    fixture.store.append_at("合成二", 2, || Ok(())).unwrap();
}
fn held_sidecar(fixture: &Fixture) -> File {
    let path = fixture.path.with_file_name("history.json.lock");
    crate::script_runtime::check_local_path(&path).unwrap();
    let held = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    held.try_lock().unwrap();
    held
}

#[test]
fn cancelling_start_waits_for_real_stop_and_does_not_reopen_on_late_start_reply() {
    let fixture = Fixture::new();
    let token = Cancellation::default();
    let (mut background, listeners, calls) =
        driver(&fixture, vec![feed(vec![text(11, "合成不应采集")])]);
    let mut ui = HistoryUiState::new(true);
    let (start, _) = ui.primary().unwrap();
    let running = background.enable(token.clone()).unwrap();
    let (stop, command) = ui.primary().unwrap();
    assert_eq!(command, Control::Disable);
    token.cancel();
    assert_eq!(ui.phase, Phase::Closing);
    assert!(ui.primary().is_none());
    ui.acknowledge(start, running.status, None);
    assert_eq!(ui.phase, Phase::Closing);
    let stopped = background.disable().unwrap();
    ui.acknowledge(stop, stopped.status, None);
    assert_eq!(ui.phase, Phase::Off);
    assert!(!fixture.body_exists());
    assert_eq!(calls.lock().unwrap().calls, 0);
    assert_eq!(listeners.lock().unwrap().stops, 1);
}

#[test]
fn reopening_uses_new_control_id_and_receiver_without_losing_stable_selection() {
    let fixture = Fixture::new();
    let old = feed(vec![text(11, "合成旧会话项")]);
    let current = feed(vec![text(12, "合成新会话项")]);
    let token = Cancellation::default();
    let (mut background, _, calls) = driver(&fixture, vec![old.clone(), current]);
    let mut ui = HistoryUiState::new(true);
    let first = enabled(&mut ui, &mut background, token.clone());
    background.poll_once().unwrap();
    ui.listed(None, Ok(fixture.store.list(200, || Ok(())).unwrap()));
    ui.select("1");
    let (stop, _) = ui.primary().unwrap();
    token.cancel();
    ui.acknowledge(stop, background.disable().unwrap().status, None);
    let (next, command) = ui.primary().unwrap();
    assert_eq!(command, Control::Enable);
    ui.acknowledge(stop, Status::Stopped, Some("合成迟到关闭提示"));
    assert_eq!(ui.phase, Phase::Starting);
    assert!(ui.message.is_none());
    let running = background.enable(Cancellation::default()).unwrap();
    ui.acknowledge(next, running.status, None);
    ui.acknowledge(first, Status::Stopped, None);
    assert_eq!(ui.phase, Phase::On);
    old.lock()
        .unwrap()
        .records
        .push_back(text(13, "合成旧receiver迟到"));
    background.poll_once().unwrap();
    ui.listed(None, Ok(fixture.store.list(200, || Ok(())).unwrap()));
    assert_eq!(
        ui.items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        ["2", "1"]
    );
    assert_eq!(ui.selected.as_deref(), Some("1"));
    assert_eq!(calls.lock().unwrap().calls, 2);
    assert_eq!(
        fixture.rows_oldest_first(),
        [
            ("1".to_owned(), "合成旧会话项".to_owned()),
            ("2".to_owned(), "合成新会话项".to_owned())
        ]
    );
}

#[test]
fn stale_io_and_automatic_lists_cannot_complete_a_pending_explicit_query() {
    let fixture = Fixture::new();
    seed(&fixture);
    let mut ui = HistoryUiState::new(true);
    let id = ui.query().unwrap();
    ui.listed(
        Some(id + 1),
        Ok(fixture.store.list(200, || Ok(())).unwrap()),
    );
    ui.listed(None, Ok(fixture.store.list(200, || Ok(())).unwrap()));
    assert!(ui.items.is_empty());
    assert!(!ui.can_query());
    ui.listed(Some(id), Ok(fixture.store.list(200, || Ok(())).unwrap()));
    assert_eq!(
        ui.items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        ["2", "1"]
    );
    assert!(ui.can_query());
    assert_eq!(fixture.revision(), "2");
}

#[test]
fn real_unknown_removal_cannot_be_retried_until_an_explicit_summary_refresh() {
    let fixture = Fixture::new();
    seed(&fixture);
    let mut ui = HistoryUiState::new(true);
    ui.listed(None, Ok(fixture.store.list(200, || Ok(())).unwrap()));
    ui.select("1");
    let (request, item) = ui.remove().unwrap();
    let original = history_bytes(&fixture.path);
    let error = fixture
        .store
        .remove(&item, || {
            if history_bytes(&fixture.path) != original {
                Err(crate::clipboard_history_store::HistoryError::new(
                    "cancelled",
                ))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    assert_eq!((error.code, error.effect_state), ("cancelled", "unknown"));
    ui.listed(Some(request), Err("删除是否生效尚不确定，请先刷新。"));
    assert!(!ui.can_remove());
    ui.listed(None, Ok(fixture.store.list(200, || Ok(())).unwrap()));
    assert_eq!(ui.items.len(), 2);
    assert!(!ui.can_remove());
    let refresh = ui.query().unwrap();
    ui.listed(
        Some(refresh),
        Ok(fixture.store.list(200, || Ok(())).unwrap()),
    );
    assert_eq!(ui.items.len(), 1);
    assert!(ui.selected.is_none());
    assert_eq!(fixture.revision(), "3");
    assert!(!ui.can_remove()); // 还需要明确选中新的ID，而非重复旧删除。
}

#[test]
fn committed_removal_with_real_list_lock_failure_keeps_refresh_barrier() {
    let fixture = Fixture::new();
    seed(&fixture);
    let mut ui = HistoryUiState::new(true);
    ui.listed(None, Ok(fixture.store.list(200, || Ok(())).unwrap()));
    ui.select("1");
    let (request, item) = ui.remove().unwrap();
    assert!(fixture.store.remove(&item, || Ok(())).unwrap().removed);
    let held = held_sidecar(&fixture);
    assert_eq!(
        fixture.store.list(200, || Ok(())).unwrap_err().code,
        "history_busy"
    );
    ui.listed(Some(request), Err("删除已完成，但列表未能刷新。"));
    assert!(!ui.can_remove());
    drop(held);
    ui.listed(None, Ok(fixture.store.list(200, || Ok(())).unwrap()));
    assert!(!ui.can_remove());
    let refresh = ui.query().unwrap();
    ui.listed(
        Some(refresh),
        Ok(fixture.store.list(200, || Ok(())).unwrap()),
    );
    assert_eq!(ui.items[0].id, "2");
    assert!(ui.selected.is_none());
    assert_eq!(fixture.revision(), "3");
}

#[test]
fn real_unknown_append_does_not_auto_retry_or_claim_collection_closed() {
    let fixture = Fixture::new();
    let (mut background, _, calls) = driver(
        &fixture,
        vec![feed(vec![
            text(11, "合成unknown项"),
            text(11, "合成unknown项"),
        ])],
    );
    calls.lock().unwrap().next = Some(Injection::After("timeout"));
    let mut ui = HistoryUiState::new(true);
    enabled(&mut ui, &mut background, Cancellation::default());
    assert_eq!(
        background.poll_once().unwrap().commit,
        Some(CommitDisposition::Failed(StoreFailure::Unknown))
    );
    assert_eq!(ui.phase, Phase::On);
    assert_eq!(
        background.poll_once().unwrap().dropped,
        Some(DropReason::DuplicateSequence)
    );
    assert_eq!(calls.lock().unwrap().calls, 1);
    let refresh = ui.query().unwrap();
    ui.listed(
        Some(refresh),
        Ok(fixture.store.list(200, || Ok(())).unwrap()),
    );
    assert_eq!(ui.items[0].id, "1");
    assert_eq!(fixture.revision(), "1");
}

#[test]
fn stop_failure_keeps_closing_semantics_until_explicit_cleanup_ack() {
    let fixture = Fixture::new();
    let old = feed(vec![text(11, "合成不得追加")]);
    old.lock().unwrap().stop_failures = 1;
    let (mut background, listeners, calls) = driver(&fixture, vec![old, feed(vec![])]);
    let token = Cancellation::default();
    let mut ui = HistoryUiState::new(true);
    enabled(&mut ui, &mut background, token.clone());
    let (stop, _) = ui.primary().unwrap();
    token.cancel();
    assert!(ui.primary().is_none());
    ui.acknowledge(
        stop,
        background.disable().unwrap().status,
        Some("关闭未确认"),
    );
    assert_eq!(ui.phase, Phase::CloseFailed);
    let (retry, command) = ui.primary().unwrap();
    assert_eq!(command, Control::RetryCleanup);
    assert_eq!(ui.phase, Phase::Closing);
    assert!(ui.primary().is_none());
    ui.acknowledge(retry, background.retry_cleanup().unwrap().status, None);
    assert_eq!(ui.phase, Phase::Off);
    assert_eq!(listeners.lock().unwrap().starts, 1);
    assert_eq!(calls.lock().unwrap().calls, 0);
    assert!(!fixture.body_exists());
    assert_eq!(ui.primary().unwrap().1, Control::Enable); // 只给出新命令，不自动执行。
}

#[test]
fn dropping_background_cancels_and_stops_owned_listener_without_storing_queued_data() {
    let fixture = Fixture::new();
    let token = Cancellation::default();
    let (mut background, listeners, calls) =
        driver(&fixture, vec![feed(vec![text(11, "合成退出队列")])]);
    let mut ui = HistoryUiState::new(true);
    enabled(&mut ui, &mut background, token.clone());
    drop(background);
    assert!(token.is_cancelled());
    assert_eq!(listeners.lock().unwrap().stops, 1);
    assert_eq!(calls.lock().unwrap().calls, 0);
    assert!(!fixture.body_exists());
    ui.disconnected();
    assert_eq!(ui.phase, Phase::CloseFailed); // 没有把Drop信号冒称为UI收到关闭确认。
    assert!(!ui.primary_enabled());
}

#[test]
fn corrupt_store_known_not_started_must_not_be_presented_as_unknown_commit() {
    let fixture = Fixture::new();
    crate::script_runtime::check_local_path(&fixture.path).unwrap();
    let original = b"synthetic-invalid-json-private-body";
    fs::write(&fixture.path, original).unwrap();
    let error = fixture
        .store
        .append_at("合成新项", 0, || Ok(()))
        .unwrap_err();
    assert_eq!(
        (error.code, error.effect_state),
        ("history_corrupt", "not_started")
    );
    let (mut background, _, calls) = driver(
        &fixture,
        vec![feed(vec![text(11, "合成新项"), text(11, "合成新项")])],
    );
    let mut ui = HistoryUiState::new(true);
    enabled(&mut ui, &mut background, Cancellation::default());
    let step = background.poll_once().unwrap();
    assert_eq!(history_bytes(&fixture.path).unwrap(), original);
    assert_eq!(calls.lock().unwrap().calls, 1);
    assert_eq!(ui.phase, Phase::On);
    assert_eq!(
        step.commit,
        Some(CommitDisposition::Failed(StoreFailure::Unavailable))
    );
    assert_eq!(
        background.poll_once().unwrap().dropped,
        Some(DropReason::DuplicateSequence)
    );
}

#[test]
fn disconnected_state_must_not_be_revived_by_old_async_control_or_list_results() {
    let fixture = Fixture::new();
    seed(&fixture);
    let mut ui = HistoryUiState::new(true);
    let (start, _) = ui.primary().unwrap();
    let key = crate::clipboard_history_controller::SessionKey {
        generation: 1,
        listener_session: 7,
    };
    ui.acknowledge(start, Status::Running { key }, None);
    ui.listed(None, Ok(fixture.store.list(200, || Ok(())).unwrap()));
    ui.disconnected();
    let original_ids = ui
        .items
        .iter()
        .map(|item| item.id.clone())
        .collect::<Vec<_>>();
    fixture
        .store
        .append_at("合成晚到列表", 3, || Ok(()))
        .unwrap();
    ui.acknowledge(start, Status::Running { key }, None);
    ui.listed(None, Ok(fixture.store.list(200, || Ok(())).unwrap()));
    assert_eq!(ui.phase, Phase::CloseFailed);
    assert!(!ui.available);
    assert_eq!(
        ui.items
            .iter()
            .map(|item| item.id.clone())
            .collect::<Vec<_>>(),
        original_ids
    );
    assert!(!ui.primary_enabled());
    assert!(!ui.can_query());
}

#[test]
fn manual_refresh_after_real_store_busy_is_required_before_selection_actions_resume() {
    let fixture = Fixture::new();
    seed(&fixture);
    let mut ui = HistoryUiState::new(true);
    let first = ui.initial_query().unwrap();
    ui.listed(
        Some(first),
        Ok(fixture.store.as_ref().list(200, || Ok(())).unwrap()),
    );
    ui.select("1");
    assert!(ui.can_send());
    let original = history_bytes(&fixture.path);

    let held = held_sidecar(&fixture);
    let refresh = ui.query().unwrap();
    let error = fixture.store.as_ref().list(200, || Ok(())).unwrap_err();
    assert_eq!(
        (error.code, error.effect_state),
        ("history_busy", "not_started")
    );
    ui.listed(Some(refresh), Err("无法读取文本历史，请稍后明确刷新。"));
    assert_eq!(history_bytes(&fixture.path), original);
    assert!(ui.can_query());
    ui.select("2");
    assert!(
        ui.begin_send_selection().is_none(),
        "读取失败后须先明确刷新，不能从旧摘要开始发送"
    );
    assert!(ui.remove().is_none(), "更换选中项不能跳过刷新要求");
    assert_eq!(ui.phase, Phase::Off);

    drop(held);
    fixture
        .store
        .as_ref()
        .append_at("合成锁释放后的新项", 3, || Ok(()))
        .unwrap();
    ui.listed(
        None,
        Ok(fixture.store.as_ref().list(200, || Ok(())).unwrap()),
    );
    assert_eq!(ui.items.len(), 2, "后台列表不替代用户要求的明确刷新");
    assert!(ui.begin_send_selection().is_none());

    let retry = ui.query().unwrap();
    ui.listed(
        Some(retry),
        Ok(fixture.store.as_ref().list(200, || Ok(())).unwrap()),
    );
    assert_eq!(ui.items.len(), 3);
    let (_, selected) = ui.begin_send_selection().unwrap();
    assert_eq!(selected.summary.id, "2");
    assert_eq!(selected.revision, "3");
    assert_eq!(ui.phase, Phase::Off);
}
