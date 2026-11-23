use std::{cmp::Ordering, collections::HashMap, fmt::Write as _};

use omnitrigger_windows::{list_open_windows, OpenWindow};
use regex::Regex;
use serde::Serialize;

use super::{
    csv_tools::csv_cell,
    examples::search_key,
    text_tools::{compare_natural, compile_regex},
    title_rank, Destination, Results,
};

const MAX_WINDOW_EXPORT_BYTES: usize = 1024 * 1024;

enum WindowExport {
    Json,
    Csv,
    ProcessCounts,
}

#[derive(Serialize)]
struct WindowSummary<'a> {
    title: &'a str,
    process: Option<&'a str>,
}

#[derive(Debug)]
enum WindowQueryError {
    Filters,
    TitleRegex,
    Order,
}

impl WindowQueryError {
    fn message(&self) -> &'static str {
        match self {
            Self::Filters => "状态可用 已最小化 或 未最小化。进程和排除进程各填一个程序文件名，文件名含空格时加双引号。",
            Self::TitleRegex => "填写一个有效的标题正则表达式，含空格时用双引号包住。",
            Self::Order => "排序可用 标题 或 进程，每次查询可指定一种排序。",
        }
    }
}

#[derive(Clone, Copy)]
enum WindowOrder {
    Title,
    Process,
}

impl WindowOrder {
    fn compare(
        self,
        (left_process, left_title): (&str, &str),
        (right_process, right_title): (&str, &str),
    ) -> Ordering {
        let by_title = || compare_natural(left_title, right_title);
        match self {
            Self::Title => by_title(),
            Self::Process => left_process
                .is_empty()
                .cmp(&right_process.is_empty())
                .then_with(|| compare_natural(left_process, right_process))
                .then_with(by_title),
        }
    }
}

struct WindowFilter {
    process: Option<String>,
    excluded_process: Option<String>,
    minimized: Option<bool>,
    title_regex: Option<Regex>,
    order: Option<WindowOrder>,
    terms: String,
}

impl WindowFilter {
    fn matches_process(&self, process: &str) -> bool {
        self.process.as_deref().is_none_or(|name| process == name)
            && self.excluded_process.as_deref() != Some(process)
    }

    fn matches_state(&self, minimized: bool) -> bool {
        self.minimized.is_none_or(|expected| expected == minimized)
    }
}

pub(super) struct SearchWindow {
    window: OpenWindow,
    title_key: String,
    process_key: String,
}

#[derive(Default)]
pub(super) enum OpenWindowsIndex {
    #[default]
    Unloaded,
    Loading,
    Ready(Vec<SearchWindow>),
    Refreshing(Vec<SearchWindow>),
    Unavailable(String),
}

impl OpenWindowsIndex {
    pub(super) fn begin_loading(&mut self) -> bool {
        if matches!(self, Self::Loading | Self::Refreshing(_)) {
            return false;
        }
        if let Self::Ready(previous) = std::mem::replace(self, Self::Loading) {
            *self = Self::Refreshing(previous);
        }
        true
    }

    pub(super) fn load() -> Self {
        match list_open_windows() {
            Ok(windows) => Self::Ready(
                windows
                    .into_iter()
                    .map(|window| SearchWindow {
                        title_key: search_key(&window.title),
                        process_key: window
                            .process
                            .as_deref()
                            .map_or_else(String::new, search_key),
                        window,
                    })
                    .collect(),
            ),
            Err(error) => Self::Unavailable(error),
        }
    }

    pub(super) fn results(&self, query: &str) -> Results {
        let windows = match self {
            Self::Unloaded | Self::Loading => return Results::OpenWindowsLoading,
            Self::Unavailable(error) => return Results::OpenWindowsUnavailable(error.clone()),
            Self::Ready(windows) | Self::Refreshing(windows) => windows,
        };
        let (format, rest) = query.split_once(char::is_whitespace).unwrap_or((query, ""));
        let (export, filter) = match search_key(format).as_str() {
            "输出=json" | "output=json" => (Some(WindowExport::Json), rest.trim_start()),
            "输出=csv" | "output=csv" => (Some(WindowExport::Csv), rest.trim_start()),
            "输出=进程统计" | "output=process-counts" => {
                (Some(WindowExport::ProcessCounts), rest.trim_start())
            }
            _ => (None, query),
        };
        let filter = match window_query(filter) {
            Ok(filter) => filter,
            Err(error) => return Results::OpenWindowsUnavailable(error.message().into()),
        };
        let query = filter.terms.as_str();
        let terms: Vec<_> = query.split_whitespace().collect();
        let mut matches: Vec<_> = windows
            .iter()
            .filter(|window| {
                filter.matches_process(&window.process_key)
                    && filter.matches_state(window.window.minimized)
                    && filter
                        .title_regex
                        .as_ref()
                        .is_none_or(|pattern| pattern.is_match(&window.window.title))
            })
            .filter_map(|window| {
                let process_key = if filter.process.is_some() {
                    ""
                } else {
                    &window.process_key
                };
                window_rank(&window.title_key, process_key, query, &terms)
                    .map(|rank| (rank, window))
            })
            .collect();
        match filter.order {
            Some(order) if !matches!(export, Some(WindowExport::ProcessCounts)) => {
                matches.sort_by(|(_, left), (_, right)| {
                    order.compare(
                        (&left.process_key, &left.title_key),
                        (&right.process_key, &right.title_key),
                    )
                });
            }
            _ if !terms.is_empty() => matches.sort_by_key(|(rank, _)| *rank),
            _ => {}
        }
        if let Some(export) = export {
            let windows = matches.into_iter().map(|(_, window)| WindowSummary {
                title: &window.window.title,
                process: window.window.process.as_deref(),
            });
            return export_windows(export, windows);
        }
        Results::Destinations(
            matches
                .into_iter()
                .map(|(_, window)| Destination::OpenWindow(window.window.clone()))
                .collect(),
        )
    }
}

fn export_windows<'a>(
    format: WindowExport,
    windows: impl Iterator<Item = WindowSummary<'a>>,
) -> Results {
    let too_large = || Results::OpenWindowsUnavailable("窗口列表超过 1 MiB".into());
    match format {
        WindowExport::Json => match serde_json::to_string_pretty(&windows.collect::<Vec<_>>()) {
            Ok(json) if json.len() <= MAX_WINDOW_EXPORT_BYTES => Results::text_value(Ok(json)),
            Ok(_) => too_large(),
            Err(error) => Results::OpenWindowsUnavailable(format!("生成窗口列表失败：{error}")),
        },
        WindowExport::Csv => {
            let mut csv = String::from("标题,程序文件名");
            for window in windows {
                write!(
                    csv,
                    "\r\n{},{}",
                    csv_cell(window.title),
                    csv_cell(window.process.unwrap_or_default())
                )
                .expect("写入 String 不会出错");
                if csv.len() > MAX_WINDOW_EXPORT_BYTES {
                    return too_large();
                }
            }
            Results::text_value(Ok(csv))
        }
        WindowExport::ProcessCounts => {
            let mut counts = HashMap::new();
            for window in windows {
                let key = window.process.map(str::to_lowercase);
                let (_, count) = counts.entry(key).or_insert((window.process, 0usize));
                *count += 1;
            }
            let mut groups: Vec<_> = counts.into_iter().collect();
            groups.sort_unstable_by(|(left, (_, left_count)), (right, (_, right_count))| {
                right_count
                    .cmp(left_count)
                    .then_with(|| left.is_none().cmp(&right.is_none()))
                    .then_with(|| left.cmp(right))
            });
            let mut csv = String::from("程序文件名,窗口数");
            for (_, (process, count)) in groups {
                write!(csv, "\r\n{},{count}", csv_cell(process.unwrap_or_default()))
                    .expect("写入 String 不会出错");
                if csv.len() > MAX_WINDOW_EXPORT_BYTES {
                    return too_large();
                }
            }
            Results::text_value(Ok(csv))
        }
    }
}

fn window_query(mut query: &str) -> Result<WindowFilter, WindowQueryError> {
    use WindowQueryError::{Filters, Order, TitleRegex};

    let mut process = None;
    let mut excluded_process = None;
    let mut minimized = None;
    let mut title_regex = None;
    let mut order = None;
    while let Some((name, input)) = query.split_once(['=', '＝']) {
        let slot = match search_key(name).as_str() {
            "进程" | "process" => &mut process,
            "排除进程" | "exclude-process" => &mut excluded_process,
            "状态" | "state" => {
                if minimized.is_some() {
                    return Err(Filters);
                }
                let (state, rest) = input.split_once(char::is_whitespace).unwrap_or((input, ""));
                minimized = Some(match search_key(state).as_str() {
                    "已最小化" | "minimized" => true,
                    "未最小化" | "not-minimized" => false,
                    _ => return Err(Filters),
                });
                query = rest.trim_start();
                continue;
            }
            "标题正则" | "title-regex" => {
                if title_regex.is_some() {
                    return Err(TitleRegex);
                }
                let (pattern, rest) =
                    filter_value(input, |character| character == '"').ok_or(TitleRegex)?;
                title_regex = Some(compile_regex(pattern).map_err(|_| TitleRegex)?);
                query = rest;
                continue;
            }
            "排序" | "sort" => {
                if order.is_some() {
                    return Err(Order);
                }
                let (value, rest) = input.split_once(char::is_whitespace).unwrap_or((input, ""));
                order = Some(match search_key(value).as_str() {
                    "标题" | "title" => WindowOrder::Title,
                    "进程" | "process" => WindowOrder::Process,
                    _ => return Err(Order),
                });
                query = rest.trim_start();
                continue;
            }
            _ => break,
        };
        if slot.is_some() {
            return Err(Filters);
        }
        let (filename, rest) =
            filter_value(input, |character| matches!(character, '"' | '＂')).ok_or(Filters)?;
        let filename = search_key(filename);
        if filename.trim().is_empty() || filename.contains(['/', '\\', ':', '"']) {
            return Err(Filters);
        }
        *slot = Some(filename);
        query = rest;
    }
    Ok(WindowFilter {
        process,
        excluded_process,
        minimized,
        title_regex,
        order,
        terms: search_key(query),
    })
}

fn filter_value(input: &str, quote: impl Fn(char) -> bool) -> Option<(&str, &str)> {
    if let Some(quoted) = input.strip_prefix(&quote) {
        let (value, rest) = quoted.split_once(quote)?;
        if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
            return None;
        }
        return Some((value, rest.trim_start()));
    }
    let (value, rest) = input.split_once(char::is_whitespace).unwrap_or((input, ""));
    Some((value, rest.trim_start()))
}

fn window_rank(title: &str, process: &str, query: &str, terms: &[&str]) -> Option<(u8, bool)> {
    if !terms
        .iter()
        .all(|&term| title.contains(term) || process.contains(term))
    {
        return None;
    }
    let direct = title_rank(title, query).min(title_rank(process, query));
    let terms_outside_title = terms.iter().any(|&term| !title.contains(term));
    Some((direct, terms_outside_title))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_matching_windows_by_process_and_orders_equal_counts_by_name() {
        let windows = [
            WindowSummary {
                title: "甲",
                process: Some("Notepad.exe"),
            },
            WindowSummary {
                title: "乙",
                process: Some("notepad.exe"),
            },
            WindowSummary {
                title: "丙",
                process: None,
            },
            WindowSummary {
                title: "丁",
                process: Some("code.exe"),
            },
        ];
        assert!(matches!(
            export_windows(WindowExport::ProcessCounts, windows.into_iter()),
            Results::Value(Ok(value)) if value.text == "程序文件名,窗口数\r\nNotepad.exe,2\r\ncode.exe,1\r\n\"\",1"
        ));
        assert!(matches!(
            OpenWindowsIndex::Ready(Vec::new()).results("输出＝进程统计　进程=notepad.exe 标题"),
            Results::Value(Ok(value)) if value.text == "程序文件名,窗口数"
        ));
    }

    #[test]
    fn exports_window_titles_and_process_names_as_csv_in_match_order() {
        let windows = [
            WindowSummary {
                title: "预算,\"复核\"",
                process: Some("Example Tool.exe"),
            },
            WindowSummary {
                title: "资料",
                process: None,
            },
        ];
        assert!(matches!(
            export_windows(WindowExport::Csv, windows.into_iter()),
            Results::Value(Ok(result)) if result.text == "标题,程序文件名\r\n\"预算,\"\"复核\"\"\",Example Tool.exe\r\n资料,\"\""
        ));
        let index = OpenWindowsIndex::Ready(Vec::new());
        assert!(matches!(
            index.results("输出＝ＣＳＶ　进程=notepad.exe 报告"),
            Results::Value(Ok(result)) if result.text == "标题,程序文件名"
        ));
    }

    #[test]
    fn exports_empty_windows_as_json_and_retains_selector_errors() {
        let index = OpenWindowsIndex::Ready(Vec::new());
        for query in [
            "输出=JSON",
            "输出＝ＪＳＯＮ　进程=notepad.exe 报告",
            "output=json process=\"example tool.exe\" Budget",
        ] {
            assert!(matches!(
                index.results(query),
                Results::Value(Ok(value)) if value.text == "[]"
            ));
        }
        assert!(matches!(
            index.results("output=json process="),
            Results::OpenWindowsUnavailable(_)
        ));
        assert!(matches!(
            index.results("output=json助理"),
            Results::Destinations(items) if items.is_empty()
        ));
    }

    #[test]
    fn recognizes_exact_process_filters_and_separates_remaining_title_terms() {
        for (query, process, terms) in [
            (
                "进程＝ＯｍｎｉＴｒｉｇｇｅｒ．ｅｘｅ　报告",
                Some("omnitrigger.exe"),
                "报告",
            ),
            (
                r#"process="example tool.exe" 预算"#,
                Some("example tool.exe"),
                "预算",
            ),
            ("process=notes.exe", Some("notes.exe"), ""),
            ("预算 notes", None, "预算 notes"),
        ] {
            let filter = window_query(query).unwrap();
            assert_eq!(filter.process.as_deref(), process, "{query}");
            assert_eq!(filter.terms, terms, "{query}");
        }
        for invalid in [
            "进程=",
            r#"process="" 标题"#,
            r#"process="tool.exe"紧邻标题"#,
            r#"process="未闭合"#,
            r"process=C:\work\tool.exe",
        ] {
            assert!(window_query(invalid).is_err(), "{invalid}");
        }
        assert!(matches!(
            OpenWindowsIndex::Ready(Vec::new()).results("进程="),
            Results::OpenWindowsUnavailable(_)
        ));
    }

    #[test]
    fn excludes_exact_process_names_without_losing_terms_or_export() {
        let query = search_key("排除进程＝ＯｍｎｉＴｒｉｇｇｅｒ．ｅｘｅ　报告");
        let filter = window_query(&query).unwrap();
        assert_eq!(filter.terms, "报告");
        assert!(!filter.matches_process("omnitrigger.exe"));
        assert!(filter.matches_process("my-omnitrigger.exe"));
        assert!(filter.matches_process(""));
        for query in [
            r#"process=notes.exe exclude-process="example tool.exe" budget"#,
            r#"exclude-process="example tool.exe" process=notes.exe budget"#,
        ] {
            let filter = window_query(query).unwrap();
            assert_eq!(filter.terms, "budget");
            assert!(filter.matches_process("notes.exe"));
            assert!(!filter.matches_process("example tool.exe"));
        }
        let filter = window_query("进程=notes.exe 排除进程=notes.exe").unwrap();
        assert!(!filter.matches_process("notes.exe"));
        let index = OpenWindowsIndex::Ready(Vec::new());
        assert!(matches!(
            index.results("输出=CSV 排除进程=notes.exe 标题"),
            Results::Value(Ok(result)) if result.text == "标题,程序文件名"
        ));
        for query in ["排除进程=", "排除进程=a.exe 排除进程=b.exe"] {
            assert!(matches!(
                index.results(query),
                Results::OpenWindowsUnavailable(_)
            ));
        }
    }

    #[test]
    fn combines_window_state_and_process_filters_with_export_and_title_terms() {
        let query = search_key("进程=Notes.exe 状态＝已最小化 报告");
        let filter = window_query(&query).unwrap();
        assert_eq!(filter.terms, "报告");
        assert!(filter.matches_process("notes.exe"));
        assert!(!filter.matches_process("code.exe"));
        assert!(filter.matches_state(true));
        assert!(!filter.matches_state(false));

        let filter = window_query("state=not-minimized exclude-process=a.exe").unwrap();
        assert!(filter.matches_state(false));
        assert!(!filter.matches_state(true));
        let filter = window_query("报告").unwrap();
        assert!(filter.matches_state(true) && filter.matches_state(false));

        let index = OpenWindowsIndex::Ready(Vec::new());
        assert!(matches!(
            index.results("输出=CSV 状态=已最小化 进程=notes.exe"),
            Results::Value(Ok(result)) if result.text == "标题,程序文件名"
        ));
        for query in ["状态=", "状态=最大化", "状态=已最小化 state=not-minimized"] {
            assert!(matches!(
                index.results(query),
                Results::OpenWindowsUnavailable(_)
            ));
        }
    }

    #[test]
    fn filters_original_window_titles_by_regex_and_combines_other_selectors() {
        let filter = window_query(
            r#"标题正则＝"^Budget Ａ\d{2}$" ＰＲＯＣＥＳＳ=Notes.exe 状态=已最小化　预算"#,
        )
        .unwrap();
        assert_eq!(filter.terms, "预算");
        assert!(filter.matches_process("notes.exe"));
        assert!(filter.matches_state(true));
        let pattern = filter.title_regex.unwrap();
        assert!(pattern.is_match("Budget Ａ01"));
        assert!(!pattern.is_match("budget Ａ01"));
        assert!(!pattern.is_match("Budget A01"));
        assert!(!pattern.is_match("Draft Budget Ａ01"));

        let index = OpenWindowsIndex::Ready(Vec::new());
        assert!(matches!(
            index.results(r"输出＝ＪＳＯＮ title-regex=(?i)^Budget\s\d+$"),
            Results::Value(Ok(result)) if result.text == "[]"
        ));
        for query in [
            "标题正则=",
            "标题正则=[",
            "标题正则=a title-regex=b",
            r#"标题正则="未结束"#,
            r#"标题正则="a b"连着条件"#,
        ] {
            assert!(
                matches!(
                    index.results(query),
                    Results::OpenWindowsUnavailable(message) if message.contains("标题正则")
                ),
                "{query}"
            );
        }
    }

    #[test]
    fn sorts_normalized_window_titles_naturally_and_keeps_unknown_processes_last() {
        let titles = [
            ("notes.exe", "项目10"),
            ("notes.exe", "项目０２"),
            ("", "项目1"),
            ("editor.exe", "项目4"),
            ("notes.exe", "项目2"),
        ];
        let sorted = |query| {
            let filter = window_query(query).unwrap();
            assert_eq!(filter.terms, "项目");
            let order = filter.order.unwrap();
            let mut indices = [0, 1, 2, 3, 4];
            let keys = titles.map(|(process, title)| (search_key(process), search_key(title)));
            indices.sort_by(|&left, &right| {
                order.compare(
                    (&keys[left].0, &keys[left].1),
                    (&keys[right].0, &keys[right].1),
                )
            });
            indices
        };
        assert_eq!(sorted("排序=标题 状态=未最小化 项目"), [2, 1, 4, 3, 0]);
        assert_eq!(sorted("ＳＯＲＴ＝ＰＲＯＣＥＳＳ 项目"), [3, 1, 4, 0, 2]);

        let index = OpenWindowsIndex::Ready(Vec::new());
        assert!(matches!(
            index.results("输出=CSV 排序=标题 标题正则=^项目"),
            Results::Value(Ok(result)) if result.text == "标题,程序文件名"
        ));
        for query in ["排序=", "排序=创建时间", "排序=标题 sort=process"] {
            assert!(
                matches!(
                    index.results(query),
                    Results::OpenWindowsUnavailable(message) if message.contains("排序")
                ),
                "{query}"
            );
        }
    }

    #[test]
    fn matches_fullwidth_window_query_and_prioritizes_all_terms_in_the_title() {
        let query = search_key("Ｂｕｄｇｅｔ　Ｅｄｉｔｏｒ");
        let terms: Vec<_> = query.split_whitespace().collect();
        let rank = |title: &str, process: &str| {
            window_rank(&search_key(title), &search_key(process), &query, &terms)
        };
        assert_eq!(rank("Budget Editor", "notes.exe"), Some((0, false)));
        assert_eq!(rank("Ｅｄｉｔｏｒ • Budget", "notes.exe"), Some((3, false)));
        assert_eq!(rank("Budget", "Editor.exe"), Some((3, true)));
        assert_eq!(rank("Other title", "Editor.exe"), None);
    }
}
