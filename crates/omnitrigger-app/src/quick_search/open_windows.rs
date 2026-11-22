use std::fmt::Write as _;

use omnitrigger_windows::{list_open_windows, OpenWindow};
use serde::Serialize;

use super::{csv_tools::csv_cell, examples::search_key, title_rank, Destination, Results};

const MAX_WINDOW_EXPORT_BYTES: usize = 1024 * 1024;

enum WindowExport {
    Json,
    Csv,
}

#[derive(Serialize)]
struct WindowSummary<'a> {
    title: &'a str,
    process: Option<&'a str>,
}

struct WindowFilter<'a> {
    process: Option<&'a str>,
    excluded_process: Option<&'a str>,
    terms: &'a str,
}

impl WindowFilter<'_> {
    fn matches_process(&self, process: &str) -> bool {
        self.process.is_none_or(|name| process == name) && self.excluded_process != Some(process)
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
        let query = search_key(query);
        let (format, rest) = query
            .split_once(char::is_whitespace)
            .unwrap_or((&query, ""));
        let (export, filter) = match format {
            "输出=json" | "output=json" => (Some(WindowExport::Json), rest.trim_start()),
            "输出=csv" | "output=csv" => (Some(WindowExport::Csv), rest.trim_start()),
            _ => (None, query.as_str()),
        };
        let Some(filter) = window_query(filter) else {
            return Results::OpenWindowsUnavailable(
                "进程和排除进程各接受一个程序文件名，例如 排除进程=notepad.exe。文件名含空格时用双引号包住。".into(),
            );
        };
        let query = filter.terms;
        let terms: Vec<_> = query.split_whitespace().collect();
        let mut matches: Vec<_> = windows
            .iter()
            .filter(|window| filter.matches_process(&window.process_key))
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
        if !terms.is_empty() {
            matches.sort_by_key(|(rank, _)| *rank);
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
    }
}

fn window_query(mut query: &str) -> Option<WindowFilter<'_>> {
    let mut process = None;
    let mut excluded_process = None;
    loop {
        let (slot, input) = if let Some(input) = query
            .strip_prefix("进程=")
            .or_else(|| query.strip_prefix("process="))
        {
            (&mut process, input)
        } else if let Some(input) = query
            .strip_prefix("排除进程=")
            .or_else(|| query.strip_prefix("exclude-process="))
        {
            (&mut excluded_process, input)
        } else {
            break;
        };
        if slot.is_some() {
            return None;
        }
        let (filename, terms) = parse_process_name(input)?;
        *slot = Some(filename);
        query = terms;
    }
    Some(WindowFilter {
        process,
        excluded_process,
        terms: query,
    })
}

fn parse_process_name(input: &str) -> Option<(&str, &str)> {
    let (process, terms) = if let Some(quoted) = input.strip_prefix('"') {
        let (name, terms) = quoted.split_once('"')?;
        if !terms.is_empty() && !terms.starts_with(char::is_whitespace) {
            return None;
        }
        (name, terms)
    } else {
        input.split_at(input.find(char::is_whitespace).unwrap_or(input.len()))
    };
    (!process.trim().is_empty() && !process.contains(['/', '\\', ':', '"']))
        .then_some((process, terms.trim_start()))
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
        let process_and_terms =
            |query| window_query(query).map(|filter| (filter.process, filter.terms));
        let query = search_key("进程＝ＯｍｎｉＴｒｉｇｇｅｒ．ｅｘｅ　报告");
        assert_eq!(
            process_and_terms(&query),
            Some((Some("omnitrigger.exe"), "报告"))
        );
        assert_eq!(
            process_and_terms(r#"process="example tool.exe" 预算"#),
            Some((Some("example tool.exe"), "预算"))
        );
        assert_eq!(
            process_and_terms("process=notes.exe"),
            Some((Some("notes.exe"), ""))
        );
        assert_eq!(process_and_terms("预算 notes"), Some((None, "预算 notes")));
        for invalid in [
            "进程=",
            r#"process="" 标题"#,
            r#"process="tool.exe"紧邻标题"#,
            r#"process="未闭合"#,
            r"process=C:\work\tool.exe",
        ] {
            assert!(window_query(invalid).is_none(), "{invalid}");
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
