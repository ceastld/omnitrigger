//! 文本历史页的纯界面状态。关闭确认来自后台，不由按钮点击或超时推断。

use crate::clipboard_history_controller::Status;
use crate::clipboard_history_send::{HistorySendSelection, SendFailure, SendReceipt};
use crate::clipboard_history_store::{HistoryList, HistorySummary};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    Off,
    Starting,
    On,
    Closing,
    CloseFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Control {
    Enable,
    Disable,
    RetryCleanup,
}

struct PendingSendSelection {
    request: u64,
    id: String,
    changed: bool,
}

pub(super) struct HistoryUiState {
    pub phase: Phase,
    pub items: Vec<HistorySummary>,
    pub revision: Option<String>,
    pub selected: Option<String>,
    pub message: Option<&'static str>,
    pub available: bool,
    next_id: u64,
    control_floor: u64,
    pending_control: Option<u64>,
    pending_io: Option<u64>,
    pending_send_selection: Option<PendingSendSelection>,
    refresh_required: bool,
    initial_query_requested: bool,
    popup_close_requested: bool,
}

impl HistoryUiState {
    pub fn new(available: bool) -> Self {
        Self {
            phase: Phase::Off,
            items: Vec::new(),
            revision: None,
            selected: None,
            message: (!available).then_some("文本历史暂时不可用。"),
            available,
            next_id: 0,
            control_floor: 0,
            pending_control: None,
            pending_io: None,
            pending_send_selection: None,
            refresh_required: false,
            initial_query_requested: false,
            popup_close_requested: false,
        }
    }

    fn allocate(&mut self) -> Option<u64> {
        match self.next_id.checked_add(1) {
            Some(id) => {
                self.next_id = id;
                Some(id)
            }
            None => {
                self.message = Some("操作次数已达上限，请重新打开应用。关闭请求仍会取消采集。");
                None
            }
        }
    }

    pub fn primary_label(&self) -> &'static str {
        match self.phase {
            Phase::Off => "开启采集",
            Phase::Starting => "取消开启",
            Phase::On => "关闭采集",
            Phase::Closing => "关闭中…",
            Phase::CloseFailed => "重试关闭",
        }
    }

    pub fn primary_enabled(&self) -> bool {
        self.available
            && !(self.popup_close_requested && self.phase == Phase::Off)
            && match self.phase {
                Phase::Starting => true,
                Phase::Closing => false,
                _ => self.pending_control.is_none(),
            }
    }

    pub fn primary(&mut self) -> Option<(u64, Control)> {
        if !self.primary_enabled() {
            return None;
        }
        let command = match self.phase {
            Phase::Off => Control::Enable,
            Phase::Starting | Phase::On => Control::Disable,
            Phase::CloseFailed => Control::RetryCleanup,
            Phase::Closing => return None,
        };
        let id = self.allocate()?;
        self.phase = match command {
            Control::Enable => Phase::Starting,
            Control::Disable | Control::RetryCleanup => Phase::Closing,
        };
        self.control_floor = id;
        self.pending_control = Some(id);
        self.message = None;
        Some((id, command))
    }

    pub fn acknowledge(&mut self, id: u64, status: Status, problem: Option<&'static str>) {
        if !self.available || id < self.control_floor {
            return;
        }
        if self.pending_control == Some(id) {
            self.pending_control = None;
        }
        self.phase = match status {
            Status::Stopped => Phase::Off,
            Status::Starting { revoked: false, .. } => Phase::Starting,
            Status::Starting { revoked: true, .. } | Status::Stopping { .. } => Phase::Closing,
            Status::Running { .. } => Phase::On,
            Status::CleanupFailed { .. } => Phase::CloseFailed,
        };
        if problem.is_some() {
            self.message = problem;
        }
    }

    pub fn control_failed(&mut self, id: u64) {
        if !self.available || id < self.control_floor {
            return;
        }
        self.pending_control = None;
        self.phase = Phase::CloseFailed;
        self.message = Some("尚未确认采集已关闭，请重试关闭。");
    }

    pub fn disconnected(&mut self) {
        self.available = false;
        self.pending_control = None;
        self.pending_io = None;
        self.pending_send_selection = None;
        if self.phase != Phase::Off {
            self.phase = Phase::CloseFailed;
        }
        self.message = Some(if self.phase == Phase::Off {
            "文本历史后台不可用，请重新打开应用。"
        } else {
            "文本历史后台已断开，关闭状态尚未确认。请重新打开应用。"
        });
    }

    pub fn can_query(&self) -> bool {
        self.available && !self.popup_close_requested && self.pending_io.is_none()
    }

    pub fn query(&mut self) -> Option<u64> {
        if !self.can_query() {
            return None;
        }
        let id = self.allocate()?;
        self.initial_query_requested = true;
        self.pending_io = Some(id);
        self.message = None;
        Some(id)
    }

    pub fn select(&mut self, id: &str) {
        if self.items.iter().any(|item| item.id == id) && self.selected.as_deref() != Some(id) {
            if let Some(pending) = self.pending_send_selection.as_mut() {
                pending.changed = true;
            }
            self.selected = Some(id.to_owned());
        }
    }

    pub fn can_remove(&self) -> bool {
        self.can_query()
            && !self.refresh_required
            && self
                .selected
                .as_ref()
                .is_some_and(|id| self.items.iter().any(|item| &item.id == id))
    }

    pub fn can_send(&self) -> bool {
        self.can_remove()
            && self.revision.is_some()
            && self.selected.as_ref().is_some_and(|selected| {
                self.items
                    .iter()
                    .any(|item| &item.id == selected && item.utf16_units > 0)
            })
    }

    pub fn begin_send_selection(&mut self) -> Option<(u64, HistorySendSelection)> {
        if !self.can_send() {
            return None;
        }
        let selected = self.selected.as_ref()?;
        let summary = self.items.iter().find(|item| &item.id == selected)?.clone();
        let revision = self.revision.clone()?;
        let id = self.query()?;
        self.pending_send_selection = Some(PendingSendSelection {
            request: id,
            id: summary.id.clone(),
            changed: false,
        });
        Some((id, HistorySendSelection { summary, revision }))
    }

    pub fn sent(&mut self, id: u64, result: Result<SendReceipt, SendFailure>) {
        if self.pending_io != Some(id) {
            return;
        }
        self.pending_io = None;
        let sent_selection = self.pending_send_selection.take();
        self.message = Some(match result {
            Ok(receipt) => {
                if sent_selection.is_some_and(|pending| {
                    pending.request == id
                        && !pending.changed
                        && self.selected.as_ref() == Some(&pending.id)
                }) {
                    self.selected = None;
                }
                receipt.message()
            }
            Err(error) => {
                self.refresh_required |= error.needs_refresh();
                error.message()
            }
        });
    }

    pub fn remove(&mut self) -> Option<(u64, HistorySummary)> {
        if !self.can_remove() {
            return None;
        }
        let selected_id = self.selected.as_ref()?;
        let selected = self
            .items
            .iter()
            .find(|item| &item.id == selected_id)?
            .clone();
        let id = self.query()?;
        // 删除是否生效不明时，必须先明确刷新，不能从旧列表重复删除。
        self.refresh_required = true;
        Some((id, selected))
    }

    pub fn listed(&mut self, id: Option<u64>, result: Result<HistoryList, &'static str>) {
        if !self.available {
            return;
        }
        match id {
            Some(id) if self.pending_io == Some(id) => self.pending_io = None,
            Some(_) => return,
            None if self.pending_io.is_some() || self.refresh_required => return,
            None => {}
        }
        match result {
            Ok(list) => {
                if let Some(previous) = self
                    .items
                    .iter()
                    .find(|item| Some(&item.id) == self.selected.as_ref())
                {
                    let current = list.items.iter().find(|item| item.id == previous.id);
                    if current != Some(previous) {
                        self.selected = None;
                        if current.is_some() {
                            self.message = Some("选中的历史项已被替换，请重新选择。");
                        }
                    }
                }
                self.revision = Some(list.revision);
                self.items = list.items;
                self.refresh_required = false;
            }
            Err(message) => {
                // 旧摘要保留供参考；读取失败后须先明确刷新再操作。
                self.refresh_required = true;
                self.message = Some(message);
            }
        }
    }
}

impl HistoryUiState {
    /// 只由显式页面进入调用；手动查询也算已请求，不因失败或重新进入自动重试。
    pub fn initial_query(&mut self) -> Option<u64> {
        if self.initial_query_requested {
            return None;
        }
        self.query()
    }
}

impl HistoryUiState {
    pub fn initial_list_loading(&self) -> bool {
        self.revision.is_none() && self.pending_io.is_some()
    }
}

impl HistoryUiState {
    pub fn request_popup_close(&mut self) -> Option<(u64, Control)> {
        self.popup_close_requested = true;
        match self.phase {
            Phase::Off | Phase::Closing => None,
            Phase::Starting | Phase::On | Phase::CloseFailed => self.primary(),
        }
    }

    pub fn popup_close_requested(&self) -> bool {
        self.popup_close_requested
    }

    pub fn popup_close_ready(&self) -> bool {
        self.popup_close_requested && self.phase == Phase::Off
    }
}

#[cfg(test)]
mod popup_close_tests {
    use super::*;

    #[test]
    fn popup_waits_for_own_stop_ack_and_retry_without_affecting_main_collector() {
        let mut main = HistoryUiState::new(true);
        let (main_start, command) = main.primary().unwrap();
        assert_eq!(command, Control::Enable);
        main.acknowledge(
            main_start,
            Status::Running {
                key: crate::clipboard_history_controller::SessionKey {
                    generation: 1,
                    listener_session: 2,
                },
            },
            None,
        );
        let mut popup = HistoryUiState::new(true);
        let (start, _) = popup.primary().unwrap();
        let (stop, command) = popup.request_popup_close().unwrap();
        assert_eq!(command, Control::Disable);
        assert!(!popup.popup_close_ready());
        popup.acknowledge(start, Status::Stopped, None);
        assert!(!popup.popup_close_ready());
        popup.acknowledge(
            stop,
            Status::CleanupFailed {
                key: crate::clipboard_history_controller::SessionKey {
                    generation: 1,
                    listener_session: 1,
                },
            },
            None,
        );
        assert!(!popup.popup_close_ready());
        let (retry, command) = popup.request_popup_close().unwrap();
        assert_eq!(command, Control::RetryCleanup);
        popup.acknowledge(retry, Status::Stopped, None);
        assert!(popup.popup_close_ready());
        assert!(popup.primary().is_none());
        assert_eq!(main.phase, Phase::On);
    }
}

#[cfg(test)]
mod initial_loading_label_tests {
    use super::*;

    #[test]
    fn pending_first_load_is_not_reported_as_empty_and_stale_reply_cannot_hide_loading() {
        let mut state = HistoryUiState::new(true);
        assert!(!state.initial_list_loading());
        let id = state.initial_query().unwrap();
        assert!(state.initial_list_loading());
        state.listed(
            Some(id + 1),
            Ok(HistoryList {
                items: vec![],
                revision: "0".into(),
            }),
        );
        assert!(state.initial_list_loading());
        state.listed(
            Some(id),
            Ok(HistoryList {
                items: vec![],
                revision: "0".into(),
            }),
        );
        assert!(!state.initial_list_loading());
    }

    #[test]
    fn failure_restores_manual_retry_caption_and_manual_retry_shows_loading_again() {
        let mut state = HistoryUiState::new(true);
        let id = state.initial_query().unwrap();
        state.listed(Some(id), Err("合成读取失败"));
        assert!(!state.initial_list_loading());
        assert!(state.can_query());
        assert!(state.initial_query().is_none());
        assert!(state.query().is_some());
        assert!(state.initial_list_loading());
    }
}

#[cfg(test)]
mod initial_load_tests {
    use super::*;

    fn listing() -> HistoryList {
        HistoryList {
            items: vec![HistorySummary {
                id: "10".into(),
                preview: "合成摘要".into(),
                utf16_units: 20,
                created_at: "1".into(),
                text_sha256: [0; 32],
            }],
            revision: "9007199254740993".into(),
        }
    }

    #[test]
    fn first_activation_requests_one_list_and_repeated_activation_cannot_duplicate_it() {
        let mut state = HistoryUiState::new(true);
        let id = state.initial_query().unwrap();
        assert_eq!(id, 1);
        assert_eq!(state.phase, Phase::Off);
        assert_eq!(state.pending_control, None);
        assert_eq!(state.pending_io, Some(id));
        for _ in 0..20 {
            assert!(state.initial_query().is_none());
        }
        assert_eq!(state.next_id, 1);
        assert!(!state.can_query());
    }

    #[test]
    fn initial_read_failure_stays_manual_and_refresh_can_succeed_with_new_request_id() {
        let mut state = HistoryUiState::new(true);
        let id = state.initial_query().unwrap();
        state.listed(Some(id), Err("无法读取文本历史，请稍后明确刷新。"));
        assert!(state.can_query());
        assert!(state.message.is_some());
        assert_eq!(state.phase, Phase::Off);
        for _ in 0..20 {
            assert!(state.initial_query().is_none());
        }
        let manual = state.query().unwrap();
        assert_eq!(manual, id + 1);
        state.listed(Some(manual), Ok(listing()));
        assert_eq!(state.items[0].id, "10");
        assert!(state.initial_query().is_none());
    }

    #[test]
    fn stale_and_unsolicited_replies_cannot_complete_pending_initial_list() {
        let mut state = HistoryUiState::new(true);
        let id = state.initial_query().unwrap();
        state.listed(Some(id + 1), Ok(listing()));
        state.listed(None, Ok(listing()));
        assert!(state.items.is_empty());
        assert_eq!(state.pending_io, Some(id));
        assert!(state.initial_query().is_none());
        state.listed(Some(id), Ok(listing()));
        assert_eq!(state.items.len(), 1);
    }

    #[test]
    fn capture_control_before_activation_does_not_consume_the_initial_query_gate() {
        let mut state = HistoryUiState::new(true);
        let (control, command) = state.primary().unwrap();
        assert_eq!(command, Control::Enable);
        let id = state.initial_query().unwrap();
        assert_ne!(control, id);
        assert_eq!(state.phase, Phase::Starting);
        assert_eq!(state.pending_control, Some(control));
        assert_eq!(state.pending_io, Some(id));
    }

    #[test]
    fn unavailable_or_disconnected_state_does_not_start_or_revive_initial_work() {
        let mut unavailable = HistoryUiState::new(false);
        assert!(unavailable.initial_query().is_none());
        assert_eq!(unavailable.next_id, 0);
        let mut state = HistoryUiState::new(true);
        let id = state.initial_query().unwrap();
        state.disconnected();
        state.listed(Some(id), Ok(listing()));
        assert!(state.initial_query().is_none());
        assert!(!state.available);
        assert!(state.items.is_empty());
        assert_eq!(state.phase, Phase::Off);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard_history_controller::SessionKey;

    fn running() -> Status {
        Status::Running {
            key: SessionKey {
                generation: 1,
                listener_session: 2,
            },
        }
    }

    fn list(ids: &[&str]) -> HistoryList {
        HistoryList {
            items: ids
                .iter()
                .map(|id| HistorySummary {
                    id: (*id).to_owned(),
                    preview: "合成摘要".into(),
                    utf16_units: 4,
                    created_at: "1".into(),
                    text_sha256: [0; 32],
                })
                .collect(),
            revision: "1".into(),
        }
    }

    #[test]
    fn constructor_is_off_and_has_no_pending_work() {
        let state = HistoryUiState::new(true);
        assert_eq!(state.phase, Phase::Off);
        assert!(state.pending_control.is_none());
        assert!(state.pending_io.is_none());
        assert!(state.items.is_empty());
    }

    #[test]
    fn enable_waits_for_backend_confirmation() {
        let mut state = HistoryUiState::new(true);
        let (id, command) = state.primary().unwrap();
        assert_eq!(command, Control::Enable);
        assert_eq!(state.phase, Phase::Starting);
        state.acknowledge(id, running(), None);
        assert_eq!(state.phase, Phase::On);
    }

    #[test]
    fn cancelled_start_ignores_stale_running_until_stop_confirmation() {
        let mut state = HistoryUiState::new(true);
        let (start, _) = state.primary().unwrap();
        let (stop, command) = state.primary().unwrap();
        assert_eq!(command, Control::Disable);
        state.acknowledge(start, running(), None);
        assert_eq!(state.phase, Phase::Closing);
        assert!(!state.primary_enabled());
        state.acknowledge(stop, Status::Stopped, None);
        assert_eq!(state.phase, Phase::Off);
    }

    #[test]
    fn close_failure_only_offers_cleanup_retry() {
        let mut state = HistoryUiState::new(true);
        let (id, _) = state.primary().unwrap();
        state.acknowledge(id, running(), None);
        let (id, _) = state.primary().unwrap();
        let Status::Running { key } = running() else {
            unreachable!()
        };
        state.acknowledge(id, Status::CleanupFailed { key }, Some("关闭失败"));
        assert_eq!(state.primary().unwrap().1, Control::RetryCleanup);
        assert_eq!(state.phase, Phase::Closing);
    }

    #[test]
    fn lost_worker_never_claims_an_active_session_is_off() {
        let mut state = HistoryUiState::new(true);
        state.primary().unwrap();
        state.disconnected();
        assert_eq!(state.phase, Phase::CloseFailed);
        assert!(!state.primary_enabled());
        assert!(!state.can_query());
    }

    #[test]
    fn control_error_requires_confirmed_cleanup() {
        let mut state = HistoryUiState::new(true);
        let (id, _) = state.primary().unwrap();
        state.control_failed(id);
        assert_eq!(state.phase, Phase::CloseFailed);
        assert_eq!(state.primary().unwrap().1, Control::RetryCleanup);
    }

    #[test]
    fn selection_is_stable_and_never_removes_by_itself() {
        let mut state = HistoryUiState::new(true);
        state.listed(None, Ok(list(&["1", "2"])));
        state.select("2");
        state.listed(None, Ok(list(&["2", "1"])));
        assert_eq!(state.selected.as_deref(), Some("2"));
        assert_eq!(state.items.len(), 2);
        assert!(state.pending_io.is_none());
        state.listed(None, Ok(list(&["1"])));
        assert!(state.selected.is_none());
    }

    #[test]
    fn stale_or_automatic_reply_cannot_complete_explicit_query() {
        let mut state = HistoryUiState::new(true);
        let id = state.query().unwrap();
        state.listed(Some(id + 1), Ok(list(&["1"])));
        state.listed(None, Ok(list(&["2"])));
        assert!(state.items.is_empty());
        assert!(!state.can_query());
        state.listed(Some(id), Ok(list(&["3"])));
        assert_eq!(state.items[0].id, "3");
        assert!(state.can_query());
    }

    #[test]
    fn unknown_removal_requires_explicit_refresh_before_retry() {
        let mut state = HistoryUiState::new(true);
        state.listed(None, Ok(list(&["1"])));
        state.select("1");
        let (id, item) = state.remove().unwrap();
        assert_eq!(item.id, "1");
        state.listed(Some(id), Err("删除是否生效尚不确定，请刷新。"));
        assert!(!state.can_remove());
        state.listed(None, Ok(list(&["1"])));
        assert!(!state.can_remove());
        let id = state.query().unwrap();
        state.listed(Some(id), Ok(list(&["1"])));
        assert!(state.can_remove());
    }

    #[test]
    fn removed_selection_is_cleared_by_refreshed_summary_list() {
        let mut state = HistoryUiState::new(true);
        state.listed(None, Ok(list(&["1", "2"])));
        state.select("1");
        let (id, _) = state.remove().unwrap();
        state.listed(Some(id), Ok(list(&["2"])));
        assert_eq!(state.items.len(), 1);
        assert!(state.selected.is_none());
    }

    #[test]
    fn send_selection_keeps_stable_id_and_displayed_revision() {
        let mut state = HistoryUiState::new(true);
        let mut listing = list(&["1", "2"]);
        listing.revision = "9007199254740993".into();
        state.listed(None, Ok(listing));
        state.select("2");
        let (id, selection) = state.begin_send_selection().unwrap();
        assert_eq!(selection.summary.id, "2");
        assert_eq!(selection.revision, "9007199254740993");
        assert!(!state.can_query());
        state.sent(id + 1, Ok(SendReceipt { utf16_units: 4 }));
        assert!(!state.can_query());
        state.sent(id, Ok(SendReceipt { utf16_units: 4 }));
        assert!(state.can_query());
        assert!(state.selected.is_none());
    }

    #[test]
    fn unknown_input_requires_explicit_refresh_and_keeps_no_success_claim() {
        use crate::clipboard_history_send::InputEffect;
        let mut state = HistoryUiState::new(true);
        state.listed(None, Ok(list(&["1"])));
        state.select("1");
        let (id, _) = state.begin_send_selection().unwrap();
        state.sent(
            id,
            Err(SendFailure {
                code: "input_result_unknown",
                effect: InputEffect::Unknown,
            }),
        );
        assert!(!state.can_remove());
        assert!(state.begin_send_selection().is_none());
        assert!(state.message.unwrap().contains("尚不确定"));
        state.listed(None, Ok(list(&["1"])));
        assert!(!state.can_remove());
        let id = state.query().unwrap();
        state.listed(Some(id), Ok(list(&["1"])));
        assert!(state.can_remove());
    }

    #[test]
    fn changing_selection_during_pending_send_does_not_change_request_identity() {
        let mut state = HistoryUiState::new(true);
        state.listed(None, Ok(list(&["1", "2"])));
        state.select("1");
        let (_, selection) = state.begin_send_selection().unwrap();
        state.select("2");
        assert_eq!(selection.summary.id, "1");
        assert_eq!(state.selected.as_deref(), Some("2"));
    }

    #[test]
    fn empty_text_can_be_deleted_but_cannot_start_a_send() {
        let mut state = HistoryUiState::new(true);
        let mut listing = list(&["1"]);
        listing.items[0].utf16_units = 0;
        listing.items[0].preview.clear();
        state.listed(None, Ok(listing));
        state.select("1");
        assert!(state.can_remove());
        assert!(!state.can_send());
        assert!(state.begin_send_selection().is_none());
        assert!(state.can_query());
    }
}
