use std::{
    collections::HashMap,
    fmt::Write as _,
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

use super::{
    conversion::exact_byte_quantity, csv_tools::csv_cell, title_rank, Destination, Results,
};

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
    entry_type: DirectoryEntryType,
    cancelled: AtomicBool,
}

impl DirectoryLoad {
    fn matches(&self, query: FileQuery<'_>) -> bool {
        self.directory == query.directory
            && self.pattern.as_deref() == wildcard_pattern(query.filter)
            && self.order == query.order
            && self.entry_type == query.entry_type
    }

    pub(super) fn read(&self) -> Result<Vec<LocalDirectoryEntry>, String> {
        let deadline = Instant::now() + FOLDER_READ_TIMEOUT;
        let options = LocalDirectoryOptions {
            max_entries: MAX_DIRECTORY_ENTRIES,
            order: self.order,
            metadata: false,
            entry_type: self.entry_type,
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
    pub(super) fn invalidate(&mut self) {
        self.current = None;
    }

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
            self.invalidate();
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
            entry_type: query.entry_type,
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
            .filter(|entry| {
                matches!(entry.kind, "file" | "directory")
                    && query.size_bounds.is_none_or(|(minimum, maximum)| {
                        entry
                            .size_bytes
                            .is_some_and(|size| (minimum..=maximum).contains(&size))
                    })
            })
            .filter_map(|entry| {
                let name = entry.name.to_lowercase();
                filter
                    .split_whitespace()
                    .all(|keyword| name.contains(keyword))
                    .then(|| (title_rank(&name, &filter), entry))
            })
            .collect();
        if query.output == FileOutput::ExtensionCounts {
            let names = matches
                .iter()
                .filter(|(_, entry)| entry.kind == "file")
                .map(|(_, entry)| entry.name.as_str());
            return Results::text_value(Ok(extension_counts(names)));
        }
        if query.order == DirectoryOrder::Name {
            matches.sort_by_key(|(rank, _)| *rank);
        }
        if query.output == FileOutput::Json
            || (matches!(query.output, FileOutput::Lines | FileOutput::Names)
                && !matches.is_empty())
        {
            let values = matches
                .iter()
                .map(|(_, entry)| {
                    if query.output == FileOutput::Names {
                        entry.name.as_str()
                    } else {
                        entry.path.as_str()
                    }
                })
                .collect::<Vec<_>>();
            let output = if query.output == FileOutput::Json {
                serde_json::to_string(&values).expect("路径字符串可以序列化为 JSON")
            } else {
                values.join("\n")
            };
            return Results::text_value(Ok(output));
        }
        Results::Destinations(
            matches
                .into_iter()
                .map(|(_, entry)| Destination::LocalFile {
                    name: entry.name.clone(),
                    path: entry.path.clone(),
                    is_directory: entry.kind == "directory",
                    size_bytes: entry.size_bytes,
                })
                .collect(),
        )
    }
}

fn extension_counts<'a>(names: impl Iterator<Item = &'a str>) -> String {
    let mut counts = HashMap::new();
    for name in names {
        let extension = Path::new(name)
            .extension()
            .and_then(|part| part.to_str())
            .unwrap_or_default()
            .to_lowercase();
        *counts.entry(extension).or_insert(0usize) += 1;
    }
    let mut rows = counts.into_iter().collect::<Vec<_>>();
    rows.sort_unstable_by(|(left, left_count), (right, right_count)| {
        right_count.cmp(left_count).then_with(|| left.cmp(right))
    });
    let mut output = String::from("扩展名,数量");
    for (extension, count) in rows {
        let label = if extension.is_empty() {
            "无扩展名".to_owned()
        } else {
            format!(".{extension}")
        };
        write!(output, "\r\n{},{count}", csv_cell(&label)).expect("写入字符串不会失败");
    }
    output
}

#[derive(Clone, Copy)]
struct FileQuery<'a> {
    directory: &'a str,
    filter: &'a str,
    order: DirectoryOrder,
    entry_type: DirectoryEntryType,
    size_bounds: Option<(u64, u64)>,
    output: FileOutput,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FileOutput {
    Entries,
    Lines,
    Names,
    Json,
    ExtensionCounts,
}

fn split_query(query: &str) -> Option<FileQuery<'_>> {
    let (directory, filter) = query.split_once('|')?;
    let directory = super::path_tools::strip_path_quotes(directory.trim());
    let (filter, options) = filter.split_once('|').unwrap_or((filter, ""));
    let mut order = DirectoryOrder::Name;
    let mut entry_type = DirectoryEntryType::All;
    let (mut minimum, mut maximum) = (None, None);
    let mut output = FileOutput::Entries;
    for option in options.split_whitespace() {
        match option.to_ascii_lowercase().as_str() {
            "最近修改" | "modified" if order == DirectoryOrder::Name => {
                order = DirectoryOrder::ModifiedDescending;
            }
            "最近创建" | "created" if order == DirectoryOrder::Name => {
                order = DirectoryOrder::CreatedDescending;
            }
            "大小" | "size" if order == DirectoryOrder::Name => {
                order = DirectoryOrder::SizeDescending;
            }
            "仅文件" | "files" if entry_type == DirectoryEntryType::All => {
                entry_type = DirectoryEntryType::File;
            }
            "仅文件夹" | "folders" if entry_type == DirectoryEntryType::All => {
                entry_type = DirectoryEntryType::Directory;
            }
            "路径列表" | "paths" if output == FileOutput::Entries => output = FileOutput::Lines,
            "名称列表" | "names" if output == FileOutput::Entries => output = FileOutput::Names,
            "路径json" | "json-paths" if output == FileOutput::Entries => {
                output = FileOutput::Json
            }
            "后缀统计" | "extension-counts" if output == FileOutput::Entries => {
                output = FileOutput::ExtensionCounts
            }
            _ => {
                let (comparison, quantity) = option.split_once('=')?;
                let bound = match comparison.to_ascii_lowercase().as_str() {
                    "大小>" | "size>" if minimum.is_none() => &mut minimum,
                    "大小<" | "size<" if maximum.is_none() => &mut maximum,
                    _ => return None,
                };
                *bound = Some(exact_byte_quantity(quantity)?);
            }
        }
    }
    if output == FileOutput::ExtensionCounts {
        if entry_type == DirectoryEntryType::Directory {
            return None;
        }
        entry_type = DirectoryEntryType::File;
    }
    let size_bounds = if minimum.is_some() || maximum.is_some() {
        let (minimum, maximum) = (minimum.unwrap_or(0), maximum.unwrap_or(u64::MAX));
        if minimum > maximum
            || !matches!(order, DirectoryOrder::Name | DirectoryOrder::SizeDescending)
            || entry_type == DirectoryEntryType::Directory
        {
            return None;
        }
        order = DirectoryOrder::SizeDescending;
        entry_type = DirectoryEntryType::File;
        Some((minimum, maximum))
    } else {
        None
    };
    (!directory.is_empty()).then_some(FileQuery {
        directory,
        filter: filter.trim(),
        order,
        entry_type,
        size_bounds,
        output,
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
            size_bytes: None,
            metadata: None,
        }
    }

    #[test]
    fn invalidating_a_directory_snapshot_reloads_the_same_query() {
        let mut index = LocalFilesIndex::default();
        let query = r"C:\Work | *.txt";
        let previous = index.request(Some(query)).unwrap();
        assert!(index.finish(&previous, Ok(vec![entry("a.txt", "file")])));
        assert!(index.request(Some(query)).is_none());
        index.invalidate();
        assert!(previous.cancelled.load(Ordering::Relaxed));
        let next = index.request(Some(query)).unwrap();
        assert!(!Arc::ptr_eq(&previous, &next));
        assert!(!index.finish(&previous, Ok(Vec::new())));
        assert!(index.finish(&next, Ok(vec![entry("b.txt", "file")])));
        assert!(matches!(index.results(query), Results::Destinations(items)
            if matches!(items.as_slice(), [Destination::LocalFile { name, .. }] if name == "b.txt")));
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
    fn creation_order_accepts_aliases_and_composes_with_type_and_json_output() {
        let mut index = LocalFilesIndex::default();
        let query = r"C:\Work | *.txt | 最近创建 仅文件 路径JSON";
        let latest = index.request(Some(query)).unwrap();
        assert_eq!(latest.order, DirectoryOrder::CreatedDescending);
        assert_eq!(latest.entry_type, DirectoryEntryType::File);
        assert_eq!(latest.pattern.as_deref(), Some("*.txt"));
        assert!(index
            .request(Some(r"C:\Work | *.txt | created files json-paths"))
            .is_none());
        assert!(index.finish(
            &latest,
            Ok(vec![entry("z.txt", "file"), entry("a.txt", "file")])
        ));
        assert!(matches!(index.results(query), Results::Value(Ok(value))
            if value.text == r#"["C:\\Work\\z.txt","C:\\Work\\a.txt"]"#));
    }

    #[test]
    fn previews_matching_paths_in_result_order_using_the_loaded_directory() {
        let mut index = LocalFilesIndex::default();
        let folder = r"C:\Work | report | 大小";
        let load = index.request(Some(folder)).unwrap();
        assert!(index.finish(
            &load,
            Ok(vec![
                entry("z report.txt", "file"),
                entry("report.txt", "file"),
                entry("notes.txt", "file"),
                entry("report.url", "unavailable"),
            ]),
        ));
        let paths_query = r"C:\Work | report | 大小 路径列表";
        assert!(index.request(Some(paths_query)).is_none());
        assert!(matches!(
            index.results(paths_query),
            Results::Value(Ok(value)) if value.text == "C:\\Work\\z report.txt\nC:\\Work\\report.txt"
        ));
        assert!(matches!(
            index.results(r"C:\Work | omitted | paths size"),
            Results::Destinations(items) if items.is_empty()
        ));
        assert!(split_query(r"C:\Work | | 路径列表 paths").is_none());
        let json_query = r"C:\Work | report | 大小 路径JSON";
        assert!(index.request(Some(json_query)).is_none());
        assert!(matches!(
            index.results(json_query),
            Results::Value(Ok(value))
                if value.text == r#"["C:\\Work\\z report.txt","C:\\Work\\report.txt"]"#
        ));
        assert!(matches!(
            index.results(r"C:\Work | omitted | json-paths size"),
            Results::Value(Ok(value)) if value.text == "[]"
        ));
        assert!(split_query(r"C:\Work | | 路径列表 路径JSON").is_none());
        let names = r"C:\Work | report | 大小 名称列表";
        assert!(index.request(Some(names)).is_none());
        assert!(matches!(
            index.results(names),
            Results::Value(Ok(value)) if value.text == "z report.txt\nreport.txt"
        ));
        assert!(matches!(
            index.results(r"C:\Work | omitted | names size"),
            Results::Destinations(items) if items.is_empty()
        ));
        assert!(split_query(r"C:\Work | | 名称列表 paths").is_none());
    }

    #[test]
    fn filters_files_by_inclusive_byte_bounds_without_reopening_the_directory() {
        let mut index = LocalFilesIndex::default();
        let query = r"C:\Work | *.txt | 大小>=1.5KiB 大小<=2kB 名称列表";
        let load = index.request(Some(query)).unwrap();
        assert_eq!(load.order, DirectoryOrder::SizeDescending);
        assert_eq!(load.entry_type, DirectoryEntryType::File);
        assert_eq!(split_query(query).unwrap().size_bounds, Some((1536, 2000)));
        assert!(index.finish(
            &load,
            Ok([
                ("over.txt", 2001),
                ("high.txt", 2000),
                ("low.txt", 1536),
                ("under.txt", 1535)
            ]
            .into_iter()
            .map(|(name, size)| LocalDirectoryEntry {
                size_bytes: Some(size),
                ..entry(name, "file")
            })
            .collect())
        ));
        assert!(matches!(index.results(query), Results::Value(Ok(value))
            if value.text == "high.txt\nlow.txt"));
        let changed = r"C:\Work | *.txt | size>=2000 names";
        assert!(index.request(Some(changed)).is_none());
        assert!(matches!(index.results(changed), Results::Value(Ok(value))
            if value.text == "over.txt\nhigh.txt"));
        for invalid in [
            r"C:\Work | | 大小>=0.5B",
            r"C:\Work | | 大小>=1Mib",
            r"C:\Work | | 大小>=18446744073709551616",
            r"C:\Work | | 大小>=2 大小<=1",
            r"C:\Work | | 大小>=1 大小>=2",
            r"C:\Work | | 大小>=1 最近修改",
            r"C:\Work | | 大小>=1 仅文件夹",
        ] {
            assert!(split_query(invalid).is_none(), "{invalid}");
        }
    }

    #[test]
    fn counts_filtered_file_extensions_case_insensitively_and_outputs_csv() {
        let mut index = LocalFilesIndex::default();
        let query = r"C:\Work | | 后缀统计";
        let load = index.request(Some(query)).unwrap();
        assert_eq!(load.entry_type, DirectoryEntryType::File);
        assert!(index.finish(
            &load,
            Ok(vec![
                entry("a.tar.GZ", "file"),
                entry("b.gz", "file"),
                entry("c.TXT", "file"),
                entry(".gitignore", "file"),
                entry("README", "file"),
                entry("d.csv,x", "file"),
                entry("folder.gz", "directory"),
                entry("cloud.gz", "unavailable"),
            ])
        ));
        assert!(matches!(
            index.results(query), Results::Value(Ok(value))
                if value.text == "扩展名,数量\r\n无扩展名,2\r\n.gz,2\r\n\".csv,x\",1\r\n.txt,1"
        ));
        assert!(matches!(
            index.results(r"C:\Work | README | extension-counts"), Results::Value(Ok(value))
                if value.text == "扩展名,数量\r\n无扩展名,1"
        ));
        assert!(matches!(
            index.results(r"C:\Work | missing | 后缀统计"), Results::Value(Ok(value))
                if value.text == "扩展名,数量"
        ));
        assert!(split_query(r"C:\Work | | 后缀统计 仅文件夹").is_none());
        assert!(split_query(r"C:\Work | | 后缀统计 路径列表").is_none());
    }

    #[test]
    fn combines_directory_entry_type_with_sort_and_pattern() {
        let mut index = LocalFilesIndex::default();
        let first = index.request(Some(r"C:\Work | *.txt | 大小")).unwrap();
        let query = r"C:\Work | *.txt | 仅文件 大小";
        let filtered = index.request(Some(query)).unwrap();
        assert!(first.cancelled.load(Ordering::Relaxed));
        assert_eq!(filtered.pattern.as_deref(), Some("*.txt"));
        assert_eq!(filtered.order, DirectoryOrder::SizeDescending);
        assert_eq!(filtered.entry_type, DirectoryEntryType::File);
        assert!(index
            .request(Some(r"C:\Work | *.txt | SIZE FILES"))
            .is_none());
        assert_eq!(
            split_query(r"C:\Work | | 仅文件夹").map(|query| query.entry_type),
            Some(DirectoryEntryType::Directory)
        );
        for invalid in [
            r"C:\Work | | 仅文件 仅文件夹",
            r"C:\Work | | 大小 最近修改",
            r"C:\Work | | 未知选项",
        ] {
            assert!(split_query(invalid).is_none(), "{invalid}");
        }
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

    #[test]
    fn requests_file_size_order_with_the_same_filename_pattern() {
        let mut index = LocalFilesIndex::default();
        let newest = index.request(Some(r"C:\Work | *.txt | 最近修改")).unwrap();
        let largest = index.request(Some(r"C:\Work | *.txt | 大小")).unwrap();
        assert!(newest.cancelled.load(Ordering::Relaxed));
        assert_eq!(largest.order, DirectoryOrder::SizeDescending);
        assert_eq!(largest.pattern.as_deref(), Some("*.txt"));
        assert!(index.request(Some(r"C:\Work | *.txt | SIZE")).is_none());
    }
}
