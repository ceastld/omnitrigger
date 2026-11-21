use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use omnitrigger_windows::{
    read_local_directory, LocalDirectoryEntry, LocalStatError, MAX_DIRECTORY_ENTRIES,
};

use super::{title_rank, Destination, Results};

pub(super) const FOLDER_DEBOUNCE: Duration = Duration::from_millis(150);
const FOLDER_READ_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Default)]
pub(super) struct LocalFilesIndex {
    current: Option<DirectoryRequest>,
}

struct DirectoryRequest {
    directory: String,
    cancelled: Arc<AtomicBool>,
    entries: Option<Result<Vec<LocalDirectoryEntry>, String>>,
}

impl Drop for DirectoryRequest {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

impl LocalFilesIndex {
    pub(super) fn request(&mut self, query: Option<&str>) -> Option<(String, Arc<AtomicBool>)> {
        let Some((directory, _)) = query.and_then(split_query) else {
            self.current = None;
            return None;
        };
        if self
            .current
            .as_ref()
            .is_some_and(|request| request.directory == directory)
        {
            return None;
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        self.current = Some(DirectoryRequest {
            directory: directory.to_owned(),
            cancelled: cancelled.clone(),
            entries: None,
        });
        Some((directory.to_owned(), cancelled))
    }

    pub(super) fn finish(
        &mut self,
        cancelled: &Arc<AtomicBool>,
        entries: Result<Vec<LocalDirectoryEntry>, String>,
    ) -> bool {
        let Some(current) = self.current.as_mut() else {
            return false;
        };
        if !Arc::ptr_eq(cancelled, &current.cancelled) {
            return false;
        }
        current.entries = Some(entries);
        true
    }

    pub(super) fn results(&self, query: &str) -> Results {
        let Some((directory, filter)) = split_query(query) else {
            return Results::LocalFilesPrompt;
        };
        let Some(entries) = self
            .current
            .as_ref()
            .filter(|request| request.directory == directory)
            .and_then(|request| request.entries.as_ref())
        else {
            return Results::LocalFilesLoading;
        };
        let entries = match entries {
            Ok(entries) => entries,
            Err(error) => return Results::LocalFilesUnavailable(error.clone()),
        };
        let filter = filter.to_lowercase();
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
        matches.sort_by_key(|(rank, _)| *rank);
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

fn split_query(query: &str) -> Option<(&str, &str)> {
    let (directory, filter) = query.split_once('|')?;
    let directory = directory.trim();
    let directory = directory
        .strip_prefix('"')
        .and_then(|path| path.strip_suffix('"'))
        .unwrap_or(directory);
    (!directory.is_empty()).then_some((directory, filter.trim()))
}

pub(super) fn load_directory(
    directory: &str,
    cancelled: &AtomicBool,
) -> Result<Vec<LocalDirectoryEntry>, String> {
    let deadline = Instant::now() + FOLDER_READ_TIMEOUT;
    read_local_directory(directory, MAX_DIRECTORY_ENTRIES, || {
        let code = if cancelled.load(Ordering::Relaxed) {
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
        let (directory, token) = index.request(Some(r"C:\Work |")).unwrap();
        assert_eq!(directory, r"C:\Work");
        assert!(matches!(
            index.results(r"C:\Work |"),
            Results::LocalFilesLoading
        ));
        assert!(index
            .request(Some(r#" "C:\Work" | BUDGET 2026 "#))
            .is_none());
        assert!(index.finish(
            &token,
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
    }

    #[test]
    fn discards_stale_reads_when_a_directory_changes_or_search_closes() {
        let mut index = LocalFilesIndex::default();
        let (_, first) = index.request(Some(r"C:\Work |")).unwrap();
        let (_, second) = index.request(Some(r"C:\Other |")).unwrap();
        assert!(first.load(Ordering::Relaxed));
        assert!(!index.finish(&first, Ok(vec![entry("old", "file")])));
        assert!(index.request(None).is_none());
        assert!(second.load(Ordering::Relaxed));
        assert!(!index.finish(&second, Err("过期错误".to_owned())));
        let (_, closing) = index.request(Some(r"C:\Work |")).unwrap();
        drop(index);
        assert!(closing.load(Ordering::Relaxed));
    }
}
