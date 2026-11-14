//! 有界查询与摘要/正文命中ID呈现；本模块不读取正文或访问存储。

use crate::clipboard_history_store::{text_matches_terms, HistorySummary};

pub(super) const MAX_QUERY_CHARACTERS: usize = 256;

#[derive(Clone, Default, PartialEq, Eq)]
pub(super) struct SummaryQuery {
    terms: Vec<String>,
    ids: Option<Vec<String>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FilterError {
    TooLong,
}

/// 只解释摘要及长度元数据；摘要全空白不能推断整段正文也全空白。
pub(super) fn summary_label(item: &HistorySummary) -> String {
    if item.utf16_units == 0 {
        "（空文本）".into()
    } else if item.preview.trim().is_empty() {
        "（空白摘要）".into()
    } else {
        item.preview.clone()
    }
}

impl SummaryQuery {
    pub fn new(query: &str) -> Result<Self, FilterError> {
        if query.chars().take(MAX_QUERY_CHARACTERS + 1).count() > MAX_QUERY_CHARACTERS {
            return Err(FilterError::TooLong);
        }
        Ok(Self {
            terms: query.split_whitespace().map(str::to_lowercase).collect(),
            ids: None,
        })
    }

    pub fn terms(&self) -> &[String] {
        &self.terms
    }
    fn for_ids(ids: Vec<String>) -> Self {
        Self {
            terms: Vec::new(),
            ids: Some(ids),
        }
    }

    fn matches(&self, item: &HistorySummary) -> bool {
        if let Some(ids) = &self.ids {
            return ids.contains(&item.id);
        }
        text_matches_terms(&item.preview, &self.terms)
    }

    pub fn matching_indices(&self, items: &[HistorySummary]) -> Vec<usize> {
        items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| self.matches(item).then_some(index))
            .collect()
    }

    pub fn selected_visible(&self, items: &[HistorySummary], selected: Option<&str>) -> bool {
        selected.is_some_and(|id| items.iter().any(|item| item.id == id && self.matches(item)))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SummaryDirection {
    Previous,
    Next,
    First,
    Last,
}

impl SummaryQuery {
    /// 沿已加载的可见摘要移动；不排序、不读取正文，抵达边界时不循环。
    pub fn adjacent_id<'a>(
        &self,
        items: &'a [HistorySummary],
        selected: Option<&str>,
        direction: SummaryDirection,
    ) -> Option<&'a str> {
        let visible = self.matching_indices(items);
        let current =
            selected.and_then(|id| visible.iter().position(|&index| items[index].id == id));
        let target = match direction {
            SummaryDirection::First => 0,
            SummaryDirection::Last => visible.len().checked_sub(1)?,
            SummaryDirection::Previous | SummaryDirection::Next => match current {
                None => 0,
                Some(position) if direction == SummaryDirection::Previous => {
                    position.checked_sub(1)?
                }
                Some(position) => position.checked_add(1)?,
            },
        };
        visible.get(target).map(|&index| items[index].id.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SummaryKeyboardAction {
    Navigate(SummaryDirection),
    ArmSend,
}

/// 调用方负责列表焦点；能力门与现有按钮一致，不从按键推断可以发送。
pub(super) fn summary_keyboard_action(
    key: &str,
    is_held: bool,
    modified: bool,
    can_navigate: bool,
    can_send: bool,
) -> Option<SummaryKeyboardAction> {
    if modified {
        return None;
    }
    match key {
        "up" if can_navigate => Some(SummaryKeyboardAction::Navigate(SummaryDirection::Previous)),
        "down" if can_navigate => Some(SummaryKeyboardAction::Navigate(SummaryDirection::Next)),
        "home" if can_navigate => Some(SummaryKeyboardAction::Navigate(SummaryDirection::First)),
        "end" if can_navigate => Some(SummaryKeyboardAction::Navigate(SummaryDirection::Last)),
        "enter" if !is_held && can_send => Some(SummaryKeyboardAction::ArmSend),
        _ => None,
    }
}

/// 仅保存按下时的摘要身份；不读取正文，也不在按下边沿发送。
#[derive(Default)]
pub(super) struct SummaryEnterLatch {
    selected: Option<(String, String)>,
}
impl SummaryEnterLatch {
    pub fn arm(&mut self, id: &str, revision: &str) {
        self.selected = Some((id.to_owned(), revision.to_owned()));
    }
    pub fn cancel(&mut self) {
        self.selected = None;
    }
    pub fn is_armed(&self) -> bool {
        self.selected.is_some()
    }
    pub fn release(
        &mut self,
        selected: Option<&str>,
        revision: Option<&str>,
        allowed: bool,
    ) -> bool {
        let Some((id, version)) = self.selected.take() else {
            return false;
        };
        allowed && selected == Some(id.as_str()) && revision == Some(version.as_str())
    }
}

/// 单个历史页的查询参数与可见结果；不保存正文，也不隐式发起重试。
#[derive(Default)]
pub(super) struct HistoryQueryView {
    draft: SummaryQuery,
    visible: SummaryQuery,
    pending: Option<(u64, SummaryQuery)>,
    applied_revision: Option<String>,
}
impl HistoryQueryView {
    pub fn draft(&self) -> &SummaryQuery {
        &self.draft
    }
    pub fn set_draft(&mut self, query: SummaryQuery) {
        self.draft = query.clone();
        self.visible = query;
        self.applied_revision = None;
    }
    pub fn begin(&mut self, id: u64) -> SummaryQuery {
        self.pending = Some((id, self.draft.clone()));
        self.draft.clone()
    }
    pub fn is_pending(&self, id: u64) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|(pending, _)| *pending == id)
    }
    pub fn loading(&self) -> bool {
        self.pending.is_some()
    }
    pub fn full_text_active(&self) -> bool {
        self.applied_revision.is_some()
    }
    pub fn complete(
        &mut self,
        id: u64,
        revision: &str,
        current: Option<&str>,
        accepted: bool,
        ids: Vec<String>,
    ) -> bool {
        if !self.is_pending(id) {
            return false;
        }
        let (_, query) = self.pending.take().expect("已核对本次查询");
        if !accepted || current != Some(revision) || query != self.draft {
            self.visible = self.draft.clone();
            self.applied_revision = None;
            return false;
        }
        self.visible = SummaryQuery::for_ids(ids);
        self.applied_revision = Some(revision.to_owned());
        true
    }
    pub fn failed(&mut self, id: u64) {
        if self.is_pending(id) {
            self.pending = None;
            self.visible = self.draft.clone();
            self.applied_revision = None;
        }
    }
    pub fn invalidate(&mut self, current: Option<&str>) -> bool {
        if self
            .applied_revision
            .as_deref()
            .is_some_and(|revision| Some(revision) != current)
        {
            self.visible = self.draft.clone();
            self.applied_revision = None;
            true
        } else {
            false
        }
    }
    pub fn disconnected(&mut self) {
        self.pending = None;
        self.visible = self.draft.clone();
        self.applied_revision = None;
    }
    pub fn matching_indices(&self, items: &[HistorySummary]) -> Vec<usize> {
        self.visible.matching_indices(items)
    }
    pub fn selected_visible(&self, items: &[HistorySummary], selected: Option<&str>) -> bool {
        self.visible.selected_visible(items, selected)
    }
    pub fn adjacent_id<'a>(
        &self,
        items: &'a [HistorySummary],
        selected: Option<&str>,
        direction: SummaryDirection,
    ) -> Option<&'a str> {
        self.visible.adjacent_id(items, selected, direction)
    }
}

#[cfg(test)]
mod navigation_tests {
    use super::*;
    use crate::clipboard_history_send::{InputEffect, SendFailure};
    use crate::clipboard_history_store::HistoryList;
    use crate::clipboard_history_ui_state::HistoryUiState;

    fn summaries() -> Vec<HistorySummary> {
        [
            ("100", "Rust 最近", "5"),
            ("20", "其他摘要", "99"),
            ("3", "Rust 更早", "10"),
        ]
        .into_iter()
        .map(|(id, preview, created_at)| HistorySummary {
            id: id.into(),
            preview: preview.into(),
            utf16_units: 20,
            created_at: created_at.into(),
            text_sha256: [0; 32],
        })
        .collect()
    }

    #[test]
    fn either_direction_without_selection_starts_at_first_visible_summary() {
        let items = summaries();
        for direction in [SummaryDirection::Previous, SummaryDirection::Next] {
            assert_eq!(
                SummaryQuery::default().adjacent_id(&items, None, direction),
                Some("100")
            );
        }
    }

    #[test]
    fn movement_uses_loaded_order_instead_of_sorting_ids_or_timestamps() {
        let items = summaries();
        let query = SummaryQuery::default();
        assert_eq!(
            query.adjacent_id(&items, Some("100"), SummaryDirection::Next),
            Some("20")
        );
        assert_eq!(
            query.adjacent_id(&items, Some("20"), SummaryDirection::Previous),
            Some("100")
        );
        assert_eq!(
            query.adjacent_id(&items, Some("20"), SummaryDirection::Next),
            Some("3")
        );
    }

    #[test]
    fn first_and_last_boundaries_do_not_wrap_or_keep_a_false_next_target() {
        let items = summaries();
        let query = SummaryQuery::default();
        assert_eq!(
            query.adjacent_id(&items, Some("100"), SummaryDirection::Previous),
            None
        );
        assert_eq!(
            query.adjacent_id(&items, Some("3"), SummaryDirection::Next),
            None
        );
    }

    #[test]
    fn filtering_skips_hidden_rows_in_both_directions() {
        let items = summaries();
        let query = SummaryQuery::new("rust").unwrap();
        assert_eq!(
            query.adjacent_id(&items, Some("100"), SummaryDirection::Next),
            Some("3")
        );
        assert_eq!(
            query.adjacent_id(&items, Some("3"), SummaryDirection::Previous),
            Some("100")
        );
    }

    #[test]
    fn missing_or_filtered_out_selection_restarts_at_first_visible_id() {
        let items = summaries();
        let query = SummaryQuery::new("rust").unwrap();
        for selected in [Some("20"), Some("deleted"), None] {
            for direction in [SummaryDirection::Previous, SummaryDirection::Next] {
                assert_eq!(query.adjacent_id(&items, selected, direction), Some("100"));
            }
        }
    }

    #[test]
    fn empty_list_and_no_matches_have_no_navigation_target() {
        let items = summaries();
        for direction in [SummaryDirection::Previous, SummaryDirection::Next] {
            assert_eq!(
                SummaryQuery::default().adjacent_id(&[], None, direction),
                None
            );
            assert_eq!(
                SummaryQuery::new("不匹配")
                    .unwrap()
                    .adjacent_id(&items, Some("100"), direction),
                None
            );
        }
    }

    #[test]
    fn one_visible_row_is_selectable_once_and_has_no_adjacent_target() {
        let items = summaries();
        let query = SummaryQuery::new("其他").unwrap();
        assert_eq!(
            query.adjacent_id(&items, None, SummaryDirection::Next),
            Some("20")
        );
        assert_eq!(
            query.adjacent_id(&items, Some("20"), SummaryDirection::Previous),
            None
        );
        assert_eq!(
            query.adjacent_id(&items, Some("20"), SummaryDirection::Next),
            None
        );
    }

    #[test]
    fn refresh_resolves_current_id_again_and_navigation_does_not_mutate_summaries() {
        let mut items = summaries();
        let query = SummaryQuery::default();
        items.remove(1);
        let before = items.clone();
        assert_eq!(
            query.adjacent_id(&items, Some("3"), SummaryDirection::Previous),
            Some("100")
        );
        assert_eq!(
            query.adjacent_id(&items, Some("20"), SummaryDirection::Next),
            Some("100")
        );
        assert_eq!(items, before);
    }

    #[test]
    fn empty_text_can_be_navigated_but_does_not_become_sendable() {
        let mut items = summaries();
        items[0].preview.clear();
        items[0].utf16_units = 0;
        let mut state = HistoryUiState::new(true);
        state.listed(
            None,
            Ok(HistoryList {
                items,
                revision: "100".into(),
            }),
        );
        let id = SummaryQuery::default()
            .adjacent_id(&state.items, None, SummaryDirection::Next)
            .unwrap()
            .to_owned();
        state.select(&id);
        assert!(!state.can_send());
        assert!(state.begin_send_selection().is_none());
    }

    #[test]
    fn moved_selection_reuses_real_ui_selection_and_displayed_revision() {
        let mut state = HistoryUiState::new(true);
        state.listed(
            None,
            Ok(HistoryList {
                items: summaries(),
                revision: "9007199254740993".into(),
            }),
        );
        state.select("100");
        let id = SummaryQuery::new("rust")
            .unwrap()
            .adjacent_id(
                &state.items,
                state.selected.as_deref(),
                SummaryDirection::Next,
            )
            .unwrap()
            .to_owned();
        state.select(&id);
        let (_, selection) = state.begin_send_selection().unwrap();
        assert_eq!(selection.summary.id, "3");
        assert_eq!(selection.revision, "9007199254740993");
        assert!(!state.can_query());
    }

    #[test]
    fn moving_after_unknown_send_never_clears_real_ui_refresh_barrier() {
        let mut state = HistoryUiState::new(true);
        state.listed(
            None,
            Ok(HistoryList {
                items: summaries(),
                revision: "100".into(),
            }),
        );
        state.select("100");
        let (request, _) = state.begin_send_selection().unwrap();
        state.sent(
            request,
            Err(SendFailure {
                code: "input_result_unknown",
                effect: InputEffect::Unknown,
            }),
        );
        let id = SummaryQuery::default()
            .adjacent_id(
                &state.items,
                state.selected.as_deref(),
                SummaryDirection::Next,
            )
            .unwrap()
            .to_owned();
        state.select(&id);
        assert!(!state.can_send());
        assert!(state.begin_send_selection().is_none());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<HistorySummary> {
        [
            "Rust 输入引擎",
            "GPUI 输入面板 😀",
            "a.*b / literal",
            "ÉCOLE",
            "",
        ]
        .iter()
        .enumerate()
        .map(|(index, preview)| HistorySummary {
            id: (index + 1).to_string(),
            preview: (*preview).into(),
            utf16_units: 100,
            created_at: "10".into(),
            text_sha256: [0; 32],
        })
        .collect()
    }

    #[test]
    fn empty_or_whitespace_query_keeps_original_order() {
        for query in ["", "  \t\u{3000}"] {
            assert_eq!(
                SummaryQuery::new(query).unwrap().matching_indices(&items()),
                [0, 1, 2, 3, 4]
            );
        }
    }

    #[test]
    fn words_match_summary_case_insensitively_and_in_any_order() {
        assert_eq!(
            SummaryQuery::new("输入 rUsT")
                .unwrap()
                .matching_indices(&items()),
            [0]
        );
        assert_eq!(
            SummaryQuery::new("😀  gpui")
                .unwrap()
                .matching_indices(&items()),
            [1]
        );
        assert_eq!(
            SummaryQuery::new("école")
                .unwrap()
                .matching_indices(&items()),
            [3]
        );
    }

    #[test]
    fn every_word_is_required_and_punctuation_is_literal() {
        assert!(SummaryQuery::new("Rust GPUI")
            .unwrap()
            .matching_indices(&items())
            .is_empty());
        assert_eq!(
            SummaryQuery::new(".*").unwrap().matching_indices(&items()),
            [2]
        );
    }

    #[test]
    fn identifier_timestamp_and_full_text_length_are_not_search_fields() {
        for query in ["100", "10", "5"] {
            assert!(SummaryQuery::new(query)
                .unwrap()
                .matching_indices(&items())
                .is_empty());
        }
    }

    #[test]
    fn hidden_missing_or_absent_selection_cannot_be_deleted() {
        let query = SummaryQuery::new("Rust").unwrap();
        assert!(query.selected_visible(&items(), Some("1")));
        assert!(!query.selected_visible(&items(), Some("2")));
        assert!(!query.selected_visible(&items(), Some("missing")));
        assert!(!query.selected_visible(&items(), None));
    }

    #[test]
    fn filtering_never_modifies_loaded_summaries() {
        let items = items();
        let before = items.clone();
        assert_eq!(
            SummaryQuery::new("输入").unwrap().matching_indices(&items),
            [0, 1]
        );
        assert_eq!(items, before);
    }

    #[test]
    fn limit_counts_unicode_characters_and_rejects_extra_input() {
        assert!(SummaryQuery::new(&"😀".repeat(MAX_QUERY_CHARACTERS)).is_ok());
        assert_eq!(
            SummaryQuery::new(&"😀".repeat(MAX_QUERY_CHARACTERS + 1)).err(),
            Some(FilterError::TooLong)
        );
    }

    #[test]
    fn equivalent_spacing_and_case_do_not_change_selection_semantics() {
        assert!(
            SummaryQuery::new("  RUST\t输入 ").unwrap() == SummaryQuery::new("rust 输入").unwrap()
        );
    }

    #[test]
    fn matching_does_not_assume_unicode_normalization() {
        assert!(SummaryQuery::new("e\u{301}cole")
            .unwrap()
            .matching_indices(&items())
            .is_empty());
    }

    #[test]
    fn empty_and_blank_previews_have_readable_labels_without_reading_body() {
        let mut item = items().pop().unwrap();
        item.utf16_units = 0;
        assert_eq!(summary_label(&item), "（空文本）");
        item.preview = " \u{3000} ".into();
        item.utf16_units = 1000;
        assert_eq!(summary_label(&item), "（空白摘要）");
        assert_eq!(item.utf16_units, 1000);
    }

    #[test]
    fn readable_label_preserves_meaningful_preview_whitespace() {
        let mut item = items().remove(0);
        item.preview = "  Rust 😀  ".into();
        assert_eq!(summary_label(&item), "  Rust 😀  ");
    }

    #[test]
    fn synthetic_placeholder_is_not_a_search_field_and_empty_query_can_select_it() {
        let item = items().pop().unwrap();
        assert!(SummaryQuery::new("空白摘要")
            .unwrap()
            .matching_indices(std::slice::from_ref(&item))
            .is_empty());
        assert!(
            SummaryQuery::default().selected_visible(std::slice::from_ref(&item), Some(&item.id))
        );
    }
}

#[cfg(test)]
mod keyboard_operation_tests {
    use super::*;
    use crate::clipboard_history_store::HistoryList;
    use crate::clipboard_history_ui_state::HistoryUiState;

    fn items() -> Vec<HistorySummary> {
        [
            ("100", "Rust 第一条", 20),
            ("20", "其他摘要", 20),
            ("3", "Rust 最后一条", 20),
        ]
        .into_iter()
        .map(|(id, preview, utf16_units)| HistorySummary {
            id: id.into(),
            preview: preview.into(),
            utf16_units,
            created_at: "1".into(),
            text_sha256: [0; 32],
        })
        .collect()
    }

    fn loaded(items: Vec<HistorySummary>) -> HistoryUiState {
        let mut state = HistoryUiState::new(true);
        let id = state.query().unwrap();
        state.listed(
            Some(id),
            Ok(HistoryList {
                items,
                revision: "9007199254740993".into(),
            }),
        );
        state
    }

    fn action(
        key: &str,
        state: &HistoryUiState,
        query: &SummaryQuery,
    ) -> Option<SummaryKeyboardAction> {
        summary_keyboard_action(
            key,
            false,
            false,
            state.can_query(),
            state.can_send() && query.selected_visible(&state.items, state.selected.as_deref()),
        )
    }

    #[test]
    fn only_unmodified_list_keys_route_and_held_enter_never_sends() {
        for key in ["up", "down", "home", "end", "enter"] {
            assert!(summary_keyboard_action(key, false, true, true, true).is_none());
            assert!(summary_keyboard_action(key, false, false, false, false).is_none());
        }
        assert!(summary_keyboard_action("enter", true, false, true, true).is_none());
        assert_eq!(
            summary_keyboard_action("down", true, false, true, false),
            Some(SummaryKeyboardAction::Navigate(SummaryDirection::Next))
        );
        for key in ["a", "backspace", "delete", "escape", "left", "right"] {
            assert!(summary_keyboard_action(key, false, false, true, true).is_none());
        }
    }

    #[test]
    fn home_end_choose_visible_edges_without_selection_or_id_sorting() {
        let items = items();
        let before = items.clone();
        let query = SummaryQuery::new("rust").unwrap();
        assert_eq!(
            query.adjacent_id(&items, None, SummaryDirection::First),
            Some("100")
        );
        assert_eq!(
            query.adjacent_id(&items, None, SummaryDirection::Last),
            Some("3")
        );
        assert_eq!(
            query.adjacent_id(&items, Some("20"), SummaryDirection::Last),
            Some("3")
        );
        assert_eq!(
            query.adjacent_id(&items, Some("3"), SummaryDirection::First),
            Some("100")
        );
        assert_eq!(items, before);
        for direction in [SummaryDirection::First, SummaryDirection::Last] {
            assert!(query.adjacent_id(&[], None, direction).is_none());
            assert!(SummaryQuery::new("不存在")
                .unwrap()
                .adjacent_id(&items, None, direction)
                .is_none());
        }
    }

    #[test]
    fn key_navigation_reuses_stable_id_and_displayed_revision_before_send() {
        let mut state = loaded(items());
        let query = SummaryQuery::new("rust").unwrap();
        state.select("100");
        let Some(SummaryKeyboardAction::Navigate(direction)) = action("end", &state, &query) else {
            panic!("End不可达");
        };
        let id = query
            .adjacent_id(&state.items, state.selected.as_deref(), direction)
            .unwrap()
            .to_owned();
        state.select(&id);
        assert_eq!(
            action("enter", &state, &query),
            Some(SummaryKeyboardAction::ArmSend)
        );
        let (_, selection) = state.begin_send_selection().unwrap();
        assert_eq!(selection.summary.id, "3");
        assert_eq!(selection.revision, "9007199254740993");
        // 发送已开始时第二次Enter既不重发，也不能变成取消按钮。
        assert!(action("enter", &state, &query).is_none());
        assert!(action("down", &state, &query).is_none());
    }

    #[test]
    fn hidden_or_empty_selection_cannot_send_but_can_be_navigated() {
        let mut state = loaded(items());
        state.select("20");
        let query = SummaryQuery::new("rust").unwrap();
        assert!(action("enter", &state, &query).is_none());
        assert!(matches!(
            action("down", &state, &query),
            Some(SummaryKeyboardAction::Navigate(_))
        ));
        let mut empty = items();
        empty[0].preview.clear();
        empty[0].utf16_units = 0;
        let mut state = loaded(empty);
        state.select("100");
        assert!(action("enter", &state, &SummaryQuery::default()).is_none());
        assert!(state.can_remove());
    }

    #[test]
    fn failed_refresh_blocks_enter_until_explicit_successful_refresh() {
        let mut state = loaded(items());
        state.select("100");
        let query = SummaryQuery::default();
        let id = state.query().unwrap();
        state.listed(Some(id), Err("合成读取失败"));
        assert!(action("enter", &state, &query).is_none());
        let id = state.query().unwrap();
        state.listed(
            Some(id),
            Ok(HistoryList {
                items: items(),
                revision: "9007199254740994".into(),
            }),
        );
        assert_eq!(
            action("enter", &state, &query),
            Some(SummaryKeyboardAction::ArmSend)
        );
    }

    #[test]
    fn closing_or_disconnected_ui_has_no_keyboard_actions() {
        let mut state = loaded(items());
        state.select("100");
        let query = SummaryQuery::default();
        state.request_popup_close();
        for key in ["up", "down", "home", "end", "enter"] {
            assert!(action(key, &state, &query).is_none());
        }
        let mut state = loaded(items());
        state.select("100");
        state.disconnected();
        for key in ["up", "down", "home", "end", "enter"] {
            assert!(action(key, &state, &query).is_none());
        }
    }
    #[test]
    fn enter_only_releases_once_for_the_same_id_revision_and_live_permission() {
        let mut latch = SummaryEnterLatch::default();
        assert!(!latch.release(Some("100"), Some("7"), true));
        latch.arm("100", "7");
        assert!(latch.is_armed());
        assert!(latch.release(Some("100"), Some("7"), true));
        assert!(!latch.release(Some("100"), Some("7"), true));
        for (id, revision, allowed) in [("20", "7", true), ("100", "8", true), ("100", "7", false)]
        {
            latch.arm("100", "7");
            assert!(!latch.release(Some(id), Some(revision), allowed));
            assert!(!latch.is_armed());
            assert!(!latch.release(Some("100"), Some("7"), true));
        }
    }

    #[test]
    fn blur_or_selection_change_cancels_enter_and_requires_a_fresh_press() {
        let mut latch = SummaryEnterLatch::default();
        latch.arm("100", "7");
        latch.cancel();
        assert!(!latch.release(Some("100"), Some("7"), true));
        latch.arm("20", "8");
        assert!(latch.release(Some("20"), Some("8"), true));
    }
}

#[cfg(test)]
mod full_text_view_tests {
    use super::*;
    use crate::clipboard_history_store::HistoryList;
    use crate::clipboard_history_ui_state::HistoryUiState;

    fn items() -> Vec<HistorySummary> {
        [("9", "只有开头"), ("2", "另一条摘要")]
            .into_iter()
            .map(|(id, preview)| HistorySummary {
                id: id.into(),
                preview: preview.into(),
                utf16_units: 100,
                created_at: "1".into(),
                text_sha256: [0; 32],
            })
            .collect()
    }
    fn body_query() -> HistoryQueryView {
        let mut view = HistoryQueryView::default();
        view.set_draft(SummaryQuery::new("尾部词").unwrap());
        view
    }
    #[test]
    fn body_id_match_is_visible_without_replacing_canonical_preview_or_revision() {
        let mut state = HistoryUiState::new(true);
        let id = state.query().unwrap();
        let mut view = body_query();
        view.begin(id);
        let summaries = items();
        let before = summaries.clone();
        state.listed(
            Some(id),
            Ok(HistoryList {
                items: summaries,
                revision: "9007199254740993".into(),
            }),
        );
        assert!(view.complete(
            id,
            "9007199254740993",
            state.revision.as_deref(),
            state.can_query(),
            vec!["2".into()]
        ));
        assert_eq!(view.matching_indices(&state.items), vec![1]);
        state.select("2");
        assert!(view.selected_visible(&state.items, state.selected.as_deref()));
        let (_, selection) = state.begin_send_selection().unwrap();
        assert_eq!(selection.revision, "9007199254740993");
        assert_eq!(selection.summary, before[1]);
        assert_eq!(state.items, before);
    }
    #[test]
    fn empty_body_matches_mean_no_results_and_do_not_fall_back_to_all() {
        let mut view = body_query();
        view.begin(1);
        assert!(view.complete(1, "1", Some("1"), true, vec![]));
        assert!(view.full_text_active());
        assert!(view.matching_indices(&items()).is_empty());
        assert!(!view.selected_visible(&items(), Some("9")));
    }
    #[test]
    fn changed_query_or_revision_rejects_old_read_and_does_not_requeue() {
        let mut view = body_query();
        view.begin(1);
        view.set_draft(SummaryQuery::new("另一条").unwrap());
        assert!(!view.complete(1, "1", Some("1"), true, vec!["9".into()]));
        assert!(!view.loading());
        assert!(!view.full_text_active());
        assert_eq!(view.matching_indices(&items()), vec![1]);
        view.begin(2);
        assert!(!view.complete(2, "1", Some("2"), true, vec!["9".into()]));
        assert!(!view.loading());
        assert!(!view.full_text_active());
    }
    #[test]
    fn stale_request_cannot_consume_current_search_or_make_a_hidden_selection_visible() {
        let mut view = body_query();
        view.begin(7);
        assert!(!view.complete(6, "1", Some("1"), true, vec!["9".into()]));
        view.failed(6);
        assert!(view.is_pending(7));
        assert!(view.complete(7, "1", Some("1"), true, vec!["2".into()]));
        assert!(!view.selected_visible(&items(), Some("9")));
        assert_eq!(
            view.adjacent_id(&items(), None, SummaryDirection::Next),
            Some("2")
        );
    }
    #[test]
    fn history_revision_change_invalidates_body_ids_then_returns_to_preview_filter() {
        let mut view = body_query();
        view.begin(1);
        assert!(view.complete(1, "1", Some("1"), true, vec!["2".into()]));
        assert!(!view.invalidate(Some("1")));
        assert!(view.full_text_active());
        assert!(view.invalidate(Some("2")));
        assert!(!view.full_text_active());
        assert!(view.matching_indices(&items()).is_empty());
        assert!(!view.loading());
    }
    #[test]
    fn failed_or_disconnected_search_never_applies_results_or_automatically_restarts() {
        let mut view = body_query();
        view.begin(1);
        view.failed(1);
        assert!(!view.loading());
        assert!(!view.full_text_active());
        view.begin(2);
        view.disconnected();
        assert!(!view.complete(2, "1", Some("1"), true, vec!["9".into()]));
        assert!(!view.loading());
        assert!(!view.full_text_active());
        view.begin(3);
        assert!(!view.complete(3, "1", Some("1"), false, vec!["9".into()]));
    }
}
