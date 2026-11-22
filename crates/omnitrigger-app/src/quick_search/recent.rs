use std::{
    cell::RefCell,
    fs::File,
    io::{ErrorKind, Read},
    path::{Path, PathBuf},
    rc::Rc,
};

use super::{
    examples::{matching_examples, search_key},
    Destination, InputSuggestion, SearchTrigger, DESTINATION_PAGE_SIZE, INPUT_CHARACTER_LIMIT,
    PAGES,
};
use crate::Page;

const EXAMPLE_HISTORY_LIMIT: usize = 12;
const BOOKMARK_LIMIT: usize = 12;
const BOOKMARK_FILE: &str = "quick-search-bookmarks.json";
const BOOKMARK_FILE_LIMIT: u64 = (BOOKMARK_LIMIT * (INPUT_CHARACTER_LIMIT * 6 + 4) + 8) as u64;

#[derive(Clone, Default)]
pub(super) struct QueryBookmarks(Rc<RefCell<BookmarkStore>>);

#[derive(Default)]
struct BookmarkStore {
    path: Option<PathBuf>,
    items: Vec<String>,
    load_error: Option<String>,
}

impl QueryBookmarks {
    pub(super) fn from_bindings_path(bindings_path: &Path) -> Self {
        let path = bindings_path.with_file_name(BOOKMARK_FILE);
        let (items, load_error) = match read_bookmark_file(&path) {
            Ok(items) => (items, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        Self(Rc::new(RefCell::new(BookmarkStore {
            path: Some(path),
            items,
            load_error,
        })))
    }

    pub(super) fn load_error(&self) -> Option<String> {
        self.0.borrow().load_error.clone()
    }

    pub(super) fn contains(&self, query: &str) -> bool {
        self.0.borrow().items.iter().any(|item| item == query)
    }

    pub(super) fn toggle(&self, query: &str) -> Result<bool, String> {
        let mut store = self.0.borrow_mut();
        let add = !store.items.iter().any(|item| item == query);
        let Some(path) = &store.path else {
            return Ok(edit_bookmark_items(&mut store.items, query, add));
        };
        check_bookmark_path(path)?;
        check_bookmark_path(&crate::settings::config_lock_path(path)?)?;
        let _lock = crate::settings::lock_config(path, "搜索收藏")?;
        let mut items = read_bookmark_file(path)?;
        let changed = items.iter().any(|item| item == query) != add;
        let accepted = edit_bookmark_items(&mut items, query, add);
        if accepted && changed {
            let bytes =
                serde_json::to_vec(&items).map_err(|error| format!("生成搜索收藏失败：{error}"))?;
            crate::settings::atomic_write(path, &bytes)?;
        }
        store.items = items;
        store.load_error = None;
        Ok(accepted)
    }

    pub(super) fn matching(&self, filter: &str) -> Vec<InputSuggestion> {
        let filter = search_key(filter);
        self.0
            .borrow()
            .items
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

fn edit_bookmark_items(items: &mut Vec<String>, query: &str, add: bool) -> bool {
    match (add, items.iter().position(|item| item == query)) {
        (true, None) if items.len() == BOOKMARK_LIMIT => return false,
        (true, None) => items.insert(0, query.to_owned()),
        (false, Some(index)) => {
            items.remove(index);
        }
        _ => {}
    }
    true
}

fn check_bookmark_path(path: &Path) -> Result<(), String> {
    let path = std::path::absolute(path).map_err(|error| format!("搜索收藏路径不可用：{error}"))?;
    if crate::local_files::is_cloud_path(&path) {
        return Err("搜索收藏需要本地配置目录".into());
    }
    for ancestor in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
        match ancestor.symlink_metadata() {
            Ok(info)
                if crate::local_files::is_plain(&info)
                    && if ancestor == path.as_path() {
                        info.is_file()
                    } else {
                        info.is_dir()
                    } => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            _ => return Err("搜索收藏文件或目录不可用".into()),
        }
    }
    Ok(())
}

fn read_bookmark_file(path: &Path) -> Result<Vec<String>, String> {
    check_bookmark_path(path)?;
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("读取搜索收藏失败：{error}")),
    };
    let mut bytes = Vec::new();
    file.take(BOOKMARK_FILE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("读取搜索收藏失败：{error}"))?;
    if bytes.len() as u64 > BOOKMARK_FILE_LIMIT {
        return Err("搜索收藏文件超过大小上限".into());
    }
    let items: Vec<String> = crate::local_files::parse_config_json_bytes(&bytes)
        .map_err(|error| format!("搜索收藏格式无效：{error}"))?;
    if items.len() > BOOKMARK_LIMIT
        || items.iter().enumerate().any(|(index, query)| {
            query.trim().is_empty()
                || query.chars().take(INPUT_CHARACTER_LIMIT + 1).count() > INPUT_CHARACTER_LIMIT
                || items[..index].contains(query)
        })
    {
        return Err("搜索收藏包含超出上限或重复的查询".into());
    }
    Ok(items)
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
        assert!(bookmarks.toggle(multiline).unwrap());
        assert!(bookmarks.toggle("2**10").unwrap());
        assert_eq!(reopened.matching("ｃｓｖ ００１")[0].expression, multiline);
        assert!(reopened.contains("2**10"));
        assert!(reopened.toggle("2**10").unwrap());
        assert!(!bookmarks.contains("2**10"));
        assert!(reopened.matching("1024").is_empty());
        for index in 1..BOOKMARK_LIMIT {
            assert!(bookmarks.toggle(&format!("1+{index}")).unwrap());
        }
        assert!(!bookmarks.toggle("8+9999").unwrap());
        assert_eq!(reopened.matching("").len(), BOOKMARK_LIMIT);
        assert_eq!(reopened.matching("").last().unwrap().expression, multiline);
    }

    #[test]
    fn persists_explicit_bookmarks_and_merges_updates_from_other_instances() {
        let root = tempfile::tempdir().unwrap();
        let bindings = root.path().join("new-profile").join("bindings.json");
        let first = QueryBookmarks::from_bindings_path(&bindings);
        let stale = QueryBookmarks::from_bindings_path(&bindings);
        assert!(first.load_error().is_none());
        let multiline = "CSV: 选择 1\n键,数值\n001,2  ";
        assert!(first.toggle(multiline).unwrap());
        assert!(stale.toggle("2**10").unwrap());
        let reopened = QueryBookmarks::from_bindings_path(&bindings);
        assert_eq!(reopened.matching("csv")[0].expression, multiline);
        assert!(reopened.contains("2**10"));
        assert!(first.toggle(multiline).unwrap());
        let reopened = QueryBookmarks::from_bindings_path(&bindings);
        assert!(!reopened.contains(multiline));
        assert!(reopened.contains("2**10"));
    }

    #[test]
    fn leaves_malformed_or_busy_bookmark_files_untouched() {
        let root = tempfile::tempdir().unwrap();
        let bindings = root.path().join("bindings.json");
        let path = bindings.with_file_name(BOOKMARK_FILE);
        let bookmarks = QueryBookmarks::from_bindings_path(&bindings);
        assert!(bookmarks.toggle("2**10").unwrap());
        let lock = crate::settings::lock_config(&path, "搜索收藏").unwrap();
        assert!(bookmarks.toggle("2**10").is_err());
        drop(lock);
        assert!(bookmarks.contains("2**10"));
        std::fs::write(&path, b"broken").unwrap();
        let corrupt = QueryBookmarks::from_bindings_path(&bindings);
        assert!(corrupt.load_error().is_some());
        assert!(corrupt.toggle("2**10").is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"broken");
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
