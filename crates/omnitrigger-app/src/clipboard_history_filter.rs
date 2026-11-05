//! 只在已加载摘要中筛选，不读取历史正文或访问存储。

use crate::clipboard_history_store::HistorySummary;

pub(super) const MAX_QUERY_CHARACTERS: usize = 256;

#[derive(Clone, Default, PartialEq, Eq)]
pub(super) struct SummaryQuery {
    terms: Vec<String>,
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
        })
    }

    fn matches(&self, item: &HistorySummary) -> bool {
        let preview = item.preview.to_lowercase();
        self.terms.iter().all(|term| preview.contains(term))
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
        let target = match current {
            None => 0,
            Some(position) => match direction {
                SummaryDirection::Previous => position.checked_sub(1)?,
                SummaryDirection::Next => position.checked_add(1)?,
            },
        };
        visible.get(target).map(|&index| items[index].id.as_str())
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
