use super::{Destination, SearchTrigger, PAGES, PAGE_RESULT_LIMIT};
use crate::Page;

/// 当前工作台会话中，从快速搜索打开的页面或触发器。
#[derive(Clone, Default)]
pub(crate) struct RecentDestinations(Vec<Destination>);

impl RecentDestinations {
    pub(crate) fn record(&mut self, destination: &Destination) {
        let item = match destination {
            Destination::TextHistoryQuery(_) => Destination::Page(Page::TextHistory),
            Destination::ActionLibraryQuery(_) | Destination::ActionLibraryEntry { .. } => {
                Destination::Page(Page::Actions)
            }
            Destination::Wheel(_) => Destination::Page(Page::Wheels),
            Destination::Preset(_) => Destination::Page(Page::Presets),
            item => item.clone(),
        };
        self.0.retain(|existing| existing != &item);
        self.0.insert(0, item);
        self.0.truncate(PAGE_RESULT_LIMIT);
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
            if items.len() == PAGE_RESULT_LIMIT {
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
