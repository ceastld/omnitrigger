use std::{
    cell::RefCell,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use omnitrigger_windows::{list_shell_apps, ShellApp};

use super::{title_rank, Destination, Results};

const REFRESH_INTERVAL: Duration = Duration::from_secs(30);

pub(super) struct SearchApplication {
    app: ShellApp,
    title_key: String,
    identifier_key: String,
}

#[derive(Clone, Default)]
pub(super) enum ApplicationIndex {
    #[default]
    Unloaded,
    Ready {
        apps: Arc<[SearchApplication]>,
        loaded_at: Instant,
    },
    Unavailable(String),
}

#[derive(Clone, Default)]
pub(super) struct ApplicationCache(Rc<RefCell<ApplicationIndex>>);

impl ApplicationCache {
    pub(super) fn snapshot(&self) -> ApplicationIndex {
        self.0.borrow().clone()
    }

    pub(super) fn remember(&self, index: &ApplicationIndex) {
        if matches!(index, ApplicationIndex::Ready { .. }) {
            *self.0.borrow_mut() = index.clone();
        }
    }
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
        Self::Ready {
            apps: apps.into(),
            loaded_at: Instant::now(),
        }
    }

    pub(super) fn needs_refresh(&self, now: Instant) -> bool {
        match self {
            Self::Unloaded => true,
            Self::Ready { loaded_at, .. } => {
                now.saturating_duration_since(*loaded_at) >= REFRESH_INTERVAL
            }
            Self::Unavailable(_) => false,
        }
    }

    pub(super) fn results(&self, query: &str) -> Results {
        let apps = match self {
            Self::Unloaded => return Results::ApplicationsLoading,
            Self::Unavailable(error) => return Results::ApplicationsUnavailable(error.clone()),
            Self::Ready { apps, .. } => apps,
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
        matches!(self, Self::Ready { apps, .. } if apps.iter().any(|other| {
            other.app.title() == app.title() && &other.app != app
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retains_the_last_catalog_and_refreshes_it_without_hiding_matches() {
        let cache = ApplicationCache::default();
        let now = Instant::now();
        assert!(cache.snapshot().needs_refresh(now));
        cache.remember(&ApplicationIndex::Ready {
            apps: Arc::from([]),
            loaded_at: now,
        });
        cache.remember(&ApplicationIndex::Unavailable("读取失败".into()));
        assert!(!cache.snapshot().needs_refresh(now));
        let refreshed_at = now + REFRESH_INTERVAL;
        assert!(cache.snapshot().needs_refresh(refreshed_at));
        assert!(matches!(
            cache.snapshot().results(""),
            Results::Destinations(items) if items.is_empty()
        ));
    }
}
