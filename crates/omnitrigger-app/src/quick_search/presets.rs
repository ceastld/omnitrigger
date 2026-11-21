use crate::presets::{TriggerPreset, PRESETS};

pub(super) fn matching_presets(query: &str, limit: usize) -> Vec<&'static TriggerPreset> {
    if limit == 0 {
        return Vec::new();
    }
    let query = query.to_lowercase();
    let mut matches: Vec<_> = PRESETS
        .iter()
        .filter_map(|preset| {
            let title = preset.title.to_lowercase();
            if !query.split_whitespace().all(|word| {
                [title.as_str(), preset.key, preset.group]
                    .iter()
                    .any(|field| field.contains(word))
            }) {
                return None;
            }
            let rank = super::title_rank(&title, &query).min(super::title_rank(preset.key, &query));
            Some((rank, preset))
        })
        .collect();
    matches.sort_by_key(|(rank, _)| *rank);
    matches
        .into_iter()
        .take(limit)
        .map(|(_, preset)| preset)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_search_matches_title_parts_and_keys() {
        for (query, expected) in [
            ("顶部 边缘", "edge-top"),
            ("屏幕区域 顶部", "edge-top"),
            ("RIGHT-QUICK-DRAG", "right-quick-drag"),
            ("右键 长按 300ms", "right-300"),
        ] {
            assert_eq!(matching_presets(query, 1)[0].key, expected, "{query}");
        }
        assert!(matching_presets("无对应的预设", 4).is_empty());
    }
}
