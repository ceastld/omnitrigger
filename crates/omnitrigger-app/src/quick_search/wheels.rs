use crate::wheels::WheelDefinition;

pub(super) struct SearchWheel {
    pub(super) id: String,
    pub(super) name: String,
    id_key: String,
    name_key: String,
}

impl SearchWheel {
    pub(super) fn from_definition(wheel: &WheelDefinition) -> Self {
        Self {
            id: wheel.id.clone(),
            name: wheel.name.clone(),
            id_key: wheel.id.to_lowercase(),
            name_key: wheel.name.to_lowercase(),
        }
    }
}

pub(super) fn matching_wheels<'a>(
    wheels: &'a [SearchWheel],
    query: &str,
    limit: usize,
) -> Vec<&'a SearchWheel> {
    if limit == 0 {
        return Vec::new();
    }
    let query = query.to_lowercase();
    let mut matches: Vec<_> = wheels
        .iter()
        .filter(|wheel| {
            query
                .split_whitespace()
                .all(|word| wheel.name_key.contains(word) || wheel.id_key.contains(word))
        })
        .collect();
    matches.sort_by_key(|wheel| {
        super::title_rank(&wheel.name_key, &query).min(super::title_rank(&wheel.id_key, &query))
    });
    matches.truncate(limit);
    matches
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wheel_search_ranks_exact_names_and_matches_case_insensitive_ids() {
        let base = crate::wheels::Wheels::default().wheels.remove(0);
        let preview = WheelDefinition {
            id: "orbit-preview".into(),
            name: "编辑轨道预览".into(),
            ..base.clone()
        };
        let wheels = [
            SearchWheel::from_definition(&preview),
            SearchWheel::from_definition(&base),
        ];
        for (query, limit, expected) in [
            ("编辑轨道", 1, vec!["orbit-edit"]),
            ("ORBIT-Edit", 4, vec!["orbit-edit"]),
            ("预览 ORBIT", 4, vec!["orbit-preview"]),
            ("无对应轮盘", 4, vec![]),
        ] {
            assert_eq!(
                matching_wheels(&wheels, query, limit)
                    .into_iter()
                    .map(|wheel| wheel.id.as_str())
                    .collect::<Vec<_>>(),
                expected,
                "{query}",
            );
        }
    }
}
