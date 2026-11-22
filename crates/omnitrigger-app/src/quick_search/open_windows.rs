use omnitrigger_windows::{list_open_windows, OpenWindow};

use super::{examples::search_key, title_rank, Destination, Results};

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
        let Some((process, query)) = window_query(&query) else {
            return Results::OpenWindowsUnavailable(
                "填写程序文件名，例如 进程=notepad.exe。文件名含空格时用双引号包住。".into(),
            );
        };
        let terms: Vec<_> = query.split_whitespace().collect();
        let mut matches: Vec<_> = windows
            .iter()
            .filter(|window| process.is_none_or(|name| window.process_key == name))
            .filter_map(|window| {
                let process_key = if process.is_some() {
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
        Results::Destinations(
            matches
                .into_iter()
                .map(|(_, window)| Destination::OpenWindow(window.window.clone()))
                .collect(),
        )
    }
}

fn window_query(query: &str) -> Option<(Option<&str>, &str)> {
    let Some(input) = query
        .strip_prefix("进程=")
        .or_else(|| query.strip_prefix("process="))
    else {
        return Some((None, query));
    };
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
        .then_some((Some(process), terms.trim_start()))
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
    fn recognizes_exact_process_filters_and_separates_remaining_title_terms() {
        let query = search_key("进程＝ＯｍｎｉＴｒｉｇｇｅｒ．ｅｘｅ　报告");
        assert_eq!(
            window_query(&query),
            Some((Some("omnitrigger.exe"), "报告"))
        );
        assert_eq!(
            window_query(r#"process="example tool.exe" 预算"#),
            Some((Some("example tool.exe"), "预算"))
        );
        assert_eq!(
            window_query("process=notes.exe"),
            Some((Some("notes.exe"), ""))
        );
        assert_eq!(window_query("预算 notes"), Some((None, "预算 notes")));
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
