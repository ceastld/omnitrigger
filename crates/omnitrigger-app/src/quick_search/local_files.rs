use std::{
    borrow::Cow,
    cell::RefCell,
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    fmt::Write as _,
    path::{Component, Path, Prefix},
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use chrono::{DateTime, Datelike, Local, NaiveDate, SecondsFormat};
use omnitrigger_windows::{
    read_local_directory_with_options, DirectoryEntryType, DirectoryOrder, LocalDirectoryEntry,
    LocalDirectoryOptions, LocalStatError, MAX_DIRECTORY_ENTRIES,
};

use super::{
    conversion::exact_byte_quantity,
    csv_tools::csv_cell,
    path_tools::file_url,
    text_tools::{compare_natural, compile_regex},
    title_rank, Destination, FilePreviewFormat, InputSuggestion, Results, INPUT_CHARACTER_LIMIT,
};

pub(super) const FOLDER_DEBOUNCE: Duration = Duration::from_millis(150);
const FOLDER_READ_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_NAVIGATION_HISTORY: usize = 16;
const MAX_RECENT_PREVIEWS: usize = 12;
const TICKS_PER_DAY: u64 = 864_000_000_000;
const FILETIME_TICKS_PER_SECOND: u64 = 10_000_000;
const FILETIME_UNIX_EPOCH_SECONDS: i64 = 11_644_473_600;
const OPTION_HELP: &[(&str, &str)] = &[
    ("仅文件", "只匹配文件"),
    ("仅文件夹", "只匹配文件夹"),
    ("仅可预览", "按支持文本预览的扩展名筛选文件"),
    ("最近修改", "按修改时间从新到旧"),
    ("最早修改", "按修改时间从旧到新"),
    ("最近创建", "按创建时间从新到旧"),
    ("最早创建", "按创建时间从旧到新"),
    ("修改近=7天", "只取含今天在内最近 7 天修改的项"),
    ("创建近=7天", "只取含今天在内最近 7 天创建的项"),
    ("大小", "按字节数从大到小"),
    ("最小优先", "按字节数从小到大"),
    ("自然排序", "名称中的数字按数值排"),
    ("按后缀", "相同扩展名的文件排在一起"),
    ("任一名称=预算,发票", "名称只需包含其中一个片段"),
    ("排除名称=草稿,副本", "跳过名称中含有指定文字的项"),
    (r"名称正则=^报告[0-9]+\.pdf$", "按正则表达式筛选名称"),
    ("前=10", "取排序后的前 10 项"),
    ("路径列表", "每行一个绝对路径"),
    ("名称列表", "每行一个名称"),
    ("路径JSON", "导出路径数组"),
    ("文件网址列表", "每行一个文件网址"),
    ("时间CSV", "导出 UTC 时间与路径"),
    ("清单CSV", "导出名称、类型、字节数、时间与路径"),
    ("大小合计", "汇总文件数和字节数"),
    ("后缀统计", "按扩展名汇总数量"),
    ("后缀大小", "按扩展名汇总字节数"),
    ("同主名CSV", "列出主名相同的文件及其路径"),
    ("修改月份统计", "按本地修改月份计数"),
    ("创建月份统计", "按本地创建月份计数"),
    ("修改周统计", "按本地修改日期的 ISO 周计数"),
    ("创建周统计", "按本地创建日期的 ISO 周计数"),
    ("修改日期统计", "按本地修改日期计数"),
    ("创建日期统计", "按本地创建日期计数"),
];

#[derive(Clone, Default)]
pub(super) struct RecentFilePreviews(Rc<RefCell<Vec<RecentFilePreview>>>);

struct RecentFilePreview {
    path: String,
    format: FilePreviewFormat,
}

impl RecentFilePreview {
    fn name(&self) -> &str {
        Path::new(&self.path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(&self.path)
    }
}

impl RecentFilePreviews {
    pub(super) fn record(&self, path: &str, format: FilePreviewFormat) {
        let mut previews = self.0.borrow_mut();
        previews.retain(|entry| entry.path != path);
        previews.insert(
            0,
            RecentFilePreview {
                path: path.to_owned(),
                format,
            },
        );
        previews.truncate(MAX_RECENT_PREVIEWS);
    }

    pub(super) fn remove(&self, path: &str) {
        self.0.borrow_mut().retain(|entry| entry.path != path);
    }

    pub(super) fn format_for(&self, path: &str) -> Option<FilePreviewFormat> {
        self.0
            .borrow()
            .iter()
            .find(|entry| entry.path == path)
            .map(|entry| entry.format)
    }

    pub(super) fn results(&self, query: &str) -> Results {
        let query = query.trim().to_lowercase();
        let (first, rest) = query
            .split_once(char::is_whitespace)
            .unwrap_or((&query, ""));
        let export = matches!(first, "输出=csv" | "output=csv");
        let filter = if export { rest.trim_start() } else { &query };
        let recent = self.0.borrow();
        let matches = recent
            .iter()
            .filter(|entry| filter.is_empty() || entry.path.to_lowercase().contains(filter));
        if export {
            let mut csv = String::from("文件名,预览方式,路径");
            for entry in matches {
                let mode = match entry.format {
                    FilePreviewFormat::Automatic => "自动",
                    FilePreviewFormat::Raw => "原文",
                    FilePreviewFormat::Head => "开头",
                    FilePreviewFormat::Tail => "末尾",
                    FilePreviewFormat::Table => "表格",
                    FilePreviewFormat::TailTable => "末尾表格",
                };
                write!(
                    csv,
                    "\r\n{},{mode},{}",
                    csv_cell(entry.name()),
                    csv_cell(&entry.path),
                )
                .expect("写入字符串不会失败");
            }
            return Results::table_value(Ok(csv));
        }
        Results::Destinations(
            matches
                .map(|entry| Destination::LocalFile {
                    name: entry.name().to_owned(),
                    path: entry.path.clone(),
                    is_directory: false,
                    size_bytes: None,
                })
                .collect(),
        )
    }
}

#[derive(Default)]
pub(super) struct LocalFilesIndex {
    current: Option<DirectoryRequest>,
    back: VecDeque<String>,
    forward: VecDeque<String>,
}

struct DirectoryRequest {
    load: Arc<DirectoryLoad>,
    entries: Option<Result<Vec<LocalDirectoryEntry>, String>>,
    loading: bool,
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
    timestamps: bool,
    file_sizes: bool,
    cancelled: AtomicBool,
}

impl DirectoryLoad {
    fn matches(&self, query: FileQuery<'_>) -> bool {
        self.directory == query.directory
            && self.pattern.as_deref() == wildcard_pattern(query.filter)
            && self.order == query.order
            && self.entry_type == query.entry_type
            && self.timestamps == query.needs_timestamps()
            && self.file_sizes == query.needs_file_sizes()
    }

    pub(super) fn read(&self) -> Result<Vec<LocalDirectoryEntry>, String> {
        let deadline = Instant::now() + FOLDER_READ_TIMEOUT;
        let options = LocalDirectoryOptions {
            max_entries: MAX_DIRECTORY_ENTRIES,
            order: self.order,
            metadata: self.timestamps,
            entry_type: self.entry_type,
            search_pattern: self.pattern.clone(),
        };
        let check = || {
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
        };
        let result = if self.file_sizes {
            options.read_with_file_sizes(&self.directory, check)
        } else {
            read_local_directory_with_options(&self.directory, &options, check)
        };
        result.map_err(|error| error.message)
    }
}

impl LocalFilesIndex {
    pub(super) fn remember_navigation(&mut self, current: String) {
        Self::push_navigation(&mut self.back, current);
        self.forward.clear();
    }

    pub(super) fn move_navigation(&mut self, current: String, forward: bool) -> Option<String> {
        let (source, destination) = if forward {
            (&mut self.forward, &mut self.back)
        } else {
            (&mut self.back, &mut self.forward)
        };
        let query = source.pop_back()?;
        Self::push_navigation(destination, current);
        Some(query)
    }

    fn push_navigation(history: &mut VecDeque<String>, query: String) {
        if history.len() == MAX_NAVIGATION_HISTORY {
            history.pop_front();
        }
        history.push_back(query);
    }

    fn invalidate(&mut self) {
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
        if query.is_some_and(|query| option_help_prefix(query).is_some()) {
            return None;
        }
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
        Some(self.start_request(query, None))
    }

    pub(super) fn reload(&mut self, query: &str) -> Option<Arc<DirectoryLoad>> {
        let query = split_query(query)?;
        let current = self.current.as_mut()?;
        if current.loading || !current.load.matches(query) {
            return None;
        }
        let entries = current.entries.take();
        Some(self.start_request(query, entries))
    }

    fn start_request(
        &mut self,
        query: FileQuery<'_>,
        entries: Option<Result<Vec<LocalDirectoryEntry>, String>>,
    ) -> Arc<DirectoryLoad> {
        let load = Arc::new(DirectoryLoad {
            directory: query.directory.to_owned(),
            pattern: wildcard_pattern(query.filter).map(str::to_owned),
            order: query.order,
            entry_type: query.entry_type,
            timestamps: query.needs_timestamps(),
            file_sizes: query.needs_file_sizes(),
            cancelled: AtomicBool::new(false),
        });
        self.current = Some(DirectoryRequest {
            load: load.clone(),
            entries,
            loading: true,
        });
        load
    }

    pub(super) fn finish(
        &mut self,
        load: &Arc<DirectoryLoad>,
        entries: Result<Vec<LocalDirectoryEntry>, String>,
    ) -> bool {
        let Some(current) = self.current.as_mut() else {
            return false;
        };
        if !current.loading || !Arc::ptr_eq(load, &current.load) {
            return false;
        }
        current.entries = Some(entries);
        current.loading = false;
        true
    }

    pub(super) fn results(&self, query: &str) -> Results {
        if let Some((prefix, current)) = option_help_prefix(query) {
            return Results::FileOptions(file_option_suggestions(prefix, current));
        }
        let Some(query) = split_query(query) else {
            return directory_input(query)
                .map_or(Results::LocalFilesPrompt, Results::DirectoryInput);
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
        let name_regex = match query.name_regex.map(compile_regex).transpose() {
            Ok(pattern) => pattern,
            Err(error) => return Results::LocalFilesUnavailable(error.message().into()),
        };
        let filter = if wildcard_pattern(query.filter).is_some() {
            String::new()
        } else {
            query.filter.to_lowercase()
        };
        let keywords = file_name_keywords(&filter);
        let rank_filter = if filter.contains('"') {
            Cow::Owned(filter.replace('"', ""))
        } else {
            Cow::Borrowed(filter.as_str())
        };
        let extensions = extension_set(query.extensions);
        let excluded_extensions = extension_set(query.excluded_extensions);
        let any_names = name_fragments(query.any_names);
        let excluded_names = name_fragments(query.excluded_names);
        let mut matches: Vec<_> = entries
            .iter()
            .filter(|entry| {
                matches!(entry.kind, "file" | "directory")
                    && matches_extensions(entry, extensions.as_ref(), excluded_extensions.as_ref())
                    && (!query.previewable_only
                        || (entry.kind == "file"
                            && super::supports_text_file_preview(Path::new(&entry.name))))
                    && query.matches_dates(entry)
                    && name_regex
                        .as_ref()
                        .is_none_or(|pattern| pattern.is_match(&entry.name))
                    && query.size_bounds.is_none_or(|(minimum, maximum)| {
                        entry
                            .size_bytes
                            .is_some_and(|size| (minimum..=maximum).contains(&size))
                    })
            })
            .filter_map(|entry| {
                let name = entry.name.to_lowercase();
                let matches_name = keywords.iter().all(|&keyword| name.contains(keyword))
                    && (any_names.is_empty()
                        || any_names.iter().any(|term| name.contains(term.as_str())));
                let excluded = excluded_names
                    .iter()
                    .any(|keyword| name.contains(keyword.as_str()));
                (matches_name && !excluded).then(|| (title_rank(&name, &rank_filter), entry))
            })
            .collect();
        if query.order == DirectoryOrder::Name
            && (query.output != FileOutput::ExtensionCounts || query.limit.is_some())
        {
            match query.name_order {
                NameOrder::Matching => matches.sort_by_key(|(rank, _)| *rank),
                NameOrder::Natural => {
                    matches
                        .sort_by(|(_, left), (_, right)| compare_natural(&left.name, &right.name));
                }
                NameOrder::Extension => matches.sort_by_cached_key(|(_, entry)| {
                    let is_directory = entry.kind == "directory";
                    let extension = if is_directory {
                        None
                    } else {
                        file_extension(&entry.name).filter(|part| !part.is_empty())
                    };
                    (
                        is_directory,
                        extension.is_none(),
                        extension.map(str::to_lowercase),
                    )
                }),
            }
        }
        if let Some(limit) = query.limit {
            matches.truncate(limit);
        }
        if query.output == FileOutput::SizeTotal {
            let summary = matches
                .iter()
                .filter(|(_, entry)| entry.kind == "file")
                .try_fold((0usize, 0u128), |(count, total), (_, entry)| {
                    Some((count + 1, total + u128::from(entry.size_bytes?)))
                });
            return match summary {
                Some((count, total)) => {
                    Results::text_value(Ok(format!("{count} 个文件 · 合计 {total} B")))
                }
                None => Results::LocalFilesUnavailable("有文件的大小无法读取".to_owned()),
            };
        }
        if matches!(
            query.output,
            FileOutput::ExtensionCounts | FileOutput::ExtensionSizes
        ) {
            let files = matches
                .iter()
                .filter(|(_, entry)| entry.kind == "file")
                .map(|(_, entry)| *entry);
            return match extension_summary(files, query.output == FileOutput::ExtensionSizes) {
                Some(output) => Results::table_value(Ok(output)),
                None => Results::LocalFilesUnavailable("有文件的大小无法读取".to_owned()),
            };
        }
        if matches!(query.output, FileOutput::TimeCsv | FileOutput::InventoryCsv) {
            let csv = file_metadata_csv(
                matches.into_iter().map(|(_, entry)| entry),
                query.output == FileOutput::InventoryCsv,
            );
            return Results::table_value_with_columns(Ok(csv), 2);
        }
        if query.output == FileOutput::SharedStemCsv {
            let csv = shared_stem_csv(matches.into_iter().map(|(_, entry)| entry));
            return Results::table_value_with_columns(Ok(csv), 3);
        }
        if let FileOutput::DateCounts(field, grouping) = query.output {
            return Results::table_value(Ok(file_date_counts(
                matches.into_iter().map(|(_, entry)| entry),
                field,
                grouping,
            )));
        }
        if query.output == FileOutput::Urls && !matches.is_empty() {
            let urls = matches
                .iter()
                .map(|(_, entry)| file_url(Path::new(&entry.path), entry.kind == "directory"))
                .collect::<Result<Vec<_>, _>>();
            return match urls {
                Ok(values) => Results::text_value(Ok(values.join("\n"))),
                Err(error) => Results::LocalFilesUnavailable(error.message().to_owned()),
            };
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

fn shared_stem_csv<'a>(entries: impl Iterator<Item = &'a LocalDirectoryEntry>) -> String {
    let files = entries
        .filter(|entry| entry.kind == "file")
        .filter_map(|file| {
            let stem = Path::new(&file.name).file_stem()?.to_str()?;
            Some((stem, stem.to_lowercase(), file))
        })
        .collect::<Vec<_>>();
    let mut counts = HashMap::new();
    for (_, key, _) in &files {
        *counts.entry(key.as_str()).or_insert(0usize) += 1;
    }
    let mut csv = String::from("主名,组内文件数,文件名,路径");
    for (stem, key, file) in &files {
        let count = counts[key.as_str()];
        if count > 1 {
            write!(
                csv,
                "\r\n{},{count},{},{}",
                csv_cell(stem),
                csv_cell(&file.name),
                csv_cell(&file.path),
            )
            .expect("写入字符串不会失败");
        }
    }
    csv
}

fn extension_summary<'a>(
    files: impl Iterator<Item = &'a LocalDirectoryEntry>,
    include_sizes: bool,
) -> Option<String> {
    let mut counts = HashMap::new();
    for file in files {
        let extension = file_extension(&file.name)
            .unwrap_or_default()
            .to_lowercase();
        let (count, total) = counts.entry(extension).or_insert((0usize, 0u128));
        *count += 1;
        if include_sizes {
            *total += u128::from(file.size_bytes?);
        }
    }
    let mut rows = counts.into_iter().collect::<Vec<_>>();
    rows.sort_unstable_by(
        |(left, (left_count, left_total)), (right, (right_count, right_total))| {
            right_total
                .cmp(left_total)
                .then_with(|| right_count.cmp(left_count))
                .then_with(|| left.cmp(right))
        },
    );
    let mut output = String::from("扩展名,数量");
    if include_sizes {
        output.push_str(",字节数");
    }
    for (extension, (count, total)) in rows {
        let label = if extension.is_empty() {
            "无扩展名".to_owned()
        } else {
            format!(".{extension}")
        };
        write!(output, "\r\n{},{count}", csv_cell(&label)).expect("写入字符串不会失败");
        if include_sizes {
            write!(output, ",{total}").expect("写入字符串不会失败");
        }
    }
    Some(output)
}

fn file_metadata_csv<'a>(
    entries: impl Iterator<Item = &'a LocalDirectoryEntry>,
    inventory: bool,
) -> String {
    let mut output = String::from(if inventory {
        "名称,类型,字节数,创建时间UTC,修改时间UTC,路径"
    } else {
        "名称,创建时间UTC,修改时间UTC,路径"
    });
    for entry in entries {
        let (created, modified) = entry.metadata.as_ref().map_or((0, 0), |time| {
            (time.creation_utc_100ns, time.write_utc_100ns)
        });
        write!(output, "\r\n{},", csv_cell(&entry.name)).expect("写入字符串不会失败");
        if inventory {
            let (kind, size) = if entry.kind == "directory" {
                ("文件夹", String::new())
            } else {
                (
                    "文件",
                    entry
                        .size_bytes
                        .map(|bytes| bytes.to_string())
                        .unwrap_or_default(),
                )
            };
            write!(output, "{kind},{},", csv_cell(&size)).expect("写入字符串不会失败");
        }
        write!(
            output,
            "{},{},{}",
            csv_cell(&format_filetime(created)),
            csv_cell(&format_filetime(modified)),
            csv_cell(&entry.path)
        )
        .expect("写入字符串不会失败");
    }
    output
}

fn format_filetime(ticks: u64) -> String {
    if ticks == 0 {
        return String::new();
    }
    let seconds = (ticks / FILETIME_TICKS_PER_SECOND) as i64 - FILETIME_UNIX_EPOCH_SECONDS;
    let nanos = ((ticks % FILETIME_TICKS_PER_SECOND) * 100) as u32;
    DateTime::from_timestamp(seconds, nanos)
        .map(|time| time.to_rfc3339_opts(SecondsFormat::AutoSi, true))
        .unwrap_or_default()
}

fn file_date_counts<'a>(
    entries: impl Iterator<Item = &'a LocalDirectoryEntry>,
    field: FileDateField,
    grouping: FileDateGrouping,
) -> String {
    let mut counts = BTreeMap::new();
    for entry in entries.filter(|entry| entry.kind == "file") {
        let bucket = entry.metadata.as_ref().and_then(|time| {
            let (raw, local) = match field {
                FileDateField::Modified => (time.write_utc_100ns, time.write_local_ticks),
                FileDateField::Created => (time.creation_utc_100ns, time.creation_local_ticks),
            };
            if raw == 0 {
                return None;
            }
            let day = i32::try_from(local / TICKS_PER_DAY + 1).ok()?;
            let date = NaiveDate::from_num_days_from_ce_opt(day)?;
            Some(match grouping {
                FileDateGrouping::Day => (date.year(), date.month(), Some(date.day())),
                FileDateGrouping::Month => (date.year(), date.month(), None),
                FileDateGrouping::Week => {
                    let week = date.iso_week();
                    (week.year(), week.week(), None)
                }
            })
        });
        *counts.entry(bucket).or_insert(0usize) += 1;
    }
    let mut output = String::from(match grouping {
        FileDateGrouping::Day => "日期,数量",
        FileDateGrouping::Month => "月份,数量",
        FileDateGrouping::Week => "周,数量",
    });
    for (bucket, count) in counts.into_iter().rev() {
        match bucket {
            Some((year, month, Some(day))) => {
                write!(output, "\r\n{year:04}-{month:02}-{day:02},{count}")
            }
            Some((year, week, None)) if grouping == FileDateGrouping::Week => {
                write!(output, "\r\n{year:04}-W{week:02},{count}")
            }
            Some((year, month, None)) => write!(output, "\r\n{year:04}-{month:02},{count}"),
            None => write!(output, "\r\n时间未知,{count}"),
        }
        .expect("写入字符串不会失败");
    }
    output
}

fn file_extension(name: &str) -> Option<&str> {
    Path::new(name).extension().and_then(|part| part.to_str())
}

fn extension_options(list: &str) -> impl Iterator<Item = &str> {
    list.split([',', '，'])
        .map(|extension| extension.strip_prefix('.').unwrap_or(extension))
}

fn extension_set(list: Option<&str>) -> Option<HashSet<String>> {
    list.map(|list| extension_options(list).map(str::to_lowercase).collect())
}

fn matches_extensions(
    entry: &LocalDirectoryEntry,
    included: Option<&HashSet<String>>,
    excluded: Option<&HashSet<String>>,
) -> bool {
    if included.is_none() && excluded.is_none() {
        return true;
    }
    if entry.kind != "file" {
        return false;
    }
    let extension = file_extension(&entry.name).map(str::to_lowercase);
    included.is_none_or(|list| extension.as_ref().is_some_and(|name| list.contains(name)))
        && excluded.is_none_or(|list| extension.as_ref().is_none_or(|name| !list.contains(name)))
}

fn set_extension_filter<'a>(target: &mut Option<&'a str>, list: &'a str) -> Option<()> {
    if target.is_some()
        || extension_options(list).any(|extension| {
            extension.is_empty() || extension.contains(['.', '*', '?', '/', '\\', ':', '|'])
        })
    {
        return None;
    }
    *target = Some(list);
    Some(())
}

fn name_fragments(list: Option<&str>) -> Vec<String> {
    list.into_iter()
        .flat_map(|terms| terms.split([',', '，']))
        .map(str::to_lowercase)
        .collect()
}

fn set_name_filter<'a>(target: &mut Option<&'a str>, list: &'a str) -> Option<()> {
    if target.is_some() || list.split([',', '，']).any(str::is_empty) {
        return None;
    }
    *target = Some(list);
    Some(())
}

pub(super) fn option_help_query(query: &str) -> Option<String> {
    split_query(query)?;
    let separator = if query.split('|').nth(2).is_some() {
        " "
    } else {
        " | "
    };
    let help = format!("文件: {query}{separator}?");
    (help.chars().count() <= INPUT_CHARACTER_LIMIT).then_some(help)
}

fn option_help_prefix(query: &str) -> Option<(&str, FileQuery<'_>)> {
    let prefix = query.trim_end().strip_suffix(['?', '？'])?;
    if prefix.split('|').nth(2).is_none()
        || !prefix.ends_with(|ch: char| ch == '|' || ch.is_whitespace())
    {
        return None;
    }
    Some((prefix.trim_end(), split_query(prefix)?))
}

fn file_option_suggestions(prefix: &str, current: FileQuery<'_>) -> Vec<InputSuggestion> {
    OPTION_HELP
        .iter()
        .filter_map(|&(label, detail)| {
            let term = format!("{prefix} {label}");
            let candidate = split_query(&term)?;
            let expression = format!("文件: {term}");
            (candidate != current && expression.chars().count() <= INPUT_CHARACTER_LIMIT).then(
                || InputSuggestion {
                    expression,
                    label: Some(label),
                    result: detail.to_owned(),
                },
            )
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NameOrder {
    Matching,
    Natural,
    Extension,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct FileQuery<'a> {
    directory: &'a str,
    filter: &'a str,
    order: DirectoryOrder,
    name_order: NameOrder,
    entry_type: DirectoryEntryType,
    previewable_only: bool,
    size_bounds: Option<(u64, u64)>,
    modified: DateBounds,
    created: DateBounds,
    extensions: Option<&'a str>,
    excluded_extensions: Option<&'a str>,
    any_names: Option<&'a str>,
    excluded_names: Option<&'a str>,
    name_regex: Option<&'a str>,
    limit: Option<usize>,
    output: FileOutput,
}

impl FileQuery<'_> {
    fn needs_file_sizes(&self) -> bool {
        self.output == FileOutput::InventoryCsv
    }

    fn needs_timestamps(&self) -> bool {
        self.modified.is_active()
            || self.created.is_active()
            || matches!(
                self.output,
                FileOutput::TimeCsv | FileOutput::InventoryCsv | FileOutput::DateCounts(..)
            )
    }

    fn matches_dates(&self, entry: &LocalDirectoryEntry) -> bool {
        (!self.modified.is_active() && !self.created.is_active())
            || entry.metadata.as_ref().is_some_and(|time| {
                self.modified
                    .contains(time.write_utc_100ns, time.write_local_ticks)
                    && self
                        .created
                        .contains(time.creation_utc_100ns, time.creation_local_ticks)
            })
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct DateBounds {
    minimum: Option<u64>,
    maximum: Option<u64>,
}

impl DateBounds {
    fn set_recent(&mut self, count: &str, today: NaiveDate) -> Option<()> {
        if self.is_active() {
            return None;
        }
        let end = u64::try_from(today.num_days_from_ce() - 1).ok()?;
        let count = count
            .strip_suffix('天')
            .unwrap_or(count)
            .parse::<u64>()
            .ok()?;
        let start = end.checked_sub(count.checked_sub(1)?)?;
        *self = Self {
            minimum: Some(start),
            maximum: Some(end),
        };
        Some(())
    }

    fn is_active(self) -> bool {
        self.minimum.is_some() || self.maximum.is_some()
    }

    fn is_valid(self) -> bool {
        self.minimum
            .zip(self.maximum)
            .is_none_or(|(first, last)| first <= last)
    }

    fn contains(self, raw_time: u64, local_ticks: u64) -> bool {
        let day = local_ticks / TICKS_PER_DAY;
        !self.is_active()
            || raw_time != 0
                && self.minimum.is_none_or(|first| day >= first)
                && self.maximum.is_none_or(|last| day <= last)
    }
}

fn calendar_day(input: &str) -> Option<u64> {
    if input.len() != 10 {
        return None;
    }
    let date = NaiveDate::parse_from_str(input, "%Y-%m-%d").ok()?;
    u64::try_from(date.num_days_from_ce() - 1).ok()
}

fn set_bound(bound: &mut Option<u64>, value: u64) -> Option<()> {
    if bound.is_some() {
        return None;
    }
    *bound = Some(value);
    Some(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FileDateField {
    Modified,
    Created,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FileDateGrouping {
    Month,
    Week,
    Day,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FileOutput {
    Entries,
    Lines,
    Names,
    Json,
    Urls,
    TimeCsv,
    InventoryCsv,
    SharedStemCsv,
    DateCounts(FileDateField, FileDateGrouping),
    ExtensionCounts,
    ExtensionSizes,
    SizeTotal,
}

fn split_query(query: &str) -> Option<FileQuery<'_>> {
    split_query_on(query, Local::now().date_naive())
}

fn split_query_on(query: &str, today: NaiveDate) -> Option<FileQuery<'_>> {
    let (directory, filter) = query.split_once('|')?;
    let directory = super::path_tools::strip_path_quotes(directory.trim());
    let (filter, options) = filter.split_once('|').unwrap_or((filter, ""));
    let mut order = DirectoryOrder::Name;
    let mut name_order = NameOrder::Matching;
    let mut entry_type = DirectoryEntryType::All;
    let mut previewable_only = false;
    let (mut minimum, mut maximum) = (None, None);
    let (mut modified, mut created) = (DateBounds::default(), DateBounds::default());
    let mut extensions = None;
    let mut excluded_extensions = None;
    let mut any_names = None;
    let mut excluded_names = None;
    let mut name_regex = None;
    let mut limit = None;
    let mut output = FileOutput::Entries;
    for option in options.split_whitespace() {
        match option.to_ascii_lowercase().as_str() {
            "最近修改" | "modified" if order == DirectoryOrder::Name => {
                order = DirectoryOrder::ModifiedDescending;
            }
            "最早修改" | "oldest-modified" if order == DirectoryOrder::Name => {
                order = DirectoryOrder::ModifiedAscending;
            }
            "最近创建" | "created" if order == DirectoryOrder::Name => {
                order = DirectoryOrder::CreatedDescending;
            }
            "最早创建" | "oldest-created" if order == DirectoryOrder::Name => {
                order = DirectoryOrder::CreatedAscending;
            }
            "大小" | "size" if order == DirectoryOrder::Name => {
                order = DirectoryOrder::SizeDescending;
            }
            "最小优先" | "smallest" if order == DirectoryOrder::Name => {
                order = DirectoryOrder::SizeAscending;
            }
            "自然排序" | "natural-order" if name_order == NameOrder::Matching => {
                name_order = NameOrder::Natural;
            }
            "按后缀" | "extension-order" if name_order == NameOrder::Matching => {
                name_order = NameOrder::Extension;
            }
            "仅文件" | "files" if entry_type == DirectoryEntryType::All => {
                entry_type = DirectoryEntryType::File;
            }
            "仅文件夹" | "folders" if entry_type == DirectoryEntryType::All => {
                entry_type = DirectoryEntryType::Directory;
            }
            "仅可预览" | "previewable" if !previewable_only => previewable_only = true,
            "路径列表" | "paths" if output == FileOutput::Entries => output = FileOutput::Lines,
            "名称列表" | "names" if output == FileOutput::Entries => output = FileOutput::Names,
            "路径json" | "json-paths" if output == FileOutput::Entries => {
                output = FileOutput::Json
            }
            "文件网址列表" | "file-urls" if output == FileOutput::Entries => {
                output = FileOutput::Urls
            }
            "时间csv" | "time-csv" if output == FileOutput::Entries => {
                output = FileOutput::TimeCsv
            }
            "清单csv" | "inventory-csv" if output == FileOutput::Entries => {
                output = FileOutput::InventoryCsv
            }
            "修改月份统计" | "modified-month-counts" if output == FileOutput::Entries => {
                output = FileOutput::DateCounts(FileDateField::Modified, FileDateGrouping::Month)
            }
            "创建月份统计" | "created-month-counts" if output == FileOutput::Entries => {
                output = FileOutput::DateCounts(FileDateField::Created, FileDateGrouping::Month)
            }
            "修改周统计" | "modified-week-counts" if output == FileOutput::Entries => {
                output = FileOutput::DateCounts(FileDateField::Modified, FileDateGrouping::Week)
            }
            "创建周统计" | "created-week-counts" if output == FileOutput::Entries => {
                output = FileOutput::DateCounts(FileDateField::Created, FileDateGrouping::Week)
            }
            "修改日期统计" | "modified-day-counts" if output == FileOutput::Entries => {
                output = FileOutput::DateCounts(FileDateField::Modified, FileDateGrouping::Day)
            }
            "创建日期统计" | "created-day-counts" if output == FileOutput::Entries => {
                output = FileOutput::DateCounts(FileDateField::Created, FileDateGrouping::Day)
            }
            "后缀统计" | "extension-counts" if output == FileOutput::Entries => {
                output = FileOutput::ExtensionCounts
            }
            "后缀大小" | "extension-sizes" if output == FileOutput::Entries => {
                output = FileOutput::ExtensionSizes
            }
            "同主名csv" | "shared-stem-csv" if output == FileOutput::Entries => {
                output = FileOutput::SharedStemCsv
            }
            "大小合计" | "size-total" if output == FileOutput::Entries => {
                output = FileOutput::SizeTotal
            }
            _ => {
                let (comparison, quantity) = option.split_once('=')?;
                match comparison.to_ascii_lowercase().as_str() {
                    "后缀" | "ext" => set_extension_filter(&mut extensions, quantity)?,
                    "排除后缀" | "exclude-ext" => {
                        set_extension_filter(&mut excluded_extensions, quantity)?
                    }
                    "任一名称" | "any-name" => set_name_filter(&mut any_names, quantity)?,
                    "排除名称" | "exclude-name" => {
                        set_name_filter(&mut excluded_names, quantity)?
                    }
                    "名称正则" | "name-regex" if name_regex.is_none() => {
                        name_regex = Some(quantity)
                    }
                    "前" | "limit" if limit.is_none() => {
                        limit = Some(
                            quantity
                                .parse::<usize>()
                                .ok()
                                .filter(|count| (1..=MAX_DIRECTORY_ENTRIES).contains(count))?,
                        );
                    }
                    "大小>" | "size>" => set_bound(&mut minimum, exact_byte_quantity(quantity)?)?,
                    "大小<" | "size<" => set_bound(&mut maximum, exact_byte_quantity(quantity)?)?,
                    "修改>" | "modified>" => {
                        set_bound(&mut modified.minimum, calendar_day(quantity)?)?
                    }
                    "修改<" | "modified<" => {
                        set_bound(&mut modified.maximum, calendar_day(quantity)?)?
                    }
                    "修改近" | "modified-days" => modified.set_recent(quantity, today)?,
                    "创建>" | "created>" => {
                        set_bound(&mut created.minimum, calendar_day(quantity)?)?
                    }
                    "创建<" | "created<" => {
                        set_bound(&mut created.maximum, calendar_day(quantity)?)?
                    }
                    "创建近" | "created-days" => created.set_recent(quantity, today)?,
                    _ => return None,
                }
            }
        }
    }
    if matches!(
        output,
        FileOutput::ExtensionCounts
            | FileOutput::ExtensionSizes
            | FileOutput::SharedStemCsv
            | FileOutput::DateCounts(..)
            | FileOutput::SizeTotal
    ) || extensions.is_some()
        || excluded_extensions.is_some()
        || previewable_only
    {
        if entry_type == DirectoryEntryType::Directory {
            return None;
        }
        entry_type = DirectoryEntryType::File;
    }
    let has_size_bounds = minimum.is_some() || maximum.is_some();
    if has_size_bounds || matches!(output, FileOutput::SizeTotal | FileOutput::ExtensionSizes) {
        if !matches!(
            order,
            DirectoryOrder::Name | DirectoryOrder::SizeDescending | DirectoryOrder::SizeAscending
        ) {
            return None;
        }
        if order == DirectoryOrder::Name {
            order = DirectoryOrder::SizeDescending;
        }
    }
    let size_bounds = if has_size_bounds {
        let (minimum, maximum) = (minimum.unwrap_or(0), maximum.unwrap_or(u64::MAX));
        if minimum > maximum || entry_type == DirectoryEntryType::Directory {
            return None;
        }
        entry_type = DirectoryEntryType::File;
        Some((minimum, maximum))
    } else {
        None
    };
    (!directory.is_empty()
        && modified.is_valid()
        && created.is_valid()
        && (name_order == NameOrder::Matching || order == DirectoryOrder::Name)
        && (name_order != NameOrder::Extension || entry_type != DirectoryEntryType::Directory))
        .then_some(FileQuery {
            directory,
            filter: filter.trim(),
            order,
            name_order,
            entry_type,
            previewable_only,
            size_bounds,
            modified,
            created,
            extensions,
            excluded_extensions,
            any_names,
            excluded_names,
            name_regex,
            limit,
            output,
        })
}

fn wildcard_pattern(filter: &str) -> Option<&str> {
    let filter = super::path_tools::strip_path_quotes(filter);
    filter.contains(['*', '?']).then_some(filter)
}

fn file_name_keywords(filter: &str) -> Vec<&str> {
    let mut keywords = Vec::new();
    let mut quoted = false;
    for part in filter.split('"') {
        if quoted {
            if !part.is_empty() {
                keywords.push(part);
            }
        } else {
            keywords.extend(part.split_whitespace());
        }
        quoted = !quoted;
    }
    keywords
}

pub(super) fn directory_query(directory: &str) -> String {
    format!("文件: {directory} |")
}

pub(super) fn local_disk_path(query: &str) -> Option<&str> {
    let source = super::path_tools::strip_path_quotes(query.trim());
    if source.contains(['|', '<', '>', '?', '*']) {
        return None;
    }
    let path = super::path_tools::absolute_path(source)?;
    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return None;
    };
    (matches!(prefix.kind(), Prefix::Disk(_)) && !source.get(2..)?.contains(':')).then_some(source)
}

pub(super) fn local_file_url(query: &str) -> Option<&str> {
    const PREFIX: &str = "file:///";
    let source = super::path_tools::strip_path_quotes(query.trim());
    source
        .get(..PREFIX.len())?
        .eq_ignore_ascii_case(PREFIX)
        .then_some(source)
}

pub(super) fn decode_local_file_url(url: &str) -> Option<String> {
    super::path_tools::path_from_file_url(url)
        .ok()
        .filter(|path| local_disk_path(path) == Some(path.as_str()))
}

fn directory_input(query: &str) -> Option<InputSuggestion> {
    let expression = match local_disk_path(query) {
        Some(directory) => directory_query(directory),
        None => {
            let url = local_file_url(query).filter(|url| url.ends_with('/'))?;
            directory_query(&decode_local_file_url(url)?)
        }
    };
    (expression.chars().count() <= INPUT_CHARACTER_LIMIT).then_some(InputSuggestion {
        expression,
        label: Some("浏览目录"),
        result: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quick_search::ValueKind;

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
    fn recent_previews_keep_the_most_recent_format_for_each_path() {
        let previews = RecentFilePreviews::default();
        let log = r"C:\Work\service.log";
        let json = r"C:\Work\records.jsonl";
        previews.record(log, FilePreviewFormat::Head);
        previews.record(json, FilePreviewFormat::Raw);
        previews.record(log, FilePreviewFormat::Tail);
        assert!(matches!(
            previews.format_for(log),
            Some(FilePreviewFormat::Tail)
        ));
        assert!(matches!(
            previews.format_for(json),
            Some(FilePreviewFormat::Raw)
        ));
        assert!(matches!(previews.results(""), Results::Destinations(items)
            if matches!(items.as_slice(), [Destination::LocalFile { path, .. }, Destination::LocalFile { .. }] if path == log)));
        previews.remove(log);
        assert!(previews.format_for(log).is_none());
    }

    #[test]
    fn exports_recent_preview_paths_and_formats_in_matching_order() {
        let recent = RecentFilePreviews::default();
        recent.record(r"C:/Work/日志,1.LOG", FilePreviewFormat::Tail);
        recent.record(r"C:/Work/文档.txt", FilePreviewFormat::Raw);
        recent.record(r"C:/Work/日志2.NDJSON", FilePreviewFormat::TailTable);
        assert_eq!(
            recent.results("输出=CSV 日志").refillable_text(),
            Some(concat!(
                "文件名,预览方式,路径",
                "\r\n日志2.NDJSON,末尾表格,C:/Work/日志2.NDJSON",
                "\r\n\"日志,1.LOG\",末尾,\"C:/Work/日志,1.LOG\"",
            ))
        );
        assert_eq!(
            recent.results("output=csv 无匹配").refillable_text(),
            Some("文件名,预览方式,路径")
        );
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
        assert!(index.current.is_none());
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
    fn navigates_back_and_forward_with_the_exact_filter_query() {
        let root = r"文件: C:\Work | budget | 仅文件夹 最近修改";
        let child = r"文件: C:\Work\Budget | *.pdf | 后缀=pdf";
        let nested = r"文件: C:\Work\Budget\2026 |";
        let mut index = LocalFilesIndex::default();
        index.remember_navigation(root.to_owned());
        index.remember_navigation(child.to_owned());
        assert_eq!(
            index.move_navigation(nested.to_owned(), false).as_deref(),
            Some(child)
        );
        assert_eq!(
            index.move_navigation(child.to_owned(), false).as_deref(),
            Some(root)
        );
        assert_eq!(
            index.move_navigation(root.to_owned(), true).as_deref(),
            Some(child)
        );
        assert_eq!(
            index.move_navigation(child.to_owned(), true).as_deref(),
            Some(nested)
        );
        assert_eq!(index.move_navigation(nested.to_owned(), true), None);
        assert_eq!(
            index.move_navigation(nested.to_owned(), false).as_deref(),
            Some(child)
        );
        index.remember_navigation(child.to_owned());
        assert_eq!(
            index.move_navigation("文件: C:\\Other |".to_owned(), true),
            None
        );

        let mut index = LocalFilesIndex::default();
        for i in 0..=MAX_NAVIGATION_HISTORY {
            index.remember_navigation(format!("文件: C:\\Work\\{i} |"));
        }
        assert_eq!(index.back.len(), MAX_NAVIGATION_HISTORY);
        assert_eq!(
            index.back.front().map(String::as_str),
            Some(r"文件: C:\Work\1 |")
        );
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
    #[cfg(windows)]
    fn exports_escaped_file_and_folder_urls_in_filtered_result_order() {
        let mut index = LocalFilesIndex::default();
        let query = r"C:\Work | | 文件网址列表";
        let load = index.request(Some(query)).unwrap();
        assert!(index.finish(
            &load,
            Ok(vec![
                entry("a # folder", "directory"),
                entry("z # report.txt", "file"),
                entry("other.url", "unavailable"),
            ]),
        ));
        assert!(matches!(
            index.results(query),
            Results::Value(Ok(value)) if value.text
                == "file:///C:/Work/a%20%23%20folder/\nfile:///C:/Work/z%20%23%20report.txt"
        ));
        assert!(matches!(
            index.results(r"C:\Work | omitted | file-urls"),
            Results::Destinations(items) if items.is_empty()
        ));
        assert!(split_query(r"C:\Work | | 文件网址列表 路径列表").is_none());
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
    fn filters_file_dates_by_inclusive_local_calendar_days_and_reuses_timestamps() {
        use omnitrigger_windows::LocalDirectoryMetadata;

        let mut index = LocalFilesIndex::default();
        let plain = index.request(Some(r"C:\Work |")).unwrap();
        assert!(!plain.timestamps);
        let query = r"C:\Work | | 修改>=2026-11-21 修改<=2026-11-22 created<=2026-11-20";
        let load = index.request(Some(query)).unwrap();
        assert!(load.timestamps);
        let first = calendar_day("2026-11-21").unwrap();
        let with_times = |name, created, modified, available| {
            let mut file = entry(name, "file");
            file.metadata = Some(LocalDirectoryMetadata {
                ordinal: 0,
                attributes: 0,
                creation_utc_100ns: 1,
                access_utc_100ns: 0,
                write_utc_100ns: u64::from(available),
                creation_local_ticks: created * TICKS_PER_DAY,
                access_local_ticks: 0,
                write_local_ticks: (modified + 1) * TICKS_PER_DAY - 1,
                time_zone_key: "测试".into(),
            });
            file
        };
        assert!(index.finish(
            &load,
            Ok(vec![
                with_times("lower.txt", first - 1, first, true),
                with_times("upper.txt", first - 2, first + 1, true),
                with_times("old.txt", first - 2, first - 1, true),
                with_times("new.txt", first - 1, first + 2, true),
                with_times("created.txt", first, first + 1, true),
                with_times("unknown.txt", first - 1, first, false),
                entry("unavailable.txt", "file"),
            ])
        ));
        assert!(matches!(
            index.results(query), Results::Destinations(items)
                if items.iter().filter_map(|item| match item {
                    Destination::LocalFile { name, .. } => Some(name.as_str()),
                    _ => None,
                }).eq(["lower.txt", "upper.txt"])
        ));
        assert!(index
            .request(Some(r"C:\Work | | 创建>=2026-11-09"))
            .is_none());
        assert!(split_query(r"C:\Work | | 修改>=2026-11-31").is_none());
        assert!(split_query(r"C:\Work | | 创建>=2026-11-22 创建<=2026-11-21").is_none());
        assert!(split_query(r"C:\Work | | 修改>=2026-11-21 modified>=2026-11-21").is_none());
    }

    #[test]
    fn exports_precise_utc_timestamps_and_keeps_filtered_names_and_order() {
        let mut index = LocalFilesIndex::default();
        let query = r"C:\Work | report | 最近修改 时间CSV";
        let load = index.request(Some(query)).unwrap();
        assert!(load.timestamps);
        let mut report = entry("report,final.txt", "file");
        report.size_bytes = Some(0);
        report.metadata = Some(omnitrigger_windows::LocalDirectoryMetadata {
            ordinal: 0,
            attributes: 0,
            creation_utc_100ns: 116_444_735_999_999_999,
            access_utc_100ns: 0,
            write_utc_100ns: 116_444_736_000_000_000,
            creation_local_ticks: 0,
            access_local_ticks: 0,
            write_local_ticks: 0,
            time_zone_key: "测试".to_owned(),
        });
        let entries = vec![
            report,
            entry("other.txt", "file"),
            entry("report folder", "directory"),
        ];
        assert!(index.finish(&load, Ok(entries.clone())));
        assert!(matches!(
            index.results(query),
            Results::Value(Ok(value))
                if matches!(&value.kind, ValueKind::Table(table) if table.column_pages() == 2) && value.text == concat!(
                    "名称,创建时间UTC,修改时间UTC,路径\r\n",
                    "\"report,final.txt\",1969-12-31T23:59:59.999999900Z,",
                    "1970-01-01T00:00:00Z,\"C:\\Work\\report,final.txt\"\r\n",
                    "report folder,\"\",\"\",C:\\Work\\report folder",
                )
        ));
        assert!(matches!(
            index.results(r"C:\Work | missing | 最近修改 时间CSV"),
            Results::Value(Ok(value)) if value.text == "名称,创建时间UTC,修改时间UTC,路径"
        ));
        let inventory = r"C:\Work | report | 最近修改 清单CSV";
        let inventory_load = index.request(Some(inventory)).unwrap();
        assert!(inventory_load.file_sizes && inventory_load.timestamps);
        assert_eq!(inventory_load.order, DirectoryOrder::ModifiedDescending);
        assert!(index.finish(&inventory_load, Ok(entries)));
        assert!(matches!(
            index.results(inventory),
            Results::Value(Ok(value))
                if matches!(&value.kind, ValueKind::Table(table) if table.column_pages() == 3) && value.text == concat!(
                    "名称,类型,字节数,创建时间UTC,修改时间UTC,路径\r\n",
                    "\"report,final.txt\",文件,0,1969-12-31T23:59:59.999999900Z,",
                    "1970-01-01T00:00:00Z,\"C:\\Work\\report,final.txt\"\r\n",
                    "report folder,文件夹,\"\",\"\",\"\",C:\\Work\\report folder",
                )
        ));
        assert!(matches!(
            index.results(r"C:\Work | missing | 最近修改 inventory-csv"),
            Results::Value(Ok(value)) if value.text == "名称,类型,字节数,创建时间UTC,修改时间UTC,路径"
        ));
        assert!(split_query(r"C:\Work | | 时间CSV 清单CSV").is_none());
    }

    #[test]
    fn totals_exact_bytes_for_filtered_local_files() {
        let mut index = LocalFilesIndex::default();
        let query = r"C:\Work | 报告 | 大小合计";
        let load = index.request(Some(query)).unwrap();
        assert_eq!(load.order, DirectoryOrder::SizeDescending);
        assert_eq!(load.entry_type, DirectoryEntryType::File);
        let sized = |name, size| {
            let mut file = entry(name, "file");
            file.size_bytes = Some(size);
            file
        };
        assert!(index.finish(
            &load,
            Ok(vec![
                sized("报告一.txt", u64::MAX),
                sized("报告二.txt", u64::MAX),
                sized("草稿.md", 12),
                entry("未知.bin", "file"),
            ])
        ));
        assert!(matches!(index.results(query), Results::Value(Ok(value))
            if value.text == "2 个文件 · 合计 36893488147419103230 B"));
        assert!(
            matches!(index.results(r"C:\Work | 草稿 | size-total"), Results::Value(Ok(value))
            if value.text == "1 个文件 · 合计 12 B")
        );
        assert!(
            matches!(index.results(r"C:\Work | 缺失 | 大小合计"), Results::Value(Ok(value))
            if value.text == "0 个文件 · 合计 0 B")
        );
        assert!(matches!(
            index.results(r"C:\Work | 未知 | 大小合计"),
            Results::LocalFilesUnavailable(_)
        ));
        assert!(split_query(r"C:\Work | | 大小合计 仅文件夹").is_none());
    }

    #[test]
    fn filters_several_case_insensitive_file_extensions_without_a_new_read() {
        let mut index = LocalFilesIndex::default();
        let query = r"C:\Work | | 后缀=.PDF,md";
        let load = index.request(Some(query)).unwrap();
        assert_eq!(load.entry_type, DirectoryEntryType::File);
        assert!(index.finish(
            &load,
            Ok(vec![
                entry("报告.PDF", "file"),
                entry("说明.MD", "file"),
                entry("归档.pdf.zip", "file"),
                entry(".md", "file"),
                entry("旧资料.pdf", "directory"),
            ])
        ));
        assert!(matches!(
            index.results(query), Results::Destinations(items)
                if items.iter().filter_map(|item| match item {
                    Destination::LocalFile { name, .. } => Some(name.as_str()),
                    _ => None,
                }).eq(["报告.PDF", "说明.MD"])
        ));
        assert!(index.request(Some(r"C:\Work | | ext=zip")).is_none());
        assert!(
            matches!(index.results(r"C:\Work | | ext=zip"), Results::Destinations(items)
            if matches!(items.as_slice(), [Destination::LocalFile { name, .. }] if name == "归档.pdf.zip"))
        );
        assert!(split_query(r"C:\Work | | 后缀=pdf,").is_none());
        assert!(split_query(r"C:\Work | | 后缀=pdf 仅文件夹").is_none());
    }

    #[test]
    fn filters_previewable_file_names_using_the_same_extensions_as_preview() {
        let mut index = LocalFilesIndex::default();
        let query = r"C:\Work | | 仅可预览";
        let load = index.request(Some(query)).unwrap();
        assert_eq!(load.entry_type, DirectoryEntryType::File);
        assert!(index.finish(
            &load,
            Ok(vec![
                entry("说明.MD", "file"),
                entry("配置.YAML", "file"),
                entry("报告.pdf", "file"),
                entry("设置.yaml.exe", "file"),
                entry("旧配置.yaml", "directory"),
            ])
        ));
        assert!(matches!(
            index.results(query),
            Results::Destinations(items)
                if items.iter().filter_map(|item| match item {
                    Destination::LocalFile { name, .. } => Some(name.as_str()),
                    _ => None,
                }).eq(["说明.MD", "配置.YAML"])
        ));
        let combined = r"C:\Work | | previewable 后缀=yaml";
        assert!(index.request(Some(combined)).is_none());
        assert!(matches!(
            index.results(combined),
            Results::Destinations(items)
                if matches!(items.as_slice(), [Destination::LocalFile { name, .. }] if name == "配置.YAML")
        ));
        assert!(split_query(r"C:\Work | | 仅可预览 仅文件夹").is_none());
        assert!(split_query(r"C:\Work | | 仅可预览 previewable").is_none());
    }

    #[test]
    fn limits_the_ranked_matches_before_exporting_or_summarizing_them() {
        let mut index = LocalFilesIndex::default();
        let query = r"C:\Work | report | 前=1";
        let load = index.request(Some(query)).unwrap();
        assert!(index.finish(
            &load,
            Ok(vec![
                entry("draft report.txt", "file"),
                entry("report", "directory"),
            ])
        ));
        assert!(matches!(
            index.results(query),
            Results::Destinations(items) if matches!(
                items.as_slice(), [Destination::LocalFile { name, .. }] if name == "report"
            )
        ));
        let list = r"C:\Work | report | limit=1 路径JSON";
        assert!(index.request(Some(list)).is_none());
        assert!(matches!(index.results(list), Results::Value(Ok(value))
            if value.text == r#"["C:\\Work\\report"]"#));

        let sum = r"C:\Work | | 大小合计 前=2";
        let load = index.request(Some(sum)).unwrap();
        let with_size = |name, size| {
            let mut file = entry(name, "file");
            file.size_bytes = Some(size);
            file
        };
        assert!(index.finish(
            &load,
            Ok(vec![
                with_size("b.docx", 7),
                with_size("c.tmp", 4),
                with_size("a.PDF", 3),
            ])
        ));
        assert!(matches!(index.results(sum), Results::Value(Ok(value))
            if value.text == "2 个文件 · 合计 11 B"));
        for invalid in [
            r"C:\Work | | 前=0",
            r"C:\Work | | 前=2 limit=1",
            r"C:\Work | | 前=1025",
        ] {
            assert!(split_query(invalid).is_none(), "{invalid}");
        }
    }

    #[test]
    fn groups_files_by_local_month_week_or_day_and_keeps_missing_times_separate() {
        let modified = r"C:\Work | | 修改月份统计";
        let created = r"C:\Work | | created-month-counts";
        let modified_days = r"C:\Work | | 修改日期统计";
        let created_days = r"C:\Work | | created-day-counts";
        let mut index = LocalFilesIndex::default();
        let load = index.request(Some(modified)).unwrap();
        assert!(load.timestamps);
        assert_eq!(load.entry_type, DirectoryEntryType::File);
        let timed_entry = |name, kind, created, modified| {
            let mut entry = entry(name, kind);
            entry.metadata = Some(omnitrigger_windows::LocalDirectoryMetadata {
                ordinal: 0,
                attributes: 0,
                creation_utc_100ns: 116_444_736_000_000_000,
                access_utc_100ns: 0,
                write_utc_100ns: 116_444_736_000_000_000,
                creation_local_ticks: calendar_day(created).unwrap() * TICKS_PER_DAY,
                access_local_ticks: 0,
                write_local_ticks: (calendar_day(modified).unwrap() + 1) * TICKS_PER_DAY - 1,
                time_zone_key: "测试".to_owned(),
            });
            entry
        };
        assert!(index.finish(
            &load,
            Ok(vec![
                timed_entry("a.txt", "file", "2020-12-31", "2024-01-31"),
                timed_entry("b.txt", "file", "2021-01-01", "2024-02-01"),
                timed_entry("c.pdf", "file", "2021-01-01", "2024-02-29"),
                timed_entry("ignored", "directory", "2024-02-01", "2024-02-01"),
                entry("unknown.txt", "file"),
            ]),
        ));
        assert!(index.request(Some(created)).is_none());
        assert!(index.request(Some(modified_days)).is_none());
        for (query, expected) in [
            (
                modified,
                "月份,数量\r\n2024-02,2\r\n2024-01,1\r\n时间未知,1",
            ),
            (created, "月份,数量\r\n2021-01,2\r\n2020-12,1\r\n时间未知,1"),
            (
                modified_days,
                "日期,数量\r\n2024-02-29,1\r\n2024-02-01,1\r\n2024-01-31,1\r\n时间未知,1",
            ),
            (
                created_days,
                "日期,数量\r\n2021-01-01,2\r\n2020-12-31,1\r\n时间未知,1",
            ),
            (
                r"C:\Work | | 修改周统计",
                "周,数量\r\n2024-W09,1\r\n2024-W05,2\r\n时间未知,1",
            ),
            (
                r"C:\Work | | created-week-counts",
                "周,数量\r\n2020-W53,3\r\n时间未知,1",
            ),
            (
                r"C:\Work | | 修改月份统计 修改>=2024-02-01 后缀=txt",
                "月份,数量\r\n2024-02,1",
            ),
            (r"C:\Work | absent | 修改月份统计", "月份,数量"),
            (r"C:\Work | absent | 修改日期统计", "日期,数量"),
            (r"C:\Work | absent | 修改周统计", "周,数量"),
        ] {
            assert!(
                matches!(
                    index.results(query), Results::Value(Ok(value))
                        if matches!(&value.kind, ValueKind::Table(_)) && value.text == expected
                ),
                "{query}"
            );
        }
        assert!(split_query(r"C:\Work | | 修改月份统计 仅文件夹").is_none());
        assert!(split_query(r"C:\Work | | 修改月份统计 创建月份统计").is_none());
    }

    #[test]
    #[cfg(windows)]
    fn suggests_browsing_a_bare_drive_path_without_requesting_directory_contents() {
        let mut index = LocalFilesIndex::default();
        let path = r#""C:\Work Folder""#;
        let result = index.results(path);
        assert_eq!(result.suggested_input(0), Some(r"文件: C:\Work Folder |"));
        assert!(index.request(Some(path)).is_none());
        assert!(index.current.is_none());
        for path in [
            r"C:Work",
            r"C:\Work\..\Project",
            r"\\server\share",
            r"C:\Work?",
            r"C:\Work:stream",
        ] {
            assert!(matches!(index.results(path), Results::LocalFilesPrompt));
        }
        assert!(matches!(
            index.results(r"C:\Work |"),
            Results::LocalFilesLoading
        ));
    }

    #[test]
    fn excludes_file_extensions_case_insensitively_and_keeps_extensionless_files() {
        let query = r"C:\Work | | 排除后缀=.tmp，BAK 名称列表";
        let mut index = LocalFilesIndex::default();
        let load = index.request(Some(query)).unwrap();
        assert_eq!(load.entry_type, DirectoryEntryType::File);
        assert!(index.finish(
            &load,
            Ok(vec![
                entry("a.TMP", "file"),
                entry("b.bak", "file"),
                entry("c.pdf", "file"),
                entry("README", "file"),
                entry("directory", "directory"),
            ])
        ));
        assert!(matches!(
            index.results(query),
            Results::Value(Ok(value)) if value.text == "c.pdf\nREADME"
        ));
        let combined = r"C:\Work | | 后缀=pdf,tmp exclude-ext=tmp 名称列表";
        assert!(index.request(Some(combined)).is_none());
        assert!(matches!(
            index.results(combined),
            Results::Value(Ok(value)) if value.text == "c.pdf"
        ));
        for invalid in [
            r"C:\Work | | 排除后缀=tmp,",
            r"C:\Work | | 排除后缀=tmp exclude-ext=bak",
            r"C:\Work | | 排除后缀=tmp 仅文件夹",
        ] {
            assert!(split_query(invalid).is_none(), "{invalid}");
        }
    }

    #[test]
    fn sorts_by_extension_within_the_snapshot_before_taking_the_requested_prefix() {
        let mut index = LocalFilesIndex::default();
        let load = index.request(Some(r"C:\Work |")).unwrap();
        assert!(index.finish(
            &load,
            Ok(vec![
                entry("a.PDF", "file"),
                entry("archive.zip", "file"),
                entry("b.DOCX", "file"),
                entry("c.pdf", "file"),
                entry("folder.docx", "directory"),
                entry("README", "file"),
                entry("z.docx", "file"),
            ])
        ));
        let limited = r"C:\Work | | 按后缀 前=4 名称列表";
        assert!(index.request(Some(limited)).is_none());
        assert!(matches!(
            index.results(limited),
            Results::Value(Ok(value)) if value.text == "b.DOCX\nz.docx\na.PDF\nc.pdf"
        ));
        assert!(matches!(
            index.results(r"C:\Work | | extension-order 名称列表"),
            Results::Value(Ok(value))
                if value.text == "b.DOCX\nz.docx\na.PDF\nc.pdf\narchive.zip\nREADME\nfolder.docx"
        ));
        for options in ["按后缀 仅文件夹", "按后缀 大小", "按后缀 自然排序"] {
            assert!(split_query(&format!(r"C:\Work | | {options}")).is_none());
        }
    }

    #[test]
    fn sorts_file_names_naturally_before_taking_the_requested_prefix() {
        let plain = r"C:\Work | report | 名称列表";
        let sorted = r"C:\Work | report | 自然排序 名称列表 前=2";
        let mut index = LocalFilesIndex::default();
        let load = index.request(Some(plain)).unwrap();
        assert!(index.finish(
            &load,
            Ok(vec![
                entry("report1.txt", "file"),
                entry("report10.txt", "file"),
                entry("report2.txt", "file"),
                entry("other.txt", "file"),
            ])
        ));
        assert!(index.request(Some(sorted)).is_none());
        assert!(matches!(
            index.results(plain),
            Results::Value(Ok(value)) if value.text == "report1.txt\nreport10.txt\nreport2.txt"
        ));
        assert!(matches!(
            index.results(sorted),
            Results::Value(Ok(value)) if value.text == "report1.txt\nreport2.txt"
        ));
        assert!(split_query(r"C:\Work | | 最近修改 自然排序").is_none());
        assert!(split_query(r"C:\Work | | 自然排序 大小>=1").is_none());
    }

    #[test]
    fn offers_only_compatible_file_options_and_preserves_the_directory_query() {
        let mut index = LocalFilesIndex::default();
        let query = r#""C:\Work Folder" | report | 大小"#;
        let help = r#""C:\Work Folder" | report | 大小 ？"#;
        assert_eq!(
            option_help_query(query).as_deref(),
            Some(r#"文件: "C:\Work Folder" | report | 大小 ?"#)
        );
        assert_eq!(
            option_help_query(r"C:\Work | report").as_deref(),
            Some(r"文件: C:\Work | report | ?")
        );
        assert!(index.request(Some(help)).is_none());
        assert!(index.current.is_none());
        let Results::FileOptions(all) = index.results(r"C:\Work | | ?") else {
            panic!("缺少文件选项");
        };
        assert_eq!(all.len(), OPTION_HELP.len());
        let load = index.request(Some(query)).unwrap();
        assert!(index.request(Some(help)).is_none());
        assert!(Arc::ptr_eq(&load, &index.current.as_ref().unwrap().load));
        let Results::FileOptions(items) = index.results(help) else {
            panic!("缺少文件选项");
        };
        assert!(items.iter().any(|item| {
            item.label == Some("大小合计")
                && item.expression == r#"文件: "C:\Work Folder" | report | 大小 大小合计"#
        }));
        assert!(!items.iter().any(|item| {
            matches!(
                item.label,
                Some("大小" | "最小优先" | "最近修改" | "最早修改" | "最近创建" | "最早创建")
            )
        }));
        assert!(matches!(
            index.results(r"C:\Work | report?"),
            Results::LocalFilesLoading
        ));
        assert!(matches!(
            index.results(r"C:\Work | report | 前=1?"),
            Results::LocalFilesPrompt
        ));
    }

    #[test]
    fn totals_bytes_by_extension_and_sorts_groups_from_largest_to_smallest() {
        let mut index = LocalFilesIndex::default();
        let query = r"C:\Work | | 后缀大小";
        let load = index.request(Some(query)).unwrap();
        assert_eq!(load.entry_type, DirectoryEntryType::File);
        assert_eq!(load.order, DirectoryOrder::SizeDescending);
        let with_size = |name, size| {
            let mut file = entry(name, "file");
            file.size_bytes = Some(size);
            file
        };
        assert!(index.finish(
            &load,
            Ok(vec![
                with_size("a.PDF", u64::MAX),
                with_size("b.pdf", 1),
                with_size("a.txt", 5),
                with_size("README", 5),
                entry("ignored.txt", "directory"),
            ])
        ));
        assert!(matches!(
            index.results(query),
            Results::Value(Ok(value)) if matches!(&value.kind, ValueKind::Table(_)) && value.text ==
                "扩展名,数量,字节数\r\n.pdf,2,18446744073709551616\r\n无扩展名,1,5\r\n.txt,1,5"
        ));
        assert!(matches!(
            index.results(r"C:\Work | | 后缀=txt 后缀大小"),
            Results::Value(Ok(value)) if value.text == "扩展名,数量,字节数\r\n.txt,1,5"
        ));
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
                if matches!(&value.kind, ValueKind::Table(_)) && value.text == "扩展名,数量"
        ));
        assert!(split_query(r"C:\Work | | 后缀统计 仅文件夹").is_none());
        assert!(split_query(r"C:\Work | | 后缀统计 路径列表").is_none());
    }

    #[test]
    fn reports_files_with_shared_stems_using_the_filtered_file_order() {
        let mut index = LocalFilesIndex::default();
        let query = r"C:\Work | | 同主名CSV";
        let load = index.request(Some(query)).unwrap();
        assert_eq!(load.entry_type, DirectoryEntryType::File);
        assert!(index.finish(
            &load,
            Ok(vec![
                entry("报告,1.PDF", "file"),
                entry("readme", "file"),
                entry("报告,1.docx", "file"),
                entry("README.md", "file"),
                entry("报告,1.txt", "directory"),
                entry("单独.xlsx", "file"),
            ]),
        ));
        assert_eq!(
            index.results(query).refillable_text(),
            Some(concat!(
                "主名,组内文件数,文件名,路径",
                "\r\n\"报告,1\",2,\"报告,1.PDF\",\"C:\\Work\\报告,1.PDF\"",
                "\r\nreadme,2,readme,C:\\Work\\readme",
                "\r\n\"报告,1\",2,\"报告,1.docx\",\"C:\\Work\\报告,1.docx\"",
                "\r\nREADME,2,README.md,C:\\Work\\README.md",
            )),
        );
        assert_eq!(
            index
                .results(r"C:\Work | 报告 | 同主名CSV 后缀=docx")
                .refillable_text(),
            Some("主名,组内文件数,文件名,路径"),
        );
        assert_eq!(
            index
                .results(r"C:\Work | | shared-stem-csv 前=2")
                .refillable_text(),
            Some("主名,组内文件数,文件名,路径"),
        );
        assert!(split_query(r"C:\Work | | 同主名CSV 仅文件夹").is_none());
    }

    #[test]
    fn retains_ready_file_results_while_reloading() {
        let mut index = LocalFilesIndex::default();
        let query = r"C:\Work |";
        let initial = index.request(Some(query)).unwrap();
        assert!(index.reload(query).is_none());
        assert!(index.finish(&initial, Ok(vec![entry("a.txt", "file")])));
        let previous = index.results(query);
        assert!(matches!(&previous, Results::Destinations(items) if items.len() == 1));
        let reload = index.reload(query).unwrap();
        assert!(index.reload(query).is_none());
        assert!(index.request(Some(query)).is_none());
        assert!(index.results(query) == previous);
        assert!(!index.finish(&initial, Ok(Vec::new())));
        assert!(index.results(query) == previous);
        assert!(index.finish(&reload, Ok(vec![entry("b.txt", "file")])));
        assert!(index.results(query) != previous);
        assert!(!index.finish(&reload, Ok(Vec::new())));

        let reload = index.reload(query).unwrap();
        let other = index.request(Some(r"C:\Other |")).unwrap();
        assert!(!index.finish(&reload, Ok(Vec::new())));
        assert!(index.finish(&other, Ok(Vec::new())));
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
    fn supports_ascending_file_sort_with_range_limit_and_export() {
        let mut index = LocalFilesIndex::default();
        let query = r"C:\Work | | 最小优先 大小>=1 前=1 名称列表";
        let load = index.request(Some(query)).unwrap();
        assert_eq!(load.order, DirectoryOrder::SizeAscending);
        assert_eq!(load.entry_type, DirectoryEntryType::File);
        let with_size = |name, size| {
            let mut file = entry(name, "file");
            file.size_bytes = Some(size);
            file
        };
        assert!(index.finish(
            &load,
            Ok(vec![
                with_size("empty.txt", 0),
                with_size("small.txt", 1),
                with_size("large.txt", 9),
            ]),
        ));
        assert!(index.results(query) == Results::text_value(Ok("small.txt".into())));
        for (options, order) in [
            (
                "最早修改 修改>=2026-01-01",
                DirectoryOrder::ModifiedAscending,
            ),
            ("最早创建 仅文件夹", DirectoryOrder::CreatedAscending),
            ("最小优先 大小合计", DirectoryOrder::SizeAscending),
        ] {
            assert_eq!(
                split_query(&format!(r"C:\Work | | {options}")).map(|query| query.order),
                Some(order)
            );
        }
        assert!(split_query(r"C:\Work | | 大小 最小优先").is_none());
    }

    #[test]
    fn excludes_any_name_fragment_without_loading_the_directory_again() {
        let mut index = LocalFilesIndex::default();
        let original = r"C:\Work | Budget";
        let load = index.request(Some(original)).unwrap();
        assert!(index.finish(
            &load,
            Ok(vec![
                entry("Budget Final.txt", "file"),
                entry("Budget Draft.txt", "file"),
                entry("Budget 副本", "directory"),
                entry("Notes.txt", "file"),
            ]),
        ));
        for options in ["排除名称=DRAFT，副本", "EXCLUDE-NAME=draft,副本"] {
            let query = format!("{original} | {options} 名称列表");
            assert!(index.request(Some(&query)).is_none());
            assert!(index.results(&query) == Results::text_value(Ok("Budget Final.txt".into())));
        }
        for invalid in ["排除名称=", "排除名称=a,,b", "排除名称=a 排除名称=b"] {
            assert!(split_query(&format!("{original} | {invalid}")).is_none());
        }
    }

    #[test]
    fn includes_any_name_fragment_and_combines_with_required_and_excluded_terms() {
        let mut index = LocalFilesIndex::default();
        let original = r"C:\Work | 2026";
        let load = index.request(Some(original)).unwrap();
        assert!(index.finish(
            &load,
            Ok(vec![
                entry("2026 Budget.txt", "file"),
                entry("2026 发票", "directory"),
                entry("2026 Budget Draft.txt", "file"),
                entry("2026 Notes.txt", "file"),
                entry("2025 发票", "directory"),
            ]),
        ));
        for options in ["任一名称=BUDGET，发票", "ANY-NAME=budget,发票"] {
            let query = format!("{original} | {options} 排除名称=draft 名称列表");
            assert!(index.request(Some(&query)).is_none());
            assert!(
                index.results(&query)
                    == Results::text_value(Ok("2026 Budget.txt\n2026 发票".into()))
            );
        }
        let unmatched = format!("{original} | 任一名称=receipt 路径JSON");
        assert!(index.request(Some(&unmatched)).is_none());
        assert!(index.results(&unmatched) == Results::text_value(Ok("[]".into())));
        let help = format!("{original} | 任一名称=budget ?");
        let Results::FileOptions(items) = index.results(&help) else {
            panic!("缺少文件选项");
        };
        assert!(!items
            .iter()
            .any(|item| item.label == Some("任一名称=预算,发票")));
        for invalid in ["任一名称=", "任一名称=a，，b", "任一名称=a any-name=b"] {
            assert!(split_query(&format!("{original} | {invalid}")).is_none());
        }
    }

    #[test]
    fn treats_quoted_file_name_phrases_as_contiguous_text() {
        let mut index = LocalFilesIndex::default();
        let phrase = r#"C:\Work | 2026 "报告 初稿""#;
        let words = r"C:\Work | 2026 报告 初稿";
        let load = index.request(Some(phrase)).unwrap();
        assert!(index.finish(
            &load,
            Ok(vec![
                entry("2026 报告 初稿.docx", "file"),
                entry("2026 报告-初稿.docx", "file"),
                entry("2025 报告 初稿.docx", "file"),
            ])
        ));
        assert!(matches!(
            index.results(phrase),
            Results::Destinations(items)
                if matches!(items.as_slice(), [Destination::LocalFile { name, .. }] if name == "2026 报告 初稿.docx")
        ));
        assert!(index.request(Some(words)).is_none());
        assert_eq!(index.results(words).selectable_count(), 2);
        assert_eq!(wildcard_pattern(r#""报告 *.pdf""#), Some("报告 *.pdf"));
    }

    #[test]
    fn filters_original_file_names_by_regex_without_reloading_the_directory() {
        let mut index = LocalFilesIndex::default();
        let original = r"C:\Work | Budget";
        let load = index.request(Some(original)).unwrap();
        assert!(index.finish(
            &load,
            Ok(vec![
                entry("Budget 02.txt", "file"),
                entry("Budget 2.txt", "file"),
                entry("Budget 07.TXT", "file"),
                entry("Notes 02.txt", "file"),
                entry("Budget 05 Archive", "directory"),
            ]),
        ));
        let query = format!(r"{original} | 名称正则=\s[0-9]{{2}}\.txt$ 名称列表");
        assert!(index.request(Some(&query)).is_none());
        assert!(index.results(&query) == Results::text_value(Ok("Budget 02.txt".into())));
        let query = format!(r"{original} | NAME-REGEX=(?i)^\w+\s[0-9]{{2}}\.txt$ 名称列表");
        assert!(index.request(Some(&query)).is_none());
        assert!(
            index.results(&query) == Results::text_value(Ok("Budget 02.txt\nBudget 07.TXT".into()))
        );
        for pattern in ["", "("] {
            let query = format!("{original} | 名称正则={pattern}");
            assert!(matches!(
                index.results(&query),
                Results::LocalFilesUnavailable(_)
            ));
        }
        assert!(split_query(r"C:\Work | | 名称正则=a name-regex=b").is_none());
        let Results::FileOptions(items) = index.results(r"C:\Work | | ?") else {
            panic!("缺少文件选项");
        };
        assert!(items
            .iter()
            .any(|item| item.label == Some(r"名称正则=^报告[0-9]+\.pdf$")));
    }

    #[test]
    fn recent_file_dates_include_today_across_year_boundaries() {
        let today = NaiveDate::from_ymd_opt(2027, 1, 2).unwrap();
        let query = split_query_on(
            r"C:\Work | *.txt | 修改近=3天 创建近=1 最早修改 名称列表",
            today,
        )
        .unwrap();
        let day = |input| calendar_day(input).unwrap() * TICKS_PER_DAY;
        assert!(query.needs_timestamps());
        assert!(query.modified.contains(1, day("2026-12-31")));
        assert!(query
            .modified
            .contains(1, day("2027-01-02") + TICKS_PER_DAY - 1));
        assert!(!query.modified.contains(1, day("2026-12-30")));
        assert!(!query.modified.contains(1, day("2027-01-03")));
        assert!(!query.modified.contains(0, day("2027-01-02")));
        assert!(!query.created.contains(1, day("2027-01-01")));
        assert!(query.created.contains(1, day("2027-01-02")));
        for options in [
            "修改近=0",
            "创建近=18446744073709551615",
            "修改近=7天 修改>=2027-01-01",
            "创建<=2027-01-02 创建近=5",
            "修改近=3 修改近=2",
        ] {
            assert!(split_query_on(&format!(r"C:\Work | | {options}"), today).is_none());
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
