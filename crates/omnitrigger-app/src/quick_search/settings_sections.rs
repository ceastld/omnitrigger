use crate::appearance_ui::SettingsSection;

fn keywords(section: SettingsSection) -> &'static str {
    match section {
        SettingsSection::Theme => "外观 浅色 深色 跟随系统 theme appearance light dark",
        SettingsSection::Backup => "导入 导出 恢复 还原 zip backup restore export import",
        SettingsSection::Profiles => "配置档 测试副本 测试档 本机测试 test profile",
        SettingsSection::Updates => "版本 检查更新 升级 update version upgrade",
    }
}

pub(super) fn matching_settings(
    sections: &[SettingsSection],
    query: &str,
    limit: usize,
) -> Vec<SettingsSection> {
    let query = query.to_lowercase();
    let mut results: Vec<_> = sections
        .iter()
        .copied()
        .filter(|section| {
            query
                .split_whitespace()
                .all(|word| section.title().contains(word) || keywords(*section).contains(word))
        })
        .collect();
    results.sort_by_key(|section| super::title_rank(section.title(), &query));
    results.truncate(limit);
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quick_search::{Destination, RecentSearch, Results};

    #[test]
    fn finds_settings_by_labels_and_matches_the_available_controls() {
        let visible: Vec<_> = SettingsSection::ALL
            .into_iter()
            .filter(|section| section.available(true, true))
            .collect();
        assert_eq!(
            matching_settings(&visible, "深色", 4),
            [SettingsSection::Theme]
        );
        assert_eq!(
            matching_settings(&visible, "配置备份", 4),
            [SettingsSection::Backup]
        );
        assert_eq!(
            matching_settings(&visible, "TEST PROFILE", 4),
            [SettingsSection::Profiles]
        );
        assert!(matching_settings(&visible, "更新", 4).is_empty());
        assert_eq!(
            matching_settings(&SettingsSection::ALL, "更新", 4),
            [SettingsSection::Updates]
        );
        for query in ["主题", "设置：深色", "settings: theme"] {
            let results = Results::for_query(
                query,
                &[],
                &[],
                &[],
                &[],
                &visible,
                &RecentSearch::default(),
            );
            assert!(
                matches!(results, Results::Destinations(items) if items == [Destination::SettingsSection(SettingsSection::Theme)]),
                "{query}"
            );
        }
    }
}
