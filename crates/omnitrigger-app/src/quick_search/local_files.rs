use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use omnitrigger_windows::{
    read_local_directory_with_options, DirectoryEntryType, DirectoryOrder, LocalDirectoryEntry,
    LocalDirectoryOptions, LocalStatError, MAX_DIRECTORY_ENTRIES,
};

use super::{title_rank, Destination, Results};

pub(super) const FOLDER_DEBOUNCE: Duration = Duration::from_millis(150);
const FOLDER_READ_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Default)]
pub(super) struct LocalFilesIndex {
    current: Option<DirectoryRequest>,
}

struct DirectoryRequest {
    load: Arc<DirectoryLoad>,
    entries: Option<Result<Vec<LocalDirectoryEntry>, String>>,
}

impl Drop for DirectoryRequest {
    fn drop(&mut self) {
        self.load.cancelled.store(true, Ordering::Relaxed);
    }
}

pub(super) struct DirectoryLoad {
    directory: String,
    pattern: Option<String>,
    order: DirectoryOrder,
    cancelled: AtomicBool,
}

impl DirectoryLoad {
    fn matches(&self, query: FileQuery<'_>) -> bool {
        self.directory == query.directory
            && self.pattern.as_deref() == wildcard_pattern(query.filter)
            && self.order == query.order
    }

    pub(super) fn read(&self) -> Result<Vec<LocalDirectoryEntry>, String> {
        let deadline = Instant::now() + FOLDER_READ_TIMEOUT;
        let options = LocalDirectoryOptions {
            max_entries: MAX_DIRECTORY_ENTRIES,
            order: self.order,
            metadata: false,
            entry_type: DirectoryEntryType::All,
            search_pattern: self.pattern.clone(),
        };
        read_local_directory_with_options(&self.directory, &options, || {
            let code = if self.cancelled.load(Ordering::Relaxed) {
                "cancelled"
            } else if Instant::now() >= deadline {
                "timeout"
            } else {
                return Ok(());
            };
            Err(LocalStatError {
                code,
                message: String::new(),
            })
        })
        .map_err(|error| error.message)
    }
}

impl LocalFilesIndex {
    pub(super) fn parent_query(&self, query: &str) -> Option<String> {
        let query = split_query(query)?;
        let request = self.current.as_ref()?;
        if !request.load.matches(query) {
            return None;
        }
        request.entries.as_ref()?.as_ref().ok()?;
        let parent = Path::new(query.directory).parent()?.to_str()?;
        Some(directory_query(parent))
    }

    pub(super) fn request(&mut self, query: Option<&str>) -> Option<Arc<DirectoryLoad>> {
        let Some(query) = query.and_then(split_query) else {
            self.current = None;
            return None;
        };
        if self
            .current
            .as_ref()
            .is_some_and(|request| request.load.matches(query))
        {
            return None;
        }
        let load = Arc::new(DirectoryLoad {
            directory: query.directory.to_owned(),
            pattern: wildcard_pattern(query.filter).map(str::to_owned),
            order: query.order,
            cancelled: AtomicBool::new(false),
        });
        self.current = Some(DirectoryRequest {
            load: load.clone(),
            entries: None,
        });
        Some(load)
    }

    pub(super) fn finish(
        &mut self,
        load: &Arc<DirectoryLoad>,
        entries: Result<Vec<LocalDirectoryEntry>, String>,
    ) -> bool {
        let Some(current) = self.current.as_mut() else {
            return false;
        };
        if !Arc::ptr_eq(load, &current.load) {
            return false;
        }
        current.entries = Some(entries);
        true
    }

    pub(super) fn results(&self, query: &str) -> Results {
        let Some(query) = split_query(query) else {
            return Results::LocalFilesPrompt;
        };
        let Some(entries) = self
            .current
            .as_ref()
            .filter(|request| request.load.matches(query))
            .and_then(|request| request.entries.as_ref())
        else {
            return Results::LocalFilesLoading;
        };
        let entries = match entries {
            Ok(entries) => entries,
            Err(error) => return Results::LocalFilesUnavailable(error.clone()),
        };
        let filter = if wildcard_pattern(query.filter).is_some() {
            String::new()
        } else {
            query.filter.to_lowercase()
        };
        let mut matches: Vec<_> = entries
            .iter()
            .filter(|entry| matches!(entry.kind, "file" | "directory"))
            .filter_map(|entry| {
                let name = entry.name.to_lowercase();
                filter
                    .split_whitespace()
                    .all(|keyword| name.contains(keyword))
                    .then(|| (title_rank(&name, &filter), entry))
            })
            .collect();
        if query.order == DirectoryOrder::Name {
            matches.sort_by_key(|(rank, _)| *rank);
        }
        Results::Destinations(
            matches
                .into_iter()
                .map(|(_, entry)| Destination::LocalFile {
                    name: entry.name.clone(),
                    path: entry.path.clone(),
                    is_directory: entry.kind == "directory",
                })
                .collect(),
        )
    }
}

#[derive(Clone, Copy)]
struct FileQuery<'a> {
    directory: &'a str,
    filter: &'a str,
    order: DirectoryOrder,
}

fn split_query(query: &str) -> Option<FileQuery<'_>> {
    let (directory, filter) = query.split_once('|')?;
    let directory = super::path_tools::strip_path_quotes(directory.trim());
    let (filter, order) = match filter.split_once('|') {
        Some((filter, order))
            if order.trim() == "最近修改" || order.trim().eq_ignore_ascii_case("modified") =>
        {
            (filter, DirectoryOrder::ModifiedDescending)
        }
        Some((filter, order)) if order.trim().is_empty() => (filter, DirectoryOrder::Name),
        Some(_) => return None,
        None => (filter, DirectoryOrder::Name),
    };
    (!directory.is_empty()).then_some(FileQuery {
        directory,
        filter: filter.trim(),
        order,
    })
}

fn wildcard_pattern(filter: &str) -> Option<&str> {
    filter.contains(['*', '?']).then_some(filter)
}

pub(super) fn directory_query(directory: &str) -> String {
    format!("文件: {directory} |")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, kind: &'static str) -> LocalDirectoryEntry {
        LocalDirectoryEntry {
            name: name.to_owned(),
            path: format!(r"C:\Work\{name}"),
            kind,
            reason: None,
            metadata: None,
        }
    }

    #[test]
    fn reads_only_explicit_directories_and_filters_the_current_snapshot_by_name() {
        let mut index = LocalFilesIndex::default();
        assert!(index.request(Some(r"C:\Work")).is_none());
        assert!(matches!(
            index.results(r"C:\Work"),
            Results::LocalFilesPrompt
        ));
        let load = index.request(Some(r"C:\Work |")).unwrap();
        assert_eq!(load.directory, r"C:\Work");
        assert!(matches!(
            index.results(r"C:\Work |"),
            Results::LocalFilesLoading
        ));
        assert!(index
            .request(Some(r#" "C:\Work" | BUDGET 2026 "#))
            .is_none());
        assert!(index.finish(
            &load,
            Ok(vec![
                entry("2026 Budget Draft.txt", "file"),
                entry("Budget 2026.xlsx", "file"),
                entry("Budget 2026", "directory"),
                entry("Budget 2026.url", "unavailable"),
                entry("Notes.txt", "file"),
            ])
        ));
        let Results::Destinations(items) = index.results(r"C:\Work | BUDGET 2026") else {
            panic!("expected local file results");
        };
        let paths: Vec<_> = items
            .iter()
            .filter_map(|item| match item {
                Destination::LocalFile { path, .. } => Some(path.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            paths,
            [
                r"C:\Work\Budget 2026",
                r"C:\Work\Budget 2026.xlsx",
                r"C:\Work\2026 Budget Draft.txt",
            ]
        );
        let results = Results::Destinations(items);
        assert_eq!(
            results.subdirectory_query(0).as_deref(),
            Some(r"文件: C:\Work\Budget 2026 |")
        );
        assert_eq!(results.subdirectory_query(1), None);
    }

    #[test]
    fn discards_stale_reads_when_a_directory_changes_or_search_closes() {
        let mut index = LocalFilesIndex::default();
        let first = index.request(Some(r"C:\Work |")).unwrap();
        let second = index.request(Some(r"C:\Other |")).unwrap();
        assert!(first.cancelled.load(Ordering::Relaxed));
        assert!(!index.finish(&first, Ok(vec![entry("old", "file")])));
        assert!(index.request(None).is_none());
        assert!(second.cancelled.load(Ordering::Relaxed));
        assert!(!index.finish(&second, Err("过期错误".to_owned())));
        let closing = index.request(Some(r"C:\Work |")).unwrap();
        drop(index);
        assert!(closing.cancelled.load(Ordering::Relaxed));
    }

    #[test]
    #[cfg(windows)]
    fn navigates_to_the_parent_of_the_current_directory() {
        let mut index = LocalFilesIndex::default();
        let child = r#""C:\Work\Budget 2026" | draft"#;
        let current = index.request(Some(child)).unwrap();
        assert_eq!(index.parent_query(child), None);
        assert!(index.finish(&current, Ok(Vec::new())));
        assert_eq!(
            index.parent_query(child).as_deref(),
            Some(r"文件: C:\Work |")
        );
        assert_eq!(index.parent_query(r"C:\Other |"), None);
        let root_query = r"C:\ |";
        let root = index.request(Some(root_query)).unwrap();
        assert!(index.finish(&root, Ok(Vec::new())));
        assert_eq!(index.parent_query(root_query), None);
    }

    #[test]
    fn newest_order_replaces_the_snapshot_and_takes_priority_over_name_relevance() {
        let mut index = LocalFilesIndex::default();
        let ordinary = index.request(Some(r"C:\Work | report")).unwrap();
        let sorted_query = r"C:\Work | report | 最近修改";
        let sorted = index.request(Some(sorted_query)).unwrap();
        assert_eq!(sorted.order, DirectoryOrder::ModifiedDescending);
        assert!(ordinary.cancelled.load(Ordering::Relaxed));
        assert!(index
            .request(Some(r"C:\Work | report | modified"))
            .is_none());
        assert!(index.finish(
            &sorted,
            Ok(vec![
                entry("Old unrelated.txt", "file"),
                entry("Z report.txt", "file"),
                entry("report.txt", "file"),
            ])
        ));
        let Results::Destinations(items) = index.results(sorted_query) else {
            panic!("预期按修改时间排列的文件列表");
        };
        assert!(matches!(
            items.as_slice(),
            [Destination::LocalFile { name: first, .. }, Destination::LocalFile { name: second, .. }]
            if first == "Z report.txt" && second == "report.txt"
        ));
    }

    #[test]
    fn delegates_explicit_filename_patterns_and_replaces_stale_results() {
        let mut index = LocalFilesIndex::default();
        let first = index.request(Some(r"C:\Work | *.txt")).unwrap();
        assert_eq!(first.pattern.as_deref(), Some("*.txt"));
        assert!(index.request(Some(r#" "C:\Work" | *.txt "#)).is_none());
        let second = index.request(Some(r"C:\Work | Budget ?.txt")).unwrap();
        assert!(first.cancelled.load(Ordering::Relaxed));
        assert_eq!(second.pattern.as_deref(), Some("Budget ?.txt"));
        assert!(!index.finish(&first, Ok(vec![entry("stale.txt", "file")])));
        assert!(index.finish(&second, Ok(vec![entry("Budget 1.txt", "file")])));
        assert!(matches!(
            index.results(r"C:\Work | Budget ?.txt"),
            Results::Destinations(items) if matches!(
                items.as_slice(), [Destination::LocalFile { name, .. }] if name == "Budget 1.txt"
            )
        ));
        let ordinary = index.request(Some(r"C:\Work | Budget")).unwrap();
        assert_eq!(ordinary.pattern, None);
    }
}
