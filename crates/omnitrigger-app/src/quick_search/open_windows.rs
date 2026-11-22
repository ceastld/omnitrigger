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
        let terms: Vec<_> = query.split_whitespace().collect();
        let mut matches: Vec<_> = windows
            .iter()
            .filter_map(|window| {
                window_rank(&window.title_key, &window.process_key, &query, &terms)
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
