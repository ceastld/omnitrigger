use omnitrigger_windows::{list_shell_apps, ShellApp};

use super::{title_rank, Destination, Results};

pub(super) struct SearchApplication {
    app: ShellApp,
    title_key: String,
    identifier_key: String,
}

#[derive(Default)]
pub(super) enum ApplicationIndex {
    #[default]
    Unloaded,
    Loading,
    Ready(Vec<SearchApplication>),
    Unavailable(String),
}

impl ApplicationIndex {
    pub(super) fn load() -> Self {
        let apps = match list_shell_apps() {
            Ok(apps) => apps,
            Err(error) => return Self::Unavailable(error),
        };
        let mut apps: Vec<_> = apps
            .into_iter()
            .map(|app| SearchApplication {
                title_key: app.title().to_lowercase(),
                identifier_key: app.identifier().to_lowercase(),
                app,
            })
            .collect();
        apps.sort_unstable_by(|a, b| {
            (&a.title_key, &a.identifier_key).cmp(&(&b.title_key, &b.identifier_key))
        });
        Self::Ready(apps)
    }

    pub(super) fn results(&self, query: &str) -> Results {
        let apps = match self {
            Self::Unloaded | Self::Loading => return Results::ApplicationsLoading,
            Self::Unavailable(error) => return Results::ApplicationsUnavailable(error.clone()),
            Self::Ready(apps) => apps,
        };
        let query = query.to_lowercase();
        let mut matches: Vec<_> = apps
            .iter()
            .filter(|app| {
                query
                    .split_whitespace()
                    .all(|word| app.title_key.contains(word) || app.identifier_key.contains(word))
            })
            .collect();
        matches.sort_by_key(|app| title_rank(&app.title_key, &query));
        Results::Destinations(
            matches
                .into_iter()
                .map(|app| Destination::InstalledApp(app.app.clone()))
                .collect(),
        )
    }

    pub(super) fn has_namesake(&self, app: &ShellApp) -> bool {
        matches!(self, Self::Ready(apps) if apps.iter().any(|other| {
            other.app.title() == app.title() && &other.app != app
        }))
    }
}
