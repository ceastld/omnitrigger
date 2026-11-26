use std::{
    cell::RefCell,
    fs::File,
    io::{ErrorKind, Read},
    path::{Path, PathBuf},
    rc::Rc,
};

use serde::{Deserialize, Serialize};

use super::{
    csv_tools::csv_cell,
    examples::{matching_examples, search_key},
    value_history::csv_export_filter,
    Destination, InputSuggestion, Results, SearchTrigger, DESTINATION_PAGE_SIZE,
    INPUT_CHARACTER_LIMIT, PAGES,
};
use crate::Page;

const EXAMPLE_HISTORY_LIMIT: usize = 12;
const BOOKMARK_LIMIT: usize = 12;
const BOOKMARK_NAME_LIMIT: usize = 80;
const BOOKMARK_FILE: &str = "quick-search-bookmarks.json";
const BOOKMARK_FILE_LIMIT: u64 =
    (BOOKMARK_LIMIT * ((INPUT_CHARACTER_LIMIT + BOOKMARK_NAME_LIMIT) * 6 + 32) + 8) as u64;

#[derive(Clone, Default)]
pub(super) struct QueryBookmarks(Rc<RefCell<BookmarkStore>>);

#[derive(Default)]
struct BookmarkStore {
    path: Option<PathBuf>,
    items: Vec<Bookmark>,
    load_error: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum Bookmark {
    Unnamed(String),
    Named { query: String, name: String },
}

impl Bookmark {
    fn query(&self) -> &str {
        match self {
            Self::Unnamed(query) | Self::Named { query, .. } => query,
        }
    }

    fn name(&self) -> &str {
        match self {
            Self::Unnamed(_) => "",
            Self::Named { name, .. } => name,
        }
    }
}

#[derive(PartialEq, Eq)]
enum BookmarkChange {
    Changed,
    Unchanged,
    Full,
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
        self.0
            .borrow()
            .items
            .iter()
            .any(|item| item.query() == query)
    }

    pub(super) fn toggle(&self, query: &str) -> Result<bool, String> {
        let add = !self.contains(query);
        self.update(|items| Ok(toggle_bookmark_item(items, query, add)))
            .map(|outcome| outcome != BookmarkChange::Full)
    }

    pub(super) fn swap_visible_items(&self, first: &str, second: &str) -> Result<(), String> {
        self.update(|items| {
            let first = items.iter().position(|item| item.query() == first);
            let second = items.iter().position(|item| item.query() == second);
            let (Some(first), Some(second)) = (first, second) else {
                return Ok(BookmarkChange::Unchanged);
            };
            if first >= second {
                return Ok(BookmarkChange::Unchanged);
            }
            items.swap(first, second);
            Ok(BookmarkChange::Changed)
        })
        .map(|_| ())
    }

    pub(super) fn rename(&self, query: &str, name: &str) -> Result<(), String> {
        let name = name.trim();
        if !valid_bookmark_name(name) {
            return Err("收藏名最多 80 个字符，请填写一行文字".into());
        }
        self.update(|items| {
            let item = items
                .iter_mut()
                .find(|item| item.query() == query)
                .ok_or_else(|| "这条查询已从收藏中移除".to_owned())?;
            if item.name() == name {
                return Ok(BookmarkChange::Unchanged);
            }
            *item = if name.is_empty() {
                Bookmark::Unnamed(query.to_owned())
            } else {
                Bookmark::Named {
                    query: query.to_owned(),
                    name: name.to_owned(),
                }
            };
            Ok(BookmarkChange::Changed)
        })
        .map(|_| ())
    }

    fn update(
        &self,
        edit: impl FnOnce(&mut Vec<Bookmark>) -> Result<BookmarkChange, String>,
    ) -> Result<BookmarkChange, String> {
        let mut store = self.0.borrow_mut();
        let Some(path) = &store.path else {
            return edit(&mut store.items);
        };
        check_bookmark_path(path)?;
        check_bookmark_path(&crate::settings::config_lock_path(path)?)?;
        let _lock = crate::settings::lock_config(path, "搜索收藏")?;
        let mut items = read_bookmark_file(path)?;
        let outcome = edit(&mut items)?;
        if outcome == BookmarkChange::Changed {
            let bytes =
                serde_json::to_vec(&items).map_err(|error| format!("生成搜索收藏失败：{error}"))?;
            crate::settings::atomic_write(path, &bytes)?;
        }
        store.items = items;
        store.load_error = None;
        Ok(outcome)
    }

    pub(super) fn results(&self, query: &str) -> Results {
        if let Some(error) = self.load_error() {
            return Results::BookmarksUnavailable(error);
        }
        let Some(filter) = csv_export_filter(query) else {
            return Results::Bookmarks(self.matching(query));
        };
        let mut csv = String::from("名称,查询");
        for item in self.matching(filter) {
            csv.push_str(&format!(
                "\r\n{},{}",
                csv_cell(&item.result),
                csv_cell(&item.expression),
            ));
        }
        Results::table_value(Ok(csv))
    }

    pub(super) fn matching(&self, filter: &str) -> Vec<InputSuggestion> {
        let filter = search_key(filter);
        self.0
            .borrow()
            .items
            .iter()
            .filter(|item| {
                let query = search_key(item.query());
                let name = search_key(item.name());
                filter
                    .split_whitespace()
                    .all(|term| query.contains(term) || name.contains(term))
            })
            .map(|item| InputSuggestion {
                expression: item.query().to_owned(),
                label: None,
                result: item.name().to_owned(),
            })
            .collect()
    }
}

fn toggle_bookmark_item(items: &mut Vec<Bookmark>, query: &str, add: bool) -> BookmarkChange {
    match (add, items.iter().position(|item| item.query() == query)) {
        (true, None) if items.len() == BOOKMARK_LIMIT => BookmarkChange::Full,
        (true, None) => {
            items.insert(0, Bookmark::Unnamed(query.to_owned()));
            BookmarkChange::Changed
        }
        (false, Some(index)) => {
            items.remove(index);
            BookmarkChange::Changed
        }
        _ => BookmarkChange::Unchanged,
    }
}

fn valid_bookmark_name(name: &str) -> bool {
    name.trim() == name
        && name.chars().count() <= BOOKMARK_NAME_LIMIT
        && !name
            .chars()
            .any(|character| character.is_control() || matches!(character, '\u{2028}' | '\u{2029}'))
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

fn read_bookmark_file(path: &Path) -> Result<Vec<Bookmark>, String> {
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
    let items: Vec<Bookmark> = crate::local_files::parse_config_json_bytes(&bytes)
        .map_err(|error| format!("搜索收藏格式无效：{error}"))?;
    if items.len() > BOOKMARK_LIMIT
        || items.iter().enumerate().any(|(index, item)| {
            let query = item.query();
            query.trim().is_empty()
                || query.chars().take(INPUT_CHARACTER_LIMIT + 1).count() > INPUT_CHARACTER_LIMIT
                || !valid_bookmark_name(item.name())
                || items[..index].iter().any(|other| other.query() == query)
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
    fn persists_visible_swaps_and_reloads_during_concurrent_changes() {
        let root = tempfile::tempdir().unwrap();
        let bindings = root.path().join("bindings.json");
        let first = QueryBookmarks::from_bindings_path(&bindings);
        for query in ["11 + 11", "2**3", "11 + 22"] {
            first.toggle(query).unwrap();
        }
        first.rename("11 + 11", "保留名称").unwrap();
        let stale = QueryBookmarks::from_bindings_path(&bindings);
        first.swap_visible_items("11 + 22", "11 + 11").unwrap();
        stale.swap_visible_items("11 + 22", "11 + 11").unwrap();
        let reopened = QueryBookmarks::from_bindings_path(&bindings);
        let items = reopened.matching("");
        assert_eq!(
            items
                .iter()
                .map(|item| item.expression.as_str())
                .collect::<Vec<_>>(),
            ["11 + 11", "2**3", "11 + 22"]
        );
        assert_eq!(items[0].result, "保留名称");
        assert_eq!(stale.matching("１１")[0].expression, "11 + 11");
        reopened.toggle("11 + 22").unwrap();
        stale.swap_visible_items("11 + 11", "11 + 22").unwrap();
        assert!(!stale.contains("11 + 22"));
    }

    #[test]
    fn names_old_bookmarks_and_merges_with_a_later_addition() {
        let root = tempfile::tempdir().unwrap();
        let bindings = root.path().join("bindings.json");
        std::fs::write(bindings.with_file_name(BOOKMARK_FILE), br#"["2**10"]"#).unwrap();
        let original = QueryBookmarks::from_bindings_path(&bindings);
        let other = QueryBookmarks::from_bindings_path(&bindings);
        assert_eq!(original.matching("")[0].expression, "2**10");
        assert!(other.toggle("3**4").unwrap());
        assert!(original.rename("2**10", "  每周汇总  ").is_ok());
        assert!(original.rename("2**10", "两\n行").is_err());
        assert!(original
            .rename("2**10", &"长".repeat(BOOKMARK_NAME_LIMIT + 1))
            .is_err());
        let reopened = QueryBookmarks::from_bindings_path(&bindings);
        let matches = reopened.matching("每周 ２＊＊１０");
        assert_eq!(matches[0].expression, "2**10");
        assert_eq!(matches[0].result, "每周汇总");
        assert!(reopened.contains("3**4"));
        assert!(other.toggle("2**10").unwrap());
        assert!(reopened.rename("2**10", "已移除").is_err());
        assert!(reopened.rename("3**4", " 留着 ").is_ok());
        assert!(reopened.rename("3**4", "").is_ok());
        let final_store = QueryBookmarks::from_bindings_path(&bindings);
        assert_eq!(final_store.matching("")[0].expression, "3**4");
        assert!(final_store.matching("")[0].result.is_empty());
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
