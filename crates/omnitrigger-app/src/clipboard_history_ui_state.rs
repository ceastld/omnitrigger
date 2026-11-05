//! 文本历史页的纯界面状态。关闭确认来自后台，不由按钮点击或超时推断。

use crate::clipboard_history_controller::Status;
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

pub(super) struct HistoryUiState {
    pub phase: Phase,
    pub items: Vec<HistorySummary>,
    pub selected: Option<String>,
    pub message: Option<&'static str>,
    pub available: bool,
    next_id: u64,
    control_floor: u64,
    pending_control: Option<u64>,
    pending_io: Option<u64>,
    refresh_required: bool,
}

impl HistoryUiState {
    pub fn new(available: bool) -> Self {
        Self {
            phase: Phase::Off,
            items: Vec::new(),
            selected: None,
            message: (!available).then_some("文本历史暂时不可用。"),
            available,
            next_id: 0,
            control_floor: 0,
            pending_control: None,
            pending_io: None,
            refresh_required: false,
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
        if id < self.control_floor {
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
        if id < self.control_floor {
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
        self.available && self.pending_io.is_none()
    }

    pub fn query(&mut self) -> Option<u64> {
        if !self.can_query() {
            return None;
        }
        let id = self.allocate()?;
        self.pending_io = Some(id);
        self.message = None;
        Some(id)
    }

    pub fn select(&mut self, id: &str) {
        if self.items.iter().any(|item| item.id == id) {
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

    pub fn remove(&mut self) -> Option<(u64, String)> {
        if !self.can_remove() {
            return None;
        }
        let selected = self.selected.clone()?;
        let id = self.query()?;
        // 删除是否生效不明时，必须先明确刷新，不能从旧列表重复删除。
        self.refresh_required = true;
        Some((id, selected))
    }

    pub fn listed(&mut self, id: Option<u64>, result: Result<HistoryList, &'static str>) {
        match id {
            Some(id) if self.pending_io == Some(id) => self.pending_io = None,
            Some(_) => return,
            None if self.pending_io.is_some() || self.refresh_required => return,
            None => {}
        }
        match result {
            Ok(list) => {
                self.items = list.items;
                if !self
                    .selected
                    .as_ref()
                    .is_some_and(|id| self.items.iter().any(|item| &item.id == id))
                {
                    self.selected = None;
                }
                self.refresh_required = false;
            }
            Err(message) => self.message = Some(message),
        }
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
        assert_eq!(item, "1");
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
}
