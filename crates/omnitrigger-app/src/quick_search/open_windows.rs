use omnitrigger_windows::{list_open_windows, OpenWindow};

use super::{title_rank, Destination, Results};

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
                        title_key: window.title.to_lowercase(),
                        process_key: window.process.as_deref().unwrap_or_default().to_lowercase(),
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
        let query = query.to_lowercase();
        let mut matches: Vec<_> = windows
            .iter()
            .filter(|window| {
                query.split_whitespace().all(|term| {
                    window.title_key.contains(term) || window.process_key.contains(term)
                })
            })
            .collect();
        if !query.is_empty() {
            matches.sort_by_key(|window| {
                title_rank(&window.title_key, &query).min(title_rank(&window.process_key, &query))
            });
        }
        Results::Destinations(
            matches
                .into_iter()
                .map(|window| Destination::OpenWindow(window.window.clone()))
                .collect(),
        )
    }
}
