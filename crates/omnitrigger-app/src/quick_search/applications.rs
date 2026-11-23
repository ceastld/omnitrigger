use std::{
    cell::RefCell,
    rc::Rc,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use omnitrigger_windows::{list_shell_apps, ShellApp};

use super::{calculator::narrow_ascii_graphic, title_rank, Destination, Results};

const REFRESH_INTERVAL: Duration = Duration::from_secs(30);
const RECENT_APP_LIMIT: usize = 12;

pub(super) struct SearchApplication {
    app: ShellApp,
    title_key: String,
    identifier_key: String,
    initials: String,
}

impl SearchApplication {
    fn new(app: ShellApp) -> Self {
        let title_key = application_search_key(app.title());
        Self {
            initials: application_initials(&title_key),
            title_key,
            identifier_key: application_search_key(app.identifier()),
            app,
        }
    }

    fn rank(&self, query: &str) -> Option<u8> {
        application_rank(&self.title_key, &self.identifier_key, &self.initials, query)
    }
}

#[derive(Clone, Default)]
pub(crate) struct RecentApplications(Arc<Mutex<Vec<SearchApplication>>>);

impl RecentApplications {
    pub(crate) fn launch(&self, app: ShellApp) -> Result<(), String> {
        let recent = app.clone();
        app.open()?;
        self.record(recent);
        Ok(())
    }

    fn record(&self, app: ShellApp) {
        let mut history = self.0.lock().unwrap_or_else(|error| error.into_inner());
        let identifier = app.identifier();
        history.retain(|previous| {
            previous.app != app
                && (identifier.is_empty() || previous.app.identifier() != identifier)
        });
        history.insert(0, SearchApplication::new(app));
        history.truncate(RECENT_APP_LIMIT);
    }

    pub(super) fn results(&self, query: &str) -> Results {
        let query = application_search_key(query);
        let history = self.0.lock().unwrap_or_else(|error| error.into_inner());
        Results::Destinations(
            history
                .iter()
                .filter(|app| app.rank(&query).is_some())
                .map(|app| Destination::InstalledApp(app.app.clone()))
                .collect(),
        )
    }
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
        let mut apps: Vec<_> = apps.into_iter().map(SearchApplication::new).collect();
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
        let query = application_search_key(query);
        let mut matches: Vec<_> = apps
            .iter()
            .filter_map(|app| app.rank(&query).map(|rank| (rank, app)))
            .collect();
        matches.sort_by_key(|(rank, _)| *rank);
        Results::Destinations(
            matches
                .into_iter()
                .map(|(_, app)| Destination::InstalledApp(app.app.clone()))
                .collect(),
        )
    }

    pub(super) fn has_namesake(&self, app: &ShellApp) -> bool {
        matches!(self, Self::Ready { apps, .. } if apps.iter().any(|other| {
            other.app.title() == app.title() && &other.app != app
        }))
    }
}

fn application_search_key(text: &str) -> String {
    text.chars()
        .map(narrow_ascii_graphic)
        .collect::<String>()
        .to_lowercase()
}

fn application_initials(title: &str) -> String {
    title
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter_map(|word| word.chars().next())
        .map(|initial| initial.to_ascii_lowercase())
        .collect()
}

fn application_rank(title: &str, identifier: &str, initials: &str, query: &str) -> Option<u8> {
    let mut shorthand = query
        .split_whitespace()
        .filter(|word| !title.contains(word) && !identifier.contains(word));
    let Some(abbreviation) = shorthand.next() else {
        return Some(title_rank(title, query));
    };
    if abbreviation.len() < 2 || !initials.starts_with(abbreviation) || shorthand.next().is_some() {
        return None;
    }
    if abbreviation == query {
        return Some(if initials == abbreviation { 4 } else { 5 });
    }
    Some(6)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_name_and_identifier_matches_before_initials() {
        let title = "Visual Studio Code";
        let initials = application_initials(title);
        assert_eq!(initials, "vsc");
        assert_eq!(application_initials("Windows PowerShell (x86)"), "wpx");
        assert_eq!(application_initials("7-Zip File Manager"), "7zfm");

        let rank =
            |query: &str| application_rank("visual studio code", "installer", &initials, query);
        assert_eq!(rank("visual studio code"), Some(0));
        assert_eq!(rank("visual"), Some(1));
        assert_eq!(rank("studio"), Some(2));
        assert_eq!(rank("code installer"), Some(3));
        assert_eq!(rank("vsc"), Some(4));
        assert_eq!(rank("vs"), Some(5));
        assert_eq!(rank("vs code"), Some(6));
        assert_eq!(rank("code vsc installer"), Some(6));
        assert_eq!(rank(&application_search_key("ＶＳ　ＣＯＤＥ")), Some(6));
        assert_eq!(application_search_key("ΟΣ"), "ος");
        assert_eq!(rank("sc"), None);
        assert_eq!(rank("vs unknown"), None);
        assert_eq!(rank("vs vsc code"), None);
        assert_eq!(rank("vc code"), None);
    }

    #[test]
    #[ignore = "需要 Windows 应用目录"]
    fn remembers_recent_shell_applications_without_duplicates() {
        let catalog = list_shell_apps().unwrap();
        let first = catalog
            .iter()
            .find(|app| !app.identifier().is_empty())
            .unwrap();
        let second = catalog
            .iter()
            .find(|app| !app.identifier().is_empty() && app.identifier() != first.identifier())
            .unwrap();
        let history = RecentApplications::default();
        history.record(first.clone());
        history.record(second.clone());
        history.record(first.clone());
        let Results::Destinations(items) = history.results("") else {
            panic!("应返回应用列表");
        };
        assert_eq!(
            items,
            vec![
                Destination::InstalledApp(first.clone()),
                Destination::InstalledApp(second.clone()),
            ],
        );
        let Results::Destinations(matches) = history.results(first.identifier()) else {
            panic!("应返回筛选后的应用");
        };
        assert_eq!(matches.first(), items.first());
    }

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
