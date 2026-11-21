use omnitrigger_actions::CatalogAction;

use crate::plugins::PluginCatalogAction;

use super::{title_rank, Destination};

pub(super) struct SearchAction {
    title: String,
    provider: String,
    filter: String,
    title_key: String,
    search_key: String,
}

impl SearchAction {
    pub(super) fn from_package(action: &CatalogAction) -> Self {
        let title = &action.definition.title;
        let provider = &action.package_name;
        let filter = format!("{provider} / {title}");
        let keywords = format!("{} {}", action.package_id, action.definition.id);
        Self::new(title, provider, filter, &keywords)
    }

    pub(super) fn from_plugin(action: &PluginCatalogAction) -> Self {
        let title = &action.title;
        let provider = &action.plugin_name;
        let filter = format!("{provider} {title}");
        let keywords = format!("{} {}", action.reference.action_id, action.description);
        Self::new(title, provider, filter, &keywords)
    }

    fn new(title: &str, provider: &str, filter: String, keywords: &str) -> Self {
        Self {
            title: title.to_owned(),
            provider: provider.to_owned(),
            title_key: title.to_lowercase(),
            search_key: format!("{filter} {keywords}").to_lowercase(),
            filter,
        }
    }
}

pub(super) fn matching_actions(
    actions: &[SearchAction],
    query: &str,
    limit: usize,
) -> Vec<Destination> {
    if limit == 0 || query.is_empty() {
        return Vec::new();
    }
    let query = query.to_lowercase();
    let mut matches: Vec<_> = actions
        .iter()
        .filter(|action| {
            query
                .split_whitespace()
                .all(|word| action.search_key.contains(word))
        })
        .collect();
    matches.sort_by_key(|action| title_rank(&action.title_key, &query));

    let mut destinations = Vec::new();
    for action in matches {
        let destination = Destination::ActionLibraryEntry {
            title: action.title.clone(),
            provider: action.provider.clone(),
            filter: action.filter.clone(),
        };
        if !destinations.contains(&destination) {
            destinations.push(destination);
        }
        if destinations.len() == limit {
            break;
        }
    }
    destinations
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quick_search::{RecentDestinations, Results};

    #[test]
    fn ranks_action_names_and_builds_the_library_filter() {
        let actions = [
            SearchAction::new(
                "转换 JSON",
                "工具包",
                "工具包 / 转换 JSON".into(),
                "text.json",
            ),
            SearchAction::new("JSON", "文本工具", "文本工具 / JSON".into(), "format-json"),
        ];
        let exact = Destination::ActionLibraryEntry {
            title: "JSON".into(),
            provider: "文本工具".into(),
            filter: "文本工具 / JSON".into(),
        };
        for query in ["JSON", "文本工具 format-JSON"] {
            assert_eq!(
                matching_actions(&actions, query, 1).as_slice(),
                std::slice::from_ref(&exact)
            );
        }
        assert!(matching_actions(&actions, "", 4).is_empty());
        assert!(matching_actions(&actions, "无匹配内容", 4).is_empty());

        let result = Results::for_query(
            "动作：文本工具 format-JSON",
            &[],
            &[],
            &actions,
            &[],
            &RecentDestinations::default(),
        );
        assert!(matches!(
            result,
            Results::Destinations(items) if items == [
                exact,
                Destination::ActionLibraryQuery("文本工具 format-JSON".into())
            ]
        ));
    }
}
