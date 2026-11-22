use std::{cell::RefCell, rc::Rc};

use super::{
    examples::{matching_examples, search_key},
    Destination, InputSuggestion, SearchTrigger, DESTINATION_PAGE_SIZE, PAGES,
};
use crate::Page;

const EXAMPLE_HISTORY_LIMIT: usize = 12;
const BOOKMARK_LIMIT: usize = 12;

#[derive(Clone, Default)]
pub(super) struct QueryBookmarks(Rc<RefCell<Vec<String>>>);

impl QueryBookmarks {
    pub(super) fn contains(&self, query: &str) -> bool {
        self.0.borrow().iter().any(|item| item == query)
    }

    pub(super) fn toggle(&self, query: &str) -> bool {
        let mut items = self.0.borrow_mut();
        if let Some(index) = items.iter().position(|item| item == query) {
            items.remove(index);
        } else if items.len() < BOOKMARK_LIMIT {
            items.insert(0, query.to_owned());
        } else {
            return false;
        }
        true
    }

    pub(super) fn matching(&self, filter: &str) -> Vec<InputSuggestion> {
        let filter = search_key(filter);
        self.0
            .borrow()
            .iter()
            .filter(|query| {
                let key = search_key(query);
                filter.split_whitespace().all(|term| key.contains(term))
            })
            .map(|query| InputSuggestion {
                expression: query.clone(),
                label: None,
                result: String::new(),
            })
            .collect()
    }
}

#[derive(Clone, Default)]
pub(super) struct RecentExamples(Rc<RefCell<Vec<usize>>>);

impl RecentExamples {
    pub(super) fn record(&self, index: usize) {
        let mut recent = self.0.borrow_mut();
        recent.retain(|&existing| existing != index);
        recent.insert(0, index);
        recent.truncate(EXAMPLE_HISTORY_LIMIT);
    }

    pub(super) fn matching(&self, query: &str) -> Vec<usize> {
        let recent = self.0.borrow();
        if query.trim().is_empty() {
            return recent.clone();
        }
        let matching = matching_examples(query);
        recent
            .iter()
            .copied()
            .filter(|index| matching.contains(index))
            .collect()
    }
}

/// 当前工作台会话中，从快速搜索打开的页面或触发器。
#[derive(Clone, Default)]
pub(crate) struct RecentDestinations(Vec<Destination>);

impl RecentDestinations {
    pub(crate) fn record(&mut self, destination: &Destination) {
        let item = match destination {
            Destination::InstalledApp(_)
            | Destination::KnownFolder(_)
            | Destination::WindowsSetting { .. }
            | Destination::LocalFile { .. }
            | Destination::OpenWindow(_) => return,
            Destination::TextHistoryQuery(_) => Destination::Page(Page::TextHistory),
            Destination::TriggerHistoryQuery(_) => Destination::Page(Page::History),
            Destination::ActionLibraryQuery(_) | Destination::ActionLibraryEntry { .. } => {
                Destination::Page(Page::Actions)
            }
            Destination::Wheel(_) | Destination::WheelSlot { .. } => {
                Destination::Page(Page::Wheels)
            }
            Destination::Preset(_) => Destination::Page(Page::Presets),
            Destination::SettingsSection(_) => Destination::Page(Page::Settings),
            item => item.clone(),
        };
        self.0.retain(|existing| existing != &item);
        self.0.insert(0, item);
        self.0.truncate(DESTINATION_PAGE_SIZE);
    }

    pub(crate) fn forget_trigger(&mut self, id: u64) {
        self.0.retain(|item| item != &Destination::Trigger(id));
    }

    pub(super) fn suggestions(&self, triggers: &[SearchTrigger]) -> Vec<Destination> {
        let mut items: Vec<_> = self
            .0
            .iter()
            .filter(|destination| match destination {
                Destination::Trigger(id) => triggers.iter().any(|trigger| trigger.id() == *id),
                _ => true,
            })
            .cloned()
            .collect();
        for (page, _) in PAGES {
            if items.len() == DESTINATION_PAGE_SIZE {
                break;
            }
            let item = Destination::Page(page);
            if !items.contains(&item) {
                items.push(item);
            }
        }
        items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bookmarks_preserve_query_text_and_keep_items_until_explicitly_removed() {
        let bookmarks = QueryBookmarks::default();
        let reopened = bookmarks.clone();
        let multiline = "CSV: 选择 1\n键,数值\n001,2  ";
        assert!(bookmarks.toggle(multiline));
        assert!(bookmarks.toggle("2**10"));
        assert_eq!(reopened.matching("ｃｓｖ ００１")[0].expression, multiline);
        assert!(reopened.contains("2**10"));
        assert!(reopened.toggle("2**10"));
        assert!(!bookmarks.contains("2**10"));
        assert!(reopened.matching("1024").is_empty());
        for index in 1..BOOKMARK_LIMIT {
            assert!(bookmarks.toggle(&format!("1+{index}")));
        }
        assert!(!bookmarks.toggle("8+9999"));
        assert_eq!(reopened.matching("").len(), BOOKMARK_LIMIT);
        assert_eq!(reopened.matching("").last().unwrap().expression, multiline);
    }

    #[test]
    fn example_history_is_shared_filters_in_recent_order_and_deduplicates() {
        let examples = super::super::EXAMPLES;
        let position = |query| {
            examples
                .iter()
                .position(|example| example.query == query)
                .unwrap()
        };
        let power = position("2**10");
        let alternate = position("分数: 计算 isqrt(2**100 - 1)");
        let history = RecentExamples::default();
        let reopened = history.clone();
        history.record(power);
        history.record(alternate);
        assert_eq!(reopened.matching("２＊＊１０"), [alternate, power]);
        history.record(power);
        assert_eq!(reopened.matching(""), [power, alternate]);
        assert!(reopened.matching("无对应记录").is_empty());
        for index in 0..EXAMPLE_HISTORY_LIMIT + 2 {
            history.record(index);
        }
        assert_eq!(
            history.matching(""),
            (2..EXAMPLE_HISTORY_LIMIT + 2).rev().collect::<Vec<_>>()
        );
    }

    #[test]
    fn empty_search_prioritizes_recent_valid_destinations() {
        let bindings = crate::settings::defaults();
        let trigger = SearchTrigger::from_binding(&bindings[0]);
        let item = Destination::Trigger(trigger.id());
        let mut recent = RecentDestinations::default();
        recent.record(&Destination::Page(Page::Settings));
        recent.record(&item);
        recent.record(&Destination::TextHistoryQuery("仅作为查询词".into()));
        recent.record(&Destination::Page(Page::Settings));

        assert_eq!(
            recent.suggestions(std::slice::from_ref(&trigger)),
            [
                Destination::Page(Page::Settings),
                Destination::Page(Page::TextHistory),
                item.clone(),
                Destination::Page(Page::Triggers),
            ],
        );
        assert_eq!(
            recent.suggestions(&[]),
            [
                Destination::Page(Page::Settings),
                Destination::Page(Page::TextHistory),
                Destination::Page(Page::Triggers),
                Destination::Page(Page::Actions),
            ],
        );
        recent.forget_trigger(trigger.id());
        assert!(!recent.suggestions(&[trigger]).contains(&item));
    }
}
