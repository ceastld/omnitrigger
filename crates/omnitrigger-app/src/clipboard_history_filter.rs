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
